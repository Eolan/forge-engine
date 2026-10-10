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
use forge_terrain::planet::{NORMAL_MAP_SIZE, gpu_tile_map, tile_normal_map};
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

/// An octave's lattice at a map's centre: its cell and the centre's place in it (the shader's
/// `Lattice`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLattice {
    cell: [i32; 3],
    frac: [f32; 3],
    fade: f32,
    inv_width: f32,
}

/// A grid as a map reads it (the shader's `GridJob`): the level `Elevation::height` picks for the
/// map's narrowest wavelength and the next, and the map's centre's place in each.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuGridJob {
    level: u32,
    blended: u32,
    t: f32,
    blend: f32,
    u_cell: [i32; 2],
    v_cell: [i32; 2],
    u_frac: [f32; 2],
    v_frac: [f32; 2],
    u_scale: [f32; 2],
    v_scale: [f32; 2],
    edge: [f32; 4],
}

/// What a job needs of a grid, kept from [`forge_terrain::planet::GpuHeights`].
struct GridMeta {
    whole: bool,
    bounds: [f64; 4],
    blend: f64,
    levels: Vec<(u32, u32)>,
    detail: (f64, u32),
}

/// A map to make (the shader's `MapJob`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuJob {
    axes: [f64; 9],
    min_wavelength: f64,
    coast_range: f64,
    columns: u64,
    rows: u64,
    lattice: u64,
    grids: u64,
    centre: [f64; 3],
    lat0: f64,
    lon0: f64,
    relative: u32,
    storage: [u32; LEVELS as usize],
}

/// The passes' push constants (the shader's `Push`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    grids: u64,
    samples: u64,
    seeds: u64,
    jobs: u64,
    rough_above: f64,
    radius: f64,
    grid_count: u32,
    sea: u32,
    level: u32,
    size: u32,
}

/// Maps made a frame once the first scene is drawn: about 1.7 ms of the RTX 5070 Ti's time,
/// where a cut of hundreds of new tiles made at once took tens of milliseconds. A cut waits for
/// its maps before it is taken in ([`NormalMaps::pending`]).
const MAPS_PER_FRAME: usize = 8;

/// Maps `--check-normal-maps` compares with the CPU's.
const CHECKED: usize = 6;

/// The first maps made, read back to be compared with the CPU's (`--check-normal-maps`).
struct Check {
    cells: Vec<CellId>,
    readback: Vec<GraphBuffer>,
    /// Frames until the copies are surely done.
    frames: usize,
}

/// A job's bytes in a frame's buffer: its record, then its columns' and rows' tangents, its
/// lattice table and its grids'.
const JOB_BYTES: u64 = (std::mem::size_of::<GpuJob>()
    + 2 * SIDE * 8
    + MAX_GRIDS * MAX_OCTAVES * std::mem::size_of::<GpuLattice>()
    + MAX_GRIDS * std::mem::size_of::<GpuGridJob>()) as u64;

/// Tiles at least this fine take their latitude and longitude from their centre's in floats.
const RELATIVE_FROM_LEVEL: u8 = 3;
/// ... unless their centre is nearer a pole than this (the sine of its latitude).
const RELATIVE_BELOW: f64 = 0.9;

/// `t` clamped to 0..1 and eased, as `forge_terrain::planet`'s.
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `meta` as a map centred at `lat0`, `lon0` (radians) reads it: the level `Elevation::height`
/// picks for the narrowest wavelength `min_wavelength` on a body of `radius` and the next, and
/// the centre's place in each, as `Elevation::sample` finds it.
fn grid_job(meta: &GridMeta, lat0: f64, lon0: f64, min_wavelength: f64, radius: f64) -> GpuGridJob {
    use std::f64::consts::{PI, TAU};
    let [west, east, south, north] = meta.bounds;
    let spacing = if meta.whole {
        TAU * radius / f64::from(meta.levels[0].0)
    } else {
        (north - south) * radius / f64::from(meta.levels[0].1)
    };
    let wanted = 0.5 * min_wavelength;
    let (mut level, mut at) = (0, spacing);
    while at * 2.0 <= wanted && level + 1 < meta.levels.len() {
        at *= 2.0;
        level += 1;
    }
    let blended = !(wanted <= at || level + 1 == meta.levels.len());
    let mut job = GpuGridJob {
        level: level as u32,
        blended: u32::from(blended),
        t: if blended {
            ((wanted - at) / at) as f32
        } else {
            0.0
        },
        blend: meta.blend as f32,
        edge: [
            (lon0 - west) as f32,
            (east - lon0) as f32,
            (lat0 - south) as f32,
            (north - lat0) as f32,
        ],
        ..GpuGridJob::zeroed()
    };
    for k in 0..=usize::from(blended) {
        let (w, h) = meta.levels[level + k];
        let (w, h) = (f64::from(w), f64::from(h));
        let (u, v, u_scale, v_scale) = if meta.whole {
            (
                (lon0 / TAU + 0.5) * w - 0.5,
                (0.5 - lat0 / PI) * h - 0.5,
                w / TAU,
                -h / PI,
            )
        } else {
            (
                (lon0 - west) / (east - west) * w - 0.5,
                (north - lat0) / (north - south) * h - 0.5,
                w / (east - west),
                -h / (north - south),
            )
        };
        job.u_cell[k] = u.floor() as i32;
        job.v_cell[k] = v.floor() as i32;
        job.u_frac[k] = (u - u.floor()) as f32;
        job.v_frac[k] = (v - v.floor()) as f32;
        job.u_scale[k] = u_scale as f32;
        job.v_scale[k] = v_scale as f32;
    }
    job
}

/// The planet's maps made on the GPU.
pub(crate) struct NormalMaps {
    device: Arc<Device>,
    map: Pipeline,
    mip: Pipeline,
    /// Every level of every grid, `i16` metres.
    samples: forge_gpu::Buffer,
    grids: forge_gpu::Buffer,
    seeds: forge_gpu::Buffer,
    grid_count: u32,
    /// What the jobs need of each grid.
    meta: Vec<GridMeta>,
    rough_above: f64,
    radius: f64,
    sea: bool,
    planet: Arc<Planet>,
    queue: Arc<MapQueue>,
    /// Per frame slot: its jobs' records and tangents, grown as a frame needs.
    jobs: Vec<Option<GraphBuffer>>,
    /// The maps this frame makes, borrowed by its passes.
    making: Vec<MapJob>,
    /// Maps made so far, for the log.
    pub made: usize,
    /// `--check-normal-maps`: compare the first maps with the CPU's.
    pub check: bool,
    checking: Option<Check>,
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
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                name: entry,
            })?;
            device.destroy_shader_module(module);
            Ok(pipeline)
        };
        let (map, mip) = (pipeline("normal_map")?, pipeline("normal_mip")?);
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
        let seeds: Vec<u64> = heights
            .detail_seeds
            .iter()
            .chain(heights.region_seeds)
            .copied()
            .collect();
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let grids = device.create_buffer_with_data(
            &records,
            storage,
            MemoryCategory::Geometry,
            "planet elevation grids",
        )?;
        let seeds = device.create_buffer_with_data(
            &seeds,
            storage,
            MemoryCategory::Geometry,
            "planet detail seeds",
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
            seeds,
            grid_count: records.len() as u32,
            meta: heights
                .grids
                .iter()
                .map(|g| GridMeta {
                    whole: g.whole,
                    bounds: g.bounds,
                    blend: g.blend,
                    levels: g.levels.iter().map(|(w, h, _)| (*w, *h)).collect(),
                    detail: (g.detail.0, g.detail.1),
                })
                .collect(),
            rough_above: heights.rough_above,
            radius: heights.radius,
            sea: heights.sea,
            planet: Arc::clone(planet),
            queue,
            jobs: (0..forge_gpu::FRAMES_IN_FLIGHT).map(|_| None).collect(),
            making: Vec::new(),
            made: 0,
            check: false,
            checking: None,
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
        // The first frame makes every map of the first scene, which it draws.
        let most = if self.made == 0 {
            usize::MAX
        } else {
            MAPS_PER_FRAME
        };
        self.making = self.queue.take(most);
        if self.making.is_empty() {
            return Ok(());
        }
        let count = self.making.len();
        self.made += count;
        // This slot's buffer, grown to the frame's jobs: the frame that last used it is done.
        let needed = JOB_BYTES * count as u64;
        let buffer = &mut self.jobs[slot.index];
        if buffer.as_ref().is_none_or(|b| b.size() < needed) {
            *buffer = Some(GraphBuffer::new(self.device.create_buffer(BufferDesc {
                size: needed.next_power_of_two().max(JOB_BYTES * 16),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Frame,
                name: "planet normal map jobs",
            })?));
        }
        let buffer = self.jobs[slot.index].as_ref().expect("made above");
        let base = buffer.address();
        let tangents = base + std::mem::size_of::<GpuJob>() as u64 * count as u64;
        let lattices = tangents + (count * 2 * SIDE * 8) as u64;
        let lattice_bytes = (MAX_GRIDS * MAX_OCTAVES * std::mem::size_of::<GpuLattice>()) as u64;
        let grid_tables = lattices + count as u64 * lattice_bytes;
        let grid_bytes = (MAX_GRIDS * std::mem::size_of::<GpuGridJob>()) as u64;
        for (k, job) in self.making.iter().enumerate() {
            let tile = gpu_tile_map(&self.planet.world, job.cell);
            let at = tangents + (k * 2 * SIDE * 8) as u64;
            let [n, u, v] = tile.axes;
            // The middle sample, from which the shader takes every other in floats.
            let middle = SIDE / 2;
            let centre = (n + u * tile.columns[middle] + v * tile.rows[middle]).normalize();
            let lat0 = centre
                .y
                .atan2((centre.x * centre.x + centre.z * centre.z).sqrt());
            let lon0 = centre.x.atan2(centre.z);
            let relative =
                job.cell.level() >= RELATIVE_FROM_LEVEL && centre.y.abs() < RELATIVE_BELOW;
            let mut table = [GpuLattice::zeroed(); MAX_GRIDS * MAX_OCTAVES];
            let mut grid_table = [GpuGridJob::zeroed(); MAX_GRIDS];
            let p = centre * self.radius;
            for (g, meta) in self.meta.iter().enumerate() {
                // Its octaves at the centre (octaves_by).
                let mut width = meta.detail.0;
                for octave in 0..meta.detail.1 as usize {
                    let q = p / width + glam::DVec3::new(0.371, 0.613, 0.229) * octave as f64;
                    let cell = q.floor();
                    table[g * MAX_OCTAVES + octave] = GpuLattice {
                        cell: [cell.x as i32, cell.y as i32, cell.z as i32],
                        frac: (q - cell).as_vec3().to_array(),
                        fade: smooth(width / tile.min_wavelength - 1.0) as f32,
                        inv_width: (1.0 / width) as f32,
                    };
                    width *= 0.5;
                }
                grid_table[g] = grid_job(meta, lat0, lon0, tile.min_wavelength, self.radius);
            }
            let lattice = lattices + k as u64 * lattice_bytes;
            buffer.write(lattice - base, &table);
            let grids = grid_tables + k as u64 * grid_bytes;
            buffer.write(grids - base, &grid_table);
            let mut storage = [0_u32; LEVELS as usize];
            for (level, id) in storage.iter_mut().enumerate() {
                *id = job.image.storage(level as u32).0;
            }
            let record = GpuJob {
                axes: [n.x, n.y, n.z, u.x, u.y, u.z, v.x, v.y, v.z],
                min_wavelength: tile.min_wavelength,
                coast_range: tile.coast_range,
                columns: at,
                rows: at + (SIDE * 8) as u64,
                lattice,
                grids,
                centre: centre.to_array(),
                lat0,
                lon0,
                relative: u32::from(relative),
                storage,
            };
            buffer.write((k * std::mem::size_of::<GpuJob>()) as u64, &[record]);
            let written = at - base;
            buffer.write(written, &tile.columns);
            buffer.write(written + (SIDE * 8) as u64, &tile.rows);
        }
        let push = Push {
            grids: self.grids.address(),
            samples: self.samples.address(),
            seeds: self.seeds.address(),
            jobs: base,
            rough_above: self.rough_above,
            radius: self.radius,
            grid_count: self.grid_count,
            sea: u32::from(self.sea),
            level: 0,
            size: NORMAL_MAP_SIZE,
        };
        let jobs = graph.import_buffer(buffer);
        let images: Vec<_> = self.making.iter().map(|j| graph.import(&j.image)).collect();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        for level in 0..LEVELS {
            let label = if level == 0 {
                "planet/normal maps"
            } else {
                "planet/normal map mips"
            };
            let mut pass = graph
                .pass(label)
                .buffer(jobs, BufferAccess::ShaderRead(compute));
            for &image in &images {
                if level > 0 {
                    pass = pass.image_mip(image, level - 1, ImageAccess::StorageRead(compute));
                }
                pass = pass.image_mip(image, level, ImageAccess::StorageWrite(compute));
            }
            let pipeline = if level == 0 { &self.map } else { &self.mip };
            let side = (NORMAL_MAP_SIZE >> level).max(1);
            pass.run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &Push { level, ..push });
                commands.dispatch(side.div_ceil(8), side.div_ceil(8), count as u32);
                Ok(())
            });
        }
        // `--check-normal-maps`: the first maps copied back, to compare a few frames later.
        if self.check && self.checking.is_none() {
            // The finest maps, which take their angles from their centre in floats, and some
            // others spread over the rest.
            let mut order: Vec<usize> = (0..count).collect();
            order.sort_by_key(|&k| std::cmp::Reverse(self.making[k].cell.level()));
            let mut picked: Vec<usize> = order.iter().take(CHECKED / 2).copied().collect();
            let rest = &order[picked.len()..];
            for i in 0..(CHECKED - picked.len()).min(rest.len()) {
                picked.push(rest[i * rest.len() / (CHECKED - CHECKED / 2)]);
            }
            let checked = picked.len();
            let readback = (0..checked)
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
                readback,
                frames: forge_gpu::FRAMES_IN_FLIGHT + 1,
            });
            let check = self.checking.as_ref().expect("made above");
            for (k, buffer) in check.readback.iter().enumerate() {
                let handle = graph.import_buffer(buffer);
                let image = &self.making[picked[k]].image;
                graph
                    .pass("planet/normal maps check")
                    .image_mip(images[picked[k]], 0, ImageAccess::TransferSrc)
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
