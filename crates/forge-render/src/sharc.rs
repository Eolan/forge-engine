//! D-088's surface cache through NVIDIA's SHaRC 1.8.3 (Spatially Hashed Radiance Cache): the
//! light that reaches the scene's surfaces round the camera, read where the diffuse bounce's rays
//! meet a surface ([`crate::MeshletRenderer::trace_diffuse`]) in place of the probes', whose
//! probes float in the air and see more sky than a surface does.
//!
//! SHaRC is shader-only, used unmodified from the git-ignored `sharc-sdk/` under the NVIDIA RTX
//! SDKs licence; `shaders/sharc.slang` is Forge's side and says how it lights the voxels
//! (`docs/research/sharc-and-nrc.md`, D-088). Each frame:
//! 1. `gi/sharc update`: one pixel in each 5 × 5 block starts a path at its surface, whose hits
//!    add their light to the voxels of a hash grid in the scene's frame, finer near the camera.
//! 2. `gi/sharc resolve`: each slot's sums blended into the light it keeps (over up to 32
//!    frames), the voxels no path met for 64 frames emptied.
//! 3. `gi/trace` built from `sharc.slang` reads the voxels at its hits.
//!
//! Without the SDK's headers, 16-bit storage or 64-bit buffer atomics ([`Sharc::available`]), the
//! demo keeps the probes.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, ImageAccess, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, vk,
};
use glam::Vec3;

use crate::meshlet::{DrawTargets, MoversFrame};
use crate::probes::ProbeLight;
use crate::sky::SkyLight;

/// Forge's side of SHaRC, which includes its headers: compiled only where `sharc-sdk/` is.
pub const SHADER: &str = "sharc.slang";
/// The slots: 2²⁰, 40 MiB with their keys and light (D-088: raised if a scene fills more than a
/// third of them).
pub const CAPACITY: u32 = 1 << 20;
/// SHaRC's `sceneScale`: a voxel of the grid's finest level, within 4 m of the camera, is
/// 2 / SCENE_SCALE metres across (4 cm), twice that from 4 m, four times from 8 m (16 cm), and so
/// on with the distance; thinner than the village's walls near the camera.
const SCENE_SCALE: f32 = 50.0;
/// SHaRC's `radianceScale`, the fixed point of the update's sums: a sunlit white surface sends
/// back about 1 in the resolve's units, and a room's bounce a thousandth of that, kept to about
/// 1 % by 1e5 a unit. A slot's sums of a frame overflow past 42,950 units, a thousand of the
/// brightest paths at once.
const RADIANCE_SCALE: f32 = 1e5;
/// The frames a voxel's light blends over (the RTXGI sample starts at 20): with four vertices
/// carried back (`sharc.slang`), 8 to 32 measured the same on the landing, and 32 is the quieter.
const ACCUMULATION_FRAMES: u32 = 32;
/// The frames a voxel no path met is kept before its slot is emptied (the sample starts at 60);
/// 120 changed nothing on the landing.
const STALE_FRAMES: u32 = 64;
/// The side of the pixel blocks the update takes one pixel of (`SHARC_BLOCK`).
const BLOCK: u32 = 5;
/// The update's and the resolve's workgroups (`sharc.slang`).
const UPDATE_GROUP: u32 = 8;
const RESOLVE_GROUP: u32 = 256;
/// A slot's key (`HashGridKey`), sums (`SharcAccumulationData`) and light (`SharcPackedData`).
const KEY_BYTES: u64 = 8;
const SUMS_BYTES: u64 = 16;
const LIGHT_BYTES: u64 = 16;

/// `SharcField` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuField {
    camera: [f32; 3],
    scene_scale: f32,
    camera_prev: [f32; 3],
    radiance_scale: f32,
    keys: u64,
    accumulation: u64,
    resolved: u64,
    live: u64,
    capacity: u32,
    accumulation_frames: u32,
    stale_frames: u32,
    frame: u32,
}

const _: () = assert!(std::mem::size_of::<GpuField>() == 80);

/// `SharcUpdatePush` in `sharc.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpdatePush {
    frame: u64,
    sky: u64,
    probes: u64,
    sharc: u64,
    vis_image: u32,
    width: u32,
    height: u32,
    pad: u32,
}

/// `SharcResolvePush` in `sharc.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolvePush {
    sharc: u64,
}

/// What the bounce's rays read of this frame's cache ([`Sharc::update`]).
#[derive(Clone, Copy)]
pub struct SharcLight<'f> {
    /// The frame's `SharcField` (`meshlet.slang`).
    pub(crate) address: u64,
    pub(crate) keys: BufferHandle,
    pub(crate) resolved: BufferHandle,
    /// `gi/trace` built from `sharc.slang`, which reads the voxels.
    pub(crate) trace: &'f Pipeline,
}

/// NVIDIA's SHaRC under D-088's bounce: its slots, and the passes that fill and blend them.
pub struct Sharc {
    update: Pipeline,
    resolve: Pipeline,
    trace: Pipeline,
    keys: GraphBuffer,
    accumulation: GraphBuffer,
    resolved: GraphBuffer,
    /// The slots in use after the resolve: counted on the device, copied for the host per frame
    /// slot (`counted`: by the frame that last used the slot).
    live: GraphBuffer,
    live_read: Vec<GraphBuffer>,
    counted: [bool; FRAMES_IN_FLIGHT],
    most_live: u32,
    fields: Vec<Buffer>,
    /// Every slot is emptied before the next update: at the start, and after [`Self::reset`].
    reset: bool,
    frame: u32,
    camera_prev: Option<Vec3>,
}

impl Sharc {
    /// Whether SHaRC can run on `device`: its headers are beside the shaders (`sharc-sdk/`), and
    /// the device has ray queries, 16-bit storage buffers (its blended light is `float16_t4`) and
    /// 64-bit buffer atomics (its keys).
    pub fn available(device: &Device, shaders: &ShaderCompiler) -> bool {
        let features = device.features();
        shaders.sdk_present(SHADER)
            && features.ray_query
            && features.storage16
            && features.int64_atomics
    }

    /// Compiles the three passes and creates the slots, all empty once the first update has
    /// cleared them.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
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
        let slots = |bytes: u64, name: &str| -> Result<GraphBuffer> {
            Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                size: bytes,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST
                    | vk::BufferUsageFlags::TRANSFER_SRC,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })?))
        };
        let capacity = u64::from(CAPACITY);
        let fields = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuField>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("sharc field {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let live_read = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: 4,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("sharc slots in use {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            update: compute(
                "sharc_update_main",
                std::mem::size_of::<UpdatePush>(),
                "sharc update",
            )?,
            resolve: compute(
                "sharc_resolve_main",
                std::mem::size_of::<ResolvePush>(),
                "sharc resolve",
            )?,
            trace: compute(
                "diffuse_trace_main",
                crate::meshlet::DIFFUSE_TRACE_PUSH_BYTES,
                "diffuse bounce rays (sharc)",
            )?,
            keys: slots(capacity * KEY_BYTES, "sharc keys")?,
            accumulation: slots(capacity * SUMS_BYTES, "sharc sums")?,
            resolved: slots(capacity * LIGHT_BYTES, "sharc light")?,
            live: slots(4, "sharc slots in use")?,
            live_read,
            counted: [false; FRAMES_IN_FLIGHT],
            most_live: 0,
            fields,
            reset: true,
            frame: 0,
            camera_prev: None,
        })
    }

    /// The most slots in use at once so far, of [`CAPACITY`].
    pub fn most_live(&self) -> u32 {
        self.most_live
    }

    /// Empties every slot before the next update. The demo calls it when the cache is turned on
    /// again: its voxels are in the scene's frame, so they hold across a camera cut, and nothing
    /// shifts that frame's origin or swaps the scene under them at run time yet (D-088).
    pub fn reset(&mut self) {
        self.reset = true;
    }

    /// The device memory it holds, bytes.
    pub fn bytes(&self) -> u64 {
        self.keys.size()
            + self.accumulation.size()
            + self.resolved.size()
            + self.live.size()
            + self.live_read.iter().map(|b| b.size()).sum::<u64>()
            + self.fields.iter().map(Buffer::size).sum::<u64>()
    }

    /// Declares this frame's update and resolve round the camera at `camera` (metres in the scene
    /// frame, as [`crate::Probes::update`]'s), the paths starting at the surfaces of `targets`'
    /// visibility buffer (`extent`), with the scene of the renderer's frame block at `frame`, the
    /// sky's light and the probes' (where a path ends early), and returns what the bounce's rays
    /// read.
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
    ) -> SharcLight<'f> {
        // The count the frame that last used `slot` copied (its commands have completed).
        if std::mem::take(&mut self.counted[slot.index]) {
            let mut live = [0_u32];
            self.live_read[slot.index].read(0, &mut live);
            self.most_live = self.most_live.max(live[0]);
        }
        self.counted[slot.index] = true;
        let field = GpuField {
            camera: camera.to_array(),
            scene_scale: SCENE_SCALE,
            camera_prev: self.camera_prev.unwrap_or(camera).to_array(),
            radiance_scale: RADIANCE_SCALE,
            keys: self.keys.address(),
            accumulation: self.accumulation.address(),
            resolved: self.resolved.address(),
            live: self.live.address(),
            capacity: CAPACITY,
            accumulation_frames: ACCUMULATION_FRAMES,
            stale_frames: STALE_FRAMES,
            frame: self.frame,
        };
        self.fields[slot.index].write(0, &[field]);
        self.frame = self.frame.wrapping_add(1);
        self.camera_prev = Some(camera);
        let reset = std::mem::take(&mut self.reset);

        let this: &'f Self = self;
        let address = this.fields[slot.index].address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let keys = graph.import_buffer(&this.keys);
        let accumulation = graph.import_buffer(&this.accumulation);
        let resolved = graph.import_buffer(&this.resolved);
        let live = graph.import_buffer(&this.live);
        let live_read = graph.import_buffer(&this.live_read[slot.index]);
        if reset {
            // SHaRC's empty slot is all zeros: no key, no sums, no light.
            graph
                .pass("gi/sharc clear")
                .buffer(keys, BufferAccess::TransferDst)
                .buffer(accumulation, BufferAccess::TransferDst)
                .buffer(resolved, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    for buffer in [&this.keys, &this.accumulation, &this.resolved] {
                        commands.fill_buffer(buffer, 0, buffer.size(), 0);
                    }
                    Ok(())
                });
        }
        // The update reads this frame's visibility buffer: on the main queue, after the draw.
        let update = &this.update;
        let update_push = UpdatePush {
            frame,
            sky: sky.address,
            probes: probes.map_or(0, |p| p.address),
            sharc: address,
            vis_image: 0,
            width: extent.width,
            height: extent.height,
            pad: 0,
        };
        let mut builder = graph
            .pass("gi/sharc update")
            .image(targets.visibility, ImageAccess::Sampled(compute))
            .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
            .buffer(targets.pages, BufferAccess::ShaderRead(compute))
            .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
            .with(|b| MoversFrame::declare(targets.movers, b, compute, true))
            .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.table, ImageAccess::Sampled(compute))
            .buffer(keys, BufferAccess::ShaderReadWrite(compute))
            .buffer(accumulation, BufferAccess::ShaderReadWrite(compute))
            .buffer(resolved, BufferAccess::ShaderRead(compute));
        if let Some(p) = probes {
            builder = builder
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .buffer(p.region, BufferAccess::ShaderRead(compute))
                .with(|b| p.declare_far(b, compute))
                .image(p.irradiance, ImageAccess::Sampled(compute))
                .image(p.distance, ImageAccess::Sampled(compute));
        }
        let blocks = (
            extent.width.div_ceil(BLOCK).div_ceil(UPDATE_GROUP),
            extent.height.div_ceil(BLOCK).div_ceil(UPDATE_GROUP),
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
            commands.dispatch(blocks.0, blocks.1, 1);
            Ok(())
        });
        let live_buffer = &this.live;
        graph
            .pass("gi/sharc resolve")
            .buffer(live, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(live_buffer, 0, 4, 0);
                Ok(())
            });
        let resolve = &this.resolve;
        graph
            .pass("gi/sharc resolve")
            .buffer(keys, BufferAccess::ShaderReadWrite(compute))
            .buffer(accumulation, BufferAccess::ShaderReadWrite(compute))
            .buffer(resolved, BufferAccess::ShaderReadWrite(compute))
            .buffer(live, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(resolve);
                commands.push_constants(resolve, &ResolvePush { sharc: address });
                commands.dispatch(CAPACITY.div_ceil(RESOLVE_GROUP), 1, 1);
                Ok(())
            });
        let live_read_buffer = &this.live_read[slot.index];
        graph
            .pass("gi/sharc resolve")
            .buffer(live, BufferAccess::TransferSrc)
            .buffer(live_read, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(live_buffer, live_read_buffer, 4);
                Ok(())
            });
        graph
            .pass("gi/sharc resolve")
            .buffer(live_read, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
        SharcLight {
            address,
            keys,
            resolved,
            trace: &this.trace,
        }
    }
}
