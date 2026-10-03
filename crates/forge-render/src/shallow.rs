//! The GPU's shallow-water layer (Phase 3's step 8, D-044 item 5; `shaders/shallow.slang`): a
//! grid of water columns `ratio` times finer than the authoritative column model
//! (`forge_physics::shallow::Pool`, on the CPU), over the same ground. It runs the column model's
//! scheme on the GPU and shadows it: it starts from the columns, and once a frame each block of
//! `ratio × ratio` fine cells is pulled towards its column's depth, each fine face towards the
//! columns' velocity there. So it adds detail between the columns (a sharper front, the water
//! round a corner) and never drifts from the water the game owns. Nothing reads it back:
//! buoyancy, the network and the saves stay on the columns. The water pass draws its samples
//! ([`crate::WaterSurface::set_pool_on_gpu`]).
//!
//! Every pass reads one buffer and writes another (or only its own elements), with no atomics:
//! the same bits every run on one GPU.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

/// The column model's state for a frame, as [`ShallowLayer::simulate`] takes it.
#[derive(Clone, Copy, Debug)]
pub struct ShallowColumns<'a> {
    /// Each column's depth, metres (row-major, rows along +z).
    pub depth: &'a [f32],
    /// The velocities along x on the faces between columns, `(nx + 1) × nz`, m/s.
    pub u: &'a [f32],
    /// Along z on the faces between rows, `nx × (nz + 1)`.
    pub w: &'a [f32],
    /// What floats in each column's water, as a thickness, metres.
    pub floats: &'a [f32],
    /// Each column's bed, metres.
    pub bed: &'a [f32],
}

/// The layer's ground: the column model's grid and how much finer the layer is.
#[derive(Clone, Copy, Debug)]
pub struct ShallowGrid {
    /// Columns along x and z.
    pub columns: [u32; 2],
    /// Metres between the columns' centres.
    pub spacing: f32,
    /// World x and z of the first column's centre (the water pass's frame).
    pub origin: [f32; 2],
    /// Fine cells along a column's side.
    pub ratio: u32,
}

impl ShallowGrid {
    /// Fine cells along x and z.
    pub fn cells(&self) -> [u32; 2] {
        self.columns.map(|c| c * self.ratio)
    }

    /// Metres between fine cells.
    pub fn fine_spacing(&self) -> f32 {
        self.spacing / self.ratio as f32
    }

    /// World x and z of the first fine cell's centre.
    pub fn fine_origin(&self) -> [f32; 2] {
        let h = self.fine_spacing();
        self.origin.map(|o| o - 0.5 * self.spacing + 0.5 * h)
    }

    fn column_floats(&self) -> usize {
        let [cx, cz] = self.columns.map(|c| c as usize);
        // Depths, u faces, w faces, floats, bed.
        cx * cz + (cx + 1) * cz + cx * (cz + 1) + cx * cz + cx * cz
    }
}

/// How the layer steps and follows the columns.
#[derive(Clone, Copy, Debug)]
pub struct ShallowParams {
    /// Steps of the layer for each of the column model's ticks (the finer grid needs shorter
    /// steps to stay stable).
    pub substeps: u32,
    /// The share of its gap to the columns each fine cell and face closes once a frame.
    pub rate: f32,
    /// The column model's friction (velocity lost a second) and gravity (m/s², downwards).
    pub friction: f32,
    /// m/s², downwards.
    pub gravity: f32,
}

impl Default for ShallowParams {
    fn default() -> Self {
        Self {
            substeps: 2,
            rate: 0.25,
            friction: 0.3,
            gravity: 9.81,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShallowPush {
    depth_in: u64,
    depth_out: u64,
    u_in: u64,
    w_in: u64,
    u_out: u64,
    w_out: u64,
    give: u64,
    columns: u64,
    samples: u64,
    nx: u32,
    nz: u32,
    ratio: u32,
    spacing: f32,
    dt: f32,
    gravity: f32,
    friction: f32,
    rate: f32,
}

/// The passes, in `shaders/shallow.slang`.
#[derive(Clone, Copy)]
enum Kernel {
    Advect,
    Give,
    Apply,
    Front,
    ShadowMeans,
    Shadow,
    Samples,
}

const KERNELS: [(&str, &str); 7] = [
    ("advect_main", "shallow/advect"),
    ("give_main", "shallow/give"),
    ("apply_main", "shallow/apply"),
    ("front_main", "shallow/front"),
    ("shadow_means_main", "shallow/shadow means"),
    ("shadow_main", "shallow/shadow"),
    ("samples_main", "shallow/samples"),
];

/// Where each array lives in the layer's buffer, bytes from its start.
#[derive(Clone, Copy, Debug)]
struct Layout {
    depth: [u64; 2],
    u: [u64; 2],
    w: [u64; 2],
    give: u64,
    samples: u64,
    total: u64,
}

impl Layout {
    fn new(cells: [u32; 2]) -> Self {
        let [nx, nz] = cells.map(u64::from);
        let mut at = 0;
        let mut take = |bytes: u64| {
            let start = at;
            at += bytes.div_ceil(256) * 256;
            start
        };
        let depth = [take(nx * nz * 4), take(nx * nz * 4)];
        let u = [take((nx + 1) * nz * 4), take((nx + 1) * nz * 4)];
        let w = [take(nx * (nz + 1) * 4), take(nx * (nz + 1) * 4)];
        let give = take(nx * nz * 4);
        let samples = take(nx * nz * 16);
        Self {
            depth,
            u,
            w,
            give,
            samples,
            total: at,
        }
    }
}

/// What the layer held before a frame's pull towards the columns ([`ShallowLayer::take_stats`]).
#[derive(Clone, Copy, Debug)]
pub struct ShallowStats {
    /// The layer's water, m³.
    pub volume: f64,
    /// The columns', m³.
    pub columns: f64,
    /// How far a wet column's fine cells stood from it on average, metres of depth.
    pub mean_gap: f32,
    /// The most any column's did, metres.
    pub largest_gap: f32,
}

/// This frame's layer in the graph, from [`ShallowLayer::import`]: imported once a frame and
/// handed to [`ShallowLayer::simulate`] and to the water pass.
#[derive(Clone, Copy, Debug)]
pub struct ShallowState {
    /// The layer's buffer.
    pub buffer: BufferHandle,
    /// The device address of its samples, `[surface, depth, velocity x, velocity z]` per fine
    /// cell, row-major.
    pub samples: u64,
}

/// The GPU's shallow-water layer over a column model (see the module documentation).
pub struct ShallowLayer {
    pipelines: Vec<Pipeline>,
    state: GraphBuffer,
    layout: Layout,
    columns: Vec<Buffer>,
    grid: ShallowGrid,
    /// How it steps and follows the columns.
    pub params: ShallowParams,
    /// Whether the layer holds water yet: the first frame starts it from the columns.
    started: Cell<bool>,
    /// Which of the two sets holds the depths, and which the faces.
    current: Cell<[usize; 2]>,
    readback: Vec<GraphBuffer>,
    /// Per slot, the columns' depths when its frame asked for statistics.
    asked: [Cell<Option<Vec<f32>>>; FRAMES_IN_FLIGHT],
}

impl ShallowLayer {
    /// Compiles the passes and makes the layer's buffers over `grid`.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, grid: ShallowGrid) -> Result<Self> {
        assert!(
            grid.columns.iter().all(|&c| c >= 2) && grid.ratio >= 1,
            "a layer over two columns a side at least"
        );
        let pipelines = KERNELS
            .iter()
            .map(|&(entry, name)| {
                let module = device.create_shader_module(
                    &shaders.compile("shallow.slang", entry, ShaderStage::Compute)?,
                    name,
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, entry),
                    push_constant_bytes: std::mem::size_of::<ShallowPush>() as u32,
                    name,
                });
                device.destroy_shader_module(module);
                pipeline
            })
            .collect::<Result<Vec<_>>>()?;
        let layout = Layout::new(grid.cells());
        let state = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: layout.total,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "shallow layer",
        })?);
        let columns = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (grid.column_floats() * 4) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("shallow columns {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: u64::from(grid.columns[0] * grid.columns[1]) * 4,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("shallow statistics readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipelines,
            state,
            layout,
            columns,
            grid,
            params: ShallowParams::default(),
            started: Cell::new(false),
            current: Cell::new([0, 0]),
            readback,
            asked: Default::default(),
        })
    }

    /// The layer's grid.
    pub fn grid(&self) -> &ShallowGrid {
        &self.grid
    }

    /// Bytes of the layer's buffer.
    pub fn bytes(&self) -> u64 {
        self.state.size()
    }

    /// Starts the layer again from the columns at the next [`ShallowLayer::simulate`].
    pub fn reset(&self) {
        self.started.set(false);
    }

    /// The layer's buffer in this frame's graph (once a frame: two imports are two buffers to
    /// the graph, with nothing ordering them).
    pub fn import<'f>(&'f self, graph: &mut FrameGraph<'f>) -> ShallowState {
        ShallowState {
            buffer: graph.import_buffer(&self.state),
            samples: self.state.address() + self.layout.samples,
        }
    }

    /// Declares the layer's passes for this frame: `ticks` of the column model's, `dt` seconds
    /// each, then the pull towards `columns` (the state the ticks ended in) and the samples the
    /// water pass draws; with `stats`, what [`ShallowLayer::take_stats`] returns once the frame is
    /// done.
    #[allow(clippy::too_many_arguments)]
    pub fn simulate<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        state: ShallowState,
        slot: FrameSlot,
        columns: ShallowColumns<'_>,
        ticks: u32,
        dt: f32,
        stats: bool,
    ) {
        let grid = self.grid;
        let [cx, cz] = grid.columns.map(|c| c as usize);
        assert!(
            columns.depth.len() == cx * cz
                && columns.u.len() == (cx + 1) * cz
                && columns.w.len() == cx * (cz + 1)
                && columns.floats.len() == cx * cz
                && columns.bed.len() == cx * cz,
            "the columns' arrays fit the layer's grid"
        );
        let buffer = &self.columns[slot.index];
        let mut at = 0;
        for part in [
            columns.depth,
            columns.u,
            columns.w,
            columns.floats,
            columns.bed,
        ] {
            buffer.write(at, part);
            at += (part.len() * 4) as u64;
        }
        let [nx, nz] = grid.cells();
        let base = self.state.address();
        let l = self.layout;
        let params = self.params;
        let columns_address = buffer.address();
        // Which set holds the depths (`d`) and which the faces (`f`): a half step writes the
        // depths into the other set and leaves the faces where they were; the pull towards the
        // columns flips both.
        let push = |d: [usize; 2], f: [usize; 2], dt: f32, rate: f32| ShallowPush {
            depth_in: base + l.depth[d[0]],
            depth_out: base + l.depth[d[1]],
            u_in: base + l.u[f[0]],
            w_in: base + l.w[f[0]],
            u_out: base + l.u[f[1]],
            w_out: base + l.w[f[1]],
            give: base + l.give,
            columns: columns_address,
            samples: base + l.samples,
            nx,
            nz,
            ratio: grid.ratio,
            spacing: grid.fine_spacing(),
            dt,
            gravity: params.gravity,
            friction: params.friction,
            rate,
        };
        let faces = [nx + 1, nz + 1];
        let cells = [nx, nz];
        let [mut d, mut f] = self.current.get();
        // The first frame starts the layer from the columns.
        if !self.started.replace(true) {
            let p = push([d, 1 - d], [f, 1 - f], 0.0, 1.0);
            self.pass(graph, state, Kernel::ShadowMeans, grid.columns, p);
            self.pass(graph, state, Kernel::Shadow, faces, p);
            [d, f] = [1 - d, 1 - f];
        }
        let substeps = params.substeps.max(1);
        let half = 0.5 * dt / substeps as f32;
        for _ in 0..ticks * substeps * 2 {
            // The faces advected into the other set; the shares and the depths from the flows
            // they carry; the front and the slopes from the advected faces back into this set's.
            self.pass(
                graph,
                state,
                Kernel::Advect,
                faces,
                push([d, 1 - d], [f, 1 - f], half, 0.0),
            );
            let p = push([d, 1 - d], [1 - f, f], half, 0.0);
            self.pass(graph, state, Kernel::Give, cells, p);
            self.pass(graph, state, Kernel::Apply, cells, p);
            self.pass(graph, state, Kernel::Front, faces, p);
            d = 1 - d;
        }
        // The pull towards the columns. Each column's fine mean, which it computes first, is
        // what `stats` reads back: how far the layer went from the columns on its own.
        let p = push([d, 1 - d], [f, 1 - f], 0.0, params.rate);
        self.pass(graph, state, Kernel::ShadowMeans, grid.columns, p);
        if stats {
            self.asked[slot.index].set(Some(columns.depth.to_vec()));
            let readback_buffer = &self.readback[slot.index];
            let readback = graph.import_buffer(readback_buffer);
            let (source, at, bytes) = (&*self.state, l.give, (cx * cz * 4) as u64);
            graph
                .pass("shallow/stats")
                .buffer(state.buffer, BufferAccess::TransferSrc)
                .buffer(readback, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    commands.copy_buffer_regions(source, readback_buffer, &[(at, 0, bytes)]);
                    Ok(())
                });
            graph
                .pass("shallow/stats")
                .buffer(readback, BufferAccess::HostRead)
                .run(|_, _| Ok(()));
        }
        self.pass(graph, state, Kernel::Shadow, faces, p);
        [d, f] = [1 - d, 1 - f];
        self.pass(
            graph,
            state,
            Kernel::Samples,
            cells,
            push([d, 1 - d], [f, 1 - f], 0.0, 0.0),
        );
        self.current.set([d, f]);
    }

    /// What the layer held before the pull of the frame that last used `slot`, when that frame
    /// asked for it (its commands have completed).
    pub fn take_stats(&self, slot: FrameSlot) -> Option<ShallowStats> {
        let columns = self.asked[slot.index].take()?;
        let [cx, cz] = self.grid.columns.map(|c| c as usize);
        let mut means = vec![0.0_f32; cx * cz];
        self.readback[slot.index].read(0, &mut means);
        let area = f64::from(self.grid.spacing) * f64::from(self.grid.spacing);
        let sum = |values: &[f32]| values.iter().map(|&v| f64::from(v)).sum::<f64>();
        let wet = columns.iter().filter(|&&c| c >= 0.002).count().max(1);
        let gap = means
            .iter()
            .zip(&columns)
            .map(|(&m, &c)| f64::from((m - c).abs()))
            .sum::<f64>();
        Some(ShallowStats {
            volume: sum(&means) * area,
            columns: sum(&columns) * area,
            mean_gap: (gap / wet as f64) as f32,
            largest_gap: means
                .iter()
                .zip(&columns)
                .map(|(&m, &c)| (m - c).abs())
                .fold(0.0, f32::max),
        })
    }

    fn pass<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        state: ShallowState,
        kernel: Kernel,
        threads: [u32; 2],
        push: ShallowPush,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = &self.pipelines[kernel as usize];
        graph
            .pass(KERNELS[kernel as usize].1)
            .buffer(state.buffer, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push);
                commands.dispatch(threads[0].div_ceil(8), threads[1].div_ceil(8), 1);
                Ok(())
            });
    }
}
