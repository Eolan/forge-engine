//! The tiles' normal maps made on the GPU (D-058, #220): `shaders/planet_normals.slang` makes a
//! tile's map and its mips in passes of the frame, where the CPU took about 50 ms of a core a
//! tile ([`forge_terrain::planet::tile_normal_map`], kept as the reference and the fallback).
//!
//! - **The worker** makes each new tile's image (its storage views for the passes, its sampled
//!   view for the scene) and queues a job ([`MapQueue`]); the scene it builds or edits samples
//!   the image as it did an uploaded one.
//! - **The frame** takes the jobs queued so far and declares their passes before the scene's,
//!   so a map is made in the frame that first draws its tile.
//! - **The elevation** is on the GPU for the whole run: every level of the planet's grids, one
//!   buffer of `i16` (the Earth's about 0.74 GB with the Alps' region).

use std::sync::{Arc, Mutex};

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, FrameSlot, GraphBuffer,
    GraphImage, ImageAccess, ImageDesc, MemoryCategory, MemoryLocation, Pipeline, ShaderCompiler,
    ShaderStage, vk,
};
use forge_terrain::Planet;
use forge_terrain::planet::{
    COAST_RANGE, FixedHeights, NORMAL_MAP_SIZE, fixed_direction, fixed_warp, tile_normal_map,
};
use forge_world::CellId;

/// Mip levels of a tile's map: 256² down to 1².
const LEVELS: u32 = NORMAL_MAP_SIZE.trailing_zeros() + 1;
/// Samples a side of a map's grid (`2 size + 1`), the tangents a job carries per axis.
const SIDE: usize = 2 * NORMAL_MAP_SIZE as usize + 1;
/// Levels an elevation grid may have on the GPU (the shader's `Grid::levels`).
const MAX_GRID_LEVELS: usize = 16;
/// Grids a planet may have on the GPU: the whole map and its regions.
const MAX_GRIDS: usize = 4;
/// Octaves of detail a grid may have (the shader's `MAX_OCTAVES`).
const MAX_OCTAVES: usize = 16;

/// A map for the frame to make: its image and its tile.
pub(crate) struct MapJob {
    pub image: Arc<GraphImage>,
    pub cell: CellId,
}

/// An ask for a tile's heights: its cell and where the answer goes ([`HeightsQueue`]).
type Ask = (CellId, std::sync::mpsc::Sender<Vec<i64>>);

/// Tiles' heights the worker asks the GPU for (D-058's step 2): each answered a frame slot
/// later, or dropped with its frame's passes (the worker then makes them on the CPU).
#[derive(Default)]
pub(crate) struct HeightsQueue {
    /// The asks, and whether the frames stopped answering ([`Self::close`]).
    asked: Mutex<(Vec<Ask>, bool)>,
}

impl HeightsQueue {
    /// Asks for `cell`'s heights ([`forge_terrain::planet::tile_mesh_from`]'s): the answer
    /// comes on the receiver, or it hangs up.
    pub(crate) fn ask(&self, cell: CellId) -> std::sync::mpsc::Receiver<Vec<i64>> {
        let (send, receive) = std::sync::mpsc::channel();
        let mut asked = self.asked.lock().expect("the heights queue's lock");
        // Closed: the sender drops here and the asker hears the hang-up at once.
        if !asked.1 {
            asked.0.push((cell, send));
        }
        receive
    }

    fn take(&self, most: usize) -> Vec<Ask> {
        let mut asked = self.asked.lock().expect("the heights queue's lock");
        let n = most.min(asked.0.len());
        asked.0.drain(..n).collect()
    }

    /// No more answers: every ask waiting and every later one hangs up, so a worker waiting on
    /// one makes its heights on the CPU and can end.
    fn close(&self) {
        let mut asked = self.asked.lock().expect("the heights queue's lock");
        asked.1 = true;
        asked.0.clear();
    }
}

/// Tiles' heights the GPU makes a frame: about 0.6 ms of the RTX 5070 Ti.
const HEIGHTS_PER_FRAME: usize = 8;

/// A frame slot's heights in flight: the asks it answers once the slot comes round again, its
/// jobs and its results.
#[derive(Default)]
struct HeightsInFlight {
    asked: Vec<std::sync::mpsc::Sender<Vec<i64>>>,
    jobs: Option<GraphBuffer>,
    out: Option<GraphBuffer>,
}

/// The maps the worker made images for and the frame has yet to make.
#[derive(Default)]
pub(crate) struct MapQueue {
    jobs: Mutex<Vec<MapJob>>,
}

impl MapQueue {
    /// A new tile's map image (RGBA8, every mip, sampled and written by the passes), its job
    /// queued.
    pub(crate) fn add(
        &self,
        device: &Arc<Device>,
        cell: CellId,
        name: &str,
    ) -> Result<Arc<GraphImage>> {
        let image = Arc::new(GraphImage::new(
            device,
            ImageDesc {
                width: NORMAL_MAP_SIZE,
                height: NORMAL_MAP_SIZE,
                format: vk::Format::R8G8B8A8_UNORM,
                usage: vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::STORAGE
                    | vk::ImageUsageFlags::TRANSFER_SRC,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: LEVELS,
                name,
            },
        )?);
        self.jobs
            .lock()
            .expect("the map queue's lock")
            .push(MapJob {
                image: Arc::clone(&image),
                cell,
            });
        Ok(image)
    }

    /// The first `most` jobs queued, in the order they came.
    fn take(&self, most: usize) -> Vec<MapJob> {
        let mut jobs = self.jobs.lock().expect("the map queue's lock");
        let n = most.min(jobs.len());
        jobs.drain(..n).collect()
    }

    /// Jobs queued and not yet taken.
    pub(crate) fn len(&self) -> usize {
        self.jobs.lock().expect("the map queue's lock").len()
    }
}

/// One level of a grid (the shader's `Level`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLevel {
    offset: u64,
    width: u32,
    height: u32,
}

/// An elevation grid (the shader's `Grid`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuGridRecord {
    bounds: [f64; 4],
    blend: f64,
    detail_width: f64,
    detail_strength: f64,
    detail_gain: f64,
    detail_count: u32,
    whole: u32,
    level_count: u32,
    seeds_offset: u32,
    levels: [GpuLevel; MAX_GRID_LEVELS],
}

/// One map for the normal pass (`shaders/planet_normals.slang`'s `MapRecord`): its samples'
/// warp, heights and face, and its images.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMapRecord {
    columns: u64,
    rows: u64,
    heights: u64,
    radius: f64,
    coast_range: f64,
    sea: u32,
    side: u32,
    axes: [i32; 9],
    storage: [u32; LEVELS as usize],
}

/// The normal passes' push constants (`planet_normals.slang`'s `Push`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapPush {
    maps: u64,
    level: u32,
    size: u32,
}

/// Maps made a frame once the first scene is drawn: about 1.7 ms of the RTX 5070 Ti's time,
/// where a cut of hundreds of new tiles made at once took tens of milliseconds. A cut waits for
/// its maps before it is taken in ([`NormalMaps::pending`]).
const MAPS_PER_FRAME: usize = 8;

/// A level of a grid as a tile reads it in integers (the heights shader's `FixedLevel`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFixedLevel {
    level: u32,
    pad: u32,
    u_scale: i64,
    u_offset: i64,
    v_scale: i64,
    v_offset: i64,
}

/// An octave in integers (the heights shader's `FixedOctave`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFixedOctave {
    inv_width: i64,
    offset: [i64; 3],
    fade: i64,
    strength: i64,
    seed: u64,
}

/// A grid in integers (the heights shader's `FixedGrid`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFixedGrid {
    bounds: [i64; 4],
    inv_blend: i64,
    t: i64,
    strength: i64,
    inv_norm: i64,
    whole: u32,
    blended: u32,
    octave_count: u32,
    pad: u32,
    levels: [GpuFixedLevel; 2],
    octaves: [GpuFixedOctave; MAX_OCTAVES],
}

/// A tile's heights to make in integers (the heights shader's `HeightsJob`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuHeightsJob {
    columns: u64,
    rows: u64,
    out: u64,
    radius: i64,
    inv_rough: i64,
    grid_count: u32,
    sea: u32,
    side: u32,
    pad: u32,
    axes: [i32; 9],
    pad2: u32,
    grids: [GpuFixedGrid; MAX_GRIDS],
}

/// The CORDIC's constants (`forge_core::fixed::Cordic`, the shaders' `Cordic`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCordic {
    /// The angles, eight to a row (`bytemuck` has no `Pod` for 56).
    angles: [[i64; 8]; forge_core::fixed::CORDIC_STEPS / 8],
    inverse_gain: i64,
    gain: i64,
    half_pi: i64,
    pi: i64,
}

/// The heights pass's push constants (the shader's `Push`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HeightsPush {
    jobs: u64,
    elevation: u64,
    samples: u64,
    cordic: u64,
}

/// `heights` and `cell`'s grid of `side` samples a side from `first`, with `steps` a tile, as the
/// heights shader reads them; `columns`, `rows` and `out` are set by the caller.
fn heights_job(
    planet: &Planet,
    heights: &FixedHeights,
    cell: CellId,
    (side, steps, first): (u32, i64, i64),
) -> (GpuHeightsJob, Vec<i64>, Vec<i64>) {
    let bits = u32::from(cell.level()) + steps.trailing_zeros();
    let (x, y) = cell.xy();
    let warp = |base: i64| -> Vec<i64> {
        (0..i64::from(side))
            .flat_map(|k| {
                let (c, s) = fixed_warp(planet.cordic(), base * steps + first + k, bits);
                [c, s]
            })
            .collect()
    };
    let (normal, u, v) = forge_world::Face::from_index(cell.face()).axes();
    let mut axes = [0_i32; 9];
    for (k, a) in [normal, u, v].iter().flat_map(|a| a.to_array()).enumerate() {
        axes[k] = a as i32;
    }
    let mut grids = [GpuFixedGrid::zeroed(); MAX_GRIDS];
    for (slot, g) in grids.iter_mut().zip(&heights.grids) {
        let mut levels = [GpuFixedLevel::zeroed(); 2];
        for (l, fl) in levels.iter_mut().zip(&g.levels) {
            *l = GpuFixedLevel {
                level: fl.level,
                pad: 0,
                u_scale: fl.u_scale,
                u_offset: fl.u_offset,
                v_scale: fl.v_scale,
                v_offset: fl.v_offset,
            };
        }
        let mut octaves = [GpuFixedOctave::zeroed(); MAX_OCTAVES];
        for (o, fo) in octaves.iter_mut().zip(&g.octaves) {
            *o = GpuFixedOctave {
                inv_width: fo.inv_width,
                offset: fo.offset,
                fade: fo.fade,
                strength: fo.strength,
                seed: fo.seed,
            };
        }
        *slot = GpuFixedGrid {
            bounds: g.bounds,
            inv_blend: g.inv_blend,
            t: g.t,
            strength: g.strength,
            inv_norm: g.inv_norm,
            whole: u32::from(g.whole),
            blended: u32::from(g.blended),
            octave_count: g.octaves.len() as u32,
            pad: 0,
            levels,
            octaves,
        };
    }
    let job = GpuHeightsJob {
        columns: 0,
        rows: 0,
        out: 0,
        radius: heights.radius,
        inv_rough: heights.inv_rough,
        grid_count: heights.grids.len() as u32,
        sea: u32::from(heights.sea),
        side,
        pad: 0,
        axes,
        pad2: 0,
        grids,
    };
    (job, warp(x), warp(y))
}

/// Tiles `--check-heights` makes on the GPU and compares with the CPU's, bit for bit.
const HEIGHTS_CHECKED: usize = 4;

/// The tiles' heights made on the GPU at the first frame, read back (`--check-heights`).
struct HeightsCheck {
    cells: Vec<CellId>,
    out: Vec<GraphBuffer>,
    /// The jobs' address, and the buffer holding them.
    jobs: u64,
    _jobs: GraphBuffer,
    frames: usize,
    /// Whether its passes were declared (once).
    declared: std::cell::Cell<bool>,
}

/// Maps `--check-normal-maps` compares with the CPU's.
const CHECKED: usize = 6;

/// The first maps made, read back to be compared with the CPU's (`--check-normal-maps`).
struct Check {
    cells: Vec<CellId>,
    /// Their places among the frame's maps.
    picked: Vec<usize>,
    /// Whether its passes were declared (once).
    declared: std::cell::Cell<bool>,
    readback: Vec<GraphBuffer>,
    /// Frames until the copies are surely done.
    frames: usize,
}

/// Maps whose heights a frame keeps at once: their heights pass, then their normal pass, then
/// the next group's in the same buffer (16 maps' samples, 34 MB).
const MAP_GROUP: usize = 16;

/// A map's samples' heights, bytes.
const MAP_HEIGHT_BYTES: u64 = (SIDE * SIDE * 8) as u64;

/// A map's bytes in a frame's jobs: its heights job, its warp's columns and rows, its record.
const MAP_JOB_BYTES: u64 = (std::mem::size_of::<GpuHeightsJob>()
    + 2 * SIDE * 16
    + std::mem::size_of::<GpuMapRecord>()) as u64;

/// The planet's maps made on the GPU.
pub(crate) struct NormalMaps {
    device: Arc<Device>,
    map: Pipeline,
    mip: Pipeline,
    /// Every level of every grid, `i16` metres.
    samples: forge_gpu::Buffer,
    grids: forge_gpu::Buffer,
    planet: Arc<Planet>,
    queue: Arc<MapQueue>,
    /// Per frame slot: its maps' jobs, warps and records, grown as a frame needs.
    jobs: Vec<Option<GraphBuffer>>,
    /// Per frame slot: a group of maps' samples' heights ([`MAP_GROUP`]).
    map_heights: Vec<Option<GraphBuffer>>,
    /// The maps this frame makes, borrowed by its passes.
    making: Vec<MapJob>,
    /// Maps made so far, for the log.
    pub made: usize,
    /// `--check-normal-maps`: compare the first maps with the CPU's.
    pub check: bool,
    checking: Option<Check>,
    /// The heights the worker asks for, and each frame slot's in flight.
    pub heights_queue: Arc<HeightsQueue>,
    heights_in_flight: Vec<HeightsInFlight>,
    /// The tiles' heights in integers (D-058's step 2): its pass and the CORDIC's constants.
    heights: Pipeline,
    cordic: forge_gpu::Buffer,
    /// `--check-heights`: the first tiles' heights made on the GPU against the CPU's, from
    /// `check_cells` (the first scene's).
    pub check_heights: bool,
    pub check_cells: Vec<CellId>,
    heights_check: Option<HeightsCheck>,
}

impl NormalMaps {
    /// The passes and the planet's elevation on the GPU, or None where the maps stay on the
    /// CPU: a device without doubles in shaders, or a planet of noise alone.
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        planet: &Arc<Planet>,
        queue: Arc<MapQueue>,
    ) -> Result<Option<Self>> {
        if !device.features().float64 {
            tracing::info!("no doubles in shaders: the tiles' normal maps are made on the CPU");
            return Ok(None);
        }
        let Some(heights) = planet.gpu_heights() else {
            return Ok(None);
        };
        let started = std::time::Instant::now();
        let pipeline = |entry: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("planet_normals.slang", entry, ShaderStage::Compute)?,
                entry,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<MapPush>() as u32,
                name: entry,
            })?;
            device.destroy_shader_module(module);
            Ok(pipeline)
        };
        let (map, mip) = (pipeline("normal_map")?, pipeline("normal_mip")?);
        let heights_pipeline = {
            let module = device.create_shader_module(
                &shaders.compile("planet_heights.slang", "tile_heights", ShaderStage::Compute)?,
                "tile_heights",
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, "tile_heights"),
                push_constant_bytes: std::mem::size_of::<HeightsPush>() as u32,
                name: "tile_heights",
            })?;
            device.destroy_shader_module(module);
            pipeline
        };
        let c = planet.cordic();
        let cordic = device.create_buffer_with_data(
            &[GpuCordic {
                angles: std::array::from_fn(|r| std::array::from_fn(|k| c.angles[r * 8 + k])),
                inverse_gain: c.inverse_gain,
                gain: c.gain,
                half_pi: c.half_pi,
                pi: c.pi,
            }],
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Geometry,
            "cordic constants",
        )?;
        // Every level of every grid in one buffer, one after another.
        let mut records = Vec::with_capacity(heights.grids.len());
        let mut pieces: Vec<&[i16]> = Vec::new();
        let mut offset = 0_u64;
        anyhow::ensure!(
            heights.grids.len() <= MAX_GRIDS,
            "a planet of {} elevation grids",
            heights.grids.len()
        );
        for (g, grid) in heights.grids.iter().enumerate() {
            anyhow::ensure!(
                grid.detail.1 as usize <= MAX_OCTAVES,
                "{} octaves of detail",
                grid.detail.1
            );
            anyhow::ensure!(
                grid.levels.len() <= MAX_GRID_LEVELS,
                "an elevation grid of {} levels",
                grid.levels.len()
            );
            let mut levels = [GpuLevel::zeroed(); MAX_GRID_LEVELS];
            for (slot, (width, height, samples)) in levels.iter_mut().zip(&grid.levels) {
                *slot = GpuLevel {
                    offset,
                    width: *width,
                    height: *height,
                };
                offset += samples.len() as u64;
                pieces.push(samples);
            }
            records.push(GpuGridRecord {
                bounds: grid.bounds,
                blend: grid.blend,
                detail_width: grid.detail.0,
                detail_strength: grid.detail.2,
                detail_gain: grid.detail.3,
                detail_count: grid.detail.1,
                whole: u32::from(grid.whole),
                level_count: grid.levels.len() as u32,
                seeds_offset: if g == 0 {
                    0
                } else {
                    heights.detail_seeds.len() as u32
                },
                levels,
            });
        }
        let samples = device.create_buffer(BufferDesc {
            size: (offset * 2).next_multiple_of(4).max(4),
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Geometry,
            name: "planet elevation",
        })?;
        let mut at = 0_u64;
        let writes: Vec<(&forge_gpu::Buffer, u64, &[u8])> = pieces
            .iter()
            .map(|piece| {
                let bytes: &[u8] = bytemuck::cast_slice(piece);
                let write = (&samples, at, bytes);
                at += bytes.len() as u64;
                write
            })
            .collect();
        let uploading = std::time::Instant::now();
        device.write_buffers_staged(&writes)?;
        let upload_ms = uploading.elapsed().as_millis();
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let grids = device.create_buffer_with_data(
            &records,
            storage,
            MemoryCategory::Geometry,
            "planet elevation grids",
        )?;
        tracing::info!(
            grids = records.len(),
            mib = offset * 2 / (1 << 20),
            upload_ms,
            ms = started.elapsed().as_millis(),
            "the planet's elevation on the GPU, for the tiles' normal maps (D-058)"
        );
        Ok(Some(Self {
            device: Arc::clone(device),
            map,
            mip,
            samples,
            grids,
            planet: Arc::clone(planet),
            queue,
            jobs: (0..forge_gpu::FRAMES_IN_FLIGHT).map(|_| None).collect(),
            map_heights: (0..forge_gpu::FRAMES_IN_FLIGHT).map(|_| None).collect(),
            making: Vec::new(),
            made: 0,
            check: false,
            checking: None,
            heights: heights_pipeline,
            cordic,
            check_heights: false,
            check_cells: Vec::new(),
            heights_check: None,
            heights_queue: Arc::default(),
            heights_in_flight: (0..forge_gpu::FRAMES_IN_FLIGHT)
                .map(|_| HeightsInFlight::default())
                .collect(),
        }))
    }

    /// Declares the passes that make the maps queued so far, before the scene's passes of the
    /// frame: the maps at their first level, each mip from the one before, then the maps ready
    /// for the scene's shaders to sample.
    pub(crate) fn record<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
    ) -> Result<()> {
        self.compare_checked();
        self.compare_heights();
        let asked = self.prepare_heights(slot)?;
        // The first frame makes every map of the first scene, which it draws.
        let most = if self.made == 0 {
            usize::MAX
        } else {
            MAPS_PER_FRAME
        };
        self.making = self.queue.take(most);
        let count = self.making.len();
        self.made += count;
        if self.check_heights && self.heights_check.is_none() && !self.check_cells.is_empty() {
            self.prepare_heights_check()?;
        }
        // This slot's buffers, grown to the frame's maps: the frame that last used them is done.
        let needed = MAP_JOB_BYTES * count as u64;
        let buffer = &mut self.jobs[slot.index];
        if count > 0 && buffer.as_ref().is_none_or(|b| b.size() < needed) {
            *buffer = Some(GraphBuffer::new(self.device.create_buffer(BufferDesc {
                size: needed.next_power_of_two().max(MAP_JOB_BYTES * 16),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Frame,
                name: "planet normal map jobs",
            })?));
        }
        let heights_buffer = &mut self.map_heights[slot.index];
        if count > 0 && heights_buffer.is_none() {
            *heights_buffer = Some(GraphBuffer::new(self.device.create_buffer(BufferDesc {
                size: MAP_HEIGHT_BYTES * MAP_GROUP as u64,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name: "planet normal map heights",
            })?));
        }
        // `--check-normal-maps`: the finest maps, which take their angles from their centre in
        // floats, and some others spread over the rest, copied back to compare a few frames later.
        if self.check && self.checking.is_none() && count > 0 {
            let mut order: Vec<usize> = (0..count).collect();
            order.sort_by_key(|&k| std::cmp::Reverse(self.making[k].cell.level()));
            let mut picked: Vec<usize> = order.iter().take(CHECKED / 2).copied().collect();
            let rest = &order[picked.len()..];
            for i in 0..(CHECKED - picked.len()).min(rest.len()) {
                picked.push(rest[i * rest.len() / (CHECKED - CHECKED / 2)]);
            }
            let readback = (0..picked.len())
                .map(|k| -> Result<GraphBuffer> {
                    Ok(GraphBuffer::new(self.device.create_buffer(BufferDesc {
                        size: u64::from(NORMAL_MAP_SIZE * NORMAL_MAP_SIZE * 4),
                        usage: vk::BufferUsageFlags::TRANSFER_DST,
                        location: MemoryLocation::GpuToCpu,
                        category: MemoryCategory::Transfer,
                        name: &format!("planet normal map check {k}"),
                    })?))
                })
                .collect::<Result<Vec<_>>>()?;
            self.checking = Some(Check {
                cells: picked.iter().map(|&k| self.making[k].cell).collect(),
                picked,
                readback,
                frames: forge_gpu::FRAMES_IN_FLIGHT + 1,
                declared: std::cell::Cell::new(false),
            });
        }
        // Nothing changes from here: the passes borrow the maps for the frame.
        let this: &'f Self = self;
        // `--check-heights`: the first tiles' heights in integers on the GPU, read back.
        if let Some(check) = this.heights_check.as_ref().filter(|c| !c.declared.get()) {
            this.declare_heights_check(graph, check);
        }
        if asked > 0 {
            this.declare_heights(graph, slot, asked);
        }
        if count == 0 {
            return Ok(());
        }
        let buffer = this.jobs[slot.index].as_ref().expect("made above");
        let heights_buffer = this.map_heights[slot.index].as_ref().expect("made above");
        let base = buffer.address();
        let job_bytes = std::mem::size_of::<GpuHeightsJob>() as u64;
        let record_bytes = std::mem::size_of::<GpuMapRecord>() as u64;
        let tables = (SIDE * 16) as u64;
        let warps = base + count as u64 * job_bytes;
        let records = warps + count as u64 * 2 * tables;
        let world = &this.planet.world;
        for (k, job) in this.making.iter().enumerate() {
            // The map's samples: the next level's heights, half a texel apart.
            let level = job.cell.level();
            let heights = this
                .planet
                .fixed_heights(world.min_wavelength(level + 1))
                .expect("a planet with a map");
            let grid = (SIDE as u32, 2 * i64::from(NORMAL_MAP_SIZE), 0);
            let (mut heights_job, columns, rows) =
                heights_job(&this.planet, &heights, job.cell, grid);
            let at = warps + k as u64 * 2 * tables;
            let out = heights_buffer.address() + (k % MAP_GROUP) as u64 * MAP_HEIGHT_BYTES;
            heights_job.columns = at;
            heights_job.rows = at + tables;
            heights_job.out = out;
            buffer.write(k as u64 * job_bytes, &[heights_job]);
            buffer.write(at - base, &columns);
            buffer.write(at + tables - base, &rows);
            let mut storage = [0_u32; LEVELS as usize];
            for (l, id) in storage.iter_mut().enumerate() {
                *id = job.image.storage(l as u32).0;
            }
            let record = GpuMapRecord {
                columns: at,
                rows: at + tables,
                heights: out,
                radius: world.planet.radius,
                coast_range: COAST_RANGE * world.spacing(level),
                sea: heights_job.sea,
                side: SIDE as u32,
                axes: heights_job.axes,
                storage,
            };
            buffer.write(records - base + k as u64 * record_bytes, &[record]);
        }
        let jobs = graph.import_buffer(buffer);
        let heights_handle = graph.import_buffer(heights_buffer);
        let images: Vec<_> = this.making.iter().map(|j| graph.import(&j.image)).collect();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        // A group of maps at a time: their heights, then their first level from them.
        for group in 0..count.div_ceil(MAP_GROUP) {
            let first = group * MAP_GROUP;
            let n = (count - first).min(MAP_GROUP);
            let heights_push = HeightsPush {
                jobs: base + first as u64 * job_bytes,
                elevation: this.grids.address(),
                samples: this.samples.address(),
                cordic: this.cordic.address(),
            };
            let pipeline = &this.heights;
            graph
                .pass("planet/normal map heights")
                .buffer(jobs, BufferAccess::ShaderRead(compute))
                .buffer(heights_handle, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(pipeline, &heights_push);
                    let side = SIDE as u32;
                    commands.dispatch(side.div_ceil(8), side.div_ceil(8), n as u32);
                    Ok(())
                });
            let map_push = MapPush {
                maps: records + first as u64 * record_bytes,
                level: 0,
                size: NORMAL_MAP_SIZE,
            };
            let mut pass = graph
                .pass("planet/normal maps")
                .buffer(jobs, BufferAccess::ShaderRead(compute))
                .buffer(heights_handle, BufferAccess::ShaderRead(compute));
            for &image in &images[first..first + n] {
                pass = pass.image_mip(image, 0, ImageAccess::StorageWrite(compute));
            }
            let map = &this.map;
            pass.run(move |_, commands| {
                commands.bind_pipeline(map);
                commands.push_constants(map, &map_push);
                let side = NORMAL_MAP_SIZE.div_ceil(8);
                commands.dispatch(side, side, n as u32);
                Ok(())
            });
        }
        for level in 1..LEVELS {
            let mut pass = graph
                .pass("planet/normal map mips")
                .buffer(jobs, BufferAccess::ShaderRead(compute));
            for &image in &images {
                pass = pass
                    .image_mip(image, level - 1, ImageAccess::StorageRead(compute))
                    .image_mip(image, level, ImageAccess::StorageWrite(compute));
            }
            let mip = &this.mip;
            let mip_push = MapPush {
                maps: records,
                level,
                size: NORMAL_MAP_SIZE,
            };
            let side = (NORMAL_MAP_SIZE >> level).max(1);
            pass.run(move |_, commands| {
                commands.bind_pipeline(mip);
                commands.push_constants(mip, &mip_push);
                commands.dispatch(side.div_ceil(8), side.div_ceil(8), count as u32);
                Ok(())
            });
        }
        // `--check-normal-maps`: the first maps copied back, to compare a few frames later.
        if let Some(check) = this.checking.as_ref().filter(|c| !c.declared.get()) {
            check.declared.set(true);
            for (k, buffer) in check.readback.iter().enumerate() {
                let handle = graph.import_buffer(buffer);
                let image = &this.making[check.picked[k]].image;
                graph
                    .pass("planet/normal maps check")
                    .image_mip(images[check.picked[k]], 0, ImageAccess::TransferSrc)
                    .buffer(handle, BufferAccess::TransferDst)
                    .run(move |_, commands| {
                        let extent = vk::Extent2D {
                            width: NORMAL_MAP_SIZE,
                            height: NORMAL_MAP_SIZE,
                        };
                        commands.copy_image_to_buffer(image.raw(), extent, buffer);
                        Ok(())
                    });
                graph
                    .pass("planet/normal maps check")
                    .buffer(handle, BufferAccess::HostRead)
                    .run(|_, _| Ok(()));
            }
        }
        let sampled = vk::PipelineStageFlags2::FRAGMENT_SHADER | compute;
        let mut ready = graph.pass("planet/normal maps ready");
        for &image in &images {
            ready = ready.image(image, ImageAccess::Sampled(sampled));
        }
        ready.run(|_, _| Ok(()));
        Ok(())
    }

    /// `--check-heights`: the heights of a few of this frame's tiles (the finest, and coarser
    /// ones) made in integers on the GPU into buffers the CPU reads a few frames later.
    fn prepare_heights_check(&mut self) -> Result<()> {
        let mut cells = self.check_cells.clone();
        cells.sort_by_key(|c| std::cmp::Reverse(c.level()));
        let mut picked: Vec<CellId> = cells.iter().take(HEIGHTS_CHECKED / 2).copied().collect();
        let rest = &cells[picked.len()..];
        for i in 0..(HEIGHTS_CHECKED - picked.len()).min(rest.len()) {
            picked.push(rest[i * rest.len() / (HEIGHTS_CHECKED - HEIGHTS_CHECKED / 2)]);
        }
        let side = self.planet.world.tiles.samples + 2;
        let cells_bytes = (side * side) as u64 * 8;
        let out = picked
            .iter()
            .map(|_| -> Result<GraphBuffer> {
                Ok(GraphBuffer::new(self.device.create_buffer(BufferDesc {
                    size: cells_bytes,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: "planet heights check",
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        let job_bytes = std::mem::size_of::<GpuHeightsJob>() as u64;
        let tables = u64::from(side) * 2 * 8;
        let jobs = GraphBuffer::new(self.device.create_buffer(BufferDesc {
            size: picked.len() as u64 * (job_bytes + 2 * tables),
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Frame,
            name: "planet heights check jobs",
        })?);
        let base = jobs.address();
        let warps = base + picked.len() as u64 * job_bytes;
        for (k, (&cell, out)) in picked.iter().zip(&out).enumerate() {
            let heights = self
                .planet
                .fixed_heights(self.planet.world.min_wavelength(cell.level()))
                .expect("a planet with a map");
            let (mut job, columns, rows) = heights_job(
                &self.planet,
                &heights,
                cell,
                (side, i64::from(side) - 3, -1),
            );
            let at = warps + k as u64 * 2 * tables;
            job.columns = at;
            job.rows = at + tables;
            job.out = out.address();
            jobs.write(k as u64 * job_bytes, &[job]);
            jobs.write(at - base, &columns);
            jobs.write(at + tables - base, &rows);
        }
        self.heights_check = Some(HeightsCheck {
            cells: picked,
            out,
            jobs: base,
            _jobs: jobs,
            frames: forge_gpu::FRAMES_IN_FLIGHT + 1,
            declared: std::cell::Cell::new(false),
        });
        Ok(())
    }

    /// The passes of `--check-heights`, this frame.
    fn declare_heights_check<'f>(&'f self, graph: &mut FrameGraph<'f>, check: &'f HeightsCheck) {
        check.declared.set(true);
        let side = self.planet.world.tiles.samples + 2;
        let base = check.jobs;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let handles: Vec<_> = check.out.iter().map(|b| graph.import_buffer(b)).collect();
        let mut pass = graph.pass("planet/tile heights check");
        for &h in &handles {
            pass = pass.buffer(h, BufferAccess::ShaderReadWrite(compute));
        }
        let push = HeightsPush {
            jobs: base,
            elevation: self.grids.address(),
            samples: self.samples.address(),
            cordic: self.cordic.address(),
        };
        let (pipeline, count) = (&self.heights, check.cells.len() as u32);
        pass.run(move |_, commands| {
            commands.bind_pipeline(pipeline);
            commands.push_constants(pipeline, &push);
            commands.dispatch(side.div_ceil(8), side.div_ceil(8), count);
            Ok(())
        });
        let mut read = graph.pass("planet/tile heights check");
        for &h in &handles {
            read = read.buffer(h, BufferAccess::HostRead);
        }
        read.run(|_, _| Ok(()));
    }

    /// `--check-heights`, once the GPU is done: each tile's heights against the CPU's, bit for
    /// bit, in the log.
    fn compare_heights(&mut self) {
        let Some(check) = self.heights_check.as_mut() else {
            return;
        };
        if check.frames == 0 {
            return;
        }
        check.frames -= 1;
        if check.frames > 0 {
            return;
        }
        let planet = &self.planet;
        let n = i64::from(planet.world.tiles.samples);
        let side = (n + 2) as usize;
        for (cell, out) in check.cells.iter().zip(&check.out) {
            let mut gpu = vec![0_i64; side * side];
            out.read(0, &mut gpu);
            let heights = planet
                .fixed_heights(planet.world.min_wavelength(cell.level()))
                .expect("a planet with a map");
            let (_, columns, rows) =
                heights_job(planet, &heights, *cell, (side as u32, side as i64 - 3, -1));
            let face = forge_world::Face::from_index(cell.face());
            let (mut differ, mut digest_cpu, mut digest_gpu, mut worst) = (0, 0_u64, 0_u64, 0_i64);
            for j in 0..side {
                for i in 0..side {
                    let d = fixed_direction(
                        face,
                        (columns[2 * i], columns[2 * i + 1]),
                        (rows[2 * j], rows[2 * j + 1]),
                    );
                    let cpu = planet.fixed_height(&heights, planet.cordic(), d);
                    let g = gpu[j * side + i];
                    differ += usize::from(cpu != g);
                    worst = worst.max((cpu - g).abs());
                    digest_cpu = forge_core::hash::mix64(digest_cpu ^ cpu as u64);
                    digest_gpu = forge_core::hash::mix64(digest_gpu ^ g as u64);
                }
            }
            tracing::info!(
                tile = ?cell,
                level = cell.level(),
                samples = side * side,
                differ,
                worst_q32 = worst,
                digest_cpu = %format_args!("{digest_cpu:016x}"),
                digest_gpu = %format_args!("{digest_gpu:016x}"),
                "a tile's heights in integers on the GPU against the CPU's (--check-heights)"
            );
        }
    }

    /// The heights `slot`'s last frame made, sent to who asked; then this frame's asks, at most
    /// [`HEIGHTS_PER_FRAME`], their jobs written. Returns how many.
    fn prepare_heights(&mut self, slot: FrameSlot) -> Result<usize> {
        let side = (self.planet.world.tiles.samples + 2) as usize;
        let tile = side * side;
        let in_flight = &mut self.heights_in_flight[slot.index];
        if let Some(out) = in_flight.out.as_ref() {
            for (k, send) in in_flight.asked.drain(..).enumerate() {
                let mut heights = vec![0_i64; tile];
                out.read((k * tile * 8) as u64, &mut heights);
                // A worker that stopped waiting has hung up: nothing to do.
                let _ = send.send(heights);
            }
        }
        let asks = self.heights_queue.take(HEIGHTS_PER_FRAME);
        if asks.is_empty() {
            return Ok(0);
        }
        let count = asks.len();
        let job_bytes = std::mem::size_of::<GpuHeightsJob>() as u64;
        let tables = side as u64 * 2 * 8;
        let jobs_size = count as u64 * (job_bytes + 2 * tables);
        let out_size = (count * tile * 8) as u64;
        let device = Arc::clone(&self.device);
        let grow =
            |buffer: &mut Option<GraphBuffer>, size: u64, location, name: &str| -> Result<()> {
                if buffer.as_ref().is_none_or(|b| b.size() < size) {
                    *buffer = Some(GraphBuffer::new(device.create_buffer(BufferDesc {
                        size: size.next_power_of_two(),
                        usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                        location,
                        category: MemoryCategory::Frame,
                        name,
                    })?));
                }
                Ok(())
            };
        let in_flight = &mut self.heights_in_flight[slot.index];
        grow(
            &mut in_flight.jobs,
            jobs_size,
            MemoryLocation::CpuToGpu,
            "planet heights jobs",
        )?;
        grow(
            &mut in_flight.out,
            out_size,
            MemoryLocation::GpuToCpu,
            "planet heights",
        )?;
        let (jobs, out) = (
            in_flight.jobs.as_ref().expect("made above"),
            in_flight.out.as_ref().expect("made above"),
        );
        let base = jobs.address();
        let warps = base + count as u64 * job_bytes;
        for (k, (cell, send)) in asks.into_iter().enumerate() {
            let heights = self
                .planet
                .fixed_heights(self.planet.world.min_wavelength(cell.level()))
                .expect("a planet with a map");
            let (mut job, columns, rows) = heights_job(
                &self.planet,
                &heights,
                cell,
                (side as u32, side as i64 - 3, -1),
            );
            let at = warps + k as u64 * 2 * tables;
            job.columns = at;
            job.rows = at + tables;
            job.out = out.address() + (k * tile * 8) as u64;
            jobs.write(k as u64 * job_bytes, &[job]);
            jobs.write(at - base, &columns);
            jobs.write(at + tables - base, &rows);
            in_flight.asked.push(send);
        }
        Ok(count)
    }

    /// The pass making this frame's `count` asked heights into `slot`'s results.
    fn declare_heights<'f>(&'f self, graph: &mut FrameGraph<'f>, slot: FrameSlot, count: usize) {
        let in_flight = &self.heights_in_flight[slot.index];
        let (jobs, out) = (
            in_flight.jobs.as_ref().expect("prepared"),
            in_flight.out.as_ref().expect("prepared"),
        );
        let side = self.planet.world.tiles.samples + 2;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let (jobs_handle, out_handle) = (graph.import_buffer(jobs), graph.import_buffer(out));
        let push = HeightsPush {
            jobs: jobs.address(),
            elevation: self.grids.address(),
            samples: self.samples.address(),
            cordic: self.cordic.address(),
        };
        let pipeline = &self.heights;
        graph
            .pass("planet/tile heights")
            .buffer(jobs_handle, BufferAccess::ShaderRead(compute))
            .buffer(out_handle, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push);
                commands.dispatch(side.div_ceil(8), side.div_ceil(8), count as u32);
                Ok(())
            });
        graph
            .pass("planet/tile heights")
            .buffer(out_handle, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// Maps queued and not made yet: a new cut waits for them before it is drawn.
    pub(crate) fn pending(&self) -> usize {
        self.queue.len()
    }

    /// `--check-normal-maps`, once the copies are done: each map read back against the CPU's for
    /// its tile, in the log.
    fn compare_checked(&mut self) {
        let Some(check) = self.checking.as_mut() else {
            return;
        };
        if check.frames == 0 {
            return;
        }
        check.frames -= 1;
        if check.frames > 0 {
            return;
        }
        let texels = (NORMAL_MAP_SIZE * NORMAL_MAP_SIZE) as usize;
        let decode = |t: &[u8; 4]| {
            glam::Vec3::new(f32::from(t[0]), f32::from(t[1]), f32::from(t[2])) / 127.5
                - glam::Vec3::ONE
        };
        for (cell, buffer) in check.cells.iter().zip(&check.readback) {
            let mut gpu = vec![0_u8; texels * 4];
            buffer.read(0, &mut gpu);
            let cpu = tile_normal_map(&self.planet, *cell);
            let (mut channel, mut coast, mut off, mut angle) = (0_u8, 0_u8, 0_usize, 0.0_f32);
            for (g, c) in gpu.as_chunks::<4>().0.iter().zip(cpu.as_chunks::<4>().0) {
                let d = (0..3).map(|k| g[k].abs_diff(c[k])).max().unwrap_or(0);
                channel = channel.max(d);
                coast = coast.max(g[3].abs_diff(c[3]));
                off += usize::from(d > 2);
                let cos = decode(g)
                    .normalize_or_zero()
                    .dot(decode(c).normalize_or_zero());
                angle = angle.max(cos.clamp(-1.0, 1.0).acos().to_degrees());
            }
            tracing::info!(
                tile = ?cell,
                level = cell.level(),
                max_channel = channel,
                max_coast = coast,
                texels_over_2 = off,
                max_angle_deg = %format_args!("{angle:.2}"),
                "a normal map made on the GPU against the CPU's (--check-normal-maps)"
            );
        }
    }
}

impl Drop for NormalMaps {
    fn drop(&mut self) {
        // The tile worker may be waiting on heights the frames will no longer make.
        self.heights_queue.close();
    }
}
