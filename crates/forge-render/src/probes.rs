//! Diffuse light from probes updated by ray queries (issue #53, D-008's T1 tier,
//! `shaders/probes.slang`): DDGI after Majercik et al. 2019 and 2021, written from the papers.
//!
//! Cascades of probes follow the camera, each twice as coarse as the one before. Every probe
//! keeps two octahedral maps in an atlas: the irradiance its rays measured (6 × 6 texels) and
//! the mean and mean square of the distance to what they met (14 × 14), each with a
//! one-texel border for bilinear filtering. Three graph passes a frame, after the sky's tables
//! and before the resolve:
//! - `gi/probe rays`: every probe's rays against the scene's TLAS, a thread per ray. Hits are
//!   lit by the sun (a shadow ray) and by the probes as the frame before left them, so the
//!   light bounces once more every frame; misses take the sky-view table.
//! - `gi/probe state`: from the 32 rays whose directions never change, each probe moves out
//!   of walls and away from surfaces it nearly touches, and turns inactive where it is inside
//!   something or has nothing within a cell of it.
//! - `gi/probe blend`: the other rays blended into the maps, keeping `hysteresis` of what
//!   they held; a probe new to its cascade (it scrolled in) starts from its first rays.
//!
//! A settled probe (past its first eight updates) traces and blends every
//! [`ProbeParams::cadence`] frames, on its turn (issue #103).
//!
//! The resolve then lights every pixel's diffuse side with the probes around it
//! ([`crate::AmbientLight::probes`]) instead of the open sky's irradiance.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, MemoryLocation, Pipeline, QueueKind, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::{IVec3, Mat3, Quat, Vec3};

use crate::meshlet::MoversFrame;
use crate::sky::SkyLight;

/// Most cascades (`PROBE_MAX_CASCADES` in `probes.slang`).
pub const MAX_CASCADES: usize = 6;
/// Rays per probe whose directions never change (`PROBE_FIXED_RAYS`).
const FIXED_RAYS: u32 = 32;
/// Most rays per probe (`PROBE_MAX_RAYS`).
const MAX_RAYS: u32 = 256;
/// Texels a side of a probe's maps with their borders (`IRRADIANCE_TILE`, `DISTANCE_TILE`).
const IRRADIANCE_TILE: u32 = 8;
const DISTANCE_TILE: u32 = 16;
/// Rays a workgroup of `gi/probe rays` (`PROBE_TRACE_GROUP` in `meshlet.slang`).
const TRACE_GROUP: u32 = 64;
/// Workgroups along one dimension of a dispatch (the shaders spread probes over two).
const MAX_GROUPS: u32 = 65535;
/// `probe_data`'s w for an inactive probe (`PROBE_INACTIVE`).
const PROBE_INACTIVE: f32 = 1.0;

/// Mirrors `ProbeCascade` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCascade {
    origin: [i32; 3],
    spacing: f32,
    previous: [i32; 3],
    pad: u32,
}

/// Mirrors `ProbeField` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuProbeField {
    rotation: [[f32; 4]; 3],
    cascades: [GpuCascade; MAX_CASCADES],
    counts: [u32; 3],
    cascade_count: u32,
    camera: [f32; 3],
    rays: u32,
    ray_data: u64,
    probe_data: u64,
    irradiance_image: u32,
    distance_image: u32,
    irradiance_storage: u32,
    distance_storage: u32,
    irradiance_texel: [f32; 2],
    distance_texel: [f32; 2],
    hysteresis: f32,
    settled_hysteresis: f32,
    cadence: u32,
    phase: u32,
    probe_region: u64,
    region_volumes: u64,
    region_cells: u64,
    region_indices: u64,
    region_origin: [f32; 2],
    region_cell: f32,
    region_count: u32,
    region_dims: [u32; 2],
    region_pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuProbeField>() == 400);

/// Mirrors `RegionVolume` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuRegionVolume {
    corners: [[f32; 2]; 4],
    floor: f32,
    ceiling: f32,
    region: u32,
    other: u32,
    kind: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuRegionVolume>() == 56);

/// `REGION_NONE` in `probes.slang`: a probe in a recess or a doorway, no region's.
const REGION_NONE: u32 = u32::MAX;

/// The side of the cells of the grid that finds a point's region volumes, metres.
const REGION_CELL: f32 = 2.0;

/// What a region volume is (D-068, `RegionVolume` in `probes.slang`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// A room, a hall: its probes light its surfaces.
    Room,
    /// A window's inner reveal, from the glass to the room: its room's surfaces, and no probe of
    /// any region stands in it.
    Recess,
    /// A doorway between two regions: across it, the light goes from one side's probes to the
    /// other's; no probe of any region stands in it.
    Doorway,
    /// A house's body outside its rooms (its walls, its slabs), last of all: a surface there is
    /// lit by every probe, as without regions (a face whose point 2 cm off it missed its room's
    /// outline: a corner, a ceiling's edge), and no probe of any region stands in it.
    Solid,
}

/// A volume of a scene that knows its rooms (D-068): a prism over a convex quadrilateral, in the
/// scene frame (the probes', [`Probes::update`]'s `camera`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionVolume {
    /// Its outline, x and z in the scene frame, either way round; a doorway's first two corners
    /// on `region`'s side, its last two on `other`'s.
    pub corners: [glam::Vec2; 4],
    /// Its floor and its ceiling, metres in the scene frame.
    pub floor: f32,
    /// See `floor`.
    pub ceiling: f32,
    /// Its region, 1 and up (0 is outside): a doorway's side of its first two corners.
    pub region: u32,
    /// A doorway's other side (0: outside); else `region`.
    pub other: u32,
    /// What it is.
    pub kind: RegionKind,
}

/// What [`region_at`] finds at a point (`RegionHit` in `probes.slang`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionHit {
    /// The volume's kind; outside every volume, a room's.
    pub kind: RegionKind,
    /// Its region (0 outside): on a doorway, the side its ramp starts from.
    pub region: u32,
    /// A doorway's other side; else `region`.
    pub other: u32,
    /// How far across a doorway the point stands, 0 on `region`'s side to 1 on `other`'s;
    /// else 0.
    pub ramp: f32,
}

impl RegionHit {
    /// How much region `region`'s probes count at the point (the lookup's `share`): fully in
    /// their own room, across a doorway as far as the point stands towards their side, else
    /// not at all.
    pub fn share(&self, region: u32) -> f32 {
        if region == self.region {
            1.0 - self.ramp
        } else if region == self.other {
            self.ramp
        } else {
            0.0
        }
    }
}

/// The region at `p` (scene frame) among `volumes`, as the probes' lookup finds it
/// (`region_at` in `probes.slang`, for tests and tools): the first volume holding it in the
/// lookup's order, recesses and doorways, then rooms, then the houses' bodies; outside them
/// all, region 0.
pub fn region_at(volumes: &[RegionVolume], p: Vec3) -> RegionHit {
    let xz = glam::Vec2::new(p.x, p.z);
    for v in ordered(volumes) {
        if p.y < v.floor || p.y > v.ceiling {
            continue;
        }
        let c = v.corners.map(glam::Vec2::from);
        let inside = (0..4).all(|k| {
            let e = c[(k + 1) % 4] - c[k];
            let d = xz - c[k];
            e.x * d.y - e.y * d.x >= 0.0
        });
        if !inside {
            continue;
        }
        let kind = match v.kind {
            0 => RegionKind::Room,
            1 => RegionKind::Recess,
            2 => RegionKind::Doorway,
            _ => RegionKind::Solid,
        };
        let ramp = if kind == RegionKind::Doorway {
            let e = c[1] - c[0];
            let across = glam::Vec2::new(-e.y, e.x).normalize();
            let depth = (c[3] - c[0]).dot(across);
            ((xz - c[0]).dot(across) / depth.max(1e-4)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return RegionHit {
            kind,
            region: v.region,
            other: v.other,
            ramp,
        };
    }
    RegionHit {
        kind: RegionKind::Room,
        region: 0,
        other: 0,
        ramp: 0.0,
    }
}

/// Mirrors `ProbeTracePush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TracePush {
    frame: u64,
    field: u64,
    sky: u64,
    probe_count: u32,
    /// 1 on the first update after a reset: no probe has measured anything yet.
    first_update: u32,
    /// 1: the rays meet the movers ([`ProbeParams::movers`]).
    movers: u32,
    pad: u32,
}

/// Mirrors `WakePush` in `probe_update.slang` (#79).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WakePush {
    field: u64,
    now: u64,
    before: u64,
    count: u32,
    cascades: u32,
    origin_cell: [i32; 4],
    origin_local: [f32; 4],
}

/// Mirrors `Push` in `probe_update.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpdatePush {
    field: u64,
    probe_count: u32,
    pad: u32,
}

/// The probes' layout and update.
#[derive(Clone, Copy, Debug)]
pub struct ProbeParams {
    /// Probes per cascade along x, y (up) and z, each even.
    pub counts: [u32; 3],
    /// Cascades, each twice as coarse as the one before (at most [`MAX_CASCADES`]).
    pub cascades: u32,
    /// Metres between the finest cascade's probes.
    pub spacing: f32,
    /// Rays per probe and frame, the first 32 with fixed directions (64 to 256).
    pub rays: u32,
    /// Share of the maps an update keeps: 0.97 lets a change settle in about 30 updates.
    pub hysteresis: f32,
    /// A settled probe (one that has stopped moving, after its first eight updates) updates
    /// every `cadence` frames, on its turn, keeping `hysteresis^cadence` so a change settles
    /// in the same time (issue #103). New and young probes update every frame. 1: every probe
    /// every frame.
    pub cadence: u32,
    /// Whether the probes' rays meet the movers (#79), and the probes they enter settle again.
    /// Off by default (#299): reading this frame's movers made the probes, on the compute queue,
    /// wait for their upload on the graphics queue, and the village with 23 walkers took
    /// 0.65 ms more a frame (6.58 against 7.18 ms; the serial frame the same with or without
    /// them). The movers keep their sun and lamp shadows and their reflections; only the
    /// probes' diffuse light does not see them.
    pub movers: bool,
}

impl Default for ProbeParams {
    /// The city's: 24 × 12 × 24 probes in five cascades 4, 8, 16, 32 and 64 m apart (the
    /// finest reaches 40 m around the camera, the coarsest 640 m), 128 rays each, the settled
    /// ones every other frame (issue #103: the south view 2.03 → 1.81 ms).
    fn default() -> Self {
        Self {
            counts: [24, 12, 24],
            cascades: 5,
            spacing: 4.0,
            rays: 128,
            hysteresis: 0.97,
            cadence: 2,
            movers: false,
        }
    }
}

impl ProbeParams {
    /// Probes in all the cascades.
    pub fn probe_count(&self) -> u32 {
        self.counts.iter().product::<u32>() * self.cascades
    }

    /// The atlas of maps `tile` texels a side: a row of x × y tiles per z of every cascade.
    fn atlas_size(&self, tile: u32) -> [u32; 2] {
        [
            self.counts[0] * self.counts[1] * tile,
            self.counts[2] * self.cascades * tile,
        ]
    }
}

/// What the resolve reads of this frame's probes (see [`crate::AmbientLight`]).
#[derive(Clone, Copy, Debug)]
pub struct ProbeLight {
    /// The frame's `ProbeField` (`probes.slang`).
    pub(crate) address: u64,
    pub(crate) data: BufferHandle,
    /// Each probe's region (D-068).
    pub(crate) region: BufferHandle,
    pub(crate) irradiance: ImageHandle,
    pub(crate) distance: ImageHandle,
}

impl ProbeLight {
    /// The irradiance atlas, [`DiffuseProbes::irradiance_extent`] texels (for the image
    /// hashes, #207).
    pub fn irradiance(&self) -> ImageHandle {
        self.irradiance
    }

    /// The distance atlas, [`DiffuseProbes::distance_extent`] texels (for the image hashes).
    pub fn distance(&self) -> ImageHandle {
        self.distance
    }
}

/// The world cell of a cascade's first probe: the camera rounded to the cascade's cells, less
/// half the counts, so the probes always reach (counts / 2 − 1) cells from the camera
/// whichever way it sits in its cell (`cascade_weight` in `probes.slang` relies on it).
fn cascade_origin(camera: Vec3, spacing: f32, counts: [u32; 3]) -> IVec3 {
    let cell = (camera / spacing).round();
    IVec3::new(cell.x as i32, cell.y as i32, cell.z as i32)
        - IVec3::new(counts[0] as i32, counts[1] as i32, counts[2] as i32) / 2
}

/// A uniformly random rotation from `index` alone (reproducible captures): three
/// numbers from a hash of the index, turned into a unit quaternion (Shoemake, "Uniform Random
/// Rotations", Graphics Gems III, 1992).
fn frame_rotation(index: u64) -> Mat3 {
    let mut state = index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut next = || {
        // splitmix64
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let (u1, u2, u3) = (next(), next(), next());
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
    Mat3::from_quat(
        Quat::from_xyzw(
            a * (tau * u2).sin(),
            a * (tau * u2).cos(),
            b * (tau * u3).sin(),
            b * (tau * u3).cos(),
        )
        .normalize(),
    )
}

/// A scene's region volumes on the GPU (D-068) and the grid over them.
struct Regions {
    volumes: Buffer,
    cells: Buffer,
    indices: Buffer,
    origin: [f32; 2],
    dims: [u32; 2],
    count: u32,
}

/// The volumes in the order the lookup tries them, recesses and doorways first (they overlap
/// their rooms' edges), the houses' bodies last (round their rooms), each anticlockwise in x and z
/// (a doorway turned round swaps its sides).
fn ordered(volumes: &[RegionVolume]) -> Vec<GpuRegionVolume> {
    let mut out: Vec<GpuRegionVolume> = volumes
        .iter()
        .map(|v| {
            let c = v.corners;
            let area: f32 = (0..4)
                .map(|i| {
                    let (a, b) = (c[i], c[(i + 1) % 4]);
                    a.x * b.y - b.x * a.y
                })
                .sum();
            let (corners, region, other) = if area >= 0.0 {
                (c, v.region, v.other)
            } else {
                ([c[3], c[2], c[1], c[0]], v.other, v.region)
            };
            GpuRegionVolume {
                corners: corners.map(|p| p.to_array()),
                floor: v.floor,
                ceiling: v.ceiling,
                region,
                other,
                kind: match v.kind {
                    RegionKind::Room => 0,
                    RegionKind::Recess => 1,
                    RegionKind::Doorway => 2,
                    RegionKind::Solid => 3,
                },
                pad: 0,
            }
        })
        .collect();
    // Recesses and doorways, then the rooms, then the houses' bodies.
    out.sort_by_key(|v| match v.kind {
        1 | 2 => 0,
        0 => 1,
        _ => 2,
    });
    out
}

/// The grid over `volumes` ([`REGION_CELL`] cells): its corner, its size in cells, and per cell
/// the first of its volumes' indices and their count, then the indices.
fn region_grid(volumes: &[GpuRegionVolume]) -> ([f32; 2], [u32; 2], Vec<[u32; 2]>, Vec<u32>) {
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for v in volumes {
        for c in v.corners {
            lo = lo.min(glam::Vec2::from(c));
            hi = hi.max(glam::Vec2::from(c));
        }
    }
    let origin = lo - glam::Vec2::splat(REGION_CELL);
    let dims = ((hi - origin) / REGION_CELL).ceil().as_uvec2() + glam::UVec2::ONE;
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); (dims.x * dims.y) as usize];
    for (i, v) in volumes.iter().enumerate() {
        let (mut a, mut b) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
        for c in v.corners {
            a = a.min(glam::Vec2::from(c));
            b = b.max(glam::Vec2::from(c));
        }
        let (c0, c1) = (
            ((a - origin) / REGION_CELL).floor().as_uvec2(),
            ((b - origin) / REGION_CELL)
                .floor()
                .as_uvec2()
                .min(dims - 1),
        );
        for z in c0.y..=c1.y {
            for x in c0.x..=c1.x {
                lists[(z * dims.x + x) as usize].push(i as u32);
            }
        }
    }
    let mut cells = Vec::with_capacity(lists.len());
    let mut indices = Vec::new();
    for list in lists {
        cells.push([indices.len() as u32, list.len() as u32]);
        indices.extend(list);
    }
    (origin.to_array(), dims.to_array(), cells, indices)
}

/// The probes: their maps, their rays and state, and the passes that update them.
pub struct Probes {
    params: ProbeParams,
    trace: Pipeline,
    state: Pipeline,
    blend: Pipeline,
    /// The settled probes woken where the movers pass (#79, #69).
    wake: Pipeline,
    irradiance: GraphImage,
    distance: GraphImage,
    rays: GraphBuffer,
    data: GraphBuffer,
    /// Each probe's region (D-068), `REGION_NONE` until its first update with regions.
    region: GraphBuffer,
    /// The scene's region volumes and the grid that finds them, when it knows its rooms.
    regions: Option<Regions>,
    /// Zeros, what the field's region pointers name in a scene without regions: one empty cell, a
    /// volume and an index, so the region lookup's reads stay in a buffer where the GPU reads
    /// ahead of its early return (`region_at` in `probes.slang`).
    no_regions: Buffer,
    fields: Vec<Buffer>,
    /// Per cascade, the origin of the last update.
    origins: [IVec3; MAX_CASCADES],
    /// Every probe starts over at the next update.
    reset: bool,
}

impl Probes {
    /// Compiles the passes (`gi/probe rays` needs ray queries) and creates the atlases, the
    /// rays' buffer and the probes' state, every probe inactive until its first update.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        params: ProbeParams,
    ) -> Result<Self> {
        assert!(
            params.cascades >= 1 && params.cascades as usize <= MAX_CASCADES,
            "1 to {MAX_CASCADES} cascades"
        );
        assert!(
            params.counts.iter().all(|&n| n >= 4 && n % 2 == 0),
            "even counts of at least 4"
        );
        assert!(
            (FIXED_RAYS + 32..=MAX_RAYS).contains(&params.rays),
            "64 to 256 rays"
        );
        assert!(params.cadence >= 1, "a cadence of at least 1");
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
        let atlas = |tile: u32, format: vk::Format, name: &str| {
            let [width, height] = params.atlas_size(tile);
            GraphImage::new(
                device,
                ImageDesc {
                    width,
                    height,
                    format,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let probes = u64::from(params.probe_count());
        let rays = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: probes * u64::from(params.rays) * 8,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "probe rays",
        })?);
        let data = device.create_buffer(BufferDesc {
            size: probes * 16,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "probe state",
        })?;
        let inactive = vec![[0.0, 0.0, 0.0, PROBE_INACTIVE]; probes as usize];
        device.write_buffer_staged(&data, 0, bytemuck::cast_slice(&inactive))?;
        let region = device.create_buffer(BufferDesc {
            size: probes * 4,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "probe regions",
        })?;
        device.write_buffer_staged(
            &region,
            0,
            bytemuck::cast_slice(&vec![REGION_NONE; probes as usize]),
        )?;
        let no_regions = device.create_buffer(BufferDesc {
            size: 256,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "no probe regions",
        })?;
        device.write_buffer_staged(&no_regions, 0, &[0_u8; 256])?;
        let fields = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuProbeField>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("probe field {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            params,
            trace: compute(
                "meshlet.slang",
                "probe_trace_main",
                std::mem::size_of::<TracePush>(),
                "probe rays",
            )?,
            state: compute(
                "probe_update.slang",
                "probe_state_main",
                std::mem::size_of::<UpdatePush>(),
                "probe state",
            )?,
            blend: compute(
                "probe_update.slang",
                "probe_blend_main",
                std::mem::size_of::<UpdatePush>(),
                "probe blend",
            )?,
            wake: compute(
                "probe_update.slang",
                "probe_wake_main",
                std::mem::size_of::<WakePush>(),
                "probe wake",
            )?,
            irradiance: atlas(
                IRRADIANCE_TILE,
                vk::Format::R16G16B16A16_SFLOAT,
                "probe irradiance",
            )?,
            distance: atlas(DISTANCE_TILE, vk::Format::R16G16_SFLOAT, "probe distance")?,
            rays,
            data: GraphBuffer::new(data),
            region: GraphBuffer::new(region),
            regions: None,
            no_regions,
            fields,
            origins: [IVec3::ZERO; MAX_CASCADES],
            reset: true,
        })
    }

    /// The scene's rooms (D-068, `research/probe-leaks-indoors.md`): a probe then lights only
    /// the surfaces of its own region, across a doorway both sides' in proportion, and outside
    /// every volume the outside's. None: every probe lights every surface, as before. Every
    /// probe starts over at the next update.
    pub fn set_regions(&mut self, device: &Arc<Device>, volumes: &[RegionVolume]) -> Result<()> {
        self.reset = true;
        if volumes.is_empty() {
            self.regions = None;
            return Ok(());
        }
        let ordered = ordered(volumes);
        let (origin, dims, cells, indices) = region_grid(&ordered);
        let buffer = |bytes: &[u8], name: &str| -> Result<Buffer> {
            let buffer = device.create_buffer(BufferDesc {
                size: bytes.len().max(4) as u64,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })?;
            if !bytes.is_empty() {
                device.write_buffer_staged(&buffer, 0, bytes)?;
            }
            Ok(buffer)
        };
        self.regions = Some(Regions {
            volumes: buffer(bytemuck::cast_slice(&ordered), "probe region volumes")?,
            cells: buffer(bytemuck::cast_slice(&cells), "probe region cells")?,
            indices: buffer(bytemuck::cast_slice(&indices), "probe region indices")?,
            origin,
            dims,
            count: ordered.len() as u32,
        });
        Ok(())
    }

    /// The layout and update this was made with.
    pub fn params(&self) -> &ProbeParams {
        &self.params
    }

    /// The irradiance atlas's size, texels ([`ProbeLight::irradiance`]).
    pub fn irradiance_extent(&self) -> [u32; 2] {
        self.params.atlas_size(IRRADIANCE_TILE)
    }

    /// The distance atlas's size, texels ([`ProbeLight::distance`]).
    pub fn distance_extent(&self) -> [u32; 2] {
        self.params.atlas_size(DISTANCE_TILE)
    }

    /// Bytes of the atlases, the rays and the probes' state.
    pub fn bytes(&self) -> u64 {
        let texels = |tile: u32| {
            let [w, h] = self.params.atlas_size(tile);
            u64::from(w) * u64::from(h)
        };
        texels(IRRADIANCE_TILE) * 8
            + texels(DISTANCE_TILE) * 4
            + self.rays.size()
            + self.data.size()
            + self.region.size()
            + self.no_regions.size()
            + self
                .regions
                .as_ref()
                .map_or(0, |r| r.volumes.size() + r.cells.size() + r.indices.size())
    }

    /// Every probe's offset from its cell's centre (xyz, metres) and state and age (w: the
    /// state, 0 active, 1 inactive, 2 active and starting over, plus 4 × the updates it has
    /// had, up to 8, then 9 once settled, `probe_state` in `probes.slang`) as the last
    /// submitted frame left them, in probe
    /// order (`probe_index` in `probes.slang`). Waits for the device: for logs and tests.
    pub fn read_states(&self, device: &Arc<Device>) -> Result<Vec<[f32; 4]>> {
        let bytes = device.read_back(&self.data, 0, self.data.size())?;
        Ok(bytemuck::cast_slice(&bytes).to_vec())
    }

    /// Starts every probe over at the next update (after the probes were off for a while, or
    /// the camera jumped).
    pub fn reset(&mut self) {
        self.reset = true;
    }

    /// Declares this frame's three passes around the camera at `camera` (metres in the scene
    /// frame, `camera.relative_to(scene.origin())`: the frame the rays trace in, issue #93), with
    /// the scene of the renderer's frame block at `frame` (its TLAS, instances and materials,
    /// [`crate::MeshletRenderer::frame_address`]) and the sky's light and table, and returns
    /// what the resolve reads.
    ///
    /// `noise_frame` picks the rays' rotation and the settled probes' turn. The city passes
    /// TAA's frame modulo its jitter's period times the cadence, as for the ambient occlusion
    /// (issue #48): the rotations then repeat with the jitter, the maps of a still scene settle
    /// into the same cycle and TAA averages it, where a new rotation every frame kept them
    /// wandering (measured: the static view's slow change 0.27 % of pixels, 0.09 % without
    /// probes). A rotation lasts one round of turns (`cadence` frames), so every probe meets
    /// each of the cycle's rotations.
    ///
    /// `movers` are the frame's movers ([`crate::DrawTargets::movers`], #79): the rays meet
    /// them in their own acceleration structure.
    #[allow(clippy::too_many_arguments)]
    pub fn update<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        sky: SkyLight,
        camera: Vec3,
        noise_frame: u64,
        movers: Option<MoversFrame>,
    ) -> ProbeLight {
        let p = self.params;
        let mut cascades = [GpuCascade::zeroed(); MAX_CASCADES];
        for (c, cascade) in cascades.iter_mut().enumerate().take(p.cascades as usize) {
            let spacing = p.spacing * (1u32 << c) as f32;
            let origin = cascade_origin(camera, spacing, p.counts);
            // After a reset, a previous origin two volumes away: every probe is new.
            let previous = if self.reset {
                origin + 2 * IVec3::new(p.counts[0] as i32, p.counts[1] as i32, p.counts[2] as i32)
            } else {
                self.origins[c]
            };
            self.origins[c] = origin;
            *cascade = GpuCascade {
                origin: origin.to_array(),
                spacing,
                previous: previous.to_array(),
                pad: 0,
            };
        }
        // The first update after a reset lights its rays' hits by the sun alone (#171).
        let first_update = std::mem::take(&mut self.reset);
        // One rotation per round of turns: each probe meets every rotation of the cycle.
        let rotation = frame_rotation(noise_frame / u64::from(p.cadence));
        let row = |r: usize| {
            let v = rotation.row(r);
            [v.x, v.y, v.z, 0.0]
        };
        let texel = |tile: u32| {
            let [w, h] = p.atlas_size(tile);
            [1.0 / w as f32, 1.0 / h as f32]
        };
        let empty = self.no_regions.address();
        let field = GpuProbeField {
            rotation: [row(0), row(1), row(2)],
            cascades,
            counts: p.counts,
            cascade_count: p.cascades,
            camera: camera.to_array(),
            rays: p.rays,
            ray_data: self.rays.address(),
            probe_data: self.data.address(),
            irradiance_image: self.irradiance.sampled().0,
            distance_image: self.distance.sampled().0,
            irradiance_storage: self.irradiance.storage(0).0,
            distance_storage: self.distance.storage(0).0,
            irradiance_texel: texel(IRRADIANCE_TILE),
            distance_texel: texel(DISTANCE_TILE),
            hysteresis: p.hysteresis,
            settled_hysteresis: p.hysteresis.powi(p.cadence as i32),
            cadence: p.cadence,
            phase: (noise_frame % u64::from(p.cadence)) as u32,
            probe_region: self.region.address(),
            region_volumes: self.regions.as_ref().map_or(empty, |r| r.volumes.address()),
            region_cells: self.regions.as_ref().map_or(empty, |r| r.cells.address()),
            region_indices: self.regions.as_ref().map_or(empty, |r| r.indices.address()),
            region_origin: self.regions.as_ref().map_or([0.0; 2], |r| r.origin),
            region_cell: REGION_CELL,
            region_count: self.regions.as_ref().map_or(0, |r| r.count),
            region_dims: self.regions.as_ref().map_or([1; 2], |r| r.dims),
            region_pad: [0; 2],
        };
        self.fields[slot.index].write(0, &[field]);

        let this: &'f Self = self;
        let address = this.fields[slot.index].address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let rays = graph.import_buffer(&this.rays);
        let data = graph.import_buffer(&this.data);
        let region = graph.import_buffer(&this.region);
        let irradiance = graph.import(&this.irradiance);
        let distance = graph.import(&this.distance);
        let count = p.probe_count();
        let spread = [count.min(MAX_GROUPS), count.div_ceil(MAX_GROUPS)];
        // The settled probes whose cells the movers entered or left settle again (#79, #69),
        // before this frame's rays: as young probes, they take them.
        let movers = movers.filter(|_| p.movers);
        if let Some(m) = movers.filter(|m| m.count > 0) {
            let wake = &this.wake;
            let push = WakePush {
                field: address,
                now: m.records,
                before: m.previous,
                count: m.count,
                cascades: p.cascades,
                origin_cell: m.origin.cell.extend(0).to_array(),
                origin_local: m.origin.local.extend(0.0).to_array(),
            };
            graph
                .pass("gi/probe wake")
                .queue(QueueKind::Compute)
                .buffer(m.instances, BufferAccess::ShaderRead(compute))
                .buffer(data, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(wake);
                    commands.push_constants(wake, &push);
                    commands.dispatch((m.count * p.cascades).div_ceil(64), 1, 1);
                    Ok(())
                });
        }
        let trace = &this.trace;
        graph
            .pass("gi/probe rays")
            .queue(QueueKind::Compute)
            .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.table, ImageAccess::Sampled(compute))
            .image(irradiance, ImageAccess::Sampled(compute))
            .image(distance, ImageAccess::Sampled(compute))
            .buffer(data, BufferAccess::ShaderRead(compute))
            .buffer(region, BufferAccess::ShaderRead(compute))
            .buffer(rays, BufferAccess::ShaderWrite(compute))
            .with(|b| MoversFrame::declare(movers, b, compute, true))
            .run(move |_, commands| {
                commands.bind_pipeline(trace);
                commands.push_constants(
                    trace,
                    &TracePush {
                        frame,
                        field: address,
                        sky: sky.address,
                        probe_count: count,
                        first_update: u32::from(first_update),
                        movers: u32::from(movers.is_some()),
                        pad: 0,
                    },
                );
                commands.dispatch(p.rays.div_ceil(TRACE_GROUP), spread[0], spread[1]);
                Ok(())
            });
        let push = UpdatePush {
            field: address,
            probe_count: count,
            pad: 0,
        };
        let state = &this.state;
        graph
            .pass("gi/probe state")
            .queue(QueueKind::Compute)
            .buffer(rays, BufferAccess::ShaderRead(compute))
            .buffer(data, BufferAccess::ShaderReadWrite(compute))
            .buffer(region, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(state);
                commands.push_constants(state, &push);
                commands.dispatch(count.div_ceil(64), 1, 1);
                Ok(())
            });
        let blend = &this.blend;
        graph
            .pass("gi/probe blend")
            .queue(QueueKind::Compute)
            .buffer(rays, BufferAccess::ShaderRead(compute))
            .buffer(data, BufferAccess::ShaderRead(compute))
            .image(irradiance, ImageAccess::StorageReadWrite(compute))
            .image(distance, ImageAccess::StorageReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(blend);
                commands.push_constants(blend, &push);
                commands.dispatch(spread[0], spread[1], 1);
                Ok(())
            });
        ProbeLight {
            address,
            data,
            region,
            irradiance,
            distance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    /// A box's outline from (x0, z0) to (x1, z1).
    fn outline(x0: f32, z0: f32, x1: f32, z1: f32) -> [Vec2; 4] {
        [
            Vec2::new(x0, z0),
            Vec2::new(x1, z0),
            Vec2::new(x1, z1),
            Vec2::new(x0, z1),
        ]
    }

    /// Two rooms 4 m wide, a partition between them at x = 4 with a doorway, a window's reveal
    /// in the first room's outer wall at z = 0, and the house's body round them, given first.
    fn little_house() -> Vec<RegionVolume> {
        let room = |region, x0, x1| RegionVolume {
            corners: outline(x0, 0.0, x1, 4.0),
            floor: 0.0,
            ceiling: 3.0,
            region,
            other: region,
            kind: RegionKind::Room,
        };
        vec![
            RegionVolume {
                corners: outline(-0.3, -0.3, 8.3, 4.3),
                floor: -0.5,
                ceiling: 3.3,
                region: 0,
                other: 0,
                kind: RegionKind::Solid,
            },
            room(1, 0.0, 3.9),
            room(2, 4.1, 8.0),
            // Corners 0–1 on room 1's side, 3–2 on room 2's, clockwise: the lookup turns it
            // round and swaps its sides.
            RegionVolume {
                corners: [
                    Vec2::new(3.89, 1.0),
                    Vec2::new(3.89, 2.0),
                    Vec2::new(4.11, 2.0),
                    Vec2::new(4.11, 1.0),
                ],
                floor: 0.0,
                ceiling: 2.1,
                region: 1,
                other: 2,
                kind: RegionKind::Doorway,
            },
            RegionVolume {
                corners: outline(1.0, -0.3, 2.0, 0.01),
                floor: 0.9,
                ceiling: 2.1,
                region: 1,
                other: 1,
                kind: RegionKind::Recess,
            },
        ]
    }

    /// D-068's lookup: rooms, a doorway's ramp either way round, a reveal, the body, outside.
    #[test]
    fn region_at_finds_rooms_doorways_reveals_and_the_body() {
        let v = little_house();
        let at = |x, y, z| region_at(&v, Vec3::new(x, y, z));
        assert_eq!(at(2.0, 1.5, 2.0).kind, RegionKind::Room);
        assert_eq!(at(2.0, 1.5, 2.0).region, 1);
        assert_eq!(at(6.0, 1.5, 2.0).region, 2);
        // Across the doorway, room 1's probes give way to room 2's.
        let near = at(3.9, 1.0, 1.5);
        assert_eq!(near.kind, RegionKind::Doorway);
        assert!(near.share(1) > 0.9 && near.share(2) < 0.1, "{near:?}");
        let middle = at(4.0, 1.0, 1.5);
        assert!((middle.share(1) - 0.5).abs() < 0.01, "{middle:?}");
        assert!((middle.share(2) - 0.5).abs() < 0.01, "{middle:?}");
        assert!(at(4.1, 1.0, 1.5).share(2) > 0.9);
        assert_eq!(at(4.0, 1.0, 1.5).share(3), 0.0);
        // Over the doorway's head and beside it, the partition: the body's.
        assert_eq!(at(4.0, 2.5, 1.5).kind, RegionKind::Solid);
        assert_eq!(at(4.0, 1.0, 3.0).kind, RegionKind::Solid);
        // The window's reveal is its room's, before the room's own outline.
        let reveal = at(1.5, 1.5, -0.15);
        assert_eq!((reveal.kind, reveal.region), (RegionKind::Recess, 1));
        assert_eq!(at(1.5, 1.5, 0.005).kind, RegionKind::Recess);
        assert_eq!(at(3.0, 1.5, 0.02).kind, RegionKind::Room);
        // Outside the house and over its roof.
        for p in [Vec3::new(10.0, 1.5, 2.0), Vec3::new(2.0, 3.5, 2.0)] {
            let o = region_at(&v, p);
            assert_eq!((o.kind, o.region), (RegionKind::Room, 0), "{p}");
        }
    }

    /// The grid the shader walks finds what the plain lookup finds: each volume listed in every
    /// cell it covers.
    #[test]
    fn region_grid_lists_every_volume_where_it_stands() {
        let v = little_house();
        let gpu = ordered(&v);
        let (origin, dims, cells, indices) = region_grid(&gpu);
        for i in 0..400 {
            let p = Vec3::new(
                -1.0 + 10.0 * ((i % 20) as f32 + 0.37) / 20.0,
                1.2,
                -1.0 + 6.0 * ((i / 20) as f32 + 0.61) / 20.0,
            );
            let c = ((Vec2::new(p.x, p.z) - Vec2::from(origin)) / REGION_CELL).floor();
            let listed: Vec<RegionVolume> =
                if c.x < 0.0 || c.y < 0.0 || c.x >= dims[0] as f32 || c.y >= dims[1] as f32 {
                    Vec::new()
                } else {
                    let [first, count] = cells[(c.y as u32 * dims[0] + c.x as u32) as usize];
                    indices[first as usize..(first + count) as usize]
                        .iter()
                        .map(|&k| {
                            let g = gpu[k as usize];
                            RegionVolume {
                                corners: g.corners.map(Vec2::from),
                                floor: g.floor,
                                ceiling: g.ceiling,
                                region: g.region,
                                other: g.other,
                                kind: [
                                    RegionKind::Room,
                                    RegionKind::Recess,
                                    RegionKind::Doorway,
                                    RegionKind::Solid,
                                ][g.kind as usize],
                            }
                        })
                        .collect()
                };
            assert_eq!(region_at(&listed, p), region_at(&v, p), "{p}");
        }
    }

    /// `oct_encode` in `probes.slang`.
    fn oct_encode(d: Vec3) -> [f32; 2] {
        let l1 = d.x.abs() + d.y.abs() + d.z.abs();
        let (x, z) = (d.x / l1, d.z / l1);
        let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        if d.y >= 0.0 {
            [x, z]
        } else {
            [(1.0 - z.abs()) * sign(x), (1.0 - x.abs()) * sign(z)]
        }
    }

    /// `oct_decode` in `probes.slang`.
    fn oct_decode(p: [f32; 2]) -> Vec3 {
        let h = 1.0 - p[0].abs() - p[1].abs();
        let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        let q = if h >= 0.0 {
            p
        } else {
            [
                (1.0 - p[1].abs()) * sign(p[0]),
                (1.0 - p[0].abs()) * sign(p[1]),
            ]
        };
        Vec3::new(q[0], h, q[1]).normalize()
    }

    /// `border_source` in `probes.slang`.
    fn border_source(t: [u32; 2], texels: u32) -> [u32; 2] {
        let last = texels + 1;
        let (left, right, top, bottom) = (t[0] == 0, t[0] == last, t[1] == 0, t[1] == last);
        let c = if (left || right) && (top || bottom) {
            [if left { texels } else { 1 }, if top { texels } else { 1 }]
        } else if top || bottom {
            [last - t[0], if top { 1 } else { texels }]
        } else if left || right {
            [if left { 1 } else { texels }, last - t[1]]
        } else {
            t
        };
        [c[0] - 1, c[1] - 1]
    }

    #[test]
    fn the_octahedral_map_round_trips() {
        for i in 0..200 {
            let d = Vec3::new(
                (i as f32 * 0.37).sin(),
                (i as f32 * 0.91).cos(),
                (i as f32 * 1.73).sin(),
            )
            .normalize();
            let back = oct_decode(oct_encode(d));
            assert!((back - d).length() < 1e-5, "{d} → {back}");
        }
        assert!((oct_decode([0.0, 0.0]) - Vec3::Y).length() < 1e-6);
        assert!((oct_decode([1.0, 1.0]) + Vec3::Y).length() < 1e-6);
    }

    /// A border texel holds the interior texel nearest to where the map continues across its
    /// edge: across an edge the octahedron folds, (u, v) beyond u = 1 being (2 − u, −v).
    #[test]
    fn a_border_texel_holds_the_direction_just_across_its_edge() {
        for texels in [6u32, 14] {
            let tile = texels + 2;
            let to_p = |t: f32| (t - 1.0) / texels as f32 * 2.0 - 1.0; // tile coordinate → [-1, 1]
            let wrap = |v: f32, w: f32| -> [f32; 2] {
                // Fold a coordinate beyond ±1 back: the other coordinate changes sign.
                if v > 1.0 {
                    [2.0 - v, -w]
                } else if v < -1.0 {
                    [-2.0 - v, -w]
                } else {
                    [v, w]
                }
            };
            for y in 0..tile {
                for x in 0..tile {
                    let src = border_source([x, y], texels);
                    let (px, py) = (to_p(x as f32 + 0.5), to_p(y as f32 + 0.5));
                    let [px, py] = wrap(px, py);
                    let [py, px] = wrap(py, px);
                    let want = oct_decode([px, py]);
                    let got = oct_decode([
                        (src[0] as f32 + 0.5) / texels as f32 * 2.0 - 1.0,
                        (src[1] as f32 + 0.5) / texels as f32 * 2.0 - 1.0,
                    ]);
                    assert!(
                        (want - got).length() < 1e-4,
                        "tile {tile}: ({x}, {y}) holds {src:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_cascade_reaches_past_the_fade_whatever_the_camera() {
        let counts = [24, 12, 24];
        for i in 0..1000 {
            let camera = Vec3::new(
                i as f32 * 1.37 - 700.0,
                i as f32 * 0.29 - 60.0,
                i as f32 * -2.11,
            );
            for spacing in [4.0, 8.0, 64.0] {
                let origin = cascade_origin(camera, spacing, counts);
                for a in 0..3 {
                    let n = counts[a] as f32;
                    let first = (origin[a] as f32 + 0.5) * spacing;
                    let last = (origin[a] as f32 + n - 0.5) * spacing;
                    let reach = (n * 0.5 - 1.0) * spacing;
                    assert!(first <= camera[a] - reach + 1e-3 && last >= camera[a] + reach - 1e-3);
                }
            }
        }
    }

    #[test]
    fn the_frame_rotation_is_a_rotation_and_changes() {
        let a = frame_rotation(0);
        let b = frame_rotation(1);
        assert!((a.determinant() - 1.0).abs() < 1e-5);
        assert!((a * a.transpose() - Mat3::IDENTITY).abs_diff_eq(Mat3::ZERO, 1e-5));
        assert!(!a.abs_diff_eq(b, 1e-3));
        assert!(a.abs_diff_eq(frame_rotation(0), 0.0));
    }
}
