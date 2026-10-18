//! Particles (D-064, #271, `shaders/particles.slang`): smoke, dust and embers from emitters as
//! data, grown from the splashes (#107, [`crate::WaterSplashes`]).
//!
//! An effect is a record of `data/particles.toml` ([`Effect`], [`effects`]): its rate, its
//! particles' life, size and colour over life, how they are launched, the forces on them
//! (gravity or buoyancy, a drag towards the air, the scene's wind, a curl-noise swirl) and how
//! they are drawn. The caller places emitters ([`Emitter`]: a stream from a point) and bursts
//! ([`Burst`]: a footstep's dust) in the scene's frame. The particles live in a ring of
//! [`PARTICLE_CAPACITY`] slots handed out in blocks, in order, so the draw's order is the same
//! from frame to frame. A stream's `k`-th particle is born at `start + (k + ½) / rate`, and its
//! random numbers come from its emitter's seed and `k`, so an effect is the same whatever the
//! frame rate and the map check's captures stay exact. Every frame:
//! - `particles/emit` and `particles/advance` on the async compute queue: the new particles,
//!   then every live one moved to the frame's time and lit once (D-064's point 4: one ray
//!   towards the sun against the scene's structure, the probes' light from above and below, the
//!   open sky's beyond them);
//! - `particles/draw` after the sky's compose: soft puffs facing the camera, smoke
//!   alpha-blended and embers added in one premultiplied blend, writing the reactive mask TAA
//!   reads ([`crate::Taa::resolve`]), over the splashes' when they drew this frame.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    QueueKind, Result, ShaderCompiler, ShaderStage, TransientDesc, VertexPipelineDesc, vk,
};
use glam::{Mat4, Vec3};
use serde::Deserialize;

use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// Slots of the ring: a village's chimneys, at a dozen puffs a second over lives of seven
/// seconds, are a few hundred each.
pub const PARTICLE_CAPACITY: u32 = 1 << 15;
/// Bytes of a particle (`Particle` in `particles.slang`).
const PARTICLE_BYTES: u64 = 64;
/// Blocks a frame may emit.
const MAX_BLOCKS: usize = 1024;
/// Effects the table holds.
const MAX_EFFECTS: usize = 64;
/// The farthest an emitter is simulated from the camera, metres.
const REACH: f32 = 400.0;
/// The format of the reactive mask TAA reads (the splashes').
const REACTIVE_FORMAT: vk::Format = vk::Format::R8_UNORM;

/// How an effect's particles are drawn (`DRAW_*` in `particles.slang`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawKind {
    /// Lit, alpha-blended puffs: smoke, dust, steam.
    Smoke,
    /// Unlit and added, their colour in thousands of cd/m²: embers, sparks.
    Glow,
}

/// An effect (D-064): a record of `data/particles.toml`, whose header says what each field is.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// Its name, the record's key (set by [`parse`]).
    #[serde(skip)]
    pub name: String,
    /// Particles a second, or a burst's count.
    pub rate: f32,
    /// A particle's life, seconds: the shortest and the longest.
    pub life: [f32; 2],
    /// Its drawn radius at birth and at death, metres.
    pub radius: [f32; 2],
    /// Its launch speed, m/s: the slowest and the fastest.
    pub speed: [f32; 2],
    /// Degrees from the emitter's direction it is launched within.
    pub cone: f32,
    /// Metres from the emitter's point it is born within, across the direction.
    pub spread: f32,
    /// Times g, downwards; negative rises.
    pub gravity: f32,
    /// 1/s: how fast it takes the air's velocity.
    pub drag: f32,
    /// The share of the scene's wind the air it rides has.
    pub wind: f32,
    /// The swirl's speed (m/s) and its eddies' size (metres).
    pub curl: [f32; 2],
    /// rgba at birth and at death: linear albedo and opacity for smoke, light for a glow.
    pub colour: [[f32; 4]; 2],
    /// How it is drawn.
    pub draw: DrawKind,
    /// A burst of `rate` particles when triggered, not a stream.
    #[serde(default)]
    pub burst: bool,
    /// The flipbook its smoke is drawn from: `"smoke"`, the puff our own solver bakes
    /// ([`crate::smoke_bake`], six-way lit); none, a procedural puff.
    #[serde(default)]
    pub texture: Option<String>,
    /// It meets the scene (D-064's point 2, the TLAS for sparks and debris): the share of its
    /// speed into a surface it keeps, bounced back (0 to 1). None: it passes through.
    #[serde(default)]
    pub bounce: Option<f32>,
    /// It is drawn as a streak along its flight, as a shutter sees it.
    #[serde(default)]
    pub streak: bool,
}

/// Reads effects in the form of `data/particles.toml`, in the file's order.
pub fn parse(text: &str) -> std::result::Result<Vec<Effect>, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut effects = Vec::with_capacity(table.len());
    for (name, value) in table {
        let mut effect: Effect = value
            .try_into()
            .map_err(|e: toml::de::Error| format!("{name}: {e}"))?;
        effect.name = name.clone();
        let positive = effect.rate > 0.0
            && effect.life[0] > 0.0
            && effect.life[1] >= effect.life[0]
            && effect.speed[1] >= effect.speed[0]
            && effect.radius.iter().all(|r| *r > 0.0)
            && effect.drag >= 0.0
            && (0.0..=180.0).contains(&effect.cone)
            && effect.texture.as_deref().is_none_or(|t| t == "smoke")
            && effect.bounce.is_none_or(|b| (0.0..=1.0).contains(&b));
        if !positive {
            return Err(format!(
                "{name}: a rate, lives, radii and a drag of 0 or more, ranges in order, a cone of 0–180°, no texture but \"smoke\", a bounce of 0–1"
            ));
        }
        effects.push(effect);
    }
    if effects.len() > MAX_EFFECTS {
        return Err(format!("{} effects: at most {MAX_EFFECTS}", effects.len()));
    }
    Ok(effects)
}

/// The built-in effects (`data/particles.toml`).
pub fn effects() -> Vec<Effect> {
    parse(include_str!("../data/particles.toml")).expect("the built-in effects parse")
}

/// A stream of particles from a point (the scene's frame): a chimney.
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    /// Its effect's index in the table [`Particles::new`] was given.
    pub effect: usize,
    /// Where its particles are born.
    pub position: Vec3,
    /// Which way they are launched.
    pub direction: Vec3,
    /// Its own, for its random numbers.
    pub seed: u32,
    /// When its stream began, the scene's seconds: its first particle is born half an interval
    /// later.
    pub start: f32,
}

/// A burst of an effect's `rate` particles at one time (the scene's frame): a footstep's dust.
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    /// Its effect's index.
    pub effect: usize,
    /// Where its particles are born.
    pub position: Vec3,
    /// Which way they are launched.
    pub direction: Vec3,
    /// Its velocity, added to every particle's (a walker's).
    pub carry: Vec3,
    /// Its own, for its random numbers.
    pub seed: u32,
    /// When, the scene's seconds. Pass it on for its effect's longest life after: it comes in
    /// the frame its time falls in.
    pub time: f32,
}

/// What a frame's particles need besides their emitters.
#[derive(Clone, Copy, Debug)]
pub struct ParticleParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The camera in the scene's frame.
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
    /// The pre-exposure: what a cd/m² becomes (a glow's colour is in thousands of them).
    pub exposure: f32,
    /// The air's velocity, m/s.
    pub wind: Vec3,
    /// The scene's clock, seconds.
    pub time: f32,
    /// The scene's top-level structure for the sun's shadow (0 for none): one built before the
    /// frame (the static one).
    pub tlas: u64,
    /// This frame's light probes (D-036): the ambient light at each particle where they reach,
    /// the open sky's beyond. None: the sky's everywhere.
    pub probes: Option<crate::probes::ProbeLight>,
}

/// The particles' counters, for the log and the overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParticleStats {
    /// Slots in the ring's live range.
    pub live: u32,
    /// Particles born this frame.
    pub fresh: u32,
    /// Particles the ring had no room for, since the start.
    pub dropped: u64,
}

impl ParticleStats {
    /// One line for the overlay.
    pub fn line(&self) -> String {
        format!(
            "particles: {} live, {} born this frame{}",
            self.live,
            self.fresh,
            if self.dropped > 0 {
                format!(", {} dropped", self.dropped)
            } else {
                String::new()
            }
        )
    }
}

/// Mirrors `Effect` in `particles.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuEffect {
    colour0: [f32; 4],
    colour1: [f32; 4],
    radius: [f32; 2],
    gravity: f32,
    drag: f32,
    wind: f32,
    curl: f32,
    curl_size: f32,
    draw: u32,
}

const _: () = assert!(std::mem::size_of::<GpuEffect>() == 64);

impl GpuEffect {
    fn of(e: &Effect) -> Self {
        Self {
            colour0: e.colour[0],
            colour1: e.colour[1],
            radius: e.radius,
            gravity: e.gravity,
            drag: e.drag,
            wind: e.wind,
            curl: e.curl[0],
            curl_size: e.curl[1],
            // The way it is drawn, its flipbook in bits 8 to 15 (1: the baked smoke), its bounce
            // in 16 to 23 with bit 24 set when it meets the scene, and bit 25 for a streak
            // (`DRAW_COLLIDE`, `DRAW_STREAK` in `particles.slang`).
            draw: match e.draw {
                DrawKind::Smoke => 0,
                DrawKind::Glow => 1,
            } | (u32::from(e.texture.is_some()) << 8)
                | e.bounce.map_or(0, |b| {
                    (1 << 24) | (((b.clamp(0.0, 1.0) * 255.0).round() as u32) << 16)
                })
                | (u32::from(e.streak) << 25),
        }
    }
}

/// Mirrors `Block` in `particles.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuBlock {
    origin: [f32; 4],
    axis: [f32; 4],
    speed: [f32; 4],
    carry: [f32; 4],
    birth: f32,
    interval: f32,
    first: u32,
    count: u32,
    seed: u32,
    effect: u32,
    start: u32,
    index: u32,
}

const _: () = assert!(std::mem::size_of::<GpuBlock>() == 96);

/// Mirrors `ParticleFrame` in `particles.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFrame {
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
    /// The flipbook: its positive and negative atlases (sampled), its frames and columns.
    flip: [u32; 4],
    tlas: u64,
    sky: u64,
    ring: u64,
    blocks: u64,
    effects: u64,
    /// The frame's `ProbeField`, 0 for none.
    probes: u64,
}

const _: () = assert!(std::mem::size_of::<GpuFrame>() == 240);

/// The ring's bookkeeping on the CPU: slots counted from the start, so a block's are
/// `first % PARTICLE_CAPACITY` onwards.
#[derive(Debug, Default)]
struct Ring {
    /// The next slot to hand out.
    head: u64,
    /// The blocks still alive, oldest first: their first slot, their count, when they expire.
    blocks: VecDeque<(u64, u32, f32)>,
    /// The time of the last update, the scene's seconds (`None` before the first).
    previous: Option<f32>,
    dropped: u64,
}

impl Ring {
    fn tail(&self) -> u64 {
        self.blocks.front().map_or(self.head, |b| b.0)
    }
}

/// The births of a stream at `rate` a second from `start` that fall in `(previous, now]`: the
/// first one's index and how many.
fn births(start: f32, rate: f32, previous: f32, now: f32) -> (u32, u32) {
    // The k-th is born at start + (k + ½) / rate.
    let first = ((previous - start) * rate - 0.5).floor() + 1.0;
    let last = ((now - start) * rate - 0.5).floor();
    let first = first.max(0.0);
    if last < first {
        return (first as u32, 0);
    }
    (first as u32, (last - first + 1.0) as u32)
}

/// The particles' effects, ring and passes (D-064).
pub struct Particles {
    effects: Vec<Effect>,
    emit: Pipeline,
    advance: Pipeline,
    draw: Pipeline,
    ring: GraphBuffer,
    table: Buffer,
    /// Per frame slot: the frame's block and its blocks, written by the CPU.
    frames: Vec<Buffer>,
    blocks: Vec<Buffer>,
    state: RefCell<Ring>,
    stats: Cell<ParticleStats>,
    /// The baked smoke's atlases, when an effect draws from them, and their sampled indices.
    flipbook: Option<(crate::material::TextureSet, [u32; 2])>,
}

impl Particles {
    /// Compiles the passes, makes the ring and uploads `effects` (from [`effects`] or [`parse`]).
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        effects: Vec<Effect>,
    ) -> Result<Self> {
        assert!(
            effects.len() <= MAX_EFFECTS,
            "at most {MAX_EFFECTS} effects"
        );
        let push = std::mem::size_of::<u64>() as u32;
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("particles.slang", entry, ShaderStage::Compute)?,
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
            &shaders.compile("particles.slang", "vert_main", ShaderStage::Vertex)?,
            "particles vertex",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("particles.slang", "frag_main", ShaderStage::Fragment)?,
            "particles fragment",
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
            premultiplied: true,
            name: "particles draw",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        let ring = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: u64::from(PARTICLE_CAPACITY) * PARTICLE_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "particles",
        })?);
        let table = device.create_buffer(BufferDesc {
            size: (MAX_EFFECTS * std::mem::size_of::<GpuEffect>()) as u64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Work,
            name: "particle effects",
        })?;
        let gpu: Vec<GpuEffect> = effects.iter().map(GpuEffect::of).collect();
        if !gpu.is_empty() {
            table.write(0, &gpu);
        }
        // The smoke's flipbook (D-064's point 6), when an effect draws from it.
        let flipbook = if effects.iter().any(|e| e.texture.is_some()) {
            let mut set = crate::material::TextureSet::new(device);
            let started = std::time::Instant::now();
            let data = crate::smoke_bake::flipbook();
            tracing::info!(
                ms = started.elapsed().as_millis(),
                "particles: the smoke's flipbook ready (baked once, then from the cache)"
            );
            let ids = [set.add(&data[0])?, set.add(&data[1])?];
            let sampled = ids.map(|id| set.sampled(id));
            Some((set, sampled))
        } else {
            None
        };
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
            effects,
            emit: compute("emit_main", "particles emit")?,
            advance: compute("advance_main", "particles advance")?,
            draw: draw?,
            ring,
            table,
            frames: per_slot(std::mem::size_of::<GpuFrame>(), "particle frame")?,
            blocks: per_slot(
                MAX_BLOCKS * std::mem::size_of::<GpuBlock>(),
                "particle blocks",
            )?,
            state: RefCell::new(Ring::default()),
            stats: Cell::new(ParticleStats::default()),
            flipbook,
        })
    }

    /// The effects, in the order emitters name them.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The index of the effect named `name`.
    pub fn effect(&self, name: &str) -> Option<usize> {
        self.effects.iter().position(|e| e.name == name)
    }

    /// Bytes of the ring.
    pub fn bytes(&self) -> u64 {
        self.ring.size()
    }

    /// The last frame's counters.
    pub fn stats(&self) -> ParticleStats {
        self.stats.get()
    }

    /// Emits this frame's particles from `emitters` and `bursts` and declares the passes: the
    /// emission and the motion on the async compute queue, and the draw over `color` (the
    /// frame's HDR image, after the sky's compose) against `depth`. With `reactive` (the
    /// splashes' mask this frame), the particles' go over it; without, they make their own.
    /// Returns the reactive mask TAA reads, or `reactive` when no particle is alive.
    #[allow(clippy::too_many_arguments)]
    pub fn update<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        emitters: &[Emitter],
        bursts: &[Burst],
        sky: &SkyFrame,
        params: ParticleParams,
        color: ImageHandle,
        depth: ImageHandle,
        reactive: Option<ImageHandle>,
        extent: vk::Extent2D,
    ) -> Option<ImageHandle> {
        let now = params.time;
        let mut ring = self.state.borrow_mut();
        // A clock that jumped or went back starts the ring over.
        let previous = match ring.previous {
            Some(p) if (0.0..=0.25).contains(&(now - p)) => p,
            _ => {
                ring.blocks.clear();
                now
            }
        };
        let first_frame = ring
            .previous
            .is_none_or(|p| !(0.0..=0.25).contains(&(now - p)));
        ring.previous = Some(now);
        while ring.blocks.front().is_some_and(|b| b.2 < now) {
            ring.blocks.pop_front();
        }
        let mut blocks: Vec<(GpuBlock, f32)> = Vec::new();
        for e in emitters {
            let Some(effect) = self.effects.get(e.effect) else {
                continue;
            };
            if effect.burst || e.position.distance(params.camera) > REACH {
                continue;
            }
            // A stream already running when the ring starts over (the first frame, a jump)
            // begins with the particles still alive then.
            let from = if first_frame {
                now - effect.life[1]
            } else {
                previous
            };
            let (index, count) = births(e.start, effect.rate, from, now);
            if count == 0 {
                continue;
            }
            let interval = 1.0 / effect.rate;
            let birth = e.start + (index as f32 + 0.5) * interval;
            blocks.push((
                block_of(
                    effect,
                    e.effect,
                    e.position,
                    e.direction,
                    Vec3::ZERO,
                    e.seed,
                    index,
                    count,
                    birth,
                    interval,
                ),
                birth + (count - 1) as f32 * interval + effect.life[1],
            ));
        }
        for b in bursts {
            let Some(effect) = self.effects.get(b.effect) else {
                continue;
            };
            let in_frame = if first_frame {
                b.time <= now && now - b.time < effect.life[1]
            } else {
                b.time > previous && b.time <= now
            };
            if !in_frame || b.position.distance(params.camera) > REACH {
                continue;
            }
            let count = effect.rate.round() as u32;
            blocks.push((
                block_of(
                    effect,
                    b.effect,
                    b.position,
                    b.direction,
                    b.carry,
                    b.seed,
                    0,
                    count,
                    b.time,
                    0.0,
                ),
                b.time + effect.life[1],
            ));
        }
        let mut gpu: Vec<GpuBlock> = Vec::with_capacity(blocks.len().min(MAX_BLOCKS));
        let mut fresh = 0_u32;
        for (block, expires) in blocks.into_iter().take(MAX_BLOCKS) {
            let room = u64::from(PARTICLE_CAPACITY) - (ring.head - ring.tail());
            let count = u64::from(block.count).min(room) as u32;
            ring.dropped += u64::from(block.count - count);
            if count == 0 {
                continue;
            }
            let first = ring.head;
            ring.head += u64::from(count);
            ring.blocks.push_back((first, count, expires));
            gpu.push(GpuBlock {
                first: (first % u64::from(PARTICLE_CAPACITY)) as u32,
                count,
                start: fresh,
                ..block
            });
            fresh += count;
        }
        // Blocks expire in the order they were handed out only roughly (a burst's life is not
        // a stream's): the tail stays at the oldest live block, which costs a few dead slots.
        let base = (ring.tail() % u64::from(PARTICLE_CAPACITY)) as u32;
        let live = (ring.head - ring.tail()) as u32;
        self.stats.set(ParticleStats {
            live,
            fresh,
            dropped: ring.dropped,
        });
        drop(ring);
        if live == 0 {
            return reactive;
        }
        let blocks_buffer = &self.blocks[slot.index];
        if !gpu.is_empty() {
            blocks_buffer.write(0, &gpu);
        }
        let frame_buffer = &self.frames[slot.index];
        frame_buffer.write(
            0,
            &[GpuFrame {
                view_proj: params.view_proj.to_cols_array(),
                camera: params.camera.extend(params.near).to_array(),
                sun: params
                    .sun_dir
                    .normalize_or(Vec3::Y)
                    .extend(params.exposure)
                    .to_array(),
                sun_radiance: params.sun_radiance.extend(params.sky_scale).to_array(),
                wind: params.wind.extend(now).to_array(),
                now,
                previous,
                focal: params.focal,
                depth: 0,
                width: extent.width as f32,
                height: extent.height as f32,
                base,
                live,
                fresh,
                block_count: gpu.len() as u32,
                capacity: PARTICLE_CAPACITY,
                pad: 0,
                flip: self.flipbook.as_ref().map_or([0; 4], |(_, [p, n])| {
                    [
                        *p,
                        *n,
                        crate::smoke_bake::FRAMES as u32,
                        crate::smoke_bake::COLUMNS as u32,
                    ]
                }),
                tlas: params.tlas,
                sky: sky.address(),
                ring: self.ring.address(),
                blocks: blocks_buffer.address(),
                effects: self.table.address(),
                probes: params.probes.map_or(0, |p| p.address),
            }],
        );
        let address = frame_buffer.address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let ring_handle = graph.import_buffer(&self.ring);
        if fresh > 0 {
            let emit = &self.emit;
            graph
                .pass("particles/emit")
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
            .pass("particles/advance")
            .queue(QueueKind::Compute)
            .buffer(ring_handle, BufferAccess::ShaderReadWrite(compute))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(compute));
        if let Some(p) = params.probes {
            pass = pass
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .buffer(p.region, BufferAccess::ShaderRead(compute))
                .image(p.irradiance, ImageAccess::Sampled(compute))
                .image(p.distance, ImageAccess::Sampled(compute));
        }
        pass.run(move |_, commands| {
            commands.bind_pipeline(advance);
            commands.push_constants(advance, &address);
            commands.dispatch(live.div_ceil(64), 1, 1);
            Ok(())
        });
        let (mask, load) = match reactive {
            Some(mask) => (mask, vk::AttachmentLoadOp::LOAD),
            None => (
                graph.transient(TransientDesc {
                    name: "particle reactive mask",
                    width: extent.width,
                    height: extent.height,
                    format: REACTIVE_FORMAT,
                    usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                }),
                vk::AttachmentLoadOp::CLEAR,
            ),
        };
        let vertex = vk::PipelineStageFlags2::VERTEX_SHADER;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let draw = &self.draw;
        graph
            .pass("particles/draw")
            .buffer(ring_handle, BufferAccess::ShaderRead(vertex))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(vertex))
            .image(sky.aerial(), ImageAccess::Sampled(vertex))
            .image(depth, ImageAccess::Sampled(fragment))
            .image(color, ImageAccess::ColorAttachment)
            .image(mask, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                frame_buffer.write(
                    std::mem::offset_of!(GpuFrame, depth) as u64,
                    &[resources.sampled(depth).0],
                );
                let attachments = [
                    vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(color))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(vk::AttachmentLoadOp::LOAD)
                        .store_op(vk::AttachmentStoreOp::STORE),
                    vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(mask))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(load)
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
        Some(mask)
    }
}

/// A block of `effect`'s particles from `position` along `direction`, the first being its
/// emitter's `index`-th, born at `birth` and every `interval` seconds after (0: all at once).
#[allow(clippy::too_many_arguments)]
fn block_of(
    effect: &Effect,
    effect_index: usize,
    position: Vec3,
    direction: Vec3,
    carry: Vec3,
    seed: u32,
    index: u32,
    count: u32,
    birth: f32,
    interval: f32,
) -> GpuBlock {
    GpuBlock {
        origin: position.extend(effect.spread).to_array(),
        axis: direction
            .normalize_or(Vec3::Y)
            .extend(effect.cone.to_radians())
            .to_array(),
        speed: [
            effect.speed[0],
            effect.speed[1],
            effect.life[0],
            effect.life[1],
        ],
        carry: carry.extend(0.0).to_array(),
        birth,
        interval,
        first: 0,
        count,
        seed,
        effect: effect_index as u32,
        start: 0,
        index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_effects_parse() {
        let effects = effects();
        let names: Vec<&str> = effects.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["chimney-smoke", "footstep-dust", "embers", "sparks"]
        );
        assert!(effects[1].burst);
        assert_eq!(effects[2].draw, DrawKind::Glow);
        // The sparks meet the scene and streak: a glow (1), a bounce of 0.35 (89 of 255), both
        // flags.
        let sparks = GpuEffect::of(&effects[3]);
        assert_eq!(sparks.draw, 1 | (89 << 16) | (1 << 24) | (1 << 25));
        assert_eq!(GpuEffect::of(&effects[2]).draw, 1);
    }

    #[test]
    fn a_bad_effect_is_refused_by_name() {
        let text = "[puff]\nrate = 0.0\nlife = [1.0, 2.0]\nradius = [0.1, 0.2]\nspeed = [0.1, 0.2]\n\
                    cone = 10.0\nspread = 0.1\ngravity = 0.0\ndrag = 1.0\nwind = 1.0\ncurl = [0.0, 1.0]\n\
                    colour = [[1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]]\ndraw = \"smoke\"\n";
        assert!(parse(text).unwrap_err().starts_with("puff:"));
        assert!(parse(&text.replace("rate = 0.0", "rate = 1.0")).is_ok());
        assert!(parse(&text.replace("draw = \"smoke\"", "draw = \"fire\"")).is_err());
        assert!(parse(&format!("{text}typo = 1\n")).is_err());
    }

    #[test]
    fn a_stream_is_born_at_the_same_times_whatever_the_frames() {
        // Twelve a second from t = 1: the k-th at 1 + (k + ½) / 12.
        let rate = 12.0;
        let whole = births(1.0, rate, 1.0, 3.0);
        assert_eq!(whole, (0, 24));
        // The same two seconds in frames of a 60th and of a 7th: the same particles, once each.
        for step in [1.0 / 60.0, 1.0 / 7.0] {
            let mut next = 0;
            let mut t = 1.0_f32;
            while t < 3.0 - 1e-4 {
                let now = (t + step).min(3.0);
                let (first, count) = births(1.0, rate, t, now);
                if count > 0 {
                    assert_eq!(first, next, "step {step}");
                    next = first + count;
                }
                t = now;
            }
            assert_eq!(next, 24, "step {step}");
        }
        // Before its start, nothing.
        assert_eq!(births(5.0, rate, 1.0, 3.0).1, 0);
    }
}
