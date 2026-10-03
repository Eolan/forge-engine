//! The lab's liquid (#156, D-044): water in a glass tank as particles on a grid, simulated and
//! drawn on the GPU (`shaders/liquid.slang`, `shaders/liquid_draw.slang`;
//! `docs/research/particle-fluids.md`).
//!
//! The particles carry the water (APIC, Jiang et al. 2015); a MAC grid over the tank's inside
//! keeps it incompressible (a pressure projection, warm-started) and holds its volume (the
//! particles' crowding undone a share a substep). The grid's sums are fixed-point atomics, so a
//! run replays to the same bits on one GPU. Visual and lab-only (the owner's answer 2): nothing
//! the game keeps depends on it.
//!
//! Every frame, on the async compute queue, `liquid/simulate`: the substeps the lab's ticks owe
//! ([`LiquidStep`], each with where the gate stands), and when asked the statistics the log
//! reads ([`LiquidStats`]). Then on the graphics queue, `liquid/draw`: the density smoothed, the
//! scene copied, and the tank ray-marched over it (glass, refraction, absorption), with the
//! reactive mask TAA reads where the water moves.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    QueueKind, Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::{Mat4, UVec3, Vec2, Vec3};

use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// Substeps a frame may run (two of the lab's ticks).
pub const LIQUID_MAX_SUBSTEPS: usize = 8;
/// Particles a full cell starts with: 2 × 2 × 2.
const PER_CELL: u32 = 8;
/// Cells a side of the bricks the march skips over (`BRICK` in `liquid_draw.slang`).
const BRICK: u32 = 4;
/// Words of the statistics (`STAT_*` in `liquid.slang`).
const STAT_WORDS: usize = 16;
const STAT_BYTES: u64 = (STAT_WORDS * 4) as u64;

/// A glass tank of water: its inside, its grid, the water at the start and the gate.
#[derive(Clone, Copy, Debug)]
pub struct LiquidTank {
    /// The inside's size, metres (x along the tank, y up).
    pub size: Vec3,
    /// A cell's side, metres.
    pub cell: f32,
    /// The water at the start: a block of cells from the inside's corner.
    pub water: UVec3,
    /// The gate's faces along x, metres from the inside's corner: a slab across the tank.
    pub gate: Option<[f32; 2]>,
    /// A round hole through the gate, plugged until a step says otherwise.
    pub hole: Option<LiquidHole>,
    /// The glass's thickness, metres.
    pub glass: f32,
}

impl LiquidTank {
    /// Cells along x, y and z.
    pub fn cells(&self) -> UVec3 {
        (self.size / self.cell).round().as_uvec3()
    }

    /// The particles it starts with.
    pub fn particles(&self) -> u32 {
        self.water.x * self.water.y * self.water.z * PER_CELL
    }
}

/// A round hole through the gate.
#[derive(Clone, Copy, Debug)]
pub struct LiquidHole {
    /// Its centre's height and depth (y, z), metres from the inside's corner.
    pub centre: Vec2,
    /// Its radius, metres.
    pub radius: f32,
}

/// One substep's gate.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiquidStep {
    /// The gate's bottom over the tank's floor, metres (0 while it is shut).
    pub gate_bottom: f32,
    /// How fast it rises, m/s.
    pub gate_speed: f32,
    /// Whether the hole is still plugged.
    pub plugged: bool,
}

/// How the solver runs: the owner wanted it found by trying (D-044's answer 4).
#[derive(Clone, Copy, Debug)]
pub struct LiquidSolver {
    /// A substep, seconds.
    pub dt: f32,
    /// Red-black Gauss–Seidel sweeps of the pressure a substep (a dispatch a colour).
    pub sweeps: u32,
    /// Their over-relaxation.
    pub omega: f32,
    /// The share of the particles' crowding undone a substep.
    pub drift: f32,
    /// Gravity, m/s².
    pub gravity: Vec3,
}

impl Default for LiquidSolver {
    fn default() -> Self {
        Self {
            dt: 1.0 / 240.0,
            sweeps: 32,
            omega: 1.7,
            drift: 0.25,
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

/// How the liquid looks: physically pale water by default (the owner's answer 5); a stylised
/// liquid is other values.
#[derive(Clone, Copy, Debug)]
pub struct LiquidLook {
    /// Absorption, 1/m, per channel.
    pub absorption: Vec3,
    /// Scattering, 1/m, per channel.
    pub scattering: Vec3,
    /// The index of refraction.
    pub ior: f32,
    /// What one pane of the tank's glass lets through, per channel; `None`: no glass to see (the
    /// bench's, whose walls hold the water and show nothing).
    pub glass: Option<Vec3>,
}

impl LiquidLook {
    /// Pure water (Pope & Fry 1997's absorption at 650, 550 and 450 nm; the sea's scattering,
    /// `water.rs`, a tenth: clear water) behind clear float glass.
    pub fn pure_water() -> Self {
        Self {
            absorption: Vec3::new(0.35, 0.064, 0.015),
            scattering: Vec3::new(0.0003, 0.0013, 0.0016),
            ior: 1.333,
            glass: Some(Vec3::new(0.90, 0.93, 0.91)),
        }
    }

    /// Water tinted to tune by (the bench, after Sebastian Lague's renders): a deep teal at the
    /// tank's depths (absorption some thirty times pure water's in red and green) and a little
    /// cloudy, so its depth and its motion show; no glass.
    pub fn tinted() -> Self {
        Self {
            absorption: Vec3::new(14.0, 4.5, 2.2),
            scattering: Vec3::new(0.5, 1.6, 2.2),
            ior: 1.333,
            glass: None,
        }
    }
}

/// What the log reads of the liquid.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiquidStats {
    /// Particles inside the tank.
    pub particles: u32,
    /// Particles lost: outside it, or not numbers.
    pub lost: u32,
    /// Their mean height, metres.
    pub mean_height: f32,
    /// The farthest one near the floor has gone along x, metres.
    pub front: f32,
    /// Their mean speed², m²/s².
    pub mean_speed2: f32,
    /// The fastest, m/s.
    pub max_speed: f32,
    /// A digest of every particle's position and velocity, to the bit.
    pub digest: [u32; 2],
    /// Cells holding a particle.
    pub cells: u32,
    /// The surface's mean height over the columns that hold water, metres.
    pub level: f32,
    /// Columns that hold water.
    pub columns: u32,
    /// The water's volume (each cell's density up to full), m³.
    pub volume: f32,
}

/// What the drawing needs.
#[derive(Clone, Copy, Debug)]
pub struct LiquidDrawParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The tank's inside corner relative to the camera, metres.
    pub corner: Vec3,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sun's pre-exposed illuminance on a surface facing it, per channel.
    pub sun_radiance: Vec3,
    /// The pre-exposed luminance of a unit of sun illuminance (the sky's scale).
    pub sky_scale: f32,
    /// A plain background (pre-exposed radiance) where the depth is empty, also what the water
    /// mirrors and is lit by besides the sun; `None`: the sky.
    pub background: Option<Vec3>,
}

/// Mirrors `LiquidFrame` in `liquid.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLiquidFrame {
    size: [f32; 4],
    cells: [u32; 4],
    block: [u32; 4],
    gate: [f32; 4],
    hole: [f32; 4],
    steps: [[f32; 4]; LIQUID_MAX_SUBSTEPS],
    gravity: [f32; 4],
    dt: f32,
    rest: f32,
    drift: f32,
    omega: f32,
    nodes: u32,
    cell_count: u32,
    pad: [u32; 2],
    position: u64,
    velocity: u64,
    affine: u64,
    sums: u64,
    face: u64,
    count: u64,
    density: u64,
    kind: u64,
    pressure: u64,
    divergence: u64,
    crowding: u64,
    stats: u64,
}

const _: () = assert!(std::mem::size_of::<GpuLiquidFrame>() == 352);

/// Mirrors `LiquidView` in `liquid_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLiquidView {
    inv_view_proj: [f32; 16],
    view_proj: [f32; 16],
    tank: [f32; 4],
    size: [f32; 4],
    cells: [u32; 4],
    bricks: [u32; 4],
    sun: [f32; 4],
    sun_radiance: [f32; 4],
    absorption: [f32; 4],
    scattering: [f32; 4],
    glass: [f32; 4],
    background: [f32; 4],
    rest: f32,
    nodes: u32,
    color: u32,
    scene: u32,
    depth: u32,
    reactive: u32,
    width: u32,
    height: u32,
    density: u64,
    blur: u64,
    render: u64,
    brick_range: u64,
    face: u64,
    sky_light: u64,
}

const _: () = assert!(std::mem::size_of::<GpuLiquidView>() == 368);

/// Mirrors `LiquidPush` in `liquid.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SolverPush {
    frame: u64,
    substep: u32,
    colour: u32,
}

/// Mirrors `ClearPush` in `liquid_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImagePush {
    a: u32,
    b: u32,
    width: u32,
    height: u32,
}

/// Mirrors `CopyPush` in `liquid_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CopyPush {
    color: u32,
    copy: u32,
    width: u32,
    height: u32,
    depth: u32,
    plain: u32,
    pad: [u32; 2],
    background: [f32; 4],
}

/// Where each array lives in the one buffer that holds them all, bytes.
#[derive(Clone, Copy, Debug)]
struct Layout {
    position: u64,
    velocity: u64,
    affine: u64,
    sums: u64,
    count: u64,
    density: u64,
    face: u64,
    kind: u64,
    pressure: u64,
    divergence: u64,
    crowding: u64,
    blur: u64,
    render: u64,
    bricks: u64,
    stats: u64,
    total: u64,
}

impl Layout {
    fn new(particles: u64, nodes: u64, cells: u64, bricks: u64) -> Self {
        let mut at = 0_u64;
        let mut take = |bytes: u64| {
            let start = at;
            at = (at + bytes).next_multiple_of(256);
            start
        };
        // The sums a substep clears, side by side: the faces' (weight and momentum), count, density.
        let position = take(particles * 16);
        let velocity = take(particles * 16);
        let affine = take(particles * 48);
        let sums = take(nodes * 24);
        let count = take(cells * 4);
        let density = take(cells * 4);
        let face = take(nodes * 12);
        let kind = take(cells * 4);
        let pressure = take(cells * 4);
        let divergence = take(cells * 4);
        let crowding = take(cells * 4);
        let blur = take(cells * 4);
        let render = take(cells * 4);
        let bricks = take(bricks * 8);
        let stats = take(STAT_BYTES);
        Self {
            position,
            velocity,
            affine,
            sums,
            count,
            density,
            face,
            kind,
            pressure,
            divergence,
            crowding,
            blur,
            render,
            bricks,
            stats,
            total: at,
        }
    }
}

/// Bricks along x, y and z over `cells`.
fn bricks_of(cells: UVec3) -> UVec3 {
    UVec3::new(
        cells.x.div_ceil(BRICK),
        cells.y.div_ceil(BRICK),
        cells.z.div_ceil(BRICK),
    )
}

/// Indices of the passes' pipelines in [`Liquid::pipelines`].
#[derive(Clone, Copy)]
enum Kernel {
    Seed,
    SeedCells,
    P2g,
    Faces,
    Cells,
    Pressure,
    Project,
    G2p,
    Stats,
    Level,
    Smooth,
    Smooth2,
    Bricks,
    Copy,
    March,
    Clear,
}

impl Kernel {
    /// Its profiler zone.
    fn label(self) -> &'static str {
        match self {
            Kernel::Seed | Kernel::SeedCells => "liquid/seed",
            Kernel::P2g => "liquid/p2g",
            Kernel::Faces => "liquid/faces",
            Kernel::Cells => "liquid/cells",
            Kernel::Pressure => "liquid/pressure",
            Kernel::Project => "liquid/project",
            Kernel::G2p => "liquid/g2p",
            Kernel::Stats | Kernel::Level => "liquid/stats",
            Kernel::Smooth
            | Kernel::Smooth2
            | Kernel::Bricks
            | Kernel::Copy
            | Kernel::March
            | Kernel::Clear => "liquid/draw",
        }
    }
}

const KERNELS: [(&str, &str, u32); 16] = [
    ("liquid.slang", "seed_main", 16),
    ("liquid.slang", "seed_cells_main", 16),
    ("liquid.slang", "p2g_main", 16),
    ("liquid.slang", "faces_main", 16),
    ("liquid.slang", "cells_main", 16),
    ("liquid.slang", "pressure_main", 16),
    ("liquid.slang", "project_main", 16),
    ("liquid.slang", "g2p_main", 16),
    ("liquid.slang", "stats_main", 16),
    ("liquid.slang", "level_main", 16),
    ("liquid_draw.slang", "smooth_main", 8),
    ("liquid_draw.slang", "smooth2_main", 8),
    ("liquid_draw.slang", "bricks_main", 8),
    ("liquid_draw.slang", "copy_main", 48),
    ("liquid_draw.slang", "march_main", 8),
    ("liquid_draw.slang", "clear_main", 16),
];

/// The tank's liquid: its particles and grid, its passes (#156).
pub struct Liquid {
    pipelines: Vec<Pipeline>,
    state: GraphBuffer,
    layout: Layout,
    tank: LiquidTank,
    solver: LiquidSolver,
    look: LiquidLook,
    /// Per frame slot: the solver's block, the drawing's, the statistics' readback.
    frames: Vec<Buffer>,
    views: Vec<Buffer>,
    readback: Vec<GraphBuffer>,
    /// Whether the particles have been seeded, and per slot whether its readback holds a
    /// frame's statistics.
    seeded: Cell<bool>,
    asked: [Cell<bool>; FRAMES_IN_FLIGHT],
}

impl Liquid {
    /// Compiles the passes and makes the tank's buffers; the water is seeded by the first
    /// [`Liquid::simulate`].
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        tank: LiquidTank,
        solver: LiquidSolver,
        look: LiquidLook,
    ) -> Result<Self> {
        let pipelines = KERNELS
            .iter()
            .map(|&(file, entry, push)| {
                let module = device.create_shader_module(
                    &shaders.compile(file, entry, ShaderStage::Compute)?,
                    entry,
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, entry),
                    push_constant_bytes: push,
                    name: entry,
                });
                device.destroy_shader_module(module);
                pipeline
            })
            .collect::<Result<Vec<_>>>()?;
        let cells = tank.cells();
        let nodes = u64::from((cells.x + 1) * (cells.y + 1) * (cells.z + 1));
        let bricks = bricks_of(cells);
        let layout = Layout::new(
            u64::from(tank.particles()),
            nodes,
            u64::from(cells.x * cells.y * cells.z),
            u64::from(bricks.x * bricks.y * bricks.z),
        );
        let state = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: layout.total,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "liquid",
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
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: STAT_BYTES,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("liquid statistics readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipelines,
            state,
            layout,
            tank,
            solver,
            look,
            frames: per_slot(std::mem::size_of::<GpuLiquidFrame>(), "liquid frame")?,
            views: per_slot(std::mem::size_of::<GpuLiquidView>(), "liquid view")?,
            readback,
            seeded: Cell::new(false),
            asked: std::array::from_fn(|_| Cell::new(false)),
        })
    }

    /// Bytes of the particles and the grid.
    pub fn bytes(&self) -> u64 {
        self.state.size()
    }

    /// The tank.
    pub fn tank(&self) -> &LiquidTank {
        &self.tank
    }

    /// The solver's settings.
    pub fn solver(&self) -> &LiquidSolver {
        &self.solver
    }

    /// Starts the water over from its block at the next [`Liquid::simulate`].
    pub fn reset(&self) {
        self.seeded.set(false);
    }

    /// The statistics the frame that last used `slot` gathered, once its commands have
    /// completed; `None` if it gathered none.
    pub fn take_stats(&self, slot: FrameSlot) -> Option<LiquidStats> {
        if !self.asked[slot.index].replace(false) {
            return None;
        }
        let mut w = [0_u32; STAT_WORDS];
        self.readback[slot.index].read(0, &mut w);
        let particles = w[0];
        let n = particles.max(1) as f32;
        let cell = self.tank.cell;
        Some(LiquidStats {
            particles,
            lost: w[1],
            mean_height: w[2] as f32 / 1000.0 / n,
            front: w[3] as f32 / 10_000.0,
            mean_speed2: w[4] as f32 / 100.0 / n,
            digest: [w[5], w[6]],
            cells: w[7],
            level: if w[9] > 0 {
                w[8] as f32 / 10_000.0 / w[9] as f32
            } else {
                0.0
            },
            columns: w[9],
            volume: w[10] as f32 / 1000.0 * cell * cell * cell,
            max_speed: w[11] as f32 / 1000.0,
        })
    }

    fn pipeline(&self, kernel: Kernel) -> &Pipeline {
        &self.pipelines[kernel as usize]
    }

    /// Declares `liquid/simulate` on the async compute queue: the water seeded if it is new,
    /// then a substep for each of `steps` (at most [`LIQUID_MAX_SUBSTEPS`]), then with `stats`
    /// the statistics [`Liquid::take_stats`] returns once the frame is done.
    pub fn simulate<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        steps: &[LiquidStep],
        stats: bool,
    ) {
        let steps = &steps[..steps.len().min(LIQUID_MAX_SUBSTEPS)];
        let seed = !self.seeded.replace(true);
        if steps.is_empty() && !seed && !stats {
            return;
        }
        let tank = &self.tank;
        let cells = tank.cells();
        let nodes = (cells.x + 1) * (cells.y + 1) * (cells.z + 1);
        let cell_count = cells.x * cells.y * cells.z;
        let particles = tank.particles();
        let base = self.state.address();
        let l = self.layout;
        let mut gpu_steps = [[0.0_f32; 4]; LIQUID_MAX_SUBSTEPS];
        for (g, s) in gpu_steps.iter_mut().zip(steps) {
            *g = [
                s.gate_bottom,
                s.gate_speed,
                if s.plugged { 1.0 } else { 0.0 },
                0.0,
            ];
        }
        let frame = &self.frames[slot.index];
        frame.write(
            0,
            &[GpuLiquidFrame {
                size: tank.size.extend(tank.cell).to_array(),
                cells: [cells.x, cells.y, cells.z, particles],
                block: [tank.water.x, tank.water.y, tank.water.z, 0],
                gate: tank.gate.map_or([0.0; 4], |[a, b]| [a, b, 0.0, 0.0]),
                hole: tank
                    .hole
                    .map_or([0.0; 4], |h| [h.centre.x, h.centre.y, h.radius, 0.0]),
                steps: gpu_steps,
                gravity: self.solver.gravity.extend(0.0).to_array(),
                dt: self.solver.dt,
                rest: PER_CELL as f32,
                drift: self.solver.drift,
                omega: self.solver.omega,
                nodes,
                cell_count,
                pad: [0; 2],
                position: base + l.position,
                velocity: base + l.velocity,
                affine: base + l.affine,
                sums: base + l.sums,
                face: base + l.face,
                count: base + l.count,
                density: base + l.density,
                kind: base + l.kind,
                pressure: base + l.pressure,
                divergence: base + l.divergence,
                crowding: base + l.crowding,
                stats: base + l.stats,
            }],
        );
        let address = frame.address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let state = graph.import_buffer(&self.state);
        let buffer: &'f Buffer = &self.state;
        let dispatch = |graph: &mut FrameGraph<'f>,
                        kernel: Kernel,
                        threads: u32,
                        substep: u32,
                        colour: u32| {
            let pipeline = self.pipeline(kernel);
            graph
                .pass(kernel.label())
                .queue(QueueKind::Compute)
                .buffer(state, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &SolverPush {
                            frame: address,
                            substep,
                            colour,
                        },
                    );
                    commands.dispatch(threads.div_ceil(64), 1, 1);
                    Ok(())
                });
        };
        if seed {
            dispatch(graph, Kernel::Seed, particles, 0, 0);
            dispatch(graph, Kernel::SeedCells, cell_count, 0, 0);
        }
        let half = cells.x.div_ceil(2) * cells.y * cells.z;
        for s in 0..steps.len() as u32 {
            // Clear the sums: the faces', the counts and the densities lie side by side.
            let (from, to) = (l.sums, l.density + u64::from(cell_count) * 4);
            graph
                .pass("liquid/clear")
                .queue(QueueKind::Compute)
                .buffer(state, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    commands.fill_buffer(buffer, from, to - from, 0);
                    Ok(())
                });
            dispatch(graph, Kernel::P2g, particles, s, 0);
            dispatch(graph, Kernel::Faces, nodes, s, 0);
            dispatch(graph, Kernel::Cells, cell_count, s, 0);
            for _ in 0..self.solver.sweeps {
                dispatch(graph, Kernel::Pressure, half, s, 0);
                dispatch(graph, Kernel::Pressure, half, s, 1);
            }
            dispatch(graph, Kernel::Project, nodes, s, 0);
            dispatch(graph, Kernel::G2p, particles, s, 0);
        }
        self.asked[slot.index].set(stats);
        if stats {
            let stats_at = l.stats;
            graph
                .pass("liquid/stats")
                .queue(QueueKind::Compute)
                .buffer(state, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    commands.fill_buffer(buffer, stats_at, STAT_BYTES, 0);
                    Ok(())
                });
            dispatch(graph, Kernel::Stats, particles, 0, 0);
            dispatch(graph, Kernel::Level, cells.x * cells.z, 0, 0);
            let readback_buffer = &self.readback[slot.index];
            let readback = graph.import_buffer(readback_buffer);
            graph
                .pass("liquid/stats")
                .queue(QueueKind::Compute)
                .buffer(state, BufferAccess::TransferSrc)
                .buffer(readback, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    commands.copy_buffer_regions(
                        buffer,
                        readback_buffer,
                        &[(stats_at, 0, STAT_BYTES)],
                    );
                    Ok(())
                });
            graph
                .pass("liquid/stats")
                .queue(QueueKind::Compute)
                .buffer(readback, BufferAccess::HostRead)
                .run(|_, _| Ok(()));
        }
    }

    /// Declares `liquid/draw` over `color` (the frame's HDR image, the scene's sky composed)
    /// against `depth`: the tank's glass and water. Returns the reactive mask TAA reads.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sky: &SkyFrame,
        params: LiquidDrawParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) -> ImageHandle {
        const LABEL: &str = "liquid/draw";
        let tank = &self.tank;
        let cells = tank.cells();
        let bricks = bricks_of(cells);
        let base = self.state.address();
        let l = self.layout;
        let view = &self.views[slot.index];
        let look = &self.look;
        view.write(
            0,
            &[GpuLiquidView {
                inv_view_proj: params.view_proj.inverse().to_cols_array(),
                view_proj: params.view_proj.to_cols_array(),
                tank: params.corner.extend(tank.cell).to_array(),
                size: tank.size.extend(tank.glass).to_array(),
                cells: [cells.x, cells.y, cells.z, 0],
                bricks: [bricks.x, bricks.y, bricks.z, 0],
                // The sun's highlight a little rough, so it does not alias.
                sun: params
                    .sun_dir
                    .normalize_or(Vec3::Y)
                    .extend(0.004)
                    .to_array(),
                sun_radiance: params.sun_radiance.extend(params.sky_scale).to_array(),
                absorption: look.absorption.extend(look.ior).to_array(),
                scattering: look.scattering.extend(0.0).to_array(),
                glass: look
                    .glass
                    .map_or([1.0, 1.0, 1.0, 0.0], |g| g.extend(1.0).to_array()),
                background: params
                    .background
                    .map_or([0.0; 4], |b| b.extend(1.0).to_array()),
                rest: PER_CELL as f32,
                nodes: (cells.x + 1) * (cells.y + 1) * (cells.z + 1),
                color: 0,
                scene: 0,
                depth: 0,
                reactive: 0,
                width: extent.width,
                height: extent.height,
                density: base + l.density,
                blur: base + l.blur,
                render: base + l.render,
                brick_range: base + l.bricks,
                face: base + l.face,
                sky_light: sky.light.address,
            }],
        );
        let address = view.address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let state = graph.import_buffer(&self.state);
        let cell_count = cells.x * cells.y * cells.z;
        let brick_count = bricks.x * bricks.y * bricks.z;
        for (kernel, threads) in [
            (Kernel::Smooth, cell_count),
            (Kernel::Smooth2, cell_count),
            (Kernel::Bricks, brick_count),
        ] {
            let pipeline = self.pipeline(kernel);
            graph
                .pass(LABEL)
                .buffer(state, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(pipeline, &address);
                    commands.dispatch(threads.div_ceil(64), 1, 1);
                    Ok(())
                });
        }
        let image = |name, format| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let scene = graph.transient(image("liquid scene", HDR_FORMAT));
        let reactive = graph.transient(image("liquid reactive mask", vk::Format::R32_SFLOAT));
        let groups = (extent.width.div_ceil(8), extent.height.div_ceil(8));
        let copy = self.pipeline(Kernel::Copy);
        let background = params.background;
        graph
            .pass(LABEL)
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(scene, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(copy);
                commands.push_constants(
                    copy,
                    &CopyPush {
                        color: resources.sampled(color).0,
                        copy: resources.storage(scene, 0).0,
                        width: extent.width,
                        height: extent.height,
                        depth: resources.sampled(depth).0,
                        plain: u32::from(background.is_some()),
                        pad: [0; 2],
                        background: background.unwrap_or(Vec3::ZERO).extend(0.0).to_array(),
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        let clear = self.pipeline(Kernel::Clear);
        graph
            .pass(LABEL)
            .image(reactive, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(clear);
                commands.push_constants(
                    clear,
                    &ImagePush {
                        a: resources.storage(reactive, 0).0,
                        b: 0,
                        width: extent.width,
                        height: extent.height,
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        let march = self.pipeline(Kernel::March);
        graph
            .pass(LABEL)
            .buffer(state, BufferAccess::ShaderRead(compute))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.light.table, ImageAccess::Sampled(compute))
            .image(scene, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageReadWrite(compute))
            .image(reactive, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                let offset = std::mem::offset_of!(GpuLiquidView, color) as u64;
                view.write(
                    offset,
                    &[
                        resources.storage(color, 0).0,
                        resources.sampled(scene).0,
                        resources.sampled(depth).0,
                        resources.storage(reactive, 0).0,
                    ],
                );
                commands.bind_pipeline(march);
                commands.push_constants(march, &address);
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        reactive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_keeps_the_cleared_sums_together_and_aligned() {
        let tank = LiquidTank {
            size: Vec3::new(1.0, 0.6, 0.5),
            cell: 0.01,
            water: UVec3::new(40, 40, 50),
            gate: Some([0.4, 0.42]),
            hole: None,
            glass: 0.01,
        };
        let cells = tank.cells();
        assert_eq!(cells, UVec3::new(100, 60, 50));
        assert_eq!(tank.particles(), 640_000);
        let nodes = u64::from((cells.x + 1) * (cells.y + 1) * (cells.z + 1));
        let count = u64::from(cells.x * cells.y * cells.z);
        let l = Layout::new(u64::from(tank.particles()), nodes, count, 25 * 15 * 13);
        for at in [
            l.position,
            l.velocity,
            l.affine,
            l.sums,
            l.count,
            l.density,
            l.face,
            l.kind,
            l.pressure,
            l.divergence,
            l.crowding,
            l.blur,
            l.render,
            l.bricks,
            l.stats,
        ] {
            assert_eq!(at % 256, 0);
        }
        // One fill clears the faces' sums, the counts and the densities, and nothing else.
        assert!(l.sums < l.count && l.count < l.density);
        assert!(l.density + count * 4 <= l.face);
        assert!(l.sums >= l.affine + u64::from(tank.particles()) * 48);
    }
}
