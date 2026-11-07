//! D-088's trial of NVIDIA's NRC 0.16 (Neural Radiance Cache) under the diffuse bounce, behind
//! `r.gi.cache nrc` and NVIDIA's GPUs: a small network, trained every frame on sparse paths and
//! asked for the light leaving the points where the bounce's rays meet a surface
//! ([`crate::MeshletRenderer::trace_diffuse`]), in SHaRC's place (`docs/research/sharc-and-nrc.md`).
//!
//! NRC is a binary library under the NVIDIA RTX SDKs licence, never in the repository:
//! [`forge_gpu::nrc`] loads Forge's shim over it from the git-ignored `nrc-sdk/bin`
//! (`tools/fetch-nrc.sh`), and `shaders/nrc.slang` is Forge's side of its shaders, which includes
//! its headers unmodified. Each frame:
//! 1. `gi/nrc begin`: NRC's start of the frame (its counters cleared) and the shaders' constants.
//! 2. `gi/nrc update`: NRC's training paths, at its training size (197 × 111 for 1080p), from the
//!    surfaces of the visibility buffer along cosine-weighted rays, up to eight hits.
//! 3. `gi/trace` built from `nrc.slang`: the bounce, whose hits make NRC's queries.
//! 4. `gi/nrc train`: NRC's inference at the queries and its training on the paths (its CUDA
//!    kernels, recorded into the frame's command buffer).
//! 5. `gi/nrc resolve`: each pixel's answer added to the bounce's light, before REBLUR.
//!
//! NRC is told the light's scale each frame (its `maxExpectedAverageRadianceValue`,
//! [`expected_radiance`]): the frame's exposure gives the light a frame averages, in the resolve's
//! units, whole stops apart. Told 1 (a sunlit white surface's light) at night, where a lamp-lit room
//! averages about 1e-4 of it, its answers went to nothing (the bedroom in clay, 0.687 from Forge's
//! path against SHaRC's 0.189).
//!
//! Knobs for the trial, from the environment: `FORGE_NRC_RADIANCE` (that scale, fixed, in the
//! resolve's units), `FORGE_NRC_HEADROOM` (the scale over the frame's average light, in stops,
//! [`HEADROOM_STOPS`] by default), `FORGE_NRC_IRRADIANCE=0` (the network learns the light leaving a point,
//! the albedo kept in), `FORGE_NRC_ITERATIONS` (training batches a frame, NRC's 4),
//! `FORGE_NRC_DIRECT` (1: the cache holds the hits' own light too, NRC's default; 0: the bounce's
//! hit adds its own sun, glow and lamp, exact) and `FORGE_NRC_PRIMARY` (the share of the paths'
//! first vertices that train the cache, NRC's 0.05), `FORGE_NRC_TERMINATION` (the training paths'
//! spread at which NRC ends them, over their first segment's, NRC's 0.1; its RTXGI sample 0.01),
//! `FORGE_NRC_BOUNDS` (the bounds' half size, metres, [`BOUNDS_HALF`] by default).
//!
//! Without the shim, NRC's headers in `nrc-sdk/` or the device's NRC extensions
//! ([`NrcCache::available`]), the demo keeps the probes.

use std::path::Path;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::nrc::{CONSTANTS_BYTES, Nrc, NrcBuffer, NrcFrameSettings, NrcSettings};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GpuError, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory,
    MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::Vec3;

use crate::meshlet::{DrawTargets, MoversFrame};
use crate::probes::ProbeLight;
use crate::sky::SkyLight;

/// Forge's side of NRC, which includes its headers: compiled only where `nrc-sdk/` is.
pub const SHADER: &str = "nrc.slang";
/// A training path's most vertices (NRC's default and the RTXGI sample's, 8).
const MAX_PATH_VERTICES: u32 = 8;
/// The smallest detail the cache resolves, metres (NRC's default).
const FEATURE_SIZE: f32 = 0.01;
/// The cache's bounds round the camera, metres each way: every point a ray meets must be inside
/// them (NRC's rule). They move, and the cache starts over, when the camera goes further than
/// half of this from their middle, which sits on a grid of [`BOUNDS_STEP`].
const BOUNDS_HALF: f32 = 1024.0;
const BOUNDS_STEP: f32 = 256.0;
/// The passes' workgroups (`nrc.slang`).
const GROUP: u32 = 8;
/// How far over the frame's average light NRC's scale stands, in stops ([`expected_radiance`]).
pub const HEADROOM_STOPS: f32 = 4.0;

/// NRC's `maxExpectedAverageRadianceValue` for a frame at `exposure` (the resolve's pre-exposure,
/// `crate::exposure`), where the sun's `sun_illuminance` lux sets the resolve's units (a white
/// surface it lights sends back 1; a luminance of L cd/m² is L·π / `sun_illuminance`): the light a
/// frame metered at that exposure averages (12.5 / 100 · 2^EV100 cd/m², `exposure` being
/// 1 / (1.2 · 2^EV100)), `headroom` stops over it, rounded to a whole stop so that a slow change of
/// exposure leaves it be (NRC asks for a value that changes little from frame to frame).
pub fn expected_radiance(exposure: f32, sun_illuminance: f32, headroom: f32) -> f32 {
    let average =
        0.125 / (1.2 * exposure.max(1e-12)) * std::f32::consts::PI / sun_illuminance.max(1e-6);
    (average.log2() + headroom).round().exp2()
}

/// `NrcField` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuField {
    constants: [u32; CONSTANTS_BYTES / 4],
    query_path_info: u64,
    training_path_info: u64,
    training_path_vertices: u64,
    query_radiance_params: u64,
    counters: u64,
    query_radiance: u64,
    frame: u32,
    pad: u32,
    nrc_counters: u64,
}

const _: () = assert!(std::mem::size_of::<GpuField>() == 160);

/// `NrcUpdatePush` in `nrc.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpdatePush {
    frame: u64,
    sky: u64,
    probes: u64,
    nrc: u64,
    vis_image: u32,
    width: u32,
    height: u32,
    pad: u32,
}

/// `NrcResolvePush` in `nrc.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolvePush {
    nrc: u64,
    bounce_image: u32,
    width: u32,
    height: u32,
    pad: u32,
}

/// What the bounce's rays write of this frame's cache ([`NrcCache::update`]), and the passes
/// after them ([`NrcLight::train_and_resolve`]).
#[derive(Clone, Copy)]
pub struct NrcLight<'f> {
    /// The frame's `NrcField` (`meshlet.slang`).
    pub(crate) address: u64,
    pub(crate) query_path_info: BufferHandle,
    pub(crate) query_radiance_params: BufferHandle,
    /// Forge's counters, which the shaders count in.
    pub(crate) counter: BufferHandle,
    /// `gi/trace` built from `nrc.slang`, which makes the queries.
    pub(crate) trace: &'f Pipeline,
    cache: &'f NrcCache,
    /// Every buffer of NRC's this frame, `None` for one it made none of.
    buffers: [Option<BufferHandle>; NrcBuffer::ALL.len()],
}

impl<'f> NrcLight<'f> {
    /// Declares NRC's inference and training after `gi/trace` (`gi/nrc train`), and the answers
    /// added to its light in `bounce` (`extent`, `gi/nrc resolve`), before the denoiser reads it.
    pub fn train_and_resolve(
        &self,
        graph: &mut FrameGraph<'f>,
        bounce: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let cache = self.cache;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let address = self.address;
        // The counts into NRC's own counters (`nrc_count_main`).
        let count = &cache.count;
        let mut builder = graph
            .pass("gi/nrc count")
            .buffer(self.counter, BufferAccess::ShaderRead(compute));
        if let Some(theirs) = self.buffers[NrcBuffer::Counter as usize] {
            builder = builder.buffer(theirs, BufferAccess::ShaderWrite(compute));
        }
        builder.run(move |_, commands| {
            commands.bind_pipeline(count);
            commands.push_constants(
                count,
                &ResolvePush {
                    nrc: address,
                    ..Zeroable::zeroed()
                },
            );
            commands.dispatch(1, 1, 1);
            Ok(())
        });
        // NRC's own commands (its kernels, its copies): declared as every stage's, behind a full
        // barrier each side, as the graph cannot see what they touch.
        let all = vk::PipelineStageFlags2::ALL_COMMANDS;
        let mut builder = graph.pass("gi/nrc train").full_barrier();
        for handle in self.buffers.iter().flatten() {
            builder = builder.buffer(*handle, BufferAccess::ShaderReadWrite(all));
        }
        builder.run(move |_, commands| cache.nrc.query_and_train(commands));
        let resolve = &cache.resolve;
        let mut builder = graph
            .pass("gi/nrc resolve")
            .full_barrier()
            .buffer(self.query_path_info, BufferAccess::ShaderRead(compute))
            .image(bounce, ImageAccess::StorageReadWrite(compute));
        if let Some(answers) = self.buffers[NrcBuffer::QueryRadiance as usize] {
            builder = builder.buffer(answers, BufferAccess::ShaderRead(compute));
        }
        builder.run(move |resources, commands| {
            commands.bind_pipeline(resolve);
            commands.push_constants(
                resolve,
                &ResolvePush {
                    nrc: address,
                    bounce_image: resources.storage(bounce, 0).0,
                    width: extent.width,
                    height: extent.height,
                    pad: 0,
                },
            );
            commands.dispatch(
                extent.width.div_ceil(GROUP),
                extent.height.div_ceil(GROUP),
                1,
            );
            Ok(())
        });
    }
}

/// NVIDIA's NRC under D-088's bounce: the library's context, and Forge's passes round it.
pub struct NrcCache {
    nrc: Nrc,
    update: Pipeline,
    resolve: Pipeline,
    count: Pipeline,
    trace: Pipeline,
    fields: Vec<Buffer>,
    /// The queries and training records counted this frame, in VRAM: NRC keeps its own where the
    /// host reads them back, slow to count in from every wave (`nrc.slang`).
    counters: GraphBuffer,
    /// The settings NRC was last configured with (`request_reset` 0).
    configured: Option<NrcSettings>,
    /// The cache starts over at the next update: at the start, and after [`Self::reset`].
    reset: bool,
    /// The middle of the cache's bounds, metres in the scene frame.
    centre: Option<Vec3>,
    frame: u32,
    frame_settings: NrcFrameSettings,
    /// `FORGE_NRC_RADIANCE`: NRC's scale fixed, rather than the exposure's.
    fixed_radiance: Option<f32>,
    headroom: f32,
    /// `FORGE_NRC_BOUNDS`: the bounds' half size, metres ([`BOUNDS_HALF`] by default).
    bounds_half: f32,
    learn_irradiance: bool,
    include_direct: bool,
}

/// An environment variable's value, parsed, or `default`.
fn knob<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

impl NrcCache {
    /// Whether NRC can run on `device`: its headers are beside the shaders (`nrc-sdk/`), and the
    /// device has ray queries and NRC's extensions (an NVIDIA GPU, made with
    /// [`forge_gpu::DeviceOptions::nrc`]).
    pub fn available(device: &Device, shaders: &ShaderCompiler) -> bool {
        let features = device.features();
        shaders.sdk_present(SHADER) && features.ray_query && features.nrc
    }

    /// Loads NRC from `dir` (`nrc-sdk/bin`) and compiles Forge's three passes; the cache is
    /// configured, empty, at the first update.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, dir: &Path) -> Result<Self> {
        let nrc = Nrc::load(device, dir)?;
        let compute = |entry: &str, push: usize, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile(SHADER, entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let fields = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuField>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("nrc field {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let frame_settings = NrcFrameSettings {
            training_iterations: knob("FORGE_NRC_ITERATIONS", 4),
            proportion_primary: knob("FORGE_NRC_PRIMARY", 0.05),
            training_termination: knob("FORGE_NRC_TERMINATION", 0.1),
            ..NrcFrameSettings::default()
        };
        Ok(Self {
            update: compute(
                "nrc_update_main",
                std::mem::size_of::<UpdatePush>(),
                "nrc update",
            )?,
            resolve: compute(
                "nrc_resolve_main",
                std::mem::size_of::<ResolvePush>(),
                "nrc resolve",
            )?,
            count: compute(
                "nrc_count_main",
                std::mem::size_of::<ResolvePush>(),
                "nrc count",
            )?,
            counters: GraphBuffer::new(device.create_buffer(BufferDesc {
                size: 16,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name: "nrc counters",
            })?),
            trace: compute(
                "diffuse_trace_main",
                crate::meshlet::DIFFUSE_TRACE_PUSH_BYTES,
                "diffuse bounce rays (nrc)",
            )?,
            nrc,
            fields,
            configured: None,
            reset: true,
            centre: None,
            frame: 0,
            frame_settings,
            fixed_radiance: std::env::var("FORGE_NRC_RADIANCE")
                .ok()
                .and_then(|v| v.trim().parse().ok()),
            headroom: knob("FORGE_NRC_HEADROOM", HEADROOM_STOPS),
            bounds_half: knob("FORGE_NRC_BOUNDS", BOUNDS_HALF),
            learn_irradiance: knob("FORGE_NRC_IRRADIANCE", 1u32) != 0,
            include_direct: knob("FORGE_NRC_DIRECT", 0u32) != 0,
        })
    }

    /// Starts the cache over at the next update. The demo calls it when the cache is turned on
    /// again, as SHaRC's.
    pub fn reset(&mut self) {
        self.reset = true;
    }

    /// The device memory NRC's buffers hold, bytes (NRC's own network besides).
    pub fn bytes(&self) -> u64 {
        NrcBuffer::ALL
            .iter()
            .filter_map(|&b| self.nrc.buffer(b))
            .map(|b| b.size())
            .sum::<u64>()
            + self.fields.iter().map(Buffer::size).sum::<u64>()
            + self.counters.size()
    }

    /// Ends NRC's last frame, configures it for frames of `extent` round the camera at `camera`
    /// (metres in the scene frame) where that changed, with the light's scale of a frame at
    /// `exposure` under a sun of `sun_illuminance` lux ([`expected_radiance`]), and declares this
    /// frame's start and its
    /// training paths from the surfaces of `targets`' visibility buffer, with the scene of the
    /// renderer's frame block at `frame`, the sky's light and the probes' (where a path meets a
    /// leaf). Returns what the bounce's rays write and the passes after them.
    #[allow(clippy::too_many_arguments)]
    pub fn update<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        targets: DrawTargets,
        extent: vk::Extent2D,
        sky: SkyLight,
        probes: Option<ProbeLight>,
        camera: Vec3,
        exposure: f32,
        sun_illuminance: f32,
    ) -> Result<NrcLight<'f>> {
        // The frame before was submitted by now.
        self.nrc.end_frame()?;
        let centre = match self.centre {
            Some(c) if (camera - c).abs().max_element() <= 0.5 * self.bounds_half => c,
            _ => {
                self.reset = true;
                let step = BOUNDS_STEP.min(self.bounds_half);
                (camera / step).round() * step
            }
        };
        self.centre = Some(centre);
        let settings = NrcSettings {
            learn_irradiance: u32::from(self.learn_irradiance),
            include_direct: u32::from(self.include_direct),
            request_reset: 0,
            samples_per_pixel: 1,
            bounds_min: (centre - self.bounds_half).to_array(),
            feature_size: FEATURE_SIZE,
            bounds_max: (centre + self.bounds_half).to_array(),
            max_path_vertices: MAX_PATH_VERTICES,
            frame: [extent.width, extent.height],
            // NRC's size for this many batches a frame, all traced every frame.
            training: {
                let t = self
                    .nrc
                    .training_size(extent, self.frame_settings.training_iterations);
                [t.width, t.height]
            },
        };
        let reset = std::mem::take(&mut self.reset);
        if reset || self.configured != Some(settings) {
            self.nrc.configure(&NrcSettings {
                request_reset: u32::from(reset),
                ..settings
            })?;
            self.configured = Some(settings);
        }
        let frame_settings = NrcFrameSettings {
            max_average_radiance: self
                .fixed_radiance
                .unwrap_or_else(|| expected_radiance(exposure, sun_illuminance, self.headroom)),
            ..self.frame_settings
        };
        let field_frame = self.frame;
        self.frame = self.frame.wrapping_add(1);

        let this: &'f Self = self;
        let buffer = |b: NrcBuffer| this.nrc.buffer(b);
        let needed = |b: NrcBuffer| {
            buffer(b).ok_or_else(|| GpuError::Nrc(format!("NRC made no {b:?} buffer")))
        };
        let mut buffers = [None; NrcBuffer::ALL.len()];
        for b in NrcBuffer::ALL {
            buffers[b as usize] = buffer(b).map(|g| graph.import_buffer(g));
        }
        let handle = |b: NrcBuffer| {
            buffers[b as usize].ok_or_else(|| GpuError::Nrc(format!("NRC made no {b:?} buffer")))
        };
        let field = GpuField {
            constants: [0; CONSTANTS_BYTES / 4],
            query_path_info: needed(NrcBuffer::QueryPathInfo)?.address(),
            training_path_info: needed(NrcBuffer::TrainingPathInfo)?.address(),
            training_path_vertices: needed(NrcBuffer::TrainingPathVertices)?.address(),
            query_radiance_params: needed(NrcBuffer::QueryRadianceParams)?.address(),
            counters: this.counters.address(),
            query_radiance: needed(NrcBuffer::QueryRadiance)?.address(),
            frame: field_frame,
            pad: 0,
            nrc_counters: needed(NrcBuffer::Counter)?.address(),
        };
        let nrc_counter = handle(NrcBuffer::Counter)?;
        let counter = graph.import_buffer(&this.counters);
        let query_path_info = handle(NrcBuffer::QueryPathInfo)?;
        let query_radiance_params = handle(NrcBuffer::QueryRadianceParams)?;
        let training_path_info = handle(NrcBuffer::TrainingPathInfo)?;
        let training_path_vertices = handle(NrcBuffer::TrainingPathVertices)?;
        let field_buffer = &this.fields[slot.index];
        let address = field_buffer.address();
        let all = vk::PipelineStageFlags2::ALL_COMMANDS;
        // NRC's start: its counters cleared (and Forge's), and the constants of this frame,
        // written in the frame slot's block before the passes after it run.
        graph
            .pass("gi/nrc begin")
            .full_barrier()
            .buffer(nrc_counter, BufferAccess::ShaderReadWrite(all))
            .buffer(counter, BufferAccess::TransferDst)
            .run(move |_, commands| {
                let constants = this.nrc.begin_frame(commands, &frame_settings)?;
                let mut field = field;
                field.constants = bytemuck::cast(constants);
                field_buffer.write(0, &[field]);
                commands.fill_buffer(&this.counters, 0, 16, 0);
                Ok(())
            });
        // The training paths read this frame's visibility buffer: on the main queue, after the
        // draw.
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let update = &this.update;
        let update_push = UpdatePush {
            frame,
            sky: sky.address,
            probes: probes.map_or(0, |p| p.address),
            nrc: address,
            vis_image: 0,
            width: extent.width,
            height: extent.height,
            pad: 0,
        };
        let mut builder = graph
            .pass("gi/nrc update")
            .full_barrier()
            .image(targets.visibility, ImageAccess::Sampled(compute))
            .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
            .buffer(targets.pages, BufferAccess::ShaderRead(compute))
            .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
            .with(|b| MoversFrame::declare(targets.movers, b, compute, true))
            .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.table, ImageAccess::Sampled(compute))
            .buffer(counter, BufferAccess::ShaderReadWrite(compute))
            .buffer(training_path_info, BufferAccess::ShaderWrite(compute))
            .buffer(
                training_path_vertices,
                BufferAccess::ShaderReadWrite(compute),
            )
            .buffer(query_radiance_params, BufferAccess::ShaderWrite(compute));
        if let Some(p) = probes {
            builder = builder
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .buffer(p.region, BufferAccess::ShaderRead(compute))
                .with(|b| p.declare_tables(b, compute))
                .image(p.irradiance, ImageAccess::Sampled(compute))
                .image(p.distance, ImageAccess::Sampled(compute));
        }
        let groups = (
            settings.training[0].div_ceil(GROUP),
            settings.training[1].div_ceil(GROUP),
        );
        builder.run(move |resources, commands| {
            commands.bind_pipeline(update);
            commands.push_constants(
                update,
                &UpdatePush {
                    vis_image: resources.sampled(targets.visibility).0,
                    ..update_push
                },
            );
            commands.dispatch(groups.0, groups.1, 1);
            Ok(())
        });
        Ok(NrcLight {
            address,
            query_path_info,
            query_radiance_params,
            counter,
            trace: &this.trace,
            cache: this,
            buffers,
        })
    }
}
