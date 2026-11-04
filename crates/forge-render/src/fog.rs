//! Fog lit by the lamps (D-099, `shaders/fog.slang`): authored fog volumes (boxes with a
//! density, a height falloff, a colour and an absorption) laid into a froxel volume in front of
//! the camera, D-032's layout ([`crate::dust`]), each froxel lit by the frame's lamps whose reach
//! meets its column of the lamps' grid (#386, unshadowed) and by the sun through a shadow ray.
//!
//! Three graph passes a frame, over the HDR image before the upscaler:
//! - `fog/light`: per froxel (160 × 90 × 64, quadratic in depth to the volume's far end), the
//!   medium (the boxes there in priority order, each over the others by its edges' fade) and the
//!   light it scatters
//!   towards the camera (Henyey–Greenstein), at the froxel's middle (a jittered point shimmered,
//!   and lit a froxel white beside a bulb);
//! - `fog/integrate`: per froxel column, the light gathered and the transmittance to each slice;
//! - `fog/apply`: per pixel, the colour dimmed by the fog in front of it and brightened by what
//!   it scatters, by the pixel's depth (one trilinear read of the volume).
//!
//! The patent rules D-099 keeps (its search, 2026-11-04): one read of the volume a pixel, the
//! lamps' lists per world column filled by their reach, their weights analytic, one colour a
//! froxel.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler,
    ShaderStage, TransientDesc, vk,
};
use glam::{Mat4, Quat, Vec3};

const FROXELS_X: u32 = 160;
const FROXELS_Y: u32 = 90;
const SLICES: u32 = 64;
const GROUP: u32 = 8;
/// The most fog volumes a frame lays into its froxels: those nearest the camera that reach the
/// volume.
pub const MAX_FOG_BOXES: usize = 32;
/// The most lamps of the fog alone a frame weighs: those nearest the camera whose reach meets the
/// volume.
pub const MAX_FOG_LAMPS: usize = 256;

/// A fog volume: a box of fog in the scene frame.
#[derive(Clone, Copy, Debug)]
pub struct FogBox {
    /// The scene frame to the box's own: a unit cube centred on its origin.
    pub to_box: Mat4,
    /// Its centre and the radius of the sphere round it, metres (to keep only those near the
    /// volume).
    pub centre: Vec3,
    /// See `centre`.
    pub radius: f32,
    /// Its extinction at its floor, per metre.
    pub density: f32,
    /// How fast it thins with height above its floor, per metre (0: evenly).
    pub height_falloff: f32,
    /// The share of its extinction absorbed rather than scattered (0 to 1).
    pub absorption: f32,
    /// Where boxes overlap, the highest priority's medium.
    pub priority: f32,
    /// Its height, metres (the falloff's and the blend's scale).
    pub height: f32,
    /// How far in from its faces it fades in, metres.
    pub blend: f32,
    /// The colour its scattering takes (linear).
    pub tint: Vec3,
}

impl FogBox {
    /// A box of `size` metres (x, y up, z) at `centre`, turned by `rotation`, of `density` per
    /// metre at its floor, thinning by `height_falloff` per metre above it, absorbing that share
    /// of its extinction, its priority among overlapping boxes, fading in over `blend` metres
    /// from its faces, its scattering's colour `tint`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        centre: Vec3,
        rotation: Quat,
        size: Vec3,
        density: f32,
        height_falloff: f32,
        absorption: f32,
        priority: f32,
        blend: f32,
        tint: Vec3,
    ) -> Self {
        let size = size.abs().max(Vec3::splat(0.01));
        Self {
            to_box: Mat4::from_scale_rotation_translation(size, rotation, centre).inverse(),
            centre,
            radius: size.length() * 0.5,
            density,
            height_falloff,
            absorption,
            priority,
            height: size.y,
            blend,
            tint,
        }
    }

    /// The same box in a frame whose origin stands at `-offset` in this one: moved by `offset`.
    pub fn moved(&self, offset: Vec3) -> Self {
        Self {
            to_box: self.to_box * Mat4::from_translation(-offset),
            centre: self.centre + offset,
            ..*self
        }
    }
}

/// Mirrors `FogBox` in `fog.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFogBox {
    rows: [[f32; 4]; 3],
    medium: [f32; 4],
    shape: [f32; 4],
    tint: [f32; 4],
}

/// Mirrors `Fog` in `fog.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFog {
    inv_view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    global: [f32; 4],
    scales: [f32; 4],
    frame: u64,
    boxes: u64,
    fog_lamps: u64,
    box_count: u32,
    light: u32,
    light_sampled: u32,
    volume: u32,
    volume_sampled: u32,
    depth: u32,
    color: u32,
    width: u32,
    height: u32,
    fog_lamp_count: u32,
}

/// What a frame's fog needs.
#[derive(Clone, Copy)]
pub struct FogParams {
    /// The drawing camera's view-projection, camera-relative, jitter included.
    pub view_proj: Mat4,
    /// The camera in the scene frame.
    pub camera: Vec3,
    /// The volume's far end, metres.
    pub far: f32,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sunlight's colour.
    pub sun_color: Vec3,
    /// The sun's illuminance times the frame's exposure.
    pub sun_illuminance: f32,
    /// The phase's anisotropy g (0: every way alike).
    pub anisotropy: f32,
    /// The resolve's frame record ([`crate::MeshletRenderer::frame_address`]): its lamps, their
    /// grid, the TLAS.
    pub frame: u64,
    /// The movers' structures, for the shadow rays.
    pub movers: Option<crate::MoversFrame>,
    /// The scene's own fog everywhere, under its boxes (none: the boxes' alone).
    pub global: Option<GlobalFog>,
    /// How strongly the sun's light, then the lamps', scatter in the fog (1: whole). An authored
    /// city's environment lets a hundredth of its sun into its fog.
    pub light_scales: [f32; 2],
}

/// A scene's fog everywhere (D-099): an authored environment's volumetric fog, the boxes laid
/// over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalFog {
    /// Its extinction, per metre.
    pub density: f32,
    /// The height in the scene frame up to which it is even, metres.
    pub height: f32,
    /// How fast it thins above that height, per metre.
    pub falloff: f32,
    /// The share of its extinction absorbed (0 to 1).
    pub absorption: f32,
}

/// The passes' pipelines and the per-frame parameters and boxes.
pub struct FogVolume {
    light: Pipeline,
    integrate: Pipeline,
    apply: Pipeline,
    params: Vec<Buffer>,
    boxes: Vec<Buffer>,
    lamps: Vec<Buffer>,
}

impl FogVolume {
    /// Compiles the passes; `fog_light_main` traces, so the device needs ray queries.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("fog.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: 8,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let buffers = |size: usize, name: &str| {
            (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device.create_buffer(BufferDesc {
                        size: size as u64,
                        usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                        location: MemoryLocation::CpuToGpu,
                        category: MemoryCategory::Frame,
                        name: &format!("{name} {i}"),
                    })
                })
                .collect::<Result<Vec<_>>>()
        };
        Ok(Self {
            light: compute("fog_light_main", "fog light")?,
            integrate: compute("fog_integrate_main", "fog integrate")?,
            apply: compute("fog_apply_main", "fog apply")?,
            params: buffers(std::mem::size_of::<GpuFog>(), "fog")?,
            boxes: buffers(
                std::mem::size_of::<GpuFogBox>() * MAX_FOG_BOXES,
                "fog boxes",
            )?,
            lamps: buffers(
                std::mem::size_of::<crate::lamps::GpuLamp>() * MAX_FOG_LAMPS,
                "fog lamps",
            )?,
        })
    }

    /// Declares the three passes over `color` (the HDR image, read and written) behind and over
    /// what `depth` shows, the fog of `boxes` that reach the volume (the nearest
    /// [`MAX_FOG_BOXES`]), lit by the frame's lamps (each by its [`crate::lamps::Lamp::fog`]
    /// share) and by `fog_lamps`, lamps of the fog alone in the scene frame (the nearest
    /// [`MAX_FOG_LAMPS`] whose reach meets the volume), over the scene's fog everywhere
    /// ([`FogParams::global`]). A box of no fog cuts a hole in that fog. Nothing when there is no
    /// fog to draw. The volume ends [`FogParams::far`] from the camera: past it, the fog
    /// everywhere adds no more.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        params: FogParams,
        boxes: &[FogBox],
        fog_lamps: &[crate::lamps::Lamp],
        depth: ImageHandle,
        color: ImageHandle,
        extent: vk::Extent2D,
    ) {
        // The boxes nearest the camera by their bounding spheres (a large box round the camera
        // first, whatever its centre's distance), then laid in priority order, ties by their order
        // in the scene: the shader blends each covering box over those before it.
        let gap = |b: &FogBox| b.centre.distance(params.camera) - b.radius;
        let mut near: Vec<(usize, &FogBox)> = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.density >= 0.0 && gap(b) < params.far)
            .collect();
        let global_fog = params.global.is_some_and(|g| g.density > 0.0);
        if (near.is_empty() || !near.iter().any(|(_, b)| b.density > 0.0)) && !global_fog {
            return;
        }
        near.sort_by(|a, b| gap(a.1).total_cmp(&gap(b.1)));
        near.truncate(MAX_FOG_BOXES);
        near.sort_by(|a, b| a.1.priority.total_cmp(&b.1.priority).then(a.0.cmp(&b.0)));
        let gpu_boxes: Vec<GpuFogBox> = near
            .iter()
            .map(|&(_, b)| {
                let m = b.to_box.transpose();
                GpuFogBox {
                    rows: [
                        m.x_axis.to_array(),
                        m.y_axis.to_array(),
                        m.z_axis.to_array(),
                    ],
                    medium: [
                        b.density.max(0.0),
                        b.height_falloff.max(0.0),
                        b.absorption.clamp(0.0, 1.0),
                        b.priority,
                    ],
                    shape: [b.height, b.blend, 0.0, 0.0],
                    tint: b.tint.extend(0.0).to_array(),
                }
            })
            .collect();
        self.boxes[slot.index].write(0, &gpu_boxes);
        // The fog's own lamps whose reach meets the volume, the nearest first.
        let mut lit: Vec<(f32, crate::lamps::GpuLamp)> = fog_lamps
            .iter()
            .filter(|l| l.candela > 0.0 && l.fog > 0.0)
            .map(|l| {
                let g = crate::lamps::gpu_lamp(l, 1.0);
                (l.position.distance(params.camera) - g.colour_reach[3], g)
            })
            .filter(|(gap, _)| *gap < params.far)
            .collect();
        lit.sort_by(|a, b| a.0.total_cmp(&b.0));
        lit.truncate(MAX_FOG_LAMPS);
        let gpu_lamps: Vec<crate::lamps::GpuLamp> = lit.into_iter().map(|(_, g)| g).collect();
        if !gpu_lamps.is_empty() {
            self.lamps[slot.index].write(0, &gpu_lamps);
        }
        let fog_lamp_count = gpu_lamps.len() as u32;
        let lamps_address = self.lamps[slot.index].address();
        let box_count = gpu_boxes.len() as u32;
        let boxes_address = self.boxes[slot.index].address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let atlas = |name: &'static str| TransientDesc {
            name,
            width: FROXELS_X * SLICES,
            height: FROXELS_Y,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let light = graph.transient(atlas("fog light"));
        let volume = graph.transient(atlas("fog volume"));
        let buffer: &'f Buffer = &self.params[slot.index];
        let address = buffer.address();
        let inverse = params.view_proj.inverse();
        let pipeline = &self.light;
        graph
            .pass("fog/light")
            .image(light, ImageAccess::StorageWrite(compute))
            .with(|p| crate::MoversFrame::declare(params.movers, p, compute, true))
            .run(move |resources, commands| {
                buffer.write(
                    0,
                    &[GpuFog {
                        inv_view_proj: inverse.to_cols_array(),
                        camera: params.camera.extend(params.far).to_array(),
                        sun: params
                            .sun_dir
                            .normalize_or(Vec3::Y)
                            .extend(params.sun_illuminance)
                            .to_array(),
                        sun_color: params.sun_color.extend(params.anisotropy).to_array(),
                        global: params.global.map_or([0.0; 4], |g| {
                            [
                                g.density.max(0.0),
                                g.height,
                                g.falloff.max(0.0),
                                g.absorption.clamp(0.0, 1.0),
                            ]
                        }),
                        scales: [params.light_scales[0], params.light_scales[1], 0.0, 0.0],
                        frame: params.frame,
                        boxes: boxes_address,
                        fog_lamps: lamps_address,
                        box_count,
                        light: resources.storage(light, 0).0,
                        light_sampled: resources.sampled(light).0,
                        volume: resources.storage(volume, 0).0,
                        volume_sampled: resources.sampled(volume).0,
                        depth: resources.sampled(depth).0,
                        color: resources.storage(color, 0).0,
                        width: extent.width,
                        height: extent.height,
                        fog_lamp_count,
                    }],
                );
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(FROXELS_X.div_ceil(GROUP), FROXELS_Y.div_ceil(GROUP), SLICES);
                Ok(())
            });
        let pipeline = &self.integrate;
        graph
            .pass("fog/integrate")
            .image(light, ImageAccess::StorageReadWrite(compute))
            .image(volume, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(FROXELS_X.div_ceil(GROUP), FROXELS_Y.div_ceil(GROUP), 1);
                Ok(())
            });
        let pipeline = &self.apply;
        graph
            .pass("fog/apply")
            .image(volume, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(
                    extent.width.div_ceil(GROUP),
                    extent.height.div_ceil(GROUP),
                    1,
                );
                Ok(())
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_records_keep_the_shaders_layout() {
        assert_eq!(std::mem::size_of::<GpuFogBox>(), 96);
        assert_eq!(std::mem::size_of::<GpuFog>(), 208);
        assert_eq!(std::mem::offset_of!(GpuFog, fog_lamps), 160);
    }
}
