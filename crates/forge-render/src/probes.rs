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
//! Rooms near the camera have probe grids of their own (D-078, [`ProbeParams::rooms`],
//! `rooms.rs`): their probes take a pool after the cascades' rows, update every frame, and light
//! their rooms' surfaces alone; the cascades' probes standing in such a room retire.
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

mod regions;
mod rooms;

use regions::{REGION_CELL, RegionStore, TOP_CELL, ordered};
pub use regions::{RegionCapacity, RegionSetId};
pub use rooms::RoomGridParams;
use rooms::RoomGrids;

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
/// The probes' updates on the frame after a reset (#320): the sun's light, then seven bounces.
/// A whitewashed room lit through its window, on its first frame against its twentieth: 15 % of
/// its light with one round, 62 % with three, 85 % with six, 91 % with eight (the village's room
/// and hall, the same at a fixed exposure); about 1.4 ms a round, on that frame alone.
const RESET_ROUNDS: u32 = 8;
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
    counts: [u32; 3],
    first_row: u32,
}

/// Mirrors `ProbeField` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuProbeField {
    rotation: [[f32; 4]; 3],
    cascades: [GpuCascade; MAX_CASCADES],
    atlas_row: u32,
    atlas_pad: [u32; 2],
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
    region_sets: u64,
    region_top: u64,
    region_top_list: u64,
    region_splits: u64,
    region_cell: f32,
    region_top_cell: f32,
    region_top_mask: u32,
    region_count: u32,
    far_room_sky: f32,
    sample_bias: [f32; 2],
    region_pad: u32,
    room_slots: u64,
    room_grids: u64,
    room_table: u64,
    room_first: u32,
    room_mask: u32,
    room_count: u32,
    room_rays: u32,
}

const _: () = assert!(std::mem::size_of::<GpuProbeField>() == 568);

/// `REGION_NONE` in `probes.slang`: a probe in a recess or a doorway, no region's.
const REGION_NONE: u32 = u32::MAX;

/// Boxes of region sets added or removed whose probes one frame wakes, at most (#338): the rest
/// wait for the next.
const MAX_WAKE_BOXES: usize = 64;

/// Mirrors `WakeRegionsPush` in `probe_update.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WakeRegionsPush {
    field: u64,
    boxes: u64,
    count: u32,
    probes: u32,
}

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
    /// The finest cascade's own counts, when it reaches farther than the others (#353): a room as
    /// long as a hall kept in the room-sized cascade. `None`: `counts`.
    pub finest_counts: Option<[u32; 3]>,
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
    /// The share of the open sky's luminance a room takes where no room-sized cascade covers it
    /// and none of its region's probes stands among a coarser cascade's eight (#321): what its
    /// windows let in, a daylight factor. 0.06 matches the village's rooms (their own probes
    /// against the fallback forced on them); deep rooms behind small windows want less, shallow
    /// rooms with wide ones more (`--probe-far-sky`, the scene's look).
    pub far_room_sky: f32,
    /// Where a surface's point is looked up among the probes, off the surface: this share of its
    /// normal and of the way to the viewer, times 0.225 of the cascade's spacing (the 2021 paper's
    /// bias, so the visibility test does not sit on the surface's own shadow boundary). (0.2, 0.8)
    /// by default. The viewer's share moves the point as the camera moves (#351 tests whether
    /// that steps a face's light while walking, `--probe-bias`).
    pub sample_bias: [f32; 2],
    /// Rooms lit by probe grids of their own (D-078, [`RoomGridParams`]): the rooms of the
    /// scene's regions near the camera. None: the cascades light every room.
    pub rooms: Option<RoomGridParams>,
}

impl Default for ProbeParams {
    /// The city's: 24 × 12 × 24 probes in five cascades 4, 8, 16, 32 and 64 m apart (the
    /// finest reaches 40 m around the camera, the coarsest 640 m), 128 rays each, the settled
    /// ones every other frame (issue #103: the south view 2.03 → 1.81 ms).
    fn default() -> Self {
        Self {
            counts: [24, 12, 24],
            finest_counts: None,
            cascades: 5,
            spacing: 4.0,
            rays: 128,
            hysteresis: 0.97,
            cadence: 2,
            movers: false,
            far_room_sky: 0.06,
            sample_bias: [0.2, 0.8],
            rooms: None,
        }
    }
}

impl ProbeParams {
    /// Cascade `c`'s probes along x, y and z.
    pub fn cascade_counts(&self, c: u32) -> [u32; 3] {
        match self.finest_counts {
            Some(counts) if c == 0 => counts,
            _ => self.counts,
        }
    }

    /// Probes a row of the atlases: the widest cascade's x × y. A narrower cascade leaves the
    /// rest of its rows unused (`ProbeSlot::unused`).
    fn atlas_row(&self) -> u32 {
        (0..self.cascades)
            .map(|c| {
                let n = self.cascade_counts(c);
                n[0] * n[1]
            })
            .max()
            .unwrap_or(0)
    }

    /// The atlases' first row of cascade `c`: a row per z of every cascade before it.
    fn first_row(&self, c: u32) -> u32 {
        (0..c).map(|c| self.cascade_counts(c)[2]).sum()
    }

    /// Probe slots in all the cascades, the unused ones of narrower cascades included.
    pub fn cascade_probes(&self) -> u32 {
        self.atlas_row() * self.first_row(self.cascades)
    }

    /// The atlases' rows of the room grids' pool (D-078), after the cascades'.
    fn room_rows(&self) -> u32 {
        self.rooms
            .map_or(0, |r| r.probes.div_ceil(self.atlas_row().max(1)))
    }

    /// Probe slots in all: the cascades', then the room grids' pool.
    pub fn probe_count(&self) -> u32 {
        self.atlas_row() * (self.first_row(self.cascades) + self.room_rows())
    }

    /// The atlas of maps `tile` texels a side: a row of x × y tiles per z of every cascade, then
    /// the room grids' rows.
    fn atlas_size(&self, tile: u32) -> [u32; 2] {
        [
            self.atlas_row() * tile,
            (self.first_row(self.cascades) + self.room_rows()) * tile,
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

/// The probes: their maps, their rays and state, and the passes that update them.
pub struct Probes {
    params: ProbeParams,
    trace: Pipeline,
    state: Pipeline,
    blend: Pipeline,
    /// The settled probes woken where the movers pass (#79, #69).
    wake: Pipeline,
    /// The settled probes woken about a region set added or removed (#338), and per frame slot
    /// the boxes they stand in.
    wake_regions: Pipeline,
    wake_boxes: Vec<Buffer>,
    irradiance: GraphImage,
    distance: GraphImage,
    rays: GraphBuffer,
    data: GraphBuffer,
    /// Each probe's region (D-068), `REGION_NONE` until its first update with regions.
    region: GraphBuffer,
    /// The scene's region volumes in sets and the grids that find them, when it knows its rooms
    /// (D-068; added and removed as it runs, #338).
    regions: Option<RegionStore>,
    /// The set [`Self::set_regions`] made, which [`Self::set_doorway`] edits.
    whole: Option<RegionSetId>,
    /// Zeros, what the field's region pointers name in a scene without regions: one empty cell, a
    /// volume and an index, so the region lookup's reads stay in a buffer where the GPU reads
    /// ahead of its early return (`region_at` in `probes.slang`).
    no_regions: Buffer,
    fields: Vec<Buffer>,
    /// The rooms' own probe grids (D-078), with [`ProbeParams::rooms`].
    rooms: Option<RoomGrids>,
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
            params
                .counts
                .iter()
                .chain(params.finest_counts.iter().flatten())
                .all(|&n| n >= 4 && n % 2 == 0),
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
        let wake_boxes = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (MAX_WAKE_BOXES * 32) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("probe wake boxes {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
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
            wake_regions: compute(
                "probe_update.slang",
                "probe_wake_regions_main",
                std::mem::size_of::<WakeRegionsPush>(),
                "probe wake regions",
            )?,
            wake_boxes,
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
            whole: None,
            no_regions,
            fields,
            rooms: params
                .rooms
                .map(|r| {
                    let pool = RoomGridParams {
                        probes: params.probe_count() - params.cascade_probes(),
                        ..r
                    };
                    RoomGrids::new(device, pool, params.cascade_probes())
                })
                .transpose()?,
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
        self.whole = None;
        if let Some(rooms) = self.rooms.as_mut() {
            rooms.clear();
        }
        if volumes.is_empty() {
            self.regions = None;
            return Ok(());
        }
        let mut store = RegionStore::new(device, RegionStore::fitting(volumes))?;
        self.whole = Some(store.add(device, volumes, false)?);
        self.regions = Some(store);
        Ok(())
    }

    /// Shuts or opens doorway `volume` (its place in what [`Self::set_regions`] was given, #327):
    /// `Some(s)`, its door's leaf at `s` (0 to 1) of the way across from its `region` side, each
    /// side of it lit by its own probes alone; `None`, open, the doorway ramping from one side to
    /// the other. From the next frame, without starting the probes over: those round the door
    /// settle as their light changes. Any other volume is left as it is.
    pub fn set_doorway(
        &mut self,
        _device: &Arc<Device>,
        volume: usize,
        shut: Option<f32>,
    ) -> Result<()> {
        if let Some(id) = self.whole {
            self.set_doorway_of(id, volume, shut);
        }
        Ok(())
    }

    /// Makes room for region sets a scene adds and removes as it runs (#338: a streamed cell's
    /// rooms, [`Self::add_regions`]): the regions so far are dropped, and every probe starts over
    /// at the next update.
    pub fn reserve_regions(
        &mut self,
        device: &Arc<Device>,
        capacity: RegionCapacity,
    ) -> Result<()> {
        self.reset = true;
        self.whole = None;
        if let Some(rooms) = self.rooms.as_mut() {
            rooms.clear();
        }
        self.regions = Some(RegionStore::new(device, capacity)?);
        Ok(())
    }

    /// Adds `volumes` as a set (#338: a cell's rooms, its regions' ids its own, global to the
    /// scene, and its volumes not overlapping another set's), from the next frame: the probes
    /// about it are woken and take their region anew, the others keep theirs. Fails without room
    /// ([`Self::reserve_regions`]).
    pub fn add_regions(
        &mut self,
        device: &Arc<Device>,
        volumes: &[RegionVolume],
    ) -> Result<RegionSetId> {
        let store = self.regions.as_mut().ok_or_else(|| {
            forge_gpu::GpuError::Unsupported(
                "region sets need room first (`Probes::reserve_regions`)".into(),
            )
        })?;
        store.add(device, volumes, true)
    }

    /// Removes set `id` (#338) from the next frame: the probes about it are woken. Nothing for a
    /// set not in.
    pub fn remove_regions(&mut self, id: RegionSetId) -> Result<()> {
        match self.regions.as_mut() {
            Some(store) => store.remove(id),
            None => Ok(()),
        }
    }

    /// Shuts or opens doorway `volume` of set `id` (its place in what [`Self::add_regions`] was
    /// given), as [`Self::set_doorway`] does.
    pub fn set_doorway_of(&mut self, id: RegionSetId, volume: usize, shut: Option<f32>) {
        if let Some(store) = self.regions.as_mut() {
            store.set_doorway(id, volume, shut.map_or(0.0, |s| s.clamp(1e-3, 1.0)));
        }
    }

    /// The room grids live (D-078), for logs and tests: each one's region, probes and share of
    /// its room's light.
    pub fn room_grids(&self) -> Vec<(u32, u32, f32)> {
        self.rooms.as_ref().map_or_else(Vec::new, RoomGrids::live)
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
            + self.regions.as_ref().map_or(0, RegionStore::bytes)
            + self.wake_boxes.iter().map(Buffer::size).sum::<u64>()
            + self.rooms.as_ref().map_or(0, RoomGrids::bytes)
    }

    /// Every probe's offset from its cell's centre (xyz, metres) and state and age (w: the
    /// state, 0 active, 1 inactive, 2 active and starting over, plus 4 × the updates it has
    /// had, up to 8, then 9 once settled, `probe_state` in `probes.slang`) as the last
    /// submitted frame left them, in probe
    /// order (`probe_index` in `probes.slang`). Waits for the device: for logs and tests.
    pub fn read_states(&self, device: &Arc<Device>) -> Result<Vec<[f32; 4]>> {
        let bytes =
            device.read_back(&self.data, 0, u64::from(self.params.cascade_probes()) * 16)?;
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
            let counts = p.cascade_counts(c as u32);
            let origin = cascade_origin(camera, spacing, counts);
            // After a reset, a previous origin two volumes away: every probe is new.
            let previous = if self.reset {
                origin + 2 * IVec3::new(counts[0] as i32, counts[1] as i32, counts[2] as i32)
            } else {
                self.origins[c]
            };
            self.origins[c] = origin;
            *cascade = GpuCascade {
                origin: origin.to_array(),
                spacing,
                previous: previous.to_array(),
                pad: 0,
                counts,
                first_row: p.first_row(c as u32),
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
        // The regions this frame reads (#338): this slot's copies of the sets' table, the coarse
        // grid and the doorways' splits brought up to the edits; zeros without regions.
        let regions = self.regions.as_mut().map(|r| r.frame(slot.index));
        let wake_boxes = self
            .regions
            .as_mut()
            .map(RegionStore::take_wake)
            .unwrap_or_default();
        // The rooms' grids this frame reads (D-078), started over with the cascades.
        let rooms = match (self.rooms.as_mut(), self.regions.as_ref()) {
            (Some(grids), store) => {
                if first_update {
                    grids.restart();
                }
                Some(match store {
                    Some(store) => grids.frame(slot.index, camera, store.rooms()),
                    None => grids.frame(slot.index, camera, std::iter::empty()),
                })
            }
            (None, _) => None,
        };
        let field = GpuProbeField {
            rotation: [row(0), row(1), row(2)],
            cascades,
            atlas_row: p.atlas_row(),
            atlas_pad: [0; 2],
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
            region_volumes: regions.as_ref().map_or(empty, |r| r.volumes),
            region_cells: regions.as_ref().map_or(empty, |r| r.cells),
            region_indices: regions.as_ref().map_or(empty, |r| r.indices),
            region_sets: regions.as_ref().map_or(empty, |r| r.sets),
            region_top: regions.as_ref().map_or(empty, |r| r.top),
            region_top_list: regions.as_ref().map_or(empty, |r| r.top_list),
            region_splits: regions.as_ref().map_or(empty, |r| r.splits),
            region_cell: REGION_CELL,
            region_top_cell: TOP_CELL,
            region_top_mask: regions.as_ref().map_or(0, |r| r.top_mask),
            region_count: regions.as_ref().map_or(0, |r| r.count),
            far_room_sky: p.far_room_sky,
            sample_bias: p.sample_bias,
            region_pad: 0,
            room_slots: rooms.as_ref().map_or(empty, |r| r.slots),
            room_grids: rooms.as_ref().map_or(empty, |r| r.grids),
            room_table: rooms.as_ref().map_or(empty, |r| r.table),
            room_first: p.cascade_probes(),
            room_mask: rooms.as_ref().map_or(0, |r| r.table_mask),
            room_count: rooms.as_ref().map_or(0, |r| r.count),
            room_rays: p
                .rooms
                .and_then(|r| r.rays)
                .map_or(p.rays, |n| n.clamp(FIXED_RAYS + 32, p.rays)),
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
        // The settled probes about the region sets added or removed since the last frame (#338),
        // young again before this frame's rays: they take their region anew. Past
        // MAX_WAKE_BOXES, the rest as one box round them.
        if !wake_boxes.is_empty() && !first_update {
            let mut boxes: Vec<[f32; 4]> = Vec::with_capacity(2 * MAX_WAKE_BOXES);
            let (head, tail) = wake_boxes.split_at(wake_boxes.len().min(MAX_WAKE_BOXES - 1));
            let mut push_box = |lo: Vec3, hi: Vec3| {
                boxes.push(lo.extend(0.0).to_array());
                boxes.push(hi.extend(0.0).to_array());
            };
            for &(lo, hi) in head {
                push_box(lo, hi);
            }
            if !tail.is_empty() {
                let lo = tail.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(b.0));
                let hi = tail.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(b.1));
                push_box(lo, hi);
            }
            let buffer = &this.wake_boxes[slot.index];
            buffer.write(0, &boxes);
            let wake = &this.wake_regions;
            let push = WakeRegionsPush {
                field: address,
                boxes: buffer.address(),
                count: (boxes.len() / 2) as u32,
                probes: count,
            };
            graph
                .pass("gi/probe wake regions")
                .queue(QueueKind::Compute)
                .buffer(data, BufferAccess::ShaderReadWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(wake);
                    commands.push_constants(wake, &push);
                    commands.dispatch(count.div_ceil(64), 1, 1);
                    Ok(())
                });
        }
        // After a reset the frame updates every probe `RESET_ROUNDS` times (#320): the first round
        // lights its rays' hits by the sun alone, each next one by the sun and the maps the round
        // before left, and every probe is new in each (its maps replaced whole). With one round
        // a room lit only by bounce stood black on the first frame drawn (seen through a window
        // as the scene opened), and the light came on over the next ten.
        let rounds = if first_update { RESET_ROUNDS } else { 1 };
        for round in 0..rounds {
            let trace = &this.trace;
            let first_round = first_update && round == 0;
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
                            first_update: u32::from(first_round),
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
        }
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
    use super::regions::{places, region_grid, split_as_stored, turned};
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

    /// A doorway set by its place among the volumes given (#327) is the one the table holds there.
    #[test]
    fn a_volume_given_finds_its_place_in_the_lookup_order() {
        let v = little_house();
        let table = ordered(&v);
        let place = places(&v);
        for (i, given) in v.iter().enumerate() {
            let kind = match given.kind {
                RegionKind::Room => 0,
                RegionKind::Recess => 1,
                RegionKind::Doorway => 2,
                RegionKind::Solid => 3,
            };
            let record = &table[place[i] as usize];
            assert_eq!(record.kind, kind);
            assert_eq!((record.floor, record.ceiling), (given.floor, given.ceiling));
        }
        let mut seen = place.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..v.len() as u32).collect::<Vec<_>>());
    }

    /// A shut doorway's leaf stays where it was given whichever way round its outline goes: a
    /// doorway given clockwise, turned round by the lookup, had its leaf mirrored, so the door
    /// stood on the far room's side and took its probes (#327).
    #[test]
    fn a_shut_doorway_splits_where_given_either_way_round() {
        let house = little_house();
        let doorway = house
            .iter()
            .position(|v| v.kind == RegionKind::Doorway)
            .unwrap();
        let clockwise = house[doorway];
        let c = clockwise.corners;
        let anticlockwise = RegionVolume {
            corners: [c[1], c[0], c[3], c[2]],
            ..clockwise
        };
        assert!(turned(&clockwise) && !turned(&anticlockwise));
        // The leaf 0.2 of the way across from room 1's side (x from 3.89 to 4.11): at 3.934.
        let leaf = 0.2;
        for v in [clockwise, anticlockwise] {
            let mut volumes = house.clone();
            volumes[doorway] = v;
            let split = split_as_stored(leaf, turned(&v));
            for (x, room) in [(3.9, 1), (3.92, 1), (3.95, 2), (4.1, 2)] {
                let hit = region_at(&volumes, Vec3::new(x, 1.0, 1.5));
                assert_eq!(hit.kind, RegionKind::Doorway);
                // The shader's rule (`region_at` in probes.slang): a shut doorway's point is its
                // `region`'s before the leaf, its `other`'s after.
                let side = if hit.ramp < split {
                    hit.region
                } else {
                    hit.other
                };
                assert_eq!(side, room, "x {x}, turned {}", turned(&v));
            }
        }
        // Open stays open.
        assert_eq!(split_as_stored(0.0, true), 0.0);
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
