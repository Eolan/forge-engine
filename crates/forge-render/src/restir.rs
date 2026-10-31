//! The lamps by ReSTIR DI (D-065, `shaders/restir.slang`): NVIDIA's RTXDI resamples, per pixel,
//! one light sample of the lamps over this frame's candidates, the pixel's last frame and its
//! neighbours' (Bitterli et al. 2020). The resolve lights the pixel with it
//! ([`crate::LampShadows::restir`]): with `--street-restir` of the street lamps alone, every one
//! however far, beside the light lists' share of the houses' lamps
//! ([`RestirParams::outside_only`], the owner's option 2 of 2026-10-18, behind a switch until
//! its light has a denoiser), or for D-065's comparison of every lamp (`--lamp-restir`).
//!
//! RTXDI stays outside Forge's code (the NVIDIA RTX SDKs licence, D-065): the shader includes its
//! headers from `rtxdi-sdk/Include` and this module is Forge's own host side, written from what
//! the headers' parameters mean. Without that directory [`LampRestir::available`] says no and the
//! switch is absent.
//!
//! Each frame, [`LampRestir::prepare`] writes the frame's parameters and the map of every lamp's
//! index from the last frame to this one (the kept lamps change as the camera moves), then
//! [`LampRestir::draw`] declares one pass, "shadow/lamp restir", before the resolve:
//! - the pixel's surface, stored for the next frame's reuse;
//! - its initial reservoir: [`RestirSettings::candidates`] points of the lamps of its place
//!   (lamps picked uniformly, or every one in turn where it has no more), one kept by its light,
//!   its shadow ray traced;
//! - RTXDI's spatiotemporal resampling against the last frame's reservoirs (TAA's
//!   reprojection) and three neighbours, its bias correction with visibility (`RayTraced`), its
//!   permutation sampling and naive discount off (#372);
//! - the kept sample's shadow ray, and its light times the reservoir's weight.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::{Mat4, Vec2};

use crate::lamps::{GpuLamp, LAMP_CAPACITY};
use crate::meshlet::{DrawTargets, MeshletRenderer, MeshletScene, MoversFrame};
use crate::probes::ProbeLight;

const SHADER: &str = "restir.slang";
const ENTRY: &str = "lamp_restir_main";
const PACK_ENTRY: &str = "lamp_restir_pack_main";

/// Mirrors `RestirPackPush` in `restir.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct RestirPackPush {
    diffuse_image: u32,
    specular_image: u32,
    packed_image: u32,
    width: u32,
    height: u32,
    pad: [u32; 3],
}

/// RTXDI's reservoirs are laid out in square blocks of this many pixels a side
/// (`RTXDI_RESERVOIR_BLOCK_SIZE`).
const BLOCK: u32 = 16;

/// Bytes of a stored reservoir (`RTXDI_PackedDIReservoir`).
const RESERVOIR_BYTES: u64 = 24;

/// The spatial reuse's offsets: points of the unit disc, which RTXDI scales by the sampling
/// radius and walks from a random start.
const NEIGHBOR_OFFSETS: usize = 8192;

/// Where the lamps' index maps sit in a frame's block.
const TO_PREVIOUS: u64 = 512;
const TO_CURRENT: u64 = TO_PREVIOUS + 4 * LAMP_CAPACITY as u64;
const FRAME_BYTES: u64 = TO_CURRENT + 4 * LAMP_CAPACITY as u64;

/// RTXDI's bias correction with visibility (`RTXDI_BIAS_CORRECTION_RAY_TRACED`, #372): a shadow
/// ray from each neighbour to the kept sample. Without it (`Basic`, pairwise MIS) a neighbour
/// counted the lamps round a wall from it as its own, and over three neighbours the Bistro's night
/// lost 40 % of its light. (`Basic` and `RayTraced` first darkened the Cornell box's lit walls
/// 10-15 %: RTXDI's naive discount, below, not the correction.)
const BIAS_CORRECTION_RAY_TRACED: u32 = 3;

/// How the lamps are resampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestirSettings {
    /// The initial candidates a pixel takes: points of its place's lamps (16, #372: with 63
    /// frames of REBLUR's history, a still camera's shimmer at the Bistro's night 46,000 → 43,000
    /// pixels for 0.3 ms).
    pub candidates: u32,
    /// The most frames a reservoir's history counts (RTXDI's default, 20).
    pub max_history: u32,
    /// The last frame's reservoirs the reuse takes, the pixel's own (reprojected) first, and where
    /// the last frame shows nothing of the pixel, how many round it. 3 and 3 (#372): at the
    /// Bistro's night a still camera's shimmer 42,900 → 24,700 pixels (five: 23,200 for 1 ms
    /// more); RTXDI's 8 where there is no history lost 2 % more of a turning camera's light.
    pub spatial_samples: [u32; 2],
    /// Pixels around the pixel the neighbours come from (RTXDI's default, 32).
    pub radius: f32,
    /// How far a neighbour's depth and normal may stray, relative and as a cosine (RTXDI's
    /// defaults, 0.1 and 0.5).
    pub thresholds: [f32; 2],
}

impl Default for RestirSettings {
    fn default() -> Self {
        Self {
            candidates: 16,
            max_history: 20,
            spatial_samples: [3, 3],
            radius: 32.0,
            thresholds: [0.1, 0.5],
        }
    }
}

/// What a frame's resampling needs from the camera.
#[derive(Clone, Copy, Debug)]
pub struct RestirParams {
    /// The view-projection, camera-relative and unjittered.
    pub clip_from_relative: Mat4,
    /// TAA's: the previous frame's unjittered clip coordinates from this frame's
    /// ([`crate::taa::TaaFrame::previous_from_current`]).
    pub previous_from_current: Mat4,
    /// Nothing on screen matches the frame before (TAA's reset): no reuse.
    pub reset: bool,
    /// The street lamps alone ([`crate::lamps::Lamp::outside`]), the light lists the rest
    /// ([`crate::LampShadows::restir`]); else every lamp.
    pub outside_only: bool,
    /// The frame's lamps from this one on (they come nearest first), the light lists the nearer
    /// (#372, [`crate::MeshletRenderer::lamp_restir_from`]); 0: every lamp.
    pub first_lamp: u32,
    /// The frame's size.
    pub extent: vk::Extent2D,
}

/// Mirrors `RTXDI_RuntimeParameters`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuRuntime {
    neighbor_offset_mask: u32,
    active_checkerboard_field: u32,
    frame_index: u32,
    pad: u32,
}

/// Mirrors `RTXDI_ReservoirBufferParameters`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuReservoirParams {
    block_row_pitch: u32,
    array_pitch: u32,
    pad: [u32; 2],
}

/// Mirrors `RTXDI_DISpatioTemporalResamplingParameters`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuSpatioTemporal {
    depth_threshold: f32,
    normal_threshold: f32,
    bias_correction: u32,
    max_history: u32,
    permutation_sampling: u32,
    uniform_random: u32,
    visibility_shortcut: u32,
    samples: u32,
    disocclusion_samples: u32,
    radius: f32,
    material_test: u32,
    discount_naive: u32,
}

/// Mirrors `RestirFrame` in `restir.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuRestirFrame {
    clip_from_relative: [f32; 16],
    previous_from_current: [f32; 16],
    runtime: GpuRuntime,
    reservoir_params: GpuReservoirParams,
    spatiotemporal: GpuSpatioTemporal,
    reservoirs: u64,
    neighbor_offsets: u64,
    surfaces: u64,
    places: u64,
    to_previous: u64,
    to_current: u64,
    surface_width: u32,
    surface_pitch: u32,
    current: u32,
    candidates: u32,
    history: u32,
    outside_only: u32,
    first_lamp: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuRestirFrame>() == 288);
const _: () = assert!(std::mem::size_of::<GpuRestirFrame>() as u64 <= TO_PREVIOUS);

/// Mirrors `RestirPush` in `restir.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct RestirPush {
    frame: u64,
    probes: u64,
    restir: u64,
    vis_image: u32,
    light_image: u32,
    width: u32,
    height: u32,
    diffuse_image: u32,
    specular_image: u32,
    guide_image: u32,
    pad: [u32; 3],
    hit_distance: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<RestirPush>() == 80);

/// The lux in one unit of [`RestirLight::diffuse`] and [`RestirLight::specular`] (#372,
/// `RESTIR_DENOISE_SCALE` in `meshlet.slang`): REBLUR keeps half floats, and a bulb lights a pixel
/// by it with up to 65,000 lux.
pub const DENOISE_SCALE: f32 = 256.0;

/// What [`LampRestir::draw`] lights a frame with.
#[derive(Clone, Copy, Debug)]
pub struct RestirLight {
    /// Each pixel's sample: its light in lux (rgb) and the lamp it came from + 1 (a, 0: none), for
    /// [`crate::LampShadows::restir`].
    pub light: ImageHandle,
    /// Its diffuse light for REBLUR ([`LampDenoiser`], #372), by the shading normal: over
    /// [`DENOISE_SCALE`] in YCoCg, and the distance to the sample's point normalised as REBLUR
    /// reads it ([`crate::diffuse_bounce::HIT_DISTANCE`]).
    pub diffuse: ImageHandle,
    /// Its highlight for REBLUR, over the surface's directional albedo, likewise (the distance
    /// normalised for the surface's roughness).
    pub specular: ImageHandle,
    /// REBLUR's guide ([`forge_gpu::nrd::NORMAL_ROUGHNESS_FORMAT`]): the shading normal and the
    /// linear roughness both were lit with.
    pub guide: ImageHandle,
}

/// ReSTIR's light denoised (#372): NVIDIA's REBLUR (NRD's diffuse and specular denoiser), its own
/// instance and history, on the guide ReSTIR's pass writes and the sun's shadow rays' view depth
/// and TAA's motion vectors. Its passes are `shadow/lamp REBLUR ...`.
pub struct LampDenoiser {
    device: Arc<Device>,
    reblur: forge_gpu::nrd::ReblurDiffuseSpecular,
    dir: std::path::PathBuf,
    reset: bool,
}

impl LampDenoiser {
    /// Loads NRD's REBLUR from `dir` (`nrd-sdk/bin`) for frames of `extent`. Fails without the
    /// library, or built without the `nrd` feature.
    pub fn reblur(
        device: &Arc<Device>,
        dir: &std::path::Path,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        Ok(Self {
            device: Arc::clone(device),
            reblur: forge_gpu::nrd::ReblurDiffuseSpecular::load(device, dir, extent)?,
            dir: dir.to_owned(),
            reset: true,
        })
    }

    /// Recreates the denoiser for a new frame size. The caller makes sure the GPU is done with the
    /// old one's frames.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        if self.reblur.extent() != extent {
            self.reblur =
                forge_gpu::nrd::ReblurDiffuseSpecular::load(&self.device, &self.dir, extent)?;
        }
        self.reset = true;
        Ok(())
    }

    /// Discards the history at the next frame (a cut, or a frame it did not denoise).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// The frame's settings for `camera` (the sun's shadows', as the bounce's denoiser).
    pub fn frame(&mut self, camera: forge_gpu::nrd::NrdCamera) -> forge_gpu::nrd::ReblurFrame {
        let frame = forge_gpu::nrd::ReblurFrame {
            camera: forge_gpu::nrd::NrdCamera {
                reset: camera.reset || self.reset,
                ..camera
            },
            hit_distance: crate::diffuse_bounce::HIT_DISTANCE,
            // REBLUR's most (#372): with the bounce's 30, a still camera's frames 10 apart differed
            // on 95,000 pixels at the Bistro's night (6.6 %), with 63 on 46,000, for 2 % of the
            // light (the lists, 4,900 and 6,900).
            max_accumulated_frames: 63,
            max_fast_accumulated_frames: 6,
            checkerboard: false,
        };
        self.reset = false;
        frame
    }

    /// Declares REBLUR's passes over `light` (this frame's, [`LampRestir::draw`]), the sun's
    /// shadow rays' `view_z` and TAA's `motion`, and returns the denoised diffuse light and
    /// highlight for [`crate::LampShadows`].
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        light: RestirLight,
        view_z: ImageHandle,
        motion: ImageHandle,
        frame: &forge_gpu::nrd::ReblurFrame,
    ) -> Result<(ImageHandle, ImageHandle)> {
        self.reblur.denoise(
            graph,
            slot,
            forge_gpu::nrd::ReblurSpecularImages {
                diffuse: light.diffuse,
                specular: light.specular,
                normal_roughness: light.guide,
                view_z,
                motion,
            },
            frame,
        )
    }
}

/// The pass, its reservoirs and the surfaces their reuse reads back.
pub struct LampRestir {
    /// How the lamps are resampled.
    pub settings: RestirSettings,
    pipeline: Pipeline,
    pack_pipeline: Pipeline,
    /// Two arrays of reservoirs in RTXDI's block layout: this frame's and the last.
    reservoirs: GraphBuffer,
    /// Per pixel of the two frames, the surface (16 bytes), then its place (4 bytes).
    surfaces: GraphBuffer,
    offsets: Buffer,
    /// Per frame slot: the frame's block and the lamps' index maps, written by the CPU.
    frames: Vec<Buffer>,
    /// The largest frame the buffers hold.
    size: vk::Extent2D,
    /// What [`Self::prepare`] wrote for the frame being declared.
    current: u32,
    usable: bool,
    frame_index: u32,
    /// The last frame's lamps, by their middle and radius, in its order.
    last_lamps: Vec<[u32; 4]>,
    last_extent: vk::Extent2D,
}

impl LampRestir {
    /// Whether the comparison can run here: ray queries, and RTXDI's headers in `rtxdi-sdk/`.
    pub fn available(device: &Device, shaders: &ShaderCompiler) -> bool {
        device.features().ray_query && shaders.sdk_present(SHADER)
    }

    /// Compiles the pass and makes its buffers for frames up to `size`.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, size: vk::Extent2D) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile(SHADER, ENTRY, ShaderStage::Compute)?,
            "lamp restir",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, ENTRY),
            push_constant_bytes: std::mem::size_of::<RestirPush>() as u32,
            name: "lamp restir",
        });
        device.destroy_shader_module(module);
        let module = device.create_shader_module(
            &shaders.compile(SHADER, PACK_ENTRY, ShaderStage::Compute)?,
            "lamp restir pack",
        )?;
        let pack_pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, PACK_ENTRY),
            push_constant_bytes: std::mem::size_of::<RestirPackPush>() as u32,
            name: "lamp restir pack",
        });
        device.destroy_shader_module(module);
        let (_, array_pitch) = reservoir_pitches(size);
        let pixels = u64::from(size.width) * u64::from(size.height);
        let storage = |size: u64, name: &str| {
            device.create_buffer(BufferDesc {
                size,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })
        };
        let reservoirs = storage(
            2 * u64::from(array_pitch) * RESERVOIR_BYTES,
            "lamp restir reservoirs",
        )?;
        let surfaces = storage(2 * pixels * (16 + 4), "lamp restir surfaces")?;
        let offsets = device.create_buffer_with_data(
            &neighbor_offsets(),
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Textures,
            "lamp restir neighbour offsets",
        )?;
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: FRAME_BYTES,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("lamp restir frame {i}"),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            settings: RestirSettings::default(),
            pipeline: pipeline?,
            pack_pipeline: pack_pipeline?,
            reservoirs: GraphBuffer::new(reservoirs),
            surfaces: GraphBuffer::new(surfaces),
            offsets,
            frames,
            size,
            current: 0,
            usable: false,
            frame_index: 0,
            last_lamps: Vec::new(),
            last_extent: vk::Extent2D::default(),
        })
    }

    /// Bytes of the reservoirs and surfaces kept between frames.
    pub fn bytes(&self) -> u64 {
        self.reservoirs.size() + self.surfaces.size() + self.offsets.size()
    }

    /// Writes frame `slot`'s parameters: `kept` is the frame's lamps
    /// ([`MeshletScene::set_lamps`]). Call it each frame before [`Self::draw`].
    pub fn prepare(&mut self, slot: FrameSlot, kept: &[GpuLamp], params: RestirParams) {
        let extent = params.extent;
        self.usable = extent.width <= self.size.width && extent.height <= self.size.height;
        let history = self.usable
            && self.frame_index > 0
            && !params.reset
            && extent == self.last_extent
            && !self.last_lamps.is_empty();
        self.current = self.frame_index % 2;
        // A lamp is known from one frame to the next by its middle and radius (the bits).
        let lamps: Vec<[u32; 4]> = kept
            .iter()
            .map(|l| l.position_radius.map(f32::to_bits))
            .collect();
        let index = |list: &[[u32; 4]]| -> HashMap<[u32; 4], u32> {
            list.iter()
                .enumerate()
                .map(|(i, l)| (*l, i as u32))
                .collect()
        };
        let (now, before) = (index(&lamps), index(&self.last_lamps));
        let mut to_previous = [u32::MAX; LAMP_CAPACITY];
        let mut to_current = [u32::MAX; LAMP_CAPACITY];
        for (i, lamp) in lamps.iter().enumerate().take(LAMP_CAPACITY) {
            to_previous[i] = before.get(lamp).copied().unwrap_or(u32::MAX);
        }
        for (i, lamp) in self.last_lamps.iter().enumerate().take(LAMP_CAPACITY) {
            to_current[i] = now.get(lamp).copied().unwrap_or(u32::MAX);
        }
        let (block_row_pitch, array_pitch) = reservoir_pitches(self.size);
        let s = self.settings;
        let buffer = &self.frames[slot.index];
        let gpu = GpuRestirFrame {
            clip_from_relative: params.clip_from_relative.to_cols_array(),
            previous_from_current: params.previous_from_current.to_cols_array(),
            runtime: GpuRuntime {
                neighbor_offset_mask: NEIGHBOR_OFFSETS as u32 - 1,
                active_checkerboard_field: 0,
                frame_index: self.frame_index,
                pad: 0,
            },
            reservoir_params: GpuReservoirParams {
                block_row_pitch,
                array_pitch,
                pad: [0; 2],
            },
            spatiotemporal: GpuSpatioTemporal {
                depth_threshold: s.thresholds[0],
                normal_threshold: s.thresholds[1],
                bias_correction: BIAS_CORRECTION_RAY_TRACED,
                max_history: s.max_history,
                // Off (#372): reusing the last frame's reservoir of a pixel up to 3 px away
                // (RTXDI's permutation, for a denoiser's sake) darkened the Bistro's night by
                // 0.2 stops against the light lists with the same lamps, whichever bias
                // correction; without it the reuse matches the resampling without reuse.
                permutation_sampling: 0,
                uniform_random: hash(self.frame_index),
                visibility_shortcut: 0,
                samples: s.spatial_samples[0],
                disocclusion_samples: s.spatial_samples[1],
                radius: s.radius,
                material_test: 0,
                // Off (#372): RTXDI skips a neighbour of two samples or fewer as naive, and a
                // reservoir starts with one, so a pixel's history never grew past it; under `Basic`
                // the skipped neighbour still counted in the normalisation, half the light (the
                // Cornell box's lit walls 0.892 of the lists' light, 0.995 without it).
                discount_naive: 0,
            },
            reservoirs: self.reservoirs.address(),
            neighbor_offsets: self.offsets.address(),
            surfaces: self.surfaces.address(),
            places: self.surfaces.address()
                + 2 * 16 * u64::from(self.size.width) * u64::from(self.size.height),
            to_previous: buffer.address() + TO_PREVIOUS,
            to_current: buffer.address() + TO_CURRENT,
            surface_width: self.size.width,
            surface_pitch: self.size.width * self.size.height,
            current: self.current,
            candidates: s.candidates.max(1),
            history: u32::from(history),
            outside_only: u32::from(params.outside_only),
            first_lamp: params.first_lamp,
            pad: 0,
        };
        buffer.write(0, &[gpu]);
        buffer.write(TO_PREVIOUS, &to_previous);
        buffer.write(TO_CURRENT, &to_current);
        self.last_lamps = lamps;
        self.last_extent = extent;
        self.frame_index = self.frame_index.wrapping_add(1);
    }

    /// Declares "shadow/lamp restir pack" (#372): the lamps' `diffuse` light and `specular`
    /// highlight (REBLUR's, or the pass's own without it) packed into one image for the resolve,
    /// beside the light lists' share of the nearer lamps
    /// ([`crate::MeshletRenderer::lamp_restir_from`]): the diffuse light (rgb) and the highlight's
    /// luminance over the albedo (a), lux over [`DENOISE_SCALE`].
    pub fn pack<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        diffuse: ImageHandle,
        specular: ImageHandle,
        extent: vk::Extent2D,
    ) -> ImageHandle {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let packed = graph.transient(TransientDesc {
            name: "lamp restir packed",
            width: extent.width,
            height: extent.height,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let pipeline = &self.pack_pipeline;
        graph
            .pass("shadow/lamp restir pack")
            .image(diffuse, ImageAccess::Sampled(compute))
            .image(specular, ImageAccess::Sampled(compute))
            .image(packed, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &RestirPackPush {
                        diffuse_image: resources.sampled(diffuse).0,
                        specular_image: resources.sampled(specular).0,
                        packed_image: resources.storage(packed, 0).0,
                        width: extent.width,
                        height: extent.height,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });
        packed
    }

    /// Declares "shadow/lamp restir" for frame `slot` of `extent` (the resolve's), lighting the
    /// visibility buffer of `targets` with `scene`'s lamps; `probes` are the probes the resolve
    /// lights with (their regions are the lamps' places). Returns the lamps' light, each pixel's
    /// sample and the same for REBLUR ([`RestirLight`]): `None`, and nothing declared, when the
    /// frame lights no lamp or is larger than the buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        renderer: &MeshletRenderer,
        scene: &MeshletScene,
        targets: DrawTargets,
        extent: vk::Extent2D,
        probes: Option<ProbeLight>,
    ) -> Option<RestirLight> {
        if !self.usable || scene.lamp_count(slot) == 0 {
            return None;
        }
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let light = graph.transient(TransientDesc {
            name: "lamp restir light",
            width: extent.width,
            height: extent.height,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let diffuse = graph.transient(TransientDesc {
            name: "lamp restir diffuse",
            width: extent.width,
            height: extent.height,
            format: forge_gpu::nrd::RADIANCE_FORMAT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let specular = graph.transient(TransientDesc {
            name: "lamp restir specular",
            width: extent.width,
            height: extent.height,
            format: forge_gpu::nrd::RADIANCE_FORMAT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let guide = graph.transient(TransientDesc {
            name: "lamp restir guide",
            width: extent.width,
            height: extent.height,
            format: forge_gpu::nrd::NORMAL_ROUGHNESS_FORMAT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let reservoirs = graph.import_buffer(&self.reservoirs);
        let surfaces = graph.import_buffer(&self.surfaces);
        let frame = renderer.frame_address(slot);
        let restir = self.frames[slot.index].address();
        let pipeline = &self.pipeline;
        let mut builder = graph
            .pass("shadow/lamp restir")
            .image(targets.visibility, ImageAccess::Sampled(compute))
            .image(light, ImageAccess::StorageWrite(compute))
            .image(diffuse, ImageAccess::StorageWrite(compute))
            .image(specular, ImageAccess::StorageWrite(compute))
            .image(guide, ImageAccess::StorageWrite(compute))
            .buffer(reservoirs, BufferAccess::ShaderReadWrite(compute))
            .buffer(surfaces, BufferAccess::ShaderReadWrite(compute))
            .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
            .buffer(targets.pages, BufferAccess::ShaderRead(compute))
            .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
            .with(|b| MoversFrame::declare(targets.movers, b, compute, true));
        if let Some(p) = probes {
            builder = builder
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .buffer(p.region, BufferAccess::ShaderRead(compute));
        }
        builder.run(move |resources, commands| {
            commands.bind_pipeline(pipeline);
            commands.push_constants(
                pipeline,
                &RestirPush {
                    frame,
                    probes: probes.map_or(0, |p| p.address),
                    restir,
                    vis_image: resources.sampled(targets.visibility).0,
                    light_image: resources.storage(light, 0).0,
                    width: extent.width,
                    height: extent.height,
                    diffuse_image: resources.storage(diffuse, 0).0,
                    specular_image: resources.storage(specular, 0).0,
                    guide_image: resources.storage(guide, 0).0,
                    pad: [0; 3],
                    hit_distance: {
                        let [a, b, c] = crate::diffuse_bounce::HIT_DISTANCE;
                        [a, b, c, 0.0]
                    },
                },
            );
            commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
            Ok(())
        });
        Some(RestirLight {
            light,
            diffuse,
            specular,
            guide,
        })
    }
}

/// RTXDI's reservoir layout for frames up to `size`: a row of blocks' reservoirs, and an array's.
fn reservoir_pitches(size: vk::Extent2D) -> (u32, u32) {
    let row = size.width.div_ceil(BLOCK) * BLOCK * BLOCK;
    (row, row * size.height.div_ceil(BLOCK))
}

/// The spatial reuse's offsets: [`NEIGHBOR_OFFSETS`] points of the unit disc, the R2 sequence's
/// (Roberts 2018, the plastic number's) points of the square that fall in it, so any run of them
/// covers the disc evenly.
fn neighbor_offsets() -> Vec<[f32; 2]> {
    const G: f64 = 1.324_717_957_244_746;
    let step = [1.0 / G, 1.0 / (G * G)];
    let mut out = Vec::with_capacity(NEIGHBOR_OFFSETS);
    let mut n = 1.0;
    while out.len() < NEIGHBOR_OFFSETS {
        let p = Vec2::new(
            ((0.5 + step[0] * n) % 1.0) as f32,
            ((0.5 + step[1] * n) % 1.0) as f32,
        ) * 2.0
            - 1.0;
        if p.length_squared() <= 1.0 {
            out.push(p.to_array());
        }
        n += 1.0;
    }
    out
}

/// A frame's random number for RTXDI's permutation sampling (a 32-bit integer hash, Wellons'
/// "lowbias32").
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservoirs_cover_every_pixel_in_blocks() {
        let (row, array) = reservoir_pitches(vk::Extent2D {
            width: 2560,
            height: 1440,
        });
        assert_eq!(row, 160 * 256);
        assert_eq!(array, row * 90);
        // A size that is not a whole number of blocks rounds up.
        let (row, array) = reservoir_pitches(vk::Extent2D {
            width: 17,
            height: 1,
        });
        assert_eq!((row, array), (512, 512));
    }

    #[test]
    fn neighbour_offsets_fill_the_disc_evenly() {
        let offsets = neighbor_offsets();
        assert_eq!(offsets.len(), NEIGHBOR_OFFSETS);
        assert!(offsets.iter().all(|p| p[0] * p[0] + p[1] * p[1] <= 1.0));
        // Each quadrant holds about a quarter of them.
        let upper_right = offsets.iter().filter(|p| p[0] > 0.0 && p[1] > 0.0).count();
        assert!((upper_right as f32 / NEIGHBOR_OFFSETS as f32 - 0.25).abs() < 0.01);
    }
}
