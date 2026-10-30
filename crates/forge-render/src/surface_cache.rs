//! D-088's surface cache: the light round the scene's surfaces (sky and bounces), measured on
//! them and read where the diffuse bounce's rays meet a surface
//! ([`crate::MeshletRenderer::trace_diffuse`]), in place of the probes' light, whose probes float
//! in the air and see more sky than a surface does.
//!
//! Bevy Solari's world cache on kajiya's dense index (`shaders/surface_cache.slang` has the
//! credits and Forge's changes, D-088 the design and the patents). A pool of entries, each made
//! where a lookup finds none, at a surface point it keeps: one cosine ray from it a frame, whose
//! hit reads the cache again (the bounces after), blended over up to 32 frames. The index is
//! twelve levels of 32³ cells round the camera, each twice the last, a slot per cell and facing;
//! the cells wrap as the camera moves, and an entry whose cell left its level's window is freed
//! before the frame's lookups. An entry unread for ten frames is freed too.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, ImageAccess, MemoryCategory, MemoryLocation, Pipeline,
    QueueKind, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::{IVec3, Vec3};

use crate::meshlet::MoversFrame;
use crate::probes::ProbeLight;
use crate::sky::SkyLight;

/// The index's levels (`CACHE_LEVELS` in `surface_cache.slang`).
const LEVELS: usize = 12;
/// A level's cells along each axis (`CACHE_CELLS`).
const CELLS: i32 = 32;
/// A cell's slots, one per facing (`CACHE_FACINGS`).
const FACINGS: u64 = 6;
/// The entries.
pub const POOL: u32 = 65_536;
/// The finest cell, metres (Solari's `world_cache_position_base_cell_size`).
pub const FINEST_CELL: f32 = 0.15;
/// The most samples an entry's light averages (Solari's `world_cache_max_temporal_samples`).
const MOST_SAMPLES: f32 = 32.0;
/// An entry's bytes (`CacheEntry`).
const ENTRY_BYTES: u64 = 80;
const GROUP: u32 = 64;

/// `SurfaceCacheField` in `surface_cache.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuField {
    origins: [[i32; 4]; LEVELS],
    centre: [f32; 3],
    cell: f32,
    entries: u64,
    index: u64,
    free: u64,
    used: u64,
    pool: u32,
    frame: u32,
    most_samples: f32,
    reset: u32,
}

/// `CacheEntry` in `surface_cache.slang`, as a free entry starts: no slot, no light, no sample.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuEntry {
    position: [f32; 3],
    slot: u32,
    normal: [f32; 3],
    life: u32,
    cell: [i32; 4],
    irradiance: [f32; 4],
    fresh: [f32; 3],
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuEntry>() as u64 == ENTRY_BYTES);

/// The age and blend passes' push block (`surface_cache_update.slang`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpdatePush {
    field: u64,
}

/// `CacheTracePush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TracePush {
    frame: u64,
    cache: u64,
    sky: u64,
    probes: u64,
    movers: u32,
    pad: [u32; 3],
}

/// What the bounce's rays read of this frame's cache ([`SurfaceCache::update`]).
#[derive(Clone, Copy, Debug)]
pub struct CacheLight {
    /// The frame's `SurfaceCacheField` (`surface_cache.slang`).
    pub(crate) address: u64,
    pub(crate) entries: BufferHandle,
    pub(crate) index: BufferHandle,
    pub(crate) free: BufferHandle,
}

impl CacheLight {
    /// Declares the cache's buffers for a pass that looks it up (and makes the entries it misses)
    /// at `stages`.
    pub(crate) fn declare<'b, 'f>(
        &self,
        builder: forge_gpu::PassBuilder<'b, 'f>,
        stages: vk::PipelineStageFlags2,
    ) -> forge_gpu::PassBuilder<'b, 'f> {
        builder
            .buffer(self.entries, BufferAccess::ShaderReadWrite(stages))
            .buffer(self.index, BufferAccess::ShaderReadWrite(stages))
            .buffer(self.free, BufferAccess::ShaderReadWrite(stages))
    }
}

/// The surface cache: its entries, index and free stack, and the passes that keep them.
pub struct SurfaceCache {
    age: Pipeline,
    trace: Pipeline,
    blend: Pipeline,
    entries: GraphBuffer,
    index: GraphBuffer,
    free: GraphBuffer,
    fields: Vec<Buffer>,
    /// The entries in use after each frame slot's blend, read back (`counted`: written by the
    /// frame that last used the slot).
    used: Vec<GraphBuffer>,
    counted: [bool; FRAMES_IN_FLIGHT],
    most_used: u32,
    /// Every entry is freed at the next update.
    reset: bool,
    frame: u32,
}

impl SurfaceCache {
    /// Compiles the passes (`gi/cache rays` needs ray queries) and creates the pool, every entry
    /// free, and the index, every slot empty.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |file: &str, entry: &str, push: usize, name: &str| -> Result<Pipeline> {
            let module = device
                .create_shader_module(&shaders.compile(file, entry, ShaderStage::Compute)?, name)?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let buffer = |size: u64, name: &str| {
            device.create_buffer(BufferDesc {
                size,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })
        };
        let slots = LEVELS as u64 * (CELLS as u64).pow(3) * FACINGS;
        let entries = buffer(u64::from(POOL) * ENTRY_BYTES, "surface cache entries")?;
        let free_entry = GpuEntry {
            slot: u32::MAX,
            fresh: [-1.0; 3],
            ..GpuEntry::zeroed()
        };
        device.write_buffer_staged(
            &entries,
            0,
            bytemuck::cast_slice(&vec![free_entry; POOL as usize]),
        )?;
        // Every byte 0xFF: each index slot CACHE_EMPTY.
        let index = buffer(slots * 4, "surface cache index")?;
        device.write_buffer_staged(&index, 0, &vec![0xFF_u8; (slots * 4) as usize])?;
        let free = buffer((u64::from(POOL) + 1) * 4, "surface cache free list")?;
        let stack: Vec<u32> = std::iter::once(POOL).chain(0..POOL).collect();
        device.write_buffer_staged(&free, 0, bytemuck::cast_slice(&stack))?;
        let fields = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuField>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("surface cache field {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let used = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: 4,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("surface cache entries in use {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            age: compute(
                "surface_cache_update.slang",
                "surface_cache_age_main",
                std::mem::size_of::<UpdatePush>(),
                "surface cache age",
            )?,
            trace: compute(
                "meshlet.slang",
                "surface_cache_trace_main",
                std::mem::size_of::<TracePush>(),
                "surface cache rays",
            )?,
            blend: compute(
                "surface_cache_update.slang",
                "surface_cache_blend_main",
                std::mem::size_of::<UpdatePush>(),
                "surface cache blend",
            )?,
            entries: GraphBuffer::new(entries),
            index: GraphBuffer::new(index),
            free: GraphBuffer::new(free),
            fields,
            used,
            counted: [false; FRAMES_IN_FLIGHT],
            most_used: 0,
            reset: false,
            frame: 0,
        })
    }

    /// The most entries in use at once so far, of [`POOL`]; at the pool's size, the surfaces met
    /// after it filled took the probes' light.
    pub fn most_used(&self) -> u32 {
        self.most_used
    }

    /// Frees every entry at the next update (a cut, or a scene changed under the cache).
    pub fn reset(&mut self) {
        self.reset = true;
    }

    /// The device memory it holds, bytes.
    pub fn bytes(&self) -> u64 {
        self.entries.size()
            + self.index.size()
            + self.free.size()
            + self.fields.iter().map(Buffer::size).sum::<u64>()
    }

    /// Declares this frame's three passes round the camera at `camera` (metres in the scene
    /// frame, as [`crate::Probes::update`]'s), with the scene of the renderer's frame block at
    /// `frame`, the sky's light and the probes' (the light at the hits the cache does not hold
    /// yet), and returns what the bounce's rays read. `movers`: the frame's movers, which the
    /// rays meet.
    #[allow(clippy::too_many_arguments)]
    pub fn update<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        sky: SkyLight,
        probes: Option<ProbeLight>,
        camera: Vec3,
        movers: Option<MoversFrame>,
    ) -> CacheLight {
        // The count the frame that last used `slot` read (its commands have completed).
        if std::mem::take(&mut self.counted[slot.index]) {
            let mut used = [0_u32];
            self.used[slot.index].read(0, &mut used);
            if used[0] >= POOL && self.most_used < POOL {
                tracing::warn!(
                    pool = POOL,
                    "the surface cache is full: the surfaces it cannot take keep the probes' light"
                );
            }
            self.most_used = self.most_used.max(used[0]);
        }
        self.counted[slot.index] = true;
        let mut origins = [[0_i32; 4]; LEVELS];
        for (level, origin) in origins.iter_mut().enumerate() {
            let size = FINEST_CELL * (1u32 << level) as f32;
            let cell = (camera / size).floor();
            let first =
                IVec3::new(cell.x as i32, cell.y as i32, cell.z as i32) - IVec3::splat(CELLS / 2);
            *origin = first.extend(0).to_array();
        }
        let field = GpuField {
            origins,
            centre: camera.to_array(),
            cell: FINEST_CELL,
            entries: self.entries.address(),
            index: self.index.address(),
            free: self.free.address(),
            used: self.used[slot.index].address(),
            pool: POOL,
            frame: self.frame,
            most_samples: MOST_SAMPLES,
            reset: u32::from(std::mem::take(&mut self.reset)),
        };
        self.frame = self.frame.wrapping_add(1);
        self.fields[slot.index].write(0, &[field]);

        let this: &'f Self = self;
        let address = this.fields[slot.index].address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let light = CacheLight {
            address,
            entries: graph.import_buffer(&this.entries),
            index: graph.import_buffer(&this.index),
            free: graph.import_buffer(&this.free),
        };
        let groups = POOL.div_ceil(GROUP);
        let push = UpdatePush { field: address };
        let age = &this.age;
        light
            .declare(
                graph.pass("gi/cache age").queue(QueueKind::Compute),
                compute,
            )
            .run(move |_, commands| {
                commands.bind_pipeline(age);
                commands.push_constants(age, &push);
                commands.dispatch(groups, 1, 1);
                Ok(())
            });
        let trace = &this.trace;
        let trace_push = TracePush {
            frame,
            cache: address,
            sky: sky.address,
            probes: probes.map_or(0, |p| p.address),
            movers: u32::from(movers.is_some()),
            pad: [0; 3],
        };
        let mut builder = light
            .declare(
                graph.pass("gi/cache rays").queue(QueueKind::Compute),
                compute,
            )
            .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.table, ImageAccess::Sampled(compute))
            .with(|b| MoversFrame::declare(movers, b, compute, true));
        if let Some(p) = probes {
            builder = builder
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .buffer(p.region, BufferAccess::ShaderRead(compute))
                .with(|b| p.declare_far(b, compute))
                .image(p.irradiance, ImageAccess::Sampled(compute))
                .image(p.distance, ImageAccess::Sampled(compute));
        }
        builder.run(move |_, commands| {
            commands.bind_pipeline(trace);
            commands.push_constants(trace, &trace_push);
            commands.dispatch(groups, 1, 1);
            Ok(())
        });
        let blend = &this.blend;
        let used = graph.import_buffer(&this.used[slot.index]);
        graph
            .pass("gi/cache blend")
            .queue(QueueKind::Compute)
            .buffer(light.entries, BufferAccess::ShaderReadWrite(compute))
            .buffer(light.free, BufferAccess::ShaderRead(compute))
            .buffer(used, BufferAccess::ShaderWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(blend);
                commands.push_constants(blend, &push);
                commands.dispatch(groups, 1, 1);
                Ok(())
            });
        graph
            .pass("gi/cache blend")
            .buffer(used, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
        light
    }
}
