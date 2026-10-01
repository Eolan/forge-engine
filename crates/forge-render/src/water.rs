//! The sea's waves on the GPU (issue #105, D-038, `shaders/water.slang`): cascades of FFT
//! waves, each a tiling patch of [`WATER_SIZE`]² samples evolved to the frame's time and
//! transformed on the async compute queue into two persistent images: the displacement (x,
//! height, y) and the slopes with the Jacobian, which marks the whitecaps (Tessendorf 2001).
//! Five graph passes a frame, each over every cascade:
//! - `water/evolve`: the spectrum at the frame's time, the fields' spectra packed two to a
//!   complex value;
//! - `water/fft-rows` and `water/fft-cols`: the inverse transform, radix-2 Stockham in
//!   groupshared memory, a workgroup per line;
//! - `water/derive`: the fields unpacked into the images;
//! - `water/mips`: their mip chains, a level from the one below (the slopes image keeps the
//!   squared slope too, so a mip knows the variance of the slopes it no longer resolves).
//!
//! The spectrum comes from the CPU (`forge_procgen::Ocean::gpu_samples`, uploaded once), so
//! the GPU transforms the CPU's amplitudes and its surface is the CPU's `Ocean::surface` in
//! single precision; [`WaterCascades::read_fields`] reads a cascade back for that comparison.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory,
    MemoryLocation, Pipeline, QueueKind, Result, ShaderCompiler, ShaderStage, TransientDesc,
    VertexPipelineDesc, vk,
};
use glam::{DVec3, Mat4, Vec3};

use crate::aces2::f32_to_f16;
use crate::meshlet::RayRequests;
use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// Samples per side of a cascade (`N` in `water.slang`).
pub const WATER_SIZE: u32 = 256;
/// Mip levels of a cascade's images, down to one texel: the mean displacement, slope and
/// squared slope over ever larger areas, for the surface far away.
pub const WATER_MIPS: u32 = WATER_SIZE.ilog2() + 1;
/// Complex values per sample in the work buffer (`FIELDS`).
const FIELDS: u64 = 4;
/// Threads per side of `evolve` and `derive`'s workgroups.
const GROUP: u32 = 16;

/// Mirrors `WaterPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterPush {
    spectrum: u64,
    work: u64,
    time: f32,
    patch: f32,
    choppiness: f32,
    displacement: u32,
    slopes: u32,
    pad0: u32,
    pad1: [u32; 2],
}

/// Mirrors `MipsPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MipsPush {
    displacement: u32,
    slopes: u32,
    displacement_out: u32,
    slopes_out: u32,
    level: u32,
    size: u32,
    pad: [u32; 2],
}

/// One cascade's spectrum and layout, from the CPU.
#[derive(Clone, Debug)]
pub struct WaterCascadeDesc {
    /// The patch's side, metres.
    pub patch: f32,
    /// The horizontal displacement's scale.
    pub choppiness: f32,
    /// Per sample in index order: `h0(k)` (real, imaginary), `ω(k)`, 0
    /// (`forge_procgen::Ocean::gpu_samples`), [`WATER_SIZE`]² of them.
    pub samples: Vec<[f32; 4]>,
    /// The angular frequency the cascade's energy centres on, rad/s
    /// (`forge_procgen::Ocean::mean_frequency`): the shore damps the cascade by the root of the
    /// TMA factor there (`forge_procgen::tma`, mirrored in `water.slang`).
    pub omega: f32,
    /// The TMA factor at `omega` in the depth the spectrum was made for (1 in deep water): the
    /// shore damps relative to it, so the open sea keeps its waves.
    pub shelf: f32,
}

/// A cascade's fields at one sample, as [`WaterCascades::read_fields`] returns them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaterSample {
    /// The height above the mean sea, metres.
    pub height: f32,
    /// The displacement along +x, metres.
    pub dx: f32,
    /// The displacement along the patch's +y (the world's +z), metres.
    pub dy: f32,
    /// `∂h/∂x`.
    pub slope_x: f32,
    /// `∂h/∂y`.
    pub slope_y: f32,
    /// The Jacobian of the displaced surface (below zero the wave folds over).
    pub jacobian: f32,
}

struct Cascade {
    spectrum: Buffer,
    work: GraphBuffer,
    displacement: GraphImage,
    slopes: GraphImage,
    patch: f32,
    choppiness: f32,
    omega: f32,
    shelf: f32,
}

/// What this frame's passes write, for the surface pass to read.
#[derive(Clone, Debug)]
pub struct WaterFrame {
    /// Per cascade, with their mips: the displacement image (x, height, y) and the slopes
    /// image (∂h/∂x, ∂h/∂y, the Jacobian, the squared slope).
    pub cascades: Vec<(ImageHandle, ImageHandle)>,
}

/// The cascades: their spectra, work buffers and images, and the passes that update them.
pub struct WaterCascades {
    evolve: Pipeline,
    rows: Pipeline,
    cols: Pipeline,
    derive: Pipeline,
    mips: Pipeline,
    cascades: Vec<Cascade>,
}

impl WaterCascades {
    /// Compiles the passes and uploads the cascades' spectra.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        descs: &[WaterCascadeDesc],
    ) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("water.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<WaterPush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let samples = u64::from(WATER_SIZE * WATER_SIZE);
        let image = |name: &str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: WATER_SIZE,
                    height: WATER_SIZE,
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: WATER_MIPS,
                    name,
                },
            )
        };
        let cascades = descs
            .iter()
            .enumerate()
            .map(|(c, desc)| {
                assert_eq!(
                    desc.samples.len() as u64,
                    samples,
                    "a cascade's spectrum has WATER_SIZE² samples"
                );
                Ok(Cascade {
                    spectrum: device.create_buffer_with_data(
                        &desc.samples,
                        vk::BufferUsageFlags::STORAGE_BUFFER,
                        MemoryCategory::Work,
                        &format!("water spectrum {c}"),
                    )?,
                    work: GraphBuffer::new(device.create_buffer(BufferDesc {
                        size: samples * FIELDS * 8,
                        usage: vk::BufferUsageFlags::STORAGE_BUFFER
                            | vk::BufferUsageFlags::TRANSFER_SRC,
                        location: MemoryLocation::GpuOnly,
                        category: MemoryCategory::Work,
                        name: &format!("water fields {c}"),
                    })?),
                    displacement: image(&format!("water displacement {c}"))?,
                    slopes: image(&format!("water slopes {c}"))?,
                    patch: desc.patch,
                    choppiness: desc.choppiness,
                    omega: desc.omega,
                    shelf: desc.shelf,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            evolve: compute("evolve_main", "water evolve")?,
            rows: compute("fft_rows_main", "water fft rows")?,
            cols: compute("fft_cols_main", "water fft cols")?,
            derive: compute("derive_main", "water derive")?,
            mips: {
                let module = device.create_shader_module(
                    &shaders.compile("water.slang", "mips_main", ShaderStage::Compute)?,
                    "water mips",
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, "mips_main"),
                    push_constant_bytes: std::mem::size_of::<MipsPush>() as u32,
                    name: "water mips",
                });
                device.destroy_shader_module(module);
                pipeline?
            },
            cascades,
        })
    }

    /// The cascades.
    pub fn len(&self) -> usize {
        self.cascades.len()
    }

    /// Whether there is none.
    pub fn is_empty(&self) -> bool {
        self.cascades.is_empty()
    }

    /// Bytes of the spectra, the work buffers and the images.
    pub fn bytes(&self) -> u64 {
        let texels = u64::from(WATER_SIZE * WATER_SIZE);
        self.cascades
            .iter()
            .map(|c| c.spectrum.size() + c.work.size() + 2 * texels * 8 * 4 / 3)
            .sum()
    }

    /// Declares this frame's passes, the waves at `time` seconds, on the async compute queue
    /// (they depend on nothing in the frame's geometry), and returns the images they write.
    pub fn update<'f>(&'f self, graph: &mut FrameGraph<'f>, time: f32) -> WaterFrame {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let imported: Vec<_> = self
            .cascades
            .iter()
            .map(|c| {
                (
                    graph.import_buffer(&c.work),
                    graph.import(&c.displacement),
                    graph.import(&c.slopes),
                )
            })
            .collect();
        let push = |c: &Cascade| WaterPush {
            spectrum: c.spectrum.address(),
            work: c.work.address(),
            time,
            patch: c.patch,
            choppiness: c.choppiness,
            displacement: c.displacement.storage(0).0,
            slopes: c.slopes.storage(0).0,
            pad0: 0,
            pad1: [0; 2],
        };
        let groups = WATER_SIZE / GROUP;
        // Each stage over every cascade, so a stage is one zone.
        let stages: [(&'static str, &'f Pipeline, [u32; 3]); 3] = [
            ("water/evolve", &self.evolve, [groups, groups, 1]),
            ("water/fft-rows", &self.rows, [WATER_SIZE, 1, 1]),
            ("water/fft-cols", &self.cols, [WATER_SIZE, 1, 1]),
        ];
        for (label, pipeline, groups) in stages {
            for (cascade, &(work, _, _)) in self.cascades.iter().zip(&imported) {
                let push = push(cascade);
                let access = if label == "water/evolve" {
                    BufferAccess::ShaderWrite(compute)
                } else {
                    BufferAccess::ShaderReadWrite(compute)
                };
                graph
                    .pass(label)
                    .queue(QueueKind::Compute)
                    .buffer(work, access)
                    .run(move |_, commands| {
                        commands.bind_pipeline(pipeline);
                        commands.push_constants(pipeline, &push);
                        commands.dispatch(groups[0], groups[1], groups[2]);
                        Ok(())
                    });
            }
        }
        let derive = &self.derive;
        for (cascade, &(work, displacement, slopes)) in self.cascades.iter().zip(&imported) {
            let push = push(cascade);
            graph
                .pass("water/derive")
                .queue(QueueKind::Compute)
                .buffer(work, BufferAccess::ShaderRead(compute))
                .image_mip(displacement, 0, ImageAccess::StorageWrite(compute))
                .image_mip(slopes, 0, ImageAccess::StorageWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(derive);
                    commands.push_constants(derive, &push);
                    commands.dispatch(groups, groups, 1);
                    Ok(())
                });
        }
        // The mips, each level from the one below, every cascade in one pass a level.
        let mips = &self.mips;
        for level in 1..WATER_MIPS {
            let size = WATER_SIZE >> level;
            let mut pass = graph.pass("water/mips").queue(QueueKind::Compute);
            for &(_, displacement, slopes) in &imported {
                pass = pass
                    .image_mip(displacement, level - 1, ImageAccess::Sampled(compute))
                    .image_mip(slopes, level - 1, ImageAccess::Sampled(compute))
                    .image_mip(displacement, level, ImageAccess::StorageWrite(compute))
                    .image_mip(slopes, level, ImageAccess::StorageWrite(compute));
            }
            let images: Vec<_> = imported.iter().map(|&(_, d, s)| (d, s)).collect();
            pass.run(move |resources, commands| {
                commands.bind_pipeline(mips);
                for &(displacement, slopes) in &images {
                    commands.push_constants(
                        mips,
                        &MipsPush {
                            displacement: resources.sampled(displacement).0,
                            slopes: resources.sampled(slopes).0,
                            displacement_out: resources.storage(displacement, level).0,
                            slopes_out: resources.storage(slopes, level).0,
                            level,
                            size,
                            pad: [0; 2],
                        },
                    );
                    commands.dispatch(size.div_ceil(8), size.div_ceil(8), 1);
                }
                Ok(())
            });
        }
        WaterFrame {
            cascades: imported.iter().map(|&(_, d, s)| (d, s)).collect(),
        }
    }

    /// Cascade `index`'s fields as the last submitted frame left them, in sample order.
    /// Waits for the device: for the start-up check and tests.
    pub fn read_fields(&self, device: &Arc<Device>, index: usize) -> Result<Vec<WaterSample>> {
        let work = &self.cascades[index].work;
        let bytes = device.read_back(work, 0, work.size())?;
        let values: &[[[f32; 2]; FIELDS as usize]] = bytemuck::cast_slice(&bytes);
        Ok(values
            .iter()
            .map(|p| {
                let (dxx, dyy, dxy) = (p[2][1], p[3][0], p[3][1]);
                WaterSample {
                    height: p[0][0],
                    dx: p[0][1],
                    dy: p[1][0],
                    slope_x: p[1][1],
                    slope_y: p[2][0],
                    jacobian: (1.0 + dxx) * (1.0 + dyy) - dxy * dxy,
                }
            })
            .collect())
    }

    /// Cascade `index`'s patch side, metres.
    pub fn patch(&self, index: usize) -> f32 {
        self.cascades[index].patch
    }
}

// ------------------------------------------------------------------- surface ---

/// Cascades the surface reads (`WATER_CASCADES` in `water.slang`).
const SURFACE_CASCADES: usize = 3;
/// Clipmap levels the block holds (`WATER_MAX_LEVELS`).
const MAX_LEVELS: usize = 16;
/// Quads a side of a clipmap level: a multiple of 4, so a level's hole falls on the next
/// level's lattice.
const GRID: u32 = 128;
/// Metres between the finest level's vertices.
const FINEST_SPACING: f64 = 0.5;
/// Levels: the finest 64 m across, the coarsest 262 km (the stand-in sea's extent).
const LEVELS: u32 = 13;
/// Quads a side of a level's blocks, which a frame draws or skips by whether they can show.
const SURFACE_BLOCK: u32 = 32;
/// Blocks a level holds.
const SURFACE_BLOCKS: usize = ((GRID / SURFACE_BLOCK) * (GRID / SURFACE_BLOCK)) as usize;
/// Metres a block's box grows by every way: the waves' reach (the cascades' displacement, the
/// shore's trains and the swash stay well under it).
const SURFACE_BLOCK_MARGIN: f32 = 40.0;

/// Per block of a set of the surface's quads, its first index and its count.
type SurfaceRanges = [(u32, u32); SURFACE_BLOCKS];

/// The surface's quads as indices into a level's `(GRID + 1)²` vertices (row-major from the
/// level's corner), two triangles each as `surface_vert_main` reads them. Five sets: the whole
/// level (the finest), then a level less the quads its finer level covers, for each way the
/// finer level sits on its lattice: the finer level is centred on the camera snapped to this
/// level's spacing and this one to twice it, so its hole starts a quarter of the way in, or a
/// quad further, along each axis (set `1 + x + 2 z`). Each set is laid out block after block,
/// so neighbouring blocks are a run of indices. Returns the indices and each set's ranges.
fn surface_indices() -> (Vec<u16>, Vec<SurfaceRanges>) {
    let side = GRID + 1;
    let blocks = GRID / SURFACE_BLOCK;
    let mut indices: Vec<u16> = Vec::new();
    let mut sets = Vec::new();
    for set in 0..5u32 {
        let hole = (set > 0).then(|| {
            let (kx, kz) = ((set - 1) & 1, (set - 1) >> 1);
            (
                GRID / 4 + kx..3 * GRID / 4 + kx,
                GRID / 4 + kz..3 * GRID / 4 + kz,
            )
        });
        let mut ranges = [(0, 0); SURFACE_BLOCKS];
        for (b, range) in ranges.iter_mut().enumerate() {
            let (bx, bz) = (b as u32 % blocks, b as u32 / blocks);
            let first = indices.len() as u32;
            for j in bz * SURFACE_BLOCK..(bz + 1) * SURFACE_BLOCK {
                for i in bx * SURFACE_BLOCK..(bx + 1) * SURFACE_BLOCK {
                    if hole
                        .as_ref()
                        .is_some_and(|(x, z)| x.contains(&i) && z.contains(&j))
                    {
                        continue;
                    }
                    let at = |di: u32, dj: u32| ((j + dj) * side + i + di) as u16;
                    // `QUAD_CORNERS` in `water.slang`: (0, 0), (0, 1), (1, 0), (1, 0), (0, 1),
                    // (1, 1), as (x, z).
                    indices.extend([at(0, 0), at(0, 1), at(1, 0), at(1, 0), at(0, 1), at(1, 1)]);
                }
            }
            *range = (first, indices.len() as u32 - first);
        }
        sets.push(ranges);
    }
    (indices, sets)
}

/// Level `l`'s centre relative to the camera at (`x`, `z`): the camera snapped to twice the
/// level's spacing, in f64.
fn surface_centre(x: f64, z: f64, l: usize) -> [f64; 2] {
    let spacing = FINEST_SPACING * f64::from(1u32 << l);
    let snap = |c: f64| (c / (2.0 * spacing)).floor() * 2.0 * spacing - c;
    [snap(x), snap(z)]
}

/// The set of [`surface_indices`] level `l` draws, from the levels' `centres`: the whole level
/// for the finest, else the one whose hole sits where the finer level does (its centre less
/// this one's is 0 or 1 of this level's spacings along each axis).
fn surface_set(centres: &[[f64; 2]], l: usize) -> usize {
    if l == 0 {
        return 0;
    }
    let spacing = FINEST_SPACING * f64::from(1u32 << l);
    let k = |a: usize| ((centres[l - 1][a] - centres[l][a]) / spacing).round() as usize;
    1 + k(0) + 2 * k(1)
}
/// The surface's second and third targets: per pixel, the mirror ray it asks for (direction,
/// weight) and the shadow ray (the sun's share of the colour, 1).
const REQUEST_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// Mirrors `WaterCascadeView` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuWaterCascadeView {
    offset: [f32; 2],
    inv_patch: f32,
    texel: f32,
    displacement: u32,
    slopes: u32,
    omega: f32,
    shelf: f32,
}

/// Mirrors `WaterSurface` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuWaterSurface {
    view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    sun_radiance: [f32; 4],
    absorption: [f32; 4],
    scatter: [f32; 4],
    shore_frame: [f32; 4],
    trains: [[f32; 4]; MAX_TRAINS],
    cascades: [GpuWaterCascadeView; SURFACE_CASCADES],
    levels: [[f32; 4]; MAX_LEVELS],
    level_count: u32,
    grid: u32,
    scene_color: u32,
    scene_depth: u32,
    width: u32,
    height: u32,
    shore: u32,
    shore_texels: u32,
    sky: u64,
    sky_light: u64,
    shore_table: u64,
    shore_bins: u32,
    train_count: u32,
    shore_bin: f32,
    time: f32,
    pixel: f32,
    river_points: u32,
    ground: u64,
    rivers: u64,
    mouths: u64,
    mouth_count: u32,
    stone_count: u32,
    stones: u64,
    mouth_grid: u64,
    mouth_cells: u32,
    mouth_cell: f32,
    lakes: u64,
    lake_masks: u64,
    lake_count: u32,
    pad: u32,
    at_camera: u64,
    pad_at_camera: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuWaterSurface>() == 752);

/// Bytes of `WaterAtCamera` in `water.slang`: the water's surface at the camera as a plane,
/// its absorption and its scattering, then `water/under`'s dispatch.
const AT_CAMERA_BYTES: u64 = 64;
/// Where `water/under`'s dispatch starts in `WaterAtCamera`.
const AT_CAMERA_DISPATCH: u64 = 48;

/// Metres over the sea's mean level under which the camera may be under its water (the waves'
/// crests and the shore's trains stand well under it): the frame then finds the water at the
/// camera and draws the view from under it (#108).
const UNDER_REACH: f64 = 20.0;

/// Mirrors `Lake` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLake {
    /// The mask's first sample (world x, z), the level, metres between samples.
    a: [f32; 4],
    /// The mask's samples along x and z, its first word in the masks' bits, its depth (f32 bits).
    b: [u32; 4],
}

/// A lake's water ([`WaterShore::lakes`], `forge_procgen::LakeWater`): a level plane over the
/// samples of its mask, where the ground rising through it draws the shore.
#[derive(Clone, Debug)]
pub struct WaterLake {
    /// The water's level, metres.
    pub level: f32,
    /// Its deepest point under the level, metres: deeper than that (and a margin), the view ray
    /// finds nothing under it because the ground is not drawn there yet.
    pub depth: f32,
    /// World x and z (the sea's frame) of the mask's first sample, metres.
    pub origin: [f32; 2],
    /// The mask's samples along x and z, the shore's spacing apart.
    pub size: [u32; 2],
    /// Per sample, row-major along +z: whether the water may stand there.
    pub mask: Vec<bool>,
}

/// How far a river's plume reaches in its half widths, and how fast it spreads
/// (`RIVER_PLUME_LENGTH` and `RIVER_PLUME_SPREAD` in `water.slang`).
const PLUME_LENGTH: f32 = 60.0;
const PLUME_SPREAD: f32 = 0.2;
/// How far a plume's axis wanders, metres a metre out at most (`RIVER_PLUME_WANDER`).
const PLUME_WANDER: f32 = 0.25;

/// Cells a side of the grid that lists the mouths whose plume reaches each cell, and the mouths
/// a cell lists at most (a byte each in a `u32`, `0xFF` for none).
const MOUTH_CELLS: u32 = 128;
const MOUTHS_PER_CELL: usize = 4;

/// The grid of the mouths whose plumes reach each cell of the shore's square (`mouth_grid` in
/// `water.slang`): per cell, up to four indices into `mouths` in a `u32`, `0xFF` where there
/// is none. The plume's box: from 200 m up the river's channel to its reach out to sea, its
/// width there either side.
fn mouth_grid(mouths: &[WaterMouth], origin: [f32; 2], extent: f32) -> Vec<u32> {
    let n = MOUTH_CELLS as usize;
    let cell = extent / MOUTH_CELLS as f32;
    let mut grid = vec![u32::MAX; n * n];
    for (index, m) in mouths.iter().enumerate().take(MAX_MOUTHS) {
        let reach = PLUME_LENGTH * m.half_width;
        let spread = m.half_width + (PLUME_SPREAD + PLUME_WANDER) * reach + 3.0;
        let (down, side) = (m.direction, [-m.direction[1], m.direction[0]]);
        let corners = [
            (-200.0, -spread),
            (-200.0, spread),
            (reach, -spread),
            (reach, spread),
        ]
        .map(|(a, c)| {
            [
                m.position[0] + down[0] * a + side[0] * c,
                m.position[1] + down[1] * a + side[1] * c,
            ]
        });
        let range = |axis: usize| {
            let lo = corners.iter().map(|p| p[axis]).fold(f32::MAX, f32::min);
            let hi = corners.iter().map(|p| p[axis]).fold(f32::MIN, f32::max);
            let at = |v: f32| {
                (((v - origin[axis]) / cell).floor() as i64).clamp(0, n as i64 - 1) as usize
            };
            at(lo)..=at(hi)
        };
        for y in range(1) {
            for x in range(0) {
                let slot = &mut grid[y * n + x];
                if let Some(free) =
                    (0..MOUTHS_PER_CELL).find(|&k| (*slot >> (8 * k)) & 0xFF == 0xFF)
                {
                    *slot = (*slot & !(0xFF << (8 * free))) | ((index as u32) << (8 * free));
                }
            }
        }
    }
    grid
}

/// Mirrors `RiverStone` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuRiverStone {
    /// World x, z, the radius at the water's level (0 under it), the mean radius.
    a: [f32; 4],
}

/// A stone in a river ([`WaterShore::stones`]): the water flows around it where it breaks the
/// surface, and breaks white on it.
#[derive(Clone, Copy, Debug)]
pub struct WaterStone {
    /// World x and z (the sea's frame), metres.
    pub position: [f32; 2],
    /// The radius of its outline at the water's level, metres (0 where it stands under water).
    pub waterline: f32,
    /// Its mean radius, metres.
    pub radius: f32,
    /// The river point it stands past, counting the points of every river in
    /// [`WaterShore::rivers`] one river after the other.
    pub point: u32,
}

/// Mirrors `RiverMouth` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuRiverMouth {
    /// World x, z, downstream (x, z).
    a: [f32; 4],
    /// The half width, the speed, the white water's share, 0.
    b: [f32; 4],
}

/// Where a river's water meets the sea's ([`WaterShore::mouths`]): from there the sea's surface
/// carries the river's flow and its water out into a plume.
#[derive(Clone, Copy, Debug)]
pub struct WaterMouth {
    /// World x and z (the sea's frame), metres: the river's point whose level reaches the sea's.
    pub position: [f32; 2],
    /// Downstream, unit (world x and z).
    pub direction: [f32; 2],
    /// Half the river's width there, metres.
    pub half_width: f32,
    /// Its speed there, m/s; negative where the water is drawn into the river rather than
    /// carried out (a lake's outlet, its direction pointing into the lake).
    pub speed: f32,
    /// How much of it runs white there, 0..1: the sea carries the white water on and out along
    /// the plume, fading.
    pub white: f32,
}

/// Quads across a river's ribbon (`RIVER_ACROSS` in `water.slang`).
const RIVER_ACROSS: u32 = 4;

/// Mirrors `RiverPoint` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuRiverPoint {
    /// World x, z, the half width at the water's edge, the depth.
    a: [f32; 4],
    /// Downstream (x, z), the speed, the water surface's slope.
    b: [f32; 4],
    /// The fade, the half width drawn whole where a segment starts (0 at a river's last point:
    /// none starts), the ground under the last vertex across, the water's level.
    c: [f32; 4],
    /// The ground under the first four vertices across.
    d: [f32; 4],
    /// The ribbon's half width (under the banks), the lowest bank before the carve, the first
    /// stone past the point, the metres along the river from its head.
    e: [f32; 4],
    /// The white water a step's fall leaves, and how far its line bows downstream in the middle
    /// and towards the left bank (#122); one unused.
    f: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuRiverPoint>() == 96);

/// A point of a river's ribbon (`forge_procgen::RibbonPoint`), as the surface draws it.
#[derive(Clone, Copy, Debug)]
pub struct WaterRiverPoint {
    /// World x and z (the sea's frame), metres, on the river's smoothed course.
    pub position: [f32; 2],
    /// The water's level, metres: level across the river.
    pub level: f32,
    /// Downstream, unit (world x and z).
    pub direction: [f32; 2],
    /// Half the river's width at its level, metres: where the water meets its banks.
    pub half_width: f32,
    /// Half the width over which its water is drawn whole, metres: the half width, more where it
    /// fills a confluence's rounded corner (`forge_procgen::Corner`, #119).
    pub cover: f32,
    /// Half the ribbon's width, metres: past the water's edge, under the banks.
    pub reach: f32,
    /// The water's depth in the middle, metres.
    pub depth: f32,
    /// The lowest the ground stands on the banks before the channel is carved, metres.
    pub bank: f32,
    /// The water's speed, m/s.
    pub speed: f32,
    /// The water surface's slope downstream.
    pub slope: f32,
    /// The white water a step's fall leaves, 0..1 (`forge_procgen::RibbonPoint::foam`, #122).
    pub foam: f32,
    /// At a step's points, how far its line bows downstream: in the middle, and towards the left
    /// bank (negative: the right), metres (`forge_procgen::RibbonPoint::lip`, #122).
    pub lip: [f32; 2],
    /// How much of the river is drawn there, 0..1 (it fades in from its head, and out into a
    /// lake, the river it joins or the sea).
    pub fade: f32,
    /// Per vertex across, from `position − side × reach` to `position + side × reach` with
    /// `side = (−direction.z, direction.x)`: the highest the drawn ground stands under the quads
    /// around the vertex, metres (`forge_procgen::RibbonPoint::ground`), where the water lies
    /// far away.
    pub ground: [f32; RIVER_ACROSS as usize + 1],
}

/// Shore trains the surface draws at most (`WATER_MAX_TRAINS` in `water.slang`).
const MAX_TRAINS: usize = 4;

/// River mouths the sea's surface mixes in at most.
const MAX_MOUTHS: usize = 64;

/// Mirrors `CausticCascade` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCausticCascade {
    /// 1 / the patch's side, metres a texel, ω, the TMA factor at the spectrum's depth.
    a: [f32; 4],
    /// The slopes' sampled index, 0, 0, 0.
    b: [u32; 4],
}

/// Mirrors `ShoreGround` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuShoreGround {
    frame: [f32; 4],
    trains: [[f32; 4]; MAX_TRAINS],
    shore: u32,
    train_count: u32,
    time: f32,
    pad: f32,
    sun_water: [f32; 4],
    cascades: [GpuCausticCascade; SURFACE_CASCADES],
    cascade_count: u32,
    pad_caustics: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<GpuShoreGround>() == 224);

/// What the ground's shading reads of the shore in a frame ([`WaterSurface::wet_ground`]): the
/// sand is wet where the swash ran up (issue #105), and the floor under the sea takes the
/// waves' caustics (#108).
#[derive(Clone, Copy, Debug)]
pub struct WetGround {
    /// The shore's fields (the floor's height, the coast distance), which the shading samples.
    pub image: ImageHandle,
    /// The cascades' slopes of this frame, which the caustics sample (none without them).
    pub slopes: [Option<ImageHandle>; SURFACE_CASCADES],
    /// The frame's `ShoreGround` block (`meshlet.slang`): the fields' frame, the trains, the
    /// sea's time, the cascades and the sun under the water.
    pub address: u64,
}

impl WetGround {
    /// The images the ground's shading samples: the shore's fields, and the cascades' slopes
    /// for the caustics.
    pub fn images(&self) -> impl Iterator<Item = ImageHandle> {
        std::iter::once(self.image).chain(self.slopes.into_iter().flatten())
    }
}

/// What the floor's caustics need of a frame ([`WaterSurface::wet_ground`], #108).
#[derive(Clone, Copy)]
pub struct WaterCaustics<'a> {
    /// The sea's cascades.
    pub cascades: &'a WaterCascades,
    /// Their images of this frame ([`WaterCascades::update`]).
    pub waves: &'a WaterFrame,
    /// Towards the sun.
    pub sun_dir: Vec3,
}

/// Metres of the largest patch whose waves throw caustics (#108). The swell's patch (1 km of
/// waves 80 m long) curves the surface too gently: under 1 % of the light moves at 3 m.
const CAUSTICS_MAX_PATCH: f32 = 256.0;

/// The way the sun's light runs under a level water surface (unit, downwards): its ray from
/// `towards_sun` bent into the water (Snell's law), or straight down with the sun under the
/// horizon.
fn sun_under_water(towards_sun: Vec3) -> Vec3 {
    let l = towards_sun.normalize_or(Vec3::Y);
    if l.y <= 0.0 {
        return Vec3::NEG_Y;
    }
    let eta = 1.0 / 1.333;
    let k = 1.0 - eta * eta * (1.0 - l.y * l.y);
    (-l * eta + Vec3::Y * (eta * l.y - k.sqrt())).normalize()
}

/// One of the shore's wave trains (`forge_procgen::ShoreTrain`), as the surface draws it.
#[derive(Clone, Copy, Debug)]
pub struct WaterShoreTrain<'a> {
    /// Radians a second.
    pub omega: f32,
    /// The height in deep water, crest to trough, metres.
    pub height: f32,
    /// Per boundary of the shore's bins, out from the shore: the time a crest takes from there
    /// to the shore (seconds) and the shoaling coefficient (`ShoreTrain::table`).
    pub table: &'a [[f32; 2]],
}

/// The sea floor and the coast under the sea (issue #105's shore), on a square grid of samples:
/// what the surface damps its waves by, and the trains of waves that come in to the shore.
#[derive(Clone, Copy, Debug)]
pub struct WaterShore<'a> {
    /// Samples a side.
    pub texels: u32,
    /// Metres between samples.
    pub spacing: f32,
    /// The world x and z of the first sample (the sea's frame, metres).
    pub origin: [f32; 2],
    /// Per sample, row-major along +z: the floor's height, metres (the sea's level at 0, the
    /// land above it).
    pub floor: &'a [f32],
    /// Per sample: the signed distance to the coast, metres, positive inland
    /// (`forge_procgen::coast_distance`).
    pub coast: &'a [f32],
    /// Metres a bin of the trains' tables.
    pub bin: f32,
    /// The trains, [`MAX_TRAINS`] at most, their tables of one length.
    pub trains: &'a [WaterShoreTrain<'a>],
    /// The rivers over the land, each a ribbon of points from its head, in the order they are
    /// drawn (a tributary before the river that covers its end). They lie on the ground that
    /// `floor` describes, drawn as the island's mesh draws it.
    pub rivers: &'a [Vec<WaterRiverPoint>],
    /// Where the rivers meet the sea (`MAX_MOUTHS` at most).
    pub mouths: &'a [WaterMouth],
    /// The stones in the rivers, in the order of the points they stand past.
    pub stones: &'a [WaterStone],
    /// The lakes.
    pub lakes: &'a [WaterLake],
}

/// Mirrors `SurfacePush` and `CopyPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfacePush {
    surface: u64,
    /// The sea: the clipmap level drawn. The rivers: the first vertex of the run of segments
    /// drawn (the vertex index's offset).
    first: u32,
    pad: u32,
}

/// Segments of the rivers in a run the frame draws or skips together.
const RIVER_CHUNK: u32 = 64;

/// Metres the box of a run of segments grows by: far away a ribbon widens to a pixel's footprint
/// either side and rises by one and a half of it, which over the island's 16 km stays under this.
const RIVER_CHUNK_MARGIN: f32 = 25.0;

/// A run of the rivers' segments and the box it can be drawn in (the sea's frame).
#[derive(Clone, Copy, Debug)]
struct RiverChunk {
    /// The river it is part of (a chunk never spans two).
    river: u32,
    first: u32,
    count: u32,
    lo: Vec3,
    hi: Vec3,
}

/// The runs of at most [`RIVER_CHUNK`] segments of each of `rivers` (one after the other as
/// uploaded, the tributaries first), each with its box: the points' positions out to their
/// reach, their level and the ground they rest on far away, and [`RIVER_CHUNK_MARGIN`] more.
fn river_chunks(rivers: &[Vec<WaterRiverPoint>]) -> Vec<RiverChunk> {
    let points: Vec<&WaterRiverPoint> = rivers.iter().flatten().collect();
    let mut runs = Vec::new();
    let mut start = 0_u32;
    for (river, r) in rivers.iter().enumerate() {
        // A river's segments: from each of its points but the last.
        let segments = (r.len() as u32).saturating_sub(1);
        let mut first = start;
        while first < start + segments {
            let count = RIVER_CHUNK.min(start + segments - first);
            runs.push((river as u32, first, count));
            first += count;
        }
        start += r.len() as u32;
    }
    runs.into_iter()
        .map(|(river, first, count)| {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &points[first as usize..=(first + count) as usize] {
                let ground = p.ground.iter().copied();
                let low = ground.clone().fold(p.level, f32::min);
                let high = ground.fold(p.level, f32::max);
                lo = lo.min(Vec3::new(
                    p.position[0] - p.reach,
                    low,
                    p.position[1] - p.reach,
                ));
                hi = hi.max(Vec3::new(
                    p.position[0] + p.reach,
                    high,
                    p.position[1] + p.reach,
                ));
            }
            RiverChunk {
                river,
                first,
                count,
                lo: lo - RIVER_CHUNK_MARGIN,
                hi: hi + RIVER_CHUNK_MARGIN,
            }
        })
        .collect()
}

/// Whether a box (camera-relative) can show through `view_proj` (reversed Z, an infinite far
/// plane): it is not wholly outside one of the left, right, bottom, top and near planes.
fn box_in_view(view_proj: &Mat4, lo: Vec3, hi: Vec3) -> bool {
    let rows = [0, 1, 2, 3].map(|i| view_proj.row(i));
    let planes = [
        rows[3] + rows[0],
        rows[3] - rows[0],
        rows[3] + rows[1],
        rows[3] - rows[1],
        rows[3] - rows[2],
    ];
    planes.iter().all(|p| {
        // The box's corner furthest along the plane's normal.
        let far = Vec3::new(
            if p.x > 0.0 { hi.x } else { lo.x },
            if p.y > 0.0 { hi.y } else { lo.y },
            if p.z > 0.0 { hi.z } else { lo.z },
        );
        p.x * far.x + p.y * far.y + p.z * far.z + p.w >= 0.0
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CopyPush {
    color: u32,
    depth: u32,
    color_out: u32,
    depth_out: u32,
    width: u32,
    height: u32,
    pad: [u32; 2],
}

/// Mirrors `AtCameraPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AtCameraPush {
    surface: u64,
}

/// Mirrors `UnderPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UnderPush {
    surface: u64,
    color: u32,
    depth: u32,
    width: u32,
    height: u32,
}

/// What a frame's sea surface needs besides the cascades and the sky.
#[derive(Clone, Copy, Debug)]
pub struct WaterSurfaceParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The camera in the sea's frame, metres: the sea's mean level is y = 0.
    pub camera: DVec3,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sun's pre-exposed illuminance on a surface facing it, per channel (the resolve's
    /// sun colour times its illuminance times the exposure).
    pub sun_radiance: Vec3,
    /// The pre-exposed luminance of a unit of sun illuminance (the sky's scale).
    pub sky_scale: f32,
    /// The sea's clock, seconds (the cascades' time): the shore's trains move with it.
    pub time: f32,
    /// Metres a pixel spans a metre away (the vertical field of view over the image's height):
    /// far away, the rivers stay a pixel wide either side.
    pub pixel: f32,
}

/// The sea's surface (issue #105, step 2): a clipmap of grids around the camera displaced by
/// the cascades, drawn after the sky's compose (`water/surface`), with the scene it lets
/// through copied first (`water/scene-copy`). It reflects the sky and takes the sun's light,
/// and asks for the rays that bring the scene in ([`RayRequests`]): the mirror rays that put it
/// in the reflection (step 3), and the shadow rays that take the sun away where it shades the
/// water (step 4). With the shore come the rivers, ribbons drawn in the same pass over the land.
/// When the camera comes down near the sea it can look out from under it (#108): the water at
/// the camera is found first (`water/at-camera`), the surface shades as seen from below where a
/// pixel looks out from under it, and the water between the camera and what it meets is added
/// after (`water/under`).
pub struct WaterSurface {
    copy: Pipeline,
    surface: Pipeline,
    /// The surface within reach of the camera: seen from below where a pixel looks out from
    /// under it.
    surface_under: Pipeline,
    rivers: Pipeline,
    lakes: Pipeline,
    at_camera_pass: Pipeline,
    under: Pipeline,
    /// The water at the camera (`WaterAtCamera`), which `water/at-camera` writes.
    at_camera: GraphBuffer,
    blocks: Vec<Buffer>,
    /// The surface's quads as indices into a level's vertices ([`surface_indices`]), and each
    /// set's ranges per block.
    surface_indices: Buffer,
    surface_ranges: Vec<SurfaceRanges>,
    /// Without it the sea is deep everywhere.
    shore: Option<ShoreFields>,
}

/// The shore on the GPU ([`WaterShore`]).
struct ShoreFields {
    /// `RG16F`: the floor's height, the coast distance.
    image: GraphImage,
    /// The first sample's x and z, 1 / spacing, 0.
    frame: [f32; 4],
    texels: u32,
    /// The trains' tables one after the other, `bins` entries each (none without trains).
    tables: Option<Buffer>,
    /// Per train: ω, the deep-water height, a phase offset, 0.
    trains: [[f32; 4]; MAX_TRAINS],
    train_count: u32,
    bins: u32,
    bin: f32,
    /// Per frame slot, the ground's `ShoreGround` block.
    ground: Vec<Buffer>,
    /// The rivers' points one river after the other, and the floor's heights in full
    /// precision, which the ribbons lie on (none without rivers).
    rivers: Option<(Buffer, Buffer)>,
    river_points: u32,
    /// The rivers' segments in runs of [`RIVER_CHUNK`], each with the box it can be drawn in, so
    /// a frame draws only those in view.
    river_chunks: Vec<RiverChunk>,
    /// The rivers' mouths at the sea, and the grid of those reaching each cell (none without).
    mouths: Option<(Buffer, Buffer)>,
    mouth_count: u32,
    /// Metres a cell of the mouths' grid.
    mouth_cell: f32,
    /// The stones in the rivers (none without).
    stones: Option<Buffer>,
    stone_count: u32,
    /// The lakes and their masks' bits (none without).
    lakes: Option<(Buffer, Buffer)>,
    lake_count: u32,
}

impl WaterSurface {
    /// Compiles the passes and uploads the shore's fields.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        shore: Option<WaterShore<'_>>,
    ) -> Result<Self> {
        let copy_module = device.create_shader_module(
            &shaders.compile("water.slang", "copy_main", ShaderStage::Compute)?,
            "water scene copy",
        )?;
        let copy = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (copy_module, "copy_main"),
            push_constant_bytes: std::mem::size_of::<CopyPush>() as u32,
            name: "water scene copy",
        });
        device.destroy_shader_module(copy_module);
        let at_camera_module = device.create_shader_module(
            &shaders.compile("water.slang", "at_camera_main", ShaderStage::Compute)?,
            "water at the camera",
        )?;
        let at_camera_pass = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (at_camera_module, "at_camera_main"),
            push_constant_bytes: std::mem::size_of::<AtCameraPush>() as u32,
            name: "water at the camera",
        });
        device.destroy_shader_module(at_camera_module);
        let under_module = device.create_shader_module(
            &shaders.compile("water.slang", "under_main", ShaderStage::Compute)?,
            "water under",
        )?;
        let under = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (under_module, "under_main"),
            push_constant_bytes: std::mem::size_of::<UnderPush>() as u32,
            name: "water under",
        });
        device.destroy_shader_module(under_module);
        let at_camera = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: AT_CAMERA_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "water at the camera",
        })?);
        let vertex = device.create_shader_module(
            &shaders.compile("water.slang", "surface_vert_main", ShaderStage::Vertex)?,
            "water surface vertices",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("water.slang", "surface_frag_main", ShaderStage::Fragment)?,
            "water surface",
        )?;
        let surface_desc = |fragment, entry, name| VertexPipelineDesc {
            vertex: (vertex, "surface_vert_main"),
            fragment: (fragment, entry),
            color_formats: &[HDR_FORMAT, REQUEST_FORMAT, REQUEST_FORMAT],
            depth_format: Some(vk::Format::D32_SFLOAT),
            push_constant_bytes: std::mem::size_of::<SurfacePush>() as u32,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: true,
            alpha_blend: false,
            name,
        };
        let surface = device.create_vertex_pipeline(&surface_desc(
            fragment,
            "surface_frag_main",
            "water surface",
        ));
        // Within reach of the camera, the surface shades as seen from below where a pixel
        // looks out from under the water (#108).
        let under_fragment = device.create_shader_module(
            &shaders.compile(
                "water.slang",
                "surface_under_frag_main",
                ShaderStage::Fragment,
            )?,
            "water surface from below",
        )?;
        let surface_under = device.create_vertex_pipeline(&surface_desc(
            under_fragment,
            "surface_under_frag_main",
            "water surface from below",
        ));
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        device.destroy_shader_module(under_fragment);
        // The rivers blend over what is under them by how much of the pixel they cover: their
        // soft edges, their thinning far away.
        let river_vertex = device.create_shader_module(
            &shaders.compile("water.slang", "river_vert_main", ShaderStage::Vertex)?,
            "water river vertices",
        )?;
        let river_fragment = device.create_shader_module(
            &shaders.compile("water.slang", "river_frag_main", ShaderStage::Fragment)?,
            "water rivers",
        )?;
        let rivers = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (river_vertex, "river_vert_main"),
            fragment: (river_fragment, "river_frag_main"),
            color_formats: &[HDR_FORMAT, REQUEST_FORMAT, REQUEST_FORMAT],
            depth_format: Some(vk::Format::D32_SFLOAT),
            push_constant_bytes: std::mem::size_of::<SurfacePush>() as u32,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: true,
            alpha_blend: true,
            name: "water rivers",
        });
        device.destroy_shader_module(river_vertex);
        device.destroy_shader_module(river_fragment);
        // The lakes, drawn before the rivers so a river blends over a lake it runs into: a plane
        // each, clipped to its mask, its edge where the ground rises through it.
        let lake_vertex = device.create_shader_module(
            &shaders.compile("water.slang", "lake_vert_main", ShaderStage::Vertex)?,
            "water lake vertices",
        )?;
        let lake_fragment = device.create_shader_module(
            &shaders.compile("water.slang", "lake_frag_main", ShaderStage::Fragment)?,
            "water lakes",
        )?;
        let lakes = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (lake_vertex, "lake_vert_main"),
            fragment: (lake_fragment, "lake_frag_main"),
            color_formats: &[HDR_FORMAT, REQUEST_FORMAT, REQUEST_FORMAT],
            depth_format: Some(vk::Format::D32_SFLOAT),
            push_constant_bytes: std::mem::size_of::<SurfacePush>() as u32,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: true,
            alpha_blend: true,
            name: "water lakes",
        });
        device.destroy_shader_module(lake_vertex);
        device.destroy_shader_module(lake_fragment);
        let blocks = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuWaterSurface>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("water surface {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let (indices, surface_ranges) = surface_indices();
        let surface_indices = device.create_buffer_with_data(
            &indices,
            vk::BufferUsageFlags::INDEX_BUFFER,
            MemoryCategory::Geometry,
            "water surface indices",
        )?;
        let shore = shore
            .map(|s| -> Result<_> {
                let count = (s.texels as usize) * (s.texels as usize);
                assert!(
                    s.floor.len() == count && s.coast.len() == count,
                    "the shore's fields hold texels² samples"
                );
                let bytes: Vec<u8> = s
                    .floor
                    .iter()
                    .zip(s.coast)
                    .flat_map(|(&h, &d)| {
                        let [a, b] = f32_to_f16(h).to_le_bytes();
                        let [c, e] = f32_to_f16(d).to_le_bytes();
                        [a, b, c, e]
                    })
                    .collect();
                let image = GraphImage::uploaded(
                    device,
                    ImageDesc {
                        width: s.texels,
                        height: s.texels,
                        format: vk::Format::R16G16_SFLOAT,
                        usage: vk::ImageUsageFlags::SAMPLED,
                        aspect: vk::ImageAspectFlags::COLOR,
                        mip_levels: 1,
                        name: "water shore",
                    },
                    &bytes,
                )?;
                assert!(s.trains.len() <= MAX_TRAINS, "at most {MAX_TRAINS} trains");
                let bins = s.trains.first().map_or(0, |t| t.table.len());
                assert!(
                    s.trains.iter().all(|t| t.table.len() == bins)
                        && (s.trains.is_empty() || bins >= 2),
                    "the trains' tables have one length, two entries at least"
                );
                let table: Vec<[f32; 2]> = s
                    .trains
                    .iter()
                    .flat_map(|t| t.table.iter().copied())
                    .collect();
                let tables = (!table.is_empty())
                    .then(|| {
                        device.create_buffer_with_data(
                            &table,
                            vk::BufferUsageFlags::STORAGE_BUFFER,
                            MemoryCategory::Work,
                            "water shore trains",
                        )
                    })
                    .transpose()?;
                let mut trains = [[0.0; 4]; MAX_TRAINS];
                for (i, (slot, t)) in trains.iter_mut().zip(s.trains).enumerate() {
                    // Each train starts a third of a turn after the one before.
                    *slot = [t.omega, t.height, i as f32 * 2.1, 0.0];
                }
                let ground = (0..FRAMES_IN_FLIGHT)
                    .map(|i| {
                        device.create_buffer(BufferDesc {
                            size: std::mem::size_of::<GpuShoreGround>() as u64,
                            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                            location: MemoryLocation::CpuToGpu,
                            category: MemoryCategory::Frame,
                            name: &format!("water shore ground {i}"),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                // The rivers' points, each marking whether a segment starts there and where the
                // stones past it start: per point, the first stone standing past it or a later
                // point (the stones are in the points' order).
                let total: usize = s.rivers.iter().map(Vec::len).sum();
                let mut first = vec![0u32; total + 1];
                for stone in s.stones {
                    first[(stone.point as usize + 1).min(total)] += 1;
                }
                for g in 0..total {
                    first[g + 1] += first[g];
                }
                let mut points: Vec<GpuRiverPoint> = Vec::with_capacity(total);
                for river in s.rivers {
                    let mut along = 0.0_f32;
                    for (k, p) in river.iter().enumerate() {
                        if k > 0 {
                            let q = river[k - 1].position;
                            along += (p.position[0] - q[0]).hypot(p.position[1] - q[1]);
                        }
                        points.push(GpuRiverPoint {
                            a: [p.position[0], p.position[1], p.half_width, p.depth],
                            b: [p.direction[0], p.direction[1], p.speed, p.slope],
                            c: [
                                p.fade,
                                if k + 1 < river.len() {
                                    p.cover.max(1e-3)
                                } else {
                                    0.0
                                },
                                p.ground[4],
                                p.level,
                            ],
                            d: [p.ground[0], p.ground[1], p.ground[2], p.ground[3]],
                            e: [p.reach, p.bank, first[points.len()] as f32, along],
                            f: [p.foam, p.lip[0], p.lip[1], 0.0],
                        });
                    }
                }
                let rivers = (!points.is_empty())
                    .then(|| -> Result<_> {
                        Ok((
                            device.create_buffer_with_data(
                                &points,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water rivers",
                            )?,
                            device.create_buffer_with_data(
                                s.floor,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water ground heights",
                            )?,
                        ))
                    })
                    .transpose()?;
                let mouths: Vec<GpuRiverMouth> = s
                    .mouths
                    .iter()
                    .take(MAX_MOUTHS)
                    .map(|m| GpuRiverMouth {
                        a: [m.position[0], m.position[1], m.direction[0], m.direction[1]],
                        b: [m.half_width, m.speed, m.white, 0.0],
                    })
                    .collect();
                let extent = s.texels.saturating_sub(1) as f32 * s.spacing;
                let grid = mouth_grid(s.mouths, s.origin, extent);
                let mouth_buffers = (!mouths.is_empty())
                    .then(|| -> Result<_> {
                        Ok((
                            device.create_buffer_with_data(
                                &mouths,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water river mouths",
                            )?,
                            device.create_buffer_with_data(
                                &grid,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water river mouths grid",
                            )?,
                        ))
                    })
                    .transpose()?;
                let stones: Vec<GpuRiverStone> = s
                    .stones
                    .iter()
                    .map(|t| GpuRiverStone {
                        a: [t.position[0], t.position[1], t.waterline, t.radius],
                    })
                    .collect();
                let stone_buffer = (!stones.is_empty())
                    .then(|| {
                        device.create_buffer_with_data(
                            &stones,
                            vk::BufferUsageFlags::STORAGE_BUFFER,
                            MemoryCategory::Work,
                            "water river stones",
                        )
                    })
                    .transpose()?;
                // The lakes and their masks, a bit a sample, each mask from a word of its own.
                let mut bits: Vec<u32> = Vec::new();
                let lakes: Vec<GpuLake> = s
                    .lakes
                    .iter()
                    .map(|lake| {
                        let first = bits.len() as u32;
                        bits.resize(bits.len() + lake.mask.len().div_ceil(32), 0);
                        for (k, &on) in lake.mask.iter().enumerate() {
                            if on {
                                bits[first as usize + k / 32] |= 1 << (k % 32);
                            }
                        }
                        GpuLake {
                            a: [lake.origin[0], lake.origin[1], lake.level, s.spacing],
                            b: [lake.size[0], lake.size[1], first, lake.depth.to_bits()],
                        }
                    })
                    .collect();
                let lake_buffers = (!lakes.is_empty())
                    .then(|| -> Result<_> {
                        Ok((
                            device.create_buffer_with_data(
                                &lakes,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water lakes",
                            )?,
                            device.create_buffer_with_data(
                                &bits,
                                vk::BufferUsageFlags::STORAGE_BUFFER,
                                MemoryCategory::Work,
                                "water lake masks",
                            )?,
                        ))
                    })
                    .transpose()?;
                Ok(ShoreFields {
                    image,
                    frame: [s.origin[0], s.origin[1], 1.0 / s.spacing, 0.0],
                    texels: s.texels,
                    tables,
                    trains,
                    train_count: s.trains.len() as u32,
                    bins: bins as u32,
                    bin: s.bin,
                    ground,
                    rivers,
                    river_points: points.len() as u32,
                    river_chunks: river_chunks(s.rivers),
                    mouths: mouth_buffers,
                    mouth_cell: extent / MOUTH_CELLS as f32,
                    mouth_count: mouths.len() as u32,
                    stones: stone_buffer,
                    stone_count: stones.len() as u32,
                    lakes: lake_buffers,
                    lake_count: lakes.len() as u32,
                })
            })
            .transpose()?;
        Ok(Self {
            copy: copy?,
            surface: surface?,
            surface_under: surface_under?,
            rivers: rivers?,
            lakes: lakes?,
            at_camera_pass: at_camera_pass?,
            under: under?,
            at_camera,
            blocks,
            surface_indices,
            surface_ranges,
            shore,
        })
    }

    /// Bytes of the rivers' points and of the heights they lie on (0 without rivers).
    pub fn river_bytes(&self) -> u64 {
        self.shore
            .as_ref()
            .and_then(|s| s.rivers.as_ref())
            .map_or(0, |(points, heights)| points.size() + heights.size())
    }

    /// The shore as the ground's shading reads it in this frame, at the sea's `time` (the
    /// surface's time): the sand is wet where the swash ran up (issue #105), and with
    /// `caustics` the floor under the sea takes the waves' caustics (#108). `None` without a
    /// shore. Pass it to the resolve ([`crate::AmbientLight::wet_ground`]).
    pub fn wet_ground<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        time: f32,
        caustics: Option<WaterCaustics<'_>>,
    ) -> Option<WetGround> {
        let shore = self.shore.as_ref()?;
        let block = &shore.ground[slot.index];
        // The caustics' cascades: those whose waves curve the surface enough to focus the light
        // (the swell's do not), first in the block.
        let mut cascades = [GpuCausticCascade::zeroed(); SURFACE_CASCADES];
        let mut slopes = [None; SURFACE_CASCADES];
        let mut cascade_count = 0;
        if let Some(c) = caustics {
            assert_eq!(
                c.cascades.len(),
                SURFACE_CASCADES,
                "the caustics read three cascades"
            );
            for (cascade, &(_, image)) in c.cascades.cascades.iter().zip(&c.waves.cascades) {
                if cascade.patch > CAUSTICS_MAX_PATCH {
                    continue;
                }
                cascades[cascade_count] = GpuCausticCascade {
                    a: [
                        1.0 / cascade.patch,
                        cascade.patch / WATER_SIZE as f32,
                        cascade.omega,
                        cascade.shelf,
                    ],
                    b: [cascade.slopes.sampled().0, 0, 0, 0],
                };
                slopes[cascade_count] = Some(image);
                cascade_count += 1;
            }
        }
        block.write(
            0,
            &[GpuShoreGround {
                frame: [
                    shore.frame[0],
                    shore.frame[1],
                    shore.frame[2],
                    shore.texels as f32,
                ],
                trains: shore.trains,
                shore: shore.image.sampled().0,
                train_count: shore.train_count,
                time,
                pad: 0.0,
                sun_water: caustics
                    .map_or(Vec3::NEG_Y, |c| sun_under_water(c.sun_dir))
                    .extend(0.0)
                    .to_array(),
                cascades,
                cascade_count: cascade_count as u32,
                pad_caustics: [0; 3],
            }],
        );
        Some(WetGround {
            image: graph.import(&shore.image),
            slopes,
            address: block.address(),
        })
    }

    /// Declares the copy of what the water lets through and the surface over `color` and
    /// `depth` (the frame's HDR image and depth, after the sky's compose), from the cascades'
    /// images of this frame (`waves`) and the sky's frame. Returns the rays the surface asks
    /// for, which [`crate::MeshletRenderer::trace_requested`] traces against the scene;
    /// untraced, the water reflects the sky alone and takes the sun everywhere.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        cascades: &'f WaterCascades,
        waves: &WaterFrame,
        sky: &SkyFrame,
        params: WaterSurfaceParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) -> RayRequests {
        assert_eq!(
            cascades.len(),
            SURFACE_CASCADES,
            "the surface reads three cascades"
        );
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let vertex = vk::PipelineStageFlags2::VERTEX_SHADER;
        let transient = |name, format| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let scene_color = graph.transient(transient("water scene color", HDR_FORMAT));
        let scene_depth = graph.transient(transient("water scene depth", vk::Format::R32_SFLOAT));
        let mut request = |name| {
            graph.transient(TransientDesc {
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                ..transient(name, REQUEST_FORMAT)
            })
        };
        let mirror_requests = request("water mirror ray requests");
        let sun_requests = request("water shadow ray requests");
        let copy = &self.copy;
        graph
            .pass("water/scene-copy")
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(scene_color, ImageAccess::StorageWrite(compute))
            .image(scene_depth, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(copy);
                commands.push_constants(
                    copy,
                    &CopyPush {
                        color: resources.sampled(color).0,
                        depth: resources.sampled(depth).0,
                        color_out: resources.storage(scene_color, 0).0,
                        depth_out: resources.storage(scene_depth, 0).0,
                        width: extent.width,
                        height: extent.height,
                        pad: [0; 2],
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });

        // The clipmap: each level centred on the camera snapped to twice its spacing, in f64,
        // then relative to the camera.
        let mut levels = [[0.0_f32; 4]; MAX_LEVELS];
        let mut centres = [[0.0_f64; 2]; LEVELS as usize];
        for (l, level) in levels.iter_mut().enumerate().take(LEVELS as usize) {
            centres[l] = surface_centre(params.camera.x, params.camera.z, l);
            *level = [
                centres[l][0] as f32,
                centres[l][1] as f32,
                (FINEST_SPACING * f64::from(1u32 << l)) as f32,
                0.0,
            ];
        }
        // The surface's blocks that can show, each level's from the set of quads its finer
        // level leaves (first index, indices, the level), the neighbours merged into one draw.
        let mut surface_runs: Vec<(u32, u32, u32)> = Vec::new();
        let half = (GRID / 2) as f32;
        for (l, &[x, z, spacing, _]) in levels.iter().enumerate().take(LEVELS as usize) {
            let set = surface_set(&centres, l);
            let level = l as u32;
            let blocks = GRID / SURFACE_BLOCK;
            for (b, &(first, count)) in self.surface_ranges[set].iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let (bx, bz) = ((b as u32 % blocks) as f32, (b as u32 / blocks) as f32);
                let block = SURFACE_BLOCK as f32;
                // The lattice the block spans, a spacing more towards the coarser lattice
                // (the vertices sliding onto it), and the waves' reach.
                let lo = Vec3::new(
                    x + (bx * block - half - 1.0) * spacing,
                    -params.camera.y as f32,
                    z + (bz * block - half - 1.0) * spacing,
                ) - SURFACE_BLOCK_MARGIN;
                let hi = Vec3::new(
                    x + ((bx + 1.0) * block - half) * spacing,
                    -params.camera.y as f32,
                    z + ((bz + 1.0) * block - half) * spacing,
                ) + SURFACE_BLOCK_MARGIN;
                if !box_in_view(&params.view_proj, lo, hi) {
                    continue;
                }
                match surface_runs.last_mut() {
                    Some((f, c, o)) if *o == level && *f + *c == first => *c += count,
                    _ => surface_runs.push((first, count, level)),
                }
            }
        }
        let views: Vec<GpuWaterCascadeView> = cascades
            .cascades
            .iter()
            .map(|c| {
                let patch = f64::from(c.patch);
                let offset = |v: f64| (v / patch).rem_euclid(1.0) as f32;
                GpuWaterCascadeView {
                    offset: [offset(params.camera.x), offset(params.camera.z)],
                    inv_patch: 1.0 / c.patch,
                    texel: c.patch / WATER_SIZE as f32,
                    displacement: c.displacement.sampled().0,
                    slopes: c.slopes.sampled().0,
                    omega: c.omega,
                    shelf: c.shelf,
                }
            })
            .collect();
        let block: &'f Buffer = &self.blocks[slot.index];
        let address = block.address();
        let sky_address = sky.address();
        let sky_light = sky.light.address;
        // Whether the camera stands low enough to be under the water (#108).
        let in_reach = params.camera.y < UNDER_REACH;
        let surface = if in_reach {
            &self.surface_under
        } else {
            &self.surface
        };
        let surface_indices = &self.surface_indices;
        let river_pipeline = &self.rivers;
        let lake_pipeline = &self.lakes;
        let shore: Option<&'f ShoreFields> = self.shore.as_ref();
        let rivers = shore.and_then(|s| s.rivers.as_ref());
        let lake_count = shore.map_or(0, |s| s.lake_count);
        let river_points = shore.map_or(0, |s| s.river_points);
        // The runs of the rivers' segments in view (first segment, segments), a river's
        // neighbouring chunks merged into one draw. The largest river first (the last uploaded):
        // a tributary's water, which fades out inside the river it joins, then blends over that
        // river's water, not over the ground (#115; the same depth passes).
        let mut river_runs: Vec<(u32, u32)> = Vec::new();
        if let Some(s) = shore {
            let camera = params.camera.as_vec3();
            let mut run_river = u32::MAX;
            for chunk in s.river_chunks.iter().rev() {
                if !box_in_view(&params.view_proj, chunk.lo - camera, chunk.hi - camera) {
                    continue;
                }
                match river_runs.last_mut() {
                    Some((first, count))
                        if run_river == chunk.river && chunk.first + chunk.count == *first =>
                    {
                        *first = chunk.first;
                        *count += chunk.count;
                    }
                    _ => river_runs.push((chunk.first, chunk.count)),
                }
                run_river = chunk.river;
            }
        }
        let shore_image = shore.map(|s| graph.import(&s.image));
        // The water at the camera.
        let at_camera = in_reach.then(|| graph.import_buffer(&self.at_camera));
        let at_camera_address = at_camera.map_or(0, |_| self.at_camera.address());
        if let Some(at_camera) = at_camera {
            let pipeline = &self.at_camera_pass;
            let mut pass = graph
                .pass("water/at-camera")
                .buffer(at_camera, BufferAccess::ShaderWrite(compute));
            for &(displacement, _) in &waves.cascades {
                pass = pass.image(displacement, ImageAccess::Sampled(compute));
            }
            if let Some(image) = shore_image {
                pass = pass.image(image, ImageAccess::Sampled(compute));
            }
            pass.run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &AtCameraPush { surface: address });
                commands.dispatch(1, 1, 1);
                Ok(())
            });
        }
        let mut pass = graph
            .pass("water/surface")
            .image(color, ImageAccess::ColorAttachment)
            .image(mirror_requests, ImageAccess::ColorAttachment)
            .image(sun_requests, ImageAccess::ColorAttachment)
            .image(depth, ImageAccess::DepthAttachment)
            .image(scene_color, ImageAccess::Sampled(fragment))
            .image(scene_depth, ImageAccess::Sampled(fragment))
            .image(sky.light.table, ImageAccess::Sampled(fragment))
            .image(sky.aerial(), ImageAccess::Sampled(fragment))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(fragment));
        for &(displacement, slopes) in &waves.cascades {
            pass = pass
                .image(displacement, ImageAccess::Sampled(vertex))
                .image(slopes, ImageAccess::Sampled(fragment));
        }
        if let Some(image) = shore_image {
            pass = pass.image(image, ImageAccess::Sampled(vertex | fragment));
        }
        if let Some(at_camera) = at_camera {
            pass = pass.buffer(at_camera, BufferAccess::ShaderRead(fragment));
        }
        pass.run(move |resources, commands| {
            let mut cascade_views = [GpuWaterCascadeView::zeroed(); SURFACE_CASCADES];
            cascade_views.copy_from_slice(&views);
            block.write(
                0,
                &[GpuWaterSurface {
                    view_proj: params.view_proj.to_cols_array(),
                    camera: [
                        params.camera.x as f32,
                        params.camera.y as f32,
                        params.camera.z as f32,
                        0.0,
                    ],
                    sun: params.sun_dir.normalize_or(Vec3::Y).extend(0.0).to_array(),
                    sun_radiance: params.sun_radiance.extend(params.sky_scale).to_array(),
                    absorption: [0.35, 0.07, 0.05, 0.0],
                    scatter: [0.003, 0.013, 0.016, 0.0],
                    shore_frame: shore.map_or([0.0; 4], |s| s.frame),
                    trains: shore.map_or([[0.0; 4]; MAX_TRAINS], |s| s.trains),
                    cascades: cascade_views,
                    levels,
                    level_count: LEVELS,
                    grid: GRID,
                    scene_color: resources.sampled(scene_color).0,
                    scene_depth: resources.sampled(scene_depth).0,
                    width: extent.width,
                    height: extent.height,
                    shore: shore_image.map_or(u32::MAX, |image| resources.sampled(image).0),
                    shore_texels: shore.map_or(0, |s| s.texels),
                    sky: sky_address,
                    sky_light,
                    shore_table: shore
                        .and_then(|s| s.tables.as_ref())
                        .map_or(0, |b| b.address()),
                    shore_bins: shore.map_or(0, |s| s.bins),
                    train_count: shore.map_or(0, |s| s.train_count),
                    shore_bin: shore.map_or(1.0, |s| s.bin),
                    time: params.time,
                    pixel: params.pixel,
                    river_points,
                    ground: rivers.map_or(0, |(_, heights)| heights.address()),
                    rivers: rivers.map_or(0, |(points, _)| points.address()),
                    mouths: shore
                        .and_then(|s| s.mouths.as_ref())
                        .map_or(0, |(m, _)| m.address()),
                    mouth_count: shore.map_or(0, |s| s.mouth_count),
                    mouth_grid: shore
                        .and_then(|s| s.mouths.as_ref())
                        .map_or(0, |(_, g)| g.address()),
                    mouth_cells: MOUTH_CELLS,
                    mouth_cell: shore.map_or(1.0, |s| s.mouth_cell),
                    stone_count: shore.map_or(0, |s| s.stone_count),
                    stones: shore
                        .and_then(|s| s.stones.as_ref())
                        .map_or(0, |b| b.address()),
                    lakes: shore
                        .and_then(|s| s.lakes.as_ref())
                        .map_or(0, |(l, _)| l.address()),
                    lake_masks: shore
                        .and_then(|s| s.lakes.as_ref())
                        .map_or(0, |(_, m)| m.address()),
                    lake_count,
                    pad: 0,
                    at_camera: at_camera_address,
                    pad_at_camera: [0; 2],
                }],
            );
            // The requests start at zero: no ray where the water is not drawn.
            let cleared = |image| {
                vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(image))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(vk::ClearValue::default())
            };
            let attachment = [
                vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(color))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE),
                cleared(mirror_requests),
                cleared(sun_requests),
            ];
            let depth_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(depth))
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE);
            let info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&attachment)
                .depth_attachment(&depth_attachment);
            commands.begin_rendering(&info);
            commands.bind_pipeline(surface);
            commands.set_viewport_full(extent);
            commands.bind_index_buffer(surface_indices, 0, vk::IndexType::UINT16);
            for &(first, count, level) in &surface_runs {
                commands.push_constants(
                    surface,
                    &SurfacePush {
                        surface: address,
                        first: level,
                        pad: 0,
                    },
                );
                commands.draw_indexed(count, first, 0);
            }
            // The lakes, then the rivers over the land (a segment between each pair of a river's
            // points), which blend over the lakes they run into.
            if lake_count > 0 {
                commands.bind_pipeline(lake_pipeline);
                commands.push_constants(
                    lake_pipeline,
                    &SurfacePush {
                        surface: address,
                        first: 0,
                        pad: 0,
                    },
                );
                commands.draw(lake_count * 6, 1);
            }
            if !river_runs.is_empty() {
                commands.bind_pipeline(river_pipeline);
                for &(first, count) in &river_runs {
                    commands.push_constants(
                        river_pipeline,
                        &SurfacePush {
                            surface: address,
                            first: first * RIVER_ACROSS * 6,
                            pad: 0,
                        },
                    );
                    commands.draw(count * RIVER_ACROSS * 6, 1);
                }
            }
            commands.end_rendering();
            Ok(())
        });
        // The water between the camera and what each pixel under it meets (#108): none of its
        // groups while the near plane stands over the water (`water/at-camera` sets them).
        if let Some(at_camera) = at_camera {
            let under = &self.under;
            let args: &'f GraphBuffer = &self.at_camera;
            graph
                .pass("water/under")
                .image(color, ImageAccess::StorageReadWrite(compute))
                .image(depth, ImageAccess::Sampled(compute))
                .image(scene_depth, ImageAccess::Sampled(compute))
                .buffer(at_camera, BufferAccess::IndirectArgsAndShaderRead(compute))
                .buffer(sky.light.buffer, BufferAccess::ShaderRead(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(under);
                    commands.push_constants(
                        under,
                        &UnderPush {
                            surface: address,
                            color: resources.storage(color, 0).0,
                            depth: resources.sampled(depth).0,
                            width: extent.width,
                            height: extent.height,
                        },
                    );
                    commands.dispatch_indirect(args, AT_CAMERA_DISPATCH);
                    Ok(())
                });
        }
        RayRequests {
            mirror: mirror_requests,
            sun: sun_requests,
            depth,
            view_proj: params.view_proj,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn each_level_draws_once_every_quad_its_finer_level_leaves() {
        let (indices, sets) = surface_indices();
        let side = GRID + 1;
        // A set's quads by their first corner, each once.
        let quads = |set: usize| {
            let mut seen = HashSet::new();
            for &(first, count) in &sets[set] {
                for quad in indices[first as usize..(first + count) as usize].chunks(6) {
                    let at = u32::from(quad[0]);
                    assert!(seen.insert((at % side, at / side)), "a quad drawn twice");
                }
            }
            seen
        };
        assert_eq!(quads(0).len(), (GRID * GRID) as usize);
        // The quads the shader dropped before the indices (`surface_vert_main`, in f32): those
        // whose middle lies within the finer level's extent.
        let half = (GRID / 2) as f32;
        for camera in [
            [0.0, 0.0],
            [0.3, -0.7],
            [1234.567, -9876.5],
            [-5083.2, -1393.9],
            [77.75, 3.25],
            [-0.25, 1.0e5 + 0.75],
        ] {
            let centres: Vec<[f64; 2]> = (0..LEVELS as usize)
                .map(|l| surface_centre(camera[0], camera[1], l))
                .collect();
            for l in 1..LEVELS as usize {
                let drawn = quads(surface_set(&centres, l));
                let s = (FINEST_SPACING * f64::from(1u32 << l)) as f32;
                let centre = centres[l].map(|c| c as f32);
                let fine = centres[l - 1].map(|c| c as f32);
                for j in 0..GRID {
                    for i in 0..GRID {
                        let middle = [
                            centre[0] + (i as f32 + 0.5 - half) * s,
                            centre[1] + (j as f32 + 0.5 - half) * s,
                        ];
                        let covered = (0..2).all(|a| (middle[a] - fine[a]).abs() < half * s * 0.5);
                        assert_eq!(
                            drawn.contains(&(i, j)),
                            !covered,
                            "level {l}, quad ({i}, {j})"
                        );
                    }
                }
            }
        }
    }
}
