// Meshlet rendering: compute passes cull instances and clusters into a compacted list of
// visible clusters, which the mesh shader draws or, on GPUs without mesh shaders, the vertex
// fallback through one `vkCmdDrawIndexedIndirectCount` (same list, same ids, same pixels).
// Clusters that project to a few pixels are rasterised in compute instead (issue #3), into
// 64-bit samples (depth << 32 | visibility id, kept with an atomic max) that a merge pass
// then writes into the hardware's targets. Every path keeps the same pixel: the nearest
// and, at equal depth, the larger id (the hardware's depth test keeps the one drawn last,
// and it draws in id order).
//
// Every buffer is reached through a device address held in the per-frame `Frame` block, so
// the CPU hands the GPU one pointer (push constant) and the GPU walks the scene from there.
// Layouts are std430 and mirrored by `#[repr(C)]` structs in `forge-geom` / `forge-render`.
//
// Occlusion is two-pass (Haar & Aaltonen 2015, Nanite 2021): pass 1 draws the meshlets that
// the previous frame's hierarchical-Z pyramid shows, seen from the previous culling camera;
// the renderer then builds this frame's pyramid from that depth (into the other of two
// images); pass 2 re-derives pass 1's answer to skip what it drew and tests every other
// meshlet against the new pyramid. No state is kept per cluster, so the scene can hold any
// number of instances (issue #33). Each mesh pass is a cluster-cull compute pass followed by
// the draw of what it kept.
import bindless;
import vis64;
import barycentrics;
import sh;
import skyview;
import noise;
import probes;
import shore;
import brdf;

// ---------------------------------------------------------------- shared data ---

// A vertex in a cluster page (crates/forge-geom/src/page.rs): the exact object-space
// position and a 16-bit octahedral normal (`decode_normal`).
public struct PagedVertex
{
    public float3 position;
    public uint normal;
};

// Bytes per cluster page (forge_geom::PAGE_SIZE) and per paged vertex.
static const uint PAGE_SIZE = 131072;
static const uint PAGED_VERTEX_BYTES = 16;

// One cluster of at most 64 vertices / 124 triangles (meshoptimizer output).
public struct Meshlet
{
    public float3 center;      // bounding sphere, object space
    public float radius;
    public float3 cone_apex;   // normal cone, object space (meshoptimizer)
    public float cone_cutoff;
    public float3 cone_axis;
    public uint page;            // the page holding the payload (an index into `page_table`)
    public uint payload;         // bytes into the page: the vertices, then 3-byte triangles
    public uint vertex_count;
    public uint triangle_count;
    public uint child_page;      // the page of the clusters this one is refined into (none at level 0)
    // Cluster LOD DAG (crates/forge-geom/src/lod.rs): the sphere and error of the group this
    // cluster came from (`self`) and of the group that simplified it away (`parent`; an
    // infinite error marks a root). Draw when project(parent) > threshold >= project(self).
    public float3 self_center;
    public float self_radius;
    public float3 parent_center;
    public float parent_radius;
    public float self_error;
    public float parent_error;
    public uint lod_level;
    public uint section;         // its triangles' material sections, a | b << 8 | split << 16 (`triangle_section`)
};

static const uint LOD_LEVELS = 16; // forge_geom::MAX_LEVELS

// One mesh: a range of meshlets stored level by level, with per-level bounds so a work item
// can tell without touching a cluster whether any of its clusters can be selected.
public struct Mesh
{
    public uint meshlet_offset;
    public uint meshlet_count;
    public uint triangle_count;
    public uint level_count;
    public float3 center;
    public float radius;
    public uint level_offset[LOD_LEVELS + 1]; // relative to meshlet_offset, plus the end
    public uint skin;              // a skinned mesh's first cluster in the skin pass's table (#165), 0xFFFFFFFF for others
    public uint uvs;               // 1: its payloads carry a UV stream after their triangles (D-047)
    public uint flags;             // MESH_WINDOWED_GROUND, MESH_SCENE_SHADED (#197)
    public float self_error_min[LOD_LEVELS];
    public float parent_error_max[LOD_LEVELS];
    public float self_reach_max[LOD_LEVELS];   // max |self_center - center| + self_radius per level
    public float parent_reach_max[LOD_LEVELS];
    // The DAG's roots (issue #37): their local indices when there are at most four, the
    // largest `self_error` among them and how far their `self` spheres reach from `center`.
    public uint roots[4];
    public uint root_count;        // 0: more than four roots, no shortcut (`instance_roots`)
    public float root_error_max;
    public float root_reach_max;
    public uint material;          // the material its instances take unless they name their own (placement)
    // D-075's ladder (#302): the mesh its instances cross to when they stand smaller on screen,
    // LADDER_NONE for none, LADDER_VANISH for nothing (#343: it dissolves away); the projected
    // size of an instance's sphere, output pixels across, at which the crossing is half done;
    // and the band's half width, a share of that size.
    public uint ladder_next;
    public float ladder_px;
    public float ladder_band;
    public uint ladder_pad;
};

static const uint LADDER_NONE = 0xFFFFFFFFu;
static const uint LADDER_VANISH = 0xFFFFFFFEu;

// A row of the material table (`GpuMaterial` in crates/forge-render/src/material.rs, from
// the D-007 record in crates/forge-core/src/material.rs). Colours are in units of a white
// Lambertian surface facing the sun.
public struct Material
{
    public float3 color_a;          // standard: each instance mixes a and b; ice: hollows to crests
    public uint class_index;        // MATERIAL_CLASS_*: the resolve pass that shades it
    public float3 color_b;
    public float cavity;            // standard: how much darker the surface gets facing its object's centre
    public float3 emissive;
    public float specular_power;    // Blinn-Phong exponent (from the roughness)
    public float3 scatter;          // ice: light scattered back where the surface turns from the sun
    public float specular;          // weight of the highlight
    public uint albedo_texture;     // sampled-image indices, TEXTURE_NONE for none
    public uint normal_texture;
    public float inv_texture_scale; // texture repeats per metre of object space
    public float normal_strength;
    public float reflectance;       // Fresnel's F0: the share reflected at normal incidence (issue #56)
    public float scattering;        // ice: light scattered per metre by its bubbles (issue #61)
    public uint flags;              // MATERIAL_FLAG_*
    public uint contour;            // layered: the contour's layer under its height plus one, 0 for none
    public uint contour_above;      // its layers over the height, one bit each
    public float contour_height;    // its height, object space, and how far it wanders (metres)
    public float contour_wander;
    public uint contour_below;      // its other layers under the height, one bit each (#128)
    // UV-mapped rows (MATERIAL_FLAG_UV, D-047): a model's material as glTF gives it.
    public uint metal_rough_texture; // roughness in green, metalness in blue
    public uint occlusion_texture;  // occlusion in red
    public uint emissive_texture;
    public uint sampler;            // SAMPLER_ANISOTROPIC_WRAP plus its wrapping
    public float3 uv_u;             // the UV transform: u' = dot(uv_u, (u, v, 1))
    public float metallic;
    public float3 uv_v;             // v' = dot(uv_v, (u, v, 1))
    public float roughness;
    public float occlusion_strength;
    public float alpha_cutoff; // MATERIAL_FLAG_MASKED: cut out under this base colour alpha (#171)
    public float alpha;        // the base colour factor's alpha
    public uint pad;
};

static const uint MATERIAL_FLAG_HEX_TILING = 1; // Material::flags, forge_render::material
static const uint MATERIAL_FLAG_UV = 2;         // the textures read the mesh's UVs (D-047)
static const uint MATERIAL_FLAG_MASKED = 4;     // cut out by the base colour's alpha (#171)
static const uint MATERIAL_FLAG_DOUBLE_SIDED = 8; // drawn from both sides (#171)
static const uint MATERIAL_FLAG_PLANET = 16;       // layered: the layers by height, slope and latitude on the frame's planet (#220)
static const uint MATERIAL_FLAG_WORLD_PROJECTED = 32; // the textures and their noise from the scene frame, not the object's
static const uint MATERIAL_FLAG_INTERIOR = 64;   // a window's glass with a room behind it (D-060's step 3)
static const uint MATERIAL_FLAG_GRATING = 128;   // with MASKED: upright bars on one card, `alpha` their share of the mesh's u (`grating_coverage`)
static const uint MATERIAL_FLAG_SHOP = 256;      // with INTERIOR: a shop behind the glass (`interior_room`)
static const uint MATERIAL_FLAG_EMISSIVE_ABSOLUTE = 512; // `emissive` in cd/m², converted by the frame's reference illuminance (`emission`, #254)
static const uint MATERIAL_FLAG_FOLIAGE = 1024; // with MASKED and DOUBLE_SIDED: leaf cards, their normals never turned, a mix of the two colours per instance (D-013)
static const uint MATERIAL_FLAG_GLAZED = 2048;  // a window's glass a probe's ray from its room passes (D-068)
static const uint MATERIAL_FLAG_LAMP = 4096;    // a lamp's glowing part: the probes' rays take none of its glow (D-065)
static const uint MATERIAL_FLAG_DRAWN_AFTER = 8192; // left out of the visibility pass, drawn after the resolve; the rays meet it (D-065's glass)
static const uint CUTOUT_GROUND_WINDOW = 1u << 31; // `cluster_cutouts`: windowed ground reaching into the ground window (#197)
static const uint CUTOUT_LADDER = 1u << 30;        // `describe_cluster`: a cluster of an instance crossing its ladder (D-075), dithered (`cutout_kept`)
static const uint MESH_WINDOWED_GROUND = 1;     // Mesh::flags: its fragments in the frame's ground window go (#197)
static const uint MESH_SCENE_SHADED = 2;        // Mesh::flags: a ground window's mesh, shaded where it stands in the scene (#197)
static const uint INSTANCE_STILL = 1;          // Instance::flags: a mover with no motion of its own this frame (#197)
static const uint INSTANCE_VACANT = 2;         // Instance::flags: a dynamic scene's slot no instance holds (#220)
static const uint INSTANCE_NO_RASTER = 4;      // Instance::flags: left out of the picture, a stand-in the rays alone meet (#301)
static const uint INSTANCE_NO_SHADOW_RAYS = 8; // Instance::flags: not met by the shadow rays (the sun's, the lamps'; #301)
static const uint INSTANCE_NO_PROBE_RAYS = 16; // Instance::flags: not met by the probes' rays (#301)
static const uint INSTANCE_NO_MIRROR_RAYS = 32; // Instance::flags: not met by the mirror rays (#301)
static const uint INSTANCE_NO_FAR_SHADOW_RAYS = 64; // Instance::flags: not met by the shadow rays from far points (D-075 step 3, `shadow_world`)
// The TLAS records' mask bits, one a world of rays (#301): an instance a kind of ray does not meet
// leaves its bit out, and a pass's rays carry their own (`g_ray_mask`).
static const uint RAY_SHADOW = 1;
static const uint RAY_PROBE = 2;
static const uint RAY_MIRROR = 4;
static const uint RAY_SHADOW_FAR = 8; // the shadow rays from points `Frame::far_shadows` metres off and more

public static const uint MATERIAL_CLASS_STANDARD = 0;
public static const uint MATERIAL_CLASS_ICE = 1;
public static const uint MATERIAL_CLASS_LAYERED = 2;
public static const uint MATERIAL_CLASS_JELLY = 3;
public static const uint MATERIAL_CLASSES = 4;
static const uint TEXTURE_NONE = 0xFFFFFFFFu;

// An instance's position is an integer cell of CELL_SIZE metres and an offset inside it
// (issue #93, D-004's amendment; crates/forge-render/src/cells.rs): its camera-relative position
// is the cell difference to the camera's, taken in integers, plus the difference of the offsets
// (`instance_relative`), exact near the camera wherever the scene stands in the world. The
// transform is a unit quaternion and a uniform scale (`instance_frame`).
public struct Instance
{
    public int3 cell;
    public uint mesh;
    public float3 local;    // metres from the cell's corner
    public float scale;
    public float4 rotation; // unit quaternion, xyz w
    public float3 center;   // the bounding sphere's centre, from the same cell's corner
    public float radius;
    public uint id;
    public uint material;   // its row in `Frame::materials`
    public uint flags;      // INSTANCE_STILL (#197), INSTANCE_VACANT (#220), INSTANCE_NO_* (#301)
    public uint texture;    // its own texture's sampled index plus one, 0 for none: a planet tile's normal map (#220)
};

// Side of a cell, metres (forge_render::cells::CELL_SIZE): a power of two, so the product with
// a cell difference is exact.
public static const float CELL_SIZE = 1024.0;

// With FLAG_INSTANCE_CELLS: the bounding sphere of a cell of 64 instances (issue #38), its
// centre from the corner of the cell (`CELL_SIZE`) of the first of the 64 (issue #93).
public struct CellBounds
{
    public int3 cell;
    public float radius;
    public float3 center;
    public float pad;
};

public static const uint FLAG_FREEZE_CULLING = 1;
public static const uint FLAG_CONE_CULLING = 2;
public static const uint FLAG_FRUSTUM_CULLING = 4;
public static const uint FLAG_OCCLUSION_CULLING = 8;
public static const uint FLAG_MESHLET_COLORS = 16;
// Debug: draw what cone / occlusion culling rejected, tinted red. A correctly culled meshlet
// is back-facing or hidden and leaves no pixel, so every red pixel is a culling error.
public static const uint FLAG_SHOW_CULLED = 32;
// Select clusters by projected LOD error; off = level 0 only (the full-detail mesh).
public static const uint FLAG_LOD = 64;
// Debug: colour by LOD level.
public static const uint FLAG_LOD_COLORS = 128;
// Debug: disable the per-group LOD window (every group tests its clusters); for the A/B harness.
public static const uint FLAG_GROUP_WINDOW_OFF = 256;
// Rasterise the dense clusters (`dense_cluster`) of the first pass in compute.
public static const uint FLAG_SW_RASTER = 512;
// Debug: tint what the software rasteriser drew.
public static const uint FLAG_SHOW_RASTER = 1024;
// Flags in the meshlet word of a visible-list record, above the meshlet index.
static const uint CULLED_BIT = 0x80000000u;   // drawn although culling rejected it (FLAG_SHOW_CULLED)
static const uint SOFTWARE_BIT = 0x40000000u; // rasterised in compute
// The previous frame's pyramid and culling camera (`Frame::prev_*`) are valid.
public static const uint FLAG_PREV_PYRAMID = 2048;
public static const uint FLAG_STREAMING = 4096;     // a streamed scene: the cut follows the resident pages (`lod_selected`)
// Trace the sun's shadows against `Frame::tlas` (issue #45).
public static const uint FLAG_SHADOWS = 8192;
// Show the ambient occlusion in grey (issue #48).
public static const uint FLAG_SHOW_AO = 16384;
// Under a sky, reflect it (issue #49).
public static const uint FLAG_SKY_REFLECTIONS = 32768;
// On the smooth rows, trace the mirror ray against `Frame::tlas` (issue #50).
public static const uint FLAG_RAY_REFLECTIONS = 65536;
// Translucent ice: the sun through its thickness (issue #59).
public static const uint FLAG_TRANSLUCENCY = 131072;
// Show the diffuse light alone, on white (issue #53).
public static const uint FLAG_SHOW_GI = 262144;
// Instance cull 1 defers the instances the previous pyramid hides to instance cull 2 (issue #38).
public static const uint FLAG_INSTANCE_OCCLUSION = 524288;
// Instance cull 1 counts the instances the previous pyramid hides (the renderer's auto mode).
public static const uint FLAG_HIDDEN_INSTANCES = 1048576;
// A cell cull tests the instances 64 at a time before instance cull 1 (issue #38).
public static const uint FLAG_INSTANCE_CELLS = 2097152;
// The scene has cut-out or double-sided rows: their clusters take the cut-outs' raster (#171).
public static const uint FLAG_CUTOUTS = 4194304;
// The sun's shadow from 256 rays a pixel, no denoiser: the reference SIGMA is judged by (#172).
public static const uint FLAG_SHADOW_REFERENCE = 8388608;
// The scene has jelly rows: the sun's shadow rays see through them, tinted (#180).
public static const uint FLAG_JELLY = 16777216;
// Tint each surface by the probes' region that lights it (D-068).
public static const uint FLAG_SHOW_REGIONS = 33554432;
// Debug: tint the clusters of the meshes instances cross to on their ladder (D-075) blue.
public static const uint FLAG_SHOW_LADDER = 67108864;
// An impostor's bake (D-075 step 4): the base colour, or the shading normal (× 0.5 + 0.5), unlit,
// instead of the shading (`sun_light`, `leaf_transmission`). Only the bake's own resolve
// (`meshlet_bake.slang`, FORGE_IMPOSTOR_BAKE) reads them: the frame's entries carry none of it, as
// a branch they never took changed their code enough to show #349's unshaded tiles.
public static const uint FLAG_BAKE_ALBEDO = 134217728;
public static const uint FLAG_BAKE_NORMAL = 268435456;
// GGX without its multiple-scattering compensation (D-086, `r.specular.single_scatter`): Falcor's,
// for the material chart's comparisons.
public static const uint FLAG_SINGLE_SCATTER = 536870912;

static const uint PAGE_NONE = 0xFFFFFFFF;           // an absent page in `page_table`, no page in `child_page`
static const uint MESHLET_INDEX_MASK = 0x3FFFFFFFu;

public static const uint PASS_PREVIOUSLY_VISIBLE = 1;
public static const uint PASS_REMAINDER = 2;
public static const uint PASS_SINGLE = 3; // no occlusion: everything in one pass

// Statistics written by the shaders, read back by the CPU two frames later.
public static const uint STAT_MESHLETS_PASS1 = 0;
public static const uint STAT_MESHLETS_PASS2 = 1;
public static const uint STAT_TRIANGLES = 2;
public static const uint STAT_OCCLUDED = 3;
public static const uint STAT_INSTANCES_VISIBLE = 4;
public static const uint STAT_LOD_LEVEL_SUM = 5; // sum of the LOD level of every drawn meshlet
public static const uint STAT_VISIBLE_OVERFLOW = 6; // clusters dropped because the visible list was full
public static const uint STAT_SW_CLUSTERS = 7;      // clusters rasterised in compute
public static const uint STAT_SW_TRIANGLES = 8;     // their triangles (also counted in STAT_TRIANGLES)
public static const uint STAT_DENSE_TRIANGLES = 9;  // triangles of the drawn dense clusters, software rasteriser on or off
public static const uint STAT_WORK_ITEMS = 10;      // work items the instance cull emitted (the demand)
public static const uint STAT_WORK_OVERFLOW = 11;   // of those, dropped because the work list was full (and root entries past the root list)
public static const uint STAT_ROOT_ENTRIES = 12;    // root clusters listed by the instance cull (the demand)
public static const uint STAT_INSTANCES_OCCLUDED = 13; // instances in the frustum the previous pyramid hid (issue #38)
public static const uint STAT_CELLS_LISTED = 14;   // cells of 64 instances cell cull 1 listed for instance cull 1 (issue #38)
public static const uint STAT_CELLS_DEFERRED = 15; // cells the previous pyramid hid whole, deferred to cell cull 2
public static const uint STAT_CELLS_OPENED = 16;   // of those, the cells this frame's pyramid did not hide
public static const uint STAT_REJECTED = 17;       // clusters pass 1 selected in view but the previous pyramid hid: pass 2's to test (issue #92)

// The visibility buffer: every drawn pixel holds `visible_slot << 7 | triangle`, where
// `visible_slot` indexes the frame's visible-cluster list (`Frame::visible`, filled by the
// cluster cull) and `triangle` the cluster's triangle; `VISIBILITY_EMPTY` is the clear value.
// The software rasteriser keeps `depth << 32 | id` per pixel in `Push::vis64` (the depth's
// float bits: reversed-Z, nearer is larger; 0 is empty) until the merge.
public static const uint VISIBILITY_EMPTY = 0xFFFFFFFFu;
public static const uint VISIBILITY_TRIANGLE_BITS = 7;
// A grass blade's id (#300, D-075) has the top bit, which no cluster's has (the visible list
// stops at 2^24 slots): `VISIBILITY_GRASS_BIT | cell << 10 | blade << 3 | triangle`, its cell of
// the grass map, its index there and its triangle (`grass_blade`).
public static const uint VISIBILITY_GRASS_BIT = 0x80000000u;

// Whether the (non-empty) id `vis` is a grass blade's.
bool is_grass(uint vis)
{
    return (vis & VISIBILITY_GRASS_BIT) != 0u && vis != VISIBILITY_EMPTY;
}

// What a ray's hit reads to shade its triangle (issue #50, crates/forge-render/src/raytrace.rs):
// every mesh's cut, as the bottom-level structures hold it, and where each mesh starts in it.
public struct RtMesh
{
    public uint first_vertex;
    public uint first_triangle;
    public float cut_error;  // how far the cut may stand from the drawn surface, object space
    public float shadow_start; // how far its surfaces' shadow rays start, object space (a terrain's, #96; 0: by its cut, or SHADOW_BIAS with cut-outs; negative: by its cut, #326)
    public uint first_uv;      // its first vertex's UV in `RtScene::uvs`, RT_NO_UVS without them (#171)
    public uint rows;          // the rows its instances draw with: its cut's highest section and one (#265)
    public uint masked_sections;  // the sections in its structure's second geometry, not opaque, a bit each (#307)
    public uint opaque_triangles; // its first geometry's triangles: the second's triangle p is the cut's opaque_triangles + p
    public uint opacity_material; // the instance material whose cut-out rows its opacity micromaps follow (#333), RT_NO_OPACITY without
    public uint opacity_probes;   // 1: its maps hold no opaque cut-out micro-triangle, so the probes' rays may use them
};

static const uint RT_NO_UVS = 0xFFFFFFFFu;
static const uint RT_NO_OPACITY = 0xFFFFFFFFu;

public struct RtScene
{
    public float* positions;  // three floats a vertex
    public uint* indices;     // three a triangle, local to the mesh
    public uint* sections;    // one a triangle: the row after the instance's first it takes
    public RtMesh* meshes;
    public float2* uvs;       // the meshes with UVs' (#171)
    public uint64_t pad;
};

// A decal (D-074, forge_render::decals): a box in the scene frame, its `x` along its texture's
// u, its `z` along v, its `y` the axis it projects along.
public struct Decal
{
    public float3 centre;
    public uint kind;
    public float4 rotation; // a unit quaternion, xyz w
    public float3 half;     // the box's half sizes
    public float repeat;    // a strip's repeats along x
    public float phase;     // where a strip's texture starts along x, in repeats
    public float pad0;
    public float pad1;
    public float pad2;
};

// A row decal's `DecalKind::row` for a colour decal.
static const uint DECAL_NO_ROW = 0xFFFFFFFFu;

// A kind of decal: its texture (colour premultiplied by alpha), its sampler (a stamp's clamps, a
// strip's repeats along u), how it fades by depth and by the surface's facing, and its tint.
public struct DecalKind
{
    public uint texture;
    public uint sampler;
    public float depth_fade; // the share of the box's depth it fades over towards both faces
    public uint row;         // a row decal's material row, its texture's alpha the mask (~0: a colour decal)
    public float4 tint;      // rgb times its colour, a its strength
    public float4 facing;    // x: the cosine to the box's y under which it is gone, y: over which whole
};

// The decals' grid (D-074): columns of `cell` metres (x, z) in an open-addressed hash of
// `slot_mask` + 1 slots (x, z, first, count; count 0 an empty slot), each column's decals at
// `indices[first..]` in painter's order.
public struct DecalGrid
{
    public uint4* slots;
    public uint* indices;
    public Decal* decals;
    public DecalKind* kinds;
    public uint slot_mask;
    public float cell;
    public uint count;
    public uint pad;
};

// The street lamps lit beyond the frame's set (#284, lamps.rs `FarLamps`): columns of `cell`
// metres in a hash as the decals', each column's lamps at `indices[first..]`, indices into
// `lamps` (their whole light) and the frame's shares of it (`Frame::far_shares`).
public struct FarLamps
{
    public uint4* slots;
    public uint* indices;
    public Lamp* lamps;
    public uint slot_mask;
    public float cell;
    public uint count;
    public uint pad;
};

// Every matrix is camera-relative (D-004, issue #93): the camera stands at the origin of the
// space `view_proj` takes, and positions get there through `instance_relative`. Rays and the
// probes live in the scene frame, `camera_in_scene` from the camera (`to_scene`).
// A cell of the grass map (#300, D-075; `forge_render::grass::GrassCell`).
public struct GrassCell
{
    public float level;             // the ground's height at the cell's middle, the scene frame
    public uint look;               // its blades (bits 0–7, at most GRASS_CELL_BLADES), their height in centimetres (8–15), their row after `Frame::grass_row` (16–22), the ninths no blade stands in (23–31, bit 23 + x + 3 z)
};

public struct Frame
{
    public float4x4 view_proj;      // for drawing
    public float4x4 cull_view_proj; // for culling (frozen when FLAG_FREEZE_CULLING)
    public float4x4 cull_view;      // view matrix used for culling
    public float4x4 prev_cull_view; // the previous frame's, from this frame's camera-relative space (with FLAG_PREV_PYRAMID)
    public float4 planes[6];        // frustum planes of `cull_view_proj`, inward normals
    public int4 camera_cell;        // the culling camera's cell (xyz)
    public float4 camera_in_scene;  // xyz: the camera in the scene frame, the TLAS's and the probes'
    public float3 camera_local;     // the culling camera's offset in its cell, metres
    public uint instance_count;
    public uint work_capacity;      // slots in `work`, and in `roots`
    public uint flags;
    public uint pass;
    public uint hzb_image;          // sampled image index of the depth pyramid
    public uint2 hzb_size;          // level-0 size of the pyramid
    public float p00;               // projection scale x (cull projection)
    public float p11;               // projection scale y
    public float znear;
    public float3 sun_dir;          // direction to the light
    public float2 draw_jitter;      // where the drawn image sits relative to the culling camera (uv, y down)
    public float lod_threshold;     // pixels of projected error a drawn cluster may have
    public float viewport_height;   // pixels
    public uint* pool;              // the resident cluster pages, `PAGE_SIZE` bytes each (see `payload_base`)
    public Meshlet* meshlets;
    public uint* page_table;        // per page of the scene, its slot in `pool`
    public Mesh* meshes;
    public Instance* instances;
    public uint* stats;
    public uint64_t* cluster_lookback; // status words of the cluster culls' ordered appends: `cluster_groups` per pass, then as many per pass for (pass 1's rejects, issue #92; the cut-out clusters, #265)
    public uint cluster_groups;     // cluster-cull workgroups the fullest lists need: `work_capacity` items, then `work_capacity` roots at 32 an item
    public uint prev_hzb_image;     // sampled image index of the previous frame's pyramid
    public uint2* work;             // (instance, local group) per work item, built by `cull_main`
    public uint* indirect;          // the cluster cull's grid (x, y, 1, work item count), the instance cull's ticket (4), the regular work items (5) and the root entries (6) the items cover
    public uint2* visible;          // (instance, meshlet | flag bits) per drawn cluster, in cull order: pass 1, then pass 2
    public uint visible_capacity;   // slots in `visible`
    public float exposure;          // pre-exposure of the frame (crates/forge-render/src/exposure.rs)
    public float sun_illuminance;   // lux; the rocks return albedo × E / π
    public float sun_angular_radius; // the sun disc's angular radius, radians; 0: hard shadows (issue #54)
    public uint* clusters;          // per pass (pass 1 or the single pass at 0, pass 2 at 16): the hardware draw grid (x, y, 1) and its cluster count, the software raster grid and its count, the merge's VkDrawIndirectCommand, the cut-outs' raster grid and its count (#265); then the culls' tickets (32, 33)
    public uint* draws;             // fallback: a VkDrawIndexedIndirectCommand per hardware-drawn cluster of the pass, the cut-out ones after `visible_capacity` (#265); null on the mesh path
    public uint64_t* lookback;      // status words of the instance cull's ordered appends
    public uint* raster;            // per pass, the list slots of its clusters: hardware-drawn from the front, software-rasterised from the back, then from `visible_capacity` the cut-out ones (#265)
    public uint target_width;       // pixels
    public uint target_height;
    public float sw_raster_area;    // a cluster whose bounding rectangle holds fewer pixels than this per triangle is dense (rasterised in compute with FLAG_SW_RASTER)
    public uint noise_frame;        // the frame index of the spatio-temporal noise (noise.slang)
    public float2 prev_draw_jitter; // the previous frame's `draw_jitter`
    public float prev_p00;          // the previous culling camera's projection scales
    public float prev_p11;
    public uint2* roots;            // (instance, local cluster) per root entry of the instance cull, `work_capacity` of them
    public uint* page_need;         // streaming: per page, the largest error in pixels it takes away (a float's bits)
    public Material* materials;     // the material table (D-007), indexed by `Instance::material`
    public float4 sun_color;        // rgb: the sunlight's colour at the scene (white-ish in space, filtered by the air on a planet)
    public uint64_t tlas;           // the scene's top-level acceleration structure (0: none; issue #45)
    public RtScene* rt_scene;       // what a ray's hit reads to shade its triangle (null: none; issue #50)
    public uint* deferred;          // with occlusion: instance cull 2's grid (x, y, 1, count), then the instances instance cull 1 deferred and the cells cell cull 2 opened (issue #38; DEFERRED_LIST)
    public CellBounds* cells;       // with FLAG_INSTANCE_CELLS: per cell of 64 instances in table order, their bounding sphere (issue #38)
    public uint* cell_list;         // with FLAG_INSTANCE_CELLS: instance cull 1's grid (x, y, 1, listed cells), cell cull 2's grid over the deferred cells (4), the cell culls' tickets (8, 9), then from CELL_LIST the listed cells and after them the deferred ones
    public uint2* rejects;          // with occlusion: (instance, meshlet | CULLED_BIT) per cluster pass 1 selected in view but the previous pyramid hid, in cull order: pass 2's to test (issue #92)
    public uint rejects_capacity;   // slots in `rejects`
    public float texture_lod_scale; // the UVs' change per pixel scaled by 2^bias (1, or below when an upscaler draws smaller: the textures as sharp as at the output's size)
    public uint64_t tlas_movers;    // the movers' top-level structure, rebuilt every frame (0: none; issue #79)
    public SkinCluster* skin_clusters; // with skinned meshes (#165): the skin pass's clusters (skin.slang), else null
    public SkinVertex* skin_vertices;  // their vertices in the bind pose, from which their textures are projected (#166)
    public float4* skin_joints;        // this frame's joints' matrices, three rows each: they turn the textures' relief with the body
    public uint64_t skin_pad;
    public float4 ground_window;    // the ground window (#197): min x, min z, max x, max z in the scene's frame, where MESH_WINDOWED_GROUND meshes draw no fragment (min > max: none)
    public float4 planet;           // the planet the scene stands on (#220): the camera from its centre, world axes (xyz), and its height over the sea (w), metres
    public float4 planet_axis;      // its north (xyz) and its radius at the sea's level (w, metres; 0: none)
    public float4 planet_meridian;  // its latitude 0, longitude 0 (xyz)
    public uint4 planet_map;        // x: the sampled index of its colour map (~0: none); y: 1 when it has a sea; z: 1 when the map is its albedo from afar, 0 a tint; w: its sea mask (~0: none)
    public Lamp* lamps;             // the lamps the frame lights with (D-065, lamps.rs), the nearest first
    public uint lamp_count;
    public uint lamp_list_count;    // the entries of their light lists (lamps.rs, `lists`)
    public uint2* lamp_lists;       // (region, lamp) sorted by region, the lamps of every region under REGION_ANY
    public DecalGrid* decals;       // the decals on the static surfaces (D-074), null: none
    public FarLamps* far_lamps;     // the street lamps lit beyond the frame's set (#284), null: none
    public float* far_shares;       // their shares of their light this frame, unshadowed
    public float* ladder;           // per instance on a ladder (D-075), this frame's crossing (`ladder_crossing`) with the fade after it packed in (`ladder_fade_share`, #363), written by the instance culls; null: no ladder
    public float far_shadows;       // metres from the camera from which the shadow rays meet RAY_SHADOW_FAR's world (D-075 step 3, `shadow_world`); 0: never
    public uint far_shadows_pad;
    public GrassCell* grass;        // the grass map's cells (#300, D-075), rows of increasing z; null: no grass
    public float2 grass_origin;     // its corner (x, z), the scene frame
    public float grass_cell;        // a cell's side, metres
    public float grass_reach;       // how far from the camera blades stand, metres
    public uint grass_width;        // its cells along x
    public uint grass_row;          // the blades' material row
    public float4 grass_wind;       // xy: the wind's way on the ground (unit), z: its strength (the tips' lean in it, a share of their height), w: the sway's time (s)
    public float grass_wind_before; // the sway's time the frame before (s): the blades' motion (`grass_motion_main`)
    public uint grass_depth;        // its cells along z
    public uint show_input;         // a debug view of a standard row's shading input (D-086, `r.show.input`, `shading_input`); 0: none
    public uint gi_reference;       // D-088's reference for the diffuse light (`r.gi.reference`, `near_field_reference`); 0: the probes
};

// Whether this thread's rays meet the movers: every pass's but the probes' unless their scene asks
// (`ProbeParams::movers`, #299), which `probe_trace_main` sets.
static bool g_trace_movers = true;

// The world this thread's rays meet (#301): RAY_SHADOW unless its pass sets another, the probes'
// RAY_PROBE and the mirror rays' RAY_MIRROR, so an instance left out of one is out of all that
// pass's rays (a probe's hit's shadow rays too).
static uint g_ray_mask = RAY_SHADOW;

// The world a shadow ray from a point `relative` to the camera meets (D-075 step 3): RAY_SHADOW's
// near (a tree's leaf cards), RAY_SHADOW_FAR's from `Frame::far_shadows` metres on (its opaque
// proxy in their stead, a ray through it no candidate to test), the switch dithered by the pixel
// over the next quarter of that distance, which the shadows' denoiser averages.
uint shadow_world(Frame* f, float3 relative, uint2 pixel)
{
    float start = f[0].far_shadows;
    if (start <= 0.0)
        return RAY_SHADOW;
    float t = saturate((length(relative) - start) / (0.25 * start));
    return interleaved_gradient_noise(pixel, f[0].noise_frame, float2(17.0, 29.0)) < t ? RAY_SHADOW_FAR : RAY_SHADOW;
}

// The movers' structure the rays meet, 0 for none (`g_trace_movers`).
uint64_t movers_tlas(Frame* f)
{
    return g_trace_movers ? f[0].tlas_movers : 0;
}

static const uint REGION_ANY = 0xFFFFFFFEu; // `lamps::REGION_ANY`: the lamps that light every region

// The run of the light lists' entries under `key` (`lamps::lists`): [x, y).
uint2 lamp_run(Frame* f, uint key)
{
    uint lo = 0u;
    uint hi = f[0].lamp_list_count;
    while (lo < hi)
    {
        uint mid = (lo + hi) >> 1;
        if (f[0].lamp_lists[mid].x < key)
            lo = mid + 1u;
        else
            hi = mid;
    }
    uint end = lo;
    while (end < f[0].lamp_list_count && f[0].lamp_lists[end].x == key)
        end++;
    return uint2(lo, end);
}

// The lamps a surface in region `region` (across a doorway also `other`) takes: up to three runs
// of the light lists, the lamps of every region's, `region`'s and `other`'s (`lamp_runs`); without
// a region (REGION_NONE) or lists, every lamp of the frame, `direct`. A lamp under both `region`
// and `other` counts under `region` only (`lamp_counted`).
struct LampRuns
{
    uint2 run[3];
    bool direct;
    uint region;
};

LampRuns lamp_runs(Frame* f, uint region, uint other)
{
    LampRuns r;
    r.region = region;
    r.direct = region == REGION_NONE || f[0].lamp_lists == nullptr;
    r.run[0] = r.direct ? uint2(0u, f[0].lamp_count) : lamp_run(f, REGION_ANY);
    r.run[1] = r.direct ? uint2(0u, 0u) : lamp_run(f, region);
    r.run[2] = r.direct || other == region ? uint2(0u, 0u) : lamp_run(f, other);
    return r;
}

// The frame's index of the lamp at entry `k` of run `run`.
uint lamp_at(Frame* f, LampRuns r, uint k)
{
    return r.direct ? k : f[0].lamp_lists[k].y;
}

// Whether the lamp at entry `k` of run `run` counts there: not again under `other`.
bool lamp_counted(Frame* f, LampRuns r, uint run, uint i)
{
    return run != 2u || !lamp_lights(f[0].lamps[i], r.region, r.region);
}

// `lamp_weight` for a surface in region `region` (or `other`): a lamp with regions (a house's)
// lights its storey, from 2.7 m under it to 0.8 m over it. The hall is one region through every
// storey: its ground floor took the bedrooms' lamps and the hall's lights upstairs, their rays
// all blocked by the floors, and they took the three places from the lamp at hand, which fell to
// the picked rest in streaks. A street lamp (the outside's region first) lights the whole street.
float lamp_weight_in(Lamp lamp, float3 world, float3 n, uint region, uint other, float near = 0.0)
{
    if (lamp_storeyed(lamp) && region != REGION_NONE)
    {
        float dy = world.y - lamp.position_radius.y;
        if (dy < -2.7 || dy > 0.8)
            return 0.0;
    }
    return lamp_weight(lamp, world, n, near);
}

// Whether `lamp` lights its storey only (`lamp_weight_in`): a house's, with rooms for regions; not
// one without regions, nor a street lamp (`lamp_outside`).
bool lamp_storeyed(Lamp lamp)
{
    return lamp.regions_a.x != REGION_NONE && !lamp_outside(lamp);
}

// Whether `lamp` is a street lamp: the outside's region its first (city-plan's `street_lamps`).
bool lamp_outside(Lamp lamp)
{
    return lamp.regions_a.x == REGION_OUTSIDE;
}

// The share of `lamp`'s light that goes towards a surface the unit direction `to_lamp` points from
// to it: its emission (#288, `Emission` in lamps.rs). Every way (a zero axis): all of it. A cone:
// all of it within the cosine `beam.w` of its axis, none beyond `beam_edge.x`, a smooth step
// between; a street lamp's head is the cone down at 0.5 and 0.087, full cut-off (lit every way,
// its bulb lit the facade over it round its housing's shadow, the owner, 2026-10-18).
float lamp_beam(Lamp lamp, float3 to_lamp)
{
    if (all(lamp.beam.xyz == 0.0))
        return 1.0;
    return smoothstep(lamp.beam_edge.x, lamp.beam.w, dot(-to_lamp, lamp.beam.xyz));
}

// The least distance a probe ray's hit takes a lamp's light from, metres (#294). The surfaces inside
// a fitting and right by its bulb (a housing's inside, the ceiling over a globe) are lit by
// tens of thousands of lux, and the probes spread that over the ceiling round the fitting in a
// halo; the resolve lights them directly, each pixel its own.
static const float PROBE_LAMP_NEAR = 0.25;

// `hit_lamps`' weight of `lamp` at `hit`: `lamp_weight_in`; for a probe's ray none of a street
// lamp's, and none nearer than PROBE_LAMP_NEAR. The probes' blend spread a street lamp's bounce
// over the grass and the path in sharp blocks of their grid, away from the lamp (the owner,
// 2026-10-18; #254); its direct light and its shadows are the resolve's.
float hit_lamp_weight(Lamp lamp, RayHit hit, uint region, bool probe)
{
    if (probe && lamp_outside(lamp))
        return 0.0;
    return lamp_weight_in(lamp, hit.position, hit.n, region, region, probe ? PROBE_LAMP_NEAR : 0.0);
}

// Mirrors `GpuLamp` in crates/forge-render/src/lamps.rs (D-065): a small sphere light.
public struct Lamp
{
    public float4 position_radius; // xyz: its middle in the scene's frame; w: its radius, metres
    public float4 colour_reach;    // rgb: its intensity per channel, candelas; w: its reach, metres
    public uint4 regions_a;        // the probes' regions it lights (D-068), REGION_NONE in the slots left;
    public uint4 regions_b;        // all REGION_NONE: every surface (`lamp_lights`)
    public float4 beam;            // xyz: its cone's axis, the way its light goes (zero: every way); w:
                                   // the cosine within which its light is whole (`lamp_beam`, #288)
    public float4 beam_edge;       // x: the cosine beyond which a cone's light is none; yzw unused
};

// Whether `lamp` lights a surface in region `region` (or, across a doorway, `other`): a lamp
// lights its room's region and those its doorways open into (lamps.rs, `Lamp::regions`). Every
// surface where the lamp has no regions or the surface none (REGION_NONE: no regions known, a
// house's body).
bool lamp_lights(Lamp lamp, uint region, uint other)
{
    if (lamp.regions_a.x == REGION_NONE || region == REGION_NONE)
        return true;
    return any(lamp.regions_a == region) || any(lamp.regions_b == region)
        || any(lamp.regions_a == other) || any(lamp.regions_b == other);
}

// Mirrors `SkinVertex` in crates/forge-geom/src/skin.rs (32 bytes): a skinned cluster vertex
// in the bind pose.
public struct SkinVertex
{
    public float3 position;
    public uint normal;   // octahedral (`decode_normal`)
    public uint2 joints;
    public uint2 weights;
};

// The second half of `Frame::indirect` (issue #38): the pass 2 cluster cull's grid over both
// segments of the work lists (x, y, 1, items), segment 2's work items and roots (the work of
// the instances instance cull 2 let through, after pass 1's), then instance cull 2's ticket.
static const uint INDIRECT_PASS2 = 8;
// `Frame::indirect` word 7 (issue #92): how many clusters pass 1 left to pass 2 in
// `Frame::rejects`, or REJECTS_OVERFLOW when they did not all fit: pass 2 then walks segment 1's
// work items again, as pass 1 did.
static const uint INDIRECT_REJECTS = 7;
static const uint REJECTS_OVERFLOW = 0xFFFFFFFFu;
// `Frame::deferred`: instance cull 2's grid, then how many instances instance cull 1 deferred
// one by one and how many cells cell cull 2 opened; from DEFERRED_LIST the instances, then the
// cells (at most one entry per instance in all: a deferred instance is in no deferred cell).
static const uint DEFERRED_SINGLES = 4;
static const uint DEFERRED_OPENED = 5;
static const uint DEFERRED_LIST = 8;
// `Frame::cell_list` (issue #38): cell cull 2's grid, the cell culls' tickets, then the lists.
static const uint CELL_GRID2 = 4;
static const uint CELL_TICKET = 8;
static const uint CELL_TICKET2 = 9;
static const uint CELL_LIST = 12;
// Tags the work of an instance both pyramids hid, listed only for FLAG_SHOW_CULLED.
static const uint INSTANCE_HIDDEN_BIT = 0x80000000u;

struct Push
{
    Frame* frame;
    uint64_t* vis64;   // the software rasteriser's samples, depth << 32 | id per pixel (vis64.slang)
    uint target_width; // = frame.target_width, without a load per pixel
    uint depth_image;  // software raster: sampled-image indices of the hardware's depth and
    uint vis_image;    // visibility buffer of this pass
    uint pad;
};

[[vk::push_constant]]
Push push;

// ------------------------------------------------------------------ helpers ---

uint load_byte(uint* words, uint byte_index)
{
    return (words[byte_index >> 2] >> ((byte_index & 3) * 8)) & 0xFF;
}

// Where cluster `m`'s payload lies in the pool, in bytes: its page's slot, then its offset.
uint payload_base(Frame* f, Meshlet m)
{
    return f[0].page_table[m.page] * PAGE_SIZE + m.payload;
}

// A packed payload (#218, forge_geom::page::Layout), marked in its cluster's `section` word: a
// 16-byte header (the cluster's origin on its mesh's grid, three ints, then the widths and the
// grid's exponent), a 32-bit normal per vertex, then each position's offset from the origin,
// bit-packed. A cluster without the mark holds `PagedVertex` records (the skinned meshes').
static const uint SECTION_PACKED = 1u << 24;
static const uint PACKED_HEADER_WORDS = 4;

// The `width` bits (at most 31) at bit `bit` of the words from `start` on.
uint packed_bits(uint* words, uint start, uint bit, uint width)
{
    uint w = start + (bit >> 5);
    uint s = bit & 31u;
    uint value = words[w] >> s;
    if (s != 0u)
        value |= words[w + 1] << (32u - s);
    return value & ((1u << width) - 1u);
}

// Vertex `i` of cluster `m`, whose payload is at `base`: its position decoded from the grid
// exactly as the cook snapped it (an integer times a power of two).
PagedVertex paged_vertex(Frame* f, uint base, Meshlet m, uint i)
{
    if ((m.section & SECTION_PACKED) == 0u)
        return ((PagedVertex*)f[0].pool)[base / PAGED_VERTEX_BYTES + i];
    uint* words = f[0].pool;
    uint h = base / 4;
    uint info = words[h + 3];
    uint3 bits = uint3(info & 31u, (info >> 5) & 31u, (info >> 10) & 31u);
    float step = asfloat((((info >> 16) & 0xFFu) - 1u) << 23); // 2^(exponent), biased by 128
    uint start = h + PACKED_HEADER_WORDS + m.vertex_count;
    uint bit = i * (bits.x + bits.y + bits.z);
    uint3 offset = uint3(packed_bits(words, start, bit, bits.x),
                         packed_bits(words, start, bit + bits.x, bits.y),
                         packed_bits(words, start, bit + bits.x + bits.y, bits.z));
    int3 grid = int3(asint(words[h]), asint(words[h + 1]), asint(words[h + 2])) + int3(offset);
    PagedVertex v;
    v.position = float3(grid) * step;
    v.normal = words[h + PACKED_HEADER_WORDS + i];
    return v;
}

// Where the triangles of cluster `m`, whose payload is at `base`, start in the pool, in bytes.
uint paged_triangles(Frame* f, uint base, Meshlet m)
{
    if ((m.section & SECTION_PACKED) == 0u)
        return base + m.vertex_count * PAGED_VERTEX_BYTES;
    uint info = f[0].pool[base / 4 + 3];
    uint stride = (info & 31u) + ((info >> 5) & 31u) + ((info >> 10) & 31u);
    // The positions in whole words, and one more (forge_geom::page::Packed::position_bytes).
    return base + PACKED_HEADER_WORDS * 4 + m.vertex_count * 4 + ((m.vertex_count * stride + 31u) / 32u) * 4 + 4;
}

// Local vertex index `k` (0..2) of triangle `t` of cluster `m`, whose payload is at `base`.
uint paged_index(Frame* f, uint base, Meshlet m, uint t, uint k)
{
    return load_byte(f[0].pool, paged_triangles(f, base, m) + t * 3 + k);
}

// Where the UV stream of cluster `m`, whose payload is at `base`, starts (a mesh with UVs,
// D-047; forge_geom::page::Layout::uvs): after its vertices and triangles, 16-byte aligned. It
// holds the cluster's UV minimum and extent, then a 16-bit unorm pair per vertex.
uint uv_stream(Frame* f, uint base, Meshlet m)
{
    return (paged_triangles(f, base, m) + m.triangle_count * 3 + 15) & ~15u;
}

// The UV of vertex `i` in the stream at `stream`.
float2 paged_uv(Frame* f, uint stream, uint i)
{
    float4 range = ((float4*)f[0].pool)[stream / 16];
    uint e = f[0].pool[(stream + 16) / 4 + i];
    return range.xy + float2(float(e & 0xFFFFu), float(e >> 16)) * (1.0 / 65535.0) * range.zw;
}

// The material section of triangle `t` of cluster `m`: before the split its first section,
// after it its second (forge_geom::meshlet::triangle_section).
uint triangle_section(Meshlet m, uint t)
{
    return t < ((m.section >> 16) & 0xFFu) ? (m.section & 0xFFu) : ((m.section >> 8) & 0xFFu);
}

// What of MATERIAL_FLAG_MASKED, MATERIAL_FLAG_DOUBLE_SIDED and MATERIAL_FLAG_DRAWN_AFTER the rows
// of cluster `m` of instance `inst` set (#171): its two sections' (the second only when some
// triangle takes it). Such a cluster takes the cut-outs' raster, never the software one, and is
// not cone-culled.
uint cluster_cutouts(Frame* f, Instance inst, Meshlet m)
{
    if ((f[0].flags & FLAG_CUTOUTS) == 0)
        return 0u;
    const uint cut = MATERIAL_FLAG_MASKED | MATERIAL_FLAG_DOUBLE_SIDED | MATERIAL_FLAG_DRAWN_AFTER;
    uint flags = f[0].materials[inst.material + (m.section & 0xFFu)].flags & cut;
    if (((m.section >> 16) & 0xFFu) < m.triangle_count)
        flags |= f[0].materials[inst.material + ((m.section >> 8) & 0xFFu)].flags & cut;
    // The windowed ground's clusters that reach into the ground window (#197): their fragments
    // there go (`cutout_kept`). Its instances stand in the scene's frame, so the cluster's
    // sphere is where it stands.
    float4 w = f[0].ground_window;
    if ((f[0].meshes[inst.mesh].flags & MESH_WINDOWED_GROUND) != 0 && w.x <= w.z
        && all(m.center.xz + m.radius >= w.xy) && all(m.center.xz - m.radius <= w.zw))
        flags |= CUTOUT_GROUND_WINDOW;
    return flags;
}

// Whether `object` (x, z), of an instance of `mesh`, lies in the frame's ground window where the
// mesh is windowed ground (#197): its fragment there goes, the window's mesh drawn in its place.
bool in_ground_window(Frame* f, uint mesh, float2 object)
{
    float4 w = f[0].ground_window;
    return (f[0].meshes[mesh].flags & MESH_WINDOWED_GROUND) != 0
        && all(object >= w.xy) && all(object < w.zw);
}

// The 16-bit octahedral normal of a paged vertex (forge_geom::page::decode_normal).
float3 decode_normal(uint e)
{
    float2 o = clamp(float2(float(int(e << 16) >> 16), float(int(e) >> 16)) / 32767.0, -1.0, 1.0);
    float3 n = float3(o, 1.0 - abs(o.x) - abs(o.y));
    float t = max(-n.z, 0.0);
    n.x += n.x >= 0.0 ? -t : t;
    n.y += n.y >= 0.0 ? -t : t;
    return normalize(n);
}

// ------------------------------------------------- positions in cells (issue #93) ---

// The rotation of a unit quaternion (xyz w) as a matrix, for `mul(m, v)`.
float3x3 quat_matrix(float4 q)
{
    float x = q.x, y = q.y, z = q.z, w = q.w;
    return float3x3(
        float3(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)),
        float3(2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)),
        float3(2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)));
}

// The Hamilton product a ⊗ b: the rotation b, then a.
float4 quat_mul(float4 a, float4 b)
{
    return float4(a.w * b.xyz + b.w * a.xyz + cross(a.xyz, b.xyz), a.w * b.w - dot(a.xyz, b.xyz));
}

// The instance's rotation and scale: object-space directions and offsets to camera-relative.
float3x3 instance_frame(Instance inst)
{
    return quat_matrix(inst.rotation) * inst.scale;
}

// A point `cells` cells and `local` metres from the culling camera, in metres: the cell
// difference in integers (exact), times the cell size (exact), plus the small offsets.
float3 cells_relative(Frame* f, int3 cell, float3 local)
{
    return float3(cell - f[0].camera_cell.xyz) * CELL_SIZE + (local - f[0].camera_local);
}

// The instance's origin relative to the camera.
float3 instance_relative(Frame* f, Instance inst)
{
    return cells_relative(f, inst.cell, inst.local);
}

// The instance's bounding sphere's centre relative to the camera.
float3 instance_center_relative(Frame* f, Instance inst)
{
    return cells_relative(f, inst.cell, inst.center);
}

// A cell's bounding sphere's centre relative to the camera.
float3 cell_center_relative(Frame* f, CellBounds b)
{
    return cells_relative(f, b.cell, b.center);
}

// ------------------------------------------------------------ the ladder (D-075) ---
//
// A mesh can name the next mesh down its ladder (`Mesh::ladder_next`, #302): a tree's leaf cards
// near, its crown's closed shells farther. Each frame the instance culls measure every instance
// of such a mesh (`ladder_crossing`): above the band it draws its mesh, below it the next, and
// inside it both, its clusters in the cut-outs' raster (`CUTOUT_LADDER`), where each pixel keeps
// one of the two by the frame's spatio-temporal noise against the crossing (`cutout_kept`):
// complementary, so no hole and no double, and the frames' noise averages the two (DLAA's
// history), as the gratings' bars. The crossing is written per instance (`Frame::ladder`) and a
// work item or root entry of the next mesh carries INSTANCE_LADDER_BIT.

// With the instance in a work item or a root entry: its clusters are of the next mesh on its
// mesh's ladder. INSTANCE_HIDDEN_BIT (instance cull 2's hidden instances) is the other tag.
static const uint INSTANCE_LADDER_BIT = 0x40000000u;
static const uint INSTANCE_TAG_BITS = 0xC0000000u;

// The most crossings a ladder is followed down (#332): a tree's cards, its shells or a stand-in,
// a coarser step, and a last.
static const uint LADDER_STEPS = 3;

// An instance's crossing this frame, as the instance culls write it (`Frame::ladder`): `2 k + t`,
// crossing `t` (0 to 1) from the mesh `k` steps down its ladder to the next. Twice the step, so
// the end of one crossing (t = 1) is not the start of the next. Where the next mesh's fade has
// begun as well (`ladder_fade_after`, #363), its share in LADDER_FADE_STEPS-ths is added times
// LADDER_FADE_UNIT, measured once by the cull for the pixels to read.
static const float LADDER_FADE_UNIT = 8.0;
static const float LADDER_FADE_STEPS = 1023.0;

float ladder_rest(float value)
{
    return value - LADDER_FADE_UNIT * floor(value / LADDER_FADE_UNIT);
}

uint ladder_pair(float value)
{
    return uint(ladder_rest(value) * 0.5);
}

float ladder_t(float value)
{
    float rest = ladder_rest(value);
    return rest - 2.0 * float(uint(rest * 0.5));
}

bool ladder_fading(float value)
{
    return value >= LADDER_FADE_UNIT;
}

float ladder_fade_share(float value)
{
    return floor(value / LADDER_FADE_UNIT) / LADDER_FADE_STEPS;
}

// The mesh `steps` steps down `mesh`'s ladder.
uint ladder_step(Frame* f, uint mesh, uint steps)
{
    for (uint k = 0; k < steps && k < LADDER_STEPS; k++)
    {
        uint next = f[0].meshes[mesh].ladder_next;
        if (next == LADDER_NONE || next == LADDER_VANISH)
            break;
        mesh = next;
    }
    return mesh;
}

// The first mesh of the pair instance `instance` crosses this frame: its own mesh but on a
// ladder followed past its first step.
uint ladder_first(Frame* f, uint instance, Instance inst)
{
    if (f[0].ladder == nullptr || f[0].meshes[inst.mesh].ladder_next == LADDER_NONE)
        return inst.mesh;
    return ladder_step(f, inst.mesh, ladder_pair(f[0].ladder[instance]));
}

// The mesh instance `instance`'s work item or root entry tagged `tag` draws: the first of the
// pair it crosses, or with INSTANCE_LADDER_BIT the next.
uint drawn_mesh(Frame* f, uint instance, Instance inst, uint tag)
{
    uint first = ladder_first(f, instance, inst);
    return (tag & INSTANCE_LADDER_BIT) != 0 ? f[0].meshes[first].ladder_next : first;
}

// A value in [0, 1) from `n`, the same for the same `n`.
float hash_unit(uint n)
{
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return float(n >> 8) * (1.0 / 16777216.0);
}

// The projected size of `inst`'s sphere, output pixels across (the render's over
// `texture_lod_scale`), as its ladder measures it. Each instance's size is taken up to 15 % off by
// its id's hash, so a wood crosses tree by tree, not along a circle round the camera.
float ladder_size(Frame* f, Instance inst)
{
    float dist = max(length(instance_center_relative(f, inst)), f[0].znear);
    float k = f[0].p11 * 0.5 * f[0].viewport_height / f[0].texture_lod_scale;
    return 2.0 * inst.radius * k / dist * (0.85 + 0.3 * hash_unit(inst.id));
}

// How far an instance `px` across has crossed from `mesh` to its next: `ladder_px` against the
// size, linear over the band.
float ladder_t_at(Mesh* mesh, float px)
{
    float band = max(mesh[0].ladder_band, 1e-3);
    return saturate((1.0 + band - px / mesh[0].ladder_px) / (2.0 * band));
}

// How far `inst` has crossed to the next mesh on its mesh's ladder this frame: 0 its mesh alone
// (or no ladder), 1 the next alone, between both, by its size on screen (`ladder_size`) against
// the mesh's `ladder_px`.
//
// The ladder is followed down (#332): past its crossing, an instance whose next mesh has a
// ladder of its own measures it against that one's size and band, up to LADDER_STEPS crossings.
// It returns `2 k + t` (`ladder_pair`, `ladder_t`).
float ladder_crossing(Frame* f, Instance inst)
{
    uint m = inst.mesh;
    if (f[0].ladder == nullptr || f[0].meshes[m].ladder_next == LADDER_NONE)
        return 0.0;
    float px = ladder_size(f, inst);
    for (uint step = 0; step < LADDER_STEPS; step++)
    {
        Mesh* mesh = f[0].meshes + m;
        float t = ladder_t_at(mesh, px);
        uint next = mesh[0].ladder_next;
        bool on = t >= 1.0 && next != LADDER_VANISH && step + 1 < LADDER_STEPS
            && f[0].meshes[next].ladder_next != LADDER_NONE;
        if (!on)
            return 2.0 * float(step) + t;
        m = next;
    }
    return 0.0;
}

// While `inst` crosses (`value`, `ladder_crossing`) to a mesh that fades in turn (`set_fade`: a
// plant's shells into its impostor, D-075 step 4), how far that fade has gone, on its own band.
// The two bands overlap where the crossing ends under 1.67 times the fade's size (the village's
// trees, their lumps few across): taken one after the other, the fade would jump at the
// crossing's end to where it stood by then (#363). 0 where the next mesh does not fade.
float ladder_fade_after(Frame* f, Instance inst, float value)
{
    if (f[0].ladder == nullptr)
        return 0.0;
    uint next = f[0].meshes[ladder_step(f, inst.mesh, ladder_pair(value))].ladder_next;
    if (next == LADDER_NONE || next == LADDER_VANISH || f[0].meshes[next].ladder_next != LADDER_VANISH)
        return 0.0;
    return ladder_t_at(f[0].meshes + next, ladder_size(f, inst));
}

// Whether the clusters of instance `instance` are crossing this frame (written by its cull): its
// ladder, or the fade after it.
bool ladder_crossing_now(Frame* f, uint instance, Instance inst)
{
    if (f[0].ladder == nullptr || f[0].meshes[inst.mesh].ladder_next == LADDER_NONE)
        return false;
    float value = f[0].ladder[instance];
    float t = ladder_t(value);
    return (t > 0.0 && t < 1.0) || ladder_fading(value);
}

// An instance's rotation and scale and its origin relative to the camera, built once where
// several of its points are needed (the cluster cull takes up to five per cluster). Rebuilt
// at every point, the quaternion's matrix and the camera's cell and offset made the streamed
// city's cull 1 a third slower under async compute (#93: 0.35 → 0.48 ms, 0.32 once built once).
struct InstancePose
{
    float3x3 frame;
    float3 origin;
};

InstancePose instance_pose(Frame* f, Instance inst)
{
    InstancePose pose;
    pose.frame = instance_frame(inst);
    pose.origin = instance_relative(f, inst);
    return pose;
}

// An object-space point of a posed instance, relative to the camera.
float3 pose_point(InstancePose pose, float3 p)
{
    return mul(pose.frame, p) + pose.origin;
}

// An object-space point of the instance, relative to the camera (the old `mul(model, p)`).
float3 instance_point(Frame* f, Instance inst, float3 p)
{
    return pose_point(instance_pose(f, inst), p);
}

// An object-space vector of the instance, turned and scaled (the old `mul(model, float4(v, 0))`).
float3 instance_vector(Instance inst, float3 v)
{
    return mul(instance_frame(inst), v);
}

// A camera-relative position in the scene frame: where the rays and the probes live.
float3 to_scene(Frame* f, float3 relative)
{
    return relative + f[0].camera_in_scene.xyz;
}

bool sphere_in_frustum(Frame* f, float3 center, float radius)
{
    [unroll]
    for (int i = 0; i < 6; i++)
    {
        if (dot(f[0].planes[i].xyz, center) + f[0].planes[i].w < -radius)
            return false;
    }
    return true;
}

// Screen-space bounds of a view-space sphere (Mara & McGuire 2013, as in zeux/niagara).
// `c` is view space with the camera looking down -z; returns false when the sphere crosses
// the near plane (treated as visible).
bool project_sphere(float3 c, float r, float znear, float p00, float p11, out float4 aabb)
{
    aabb = 0;
    float cz = -c.z;
    if (cz < r + znear)
        return false;
    float2 cx = float2(c.x, cz);
    float2 vx = float2(sqrt(dot(cx, cx) - r * r), r);
    float2 minx = float2(vx.x * cx.x - vx.y * cx.y, vx.y * cx.x + vx.x * cx.y);
    float2 maxx = float2(vx.x * cx.x + vx.y * cx.y, -vx.y * cx.x + vx.x * cx.y);
    float2 cy = float2(c.y, cz);
    float2 vy = float2(sqrt(dot(cy, cy) - r * r), r);
    float2 miny = float2(vy.x * cy.x - vy.y * cy.y, vy.y * cy.x + vy.x * cy.y);
    float2 maxy = float2(vy.x * cy.x + vy.y * cy.y, -vy.y * cy.x + vy.x * cy.y);
    aabb = float4(minx.x / minx.y * p00, miny.x / miny.y * p11, maxx.x / maxx.y * p00, maxy.x / maxy.y * p11);
    // clip space -> uv space (y down in the image)
    aabb = aabb.xwzy * float4(0.5, -0.5, 0.5, -0.5) + 0.5;
    return true;
}

// True when a rectangle (uv, the sphere's projection) whose nearest point is at depth
// `nearest` may be visible against the depth pyramid `hzb_image`.
bool pyramid_visible(Frame* f, uint hzb_image, float4 aabb, float nearest)
{
    float width = (aabb.z - aabb.x) * f[0].hzb_size.x;
    float height = (aabb.w - aabb.y) * f[0].hzb_size.y;
    // Level where a texel is at most the rectangle's size (T <= w < 2T). One linear
    // min-reduction tap covers 2x2 texels but only a texel-wide window around the sample
    // point is guaranteed to lie inside them, so one tap at the centre is not conservative
    // for w > T. Four taps at the corners are: a rectangle narrower than 2T spans at most
    // three texels per axis, and the two corner footprints always include the middle one
    // (the gap would need w >= 2T). This is the tightest exact test with a 2x2 sampler.
    float level = floor(log2(max(max(width, height), 1.0)));
    float farthest = min(
        min(sample_depth_level(hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.xy, level),
            sample_depth_level(hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.zy, level)),
        min(sample_depth_level(hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.xw, level),
            sample_depth_level(hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.zw, level)));
    return nearest >= farthest;
}

// True when the sphere may be visible against this frame's depth pyramid (built after pass 1).
bool occlusion_visible(Frame* f, float3 world_center, float radius)
{
    float3 c = mul(f[0].cull_view, float4(world_center, 1)).xyz;
    float4 aabb;
    if (!project_sphere(c, radius, f[0].znear, f[0].p00, f[0].p11, aabb))
        return true;
    // The depth was drawn with a jittered projection: test where the sphere is in *that* image.
    aabb += f[0].draw_jitter.xyxy;
    // Reversed-Z, infinite far plane: depth = znear / distance. Nearest point of the sphere:
    return pyramid_visible(f, f[0].hzb_image, aabb, f[0].znear / (-c.z - radius));
}

// Whether the previous frame showed the sphere: its pyramid (built after that frame's first
// pass), seen from that frame's culling camera, does not hide it. A sphere entirely outside
// that image counts as not shown, one crossing the near plane as shown, one partly outside is
// tested where it was (the clamped samples, as in `occlusion_visible`). Pass 1 draws what
// this accepts and lists the rest for pass 2; when that list overflows, pass 2 asks the same
// question on the same inputs to skip what pass 1 drew (issue #92). The answer
// only decides which pass draws a cluster, never whether: what pass 1 skips, pass 2 tests
// against a pyramid of depth that is all in the final image, so a visible cluster always
// passes there.
bool visible_last_frame(Frame* f, float3 world_center, float radius)
{
    if ((f[0].flags & FLAG_PREV_PYRAMID) == 0)
        return false;
    float3 c = mul(f[0].prev_cull_view, float4(world_center, 1)).xyz;
    float4 aabb;
    if (!project_sphere(c, radius, f[0].znear, f[0].prev_p00, f[0].prev_p11, aabb))
        return true;
    aabb += f[0].prev_draw_jitter.xyxy;
    if (any(aabb.zw < 0.0) || any(aabb.xy > 1.0))
        return false;
    return pyramid_visible(f, f[0].prev_hzb_image, aabb, f[0].znear / (-c.z - radius));
}

// Projected size in pixels of an object-space error `error` carried by a world-space sphere,
// measured at the sphere's nearest point (conservative): a length L at view distance d spans
// L * p11 / d in NDC, i.e. half that times the viewport height in pixels.
float lod_error_pixels(Frame* f, float3 center, float radius, float error)
{
    float d = max(length(center) - radius, f[0].znear);
    return error * f[0].p11 * 0.5 * f[0].viewport_height / d;
}

// Exactly one cut through the DAG: draw when the parent is too coarse and this is fine
// enough. Siblings share the parent values and the children of a group share its values as
// their `self`, so the decision is consistent and watertight.
// `streamed` is a compile-time constant of each cluster-cull entry point (`cluster_cull`):
// the streamed cut's code and registers stay out of a resident scene's cull.
bool lod_selected(Frame* f, Meshlet m, InstancePose pose, float scale, bool streamed)
{
    if ((f[0].flags & FLAG_LOD) == 0)
    {
        // Full detail; a streamed scene shows what of it is resident (a debug view).
        bool resident = !streamed || f[0].page_table[m.page] != PAGE_NONE;
        return m.lod_level == 0 && resident;
    }
    float3 self_center = pose_point(pose, m.self_center);
    float self_px = lod_error_pixels(f, self_center, m.self_radius * scale, m.self_error * scale);
    if (streamed)
        return lod_selected_streamed(f, m, pose, scale, self_px);
    if (self_px > f[0].lod_threshold)
        return false;
    if (isinf(m.parent_error))
        return true;
    float3 parent_center = pose_point(pose, m.parent_center);
    float parent_px = lod_error_pixels(f, parent_center, m.parent_radius * scale, m.parent_error * scale);
    return parent_px > f[0].lod_threshold;
}

// Records that `page` is wanted: `px` pixels of error would appear without it.
void want_page(Frame* f, uint page, float px)
{
    uint unused;
    InterlockedMax(f[0].page_need[page], asuint(max(px, 0.0)), unused);
}

// The cut through the resident part of the DAG (issue #36, crates/forge-render/src/
// streaming.rs): a cluster draws when its page is resident, its parent is too coarse, and it
// is fine enough or its children's page is absent. Children are refined into only when
// their page is resident, and that page holds every child of the group (a group never spans
// two pages), so the cut stays one watertight surface through any resident set that holds
// the roots. On the way it records which pages it wants: a drawn cluster its own page, a
// too-coarse one its children's.
bool lod_selected_streamed(Frame* f, Meshlet m, InstancePose pose, float scale, float self_px)
{
    if (f[0].page_table[m.page] == PAGE_NONE)
        return false;
    if (!isinf(m.parent_error))
    {
        float3 parent_center = pose_point(pose, m.parent_center);
        float parent_px = lod_error_pixels(f, parent_center, m.parent_radius * scale, m.parent_error * scale);
        if (parent_px <= f[0].lod_threshold)
            return false;
        want_page(f, m.page, parent_px);
    }
    if (self_px <= f[0].lod_threshold)
        return true;
    want_page(f, m.child_page, self_px);
    return f[0].page_table[m.child_page] == PAGE_NONE;
}

uint level_of(Mesh* mesh, uint cluster)
{
    for (uint l = 0; l < mesh[0].level_count; l++)
    {
        if (cluster < mesh[0].level_offset[l + 1])
            return l;
    }
    return max(mesh[0].level_count, 1u) - 1;
}

// Whether any cluster of work item `local_group` of `inst` can be selected: the group's 32
// clusters span one or two levels; a level is impossible when every one of its clusters
// projects its own error above the threshold even at the farthest distance, or its parent's
// error at or below the threshold even at the nearest. Conservative bounds from the per-level
// tables, so a group is never skipped wrongly; most groups of a far instance exit here.
bool group_possible(Frame* f, Mesh* mesh, Instance* inst, uint local_group, float scale, bool streamed)
{
    uint first = local_group * TASK_GROUP_SIZE;
    uint count = mesh[0].meshlet_count;
    if (first >= count)
        return false;
    if ((f[0].flags & FLAG_GROUP_WINDOW_OFF) != 0)
        return true;
    uint last = min(first + TASK_GROUP_SIZE - 1, count - 1);
    uint l0 = level_of(mesh, first);
    uint l1 = level_of(mesh, last);
    if ((f[0].flags & FLAG_LOD) == 0)
        return l0 == 0;
    float k = f[0].p11 * 0.5 * f[0].viewport_height;
    float dist = length(instance_center_relative(f, inst[0]));
    float t = f[0].lod_threshold;
    // Streaming draws a too-coarse cluster whose children are absent: only the parent bound
    // holds.
    for (uint l = l0; l <= l1; l++)
    {
        float self_far = dist + mesh[0].self_reach_max[l] * scale;
        float parent_near = max(dist - mesh[0].parent_reach_max[l] * scale, f[0].znear);
        float self_min_px = mesh[0].self_error_min[l] * scale * k / self_far;
        float parent_max_px = mesh[0].parent_error_max[l] * scale * k / parent_near;
        if ((self_min_px <= t || streamed) && parent_max_px > t)
            return true;
    }
    return false;
}

float3 lod_color(uint level)
{
    static const float3 COLORS[8] = {
        float3(0.70, 0.70, 0.70), float3(0.30, 0.85, 0.35), float3(0.95, 0.85, 0.25), float3(0.95, 0.55, 0.20),
        float3(0.90, 0.25, 0.25), float3(0.80, 0.30, 0.85), float3(0.30, 0.55, 0.95), float3(0.20, 0.85, 0.85),
    };
    return COLORS[min(level, 7u)];
}

float3 hash_color(uint n)
{
    n = n * 1664525u + 1013904223u;
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return float3(n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF) / 255.0;
}

// ------------------------------------------------------------- cluster cull ---

static const uint TASK_GROUP_SIZE = 32; // clusters per work item (an instance's group of 32)

// Indirect grids are two-dimensional so that a count above the per-dimension limit (65 535
// is all the spec guarantees for compute and mesh dispatches) still launches: x = min(count,
// GRID_ROW), y = ceil(count / GRID_ROW). The rest of the last row exits.
static const uint GRID_ROW = 32768;

uint grid_index(uint3 group)
{
    return group.y * GRID_ROW + group.x;
}

// Pass 2's work items over segment 1 (issue #92): the clusters pass 1 left to it, 32 to an
// item, or, when they did not fit in `Frame::rejects`, pass 1's own work items.
uint pass2_segment1_items(Frame* f)
{
    uint rejects = f[0].indirect[INDIRECT_REJECTS];
    return rejects == REJECTS_OVERFLOW ? f[0].indirect[3] : (rejects + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE;
}

// Clusters pass 1 listed (hardware, software and cut out), final once pass 2 culls.
uint pass1_listed(Frame* f)
{
    return f[0].clusters[3] + f[0].clusters[7] + f[0].clusters[15];
}

// The slot of the `index`-th cluster of this pass in the visible-cluster list: pass 2 follows
// pass 1, so the ids grow with the draw order across the passes.
uint visible_slot(Frame* f, uint index)
{
    return f[0].pass == PASS_REMAINDER ? pass1_listed(f) + index : index;
}

// Words of one pass's arguments in `Frame::clusters`, and where the culls' tickets follow both.
static const uint CLUSTER_PASS_WORDS = 16;
static const uint CLUSTER_TICKETS = 2 * CLUSTER_PASS_WORDS;

// This pass's hardware draw arguments (x, y, 1, count) in `Frame::clusters`; its software
// raster arguments follow (`pass_args(f) + 4`), then the merge's draw (`+ 8`), then the
// cut-outs' raster's (`+ 12`, #265). forge_render::meshlet::CLUSTER_ARGS_START.
uint* pass_args(Frame* f)
{
    return f[0].clusters + (f[0].pass == PASS_REMAINDER ? CLUSTER_PASS_WORDS : 0);
}

// What one thread decided about its cluster.
struct ClusterVerdict
{
    bool draw;
    bool occluded;     // rejected by the depth pyramid (counted)
    bool rejected;     // pass 1: selected and in view, but the previous pyramid hid it (counted)
    bool culled_debug; // FLAG_SHOW_CULLED: drawn although culling rejected it
    bool dense;        // drawn, and dense enough for the software rasteriser (`dense_cluster`)
    bool software;     // dense, and rasterised in compute
    uint instance;
    uint meshlet;      // global meshlet index
    uint triangle_count;
    uint vertex_count;
    uint payload;      // where its payload lies in the pool, in bytes (`payload_base`)
    uint lod_level;
    uint cutouts;      // its rows' MATERIAL_FLAG_MASKED and MATERIAL_FLAG_DOUBLE_SIDED (`cluster_cutouts`)
};

// Largest cluster the software rasteriser takes, in pixels across its bounding rectangle
// (forge_render::meshlet::SW_RASTER_MAX_PX): the rectangle bounds every triangle, which
// keeps the rasteriser's integer edge functions in range (`SW_RASTER_MAX_EXTENT`).
static const float SW_RASTER_MAX_PX = 64.0;

// A pass merges its software samples by rectangles (one per cluster) while it has at most one
// software cluster per this many pixels of the target, else with one full-screen triangle.
static const uint MERGE_RECT_PIXELS = 256;

// Whether a cluster is dense, the software rasteriser's kind: entirely in front of the near
// plane, under SW_RASTER_MAX_PX across, and fewer than `sw_raster_area` pixels of its
// bounding sphere's rectangle per triangle. There the hardware's fixed cost per primitive
// dominates and a thread per triangle wins (Nanite's split); a small cluster of few, larger
// triangles costs the hardware little and stays there.
bool dense_cluster(Frame* f, float3 world_center, float radius, uint triangles)
{
    float3 c = mul(f[0].cull_view, float4(world_center, 1)).xyz;
    float4 aabb;
    if (!project_sphere(c, radius, f[0].znear, f[0].p00, f[0].p11, aabb))
        return false;
    float2 size = abs(aabb.zw - aabb.xy) * float2(f[0].target_width, f[0].target_height);
    return max(size.x, size.y) < SW_RASTER_MAX_PX && size.x * size.y < f[0].sw_raster_area * float(triangles);
}

// The cluster's counts and where its payload lies, for the appends.
void describe_cluster(Frame* f, inout ClusterVerdict v, Meshlet m)
{
    v.triangle_count = m.triangle_count;
    v.vertex_count = m.vertex_count;
    v.payload = payload_base(f, m);
    v.lod_level = m.lod_level;
    Instance inst = f[0].instances[v.instance];
    v.cutouts = cluster_cutouts(f, inst, m);
    // Both meshes of an instance crossing its ladder take the cut-outs' raster, dithered (D-075).
    if (ladder_crossing_now(f, v.instance, inst))
        v.cutouts |= CUTOUT_LADDER;
}

// Pass 2's test of a cluster pass 1 did not draw: this frame's pyramid. With
// FLAG_SHOW_CULLED what it hides is drawn as culled.
void test_this_frame(Frame* f, inout ClusterVerdict v, float3 center, float radius, bool hidden_instance)
{
    v.draw = !hidden_instance && occlusion_visible(f, center, radius);
    v.occluded = !v.draw;
    if (v.occluded && (f[0].flags & FLAG_SHOW_CULLED) != 0)
    {
        v.culled_debug = true;
        v.draw = true;
    }
}

// Whether a drawn cluster is dense, and rasterised in compute.
void test_density(Frame* f, inout ClusterVerdict v, float3 center, float radius)
{
    v.dense = v.draw && dense_cluster(f, center, radius, v.triangle_count);
    // A cut-out cluster takes the hardware raster's cut-outs pipeline (#171).
    v.software = v.dense && (f[0].flags & FLAG_SW_RASTER) != 0 && v.cutouts == 0;
}

// Pass 2's cull of the `entry`-th cluster pass 1 left to it (issue #92): pass 1 selected it
// and found it in view, but the previous pyramid hid it, so only this frame's pyramid is left
// to ask. The entries keep pass 1's order, so pass 2 appends what it draws in the order its
// walk over pass 1's work items did.
ClusterVerdict cull_rejected(Frame* f, uint entry)
{
    ClusterVerdict v = {};
    if (entry >= f[0].indirect[INDIRECT_REJECTS])
        return v;
    uint2 r = f[0].rejects[entry];
    v.instance = r.x;
    v.meshlet = r.y & MESHLET_INDEX_MASK;
    v.culled_debug = (r.y & CULLED_BIT) != 0; // the normal cone rejected it (FLAG_SHOW_CULLED)
    Instance inst = f[0].instances[v.instance];
    Meshlet m = f[0].meshlets[v.meshlet];
    describe_cluster(f, v, m);
    float3 center = instance_point(f, inst, m.center);
    float world_radius = m.radius * inst.scale;
    test_this_frame(f, v, center, world_radius, false);
    test_density(f, v, center, world_radius);
    return v;
}

// Culls cluster `thread` of work item `item`: LOD selection, frustum and normal cone; with
// occlusion, pass 1 keeps what the previous frame showed and lists the rest in view for pass 2
// (`Frame::rejects`), which keeps what this frame's pyramid does not hide.
ClusterVerdict cull_cluster(Frame* f, uint item, uint thread, bool streamed)
{
    ClusterVerdict v = {};
    uint pass = f[0].pass;
    // The first items are an instance's group of 32 clusters each; the rest cover the root
    // list, 32 root clusters (of any instances) each. Pass 2 reads the clusters pass 1 left to
    // it instead (issue #92), or walks the same items again when they did not fit. It then
    // reads a second segment of both lists: the instances pass 1 deferred and instance cull 2
    // let through, none of whose clusters pass 1 saw (issue #38).
    uint regular = f[0].indirect[5];
    uint listed = f[0].indirect[6];
    uint2 base = 0;
    bool fresh = false;
    if (pass == PASS_REMAINDER)
    {
        uint items1 = pass2_segment1_items(f);
        if (item >= items1)
        {
            fresh = true;
            item -= items1;
            base = uint2(regular, listed);
            regular = f[0].indirect[INDIRECT_PASS2 + 4];
            listed = f[0].indirect[INDIRECT_PASS2 + 5];
        }
        else if (f[0].indirect[INDIRECT_REJECTS] != REJECTS_OVERFLOW)
        {
            return cull_rejected(f, item * TASK_GROUP_SIZE + thread);
        }
    }
    uint local_meshlet;
    uint local_group = 0;
    bool root = item >= regular;
    if (root)
    {
        uint entry = (item - regular) * TASK_GROUP_SIZE + thread;
        if (entry >= listed)
            return v;
        uint2 r = f[0].roots[base.y + entry]; // written by the instance culls: (instance, local cluster)
        v.instance = r.x;
        local_meshlet = r.y;
    }
    else
    {
        uint2 work = f[0].work[base.x + item]; // written by the instance culls: (instance, local group)
        v.instance = work.x;
        local_group = work.y;
        local_meshlet = local_group * TASK_GROUP_SIZE + thread;
    }
    bool hidden_instance = (v.instance & INSTANCE_HIDDEN_BIT) != 0;
    uint tag = v.instance & INSTANCE_TAG_BITS;
    v.instance &= ~INSTANCE_TAG_BITS;
    Instance* inst = f[0].instances + v.instance;
    Mesh* mesh = f[0].meshes + drawn_mesh(f, v.instance, inst[0], tag);
    bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
    bool show_culled = (f[0].flags & FLAG_SHOW_CULLED) != 0;
    bool instance_visible = !frustum || sphere_in_frustum(f, instance_center_relative(f, inst[0]), inst[0].radius);
    float scale = inst[0].scale;
    // Whole-group rejection from the per-level tables before any cluster is read.
    if (!instance_visible || (!root && !group_possible(f, mesh, inst, local_group, scale, streamed))
        || local_meshlet >= mesh[0].meshlet_count)
        return v;
    v.meshlet = mesh[0].meshlet_offset + local_meshlet;
    Meshlet m = f[0].meshlets[v.meshlet];
    describe_cluster(f, v, m);
    InstancePose pose = instance_pose(f, inst[0]);
    float3 center = pose_point(pose, m.center);
    float world_radius = m.radius * scale;
    bool visible = lod_selected(f, m, pose, scale, streamed);
    visible = visible && (!frustum || sphere_in_frustum(f, center, world_radius));
    // Normal-cone culling (meshoptimizer): culled when dot(normalize(apex - camera), axis) >=
    // cutoff. A cutoff of 1 marks a cone wider than a hemisphere; meshoptimizer then leaves
    // the axis at zero, whose normalisation is NaN and would (silently) cull the meshlet:
    // skip the test. The apex is camera-relative: the camera is the origin.
    // A double-sided cluster shows its back faces: no cone hides it (#171).
    if (visible && (f[0].flags & FLAG_CONE_CULLING) != 0 && m.cone_cutoff < 1.0
        && (v.cutouts & MATERIAL_FLAG_DOUBLE_SIDED) == 0)
    {
        float3 apex = pose_point(pose, m.cone_apex);
        float3 axis = normalize(mul(pose.frame, m.cone_axis));
        bool cone_visible = dot(normalize(apex), axis) < m.cone_cutoff;
        if (!cone_visible && show_culled)
            v.culled_debug = true;
        else
            visible = cone_visible;
    }

    if (pass == PASS_PREVIOUSLY_VISIBLE)
    {
        v.draw = visible && visible_last_frame(f, center, world_radius);
        v.rejected = visible && !v.draw;
    }
    else if (pass == PASS_REMAINDER)
    {
        // Pass 1 drew what the previous frame showed; the rest faces this frame's pyramid.
        // Segment 2's clusters were never pass 1's to draw.
        if (visible && (fresh || !visible_last_frame(f, center, world_radius)))
            test_this_frame(f, v, center, world_radius, hidden_instance);
    }
    else
    {
        v.draw = visible;
    }
    test_density(f, v, center, world_radius);
    return v;
}

// ------------------------------------------------------------ ordered appends ---

// Both culls append to lists in a fixed order, and the ids follow it: two triangles can meet
// a sample at exactly the same depth, and the larger id wins (the depth test,
// `GREATER_OR_EQUAL`, keeps the one drawn last, and the hardware draws in list order; the
// software rasteriser compares the ids), so a list filled by racing atomics would change the
// image from run to run (visibly through TAA's history). Each list is therefore filled in
// workgroup order with a single-pass prefix sum (decoupled look-back, Merrill &
// Garland 2016): a workgroup publishes how many entries it appends, adds up the counts of the
// workgroups before it and writes at that offset. Workgroups take their index from a ticket
// counter in the order they start, so every earlier index belongs to a workgroup already
// running and the wait always ends.
//
// A status word is 0 (nothing published yet), LOOKBACK_AGGREGATE | counts, or
// LOOKBACK_PREFIX | counts of this workgroup and all before it. It carries two counts of 31
// bits (a cluster cull counts its clusters and, among them, the software-rasterised ones),
// packed so that adding two words adds the counts.
static const uint64_t LOOKBACK_AGGREGATE = 1ull << 62;
static const uint64_t LOOKBACK_PREFIX = 2ull << 62;
static const uint64_t LOOKBACK_FLAGS = 3ull << 62;

uint64_t lookback_pack(uint2 counts)
{
    return uint64_t(counts.x) | (uint64_t(counts.y) << 31);
}

uint2 lookback_unpack(uint64_t word)
{
    return uint2(uint(word & 0x7FFFFFFFull), uint((word >> 31) & 0x7FFFFFFFull));
}

static const uint LOOKBACK_LANES = 32; // predecessors read per round and run
static const uint LOOKBACK_RUNS = 2;   // runs a workgroup looks back on at once

groupshared uint64_t s_window[LOOKBACK_RUNS * LOOKBACK_LANES];
groupshared uint64_t s_exclusive[LOOKBACK_RUNS]; // counts of the tickets before this one, summed so far
groupshared uint s_cursor[LOOKBACK_RUNS];         // tickets [0, s_cursor) not yet added
groupshared uint s_pending;                       // bit r: run r has not reached a published prefix

// Publishes `counts` (xy in `status0`, zw in `status1`) for workgroup `ticket` in each of the
// first `runs` runs (1 or 2), and returns the sums of the counts of the tickets before it in
// each (xy, zw). Every thread of the workgroup calls it with the same arguments (the wait is
// shared), at least LOOKBACK_LANES threads a run: each round, LOOKBACK_LANES threads of each
// run still looking back read one predecessor's status word each, nearest first, and the run's
// first thread adds them up to the first prefix or to the first predecessor that has published
// nothing yet, where the run's next round starts. Two runs publish together and look back in
// the same rounds: the second run's predecessors do not wait for their first look-back to end
// before they publish, as two calls one after the other made them (#333).
uint4 lookback_exclusive2(uint64_t* status0, uint64_t* status1, uint runs, uint ticket, uint4 counts, uint thread)
{
    uint run = thread / LOOKBACK_LANES;
    uint lane = thread % LOOKBACK_LANES;
    bool reader = run < runs;
    uint64_t* status = run == 0 ? status0 : status1;
    uint64_t count = lookback_pack(run == 0 ? counts.xy : counts.zw);
    uint64_t ignored;
    if (reader && lane == 0)
    {
        InterlockedExchange(status[ticket], (ticket == 0 ? LOOKBACK_PREFIX : LOOKBACK_AGGREGATE) | count, ignored);
        s_exclusive[run] = 0;
        s_cursor[run] = ticket;
    }
    if (thread == 0)
        s_pending = ticket == 0 ? 0u : (1u << runs) - 1u;
    GroupMemoryBarrierWithGroupSync();
    while (s_pending != 0)
    {
        bool looking = reader && ((s_pending >> run) & 1u) != 0;
        uint cursor = reader ? s_cursor[run] : 0u;
        if (looking)
        {
            uint64_t word = 0;
            if (lane < cursor)
                InterlockedOr(status[cursor - 1 - lane], 0ull, word); // an atomic read
            s_window[thread] = word;
        }
        GroupMemoryBarrierWithGroupSync();
        if (looking && lane == 0)
        {
            uint n = min(LOOKBACK_LANES, cursor);
            uint added = 0;
            for (; added < n; added++)
            {
                uint64_t word = s_window[thread + added];
                uint64_t flag = word & LOOKBACK_FLAGS;
                if (flag == 0)
                    break; // not published yet: the next round starts here
                s_exclusive[run] += word & ~LOOKBACK_FLAGS;
                if (flag == LOOKBACK_PREFIX)
                {
                    InterlockedAnd(s_pending, ~(1u << run));
                    break;
                }
            }
            s_cursor[run] = cursor - added;
        }
        GroupMemoryBarrierWithGroupSync();
    }
    if (reader && lane == 0 && ticket != 0)
        InterlockedExchange(status[ticket], LOOKBACK_PREFIX | (s_exclusive[run] + count), ignored);
    return uint4(lookback_unpack(s_exclusive[0]), runs > 1 ? lookback_unpack(s_exclusive[1]) : uint2(0, 0));
}

// `lookback_exclusive2` over one run.
uint2 lookback_exclusive(uint64_t* status, uint ticket, uint2 counts, uint thread)
{
    return lookback_exclusive2(status, status, 1, ticket, uint4(counts, 0, 0), thread).xy;
}

// Workgroups of the instance cull, one status word each in `Frame::lookback`.
uint instance_cull_groups(Frame* f)
{
    return max((f[0].instance_count + 63) / 64, 1u);
}

// This pass's cluster cull's status words (`cluster_groups` per pass).
uint64_t* cluster_status(Frame* f)
{
    return f[0].cluster_lookback + (f[0].pass == PASS_REMAINDER ? f[0].cluster_groups : 0);
}

// Writes the grid (x, y, 1) that covers `groups` workgroups, and the `count` they work on.
void write_grid(uint* args, uint groups, uint count)
{
    args[0] = min(groups, GRID_ROW);
    args[1] = (groups + GRID_ROW - 1) / GRID_ROW;
    args[3] = count;
}

// Work items per cluster-cull workgroup: fewer workgroups take fewer tickets and publish
// fewer status words (the ordered appends cost per workgroup, and at 1 px most work items of
// a frame draw nothing).
static const uint CULL_ITEMS = 8;

groupshared uint s_ticket;               // this workgroup's place in the order
groupshared uint s_drawn[CULL_ITEMS];    // per work item, a bit per lane: the cluster is drawn
groupshared uint s_software[CULL_ITEMS]; // among those, the ones rasterised in compute
groupshared uint s_triangles;
groupshared uint s_sw_triangles;
groupshared uint s_dense_triangles;
groupshared uint s_levels;
groupshared uint s_occluded;
groupshared uint s_rejected[CULL_ITEMS]; // pass 1: per work item, a bit per lane: the cluster is left to pass 2
groupshared uint s_cutout[CULL_ITEMS];   // per work item, a bit per lane: drawn by the cut-outs' raster (#265)
groupshared uint s_kept_software;      // where the list ends: software clusters still kept
groupshared uint s_kept_cutouts;       // where the list ends: cut-out clusters still kept

// One workgroup per CULL_ITEMS work items: cull their clusters and append the survivors to
// this pass's range of the visible-cluster list in item order, then lane order, and their
// slots to the pass's raster lists (hardware-drawn from the front, software-rasterised from
// the back, the cut-out clusters in a list of their own after them, #265; each in the same
// order); on the fallback path also one indexed draw per hardware-drawn survivor, the cut-out
// ones' after the others' capacity. The mesh shaders and the software rasteriser read the
// raster lists, the fallback's draws carry the slots. The grids are written by the last
// workgroup, or, when the list is full, by the one it ends in. A full list drops what does not
// fit and counts it, so no draw reads past the end.
// `streamed`: the scene streams its pages (`lod_selected`), a constant per entry point.
void cluster_cull(uint thread, bool streamed)
{
    Frame* f = push.frame;
    bool second = f[0].pass == PASS_REMAINDER;
    if (thread == 0)
    {
        InterlockedAdd(f[0].clusters[CLUSTER_TICKETS + (second ? 1 : 0)], 1, s_ticket);
        s_triangles = 0;
        s_sw_triangles = 0;
        s_dense_triangles = 0;
        s_levels = 0;
        s_occluded = 0;
        s_kept_software = 0;
        s_kept_cutouts = 0;
    }
    if (thread < CULL_ITEMS)
    {
        s_drawn[thread] = 0;
        s_software[thread] = 0;
        s_rejected[thread] = 0;
        s_cutout[thread] = 0;
    }
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint items = f[0].indirect[second ? INDIRECT_PASS2 + 3 : 3];
    if (ticket * CULL_ITEMS >= items)
        return; // the rest of the grid's last row: the whole workgroup leaves
    uint local = thread / TASK_GROUP_SIZE;
    uint lane = thread % TASK_GROUP_SIZE;
    uint item = ticket * CULL_ITEMS + local;
    ClusterVerdict v = {};
    if (item < items)
        v = cull_cluster(f, item, lane, streamed);
    if (v.draw)
    {
        InterlockedOr(s_drawn[local], 1u << lane);
        InterlockedAdd(s_triangles, v.triangle_count);
        InterlockedAdd(s_levels, v.lod_level);
        if (v.dense)
            InterlockedAdd(s_dense_triangles, v.triangle_count);
        if (v.software)
        {
            InterlockedOr(s_software[local], 1u << lane);
            InterlockedAdd(s_sw_triangles, v.triangle_count);
        }
        else if (v.cutouts != 0)
            InterlockedOr(s_cutout[local], 1u << lane);
    }
    if (v.occluded)
        InterlockedAdd(s_occluded, 1);
    if (v.rejected)
        InterlockedOr(s_rejected[local], 1u << lane);
    GroupMemoryBarrierWithGroupSync();
    uint* args = pass_args(f);
    // Pass 2 fills what pass 1 left of the list (pass 1's counts are final by now).
    uint capacity = f[0].visible_capacity - (second ? pass1_listed(f) : 0);
    uint2 drawn = 0;  // clusters of this workgroup drawn, and software-rasterised among them
    uint2 before = 0; // the same for the work items before this thread's
    for (uint k = 0; k < CULL_ITEMS; k++)
    {
        uint2 n = uint2(countbits(s_drawn[k]), countbits(s_software[k]));
        before += k < local ? n : uint2(0, 0);
        drawn += n;
    }
    // Pass 1 lists the clusters it leaves to pass 2 in the same order (issue #92), and both
    // passes list their cut-out clusters apart (#265), through a run of status words of each
    // pass's own after the two passes' first runs: (rejected, cut out), looked back on with
    // the first. Without a previous pyramid pass 1 draws nothing and leaves everything: pass 2
    // then walks the work items again, as it would.
    bool lists = f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_PREV_PYRAMID) != 0;
    bool cut = (f[0].flags & FLAG_CUTOUTS) != 0;
    uint rejected = 0;        // this workgroup's
    uint rejected_before = 0; // the same for the work items before this thread's
    uint cutouts = 0;         // this workgroup's cut-out clusters
    uint cut_before = 0;      // the same for the work items before this thread's
    if (lists || cut)
    {
        for (uint k = 0; k < CULL_ITEMS; k++)
        {
            uint n = lists ? countbits(s_rejected[k]) : 0u;
            uint c = countbits(s_cutout[k]);
            rejected_before += k < local ? n : 0u;
            rejected += n;
            cut_before += k < local ? c : 0u;
            cutouts += c;
        }
    }
    uint64_t* run = f[0].cluster_lookback + (second ? 3 : 2) * f[0].cluster_groups;
    uint4 firsts = lookback_exclusive2(cluster_status(f), run, lists || cut ? 2u : 1u, ticket,
                                       uint4(drawn, rejected, cutouts), thread);
    uint2 first = firsts.xy;
    uint below = (1u << lane) - 1u;
    uint index = first.x + before.x + countbits(s_drawn[local] & below);
    uint sw_index = first.y + before.y + countbits(s_software[local] & below); // software clusters before this one
    uint hw_index = index - sw_index; // for a hardware-drawn cluster, the hardware-drawn ones before it
    uint rejects_first = firsts.z; // the workgroups' before it
    uint cut_first = firsts.w;     // the workgroups' before it
    // This thread's slot in `Frame::rejects`, and the cut-out clusters before this thread's.
    uint reject_index = lists ? rejects_first + rejected_before + countbits(s_rejected[local] & below) : 0u;
    uint cut_index = cut ? cut_first + cut_before + countbits(s_cutout[local] & below) : 0u;
    // The list ends in this workgroup (at most one does); every thread agrees on both.
    bool past_end = first.x + drawn.x > capacity;
    bool ends_here = past_end && first.x <= capacity;
    if (past_end)
    {
        // This workgroup reaches past the end of the list: count only what it keeps, so the
        // triangles stay exact and the grids cover the kept clusters.
        if (thread == 0)
        {
            s_triangles = 0;
            s_sw_triangles = 0;
            s_dense_triangles = 0;
            s_levels = 0;
        }
        GroupMemoryBarrierWithGroupSync();
        if (v.draw && index < capacity)
        {
            InterlockedAdd(s_triangles, v.triangle_count);
            InterlockedAdd(s_levels, v.lod_level);
            if (v.dense)
                InterlockedAdd(s_dense_triangles, v.triangle_count);
            if (v.software)
            {
                InterlockedAdd(s_sw_triangles, v.triangle_count);
                InterlockedAdd(s_kept_software, 1);
            }
            else if (cut && v.cutouts != 0)
                InterlockedAdd(s_kept_cutouts, 1);
        }
        GroupMemoryBarrierWithGroupSync();
    }
    if (thread == 0)
    {
        if (drawn.x > 0)
        {
            InterlockedAdd(f[0].stats[STAT_TRIANGLES], s_triangles);
            InterlockedAdd(f[0].stats[STAT_LOD_LEVEL_SUM], s_levels);
            InterlockedAdd(f[0].stats[STAT_SW_TRIANGLES], s_sw_triangles);
            InterlockedAdd(f[0].stats[STAT_DENSE_TRIANGLES], s_dense_triangles);
        }
        if (s_occluded > 0)
            InterlockedAdd(f[0].stats[STAT_OCCLUDED], s_occluded);
        if (rejected > 0)
            InterlockedAdd(f[0].stats[STAT_REJECTED], rejected);
        uint2 listed = 0; // the pass's kept clusters and software ones, when known here
        uint listed_cut = 0; // and the cut-out ones among the hardware-drawn
        bool knows = false;
        if (ends_here)
        {
            listed = uint2(capacity, first.y + s_kept_software);
            listed_cut = cut_first + s_kept_cutouts;
            knows = true;
        }
        if (ticket == (items - 1) / CULL_ITEMS)
        {
            // The last workgroup knows the totals.
            uint total = first.x + drawn.x;
            if (total > capacity)
            {
                InterlockedAdd(f[0].stats[STAT_VISIBLE_OVERFLOW], total - capacity);
            }
            else
            {
                listed = first + drawn;
                listed_cut = cut_first + cutouts;
                knows = true;
            }
            if (f[0].pass == PASS_PREVIOUSLY_VISIBLE)
            {
                // Pass 2's grid over segment 1: the clusters left to it, or pass 1's work
                // items again when they were not listed or did not all fit (instance cull 2
                // adds segment 2).
                uint left = rejects_first + rejected;
                bool fits = lists && left <= f[0].rejects_capacity;
                f[0].indirect[INDIRECT_REJECTS] = fits ? left : REJECTS_OVERFLOW;
                uint items2 = fits ? (left + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE : items;
                write_grid(f[0].indirect + INDIRECT_PASS2, (items2 + CULL_ITEMS - 1) / CULL_ITEMS, items2);
            }
        }
        if (knows)
        {
            uint hardware = listed.x - listed.y - listed_cut;
            write_grid(args, hardware, hardware);
            write_grid(args + 4, listed.y, listed.y);
            write_grid(args + 12, listed_cut, listed_cut);
            // The merge: nothing, a rectangle per software cluster, or a full-screen triangle.
            bool rects = listed.y * MERGE_RECT_PIXELS <= f[0].target_width * f[0].target_height;
            args[8] = listed.y == 0 ? 0u : (rects ? 6u : 3u);
            args[9] = rects ? listed.y : 1u;
            InterlockedAdd(f[0].stats[second ? STAT_MESHLETS_PASS2 : STAT_MESHLETS_PASS1], listed.x);
            InterlockedAdd(f[0].stats[STAT_SW_CLUSTERS], listed.y);
        }
    }
    if (lists && v.rejected && reject_index < f[0].rejects_capacity)
        f[0].rejects[reject_index] = uint2(v.instance, v.meshlet | (v.culled_debug ? CULLED_BIT : 0u));
    if (!v.draw || index >= capacity)
        return;
    uint slot = visible_slot(f, index);
    uint flags = (v.culled_debug ? CULLED_BIT : 0u) | (v.software ? SOFTWARE_BIT : 0u);
    f[0].visible[slot] = uint2(v.instance, v.meshlet | flags);
    if (v.software)
    {
        f[0].raster[f[0].visible_capacity - 1 - sw_index] = slot;
        return;
    }
    // A cut-out cluster in the cut-outs' list after the capacity, the others among the
    // hardware-drawn ones less the cut-out ones before them (#265).
    bool cut_out = cut && v.cutouts != 0;
    uint raster_index = cut_out ? f[0].visible_capacity + cut_index : hw_index - cut_index;
    f[0].raster[raster_index] = slot;
    if (f[0].draws != nullptr)
    {
        // VkDrawIndexedIndirectCommand: the cluster's triangles straight from its page (the
        // pool is the index buffer, one-byte indices), its first vertex in the pool as the
        // vertex offset, one instance, and its list slot as the first instance.
        uint* draw = f[0].draws + raster_index * 5;
        draw[0] = v.triangle_count * 3;
        draw[1] = 1;
        // The header read here only: a cluster the cull describes may have no page resident.
        draw[2] = paged_triangles(f, v.payload, f[0].meshlets[v.meshlet]);
        draw[3] = v.payload / PAGED_VERTEX_BYTES;
        draw[4] = slot;
    }
}

// The cluster cull of a scene whose pages are all resident.
[shader("compute")]
[numthreads(TASK_GROUP_SIZE * CULL_ITEMS, 1, 1)]
void cluster_cull_main(uint thread : SV_GroupThreadID)
{
    cluster_cull(thread, false);
}

// The cluster cull of a streamed scene (FLAG_STREAMING).
[shader("compute")]
[numthreads(TASK_GROUP_SIZE * CULL_ITEMS, 1, 1)]
void cluster_cull_streamed_main(uint thread : SV_GroupThreadID)
{
    cluster_cull(thread, true);
}

// ------------------------------------------------------------ instance cull ---

// Whether any cluster of LOD level `l` of the mesh can be selected for this instance: the
// same bounds as `group_possible`, evaluated per level from the instance's distance.
bool level_possible(Frame* f, Mesh* mesh, uint l, float scale, float dist)
{
    float k = f[0].p11 * 0.5 * f[0].viewport_height;
    float t = f[0].lod_threshold;
    float self_far = dist + mesh[0].self_reach_max[l] * scale;
    float parent_near = max(dist - mesh[0].parent_reach_max[l] * scale, f[0].znear);
    float self_min_px = mesh[0].self_error_min[l] * scale * k / self_far;
    float parent_max_px = mesh[0].parent_error_max[l] * scale * k / parent_near;
    bool streamed = (f[0].flags & FLAG_STREAMING) != 0; // see `group_possible`
    return (self_min_px <= t || streamed) && parent_max_px > t;
}

// The root shortcut (issue #37): when every root of the mesh is fine enough at the nearest
// distance its roots can be from the camera, the DAG's one cut is the roots themselves
// (errors and spheres grow up the DAG, so no cluster below a selected root has a parent that
// is too coarse) and the instance needs no work item of its own: its roots go to the root
// list, which the cluster cull reads 32 entries to an item. Far instances of big meshes, 32
// lanes testing the coarse levels for one or two clusters, were most of the culls' time at a
// million instances. Returns how many roots the instance lists, 0 when it takes work items.
uint instance_roots(Frame* f, Mesh* mesh, float scale, float dist)
{
    if ((f[0].flags & (FLAG_LOD | FLAG_GROUP_WINDOW_OFF)) != FLAG_LOD || mesh[0].root_count == 0)
        return 0;
    float nearest = max(dist - mesh[0].root_reach_max * scale, f[0].znear);
    float px = mesh[0].root_error_max * scale * f[0].p11 * 0.5 * f[0].viewport_height / nearest;
    return px <= f[0].lod_threshold ? mesh[0].root_count : 0;
}

// The work items of a visible instance: its groups of 32 clusters covering the LOD levels
// that can hold selected clusters at its distance, in increasing order, each once. Returns
// how many; with `write`, also writes them to the work list from `base`, the instance tagged
// with `tag`: the items `lane`, `lane + stride`, ... (a thread writes them all with lane 0 and
// stride 1; a workgroup shares a large instance's, `append_instance_work`). With
// INSTANCE_LADDER_BIT in `tag`, the groups of the next mesh on its ladder.
uint instance_items(Frame* f, uint instance_index, uint base, bool write, uint tag, uint lane = 0, uint stride = 1)
{
    Instance* inst = f[0].instances + instance_index;
    Mesh* mesh = f[0].meshes + drawn_mesh(f, instance_index, inst[0], tag);
    float scale = inst[0].scale;
    float dist = length(instance_center_relative(f, inst[0]));
    uint groups = (mesh[0].meshlet_count + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE;
    bool all = (f[0].flags & FLAG_GROUP_WINDOW_OFF) != 0;
    bool lod = (f[0].flags & FLAG_LOD) != 0;
    uint level_count = max(mesh[0].level_count, 1u);
    uint count = 0;
    uint next_group = 0; // groups come out in increasing order, each at most once
    for (uint l = 0; l < level_count; l++)
    {
        uint begin = mesh[0].level_offset[l];
        uint end = mesh[0].level_offset[l + 1];
        if (end <= begin)
            continue;
        bool possible = all || (lod ? level_possible(f, mesh, l, scale, dist) : (l == 0));
        if (!possible)
            continue;
        uint first = max(begin / TASK_GROUP_SIZE, next_group);
        uint last = min((end - 1) / TASK_GROUP_SIZE, groups - 1);
        for (uint g = first + lane; write && g <= last; g += stride)
        {
            uint slot = base + count + (g - first);
            if (slot < f[0].work_capacity)
                f[0].work[slot] = uint2(instance_index | tag, g);
        }
        if (first <= last)
            count += last - first + 1;
        next_group = last + 1;
    }
    return count;
}

groupshared uint s_counts[64];
groupshared uint s_root_counts[64];
groupshared uint2 s_total;
groupshared uint s_deferred[64];
groupshared uint s_deferred_total;
groupshared uint2 s_heavy[128]; // (tagged instance, first slot) of the workgroup's meshes with many work items, two an instance on a ladder
groupshared uint s_heavy_count;

// An instance with more work items than this has them written by its whole workgroup: the
// terrain's thousands, written by one thread, held every city frame's instance cull
// (issue #38).
static const uint HEAVY_ITEMS = 64;

// Appends the work of workgroup `ticket` (of `groups`) of an instance cull, one instance a
// thread: a `visible` instance's work items to the work list, or its roots to the root list
// (`instance_roots`), both in the cull's order (see "ordered appends", `status` its words).
// Instance cull 1 writes the lists from their start (segment 1); instance cull 2 (`second`)
// after segment 1 (segment 2, issue #38). The last workgroup writes the grid its cluster cull
// reads, over the items that fit in `work_capacity` plus the root runs, and counts the demand;
// each group of an instance is appended at most once. An instance on a ladder (D-075) has its
// crossing written (`Frame::ladder`) and appends its mesh's work, the next mesh's (tagged
// INSTANCE_LADDER_BIT) or, crossing, both: its mesh's first.
void append_instance_work(Frame* f, uint64_t* status, uint ticket, uint groups, uint thread,
                          uint instance_index, bool visible, uint tag, bool second)
{
    uint count = 0;      // work items, of both meshes
    uint roots = 0;      // root entries, of both meshes
    uint own_count = 0;  // of those, its own mesh's (the next mesh's follow)
    uint own_roots = 0;
    if (visible)
    {
        Instance* inst = f[0].instances + instance_index;
        Mesh* mesh = f[0].meshes + inst[0].mesh;
        float dist = length(instance_center_relative(f, inst[0]));
        float value = ladder_crossing(f, inst[0]);
        // The fade after its crossing (#363): an instance its impostor has taken whole draws
        // nothing.
        float fade = ladder_fade_after(f, inst[0], value);
        if (fade > 0.0)
            value += LADDER_FADE_UNIT * max(round(fade * LADDER_FADE_STEPS), 1.0);
        if (mesh[0].ladder_next != LADDER_NONE && f[0].ladder != nullptr)
            f[0].ladder[instance_index] = value;
        // The pair it crosses: its own mesh and the next, or further down its ladder (#332).
        float t = ladder_t(value);
        bool taken = fade >= 1.0;
        Mesh* first_mesh = f[0].meshes + ladder_step(f, inst[0].mesh, ladder_pair(value));
        if (t < 1.0 && !taken)
        {
            own_roots = instance_roots(f, first_mesh, inst[0].scale, dist);
            own_count = own_roots > 0 ? 0u : instance_items(f, instance_index, 0, false, 0u);
        }
        count = own_count;
        roots = own_roots;
        // A ladder to nothing (LADDER_VANISH, #343): past its crossing, no work.
        if (t > 0.0 && !taken && first_mesh[0].ladder_next != LADDER_VANISH)
        {
            Mesh* next = f[0].meshes + first_mesh[0].ladder_next;
            uint next_roots = instance_roots(f, next, inst[0].scale, dist);
            roots += next_roots;
            count += next_roots > 0 ? 0u : instance_items(f, instance_index, 0, false, INSTANCE_LADDER_BIT);
        }
    }
    s_counts[thread] = count;
    s_root_counts[thread] = roots;
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        uint2 total = 0;
        for (uint i = 0; i < 64; i++)
        {
            uint2 c = uint2(s_counts[i], s_root_counts[i]);
            s_counts[i] = total.x; // now this thread's offsets in the workgroup's runs
            s_root_counts[i] = total.y;
            total += c;
        }
        s_total = total;
    }
    GroupMemoryBarrierWithGroupSync();
    uint2 total = s_total;
    uint2 first = lookback_exclusive(status, ticket, total, thread);
    uint2 base = second ? uint2(f[0].indirect[5], f[0].indirect[6]) : uint2(0, 0);
    if (thread == 0 && ticket == groups - 1)
    {
        // The last workgroup knows the demand: what fits goes to the cluster cull, the rest is
        // counted (the renderer grows the lists from it).
        uint2 demand = first + total;
        uint regular = min(demand.x, f[0].work_capacity - base.x);
        uint listed = min(demand.y, f[0].work_capacity - base.y);
        uint items = regular + (listed + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE;
        if (second)
        {
            // Pass 2's cluster cull covers both segments: segment 1 as cluster cull 1 left it
            // (issue #92), then segment 2.
            uint all = pass2_segment1_items(f) + items;
            write_grid(f[0].indirect + INDIRECT_PASS2, (all + CULL_ITEMS - 1) / CULL_ITEMS, all);
            f[0].indirect[INDIRECT_PASS2 + 4] = regular;
            f[0].indirect[INDIRECT_PASS2 + 5] = listed;
        }
        else
        {
            write_grid(f[0].indirect, (items + CULL_ITEMS - 1) / CULL_ITEMS, items);
            f[0].indirect[5] = regular;
            f[0].indirect[6] = listed;
            // Pass 2 covers segment 1 alone until cluster cull 1 lists what it leaves to it
            // (issue #92) and instance cull 2 appends a segment 2.
            write_grid(f[0].indirect + INDIRECT_PASS2, (items + CULL_ITEMS - 1) / CULL_ITEMS, items);
            f[0].indirect[INDIRECT_PASS2 + 4] = 0;
            f[0].indirect[INDIRECT_PASS2 + 5] = 0;
        }
        InterlockedAdd(f[0].stats[STAT_WORK_ITEMS], demand.x);
        InterlockedAdd(f[0].stats[STAT_ROOT_ENTRIES], demand.y);
        uint dropped = (demand.x - regular) + (demand.y - listed);
        if (dropped > 0)
            InterlockedAdd(f[0].stats[STAT_WORK_OVERFLOW], dropped);
    }
    // A thread writes its instance's items, unless they are many: those the workgroup writes
    // together, one instance after the other.
    if (thread == 0)
        s_heavy_count = 0;
    GroupMemoryBarrierWithGroupSync();
    uint slot = base.x + first.x + s_counts[thread];
    // Its own mesh's items, then the next mesh's on its ladder (`tag` is 0 or
    // INSTANCE_HIDDEN_BIT, the next mesh's add INSTANCE_LADDER_BIT).
    for (uint part = 0; part < 2; part++)
    {
        uint n = part == 0 ? own_count : count - own_count;
        uint tagged = instance_index | tag | (part == 0 ? 0u : INSTANCE_LADDER_BIT);
        uint at = slot + (part == 0 ? 0u : own_count);
        if (n > HEAVY_ITEMS)
        {
            uint k;
            InterlockedAdd(s_heavy_count, 1, k);
            s_heavy[k] = uint2(tagged, at);
        }
        else if (n > 0)
            instance_items(f, instance_index, at, true, tagged & INSTANCE_TAG_BITS);
    }
    // The whole workgroup writes a heavy instance's items, reading its crossing (`Frame::ladder`)
    // its own thread wrote: the barrier covers device memory too, not only the workgroup's.
    AllMemoryBarrierWithGroupSync();
    for (uint h = 0; h < s_heavy_count; h++)
    {
        uint tagged = s_heavy[h].x;
        instance_items(f, tagged & ~INSTANCE_TAG_BITS, s_heavy[h].y, true, tagged & INSTANCE_TAG_BITS, thread, 64);
    }
    if (roots > 0)
    {
        Instance inst = f[0].instances[instance_index];
        uint entry = base.y + first.y + s_root_counts[thread];
        for (uint k = 0; k < roots; k++)
        {
            uint part_tag = k < own_roots ? 0u : INSTANCE_LADDER_BIT;
            Mesh* mesh = f[0].meshes + drawn_mesh(f, instance_index, inst, part_tag);
            if (entry + k < f[0].work_capacity)
                f[0].roots[entry + k] = uint2(instance_index | tag | part_tag, mesh[0].roots[k < own_roots ? k : k - own_roots]);
        }
    }
}

// Instance cull 1, one thread per instance: frustum-cull it and append its work
// (`append_instance_work`). With FLAG_INSTANCE_OCCLUSION, an instance the previous frame's
// pyramid hides (`visible_last_frame`, the clusters' pass 1 question) waits for this frame's
// instead: it goes to `Frame::deferred`, in instance order, and pass 1 sees none of its
// clusters (issue #38). Its last workgroup writes instance cull 2's grid at the head of that
// list. With FLAG_INSTANCE_CELLS, a workgroup takes a cell of 64 instances cell cull 1 listed
// (`cell_cull_main`) instead of the next 64 of the table.
[shader("compute")]
[numthreads(64, 1, 1)]
void cull_main(uint thread : SV_GroupThreadID)
{
    Frame* f = push.frame;
    if (thread == 0)
        InterlockedAdd(f[0].indirect[4], 1, s_ticket);
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint groups = instance_cull_groups(f); // the length of each run of status words
    uint run = groups;                     // this cull's workgroups
    uint cell = ticket;
    bool in_cell = true;
    if ((f[0].flags & FLAG_INSTANCE_CELLS) != 0)
    {
        // A workgroup per listed cell, and one when none is (its last writes the grids).
        uint listed = f[0].cell_list[3];
        run = max(listed, 1u);
        if (ticket >= run)
            return; // the rest of the grid's last row: the whole workgroup leaves
        in_cell = ticket < listed;
        cell = in_cell ? f[0].cell_list[CELL_LIST + ticket] : 0u;
    }
    uint instance_index = cell * 64 + thread;
    bool visible = false;
    float3 sphere_center = 0.0;
    float sphere_radius = 0.0;
    if (in_cell && instance_index < f[0].instance_count)
    {
        Instance inst = f[0].instances[instance_index];
        sphere_center = instance_center_relative(f, inst);
        sphere_radius = inst.radius;
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        visible = (inst.flags & (INSTANCE_VACANT | INSTANCE_NO_RASTER)) == 0
            && (!frustum || sphere_in_frustum(f, sphere_center, sphere_radius));
    }
    uint in_frustum = WaveActiveCountBits(visible);
    if (WaveIsFirstLane() && in_frustum > 0)
        InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], in_frustum);
    // With FLAG_HIDDEN_INSTANCES, the instances the previous pyramid hides are counted (the
    // renderer's auto mode decides from them); with FLAG_INSTANCE_OCCLUSION they are also
    // deferred.
    bool hidden = false;
    if (visible && f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_HIDDEN_INSTANCES) != 0)
        hidden = !visible_last_frame(f, sphere_center, sphere_radius);
    uint hidden_count = WaveActiveCountBits(hidden);
    if (WaveIsFirstLane() && hidden_count > 0)
        InterlockedAdd(f[0].stats[STAT_INSTANCES_OCCLUDED], hidden_count);
    bool deferred = false;
    if (f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_INSTANCE_OCCLUSION) != 0)
    {
        deferred = hidden;
        s_deferred[thread] = deferred ? 1u : 0u;
        GroupMemoryBarrierWithGroupSync();
        if (thread == 0)
        {
            uint total = 0;
            for (uint i = 0; i < 64; i++)
            {
                uint d = s_deferred[i];
                s_deferred[i] = total; // now this thread's offset in the workgroup's run
                total += d;
            }
            s_deferred_total = total;
        }
        GroupMemoryBarrierWithGroupSync();
        uint first = lookback_exclusive(f[0].lookback + groups, ticket, uint2(s_deferred_total, 0), thread).x;
        if (deferred)
            f[0].deferred[DEFERRED_LIST + first + s_deferred[thread]] = instance_index;
        if (thread == 0 && ticket == run - 1)
        {
            uint n = first + s_deferred_total;
            write_grid(f[0].deferred, (n + 63) / 64, n);
            f[0].deferred[DEFERRED_SINGLES] = n; // where cell cull 2 appends
        }
    }
    append_instance_work(f, f[0].lookback, ticket, run, thread, instance_index, visible && !deferred, 0u, false);
}

// ---------------------------------------------------------------- cell culls ---
//
// Issue #38: the instance culls test every instance of the table every frame, a million in the
// city. With FLAG_INSTANCE_CELLS the table is read in cells of 64 (a cell is the instances one
// instance-cull workgroup takes, `Frame::cells` their bounding spheres, `cell_bounds_main`).
// Cell cull 1 lists the cells in the frustum for instance cull 1 and, with instance occlusion,
// defers those the previous pyramid hides whole; cell cull 2 tests those against this frame's
// pyramid and hands the instances of the cells it lets through to instance cull 2. An
// instance's sphere lies inside its cell's, so a cell out of view has no instance in view:
// both ways draw the same clusters. The cells pay when neighbours in the table are neighbours
// in the world (the city sorts its table, `crate::placement`).

uint cell_count(Frame* f)
{
    return (f[0].instance_count + 63) / 64;
}

groupshared uint2 s_cell_offsets[64];

// Cell cull 1, one thread per cell: the frustum and, with FLAG_INSTANCE_OCCLUSION, the
// previous pyramid. Lists the cells in view for instance cull 1 and those the pyramid hides
// for cell cull 2, both in cell order (look-back, run 3); the last workgroup writes both
// grids. The instances of a deferred cell count as in the frustum and hidden (the auto mode's
// statistics), all of them: they are not tested one by one.
[shader("compute")]
[numthreads(64, 1, 1)]
void cell_cull_main(uint thread : SV_GroupThreadID)
{
    Frame* f = push.frame;
    if (thread == 0)
        InterlockedAdd(f[0].cell_list[CELL_TICKET], 1, s_ticket);
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint cells = cell_count(f);
    uint groups = max((cells + 63) / 64, 1u);
    uint cell = ticket * 64 + thread;
    bool listed = false;
    bool deferred = false;
    uint hidden_instances = 0;
    if (cell < cells)
    {
        CellBounds bounds = f[0].cells[cell];
        float3 sphere = cell_center_relative(f, bounds);
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        bool in_view = !frustum || sphere_in_frustum(f, sphere, bounds.radius);
        if (in_view && f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_INSTANCE_OCCLUSION) != 0)
            deferred = !visible_last_frame(f, sphere, bounds.radius);
        listed = in_view && !deferred;
        if (deferred)
            hidden_instances = min(64u, f[0].instance_count - cell * 64);
    }
    uint hidden_sum = WaveActiveSum(hidden_instances);
    if (WaveIsFirstLane() && hidden_sum > 0)
    {
        InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], hidden_sum);
        InterlockedAdd(f[0].stats[STAT_INSTANCES_OCCLUDED], hidden_sum);
    }
    s_cell_offsets[thread] = uint2(listed ? 1u : 0u, deferred ? 1u : 0u);
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        uint2 total = 0;
        for (uint i = 0; i < 64; i++)
        {
            uint2 c = s_cell_offsets[i];
            s_cell_offsets[i] = total; // now this thread's offsets in the workgroup's runs
            total += c;
        }
        s_total = total;
    }
    GroupMemoryBarrierWithGroupSync();
    uint2 total = s_total;
    uint2 first = lookback_exclusive(f[0].lookback + 3 * instance_cull_groups(f), ticket, total, thread);
    uint2 offset = first + s_cell_offsets[thread];
    if (listed)
        f[0].cell_list[CELL_LIST + offset.x] = cell;
    if (deferred)
        f[0].cell_list[CELL_LIST + cells + offset.y] = cell;
    if (thread == 0 && ticket == groups - 1)
    {
        uint2 n = first + total;
        write_grid(f[0].cell_list, max(n.x, 1u), n.x);
        write_grid(f[0].cell_list + CELL_GRID2, (n.y + 63) / 64, n.y);
        InterlockedAdd(f[0].stats[STAT_CELLS_LISTED], n.x);
        InterlockedAdd(f[0].stats[STAT_CELLS_DEFERRED], n.y);
    }
}

// Cell cull 2, after pass 1 and the pyramid it builds, one thread per cell cell cull 1
// deferred: the cells this frame's pyramid does not hide (with FLAG_SHOW_CULLED, all of them)
// are opened, listed after the instances instance cull 1 deferred one by one, in cell order
// (look-back, run 4). Its last workgroup writes instance cull 2's grid over both, and
// instance cull 2 takes an opened cell's instances a workgroup per cell.
[shader("compute")]
[numthreads(64, 1, 1)]
void cell_cull_deferred_main(uint thread : SV_GroupThreadID)
{
    Frame* f = push.frame;
    if (thread == 0)
        InterlockedAdd(f[0].cell_list[CELL_TICKET2], 1, s_ticket);
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint deferred_cells = f[0].cell_list[CELL_GRID2 + 3];
    uint groups = (deferred_cells + 63) / 64;
    if (ticket >= groups)
        return; // the rest of the grid's last row: the whole workgroup leaves
    uint k = ticket * 64 + thread;
    uint cell = 0;
    bool open = false;
    if (k < deferred_cells)
    {
        cell = f[0].cell_list[CELL_LIST + cell_count(f) + k];
        CellBounds bounds = f[0].cells[cell];
        open = (f[0].flags & FLAG_SHOW_CULLED) != 0 || occlusion_visible(f, cell_center_relative(f, bounds), bounds.radius);
    }
    s_deferred[thread] = open ? 1u : 0u;
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        uint total = 0;
        for (uint i = 0; i < 64; i++)
        {
            uint d = s_deferred[i];
            s_deferred[i] = total; // now this thread's offset in the workgroup's run
            total += d;
        }
        s_deferred_total = total;
    }
    GroupMemoryBarrierWithGroupSync();
    uint first = lookback_exclusive(f[0].lookback + 4 * instance_cull_groups(f), ticket, uint2(s_deferred_total, 0), thread).x;
    uint singles = f[0].deferred[DEFERRED_SINGLES];
    if (open)
        f[0].deferred[DEFERRED_LIST + singles + first + s_deferred[thread]] = cell;
    if (thread == 0 && ticket == groups - 1)
    {
        uint opened = first + s_deferred_total;
        f[0].deferred[DEFERRED_OPENED] = opened;
        uint instance_groups = (singles + 63) / 64 + opened;
        write_grid(f[0].deferred, instance_groups, instance_groups);
        InterlockedAdd(f[0].stats[STAT_CELLS_OPENED], opened);
    }
}

struct CellBoundsPush
{
    Instance* instances;
    CellBounds* cells;
    uint count;
    uint first_cell; // the first cell of the dispatch: 0, or the movers' first (#79)
};

[[vk::push_constant]]
CellBoundsPush cell_bounds_push;

groupshared float3 s_bounds_min[64];
groupshared float3 s_bounds_max[64];
groupshared float s_bounds_reach[64];

// Once, after the instances are written, and every frame for the movers' cells (#79): a
// workgroup per cell of 64 instances, the sphere around their spheres (centred on the box
// around them; a tree reduction, so the same table gives the same bits), from the corner of the
// first instance's cell (issue #93).
[shader("compute")]
[numthreads(64, 1, 1)]
void cell_bounds_main(uint3 group : SV_GroupID, uint thread : SV_GroupThreadID)
{
    CellBoundsPush p = cell_bounds_push;
    uint cell = p.first_cell + group.x;
    uint index = cell * 64 + thread;
    bool valid = index < p.count && (p.instances[index].flags & INSTANCE_VACANT) == 0;
    int3 reference = p.instances[cell * 64].cell;
    float3 c = 0.0;
    float r = 0.0;
    if (valid)
    {
        Instance inst = p.instances[index];
        c = float3(inst.cell - reference) * CELL_SIZE + inst.center;
        r = inst.radius;
    }
    s_bounds_min[thread] = valid ? c - r : float3(3.0e38);
    s_bounds_max[thread] = valid ? c + r : float3(-3.0e38);
    GroupMemoryBarrierWithGroupSync();
    for (uint stride = 32; stride > 0; stride >>= 1)
    {
        if (thread < stride)
        {
            s_bounds_min[thread] = min(s_bounds_min[thread], s_bounds_min[thread + stride]);
            s_bounds_max[thread] = max(s_bounds_max[thread], s_bounds_max[thread + stride]);
        }
        GroupMemoryBarrierWithGroupSync();
    }
    float3 centre = 0.5 * (s_bounds_min[0] + s_bounds_max[0]);
    GroupMemoryBarrierWithGroupSync();
    s_bounds_reach[thread] = valid ? length(c - centre) + r : 0.0;
    GroupMemoryBarrierWithGroupSync();
    for (uint stride = 32; stride > 0; stride >>= 1)
    {
        if (thread < stride)
            s_bounds_reach[thread] = max(s_bounds_reach[thread], s_bounds_reach[thread + stride]);
        GroupMemoryBarrierWithGroupSync();
    }
    if (thread == 0)
    {
        CellBounds bounds;
        bounds.cell = reference;
        bounds.radius = s_bounds_reach[0];
        bounds.center = centre;
        bounds.pad = 0.0;
        p.cells[cell] = bounds;
    }
}

// Instance cull 2 (issue #38), after pass 1 and the pyramid it builds: the instances instance
// cull 1 deferred, against this frame's pyramid (`occlusion_visible`), a workgroup per 64 of
// them; then a workgroup per cell cell cull 2 opened, a thread per instance of the cell in the
// frustum. Those it does not hide append their work as segment 2, which only pass 2's cluster
// cull reads, without asking pass 1's question. With FLAG_SHOW_CULLED the hidden ones append
// theirs too, tagged, and their clusters are drawn as culled.
[shader("compute")]
[numthreads(64, 1, 1)]
void cull_deferred_main(uint thread : SV_GroupThreadID)
{
    Frame* f = push.frame;
    if (thread == 0)
        InterlockedAdd(f[0].indirect[INDIRECT_PASS2 + 6], 1, s_ticket);
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint singles = f[0].deferred[DEFERRED_SINGLES];
    uint single_groups = (singles + 63) / 64;
    uint groups = single_groups + f[0].deferred[DEFERRED_OPENED];
    if (ticket >= groups)
        return; // the rest of the grid's last row: the whole workgroup leaves
    uint instance_index = 0;
    bool candidate = false;
    if (ticket < single_groups)
    {
        uint index = ticket * 64 + thread;
        candidate = index < singles;
        if (candidate)
            instance_index = f[0].deferred[DEFERRED_LIST + index];
    }
    else
    {
        // An opened cell's instances, those in the frustum (its cell was, not all of them).
        uint cell = f[0].deferred[DEFERRED_LIST + singles + ticket - single_groups];
        instance_index = cell * 64 + thread;
        if (instance_index < f[0].instance_count)
        {
            Instance inst = f[0].instances[instance_index];
            bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
            candidate = (inst.flags & (INSTANCE_VACANT | INSTANCE_NO_RASTER)) == 0
                && (!frustum || sphere_in_frustum(f, instance_center_relative(f, inst), inst.radius));
        }
    }
    bool visible = false;
    bool hidden = false;
    if (candidate)
    {
        Instance inst = f[0].instances[instance_index];
        visible = occlusion_visible(f, instance_center_relative(f, inst), inst.radius);
        hidden = !visible;
    }
    bool show_culled = (f[0].flags & FLAG_SHOW_CULLED) != 0;
    append_instance_work(f, f[0].lookback + 2 * instance_cull_groups(f), ticket, groups, thread,
                         instance_index, visible || (hidden && show_culled),
                         hidden ? INSTANCE_HIDDEN_BIT : 0u, true);
}

// ---------------------------------------------------------------- mesh stage ---

static const uint MAX_VERTICES = 64;
static const uint MAX_TRIANGLES = 124;
static const uint MESH_GROUP_SIZE = 64;

struct VertexOut
{
    float4 position : SV_Position;
};

// The whole visibility id travels as the primitive id: no per-vertex attributes at all.
struct PrimitiveOut
{
    uint id : SV_PrimitiveID;
};

// One workgroup per hardware-drawn cluster of this pass (the front of `Frame::raster`); no
// task stage: the culling ran in compute (`cluster_cull_main`), shared with the fallback.
[shader("mesh")]
[outputtopology("triangle")]
[numthreads(MESH_GROUP_SIZE, 1, 1)]
void mesh_main(
    uint thread : SV_GroupThreadID,
    uint3 group : SV_GroupID,
    out vertices VertexOut verts[MAX_VERTICES],
    out indices uint3 tris[MAX_TRIANGLES],
    out primitives PrimitiveOut prims[MAX_TRIANGLES])
{
    Frame* f = push.frame;
    uint index = grid_index(group);
    uint vertex_count = 0;
    uint triangle_count = 0;
    uint slot = 0;
    uint base = 0;
    Meshlet m = {};
    Instance inst = {};
    // Nothing for the partial last row of the grid. The clusters `mesh_cutout_main` draws have
    // a list of their own (#171, #265).
    if (index < pass_args(f)[3])
    {
        slot = f[0].raster[index];
        uint2 record = f[0].visible[slot];
        m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
        inst = f[0].instances[record.x];
        vertex_count = m.vertex_count;
        triangle_count = m.triangle_count;
        base = payload_base(f, m);
    }
    // One call, at the top level, whatever the cluster: AMD's RDNA 2 driver hangs the GPU
    // when the call sits in several branches with early returns (#28).
    SetMeshOutputCounts(vertex_count, triangle_count);

    if (thread < vertex_count)
    {
        PagedVertex v = paged_vertex(f, base, m, thread);
        float3 relative = instance_point(f, inst, v.position);
        verts[thread].position = mul(f[0].view_proj, float4(relative, 1));
    }

    for (uint t = thread; t < triangle_count; t += MESH_GROUP_SIZE)
    {
        tris[t] = uint3(paged_index(f, base, m, t, 0), paged_index(f, base, m, t, 1), paged_index(f, base, m, t, 2));
        prims[t].id = (slot << VISIBILITY_TRIANGLE_BITS) | t;
    }
}

// The cut-outs' raster (#171): the clusters whose rows are cut out or double-sided, drawn by a
// second pipeline over a list of their own (#265) with no face culling, its fragments tested by
// `cutout_kept`. Kept apart so that the other clusters' fragment shader has no discard, which
// would cost them the early depth test.
struct CutoutVertexOut
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0; // the mesh's UV (0 without them)
    float2 ground : TEXCOORD1; // its (x, z) in its mesh's frame, for the ground window (#197)
    float slant : TEXCOORD2; // `card_slant`, for a grating's bars
};

// A grating's shading normal: its upright bars' faces, seen together, turn to the viewer as a
// round bar's would. The card's own normal caught the sky at grazing views, a pale sheet where
// the bars stood dark.
float3 grating_normal(float3 view)
{
    float3 h = float3(view.x, 0.0, view.z);
    return dot(h, h) > 1e-8 ? normalize(h) : view;
}

// How far the view slants along an upright card at a point `relative` to the camera, its normal
// `n`: the tangent of the view's angle off the normal in the horizontal plane. A grating's square
// bars look 1 + this times their width wide along the card (`grating_coverage`).
float card_slant(float3 n, float3 relative)
{
    float3 along = cross(n, float3(0.0, 1.0, 0.0));
    float3 v = normalize(relative);
    return min(abs(dot(v, along)) / max(abs(dot(v, n)), 1e-3), 64.0);
}

[shader("mesh")]
[outputtopology("triangle")]
[numthreads(MESH_GROUP_SIZE, 1, 1)]
void mesh_cutout_main(
    uint thread : SV_GroupThreadID,
    uint3 group : SV_GroupID,
    out vertices CutoutVertexOut verts[MAX_VERTICES],
    out indices uint3 tris[MAX_TRIANGLES],
    out primitives PrimitiveOut prims[MAX_TRIANGLES])
{
    Frame* f = push.frame;
    uint index = grid_index(group);
    uint vertex_count = 0;
    uint triangle_count = 0;
    uint slot = 0;
    uint base = 0;
    Meshlet m = {};
    Instance inst = {};
    // Nothing for the partial last row of the grid: the cut-out clusters' own list, after the
    // capacity (#265).
    if (index < (pass_args(f) + 12)[3])
    {
        slot = f[0].raster[f[0].visible_capacity + index];
        uint2 record = f[0].visible[slot];
        m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
        inst = f[0].instances[record.x];
        vertex_count = m.vertex_count;
        triangle_count = m.triangle_count;
        base = payload_base(f, m);
    }
    // One call at the top level, as in `mesh_main` (#28).
    SetMeshOutputCounts(vertex_count, triangle_count);

    if (thread < vertex_count)
    {
        bool uvs = f[0].meshes[inst.mesh].uvs != 0;
        PagedVertex v = paged_vertex(f, base, m, thread);
        float3 relative = instance_point(f, inst, v.position);
        verts[thread].position = mul(f[0].view_proj, float4(relative, 1));
        verts[thread].uv = uvs ? paged_uv(f, uv_stream(f, base, m), thread) : float2(0.0);
        verts[thread].ground = v.position.xz;
        verts[thread].slant = card_slant(normalize(instance_vector(inst, decode_normal(v.normal))), relative);
    }

    for (uint t = thread; t < triangle_count; t += MESH_GROUP_SIZE)
    {
        tris[t] = uint3(paged_index(f, base, m, t, 0), paged_index(f, base, m, t, 1), paged_index(f, base, m, t, 2));
        prims[t].id = (slot << VISIBILITY_TRIANGLE_BITS) | t;
    }
}

// ---------------------------------------------------------- vertex fallback ---

// Without mesh shaders every cluster is one indexed draw of `vkCmdDrawIndexedIndirectCount`
// (the command `cluster_cull_main` wrote): the index buffer is the pool of cluster pages,
// whose one-byte local indices plus the draw's vertex offset (the cluster's first vertex in
// the pool) index the pool's vertices, and the first instance is the cluster's slot in the
// visible-cluster list. The raw Vulkan builtins are used: Slang's `SV_VertexID` and
// `SV_InstanceID` subtract the base vertex and instance (the D3D meaning).
struct FallbackVertex
{
    float4 position : SV_Position;
    nointerpolation uint id_base : VISIBILITY_ID; // slot << VISIBILITY_TRIANGLE_BITS
};

[shader("vertex")]
FallbackVertex vertex_main(uint vertex_index : SV_VulkanVertexID, uint slot : SV_VulkanInstanceID)
{
    Frame* f = push.frame;
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    // The draw's vertex offset is the payload's start over 16: the vertex's index in it.
    uint base = payload_base(f, m);
    PagedVertex v = paged_vertex(f, base, m, vertex_index - base / PAGED_VERTEX_BYTES);
    float3 relative = instance_point(f, inst, v.position);
    FallbackVertex o;
    o.position = mul(f[0].view_proj, float4(relative, 1));
    o.id_base = slot << VISIBILITY_TRIANGLE_BITS;
    // The cut-out clusters are `vertex_cutout_main`'s, from draws of their own (#171, #265).
    return o;
}

struct FallbackCutoutVertex
{
    float4 position : SV_Position;
    nointerpolation uint id_base : VISIBILITY_ID;
    float2 uv : TEXCOORD0;
    float2 ground : TEXCOORD1; // its (x, z) in its mesh's frame, for the ground window (#197)
    float slant : TEXCOORD2;   // `card_slant`
};

// `mesh_cutout_main` on the fallback path: the cut-out clusters' draws, after the others'.
[shader("vertex")]
FallbackCutoutVertex vertex_cutout_main(uint vertex_index : SV_VulkanVertexID, uint slot : SV_VulkanInstanceID)
{
    Frame* f = push.frame;
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    uint base = payload_base(f, m);
    uint local = vertex_index - base / PAGED_VERTEX_BYTES;
    PagedVertex v = paged_vertex(f, base, m, local);
    float3 relative = instance_point(f, inst, v.position);
    FallbackCutoutVertex o;
    o.position = mul(f[0].view_proj, float4(relative, 1));
    o.id_base = slot << VISIBILITY_TRIANGLE_BITS;
    o.uv = float2(0.0);
    o.ground = v.position.xz;
    o.slant = card_slant(normalize(instance_vector(inst, decode_normal(v.normal))), relative);
    if (f[0].meshes[inst.mesh].uvs != 0)
        o.uv = paged_uv(f, uv_stream(f, base, m), local);
    return o;
}

// ------------------------------------------------------------ fragment stage ---

// Writes the visibility id; the depth test already decided who owns the pixel.
[shader("fragment")]
uint frag_main(uint id : SV_PrimitiveID) : SV_Target
{
    return id;
}

// The same id on the fallback path: the primitive id restarts at 0 in every draw of the
// indirect-count call, so it is the triangle's index in its cluster.
[shader("fragment")]
uint frag_fallback_main(nointerpolation uint id_base : VISIBILITY_ID, uint triangle : SV_PrimitiveID) : SV_Target
{
    return id_base | triangle;
}

// -------------------------------------------------------------------- grass ---
// Grass near the camera (#300, D-075): blades drawn into the visibility buffer from the region's
// grass map (`Frame::grass`), each a pure function of its cell and its index there, the same
// wherever the camera stands (D-063: nothing is generated around the viewer). Three culls list
// the patches of 16 blades within the reach in a fixed order (`grass_count_main`,
// `grass_scan_main`, `grass_emit_main`); `grass_mesh_main` draws them (the fallback:
// `grass_vertex_main`), and the resolve rebuilds a blade's triangle from its id
// (`grass_hit`). Opaque blades at least a pixel wide, the part of a pixel a thinner one covers
// kept by the frame's noise (Humus' wires; DLAA averages it). Met by no ray.

static const uint GRASS_CELL_BITS = 21;
static const uint GRASS_CELL_BLADES = 128;
static const uint GRASS_PATCH_BLADES = 16;
static const uint GRASS_SEGMENTS = 3;
static const uint GRASS_BLADE_VERTICES = 2 * GRASS_SEGMENTS + 1;
static const uint GRASS_BLADE_TRIANGLES = 2 * GRASS_SEGMENTS - 1;
static const uint GRASS_PATCH_VERTICES = GRASS_PATCH_BLADES * GRASS_BLADE_VERTICES;
static const uint GRASS_PATCH_TRIANGLES = GRASS_PATCH_BLADES * GRASS_BLADE_TRIANGLES;
// The mesh grid's rows: a patch's index is `group.y * GRASS_GRID_ROW + group.x`.
static const uint GRASS_GRID_ROW = 256;
// A blade's width at its root, metres, before its own spread.
static const float GRASS_WIDTH = 0.012;
// Where the blades' normals and contrast have converged to their clump's, metres: the distance
// tuned against shimmer with a 20 m reach (#300). The same at every resolution, so a supersampled
// reference shades as the frame it checks.
static const float GRASS_CONVERGED = 20.0;
// A blade's vertices: left and right at the foot of each segment, then the tip.
static const uint3 GRASS_TRIANGLES[GRASS_BLADE_TRIANGLES] = {
    uint3(0, 1, 3), uint3(0, 3, 2), uint3(2, 3, 5), uint3(2, 5, 4), uint3(4, 5, 6)
};

struct GrassPush
{
    Frame* frame;     // the pass 1 block
    uint* counts;     // per cell of the window: its patches; from `window_size²` on, its first patch
    uint* patches;    // per patch: its cell << 2 | its quarter of the cell's blades
    uint* args;       // the mesh grid (x, y, z), the patch count, the fallback's draw (vertices, instances, first vertex, first instance)
    int2 window;      // the window's first cell (x, z) this frame
    uint window_size; // its side, cells
    uint capacity;    // patches at most
};

[[vk::push_constant]]
GrassPush grass_push;

uint grass_hash(uint a, uint b)
{
    uint n = a * 0x9E3779B9u ^ (b + 0x7F4A7C15u) * 0x85EBCA6Bu;
    n ^= n >> 16;
    n *= 0x7feb352du;
    n ^= n >> 15;
    n *= 0x846ca68bu;
    n ^= n >> 16;
    return n;
}

float grass_unit(uint h, uint shift)
{
    return float((h >> shift) & 0xFFFFu) / 65536.0;
}

struct GrassBlade
{
    float3 root;      // the scene frame
    float2 side;      // across the blade (x, z), unit
    float2 lean;      // the tip's offset along x and z per metre of height (bent quadratically)
    float height;
    float half_width; // drawn, at the root
    float coverage;   // the share of the drawn width the blade covers (under a pixel: dithered)
    uint hash;
    bool alive;
};

// Blade `index` of grass map cell `cell` at the sway's time `time`. Compiled precise in every
// entry that calls it (`PRECISE_ENTRIES`), so the raster's triangles and the resolve's are the
// same to the bit.
GrassBlade grass_blade_at(Frame* f, uint cell, uint index, float time)
{
    GrassCell c = f[0].grass[cell];
    GrassBlade b;
    uint h = grass_hash(cell, index);
    uint h2 = grass_hash(h, 0x51ED270Bu);
    uint h3 = grass_hash(h2, 0x2545F491u);
    b.hash = h;
    uint x = cell % f[0].grass_width;
    uint z = cell / f[0].grass_width;
    float2 at = f[0].grass_origin
        + (float2(float(x), float(z)) + float2(grass_unit(h, 0), grass_unit(h, 16))) * f[0].grass_cell;
    b.root = float3(at.x, c.level, at.y);
    b.height = float((c.look >> 8) & 0xFFu) * 0.01 * (0.55 + 0.9 * grass_unit(h2, 0));
    float2 d = float2(grass_unit(h3, 0), grass_unit(h3, 16)) * 2.0 - 1.0;
    b.side = dot(d, d) > 1e-4 ? normalize(d) : float2(1.0, 0.0);
    b.lean = float2(-b.side.y, b.side.x) * (0.15 + 0.4 * grass_unit(h2, 16));
    // The wind: a gust's wave rolling across the field along the wind's way, each blade a little
    // out of step, bends the tips its way, with a flutter of their own.
    float strength = f[0].grass_wind.z;
    if (strength > 0.0)
    {
        float2 way = f[0].grass_wind.xy;
        uint h4 = grass_hash(h3, 0x1B873593u);
        float gust = 0.55 + 0.45 * sin(dot(at, way) * 0.45 - time * 1.9 + grass_unit(h4, 0) * 1.2);
        float flutter = 0.12 * sin(time * 7.3 + grass_unit(h4, 16) * 6.2831853);
        b.lean += way * (strength * (gust + flutter));
    }
    // At least a pixel wide: a thinner blade covers its share of the pixel.
    float distance = length(b.root - f[0].camera_in_scene.xyz);
    float pixel = 2.0 * distance / (f[0].p00 * float(f[0].target_width));
    float width = GRASS_WIDTH * (0.75 + 0.5 * grass_unit(grass_hash(h3, 0x68E31DA4u), 0));
    b.half_width = 0.5 * max(width, pixel);
    b.coverage = width / max(width, pixel);
    // Over the last fifth of the reach the blades sink into the ground's own grass.
    b.height *= saturate((f[0].grass_reach - distance) / (0.2 * f[0].grass_reach));
    // None stands in the cell's bare ninths, past its lawn's edge or by a wall.
    uint ninth = min(uint(grass_unit(h, 0) * 3.0), 2u) + 3u * min(uint(grass_unit(h, 16) * 3.0), 2u);
    b.alive = index < min(c.look & 0xFFu, GRASS_CELL_BLADES) && b.height > 0.005
        && ((c.look >> (23u + ninth)) & 1u) == 0u;
    return b;
}

// Blade `index` of grass map cell `cell` now.
GrassBlade grass_blade(Frame* f, uint cell, uint index)
{
    return grass_blade_at(f, cell, index, f[0].grass_wind.w);
}

// Vertex `k` of blade `b` (`GRASS_TRIANGLES`), the scene frame.
float3 grass_vertex(GrassBlade b, uint k)
{
    uint level = min(k / 2u, GRASS_SEGMENTS);
    float t = float(level) / float(GRASS_SEGMENTS);
    float3 p = b.root + float3(b.lean.x * t * t, t, b.lean.y * t * t) * b.height;
    if (k < 2u * GRASS_SEGMENTS)
    {
        float w = b.half_width * (1.0 - 0.8 * t);
        p.xz += b.side * ((k & 1u) != 0u ? w : -w);
    }
    return p;
}

// The id of triangle `j` of blade `index` of `cell`.
uint grass_id(uint cell, uint index, uint j)
{
    return VISIBILITY_GRASS_BIT | (cell << 10) | (index << 3) | j;
}

// Per cell of the window round the camera: the patches of its blades within the reach and the
// frustum, 0 for the others.
[shader("compute")]
[numthreads(64, 1, 1)]
void grass_count_main(uint3 id : SV_DispatchThreadID)
{
    uint size = grass_push.window_size;
    if (id.x >= size * size)
        return;
    Frame* f = grass_push.frame;
    int2 c = grass_push.window + int2(int(id.x % size), int(id.x / size));
    uint width = f[0].grass_width;
    uint patches = 0;
    if (all(c >= 0) && uint(c.x) < width && uint(c.y) < f[0].grass_depth)
    {
        uint cell = uint(c.y) * width + uint(c.x);
        GrassCell g = f[0].grass[cell];
        uint count = min(g.look & 0xFFu, GRASS_CELL_BLADES);
        float tallest = float((g.look >> 8) & 0xFFu) * 0.01 * 1.45;
        float2 middle = f[0].grass_origin + (float2(c) + 0.5) * f[0].grass_cell;
        float3 centre = float3(middle.x, g.level + 0.5 * tallest, middle.y) - f[0].camera_in_scene.xyz;
        float radius = 0.71 * f[0].grass_cell + tallest;
        float closest = length(centre) - radius;
        if (count > 0u && closest < f[0].grass_reach && sphere_in_frustum(f, centre, radius))
            patches = (count + GRASS_PATCH_BLADES - 1u) / GRASS_PATCH_BLADES;
    }
    grass_push.counts[id.x] = patches;
}

groupshared uint g_grass_runs[256];

// Each window cell's first patch, in the window's order (the same patches, ids and image from
// run to run), and the draws' arguments.
[shader("compute")]
[numthreads(256, 1, 1)]
void grass_scan_main(uint thread : SV_GroupThreadID)
{
    uint n = grass_push.window_size * grass_push.window_size;
    uint per = (n + 255u) / 256u;
    uint start = min(thread * per, n);
    uint end = min(start + per, n);
    uint sum = 0;
    for (uint i = start; i < end; i++)
        sum += grass_push.counts[i];
    g_grass_runs[thread] = sum;
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0u)
    {
        uint run = 0;
        for (uint t = 0; t < 256u; t++)
        {
            uint s = g_grass_runs[t];
            g_grass_runs[t] = run;
            run += s;
        }
        uint total = min(run, grass_push.capacity);
        grass_push.args[0] = min(total, GRASS_GRID_ROW);
        grass_push.args[1] = (total + GRASS_GRID_ROW - 1u) / GRASS_GRID_ROW;
        grass_push.args[2] = 1u;
        grass_push.args[3] = total;
        grass_push.args[4] = total * GRASS_PATCH_TRIANGLES * 3u;
        grass_push.args[5] = 1u;
        grass_push.args[6] = 0u;
        grass_push.args[7] = 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    uint first = g_grass_runs[thread];
    for (uint i = start; i < end; i++)
    {
        grass_push.counts[n + i] = first;
        first += grass_push.counts[i];
    }
}

// Each window cell's patches, from its first.
[shader("compute")]
[numthreads(64, 1, 1)]
void grass_emit_main(uint3 id : SV_DispatchThreadID)
{
    uint size = grass_push.window_size;
    uint n = size * size;
    if (id.x >= n)
        return;
    uint patches = grass_push.counts[id.x];
    if (patches == 0u)
        return;
    int2 c = grass_push.window + int2(int(id.x % size), int(id.x / size));
    uint cell = uint(c.y) * grass_push.frame[0].grass_width + uint(c.x);
    uint first = grass_push.counts[n + id.x];
    for (uint k = 0; k < patches; k++)
    {
        if (first + k < grass_push.capacity)
            grass_push.patches[first + k] = (cell << 2) | k;
    }
}

struct GrassVertexOut
{
    float4 position : SV_Position;
    nointerpolation float coverage : GRASS_COVERAGE;
};

struct GrassPrimitiveOut
{
    uint id : SV_PrimitiveID;
    bool cull : SV_CullPrimitive;
};

// One workgroup a patch: up to 16 blades of 7 vertices and 5 triangles.
[shader("mesh")]
[outputtopology("triangle")]
[numthreads(32, 1, 1)]
void grass_mesh_main(
    uint thread : SV_GroupThreadID,
    uint3 group : SV_GroupID,
    out vertices GrassVertexOut verts[GRASS_PATCH_VERTICES],
    out indices uint3 tris[GRASS_PATCH_TRIANGLES],
    out primitives GrassPrimitiveOut prims[GRASS_PATCH_TRIANGLES])
{
    Frame* f = grass_push.frame;
    uint index = group.y * GRASS_GRID_ROW + group.x;
    uint blades = 0;
    uint cell = 0;
    uint first = 0;
    if (index < grass_push.args[3])
    {
        uint patch = grass_push.patches[index];
        cell = patch >> 2;
        first = (patch & 3u) * GRASS_PATCH_BLADES;
        uint count = min(f[0].grass[cell].look & 0xFFu, GRASS_CELL_BLADES);
        blades = count > first ? min(count - first, GRASS_PATCH_BLADES) : 0u;
    }
    // One call at the top level (#28).
    SetMeshOutputCounts(blades * GRASS_BLADE_VERTICES, blades * GRASS_BLADE_TRIANGLES);
    float3 camera = f[0].camera_in_scene.xyz;
    for (uint v = thread; v < blades * GRASS_BLADE_VERTICES; v += 32u)
    {
        GrassBlade b = grass_blade(f, cell, first + v / GRASS_BLADE_VERTICES);
        verts[v].position = mul(f[0].view_proj, float4(grass_vertex(b, v % GRASS_BLADE_VERTICES) - camera, 1));
        verts[v].coverage = b.coverage;
    }
    for (uint t = thread; t < blades * GRASS_BLADE_TRIANGLES; t += 32u)
    {
        uint blade = t / GRASS_BLADE_TRIANGLES;
        uint j = t % GRASS_BLADE_TRIANGLES;
        tris[t] = GRASS_TRIANGLES[j] + blade * GRASS_BLADE_VERTICES;
        prims[t].id = grass_id(cell, first + blade, j);
        prims[t].cull = !grass_blade(f, cell, first + blade).alive;
    }
}

// Whether the frame's noise keeps a fragment of a blade covering `coverage` of its drawn width.
bool grass_kept(float2 position, float coverage)
{
    return coverage >= 1.0 || spatio_temporal_noise(uint2(position), grass_push.frame[0].noise_frame).x < coverage;
}

[shader("fragment")]
uint grass_frag_main(float4 position : SV_Position, nointerpolation float coverage : GRASS_COVERAGE, uint id : SV_PrimitiveID) : SV_Target
{
    if (!grass_kept(position.xy, coverage))
        discard;
    return id;
}

struct GrassFallbackOut
{
    float4 position : SV_Position;
    nointerpolation float coverage : GRASS_COVERAGE;
    nointerpolation uint id : GRASS_ID;
};

// The fallback's blades (D-003): one non-indexed draw of every patch's 80 triangles, a dead
// blade's left outside the view.
[shader("vertex")]
GrassFallbackOut grass_vertex_main(uint vertex : SV_VulkanVertexID)
{
    Frame* f = grass_push.frame;
    uint tri = vertex / 3u;
    uint patch = grass_push.patches[tri / GRASS_PATCH_TRIANGLES];
    uint local = tri % GRASS_PATCH_TRIANGLES;
    uint cell = patch >> 2;
    uint index = (patch & 3u) * GRASS_PATCH_BLADES + local / GRASS_BLADE_TRIANGLES;
    uint j = local % GRASS_BLADE_TRIANGLES;
    GrassBlade b = grass_blade(f, cell, index);
    GrassFallbackOut o;
    o.position = b.alive
        ? mul(f[0].view_proj, float4(grass_vertex(b, GRASS_TRIANGLES[j][vertex % 3u]) - f[0].camera_in_scene.xyz, 1))
        : float4(2.0, 2.0, 0.5, 1.0);
    o.coverage = b.coverage;
    o.id = grass_id(cell, index, j);
    return o;
}

[shader("fragment")]
uint grass_frag_fallback_main(float4 position : SV_Position, nointerpolation float coverage : GRASS_COVERAGE, nointerpolation uint id : GRASS_ID) : SV_Target
{
    if (!grass_kept(position.xy, coverage))
        discard;
    return id;
}

// A blade's surface at a pixel, rebuilt from its id as the raster drew it.
struct GrassHit
{
    float3 relative;  // to the camera
    float3 scene;     // the scene frame
    float3 n;
    float3 dpdx;      // the scene position's change per pixel
    float3 dpdy;
    float along;      // up the blade, 0 at its root to 1 at its tip
    uint hash;
    uint tint;        // its cell's: the row after the map's first its blades take
    float far;        // how far it stands, a share of the reach
    uint cell;        // its cell: its clump's colour
};

GrassHit grass_hit(Frame* f, uint vis, uint2 pixel, float2 size)
{
    uint cell = (vis >> 10) & ((1u << GRASS_CELL_BITS) - 1u);
    uint index = (vis >> 3) & (GRASS_CELL_BLADES - 1u);
    uint3 k = GRASS_TRIANGLES[min(vis & 7u, GRASS_BLADE_TRIANGLES - 1u)];
    GrassBlade b = grass_blade(f, cell, index);
    float3 camera = f[0].camera_in_scene.xyz;
    float3 p0 = grass_vertex(b, k.x) - camera;
    float3 p1 = grass_vertex(b, k.y) - camera;
    float3 p2 = grass_vertex(b, k.z) - camera;
    float4 c0 = mul(f[0].view_proj, float4(p0, 1));
    float4 c1 = mul(f[0].view_proj, float4(p1, 1));
    float4 c2 = mul(f[0].view_proj, float4(p2, 1));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    Barycentrics bc = barycentrics(c0, c1, c2, ndc, size);
    GrassHit h;
    h.relative = bc.lambda.x * p0 + bc.lambda.y * p1 + bc.lambda.z * p2;
    h.scene = h.relative + camera;
    h.dpdx = bc.ddx.x * p0 + bc.ddx.y * p1 + bc.ddx.z * p2;
    h.dpdy = bc.ddy.x * p0 + bc.ddy.y * p1 + bc.ddy.z * p2;
    h.along = saturate((h.scene.y - b.root.y) / max(b.height, 1e-4));
    h.hash = b.hash;
    h.tint = (f[0].grass[cell].look >> 16) & 0x7Fu;
    h.cell = cell;
    // The blade's face, turned to the viewer, rounded across it: its edges' normals lean out.
    float t = h.along;
    float3 up = normalize(float3(2.0 * b.lean.x * t, 1.0, 2.0 * b.lean.y * t));
    float3 side = float3(b.side.x, 0.0, b.side.y);
    float3 face = normalize(cross(side, up));
    if (dot(face, -h.relative) < 0.0)
        face = -face;
    float3 across = float3(k.x == 6u ? 0.0 : ((k.x & 1u) != 0u ? 1.0 : -1.0),
                           k.y == 6u ? 0.0 : ((k.y & 1u) != 0u ? 1.0 : -1.0),
                           k.z == 6u ? 0.0 : ((k.z & 1u) != 0u ? 1.0 : -1.0));
    float outward = dot(bc.lambda, across);
    float3 n = normalize(face + side * (0.6 * outward));
    // Far blades take their clump's normal, up (Ghost of Tsushima's rule against shimmer): all of
    // it by `GRASS_CONVERGED`, or by the reach where the ground's own grass takes over.
    float distance = length(h.relative);
    h.far = saturate(distance / min(max(f[0].grass_reach, 1.0), GRASS_CONVERGED));
    h.n = normalize(lerp(n, float3(0.0, 1.0, 0.0), 0.45 + 0.55 * sqrt(h.far)));
    return h;
}

// The cut-outs' alpha test's mip bias (`cutout_kept`): one level finer than the colour's.
static const float CUTOUT_MIP_BIAS = -1.0;

// Whether a fragment of the cut-outs' raster stays (#171): a single-sided row's back faces go
// (the cut-outs' pipeline culls no face), and a cut-out row's pixels whose base colour alpha,
// the factor's times its map's at the pixel's UV, falls under its cutoff; and the windowed
// ground's in the ground window, at `ground` in its mesh's frame (#197). A grating's pixels by
// the share its bars cover (`grating_coverage`) against the frame's spatio-temporal noise at
// `pixel`, `duv_dx` and `duv_dy` the UV's change per pixel and `slant` the view's (`card_slant`).
bool cutout_kept(Frame* f, uint id, float2 uv, bool front, float2 ground, float2 duv_dx, float2 duv_dy, float slant, uint2 pixel)
{
    uint slot = id >> VISIBILITY_TRIANGLE_BITS;
    uint2 record = f[0].visible[slot];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Instance inst = f[0].instances[record.x];
    if (in_ground_window(f, inst.mesh, ground))
        return false;
    // An instance crossing its ladder (D-075): its own mesh's pixels where the noise is at or
    // above the crossing, the next mesh's where it is below. Its next mesh's clusters lie outside
    // its own mesh's run.
    if (ladder_crossing_now(f, record.x, inst))
    {
        float value = f[0].ladder[record.x];
        Mesh* mesh = f[0].meshes + ladder_first(f, record.x, inst);
        uint cluster = record.y & MESHLET_INDEX_MASK;
        bool next = cluster - mesh[0].meshlet_offset >= mesh[0].meshlet_count;
        if (next != (spatio_temporal_noise(pixel, f[0].noise_frame).x < ladder_t(value)))
            return false;
        // The fade after the crossing takes its share of both by a noise of its own; the
        // impostor keeps the complement (`impostor_draw.slang`, #363).
        if (ladder_fading(value) && ladder_fade_noise(pixel, f[0].noise_frame) < ladder_fade_share(value))
            return false;
    }
    Material mat = f[0].materials[inst.material + triangle_section(m, id & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u))];
    // Drawn after the resolve, over what this pass finds behind it (D-065's glass).
    if ((mat.flags & MATERIAL_FLAG_DRAWN_AFTER) != 0)
        return false;
    if (!front && (mat.flags & MATERIAL_FLAG_DOUBLE_SIDED) == 0)
        return false;
    if ((mat.flags & MATERIAL_FLAG_MASKED) == 0)
        return true;
    if ((mat.flags & MATERIAL_FLAG_GRATING) != 0)
    {
        // Kept or dropped by turns over the frames in the share the bars cover: TAA averages
        // them into the bars' grey where they are thinner than a pixel, and they are sharp
        // where wider. As geometry they flickered past 18 m (D-013, trim sheets §2.9).
        float u = dot(mat.uv_u, float3(uv, 1.0));
        float footprint = abs(dot(mat.uv_u.xy, duv_dx)) + abs(dot(mat.uv_u.xy, duv_dy));
        float duty = mat.alpha * (1.0 + slant);
        float cover = grating_coverage(u, footprint, duty);
        // Bars two pixels wide or more keep a sharp edge, as geometry's, which TAA's jitter
        // smooths; the noise along it speckled. Thinner bars are kept by turns.
        float noise = spatio_temporal_noise(pixel, f[0].noise_frame).y;
        return cover > lerp(0.5, noise, saturate(2.0 - duty / max(footprint, 1e-6)));
    }
    // A mip finer than the colour's: a leaf card a few pixels across took a level whose alpha,
    // filtered, fell under the cutoff for most of a sparse crown's leaves, whatever their
    // coverage-preserving scale, and the young holm oaks 200 m off kept 2–5 % of their pixels
    // in their dark cores' shade (55–63 % one level finer, as supersampled and as their shells,
    // #363).
    float alpha = mat.alpha;
    if (mat.albedo_texture != TEXTURE_NONE)
    {
        float2 t = float2(dot(mat.uv_u, float3(uv, 1.0)), dot(mat.uv_v, float3(uv, 1.0)));
        // By the UV's change across the pixel the caller took where the quad was whole: this
        // runs after the ladder's and the ground window's early returns, where a quad's pixels
        // part ways and the implicit derivatives are undefined.
        float scale = exp2(CUTOUT_MIP_BIAS);
        float2 tdx = float2(dot(mat.uv_u.xy, duv_dx), dot(mat.uv_v.xy, duv_dx)) * scale;
        float2 tdy = float2(dot(mat.uv_u.xy, duv_dy), dot(mat.uv_v.xy, duv_dy)) * scale;
        alpha *= g_sampled_images[NonUniformResourceIndex(mat.albedo_texture)].SampleGrad(g_samplers[NonUniformResourceIndex(mat.sampler)], t, tdx, tdy).a;
    }
    return alpha >= mat.alpha_cutoff;
}

[shader("fragment")]
uint frag_cutout_main(float4 position : SV_Position, float2 uv : TEXCOORD0, float2 ground : TEXCOORD1, float slant : TEXCOORD2, uint id : SV_PrimitiveID, bool front : SV_IsFrontFace) : SV_Target
{
    float2 dx = ddx(uv);
    float2 dy = ddy(uv);
    if (!cutout_kept(push.frame, id, uv, front, ground, dx, dy, slant, uint2(position.xy)))
        discard;
    return id;
}

[shader("fragment")]
uint frag_fallback_cutout_main(float4 position : SV_Position, nointerpolation uint id_base : VISIBILITY_ID, float2 uv : TEXCOORD0, float2 ground : TEXCOORD1, float slant : TEXCOORD2, uint triangle : SV_PrimitiveID, bool front : SV_IsFrontFace) : SV_Target
{
    uint id = id_base | triangle;
    float2 dx = ddx(uv);
    float2 dy = ddy(uv);
    if (!cutout_kept(push.frame, id, uv, front, ground, dx, dy, slant, uint2(position.xy)))
        discard;
    return id;
}

// The share of [u - footprint/2, u + footprint/2] a grating's bars cover, `duty` of a unit wide
// and centred on the integers: the bars' count up to y = u + duty/2 is floor(y)·duty +
// min(frac(y), duty), differenced over the footprint (a box filter, exact for the stripes).
float grating_coverage(float u, float footprint, float duty)
{
    if (duty >= 1.0)
        return 1.0;
    float y = u + 0.5 * duty;
    if (footprint < 1e-4)
        return frac(y) < duty ? 1.0 : 0.0;
    float y0 = y - 0.5 * footprint;
    float y1 = y + 0.5 * footprint;
    float a = floor(y0) * duty + min(frac(y0), duty);
    float b = floor(y1) * duty + min(frac(y1), duty);
    return saturate((b - a) / footprint);
}

// ------------------------------------------------------------------- merge ---

// Writes what the software rasteriser kept in this pass into the hardware's targets. Every
// sample it kept beats the hardware's pixel (`write_software_sample`), so the depth test
// (`GREATER_OR_EQUAL`) passes; the sample is cleared for the next pass, which leaves the
// buffer empty at the end of the frame. Drawn only when the pass has software clusters. A pass
// with few of them (pass 2, issue #32) merges a rectangle per cluster: its bounding sphere's,
// as the cull projected it, grown by a pixel for the vertices' snapping. Where rectangles
// overlap, the second fragment finds the sample taken, or writes the same one again. A pass
// with many merges the whole target with one triangle: marking the tiles the software
// rasteriser touched measured slower where it pays (full detail: the marking cost more than
// the merge).
static const uint MERGE_RECT_CORNERS[6] = { 0, 1, 2, 2, 1, 3 }; // two triangles; x from bit 0, y from bit 1

[shader("vertex")]
float4 merge_vert_main(uint id : SV_VertexID, uint cluster : SV_InstanceID) : SV_Position
{
    Frame* f = push.frame;
    if (pass_args(f)[8] == 3u)
        return float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    uint slot = f[0].raster[f[0].visible_capacity - 1 - cluster];
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    float3 center = instance_point(f, inst, m.center);
    float3 c = mul(f[0].cull_view, float4(center, 1)).xyz;
    float2 size = float2(float(f[0].target_width), float(f[0].target_height));
    float4 rect = float4(0.0, 0.0, size); // pixels; the whole target should the projection fail
    float4 aabb;
    if (project_sphere(c, m.radius * inst.scale, f[0].znear, f[0].p00, f[0].p11, aabb))
    {
        aabb += f[0].draw_jitter.xyxy;
        rect = clamp(float4(floor(aabb.xy * size) - 1.0, ceil(aabb.zw * size) + 1.0), 0.0, size.xyxy);
    }
    uint corner = MERGE_RECT_CORNERS[id];
    float2 pixel = float2((corner & 1u) != 0 ? rect.z : rect.x, (corner & 2u) != 0 ? rect.w : rect.y);
    // The viewport is flipped (`set_viewport_full`): clip y is up.
    return float4(pixel.x / size.x * 2.0 - 1.0, 1.0 - pixel.y / size.y * 2.0, 0.0, 1.0);
}

struct MergeOut
{
    uint id : SV_Target0;
    float depth : SV_Depth;
};

[shader("fragment")]
MergeOut merge_main(float4 position : SV_Position)
{
    uint index = vis64_index(push.target_width, uint2(position.xy));
    if (vis64_depth_bits(push.vis64, index) == 0)
        discard; // no software sample: the hardware's pixel stays (half the bytes read)
    uint64_t* cell = push.vis64 + index;
    uint64_t value = cell[0];
    cell[0] = 0;
    MergeOut o;
    o.id = uint(value);
    o.depth = asfloat(uint(value >> 32));
    return o;
}

// ------------------------------------------------------- software rasteriser ---

// The hardware's snap of framebuffer positions: 8 bits of sub-pixel precision
// (`subPixelPrecisionBits` on NVIDIA and AMD), so that a triangle covers the same pixels
// whichever rasteriser draws it.
static const int SUBPIXEL_BITS = 8;
static const int SUBPIXEL_ONE = 1 << SUBPIXEL_BITS;
// Largest triangle extent the rasteriser takes, in sub-pixels. Its edge functions are 32-bit:
// a product of two coordinate differences stays below 2^30 and their difference below 2^31.
static const int SW_RASTER_MAX_EXTENT = 96 * SUBPIXEL_ONE;
static const uint SW_RASTER_GROUP = 64; // = MAX_VERTICES: a thread per vertex, then per triangle

groupshared int2 s_sw_position[MAX_VERTICES]; // framebuffer position, snapped to 1/256 px
groupshared float s_sw_depth[MAX_VERTICES];   // depth (z / w); negative behind the camera

// Keeps `depth << 32 | id` at `pixel` when it beats the hardware's pixel of this pass (drawn
// before the software raster runs) and the software samples already there: nearer or, at
// equal depth, a larger id (the hardware's rule: its depth test keeps the last drawn, and it
// draws in id order).
void write_software_sample(uint2 pixel, float depth, uint id)
{
    float hardware_depth = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)].Load(int3(pixel, 0));
    if (depth < hardware_depth)
        return;
    if (depth == hardware_depth
        && id < g_sampled_images_u32[NonUniformResourceIndex(push.vis_image)].Load(int3(pixel, 0)))
        return;
    InterlockedMax(push.vis64[vis64_index(push.target_width, pixel)], (uint64_t(asuint(depth)) << 32) | id);
}

// Top-left fill rule for edge a -> b of a triangle wound clockwise on screen (y down), the
// hardware's rule for samples exactly on an edge: a top edge is horizontal with the triangle
// below it, a left edge goes up.
bool top_left(int2 a, int2 b)
{
    int2 d = b - a;
    return (d.y == 0 && d.x > 0) || d.y < 0;
}

// Edge function of a -> b at p (twice the signed area of a, b, p): positive inside a triangle
// wound clockwise on screen.
int edge(int2 a, int2 b, int2 p)
{
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
}

// Rasterises one triangle as the hardware pipelines do: back faces culled (the front face is
// counter-clockwise in Vulkan's framebuffer orientation, clockwise on screen once v1 and v2
// are swapped), a sample at every pixel centre inside or on a top-left edge, depth
// interpolated linearly in screen space.
void rasterise_triangle(Frame* f, uint i0, uint i1, uint i2, uint id)
{
    int2 p0 = s_sw_position[i0];
    int2 p1 = s_sw_position[i2]; // swapped: a front face becomes clockwise on screen
    int2 p2 = s_sw_position[i1];
    float z0 = s_sw_depth[i0];
    float z1 = s_sw_depth[i2];
    float z2 = s_sw_depth[i1];
    if (min(z0, min(z1, z2)) < 0.0)
        return; // behind the camera: never routed here (the near plane test), and not drawable
    int2 lo = min(p0, min(p1, p2));
    int2 hi = max(p0, max(p1, p2));
    if (any(hi - lo > SW_RASTER_MAX_EXTENT))
        return; // never routed here (`dense_cluster`); keeps the arithmetic exact
    int area = edge(p0, p1, p2);
    if (area <= 0)
        return; // back-facing or degenerate
    // Pixels whose centre (x + 0.5) lies in [lo, hi], clipped to the target.
    int2 half_pixel = SUBPIXEL_ONE / 2;
    int2 first = max((lo - half_pixel + SUBPIXEL_ONE - 1) >> SUBPIXEL_BITS, int2(0, 0));
    int2 last = min((hi - half_pixel) >> SUBPIXEL_BITS, int2(f[0].target_width, f[0].target_height) - 1);
    if (any(last < first))
        return;
    // Edge functions at the first sample, biased so that "covered" is ">= 0" for all three;
    // a step of one pixel adds a constant.
    int2 sample = first * SUBPIXEL_ONE + half_pixel;
    int3 bias = int3(top_left(p1, p2) ? 0 : -1, top_left(p2, p0) ? 0 : -1, top_left(p0, p1) ? 0 : -1);
    int3 row = int3(edge(p1, p2, sample), edge(p2, p0, sample), edge(p0, p1, sample));
    int3 step_x = -int3(p2.y - p1.y, p0.y - p2.y, p1.y - p0.y) * SUBPIXEL_ONE;
    int3 step_y = int3(p2.x - p1.x, p0.x - p2.x, p1.x - p0.x) * SUBPIXEL_ONE;
    float inv_area = 1.0 / float(area);
    for (int y = first.y; y <= last.y; y++)
    {
        int3 e = row;
        for (int x = first.x; x <= last.x; x++)
        {
            if (all(e + bias >= 0))
            {
                // Barycentric weights of p1 and p2 from their opposite edges.
                float depth = saturate(z0 + (z1 - z0) * (float(e.y) * inv_area) + (z2 - z0) * (float(e.z) * inv_area));
                if (depth > 0.0) // 0 is the empty sample (and the far plane, where nothing is)
                    write_software_sample(uint2(x, y), depth, id);
            }
            e += step_x;
        }
        row += step_y;
    }
}

// One workgroup per software-rasterised cluster of this pass (the back of `Frame::raster`):
// its vertices are transformed and snapped once, then each thread rasterises triangles.
[shader("compute")]
[numthreads(SW_RASTER_GROUP, 1, 1)]
void sw_raster_main(uint thread : SV_GroupThreadID, uint3 group : SV_GroupID)
{
    Frame* f = push.frame;
    uint index = grid_index(group);
    if (index >= (pass_args(f) + 4)[3])
        return; // the partial last row of the grid (uniform)
    uint slot = f[0].raster[f[0].visible_capacity - 1 - index];
    uint2 record = f[0].visible[slot];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Instance inst = f[0].instances[record.x];
    uint base = payload_base(f, m);
    if (thread < m.vertex_count)
    {
        // The mesh shader's transform, then the fixed-function steps: perspective division,
        // the viewport of `set_viewport_full` (y flipped) and the snap to sub-pixels.
        PagedVertex v = paged_vertex(f, base, m, thread);
        float3 relative = instance_point(f, inst, v.position);
        float4 clip = mul(f[0].view_proj, float4(relative, 1));
        float3 ndc = clip.xyz / clip.w;
        float2 half_size = float2(f[0].target_width, f[0].target_height) * 0.5;
        float2 framebuffer = float2(ndc.x * half_size.x + half_size.x, half_size.y - ndc.y * half_size.y);
        s_sw_position[thread] = int2(round(framebuffer * float(SUBPIXEL_ONE)));
        s_sw_depth[thread] = clip.w > 0.0 ? ndc.z : -1.0;
    }
    GroupMemoryBarrierWithGroupSync();
    uint id_base = slot << VISIBILITY_TRIANGLE_BITS;
    for (uint t = thread; t < m.triangle_count; t += SW_RASTER_GROUP)
    {
        rasterise_triangle(f,
            paged_index(f, base, m, t, 0),
            paged_index(f, base, m, t, 1),
            paged_index(f, base, m, t, 2),
            id_base | t);
    }
}

// ------------------------------------------------------------------ shading ---
//
// Every pixel is shaded by the pass of its material's class (issue #20, D-026), reading the
// instance's row of the material table (D-007, crates/forge-core/src/material.rs): the
// standard class in the full-screen pass, which also lists the 8×8 tiles showing any other
// class, and each other class in a dispatch over its tiles. The lighting is the space model the rocks have had
// since Phase 0 (a hard sun, a little wrap and fill, a Blinn-Phong highlight); it waits for
// the lighting phase.

// The sun's light on a surface of `albedo` with normal `n`, in units of a white Lambertian
// surface facing the sun. Returns the light and `ndl`, which the ice's scattering needs.
// Ambient occlusion with the light that bounces inside the occluded region: bright surfaces
// lose less than their visibility says (Jimenez et al. 2016, the cubic fit of their
// multi-bounce reference, per colour channel).
float3 occlusion_bounce(float visibility, float3 albedo)
{
    float3 a = 2.0404 * albedo - 0.3324;
    float3 b = -4.7951 * albedo + 0.6417;
    float3 c = 2.7552 * albedo + 0.6903;
    return max(float3(visibility), ((visibility * a + b) * visibility + c) * visibility);
}

// Set by `sky_reflection` for the pixel being shaded: the mirror direction (xyz) and the weight
// of what a mirror ray meets there in place of the sky (w; issue #52). The resolve stores it for
// the smooth rows, and `reflections_main` traces it.
static float4 g_reflection_request;

// Set by `lamps_at` for the pixel being shaded (D-065): the lamps' light on it in the resolve's
// units (a white Lambertian surface facing the sun is 1), and their highlights; `sun_light` adds
// both, as it does the sun's.
static float3 g_lamp_diffuse = float3(0.0);
static float3 g_lamp_specular = float3(0.0);

// Geometric specular anti-aliasing (#331; Tokuyoshi and Kaplanyan, "Improved Geometric Specular
// Antialiasing", I3D 2019): how much the interpolated normal turns across the pixel being shaded,
// as a kernel's roughness² (min(2σ²(|∂n/∂x|² + |∂n/∂y|²), κ)), set where its surface is rebuilt
// from the mesh (0 elsewhere). The sun's and the lamps' highlights widen by it (`aa_power`), their
// energy kept, so a curved glossy body's highlight no longer shrinks under a pixel and flickers as
// it moves; the reflections keep their roughness, which the mirror rays' shares follow.
static float g_normal_variance = 0.0;
static const float GEOMETRIC_AA_SIGMA2 = 0.15915494; // 1 / 2π, the paper's screen-space variance
static const float GEOMETRIC_AA_KAPPA = 0.18;         // the paper's clamp

// A Blinn-Phong exponent widened by `g_normal_variance` (GGX's α² = 2 / (power + 2) and back).
float aa_power(float power)
{
    float alpha2 = 2.0 / (power + 2.0);
    return 2.0 / saturate(alpha2 + g_normal_variance) - 2.0;
}

// The highlights' model (D-086, brdf.slang): GGX's α² for a row of Blinn-Phong power `power`, widened by the normals' spread under the
// pixel (`g_normal_variance`, as `aa_power` widens the exponent) and by `extra` (a light's
// size: the sun's disc).
float ggx_alpha2(float power, float extra)
{
    return saturate(2.0 / (power + 2.0) + max(g_normal_variance, 0.0) + extra);
}

// A Blinn-Phong highlight with its exponent widened by `g_normal_variance`, its energy kept (the
// lobe's integral goes as 1 / (power + 2)).
float aa_highlight(float nh, float power)
{
    // A flat surface keeps its highlight to the bit.
    if (g_normal_variance <= 0.0)
        return pow(max(nh, 0.0), power);
    float widened = max(aa_power(power), 0.001);
    return pow(max(nh, 0.0), widened) * ((widened + 2.0) / (power + 2.0));
}

// Where a surface at `world` (the scene's frame) facing `n` takes its lamps from (D-065): with
// `probes` and their regions (D-068), its region and the one across a doorway, whose lamps light
// it (`lamp_lights`: a house's rooms are its light lists); the runs of the light lists they name
// (`lamp_runs`).
struct LampPlace
{
    uint region;
    uint other;
    LampRuns runs;
};

LampPlace lamp_place(Frame* f, ProbeField* probes, float3 world, float3 n, float3 view)
{
    LampPlace place;
    place.region = REGION_NONE;
    place.other = REGION_NONE;
    if (probes != nullptr && probes[0].region_count > 0u)
    {
        // A face whose point 2 cm off it misses its room's outline (the house's body: a wall's
        // face set back from it) looks 15 cm off: taken as the body's, it took every lamp of
        // the frame, and its own room's ones fell to the picked rest in streaks. Still in the
        // body (a thin side face set into the wall), the side it is seen from, 30 cm towards
        // the camera (`region_open`).
        RegionHit at = region_open(probes, world + n * 0.02, world + n * 0.15, world + view * 0.3);
        if (at.kind != REGION_SOLID)
        {
            place.region = at.region;
            place.other = at.other;
        }
    }
    place.runs = lamp_runs(f, place.region, place.other);
    return place;
}

// The lamps a pixel traces (`lamps_at`, `lamp_rays_main`): of its place's, the three strongest
// there, unshadowed (`lamp_weight`), and one of the others picked by its weight (Shirley, Wang and
// Zimmerman 1996), counted as its light times `scale`, the others' sum over its weight. The
// frame's lamp count where there is none.
struct LampPicks
{
    uint top[3];
    uint chosen;
    float scale;
};

LampPicks lamp_picks(Frame* f, LampPlace place, float3 world, float3 n, uint2 pixel, bool skip_outside)
{
    uint count = f[0].lamp_count;
    LampPicks picks;
    picks.top[0] = count;
    picks.top[1] = count;
    picks.top[2] = count;
    picks.chosen = count;
    picks.scale = 0.0;
    float top_w[3] = { 0.0, 0.0, 0.0 };
    float rest = 0.0;
    for (uint r = 0u; r < 3u; r++)
    {
        for (uint e = place.runs.run[r].x; e < place.runs.run[r].y; e++)
        {
            uint i = lamp_at(f, place.runs, e);
            if (!lamp_counted(f, place.runs, r, i) || (skip_outside && lamp_outside(f[0].lamps[i])))
                continue;
            float w = lamp_weight_in(f[0].lamps[i], world, n, place.region, place.other);
            if (w <= 0.0)
                continue;
            // Into the three, the weakest of them out to the rest.
            uint k = i;
            for (uint j = 0u; j < 3u; j++)
            {
                if (w > top_w[j])
                {
                    float held_w = top_w[j];
                    uint held = picks.top[j];
                    top_w[j] = w;
                    picks.top[j] = k;
                    w = held_w;
                    k = held;
                }
            }
            rest += w;
        }
    }
    if (rest <= 0.0)
        return picks;
    // One of the others, picked by its weight, its light over its chance (w / rest): every
    // lamp counts, at most five shadow rays a pixel.
    float pick = interleaved_gradient_noise(pixel, f[0].noise_frame, float2(0.0)) * rest;
    float chosen_w = 0.0;
    for (uint r = 0u; r < 3u; r++)
    {
        if (picks.chosen < count && pick <= 0.0)
            break;
        for (uint e = place.runs.run[r].x; e < place.runs.run[r].y; e++)
        {
            uint i = lamp_at(f, place.runs, e);
            if (i == picks.top[0] || i == picks.top[1] || i == picks.top[2] || !lamp_counted(f, place.runs, r, i) || (skip_outside && lamp_outside(f[0].lamps[i])))
                continue;
            float w = lamp_weight_in(f[0].lamps[i], world, n, place.region, place.other);
            if (w <= 0.0)
                continue;
            picks.chosen = i;
            chosen_w = w;
            pick -= w;
            if (pick <= 0.0)
                break;
        }
    }
    if (picks.chosen < count)
        picks.scale = rest / chosen_w;
    return picks;
}

// The lamps' light on a surface at `world` (the scene's frame) facing `n`, seen from `view`
// (D-065, crates/forge-render/src/lamps.rs), of the lamps of its place (`lamp_place`) whose reach
// it is in: each one's illuminance (candelas over the distance squared, by the cosine), let go
// smoothly at its reach, and highlights as the sun's (`lamp_add`). `code` (`LampShadows::code`):
// - its high half, ReSTIR DI's light (restir.slang, 0xFFFF: none): one sample of one lamp, its
//   shadow ray traced, in lux, its highlight towards the lamp's middle; of the street lamps,
//   which the rest leaves out (`lamp_outside`), or of every lamp (low half 0xFFFE,
//   `--lamp-restir`);
// - its low half, the penumbra filter's share (the sampled index of `lamp_filter_main`'s
//   output): every lamp unshadowed, without a ray, times the share of their light that reaches
//   the pixel, which the filter took from `lamp_rays_main`'s rays over its neighbours (Heitz et
//   al. 2018);
// - or 0xFFFF (`--no-lamp-filter`): with SHADOWS, the picked lamps' (`lamp_picks`) own shadow rays
//   towards points of their spheres that move with the frame's noise, so TAA averages the
//   penumbra as it did the sun's single ray (#54).
// `n` the surface's interpolated normal, which chooses its lamps, its region and its rays, as
// `lamp_rays_main` and ReSTIR do; `shading_n` its mapped one, which their light takes (D-086).
void lamps_at<let SHADOWS : int>(Frame* f, ProbeField* probes, float3 world, float3 n, float3 shading_n, float3 view, float specular_power, float specular_weight, float3 f0, float bias, uint2 pixel, uint code)
{
    g_lamp_diffuse = float3(0.0);
    g_lamp_specular = float3(0.0);
    uint count = f[0].lamp_count;
    if (count == 0u || f[0].lamps == nullptr)
    {
        if (f[0].far_lamps != nullptr)
            far_lamps_at(f, lamp_place(f, probes, world, n, view), world, n, shading_n, view, specular_power, specular_weight, f0);
        return;
    }
    uint share = code & 0xFFFFu;
    uint restir = code >> 16;
    // The rest's light first: the share scales it alone.
    if (share != 0xFFFEu)
    {
        bool skip_outside = restir != 0xFFFFu;
        LampPlace place = lamp_place(f, probes, world, n, view);
        if (share != 0xFFFFu)
        {
            for (uint r = 0u; r < 3u; r++)
            {
                for (uint e = place.runs.run[r].x; e < place.runs.run[r].y; e++)
                {
                    uint i = lamp_at(f, place.runs, e);
                    if (skip_outside && lamp_outside(f[0].lamps[i]))
                        continue;
                    if (lamp_counted(f, place.runs, r, i) && lamp_weight_in(f[0].lamps[i], world, n, place.region, place.other) > 0.0)
                        lamp_add<0>(f, f[0].lamps[i], i, 1.0, world, n, shading_n, view, specular_power, specular_weight, f0, bias, float2(0.0));
                }
            }
            float3 reaching = g_sampled_images[NonUniformResourceIndex(share)].Load(int3(pixel, 0)).rgb;
            g_lamp_diffuse *= reaching;
            g_lamp_specular *= reaching;
        }
        else
        {
            LampPicks picks = lamp_picks(f, place, world, n, pixel, skip_outside);
            // Where on each lamp's disc: the sun's eight Vogel points, one a frame of TAA's cycle,
            // turned per pixel (`sun_disc_point`, #54); independent points a frame left streaks in
            // the penumbrae.
            float2 u = sun_disc_point(pixel, f[0].noise_frame);
            // The strongest lamp takes two rays, at points half the cycle apart, so TAA sees the
            // eight in four frames where its penumbrae show most; the next two take one each.
            if (picks.top[0] < count)
            {
                float2 u_half = sun_disc_point(pixel, f[0].noise_frame + SUN_DISC_POINTS / 2u);
                lamp_add<SHADOWS>(f, f[0].lamps[picks.top[0]], picks.top[0], 0.5, world, n, shading_n, view, specular_power, specular_weight, f0, bias, u);
                lamp_add<SHADOWS>(f, f[0].lamps[picks.top[0]], picks.top[0], 0.5, world, n, shading_n, view, specular_power, specular_weight, f0, bias, u_half);
            }
            for (uint j = 1u; j < 3u; j++)
            {
                if (picks.top[j] < count)
                    lamp_add<SHADOWS>(f, f[0].lamps[picks.top[j]], picks.top[j], 1.0, world, n, shading_n, view, specular_power, specular_weight, f0, bias, u);
            }
            if (picks.chosen < count)
                lamp_add<SHADOWS>(f, f[0].lamps[picks.chosen], picks.chosen, picks.scale, world, n, shading_n, view, specular_power, specular_weight, f0, bias, u);
        }
        // The street lamps beyond the set, the rest of their light unshadowed (#284); ReSTIR's
        // street lamps are all in the set.
        if (!skip_outside)
            far_lamps_at(f, place, world, n, shading_n, view, specular_power, specular_weight, f0);
    }
    if (restir != 0xFFFFu)
    {
        float4 light = g_sampled_images[NonUniformResourceIndex(restir)].Load(int3(pixel, 0));
        uint i = uint(light.a + 0.5);
        if (i != 0u && i <= count)
        {
            float3 illuminance = light.rgb / max(f[0].sun_illuminance, 1e-9);
            Lamp lamp = f[0].lamps[i - 1u];
            float3 to_lamp = lamp.position_radius.xyz - world;
            float dist = length(to_lamp);
            float3 dir = to_lamp / max(dist, 1e-6);
            g_lamp_diffuse += illuminance;
            // Its light at normal incidence (ReSTIR's carries the interpolated normal's n·l)
            // through GGX at the mapped normal (D-086).
            float size = 0.5 * min(lamp.position_radius.w / max(dist, 1e-3), 1.0);
            g_lamp_specular += ggx_light(shading_n, view, dir, ggx_alpha2(specular_power, size * size), f0)
                * highlight_share(specular_weight) * illuminance / max(dot(n, dir), 1e-3);
        }
    }
}

// Adds lamp `lamp` (the frame's `i`-th) to `g_lamp_diffuse` and `g_lamp_specular`, times `scale`
// (`lamps_at`): its illuminance on the surface, with SHADOWS one shadow ray to a point of its
// sphere that moves with the frame's noise `u`.
// `n` the surface's interpolated normal (its shadow ray's start, and none from behind it),
// `shading_n` its mapped one, which the light takes (`lamps_at`).
void lamp_add<let SHADOWS : int>(Frame* f, Lamp lamp, uint i, float scale, float3 world, float3 n, float3 shading_n, float3 view, float specular_power, float specular_weight, float3 f0, float bias, float2 u)
{
    float3 d = lamp.position_radius.xyz - world;
    float dist2 = dot(d, d);
    float reach = lamp.colour_reach.w;
    float dist = sqrt(dist2);
    float3 dir = d / max(dist, 1e-6);
    float ndl = dot(n, dir);
    float radius = lamp.position_radius.w;
    if (SHADOWS != 0 && f[0].tlas != 0)
    {
        // A point of the sphere's disc facing the surface, uniform in area, its angle turned for
        // each lamp so two lamps' samples do not move together.
        float3 t;
        float3 b;
        lamp_disc_axes(dir, t, b);
        float r = radius * sqrt(u.x);
        float a = 6.28318530718 * frac(u.y + 0.618034 * float(i));
        if (ray_blocked(f, lamp_ray(lamp, world, n, bias, (t * cos(a) + b * sin(a)) * r)))
            return;
    }
    // Let go between 80 % and all of its reach, so its edge does not show.
    float window = saturate(1.0 - pow(dist / reach, 4.0));
    float to_units = 1.0 / max(f[0].sun_illuminance, 1e-9);
    float3 normal_light = lamp.colour_reach.rgb / max(dist2, radius * radius) * (window * window * to_units * scale * lamp_beam(lamp, dir));
    // None behind the surface itself: a mapped normal leaning towards a lamp round the corner of
    // its face would add it unshadowed.
    if (ndl <= 0.0)
        return;
    g_lamp_diffuse += normal_light * saturate(dot(shading_n, dir));
    // Its highlight by GGX (D-086), the lobe widened by the sphere's size as seen from the surface
    // (half its angular radius on α, as the sun's disc): a bulb in a smooth paint is a bulb, not a
    // point under a pixel, and a lamp's highlight takes the lobe's normalisation (Blinn-Phong's
    // fixed weight kept it fifty times too dim on the scooter's paint).
    float size = 0.5 * min(radius / max(dist, 1e-3), 1.0);
    g_lamp_specular += ggx_light(shading_n, view, dir, ggx_alpha2(specular_power, size * size), f0) * highlight_share(specular_weight) * normal_light;
}

// The street lamps lit beyond the frame's set (#284, `Frame::far_lamps`) on a surface at `world`
// facing `n` in `place`: those of its grid column whose reach it is in, each its share of its
// light this frame (`Frame::far_shares`: what the set does not light of it), unshadowed and
// without the penumbra filter, added to `g_lamp_diffuse` and `g_lamp_specular`. Only the outside
// takes them, as it takes the street lamps of the set.
void far_lamps_at(Frame* f, LampPlace place, float3 world, float3 n, float3 shading_n, float3 view, float specular_power, float specular_weight, float3 f0)
{
    FarLamps* g = f[0].far_lamps;
    if (g == nullptr)
        return;
    if (place.region != REGION_NONE && place.region != REGION_OUTSIDE && place.other != REGION_OUTSIDE)
        return;
    uint2 run = column_run(g[0].slots, g[0].slot_mask, g[0].cell, world);
    for (uint e = 0u; e < run.y; e++)
    {
        uint k = g[0].indices[run.x + e];
        float share = f[0].far_shares[k];
        if (share <= 0.0)
            continue;
        Lamp lamp = g[0].lamps[k];
        if (lamp_weight(lamp, world, n) > 0.0)
            lamp_add<0>(f, lamp, k, share, world, n, shading_n, view, specular_power, specular_weight, f0, 0.0, float2(0.0));
    }
}

// Two axes across `dir`, the direction from a surface to a lamp: they span the disc of the lamp's
// sphere that faces the surface.
void lamp_disc_axes(float3 dir, out float3 t, out float3 b)
{
    t = normalize(cross(abs(dir.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0), dir));
    b = cross(dir, t);
}

// The shadow ray from a surface at `world` facing `n`, starting `bias` off it, to the point
// `offset` from lamp `lamp`'s middle. It ends short of the bulb: a lamp's own globe or fitting is
// not in its way.
RayDesc lamp_ray(Lamp lamp, float3 world, float3 n, float bias, float3 offset)
{
    float3 target = lamp.position_radius.xyz + offset;
    RayDesc ray;
    ray.Origin = world + n * bias;
    float3 to = target - ray.Origin;
    float length_to = length(to);
    ray.Direction = to / max(length_to, 1e-6);
    ray.TMin = 0.0;
    ray.TMax = max(length_to - lamp.position_radius.w - 0.02, 0.0);
    return ray;
}

// The sky reflected by a dielectric (issue #49), in the resolve's units (a white Lambertian
// surface facing the sun): the sky-view table in the mirror direction on a smooth surface,
// fading into the sky's irradiance `irradiance` as it gets rough (the table has no blurred
// levels), weighted by Schlick's Fresnel with 4 % at normal incidence and the roughness
// bounding its rise at grazing angles. GTAO's visibility `ao` (or -1) occludes it as
// Lagarde and de Rousiers do (Moving Frostbite to PBR, 2014). `fresnel` is the share the
// surface reflects, which its diffuse part loses. `sky_share` scales the table's light in the
// mirror direction `r`: the share of it the probes find there (issue #68), else 1. `traced`: a
// mirror ray replaces the open sky's part of it that `traced_share` gives.
float3 sky_reflection(float4* sky, float3 irradiance, float3 n, float3 view, float3 r, float3 sky_share, float specular_power, float ao, float3 f0, bool traced, bool multiscatter, out float3 fresnel)
{
    float roughness = sqrt(sqrt(2.0 / (specular_power + 2.0))); // RenderLayer::roughness_for_power, inverted
    float nv = saturate(dot(n, view));
    fresnel = environment_specular(f0, roughness, nv, multiscatter);
    float4 up_table = sky[SH_COEFFICIENTS];
    float3 forward = sky[SH_COEFFICIENTS + 1].xyz;
    // Luminance per unit of sun illuminance, times π: the resolve's units. The traced share
    // takes the open sky, which its ray's hit replaces; the rest the probes' share of it.
    float traced_share = traced ? mirror_ray_share(roughness) : 0.0;
    float3 open = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, r) * 3.14159265;
    float3 mirror = open * lerp(sky_share, float3(1.0), traced_share);
    float blend = saturate(2.0 * roughness * roughness);
    float3 environment = lerp(mirror, irradiance, blend);
    float occlusion = ao >= 0.0 ? saturate(pow(nv + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao) : 1.0;
    // For the mirror ray of the smooth rows (issue #52): its direction, and the weight of what
    // it meets in place of the open sky. That share takes no occlusion: its ray meets what blocks
    // the light, and GTAO darkened it a second time (a smooth body's reflection by about its ao
    // at roughness 0.25; D-086, the Bistro's scooter). The rest keeps it.
    float traced_weight = (1.0 - blend) * traced_share;
    // A metal's tint stays off its traced share: the request holds one weight (its luminance).
    g_reflection_request = float4(r, dot(fresnel, float3(0.2126, 0.7152, 0.0722)) * traced_weight);
    // The open sky the traced hit replaces at the request's own weight (its Fresnel's luminance):
    // with a tinted F0 the hit's `request.w * (hit - open)` left a sky-coloured rest, negative in
    // blue on gold over a dark wall (the review of 2026-10-30).
    return fresnel * occlusion * (environment - traced_weight * open) + g_reflection_request.w * open;
}

// The rough end of the mirror rays (#306): roughness 0.30, Blinn-Phong about 245. Smoother rows
// (glass at 3000, water at 900, a vehicle's at 300) trace their whole mirror reflection.
static const float MIRROR_RAY_FULL = 0.30;

// The share of a row's mirror reflection its ray traces (#306): all of it up to MIRROR_RAY_FULL,
// fading to none at RAY_REFLECTION_POWER's roughness (0.42); the rest the probes' dimmed sky. A
// row at power 60 to 90 traced 70 % of a sharp mirror at once: a glossy floor indoors (0.38)
// showed the room, point-sampled at grazing angles, as a hatch of streaks that the reflections'
// history held steady, and where a ray found no hit, the open sky.
float mirror_ray_share(float roughness)
{
    float rough_end = sqrt(sqrt(2.0 / (RAY_REFLECTION_POWER + 2.0)));
    return saturate((rough_end - roughness) / (rough_end - MIRROR_RAY_FULL));
}

float3 sun_light(Frame* f, float3 albedo, float3 n, float3 geometric, float specular_power, float specular_weight, float3 shadow, float4* sky, float ao, float3 view, float3 reflectance, float3 world, ProbeField* probes, bool traced, out float ndl)
{
#ifdef FORGE_IMPOSTOR_BAKE
    // An impostor's bake (D-075 step 4): what the surface is, unlit.
    if ((f[0].flags & (FLAG_BAKE_ALBEDO | FLAG_BAKE_NORMAL)) != 0)
    {
        ndl = 1.0;
        return (f[0].flags & FLAG_BAKE_ALBEDO) != 0 ? albedo : n * 0.5 + 0.5;
    }
#endif
    float3 light = normalize(f[0].sun_dir);
    ndl = dot(n, light);
    float3 diffuse = max(ndl, 0.0) * shadow;
    // The sun's highlight (D-086): GGX, its lobe widened by the sun's disc (half its angular
    // radius on α, so a smooth row's glint is the disc's own size and no brighter), Fresnel from
    // the row's reflectance. Blinn-Phong's fixed weight added 5 % of the sun on every rough
    // surface (a dark tile +27 %) and missed a smooth metal's glint.
    float disc = 0.5 * f[0].sun_angular_radius;
    float3 specular = ggx_light(n, view, light, ggx_alpha2(specular_power, disc * disc), reflectance)
        * highlight_share(specular_weight) * (ndl > 0.0 ? shadow : float3(0.0));
    float3 sun = f[0].sun_color.rgb;
    // Under a sky: the light the sky and the ground send the normal (issue #47), scaled by
    // how much of it the surface sees (issue #48). With probes, what they measured there
    // instead: the sky the street's buildings leave and the light bouncing off them (issue #53).
    if (sky != nullptr)
    {
        bool reflects = (f[0].flags & FLAG_SKY_REFLECTIONS) != 0;
        float3 r = reflect(-view, n);
        float3 sky_share = float3(1.0);
        float3 irradiance = sh_irradiance(sky, n);
        if (probes != nullptr)
        {
            // The sky's reflection dims as the probes' light does (issue #68): their light for
            // a normal along the mirror direction over the open sky's there, per channel, up
            // to 1. Lazarov rescaled reflection probes so at the normal (Black Ops II,
            // SIGGRAPH 2013); along the mirror, a street's ground sees the buildings, not the
            // sky above it. A mirror ray sees for itself, in its share (`mirror_ray_share`).
            // One call with reflections or without (#348): every call site of the lookup is a
            // whole inlined copy for the driver; its irradiance is the plain lookup's.
            float3 open_r = reflects ? sh_irradiance(sky, r) : irradiance;
            float3 toward_r;
            irradiance = probe_irradiance_mirror(probes, world, n, geometric, view, irradiance, reflects ? r : n, open_r, toward_r);
            if (reflects)
                sky_share = saturate(toward_r / max(open_r, 1e-4));
        }
        // D-088's reference (`shade_pixel` traced it once): its rays see the occlusion GTAO
        // stood for. The bounce's rays do too, but REBLUR blurs its contacts, a stair's steps into
        // one slope (the owner, 2026-10-30): GTAO stays under it, at their reach
        // (`r.ao.contact`), with the material's occlusion map the rays cannot see, and the
        // reflection's occlusion as under the probes.
        if (g_near_on)
        {
            irradiance = g_near_irradiance;
            if (!g_near_contacts)
                ao = -1.0;
        }
        float3 ambient = ao >= 0.0 ? irradiance * occlusion_bounce(ao, albedo) : irradiance;
        if (!reflects)
            return albedo * (diffuse * sun + ambient + g_lamp_diffuse) + specular * sun + g_lamp_specular;
        float3 fresnel;
        float3 reflection = sky_reflection(sky, irradiance, n, view, r, sky_share, specular_power, ao, reflectance, traced, (f[0].flags & FLAG_SINGLE_SCATTER) == 0, fresnel);
        return albedo * (diffuse * sun + ambient + g_lamp_diffuse) * (1.0 - fresnel) + specular * sun + g_lamp_specular + reflection;
    }
    // Space: hard sun, faint fill from the nebula side, a little wrap so shadows are not black.
    float wrap = saturate((ndl + 0.3) / 1.3) * 0.08;
    float3 fill = float3(0.10, 0.12, 0.18) * 0.25 * (0.5 + 0.5 * n.y);
    // Both stand for light from around the surface: the occlusion takes its share (issue #55).
    if (ao >= 0.0)
        return albedo * (diffuse * sun + (wrap + fill) * occlusion_bounce(ao, albedo) + g_lamp_diffuse) + specular * sun + g_lamp_specular;
    return albedo * (diffuse * sun + wrap + fill + g_lamp_diffuse) + specular * sun + g_lamp_specular;
}

// The debug colourings (M, L) replace the material's albedo.
bool debug_albedo(Frame* f, uint meshlet_index, uint lod_level, out float3 albedo)
{
    albedo = float3(0.0);
    if ((f[0].flags & FLAG_LOD_COLORS) != 0)
        albedo = lod_color(min(lod_level, 63u));
    else if ((f[0].flags & FLAG_MESHLET_COLORS) != 0)
        albedo = hash_color(meshlet_index & 0x00FFFFFFu) * 0.6 + 0.3;
    else
        return false;
    return true;
}

// Triplanar projection (the textures have no coordinates): the object-space position seen
// along each axis, weighted by how much the normal faces that axis.
float3 triplanar_weights(float3 n)
{
    float3 w = n * n;
    w *= w;
    return w / (w.x + w.y + w.z);
}

// `image` sampled at the object-space point `p` (in texture repeats) along the three axes,
// with the derivatives of `p` across the pixel choosing the level of detail.
float4 sample_triplanar(uint image, float3 w, float3 p, float3 dpdx, float3 dpdy)
{
    float4 c = float4(0.0);
    if (w.x > 0.01)
        c += w.x * sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.zy, dpdx.zy, dpdy.zy);
    if (w.y > 0.01)
        c += w.y * sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.xz, dpdx.xz, dpdy.xz);
    if (w.z > 0.01)
        c += w.z * sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.xy, dpdx.xy, dpdy.xy);
    return c / max(w.x * float(w.x > 0.01) + w.y * float(w.y > 0.01) + w.z * float(w.z > 0.01), 1e-4);
}

float3 unpack_normal(float4 texel, float strength)
{
    float3 t = texel.xyz * 2.0 - 1.0;
    return float3(t.xy * strength, t.z);
}

// The object-space normal `n` bent by a tangent-space normal map projected along the three
// axes, each projection's normal blended into `n` with the whiteout blend (after Golus,
// "Normal Mapping for a Triplanar Shader", 2017).
float3 triplanar_normal(uint image, float strength, float3 n, float3 w, float3 p, float3 dpdx, float3 dpdy)
{
    float3 x = float3(0.0), y = float3(0.0), z = float3(0.0);
    if (w.x > 0.01)
    {
        float3 t = unpack_normal(sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.zy, dpdx.zy, dpdy.zy), strength);
        x = float3(t.xy + n.zy, abs(t.z) * n.x).zyx;
    }
    if (w.y > 0.01)
    {
        float3 t = unpack_normal(sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.xz, dpdx.xz, dpdy.xz), strength);
        y = float3(t.xy + n.xz, abs(t.z) * n.y).xzy;
    }
    if (w.z > 0.01)
    {
        float3 t = unpack_normal(sample_image_grad(image, SAMPLER_ANISOTROPIC_REPEAT, p.xy, dpdx.xy, dpdy.xy), strength);
        z = float3(t.xy + n.xy, abs(t.z) * n.z);
    }
    return normalize(x * w.x + y * w.y + z * w.z);
}

// ------------------------------------------------------------- hex-tiling ---
// Issue #66: a texture's repeats hidden on large surfaces. Mikkelsen, "Practical Real-Time
// Hex-Tiling", JCGT 11(2), 2022, after Heitz & Neyret, "High-Performance By-Example Noise
// using a Histogram-Preserving Blending Operator", 2018. Adapted from the paper's reference
// code (github.com/mmikk/hextile-demo, `hextiling.h`, MIT License, Copyright (c) 2022 mmikk;
// notice in shaders/third-party/hextile-demo-LICENSE.txt). The texture is sampled in three
// overlapping hexagonal tiles, each at a random offset and rotation, and the tiles are blended
// by weights that favour the brighter sample near a tile's edge, so the blend keeps the
// texture's contrast instead of averaging it away. Forge's changes: an integer hash for the
// offsets and angles, larger tiles (`HEX_TILE`), the tiles' weights taken from the albedo and
// shared by the normal map (colour and relief stay together), and the normals blended as
// tangent-space vectors in the triplanar projection's convention.

// How far a unit normal stored in bytes may come out shorter than 1 (each channel rounded to
// 1/255 of its range, and a BC7 block's error of two steps, D-086): no variance below it (#331).
static const float NORMAL_LENGTH_ROUNDING = 0.02;

// Size of the hex tiles, in the paper's (about a third of a repeat across). Three times that,
// about a repeat across, crosses fewer seams, where the blend softens the texture: on the
// ballad's big faces the paper's size blurred the rock's blotches, this one keeps their
// contrast, and a tile one repeat across still cannot show a repeat.
static const float HEX_TILE = 3.0;

// The three tiles over the texture coordinate `st` (in repeats): where each samples the
// texture, its rotation, and its barycentric weight.
struct HexTiles
{
    float2 uv[3];
    float2x2 rot[3];
    float3 w;
};

HexTiles hex_tiles(float2 st)
{
    HexTiles h;
    // Skew into a triangle grid: its vertices are the hexagons' centres.
    float2 skewed = mul(float2x2(1.0, -0.57735027, 0.0, 1.15470054), st * (3.46410162 / HEX_TILE));
    int2 base = int2(floor(skewed));
    float3 t = float3(frac(skewed), 0.0);
    t.z = 1.0 - t.x - t.y;
    float s = step(0.0, -t.z);
    float s2 = 2.0 * s - 1.0;
    h.w = float3(-t.z * s2, s - t.y * s2, s - t.x * s2);
    int i = int(s);
    int2 vertex[3] = { base + int2(i, i), base + int2(i, 1 - i), base + int2(1 - i, i) };
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        int2 v = vertex[k];
        uint n = asuint(v.x) * 0x8da6b343u ^ asuint(v.y) * 0xd8163841u;
        n ^= n >> 16;
        n *= 0x7feb352du;
        n ^= n >> 15;
        n *= 0x846ca68bu;
        n ^= n >> 16;
        float angle = float(n & 0x3FFu) * (6.2831853 / 1024.0);
        float2 offset = float2((n >> 10) & 0x7FFu, n >> 21) * (1.0 / 2048.0);
        float c = cos(angle);
        float sn = sin(angle);
        h.rot[k] = float2x2(c, -sn, sn, c);
        float2 centre = mul(float2x2(1.0, 0.5, 0.0, 0.8660254), float2(v)) * (HEX_TILE / 3.46410162);
        h.uv[k] = mul(st - centre, h.rot[k]) + centre + offset;
    }
    return h;
}

// One projection of a hex-tiled row: its albedo texel (white without an albedo texture) and
// its tangent-space normal (`unpack_normal`'s form; flat without a normal map), from the
// texture coordinate `st` and its derivatives across the pixel.
void hex_sample(Material mat, float2 st, float2 dx, float2 dy, out float3 albedo, out float3 tangent)
{
    HexTiles h = hex_tiles(st);
    float3 c[3];
    float3 t[3];
    float3 luminance = float3(1.0);
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        float2 gx = mul(dx, h.rot[k]);
        float2 gy = mul(dy, h.rot[k]);
        c[k] = float3(1.0);
        t[k] = float3(0.0, 0.0, 1.0);
        if (mat.albedo_texture != TEXTURE_NONE)
        {
            c[k] = sample_image_grad(mat.albedo_texture, SAMPLER_ANISOTROPIC_REPEAT, h.uv[k], gx, gy).rgb;
            luminance[k] = dot(c[k], float3(0.299, 0.587, 0.114));
        }
        if (mat.normal_texture != TEXTURE_NONE)
        {
            t[k] = unpack_normal(sample_image_grad(mat.normal_texture, SAMPLER_ANISOTROPIC_REPEAT, h.uv[k], gx, gy), mat.normal_strength);
            // Back from the tile's rotated frame to the projection's.
            t[k].xy = mul(h.rot[k], t[k].xy);
        }
    }
    // The paper's weights: the barycentrics sharpened (power 7) and scaled by each sample's
    // luminance, 60 % of the way from 1.
    float3 weight = lerp(float3(1.0), luminance, 0.6) * pow(h.w, float3(7.0));
    weight /= max(weight.x + weight.y + weight.z, 1e-6);
    albedo = weight.x * c[0] + weight.y * c[1] + weight.z * c[2];
    tangent = weight.x * t[0] + weight.y * t[1] + weight.z * t[2];
}

// `sample_triplanar` and `triplanar_normal` for a hex-tiled row, in one walk over the
// projections: the albedo texel and the bent object-space normal.
void hex_triplanar(Material mat, float3 n, float3 w, float3 p, float3 dpdx, float3 dpdy, out float3 albedo, out float3 bent)
{
    float3 c = float3(0.0);
    float3 x = float3(0.0), y = float3(0.0), z = float3(0.0);
    float3 texel, t;
    if (w.x > 0.01)
    {
        hex_sample(mat, p.zy, dpdx.zy, dpdy.zy, texel, t);
        c += w.x * texel;
        x = float3(t.xy + n.zy, abs(t.z) * n.x).zyx;
    }
    if (w.y > 0.01)
    {
        hex_sample(mat, p.xz, dpdx.xz, dpdy.xz, texel, t);
        c += w.y * texel;
        y = float3(t.xy + n.xz, abs(t.z) * n.y).xzy;
    }
    if (w.z > 0.01)
    {
        hex_sample(mat, p.xy, dpdx.xy, dpdy.xy, texel, t);
        c += w.z * texel;
        z = float3(t.xy + n.xy, abs(t.z) * n.z);
    }
    albedo = c / max(w.x * float(w.x > 0.01) + w.y * float(w.y > 0.01) + w.z * float(w.z > 0.01), 1e-4);
    bent = normalize(x * w.x + y * w.y + z * w.z);
}

// What the resolve knows of the surface behind a pixel.
// A cascade of the sea's waves as the floor's caustics read it (#108; `GpuCausticCascade` in
// water.rs).
struct CausticCascade
{
    float4 a; // 1 / its patch's side (m⁻¹), metres a texel, the angular frequency its energy centres on (rad/s), the TMA factor at its spectrum's depth
    uint4 b;  // its slopes' sampled index (every mip), 0, 0, 0
};

// The shore as the ground reads it (issue #105; `GpuShoreGround` in water.rs): the water's
// shore fields and its trains, whose swash wets the sand (`shore_wetness`), and the waves whose
// caustics light the floor under the sea (`shore_caustics`, #108).
struct ShoreGround
{
    float4 frame;     // the fields' first sample (world x, z), 1 / their spacing, their samples a side
    float4 trains[4]; // per train: ω (rad/s), its height in deep water (m), a phase offset, 0
    uint shore;       // sampled (rg16f): the floor's height, the signed coast distance
    uint train_count;
    float time;       // the sea's clock, the water surface's
    float pad;
    float4 sun_water; // xyz: the way the sun's light runs under the water (unit, downwards)
    CausticCascade cascades[3];
    uint cascade_count; // 0: no caustics
    uint3 pad_caustics;
};

struct Surface
{
    float3 n;               // world-space normal, interpolated
    float3 view;            // unit, world space, from the surface towards the camera (issue #49)
    float3 transmitted;     // the sunlight crossing the ice to this point and leaving towards the camera, per channel (issues #59, #61); the jelly's light from behind and through it (#180); else 0
    float cloud;            // jelly: the share of its path through the body its cloud scatters (#180); else 1
    float3 object_normal;   // object-space normal, interpolated
    float3 object_position; // object-space position
    float3 dpdx;            // its change per pixel to the right and down
    float3 dpdy;
    float3x3 frame;         // the instance's rotation and scale: object-space directions to the camera-relative space
    uint instance_id;
    uint instance_place;    // a hash of where the instance stands, the same whatever its index (`interior_room`)
    uint meshlet_index;
    uint lod_level;
    float3 shadow;          // 1 in the sun, 0 where a shadow ray met something (issue #45), the jelly's tint where it crossed jelly (#180)
    float4* sky;            // the sky's irradiance coefficients (sh.slang), null in space (issue #47)
    float ao;               // how much of the sky the surface sees (issue #48); -1: no occlusion computed
    float3 world;           // the position in the scene frame (the rays' and the probes'; `to_scene`)
    ProbeField* probes;     // the diffuse light's probes (probes.slang, issue #53), or null: the sky's irradiance unoccluded
    bool traced;            // a mirror ray replaces the sky's reflection where it meets something (issues #50, #52)
    ShoreGround* shore;     // the shore whose swash wets the layered ground (issue #105), or null
    bool has_uv;            // the mesh carries texture coordinates (D-047)
    float2 uv;              // they, interpolated
    float2 duvdx;           // their change per pixel to the right and down
    float2 duvdy;
    float coast;            // a planet tile's coast from its normal map's alpha (#220): the sea under 0.5; -1 without
};

// Smooth value noise in [0, 1) over lattice cells of size 1.
float value_noise3(float3 p)
{
    float3 i = floor(p);
    float3 t = p - i;
    t = t * t * (3.0 - 2.0 * t);
    int3 c = int3(i);
    float corner[8];
    [unroll]
    for (uint k = 0; k < 8; k++)
    {
        int3 q = c + int3(k & 1u, (k >> 1) & 1u, k >> 2);
        uint h = asuint(q.x) * 0x8da6b343u ^ asuint(q.y) * 0xd8163841u ^ asuint(q.z) * 0xcb1ab31fu;
        h ^= h >> 16;
        h *= 0x7feb352du;
        h ^= h >> 15;
        corner[k] = float(h >> 8) * (1.0 / 16777216.0);
    }
    float x00 = lerp(corner[0], corner[1], t.x);
    float x10 = lerp(corner[2], corner[3], t.x);
    float x01 = lerp(corner[4], corner[5], t.x);
    float x11 = lerp(corner[6], corner[7], t.x);
    return lerp(lerp(x00, x10, t.y), lerp(x01, x11, t.y), t.z);
}

// `v` turned by the unit quaternion `q` (xyz w), or back when `back`.
float3 quat_turn(float4 q, float3 v, bool back)
{
    float3 u = back ? -q.xyz : q.xyz;
    float3 t = 2.0 * cross(u, v);
    return v + q.w * t + cross(u, t);
}

// The list of the grid column over `world` in a hash of columns of `cell` metres (`slots`, of
// `slot_mask` + 1: x, z, first, count; count 0 an empty slot; crates/forge-render/src/columns.rs):
// where it starts in its indices and how long it is, (0, 0) without one.
uint2 column_run(uint4* slots, uint slot_mask, float cell, float3 world)
{
    int2 c = int2(floor(world.xz / cell));
    uint h = (uint(c.x) * 73856093u ^ uint(c.y) * 19349663u) & slot_mask;
    for (uint tries = 0u; tries <= slot_mask; tries++)
    {
        uint4 slot = slots[h];
        if (slot.w == 0u)
            break;
        if (int(slot.x) == c.x && int(slot.y) == c.y)
            return slot.zw;
        h = (h + 1u) & slot_mask;
    }
    return uint2(0u);
}

// The decals over surface `s` (D-074, forge_render::decals): the decals of its grid column, in
// painter's order, whose box holds it. A colour decal blends into `albedo` its texture's
// premultiplied colour; a row decal (`rows`: the layered class) its row's surface, colour and
// normal (`n`), by its texture's alpha, as a layer of the ground. A decal fades out towards its
// box's faces along its axis, and on the surfaces that do not face that axis (the ground under a
// wall's grime). Its texture's gradients are its box's axes dotted with the surface's analytic
// change per pixel, scaled as the rows' textures are (`texture_lod_scale`): no halo where the
// surface ends, its mips and anisotropy whole.
void apply_decals(Frame* f, Surface s, inout float3 albedo, inout float3 n, bool rows)
{
    DecalGrid* g = f[0].decals;
    if (g == nullptr)
        return;
    uint2 run = column_run(g[0].slots, g[0].slot_mask, g[0].cell, s.world);
    for (uint i = 0u; i < run.y; i++)
    {
        uint index = g[0].indices[run.x + i];
        Decal d = g[0].decals[index];
        float3 local = quat_turn(d.rotation, s.world - d.centre, true);
        if (any(abs(local) > d.half))
            continue;
        DecalKind k = g[0].kinds[d.kind];
        if (k.texture == TEXTURE_NONE || (k.row != DECAL_NO_ROW && !rows))
            continue;
        float facing = smoothstep(k.facing.x, k.facing.y, dot(s.n, quat_turn(d.rotation, float3(0.0, 1.0, 0.0), false)));
        float depth = saturate((d.half.y - abs(local.y)) / max(d.half.y * k.depth_fade, 1e-4));
        float strength = facing * depth * k.tint.a;
        if (strength <= 0.0)
            continue;
        float2 scale = float2(d.repeat, 1.0) / (2.0 * d.half.xz);
        float2 uv = (local.xz / (2.0 * d.half.xz) + 0.5) * float2(d.repeat, 1.0) + float2(d.phase, 0.0);
        float2 gx = quat_turn(d.rotation, mul(s.frame, s.dpdx), true).xz * scale * f[0].texture_lod_scale;
        float2 gy = quat_turn(d.rotation, mul(s.frame, s.dpdy), true).xz * scale * f[0].texture_lod_scale;
        float4 t = sample_image_grad(k.texture, k.sampler, uv, gx, gy);
        if (k.row == DECAL_NO_ROW)
        {
            albedo = albedo * (1.0 - t.a * strength) + t.rgb * k.tint.rgb * strength;
            continue;
        }
        float mask = t.a * strength;
        if (mask <= 0.0)
            continue;
        float3 row_albedo, row_n;
        standard_surface(f[0].materials[k.row], s, row_albedo, row_n);
        albedo = lerp(albedo, row_albedo * k.tint.rgb, mask);
        n = normalize(lerp(n, row_n, mask));
    }
}

// `apply_decals` for a class without row decals: their colour alone.
void apply_decals(Frame* f, Surface s, inout float3 albedo)
{
    float3 n = s.n;
    apply_decals(f, s, albedo, n, false);
}

// The albedo and the normal a standard row gives the surface: the row's two colours mixed by
// the instance, its cavity, and its textures projected along the object's axes.
void standard_surface(Material mat, Surface s, out float3 albedo, out float3 n)
{
    n = s.n;
    // Each instance its own mix of the two colours, darker where the surface faces the
    // object's centre (rocks' hollows).
    float3 instance_hash = hash_color(s.instance_id);
    float3 tint = lerp(mat.color_a, mat.color_b, instance_hash.x);
    float cavity = saturate(0.5 + 0.5 * dot(normalize(s.object_position), n));
    albedo = tint * ((1.0 - mat.cavity) + mat.cavity * cavity);
    if (mat.albedo_texture != TEXTURE_NONE || mat.normal_texture != TEXTURE_NONE)
    {
        float3 w = triplanar_weights(s.object_normal);
        // Projected from the scene frame, the ground's tiles meet with one pattern and one
        // noise: from each mesh's own origin, a polygon kept in the next tile's mesh showed as
        // a band of another brightness (the city's pavements, the owner).
        bool world = (mat.flags & MATERIAL_FLAG_WORLD_PROJECTED) != 0;
        float3 p = (world ? s.world : s.object_position) * mat.inv_texture_scale;
        float3 dpdx = s.dpdx * mat.inv_texture_scale;
        float3 dpdy = s.dpdy * mat.inv_texture_scale;
        if ((mat.flags & MATERIAL_FLAG_HEX_TILING) != 0)
        {
            // Each instance its own place in the texture (a few dozen repeats away), so the
            // objects of one mesh stop sharing a pattern; then the hex tiles (issue #66).
            if (!world)
                p += hash_color(s.instance_id ^ 0x9E3779B9u) * 61.0;
            float broad = 0.65 * value_noise3(p * 0.137) + 0.35 * value_noise3(p * 0.41 + 17.0);
            float3 texel, bent;
            hex_triplanar(mat, s.object_normal, w, p, dpdx, dpdy, texel, bent);
            if (mat.albedo_texture != TEXTURE_NONE)
                albedo *= texel * (0.7 + 0.6 * broad);
            if (mat.normal_texture != TEXTURE_NONE)
                n = normalize(mul(s.frame, bent));
            return;
        }
        if (mat.albedo_texture != TEXTURE_NONE)
        {
            // Against visible repeats: a brightness that drifts over several repeats.
            float broad = 0.65 * value_noise3(p * 0.137) + 0.35 * value_noise3(p * 0.41 + 17.0);
            albedo *= sample_triplanar(mat.albedo_texture, w, p, dpdx, dpdy).rgb * (0.7 + 0.6 * broad);
        }
        if (mat.normal_texture != TEXTURE_NONE)
        {
            float3 bent = triplanar_normal(mat.normal_texture, mat.normal_strength, s.object_normal, w, p, dpdx, dpdy);
            n = normalize(mul(s.frame, bent));
        }
    }
}

// A UV-mapped row (D-047): a model's material as glTF's metallic-roughness model gives it, its
// maps read by the mesh's texture coordinates through the row's transform and wrapping, each
// map times its factor. A metal keeps no diffuse colour and reflects as much as its base
// colour's brightest channel at normal incidence (the reflection is not tinted); the
// occlusion map darkens the sky's and the probes' light only.
// Leaf cards (#346, research/leaf-detail.md §1 and §3). Their rows (MATERIAL_FLAG_FOLIAGE) take
// two fields of the ice's: `scattering`, how much paler and greyer a leaf's underside is (0 the
// same, 1 a holm oak's down), and `scatter`, the share of the light behind a leaf that comes
// through it, per channel of its colour (0: none).
//
// A leaf turns its top up and out, so the crown's leaves seen from below, as from the street
// under a tree, show their undersides: LEAF_OUT is the share of the crown's bent normal in a
// leaf's up, and the underside shows from UNDER_START to UNDER_FULL of the view's cosine under it.
static const float LEAF_OUT = 0.5;
static const float UNDER_START = 0.0;
static const float UNDER_FULL = 0.6;
// The underside's colour: its leaf's greyed by UNDERSIDE_GREY and lightened by UNDERSIDE_LIFT.
static const float UNDERSIDE_GREY = 0.6;
static const float UNDERSIDE_LIFT = 1.5;
// The light through a leaf (Barré-Brisebois and Bouchard's approximation, as Frostbite's): the
// light's direction bent by LEAF_DISTORTION of the normal, the lobe round the view's
// LEAF_POWER.
static const float LEAF_DISTORTION = 0.3;
static const float LEAF_POWER = 4.0;

// The normal a surface's sun shadow takes: its own, or a leaf card's turned towards the sun when
// it faces away (the shadow then the sun's visibility from the leaf, for the light through it).
float3 leaf_shadow_normal(Frame* f, Material mat, float3 n)
{
    if ((mat.flags & MATERIAL_FLAG_FOLIAGE) != 0 && dot(n, f[0].sun_dir) < 0.0)
        return -n;
    return n;
}

// A leaf's colour `base` as seen: its top's, or towards its underside's as the view turns under
// it (`paleness`, the row's `scattering`).
float3 leaf_underside(float3 base, float paleness, Surface s)
{
    if (paleness <= 0.0)
        return base;
    float3 up = normalize(float3(0.0, 1.0, 0.0) + LEAF_OUT * s.n);
    float under = smoothstep(UNDER_START, UNDER_FULL, -dot(s.view, up));
    float grey = dot(base, float3(0.2126, 0.7152, 0.0722));
    float3 pale = lerp(base, float3(grey), UNDERSIDE_GREY) * UNDERSIDE_LIFT;
    return lerp(base, pale, under * paleness);
}

// The sun's (or the moon's) light through a leaf towards the camera, strongest looking at the
// light through it, the leaf's colour times `through` (the row's `scatter`), as far as the light
// reaches the leaf (`s.shadow`, traced from the side that faces it).
float3 leaf_transmission(Frame* f, float3 base, float3 through, Surface s)
{
    if (all(through <= 0.0))
        return float3(0.0);
#ifdef FORGE_IMPOSTOR_BAKE
    if ((f[0].flags & (FLAG_BAKE_ALBEDO | FLAG_BAKE_NORMAL)) != 0)
        return float3(0.0);
#endif
    float3 l = normalize(normalize(f[0].sun_dir) + s.n * LEAF_DISTORTION);
    float lobe = pow(saturate(dot(s.view, -l)), LEAF_POWER);
    return base * through * f[0].sun_color.rgb * s.shadow * lobe;
}

// What a surface of the standard class gives the sun's and the sky's light (`sun_light`), whose
// one call `shade_standard` makes for each of its paths: each call, inlined, cost the driver
// about 2.5 s of the resolve's compile (#348).
struct LightInputs
{
    float3 albedo;
    float3 n;
    float power;
    float specular;
    float ao;
    float3 reflectance; // Fresnel's F0 per channel: a metal's its base colour (D-086)
    float3 emissive;
    float metal;        // its metalness (the debug views)
    float variance;     // its normal map's spread under the pixel, Toksvig's (the debug views)
};

// A debug view of a standard row's shading input (D-086, `r.show.input`): 1 the diffuse albedo,
// 2 the perceptual roughness (after Toksvig), 3 the metalness, 4 the shading normal (scene frame,
// × 0.5 + 0.5), 5 the lobe's widening (Toksvig's variance in red, the geometry's under the pixel
// in green, × 4), 6 Fresnel's F0. Values as they are, before the tone curve.
float3 shading_input(uint view, LightInputs li)
{
    if (view == 1u)
        return li.albedo;
    if (view == 2u)
        return float3(sqrt(sqrt(2.0 / (li.power + 2.0))));
    if (view == 3u)
        return float3(li.metal);
    if (view == 4u)
        return li.n * 0.5 + 0.5;
    if (view == 5u)
        return float3(li.variance, max(g_normal_variance, 0.0), 0.0) * 4.0;
    return li.reflectance;
}

// A row read by UV (D-047): its maps, decals and leaves into what its light takes.
LightInputs uv_mapped_light(Frame* f, Material mat, Surface s)
{
    float2 uv = float2(dot(mat.uv_u, float3(s.uv, 1.0)), dot(mat.uv_v, float3(s.uv, 1.0)));
    float2 gx = float2(dot(mat.uv_u.xy, s.duvdx), dot(mat.uv_v.xy, s.duvdx));
    float2 gy = float2(dot(mat.uv_u.xy, s.duvdy), dot(mat.uv_v.xy, s.duvdy));
    float3 base = mat.color_a;
    bool leaves = (mat.flags & MATERIAL_FLAG_FOLIAGE) != 0;
    // Leaf cards: each instance its own mix of the two colours, as projected rows.
    if (leaves)
        base = lerp(mat.color_a, mat.color_b, hash_color(s.instance_id).x);
    if (mat.albedo_texture != TEXTURE_NONE)
        base *= sample_image_grad(mat.albedo_texture, mat.sampler, uv, gx, gy).rgb;
    if (leaves)
        base = leaf_underside(base, mat.scattering, s);
    apply_decals(f, s, base);
    float roughness = mat.roughness;
    float metal = mat.metallic;
    if (mat.metal_rough_texture != TEXTURE_NONE)
    {
        float4 mr = sample_image_grad(mat.metal_rough_texture, mat.sampler, uv, gx, gy);
        roughness *= mr.g;
        metal *= mr.b;
    }
    float ao = s.ao;
    if (mat.occlusion_texture != TEXTURE_NONE)
    {
        float occlusion = 1.0 + mat.occlusion_strength * (sample_image_grad(mat.occlusion_texture, mat.sampler, uv, gx, gy).r - 1.0);
        ao = ao >= 0.0 ? ao * occlusion : occlusion;
    }
    float3 emissive = emission(f, mat);
    if (mat.emissive_texture != TEXTURE_NONE)
        emissive *= sample_image_grad(mat.emissive_texture, mat.sampler, uv, gx, gy).rgb;
    float3 n = s.n;
    // The normals' variance under the pixel, from the normal map's filtered length (Toksvig).
    float variance = 0.0;
    if (mat.normal_texture != TEXTURE_NONE)
    {
        // The tangent frame from the derivatives of the position and of the transformed UVs
        // (Schüler 2013's cotangent frame), in object space: T along +u, B along +v. glTF's
        // maps point their +y up the image, towards −v.
        float3 N = s.object_normal;
        float3 dp1perp = cross(N, s.dpdx);
        float3 dp2perp = cross(s.dpdy, N);
        float3 T = dp2perp * gx.x + dp1perp * gy.x;
        float3 B = dp2perp * gx.y + dp1perp * gy.y;
        float scale = rsqrt(max(max(dot(T, T), dot(B, B)), 1e-30));
        float3 t = sample_image_grad(mat.normal_texture, mat.sampler, uv, gx, gy).xyz * 2.0 - 1.0;
        // The mips keep the mean normal's length (#331): shorter where the normals under the
        // pixel part (a tile's grout bevels seen from a few metres), past the bytes' rounding.
        float len = length(t);
        variance = max(1.0 - len - NORMAL_LENGTH_ROUNDING, 0.0) / max(len, 0.05) * mat.normal_strength;
        t.xy *= mat.normal_strength;
        float3 bent = (t.x * T - t.y * B) * scale + t.z * N;
        n = normalize(mul(s.frame, bent));
    }
    float alpha = clamp(roughness, 0.02, 1.0);
    alpha *= alpha;
    // RenderLayer::specular_power, kept over 0: at roughness 1 the exponent is 0, and pow(0, 0) is NaN
    // where the surface turns from the light.
    float power = max(2.0 / (alpha * alpha) - 2.0, 0.001);
    // Toksvig's factor (#331): the lobe widened by the normals' variance under the pixel, so a
    // bevel too fine for it gives a broad, steady highlight instead of a flickering line.
    power /= 1.0 + power * variance;
    LightInputs li;
    li.albedo = base * (1.0 - metal);
    li.reflectance = lerp(float3(mat.reflectance), base, metal);
    // A metal's highlight whole (D-086): its F0, its base colour, is its strength; weighed by its
    // reflectance again, a dark metal's was dimmed twice.
    li.specular = lerp(mat.specular, 1.0, metal);
    li.n = n;
    li.power = power;
    li.ao = ao;
    li.emissive = emissive;
    li.metal = metal;
    li.variance = variance;
    return li;
}

// A faked window's pane condition (#339, `glass.slang`'s for the clear glass): a tilt of up to
// WINDOW_TILT radians each way and roller waves of up to WINDOW_WAVE every WINDOW_WAVE_LENGTH
// metres across and up the pane (the window mesh's x and y), from a hash of where the window
// stands, the waves faded out where a pixel covers a quarter of one so they never shimmer.
static const float WINDOW_TILT = 0.006;
static const float WINDOW_WAVE = 0.003;
static const float WINDOW_WAVE_LENGTH = 0.9;

float3 window_pane_normal(float3 n, Surface s)
{
    float3 h = hash_color(s.instance_place * 0x2C1B3C6Du + 0x297A2D39u);
    float3 w = hash_color(s.instance_place * 0x9E3779B9u + 0x7F4A7C15u);
    float3 across = normalize(mul(s.frame, float3(1.0, 0.0, 0.0)));
    float3 rise = normalize(mul(s.frame, float3(0.0, 1.0, 0.0)));
    float footprint = max(length(s.dpdx), length(s.dpdy));
    float fade = saturate(1.0 - footprint / (0.25 * WINDOW_WAVE_LENGTH));
    float2 waves = sin(s.object_position.xy * (6.2831853 / WINDOW_WAVE_LENGTH) + w.xy * 6.2831853);
    float2 lean = (h.xy * 2.0 - 1.0) * WINDOW_TILT + waves * w.z * (WINDOW_WAVE * fade);
    return normalize(n + across * lean.x + rise * lean.y);
}

// The room behind a window's glass (interior mapping, D-060's step 3; Van Dongen, "Interior
// Mapping", 2008): a box in the window mesh's own space, the glass on its z plane and the room
// towards −z, from 1.4 m left of the glass's left edge, 0.9 m under its foot, 4.5 m deep. Its
// walls, floor and ceiling are coloured by the instance, the back wall and the depths darker:
// the light comes in through the window.
static const float3 ROOM_LOW = float3(-1.4, -0.9, -4.5);
static const float3 ROOM_HIGH = float3(2.6, 2.6, 0.0);
static const float3 ROOM_WALLS[4] = {
    float3(0.86, 0.83, 0.76), // cream
    float3(0.80, 0.83, 0.85), // pale grey-blue
    float3(0.85, 0.78, 0.68), // warm sand
    float3(0.78, 0.82, 0.74), // sage
};
static const float3 ROOM_FLOORS[3] = {
    float3(0.40, 0.28, 0.18), // dark wood
    float3(0.58, 0.44, 0.30), // oak
    float3(0.56, 0.55, 0.52), // tiles
};
static const float3 ROOM_CEILING = float3(0.90, 0.89, 0.86);
// The share of the window's daylight a room's walls get, and the share of the windows with net
// curtains (the owner: in France many homes keep sheers so the street cannot see in). The
// curtains hang at the glass and take all of it. A room lit as brightly as the wall outside
// matched the walls from afar, and the facades lost their windows; a room gets a few per cent of
// the daylight outside, but a third keeps the room in sight from the street.
static const float ROOM_DAYLIGHT = 0.35;
static const float ROOM_CURTAINS = 0.45;
// The share of a lit row's luminance (3 to 8 cd/m²) its room's lamp gives the walls: the flat glow
// was too strong and all alike at night (the owner). At 4 % (0.12 to 0.3 cd/m²) the rooms sat a
// stop or two over the moonlit night's grey, but under the walls the street lamps light now that
// the probes see them (#254): from the pavement no window looked lit. A quarter (0.75 to
// 2 cd/m²) puts a lit room level with a lamp-lit wall, the room still in sight; a real one is
// brighter still. city-plan's far facades light their windows to match.
static const float ROOM_LAMP = 0.25;
// A lit shop's share: its ceiling lights and displays twice a home's lamps.
static const float ROOM_SHOP_LAMP = 0.5;

// A row's emission in the shading's units: as given, or with MATERIAL_FLAG_EMISSIVE_ABSOLUTE a
// luminance in cd/m² over the frame's reference illuminance (D-046: the shading is per unit of
// it, so L·π / E gives L), the same light at noon and at midnight as the reference changes
// (#254: a run whose day cycles kept its windows at the brightness of the hour it was built).
float3 emission(Frame* f, Material mat)
{
    if ((mat.flags & MATERIAL_FLAG_EMISSIVE_ABSOLUTE) == 0)
        return mat.emissive;
    return mat.emissive * (3.14159265 / max(f[0].sun_illuminance, 1e-9));
}

// Shelves of goods on a shop's wall at `along` metres along it and `up` over its floor: boards
// every 0.45 m from 0.3 to 2.1 m, the goods between them in rows of boxes 0.3 m wide, short of
// the board above, a shadow between neighbours.
float3 shop_shelves(float along, float up, uint id, float3 wall)
{
    if (up < 0.3 || up > 2.1)
        return wall;
    float row = (up - 0.3) / 0.45;
    if (frac(row) < 0.08)
        return float3(0.30, 0.24, 0.18);
    float x = along / 0.3;
    uint block = uint(int(floor(x)));
    float3 goods = hash_color(id ^ block * 0x9E3779B9u ^ uint(row) * 0x85EBCA6Bu) * 0.4 + 0.25;
    // The gap over the goods and between two boxes, in their shade.
    return frac(row) > 0.8 || frac(x) < 0.08 ? goods * 0.35 : goods;
}

// The room behind a window, or with `shop` a shop behind a shopfront: its floor level with the
// glass's foot, 6 m deep, pale walls lined with shelves, a tiled floor and a counter.
float3 interior_room(float3 o, float3 d, uint id, float footprint, bool shop)
{
    float3 room_low = shop ? float3(-1.4, 0.0, -6.0) : ROOM_LOW;
    float3 room_high = shop ? float3(2.6, 3.6, 0.0) : ROOM_HIGH;
    // Rooms side by side and storey over storey behind wider and taller glass (a shopfront's).
    float2 size = room_high.xy - room_low.xy;
    float2 cell = floor((o.xy - room_low.xy) / size);
    float3 lo = float3(room_low.xy + cell * size, o.z + room_low.z);
    float3 hi = float3(room_high.xy + cell * size, o.z);
    id ^= uint(int(cell.x) * 7919 + int(cell.y) * 104729);
    // The planes the ray leaves the box by: the nearest of them.
    float3 inv = 1.0 / d;
    float3 far = max((lo - o) * inv, (hi - o) * inv);
    float t = min(far.x, min(far.y, far.z));
    float3 p = o + d * t;
    float3 h = hash_color(id ^ 0x51ED27u);
    float3 g = hash_color(id ^ 0xA5F3C1u);
    float3 wall = shop ? float3(0.90, 0.89, 0.86) : ROOM_WALLS[min(uint(h.x * 4.0), 3u)];
    float3 floor_colour = ROOM_FLOORS[shop ? 2u : min(uint(h.y * 3.0), 2u)];
    float3 c;
    float3 q = p - lo; // the hit in the room's corner frame
    float width = hi.x - lo.x;
    if (t == far.z)
    {
        c = wall * 0.8;
        if (shop)
            c = shop_shelves(q.x, q.y, id, c);
        else
        {
            // A picture on the back wall, over the furniture.
            float2 at = float2(q.x - width * (0.3 + 0.4 * g.x), q.y - 1.75);
            if (abs(at.x) < 0.45 && abs(at.y) < 0.32)
                c = abs(at.x) > 0.41 || abs(at.y) > 0.28 ? float3(0.12, 0.10, 0.08) : hash_color(id ^ 0x77u) * 0.6 + 0.15;
        }
    }
    else if (t == far.y)
        c = d.y < 0.0 ? floor_colour : ROOM_CEILING;
    else if (shop)
        c = shop_shelves(q.z, q.y, id ^ 0x5u, wall);
    else
    {
        c = wall;
        // A door in one of the side walls, towards the back.
        bool door_side = g.y < 0.5 ? p.x <= lo.x + 1e-3 : p.x >= hi.x - 1e-3;
        float back = q.z;
        if (door_side && back > 0.6 && back < 1.5 && q.y < 2.1)
            c = float3(0.36, 0.26, 0.18);
    }
    // A low piece of furniture against the back wall (a sideboard, a sofa), in front of the
    // wall behind it when the ray meets it first.
    float3 f_lo = float3(lo.x + width * (0.2 + 0.3 * g.z), lo.y, lo.z);
    float3 f_hi = float3(f_lo.x + 1.8, lo.y + 0.8, lo.z + 0.7);
    float3 n0 = (f_lo - o) * inv;
    float3 n1 = (f_hi - o) * inv;
    float t_in = max(max(min(n0.x, n1.x), min(n0.y, n1.y)), min(n0.z, n1.z));
    float t_out = min(min(max(n0.x, n1.x), max(n0.y, n1.y)), max(n0.z, n1.z));
    if (t_in <= t_out && t_in > 0.0 && t_in < t)
    {
        float3 fp = o + d * t_in;
        // A shop's counter in pale wood, a home's sofa or sideboard in its fabric.
        float3 fabric = shop ? float3(0.62, 0.52, 0.40) : hash_color(id ^ 0x3C1u) * 0.35 + 0.1;
        c = fp.y >= f_hi.y - 1e-3 ? fabric * 1.3 : fabric;
        p = fp;
    }
    c *= lerp(1.0, 0.45, saturate((o.z - p.z) / -room_low.z));
    // Far away, where a pixel covers a hand's width of the room, its edges flicker as the camera
    // moves (the room shifts behind the glass with the view): it fades into its mean colour.
    float3 mean = (wall * 0.5 + floor_colour * 0.3 + ROOM_CEILING * 0.2) * 0.65;
    return lerp(c, mean, smoothstep(0.06, 0.25, footprint));
}

// A standard row's light inputs (D-086): worked out before its lamps, so their highlights take
// the pixel's own normal, lobe and reflectance; `n` the surface's normal before a pane's bend (its
// room's daylight takes it), `uv` read by its UVs, `interior` a pane over a room.
struct StandardInputs
{
    LightInputs li;
    float3 n;
    bool uv;
    bool interior;
};

// The inputs of a row that shades as the default class, from its factors alone: the other
// classes' lamps.
StandardInputs row_inputs(Material mat, Surface s)
{
    StandardInputs si;
    si.li.albedo = mat.color_a;
    si.li.n = s.n;
    si.li.power = mat.specular_power;
    si.li.specular = mat.specular;
    si.li.ao = s.ao;
    si.li.reflectance = float3(mat.reflectance);
    si.li.emissive = float3(0.0);
    si.li.metal = 0.0;
    si.li.variance = 0.0;
    si.n = s.n;
    si.uv = false;
    si.interior = false;
    return si;
}

float3 shade_standard(Frame* f, Material mat, Surface s)
{
    return standard_light(f, mat, s, standard_inputs(f, mat, s));
}

StandardInputs standard_inputs(Frame* f, Material mat, Surface s)
{
    float3 albedo;
    float3 n = s.n;
    // The light's inputs from the row's path, the light worked out once (#348).
    LightInputs li;
    li.n = n;
    li.power = mat.specular_power;
    li.specular = mat.specular;
    li.ao = s.ao;
    li.reflectance = float3(mat.reflectance);
    li.metal = 0.0;
    li.variance = 0.0;
    bool uv = false;
    bool interior = false;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
    {
        if ((mat.flags & MATERIAL_FLAG_UV) != 0 && s.has_uv)
        {
            li = uv_mapped_light(f, mat, s);
            uv = true;
        }
        else if ((mat.flags & MATERIAL_FLAG_INTERIOR) != 0)
        {
            // The pane: its reflection over dark glass, its normal bent by its own condition
            // (#339, as the clear glass's): neighbouring windows reflect the street askew.
            interior = true;
            li.albedo = mat.color_a;
            li.n = window_pane_normal(n, s);
            li.emissive = float3(0.0);
        }
        else
        {
            standard_surface(mat, s, albedo, n);
            apply_decals(f, s, albedo);
            li.albedo = albedo;
            li.n = n;
            li.emissive = emission(f, mat);
        }
    }
    else
    {
        li.albedo = albedo;
        li.emissive = emission(f, mat);
    }
    StandardInputs si;
    si.li = li;
    si.n = n;
    si.uv = uv;
    si.interior = interior;
    return si;
}

// A standard row's light from its inputs (`standard_inputs`).
float3 standard_light(Frame* f, Material mat, Surface s, StandardInputs si)
{
    LightInputs li = si.li;
    float3 n = si.n;
    float ndl;
    float3 lit = sun_light(f, li.albedo, li.n, s.n, li.power, li.specular, s.shadow, s.sky, li.ao, s.view, li.reflectance, s.world, s.probes, s.traced, ndl) + li.emissive;
    if (si.uv && (mat.flags & MATERIAL_FLAG_FOLIAGE) != 0)
        lit += leaf_transmission(f, li.albedo, mat.scatter, s);
    if (si.interior)
    {
        float3 pane = lit;
        // The room behind, seen along the view into the mesh's space.
        float3 d = normalize(mul(transpose(s.frame), -s.view));
        if (d.z > -1e-3)
            return pane;
        // Each window's room by where it stands, so a city that gains or loses an instance
        // keeps its rooms (by its index, every window after it changed room).
        bool shop = (mat.flags & MATERIAL_FLAG_SHOP) != 0;
        float3 room = interior_room(s.object_position, d, s.instance_place, max(length(s.dpdx), length(s.dpdy)), shop);
        float3 h = hash_color(s.instance_place);
        // A shop keeps its window bare.
        float curtain = !shop && h.x < ROOM_CURTAINS ? 0.35 + 0.5 * h.x / ROOM_CURTAINS : 0.0;
        // Lit by the daylight the window gets (a white wall's facing out), or the row's lamp.
        float ndl_white;
        // Its own `sky_reflection` would overwrite the pane's mirror ray's request.
        float4 request = g_reflection_request;
        float3 daylight = sun_light(f, float3(1.0, 1.0, 1.0), n, s.n, 1.0, 0.0, s.shadow, s.sky, s.ao, s.view, float3(0.0), s.world, s.probes, false, ndl_white);
        g_reflection_request = request;
        float3 lamp = emission(f, mat) * (shop ? ROOM_SHOP_LAMP : ROOM_LAMP);
        float3 seen = lerp(room * (daylight * ROOM_DAYLIGHT + lamp), mat.color_b * (daylight + lamp), curtain);
        // Through the glass, less what it reflects (Schlick).
        float fresnel = mat.reflectance + (1.0 - mat.reflectance) * pow(1.0 - saturate(dot(n, s.view)), 5.0);
        return pane + seen * (1.0 - fresnel);
    }
    return lit;
}

// The wet sand (issue #105). The swash's sheet is a function of position and time
// (shore.slang), so the ground knows without a texture where it ran up: the highest reach over
// the last WET_LOOKBACK instants WET_STEP seconds apart, each dimmed by the drying since
// (WET_DRYING seconds to lose 1/e). The ground beyond the swash's reach is dry, and the floor
// under the sea is left to the water, which shades what it covers.
static const uint WET_LOOKBACK = 14;
static const float WET_STEP = 1.5;
static const float WET_DRYING = 25.0;
// Metres of height above the sheet's reach over which the sand goes from wet to dry.
static const float WET_DAMP = 0.25;
// What the water film does to the ground (Lagarde, "Water drop 3a/3b: Physically based wet
// surfaces", 2013): light bounces inside it and is absorbed, darkening a porous surface, and it
// smooths the surface to water's own reflection.
static const float WET_DARKENING = 0.5;
static const float WET_POWER = 400.0;
static const float WET_SPECULAR = 0.5;
static const float WATER_F0 = 0.02;

float shore_wetness(ShoreGround* g, float3 world)
{
    float2 uv = ((world.xz - g[0].frame.xy) * g[0].frame.z + 0.5) / g[0].frame.w;
    float distance = sample_image_level(g[0].shore, SAMPLER_LINEAR_CLAMP, uv, 0.0).y;
    if (world.y < 0.0 || distance >= SHORE_SWASH_REACH)
        return 0.0;
    float set = shore_set(world.xz);
    float drift = shore_drift(world.xz);
    float reach = shore_reach(distance);
    uint count = min(g[0].train_count, 4u);
    float wet = 0.0;
    for (uint i = 0; i < WET_LOOKBACK; i++)
    {
        float ago = WET_STEP * float(i);
        float level = 0.0;
        for (uint t = 0; t < count; t++)
            level += SHORE_RUN_UP * g[0].trains[t].y * set * shore_rush(g[0].trains[t], g[0].time - ago, drift);
        // Damp for a hand's height above where the sheet reached (the water the sand draws up).
        float reached = smoothstep(-WET_DAMP, 0.02, level * reach - world.y);
        wet = max(wet, reached * exp(-ago / WET_DRYING));
    }
    return wet;
}

// The caustics under the sea (#108): the waves bend the sun's light into the water and focus it
// under their crests. The light reaching the floor `depth` metres down came through the point
// of the surface up the sun's ray; a tilt of the surface turns the ray by `CAUSTICS_REFRACTION`
// of the tilt, so the surface maps onto the floor with the Jacobian I + depth · k · H (H the
// waves' Hessian there, from the cascades' slopes), and the floor's sunlight is the surface's
// over its determinant (the area the light crossed over the area it lands on): brighter where
// the crests focus it, darker between, the mean about kept. The Hessian is
// taken over a step as wide as the sun's disc blurs a caustic at that depth or the pixel spans,
// at the mip of the slopes whose texels match it, so far and deep caustics soften rather than
// alias.
static const float CAUSTICS_REFRACTION = 1.0 - 1.0 / 1.333;
// Metres past which the focusing grows no stronger. Deeper, the map folds over (the crests
// focus the light before it reaches the floor), and the light of the other points that land
// on a point of the floor, which one point's Jacobian does not see, fills the dark cells in.
static const float CAUSTICS_FOCUS = 3.0;
// The sun's disc, radians across: its light spreads over the depth times this.
static const float CAUSTICS_SUN_SPREAD = 0.0093;
// The brightest a caustic is drawn, times the surface's light.
static const float CAUSTICS_MAX = 6.0;
// The caustics fade with the depth, the light scattered on its way down (metres to 1 / e of
// their contrast), and far away, where a pixel spans metres of a pattern the finest waves
// draw a metre across (metres a pixel spans over which they fade out).
static const float CAUSTICS_FADE_DEPTH = 25.0;
static const float2 CAUSTICS_FADE_FOOTPRINT = float2(0.5, 1.5);

float shore_caustics(ShoreGround* g, float3 world, float footprint)
{
    float depth = -world.y;
    float3 t = g[0].sun_water.xyz;
    float fade = exp(-depth / CAUSTICS_FADE_DEPTH) * (1.0 - smoothstep(CAUSTICS_FADE_FOOTPRINT.x, CAUSTICS_FADE_FOOTPRINT.y, footprint));
    if (depth <= 0.0 || fade < 0.02 || g[0].cascade_count == 0u || t.y >= -1e-3)
        return 1.0;
    // Up the sun's ray to the surface, and the floor's depth there, which the waves feel.
    float2 at = world.xz + t.xz * (depth / t.y);
    float2 uv_shore = ((at - g[0].frame.xy) * g[0].frame.z + 0.5) / g[0].frame.w;
    float floor_depth = -sample_image_level(g[0].shore, SAMPLER_LINEAR_CLAMP, uv_shore, 0.0).x;
    float step = max(footprint, depth * CAUSTICS_SUN_SPREAD);
    float3 h = 0.0; // ∂²h/∂x², ∂²h/∂z², ∂²h/∂x∂z
    for (uint c = 0; c < min(g[0].cascade_count, 3u); c++)
    {
        CausticCascade cc = g[0].cascades[c];
        float a = floor_depth > 0.0 && cc.a.w > 0.0 ? min(sqrt(tma(cc.a.z, floor_depth) / cc.a.w), 1.0) : 0.0;
        if (a <= 0.0)
            continue;
        float e = max(step, cc.a.y);
        float mip = log2(e / cc.a.y);
        float2 uv = at * cc.a.x;
        float du = e * cc.a.x;
        float2 xp = sample_image_level(cc.b.x, SAMPLER_LINEAR_REPEAT, uv + float2(du, 0.0), mip).xy;
        float2 xm = sample_image_level(cc.b.x, SAMPLER_LINEAR_REPEAT, uv - float2(du, 0.0), mip).xy;
        float2 zp = sample_image_level(cc.b.x, SAMPLER_LINEAR_REPEAT, uv + float2(0.0, du), mip).xy;
        float2 zm = sample_image_level(cc.b.x, SAMPLER_LINEAR_REPEAT, uv - float2(0.0, du), mip).xy;
        h += a * float3(xp.x - xm.x, zp.y - zm.y, zp.x - zm.x) / (2.0 * e);
    }
    float3 k = min(depth, CAUSTICS_FOCUS) * CAUSTICS_REFRACTION * h;
    float det = (1.0 + k.x) * (1.0 + k.y) - k.z * k.z;
    return lerp(1.0, min(1.0 / max(abs(det), 1e-4), CAUSTICS_MAX), fade);
}

// A layered row's contour (`RenderLayer::contour`, the island's sand, #106): the rule that made
// the layer map, at the drawn mesh's resolution. Where the four texels around a pixel mix its
// layers under the height and those over it, the ground's height under the pixel shares their
// weight between their texels under and their texels over (#199): its edge then
// follows the mesh, not the map's texels, and stays within the texels that mix. The height
// wanders over two octaves of noise 7 m and 2.3 m wide, the layers blend over CONTOUR_BAND
// metres of it, or the height a pixel spans, and the share is sharpened to an edge broken up by
// finer noise (`contour_edge`).
static const float CONTOUR_BAND = 0.08;
static const uint NO_LAYER = 0xFFFFFFFFu;

// Whether layer `id` is one of the contour's layers under its height.
bool contour_low(Material mat, uint id)
{
    return id == mat.contour - 1u || (id < 32u && ((mat.contour_below >> id) & 1u) != 0u);
}

bool contour_own(Material mat, uint id)
{
    return contour_low(mat, id) || (id < 32u && ((mat.contour_above >> id) & 1u) != 0u);
}

// The height over the contour's beyond which a pixel is wholly over it (or, negated, under it),
// whatever the wander: `span` is the height the pixel spans (0 for a ray's hit).
float contour_reach(Material mat, float span)
{
    return mat.contour_wander + max(CONTOUR_BAND, span);
}

// How much of the pixel lies under the contour's height, 0..1. The noise only within reach.
float contour_under(Material mat, float3 object_position, float span)
{
    float over = object_position.y - mat.contour_height;
    float reach = contour_reach(mat, span);
    if (abs(over) >= reach)
        return over > 0.0 ? 0.0 : 1.0;
    float2 q = object_position.xz;
    float wander = (0.7 * value_noise(q / 7.0) + 0.3 * value_noise(q / 2.3 + 17.0) - 0.5) * 2.0 * mat.contour_wander;
    float band = max(CONTOUR_BAND, span);
    return 1.0 - smoothstep(-band, band, over - wander);
}

// How far into the four texels' weight from a set of one side only the height decides wholly
// (#199).
static const float CONTOUR_MIXED = 0.25;
// The edge's breakup: how far a share moves over noise 1.7 m and 0.6 m wide, and the least
// half width of its step.
static const float CONTOUR_BREAKUP = 0.3;
static const float CONTOUR_STEP = 0.06;

// How much of a pixel lies under the contour, `under`, sharpened to an edge broken up by fine
// noise (tufts of grass into the sand, #199) instead of a smooth ramp metres wide. The step
// widens with the pixel's `footprint` (metres), so far away it is a blend, not an aliased edge.
float contour_edge(float under, float2 at, float footprint)
{
    float breakup = (0.6 * value_noise(at / 1.7 + 41.0) + 0.4 * value_noise(at / 0.6 + 7.3) - 0.5)
        * 2.0 * CONTOUR_BREAKUP;
    float step = clamp(0.5 * footprint, CONTOUR_STEP, 0.5);
    return smoothstep(0.5 - step, 0.5 + step, under + breakup);
}

// Adds `weight` of layer `id` to the up to three layers `layer` holds (NO_LAYER where free).
void add_layer(inout uint layer[3], inout float weight[3], uint id, float w)
{
    if (w <= 0.0)
        return;
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        if (layer[k] == id)
        {
            weight[k] += w;
            return;
        }
    }
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        if (layer[k] == NO_LAYER)
        {
            layer[k] = id;
            weight[k] = w;
            return;
        }
    }
}

// Ground in layers (issue #42): the row at `row` names a layer map (one layer id per texel,
// over `1 / inv_texture_scale` metres of the object's x and z, centred on its origin); layer k
// is the standard row `row + 1 + k`. The two layers weighing most among the four texels
// around the pixel are shaded and blended by their bilinear weights (where the row's contour
// splits them into three, the two heaviest).
float3 shade_layered(Frame* f, uint row, Material mat, Surface s)
{
    // A layered ground is one surface, however many meshes draw it (the island's tiles, #106):
    // its layers take the first instance's tint and place in their textures, not each tile's.
    s.instance_id = 0u;
    float3 albedo;
    float3 n = s.n;
    float specular_power = mat.specular_power;
    float specular = mat.specular;
    float3 emissive = emission(f, mat);
    float reflectance = mat.reflectance;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
    {
        uint first, second;
        float w_first, w_second;
        // A planet's colour map (#220): its layers times `tint`, then `body.rgb` in place of them by
        // `body.a`.
        float3 tint = 1.0;
        float4 body = 0.0;
        if ((mat.flags & MATERIAL_FLAG_PLANET) != 0u)
            planet_layers(f, s, first, second, w_first, w_second, tint, body);
        else
            map_layers(mat, s, first, second, w_first, w_second);
        Material a = f[0].materials[row + 1 + first];
        float3 albedo_a, n_a;
        standard_surface(a, s, albedo_a, n_a);
        albedo = albedo_a;
        n = n_a;
        specular_power = a.specular_power;
        specular = a.specular;
        reflectance = a.reflectance;
        emissive = emission(f, a);
        if (second != first && w_second > 0.0)
        {
            Material b = f[0].materials[row + 1 + second];
            float3 albedo_b, n_b;
            standard_surface(b, s, albedo_b, n_b);
            float blend = w_second / (w_first + w_second);
            albedo = lerp(albedo_a, albedo_b, blend);
            n = normalize(lerp(n_a, n_b, blend));
            specular = lerp(a.specular, b.specular, blend);
            reflectance = lerp(a.reflectance, b.reflectance, blend);
            emissive = lerp(emission(f, a), emission(f, b), blend);
            // A planet's water and land meet along every coast (#220): with the heavier row's power
            // alone, the land's broad highlight took half the water's strength there, a bright line.
            if ((mat.flags & MATERIAL_FLAG_PLANET) != 0u)
                specular_power = lerp(a.specular_power, b.specular_power, blend);
        }
        albedo = lerp(albedo * tint, body.rgb, body.a);
        apply_decals(f, s, albedo, n, true);
    }
    float3 shadow = s.shadow;
    if (s.shore != nullptr)
    {
        float wet = shore_wetness(s.shore, s.world);
        albedo *= lerp(1.0, WET_DARKENING, wet);
        specular_power = lerp(specular_power, WET_POWER, wet);
        specular = lerp(specular, WET_SPECULAR, wet);
        reflectance = lerp(reflectance, WATER_F0, wet);
        // Under the sea, the waves' caustics (#108).
        if (s.world.y < 0.0 && s.shore[0].cascade_count > 0u)
            shadow *= shore_caustics(s.shore, s.world, max(length(s.dpdx), length(s.dpdy)));
    }
    float ndl;
    return sun_light(f, albedo, n, s.n, specular_power, specular, shadow, s.sky, s.ao, s.view, float3(reflectance), s.world, s.probes, s.traced, ndl) + emissive;
}

// A planet's layers (#220): the standard rows after its layered row are the sea, sand, grass,
// rock and snow (PLANET_*), their textures on the scene's frame, weighed by the surface's height over the sea's level, its slope and
// its latitude, from the frame's planet (`Frame::planet`); the two heaviest are shaded. The
// height is taken as `(h(2R + h) + 2c·p + p·p) / (|c + p| + R)` from the camera's own height
// `h`, which the CPU works out in f64: the f32 offset `c` from the planet's centre, thousands of
// kilometres long, would round it by half a metre.
static const uint PLANET_SEA = 0;
static const uint PLANET_SAND = 1;
static const uint PLANET_GRASS = 2;
static const uint PLANET_ROCK = 3;
static const uint PLANET_SNOW = 4;

void planet_layers(Frame* f, inout Surface s, out uint first, out uint second, out float w_first, out float w_second, out float3 tint, out float4 body)
{
    float3 c = f[0].planet.xyz;
    float h = f[0].planet.w;
    float radius = f[0].planet_axis.w;
    float3 p = s.world - f[0].camera_in_scene.xyz;
    float3 out_of_centre = c + p;
    float distance = length(out_of_centre);
    float height = (h * (2.0 * radius + h) + 2.0 * dot(c, p) + dot(p, p)) / (distance + radius);
    float3 up = out_of_centre / distance;
    float slope = 1.0 - saturate(dot(s.n, up));
    float latitude = abs(dot(up, f[0].planet_axis.xyz));
    float footprint = length(s.dpdx) + length(s.dpdy);
    // The edges wander over noise a few hundred metres and a few metres wide. The narrow one
    // fades to its mean where a pixel spans its width: unfiltered, it speckled the snow's edges
    // from a few kilometres up, and would shimmer as the camera moves.
    float narrow = lerp(0.5 * value_noise3(s.world / 6.0), 0.25, smoothstep(1.5, 6.0, footprint));
    float wander = value_noise3(s.world / 700.0) + narrow - 0.75;
    // Where a pixel spans more than the layers' textures show (from orbit and high up), the
    // body's maps stand for the ground (#220): its colour and its sea, which follow the maps' own
    // texels rather than a coarse tile's triangles, kilometres wide.
    float far = smoothstep(30.0, 400.0, footprint);
    // The pixel's latitude and longitude on the maps, and the level whose texels are its
    // footprint (the maps share the projection; each sampled at its own level).
    float3 axis = f[0].planet_axis.xyz;
    float3 meridian = f[0].planet_meridian.xyz;
    const float TAU = 6.28318530718;
    float latitude_angle = asin(clamp(dot(up, axis), -1.0, 1.0));
    float longitude = atan2(dot(up, cross(axis, meridian)), dot(up, meridian));
    float2 uv = float2(longitude / TAU + 0.5, 0.5 - latitude_angle / (0.5 * TAU));
    // The sea's edge: the tile's map's coast where it has one, a bilinear contour at its texels
    // (#220); otherwise the triangles' height, whose contour runs straight across each of them,
    // kilometres on a coarse tile.
    float sea = 0.0;
    if (f[0].planet_map.y != 0u)
        sea = s.coast >= 0.0 ? 1.0 - smoothstep(0.49, 0.51, s.coast) : 1.0 - smoothstep(0.02, 0.4, height);
    uint sea_mask = f[0].planet_map.w;
    if (sea_mask != 0xFFFFFFFFu && far > 0.0)
    {
        uint2 size;
        g_sampled_images[NonUniformResourceIndex(sea_mask)].GetDimensions(size.x, size.y);
        float level = max(log2(max(footprint, 1e-3) / (TAU * radius / float(size.x))), 0.0);
        float mapped = sample_image_level(sea_mask, SAMPLER_LINEAR_REPEAT, uv, level).r;
        sea = lerp(sea, smoothstep(0.3, 0.7, mapped), far);
    }
    // A beach is a few tens of metres wide: where a pixel spans more, the sand under 6 m would
    // be the width of the triangles between the sea's samples and the land's, kilometres on a
    // coarse tile.
    float sand = (1.0 - sea) * (1.0 - smoothstep(2.0, 6.0, height + 3.0 * wander)) * (1.0 - smoothstep(0.15, 0.3, slope))
        * (1.0 - smoothstep(10.0, 40.0, footprint));
    // Water lies flat: where the mask makes sea of a coarse tile's coastal slope, the slope's own
    // normal would catch the sun's highlight along every coast.
    s.n = normalize(lerp(s.n, up, sea));
    float snow_line = lerp(4200.0, 300.0, latitude * latitude) + 400.0 * wander;
    float snow = (1.0 - sea) * smoothstep(snow_line - 150.0, snow_line + 150.0, height) * (1.0 - smoothstep(0.45, 0.6, slope));
    float rock = (1.0 - sea) * smoothstep(0.28, 0.4, slope + 0.08 * wander) * (1.0 - snow);
    float grass = max(1.0 - sea - sand - snow - rock, 0.0);
    float weights[5] = { sea, sand, grass, rock, snow };
    first = 0u;
    [unroll]
    for (uint k = 1; k < 5; k++)
        first = weights[k] > weights[first] ? k : first;
    second = first;
    w_first = weights[first];
    w_second = 0.0;
    [unroll]
    for (uint k = 0; k < 5; k++)
    {
        if (k != first && weights[k] > w_second)
        {
            second = k;
            w_second = weights[k];
        }
    }
    // The body's colour map, where it has one: a tint (the Moon's: its regolith rows take its
    // colours), or the ground's albedo from afar (the Earth's), the layers near the ground.
    tint = 1.0;
    body = 0.0;
    uint map = f[0].planet_map.x;
    if (map != 0xFFFFFFFFu)
    {
        uint2 size;
        g_sampled_images[NonUniformResourceIndex(map)].GetDimensions(size.x, size.y);
        float level = max(log2(max(footprint, 1e-3) / (TAU * radius / float(size.x))), 0.0);
        float3 colour = sample_image_level(map, SAMPLER_LINEAR_REPEAT, uv, level).rgb;
        if (f[0].planet_map.z == 0u)
            tint = colour;
        else
            body = float4(colour, far);
    }
    // The layers' textures are laid on the scene's frame, one surface over every tile: a tile's own
    // frame is its cell's, so they would jump at its edges. One instance too, for the hex tiling's
    // offset, which is an instance's own.
    s.object_position = s.world;
    s.object_normal = s.n;
    s.frame = float3x3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
    s.instance_id = 0u;
}

// A layered row's layers from its layer map (issue #42): the two heaviest of the four texels
// around the pixel, their weights, the contour's share between them (#199).
void map_layers(Material mat, Surface s, out uint first, out uint second, out float w_first, out float w_second)
{
    {
        Texture2D<uint> map = g_sampled_images_u32[NonUniformResourceIndex(mat.albedo_texture)];
        uint2 size;
        map.GetDimensions(size.x, size.y);
        float2 texel = (s.object_position.xz * mat.inv_texture_scale + 0.5) * float2(size) - 0.5;
        // The lookup wanders by `cavity` texels over a noise three texels wide, so the layers'
        // edges do not follow the map's grid in steps (the island's coast, #106), and by a
        // third of that over a noise a texel wide, which breaks the bilinear blend's straight
        // runs between the steps (#199).
        if (mat.cavity > 0.0)
        {
            float2 q = texel / 3.0;
            float2 r = texel * 1.1 + 5.3;
            texel += (float2(value_noise(q), value_noise(q + 31.7)) - 0.5) * (2.0 * mat.cavity)
                + (float2(value_noise(r), value_noise(r + 13.1)) - 0.5) * (0.67 * mat.cavity);
        }
        int2 i0 = int2(floor(texel));
        float2 t = texel - float2(i0);
        uint ids[4];
        float weights[4] = { (1.0 - t.x) * (1.0 - t.y), t.x * (1.0 - t.y), (1.0 - t.x) * t.y, t.x * t.y };
        [unroll]
        for (uint k = 0; k < 4; k++)
        {
            int2 at = clamp(i0 + int2(k & 1u, k >> 1), int2(0, 0), int2(size) - 1);
            ids[k] = map.Load(int3(at, 0));
        }
        // Each texel's layer weighs the four weights of the texels that share it; the two
        // heaviest layers are shaded.
        float total[4];
        [unroll]
        for (uint k = 0; k < 4; k++)
        {
            total[k] = 0.0;
            [unroll]
            for (uint j = 0; j < 4; j++)
                total[k] += ids[j] == ids[k] ? weights[j] : 0.0;
        }
        uint heaviest = 0;
        [unroll]
        for (uint k = 1; k < 4; k++)
            heaviest = total[k] > total[heaviest] ? k : heaviest;
        first = ids[heaviest];
        second = first;
        w_first = total[heaviest];
        w_second = 0.0;
        [unroll]
        for (uint k = 0; k < 4; k++)
        {
            if (ids[k] != first && total[k] > w_second)
            {
                second = ids[k];
                w_second = total[k];
            }
        }
        // The contour shares the weight of the texels that are its own between its layer under
        // the height and its layer over, by the height where the four texels mix the two (#199):
        // a continuous function of the pixel's place, so its edge never jumps where the four
        // texels change (the old rule switched whole texels' worth of sand to grass there, in
        // straight lines and corners along the map's grid). Up to three layers, the heaviest
        // first; the texels not its own (the sea floor, the rock) keep theirs. Two are shaded:
        // the lightest of three fades out, its weight taken off the other two, so the blend
        // stays continuous where the two lightest change places (#111). A third layer shaded,
        // even flat, made NVIDIA's driver give the whole pass 128 registers instead of 96 with a
        // spill to shared memory (`FORGE_SHADER_STATS`), and cost up to 0.06 ms at 1440p on every
        // pixel of ground.
        float span = abs(s.dpdx.y) + abs(s.dpdy.y);
        float footprint = length(s.dpdx.xz) + length(s.dpdy.xz);
        // The texels' bilinear weights under and over.
        float w_under = 0.0, w_above = 0.0;
        if (mat.contour != 0u)
        {
            [unroll]
            for (uint k = 0; k < 4; k++)
            {
                if (contour_low(mat, ids[k]))
                    w_under += weights[k];
                else if (contour_own(mat, ids[k]))
                    w_above += weights[k];
            }
        }
        if (w_under > 0.0 && w_above > 0.0)
        {
            // Near the contour's height the height decides, farther from it the texels do (the
            // sand the salt water keeps over it, #199), each edge broken up, between bounds the
            // texels set: none of the layer under where its texels end, all of it where the
            // others' end.
            float share = w_under / (w_under + w_above);
            float reach = contour_reach(mat, span);
            float far = smoothstep(reach, 2.0 * reach, abs(s.object_position.y - mat.contour_height));
            float under = lerp(contour_under(mat, s.object_position, span), share, far);
            under = contour_edge(under, s.object_position.xz, footprint);
            float low_share = clamp(under, smoothstep(1.0 - CONTOUR_MIXED, 1.0, share), smoothstep(0.0, CONTOUR_MIXED, share));
            // Each side's share goes to its texels by their weights, each to its own layer, so no
            // layer stands in for another (one that did changed where the texels' heaviest did).
            float own = w_under + w_above;
            float to_under = own * low_share / w_under;
            float to_above = own * (1.0 - low_share) / w_above;
            uint layer[3] = { NO_LAYER, NO_LAYER, NO_LAYER };
            float weight[3] = { 0.0, 0.0, 0.0 };
            [unroll]
            for (uint k = 0; k < 4; k++)
            {
                float scale = contour_low(mat, ids[k]) ? to_under : contour_own(mat, ids[k]) ? to_above : 1.0;
                add_layer(layer, weight, ids[k], weights[k] * scale);
            }
            [unroll]
            for (uint i = 0; i < 2; i++)
            {
                [unroll]
                for (uint j = 0; j + 1 < 3 - i; j++)
                {
                    if (weight[j + 1] > weight[j])
                    {
                        uint id = layer[j];
                        layer[j] = layer[j + 1];
                        layer[j + 1] = id;
                        float w = weight[j];
                        weight[j] = weight[j + 1];
                        weight[j + 1] = w;
                    }
                }
            }
            float w_off = layer[2] != NO_LAYER ? weight[2] : 0.0;
            first = layer[0];
            w_first = weight[0] - w_off;
            second = layer[1] != NO_LAYER ? layer[1] : first;
            w_second = layer[1] != NO_LAYER ? weight[1] - w_off : 0.0;
        }
    }
}

// How much of the light crossing the ice leaves towards the camera rather than elsewhere
// (issue #59): the rest scatters out of the view or is absorbed on the way.
static const float ICE_GLOW = 0.3;
// Bubbly ice is white (issue #61): the bubbles within this depth (m) of the surface send the
// light back out before the ice absorbs it, and the colour tends to that of snow.
static const float ICE_WHITENING_DEPTH = 0.5;
static const float3 ICE_WHITE = float3(0.85, 0.90, 0.95);

float3 shade_ice(Frame* f, Material mat, Surface s)
{
    float3 albedo;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
    {
        // Ice: bluish, brighter on the crests than in the hollows.
        float cavity = saturate(0.5 + 0.5 * dot(normalize(s.object_position), s.n));
        albedo = lerp(mat.color_a, mat.color_b, cavity);
        albedo = lerp(albedo, ICE_WHITE, 1.0 - exp(-mat.scattering * ICE_WHITENING_DEPTH));
    }
    float ndl;
    float3 color = sun_light(f, albedo, s.n, s.n, mat.specular_power, mat.specular, s.shadow, s.sky, s.ao, s.view, float3(mat.reflectance), s.world, s.probes, s.traced, ndl);
    if (any(s.transmitted > 0.0))
    {
        // The sunlight that crossed the ice (issues #59, #61), tinted by it: the sun glows
        // through the thin edges of a backlit rock.
        color += ICE_GLOW * albedo * f[0].sun_color.rgb * s.transmitted;
    }
    return color + mat.scatter * saturate(1.0 - abs(ndl)) + emission(f, mat); // rim scatter
}

// ------------------------------------------------------------------ resolve ---

struct ResolvePush
{
    Frame* frame;
    uint* tiles;         // per class but the standard one, `tile_capacity` tiles (x | y << 16) showing it
    uint* class_args;    // per class: its dispatch (TILE_GROUPS_X, rows, 1) and its tile count
    uint vis_image;      // sampled-image index of the visibility buffer (R32_UINT)
    uint color_image;    // storage-image index of the colour target (RGBA16F)
    uint width;
    uint height;
    uint lamp_share;     // sampled index of the lamps' filtered share of their light (rgba16f, `lamp_filter_main`), or with the top bit set ReSTIR's light (restir.slang), or 0xFFFFFFFF: their own rays here
    uint tile_capacity;
    float4 background;   // written to empty pixels where its w ≥ 0 (else they are left for the sky)
    float4* sky;         // the sky's irradiance (sh.slang), or null: space's constant fill (issue #47)
    uint ao_image;       // sampled index of the ambient occlusion (gtao.slang, issue #48), or 0xFFFFFFFF; in `reflections_main`, the motion vectors' storage index (r.reflections.motion), or 0xFFFFFFFF
    uint request_image;  // storage index of the mirror rays' requests (issue #52), or 0xFFFFFFFF
    ProbeField* probes;  // the diffuse light's probes (probes.slang, issue #53), or null
    ShoreGround* shore;  // the shore whose swash wets the sand (issue #105), or null
    float4 cloud_frame;  // the clouds' shadow map (#145): its first corner (world x, z), 1 / the metres it spans
    uint cloud_shadow;   // its sampled index (r32f: the sun's share through the clouds), or 0xFFFFFFFF
    uint sun_shadow;     // sampled index of SIGMA's denoised shadow (r16f, its square root; #172), or 0xFFFFFFFF: the resolve's own ray
    ReflectionHistory* reflection_history; // the mirror rays' history (#176), or null: each frame's ray alone
};

// The mirror rays' history (#176, D-050), which `reflections_main` blends each frame's ray into.
// Mirrors `ReflectionHistoryBlock` in reflection_history.rs.
struct ReflectionHistory
{
    float4x4 previous_from_current; // the previous frame's unjittered clip from this frame's (TAA's)
    float2 jitter;                  // this frame's jitter, pixels
    uint previous;                  // sampled (rgba16f): the light the mirror rays showed so far, before the exposure (rgb); the view depth of the virtual point they showed (a; negative: a mover's; 0: none)
    uint next;                      // storage: the same for this frame, cleared before the pass
    uint first_mover;               // the movers' first instance (#79), or 0xFFFFFFFF: none
    uint mover_count;               // how many: a dynamic scene's edits add instances after them (#338)
    float blend;                    // the new ray's share of a pixel with a whole history
    uint reset;                     // 1: no history yet (the first frame, a new size)
    float near;                     // the reversed-Z projection's near plane: depth = near / view depth
    uint3 pad;
};

[[vk::push_constant]]
ResolvePush resolve_push;

// The sun's share through the clouds (#145) at `world` (the scene's frame): the shadow map holds
// it for the ground, so a point `y` up reads it `y / sun.y` along the sun's foot. 1 without
// clouds, or off the map.
float cloud_light(float3 world, float3 sun)
{
    if (resolve_push.cloud_shadow == 0xFFFFFFFFu)
        return 1.0;
    float2 ground = world.xz - sun.xz * (world.y / max(sun.y, 0.05));
    float2 uv = (ground - resolve_push.cloud_frame.xy) * resolve_push.cloud_frame.z;
    if (any(uv < 0.0) || any(uv > 1.0))
        return 1.0;
    return sample_depth_level(resolve_push.cloud_shadow, SAMPLER_LINEAR_CLAMP, uv, 0.0);
}

// The standard class shades in the full-screen pass that classifies (`resolve_standard_main`),
// so a frame of standard materials costs one pass over the visibility buffer. Every other
// class keeps a list of the 8×8 tiles that show it, and its own dispatch shades them
// (TILE_GROUPS_X groups wide and as many rows as its tiles need; `MeshletRenderer::resolve`
// starts every list at (TILE_GROUPS_X, 0, 1) and no tiles).
static const uint TILE_GROUPS_X = 64;

// How far a shadow ray starts off the surface, metres: the traced surfaces are cuts of the
// meshes' DAGs (crates/forge-render/src/raytrace.rs) and may stand a little in front of the drawn
// ones; the start also moves along the normal so grazing surfaces do not shadow themselves.
static const float SHADOW_BIAS = 0.15;
// The least start of a lamp's shadow ray off a surface, metres (#293), and of the sun's from a
// mesh that starts by its cut (#326).
static const float LAMP_SHADOW_START = 0.02;
// A terrain's ray starts this many times its errors off its drawn surface (raytrace.rs has the
// same constant for its cut's error).
static const float TERRAIN_SHADOW_START = 2.0;

// Where on the sun's disc a pixel's shadow ray aims in frame `frame` of TAA's cycle (issue
// #54), as `sun_shadow`'s (radius², turn): the eight points of a Vogel disc (the golden-angle
// spiral, evenly spread in area), one a frame, turned by an angle of the pixel's own. TAA
// averages the eight into the penumbra; the per-pixel turn dithers what eight points cannot
// resolve, where independent points per frame left streaks in the wide penumbrae of the
// ballad.
static const uint SUN_DISC_POINTS = 8;
float2 sun_disc_point(uint2 pixel, uint frame)
{
    float k = float(frame % SUN_DISC_POINTS);
    float turn = spatio_temporal_noise(pixel, 0u).x;
    return float2((k + 0.5) / float(SUN_DISC_POINTS), frac(turn + k * 0.381966011));
}

// How far a sun's shadow ray from a drawn surface of `inst` starts (#326): as the lamps'
// (`lamp_shadow_start`), twice the gap its cut may leave in front of it, from 2 cm to
// SHADOW_BIAS, so that what stands close to it shadows it (a door's leaf in its lining, a
// cornice's tier, a body resting on the ground). A mesh with cut-out sections keeps
// SHADOW_BIAS unless it asks for its cut's (`set_shadow_start_by_cut`, a negative
// `shadow_start`): a tree's leaf cards, their normals bent out from the crown, shadowed each
// other inside it from 2 cm, and the crowns lost their sunlit leaves.
// A terrain starts by its mesh's `shadow_start` scaled with the instance where that is further
// than SHADOW_BIAS (twice its cut's error, `TERRAIN_SHADOW_START` in raytrace.rs): the island's
// 8.4 M triangles of relief cut to 600 000 stand up to a metre off the drawn slopes, which
// shadowed themselves from 0.15 m (#96); the city's flat ground is no terrain and starts by its
// cut. Far away a terrain draws a
// coarser level of its DAG than the cut, up to its cluster's `self_error` further off it, so
// its ray starts twice that further too (#106: a far valley drawn under the cut shadowed
// itself in a dark blot); the levels near the camera, at no error, keep the cut's start.
float shadow_bias(Frame* f, Instance inst, Meshlet m)
{
    RtScene* rs = f[0].rt_scene;
    if (rs == nullptr)
        return SHADOW_BIAS;
    RtMesh mesh = rs[0].meshes[inst.mesh];
    float start = mesh.shadow_start;
    if (start == 0.0 && mesh.masked_sections != 0u)
        return SHADOW_BIAS;
    if (start <= 0.0)
        return clamp(2.0 * (mesh.cut_error + m.self_error) * inst.scale, LAMP_SHADOW_START, SHADOW_BIAS);
    start += TERRAIN_SHADOW_START * m.self_error;
    return max(SHADOW_BIAS, start * inst.scale);
}

// How far a lamp's shadow ray from a drawn surface of `inst` starts (#293): twice the gap the
// traced cut may leave in front of it (the cut's error, and the drawn cluster's own where a
// coarser level is drawn), at least LAMP_SHADOW_START and at most SHADOW_BIAS; a terrain's as its
// sun rays' (`shadow_bias`). From SHADOW_BIAS, nothing within 15 cm of a lit surface shadowed it:
// a fitting's housing 7 cm deep let its bulb light the ceiling round it in a halo, and a sofa
// against a lit wall cast nothing on it. The meshes cut at full detail (all but those over the
// cut's budget) stand where they are drawn near the camera.
float lamp_shadow_start(Frame* f, Instance inst, Meshlet m)
{
    RtScene* rs = f[0].rt_scene;
    if (rs == nullptr)
        return SHADOW_BIAS;
    RtMesh mesh = rs[0].meshes[inst.mesh];
    if (mesh.shadow_start > 0.0)
        return shadow_bias(f, inst, m);
    float gap = 2.0 * (mesh.cut_error + m.self_error) * inst.scale;
    return clamp(gap, LAMP_SHADOW_START, SHADOW_BIAS);
}

// 0 when something lies between `world` and the sun, else 1: one ray, the first hit ends it.
// With a sun disc (`Frame::sun_angular_radius`, issue #54) the ray aims at a point of it,
// uniform over the disc from `jitter` (two values in [0, 1): `noise.slang`'s, which repeat with
// TAA's jitter so TAA averages the points into a penumbra); (0, 0) is the disc's centre. The
// ray starts `bias` metres off the surface (`shadow_bias`; SHADOW_BIAS from a traced hit, which
// lies on the cut itself).
float sun_shadow(Frame* f, float3 world, float3 n, float2 jitter, float bias)
{
    float3 sun = normalize(f[0].sun_dir);
    if (dot(n, sun) <= 0.0)
        // Facing away: the surface's own geometry shades it. Not 1: the shading bends `n` by
        // the normal map, and a bump tilted towards the sun then caught it at full strength on
        // a wall in shade, in bright specks that sparkled under TAA's jitter.
        return 0.0;
    return ray_blocked(f, sun_ray(f, world, n, jitter, bias)) ? 0.0 : 1.0;
}

// `sun_shadow`'s ray.
RayDesc sun_ray(Frame* f, float3 world, float3 n, float2 jitter, float bias)
{
    float3 sun = normalize(f[0].sun_dir);
    float3 direction = sun;
    if (f[0].sun_angular_radius > 0.0)
    {
        float3 t = normalize(cross(sun, abs(sun.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0)));
        float3 b = cross(sun, t);
        float r = tan(f[0].sun_angular_radius) * sqrt(jitter.x);
        float phi = 6.2831853 * jitter.y;
        direction = normalize(sun + (t * cos(phi) + b * sin(phi)) * r);
    }
    RayDesc ray;
    ray.Origin = world + (n + sun) * bias;
    ray.Direction = direction;
    ray.TMin = 0.0;
    ray.TMax = 50000.0;
    return ray;
}

// The share of the sun's disc `world` sees, from SUN_REFERENCE_RAYS rays a frame aimed at the
// points of a Vogel disc, turned by an angle of the pixel's and frame's own: the reference the
// denoised shadow is judged by (`FLAG_SHADOW_REFERENCE`, #172). A development view: 256 times
// the soft shadow's cost.
static const uint SUN_REFERENCE_RAYS = 256;
float sun_shadow_reference(Frame* f, float3 world, float3 n, uint2 pixel, float bias)
{
    float turn = spatio_temporal_noise(pixel, f[0].noise_frame).x;
    float lit = 0.0;
    for (uint k = 0; k < SUN_REFERENCE_RAYS; k++)
    {
        float2 point = float2((float(k) + 0.5) / float(SUN_REFERENCE_RAYS), frac(turn + float(k) * 0.381966011));
        lit += sun_shadow(f, world, n, point, bias);
    }
    return lit / float(SUN_REFERENCE_RAYS);
}

// The sun's shadow NVIDIA's SIGMA denoised from `sun_shadow_trace_main`'s rays (#172, D-049):
// its output is the visibility's square root. A surface facing away keeps its 0, as
// `sun_shadow`'s does.
float denoised_sun_shadow(Frame* f, float3 n, uint2 pixel)
{
    if (dot(n, normalize(f[0].sun_dir)) <= 0.0)
        return 0.0;
    float stored = g_sampled_images_r[NonUniformResourceIndex(resolve_push.sun_shadow & 0xFFFFu)].Load(int3(pixel, 0));
    return saturate(stored * stored);
}

// The rays trace the static structure, then the movers' (#79), rebuilt every frame.

// The rays' flags: opaque, unless the scene has cut-out rows (#171) or jelly the shadow rays see
// through (#180). Then the instances with such rows are non-opaque in the structure
// (`tlas_instances_main`), and their triangles are candidates the query's loop keeps or passes
// (`ray_candidate_kept`, `shadow_candidate_kept`), as an any-hit shader would (#265).
uint ray_opacity(Frame* f)
{
    return (f[0].flags & (FLAG_CUTOUTS | FLAG_JELLY)) != 0 ? RAY_FLAG_NONE : RAY_FLAG_FORCE_OPAQUE;
}

// The triangle of instance `instance`'s cut a ray met: `primitive` of its structure's `geometry`,
// the second's after the first's opaque triangles (#307, `RtMesh::opaque_triangles`).
uint cut_triangle(Frame* f, uint instance, uint geometry, uint primitive)
{
    if (geometry == 0u)
        return primitive;
    RtScene* rs = f[0].rt_scene;
    return rs[0].meshes[f[0].instances[instance].mesh].opaque_triangles + primitive;
}

// The row of triangle `primitive` of instance `instance`, as the rays' scene has it.
Material candidate_row(Frame* f, uint instance, uint primitive)
{
    RtScene* rs = f[0].rt_scene;
    Instance inst = f[0].instances[instance];
    return f[0].materials[inst.material + rs[0].sections[rs[0].meshes[inst.mesh].first_triangle + primitive]];
}

// Whether a shadow ray's candidate stands: as any ray's, but the sun passes through jelly, which
// tints it instead (`jelly_transmittance`).
bool shadow_candidate_kept(Frame* f, uint instance, uint primitive, float2 b)
{
    Material mat = candidate_row(f, instance, primitive);
    if ((f[0].flags & FLAG_JELLY) != 0 && mat.class_index == MATERIAL_CLASS_JELLY)
        return false;
    // A drawn house's window lets the sun and the lamps through (#280: the sun's patches on its
    // floors, its rooms' lamps on the street).
    if (clear_glass(mat))
        return false;
    return ray_candidate_kept(f, instance, primitive, b);
}

// Whether `mat` is the clear glass of a house whose rooms are drawn: glazed and drawn after the
// resolve (D-065's part 4, `GpuMaterial::clear_glass`).
bool clear_glass(Material mat)
{
    const uint clear = MATERIAL_FLAG_GLAZED | MATERIAL_FLAG_DRAWN_AFTER;
    return (mat.flags & clear) == clear;
}

// Whether a ray's candidate hit (barycentrics `b` on triangle `primitive` of instance `instance`)
// stands: anywhere but on a cut-out row's texels whose base colour alpha falls under its cutoff,
// read at the cut's UVs (#171). The finest level: a ray has no footprint to filter over.
bool ray_candidate_kept(Frame* f, uint instance, uint primitive, float2 b)
{
    RtScene* rs = f[0].rt_scene;
    Instance inst = f[0].instances[instance];
    RtMesh mesh = rs[0].meshes[inst.mesh];
    uint tri = mesh.first_triangle + primitive;
    Material mat = f[0].materials[inst.material + rs[0].sections[tri]];
    if ((mat.flags & MATERIAL_FLAG_MASKED) == 0)
        return true;
    bool grating = (mat.flags & MATERIAL_FLAG_GRATING) != 0;
    if (grating && mesh.first_uv == RT_NO_UVS)
        return true;
    float alpha = mat.alpha;
    if ((grating || mat.albedo_texture != TEXTURE_NONE) && mesh.first_uv != RT_NO_UVS)
    {
        float2 uv[3];
        [unroll]
        for (uint k = 0; k < 3; k++)
            uv[k] = rs[0].uvs[mesh.first_uv + rs[0].indices[tri * 3 + k]];
        float2 at = uv[0] + b.x * (uv[1] - uv[0]) + b.y * (uv[2] - uv[0]);
        float2 t = float2(dot(mat.uv_u, float3(at, 1.0)), dot(mat.uv_v, float3(at, 1.0)));
        // A grating's bars themselves (`grating_coverage` with no footprint).
        if (grating)
            return frac(t.x + 0.5 * alpha) < alpha;
        alpha *= sample_image_level(mat.albedo_texture, mat.sampler, t, 0.0).a;
    }
    return alpha >= mat.alpha_cutoff;
}

// Whether a ray meets anything (a shadow ray: the first hit it keeps ends it).
bool ray_blocked(Frame* f, RayDesc ray)
{
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), ray_opacity(f), g_ray_mask, ray);
    while (q.Proceed())
    {
        if (shadow_candidate_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), q.CandidateTriangleBarycentrics()))
            q.CommitNonOpaqueTriangleHit();
    }
    if (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
        return true;
    if (movers_tlas(f) == 0)
        return false;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER> m;
    m.TraceRayInline(RaytracingAccelerationStructure(movers_tlas(f)), ray_opacity(f), g_ray_mask, ray);
    while (m.Proceed())
    {
        if (shadow_candidate_kept(f, m.CandidateInstanceID(), cut_triangle(f, m.CandidateInstanceID(), m.CandidateGeometryIndex(), m.CandidatePrimitiveIndex()), m.CandidateTriangleBarycentrics()))
            m.CommitNonOpaqueTriangleHit();
    }
    return m.CommittedStatus() == COMMITTED_TRIANGLE_HIT;
}

// The share of the sun's light, per channel, that the jelly along a shadow ray lets through
// (#180): Beer–Lambert, as the eye's ray through it (`jelly_through`), over the length the ray
// runs inside jelly. Every crossing of a jelly row counts (none committed): its distance negative
// into a body, positive out of it, so their sum is that length (of either sign, the triangles'
// facing aside), and a ray that starts inside one (the ground under its rim) counts only its way
// out. 1 in a scene without jelly. What else the ray meets is `ray_blocked`'s.
float3 jelly_transmittance(Frame* f, RayDesc ray)
{
    if ((f[0].flags & FLAG_JELLY) == 0 || f[0].rt_scene == nullptr)
        return float3(1.0);
    float3 depth = float3(0.0);
    jelly_depth_in(f, f[0].tlas, ray, depth);
    jelly_depth_in(f, movers_tlas(f), ray, depth);
    return exp(-abs(depth));
}

void jelly_depth_in(Frame* f, uint64_t tlas, RayDesc ray, inout float3 depth)
{
    if (tlas == 0)
        return;
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER> q;
    q.TraceRayInline(RaytracingAccelerationStructure(tlas), RAY_FLAG_NONE, g_ray_mask, ray);
    while (q.Proceed())
    {
        if (q.CandidateType() != CANDIDATE_NON_OPAQUE_TRIANGLE)
            continue;
        Material mat = candidate_row(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()));
        if (mat.class_index != MATERIAL_CLASS_JELLY)
            continue;
        float3 extinction = -log(clamp(mat.color_a, float3(1e-3), float3(1.0))) / JELLY_DEPTH + mat.scattering;
        float t = q.CandidateTriangleRayT();
        depth += (q.CandidateTriangleFrontFace() ? -t : t) * extinction;
    }
}

// The nearest triangle an opaque ray meets in either structure (a shadow ray, `shadow`, sees
// through jelly: `shadow_candidate_kept`).
struct ClosestHit
{
    bool hit;
    uint instance;   // its instance in the table
    uint primitive;  // its triangle in the instance's cut
    float2 barycentrics;
    float t;
    bool mover;      // met in the movers' structure (#79)
};

// Whether a probe's ray keeps a leaf it met: a quarter of the time, by a hash of the ray and the
// triangle, so a crown lets through some of the light round it as real crowns do. Kept every
// time, a crown darkened every probe about it, and the wall beside a tree took a dark blot (the
// owner, 2026-10-16: "strange shadows or lighting artifact on the wall"); kept never, the rays
// crossed whole crowns and forests (0.7-0.9 ms more on the village's green). The sun's shadows
// keep every leaf. Only the foliage's rows: the other cut-outs (an awning's fabric, a curtain, a
// sign, a railing's bars) take their alpha test whole, as the sun's rays do; at a quarter the
// probes saw the sun through three quarters of the Bistro's awnings (D-088).
static const float PROBE_LEAF_KEPT = 0.25;
bool probe_leaf_kept(Frame* f, uint instance, uint primitive, uint seed)
{
    RtScene* rs = f[0].rt_scene;
    Instance inst = f[0].instances[instance];
    uint tri = rs[0].meshes[inst.mesh].first_triangle + primitive;
    uint flags = f[0].materials[inst.material + rs[0].sections[tri]].flags;
    if ((flags & MATERIAL_FLAG_MASKED) == 0 || (flags & MATERIAL_FLAG_FOLIAGE) == 0)
        return true;
    return hash_color(seed ^ instance * 0x9E3779B9u ^ primitive * 0x85EBCA6Bu).x < PROBE_LEAF_KEPT;
}

// `probe`: a probe's ray, its `seed` its probe's and ray's (`probe_leaf_kept`).
ClosestHit trace_closest(Frame* f, RayDesc ray, bool shadow = false, bool probe = false, uint seed = 0u)
{
    ClosestHit o;
    o.hit = false;
    o.instance = 0;
    o.primitive = 0;
    o.barycentrics = 0.0;
    o.t = ray.TMax;
    o.mover = false;
    RayQuery<RAY_FLAG_NONE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), ray_opacity(f), g_ray_mask, ray);
    while (q.Proceed())
    {
        if ((!probe || probe_leaf_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), seed)) && (shadow
            ? shadow_candidate_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), q.CandidateTriangleBarycentrics())
            : ray_candidate_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), q.CandidateTriangleBarycentrics())))
            q.CommitNonOpaqueTriangleHit();
    }
    if (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
    {
        o.hit = true;
        o.instance = q.CommittedInstanceID();
        o.primitive = cut_triangle(f, q.CommittedInstanceID(), q.CommittedGeometryIndex(), q.CommittedPrimitiveIndex());
        o.barycentrics = q.CommittedTriangleBarycentrics();
        o.t = q.CommittedRayT();
    }
    if (movers_tlas(f) != 0)
    {
        // Only what stands nearer than the static hit.
        ray.TMax = o.t;
        RayQuery<RAY_FLAG_NONE> m;
        m.TraceRayInline(RaytracingAccelerationStructure(movers_tlas(f)), ray_opacity(f), g_ray_mask, ray);
        while (m.Proceed())
        {
            if ((!probe || probe_leaf_kept(f, m.CandidateInstanceID(), cut_triangle(f, m.CandidateInstanceID(), m.CandidateGeometryIndex(), m.CandidatePrimitiveIndex()), seed)) && (shadow
                ? shadow_candidate_kept(f, m.CandidateInstanceID(), cut_triangle(f, m.CandidateInstanceID(), m.CandidateGeometryIndex(), m.CandidatePrimitiveIndex()), m.CandidateTriangleBarycentrics())
                : ray_candidate_kept(f, m.CandidateInstanceID(), cut_triangle(f, m.CandidateInstanceID(), m.CandidateGeometryIndex(), m.CandidatePrimitiveIndex()), m.CandidateTriangleBarycentrics())))
                m.CommitNonOpaqueTriangleHit();
        }
        if (m.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
        {
            o.hit = true;
            o.instance = m.CommittedInstanceID();
            o.primitive = cut_triangle(f, m.CommittedInstanceID(), m.CommittedGeometryIndex(), m.CommittedPrimitiveIndex());
            o.barycentrics = m.CommittedTriangleBarycentrics();
            o.t = m.CommittedRayT();
            o.mover = true;
        }
    }
    return o;
}

// Rows at least this smooth (Blinn-Phong exponent; roughness 0.42 and below) trace their
// mirror ray (issue #50): the city's glass. Rougher rows keep the sky, blurred towards its
// irradiance (`sky_reflection`).
static const float RAY_REFLECTION_POWER = 60.0;
// How far a mirror ray travels before a hit counts, metres: the traced surfaces are cuts of
// the meshes' DAGs and may stand up to about a metre in front of the drawn ones (the towers';
// most props much less), which would otherwise hide a window behind its own facade.
static const float REFLECTION_START = 1.5;

// The least distance a mirror ray from a drawn surface travels before a hit counts, metres
// (#305): past the 5 cm the ray starts off the surface.
static const float MIRROR_START = 0.02;

// How far a mirror ray from a drawn surface of `inst` travels before a hit counts (#305): twice
// the gap the traced cut may leave in front of it (the cut's error, and the drawn cluster's own),
// at least MIRROR_START and at most REFLECTION_START; a terrain's REFLECTION_START. From
// REFLECTION_START everywhere, a glossy floor or table indoors reflected what lay past the room's
// walls (the street, a parked car), its own room's nearer walls skipped.
float mirror_start(Frame* f, Instance inst, Meshlet m)
{
    RtScene* rs = f[0].rt_scene;
    if (rs == nullptr)
        return REFLECTION_START;
    RtMesh mesh = rs[0].meshes[inst.mesh];
    if (mesh.shadow_start > 0.0)
        return REFLECTION_START;
    float gap = 2.0 * (mesh.cut_error + m.self_error) * inst.scale;
    return clamp(gap, MIRROR_START, REFLECTION_START);
}

// The surface a ray met (issues #50, #53), shaded plainly: the row's two colours mixed by the
// instance (its texture's last mip, the texture's average, for the texture). Its normal faces
// the ray; `back` says whether the ray met the triangle's back (the meshes wind
// counter-clockwise seen from outside, so a ray from inside a closed mesh meets back faces).
struct RayHit
{
    float3 position;
    float3 n;
    float3 albedo;
    float3 emissive;
    bool back;
};

// Whether a ray along `direction` met the back of triangle `primitive` of instance `instance`
// (`RayHit::back` without the shading).
bool ray_back_face(Frame* f, uint instance, uint primitive, float3 direction)
{
    RtScene* rs = f[0].rt_scene;
    Instance inst = f[0].instances[instance];
    RtMesh mesh = rs[0].meshes[inst.mesh];
    uint tri = mesh.first_triangle + primitive;
    float3 p[3];
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        uint v = mesh.first_vertex + rs[0].indices[tri * 3 + k];
        p[k] = float3(rs[0].positions[v * 3], rs[0].positions[v * 3 + 1], rs[0].positions[v * 3 + 2]);
    }
    float3 face = instance_vector(inst, cross(p[1] - p[0], p[2] - p[0]));
    // A double-sided row has no back: its leaves are not the inside of anything (#171).
    return dot(face, direction) > 0.0
        && (f[0].materials[inst.material + rs[0].sections[tri]].flags & MATERIAL_FLAG_DOUBLE_SIDED) == 0;
}

RayHit ray_hit(Frame* f, uint instance, uint primitive, float2 b, float3 origin, float3 direction, float t)
{
    return ray_hit_seen(f, instance, primitive, b, origin, direction, t, 0.0);
}

// The level of `texture` whose texels are `footprint` metres across on triangle `tri` of `mesh`
// (its face `face` as `ray_hit` scales it, once by the instance's `scale`, the hit at barycentrics
// `b`): the triangle's area over its texels'. `at`: the hit's UV, turned by `mat`'s transform. 16
// (the average) and the middle where the mesh has no UVs.
float hit_texture_level(Frame* f, RtMesh mesh, uint tri, float2 b, Material mat, float3 face, float scale, float footprint, uint texture, out float2 at)
{
    at = float2(0.5, 0.5);
    RtScene* rs = f[0].rt_scene;
    if (mesh.first_uv == RT_NO_UVS)
        return 16.0;
    float2 uv[3];
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        float2 raw = rs[0].uvs[mesh.first_uv + rs[0].indices[tri * 3 + k]];
        uv[k] = float2(dot(mat.uv_u, float3(raw, 1.0)), dot(mat.uv_v, float3(raw, 1.0)));
    }
    at = uv[0] + b.x * (uv[1] - uv[0]) + b.y * (uv[2] - uv[0]);
    uint2 size;
    g_sampled_images[NonUniformResourceIndex(texture)].GetDimensions(size.x, size.y);
    float2 e1 = (uv[1] - uv[0]) * float2(size);
    float2 e2 = (uv[2] - uv[0]) * float2(size);
    float texels = 0.5 * abs(e1.x * e2.y - e1.y * e2.x);
    float area = 0.5 * length(face) * scale;
    if (texels <= 0.0 || area <= 0.0)
        return 16.0;
    return max(log2(footprint / sqrt(area / texels)), 0.0);
}

// `ray_hit` with its maps read where it was met (D-083: the Bistro's reflections showed its
// walls flat, the stones of a pilaster and the curtains behind a window lost), at the level of
// `footprint`, the ray's width there in metres, widened as the ray grazes the surface (a ray
// cone's); with none (0), each map's average, as the probes' rays take it: read finer, the far
// facades' detail flickered in them. Only rows whose maps read the mesh's UVs (MATERIAL_FLAG_UV;
// not a layered ground's layers, nor the rows mapped by their position).
RayHit ray_hit_seen(Frame* f, uint instance, uint primitive, float2 b, float3 origin, float3 direction, float t, float footprint)
{
    RtScene* rs = f[0].rt_scene;
    Instance inst = f[0].instances[instance];
    RtMesh mesh = rs[0].meshes[inst.mesh];
    uint tri = mesh.first_triangle + primitive;
    float3 p[3];
    [unroll]
    for (uint k = 0; k < 3; k++)
    {
        uint v = mesh.first_vertex + rs[0].indices[tri * 3 + k];
        p[k] = float3(rs[0].positions[v * 3], rs[0].positions[v * 3 + 1], rs[0].positions[v * 3 + 2]);
    }
    float3 object_position = p[0] + b.x * (p[1] - p[0]) + b.y * (p[2] - p[0]);
    float3 face = instance_vector(inst, cross(p[1] - p[0], p[2] - p[0]));
    RayHit hit;
    hit.n = dot(face, face) > 1e-20 ? normalize(face) : -direction;
    hit.back = dot(hit.n, direction) > 0.0;
    if (hit.back)
        hit.n = -hit.n; // the side the ray came from
    hit.position = origin + direction * t;
    uint row = inst.material + rs[0].sections[tri];
    Material mat = f[0].materials[row];
    if ((mat.flags & MATERIAL_FLAG_DOUBLE_SIDED) != 0)
        hit.back = false; // seen from either side, its normal facing the ray (#171)
    uint tint = inst.id;
    if (mat.class_index == MATERIAL_CLASS_LAYERED)
    {
        // One surface over its tiles, as `shade_layered` shades it (#106).
        tint = 0u;
        // The layer under the hit, nearest texel (`shade_layered` blends two), and the contour's
        // layer under its height in place of one over it.
        Texture2D<uint> map = g_sampled_images_u32[NonUniformResourceIndex(mat.albedo_texture)];
        uint2 size;
        map.GetDimensions(size.x, size.y);
        float2 texel = (object_position.xz * mat.inv_texture_scale + 0.5) * float2(size);
        int2 at = clamp(int2(texel), int2(0, 0), int2(size) - 1);
        uint id = map.Load(int3(at, 0));
        if (mat.contour != 0u && contour_own(mat, id) && !contour_low(mat, id)
            && contour_under(mat, object_position, 0.0) > 0.5)
            id = mat.contour - 1u;
        mat = f[0].materials[row + 1 + id];
    }
    hit.albedo = lerp(mat.color_a, mat.color_b, hash_color(tint).x);
    bool mapped = footprint > 0.0 && (mat.flags & MATERIAL_FLAG_UV) != 0;
    float wide = footprint / max(abs(dot(hit.n, direction)), 0.1);
    if (mat.albedo_texture != TEXTURE_NONE)
    {
        if (mapped)
        {
            float2 at;
            float level = hit_texture_level(f, mesh, tri, b, mat, face, inst.scale, wide, mat.albedo_texture, at);
            hit.albedo *= sample_image_level(mat.albedo_texture, mat.sampler, at, level).rgb;
        }
        else
            hit.albedo *= sample_image_level(mat.albedo_texture, SAMPLER_LINEAR_REPEAT, float2(0.5, 0.5), 16.0).rgb;
    }
    hit.emissive = emission(f, mat);
    // What the hit emits as the frame shows it (#254): an emissive map's average (its last mip,
    // as the albedo), and a window's room its lamp's share over walls about half as bright as
    // white (`interior_room`). With the row's full value the lit windows' and the far facades'
    // light flooded the probes at night. A mirror ray's, read where it was met as its albedo.
    if (mat.emissive_texture != TEXTURE_NONE)
    {
        if (mapped)
        {
            float2 at;
            float level = hit_texture_level(f, mesh, tri, b, mat, face, inst.scale, wide, mat.emissive_texture, at);
            hit.emissive *= sample_image_level(mat.emissive_texture, mat.sampler, at, level).rgb;
        }
        else
            hit.emissive *= sample_image_level(mat.emissive_texture, SAMPLER_LINEAR_REPEAT, float2(0.5, 0.5), 16.0).rgb;
    }
    if ((mat.flags & MATERIAL_FLAG_INTERIOR) != 0)
        hit.emissive *= ((mat.flags & MATERIAL_FLAG_SHOP) != 0 ? ROOM_SHOP_LAMP : ROOM_LAMP) * 0.55;
    return hit;
}

// D-088's references for the light round a surface at `world` facing `n` (its face `geometric`),
// `r.gi.reference`: NEAR_FIELD_REFERENCE_RAYS cosine-weighted rays a pixel, no denoiser; measures,
// far from a frame's time. By `mode`:
// 1. the near field D-088 first proposed: rays to NEAR_FIELD_RANGE. A hit takes the light it
//    sends back (`hit_radiance`: the sun through a shadow ray, its albedo times its probes'
//    light); a ray that leaves the range the probes' light at its end, facing on along it.
// 2. one bounce traced to the scene's end (`path_from`), the probes' light at its hit for the
//    bounces after; the sky's own along a ray that meets nothing.
// 3. two bounces traced, the probes' light at the second hit.
// 4. a path without the probes: Forge's own diffuse reference, each hit lit by the sun and the
//    lamps, on until its weight falls under PATH_LEAST_WEIGHT.
// The hits reflect by their albedo, not their highlight: 3 to 6 % of the light under the
// Bistro's awnings in matte (2026-10-30). Irradiance in the resolve's units, as
// `probe_irradiance`'s.
static const uint GI_REFERENCE_NEAR = 1u;
static const uint GI_REFERENCE_PATH = 4u;
static const uint NEAR_FIELD_REFERENCE_RAYS = 32;
static const float NEAR_FIELD_RANGE = 2.0;
static const float PATH_RANGE = 50000.0;
static const uint PATH_MOST_BOUNCES = 32u;
static const float PATH_LEAST_WEIGHT = 0.002;

// The pixel's reference (`near_field_reference`), traced once in `shade_pixel` for every class's
// `sun_light` (one copy of its rays in the shader, #348), and whether it holds one.
static float3 g_near_irradiance = float3(0.0);
static bool g_near_on = false;
// Whether it is D-088's bounce, under GTAO's contacts, rather than a reference.
static bool g_near_contacts = false;

// A cosine-weighted direction round the unit `n` from two numbers in [0, 1).
float3 cosine_direction(float3 n, float u1, float u2)
{
    float3 t = normalize(cross(abs(n.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0), n));
    float3 b = cross(n, t);
    float r = sqrt(u1);
    float phi = 6.2831853 * u2;
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u1, 0.0)));
}

// `trace_closest` past the clear glass of a house whose rooms are drawn (two panes at most), as a
// probe's ray passes it (`probe_trace_main`, #280): the reference's rays light a room through its
// windows as the probes do. With `leaves`, the crowns' leaves as the probes' rays take them
// (`probe_leaf_kept`, a quarter of the foliage's triangles by `seed`).
ClosestHit trace_past_glass(Frame* f, RayDesc ray, bool leaves = false, uint seed = 0u)
{
    ClosestHit c = trace_closest(f, ray, false, leaves, seed);
    for (uint pane = 0u; pane < 2u && c.hit && clear_glass(candidate_row(f, c.instance, c.primitive)); pane++)
    {
        ray.TMin = c.t + 0.002;
        c = trace_closest(f, ray, false, leaves, seed);
    }
    return c;
}

// The light a path brings back from `hit`, met along `d`, in the resolve's units: each hit's sun
// and lamps, then on along a cosine-weighted ray from it, the sky's light where one meets
// nothing. After `traced` hits, the probes' light at the last for the bounces after; with 0 a path
// to its end, the probes unread.
float3 path_from(Frame* f, float4* sky, ProbeField* probes, RayHit hit, float3 d, uint seed, uint traced)
{
    float3 sum = float3(0.0);
    float3 weight = float3(1.0);
    for (uint k = 0u; k < PATH_MOST_BOUNCES; k++)
    {
        if (hit.back)
            break;
        sum += weight * (hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, (seed + k) * 0x27D4EB2Fu, false));
        weight *= hit.albedo;
        if (k + 1u == traced)
        {
            float3 ambient = sh_irradiance(sky, hit.n);
            if (probes != nullptr)
                ambient = probe_irradiance_hit(probes, hit.position, hit.n, -d, ambient, false);
            return sum + weight * ambient;
        }
        if (max(weight.x, max(weight.y, weight.z)) < PATH_LEAST_WEIGHT)
            break;
        d = cosine_direction(hit.n, hash_unit(seed + k * 2u + 101u), hash_unit(seed + k * 2u + 102u));
        RayDesc ray;
        ray.Origin = hit.position + hit.n * 0.01;
        ray.Direction = d;
        ray.TMin = 0.0;
        ray.TMax = PATH_RANGE;
        ClosestHit c = trace_past_glass(f, ray);
        if (!c.hit)
            return sum + weight * sky_along(sky, d);
        hit = ray_hit(f, c.instance, c.primitive, c.barycentrics, ray.Origin, d, c.t);
    }
    return sum;
}

float3 near_field_reference(Frame* f, float4* sky, ProbeField* probes, float3 world, float3 geometric, float3 n, uint mode)
{
    float3 t = normalize(cross(abs(n.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0), n));
    float3 b = cross(n, t);
    uint3 bits = asuint(world * 64.0);
    uint seed = (bits.x * 0x8DA6B343u) ^ (bits.y * 0xD8163841u) ^ (bits.z * 0xCB1AB31Fu) ^ (f[0].noise_frame * 0x9E3779B9u);
    float3 origin = world + geometric * 0.02;
    float3 sum = float3(0.0);
    for (uint i = 0u; i < NEAR_FIELD_REFERENCE_RAYS; i++)
    {
        // Stratified over the rays, each stratum jittered per pixel and frame.
        float u1 = (float(i) + hash_unit(seed + i * 2u)) / float(NEAR_FIELD_REFERENCE_RAYS);
        float u2 = hash_unit(seed + i * 2u + 1u);
        float r = sqrt(u1);
        float phi = 6.2831853 * u2;
        float3 d = normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u1, 0.0)));
        if (dot(d, geometric) <= 0.0)
            continue;
        RayDesc ray;
        ray.Origin = origin;
        ray.Direction = d;
        ray.TMin = 0.0;
        ray.TMax = mode == GI_REFERENCE_NEAR ? NEAR_FIELD_RANGE : PATH_RANGE;
        ClosestHit c = mode == GI_REFERENCE_NEAR ? trace_closest(f, ray) : trace_past_glass(f, ray);
        if (c.hit)
        {
            RayHit hit = ray_hit(f, c.instance, c.primitive, c.barycentrics, origin, d, c.t);
            if (mode != GI_REFERENCE_NEAR)
                sum += path_from(f, sky, probes, hit, d, seed + i * 977u, mode == GI_REFERENCE_PATH ? 0u : mode - 1u);
            else if (!hit.back)
                sum += hit_radiance(f, sky, probes, hit, d);
        }
        else if (mode == GI_REFERENCE_NEAR)
            sum += probe_irradiance(probes, origin + d * NEAR_FIELD_RANGE, d, d, -d, sh_irradiance(sky, d));
        else
            sum += sky_along(sky, d);
    }
    return sum / float(NEAR_FIELD_REFERENCE_RAYS);
}

// The light `hit` sends back from the sun alone (and its own emission), in the resolve's units.
float3 hit_sunlight(Frame* f, RayHit hit)
{
    float3 sun = normalize(f[0].sun_dir);
    float ndl = max(dot(hit.n, sun), 0.0);
    float3 shadow = float3(0.0);
    if (ndl > 0.0 && sun_shadow(f, hit.position, hit.n, float2(0.0), SHADOW_BIAS) > 0.0)
        shadow = jelly_transmittance(f, sun_ray(f, hit.position, hit.n, float2(0.0), SHADOW_BIAS));
    return hit.albedo * ndl * shadow * f[0].sun_color.rgb + hit.emissive;
}

// A lamp's unshadowed illuminance on a surface at `world` facing `n`, lux (its luminance), let go
// at its reach as `lamps_at` lets it go; 0 beyond its reach or behind the surface. Taken from
// no nearer than its bulb's radius, or `near` where that is further (a probe's hit, #294).
float lamp_weight(Lamp lamp, float3 world, float3 n, float near = 0.0)
{
    float3 d = lamp.position_radius.xyz - world;
    float dist2 = dot(d, d);
    float reach = lamp.colour_reach.w;
    if (dist2 >= reach * reach)
        return 0.0;
    float dist = sqrt(dist2);
    float ndl = dot(n, d / max(dist, 1e-6));
    if (ndl <= 0.0)
        return 0.0;
    float radius = lamp.position_radius.w;
    float window = saturate(1.0 - pow(dist / reach, 4.0));
    float luminance = dot(lamp.colour_reach.rgb, float3(0.2126, 0.7152, 0.0722));
    float least = max(radius, near);
    return luminance / max(dist2, least * least) * ndl * window * window * lamp_beam(lamp, d / max(dist, 1e-6));
}

// The lamps' light on a ray's hit (D-065: the probes lit by the lamps, and the mirror rays' hits),
// in the resolve's units: one lamp of those that light region `region` (`lamp_lights`;
// REGION_NONE: all) whose reach the hit is in, picked by its unshadowed light there, and one
// shadow ray to its middle, the light over the chance it had of being picked. A probe's
// updates average the picks as they average its rays' directions; a room's walls send its lamps'
// light back to its probes, one bounce a frame (#53). `seed`: the ray's own number, mixed with
// the frame's. `probe`: a probe's ray, which leaves the street lamps out (`hit_lamp_weight`).
float3 hit_lamps(Frame* f, RayHit hit, uint region, uint seed, bool probe)
{
    uint count = f[0].lamp_count;
    if (count == 0u || f[0].lamps == nullptr || f[0].tlas == 0)
        return float3(0.0);
    LampRuns runs = lamp_runs(f, region, region);
    float total = 0.0;
    for (uint r = 0u; r < 2u; r++)
    {
        for (uint e = runs.run[r].x; e < runs.run[r].y; e++)
            total += hit_lamp_weight(f[0].lamps[lamp_at(f, runs, e)], hit, region, probe);
    }
    if (total <= 0.0)
        return float3(0.0);
    uint n = (seed ^ (f[0].noise_frame * 0x9E3779B9u)) * 0x27D4EB2Fu;
    n ^= n >> 15;
    n *= 0x846CA68Bu;
    n ^= n >> 16;
    float pick = float(n >> 8) * (1.0 / 16777216.0) * total;
    uint chosen = count;
    for (uint r = 0u; r < 2u; r++)
    {
        if (chosen < count && pick <= 0.0)
            break;
        for (uint e = runs.run[r].x; e < runs.run[r].y; e++)
        {
            uint i = lamp_at(f, runs, e);
            float w = hit_lamp_weight(f[0].lamps[i], hit, region, probe);
            if (w <= 0.0)
                continue;
            chosen = i;
            pick -= w;
            if (pick <= 0.0)
                break;
        }
    }
    if (chosen >= count)
        return float3(0.0);
    Lamp lamp = f[0].lamps[chosen];
    // Aimed from where it starts (`lamp_ray`, #298): aimed from the hit and started SHADOW_BIAS
    // off it, the ray ran beside the line to the lamp and ended up to 15 cm from its middle, in
    // its globe or its fitting, and a room's light moved by up to an EV with the start.
    if (ray_blocked(f, lamp_ray(lamp, hit.position, hit.n, SHADOW_BIAS, float3(0.0))))
        return float3(0.0);
    // Its colour of luminance 1 times every lamp's light: its own over its chance, w / total.
    float3 colour = lamp.colour_reach.rgb / max(dot(lamp.colour_reach.rgb, float3(0.2126, 0.7152, 0.0722)), 1e-9);
    return hit.albedo * colour * (total / max(f[0].sun_illuminance, 1e-9));
}

// The light `hit` sends back along the ray that met it (travelling along `direction`), in the
// resolve's units: the sun through a shadow ray, and the sky's irradiance or, with probes,
// what they measured there (issue #53: the light bounces again, one bounce a frame).
// `outside`: the hit is the outside's (`probe_irradiance`).
float3 hit_radiance(Frame* f, float4* sky, ProbeField* probes, RayHit hit, float3 direction, bool outside = false)
{
    float3 ambient = sh_irradiance(sky, hit.n);
    if (probes != nullptr)
        ambient = probe_irradiance_hit(probes, hit.position, hit.n, -direction, ambient, outside);
    return hit_sunlight(f, hit) + hit.albedo * ambient;
}

// What the mirror ray from `origin` along `direction` meets (issue #50): the light leaving the
// cut's triangle towards the ray, in the resolve's units, with w = 1; w = 0 on a miss. `distance`:
// how far the ray went, metres (its TMax on a miss); `mover`: whether it met a mover (#79). The
// lamps light the hit as they light a probe's (`hit_lamps`, `seed` the pixel's number).
float4 trace_reflection_from(Frame* f, float4* sky, ProbeField* probes, float3 origin, float3 direction, uint seed, float start, out float distance, out bool mover)
{
    return trace_reflection_seen(f, sky, probes, origin, direction, seed, start, 0.0, 0.0, 0.0, distance, mover);
}

// `trace_reflection_from` whose hit reads its maps where it was met (`ray_hit_seen`): the ray a
// pixel's cone, `spread` radians across, from the camera `travelled` metres off the surface it
// leaves (a flat mirror's: the cone goes on as from the camera's image), widened by the
// reflector's lobe, `lobe` radians across from the surface on.
float4 trace_reflection_seen(Frame* f, float4* sky, ProbeField* probes, float3 origin, float3 direction, uint seed, float start, float spread, float travelled, float lobe, out float distance, out bool mover)
{
    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = direction;
    ray.TMin = start;
    ray.TMax = 50000.0;
    ClosestHit c = trace_closest(f, ray);
    distance = c.t;
    mover = c.mover;
    if (!c.hit)
        return float4(0.0);
    RayHit hit = ray_hit_seen(f, c.instance, c.primitive, c.barycentrics, origin, direction, c.t, spread * (travelled + c.t) + lobe * c.t);
    return float4(hit_radiance(f, sky, probes, hit, direction) + hit_lamps(f, hit, REGION_NONE, seed, false), 1.0);
}

// `trace_reflection_from` with REFLECTION_START: the water's and the glass's rays, which start on a
// surface that is not in the rays' scene, and the jelly's.
float4 trace_reflection(Frame* f, float4* sky, ProbeField* probes, float3 origin, float3 direction, uint seed, out float distance, out bool mover)
{
    return trace_reflection_from(f, sky, probes, origin, direction, seed, REFLECTION_START, distance, mover);
}

float4 trace_reflection(Frame* f, float4* sky, ProbeField* probes, float3 origin, float3 direction, uint seed)
{
    float distance;
    bool mover;
    return trace_reflection(f, sky, probes, origin, direction, seed, distance, mover);
}

// For the passes outside this file that trace the scene (#156: where the tank's bent rays land):
// the nearest surface a ray meets in either structure, and that surface's light towards the ray
// in the resolve's units, shaded plainly as a mirror ray's hit (without the probes).
public struct SceneHit
{
    public bool hit;
    public uint instance;
    public uint primitive;
    public float2 barycentrics;
    public float t; // `tmax` on a miss
};

public SceneHit trace_scene(Frame* f, float3 origin, float3 direction, float tmin, float tmax)
{
    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = direction;
    ray.TMin = tmin;
    ray.TMax = tmax;
    ClosestHit c = trace_closest(f, ray);
    SceneHit o;
    o.hit = c.hit;
    o.instance = c.instance;
    o.primitive = c.primitive;
    o.barycentrics = c.barycentrics;
    o.t = c.t;
    return o;
}

public float3 scene_hit_radiance(Frame* f, float4* sky, SceneHit c, float3 origin, float3 direction)
{
    RayHit hit = ray_hit(f, c.instance, c.primitive, c.barycentrics, origin, direction, c.t);
    return hit_radiance(f, sky, nullptr, hit, direction);
}

// Translucent ice (issues #59, #61): the sunlight that crosses the ice to reach `world`, the
// point of instance `instance` a pixel shows, and leaves it towards the camera (per unit of the
// sun's light, the phase's π included). A ray from the point towards the sun meets the
// instance's surface one or more times, within its bounding sphere; the last crossing is where
// the light went in. The rule ignores winding, and it holds whether the ray starts just inside
// or just outside the traced cut (which may stand off the drawn surface by its error). Then a
// shadow ray from there: the sun must reach it.
//
// Over that path, clear ice only absorbs (Beer–Lambert, per channel: ice absorbs red more than
// blue, so thick ice glows blue), and the light keeps the sun's direction, spread forwards by
// the surfaces (Henyey–Greenstein, g = 0.5). Bubbles (`scattering` per metre) scatter mostly
// forwards (g = 0.8). The delta-Eddington approximation keeps their forward peak, g² of what
// they scatter, in the straight beam; the rest follows the diffusion approximation. It leaves
// the far side as a Lambertian surface would (a radiance of the share × E / π, the unit here),
// dimmed by the effective attenuation √(3 σa (σa + σs')), since scattering lengthens the path
// through the absorbing ice, and by 1 / (1 + ¾ σs' L), the share a scattering slab lets
// through; σs' = σs (1 − g) is the reduced scattering.
static const float3 ICE_ABSORPTION = float3(0.16, 0.10, 0.065); // per metre
static const float BUBBLE_ANISOTROPY = 0.8;
float3 ice_transmission(Frame* f, uint instance, float3 world, float scattering)
{
    float3 sun = normalize(f[0].sun_dir);
    Instance inst = f[0].instances[instance];
    RayDesc ray;
    ray.Origin = world;
    ray.Direction = sun;
    ray.TMin = 0.0;
    ray.TMax = 2.0 * inst.radius + 1.0;
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, g_ray_mask, ray);
    float last = -1.0;
    while (q.Proceed())
    {
        // Every crossing of this instance, none committed, so the traversal sees them all.
        if (q.CandidateType() == CANDIDATE_NON_OPAQUE_TRIANGLE && q.CandidateInstanceID() == instance)
            last = max(last, q.CandidateTriangleRayT());
    }
    if (last < 0.0)
        return float3(0.0); // the ray left without crossing: the point faces the sun
    RayDesc shadow;
    shadow.Origin = world + sun * (last + SHADOW_BIAS);
    shadow.Direction = sun;
    shadow.TMin = 0.0;
    shadow.TMax = 50000.0;
    if (ray_blocked(f, shadow))
        return float3(0.0);
    float3 to_camera = normalize(f[0].camera_in_scene.xyz - world);
    float g = 0.5;
    float cos_theta = dot(-sun, to_camera);
    float phase = (1.0 - g * g) / (4.0 * 3.14159265 * pow(max(1.0 + g * g - 2.0 * g * cos_theta, 1e-4), 1.5));
    float scattered = scattering * (1.0 - BUBBLE_ANISOTROPY * BUBBLE_ANISOTROPY); // out of the forward peak
    float3 straight = exp(-(ICE_ABSORPTION + scattered) * last);
    float3 diffuse = float3(0.0);
    if (scattering > 0.0)
    {
        float reduced = scattering * (1.0 - BUBBLE_ANISOTROPY);
        float3 effective = sqrt(3.0 * ICE_ABSORPTION * (ICE_ABSORPTION + reduced));
        diffuse = (1.0 - exp(-scattered * last)) * exp(-effective * last) / (1.0 + 0.75 * reduced * last);
    }
    return straight * phase * 3.14159265 + diffuse;
}

// Jelly (#180): a clear, tinted, slightly cloudy body that the eye sees through (the slime). At a
// pixel, the view ray bends into the body (Snell, `JELLY_IOR`) and crosses it to its far side,
// found as the ice finds its crossings (D-033): every crossing of the pixel's own instance, none
// committed, the nearest past the start is where it leaves. It leaves without bending again and
// meets what lies behind, shaded plainly as a mirror ray's hit: the owner's slimes (the
// tropical island's) bend what lies behind by their front's normal alone. Bent again by the far
// side, the ray followed that side's wobbles and the cut's facets (it carries no smooth
// normals) and broke the horizon behind into hard-edged pieces. Over the path the jelly absorbs
// (Beer–Lambert, `color_a` being what `JELLY_DEPTH` of it lets through) and its cloud scatters
// (`scattering` per metre): what it scatters is the body's own colour (`color_b`), lit as a
// surface. The sunlight crossing the body towards the eye (backlit, it glows) is the ice's
// rule with the jelly's absorption. Rays from the hits behind it to the sun pass through the
// jelly, which lets most light by, so they skip it.
// Its index of refraction at the front: gelatine's is about 1.35, at which a ball of it is a lens
// showing what lies behind it upside down, a glass marble; the island's slimes bend it gently.
static const float JELLY_IOR = 1.12;
static const float JELLY_DEPTH = 0.3; // metres

// The crossings of instance `instance` along `ray` in the structure at `tlas`, none committed,
// gathered into `nearest` (with its triangle and barycentrics) and `farthest`.
struct Crossings
{
    bool any;
    float nearest;
    uint primitive;
    float2 barycentrics;
    float farthest;
};

void gather_crossings(Frame* f, uint64_t tlas, uint instance, RayDesc ray, inout Crossings c)
{
    if (tlas == 0)
        return;
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(tlas), RAY_FLAG_NONE, g_ray_mask, ray);
    while (q.Proceed())
    {
        if (q.CandidateType() != CANDIDATE_NON_OPAQUE_TRIANGLE || q.CandidateInstanceID() != instance)
            continue;
        float t = q.CandidateTriangleRayT();
        c.any = true;
        c.farthest = max(c.farthest, t);
        if (t < c.nearest)
        {
            c.nearest = t;
            c.primitive = cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex());
            c.barycentrics = q.CandidateTriangleBarycentrics();
        }
    }
}

Crossings own_crossings(Frame* f, uint instance, RayDesc ray)
{
    Crossings c;
    c.any = false;
    c.nearest = ray.TMax;
    c.primitive = 0;
    c.barycentrics = float2(0.0);
    c.farthest = -1.0;
    gather_crossings(f, f[0].tlas, instance, ray, c);
    gather_crossings(f, movers_tlas(f), instance, ray, c);
    return c;
}

// Whether a shadow ray meets anything but instance `skip` (and jelly), in either structure.
bool blocked_in(Frame* f, uint64_t tlas, RayDesc ray, uint skip)
{
    if (tlas == 0)
        return false;
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE | RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> q;
    q.TraceRayInline(RaytracingAccelerationStructure(tlas), RAY_FLAG_NONE, g_ray_mask, ray);
    while (q.Proceed())
    {
        if (q.CandidateInstanceID() != skip
            && shadow_candidate_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), q.CandidateTriangleBarycentrics()))
            q.CommitNonOpaqueTriangleHit();
    }
    return q.CommittedStatus() == COMMITTED_TRIANGLE_HIT;
}

bool ray_blocked_except(Frame* f, RayDesc ray, uint skip)
{
    return blocked_in(f, f[0].tlas, ray, skip) || blocked_in(f, movers_tlas(f), ray, skip);
}

// `trace_closest` that sees through instance `skip`: from inside the jelly, where it sits on the
// ground, the ray leaving it starts above the ground and meets it.
void closest_in(Frame* f, uint64_t tlas, inout RayDesc ray, uint skip, bool mover, inout ClosestHit o)
{
    if (tlas == 0)
        return;
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(tlas), RAY_FLAG_NONE, g_ray_mask, ray);
    while (q.Proceed())
    {
        if (q.CandidateInstanceID() != skip
            && ray_candidate_kept(f, q.CandidateInstanceID(), cut_triangle(f, q.CandidateInstanceID(), q.CandidateGeometryIndex(), q.CandidatePrimitiveIndex()), q.CandidateTriangleBarycentrics()))
            q.CommitNonOpaqueTriangleHit();
    }
    if (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
    {
        o.hit = true;
        o.instance = q.CommittedInstanceID();
        o.primitive = cut_triangle(f, q.CommittedInstanceID(), q.CommittedGeometryIndex(), q.CommittedPrimitiveIndex());
        o.barycentrics = q.CommittedTriangleBarycentrics();
        o.t = q.CommittedRayT();
        o.mover = mover;
        ray.TMax = o.t;
    }
}

ClosestHit trace_closest_except(Frame* f, RayDesc ray, uint skip)
{
    ClosestHit o;
    o.hit = false;
    o.instance = 0;
    o.primitive = 0;
    o.barycentrics = 0.0;
    o.t = ray.TMax;
    o.mover = false;
    closest_in(f, f[0].tlas, ray, skip, false, o);
    closest_in(f, movers_tlas(f), ray, skip, true, o);
    return o;
}

// `hit_radiance`, its sun's shadow ray skipping instance `skip` (the jelly it is seen through).
float3 hit_radiance_skipping(Frame* f, float4* sky, ProbeField* probes, RayHit hit, float3 direction, uint skip)
{
    float3 sun = normalize(f[0].sun_dir);
    float ndl = max(dot(hit.n, sun), 0.0);
    float lit = 0.0;
    if (ndl > 0.0)
    {
        RayDesc ray;
        ray.Origin = hit.position + (hit.n + sun) * SHADOW_BIAS;
        ray.Direction = sun;
        ray.TMin = 0.0;
        ray.TMax = 50000.0;
        lit = ray_blocked_except(f, ray, skip) ? 0.0 : 1.0;
    }
    float3 ambient = sh_irradiance(sky, hit.n);
    if (probes != nullptr)
        ambient = probe_irradiance_hit(probes, hit.position, hit.n, -direction, ambient, false);
    return hit.albedo * (ndl * lit * f[0].sun_color.rgb + ambient) + hit.emissive;
}

// The sky's light along `direction`, in the resolve's units.
float3 sky_along(float4* sky, float3 direction)
{
    float4 up_table = sky[SH_COEFFICIENTS];
    return sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, sky[SH_COEFFICIENTS + 1].xyz, direction) * 3.14159265;
}

// What the eye sees through the jelly of row `mat` at `world`, the point of instance `instance` a
// pixel shows (normal `n`, `view` towards the camera): the light from behind it, absorbed and
// thinned by the cloud, and the sunlight crossing it. `cloud` is the share of the path its cloud
// scatters (1 where no ray crosses: the jelly shows its own colour).
float3 jelly_through(Frame* f, Material mat, uint instance, float3 world, float3 n, float3 view, out float cloud)
{
    cloud = 1.0;
    Instance inst = f[0].instances[instance];
    float3 inside = refract(-view, n, 1.0 / JELLY_IOR);
    if (dot(inside, inside) < 1e-6)
        inside = -view;
    RayDesc ray;
    ray.Origin = world;
    ray.Direction = inside;
    ray.TMin = 0.002; // past the triangle it starts on
    ray.TMax = 2.0 * inst.radius + 0.1;
    Crossings c = own_crossings(f, instance, ray);
    if (!c.any)
        return float3(0.0);
    float3 absorption = -log(clamp(mat.color_a, float3(1e-3), float3(1.0))) / JELLY_DEPTH;
    float3 through = exp(-(absorption + mat.scattering) * c.nearest);
    cloud = 1.0 - exp(-mat.scattering * c.nearest);
    float4* sky = resolve_push.sky;
    ProbeField* probes = resolve_push.probes;
    RayHit exit = ray_hit(f, instance, c.primitive, c.barycentrics, world, inside, c.nearest);
    RtScene* rs = f[0].rt_scene;
    uint row = inst.material + rs[0].sections[rs[0].meshes[inst.mesh].first_triangle + c.primitive];
    float3 behind;
    if (f[0].materials[row].class_index != MATERIAL_CLASS_JELLY)
    {
        // Something of its own inside it (an eye, seen from behind).
        behind = hit_radiance_skipping(f, sky, probes, exit, inside, instance);
    }
    else
    {
        // It leaves without bending again (see above), from a little inside (the jelly's
        // bottom lies on the ground), through itself.
        float3 out_dir = inside;
        RayDesc onward;
        onward.Origin = exit.position - inside * 0.01;
        onward.Direction = out_dir;
        onward.TMin = 0.0;
        onward.TMax = 50000.0;
        ClosestHit h = trace_closest_except(f, onward, instance);
        if (h.hit)
        {
            RayHit hit = ray_hit(f, h.instance, h.primitive, h.barycentrics, onward.Origin, out_dir, h.t);
            behind = hit_radiance_skipping(f, sky, probes, hit, out_dir, instance);
        }
        else
            behind = sky_along(sky, out_dir);
    }
    // The sunlight that crossed the body to this point (backlit, it glows): the ice's rule.
    float3 sun = normalize(f[0].sun_dir);
    RayDesc toward;
    toward.Origin = world;
    toward.Direction = sun;
    toward.TMin = 0.002;
    toward.TMax = 2.0 * inst.radius + 0.1;
    Crossings s = own_crossings(f, instance, toward);
    float3 glow = float3(0.0);
    if (s.any)
    {
        RayDesc shadow;
        shadow.Origin = world + sun * (s.farthest + SHADOW_BIAS);
        shadow.Direction = sun;
        shadow.TMin = 0.0;
        shadow.TMax = 50000.0;
        if (!ray_blocked_except(f, shadow, instance))
        {
            float g = 0.5;
            float cos_theta = dot(-sun, view);
            float phase = (1.0 - g * g) / (4.0 * 3.14159265 * pow(max(1.0 + g * g - 2.0 * g * cos_theta, 1e-4), 1.5));
            float3 crossed = exp(-(absorption + mat.scattering) * s.farthest);
            // The straight beam, and what the cloud scatters of it in the jelly's colour.
            glow = f[0].sun_color.rgb * (crossed * phase * 3.14159265
                + mat.color_b * (1.0 - exp(-mat.scattering * s.farthest)) * exp(-absorption * s.farthest));
        }
    }
    return through * behind + glow;
}

// Jelly (#180): its cloud lit as a surface of its own colour, the reflections and the highlight
// on top, and what it lets through (`s.transmitted`) where the surface does not reflect. With
// rays (`traced`), its mirror ray starts at its surface and skips its own body: the mirror rays'
// pass starts theirs 1.5 m out (REFLECTION_START, for the cuts standing off the drawn surfaces),
// under the ground for a small body lying on it, while a skinned body's cut is its surface.
float3 shade_jelly(Frame* f, Material mat, Surface s, uint instance, bool traced)
{
    float3 albedo;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
        albedo = mat.color_b * s.cloud;
    float ndl;
    g_reflection_request = float4(0.0);
    float3 color = sun_light(f, albedo, s.n, s.n, mat.specular_power, mat.specular, s.shadow, s.sky, s.ao, s.view, float3(mat.reflectance), s.world, s.probes, traced, ndl);
    float4 mirror = g_reflection_request;
    if (traced && mirror.w > 0.0)
    {
        RayDesc ray;
        ray.Origin = s.world + s.n * 0.003;
        ray.Direction = mirror.xyz;
        ray.TMin = 0.0;
        ray.TMax = 50000.0;
        ClosestHit h = trace_closest_except(f, ray, instance);
        if (h.hit)
        {
            RayHit hit = ray_hit(f, h.instance, h.primitive, h.barycentrics, ray.Origin, mirror.xyz, h.t);
            color += mirror.w * (hit_radiance_skipping(f, s.sky, s.probes, hit, mirror.xyz, instance) - sky_along(s.sky, mirror.xyz));
        }
    }
    // The share the surface reflects, as `sky_reflection` weighs it.
    float roughness = sqrt(sqrt(2.0 / (mat.specular_power + 2.0)));
    float nv = saturate(dot(s.n, s.view));
    float fresnel = mat.reflectance + (max(1.0 - roughness, mat.reflectance) - mat.reflectance) * pow(1.0 - nv, 5.0);
    return color + (1.0 - fresnel) * s.transmitted + emission(f, mat);
}

// The tile list after the classes': tiles holding a smooth row whose mirror ray `reflections_main`
// traces (issue #52), and the bit `shade_pixel` returns for such a pixel.
static const uint REFLECTION_LIST = MATERIAL_CLASSES;
static const uint REFLECT_BIT = 0x100u;

// Whether a pixel of `mat` traces a mirror ray: the smooth rows, under a sky, with a scene
// structure and both reflection flags.
bool wants_mirror_ray<let SHADOWS : int>(Frame* f, Material mat)
{
    const uint flags = FLAG_SKY_REFLECTIONS | FLAG_RAY_REFLECTIONS;
    return SHADOWS != 0 && f[0].tlas != 0 && f[0].rt_scene != nullptr && resolve_push.sky != nullptr
        && (f[0].flags & flags) == flags && mat.specular_power >= RAY_REFLECTION_POWER
        && mat.class_index != MATERIAL_CLASS_JELLY // it traces its own, from its surface (#180)
        && (f[0].flags & FLAG_SHOW_GI) == 0;
}

// The turn the skin gives a bind-pose vertex `v` this frame: its joints' matrices (`base` the
// mesh's first in `Frame::skin_joints`), their 3 × 3 parts weighed as the skin pass weighs them.
// A vertex on up to eight joints (`eight`, #169) is turned by its four heaviest, weighed up to
// the whole.
float3x3 skin_turn(Frame* f, uint base, SkinVertex v, bool eight)
{
    uint4 joints = uint4(v.joints.x & 0xFFFFu, v.joints.x >> 16, v.joints.y & 0xFFFFu, v.joints.y >> 16);
    float4 weights = float4(v.weights.x & 0xFFFFu, v.weights.x >> 16, v.weights.y & 0xFFFFu, v.weights.y >> 16) / 65535.0;
    if (eight)
        weights /= max(dot(weights, float4(1.0)), 1e-6);
    float3 r0 = float3(0.0);
    float3 r1 = float3(0.0);
    float3 r2 = float3(0.0);
    for (uint k = 0; k < 4; k++)
    {
        if (weights[k] > 0.0)
        {
            uint j = 3 * (base + joints[k]);
            r0 += weights[k] * f[0].skin_joints[j].xyz;
            r1 += weights[k] * f[0].skin_joints[j + 1].xyz;
            r2 += weights[k] * f[0].skin_joints[j + 2].xyz;
        }
    }
    return float3x3(r0, r1, r2);
}

// A skinned mesh's surface as its textures see it (#166): the position, its change per pixel
// and the normal taken from the bind pose (the cluster's vertices in `Frame::skin_vertices`,
// by the pixel's barycentrics on triangle `tri`), so the projection stays on the body however
// it bends. A normal map's detail, worked out about the bind pose's normal, is turned with the
// body: `s.frame` becomes the skin's turn at the pixel (its vertices' blended joints, by the
// same barycentrics), then the instance's frame. The shading normal, the position the light
// and the rays use, and everything else stay the bent body's.
void bind_pose_surface(Frame* f, Mesh mesh, uint meshlet_index, Meshlet m, uint base, uint tri,
                       Barycentrics b, inout Surface s)
{
    SkinCluster c = f[0].skin_clusters[mesh.skin + meshlet_index - mesh.meshlet_offset];
    SkinVertex a0 = f[0].skin_vertices[c.first + paged_index(f, base, m, tri, 0)];
    SkinVertex a1 = f[0].skin_vertices[c.first + paged_index(f, base, m, tri, 1)];
    SkinVertex a2 = f[0].skin_vertices[c.first + paged_index(f, base, m, tri, 2)];
    bool eight = (c.flags & 4u) != 0u; // skin.slang's SKIN_EIGHT (#169)
    float3x3 turn = b.lambda.x * skin_turn(f, c.joints, a0, eight) + b.lambda.y * skin_turn(f, c.joints, a1, eight)
                  + b.lambda.z * skin_turn(f, c.joints, a2, eight);
    s.frame = mul(s.frame, turn);
    s.object_normal = normalize(b.lambda.x * decode_normal(a0.normal) + b.lambda.y * decode_normal(a1.normal)
                                + b.lambda.z * decode_normal(a2.normal));
    s.object_position = b.lambda.x * a0.position + b.lambda.y * a1.position + b.lambda.z * a2.position;
    s.dpdx = b.ddx.x * a0.position + b.ddx.y * a1.position + b.ddx.z * a2.position;
    s.dpdy = b.ddy.x * a0.position + b.ddy.y * a1.position + b.ddy.z * a2.position;
    // A displaced mesh (#185): its field's slopes are in the raised surface's normal, not in its
    // joint's turn. The frame turns the bind pose's normal onto that normal, so a normal map's
    // detail lies along the prints' walls as on the flat.
    if (c.field != 0xFFFFFFFFu)
        s.frame = mul(turn_onto(normalize(mul(s.frame, s.object_normal)), s.n), s.frame);
}

// The shortest turn taking unit vector `a` onto unit vector `b` (Rodrigues'), for any two not
// opposite.
float3x3 turn_onto(float3 a, float3 b)
{
    float3 v = cross(a, b);
    float c = dot(a, b);
    float k = 1.0 / max(1.0 + c, 1e-4);
    return float3x3(
        float3(v.x * v.x * k + c, v.x * v.y * k - v.z, v.x * v.z * k + v.y),
        float3(v.y * v.x * k + v.z, v.y * v.y * k + c, v.y * v.z * k - v.x),
        float3(v.z * v.x * k - v.y, v.z * v.y * k + v.x, v.z * v.z * k + c));
}

// The shadow rays' start off a grass blade, metres: the rays meet no grass.
static const float GRASS_SHADOW_BIAS = 0.02;

// A grass blade's surface (#300) for `shade_pixel`: the map's row, the blade's surface rebuilt from
// its id, each clump its own tint of the row's two (by its hash). `scale` the blade's darkening
// towards its root (the blades round it shade it), its contrast fading with distance.
void grass_surface(Frame* f, GrassHit h, uint2 pixel, inout Surface s, out float scale)
{
    s.n = h.n;
    s.object_normal = h.n;
    g_normal_variance = 0.0;
    // Shaded where it stands, as the ground it grows from is textured.
    s.object_position = h.scene;
    s.dpdx = h.dpdx * f[0].texture_lod_scale;
    s.dpdy = h.dpdy * f[0].texture_lod_scale;
    s.frame = float3x3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
    // Each clump (cell) its own of the row's two colours.
    s.instance_id = grass_hash(h.cell, 0x632BE5ABu);
    s.instance_place = s.instance_id;
    s.meshlet_index = 0u;
    s.lod_level = 0u;
    s.view = normalize(-h.relative);
    s.world = h.scene;
    s.has_uv = false;
    s.coast = -1.0;
    s.uv = float2(0.0);
    s.duvdx = float2(0.0);
    s.duvdy = float2(0.0);
    // The blades' contrast converges to their clump's mean with distance (Ghost of Tsushima's
    // rule): far, a lawn shows its colour, not stripes of dark gaps between lit tips.
    float blade = 0.88 + 0.24 * grass_unit(h.hash, 7);
    scale = lerp(lerp(0.55, 1.05, h.along) * blade, 1.0, sqrt(h.far));
}

// Shades the pixel at `pixel` whose (non-empty) visibility id is `vis`, when its material is
// of class `class_index` (a compile-time constant at every call); `SHADOWS` compiles the
// shadow ray in (devices with ray queries). Returns the class it has. A grass blade's pixel
// (#300) and a mesh's share everything after their surface: the shadows, the lamps, the class's
// shading and the debug views (#348: the grass's own copy of them made every resolve entry's
// pipeline seconds longer in the driver, each call site a whole inlined copy).
uint shade_pixel<let SHADOWS : int>(uint class_index, uint2 pixel, uint vis)
{
    Frame* f = resolve_push.frame;
    bool grass = is_grass(vis);
    Surface s;
    Material mat;
    uint row;
    // What the shared part needs of the surface's path.
    float3 surface_relative;
    float3 surface_scene;
    float sun_bias;
    float lamp_start;
    uint instance_index = 0u;
    float grass_scale = 1.0;
    bool culled_debug = false;
    bool software = false;
    bool ladder = false;
    if (grass)
    {
        GrassHit h = grass_hit(f, vis, pixel, float2(float(resolve_push.width), float(resolve_push.height)));
        row = f[0].grass_row + h.tint;
        mat = f[0].materials[row];
        if (mat.class_index != class_index)
            return mat.class_index | (wants_mirror_ray<SHADOWS>(f, mat) ? REFLECT_BIT : 0u);
        grass_surface(f, h, pixel, s, grass_scale);
        surface_relative = h.relative;
        surface_scene = h.scene;
        sun_bias = GRASS_SHADOW_BIAS;
        lamp_start = GRASS_SHADOW_BIAS;
    }
    else
    {
        uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
        uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
        uint2 record = f[0].visible[slot];
        Instance inst = f[0].instances[record.x];
        uint meshlet_index = record.y & MESHLET_INDEX_MASK;
        Meshlet m = f[0].meshlets[meshlet_index];
        // The instance names its first section's row; the others follow it (issue #41).
        row = inst.material + triangle_section(m, tri);
        mat = f[0].materials[row];
        if (mat.class_index != class_index)
            return mat.class_index | (wants_mirror_ray<SHADOWS>(f, mat) ? REFLECT_BIT : 0u);
        culled_debug = (record.y & CULLED_BIT) != 0;
        software = (record.y & SOFTWARE_BIT) != 0;
        instance_index = record.x;
        uint base = payload_base(f, m);
        PagedVertex v0 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 0));
        PagedVertex v1 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 1));
        PagedVertex v2 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 2));
        float3x3 frame = instance_frame(inst);
        float3 relative = instance_relative(f, inst);
        float4 c0 = mul(f[0].view_proj, float4(mul(frame, v0.position) + relative, 1));
        float4 c1 = mul(f[0].view_proj, float4(mul(frame, v1.position) + relative, 1));
        float4 c2 = mul(f[0].view_proj, float4(mul(frame, v2.position) + relative, 1));
        float2 size = float2(float(resolve_push.width), float(resolve_push.height));
        float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
        Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
        // The textures' mip bias (D-070): drawn smaller for an upscaler, a pixel's footprint is taken
        // as the output's, so the textures keep its detail. ×1 at the output's size: nothing changes.
        b.ddx *= f[0].texture_lod_scale;
        b.ddy *= f[0].texture_lod_scale;
        float3 o0 = decode_normal(v0.normal);
        float3 o1 = decode_normal(v1.normal);
        float3 o2 = decode_normal(v2.normal);
        float3 n0 = normalize(mul(frame, o0));
        float3 n1 = normalize(mul(frame, o1));
        float3 n2 = normalize(mul(frame, o2));
        s.n = normalize(b.lambda.x * n0 + b.lambda.y * n1 + b.lambda.z * n2);
        // How far the normal turns across the pixel (#331): the vertices' normals by the
        // barycentrics' change per pixel.
        float3 dndx = b.ddx.x * n0 + b.ddx.y * n1 + b.ddx.z * n2;
        float3 dndy = b.ddy.x * n0 + b.ddy.y * n1 + b.ddy.z * n2;
        float turn = min(2.0 * GEOMETRIC_AA_SIGMA2 * (dot(dndx, dndx) + dot(dndy, dndy)), GEOMETRIC_AA_KAPPA);
        // A flat face's normals cancel to rounding: it keeps its highlight to the bit.
        g_normal_variance = turn > 1e-6 ? turn : 0.0;
        s.object_normal = normalize(b.lambda.x * o0 + b.lambda.y * o1 + b.lambda.z * o2);
        s.object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
        s.dpdx = b.ddx.x * v0.position + b.ddx.y * v1.position + b.ddx.z * v2.position;
        s.dpdy = b.ddy.x * v0.position + b.ddy.y * v1.position + b.ddy.z * v2.position;
        // The surface relative to the camera, and in the scene frame for the rays and the probes.
        surface_relative = mul(frame, s.object_position) + relative;
        surface_scene = to_scene(f, surface_relative);
        s.frame = frame;
        s.instance_id = inst.id;
        s.instance_place = asuint(inst.local.x) * 0x8da6b343u ^ asuint(inst.local.y) * 0xd8163841u
            ^ asuint(inst.local.z) * 0xcb1ab31fu ^ asuint(inst.cell.x) * 0x9E3779B9u ^ asuint(inst.cell.z) * 0x85EBCA6Bu;
        s.meshlet_index = meshlet_index;
        s.lod_level = m.lod_level;
        s.view = normalize(-surface_relative);
        // A double-sided row seen from behind (#171): its normals turn to the viewer, and a normal
        // map's frame with them (`shade_uv_mapped` builds it about `object_normal`). Not leaf cards':
        // they bend out from the crown's middle on both sides.
        if ((mat.flags & (MATERIAL_FLAG_DOUBLE_SIDED | MATERIAL_FLAG_FOLIAGE)) == MATERIAL_FLAG_DOUBLE_SIDED
            && dot(cross(mul(frame, v1.position - v0.position), mul(frame, v2.position - v0.position)), s.view) < 0.0)
        {
            s.n = -s.n;
            s.object_normal = -s.object_normal;
        }
        if ((mat.flags & MATERIAL_FLAG_GRATING) != 0)
        {
            s.n = grating_normal(s.view);
            s.object_normal = normalize(mul(transpose(frame), s.n));
        }
        s.world = surface_scene;
        // A mesh with texture coordinates (D-047): interpolated as the position is.
        Mesh mesh = f[0].meshes[inst.mesh];
        s.has_uv = mesh.uvs != 0;
        s.coast = -1.0;
        s.uv = float2(0.0);
        s.duvdx = float2(0.0);
        s.duvdy = float2(0.0);
        if (s.has_uv)
        {
            uint stream = uv_stream(f, base, m);
            float2 t0 = paged_uv(f, stream, paged_index(f, base, m, tri, 0));
            float2 t1 = paged_uv(f, stream, paged_index(f, base, m, tri, 1));
            float2 t2 = paged_uv(f, stream, paged_index(f, base, m, tri, 2));
            s.uv = b.lambda.x * t0 + b.lambda.y * t1 + b.lambda.z * t2;
            s.duvdx = b.ddx.x * t0 + b.ddx.y * t1 + b.ddx.z * t2;
            s.duvdy = b.ddy.x * t0 + b.ddy.y * t1 + b.ddy.z * t2;
            // A planet's tile carries its normal map (#220): the next level's slopes, filtered by
            // its mips, so a tile swapped for its children changes no slope that shows and a coarse
            // cluster shows its slopes filtered rather than its vertices' samples of them.
            if ((mat.flags & MATERIAL_FLAG_PLANET) != 0u && inst.texture != 0u)
            {
                float4 texel = sample_image_grad(inst.texture - 1u, SAMPLER_LINEAR_CLAMP, s.uv, s.duvdx, s.duvdy);
                s.object_normal = normalize(texel.rgb * 2.0 - 1.0);
                s.n = normalize(mul(frame, s.object_normal));
                s.coast = texel.a;
            }
        }
        // A skinned mesh's projected textures are projected from its bind pose, or they swim over
        // the body as it bends (#166); its UV-mapped ones take their tangent frame from it. A ground
        // window's mesh is shaded where it stands in the scene, as the ground it stands in for
        // (#197): its layered rows look their layers up there (its mover moves its own frame), its
        // normal the raised surface's.
        if ((mesh.flags & MESH_SCENE_SHADED) != 0)
            s.object_position = surface_scene;
        else if (mesh.skin != 0xFFFFFFFFu)
            bind_pose_surface(f, mesh, meshlet_index, m, base, tri, b, s);
        sun_bias = shadow_bias(f, inst, m);
        lamp_start = lamp_shadow_start(f, inst, m);
        // The next meshes on the ladders (D-075), whose clusters lie outside their instance's mesh's.
        ladder = meshlet_index - mesh.meshlet_offset >= mesh.meshlet_count;
    }
    s.shadow = 1.0;
    s.sky = resolve_push.sky;
    s.ao = resolve_push.ao_image != 0xFFFFFFFFu
        ? g_sampled_images_r[NonUniformResourceIndex(resolve_push.ao_image)].Load(int3(pixel, 0))
        : -1.0;
    s.probes = resolve_push.probes; // read under a sky only
    s.shore = resolve_push.shore;
    bool mirror_ray = wants_mirror_ray<SHADOWS>(f, mat);
    s.traced = mirror_ray && resolve_push.request_image != 0xFFFFFFFFu;
    // Its shadow rays (the sun's, the lamps'): the trees' cards near, their proxies far.
    g_ray_mask = shadow_world(f, surface_relative, pixel);
    if (SHADOWS != 0 && f[0].tlas != 0 && (f[0].flags & FLAG_SHADOWS) != 0)
    {
        // A leaf card's shadow is the sun's from whichever side faces it (#346): the light
        // through the leaf needs it where the leaf turns away (`leaf_transmission`). A blade's,
        // its own normal's.
        float3 shadow_n = grass ? s.n : leaf_shadow_normal(f, mat, s.n);
        // NVIDIA's SIGMA denoised the sun's shadow before the resolve (#172), or the reference
        // traces 256 rays here; else one ray a frame and TAA's average (#54).
        if ((resolve_push.sun_shadow & 0xFFFFu) != 0xFFFFu)
            s.shadow = denoised_sun_shadow(f, shadow_n, pixel);
        else if ((f[0].flags & FLAG_SHADOW_REFERENCE) != 0)
            s.shadow = sun_shadow_reference(f, surface_scene, shadow_n, pixel, sun_bias);
        else
            s.shadow = sun_shadow(f, surface_scene, shadow_n, sun_disc_point(pixel, f[0].noise_frame), sun_bias);
        // The jelly the sunlight crossed on its way tints it (#180): the rays above see through
        // jelly, and one more ray, to a point of the disc that turns with TAA's cycle, measures
        // how much it crossed (TAA averages the points into the tint's penumbra). Not the grass's.
        if (!grass && (f[0].flags & FLAG_JELLY) != 0 && any(s.shadow > 0.0))
            s.shadow *= jelly_transmittance(f, sun_ray(f, surface_scene, shadow_n, sun_disc_point(pixel, f[0].noise_frame), sun_bias));
    }
    // Under the clouds (#145).
    if (resolve_push.cloud_shadow != 0xFFFFFFFFu)
        s.shadow *= cloud_light(surface_scene, normalize(f[0].sun_dir));
    // A standard row's inputs before its lamps (D-086): their highlights take the pixel's own
    // normal, lobe and reflectance, where they took the row's factors (on a mapped row roughness
    // 1: the Bistro's scooter matte under its lamps). The other classes, their rows' factors.
    StandardInputs si = class_index == MATERIAL_CLASS_STANDARD ? standard_inputs(f, mat, s) : row_inputs(mat, s);
    // D-088's references for the diffuse light (`r.gi.reference`), or its bounce traced before
    // the resolve (`diffuse_trace_main`, the high half of the sun shadow's word).
    uint bounce_image = resolve_push.sun_shadow >> 16;
    g_near_on = SHADOWS != 0 && f[0].gi_reference != 0u && (s.probes != nullptr || f[0].gi_reference == GI_REFERENCE_PATH) && s.sky != nullptr && f[0].tlas != 0;
    if (g_near_on)
        g_near_irradiance = near_field_reference(f, s.sky, s.probes, surface_scene, s.n, si.li.n, f[0].gi_reference);
    else if (bounce_image != 0xFFFFu && s.sky != nullptr && f[0].tlas != 0 && (mat.flags & MATERIAL_FLAG_FOLIAGE) == 0)
    {
        // In YCoCg, as REBLUR reads and writes it (`ycocg_to_linear`).
        g_near_on = true;
        g_near_contacts = true;
        g_near_irradiance = ycocg_to_linear(g_sampled_images[NonUniformResourceIndex(bounce_image)].Load(int3(pixel, 0)).rgb);
    }
    // The lamps (D-065): every one whose reach the surface is in and that lights its room, times
    // the filtered share of their light that reaches it (`lamp_filter_main`), or with a shadow ray
    // each; the classes take them where they take the sun (`sun_light`).
    lamps_at<SHADOWS>(f, s.probes, surface_scene, s.n, si.li.n, s.view, si.li.power, si.li.specular, si.li.reflectance, lamp_start, pixel, resolve_push.lamp_share);
    s.transmitted = float3(0.0);
    s.cloud = 1.0;
    if (SHADOWS != 0 && class_index == MATERIAL_CLASS_ICE && f[0].tlas != 0 && (f[0].flags & FLAG_TRANSLUCENCY) != 0)
        s.transmitted = ice_transmission(f, instance_index, surface_scene, mat.scattering);
    // The jelly's see-through (#180): under a sky, with the rays' scene.
    bool jelly_traced = SHADOWS != 0 && class_index == MATERIAL_CLASS_JELLY && (f[0].tlas != 0 || movers_tlas(f) != 0)
        && f[0].rt_scene != nullptr && resolve_push.sky != nullptr;
    if (jelly_traced)
        s.transmitted = jelly_through(f, mat, instance_index, surface_scene, s.n, s.view, s.cloud);
    // The classes work in units of a white Lambertian surface facing the sun; the frame
    // stores luminance (cd/m²) times the exposure.
    float3 color = class_index == MATERIAL_CLASS_ICE ? shade_ice(f, mat, s)
        : class_index == MATERIAL_CLASS_LAYERED ? shade_layered(f, row, mat, s)
        : class_index == MATERIAL_CLASS_JELLY ? shade_jelly(f, mat, s, instance_index, jelly_traced && (f[0].flags & FLAG_SKY_REFLECTIONS) != 0)
        : standard_light(f, mat, s, si);
    if (grass)
        color *= grass_scale;
    if (culled_debug)
        color = lerp(color, float3(1.0, 0.0, 0.0), 0.8); // a culling error (FLAG_SHOW_CULLED)
    if (software && (f[0].flags & FLAG_SHOW_RASTER) != 0)
        color = lerp(color, float3(0.1, 0.9, 0.3), 0.6); // the software rasteriser's pixels
    if ((f[0].flags & FLAG_SHOW_LADDER) != 0 && ladder)
        color = lerp(color, float3(0.15, 0.35, 1.0) * dot(color, float3(0.3, 0.6, 0.1)) * 2.0, 0.85);
    if ((f[0].flags & FLAG_SHOW_AO) != 0 && s.ao >= 0.0)
        color = float3(s.ao * 0.25); // the occlusion, as a light grey surface in the sun
    // A standard row's shading input (D-086), its value out of the exposure below.
    if (f[0].show_input != 0u && class_index == MATERIAL_CLASS_STANDARD)
    {
        color = shading_input(f[0].show_input, si.li) / max(f[0].sun_illuminance * f[0].exposure / 3.14159265, 1e-9);
        g_reflection_request = float4(0.0);
    }
    if ((f[0].flags & FLAG_SHOW_GI) != 0 && s.sky != nullptr)
    {
        // The light the sky and the city send a white surface, without the sun's own and the
        // occlusion: the probes' where they reach, the open sky's elsewhere.
        color = sh_irradiance(s.sky, s.n);
        if (s.probes != nullptr)
            color = probe_irradiance(s.probes, s.world, s.n, s.n, s.view, color);
    }
    if (!grass && (f[0].flags & FLAG_SHOW_REGIONS) != 0 && s.probes != nullptr)
        color = probe_region_tint(s.probes, s.world, s.n, s.view);
    color *= f[0].sun_illuminance * f[0].exposure / 3.14159265;
    g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)][pixel] = float4(color, 1.0);
    if (s.traced)
        g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.request_image)][pixel] = g_reflection_request;
    return class_index | (mirror_ray ? REFLECT_BIT : 0u);
}

// One thread per pixel of an 8×8 tile: empty pixels get the background (when there is one),
// standard pixels are shaded, and the tile joins the list of every other class it shows.
groupshared uint g_tile_classes;

void resolve_standard<let SHADOWS : int>(uint3 group, uint3 thread, uint index)
{
    if (index == 0)
        g_tile_classes = 0;
    GroupMemoryBarrierWithGroupSync();
    uint2 pixel = group.xy * 8 + thread.xy;
    uint others = 0;
    if (pixel.x < resolve_push.width && pixel.y < resolve_push.height)
    {
        uint vis = g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(pixel, 0));
        if (vis != VISIBILITY_EMPTY)
        {
            uint shaded = shade_pixel<SHADOWS>(MATERIAL_CLASS_STANDARD, pixel, vis);
            uint class_index = shaded & 0xFFu;
            if (class_index != MATERIAL_CLASS_STANDARD)
                others = 1u << class_index;
            if ((shaded & REFLECT_BIT) != 0)
                others |= 1u << REFLECTION_LIST;
        }
        else if (resolve_push.background.w >= 0.0)
        {
            g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)][pixel] = resolve_push.background;
        }
    }
    others = WaveActiveBitOr(others);
    if (WaveIsFirstLane() && others != 0)
        InterlockedOr(g_tile_classes, others);
    GroupMemoryBarrierWithGroupSync();
    if (index > MATERIAL_CLASS_STANDARD && index <= REFLECTION_LIST && ((g_tile_classes >> index) & 1u) != 0)
    {
        uint slot;
        InterlockedAdd(resolve_push.class_args[index * 4 + 3], 1u, slot);
        InterlockedMax(resolve_push.class_args[index * 4 + 1], slot / TILE_GROUPS_X + 1u);
        resolve_push.tiles[index * resolve_push.tile_capacity + slot] = group.x | (group.y << 16);
    }
}

// One thread per pixel of one of `class_index`'s tiles: its pixels of that class are shaded.
void resolve_tiles<let SHADOWS : int>(uint class_index, uint3 group, uint3 thread)
{
    uint tile_index = group.y * TILE_GROUPS_X + group.x;
    if (tile_index >= resolve_push.class_args[class_index * 4 + 3])
        return;
    uint tile = resolve_push.tiles[class_index * resolve_push.tile_capacity + tile_index];
    uint2 pixel = uint2(tile & 0xFFFFu, tile >> 16) * 8 + thread.xy;
    if (pixel.x >= resolve_push.width || pixel.y >= resolve_push.height)
        return;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(pixel, 0));
    if (vis != VISIBILITY_EMPTY)
        shade_pixel<SHADOWS>(class_index, pixel, vis);
}

// What a mirror ray that met nothing adds to the sky its pixel was shaded with (the table with
// the clouds over it, `sky_reflection`): the clouds as the frame shows them in its direction,
// where it shows them (the owner, 2026-10-24: the table's clouds, a degree a texel, streaked the
// still water and slid as the camera moved). The clear table under the cloud layer the compose
// lays over the sky, faded out towards the image's edges and only where the frame sees the sky
// there (what stands in front of it hides the clouds behind from the water as from the camera),
// as the water surface takes them (`water.slang`, #145). In the resolve's units; 0 without
// clouds.
float3 seen_clouds(Frame* f, float4* sky, float3 direction, float2 size)
{
    float4 seen = sky[SH_COEFFICIENTS + 2];
    if (seen.x < 0.0 || seen.y < 0.0 || seen.z <= 0.0)
        return float3(0.0);
    float4 clip = mul(f[0].view_proj, float4(direction, 0.0));
    if (clip.w <= 0.0)
        return float3(0.0);
    float2 uv = float2(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    float edge = saturate(min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)) * 10.0);
    if (edge <= 0.0)
        return float3(0.0);
    uint2 at = uint2(min(uv * size, size - 1.0));
    if (g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(at, 0)) != VISIBILITY_EMPTY)
        return float3(0.0);
    float4 up_table = sky[SH_COEFFICIENTS];
    float3 forward = sky[SH_COEFFICIENTS + 1].xyz;
    float3 clear = sky_view_luminance(uint(seen.x + 0.5), up_table.xyz, forward, direction);
    float3 clouded = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, direction);
    // The layer's light is pre-exposed by the sky's luminance scale.
    float4 c = sample_image_level(uint(seen.y + 0.5), SAMPLER_LINEAR_CLAMP, uv, 0.0);
    return edge * (clear * c.a + c.rgb / seen.z - clouded) * 3.14159265;
}

// The mirror rays of the smooth rows (issue #52), in a pass of their own over the tiles the
// standard pass listed, so the resolve does not carry the ray's code and registers. The
// resolve shaded the pixel with the sky in the mirror direction and stored that direction and
// the weight W of what a ray meets there instead. This pass traces the ray and, on a hit, adds
// W × (hit − sky): the same sum as tracing inside the resolve.
[shader("compute")]
[numthreads(8, 8, 1)]
void reflections_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    g_ray_mask = RAY_MIRROR;
    uint tile_index = group.y * TILE_GROUPS_X + group.x;
    if (tile_index >= resolve_push.class_args[REFLECTION_LIST * 4 + 3])
        return;
    uint tile = resolve_push.tiles[REFLECTION_LIST * resolve_push.tile_capacity + tile_index];
    uint2 pixel = uint2(tile & 0xFFFFu, tile >> 16) * 8 + thread.xy;
    if (pixel.x >= resolve_push.width || pixel.y >= resolve_push.height)
        return;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY || is_grass(vis))
    {
        carry_reflection(pixel);
        return;
    }
    Frame* f = resolve_push.frame;
    uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Material mat = f[0].materials[inst.material + triangle_section(m, tri)];
    if (!wants_mirror_ray<1>(f, mat))
    {
        carry_reflection(pixel);
        return;
    }
    float4 request = g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.request_image)][pixel];
    if (request.w <= 0.0)
    {
        carry_reflection(pixel);
        return;
    }
    // The surface at the pixel centre, as `shade_pixel` rebuilt it: its world position and
    // interpolated normal start the ray.
    uint base = payload_base(f, m);
    PagedVertex v0 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 0));
    PagedVertex v1 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 1));
    PagedVertex v2 = paged_vertex(f, base, m, paged_index(f, base, m, tri, 2));
    float3x3 frame = instance_frame(inst);
    float3 relative = instance_relative(f, inst);
    float4 c0 = mul(f[0].view_proj, float4(mul(frame, v0.position) + relative, 1));
    float4 c1 = mul(f[0].view_proj, float4(mul(frame, v1.position) + relative, 1));
    float4 c2 = mul(f[0].view_proj, float4(mul(frame, v2.position) + relative, 1));
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
    float3 n = normalize(b.lambda.x * normalize(mul(frame, decode_normal(v0.normal)))
        + b.lambda.y * normalize(mul(frame, decode_normal(v1.normal)))
        + b.lambda.z * normalize(mul(frame, decode_normal(v2.normal))));
    float3 object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
    // In the scene frame: the ray's.
    float3 world = to_scene(f, mul(frame, object_position) + relative);
    float4* sky = resolve_push.sky;
    float distance;
    bool mover;
    // The pixel's cone (`ladder_size`'s pixels per radian) from the camera to the surface and on,
    // and the row's lobe, its GGX width (roughness squared) each side: a glossy floor's hits read
    // as finely as a mirror's would bring back #306's hatch of streaks.
    float spread = f[0].texture_lod_scale / (f[0].p11 * 0.5 * f[0].viewport_height);
    float travelled = length(mul(frame, object_position) + relative);
    float rough = sqrt(sqrt(2.0 / (mat.specular_power + 2.0)));
    float4 hit = trace_reflection_seen(f, sky, resolve_push.probes, world + n * 0.05, request.xyz, uint(pixel.x) * 0x8DA6B343u ^ uint(pixel.y) * 0xD8163841u, mirror_start(f, inst, m), spread, travelled, 2.0 * rough * rough, distance, mover);
    ReflectionHistory* history = resolve_push.reflection_history;
    // The pixel's motion vector moved towards its reflection's virtual point (`r.reflections.motion`,
    // the motion vectors' index in the ambient occlusion's slot, which this pass never reads): on
    // the view ray as far as the surface plus the ray's length, the sky's at infinity on a miss, by
    // the reflection's weight. DLSS followed the water and dragged its reflections at the
    // surface's speed: the trees' and the clouds' reflections trailed as the camera moved sideways
    // (the owner, 2026-10-28). A mover's reflection keeps the surface's, and so does a reflection
    // in a mover (a tram's or a taxi's glass: its vector is the mover's own, which the camera's
    // motion does not explain; `steadied_reflection`'s test, the review of 2026-10-28).
    if (resolve_push.ao_image != 0xFFFFFFFFu && history != nullptr && !mover
        && (record.x & ~INSTANCE_HIDDEN_BIT) - history[0].first_mover >= history[0].mover_count)
    {
        float3 surface = mul(frame, object_position) + relative;
        float stretch = hit.w > 0.0 ? 1.0 + distance / max(length(surface), 1e-3) : 1e5;
        float4 virtual_point = mul(f[0].view_proj, float4(surface * stretch, 1.0));
        float2 uv = (float2(pixel) + 0.5 - history[0].jitter) / size;
        float w;
        float2 previous_uv = previous_uv_of(history, float3(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, virtual_point.z / virtual_point.w), w);
        if (w > 1e-9)
        {
            RWTexture2D<float2> motion = g_storage_images_rg16f[NonUniformResourceIndex(resolve_push.ao_image)];
            motion[pixel] = lerp(motion[pixel], previous_uv - uv, saturate(2.0 * request.w));
        }
    }
    // What the pixel reflects, before the exposure: what the ray met, or on a miss the sky as the
    // frame shows it there (the table and `seen_clouds`). The history settles that light, and the
    // sky the pixel was shaded with (the table in the mirror direction) is taken off after: kept
    // with it, a difference against one frame's sky came back under another's, since the sky,
    // at infinity, slides over a mirror unlike the trees it reflects. Moving sideways over the
    // village's water, the clouds showed through the trees' reflections as pale streaks, and a
    // blue band opened under the quay (the owner, 2026-10-28).
    float4 up_table = sky[SH_COEFFICIENTS];
    float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, sky[SH_COEFFICIENTS + 1].xyz, request.xyz) * 3.14159265;
    float3 reflected = hit.w > 0.0 ? hit.xyz : mirror + seen_clouds(f, sky, request.xyz, size);
    if (history != nullptr)
        reflected = steadied_reflection(history, f, pixel, size, mul(frame, object_position) + relative, record.x, reflected, hit.w > 0.0 ? distance : -1.0, mover);
    float3 added = request.w * (reflected - mirror);
    if (all(added == float3(0.0)))
        return;
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)];
    color[pixel] = float4(color[pixel].rgb + added * (f[0].sun_illuminance * f[0].exposure / 3.14159265), 1.0);
}

// A pixel of a glass tile that asks no ray this frame (a mullion, the sky beside the pane; with
// TAA's jitter, the pane's edge pixels every other frame) keeps the history it held, moved as the
// reflection moves, for the frames in which it shows glass again.
void carry_reflection(uint2 pixel)
{
    ReflectionHistory* h = resolve_push.reflection_history;
    if (h == nullptr || h[0].reset != 0)
        return;
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    Texture2D<float4> previous = g_sampled_images[NonUniformResourceIndex(h[0].previous)];
    // The depth of what the history showed here, from the same pixel: close enough to find it.
    float4 here = previous.Load(int3(pixel, 0));
    if (here.a <= 0.0)
        return;
    float2 uv = (float2(pixel) + 0.5 - h[0].jitter) / size;
    float w;
    float2 previous_uv = previous_uv_of(h, float3(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, h[0].near / here.a), w);
    int2 tap = int2(floor(float2(pixel) + 0.5 + (previous_uv - uv) * size));
    if (w <= 1e-9 || any(tap < int2(0, 0)) || any(tap >= int2(size)))
        return;
    float4 stored = previous.Load(int3(tap, 0));
    if (stored.a <= 0.0)
        return;
    g_storage_images_rgba16f[NonUniformResourceIndex(h[0].next)][pixel] = float4(stored.rgb, min(stored.a / w, 60000.0));
}

// Where the previous frame's unjittered view showed the clip-space point `at` (xy, depth).
float2 previous_uv_of(ReflectionHistory* h, float3 at, out float w)
{
    float4 previous = mul(h[0].previous_from_current, float4(at, 1.0));
    w = previous.w;
    float2 ndc = previous.xy / max(previous.w, 1e-9);
    return float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// The light `added` the pixel's mirror ray showed (what it met, or the sky) at `pixel`, blended
// into its history and stored for the next frame (#176, D-050). One exact ray a pixel lands
// somewhere else in the reflected scene with every jitter of TAA, and TAA, which follows the
// glass and not its reflection, cannot settle it. The history follows the reflection: a flat mirror shows what the ray met as a *virtual*
// point behind it, on the pixel's view ray, as far from the eye as the surface plus the ray's
// length (`distance`; negative on a miss: the sky, at infinity). That point is reprojected with
// the camera's motion, as TAA reprojects a surface. `position` is the surface's,
// camera-relative. A history texel counts while the virtual point it showed would have moved
// to within a pixel of this one's. A mover's reflection (`mover`), and a reflection in a
// mover (`instance` in the movers' trailing range of the table, #79), keep the frame's ray alone:
// the camera's motion does not explain them.
float3 steadied_reflection(ReflectionHistory* h, Frame* f, uint2 pixel, float2 size, float3 position, uint instance, float3 added, float distance, bool mover)
{
    float stretch = distance >= 0.0 ? 1.0 + distance / max(length(position), 1e-3) : 1e5;
    float4 virtual_point = mul(f[0].view_proj, float4(position * stretch, 1.0));
    float depth = virtual_point.z / virtual_point.w;
    float2 uv = (float2(pixel) + 0.5 - h[0].jitter) / size;
    float2 at = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    bool own = mover || (instance & ~INSTANCE_HIDDEN_BIT) - h[0].first_mover < h[0].mover_count;
    float3 result = added;
    float w;
    float2 previous_uv = previous_uv_of(h, float3(at, depth), w);
    if (!own && h[0].reset == 0 && w > 1e-9)
    {
        // The view depth the virtual point had in the previous frame.
        float expected = w * virtual_point.w;
        float2 texel = ((float2(pixel) + 0.5) / size + previous_uv - uv) * size - 0.5;
        int2 base = int2(floor(texel));
        float2 fraction = texel - float2(base);
        Texture2D<float4> previous = g_sampled_images[NonUniformResourceIndex(h[0].previous)];
        float3 sum = float3(0.0);
        float weight = 0.0;
        for (int k = 0; k < 4; k++)
        {
            int2 tap = base + int2(k & 1, k >> 1);
            if (any(tap < int2(0, 0)) || any(tap >= int2(size)))
                continue;
            float4 stored = previous.Load(int3(tap, 0));
            if (stored.a <= 0.0)
                continue; // no reflection there, or a mover's
            // Where this pixel's history would be had its point stood at the texel's depth.
            float unused;
            float2 other = previous_uv_of(h, float3(at, depth * expected / stored.a), unused);
            float apart = length((other - previous_uv) * size);
            float2 share = lerp(1.0 - fraction, fraction, float2(k & 1, k >> 1));
            float tap_weight = share.x * share.y * saturate(1.5 - apart);
            sum += stored.rgb * tap_weight;
            weight += tap_weight;
        }
        if (weight > 1e-3)
            result = lerp(sum / weight, added, lerp(1.0, h[0].blend, saturate(weight)));
    }
    float stored_depth = min(virtual_point.w, 60000.0);
    g_storage_images_rgba16f[NonUniformResourceIndex(h[0].next)][pixel] = float4(result, own ? -stored_depth : stored_depth);
    return result;
}

// ------------------------------------------- the surface of a pixel, before the resolve ---

// The surface the visibility buffer's (non-empty) id `vis` shows at `pixel` of a `width` ×
// `height` target, rebuilt at the pixel's centre as `shade_pixel` rebuilds it, for the passes
// that trace from it before the resolve (the sun's for SIGMA, the lamps'): its normal, turned to
// the viewer on a double-sided row seen from behind (#171, but leaf cards) and a grating's; where
// it stands relative to the camera; its instance and cluster, for where its rays start
// (`shadow_bias`).
struct PixelSurface
{
    float3 n;
    float3 g;        // its triangle's own normal, on `n`'s side (the vertices' normal bends round a box's edges)
    float3 relative;
    bool foliage;    // a foliage row's: a crown's leaf card
    Instance inst;
    Meshlet m;
    float3 shadow_n; // the normal its sun shadow takes (`leaf_shadow_normal`, #346)
};

PixelSurface pixel_surface(Frame* f, uint vis, uint2 pixel, uint width, uint height)
{
    if (is_grass(vis))
    {
        // A grass blade's (#300), as the resolve rebuilds it.
        GrassHit h = grass_hit(f, vis, pixel, float2(float(width), float(height)));
        PixelSurface g = {};
        g.n = h.n;
        g.g = h.n;
        g.relative = h.relative;
        g.foliage = false;
        g.shadow_n = h.n;
        return g;
    }
    uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    uint2 record = f[0].visible[slot];
    PixelSurface s;
    s.inst = f[0].instances[record.x];
    s.m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Material mat = f[0].materials[s.inst.material + triangle_section(s.m, tri)];
    uint base = payload_base(f, s.m);
    PagedVertex v0 = paged_vertex(f, base, s.m, paged_index(f, base, s.m, tri, 0));
    PagedVertex v1 = paged_vertex(f, base, s.m, paged_index(f, base, s.m, tri, 1));
    PagedVertex v2 = paged_vertex(f, base, s.m, paged_index(f, base, s.m, tri, 2));
    float3x3 frame = instance_frame(s.inst);
    float3 relative = instance_relative(f, s.inst);
    float4 c0 = mul(f[0].view_proj, float4(mul(frame, v0.position) + relative, 1));
    float4 c1 = mul(f[0].view_proj, float4(mul(frame, v1.position) + relative, 1));
    float4 c2 = mul(f[0].view_proj, float4(mul(frame, v2.position) + relative, 1));
    float2 size = float2(float(width), float(height));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
    s.n = normalize(b.lambda.x * normalize(mul(frame, decode_normal(v0.normal)))
        + b.lambda.y * normalize(mul(frame, decode_normal(v1.normal)))
        + b.lambda.z * normalize(mul(frame, decode_normal(v2.normal))));
    float3 object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
    s.relative = mul(frame, object_position) + relative;
    if ((mat.flags & (MATERIAL_FLAG_DOUBLE_SIDED | MATERIAL_FLAG_FOLIAGE)) == MATERIAL_FLAG_DOUBLE_SIDED
        && dot(cross(mul(frame, v1.position - v0.position), mul(frame, v2.position - v0.position)), -s.relative) < 0.0)
        s.n = -s.n;
    if ((mat.flags & MATERIAL_FLAG_GRATING) != 0)
        s.n = grating_normal(normalize(-s.relative));
    s.foliage = (mat.flags & MATERIAL_FLAG_FOLIAGE) != 0;
    float3 face = cross(mul(frame, v1.position - v0.position), mul(frame, v2.position - v0.position));
    s.g = dot(face, face) > 1e-20 ? normalize(face) : s.n;
    if (dot(s.g, s.n) < 0.0)
        s.g = -s.g;
    s.shadow_n = leaf_shadow_normal(f, mat, s.n);
    return s;
}

// ------------------------------------------------------- the sun's shadow for SIGMA ---

// The sun's shadow ray for NVIDIA's SIGMA denoiser (#172, D-049), before the resolve: per pixel,
// the surface the visibility buffer holds, rebuilt as `shade_pixel` does, and one ray to a point
// of the sun's disc that finds the *closest* occluder (SIGMA sizes the penumbra from its
// distance; the first hit could be any). It writes SIGMA's inputs as NRD's interface describes
// them (`forge_gpu::nrd`): the penumbra's radius, the normal and the view depth. The disc's
// points come from the Hilbert curve's R2 (`spatio_temporal_noise`): distinct over every pixel
// block and the frames, and repeating with TAA's jitter.
struct SunShadowPush
{
    float4 view_z;        // the view matrix's third row NRD is given: camera-relative metres → view z
    Frame* frame;
    uint vis_image;       // sampled (u32): the visibility buffer
    uint penumbra_image;  // storage (r32f): the penumbra's radius in metres, NRD_MISS lit, 0 facing away
    uint normal_image;    // storage (rgba16f): the normal (xyz) and the linear roughness (w)
    uint view_z_image;    // storage (r32f): the view depth, NOTHING_DRAWN_Z where nothing was drawn
    uint width;
    uint height;
    float tan_radius;     // tan of the sun's angular radius for the shadows
    uint closest;         // 1: find the closest occluder (SIGMA); 0: any (FFX, #173)
    uint pad1;
    uint pad2;
};

[[vk::push_constant]]
SunShadowPush sun_shadow_push;

// The penumbra of a ray that reached the sun: half precision's largest value, NRD's mark.
static const float NRD_MISS = 65504.0;
// The view depth of a pixel without a surface: past any denoising range.
static const float NOTHING_DRAWN_Z = 1.0e7;

[shader("compute")]
[numthreads(8, 8, 1)]
void sun_shadow_trace_main(uint3 id : SV_DispatchThreadID)
{
    SunShadowPush p = sun_shadow_push;
    if (id.x >= p.width || id.y >= p.height)
        return;
    uint2 pixel = id.xy;
    RWTexture2D<float> penumbra = g_storage_images_r32f[NonUniformResourceIndex(p.penumbra_image)];
    RWTexture2D<float4> normal = g_storage_images_rgba16f[NonUniformResourceIndex(p.normal_image)];
    RWTexture2D<float> view_z = g_storage_images_r32f[NonUniformResourceIndex(p.view_z_image)];
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY)
    {
        penumbra[pixel] = NRD_MISS;
        normal[pixel] = float4(0.0, 1.0, 0.0, 1.0);
        view_z[pixel] = NOTHING_DRAWN_Z;
        return;
    }
    Frame* f = p.frame;
    PixelSurface s = pixel_surface(f, vis, pixel, p.width, p.height);
    float3 n = s.n;
    float3 surface_relative = s.relative;
    normal[pixel] = float4(n, 1.0);
    view_z[pixel] = dot(p.view_z.xyz, surface_relative) + p.view_z.w;
    float3 sun = normalize(f[0].sun_dir);
    // A leaf card traces from whichever side faces the sun (#346).
    n = s.shadow_n;
    if (dot(n, sun) <= 0.0 || f[0].tlas == 0)
    {
        // Facing away: in its own shade, a penumbra of 0 (a hard shadow). Without a structure
        // to trace, lit.
        penumbra[pixel] = dot(n, sun) <= 0.0 ? 0.0 : NRD_MISS;
        return;
    }
    // The trees' cards near, their proxies far (D-075 step 3).
    g_ray_mask = shadow_world(f, surface_relative, pixel);
    // A point of the disc, uniform in area, as `sun_shadow` aims.
    float2 u = spatio_temporal_noise(pixel, f[0].noise_frame);
    float3 t = normalize(cross(sun, abs(sun.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0)));
    float3 bt = cross(sun, t);
    float r = p.tan_radius * sqrt(u.x);
    float phi = 6.2831853 * u.y;
    RayDesc ray;
    ray.Origin = to_scene(f, surface_relative) + (n + sun) * (is_grass(vis) ? GRASS_SHADOW_BIAS : shadow_bias(f, s.inst, s.m));
    ray.Direction = normalize(sun + (t * cos(phi) + bt * sin(phi)) * r);
    ray.TMin = 0.0;
    ray.TMax = 50000.0;
    // AMD's FidelityFX denoiser needs no distance (#173): the first hit ends the ray, as
    // `sun_shadow`'s, and a blocked ray is a penumbra of 0.
    if (p.closest == 0)
    {
        penumbra[pixel] = ray_blocked(f, ray) ? 0.0 : NRD_MISS;
        return;
    }
    ClosestHit hit = trace_closest(f, ray, true);
    penumbra[pixel] = hit.hit ? min(0.5 * hit.t * p.tan_radius, 32768.0) : NRD_MISS;
}

// ------------------------------------------------------ the diffuse bounce (D-088) ---

// D-088's diffuse bounce, before the resolve: per pixel of the visibility buffer, the surface
// rebuilt as `shade_pixel` does and one cosine-weighted ray from it to the scene's end, past a
// house's clear glass (`trace_past_glass`). Its hit sends back the sun through a shadow ray, the
// lamps, its glow and its albedo times the probes' light for the bounces after (`hit_radiance`,
// `hit_lamps`); a ray that meets nothing, the sky's light along it. The resolve takes it in place
// of the probes' light, under GTAO's contacts (`g_near_irradiance`): `r.gi.reference=2` with one
// ray. It writes what NVIDIA's REBLUR reads (NRD's `REBLUR_FrontEnd_PackRadianceAndNormHitDist`):
// the light in YCoCg, and the ray's length as a share of REBLUR's normalisation (`hit_distance`).
// Every pixel traces, as without REBLUR. Half of them a frame in a checkerboard (NRD's
// `CheckerboardMode::WHITE`) cost half as much, but left REBLUR's grain on a stairwell's walls
// lit through one opening (the village's landing, the owner, 2026-10-30).
struct DiffuseTracePush
{
    float4 view_z;        // the view matrix's third row: camera-relative metres → view z
    float4 hit_distance;  // REBLUR's normalisation of the rays' lengths (A, B, C); w unused
    Frame* frame;
    float4* sky;          // the sky's irradiance coefficients, as the resolve has them
    ProbeField* probes;   // the probes, or null: the sky's irradiance at the hits
    uint vis_image;       // sampled (u32): the visibility buffer
    uint bounce_image;    // storage (rgba16f): the bounce's light in YCoCg (rgb), its normalised ray length (a)
    uint width;           // the frame's
    uint height;
};

// NRD's colour space for REBLUR (`_NRD_LinearToYCoCg`, `_NRD_YCoCgToLinear`).
float3 linear_to_ycocg(float3 c)
{
    return float3(dot(c, float3(0.25, 0.5, 0.25)), dot(c, float3(0.5, 0.0, -0.5)), dot(c, float3(-0.25, 0.5, -0.25)));
}

float3 ycocg_to_linear(float3 c)
{
    float t = c.x - c.z;
    return max(float3(t + c.y, c.x + c.z, t - c.y), 0.0);
}

// A ray's length as REBLUR reads it (NRD's `REBLUR_FrontEnd_GetNormHitDist` for a roughness of 1).
float reblur_hit_distance(float length, float view_z, float3 params)
{
    return saturate(length / (params.x + abs(view_z) * params.y));
}

[[vk::push_constant]]
DiffuseTracePush diffuse_trace_push;

// How far off its surface a diffuse ray starts, metres, along the vertices' normal, as the
// reference's (`near_field_reference`). From the sun's rays' SHADOW_BIAS, 15 cm, a wall's rays
// behind a table started inside its top and legs; from the lamps' (`lamp_shadow_start`, a few
// millimetres) the points of faces seen edge on, where they meet the wall, still did: black
// seams under REBLUR both ways (the owner, 2026-10-30).
static const float DIFFUSE_START = 0.02;

[shader("compute")]
[numthreads(8, 8, 1)]
void diffuse_trace_main(uint3 id : SV_DispatchThreadID)
{
    DiffuseTracePush p = diffuse_trace_push;
    Frame* f = p.frame;
    uint2 pixel = id.xy;
    if (pixel.x >= p.width || pixel.y >= p.height)
        return;
    RWTexture2D<float4> bounce = g_storage_images_rgba16f[NonUniformResourceIndex(p.bounce_image)];
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY || f[0].tlas == 0)
    {
        bounce[pixel] = float4(0.0);
        return;
    }
    PixelSurface s = pixel_surface(f, vis, pixel, p.width, p.height);
    // A crown's leaf keeps the probes' light (the resolve takes no bounce there): far from the camera
    // its ray started inside its crown's opaque proxy and met it black, and the leaves' light is
    // the impostors' (D-075). Written here too, so that REBLUR does not blur black round a tree.
    if (s.foliage)
    {
        float3 open_sky = sh_irradiance(p.sky, s.n);
        float3 leaf = p.probes != nullptr ? probe_irradiance(p.probes, to_scene(f, s.relative), s.n, s.n, normalize(-s.relative), open_sky) : open_sky;
        bounce[pixel] = float4(linear_to_ycocg(clamp(leaf, 0.0, 65504.0)), 1.0);
        return;
    }
    // The trees' cards near, their proxies far, as the resolve's own rays.
    g_ray_mask = shadow_world(f, s.relative, pixel);
    // The noise's point moved by R2's first step, apart from the sun's disc points of
    // `sun_shadow_trace_main`. It repeats with TAA's cycle: a pixel traces the cycle's directions
    // (8 under DLAA) over and over, and REBLUR settles on them. Turned each round, they doubled a
    // still camera's changing pixels (5,054 to 10,507 at Exterior_1).
    float2 u = frac(spatio_temporal_noise(pixel, f[0].noise_frame) + float2(0.7548777, 0.5698403));
    RayDesc ray;
    // Round the vertices' normal, but never under the triangle's own face, where it bends round
    // a box's edges: a ray under the face is mirrored over it.
    float3 d = cosine_direction(s.n, u.x, u.y);
    if (dot(d, s.g) < 0.0)
        d -= 2.0 * dot(d, s.g) * s.g;
    ray.Origin = to_scene(f, s.relative) + s.n * DIFFUSE_START;
    ray.Direction = d;
    ray.TMin = 0.0;
    ray.TMax = PATH_RANGE;
    // The crowns as the probes see them, a quarter of their leaves, by a number fixed per pixel:
    // every leaf card kept, a tree's outer leaves saw the crown round them as a wall and the near
    // trees went black beside their impostors (the village from the air, 2026-10-30).
    ClosestHit c = trace_past_glass(f, ray, true, (pixel.x * 0x8DA6B343u) ^ (pixel.y * 0xD8163841u));
    float3 light;
    bool back = false;
    if (!c.hit)
        light = sky_along(p.sky, ray.Direction);
    else
    {
        RayHit hit = ray_hit(f, c.instance, c.primitive, c.barycentrics, ray.Origin, ray.Direction, c.t);
        // The pixel's own number: `hit_lamps` turns it with the frame.
        uint seed = (pixel.x * 0x8DA6B343u) ^ (pixel.y * 0xD8163841u);
        light = hit.back ? float3(0.0) : hit_radiance(f, p.sky, p.probes, hit, ray.Direction) + hit_lamps(f, hit, REGION_NONE, seed, false);
        back = hit.back;
    }
    // NRD's front end sanitises: no NaN, nothing under 0 or past half precision; a ray that met a
    // surface's back started inside it, a length of 0 as NRD asks.
    light = any(isnan(light)) || any(isinf(light)) ? float3(0.0) : clamp(light, 0.0, 65504.0);
    float view_z = dot(p.view_z.xyz, s.relative) + p.view_z.w;
    bounce[pixel] = float4(linear_to_ycocg(light), reblur_hit_distance(back ? 0.0 : c.t, view_z, p.hit_distance.xyz));
}

// --------------------------------------------------------- the lamps' penumbrae ---

// The lamps' shadow rays before the resolve and the filter of what they found (D-065,
// research/lamp-penumbra-denoising.md §7): the interleaved ratio filter.
// - `lamp_rays_main` traces, per pixel, the rays `lamps_at` traced (`lamp_picks`: the three
//   strongest lamps, the strongest with two rays, and one picked by its light), each to a point of
//   a table of 128 stratified disc points spread over 4 × 4 pixels and the 8 frames of TAA's cycle
//   (`lamp_disc_point`; Keller and Heidrich's interleaved sampling, 2001). It writes the share of
//   their light that reaches the pixel, Ŝ / Û per channel, the same rays' shadowed over unshadowed
//   irradiance (Heitz, Hill and McGuire's ratio estimator, 2018), with luminance(Û).
// - `lamp_filter_main` averages that share over a fixed 7 × 7 tent of the pixel's neighbours on
//   its surface (a 4 × 4 box twice over), each weighted by its luminance(Û): every point of a
//   frame's run weighs the same in it.
// - The resolve multiplies the exact, unshadowed light of every lamp of the pixel's place by it
//   (`lamps_at`), and TAA averages the 8 runs: all 128 points.
// No width from the occluders' distance (NVIDIA's US 10,740,954, claims 10 and 15), no variance,
// no history and no wave intrinsics (the research note's §3 and §6).
struct LampShadowPush
{
    Frame* frame;
    ProbeField* probes;  // the probes' regions (D-068), as the resolve has them, or null: no places
    uint vis_image;      // sampled (u32): the visibility buffer
    uint ratio_image;    // storage for the rays, sampled for the filter (rgba16f): Ŝ / Û per channel, luminance(Û) in lux (a); (1, 1, 1, 0) without lamp light
    uint surface_image;  // likewise (rgba16f): the surface relative to the camera (rgb, metres), its region's tag (a: 0 none, else 1 + region mod 2047, exact in half precision)
    uint normal_image;   // likewise (rg16f): its normal, octahedral (`oct_encode`)
    uint share_image;    // storage (rgba16f): the filtered share (rgb), the resolve's `lamp_share`
    uint width;
    uint height;
    uint skip_outside;   // 1: ReSTIR DI lights the street lamps (`lamp_outside`), so the rays leave them out
};

[[vk::push_constant]]
LampShadowPush lamp_shadow_push;

// A 4 × 4 Bayer matrix, row by row: the order a 4 × 4 block of pixels takes a run's 16 points in.
static const uint LAMP_BAYER[16] = { 0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u };

// Point `index` (< 128) of the lamps' disc table, in the unit disc: the first 128 points of the
// two-dimensional Sobol sequence (Sobol 1967: the radical inverse in base 2 and the second
// dimension's Pascal matrix), a (0,2)-sequence whose aligned runs of 16 are each stratified, and
// all 128 too; at their cells' middles, through Shirley and Chiu's concentric map (1997), which
// keeps the strata's areas.
float2 lamp_disc_table(uint index)
{
    uint y = 0u;
    uint v = 0x80000000u;
    for (uint k = 0u; k < 7u; k++)
    {
        if (((index >> k) & 1u) != 0u)
            y ^= v;
        v ^= v >> 1;
    }
    float2 o = (float2(float(reversebits(index) >> 25), float(y >> 25)) + 0.5) / 64.0 - 1.0;
    // The cells' middles are never on the square's axes, so neither division is by 0.
    float r;
    float phi;
    if (abs(o.x) > abs(o.y))
    {
        r = o.x;
        phi = 0.785398163 * (o.y / o.x);
    }
    else
    {
        r = o.y;
        phi = 1.570796327 - 0.785398163 * (o.x / o.y);
    }
    return r * float2(cos(phi), sin(phi));
}

// Where on its disc lamp `i` (the frame's) is traced from `pixel` in frame `frame` of TAA's
// cycle: point 16 (frame mod 8) + B[x mod 4, y mod 4] of the table (B the Bayer matrix), so every
// 4 × 4 window of pixels holds a whole stratified run each frame and the cycle's 8 frames hold
// all 128; turned by the lamp's golden angle, so two lamps' points do not move together.
float2 lamp_disc_point(uint2 pixel, uint frame, uint i)
{
    float2 p = lamp_disc_table(16u * (frame % 8u) + LAMP_BAYER[(pixel.y & 3u) * 4u + (pixel.x & 3u)]);
    float a = 6.28318530718 * frac(0.618034 * float(i));
    float c = cos(a);
    float s = sin(a);
    return float2(p.x * c - p.y * s, p.x * s + p.y * c);
}

// Whether lamp `lamp`'s light reaches a surface at `world` facing `n` from the point `disc` of
// its sphere's disc (the unit disc, `lamp_disc_point`), its ray starting `bias` off the surface.
bool lamp_seen(Frame* f, Lamp lamp, float3 world, float3 n, float bias, float2 disc)
{
    float3 d = lamp.position_radius.xyz - world;
    float3 t;
    float3 b;
    lamp_disc_axes(d / max(length(d), 1e-6), t, b);
    return !ray_blocked(f, lamp_ray(lamp, world, n, bias, (t * disc.x + b * disc.y) * lamp.position_radius.w));
}

// A lamp's unshadowed illuminance on a surface at `world` facing `n`, lux per channel, let go at
// its reach as `lamp_add` lets it go.
float3 lamp_lux(Lamp lamp, float3 world, float3 n)
{
    float3 d = lamp.position_radius.xyz - world;
    float dist2 = dot(d, d);
    float dist = sqrt(dist2);
    float ndl = dot(n, d / max(dist, 1e-6));
    float radius = lamp.position_radius.w;
    float window = saturate(1.0 - pow(dist / lamp.colour_reach.w, 4.0));
    return lamp.colour_reach.rgb / max(dist2, radius * radius) * (max(ndl, 0.0) * window * window) * lamp_beam(lamp, d / max(dist, 1e-6));
}

[shader("compute")]
[numthreads(8, 8, 1)]
void lamp_rays_main(uint3 id : SV_DispatchThreadID)
{
    LampShadowPush p = lamp_shadow_push;
    if (id.x >= p.width || id.y >= p.height)
        return;
    uint2 pixel = id.xy;
    RWTexture2D<float4> ratio = g_storage_images_rgba16f[NonUniformResourceIndex(p.ratio_image)];
    RWTexture2D<float4> surface = g_storage_images_rgba16f[NonUniformResourceIndex(p.surface_image)];
    RWTexture2D<float2> normal = g_storage_images_rg16f[NonUniformResourceIndex(p.normal_image)];
    Frame* f = p.frame;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY)
    {
        ratio[pixel] = float4(1.0, 1.0, 1.0, 0.0);
        surface[pixel] = float4(0.0);
        normal[pixel] = float2(0.0);
        return;
    }
    PixelSurface s = pixel_surface(f, vis, pixel, p.width, p.height);
    float3 world = to_scene(f, s.relative);
    // The trees' cards near, their proxies far (D-075 step 3).
    g_ray_mask = shadow_world(f, s.relative, pixel);
    LampPlace place = lamp_place(f, p.probes, world, s.n, normalize(-s.relative));
    surface[pixel] = float4(s.relative, place.region == REGION_NONE ? 0.0 : float(1u + place.region % 2047u));
    normal[pixel] = oct_encode(s.n);
    LampPicks picks = lamp_picks(f, place, world, s.n, pixel, p.skip_outside != 0u);
    uint count = f[0].lamp_count;
    float bias = is_grass(vis) ? GRASS_SHADOW_BIAS : lamp_shadow_start(f, s.inst, s.m);
    float3 unshadowed = float3(0.0);
    float3 shadowed = float3(0.0);
    for (uint j = 0u; j < 4u; j++)
    {
        uint i = j < 3u ? picks.top[j] : picks.chosen;
        if (i >= count)
            continue;
        Lamp lamp = f[0].lamps[i];
        float3 e = lamp_lux(lamp, world, s.n) * (j < 3u ? 1.0 : picks.scale);
        float seen = 1.0;
        if (f[0].tlas != 0)
        {
            // The strongest takes two rays, to the frame's run and its pair's (runs 2m and 2m + 1):
            // its 32 points a frame are an aligned run of the sequence, stratified as a whole.
            // Two runs half the cycle apart are not, and their estimate changed more from frame to
            // frame: a still camera's frames 10 apart changed on 6.1 % of the smoke yard's pixels
            // under TAA, 3.9 % with the pair (the hall 0.31 and 0.16 %).
            seen = lamp_seen(f, lamp, world, s.n, bias, lamp_disc_point(pixel, f[0].noise_frame, i)) ? 1.0 : 0.0;
            if (j == 0u)
                seen = 0.5 * seen + (lamp_seen(f, lamp, world, s.n, bias, lamp_disc_point(pixel, f[0].noise_frame ^ 1u, i)) ? 0.5 : 0.0);
        }
        unshadowed += e;
        shadowed += e * seen;
    }
    float y = dot(unshadowed, float3(0.2126, 0.7152, 0.0722));
    float3 share = float3(unshadowed.x > 0.0 ? shadowed.x / unshadowed.x : 1.0,
                          unshadowed.y > 0.0 ? shadowed.y / unshadowed.y : 1.0,
                          unshadowed.z > 0.0 ? shadowed.z / unshadowed.z : 1.0);
    // Half precision's largest value bounds the weight: a pixel by a bulb is lit by thousands of
    // lux, so it outweighs its neighbours all the same.
    ratio[pixel] = y > 0.0 ? float4(share, min(y, 65000.0)) : float4(1.0, 1.0, 1.0, 0.0);
}

// The filter's tile: its 8 × 8 pixels and the 3 more each way that their windows reach. Per
// pixel, its luminance-weighted share (rgb: luminance × share, a: luminance), its normal and its
// region's tag (`LampShadowPush::surface_image`), and where it stands.
static const uint LAMP_TILE = 14u;
groupshared float4 g_lamp_weighted[LAMP_TILE * LAMP_TILE];
groupshared float4 g_lamp_facing[LAMP_TILE * LAMP_TILE];
groupshared float3 g_lamp_position[LAMP_TILE * LAMP_TILE];
// The window's weights along each axis: the 4-pixel box twice over, a tent 7 pixels wide. Each
// of the four columns (and rows) of a 4 × 4 block takes a quarter of them, so every point of the
// frame's run weighs the same. One box left each point's shadow edge on its own 4-pixel grid,
// and the penumbrae's edges came out in 4-pixel steps that TAA's clamp kept.
static const float LAMP_TENT[7] = { 1.0, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0 };

// The share of the lamps' light that reaches each pixel, from `lamp_rays_main`'s: the shares of
// the 7 × 7 pixels around it, weighted by the tent (`LAMP_TENT`) and by their unshadowed light's
// luminance (the ratio of the sums, Heitz et al.), and kept only on the pixel's surface: in its
// region (a room holds its own lamps), facing within 25° of its normal (n·n′ > 0.9), and within
// 0.5 % of its distance (and 1 cm) of its plane, falling to 0 there. Summed in a fixed order.
[shader("compute")]
[numthreads(8, 8, 1)]
void lamp_filter_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    LampShadowPush p = lamp_shadow_push;
    int2 corner = int2(group.xy * 8u) - 3;
    for (uint k = thread.y * 8u + thread.x; k < LAMP_TILE * LAMP_TILE; k += 64u)
    {
        int2 q = corner + int2(int(k % LAMP_TILE), int(k / LAMP_TILE));
        float4 ratio = float4(1.0, 1.0, 1.0, 0.0);
        float4 surface = float4(0.0);
        float3 normal = float3(0.0);
        if (q.x >= 0 && q.y >= 0 && q.x < int(p.width) && q.y < int(p.height))
        {
            ratio = g_sampled_images[NonUniformResourceIndex(p.ratio_image)].Load(int3(q, 0));
            surface = g_sampled_images[NonUniformResourceIndex(p.surface_image)].Load(int3(q, 0));
            normal = ratio.a > 0.0 ? oct_decode(g_sampled_images[NonUniformResourceIndex(p.normal_image)].Load(int3(q, 0)).xy) : float3(0.0);
        }
        g_lamp_weighted[k] = float4(ratio.rgb * ratio.a, ratio.a);
        g_lamp_facing[k] = float4(normal, surface.w);
        g_lamp_position[k] = surface.xyz;
    }
    GroupMemoryBarrierWithGroupSync();
    uint2 pixel = group.xy * 8u + thread.xy;
    if (pixel.x >= p.width || pixel.y >= p.height)
        return;
    uint centre = (thread.y + 3u) * LAMP_TILE + thread.x + 3u;
    float3 share = float3(1.0);
    if (g_lamp_weighted[centre].a > 0.0)
    {
        float4 facing = g_lamp_facing[centre];
        float3 x = g_lamp_position[centre];
        // The plane test as 1 − |n·x′ − n·x| / tolerance.
        float per_metre = 1.0 / (0.005 * length(x) + 0.01);
        float offset = dot(facing.xyz, x) * per_metre;
        float4 sum = float4(0.0);
        for (uint dy = 0u; dy < 7u; dy++)
        {
            for (uint dx = 0u; dx < 7u; dx++)
            {
                uint k = (thread.y + dy) * LAMP_TILE + thread.x + dx;
                float4 other = g_lamp_facing[k];
                float kept = other.w == facing.w && dot(facing.xyz, other.xyz) > 0.9 ? LAMP_TENT[dx] * LAMP_TENT[dy] : 0.0;
                sum += g_lamp_weighted[k] * (kept * saturate(1.0 - abs(dot(facing.xyz, g_lamp_position[k]) * per_metre - offset)));
            }
        }
        // The pixel itself weighs its own luminance, so `sum.a` is not 0.
        share = sum.rgb / sum.a;
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(p.share_image)][pixel] = float4(share, 1.0);
}

// The rays a pass drawn after the resolve asks for (the sea's surface, issue #105): per pixel,
// in two images that hold zero where it asks none. The depth it wrote places their start.
// - A mirror ray: the pass shaded the pixel with the sky in the mirror direction and stored
//   that direction and the weight W of what a ray meets there instead. As in
//   `reflections_main`, a hit adds W × (hit − sky). With the reflection flags (F, Y).
// - A shadow ray: the pass stored the share of its colour the sun lights. A ray towards a
//   point of the sun's disc that meets the scene takes that share away; the points turn with
//   TAA's cycle (`sun_disc_point`), which averages them into a penumbra. With the shadows'
//   flag (J).
struct RequestedRaysPush
{
    float4x4 inv_view_proj; // clip → camera-relative metres (the drawing camera, jitter included)
    Frame* frame;
    float4* sky;            // the sky's irradiance and its table (sky.slang)
    ProbeField* probes;     // or null: the sky's irradiance
    uint mirror_image;      // sampled (rgba16f): direction, weight
    uint sun_image;         // sampled (rgba16f): the sun's share (rgb), 1 (w)
    uint depth_image;       // sampled: the depth the pass wrote
    uint color_image;       // storage (rgba16f): read and written
    uint width;
    uint height;
};

[[vk::push_constant]]
RequestedRaysPush requested_push;

// How far the water's shadow ray starts off it (along the surface's normal and the sun, as
// `sun_shadow` offsets): the terrain's own start (`TERRAIN_SHADOW_START` in raytrace.rs). The
// sea floor's traced cut may stand a metre above the drawn floor, so above the water in the
// shallows, where a nearer start could meet it under a clear sky.
static const float WATER_SHADOW_START = 2.0;

[shader("compute")]
[numthreads(8, 8, 1)]
void requested_rays_main(uint3 id : SV_DispatchThreadID)
{
    RequestedRaysPush p = requested_push;
    Frame* f = p.frame;
    if (id.x >= p.width || id.y >= p.height || f[0].tlas == 0 || f[0].rt_scene == nullptr
        || (f[0].flags & FLAG_SHOW_GI) != 0)
        return;
    const uint reflection_flags = FLAG_SKY_REFLECTIONS | FLAG_RAY_REFLECTIONS;
    int3 at = int3(id.xy, 0);
    float4 request = (f[0].flags & reflection_flags) == reflection_flags
        ? g_sampled_images[NonUniformResourceIndex(p.mirror_image)].Load(at)
        : float4(0.0);
    float4 sun = (f[0].flags & FLAG_SHADOWS) != 0
        ? g_sampled_images[NonUniformResourceIndex(p.sun_image)].Load(at)
        : float4(0.0);
    if (request.w <= 0.0 && sun.w <= 0.0)
        return;
    float depth = g_sampled_images_r[NonUniformResourceIndex(p.depth_image)].Load(at);
    float2 ndc = float2((float(id.x) + 0.5) / float(p.width) * 2.0 - 1.0, 1.0 - (float(id.y) + 0.5) / float(p.height) * 2.0);
    float4 point = mul(p.inv_view_proj, float4(ndc, depth, 1.0));
    float3 world = to_scene(f, point.xyz / point.w);
    float3 added = 0.0;
    if (sun.w > 0.0)
    {
        float lit = sun_shadow(f, world, float3(0.0, 1.0, 0.0), sun_disc_point(id.xy, f[0].noise_frame), WATER_SHADOW_START);
        added -= (1.0 - lit) * sun.rgb;
    }
    if (request.w > 0.0)
    {
        float3 direction = normalize(request.xyz);
        g_ray_mask = RAY_MIRROR;
        float4 hit = trace_reflection(f, p.sky, p.probes, world, direction, uint(id.x) * 0x8DA6B343u ^ uint(id.y) * 0xD8163841u);
        if (hit.w > 0.0)
        {
            float4 up_table = p.sky[SH_COEFFICIENTS];
            float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, p.sky[SH_COEFFICIENTS + 1].xyz, direction) * 3.14159265;
            added += request.w * (hit.xyz - mirror) * (f[0].sun_illuminance * f[0].exposure / 3.14159265);
        }
    }
    if (all(added == 0.0))
        return;
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(p.color_image)];
    color[id.xy] = float4(max(color[id.xy].rgb + added, 0.0), 1.0);
}

// ------------------------------------------------------------- probe rays ---

// The probes' rays (issue #53, probes.slang): a thread per ray, 64 rays a workgroup, from the
// probe's position (its cell's centre and its offset). A miss takes the sky-view table's light
// in its direction; a front face is shaded like a mirror ray's hit, lit by the probes as the
// frame before left them (so light bounces once more every frame); a back face keeps only its
// distance, negated. The fixed rays keep only their distances, up to PROBE_FIXED_REACH cells
// (relocation and classification look no farther, and need no light), and an inactive probe
// traces those alone.
static const uint PROBE_TRACE_GROUP = 64;
static const float PROBE_FIXED_REACH = 3.5;

struct ProbeTracePush
{
    Frame* frame;
    ProbeField* field;
    float4* sky;      // the sky's irradiance and its table (sky.slang)
    uint probe_count; // probes of every cascade
    uint first_update; // 1 on the first update after a reset (`Probes::update`)
    uint movers;       // 1: the rays meet the movers (`ProbeParams::movers`, #299)
    uint pad;
};

[[vk::push_constant]]
ProbeTracePush probe_push;

[shader("compute")]
[numthreads(PROBE_TRACE_GROUP, 1, 1)]
void probe_trace_main(uint3 group : SV_GroupID, uint thread : SV_GroupThreadID)
{
    g_trace_movers = probe_push.movers != 0u;
    g_ray_mask = RAY_PROBE;
    ProbeField* field = probe_push.field;
    uint probe = group.z * 65535u + group.y;
    uint ray = group.x * PROBE_TRACE_GROUP + thread;
    if (probe >= probe_push.probe_count || ray >= field[0].rays)
        return;
    ProbeSlot slot = probe_slot(field, probe);
    if (slot.unused)
        return;
    // A room grid's probe traces its own count of rays (D-078, `RoomGridParams::rays`).
    uint rays = probe_rays(field, slot);
    if (ray >= rays)
        return;
    float4 data = slot.fresh ? float4(0.0, 0.0, 0.0, PROBE_ACTIVE) : field[0].probe_data[probe];
    // A settled probe (probes.slang) needs no fixed rays; an inactive or retired one (D-078) no
    // others, and a settled one traces the others on its turn only (issue #103).
    bool fixed = ray < PROBE_FIXED_RAYS;
    bool settled = !slot.fresh && probe_age(data.w) >= PROBE_SETTLE_UPDATES;
    if (fixed ? settled : (!probe_lights(data.w) || !probe_due(field, probe, slot, probe_age(data.w), false)))
        return;
    Frame* f = probe_push.frame;
    float4* sky = probe_push.sky;
    float3 origin = slot.position + data.xyz;
    float3 direction = probe_ray_direction(field, ray, rays);
    RayDesc desc;
    desc.Origin = origin;
    desc.Direction = direction;
    desc.TMin = 0.0;
    desc.TMax = fixed ? PROBE_FIXED_REACH * slot.spacing : PROBE_MISS_DISTANCE;
    ClosestHit c = trace_closest(f, desc, false, true, probe * 0x27D4EB2Fu ^ ray);
    // The region the probe stands in (D-068), once it has one.
    uint region = REGION_NONE;
    if (!slot.fresh && field[0].region_count > 0u)
        region = field[0].probe_region[probe];
    // A probe sees through a window's glass. It is drawn from outside only, as the view from the
    // room passes it, and met from behind it ended the ray as a back face: a room took its light
    // from its doors alone, and with regions an upstairs room through a doorway went black.
    // - The clear glass of a house whose rooms are drawn (drawn after the resolve, D-065): every
    //   probe's ray passes it, either way (#280). Met as a back face, it counted a probe by a
    //   window as inside something: the probe was moved out through the glass and lit its room
    //   with the street (a regionless bedroom 1.8 EV brighter than its own light), and the
    //   probes outside saw a lit pane where the room is.
    // - A hollow shell's painted glass: only a room's probe passes its back; met from outside, a
    //   ray would find the shell's inside.
    for (uint pane = 0u; pane < 2u && c.hit; pane++)
    {
        uint flags = candidate_row(f, c.instance, c.primitive).flags;
        if ((flags & MATERIAL_FLAG_GLAZED) == 0)
            break;
        bool clear = (flags & MATERIAL_FLAG_DRAWN_AFTER) != 0;
        bool room = region != REGION_NONE && region != REGION_OUTSIDE;
        if (!clear && !(room && ray_back_face(f, c.instance, c.primitive, direction)))
            break;
        desc.TMin = c.t + 0.002;
        c = trace_closest(f, desc, false, true, probe * 0x27D4EB2Fu ^ ray);
    }
    float3 radiance = float3(0.0);
    float distance = PROBE_MISS_DISTANCE;
    if (c.hit)
    {
        float t = c.t;
        if (fixed)
        {
            distance = ray_back_face(f, c.instance, c.primitive, direction) ? -t : t;
        }
        else
        {
            RayHit hit = ray_hit(f, c.instance, c.primitive, c.barycentrics, origin, direction, t);
            distance = hit.back ? -t : t;
            // On the first update after a reset no probe has measured anything, and a hit's
            // light would fall back to the open sky's, too bright indoors: the probes started
            // in a white flash that took five frames to fade (#171). The sun alone instead:
            // they start a little dark, and the next updates add the bounces. A probe outside
            // every house (most of them) lights what its rays meet as the outside's without
            // finding its region (D-068): what it sees through an open door is a little brighter,
            // and the lookup cost the rays 0.14 ms.
            if (!hit.back)
            {
                // A lamp's globe glows with the lamp's own light, which `hit_lamps` counts
                // (D-065): met by one ray in hundreds, it flickered the probe.
                if ((candidate_row(f, c.instance, c.primitive).flags & MATERIAL_FLAG_LAMP) != 0)
                    hit.emissive = float3(0.0);
                radiance = probe_push.first_update != 0 ? hit_sunlight(f, hit) : hit_radiance(f, sky, field, hit, direction, region == REGION_OUTSIDE);
                // The lamps lighting the probe's region (all for a probe without one).
                radiance += hit_lamps(f, hit, region, probe * 0x9E3779B9u + ray, true);
            }
        }
    }
    else if (!fixed)
    {
        float4 up_table = sky[SH_COEFFICIENTS];
        radiance = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, sky[SH_COEFFICIENTS + 1].xyz, direction) * 3.14159265;
    }
    if (any(isnan(radiance)) || any(isinf(radiance)))
        radiance = float3(0.0); // one bad texel would spread through the blends for good
    field[0].ray_data[probe * field[0].rays + ray] = pack_ray(radiance, distance);
}

// The resolve's entry points: without shadow rays, and with them (`_rt`, devices with ray
// queries; `MeshletRenderer::new` picks).
[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_standard_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID, uint index : SV_GroupIndex)
{
    resolve_standard<0>(group, thread, index);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_ice_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<0>(MATERIAL_CLASS_ICE, group, thread);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_layered_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<0>(MATERIAL_CLASS_LAYERED, group, thread);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_standard_rt_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID, uint index : SV_GroupIndex)
{
    resolve_standard<1>(group, thread, index);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_ice_rt_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<1>(MATERIAL_CLASS_ICE, group, thread);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_layered_rt_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<1>(MATERIAL_CLASS_LAYERED, group, thread);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_jelly_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<0>(MATERIAL_CLASS_JELLY, group, thread);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_jelly_rt_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    resolve_tiles<1>(MATERIAL_CLASS_JELLY, group, thread);
}

// ------------------------------------------------------------ ray tracing ---

// The top-level acceleration structure's records (issue #45, crates/forge-render/src/raytrace.rs):
// a thread per instance writes its VkAccelerationStructureInstanceKHR, 64 bytes: the model's
// top three rows, the instance index (24 bits) and a full mask (none for a vacant slot, #220), no
// SBT offset, back faces traced too, and the address of its mesh's bottom-level structure.
struct TlasPush
{
    Instance* instances;
    uint64_t* blas;   // per mesh
    uint* records;    // 16 words per instance
    RtMesh* rt_meshes;   // per mesh: the rows its instances draw with (#265)
    Material* materials; // the rows
    uint count;
    uint first;       // the first instance of the table it covers: 0, or the movers' (#79)
    int4 origin_cell;    // the scene frame's origin (issue #93): its cell...
    float4 origin_local; // ...and the offset inside it; the structure is built in that frame
};

[[vk::push_constant]]
TlasPush tlas_push;

[shader("compute")]
[numthreads(64, 1, 1)]
void tlas_instances_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= tlas_push.count)
        return;
    uint index = tlas_push.first + id.x;
    Instance inst = tlas_push.instances[index];
    uint* r = tlas_push.records + id.x * 16;
    // The transform in the scene frame: the cells in integers first, then the offsets.
    float3x3 m = instance_frame(inst);
    float3 t = float3(inst.cell - tlas_push.origin_cell.xyz) * CELL_SIZE + (inst.local - tlas_push.origin_local.xyz);
    [unroll]
    for (uint row = 0; row < 3; row++)
    {
        [unroll]
        for (uint column = 0; column < 3; column++)
            r[row * 4 + column] = asuint(m[row][column]);
        r[row * 4 + 3] = asuint(t[row]);
    }
    // A vacant slot (#220) is inactive: no structure, which the build would read (its mesh's
    // may be freed), and no mask bit a ray could meet.
    bool vacant = (inst.flags & INSTANCE_VACANT) != 0;
    // The worlds of rays it is left out of (#301).
    uint mask = 0xFFu;
    if ((inst.flags & INSTANCE_NO_SHADOW_RAYS) != 0)
        mask &= ~RAY_SHADOW;
    if ((inst.flags & INSTANCE_NO_FAR_SHADOW_RAYS) != 0)
        mask &= ~RAY_SHADOW_FAR;
    if ((inst.flags & INSTANCE_NO_PROBE_RAYS) != 0)
        mask &= ~RAY_PROBE;
    if ((inst.flags & INSTANCE_NO_MIRROR_RAYS) != 0)
        mask &= ~RAY_MIRROR;
    if (vacant)
        mask = 0u;
    r[12] = (index & 0xFFFFFFu) | (mask << 24);
    // An instance whose rows are all opaque is opaque, and the rays' query loop never sees it:
    // one cut-out row made every ray of the scene loop (#265). One whose cut-out or jelly rows
    // fall in its structure's second geometry takes the geometries' own opacity, so the loop sees
    // those rows' triangles alone (#307: not a tree's wood and its crown's cores); any other
    // stays non-opaque as a whole.
    uint masked = vacant ? 0u : masked_rows(inst);
    uint opacity = 0x4u; // VK_GEOMETRY_INSTANCE_FORCE_OPAQUE_BIT
    if (masked != 0u)
    {
        opacity = 0x8u; // FORCE_NO_OPAQUE
        if (tlas_push.rt_meshes != nullptr)
        {
            if ((masked & ~tlas_push.rt_meshes[inst.mesh].masked_sections) == 0u)
                opacity = 0x0u;
        }
    }
    // Its structure's opacity micromaps (#333) where its cut-out rows read as those they follow,
    // and, where the probes' rays see it, only maps with no opaque cut-out micro-triangle: they
    // keep a quarter of a cut-out's triangles (`probe_leaf_kept`), which a micro-triangle met as
    // opaque would skip. Any other instance turns them off, and its cut-out triangles all reach
    // the query loop, as without them.
    if (!vacant && tlas_push.rt_meshes != nullptr)
    {
        RtMesh rt = tlas_push.rt_meshes[inst.mesh];
        bool probes = (inst.flags & INSTANCE_NO_PROBE_RAYS) == 0u;
        if (rt.opacity_material != RT_NO_OPACITY && (!opacity_rows_match(inst, rt) || (probes && rt.opacity_probes == 0u)))
            opacity |= 0x20u; // VK_GEOMETRY_INSTANCE_DISABLE_OPACITY_MICROMAPS_EXT
    }
    // VK_GEOMETRY_INSTANCE_TRIANGLE_FACING_CULL_DISABLE_BIT, and the opacity.
    r[13] = (0x1u | opacity) << 24;
    uint64_t blas = vacant ? 0 : tlas_push.blas[inst.mesh];
    r[14] = uint(blas);
    r[15] = uint(blas >> 32);
}

// Whether the instance's cut-out rows read as those its mesh's opacity micromaps follow: the fields
// the query loop's tests read the same (`GpuMaterial::same_ray_alpha`, #333).
bool opacity_rows_match(Instance inst, RtMesh rt)
{
    if (inst.material == rt.opacity_material)
        return true;
    if (tlas_push.materials == nullptr)
        return false;
    for (uint s = 0; s < min(rt.rows, 32u); s++)
    {
        if ((rt.masked_sections & (1u << s)) == 0u)
            continue;
        Material a = tlas_push.materials[inst.material + s];
        Material b = tlas_push.materials[rt.opacity_material + s];
        if (a.flags != b.flags || a.class_index != b.class_index || a.albedo_texture != b.albedo_texture
            || a.sampler != b.sampler || any(a.uv_u != b.uv_u) || any(a.uv_v != b.uv_v)
            || a.alpha != b.alpha || a.alpha_cutoff != b.alpha_cutoff)
            return false;
    }
    return true;
}

// The rows the instance draws with that are not opaque to the rays, a bit per section: cut-out or
// jelly. A section from 32 on sets every bit; without the rows (no table given), all are.
uint masked_rows(Instance inst)
{
    if (tlas_push.materials == nullptr || tlas_push.rt_meshes == nullptr)
        return 0xFFFFFFFFu;
    uint rows = tlas_push.rt_meshes[inst.mesh].rows;
    uint masked = 0u;
    for (uint s = 0; s < rows; s++)
    {
        Material mat = tlas_push.materials[inst.material + s];
        if ((mat.flags & MATERIAL_FLAG_MASKED) != 0 || mat.class_index == MATERIAL_CLASS_JELLY || clear_glass(mat))
            masked |= s < 32u ? 1u << s : 0xFFFFFFFFu;
    }
    return masked;
}

// ---------------------------------------------------------------- placement ---

// GPU placement of the city-blocks instances (issue #35): a thread per instance slot writes
// that slot's `Instance` from the seed and the slot's index alone, so the table is the same on
// every run. The slots come in category ranges, in this order: buildings on the city's block
// lots, lamp posts along both sides of every street, a fountain and four columns on some
// street crossings, rocks and rubble in the hills around the city. The mesh choice is mirrored
// on the CPU (`forge_render::placement`), which needs the per-mesh counts.
public struct Placement
{
    public Instance* instances; // the scene's table
    public Mesh* meshes;
    public float* heights;      // the terrain's samples, `samples` per row, rows along +z
    public uint first;          // first slot of the placed range in `instances`
    public uint count;
    public uint samples;
    public uint seed;
    public float spacing;       // metres between samples
    public float half_size;     // half the terrain's side
    public float city_half;     // half the city square's side
    public float block;         // block pitch: a block and a street, metres
    public float street;        // street width
    public float lamp_step;     // metres between lamp posts
    public uint blocks;         // blocks per side of the city
    public uint plaza_every;    // crossings k with k % plaza_every == plaza_every / 2 on both axes hold a plaza
    public uint buildings;      // slots per category
    public uint lamps;
    public uint plaza_slots;
    public uint rocks;
    public uint building_mesh_count;
    public uint rock_mesh_count;
    public uint lamp_mesh;
    public uint fountain_mesh;
    public uint column_mesh;
    public uint rock_rule;      // ROCKS_IN_HILLS (the city) or ROCKS_ON_LAND (the island, #96)
    public uint building_meshes[16];
    public uint rock_meshes[8];
    public int3 origin_cell;    // the scene's origin (issue #93): its cell...
    public int pad2;
    public float3 origin_local; // ...and the offset inside it, metres
    public float rock_above;    // ROCKS_ON_LAND: metres above which the ground takes rocks
    public uint2* sites;        // ROCKS_AT_SITES (#130): per rock, (cumulative weight, cell) of its cells with any
    public uint site_cells[2];  // entries of each rock in `sites`, the second's after the first's
    public uint first_rocks;    // ROCKS_AT_SITES: rock slots on the first rock, the rest on the second
    public uint second_mesh_count;
    public uint site_size;      // cells per side, over the ground's square, rows along +z
    public float site_spacing;  // metres across a cell
    public uint second_meshes[8];
};

struct PlacementPush
{
    Placement* placement;
};

[[vk::push_constant]]
PlacementPush placement_push;

// forge_core::hash::pcg4d, bit for bit.
uint4 pcg4d(uint4 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    v ^= v >> 16u;
    v.x += v.y * v.w;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v.w += v.y * v.z;
    return v;
}

// Four uniform numbers in [0, 1) for `slot` and a purpose `salt`.
float4 random4(Placement* p, uint slot, uint salt)
{
    return float4(pcg4d(uint4(slot, p[0].seed, salt, 0u)) >> 8u) * (1.0 / 16777216.0);
}

// The ground's height at (x, z): the terrain's samples, bilinearly.
float ground(Placement* p, float x, float z)
{
    float2 g = clamp((float2(x, z) + p[0].half_size) / p[0].spacing, 0.0, float(p[0].samples - 1) - 1e-3);
    uint2 i = uint2(g);
    float2 f = g - float2(i);
    uint n = p[0].samples;
    float h00 = p[0].heights[i.y * n + i.x];
    float h10 = p[0].heights[i.y * n + i.x + 1];
    float h01 = p[0].heights[(i.y + 1) * n + i.x];
    float h11 = p[0].heights[(i.y + 1) * n + i.x + 1];
    return lerp(lerp(h00, h10, f.x), lerp(h01, h11, f.x), f.y);
}

// Writes the instance of `slot`: `mesh` at `position`, turned by `yaw` about +Y and then by
// `tilt` about the turned X axis, scaled by `scale`, in the mesh's material.
void place(Placement* p, uint slot, uint mesh, float3 position, float yaw, float tilt, float scale)
{
    // R = Ry(yaw) × Rx(tilt): the tilt first, then the turn, as a quaternion.
    float4 q = quat_mul(float4(0.0, sin(0.5 * yaw), 0.0, cos(0.5 * yaw)), float4(sin(0.5 * tilt), 0.0, 0.0, cos(0.5 * tilt)));
    place_turned(p, slot, mesh, position, q, scale);
}

// Writes the instance of `slot`: `mesh` at `position`, turned by the quaternion `q`, scaled by
// `scale`, in the mesh's material.
void place_turned(Placement* p, uint slot, uint mesh, float3 position, float4 q, float scale)
{
    // The instance's cell and offset (issue #93): the scene's origin, a cell and an offset,
    // plus the position in the scene (the ground was sampled there), carried into the cell so
    // the offset stays under CELL_SIZE. Wherever the scene stands, the offset is as fine as at
    // the origin.
    float3 local = p[0].origin_local + position;
    int3 carry = int3(floor(local / CELL_SIZE));
    Mesh* m = p[0].meshes + mesh;
    Instance inst;
    inst.cell = p[0].origin_cell + carry;
    inst.mesh = mesh;
    inst.local = local - float3(carry) * CELL_SIZE;
    inst.scale = scale;
    inst.rotation = q;
    inst.center = mul(quat_matrix(q), m[0].center) * scale + inst.local;
    inst.radius = m[0].radius * scale;
    inst.id = p[0].first + slot;
    inst.material = m[0].material;
    inst.flags = 0u;
    inst.texture = 0u;
    p[0].instances[p[0].first + slot] = inst;
}

// Where `Placement::rock_rule` puts the rocks.
static const uint ROCKS_IN_HILLS = 0; // the four bands of hills around the city
static const uint ROCKS_ON_LAND = 1;  // the island's land (#96)
static const uint ROCKS_AT_SITES = 2; // the island's rock sites (#130)
// ROCKS_ON_LAND on low ground (#117): under x metres the rocks keep z of their chance, the
// whole of it from y metres up.
static const float3 ROCKS_LOWLAND = float3(5.0, 40.0, 0.1);

// ROCKS_ON_LAND: candidates over the whole square until one stands on land above `rock_above`
// metres and is kept, with a chance that rises with the slope (more rocks on the steep ground,
// a few on the flat); `found` is false when 32 candidates found none.
float2 rock_on_land(Placement* p, uint slot, out bool found)
{
    float h = p[0].half_size;
    float d = p[0].spacing;
    float2 at = float2(0.0);
    found = false;
    for (uint k = 0; k < 32 && !found; k++)
    {
        float4 c = random4(p, slot, 16u + k);
        at = float2(lerp(-h, h, c.x), lerp(-h, h, c.y));
        float gx = (ground(p, at.x + d, at.y) - ground(p, at.x - d, at.y)) / (2.0 * d);
        float gz = (ground(p, at.x, at.y + d) - ground(p, at.x, at.y - d)) / (2.0 * d);
        float keep = lerp(0.15, 1.0, saturate(length(float2(gx, gz)) / 0.6));
        // The coastal plain (#117): low ground keeps a tenth of them, so the plain is not a
        // field of boulders; the hills take the rest.
        float height = ground(p, at.x, at.y);
        keep *= lerp(ROCKS_LOWLAND.z, 1.0, smoothstep(ROCKS_LOWLAND.x, ROCKS_LOWLAND.y, height));
        found = height > p[0].rock_above && c.z < keep;
    }
    return at;
}

// ROCKS_AT_SITES (#130): rock `s` of the slot `slot`, on the first rock or the second by its
// slot. Its cell is drawn from that rock's sites by their weights (the first entry whose
// cumulative weight passes a uniform draw), the spot anywhere in it. Its size is mostly small,
// now and then big: 0.3 to 2.2 times its mesh. It leans two thirds of the way with the ground,
// is turned any way and tipped a little, and sinks deeper the steeper the ground.
void rock_at_site(Placement* p, uint slot, uint s)
{
    float4 r = random4(p, slot, 4u);
    float4 q = random4(p, slot, 5u);
    uint pick = pcg4d(uint4(slot, p[0].seed, 6u, 0u)).x;
    bool second = s >= p[0].first_rocks;
    uint mesh = second ? p[0].second_meshes[pick % p[0].second_mesh_count] : p[0].rock_meshes[pick % p[0].rock_mesh_count];
    uint lo = second ? p[0].site_cells[0] : 0u;
    uint hi = lo + p[0].site_cells[second ? 1 : 0];
    uint draw = pcg4d(uint4(slot, p[0].seed, 7u, 0u)).x % p[0].sites[hi - 1u].x;
    while (lo < hi)
    {
        uint mid = (lo + hi) / 2u;
        if (p[0].sites[mid].x > draw)
            hi = mid;
        else
            lo = mid + 1u;
    }
    uint cell = p[0].sites[lo].y;
    float2 at = (float2(cell % p[0].site_size, cell / p[0].site_size) + r.xy) * p[0].site_spacing - p[0].half_size;
    float d = p[0].spacing;
    float gx = (ground(p, at.x + d, at.y) - ground(p, at.x - d, at.y)) / (2.0 * d);
    float gz = (ground(p, at.x, at.y + d) - ground(p, at.x, at.y - d)) / (2.0 * d);
    float3 lean = normalize(lerp(float3(0.0, 1.0, 0.0), normalize(float3(-gx, 1.0, -gz)), 0.67));
    // From +Y to `lean`: the half-way quaternion (cross, 1 + dot), normalised.
    float4 align = normalize(float4(lean.z, 0.0, -lean.x, 1.0 + lean.y));
    float yaw = r.z * 6.2831853, tilt = (q.y - 0.5) * 0.3;
    float4 turn = quat_mul(float4(0.0, sin(0.5 * yaw), 0.0, cos(0.5 * yaw)), float4(sin(0.5 * tilt), 0.0, 0.0, cos(0.5 * tilt)));
    float scale = 0.3 + 1.9 * q.x * q.x * q.x;
    float radius = p[0].meshes[mesh].radius * scale;
    float y = ground(p, at.x, at.y) - radius * 0.3 * min(length(float2(gx, gz)), 1.5);
    place_turned(p, slot, mesh, float3(at.x, y, at.y), quat_mul(align, turn), scale);
}

[shader("compute")]
[numthreads(64, 1, 1)]
void place_main(uint3 id : SV_DispatchThreadID)
{
    Placement* p = placement_push.placement;
    uint slot = id.x;
    if (slot >= p[0].count)
        return;
    float quarter_turn = 1.5707963;
    uint s = slot;
    if (s < p[0].buildings)
    {
        // Four lots per block, a building in each, turned to one of four sides.
        uint b = s / 4u, lot = s % 4u;
        float inner = p[0].block - p[0].street;
        float2 corner = -p[0].city_half + p[0].street * 0.5 + float2(b % p[0].blocks, b / p[0].blocks) * p[0].block;
        float2 at = corner + (float2(lot & 1u, lot >> 1u) + 0.5) * inner * 0.5;
        uint4 h = pcg4d(uint4(slot, p[0].seed, 1u, 0u));
        uint mesh = p[0].building_meshes[h.x % p[0].building_mesh_count];
        place(p, slot, mesh, float3(at.x, ground(p, at.x, at.y), at.y), float(h.y % 4u) * quarter_turn, 0.0, 1.0);
        return;
    }
    s -= p[0].buildings;
    if (s < p[0].lamps)
    {
        // Along both sides of every street line, in both directions.
        uint per_line = uint(2.0 * p[0].city_half / p[0].lamp_step);
        uint post = s % per_line;
        uint side = (s / per_line) % 2u;
        uint line = (s / per_line / 2u) % (p[0].blocks + 1u);
        uint axis = s / per_line / 2u / (p[0].blocks + 1u);
        float along = -p[0].city_half + (float(post) + 0.5) * p[0].lamp_step;
        float across = -p[0].city_half + float(line) * p[0].block + (side == 0u ? -1.0 : 1.0) * (p[0].street * 0.5 - 1.5);
        float2 at = axis == 0u ? float2(along, across) : float2(across, along);
        place(p, slot, p[0].lamp_mesh, float3(at.x, ground(p, at.x, at.y), at.y), 0.0, 0.0, 1.0);
        return;
    }
    s -= p[0].lamps;
    if (s < p[0].plaza_slots)
    {
        // A fountain on the crossing and four columns around it.
        uint plaza = s / 5u, part = s % 5u;
        uint per_side = (p[0].blocks + 1u + p[0].plaza_every - 1u - p[0].plaza_every / 2u) / p[0].plaza_every;
        uint2 k = uint2(plaza % per_side, plaza / per_side) * p[0].plaza_every + p[0].plaza_every / 2u;
        float2 at = -p[0].city_half + float2(k) * p[0].block;
        uint mesh = p[0].fountain_mesh;
        if (part > 0u)
        {
            float a = (float(part) - 0.5) * quarter_turn;
            at += float2(cos(a), sin(a)) * 7.5;
            mesh = p[0].column_mesh;
        }
        place(p, slot, mesh, float3(at.x, ground(p, at.x, at.y), at.y), 0.0, 0.0, 1.0);
        return;
    }
    s -= p[0].plaza_slots;
    if (p[0].rock_rule == ROCKS_AT_SITES)
    {
        rock_at_site(p, slot, s);
        return;
    }
    // Rocks: uniform over the four bands of hills around the city (north and south span the
    // whole side, east and west the city's depth), any turn, a slight tilt, sunk a little.
    float4 r = random4(p, slot, 4u);
    float4 q = random4(p, slot, 5u);
    uint mesh = p[0].rock_meshes[pcg4d(uint4(slot, p[0].seed, 6u, 0u)).x % p[0].rock_mesh_count];
    float h = p[0].half_size, c = p[0].city_half;
    float ns = 2.0 * h * (h - c), ew = 2.0 * c * (h - c); // areas of one north/south and one east/west band
    float pick = r.x * 2.0 * (ns + ew);
    float2 at;
    bool on_land = true;
    if (p[0].rock_rule == ROCKS_ON_LAND)
        at = rock_on_land(p, slot, on_land);
    else if (pick < 2.0 * ns)
        at = float2(lerp(-h, h, r.y), lerp(c, h, r.z) * (pick < ns ? 1.0 : -1.0));
    else
        at = float2(lerp(c, h, r.y) * (pick < 2.0 * ns + ew ? 1.0 : -1.0), lerp(-c, c, r.z));
    float scale = lerp(0.5, 1.6, q.x * q.x);
    float radius = p[0].meshes[mesh].radius * scale;
    float y = ground(p, at.x, at.y) - 0.15 * radius;
    // A rock that found no land lies 50 m under the ground, out of sight (its slot is taken).
    // Apart, so the city's rocks keep the arithmetic, and the bits, they had.
    if (!on_land)
        y -= 50.0;
    place(p, slot, mesh, float3(at.x, y, at.y), r.w * 6.2831853, (q.y - 0.5) * 0.4, scale);
}

// ------------------------------------------------------------------- movers ---
//
// Moving geometry (#79; `docs/research/dynamic-scenes.md`, "Recommendation for Forge"): the
// movers are a trailing range of the instance table the CPU writes every frame, their records
// copied in before the culls (`movers/upload`) and their cells' bounds taken again
// (`cell_bounds_main`, `movers/cell bounds`). The two-pass occlusion then needs nothing more: an
// instance the previous pyramid hides is tested again against this frame's in pass 2, wherever
// it stood before. What the camera alone does not explain is their motion: `mover_motion_main`
// writes it over the camera's for the pixels a mover shows.

// Mirrors `GpuMoverMotion` in meshlet.rs.
struct MoverMotion
{
    float4x4 relative_from_clip;     // this frame's unjittered clip → camera-relative
    float4x4 previous_from_relative; // camera-relative → the frame before's unjittered clip
    Frame* frame;                    // the frame block: the camera's cell, the instances, the visible list
    Instance* previous;              // the movers' records of the frame before (their slot of the ring)
    uint visibility;                 // sampled (r32ui): the visibility buffer
    uint depth;                      // sampled (r32f): the depth
    uint motion;                     // storage (rg16f): the motion vectors, which the camera's pass wrote
    uint first;                      // the first mover's instance
    uint count;
    uint width;
    uint height;
    float jitter_x;                  // this frame's jitter in pixels
    float jitter_y;
    uint skinned;                    // the scene has skinned meshes (#165)
    SkinCluster* skin_clusters;      // the skin pass's clusters (skin.slang)
    float* skin_previous;            // where the frame before's joints put each skinned vertex, three floats each
    uint2 pad;
};

// Mirrors `SkinCluster` in skin.slang: where a skinned cluster's vertices lie in its tables.
public struct SkinCluster
{
    public uint pool;
    public uint first;          // its first vertex in `skin_previous` and `Frame::skin_vertices`
    public uint vertex_count;
    public uint ray;
    public uint joints;
    public uint field;          // its height field (#185), which the skin pass reads
    public uint flags;          // skin.slang's SKIN_* flags (#169)
    public uint pad;
};

struct MoverMotionPush
{
    MoverMotion* motion;
};

[[vk::push_constant]]
MoverMotionPush mover_motion_push;

// The weights of `q` on the triangle `a b c` (q projected onto its plane); a degenerate
// triangle's first corner.
float3 triangle_weights(float3 q, float3 a, float3 b, float3 c)
{
    float3 e0 = b - a;
    float3 e1 = c - a;
    float3 e2 = q - a;
    float d00 = dot(e0, e0);
    float d01 = dot(e0, e1);
    float d11 = dot(e1, e1);
    float d20 = dot(e2, e0);
    float d21 = dot(e2, e1);
    float denom = d00 * d11 - d01 * d01;
    if (abs(denom) <= 1e-20)
        return float3(1.0, 0.0, 0.0);
    float v = (d11 * d20 - d01 * d21) / denom;
    float w = (d00 * d21 - d01 * d20) / denom;
    return float3(1.0 - v - w, v, w);
}

// A skinned mesh's point (#165) moves with its triangle's vertices, not rigidly with its
// mover: `object`, the pixel's point in the mover's frame now, weighed on the triangle the
// visibility id `vis` names and placed by the same weights between where the frame before's
// joints put the triangle's vertices (still in the mover's frame, which the caller takes
// back through the mover's transform before). Other meshes' points come back as they are.
float3 skinned_before(MoverMotion* p, Frame* f, uint mesh_index, uint vis, float3 object)
{
    Mesh mesh = f[0].meshes[mesh_index];
    if (mesh.skin == 0xFFFFFFFFu)
        return object;
    uint meshlet_index = f[0].visible[vis >> VISIBILITY_TRIANGLE_BITS].y & MESHLET_INDEX_MASK;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    Meshlet m = f[0].meshlets[meshlet_index];
    uint base = payload_base(f, m);
    uint k0 = paged_index(f, base, m, tri, 0);
    uint k1 = paged_index(f, base, m, tri, 1);
    uint k2 = paged_index(f, base, m, tri, 2);
    float3 w = triangle_weights(object, paged_vertex(f, base, m, k0).position,
                                paged_vertex(f, base, m, k1).position, paged_vertex(f, base, m, k2).position);
    SkinCluster c = p[0].skin_clusters[mesh.skin + meshlet_index - mesh.meshlet_offset];
    float* previous = p[0].skin_previous;
    uint i0 = 3 * (c.first + k0);
    uint i1 = 3 * (c.first + k1);
    uint i2 = 3 * (c.first + k2);
    return w.x * float3(previous[i0], previous[i0 + 1], previous[i0 + 2])
         + w.y * float3(previous[i1], previous[i1 + 1], previous[i1 + 2])
         + w.z * float3(previous[i2], previous[i2 + 1], previous[i2 + 2]);
}

// A mover's pixel moves with the mover: its point, taken back onto the mover through this
// frame's transform, is placed where the transform of the frame before held it, then seen by
// the camera of the frame before. Where the water was drawn over a mover the depth is the
// water's, and the point not quite the mover's.
[shader("compute")]
[numthreads(8, 8, 1)]
void mover_motion_main(uint3 id : SV_DispatchThreadID)
{
    MoverMotion* p = mover_motion_push.motion;
    if (id.x >= p[0].width || id.y >= p[0].height)
        return;
    uint vis = g_sampled_images_u32[p[0].visibility].Load(int3(id.xy, 0));
    if (vis == 0xFFFFFFFFu || is_grass(vis))
        return;
    Frame* f = p[0].frame;
    uint instance = f[0].visible[vis >> VISIBILITY_TRIANGLE_BITS].x & ~INSTANCE_HIDDEN_BIT;
    if (instance < p[0].first || instance - p[0].first >= p[0].count)
        return;
    Instance now = f[0].instances[instance];
    // A mover moved with what it shows (a ground window's step, #197): the camera's motion.
    if ((now.flags & INSTANCE_STILL) != 0)
        return;
    Instance before = p[0].previous[instance - p[0].first];
    float depth = g_sampled_images_r[p[0].depth].Load(int3(id.xy, 0));
    float2 size = float2(float(p[0].width), float(p[0].height));
    float2 uv = (float2(id.xy) + 0.5 - float2(p[0].jitter_x, p[0].jitter_y)) / size;
    float4 h = mul(p[0].relative_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    float3 point = h.xyz / h.w;
    float3 object = mul(transpose(quat_matrix(now.rotation)), point - instance_relative(f, now)) / now.scale;
    if (p[0].skinned != 0)
        object = skinned_before(p, f, now.mesh, vis, object);
    float3 point_before = mul(quat_matrix(before.rotation), object * before.scale) + instance_relative(f, before);
    float4 clip = mul(p[0].previous_from_relative, float4(point_before, 1.0));
    if (clip.w <= 1e-9)
        return;
    float2 ndc = clip.xy / clip.w;
    float2 previous_uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    g_storage_images_rg16f[p[0].motion][id.xy] = previous_uv - uv;
}

// A grass blade's pixel moves with the wind (#300): its point, placed on its triangle as the
// blade stands now, is put on the same triangle as the blade stood at the sway's time of the
// frame before (`Frame::grass_wind_before`), then seen by the camera of the frame before. Uses
// `mover_motion_push`'s block (`first` and `count` unused).
[shader("compute")]
[numthreads(8, 8, 1)]
void grass_motion_main(uint3 id : SV_DispatchThreadID)
{
    MoverMotion* p = mover_motion_push.motion;
    if (id.x >= p[0].width || id.y >= p[0].height)
        return;
    uint vis = g_sampled_images_u32[p[0].visibility].Load(int3(id.xy, 0));
    if (!is_grass(vis))
        return;
    Frame* f = p[0].frame;
    float depth = g_sampled_images_r[p[0].depth].Load(int3(id.xy, 0));
    float2 size = float2(float(p[0].width), float(p[0].height));
    float2 uv = (float2(id.xy) + 0.5 - float2(p[0].jitter_x, p[0].jitter_y)) / size;
    float4 h = mul(p[0].relative_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    float3 point = h.xyz / h.w;
    uint cell = (vis >> 10) & ((1u << GRASS_CELL_BITS) - 1u);
    uint index = (vis >> 3) & (GRASS_CELL_BLADES - 1u);
    uint3 k = GRASS_TRIANGLES[min(vis & 7u, GRASS_BLADE_TRIANGLES - 1u)];
    float3 camera = f[0].camera_in_scene.xyz;
    GrassBlade now = grass_blade(f, cell, index);
    GrassBlade before = grass_blade_at(f, cell, index, f[0].grass_wind_before);
    float3 p0 = grass_vertex(now, k.x) - camera;
    float3 e0 = grass_vertex(now, k.y) - camera - p0;
    float3 e1 = grass_vertex(now, k.z) - camera - p0;
    // The point's barycentrics on the triangle's plane.
    float3 v = point - p0;
    float d00 = dot(e0, e0);
    float d01 = dot(e0, e1);
    float d11 = dot(e1, e1);
    float den = d00 * d11 - d01 * d01;
    if (abs(den) < 1e-14)
        return;
    float l1 = (d11 * dot(v, e0) - d01 * dot(v, e1)) / den;
    float l2 = (d00 * dot(v, e1) - d01 * dot(v, e0)) / den;
    float3 q0 = grass_vertex(before, k.x) - camera;
    float3 q1 = grass_vertex(before, k.y) - camera;
    float3 q2 = grass_vertex(before, k.z) - camera;
    float3 point_before = q0 + l1 * (q1 - q0) + l2 * (q2 - q0);
    float4 clip = mul(p[0].previous_from_relative, float4(point_before, 1.0));
    if (clip.w <= 1e-9)
        return;
    float2 ndc = clip.xy / clip.w;
    float2 previous_uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    g_storage_images_rg16f[p[0].motion][id.xy] = previous_uv - uv;
}
