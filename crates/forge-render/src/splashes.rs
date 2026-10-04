//! Splashes (#107, `shaders/splashes.slang`): the spray thrown up where things meet the water,
//! as ballistic particles with drag (`docs/research/water.md` §7, D-038's afterwards; D-009's
//! near water: GPU particles, visual only).
//!
//! The caller says where the water splashes this frame ([`SplashSource`]): something meeting
//! it, a step's fall, a bow pushing through it, water running off something lifted out. The
//! rules of `docs/research/water.md` §7.2 turn each into sprays of drops (and mist at the
//! falls): how many, how fast, how large. The particles live in a ring of
//! [`SPLASH_CAPACITY`] slots handed out in blocks, in order, so the draw's order is the same
//! from frame to frame. A stream's drops are born at fixed times (`(k + phase) / rate` for its
//! `k`-th), and each one's random numbers come from its source's seed and `k`, so a stream is
//! the same whatever the frame rate. Every frame:
//! - `splashes/emit` and `splashes/advance` on the async compute queue: the new drops, then
//!   every live one moved to the frame's time;
//! - `splashes/draw` after the water: soft sprites streaked along their velocity, lit, hazed,
//!   writing the reactive mask TAA reads ([`crate::Taa::resolve`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation,
    Pipeline, QueueKind, Result, ShaderCompiler, ShaderStage, TransientDesc, VertexPipelineDesc,
    vk,
};
use glam::{Mat4, Vec2, Vec3};

use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// Slots of the ring: 65 536 drops over lives of 2.5 s at most sustain 26 000 new ones a second.
pub const SPLASH_CAPACITY: u32 = 1 << 16;
/// Bytes of a particle (`SplashParticle` in `splashes.slang`).
const PARTICLE_BYTES: u64 = 48;
/// Blocks a frame may emit, and drops a block may hold.
const MAX_BLOCKS: usize = 1024;
const MAX_BLOCK: u32 = 4096;
/// The farthest a source sprays from the camera, metres, and where its spray starts to fade.
const REACH: f32 = 150.0;
const FADE_FROM: f32 = 100.0;
/// The format of the reactive mask TAA reads.
const REACTIVE_FORMAT: vk::Format = vk::Format::R8_UNORM;
const GRAVITY: f32 = 9.81;
/// Below this speed into the water, m/s, an impact makes no crown, only a few drops (Duez et
/// al. 2007's threshold, "a few metres per second"; Forge's choice for a smooth body).
const CROWN_THRESHOLD: f32 = 2.0;
/// Kinds of block (`SPLASH_*` in `splashes.slang`).
const RING: u32 = 0;
const CONE: u32 = 1;
const MIST: u32 = 2;

/// Where the water splashes this frame (#107), in the sea's frame (metres, the sea's mean level
/// at y = 0).
#[derive(Clone, Copy, Debug)]
pub enum SplashSource {
    /// Something meeting the water at `time` (the sea's seconds), at `position` on the water's
    /// level: a crown of drops thrown up round its waterline, and for one going in fast enough,
    /// the jet that rises when its cavity closes, `2 √(R / g)` later (Truscott et al. 2014,
    /// Rabbi et al. 2020). Pass it on for a second after `time`: each burst comes in the frame
    /// its time falls in.
    Impact {
        /// Where its waterline's middle meets the water.
        position: Vec3,
        /// Its velocity as it meets the water, m/s.
        velocity: Vec3,
        /// The radius of its outline at the water's level, metres.
        radius: f32,
        /// Its density over the water's: a buoyant body's jet is weaker (Aristoff et al. 2010).
        density: f32,
        /// When it meets the water, the sea's seconds.
        time: f32,
        /// This impact's own.
        seed: u32,
    },
    /// A step's fall (#122): drops thrown up where its water plunges into the pool below, and
    /// mist, by how far the falling sheet breaks up (Horeni's break-up length `6 q^0.32`).
    Fall {
        /// The foot of the fall, on the pool's level.
        foot: Vec3,
        /// Downstream (world x and z), unit.
        downstream: Vec2,
        /// Half the river's width there, metres.
        half_width: f32,
        /// How far the water drops, metres.
        drop: f32,
        /// The water's speed over the lip, m/s.
        speed: f32,
        /// The water's depth over the lip, metres.
        depth: f32,
        /// This fall's own.
        seed: u32,
    },
    /// Something pushing through still water: spray off its bow, by its Froude number on its
    /// width (a fringe from 0.7, fans from 1.5; Chaplin & Teigen 2003).
    Bow {
        /// The middle of its bow, on the water's level.
        bow: Vec3,
        /// Its velocity through the water, world x and z, m/s.
        velocity: Vec2,
        /// Its width across its way, metres.
        beam: f32,
        /// Its width along its way, metres: the length the Froude number takes.
        length: f32,
        /// This bow's own.
        seed: u32,
    },
    /// Water running off something lifted out of it.
    Drip {
        /// Where the drops leave it.
        position: Vec3,
        /// They leave along `position ± spread`.
        spread: Vec3,
        /// Its velocity, m/s.
        velocity: Vec3,
        /// The level of the water they fall back into.
        level: f32,
        /// Drops a second.
        rate: f32,
        /// This drip's own.
        seed: u32,
    },
}

/// What a frame's splashes need besides their sources.
#[derive(Clone, Copy, Debug)]
pub struct SplashParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The camera in the sea's frame.
    pub camera: Vec3,
    /// The near plane, metres.
    pub near: f32,
    /// Pixels a metre spans one metre in front of the camera.
    pub focal: f32,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sun's pre-exposed illuminance on a surface facing it, per channel.
    pub sun_radiance: Vec3,
    /// The pre-exposed luminance of a unit of sun illuminance (the sky's scale).
    pub sky_scale: f32,
    /// The air's velocity near the water, m/s: the spray drifts with it.
    pub wind: Vec3,
    /// The sea's clock, seconds.
    pub time: f32,
    /// Seconds a drop is streaked over: half a frame.
    pub shutter: f32,
    /// The scene's top-level structure for the sun's shadow (0 for none). Only a structure built
    /// before the frame: the static one.
    pub tlas: u64,
}

/// The splashes' counters, for the log.
#[derive(Clone, Copy, Debug, Default)]
pub struct SplashStats {
    /// Slots in the ring's live range.
    pub live: u32,
    /// Drops born this frame.
    pub fresh: u32,
    /// Drops the ring had no room for, since the start.
    pub dropped: u64,
}

/// Mirrors `SplashBlock` in `splashes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuSplashBlock {
    origin: [f32; 4],
    spread: [f32; 4],
    axis: [f32; 4],
    speed: [f32; 4],
    life: [f32; 4],
    carry: [f32; 4],
    first: u32,
    count: u32,
    seed: u32,
    kind: u32,
    start: u32,
    index: u32,
    alpha: f32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuSplashBlock>() == 128);

/// Mirrors `SplashFrame` in `splashes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuSplashFrame {
    view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    sun_radiance: [f32; 4],
    wind: [f32; 4],
    now: f32,
    previous: f32,
    focal: f32,
    depth: u32,
    width: f32,
    height: f32,
    base: u32,
    live: u32,
    fresh: u32,
    block_count: u32,
    capacity: u32,
    pad: u32,
    tlas: u64,
    sky: u64,
    ring: u64,
    blocks: u64,
    foam: u64,
    foam_origin: [i32; 2],
    foam_cell: f32,
    foam_cells: u32,
    pad_foam: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuSplashFrame>() == 240);

/// The foam field's cells a side, metres a cell, and the seconds its foam takes to fall by e
/// (#107's polish): 64 m round the camera.
const FOAM_CELLS: u32 = 256;
const FOAM_CELL: f32 = 0.25;
const FOAM_LIFE: f32 = 3.0;

/// Mirrors `FoamPush` in `splash_foam.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FoamPush {
    foam: u64,
    origin: [i32; 2],
    previous: [i32; 2],
    keep: f32,
    cells: u32,
    fresh: u32,
    pad: u32,
}

/// The foam the splashes leave where their drops land on the water (#107's polish), this
/// frame: a field of cells round the camera that the water's shading whitens by
/// ([`crate::WaterSurfaceParams::splash_foam`]). Its units are whole: `FOAM_FULL` in
/// `water.slang` whitens a cell fully.
#[derive(Clone, Copy, Debug)]
pub struct SplashFoam {
    /// The field in this frame's graph.
    pub buffer: BufferHandle,
    /// Its device address: `cells × cells` units, a world cell at its index modulo `cells`.
    pub address: u64,
    /// The window's first world cell (x, z).
    pub origin: [i32; 2],
    /// Metres a cell.
    pub cell: f32,
    /// Cells a side.
    pub cells: u32,
}

/// One block of drops before it has its slots.
#[derive(Clone, Copy, Debug)]
struct Spray {
    block: GpuSplashBlock,
    /// When its last drop dies at the latest, the sea's seconds.
    expires: f32,
}

/// The ring's bookkeeping on the CPU: slots counted from the start, so a block's are
/// `first % SPLASH_CAPACITY` onwards.
#[derive(Debug, Default)]
struct Ring {
    /// The next slot to hand out.
    head: u64,
    /// The blocks still alive, oldest first: their first slot, their count, when they expire.
    blocks: VecDeque<(u64, u32, f32)>,
    /// The time of the last update, the sea's seconds (`None` before the first).
    previous: Option<f32>,
    dropped: u64,
}

impl Ring {
    /// The first live slot.
    fn tail(&self) -> u64 {
        self.blocks.front().map_or(self.head, |b| b.0)
    }
}

/// The splashes' particles and passes (#107).
pub struct WaterSplashes {
    emit: Pipeline,
    advance: Pipeline,
    draw: Pipeline,
    ring: GraphBuffer,
    /// Per frame slot: the frame's block and its blocks, written by the CPU.
    frames: Vec<Buffer>,
    blocks: Vec<Buffer>,
    state: RefCell<Ring>,
    stats: std::cell::Cell<SplashStats>,
    /// The foam field (#107's polish) and its fade.
    foam: GraphBuffer,
    fade: Pipeline,
    /// The last frame's window and time, for the fade.
    foam_last: std::cell::Cell<Option<([i32; 2], f32)>>,
    /// This frame's field, from [`WaterSplashes::foam`], for [`WaterSplashes::update`].
    foam_now: std::cell::Cell<Option<SplashFoam>>,
}

impl WaterSplashes {
    /// Compiles the passes and makes the ring.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let push = std::mem::size_of::<u64>() as u32;
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("splashes.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let vertex = device.create_shader_module(
            &shaders.compile("splashes.slang", "vert_main", ShaderStage::Vertex)?,
            "splashes vertex",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("splashes.slang", "frag_main", ShaderStage::Fragment)?,
            "splashes fragment",
        )?;
        let draw = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[HDR_FORMAT, REACTIVE_FORMAT],
            depth_format: None,
            push_constant_bytes: push,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: false,
            alpha_blend: true,
            name: "splashes draw",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        let ring = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: u64::from(SPLASH_CAPACITY) * PARTICLE_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "splash particles",
        })?);
        let per_slot = |size: usize, name: &str| -> Result<Vec<Buffer>> {
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
                .collect()
        };
        Ok(Self {
            emit: compute("emit_main", "splashes emit")?,
            advance: compute("advance_main", "splashes advance")?,
            draw: draw?,
            ring,
            frames: per_slot(std::mem::size_of::<GpuSplashFrame>(), "splash frame")?,
            blocks: per_slot(
                MAX_BLOCKS * std::mem::size_of::<GpuSplashBlock>(),
                "splash blocks",
            )?,
            state: RefCell::new(Ring::default()),
            stats: std::cell::Cell::new(SplashStats::default()),
            foam: GraphBuffer::new(device.create_buffer(BufferDesc {
                size: u64::from(FOAM_CELLS * FOAM_CELLS) * 4,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name: "splash foam",
            })?),
            fade: {
                let module = device.create_shader_module(
                    &shaders.compile("splash_foam.slang", "fade_main", ShaderStage::Compute)?,
                    "splash foam fade",
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, "fade_main"),
                    push_constant_bytes: std::mem::size_of::<FoamPush>() as u32,
                    name: "splash foam fade",
                });
                device.destroy_shader_module(module);
                pipeline?
            },
            foam_last: std::cell::Cell::new(None),
            foam_now: std::cell::Cell::new(None),
        })
    }

    /// Bytes of the ring.
    pub fn bytes(&self) -> u64 {
        self.ring.size()
    }

    /// The last frame's counters.
    pub fn stats(&self) -> SplashStats {
        self.stats.get()
    }

    /// The foam field this frame (#107's polish), round `camera` (the sea's frame) at the sea's
    /// `time`: declares `splashes/foam`, the last frames' foam faded and the cells that came
    /// into the window cleared. Call it once a frame before the water's draw, which reads it,
    /// and before [`WaterSplashes::update`], which adds the drops that land.
    pub fn foam<'f>(&'f self, graph: &mut FrameGraph<'f>, camera: Vec3, time: f32) -> SplashFoam {
        let n = FOAM_CELLS as i32;
        let origin = [camera.x, camera.z].map(|c| (c / FOAM_CELL).floor() as i32 - n / 2);
        let last = self.foam_last.replace(Some((origin, time)));
        // A clock that jumped or went back clears the field, as the ring starts over.
        let (previous, fresh, keep) = match last {
            Some((previous, then)) if (0.0..=0.25).contains(&(time - then)) => {
                (previous, 0, (-(time - then) / FOAM_LIFE).exp())
            }
            _ => (origin, 1, 0.0),
        };
        let buffer = graph.import_buffer(&self.foam);
        let address = self.foam.address();
        let fade = &self.fade;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        graph
            .pass("splashes/foam")
            .buffer(buffer, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(fade);
                commands.push_constants(
                    fade,
                    &FoamPush {
                        foam: address,
                        origin,
                        previous,
                        keep,
                        cells: FOAM_CELLS,
                        fresh,
                        pad: 0,
                    },
                );
                commands.dispatch(FOAM_CELLS.div_ceil(8), FOAM_CELLS.div_ceil(8), 1);
                Ok(())
            });
        let foam = SplashFoam {
            buffer,
            address,
            origin,
            cell: FOAM_CELL,
            cells: FOAM_CELLS,
        };
        self.foam_now.set(Some(foam));
        foam
    }

    /// Emits this frame's drops from `sources` and declares the passes: the emission and the
    /// motion on the async compute queue, and the draw over `color` (the frame's HDR image,
    /// after the water) against `depth`. Returns the reactive mask TAA reads, or `None` when no
    /// drop is alive.
    #[allow(clippy::too_many_arguments)]
    pub fn update<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sources: &[SplashSource],
        sky: &SkyFrame,
        params: SplashParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) -> Option<ImageHandle> {
        let now = params.time;
        // The foam field, when [`WaterSplashes::foam`] made it this frame.
        let foam = self.foam_now.take();
        let mut ring = self.state.borrow_mut();
        // A clock that jumped or went back starts the ring over.
        let previous = match ring.previous {
            Some(p) if (0.0..=0.25).contains(&(now - p)) => p,
            _ => {
                ring.blocks.clear();
                now
            }
        };
        ring.previous = Some(now);
        while ring.blocks.front().is_some_and(|b| b.2 < now) {
            ring.blocks.pop_front();
        }
        // This frame's sprays, each with its slots.
        let mut sprays: Vec<Spray> = Vec::new();
        for source in sources {
            sprays_of(source, params.camera, previous, now, &mut sprays);
        }
        let mut blocks: Vec<GpuSplashBlock> = Vec::with_capacity(sprays.len().min(MAX_BLOCKS));
        let mut fresh = 0_u32;
        for spray in sprays.into_iter().take(MAX_BLOCKS) {
            let room = u64::from(SPLASH_CAPACITY) - (ring.head - ring.tail());
            let count = u64::from(spray.block.count).min(room) as u32;
            ring.dropped += u64::from(spray.block.count - count);
            if count == 0 {
                continue;
            }
            let first = ring.head;
            ring.head += u64::from(count);
            ring.blocks.push_back((first, count, spray.expires));
            blocks.push(GpuSplashBlock {
                first: (first % u64::from(SPLASH_CAPACITY)) as u32,
                count,
                start: fresh,
                ..spray.block
            });
            fresh += count;
        }
        let base = (ring.tail() % u64::from(SPLASH_CAPACITY)) as u32;
        let live = (ring.head - ring.tail()) as u32;
        self.stats.set(SplashStats {
            live,
            fresh,
            dropped: ring.dropped,
        });
        drop(ring);
        if live == 0 {
            return None;
        }
        let blocks_buffer = &self.blocks[slot.index];
        if !blocks.is_empty() {
            blocks_buffer.write(0, &blocks);
        }
        let frame_buffer = &self.frames[slot.index];
        frame_buffer.write(
            0,
            &[GpuSplashFrame {
                view_proj: params.view_proj.to_cols_array(),
                camera: params.camera.extend(params.near).to_array(),
                // The brightest a drop may show: 16 times a white surface in the sun.
                sun: params
                    .sun_dir
                    .normalize_or(Vec3::Y)
                    .extend(16.0 * luminance(params.sun_radiance) / PI)
                    .to_array(),
                sun_radiance: params.sun_radiance.extend(params.sky_scale).to_array(),
                wind: params.wind.extend(params.shutter).to_array(),
                now,
                previous,
                focal: params.focal,
                depth: 0,
                width: extent.width as f32,
                height: extent.height as f32,
                base,
                live,
                fresh,
                block_count: blocks.len() as u32,
                capacity: SPLASH_CAPACITY,
                pad: 0,
                tlas: params.tlas,
                sky: sky.address(),
                ring: self.ring.address(),
                blocks: blocks_buffer.address(),
                foam: foam.map_or(0, |f| f.address),
                foam_origin: foam.map_or([0; 2], |f| f.origin),
                foam_cell: FOAM_CELL,
                foam_cells: FOAM_CELLS,
                pad_foam: [0; 2],
            }],
        );
        let address = frame_buffer.address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let ring_handle = graph.import_buffer(&self.ring);
        if fresh > 0 {
            let emit = &self.emit;
            graph
                .pass("splashes/emit")
                .queue(QueueKind::Compute)
                .buffer(ring_handle, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(emit);
                    commands.push_constants(emit, &address);
                    commands.dispatch(fresh.div_ceil(64), 1, 1);
                    Ok(())
                });
        }
        let advance = &self.advance;
        let mut pass = graph
            .pass("splashes/advance")
            .queue(QueueKind::Compute)
            .buffer(ring_handle, BufferAccess::ShaderReadWrite(compute));
        if let Some(foam) = foam {
            pass = pass.buffer(foam.buffer, BufferAccess::ShaderReadWrite(compute));
        }
        pass.run(move |_, commands| {
            commands.bind_pipeline(advance);
            commands.push_constants(advance, &address);
            commands.dispatch(live.div_ceil(64), 1, 1);
            Ok(())
        });
        let reactive = graph.transient(TransientDesc {
            name: "splash reactive mask",
            width: extent.width,
            height: extent.height,
            format: REACTIVE_FORMAT,
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let vertex = vk::PipelineStageFlags2::VERTEX_SHADER;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let draw = &self.draw;
        graph
            .pass("splashes/draw")
            .buffer(ring_handle, BufferAccess::ShaderRead(vertex))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(vertex))
            .image(sky.aerial(), ImageAccess::Sampled(vertex))
            .image(depth, ImageAccess::Sampled(fragment))
            .image(color, ImageAccess::ColorAttachment)
            .image(reactive, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                frame_buffer.write(
                    std::mem::offset_of!(GpuSplashFrame, depth) as u64,
                    &[resources.sampled(depth).0],
                );
                let attachments = [
                    vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(color))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(vk::AttachmentLoadOp::LOAD)
                        .store_op(vk::AttachmentStoreOp::STORE),
                    vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(reactive))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)
                        .clear_value(vk::ClearValue::default()),
                ];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments);
                commands.begin_rendering(&info);
                commands.bind_pipeline(draw);
                commands.set_viewport_full(extent);
                commands.push_constants(draw, &address);
                commands.draw(live * 6, 1);
                commands.end_rendering();
                Ok(())
            });
        Some(reactive)
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// A block of drops born from `first` (the sea's seconds) every `interval` seconds, the first
/// being its source's `index`-th.
#[allow(clippy::too_many_arguments)]
fn block(
    kind: u32,
    origin: Vec3,
    level: f32,
    seed: u32,
    index: u32,
    count: u32,
    first: f32,
    interval: f32,
) -> GpuSplashBlock {
    GpuSplashBlock {
        origin: origin.extend(level).to_array(),
        kind,
        seed,
        index,
        count: count.min(MAX_BLOCK),
        life: [0.0, 0.0, first, interval],
        alpha: 1.0,
        ..GpuSplashBlock::default()
    }
}

/// The births of a stream of `rate` drops a second in `(previous, now]`: the index of the
/// first, how many, and when the first is born. Its `k`-th drop is born at
/// `(k + phase) / rate`, `phase` from its seed, so the stream does not depend on the frames.
fn stream(rate: f32, seed: u32, previous: f32, now: f32) -> Option<(u32, u32, f32)> {
    if rate <= 0.0 {
        return None;
    }
    let rate = f64::from(rate);
    let phase = f64::from(seed) / f64::from(u32::MAX);
    let first = (rate * f64::from(previous) - phase).floor() + 1.0;
    let last = (rate * f64::from(now) - phase).floor();
    if last < first || first < 0.0 {
        return None;
    }
    let count = ((last - first) as u32 + 1).min(MAX_BLOCK);
    Some((first as u32, count, ((first + phase) / rate) as f32))
}

/// Seeds a source's second stream from its first.
fn reseed(seed: u32, salt: u32) -> u32 {
    seed.wrapping_mul(0x9e37_79b9) ^ salt.wrapping_mul(0x85eb_ca6b)
}

/// The sprays `source` makes in `(previous, now]`, by the rules of `docs/research/water.md`
/// §7.2 (their numbers are starting values, tuned by eye).
fn sprays_of(source: &SplashSource, camera: Vec3, previous: f32, now: f32, out: &mut Vec<Spray>) {
    let near = |at: Vec3| {
        let distance = at.distance(camera);
        (distance < REACH)
            .then(|| 1.0 - ((distance - FADE_FROM) / (REACH - FADE_FROM)).clamp(0.0, 1.0))
    };
    let within = |t: f32| previous < t && t <= now;
    match *source {
        SplashSource::Impact {
            position,
            velocity,
            radius,
            density,
            time,
            seed,
        } => {
            let Some(fade) = near(position) else { return };
            let speed = velocity.length();
            let flat = Vec3::new(velocity.x, 0.0, velocity.z);
            if within(time) {
                // The crown: drops thrown up and out round the waterline, a hundred a metre of
                // it per m/s over the threshold (Chentanez & Müller 2010's mirrored velocity
                // gives their speed: 0.2 to 0.6 of the body's); a few without one.
                let count = if speed >= CROWN_THRESHOLD {
                    (100.0 * (speed - CROWN_THRESHOLD) * 2.0 * PI * radius).min(2000.0) as u32
                } else {
                    (10.0 * speed) as u32
                };
                if count > 0 {
                    let mut b = block(
                        RING,
                        position,
                        position.y,
                        seed,
                        0,
                        count,
                        time,
                        0.06 / count as f32,
                    );
                    b.spread[3] = radius;
                    b.axis[3] = 80_f32.to_radians();
                    b.carry = (0.2 * flat).extend(55_f32.to_radians()).to_array();
                    b.speed = [0.2 * speed, 0.6 * speed, 0.005, 0.015];
                    b.life[0] = 0.6;
                    b.life[1] = 1.5;
                    b.alpha = 0.85 * fade;
                    out.push(Spray {
                        block: b,
                        expires: time + 0.06 + 1.5,
                    });
                }
            }
            // The jet when the cavity closes, weaker for a buoyant body.
            let closes = time + 2.0 * (radius / GRAVITY).sqrt();
            if speed >= CROWN_THRESHOLD && within(closes) {
                let strength = density.clamp(0.0, 1.0);
                let count = (30.0 + 120.0 * strength) as u32;
                let rise = speed * strength.sqrt();
                let mut b = block(
                    CONE,
                    position,
                    position.y,
                    reseed(seed, 1),
                    0,
                    count,
                    closes,
                    0.1 / count as f32,
                );
                b.spread = (0.15 * radius * Vec3::X).extend(0.0).to_array();
                b.axis = Vec3::Y.extend(6_f32.to_radians()).to_array();
                b.speed = [0.2 * rise, 0.5 * rise, 0.008, 0.02];
                b.life = [0.8, 1.5, closes, 0.1 / count as f32];
                b.alpha = 0.85 * fade;
                out.push(Spray {
                    block: b,
                    expires: closes + 0.1 + 1.5,
                });
            }
        }
        SplashSource::Fall {
            foot,
            downstream,
            half_width,
            drop,
            speed,
            depth,
            seed,
        } => {
            let Some(fade) = near(foot) else { return };
            // The sheet over the lip: q = depth × speed a metre of width; it breaks up over
            // 6 q^0.32 m (Horeni), and the share of the drop past that (at most 1.5) sets the
            // spray. It meets the pool at √(v₀² + 2gH).
            let q = (depth * speed).max(0.01);
            let breakup = 6.0 * q.powf(0.32);
            let broken = (drop / breakup).min(1.5);
            let impact = (speed * speed + 2.0 * GRAVITY * drop).sqrt();
            let down = Vec3::new(downstream.x, 0.0, downstream.y).normalize_or(Vec3::X);
            let across = Vec3::new(-down.z, 0.0, down.x);
            let origin = foot + Vec3::new(0.0, 0.05, 0.0);
            let width = 2.0 * half_width;
            // Drops thrown up and downstream at 35–85°, 0.2–0.5 of the impact speed: the
            // research's 150 a metre a second at 0.15–0.35, livelier (they barely left the
            // white water).
            let rate = 200.0 * width * broken;
            if let Some((index, count, first)) = stream(rate, seed, previous, now) {
                let axis = (down * 30_f32.to_radians().sin() + Vec3::Y * 30_f32.to_radians().cos())
                    .normalize();
                let mut b = block(CONE, origin, foot.y, seed, index, count, first, 0.0);
                b.spread = (half_width * across).extend(0.0).to_array();
                b.axis = axis.extend(25_f32.to_radians()).to_array();
                b.speed = [0.2 * impact, 0.5 * impact, 0.008, 0.025];
                b.life = [0.4, 1.0, first, 1.0 / rate];
                b.alpha = 0.8 * fade;
                out.push(Spray {
                    block: b,
                    expires: now + 1.0,
                });
            }
            // Mist: large soft puffs over the foot, drifting with the air, one or two a metre of
            // the fall alive at once.
            let mist_rate = (1.0 + 1.5 * broken.min(1.0)) / 2.0 * width;
            if let Some((index, count, first)) = stream(mist_rate, reseed(seed, 2), previous, now) {
                let mut b = block(
                    CONE | MIST,
                    foot + Vec3::new(0.0, 0.3 * drop, 0.0),
                    foot.y,
                    reseed(seed, 2),
                    index,
                    count,
                    first,
                    1.0 / mist_rate,
                );
                b.spread = (0.8 * half_width * across).extend(0.0).to_array();
                b.axis = (down + Vec3::Y)
                    .normalize()
                    .extend(60_f32.to_radians())
                    .to_array();
                b.speed = [0.2, 0.6, (0.5 * drop).max(0.3), drop.max(0.4)];
                b.life = [1.6, 2.4, first, 1.0 / mist_rate];
                b.alpha = 0.08 * (0.5 + broken.min(1.0)) * fade;
                out.push(Spray {
                    block: b,
                    expires: now + 2.4,
                });
            }
        }
        SplashSource::Bow {
            bow,
            velocity,
            beam,
            length,
            seed,
        } => {
            let Some(fade) = near(bow) else { return };
            let speed = velocity.length();
            let froude = speed / (GRAVITY * length.max(0.05)).sqrt();
            if froude <= 0.7 {
                return;
            }
            let ahead = Vec3::new(velocity.x, 0.0, velocity.y) / speed;
            let across = Vec3::new(-ahead.z, 0.0, ahead.x);
            let carry = 0.5 * Vec3::new(velocity.x, 0.0, velocity.y);
            // A fringe off the bow wave from 0.7, spilling; fans either side from 1.5.
            let fringe = 50.0 * length * (froude - 0.7);
            if let Some((index, count, first)) = stream(fringe, seed, previous, now) {
                let mut b = block(CONE, bow, bow.y, seed, index, count, first, 1.0 / fringe);
                b.spread = (0.5 * beam * across).extend(0.0).to_array();
                b.axis = (ahead + Vec3::Y)
                    .normalize()
                    .extend(30_f32.to_radians())
                    .to_array();
                b.speed = [0.3 * speed, 0.6 * speed, 0.003, 0.008];
                b.life = [0.3, 0.6, first, 1.0 / fringe];
                b.carry = carry.extend(0.0).to_array();
                b.alpha = 0.8 * fade;
                out.push(Spray {
                    block: b,
                    expires: now + 0.6,
                });
            }
            if froude > 1.5 {
                let fan = 400.0 * length * (froude - 1.5);
                for (side, salt) in [(1.0_f32, 3), (-1.0, 4)] {
                    let seed = reseed(seed, salt);
                    let Some((index, count, first)) = stream(fan, seed, previous, now) else {
                        continue;
                    };
                    let out_of = (ahead * 40_f32.to_radians().cos()
                        + side * across * 40_f32.to_radians().sin())
                    .normalize();
                    let axis = (out_of + 0.6 * Vec3::Y).normalize();
                    let mut b = block(
                        CONE,
                        bow + side * 0.5 * beam * across,
                        bow.y,
                        seed,
                        index,
                        count,
                        first,
                        1.0 / fan,
                    );
                    b.axis = axis.extend(20_f32.to_radians()).to_array();
                    b.speed = [0.5 * speed, speed, 0.004, 0.012];
                    b.life = [0.4, 0.9, first, 1.0 / fan];
                    b.carry = carry.extend(0.0).to_array();
                    b.alpha = 0.8 * fade;
                    out.push(Spray {
                        block: b,
                        expires: now + 0.9,
                    });
                }
            }
        }
        SplashSource::Drip {
            position,
            spread,
            velocity,
            level,
            rate,
            seed,
        } => {
            let Some(fade) = near(position) else { return };
            if let Some((index, count, first)) = stream(rate, seed, previous, now) {
                let mut b = block(CONE, position, level, seed, index, count, first, 1.0 / rate);
                b.spread = spread.extend(0.0).to_array();
                b.axis = (-Vec3::Y).extend(0.3).to_array();
                b.speed = [0.0, 0.2, 0.003, 0.006];
                b.life = [1.5, 2.0, first, 1.0 / rate];
                b.carry = velocity.extend(0.0).to_array();
                b.alpha = 0.8 * fade;
                out.push(Spray {
                    block: b,
                    expires: now + 2.0,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_the_same_whatever_the_frames() {
        // 37.5 drops a second over 2 s, in frames of 1/60 s and of 1/7 s: the same births.
        let births = |step: f32| {
            let mut all = Vec::new();
            let mut t = 0.0_f32;
            while t < 2.0 {
                let next = (t + step).min(2.0);
                if let Some((index, count, first)) = stream(37.5, 12345, t, next) {
                    for j in 0..count {
                        all.push((index + j, first + j as f32 / 37.5));
                    }
                }
                t = next;
            }
            all
        };
        let (a, b) = (births(1.0 / 60.0), births(1.0 / 7.0));
        assert_eq!(a.len(), b.len());
        assert!((a.len() as i32 - 75).abs() <= 1, "{} drops", a.len());
        for ((ka, ta), (kb, tb)) in a.iter().zip(&b) {
            assert_eq!(ka, kb);
            assert!((ta - tb).abs() < 1e-4);
        }
        // Consecutive indices, every birth inside its window.
        assert!(a.windows(2).all(|w| w[1].0 == w[0].0 + 1));
    }

    #[test]
    fn an_impact_bursts_once_and_its_jet_follows() {
        let impact = SplashSource::Impact {
            position: Vec3::new(0.0, 10.0, 0.0),
            velocity: Vec3::new(0.0, -7.3, 0.0),
            radius: 0.41,
            density: 0.5,
            time: 5.0,
            seed: 7,
        };
        let camera = Vec3::new(10.0, 15.0, 0.0);
        let sprays = |previous: f32, now: f32| {
            let mut out = Vec::new();
            sprays_of(&impact, camera, previous, now, &mut out);
            out
        };
        // Before and after: nothing; the frame its time falls in: the crown, 100 drops a metre
        // of waterline per m/s over the threshold.
        assert!(sprays(4.9, 4.95).is_empty());
        let crown = sprays(4.99, 5.01);
        assert_eq!(crown.len(), 1);
        let expected = 100.0 * (7.3 - CROWN_THRESHOLD) * 2.0 * PI * 0.41;
        assert!((crown[0].block.count as f32 - expected).abs() <= 1.0);
        assert_eq!(crown[0].block.kind, RING);
        // The jet 2 √(R / g) later, once.
        let closes = 5.0 + 2.0 * (0.41_f32 / GRAVITY).sqrt();
        let jet = sprays(closes - 0.01, closes + 0.01);
        assert_eq!(jet.len(), 1);
        assert_eq!(jet[0].block.kind, CONE);
        assert!(sprays(closes + 0.01, closes + 0.5).is_empty());
        // Too far away: nothing.
        let mut far = Vec::new();
        sprays_of(&impact, Vec3::new(500.0, 0.0, 0.0), 4.99, 5.01, &mut far);
        assert!(far.is_empty());
    }

    #[test]
    fn a_fall_sprays_by_how_far_its_sheet_breaks_up() {
        let fall = |drop: f32| SplashSource::Fall {
            foot: Vec3::ZERO,
            downstream: Vec2::X,
            half_width: 2.0,
            drop,
            speed: 2.0,
            depth: 0.1,
            seed: 3,
        };
        let drops = |drop: f32| {
            let mut out = Vec::new();
            sprays_of(&fall(drop), Vec3::new(5.0, 2.0, 0.0), 0.0, 1.0, &mut out);
            out.iter()
                .filter(|s| s.block.kind & MIST == 0)
                .map(|s| s.block.count)
                .sum::<u32>()
        };
        // q = 0.2 m²/s breaks up over 6 q^0.32 = 3.58 m: a 1 m fall throws 200 × 4 m × 1/3.58
        // drops a second, a 2 m one twice that.
        let (low, high) = (drops(1.0), drops(2.0));
        let breakup = 6.0 * 0.2_f32.powf(0.32);
        assert!((low as f32 - 200.0 * 4.0 / breakup).abs() <= 1.0, "{low}");
        assert!((high as f32 - 2.0 * low as f32).abs() <= 2.0, "{high}");
    }
}
