//! Wakes on still water and the sea (#107): wave particles on the async compute queue (Yuksel,
//! House & Keyser 2007; `docs/research/water.md`, D-038's afterwards).
//!
//! The things moving through the lakes and the sea ([`WaterWake`]) add particles around their
//! outline, crests where they push the water out and troughs where they leave it. The particles
//! move out at the speed of the waves of their size, split as their front spreads and fade.
//! Every frame, on the compute queue:
//! - `wakes/clear`: the field of heights around the camera, and this frame's count, to zero;
//! - `wakes/advance`: last frame's particles moved on, split or dropped, into the other buffer;
//! - `wakes/emit`: the new ones;
//! - (each particle adds its bump to the field as it is written);
//! - `wakes/slopes`: the field's slopes, which the water's shading adds ([`WakeFrame`]).
//!
//! The field's heights are whole numbers added atomically, so a frame's field is the same
//! whatever order the particles come in.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory,
    MemoryLocation, Pipeline, QueueKind, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::DVec3;

/// Particles a buffer holds; past that, new ones are dropped.
const CAPACITY: u32 = 1 << 17;
/// Bytes of a particle (`WakeParticle` in `wakes.slang`).
const PARTICLE_BYTES: u64 = 32;
/// The field around the camera: cells a side and metres a cell (128 m across).
const CELLS: u32 = 1024;
const CELL: f32 = 0.125;
/// Things a frame's wakes take at most (the nearest the caller chose), and the particles each
/// adds around its outline.
pub const MAX_WAKES: usize = 32;
const AROUND: u32 = 16;
/// Seconds between emissions: 30 a second.
const EMIT_STEP: f32 = 1.0 / 30.0;

/// Something moving through still water or the sea this frame (#107): it makes waves.
#[derive(Clone, Copy, Debug)]
pub struct WaterWake {
    /// World x and z (the sea's frame), metres.
    pub position: [f32; 2],
    /// The radius of its outline at the water's level, metres.
    pub waterline: f32,
    /// Its velocity through the water, world x and z, m/s.
    pub velocity: [f32; 2],
    /// Its speed upwards, m/s: rising or sinking, it makes rings.
    pub rise: f32,
}

/// Mirrors `WakeEmitter` in `wakes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuEmitter {
    a: [f32; 4],
    b: [f32; 4],
}

/// Mirrors `WakePush` in `wakes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WakePush {
    from: u64,
    to: u64,
    counts: u64,
    heights: u64,
    emitters: u64,
    time: f32,
    from_index: u32,
    origin: [f32; 2],
    cell: f32,
    cells: u32,
    capacity: u32,
    emitter_count: u32,
    around: u32,
    step: f32,
    slopes: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<WakePush>() == 88);

/// This frame's wakes as the water's shading reads them.
#[derive(Clone, Copy, Debug)]
pub struct WakeFrame {
    /// The field's slopes (rg16f): ∂h/∂x, ∂h/∂z.
    pub slopes: ImageHandle,
    /// The field's corner (world x, z), and 1 / its size in metres.
    pub window: [f32; 3],
}

/// The wakes' particles, their field and its slopes (#107).
pub struct WaterWakes {
    advance: Pipeline,
    emit: Pipeline,
    slopes_pass: Pipeline,
    particles: [GraphBuffer; 2],
    counts: GraphBuffer,
    heights: GraphBuffer,
    slopes: GraphImage,
    /// Per frame slot, the emitters the CPU writes.
    emitters: Vec<Buffer>,
    /// Frames updated so far: which buffer is last frame's.
    frames: Cell<u64>,
    /// The sea's time of the last emission.
    emitted: Cell<f32>,
}

impl WaterWakes {
    /// Compiles the passes and makes the buffers and the field.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("wakes.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<WakePush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let buffer = |size: u64, name: &str| -> Result<GraphBuffer> {
            Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                size,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })?))
        };
        let particles = [
            buffer(u64::from(CAPACITY) * PARTICLE_BYTES, "wake particles 0")?,
            buffer(u64::from(CAPACITY) * PARTICLE_BYTES, "wake particles 1")?,
        ];
        let counts = buffer(16, "wake counts")?;
        let heights = buffer(u64::from(CELLS * CELLS) * 4, "wake heights")?;
        let slopes = GraphImage::new(
            device,
            ImageDesc {
                width: CELLS,
                height: CELLS,
                format: vk::Format::R16G16_SFLOAT,
                usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "wake slopes",
            },
        )?;
        let emitters = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (MAX_WAKES * std::mem::size_of::<GpuEmitter>()) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("wake emitters {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            advance: compute("advance_main", "wakes advance")?,
            emit: compute("emit_main", "wakes emit")?,
            slopes_pass: compute("slopes_main", "wakes slopes")?,
            particles,
            counts,
            heights,
            slopes,
            emitters,
            frames: Cell::new(0),
            emitted: Cell::new(f32::NEG_INFINITY),
        })
    }

    /// Bytes of the particles, the field and its slopes.
    pub fn bytes(&self) -> u64 {
        2 * self.particles[0].size() + self.heights.size() + u64::from(CELLS * CELLS) * 4
    }

    /// Declares this frame's passes on the async compute queue: the particles at the sea's
    /// `time`, the things in `wakes` (at most [`MAX_WAKES`]) adding new ones, and the field
    /// around `camera` (the sea's frame). Returns the slopes the water's shading reads.
    pub fn update<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        wakes: &[WaterWake],
        camera: DVec3,
        time: f32,
    ) -> WakeFrame {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let frame = self.frames.get();
        self.frames.set(frame + 1);
        let from_index = (frame % 2) as u32;
        let (from, to) = (
            &self.particles[from_index as usize],
            &self.particles[1 - from_index as usize],
        );
        // An emission every EMIT_STEP of the sea's time, covering the time since the last; none
        // while the sea stands still, and none to catch up after a jump.
        let since = time - self.emitted.get();
        let step = if !(0.0..=0.25).contains(&since) {
            self.emitted.set(time);
            0.0
        } else if since >= EMIT_STEP {
            self.emitted.set(time);
            since
        } else {
            0.0
        };
        let emitters: Vec<GpuEmitter> = wakes
            .iter()
            .take(MAX_WAKES)
            .map(|w| GpuEmitter {
                a: [w.position[0], w.position[1], w.waterline, w.rise],
                b: [w.velocity[0], w.velocity[1], 0.0, 0.0],
            })
            .collect();
        let emitter_buffer = &self.emitters[slot.index];
        if step > 0.0 && !emitters.is_empty() {
            emitter_buffer.write(0, &emitters);
        }
        let emitter_count = if step > 0.0 { emitters.len() as u32 } else { 0 };
        // The field: centred on the camera, its corner on the cells' lattice.
        let half = (CELLS / 2) as f64 * f64::from(CELL);
        let corner = |c: f64| ((c - half) / f64::from(CELL)).floor() as f32 * CELL;
        let origin = [corner(camera.x), corner(camera.z)];
        let push = WakePush {
            from: from.address(),
            to: to.address(),
            counts: self.counts.address(),
            heights: self.heights.address(),
            emitters: emitter_buffer.address(),
            time,
            from_index,
            origin,
            cell: CELL,
            cells: CELLS,
            capacity: CAPACITY,
            emitter_count,
            around: AROUND,
            step,
            slopes: self.slopes.storage(0).0,
            pad: 0,
        };
        let particles = [graph.import_buffer(from), graph.import_buffer(to)];
        let counts = graph.import_buffer(&self.counts);
        let heights = graph.import_buffer(&self.heights);
        let slopes = graph.import(&self.slopes);
        let (counts_buffer, heights_buffer) = (&self.counts, &self.heights);
        graph
            .pass("wakes/clear")
            .queue(QueueKind::Compute)
            .buffer(counts, BufferAccess::TransferDst)
            .buffer(heights, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(counts_buffer, u64::from(1 - from_index) * 4, 4, 0);
                commands.fill_buffer(heights_buffer, 0, heights_buffer.size(), 0);
                Ok(())
            });
        let advance = &self.advance;
        graph
            .pass("wakes/advance")
            .queue(QueueKind::Compute)
            .buffer(particles[0], BufferAccess::ShaderRead(compute))
            .buffer(particles[1], BufferAccess::ShaderWrite(compute))
            .buffer(counts, BufferAccess::ShaderReadWrite(compute))
            .buffer(heights, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(advance);
                commands.push_constants(advance, &push);
                commands.dispatch(CAPACITY / 64, 1, 1);
                Ok(())
            });
        if emitter_count > 0 {
            let emit = &self.emit;
            graph
                .pass("wakes/emit")
                .queue(QueueKind::Compute)
                .buffer(particles[1], BufferAccess::ShaderReadWrite(compute))
                .buffer(counts, BufferAccess::ShaderReadWrite(compute))
                .buffer(heights, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(emit);
                    commands.push_constants(emit, &push);
                    commands.dispatch((emitter_count * AROUND).div_ceil(64), 1, 1);
                    Ok(())
                });
        }
        let slopes_pass = &self.slopes_pass;
        graph
            .pass("wakes/slopes")
            .queue(QueueKind::Compute)
            .buffer(heights, BufferAccess::ShaderRead(compute))
            .image(slopes, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(slopes_pass);
                commands.push_constants(slopes_pass, &push);
                commands.dispatch(CELLS / 8, CELLS / 8, 1);
                Ok(())
            });
        WakeFrame {
            slopes,
            window: [origin[0], origin[1], 1.0 / (CELLS as f32 * CELL)],
        }
    }
}
