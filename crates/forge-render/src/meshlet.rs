//! GPU-driven meshlet rendering (`shaders/meshlet.slang`, `shaders/hzb.slang`).
//!
//! [`MeshletSceneBuilder`] concatenates any number of meshlet meshes and instances into the
//! GPU tables; [`MeshletRenderer`] owns the hierarchical-Z pyramid, the pipelines and the
//! per-slot frame blocks, and declares the passes of the two-pass occluded draw into the
//! render graph (the depth buffer is a transient of the frame). Culling runs in compute and
//! compacts the visible clusters into one list, which the mesh shader draws or, on GPUs
//! without `VK_EXT_mesh_shader`, one `vkCmdDrawIndexedIndirectCount` ([`GeometryPath`]).
//! Clusters smaller than a few pixels go to a software rasteriser in compute instead, which
//! keeps 64-bit samples (depth and visibility id, with an atomic maximum) where it beats the
//! hardware's pixel; a merge pass writes them into the visibility buffer and the depth.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, MeshletMesh, PAGE_NONE, PAGE_SIZE};

use crate::material::{GpuMaterial, TextureSet, gpu_rows};
use crate::probes::ProbeLight;
use crate::raytrace::{self, SceneRays};
use crate::sky::SkyLight;
use crate::streaming::{PageSource, PageStore, PageStreamer, Residency, StreamingStats};
use forge_core::material::{MaterialId, MaterialTable, ShadingClass};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, FullscreenPipelineDesc, GpuError, GraphBuffer, GraphImage, ImageAccess, ImageDesc,
    ImageHandle, MemoryCategory, MemoryLocation, MeshPipelineDesc, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, VertexPipelineDesc, vk,
};
use glam::{Mat4, Vec2, Vec3, Vec4};

/// Culling flags, mirrored in `meshlet.slang`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CullFlags(pub u32);

impl CullFlags {
    /// Keep the culling camera fixed while the view moves.
    pub const FREEZE: u32 = 1;
    /// Normal-cone backface culling.
    pub const CONE: u32 = 2;
    /// Frustum culling of instances and meshlets.
    pub const FRUSTUM: u32 = 4;
    /// Two-pass hierarchical-Z occlusion culling.
    pub const OCCLUSION: u32 = 8;
    /// Colour every meshlet differently (debug view).
    pub const MESHLET_COLORS: u32 = 16;
    /// Draw the meshlets that cone or occlusion culling rejected, tinted red, instead of
    /// skipping them: a correctly culled meshlet is back-facing or hidden and leaves no
    /// pixel, so every red pixel is a culling error (debug view).
    pub const SHOW_CULLED: u32 = 32;
    /// Select clusters by projected LOD error; off draws level 0 only (full detail).
    pub const LOD: u32 = 64;
    /// Colour every cluster by its LOD level (debug view).
    pub const LOD_COLORS: u32 = 128;
    /// Disable the per-group LOD window (debug: every group tests its clusters).
    pub const GROUP_WINDOW_OFF: u32 = 256;
    /// Tint the pixels the software rasteriser drew (debug view).
    pub const SHOW_RASTER: u32 = 1024;
    /// Trace the sun's shadows (issue #45; a scene with a top-level acceleration structure).
    pub const SHADOWS: u32 = 8192;
    /// Show the ambient occlusion in grey instead of the shading (issue #48; debug view).
    pub const SHOW_AO: u32 = 16384;
    /// Under a sky, reflect it: Fresnel-weighted, sharp on smooth surfaces (issue #49).
    pub const SKY_REFLECTIONS: u32 = 32768;
    /// On the smooth rows, trace the mirror ray against the TLAS instead of reading the sky
    /// alone (issue #50; with `SKY_REFLECTIONS`, a TLAS and ray queries).
    pub const RAY_REFLECTIONS: u32 = 65536;
    /// Translucent ice: the sun through its thickness, by rays (issue #59; with a TLAS and ray
    /// queries).
    pub const TRANSLUCENCY: u32 = 131072;
    /// Under a sky, show the diffuse light alone on white surfaces instead of the shading:
    /// the probes' where they reach, else the open sky's (issue #53; debug view).
    pub const SHOW_GI: u32 = 262144;
    /// Everything on except the debug views.
    pub const DEFAULT: Self = Self(Self::CONE | Self::FRUSTUM | Self::OCCLUSION | Self::LOD);

    /// Whether a bit is set.
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
    /// Toggles a bit.
    pub fn toggle(&mut self, bit: u32) {
        self.0 ^= bit;
    }
}

/// `FLAG_SW_RASTER` in the shader: set by the renderer, from [`DrawParams::sw_raster`], in the
/// first pass's frame block.
const FLAG_SW_RASTER: u32 = 512;
/// `FLAG_PREV_PYRAMID` in the shader: set by the renderer when pass 1 has a previous pyramid.
const FLAG_PREV_PYRAMID: u32 = 2048;
/// `FLAG_STREAMING` in the shader: set by the renderer for a streamed scene (the LOD cut
/// follows the resident pages and records the pages it wants, see `crate::streaming`).
const FLAG_STREAMING: u32 = 4096;

/// When the software rasteriser draws the dense clusters ([`DrawParams::sw_raster`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwRaster {
    /// Never: every cluster is drawn in hardware.
    Off,
    /// When the frames hold enough dense triangles to repay its fixed cost (a raster pass and
    /// a full-screen merge): from [`SW_RASTER_AUTO_ON`] dense triangles a frame until they
    /// fall below [`SW_RASTER_AUTO_OFF`]. Both ways give the same pixels.
    #[default]
    Auto,
    /// Every frame (the A/B harness, measurements).
    On,
}

impl SwRaster {
    /// Every mode, in the order R cycles them.
    pub const ALL: [Self; 3] = [Self::Off, Self::Auto, Self::On];

    /// Name for displays and the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::On => "on",
        }
    }

    /// The mode after this one.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Auto,
            Self::Auto => Self::On,
            Self::On => Self::Off,
        }
    }
}

impl std::str::FromStr for SwRaster {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(text))
            .ok_or_else(|| format!("unknown software raster mode {text:?}: off, auto or on"))
    }
}

/// [`SwRaster::Auto`] turns the software rasteriser on from this many dense triangles a frame.
pub const SW_RASTER_AUTO_ON: u32 = 1_500_000;
/// ...and off again below this many.
pub const SW_RASTER_AUTO_OFF: u32 = 750_000;

const PASS_PREVIOUSLY_VISIBLE: u32 = 1;
const PASS_REMAINDER: u32 = 2;
const PASS_SINGLE: u32 = 3;
const TASK_GROUP_SIZE: u32 = 32;
const STAT_COUNT: usize = 13;
const STATS_BYTES: u64 = (STAT_COUNT * 4) as u64;
/// LOD levels a mesh may have on the GPU (mirrors `forge_geom::MAX_LEVELS`).
const LOD_LEVELS: usize = forge_geom::MAX_LEVELS as usize;
const FRAME_BLOCK_STRIDE: u64 = 1024;

/// Mirrors `Mesh` in `meshlet.slang` (400 bytes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMesh {
    meshlet_offset: u32,
    meshlet_count: u32,
    triangle_count: u32,
    level_count: u32,
    center: [f32; 3],
    radius: f32,
    /// Cluster offsets per LOD level relative to `meshlet_offset`, plus the end.
    level_offset: [u32; LOD_LEVELS + 1],
    pad: [u32; 3],
    /// Per level: the smallest `self_error` of its clusters.
    self_error_min: [f32; LOD_LEVELS],
    /// Per level: the largest `parent_error` (infinite when the level holds a root).
    parent_error_max: [f32; LOD_LEVELS],
    /// Per level: how far a cluster's `self` sphere reaches from the mesh centre.
    self_reach_max: [f32; LOD_LEVELS],
    /// Per level: the same for the `parent` spheres.
    parent_reach_max: [f32; LOD_LEVELS],
    /// The DAG's roots, local indices, when there are at most [`MAX_SHORTCUT_ROOTS`].
    roots: [u32; MAX_SHORTCUT_ROOTS],
    /// How many of `roots` there are; 0 when the mesh has more (no root shortcut).
    root_count: u32,
    /// The largest `self_error` of the roots.
    root_error_max: f32,
    /// How far the roots' `self` spheres reach from the mesh centre.
    root_reach_max: f32,
    /// The material its instances take unless they name their own (GPU placement reads it).
    material: u32,
}

impl GpuMesh {
    /// What an instance can put in either work list: all its groups of 32 clusters, or its
    /// roots.
    fn work_bound(&self) -> u32 {
        self.meshlet_count
            .div_ceil(TASK_GROUP_SIZE)
            .max(self.root_count)
    }
}

/// Roots a mesh may have for the instance cull's root shortcut (`instance_roots` in
/// `meshlet.slang`): an instance whose roots are all fine enough lists them in the root list
/// instead of taking work items.
const MAX_SHORTCUT_ROOTS: usize = 4;
const _: () = assert!(std::mem::size_of::<GpuMesh>() == 400);

/// Mirrors `Instance` in `meshlet.slang` (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuInstance {
    model: [f32; 16],
    center: [f32; 3],
    radius: f32,
    mesh: u32,
    id: u32,
    /// Its row in the material table.
    material: u32,
    pad: u32,
}

/// Mirrors `Frame` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFrame {
    view_proj: [f32; 16],
    cull_view_proj: [f32; 16],
    cull_view: [f32; 16],
    /// The previous frame's culling view (with `FLAG_PREV_PYRAMID`).
    prev_cull_view: [f32; 16],
    planes: [[f32; 4]; 6],
    camera_pos: [f32; 3],
    instance_count: u32,
    /// Slots in `work`.
    work_capacity: u32,
    flags: u32,
    pass: u32,
    hzb_image: u32,
    hzb_size: [u32; 2],
    p00: f32,
    p11: f32,
    znear: f32,
    sun_dir: [f32; 3],
    draw_jitter: [f32; 2],
    lod_threshold: f32,
    viewport_height: f32,
    /// The resident cluster pages.
    pool: u64,
    meshlets: u64,
    /// Per page of the scene, its slot in `pool`.
    page_table: u64,
    meshes: u64,
    instances: u64,
    stats: u64,
    /// The cluster culls' status words, `cluster_groups` per pass.
    cluster_lookback: u64,
    cluster_groups: u32,
    /// The previous frame's pyramid (sampled-image index).
    prev_hzb_image: u32,
    work: u64,
    indirect: u64,
    /// The visible-cluster list: (instance, meshlet | flags) per listed cluster.
    visible: u64,
    visible_capacity: u32,
    /// Pre-exposure of the frame (see `crate::exposure`).
    exposure: f32,
    /// Illuminance of the sun in lux.
    sun_illuminance: f32,
    /// The sun disc's angular radius, radians: soft shadows (issue #54); 0 for hard ones.
    sun_angular_radius: f32,
    /// Per pass: the draw grid (x, y, 1) and the cluster count.
    clusters: u64,
    /// The fallback's indexed draws (0 on the mesh path).
    draws: u64,
    /// 64-bit status words of the instance cull's ordered appends.
    lookback: u64,
    /// Per pass, the list slots of the hardware-drawn clusters (from the front) and of the
    /// software-rasterised ones (from the back).
    raster: u64,
    target_width: u32,
    target_height: u32,
    /// Clusters with fewer pixels of bounding rectangle per triangle are rasterised in compute.
    sw_raster_area: f32,
    /// The frame index of the spatio-temporal noise (`noise.slang`): TAA's, modulo its jitter's period.
    noise_frame: u32,
    /// The previous frame's `draw_jitter`, `p00` and `p11`.
    prev_draw_jitter: [f32; 2],
    prev_p00: f32,
    prev_p11: f32,
    /// The root list: (instance, local cluster) per root the instance cull lists.
    roots: u64,
    /// A streamed scene's page needs (a float's bits per page), 0 otherwise.
    page_need: u64,
    /// The material table ([`GpuMaterial`] rows).
    materials: u64,
    /// The sunlight's colour at the scene (rgb; w unused).
    sun_color: [f32; 4],
    /// The scene's top-level acceleration structure (0: none, no shadow rays).
    tlas: u64,
    /// What a ray's hit reads to shade its triangle (`RtScene` in `meshlet.slang`, issue #50; 0:
    /// none).
    rt_scene: u64,
}

const _: () = assert!(std::mem::offset_of!(GpuFrame, sun_color) % 16 == 0);
// Both passes' blocks share one buffer at this stride.
const _: () = assert!(std::mem::size_of::<GpuFrame>() as u64 <= FRAME_BLOCK_STRIDE);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    frame: u64,
    /// The software rasteriser's samples, and the target width again (a pixel reaches
    /// them without a load through `frame`).
    vis64: u64,
    target_width: u32,
    /// Software raster: sampled-image indices of the hardware's depth and visibility buffer.
    depth_image: u32,
    vis_image: u32,
    pad: u32,
}

/// Mirrors `ResolvePush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolvePush {
    frame: u64,
    /// Per class, `tile_capacity` tiles (x | y << 16) that show it.
    tiles: u64,
    /// Per class: its dispatch (`TILE_GROUPS_X`, rows, 1) and its tile count.
    class_args: u64,
    vis_image: u32,
    color_image: u32,
    width: u32,
    height: u32,
    use_background: u32,
    tile_capacity: u32,
    background: [f32; 4],
    /// The sky's irradiance coefficients (`sh.slang`, issue #47), or 0: space's constant fill.
    sky: u64,
    /// Sampled index of the ambient occlusion scaling the sky's light (issue #48), or `u32::MAX`.
    ao_image: u32,
    /// Storage index of the mirror rays' requests (direction, weight; issue #52), or `u32::MAX`.
    request_image: u32,
    /// The probes' `ProbeField` (`probes.slang`, issue #53), or 0: the sky's irradiance.
    probes: u64,
}

const _: () = assert!(std::mem::size_of::<ResolvePush>() == 88);

/// What lights the resolve besides the sun (issues #47, #48, #53).
#[derive(Clone, Copy, Debug, Default)]
pub struct AmbientLight {
    /// The sky's irradiance; `None` keeps space's constant fill.
    pub sky: Option<SkyLight>,
    /// Ambient occlusion ([`crate::Gtao`]: r32f, the frame's size) scaling the sky's light.
    pub occlusion: Option<ImageHandle>,
    /// Under a sky, the probes' light ([`crate::Probes::update`]) in place of the sky's
    /// irradiance, on the pixels and on what the mirror rays meet.
    pub probes: Option<ProbeLight>,
}

/// Width of a shading class's dispatch in workgroups (`TILE_GROUPS_X` in `meshlet.slang`):
/// it grows in rows, so a class can hold more tiles than one dimension of a grid allows.
const TILE_GROUPS_X: u32 = 64;

/// Shading classes: one resolve pipeline each (`MATERIAL_CLASSES` in `meshlet.slang`).
const MATERIAL_CLASSES: usize = ShadingClass::ALL.len();
/// Tile lists: one per class, then the smooth rows' for the mirror rays (`REFLECTION_LIST` in
/// `meshlet.slang`, issue #52).
const TILE_LISTS: usize = MATERIAL_CLASSES + 1;
const REFLECTION_LIST: u64 = MATERIAL_CLASSES as u64;

/// The shading tiles of a target: 8 × 8 pixels each.
fn tile_count(extent: vk::Extent2D) -> u32 {
    extent.width.div_ceil(8) * extent.height.div_ceil(8)
}

/// The tile lists (every list can hold every tile) and their dispatches.
fn create_shading_tiles(device: &Arc<Device>, extent: vk::Extent2D) -> Result<[GraphBuffer; 2]> {
    let tiles = device.create_buffer(BufferDesc {
        size: TILE_LISTS as u64 * u64::from(tile_count(extent)) * 4,
        usage: vk::BufferUsageFlags::STORAGE_BUFFER,
        location: MemoryLocation::GpuOnly,
        category: MemoryCategory::Work,
        name: "shading tiles",
    })?;
    let args = device.create_buffer(BufferDesc {
        size: TILE_LISTS as u64 * 16,
        usage: vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::INDIRECT_BUFFER
            | vk::BufferUsageFlags::TRANSFER_DST,
        location: MemoryLocation::GpuOnly,
        category: MemoryCategory::Work,
        name: "shading class dispatches",
    })?;
    Ok([GraphBuffer::new(tiles), GraphBuffer::new(args)])
}

/// Slots the visible-cluster list starts with per frame slot, both passes together (512 KiB;
/// the LOD views of both demos list 8–15 k clusters). The cluster cull drops and counts what
/// does not fit (`FrameStats::visible_overflow`) and [`MeshletRenderer::begin_frame`] grows
/// the list from that count.
pub const VISIBLE_INITIAL_CAPACITY: u32 = 1 << 16;

/// Most slots the list can grow to: the visibility id keeps 7 bits for the triangle
/// (`slot << 7 | triangle`), the slot has the other 25 (256 MiB of list per frame slot).
pub const VISIBLE_MAX_CAPACITY: u32 = 1 << (32 - 7);
const _: () = assert!(VISIBLE_INITIAL_CAPACITY <= VISIBLE_MAX_CAPACITY);

/// The list size after a frame that wanted `wanted` slots: unchanged while they fit, else the
/// next power of two above one and a half times the demand (so a slowly rising demand does
/// not regrow every few frames), at most `max`. It never shrinks.
fn grown_capacity(current: u32, wanted: u64, max: u32) -> u32 {
    if wanted <= u64::from(current) {
        return current;
    }
    let target = (wanted + wanted / 2)
        .next_power_of_two()
        .min(u64::from(max));
    target.max(u64::from(current)) as u32
}

/// [`SwRaster::Auto`]'s decision after a frame with `dense_triangles`, from its state `on`:
/// on from [`SW_RASTER_AUTO_ON`], off below [`SW_RASTER_AUTO_OFF`], unchanged between the
/// two, so that a demand near one threshold does not flip it every frame.
fn software_worth_it(on: bool, dense_triangles: u32) -> bool {
    if on {
        dense_triangles >= SW_RASTER_AUTO_OFF
    } else {
        dense_triangles >= SW_RASTER_AUTO_ON
    }
}

/// Bytes of one `VkDrawIndexedIndirectCommand`.
const DRAW_COMMAND_BYTES: u32 = 20;

/// The cluster arguments at the start of a frame (`Frame::clusters`): per pass, the hardware
/// draw grid and its count, the software raster grid and its count and the merge's draw
/// (vertex count written by the cull), all empty; then the culls' tickets.
const CLUSTER_ARGS_START: [u32; 26] = [
    0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, // pass 1 (or the single pass)
    0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, // pass 2 (no software clusters: see `frame_block`)
    0, 0, // tickets
];
/// Bytes of one pass's cluster arguments.
const CLUSTER_ARGS_PASS_BYTES: u64 = 48;

/// Default of [`DrawParams::sw_raster_area`].
pub const SW_RASTER_DEFAULT_AREA: f32 = 2.0;
/// Largest cluster the software rasteriser takes, in pixels across its bounding rectangle
/// (`SW_RASTER_MAX_PX` in the shader): its integer edge functions take triangles up to 96
/// pixels across (`SW_RASTER_MAX_EXTENT`), and the rectangle bounds the triangles.
pub const SW_RASTER_MAX_PX: f32 = 64.0;

/// How the clusters that survive culling reach the rasteriser. Both paths share the compute
/// culling and its compacted list of visible clusters, so they produce the same pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryPath {
    /// One mesh workgroup per listed cluster (`VK_EXT_mesh_shader`).
    MeshShader,
    /// One indexed draw per listed cluster through `vkCmdDrawIndexedIndirectCount`, the
    /// cooked one-byte triangle lists as the index buffer (GPUs without mesh shaders).
    IndirectCount,
}

impl GeometryPath {
    /// Name for displays.
    pub fn name(self) -> &'static str {
        match self {
            Self::MeshShader => "mesh shaders",
            Self::IndirectCount => "indirect-count fallback",
        }
    }
}

/// What a frame's draw leaves behind for the passes after it.
#[derive(Clone, Copy, Debug)]
pub struct DrawTargets {
    /// The depth buffer (a transient), `D32_SFLOAT`.
    pub depth: ImageHandle,
    /// The visibility buffer (a transient), `R32_UINT`: `visible_slot << 7 | triangle`, or
    /// `u32::MAX` where nothing was drawn (see `forge_render::visibility`).
    pub visibility: ImageHandle,
    /// The visible-cluster list the ids index (this frame slot's): `(instance, meshlet | flags)`
    /// per slot, pass 1 then pass 2.
    pub visible_list: forge_gpu::BufferHandle,
    /// The scene's page pool and page table, which hold the clusters' vertices.
    pub pages: forge_gpu::BufferHandle,
    /// See `pages`.
    pub page_table: forge_gpu::BufferHandle,
}

/// Mirrors `Push` in `hzb.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HzbPush {
    src_image: u32,
    src_level: u32,
    dst_image: u32,
    dst_width: u32,
    dst_height: u32,
    pad: [u32; 3],
}

/// Handle of a mesh added to a [`MeshletSceneBuilder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshId(u32);

impl MeshId {
    /// The mesh's index in the scene's mesh table.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Concatenates meshes and instances into the GPU tables.
#[derive(Default)]
pub struct MeshletSceneBuilder {
    meshlets: Vec<GpuMeshlet>,
    /// Every mesh's cluster pages, one after the other: where to read each.
    store: PageStore,
    /// The pages that hold roots (always resident).
    root_pages: Vec<u32>,
    meshes: Vec<GpuMesh>,
    instances: Vec<GpuInstance>,
    total_triangles: u64,
    /// Per mesh: its finest-level clusters (LOD level 0).
    mesh_finest: Vec<u32>,
    /// Per mesh: its highest material section (its instances need that many rows after theirs).
    mesh_sections: Vec<u32>,
    finest_clusters: u64,
    /// Work items if every instance were visible with every level possible (groups of 32
    /// clusters), or its roots when it has more of those: the bound of the work lists.
    work_bound: u64,
    /// Clusters over all instances.
    instance_meshlets: u64,
    /// The material table (the default row when none is set).
    materials: Vec<GpuMaterial>,
    /// The textures its rows sample.
    textures: Option<TextureSet>,
    /// Build acceleration structures for shadow rays (issue #45), when the device has ray queries.
    ray_traced: bool,
}

impl MeshletSceneBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a mesh; its pages are rebased into the scene's page table. Pages in memory are
    /// copied; pages left in a file ([`MeshletMesh::page_file`]) are read from it when needed.
    pub fn add_mesh(&mut self, mesh: &MeshletMesh) -> MeshId {
        let page_base = self.store.sources.len() as u32;
        let meshlet_offset = self.meshlets.len() as u32;
        match &mesh.page_file {
            Some(file) if mesh.pages.is_empty() => {
                let index = self.store.files.len() as u32;
                self.store.files.push(file.path.clone());
                self.store
                    .sources
                    .extend((0..u64::from(mesh.page_count)).map(|p| PageSource::File {
                        file: index,
                        offset: file.offset + p * PAGE_SIZE as u64,
                    }));
            }
            _ => {
                let at = self.store.memory.len();
                self.store.memory.extend_from_slice(&mesh.pages);
                self.store.sources.extend(
                    (0..mesh.page_count as usize).map(|p| PageSource::Memory(at + p * PAGE_SIZE)),
                );
            }
        }
        self.root_pages
            .extend((0..mesh.root_pages).map(|p| page_base + p));
        self.meshlets
            .extend(mesh.meshlets.iter().map(|m| GpuMeshlet {
                page: m.page + page_base,
                child_page: if m.child_page == PAGE_NONE {
                    PAGE_NONE
                } else {
                    m.child_page + page_base
                },
                ..*m
            }));
        let id = MeshId(self.meshes.len() as u32);
        // Per-level tables for the culls' LOD windows: clusters are stored level by level.
        let level_count = mesh.clusters_per_level.len().min(LOD_LEVELS);
        let mut level_offset = [0_u32; LOD_LEVELS + 1];
        let mut self_error_min = [f32::INFINITY; LOD_LEVELS];
        let mut parent_error_max = [0.0_f32; LOD_LEVELS];
        let mut self_reach_max = [0.0_f32; LOD_LEVELS];
        let mut parent_reach_max = [0.0_f32; LOD_LEVELS];
        for level in 0..level_count {
            level_offset[level + 1] = level_offset[level] + mesh.clusters_per_level[level];
        }
        let reach = |c: [f32; 3], r: f32| (Vec3::from(c) - Vec3::from(mesh.center)).length() + r;
        let mut roots = [0_u32; MAX_SHORTCUT_ROOTS];
        let mut root_count = 0;
        let mut root_error_max = 0.0_f32;
        let mut root_reach_max = 0.0_f32;
        for (index, m) in mesh.meshlets.iter().enumerate() {
            if m.parent_error.is_infinite() {
                if root_count < MAX_SHORTCUT_ROOTS {
                    roots[root_count] = index as u32;
                }
                root_count += 1;
                root_error_max = root_error_max.max(m.self_error);
                root_reach_max = root_reach_max.max(reach(m.self_center, m.self_radius));
            }
        }
        if root_count > MAX_SHORTCUT_ROOTS {
            root_count = 0;
        }
        for m in &mesh.meshlets {
            let level = (m.lod_level as usize).min(level_count.saturating_sub(1));
            self_error_min[level] = self_error_min[level].min(m.self_error);
            parent_error_max[level] = parent_error_max[level].max(m.parent_error);
            self_reach_max[level] = self_reach_max[level].max(reach(m.self_center, m.self_radius));
            parent_reach_max[level] =
                parent_reach_max[level].max(reach(m.parent_center, m.parent_radius));
        }
        for value in self_error_min.iter_mut().skip(level_count) {
            *value = 0.0;
        }
        self.meshes.push(GpuMesh {
            meshlet_offset,
            meshlet_count: mesh.meshlets.len() as u32,
            triangle_count: mesh.triangle_count as u32,
            level_count: level_count as u32,
            center: mesh.center,
            radius: mesh.radius,
            level_offset,
            pad: [0; 3],
            self_error_min,
            parent_error_max,
            self_reach_max,
            parent_reach_max,
            roots,
            root_count: root_count as u32,
            root_error_max,
            root_reach_max,
            material: MaterialTable::DEFAULT.0,
        });
        self.mesh_finest
            .push(mesh.meshlets.iter().filter(|m| m.lod_level == 0).count() as u32);
        self.mesh_sections.push(
            mesh.meshlets
                .iter()
                .map(|m| (m.section & 0xFF).max((m.section >> 8) & 0xFF))
                .max()
                .unwrap_or(0),
        );
        id
    }

    /// Sets the material the instances of `mesh` take unless they name their own: those
    /// added after this call, and those the GPU placement writes.
    pub fn set_mesh_material(&mut self, mesh: MeshId, material: MaterialId) {
        self.meshes[mesh.0 as usize].material = material.0;
    }

    /// Sets the material table and the textures its rows sample (the default table, a
    /// single grey row, otherwise). The scene keeps the textures alive.
    pub fn set_materials(&mut self, table: &MaterialTable, textures: Option<TextureSet>) {
        self.materials = gpu_rows(table, textures.as_ref());
        self.textures = textures;
    }

    /// Asks for acceleration structures (issue #45): one per mesh at [`MeshletSceneBuilder::build_with`]
    /// (a cut of its DAG), the top-level one at [`MeshletScene::build_tlas`] once the instances
    /// are written. Ignored on devices without ray queries.
    pub fn set_ray_traced(&mut self, on: bool) {
        self.ray_traced = on;
    }

    /// Adds an instance of `mesh` with a uniform-scale transform, in its mesh's material.
    pub fn add_instance(&mut self, mesh: MeshId, model: Mat4) {
        let material = MaterialId(self.meshes[mesh.0 as usize].material);
        self.add_instance_with_material(mesh, model, material);
    }

    /// Adds an instance of `mesh` with a uniform-scale transform, in `material`.
    pub fn add_instance_with_material(&mut self, mesh: MeshId, model: Mat4, material: MaterialId) {
        let info = self.meshes[mesh.0 as usize];
        let scale = model.x_axis.truncate().length();
        let center = model.transform_point3(Vec3::from(info.center));
        self.total_triangles += u64::from(info.triangle_count);
        self.finest_clusters += u64::from(self.mesh_finest[mesh.0 as usize]);
        self.work_bound += u64::from(info.work_bound());
        self.instance_meshlets += u64::from(info.meshlet_count);
        self.instances.push(GpuInstance {
            model: model.to_cols_array(),
            center: center.to_array(),
            radius: info.radius * scale,
            mesh: mesh.0,
            id: self.instances.len() as u32,
            material: material.0,
            pad: 0,
        });
    }

    /// Number of instances so far.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Reserves instance slots for a GPU pass to fill after [`MeshletSceneBuilder::build`]
    /// (`crate::placement`): `per_mesh` says how many of them show each mesh, which the
    /// scene's counts (triangles, work bound, finest clusters) need. Returns the first slot.
    pub fn reserve_instances(&mut self, per_mesh: &[(MeshId, u32)]) -> u32 {
        let first = self.instances.len() as u32;
        for &(mesh, count) in per_mesh {
            let info = self.meshes[mesh.0 as usize];
            let n = u64::from(count);
            self.total_triangles += u64::from(info.triangle_count) * n;
            self.finest_clusters += u64::from(self.mesh_finest[mesh.0 as usize]) * n;
            self.work_bound += u64::from(info.work_bound()) * n;
            self.instance_meshlets += u64::from(info.meshlet_count) * n;
            // Placeholders: the GPU pass writes every one of them before the first frame.
            self.instances.extend(std::iter::repeat_n(
                GpuInstance {
                    model: Mat4::IDENTITY.to_cols_array(),
                    center: [0.0; 3],
                    radius: 0.0,
                    mesh: mesh.0,
                    id: 0,
                    material: info.material,
                    pad: 0,
                },
                count as usize,
            ));
        }
        first
    }

    /// Uploads everything, every page resident ([`Residency::All`]).
    pub fn build(self, device: &Arc<Device>) -> Result<MeshletScene> {
        self.build_with(device, Residency::All)
    }

    /// Uploads the tables and makes the pages resident as `residency` says: all of them,
    /// each in the slot of its own index, or the roots' pages in a pool that streams the
    /// rest.
    pub fn build_with(
        mut self,
        device: &Arc<Device>,
        residency: Residency,
    ) -> Result<MeshletScene> {
        let page_count = self.store.sources.len() as u32;
        let usage = vk::BufferUsageFlags::STORAGE_BUFFER;
        // The meshes' cuts for the shadow rays, read while the page store is still here.
        let rays = if self.ray_traced && device.features().ray_query {
            let start = std::time::Instant::now();
            let cuts = self
                .meshes
                .iter()
                .map(|mesh| {
                    let first = mesh.meshlet_offset as usize;
                    let meshlets = &self.meshlets[first..first + mesh.meshlet_count as usize];
                    let budget = if mesh.radius > 1000.0 {
                        raytrace::TERRAIN_BUDGET
                    } else {
                        raytrace::TRIANGLE_BUDGET
                    };
                    raytrace::mesh_cut(meshlets, &self.store, budget)
                })
                .collect::<Result<Vec<_>>>()?;
            let ms = start.elapsed().as_secs_f64() * 1e3;
            Some(SceneRays::new(device, &cuts, ms)?)
        } else {
            None
        };
        // Also the fallback path's index buffer (8-bit indices, one cluster per draw).
        let pool_usage = usage | vk::BufferUsageFlags::INDEX_BUFFER;
        let (pool, page_table, streamer) = match residency {
            Residency::All => {
                let bytes = self.store.read_pages(0..page_count)?;
                // The shaders address the pool in 32-bit bytes (`payload_base`).
                if bytes.len() as u64 >= 1 << 32 {
                    return Err(GpuError::Unsupported(format!(
                        "{} MiB of cluster pages exceed what the shaders address: stream them",
                        bytes.len() >> 20
                    )));
                }
                let table: Vec<u32> = (0..page_count.max(1)).collect();
                (
                    device.create_buffer_with_data(
                        &bytes,
                        pool_usage,
                        MemoryCategory::Geometry,
                        "cluster pages",
                    )?,
                    device.create_buffer_with_data(
                        &table,
                        usage,
                        MemoryCategory::Geometry,
                        "page table",
                    )?,
                    None,
                )
            }
            Residency::Streamed(config) => {
                let pinned = std::mem::take(&mut self.root_pages);
                if config.pool_pages as usize <= pinned.len()
                    || u64::from(config.pool_pages) * PAGE_SIZE as u64 >= 1 << 32
                {
                    return Err(GpuError::Unsupported(format!(
                        "a pool of {} pages for {} root pages (and under 4 GiB)",
                        config.pool_pages,
                        pinned.len()
                    )));
                }
                let pool = device.create_buffer(BufferDesc {
                    size: u64::from(config.pool_pages) * PAGE_SIZE as u64,
                    usage: pool_usage | vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuOnly,
                    category: MemoryCategory::Geometry,
                    name: "cluster page pool",
                })?;
                device.write_buffer_staged(
                    &pool,
                    0,
                    &self.store.read_pages(pinned.iter().copied())?,
                )?;
                let mut table = vec![PAGE_NONE; page_count.max(1) as usize];
                for (slot, &page) in pinned.iter().enumerate() {
                    table[page as usize] = slot as u32;
                }
                let page_table = device.create_buffer_with_data(
                    &table,
                    usage,
                    MemoryCategory::Geometry,
                    "page table",
                )?;
                let store = Arc::new(std::mem::take(&mut self.store));
                let streamer = PageStreamer::new(device, config, store, &self.meshlets, &pinned)?;
                tracing::info!(
                    pages = page_count,
                    pool_pages = config.pool_pages,
                    root_pages = pinned.len(),
                    upload_pages = config.upload_pages,
                    "cluster pages streamed"
                );
                (pool, page_table, Some(streamer))
            }
        };
        if self.materials.is_empty() {
            self.materials = gpu_rows(&MaterialTable::new(), None);
        }
        // A mesh's sections take the rows after its instance's: they must exist.
        if let Some(bad) = self.instances.iter().find(|i| {
            (i.material + self.mesh_sections[i.mesh as usize]) as usize >= self.materials.len()
        }) {
            return Err(GpuError::Unsupported(format!(
                "instance {} of mesh {} takes material {} and {} section rows after it, but the \n                 table has {} rows",
                bad.id,
                bad.mesh,
                bad.material,
                self.mesh_sections[bad.mesh as usize],
                self.materials.len()
            )));
        }
        let max_meshlets = self
            .meshes
            .iter()
            .map(|m| m.meshlet_count)
            .max()
            .unwrap_or(0);
        Ok(MeshletScene {
            pool: GraphBuffer::new(pool),
            meshlets: device.create_buffer_with_data(
                &self.meshlets,
                usage,
                MemoryCategory::Geometry,
                "meshlets",
            )?,
            page_table: GraphBuffer::new(page_table),
            streamer,
            page_count,
            meshes: device.create_buffer_with_data(
                &self.meshes,
                usage,
                MemoryCategory::Geometry,
                "meshes",
            )?,
            // Written by GPU placement (`crate::placement`) and read back once for its checksum.
            instances: device.create_buffer_with_data(
                &self.instances,
                usage | vk::BufferUsageFlags::TRANSFER_SRC,
                MemoryCategory::Geometry,
                "instances",
            )?,
            indirect: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device
                        .create_buffer(BufferDesc {
                            size: 32,
                            usage: usage | vk::BufferUsageFlags::INDIRECT_BUFFER,
                            location: MemoryLocation::CpuToGpu,
                            category: MemoryCategory::Frame,
                            name: &format!("cluster cull grid {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            clusters: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device
                        .create_buffer(BufferDesc {
                            size: std::mem::size_of_val(&CLUSTER_ARGS_START) as u64,
                            usage: usage | vk::BufferUsageFlags::INDIRECT_BUFFER,
                            location: MemoryLocation::CpuToGpu,
                            category: MemoryCategory::Frame,
                            name: &format!("cluster draw grids {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            lookback: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    let instance_groups = (self.instances.len() as u64).div_ceil(64).max(1);
                    device
                        .create_buffer(BufferDesc {
                            size: instance_groups * 8,
                            usage: usage | vk::BufferUsageFlags::TRANSFER_DST,
                            location: MemoryLocation::GpuOnly,
                            category: MemoryCategory::Work,
                            name: &format!("instance cull look-back {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            rays,
            materials: device.create_buffer_with_data(
                &self.materials,
                usage,
                MemoryCategory::Geometry,
                "materials",
            )?,
            material_count: self.materials.len() as u32,
            textures: self.textures.take(),
            instance_count: self.instances.len() as u32,
            work_bound: self.work_bound,
            instance_meshlets: self.instance_meshlets,
            max_meshlets,
            mesh_count: self.meshes.len() as u32,
            meshlet_count: self.meshlets.len() as u32,
            total_triangles: self.total_triangles,
            finest_clusters: self.finest_clusters,
        })
    }
}

/// The uploaded scene tables.
pub struct MeshletScene {
    /// The resident cluster pages: all of them in the slot of their index, or a streamed
    /// pool (`streamer`).
    pool: GraphBuffer,
    meshlets: Buffer,
    /// Per page, its slot in `pool` (`PAGE_NONE` when absent).
    page_table: GraphBuffer,
    /// A streamed scene's residency.
    streamer: Option<PageStreamer>,
    /// Pages over all meshes.
    pub page_count: u32,
    meshes: Buffer,
    instances: Buffer,
    /// Per frame slot: the cluster cull's indirect grid (x, y, 1, work item count), then the
    /// instance cull's ticket counter.
    indirect: Vec<GraphBuffer>,
    /// Per frame slot: per pass (pass 1 or the single pass, then pass 2) the draw grid
    /// (x, y, 1) and the count of listed clusters, then the two cluster culls' tickets.
    clusters: Vec<GraphBuffer>,
    /// Per frame slot: the status words that keep the instance cull's appends in a fixed
    /// order (one per instance-cull workgroup; the cluster culls' are the renderer's).
    lookback: Vec<GraphBuffer>,
    /// Acceleration structures for shadow rays, when asked for and supported.
    rays: Option<SceneRays>,
    /// The material table ([`GpuMaterial`] rows) and the textures they sample.
    materials: Buffer,
    /// Rows in `materials`.
    pub material_count: u32,
    textures: Option<TextureSet>,
    /// Instances.
    pub instance_count: u32,
    /// Work items if every instance were visible with every LOD level possible (groups of 32
    /// clusters), or its roots when it has more of those: the most a frame can put in either
    /// work list; a frame puts far fewer.
    pub work_bound: u64,
    instance_meshlets: u64,
    /// Largest meshlet count of any mesh.
    pub max_meshlets: u32,
    /// Distinct meshes.
    pub mesh_count: u32,
    /// Meshlets over all meshes.
    pub meshlet_count: u32,
    /// Triangles over all instances.
    pub total_triangles: u64,
    /// Finest-level clusters over all instances: the most a frame can list with LOD off (each
    /// is drawn at most once per frame; see [`MeshletRenderer::reserve_visible`]).
    pub finest_clusters: u64,
}

impl MeshletScene {
    /// Builds the top-level acceleration structure over the instances (issue #45), once they
    /// are written (after GPU placement). Nothing without [`MeshletSceneBuilder::set_ray_traced`]
    /// or ray queries.
    pub fn build_tlas(&mut self, device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<()> {
        if let Some(rays) = &mut self.rays {
            rays.build_tlas(device, shaders, &self.instances, self.instance_count)?;
        }
        Ok(())
    }

    /// The acceleration structures, when the scene has them.
    pub fn rays(&self) -> Option<&SceneRays> {
        self.rays.as_ref()
    }

    /// Bytes of texture the materials sample (every mip level).
    pub fn texture_bytes(&self) -> u64 {
        self.textures.as_ref().map_or(0, TextureSet::bytes)
    }

    /// Meshlets over all instances (the culling universe).
    pub fn instance_meshlets(&self) -> u64 {
        self.instance_meshlets
    }

    /// The instance table (`Instance` in `meshlet.slang`, 96 bytes each).
    pub(crate) fn instance_buffer(&self) -> &Buffer {
        &self.instances
    }

    /// The mesh table (`Mesh` in `meshlet.slang`).
    pub(crate) fn mesh_buffer(&self) -> &Buffer {
        &self.meshes
    }

    /// What the streamer did last frame, for a streamed scene ([`Residency::Streamed`]).
    pub fn streaming(&self) -> Option<StreamingStats> {
        self.streamer.as_ref().map(PageStreamer::stats)
    }
}

/// The culling camera, frozen as a unit by the freeze flag.
#[derive(Clone, Copy, Debug)]
pub struct CullCamera {
    /// World → view.
    pub view: Mat4,
    /// World → clip.
    pub view_proj: Mat4,
    /// Camera position.
    pub position: Vec3,
    /// Projection scale x.
    pub p00: f32,
    /// Projection scale y.
    pub p11: f32,
    /// Near plane distance (reversed-Z infinite projection).
    pub near: f32,
}

impl CullCamera {
    /// From a view matrix, a projection matrix and the camera position.
    pub fn new(view: Mat4, proj: Mat4, position: Vec3, near: f32) -> Self {
        Self {
            view,
            view_proj: proj * view,
            position,
            p00: proj.x_axis.x,
            p11: proj.y_axis.y,
            near,
        }
    }
}

/// Frustum planes (inward normals), Gribb–Hartmann; the far plane is replaced by an
/// always-pass plane because the projection is infinite.
fn frustum_planes(view_proj: Mat4) -> [[f32; 4]; 6] {
    let m = view_proj.transpose();
    let rows = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
    let plane = |p: Vec4| {
        let len = p.truncate().length().max(1e-6);
        (p / len).to_array()
    };
    [
        plane(rows[3] + rows[0]),
        plane(rows[3] - rows[0]),
        plane(rows[3] + rows[1]),
        plane(rows[3] - rows[1]),
        plane(rows[2]),
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Per-frame counters read back two frames later.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    /// Meshlets drawn in pass 1 (previously visible) or in the single pass.
    pub meshlets_pass1: u32,
    /// Meshlets drawn in pass 2 (newly visible).
    pub meshlets_pass2: u32,
    /// Triangles emitted.
    pub triangles: u32,
    /// Meshlets rejected by the depth pyramid.
    pub occluded: u32,
    /// Instances that passed the frustum test.
    pub instances_visible: u32,
    /// Sum of the LOD level of every drawn meshlet (mean = sum / drawn).
    pub lod_level_sum: u32,
    /// Clusters dropped because the visible-cluster list was full: the frames in flight when
    /// the demand jumps, until [`MeshletRenderer::begin_frame`] has grown every slot's list.
    pub visible_overflow: u32,
    /// Clusters rasterised in compute (included in the pass counts).
    pub sw_clusters: u32,
    /// Their triangles (included in `triangles`).
    pub sw_triangles: u32,
    /// Work items the instance cull emitted (groups of 32 clusters of the visible instances'
    /// LOD windows).
    pub work_items: u32,
    /// Roots the instance cull listed for the instances whose roots are the whole cut (32 to
    /// a work item of the cluster culls).
    pub root_entries: u32,
    /// Work items and roots dropped because the work or root list was full (the frames in
    /// flight when the demand jumps past the lists, until [`MeshletRenderer::begin_frame`]
    /// has grown them).
    pub work_overflow: u32,
    /// Triangles of the drawn dense clusters, the software rasteriser's kind, whether it ran
    /// or not ([`SwRaster::Auto`] decides from them).
    pub dense_triangles: u32,
}

impl FrameStats {
    /// The software rasteriser's counter line for `mode`: what it drew, and how many
    /// triangles sat in dense clusters (what [`SwRaster::Auto`] decides from).
    pub fn software_line(&self, mode: SwRaster) -> String {
        format!(
            "software raster {} (R): {:.0} k clusters, {:.2} M triangles in compute; {:.2} M triangles in dense clusters",
            mode.name(),
            f64::from(self.sw_clusters) / 1e3,
            f64::from(self.sw_triangles) / 1e6,
            f64::from(self.dense_triangles) / 1e6
        )
    }

    /// ", N k dropped (visible list full)" when clusters were dropped, and the same for work
    /// items, else nothing: for the counter lines, so a capped frame never reads as a
    /// complete one.
    pub fn overflow_note(&self) -> String {
        let mut note = String::new();
        if self.visible_overflow != 0 {
            note += &format!(
                ", {:.0} k dropped (visible list full)",
                f64::from(self.visible_overflow) / 1e3
            );
        }
        if self.work_overflow != 0 {
            note += &format!(
                ", {:.0} k work items or roots dropped (work list full)",
                f64::from(self.work_overflow) / 1e3
            );
        }
        note
    }
}

/// The two hierarchical-Z pyramids: pass 1 reads the one the previous frame built while this
/// frame builds the other (see [`PrevCull`]).
fn create_pyramids(device: &Arc<Device>, extent: vk::Extent2D) -> Result<[GraphImage; 2]> {
    Ok([
        create_pyramid(device, extent)?,
        create_pyramid(device, extent)?,
    ])
}

/// A hierarchical-Z pyramid: power-of-two, one storage view per level, sampled in `GENERAL`.
/// Persistent (a frozen culling camera keeps using the last one built).
fn create_pyramid(device: &Arc<Device>, extent: vk::Extent2D) -> Result<GraphImage> {
    let prev_pow2 = |v: u32| 1_u32 << (31 - v.max(1).leading_zeros());
    let (w, h) = (prev_pow2(extent.width), prev_pow2(extent.height));
    let mips = 32 - w.max(h).leading_zeros();
    let hzb = GraphImage::new(
        device,
        ImageDesc {
            width: w,
            height: h,
            format: vk::Format::R32_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: mips,
            name: "hzb",
        },
    )?;
    tracing::info!(hzb_width = w, hzb_height = h, mips, "depth pyramid created");
    Ok(hzb)
}

/// What pass 1 of the next frame tests against: the culling view that drew this frame's
/// first pass and the pyramid built from it.
#[derive(Clone, Copy, Debug)]
struct PrevCull {
    view: Mat4,
    p00: f32,
    p11: f32,
    jitter: Vec2,
    /// Which of the two pyramids.
    pyramid: usize,
}

/// Work items per cluster-cull workgroup (`CULL_ITEMS` in the shader).
const CULL_ITEMS: u32 = 8;
/// Work-list slots each frame slot starts with; the scene's bound, when smaller, is reserved
/// up front (see [`MeshletRenderer::begin_frame`]).
const WORK_INITIAL_CAPACITY: u32 = 1 << 14;
/// The most of a scene's bound reserved up front (8 MiB of work items).
const WORK_RESERVE_MAX: u32 = 1 << 20;
/// Most slots the work list grows to (128 MiB of work items).
const WORK_MAX_CAPACITY: u32 = 1 << 24;

/// One frame slot's work lists for the cluster culls, appended by the instance cull: an
/// instance's group of 32 clusters per work item, and the root list, (instance, local
/// cluster) per root of the instances whose roots are the whole cut, read 32 roots to a work
/// item after the others; `capacity` of each. Then the cluster culls' status words, one per
/// workgroup of `CULL_ITEMS` items and pass, cleared before the instance cull.
struct WorkList {
    work: GraphBuffer,
    roots: GraphBuffer,
    lookback: GraphBuffer,
    capacity: u32,
}

impl WorkList {
    fn new(device: &Arc<Device>, capacity: u32, slot: usize) -> Result<Self> {
        let groups = u64::from(Self::groups_for(capacity));
        let buffer = |size: u64, usage: vk::BufferUsageFlags, name: String| {
            device
                .create_buffer(BufferDesc {
                    size,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER | usage,
                    location: MemoryLocation::GpuOnly,
                    category: MemoryCategory::Work,
                    name: &name,
                })
                .map(GraphBuffer::new)
        };
        let none = vk::BufferUsageFlags::empty();
        Ok(Self {
            work: buffer(
                u64::from(capacity) * 8,
                none,
                format!("cull work list {slot}"),
            )?,
            roots: buffer(
                u64::from(capacity) * 8,
                none,
                format!("cull root list {slot}"),
            )?,
            lookback: buffer(
                2 * groups * 8,
                vk::BufferUsageFlags::TRANSFER_DST,
                format!("cluster cull look-back {slot}"),
            )?,
            capacity,
        })
    }

    /// Cluster-cull workgroups the most work the lists can hold needs: `capacity` work items,
    /// then the root list's `capacity` roots at 32 to an item.
    fn groups_for(capacity: u32) -> u32 {
        (capacity + capacity.div_ceil(TASK_GROUP_SIZE)).div_ceil(CULL_ITEMS)
    }

    fn groups(&self) -> u32 {
        Self::groups_for(self.capacity)
    }
}

/// One frame slot's visible-cluster list: `(instance, meshlet | flags)` per listed cluster,
/// pass 1 then pass 2; the pass being drawn's raster lists (list slots, hardware-drawn from
/// the front, software-rasterised from the back); and on the
/// fallback path one `VkDrawIndexedIndirectCommand` per hardware-drawn cluster of that pass.
struct VisibleList {
    visible: GraphBuffer,
    raster: GraphBuffer,
    draws: Option<GraphBuffer>,
    capacity: u32,
}

impl VisibleList {
    fn new(device: &Arc<Device>, path: GeometryPath, capacity: u32, slot: usize) -> Result<Self> {
        let visible = device.create_buffer(BufferDesc {
            size: u64::from(capacity) * 8,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: &format!("visible clusters {slot}"),
        })?;
        let raster = device.create_buffer(BufferDesc {
            size: u64::from(capacity) * 4,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: &format!("raster lists {slot}"),
        })?;
        let draws = match path {
            GeometryPath::MeshShader => None,
            GeometryPath::IndirectCount => Some(device.create_buffer(BufferDesc {
                size: u64::from(capacity) * u64::from(DRAW_COMMAND_BYTES),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name: &format!("cluster draws {slot}"),
            })?),
        };
        Ok(Self {
            visible: GraphBuffer::new(visible),
            raster: GraphBuffer::new(raster),
            draws: draws.map(GraphBuffer::new),
            capacity,
        })
    }
}

/// Bytes of the software rasteriser's samples for `extent`: 8 per pixel
/// (`shaders/vis64.slang`).
pub fn vis64_bytes(extent: vk::Extent2D) -> u64 {
    u64::from(extent.width.max(1)) * u64::from(extent.height.max(1)) * 8
}

/// The software rasteriser's samples, `depth << 32 | id` per pixel of `extent`, row-major
/// (`shaders/vis64.slang`), created empty (zero). The merge clears what it takes, so the
/// buffer is empty again at the end of every frame.
fn create_vis64(device: &Arc<Device>, extent: vk::Extent2D) -> Result<GraphBuffer> {
    let samples = vec![0_u64; (vis64_bytes(extent) / 8) as usize];
    Ok(GraphBuffer::new(device.create_buffer_with_data(
        &samples,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        MemoryCategory::Targets,
        "software raster samples",
    )?))
}

/// Declares the meshlet passes for one scene.
pub struct MeshletRenderer {
    device: Arc<Device>,
    path: GeometryPath,
    /// The draw pipelines: mesh shaders, or vertex + fragment on the fallback path.
    pipeline_solid: Pipeline,
    pipeline_wire: Pipeline,
    pipeline_hzb: Pipeline,
    pipeline_cull: Pipeline,
    pipeline_cluster_cull: Pipeline,
    /// The same for a streamed scene (the LOD cut follows the resident pages).
    pipeline_cluster_cull_streamed: Pipeline,
    /// The software rasteriser and its merge, on devices with 64-bit buffer atomics.
    pipeline_sw_raster: Option<Pipeline>,
    pipeline_merge: Option<Pipeline>,
    /// The shading passes, one per material class: the standard one over the whole target
    /// (it also lists the tiles of the others), each other over its tiles.
    pipeline_resolve: [Pipeline; MATERIAL_CLASSES],
    /// The mirror rays of the smooth rows (issue #52): devices with ray queries only.
    pipeline_reflections: Option<Pipeline>,
    /// Per class, the tiles that show it, and every class's dispatch ([`create_shading_tiles`]).
    shading_tiles: [GraphBuffer; 2],
    /// The two depth pyramids ([`create_pyramids`]).
    hzb: [GraphImage; 2],
    /// The previous frame's culling view and pyramid, when pass 1 can test against them (not
    /// after a resize, nor with occlusion off). A `Cell`: set while declaring a frame.
    prev: Cell<Option<PrevCull>>,
    /// Per frame slot: the cluster culls' work list.
    work_lists: Vec<WorkList>,
    /// Slots every frame slot's work list grows to.
    work_target: u32,
    /// The software rasteriser's samples (see [`create_vis64`]), with it.
    vis64: Option<GraphBuffer>,
    /// [`SwRaster::Auto`]'s state: the recent frames held enough dense triangles.
    sw_auto_on: bool,
    /// The target size.
    extent: vk::Extent2D,
    frame_buffers: Vec<Buffer>,
    /// The per-frame counters (`FrameStats`): cleared and counted on the GPU in device-local
    /// memory, then copied into this frame slot's host-cached readback.
    stats: GraphBuffer,
    stats_readback: Vec<GraphBuffer>,
    /// Per frame slot: the visible-cluster list and, on the fallback path, its draw commands.
    lists: Vec<VisibleList>,
    /// Slots every frame slot's list grows to (see [`MeshletRenderer::begin_frame`]).
    visible_target: u32,
    /// [`VISIBLE_MAX_CAPACITY`], or less when the fallback's `maxDrawIndirectCount` or the mesh
    /// path's `maxMeshWorkGroupTotalCount` is lower.
    visible_max: u32,
    /// Direction *to* the sun (world space), used by the visibility resolve.
    pub sun_dir: Vec3,
    /// Illuminance of the sun at the scene, in lux (the rocks return albedo × E / π).
    pub sun_illuminance: f32,
    /// The sunlight's colour at the scene: (1, 0.96, 0.9) by default (the ballad's, in space);
    /// on a planet, the sun through the air.
    pub sun_color: Vec3,
    /// The sun disc's angular radius, radians: the shadow rays aim within it and TAA averages
    /// them into penumbrae (issue #54). 0 (the default) keeps the shadows hard.
    pub sun_angular_radius: f32,
    /// The frame index the noise of soft shadows uses: TAA's frame modulo its jitter's period,
    /// so the pattern repeats with the jitter (set by the demo every frame).
    pub noise_frame: u32,
}

/// What to draw this frame.
#[derive(Clone, Copy)]
pub struct DrawParams<'a> {
    /// The scene tables.
    pub scene: &'a MeshletScene,
    /// The drawing camera.
    pub view_proj: Mat4,
    /// The culling camera (equal to the drawing one unless frozen).
    pub cull: CullCamera,
    /// Where the drawn image sits relative to the culling camera's, in uv units (x right,
    /// y down): the temporal anti-aliasing jitter divided by the extent, or zero. The
    /// occlusion test reads the depth pyramid there.
    pub draw_jitter: Vec2,
    /// Projected LOD error a drawn cluster may have, in pixels (when `CullFlags::LOD` is set).
    pub lod_threshold_px: f32,
    /// Culling flags.
    pub flags: CullFlags,
    /// Target size.
    pub extent: vk::Extent2D,
    /// Wireframe.
    pub wireframe: bool,
    /// Pre-exposure the resolve multiplies the shaded luminance by (see `crate::exposure`).
    pub exposure: f32,
    /// When the software rasteriser draws the first pass's dense clusters.
    pub sw_raster: SwRaster,
    /// Dense clusters: under [`SW_RASTER_MAX_PX`] across, with fewer pixels of bounding
    /// rectangle than this per triangle.
    pub sw_raster_area: f32,
}

/// The graph handles the cull and draw passes touch.
#[derive(Clone, Copy)]
struct MeshPassIo {
    /// The visibility buffer (colour attachment of the draws).
    visibility: ImageHandle,
    depth: ImageHandle,
    /// The pyramid pass 2 tests (built after pass 1, or the previous one when frozen).
    hzb: ImageHandle,
    /// The previous frame's pyramid, which pass 1 tests (none after a resize or with
    /// occlusion off).
    hzb_prev: Option<ImageHandle>,
    work: forge_gpu::BufferHandle,
    /// The root list, read by the cluster culls after the work items.
    roots: forge_gpu::BufferHandle,
    /// The cluster culls' status words.
    cluster_lookback: forge_gpu::BufferHandle,
    /// The resident cluster pages and the page table.
    pool: forge_gpu::BufferHandle,
    page_table: forge_gpu::BufferHandle,
    /// A streamed scene's page needs and this slot's readback of them.
    need: Option<forge_gpu::BufferHandle>,
    need_readback: Option<forge_gpu::BufferHandle>,
    indirect: forge_gpu::BufferHandle,
    /// Each pass's draw grid and cluster count, and the cluster culls' tickets.
    clusters: forge_gpu::BufferHandle,
    /// The ordered appends' status words.
    lookback: forge_gpu::BufferHandle,
    /// The fallback's indexed draws.
    draws: Option<forge_gpu::BufferHandle>,
    /// The visible-cluster list of this frame slot.
    visible: forge_gpu::BufferHandle,
    /// Its raster lists.
    raster: forge_gpu::BufferHandle,
    /// The software rasteriser's samples, when it runs this frame.
    vis64: Option<forge_gpu::BufferHandle>,
    stats: forge_gpu::BufferHandle,
    /// This frame slot's copy of the counters for the host.
    stats_readback: forge_gpu::BufferHandle,
}

impl MeshletRenderer {
    /// Compiles the pipelines and creates the depth resources for `extent`. The path follows
    /// the device: mesh shaders when it has them, the indirect-count fallback otherwise
    /// (which needs 8-bit index buffers). The software rasteriser needs 64-bit atomics on
    /// storage buffers (and stores from fragment shaders, for its merge); without them every
    /// cluster is drawn in hardware.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let path = if device.features().mesh_shader {
            GeometryPath::MeshShader
        } else {
            GeometryPath::IndirectCount
        };
        if path == GeometryPath::IndirectCount && !device.features().index_type_uint8 {
            return Err(forge_gpu::GpuError::Unsupported(format!(
                "{} has neither mesh shaders nor 8-bit index buffers \
                 (VK_KHR_index_type_uint8) for the meshlet fallback",
                device.name()
            )));
        }
        let (geometry_entry, geometry_stage, fragment_entry) = match path {
            GeometryPath::MeshShader => ("mesh_main", ShaderStage::Mesh, "frag_main"),
            GeometryPath::IndirectCount => {
                ("vertex_main", ShaderStage::Vertex, "frag_fallback_main")
            }
        };
        let geometry = device.create_shader_module(
            &shaders.compile("meshlet.slang", geometry_entry, geometry_stage)?,
            geometry_entry,
        )?;
        let frag = device.create_shader_module(
            &shaders.compile("meshlet.slang", fragment_entry, ShaderStage::Fragment)?,
            fragment_entry,
        )?;
        let hzb = device.create_shader_module(
            &shaders.compile("hzb.slang", "hzb_main", ShaderStage::Compute)?,
            "hzb",
        )?;
        let cull = device.create_shader_module(
            &shaders.compile("meshlet.slang", "cull_main", ShaderStage::Compute)?,
            "instance cull",
        )?;
        let cluster_cull = device.create_shader_module(
            &shaders.compile("meshlet.slang", "cluster_cull_main", ShaderStage::Compute)?,
            "cluster cull",
        )?;
        let cluster_cull_streamed = device.create_shader_module(
            &shaders.compile(
                "meshlet.slang",
                "cluster_cull_streamed_main",
                ShaderStage::Compute,
            )?,
            "cluster cull (streamed)",
        )?;
        let make_pipeline = |wireframe: bool| {
            let name = if wireframe {
                "meshlets wire"
            } else {
                "meshlets"
            };
            let color_formats = &[vk::Format::R32_UINT];
            let push_constant_bytes = std::mem::size_of::<Push>() as u32;
            match path {
                GeometryPath::MeshShader => device.create_mesh_pipeline(&MeshPipelineDesc {
                    task: None,
                    mesh: (geometry, geometry_entry),
                    fragment: (frag, fragment_entry),
                    color_formats,
                    depth_format: Some(vk::Format::D32_SFLOAT),
                    push_constant_bytes,
                    cull_mode: vk::CullModeFlags::BACK,
                    wireframe,
                    depth_test: true,
                    name,
                }),
                GeometryPath::IndirectCount => device.create_vertex_pipeline(&VertexPipelineDesc {
                    vertex: (geometry, geometry_entry),
                    fragment: (frag, fragment_entry),
                    color_formats,
                    depth_format: Some(vk::Format::D32_SFLOAT),
                    push_constant_bytes,
                    cull_mode: vk::CullModeFlags::BACK,
                    wireframe,
                    depth_test: true,
                    name,
                }),
            }
        };
        let pipeline_solid = make_pipeline(false)?;
        let pipeline_wire = make_pipeline(true)?;
        let pipeline_hzb = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (hzb, "hzb_main"),
            push_constant_bytes: std::mem::size_of::<HzbPush>() as u32,
            name: "hzb build",
        })?;
        let pipeline_cull = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (cull, "cull_main"),
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            name: "instance cull + LOD window",
        })?;
        let pipeline_cluster_cull = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (cluster_cull, "cluster_cull_main"),
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            name: "cluster cull",
        })?;
        let pipeline_cluster_cull_streamed =
            device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (cluster_cull_streamed, "cluster_cull_streamed_main"),
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                name: "cluster cull (streamed)",
            })?;
        let (pipeline_sw_raster, pipeline_merge) = if device.features().int64_atomics {
            let (raster, merge) = Self::software_pipelines(device, shaders)?;
            (Some(raster), Some(merge))
        } else {
            tracing::info!("no 64-bit buffer atomics: software rasteriser off");
            (None, None)
        };
        // The shading passes share their push constants.
        let shading_pipeline = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("meshlet.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<ResolvePush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        // With ray queries, the variants that trace the sun's shadows when a scene has a TLAS.
        let rt = device.features().ray_query;
        let entry = |plain: &'static str, traced: &'static str| if rt { traced } else { plain };
        let pipeline_resolve = [
            shading_pipeline(
                entry("resolve_standard_main", "resolve_standard_rt_main"),
                "shading standard",
            )?,
            shading_pipeline(
                entry("resolve_ice_main", "resolve_ice_rt_main"),
                "shading ice",
            )?,
            shading_pipeline(
                entry("resolve_layered_main", "resolve_layered_rt_main"),
                "shading layered",
            )?,
        ];
        let pipeline_reflections = if rt {
            Some(shading_pipeline("reflections_main", "shading reflections")?)
        } else {
            None
        };
        for module in [
            geometry,
            frag,
            hzb,
            cull,
            cluster_cull,
            cluster_cull_streamed,
        ] {
            device.destroy_shader_module(module);
        }
        let hzb = create_pyramids(device, extent)?;
        let work_lists = (0..FRAMES_IN_FLIGHT)
            .map(|i| WorkList::new(device, WORK_INITIAL_CAPACITY, i))
            .collect::<Result<Vec<_>>>()?;
        let vis64 = pipeline_sw_raster
            .is_some()
            .then(|| create_vis64(device, extent))
            .transpose()?;
        let frame_buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: FRAME_BLOCK_STRIDE * 2,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("meshlet frame {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // The culls count with atomics in device-local memory; the host reads a copy in cached
        // memory (reading host-visible video memory through Resizable BAR is slow for the CPU).
        let stats = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: STATS_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "meshlet stats",
        })?);
        let stats_readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let b = device.create_buffer(BufferDesc {
                    size: STATS_BYTES,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("meshlet stats readback {i}"),
                })?;
                b.write(0, &[0_u32; STAT_COUNT]);
                Ok(GraphBuffer::new(b))
            })
            .collect::<Result<Vec<_>>>()?;
        // One mesh workgroup per hardware-drawn cluster: the list stays within what one draw
        // may launch (issue #67), as the fallback's within its indirect draw count.
        let visible_max = match path {
            GeometryPath::MeshShader => device.mesh_limits().map_or(VISIBLE_MAX_CAPACITY, |l| {
                VISIBLE_MAX_CAPACITY.min(l.max_total_work_groups)
            }),
            GeometryPath::IndirectCount => {
                VISIBLE_MAX_CAPACITY.min(device.limits().max_draw_indirect_count)
            }
        };
        let visible_target = VISIBLE_INITIAL_CAPACITY.min(visible_max);
        let lists = (0..FRAMES_IN_FLIGHT)
            .map(|i| VisibleList::new(device, path, visible_target, i))
            .collect::<Result<Vec<_>>>()?;
        tracing::info!(
            path = path.name(),
            visible_capacity = visible_target,
            "meshlet renderer ready"
        );
        Ok(Self {
            device: Arc::clone(device),
            path,
            pipeline_solid,
            pipeline_wire,
            pipeline_hzb,
            pipeline_cull,
            pipeline_cluster_cull,
            pipeline_cluster_cull_streamed,
            pipeline_sw_raster,
            pipeline_merge,
            pipeline_resolve,
            pipeline_reflections,
            shading_tiles: create_shading_tiles(device, extent)?,
            hzb,
            prev: Cell::new(None),
            work_lists,
            work_target: WORK_INITIAL_CAPACITY,
            vis64,
            sw_auto_on: false,
            extent,
            frame_buffers,
            stats,
            stats_readback,
            lists,
            visible_target,
            visible_max,
            sun_dir: Vec3::new(0.4, 1.0, 0.3).normalize(),
            sun_illuminance: crate::starfield::SUN_ILLUMINANCE_1AU,
            sun_color: Vec3::new(1.0, 0.96, 0.9),
            sun_angular_radius: 0.0,
            noise_frame: 0,
        })
    }

    /// The software rasteriser (compute) and its merge (a full-screen triangle writing the
    /// visibility buffer and the depth).
    fn software_pipelines(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
    ) -> Result<(Pipeline, Pipeline)> {
        let raster = device.create_shader_module(
            &shaders.compile("meshlet.slang", "sw_raster_main", ShaderStage::Compute)?,
            "software raster",
        )?;
        let merge_vertex = device.create_shader_module(
            &shaders.compile("meshlet.slang", "merge_vert_main", ShaderStage::Vertex)?,
            "software raster merge",
        )?;
        let merge_fragment = device.create_shader_module(
            &shaders.compile("meshlet.slang", "merge_main", ShaderStage::Fragment)?,
            "software raster merge",
        )?;
        let pipelines = device
            .create_compute_pipeline(&ComputePipelineDesc {
                shader: (raster, "sw_raster_main"),
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                name: "software raster",
            })
            .and_then(|raster| {
                let merge = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
                    vertex: (merge_vertex, "merge_vert_main"),
                    fragment: (merge_fragment, "merge_main"),
                    color_formats: &[vk::Format::R32_UINT],
                    push_constant_bytes: std::mem::size_of::<Push>() as u32,
                    alpha_blend: false,
                    depth_test: Some(vk::Format::D32_SFLOAT),
                    depth_write: true,
                    name: "software raster merge",
                })?;
                Ok((raster, merge))
            });
        for module in [raster, merge_vertex, merge_fragment] {
            device.destroy_shader_module(module);
        }
        pipelines
    }

    /// The push constants of the meshlet pipelines for the frame block at `frame_address`.
    fn push(&self, frame_address: u64) -> Push {
        Push {
            frame: frame_address,
            vis64: self.vis64.as_ref().map_or(0, |b| b.address()),
            target_width: self.extent.width,
            depth_image: 0,
            vis_image: 0,
            pad: 0,
        }
    }

    /// Whether this frame rasterises its dense clusters in compute: asked for (or, in
    /// [`SwRaster::Auto`], worth it lately), available on the device, and neither a frozen
    /// culling camera (the routing sizes clusters with it, and it may be far from the drawing
    /// camera) nor wireframe.
    pub fn software_raster(&self, params: &DrawParams<'_>) -> bool {
        let wanted = match params.sw_raster {
            SwRaster::Off => false,
            SwRaster::Auto => self.sw_auto_on,
            SwRaster::On => true,
        };
        wanted
            && self.pipeline_sw_raster.is_some()
            && !params.flags.has(CullFlags::FREEZE)
            && !params.wireframe
    }

    /// How the visible clusters are drawn on this device.
    pub fn path(&self) -> GeometryPath {
        self.path
    }

    /// Recreates the depth pyramids and the software rasteriser's samples. The device must be
    /// idle. The next frame's pass 1 has no previous pyramid and draws nothing; pass 2 draws
    /// everything visible.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.hzb = create_pyramids(&self.device, extent)?;
        self.prev.set(None);
        if self.vis64.is_some() {
            self.vis64 = Some(create_vis64(&self.device, extent)?);
        }
        self.shading_tiles = create_shading_tiles(&self.device, extent)?;
        self.extent = extent;
        Ok(())
    }

    /// Size of the depth pyramids' level 0.
    pub fn hzb_extent(&self) -> vk::Extent2D {
        self.hzb[0].extent()
    }

    /// The device address of frame slot `slot`'s `Frame` block (`meshlet.slang`), written by
    /// [`MeshletRenderer::draw`]: the scene's tables, its TLAS and the sun, for passes that
    /// trace the scene outside the renderer ([`crate::Probes::update`]).
    pub fn frame_address(&self, slot: FrameSlot) -> u64 {
        self.frame_buffers[slot.index].address()
    }

    /// Sizes the visible-cluster list for `clusters` before the first frame that needs them
    /// (at most [`VISIBLE_MAX_CAPACITY`]), for a caller that knows its demand: with LOD off a
    /// frame lists at most [`MeshletScene::finest_clusters`]. Without it the list only grows
    /// after a frame has dropped clusters, which leaves holes in the frames in flight until
    /// then and, through the automatic exposure, a trace in the images that follow.
    pub fn reserve_visible(&mut self, clusters: u64) {
        let target = clusters.min(u64::from(self.visible_max)) as u32;
        if target > self.visible_target {
            tracing::info!(
                from = self.visible_target,
                to = target,
                list_mib = %format_args!("{:.1}", f64::from(target) * 8.0 / f64::from(1 << 20)),
                "visible-cluster list reserved"
            );
            self.visible_target = target;
        }
    }

    /// Call once per frame, after waiting for `slot` and before [`MeshletRenderer::draw`]:
    /// reads and clears the statistics of the frame that last used `slot` (`None` for the
    /// first frames in flight) and sizes the slot's visible-cluster list. A frame that
    /// dropped clusters raises the size every slot grows to; this slot grows now, the others
    /// when their turn comes, so a jump in demand leaves holes in the frames in flight
    /// until then (two or three).
    pub fn begin_frame(
        &mut self,
        slot: FrameSlot,
        scene: &mut MeshletScene,
    ) -> Result<Option<FrameStats>> {
        if let Some(streamer) = scene.streamer.as_mut() {
            streamer.begin_frame(slot.index, slot.frame_number);
        }
        // The work list: the scene's bound up front when small (no frame then ever drops a
        // work item), else grown from the demand the instance cull counts.
        let reserve = scene.work_bound.min(u64::from(WORK_RESERVE_MAX)) as u32;
        if reserve > self.work_target {
            tracing::info!(
                from = self.work_target,
                to = reserve,
                "cull work list reserved"
            );
            self.work_target = reserve;
        }
        let mut raw = [0_u32; STAT_COUNT];
        // Declared `HostRead` by the frame that wrote it (its commands have completed); cleared
        // so that a frame which draws nothing reads as zero.
        self.stats_readback[slot.index].read(0, &mut raw);
        self.stats_readback[slot.index].write(0, &[0_u32; STAT_COUNT]);
        let stats = (slot.frame_number >= FRAMES_IN_FLIGHT as u64).then_some(FrameStats {
            meshlets_pass1: raw[0],
            meshlets_pass2: raw[1],
            triangles: raw[2],
            occluded: raw[3],
            instances_visible: raw[4],
            lod_level_sum: raw[5],
            visible_overflow: raw[6],
            sw_clusters: raw[7],
            sw_triangles: raw[8],
            dense_triangles: raw[9],
            work_items: raw[10],
            work_overflow: raw[11],
            root_entries: raw[12],
        });
        if let Some(s) = stats {
            // One capacity for both lists: the larger demand.
            let wanted = s.work_items.max(s.root_entries);
            let target = grown_capacity(self.work_target, u64::from(wanted), WORK_MAX_CAPACITY);
            if target != self.work_target {
                tracing::info!(
                    wanted,
                    from = self.work_target,
                    to = target,
                    "cull work list grown"
                );
                self.work_target = target;
            }
        }
        let work = &mut self.work_lists[slot.index];
        if work.capacity < self.work_target {
            *work = WorkList::new(&self.device, self.work_target, slot.index)?;
        }
        if let Some(s) = stats {
            let on = software_worth_it(self.sw_auto_on, s.dense_triangles);
            if on != self.sw_auto_on {
                tracing::debug!(
                    dense_triangles = s.dense_triangles,
                    on,
                    "software raster (auto)"
                );
                self.sw_auto_on = on;
            }
        }
        if let Some(s) = stats {
            let wanted = u64::from(s.meshlets_pass1)
                + u64::from(s.meshlets_pass2)
                + u64::from(s.visible_overflow);
            let target = grown_capacity(self.visible_target, wanted, self.visible_max);
            if target != self.visible_target {
                tracing::info!(
                    wanted,
                    from = self.visible_target,
                    to = target,
                    list_mib = %format_args!("{:.1}", f64::from(target) * 8.0 / f64::from(1 << 20)),
                    "visible-cluster list grown"
                );
                self.visible_target = target;
            }
        }
        let list = &mut self.lists[slot.index];
        if list.capacity < self.visible_target {
            // The frame that last used this slot has completed (the caller waited for the
            // slot), so its list can go now.
            *list = VisibleList::new(&self.device, self.path, self.visible_target, slot.index)?;
        }
        Ok(stats)
    }

    /// The frame block of `pass`: `pyramid` is the pyramid this frame's pass 2 tests (built
    /// after pass 1, or the previous one when culling is frozen), `prev` what pass 1 tests.
    fn frame_block(
        &self,
        slot: FrameSlot,
        params: &DrawParams<'_>,
        pass: u32,
        pyramid: usize,
        prev: Option<PrevCull>,
    ) -> GpuFrame {
        let scene = params.scene;
        let hzb = self.hzb[pyramid].extent();
        let work = &self.work_lists[slot.index];
        // Only the first pass rasterises in software: pass 2 draws the few clusters that
        // became visible, and a second raster and merge would cost more than they save.
        let mut flags = params.flags;
        flags.0 &= !FLAG_SW_RASTER;
        if pass != PASS_REMAINDER && self.software_raster(params) {
            flags.0 |= FLAG_SW_RASTER;
        }
        flags.0 &= !FLAG_PREV_PYRAMID;
        if prev.is_some() {
            flags.0 |= FLAG_PREV_PYRAMID;
        }
        flags.0 &= !FLAG_STREAMING;
        if scene.streamer.is_some() {
            flags.0 |= FLAG_STREAMING;
        }
        let prev = prev.unwrap_or(PrevCull {
            view: Mat4::IDENTITY,
            p00: 1.0,
            p11: 1.0,
            jitter: Vec2::ZERO,
            pyramid,
        });
        GpuFrame {
            view_proj: params.view_proj.to_cols_array(),
            cull_view_proj: params.cull.view_proj.to_cols_array(),
            cull_view: params.cull.view.to_cols_array(),
            prev_cull_view: prev.view.to_cols_array(),
            planes: frustum_planes(params.cull.view_proj),
            camera_pos: params.cull.position.to_array(),
            instance_count: scene.instance_count,
            work_capacity: work.capacity,
            flags: flags.0,
            pass,
            hzb_image: self.hzb[pyramid].sampled().0,
            hzb_size: [hzb.width, hzb.height],
            p00: params.cull.p00,
            p11: params.cull.p11,
            znear: params.cull.near,
            sun_dir: self.sun_dir.to_array(),
            draw_jitter: params.draw_jitter.to_array(),
            lod_threshold: params.lod_threshold_px,
            viewport_height: params.extent.height as f32,
            pool: scene.pool.address(),
            meshlets: scene.meshlets.address(),
            page_table: scene.page_table.address(),
            meshes: scene.meshes.address(),
            instances: scene.instances.address(),
            stats: self.stats.address(),
            cluster_lookback: work.lookback.address(),
            cluster_groups: work.groups(),
            prev_hzb_image: self.hzb[prev.pyramid].sampled().0,
            work: work.work.address(),
            indirect: scene.indirect[slot.index].address(),
            visible: self.lists[slot.index].visible.address(),
            visible_capacity: self.lists[slot.index].capacity,
            exposure: params.exposure,
            sun_illuminance: self.sun_illuminance,
            sun_angular_radius: self.sun_angular_radius,
            clusters: scene.clusters[slot.index].address(),
            draws: self.lists[slot.index]
                .draws
                .as_ref()
                .map_or(0, |b| b.address()),
            lookback: scene.lookback[slot.index].address(),
            raster: self.lists[slot.index].raster.address(),
            target_width: self.extent.width,
            target_height: self.extent.height,
            sw_raster_area: params.sw_raster_area.max(0.0),
            noise_frame: self.noise_frame,
            prev_draw_jitter: prev.jitter.to_array(),
            prev_p00: prev.p00,
            prev_p11: prev.p11,
            roots: work.roots.address(),
            page_need: scene
                .streamer
                .as_ref()
                .map_or(0, |s| s.need_buffer.address()),
            materials: scene.materials.address(),
            sun_color: self.sun_color.extend(0.0).to_array(),
            tlas: scene.rays.as_ref().map_or(0, SceneRays::tlas_address),
            rt_scene: scene.rays.as_ref().map_or(0, SceneRays::hit_address),
        }
    }

    /// Declares the passes of this frame's draw: the instance cull, then per mesh pass a
    /// cluster cull (compute, filling the visible-cluster list), the hardware draw of what it
    /// kept and, with the software rasteriser, its raster and merge, with the depth pyramid
    /// built between the two passes when occlusion is on. Everything reads and writes through
    /// declared graph accesses. The draws write the visibility buffer and the depth (both
    /// transients); [`MeshletRenderer::resolve`] shades the result. `extent` must be the
    /// renderer's (see [`MeshletRenderer::resize`]).
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        params: DrawParams<'f>,
    ) -> Result<DrawTargets> {
        let occlusion = params.flags.has(CullFlags::OCCLUSION);
        let frozen = params.flags.has(CullFlags::FREEZE);
        let first_pass = if occlusion {
            PASS_PREVIOUSLY_VISIBLE
        } else {
            PASS_SINGLE
        };
        // The pyramids: pass 1 tests the previous frame's, this frame builds the other one
        // after pass 1 for pass 2 (and the next frame's pass 1). A frozen culling camera keeps
        // the last one built, unless there is none yet.
        let prev = self.prev.get().filter(|_| occlusion);
        let build = occlusion && (!frozen || prev.is_none());
        let pyramid = match prev {
            Some(p) if build => 1 - p.pyramid,
            Some(p) => p.pyramid,
            None => 0,
        };
        let block1 = self.frame_block(slot, &params, first_pass, pyramid, prev);
        let block2 = self.frame_block(slot, &params, PASS_REMAINDER, pyramid, prev);
        self.frame_buffers[slot.index].write(0, &[block1]);
        self.frame_buffers[slot.index].write(FRAME_BLOCK_STRIDE, &[block2]);
        // The grids start empty every frame, (x, y, z, count) for the cluster cull and for the
        // draw of each pass, and so do the culls' tickets; the last workgroup of each cull
        // writes the grid it leads to.
        let scene = params.scene;
        scene.indirect[slot.index].write(0, &[0_u32, 0, 1, 0, 0, 0, 0, 0]);
        scene.clusters[slot.index].write(0, &CLUSTER_ARGS_START);

        let extent = params.extent;
        debug_assert_eq!(
            extent, self.extent,
            "resize the meshlet renderer with the target"
        );
        let depth = graph.transient(TransientDesc {
            name: "depth",
            width: extent.width,
            height: extent.height,
            format: vk::Format::D32_SFLOAT,
            usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::DEPTH,
            mip_levels: 1,
        });
        let visibility = graph.transient(TransientDesc {
            name: "visibility",
            width: extent.width,
            height: extent.height,
            format: vk::Format::R32_UINT,
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let software = self.software_raster(&params);
        let hzb = graph.import(&self.hzb[pyramid]);
        let hzb_prev = prev.map(|p| {
            if p.pyramid == pyramid {
                hzb
            } else {
                graph.import(&self.hzb[p.pyramid])
            }
        });
        let io = MeshPassIo {
            visibility,
            depth,
            hzb,
            hzb_prev,
            work: graph.import_buffer(&self.work_lists[slot.index].work),
            roots: graph.import_buffer(&self.work_lists[slot.index].roots),
            cluster_lookback: graph.import_buffer(&self.work_lists[slot.index].lookback),
            indirect: graph.import_buffer(&scene.indirect[slot.index]),
            clusters: graph.import_buffer(&scene.clusters[slot.index]),
            lookback: graph.import_buffer(&scene.lookback[slot.index]),
            draws: self.lists[slot.index]
                .draws
                .as_ref()
                .map(|b| graph.import_buffer(b)),
            visible: graph.import_buffer(&self.lists[slot.index].visible),
            raster: graph.import_buffer(&self.lists[slot.index].raster),
            vis64: self
                .vis64
                .as_ref()
                .filter(|_| software)
                .map(|b| graph.import_buffer(b)),
            stats: graph.import_buffer(&self.stats),
            stats_readback: graph.import_buffer(&self.stats_readback[slot.index]),
            pool: graph.import_buffer(&scene.pool),
            page_table: graph.import_buffer(&scene.page_table),
            need: scene
                .streamer
                .as_ref()
                .map(|s| graph.import_buffer(&s.need_buffer)),
            need_readback: scene
                .streamer
                .as_ref()
                .map(|s| graph.import_buffer(&s.readback[slot.index])),
        };
        let frame_address = self.frame_buffers[slot.index].address();

        // Streaming: this frame's pages and page-table entries, before anything reads them.
        if let Some(streamer) = scene.streamer.as_ref() {
            let (staging, plan) = (streamer.staging(slot.index), &streamer.plans[slot.index]);
            let (pool, table): (&'f Buffer, &'f Buffer) = (&scene.pool, &scene.page_table);
            graph
                .pass("streaming/upload")
                .buffer(io.pool, BufferAccess::TransferDst)
                .buffer(io.page_table, BufferAccess::TransferDst)
                .run(move |_, commands| {
                    commands.copy_buffer_regions(staging, pool, &plan.pages);
                    commands.copy_buffer_regions(staging, table, &plan.table);
                    Ok(())
                });
        }

        // Instance culling and LOD level windows: the work and root lists and the cluster
        // cull's grid. Every cull's status words start cleared.
        let cull_pipeline = &self.pipeline_cull;
        let instance_groups = scene.instance_count.div_ceil(64).max(1);
        let lookback: &'f GraphBuffer = &scene.lookback[slot.index];
        let work_list = &self.work_lists[slot.index];
        let cluster_lookback: &'f GraphBuffer = &work_list.lookback;
        let cluster_lookback_bytes = 2 * u64::from(work_list.groups()) * 8;
        let stats: &'f GraphBuffer = &self.stats;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let need: Option<&'f GraphBuffer> = scene.streamer.as_ref().map(|s| &s.need_buffer);
        let need_bytes = u64::from(scene.page_count.max(1)) * 4;
        let mut clears = graph
            .pass("geometry/instance cull")
            .buffer(io.lookback, BufferAccess::TransferDst)
            .buffer(io.cluster_lookback, BufferAccess::TransferDst)
            .buffer(io.stats, BufferAccess::TransferDst);
        if let Some(handle) = io.need {
            clears = clears.buffer(handle, BufferAccess::TransferDst);
        }
        clears.run(move |_, commands| {
            commands.fill_buffer(lookback, 0, u64::from(instance_groups) * 8, 0);
            commands.fill_buffer(cluster_lookback, 0, cluster_lookback_bytes, 0);
            commands.fill_buffer(stats, 0, STATS_BYTES, 0);
            if let Some(need) = need {
                commands.fill_buffer(need, 0, need_bytes, 0);
            }
            Ok(())
        });
        graph
            .pass("geometry/instance cull")
            .buffer(io.work, BufferAccess::ShaderWrite(compute))
            .buffer(io.roots, BufferAccess::ShaderWrite(compute))
            .buffer(io.indirect, BufferAccess::ShaderReadWrite(compute))
            .buffer(io.lookback, BufferAccess::ShaderReadWrite(compute))
            .buffer(io.stats, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(cull_pipeline);
                commands.push_constants(cull_pipeline, &self.push(frame_address));
                commands.dispatch(instance_groups, 1, 1);
                Ok(())
            });

        let (cull_label, draw_label, raster_label) = if occlusion {
            (
                "geometry/cluster cull 1 (visible last frame)",
                "geometry/meshlet pass 1 (visible last frame)",
                "geometry/software raster 1 (visible last frame)",
            )
        } else {
            (
                "geometry/cluster cull (single pass)",
                "geometry/meshlets (single pass)",
                "geometry/software raster (single pass)",
            )
        };
        let first = MeshPass {
            io,
            frame_address,
            args_offset: 0,
            second: false,
        };
        self.cull_pass(graph, cull_label, first, &params, slot);
        if !occlusion {
            self.stats_readback_passes(graph, cull_label, io, slot);
            self.need_readback_passes(graph, cull_label, io, scene, slot);
        }
        self.draw_pass(graph, draw_label, first, &params, slot);
        self.software_passes(graph, raster_label, first, &params, slot);

        if occlusion {
            if build {
                self.pyramid_passes(graph, io, &self.hzb[pyramid]);
            }
            let second = MeshPass {
                io,
                frame_address: frame_address + FRAME_BLOCK_STRIDE,
                args_offset: CLUSTER_ARGS_PASS_BYTES,
                second: true,
            };
            self.cull_pass(
                graph,
                "geometry/cluster cull 2 (occlusion)",
                second,
                &params,
                slot,
            );
            self.stats_readback_passes(graph, "geometry/cluster cull 2 (occlusion)", io, slot);
            self.need_readback_passes(
                graph,
                "geometry/cluster cull 2 (occlusion)",
                io,
                scene,
                slot,
            );
            self.draw_pass(
                graph,
                "geometry/meshlet pass 2 (newly visible)",
                second,
                &params,
                slot,
            );
        }
        self.prev.set(match (occlusion, build) {
            (false, _) => None,
            (true, true) => Some(PrevCull {
                view: params.cull.view,
                p00: params.cull.p00,
                p11: params.cull.p11,
                jitter: params.draw_jitter,
                pyramid,
            }),
            (true, false) => prev,
        });
        Ok(DrawTargets {
            depth,
            visibility,
            visible_list: io.visible,
            pages: io.pool,
            page_table: io.page_table,
        })
    }

    /// Once the last cull has written the page needs: copies them into this slot's readback
    /// for the streamer (`crate::streaming`), under the last cull's label.
    fn need_readback_passes<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        label: &'static str,
        io: MeshPassIo,
        scene: &'f MeshletScene,
        slot: FrameSlot,
    ) {
        let (Some(streamer), Some(need), Some(readback)) =
            (scene.streamer.as_ref(), io.need, io.need_readback)
        else {
            return;
        };
        let (src, dst) = (&streamer.need_buffer, &streamer.readback[slot.index]);
        let bytes = u64::from(scene.page_count.max(1)) * 4;
        graph
            .pass(label)
            .buffer(need, BufferAccess::TransferSrc)
            .buffer(readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(src, dst, bytes);
                Ok(())
            });
        graph
            .pass(label)
            .buffer(readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// Declares the passes that shade the visibility buffer once per pixel into `color` (a
    /// storage-capable colour image of `extent`, at most the renderer's size), one per
    /// material class. The standard pass covers the target: it shades the standard pixels,
    /// writes `background` into empty ones (or leaves them for a later pass, the sky, when
    /// `None`) and lists the 8×8 tiles that show each other class; each other class then
    /// shades its pixels in its tiles. A pixel's triangle is fetched, its attributes
    /// reconstructed at the pixel centre from analytic barycentrics, and lit with the
    /// instance's row of the material table. Under `ambient.sky` (issue #47), a surface takes
    /// the sky's irradiance for its normal besides the sun, scaled by `ambient.occlusion`
    /// (issue #48); without a sky, space's constant fill.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        targets: DrawTargets,
        color: ImageHandle,
        extent: vk::Extent2D,
        background: Option<[f32; 4]>,
        ambient: AmbientLight,
    ) {
        debug_assert!(tile_count(extent) <= tile_count(self.extent));
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let [tiles_buffer, args_buffer]: &'f [GraphBuffer; 2] = &self.shading_tiles;
        let tiles = graph.import_buffer(tiles_buffer);
        let args = graph.import_buffer(args_buffer);
        // The mirror rays' requests (issue #52): the resolve writes them for the smooth rows, the
        // reflection pass reads them. Only under a sky, on devices with ray queries.
        let reflections = self
            .pipeline_reflections
            .as_ref()
            .filter(|_| ambient.sky.is_some());
        let request = reflections.map(|_| {
            graph.transient(TransientDesc {
                name: "mirror ray requests",
                width: extent.width,
                height: extent.height,
                format: vk::Format::R16G16B16A16_SFLOAT,
                usage: vk::ImageUsageFlags::STORAGE,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
            })
        });
        let frame_address = self.frame_buffers[slot.index].address();
        let push = move |resources: &forge_gpu::Resources<'_>| ResolvePush {
            frame: frame_address,
            tiles: tiles_buffer.address(),
            class_args: args_buffer.address(),
            vis_image: resources.sampled(targets.visibility).0,
            color_image: resources.storage(color, 0).0,
            width: extent.width,
            height: extent.height,
            use_background: u32::from(background.is_some()),
            tile_capacity: tile_count(self.extent),
            background: background.unwrap_or([0.0; 4]),
            sky: ambient.sky.map_or(0, |s| s.address),
            ao_image: ambient
                .occlusion
                .map_or(u32::MAX, |ao| resources.sampled(ao).0),
            request_image: request.map_or(u32::MAX, |r| resources.storage(r, 0).0),
            probes: ambient.sky.and(ambient.probes).map_or(0, |p| p.address),
        };

        // Every class starts with no tiles and a dispatch TILE_GROUPS_X wide, 0 rows deep.
        graph
            .pass("shading/standard")
            .buffer(args, BufferAccess::TransferDst)
            .run(move |_, commands| {
                for class in 0..TILE_LISTS as u64 {
                    commands.fill_buffer(args_buffer, class * 16, 4, TILE_GROUPS_X);
                    commands.fill_buffer(args_buffer, class * 16 + 4, 4, 0);
                    commands.fill_buffer(args_buffer, class * 16 + 8, 4, 1);
                    commands.fill_buffer(args_buffer, class * 16 + 12, 4, 0);
                }
                Ok(())
            });
        let standard = &self.pipeline_resolve[ShadingClass::Standard.index() as usize];
        let mut builder = graph
            .pass("shading/standard")
            .image(targets.visibility, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageWrite(compute))
            .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
            .buffer(targets.pages, BufferAccess::ShaderRead(compute))
            .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
            .buffer(tiles, BufferAccess::ShaderWrite(compute))
            .buffer(args, BufferAccess::ShaderReadWrite(compute));
        if let Some(sky) = ambient.sky {
            builder = builder
                .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
                .image(sky.table, ImageAccess::Sampled(compute));
        }
        if let Some(r) = request {
            builder = builder.image(r, ImageAccess::StorageWrite(compute));
        }
        if let Some(ao) = ambient.occlusion {
            builder = builder.image(ao, ImageAccess::Sampled(compute));
        }
        if let Some(p) = ambient.sky.and(ambient.probes) {
            builder = builder
                .buffer(p.data, BufferAccess::ShaderRead(compute))
                .image(p.irradiance, ImageAccess::Sampled(compute))
                .image(p.distance, ImageAccess::Sampled(compute));
        }
        builder.run(move |resources, commands| {
            commands.bind_pipeline(standard);
            commands.push_constants(standard, &push(resources));
            commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
            Ok(())
        });
        for class in &ShadingClass::ALL[1..] {
            let pipeline = &self.pipeline_resolve[class.index() as usize];
            let offset = u64::from(class.index()) * 16;
            let mut builder = graph
                .pass(match class {
                    ShadingClass::Standard => "shading/standard",
                    ShadingClass::Ice => "shading/ice",
                    ShadingClass::Layered => "shading/layered",
                })
                .image(targets.visibility, ImageAccess::Sampled(compute))
                .image(color, ImageAccess::StorageWrite(compute))
                .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
                .buffer(targets.pages, BufferAccess::ShaderRead(compute))
                .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
                .buffer(tiles, BufferAccess::ShaderRead(compute))
                .buffer(args, BufferAccess::IndirectArgsAndShaderRead(compute));
            if let Some(sky) = ambient.sky {
                builder = builder
                    .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
                    .image(sky.table, ImageAccess::Sampled(compute));
            }
            if let Some(r) = request {
                builder = builder.image(r, ImageAccess::StorageWrite(compute));
            }
            if let Some(ao) = ambient.occlusion {
                builder = builder.image(ao, ImageAccess::Sampled(compute));
            }
            if let Some(p) = ambient.sky.and(ambient.probes) {
                builder = builder
                    .buffer(p.data, BufferAccess::ShaderRead(compute))
                    .image(p.irradiance, ImageAccess::Sampled(compute))
                    .image(p.distance, ImageAccess::Sampled(compute));
            }
            builder.run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push(resources));
                commands.dispatch_indirect(args_buffer, offset);
                Ok(())
            });
        }
        // The mirror rays over the tiles holding smooth rows (issue #52).
        if let (Some(pipeline), Some(request), Some(sky)) = (reflections, request, ambient.sky) {
            let mut builder = graph
                .pass("shading/reflections")
                .image(targets.visibility, ImageAccess::Sampled(compute))
                .image(color, ImageAccess::StorageReadWrite(compute))
                .image(request, ImageAccess::StorageRead(compute))
                .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
                .buffer(targets.pages, BufferAccess::ShaderRead(compute))
                .buffer(targets.page_table, BufferAccess::ShaderRead(compute))
                .buffer(tiles, BufferAccess::ShaderRead(compute))
                .buffer(args, BufferAccess::IndirectArgsAndShaderRead(compute))
                .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
                .image(sky.table, ImageAccess::Sampled(compute));
            if let Some(ao) = ambient.occlusion {
                builder = builder.image(ao, ImageAccess::Sampled(compute));
            }
            if let Some(p) = ambient.sky.and(ambient.probes) {
                builder = builder
                    .buffer(p.data, BufferAccess::ShaderRead(compute))
                    .image(p.irradiance, ImageAccess::Sampled(compute))
                    .image(p.distance, ImageAccess::Sampled(compute));
            }
            builder.run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push(resources));
                commands.dispatch_indirect(args_buffer, REFLECTION_LIST * 16);
                Ok(())
            });
        }
    }

    /// One cluster cull over the work list: every work item's 32 clusters are culled and the
    /// survivors compacted into this pass's range of the visible-cluster list (and, on the
    /// fallback path, into indexed draws). The second pass also tests the depth pyramid.
    fn cull_pass<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        label: &'static str,
        pass: MeshPass,
        params: &DrawParams<'f>,
        slot: FrameSlot,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = if params.scene.streamer.is_some() {
            &self.pipeline_cluster_cull_streamed
        } else {
            &self.pipeline_cluster_cull
        };
        let grid: &'f GraphBuffer = &params.scene.indirect[slot.index];
        let MeshPass {
            io,
            frame_address,
            second,
            ..
        } = pass;
        let mut builder = graph
            .pass(label)
            .buffer(
                io.indirect,
                BufferAccess::IndirectArgsAndShaderRead(compute),
            )
            .buffer(io.work, BufferAccess::ShaderRead(compute))
            .buffer(io.roots, BufferAccess::ShaderRead(compute))
            .buffer(io.clusters, BufferAccess::ShaderReadWrite(compute))
            .buffer(io.cluster_lookback, BufferAccess::ShaderReadWrite(compute))
            .buffer(io.visible, BufferAccess::ShaderWrite(compute))
            .buffer(io.raster, BufferAccess::ShaderWrite(compute))
            .buffer(io.stats, BufferAccess::ShaderReadWrite(compute))
            .buffer(io.page_table, BufferAccess::ShaderRead(compute));
        if let Some(draws) = io.draws {
            builder = builder.buffer(draws, BufferAccess::ShaderWrite(compute));
        }
        if let Some(need) = io.need {
            builder = builder.buffer(need, BufferAccess::ShaderReadWrite(compute));
        }
        // Pass 1 tests the previous pyramid; pass 2 that one too (to skip what pass 1 drew) and
        // this frame's.
        if let Some(prev) = io.hzb_prev {
            builder = builder.image(prev, ImageAccess::Sampled(compute));
        }
        if second && io.hzb_prev != Some(io.hzb) {
            builder = builder.image(io.hzb, ImageAccess::Sampled(compute));
        }
        builder.run(move |_, commands| {
            commands.bind_pipeline(pipeline);
            commands.push_constants(pipeline, &self.push(frame_address));
            commands.dispatch_indirect(grid, 0);
            Ok(())
        });
    }

    /// Once the last cull has counted: copies the frame's counters into this slot's readback
    /// and hands them to the host, where [`MeshletRenderer::begin_frame`] reads them when the
    /// slot comes back. Under the last cull's label (the copy is part of its cost).
    fn stats_readback_passes<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        label: &'static str,
        io: MeshPassIo,
        slot: FrameSlot,
    ) {
        let (stats, readback) = (&self.stats, &self.stats_readback[slot.index]);
        graph
            .pass(label)
            .buffer(io.stats, BufferAccess::TransferSrc)
            .buffer(io.stats_readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(stats, readback, STATS_BYTES);
                Ok(())
            });
        graph
            .pass(label)
            .buffer(io.stats_readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// The hardware draw of the clusters a cull pass listed for it: a mesh workgroup per
    /// cluster, or on the fallback path an indexed draw per cluster through one
    /// `vkCmdDrawIndexedIndirectCount`, in list order (the ids grow with the draw order).
    /// The first pass clears the visibility buffer and the depth, the second loads them.
    fn draw_pass<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        label: &'static str,
        pass: MeshPass,
        params: &DrawParams<'f>,
        slot: FrameSlot,
    ) {
        use vk::PipelineStageFlags2 as S;
        let pipeline = if params.wireframe {
            &self.pipeline_wire
        } else {
            &self.pipeline_solid
        };
        let clusters: &'f GraphBuffer = &params.scene.clusters[slot.index];
        let triangles: &'f Buffer = &params.scene.pool;
        let draws: Option<&'f GraphBuffer> = self.lists[slot.index].draws.as_ref();
        let capacity = self.lists[slot.index].capacity;
        let extent = params.extent;
        let push = self.push(pass.frame_address);
        let MeshPass {
            io,
            args_offset,
            second,
            ..
        } = pass;
        let mut builder = graph.pass(label);
        builder = match io.draws {
            None => builder
                .buffer(
                    io.clusters,
                    BufferAccess::IndirectArgsAndShaderRead(S::MESH_SHADER_EXT),
                )
                .buffer(io.raster, BufferAccess::ShaderRead(S::MESH_SHADER_EXT))
                .buffer(io.visible, BufferAccess::ShaderRead(S::MESH_SHADER_EXT))
                .buffer(io.pool, BufferAccess::ShaderRead(S::MESH_SHADER_EXT))
                .buffer(io.page_table, BufferAccess::ShaderRead(S::MESH_SHADER_EXT)),
            // The fallback's draws carry the pool offsets: no page-table read.
            Some(draws) => builder
                .buffer(io.clusters, BufferAccess::IndirectArgs)
                .buffer(draws, BufferAccess::IndirectArgs)
                .buffer(io.visible, BufferAccess::ShaderRead(S::VERTEX_SHADER))
                .buffer(io.pool, BufferAccess::IndexAndShaderRead(S::VERTEX_SHADER)),
        };
        builder
            .image(io.visibility, ImageAccess::ColorAttachment)
            .image(io.depth, ImageAccess::DepthAttachment)
            .run(move |resources, commands| {
                let load = if second {
                    vk::AttachmentLoadOp::LOAD
                } else {
                    vk::AttachmentLoadOp::CLEAR
                };
                let color = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(io.visibility))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(load)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(vk::ClearValue {
                        color: vk::ClearColorValue {
                            uint32: [crate::visibility::EMPTY; 4],
                        },
                    })];
                let depth = vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(io.depth))
                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                    .load_op(load)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(vk::ClearValue {
                        depth_stencil: vk::ClearDepthStencilValue {
                            depth: 0.0,
                            stencil: 0,
                        },
                    });
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&color)
                    .depth_attachment(&depth);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(pipeline, &push);
                let result = match draws {
                    None => commands.draw_mesh_tasks_indirect(clusters, args_offset),
                    Some(draws) => {
                        commands.bind_index_buffer(triangles, 0, vk::IndexType::UINT8_KHR);
                        commands.draw_indexed_indirect_count(
                            draws,
                            0,
                            clusters,
                            args_offset + 12,
                            capacity,
                            DRAW_COMMAND_BYTES,
                        );
                        Ok(())
                    }
                };
                commands.end_rendering();
                result
            });
    }

    /// The software raster of the clusters a cull pass listed for it, after the hardware
    /// draw of the same pass: a compute workgroup per cluster, a thread per vertex then per
    /// triangle, keeping in the 64-bit samples only what beats the hardware's pixel; then the
    /// merge of those samples into the visibility buffer and the depth.
    fn software_passes<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        label: &'static str,
        pass: MeshPass,
        params: &DrawParams<'f>,
        slot: FrameSlot,
    ) {
        use vk::PipelineStageFlags2 as S;
        let (Some(raster), Some(merge), Some(vis64)) = (
            &self.pipeline_sw_raster,
            &self.pipeline_merge,
            pass.io.vis64,
        ) else {
            return;
        };
        let clusters: &'f GraphBuffer = &params.scene.clusters[slot.index];
        let extent = params.extent;
        let push = self.push(pass.frame_address);
        let MeshPass {
            io, args_offset, ..
        } = pass;
        graph
            .pass(label)
            .buffer(
                io.clusters,
                BufferAccess::IndirectArgsAndShaderRead(S::COMPUTE_SHADER),
            )
            .buffer(io.raster, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
            .buffer(io.visible, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
            .buffer(io.pool, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
            .buffer(io.page_table, BufferAccess::ShaderRead(S::COMPUTE_SHADER))
            .image(io.depth, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .image(io.visibility, ImageAccess::Sampled(S::COMPUTE_SHADER))
            .buffer(vis64, BufferAccess::ShaderReadWrite(S::COMPUTE_SHADER))
            .run(move |resources, commands| {
                commands.bind_pipeline(raster);
                commands.push_constants(
                    raster,
                    &Push {
                        depth_image: resources.sampled(io.depth).0,
                        vis_image: resources.sampled(io.visibility).0,
                        ..push
                    },
                );
                commands.dispatch_indirect(clusters, args_offset + 16);
                Ok(())
            });
        graph
            .pass("geometry/software raster merge")
            .buffer(io.clusters, BufferAccess::IndirectArgs)
            .buffer(vis64, BufferAccess::ShaderReadWrite(S::FRAGMENT_SHADER))
            .image(io.visibility, ImageAccess::ColorAttachment)
            .image(io.depth, ImageAccess::DepthAttachment)
            .run(move |resources, commands| {
                let color = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(io.visibility))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let depth = vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(io.depth))
                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE);
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&color)
                    .depth_attachment(&depth);
                commands.begin_rendering(&info);
                commands.bind_pipeline(merge);
                commands.set_viewport_full(extent);
                commands.push_constants(merge, &push);
                commands.draw_indirect(clusters, args_offset + 32);
                commands.end_rendering();
                Ok(())
            });
    }

    /// Builds the depth pyramid `hzb` (`io.hzb` in the graph) level by level: level 0 from
    /// the depth buffer, each next level from the previous one (one pass per level, one
    /// profiler zone for all).
    fn pyramid_passes<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        io: MeshPassIo,
        hzb: &'f GraphImage,
    ) {
        use vk::PipelineStageFlags2 as S;
        let pipeline = &self.pipeline_hzb;
        for level in 0..hzb.mip_levels() {
            let dst = hzb.mip_extent(level);
            let pass = graph.pass("geometry/depth pyramid");
            let pass = if level == 0 {
                pass.image(io.depth, ImageAccess::Sampled(S::COMPUTE_SHADER))
            } else {
                pass.image_mip(io.hzb, level - 1, ImageAccess::Sampled(S::COMPUTE_SHADER))
            };
            pass.image_mip(io.hzb, level, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
                .run(move |resources, commands| {
                    let (src_image, src_level) = if level == 0 {
                        (resources.sampled(io.depth).0, 0)
                    } else {
                        (hzb.sampled().0, level - 1)
                    };
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &HzbPush {
                            src_image,
                            src_level,
                            dst_image: hzb.storage(level).0,
                            dst_width: dst.width,
                            dst_height: dst.height,
                            pad: [0; 3],
                        },
                    );
                    commands.dispatch(dst.width.div_ceil(8), dst.height.div_ceil(8), 1);
                    Ok(())
                });
        }
    }
}

/// One mesh pass to declare: its cull and its draw share these.
#[derive(Clone, Copy)]
struct MeshPass {
    io: MeshPassIo,
    /// This pass's frame block.
    frame_address: u64,
    /// Offset of this pass's hardware (x, y, 1, count) in the cluster arguments; its
    /// software raster's follow 16 bytes later, then its merge's `VkDrawIndirectCommand`.
    args_offset: u64,
    /// The second pass of the two-pass occlusion: tests the depth pyramid, loads the targets.
    second: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_work_list_says_so_too() {
        let capped = FrameStats {
            visible_overflow: 1_500,
            work_overflow: 20_000,
            ..FrameStats::default()
        };
        assert_eq!(
            capped.overflow_note(),
            ", 2 k dropped (visible list full), 20 k work items or roots dropped (work list full)"
        );
    }

    #[test]
    fn a_capped_frame_says_how_much_it_dropped() {
        assert_eq!(FrameStats::default().overflow_note(), "");
        let capped = FrameStats {
            visible_overflow: 92_400,
            ..FrameStats::default()
        };
        assert_eq!(capped.overflow_note(), ", 92 k dropped (visible list full)");
    }

    #[test]
    fn the_list_grows_past_the_demand_and_never_shrinks() {
        let max = VISIBLE_MAX_CAPACITY;
        // Fits: unchanged, including a later frame that wants less.
        assert_eq!(grown_capacity(1 << 16, 15_000, max), 1 << 16);
        assert_eq!(grown_capacity(1 << 21, 10, max), 1 << 21);
        assert_eq!(grown_capacity(1 << 16, 1 << 16, max), 1 << 16);
        // Dropped: the power of two above 1.5 × the demand (1.14 M wanted → 2 M slots).
        assert_eq!(grown_capacity(1 << 16, 1_140_000, max), 1 << 21);
        assert_eq!(grown_capacity(1 << 16, (1 << 16) + 1, max), 1 << 17);
        // Capped by the id's slot bits, or by a lower fallback limit.
        assert_eq!(grown_capacity(1 << 16, 40_000_000, max), max);
        assert_eq!(grown_capacity(1 << 16, 1_000_000, 500_000), 500_000);
    }

    #[test]
    fn every_list_slot_fits_the_visibility_id() {
        // The id keeps 7 bits for the triangle (124 per cluster): the slot has the other 25.
        assert!(u64::from(VISIBLE_MAX_CAPACITY) <= 1 << (32 - 7));
    }

    #[test]
    fn the_software_raster_modes_parse_and_cycle() {
        for mode in SwRaster::ALL {
            assert_eq!(mode.name().parse::<SwRaster>(), Ok(mode));
            assert_eq!(mode.name().to_uppercase().parse::<SwRaster>(), Ok(mode));
        }
        assert!("sometimes".parse::<SwRaster>().is_err());
        assert_eq!(SwRaster::default(), SwRaster::Auto);
        let mut mode = SwRaster::Off;
        for _ in 0..SwRaster::ALL.len() {
            mode = mode.next();
        }
        assert_eq!(mode, SwRaster::Off, "R comes back to where it started");
    }

    #[test]
    fn auto_mode_switches_with_a_gap_between_its_thresholds() {
        const _: () = assert!(SW_RASTER_AUTO_OFF < SW_RASTER_AUTO_ON);
        let between = (SW_RASTER_AUTO_OFF + SW_RASTER_AUTO_ON) / 2;
        assert!(!software_worth_it(false, between), "stays off between");
        assert!(
            software_worth_it(false, SW_RASTER_AUTO_ON),
            "turns on at the upper one"
        );
        assert!(software_worth_it(true, between), "stays on between");
        assert!(
            !software_worth_it(true, SW_RASTER_AUTO_OFF - 1),
            "turns off below the lower one"
        );
    }

    #[test]
    fn the_software_samples_take_eight_bytes_a_pixel() {
        let extent = vk::Extent2D {
            width: 1600,
            height: 900,
        };
        assert_eq!(vis64_bytes(extent), 1600 * 900 * 8);
    }

    #[test]
    fn the_mesh_record_lists_up_to_four_roots() {
        let rock = forge_geom::procedural::asteroid(forge_core::Seed::new(7), 32, 1.0, 0.3);
        let mut mesh = MeshletMesh::build(&rock);
        let roots: Vec<u32> = (0..mesh.meshlets.len() as u32)
            .filter(|&i| mesh.meshlets[i as usize].parent_error.is_infinite())
            .collect();
        assert!((1..=MAX_SHORTCUT_ROOTS).contains(&roots.len()));
        let mut builder = MeshletSceneBuilder::new();
        builder.add_mesh(&mesh);
        let record = builder.meshes[0];
        assert_eq!(record.root_count as usize, roots.len());
        assert_eq!(record.roots[..roots.len()], roots[..]);
        let error = roots
            .iter()
            .map(|&r| mesh.meshlets[r as usize].self_error)
            .fold(0.0, f32::max);
        assert_eq!(record.root_error_max, error);
        assert!(record.root_reach_max > 0.0);
        let groups = record.meshlet_count.div_ceil(TASK_GROUP_SIZE);
        assert_eq!(record.work_bound(), groups.max(record.root_count));

        // More roots than the shortcut takes: the instances take work items.
        for m in mesh.meshlets.iter_mut().take(MAX_SHORTCUT_ROOTS + 1) {
            m.parent_error = f32::INFINITY;
        }
        builder.add_mesh(&mesh);
        assert_eq!(builder.meshes[1].root_count, 0);
    }
}
