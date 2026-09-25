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
    public uint3 pad;
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
};

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
    public uint pad2;
};

public static const uint MATERIAL_CLASS_STANDARD = 0;
public static const uint MATERIAL_CLASS_ICE = 1;
public static const uint MATERIAL_CLASS_LAYERED = 2;
public static const uint MATERIAL_CLASSES = 3;
static const uint TEXTURE_NONE = 0xFFFFFFFFu;

public struct Instance
{
    public float4x4 model;
    public float3 center; // world-space bounding sphere of the instance
    public float radius;
    public uint mesh;
    public uint id;
    public uint material; // its row in `Frame::materials`
    public uint pad;
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

// What a ray's hit reads to shade its triangle (issue #50, crates/forge-render/src/raytrace.rs):
// every mesh's cut, as the bottom-level structures hold it, and where each mesh starts in it.
public struct RtMesh
{
    public uint first_vertex;
    public uint first_triangle;
    public float cut_error;  // how far the cut may stand from the drawn surface, object space
    public uint pad;
};

public struct RtScene
{
    public float* positions;  // three floats a vertex
    public uint* indices;     // three a triangle, local to the mesh
    public uint* sections;    // one a triangle: the row after the instance's first it takes
    public RtMesh* meshes;
};

public struct Frame
{
    public float4x4 view_proj;      // for drawing
    public float4x4 cull_view_proj; // for culling (frozen when FLAG_FREEZE_CULLING)
    public float4x4 cull_view;      // view matrix used for culling
    public float4x4 prev_cull_view; // the previous frame's (with FLAG_PREV_PYRAMID)
    public float4 planes[6];        // frustum planes of `cull_view_proj`, inward normals
    public float3 camera_pos;       // camera used for culling
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
    public uint64_t* cluster_lookback; // status words of the cluster culls' ordered appends: `cluster_groups` per pass, then as many for pass 1's rejects (issue #92)
    public uint cluster_groups;     // cluster-cull workgroups the fullest lists need: `work_capacity` items, then `work_capacity` roots at 32 an item
    public uint prev_hzb_image;     // sampled image index of the previous frame's pyramid
    public uint2* work;             // (instance, local group) per work item, built by `cull_main`
    public uint* indirect;          // the cluster cull's grid (x, y, 1, work item count), the instance cull's ticket (4), the regular work items (5) and the root entries (6) the items cover
    public uint2* visible;          // (instance, meshlet | flag bits) per drawn cluster, in cull order: pass 1, then pass 2
    public uint visible_capacity;   // slots in `visible`
    public float exposure;          // pre-exposure of the frame (crates/forge-render/src/exposure.rs)
    public float sun_illuminance;   // lux; the rocks return albedo × E / π
    public float sun_angular_radius; // the sun disc's angular radius, radians; 0: hard shadows (issue #54)
    public uint* clusters;          // per pass (pass 1 or the single pass at 0, pass 2 at 12): the hardware draw grid (x, y, 1) and its cluster count, the software raster grid and its count, the merge's VkDrawIndirectCommand; then the culls' tickets (24, 25)
    public uint* draws;             // fallback: a VkDrawIndexedIndirectCommand per hardware-drawn cluster of the pass; null on the mesh path
    public uint64_t* lookback;      // status words of the instance cull's ordered appends
    public uint* raster;            // per pass, the list slots of its clusters: hardware-drawn from the front, software-rasterised from the back
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
    public float4* cells;           // with FLAG_INSTANCE_CELLS: per cell of 64 instances in table order, their bounding sphere (xyz centre, w radius; issue #38)
    public uint* cell_list;         // with FLAG_INSTANCE_CELLS: instance cull 1's grid (x, y, 1, listed cells), cell cull 2's grid over the deferred cells (4), the cell culls' tickets (8, 9), then from CELL_LIST the listed cells and after them the deferred ones
    public uint2* rejects;          // with occlusion: (instance, meshlet | CULLED_BIT) per cluster pass 1 selected in view but the previous pyramid hid, in cull order: pass 2's to test (issue #92)
    public uint rejects_capacity;   // slots in `rejects`
    public uint rejects_pad;
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

// Vertex `i` of the payload at `base`.
PagedVertex paged_vertex(Frame* f, uint base, uint i)
{
    return ((PagedVertex*)f[0].pool)[base / PAGED_VERTEX_BYTES + i];
}

// Local vertex index `k` (0..2) of triangle `t` of cluster `m`, whose payload is at `base`.
uint paged_index(Frame* f, uint base, Meshlet m, uint t, uint k)
{
    return load_byte(f[0].pool, base + m.vertex_count * PAGED_VERTEX_BYTES + t * 3 + k);
}

// The material section of triangle `t` of cluster `m`: before the split its first section,
// after it its second (forge_geom::meshlet::triangle_section).
uint triangle_section(Meshlet m, uint t)
{
    return t < ((m.section >> 16) & 0xFFu) ? (m.section & 0xFFu) : ((m.section >> 8) & 0xFFu);
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
    float d = max(length(center - f[0].camera_pos) - radius, f[0].znear);
    return error * f[0].p11 * 0.5 * f[0].viewport_height / d;
}

// Exactly one cut through the DAG: draw when the parent is too coarse and this is fine
// enough. Siblings share the parent values and the children of a group share its values as
// their `self`, so the decision is consistent and watertight.
// `streamed` is a compile-time constant of each cluster-cull entry point (`cluster_cull`):
// the streamed cut's code and registers stay out of a resident scene's cull.
bool lod_selected(Frame* f, Meshlet m, float4x4 model, float scale, bool streamed)
{
    if ((f[0].flags & FLAG_LOD) == 0)
    {
        // Full detail; a streamed scene shows what of it is resident (a debug view).
        bool resident = !streamed || f[0].page_table[m.page] != PAGE_NONE;
        return m.lod_level == 0 && resident;
    }
    float3 self_center = mul(model, float4(m.self_center, 1)).xyz;
    float self_px = lod_error_pixels(f, self_center, m.self_radius * scale, m.self_error * scale);
    if (streamed)
        return lod_selected_streamed(f, m, model, scale, self_px);
    if (self_px > f[0].lod_threshold)
        return false;
    if (isinf(m.parent_error))
        return true;
    float3 parent_center = mul(model, float4(m.parent_center, 1)).xyz;
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
bool lod_selected_streamed(Frame* f, Meshlet m, float4x4 model, float scale, float self_px)
{
    if (f[0].page_table[m.page] == PAGE_NONE)
        return false;
    if (!isinf(m.parent_error))
    {
        float3 parent_center = mul(model, float4(m.parent_center, 1)).xyz;
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
    float dist = length(inst[0].center - f[0].camera_pos);
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

// Clusters pass 1 listed (hardware and software), final once pass 2 culls.
uint pass1_listed(Frame* f)
{
    return f[0].clusters[3] + f[0].clusters[7];
}

// The slot of the `index`-th cluster of this pass in the visible-cluster list: pass 2 follows
// pass 1, so the ids grow with the draw order across the passes.
uint visible_slot(Frame* f, uint index)
{
    return f[0].pass == PASS_REMAINDER ? pass1_listed(f) + index : index;
}

// This pass's hardware draw arguments (x, y, 1, count) in `Frame::clusters`; its software
// raster arguments follow (`pass_args(f) + 4`), then the merge's draw (`+ 8`).
uint* pass_args(Frame* f)
{
    return f[0].clusters + (f[0].pass == PASS_REMAINDER ? 12 : 0);
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
    v.software = v.dense && (f[0].flags & FLAG_SW_RASTER) != 0;
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
    Instance* inst = f[0].instances + v.instance;
    Meshlet m = f[0].meshlets[v.meshlet];
    describe_cluster(f, v, m);
    float scale = length(inst[0].model[0].xyz);
    float3 center = mul(inst[0].model, float4(m.center, 1)).xyz;
    float world_radius = m.radius * scale;
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
    v.instance &= ~INSTANCE_HIDDEN_BIT;
    Instance* inst = f[0].instances + v.instance;
    Mesh* mesh = f[0].meshes + inst[0].mesh;
    bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
    bool show_culled = (f[0].flags & FLAG_SHOW_CULLED) != 0;
    bool instance_visible = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
    // Uniform scale is assumed for the bounding spheres: take the length of the first column.
    float scale = length(inst[0].model[0].xyz);
    // Whole-group rejection from the per-level tables before any cluster is read.
    if (!instance_visible || (!root && !group_possible(f, mesh, inst, local_group, scale, streamed))
        || local_meshlet >= mesh[0].meshlet_count)
        return v;
    v.meshlet = mesh[0].meshlet_offset + local_meshlet;
    Meshlet m = f[0].meshlets[v.meshlet];
    describe_cluster(f, v, m);
    float3 center = mul(inst[0].model, float4(m.center, 1)).xyz;
    float world_radius = m.radius * scale;
    bool visible = lod_selected(f, m, inst[0].model, scale, streamed);
    visible = visible && (!frustum || sphere_in_frustum(f, center, world_radius));
    // Normal-cone culling (meshoptimizer): culled when dot(normalize(apex - camera), axis) >=
    // cutoff. A cutoff of 1 marks a cone wider than a hemisphere; meshoptimizer then leaves
    // the axis at zero, whose normalisation is NaN and would (silently) cull the meshlet:
    // skip the test.
    if (visible && (f[0].flags & FLAG_CONE_CULLING) != 0 && m.cone_cutoff < 1.0)
    {
        float3 apex = mul(inst[0].model, float4(m.cone_apex, 1)).xyz;
        float3 axis = normalize(mul(inst[0].model, float4(m.cone_axis, 0)).xyz);
        bool cone_visible = dot(normalize(apex - f[0].camera_pos), axis) < m.cone_cutoff;
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

static const uint LOOKBACK_LANES = 32; // predecessors read per round

groupshared uint64_t s_window[LOOKBACK_LANES];
groupshared uint64_t s_exclusive; // counts of the tickets before this one, summed so far
groupshared uint s_cursor;    // tickets [0, s_cursor) not yet added
groupshared uint s_found;     // a published prefix was reached

// Publishes `counts` for workgroup `ticket` in `status` and returns the sums of the counts of
// the tickets before it. Every thread of the workgroup calls it with the same arguments (the
// wait is shared): each round, the first LOOKBACK_LANES threads read one predecessor's status
// word each, nearest first, and thread 0 adds them up to the first prefix or to the first
// predecessor that has published nothing yet, where the next round starts.
uint2 lookback_exclusive(uint64_t* status, uint ticket, uint2 counts, uint thread)
{
    uint64_t count = lookback_pack(counts);
    uint64_t ignored;
    if (thread == 0)
    {
        InterlockedExchange(status[ticket], (ticket == 0 ? LOOKBACK_PREFIX : LOOKBACK_AGGREGATE) | count, ignored);
        s_exclusive = 0;
        s_cursor = ticket;
        s_found = ticket == 0 ? 1u : 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    while (s_found == 0)
    {
        uint cursor = s_cursor;
        if (thread < LOOKBACK_LANES)
        {
            uint64_t word = 0;
            if (thread < cursor)
                InterlockedOr(status[cursor - 1 - thread], 0ull, word); // an atomic read
            s_window[thread] = word;
        }
        GroupMemoryBarrierWithGroupSync();
        if (thread == 0)
        {
            uint n = min(LOOKBACK_LANES, cursor);
            uint added = 0;
            for (; added < n; added++)
            {
                uint64_t word = s_window[added];
                uint64_t flag = word & LOOKBACK_FLAGS;
                if (flag == 0)
                    break; // not published yet: the next round starts here
                s_exclusive += word & ~LOOKBACK_FLAGS;
                if (flag == LOOKBACK_PREFIX)
                {
                    s_found = 1;
                    break;
                }
            }
            s_cursor = cursor - added;
        }
        GroupMemoryBarrierWithGroupSync();
    }
    uint64_t exclusive = s_exclusive;
    if (thread == 0 && ticket != 0)
        InterlockedExchange(status[ticket], LOOKBACK_PREFIX | (exclusive + count), ignored);
    return lookback_unpack(exclusive);
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
groupshared uint s_kept_software;      // where the list ends: software clusters still kept

// One workgroup per CULL_ITEMS work items: cull their clusters and append the survivors to
// this pass's range of the visible-cluster list in item order, then lane order, and their
// slots to the pass's raster lists (hardware-drawn from the front, software-rasterised from
// the back, each in the same order); on the fallback path also one indexed draw per
// hardware-drawn survivor. The mesh shader and the software rasteriser read the raster
// lists, the fallback's draws carry the slots. The grids are written by the last workgroup,
// or, when the list is full, by the one it ends in. A full list drops what does not fit and
// counts it, so no draw reads past the end.
// `streamed`: the scene streams its pages (`lod_selected`), a constant per entry point.
void cluster_cull(uint thread, bool streamed)
{
    Frame* f = push.frame;
    bool second = f[0].pass == PASS_REMAINDER;
    if (thread == 0)
    {
        InterlockedAdd(f[0].clusters[24 + (second ? 1 : 0)], 1, s_ticket);
        s_triangles = 0;
        s_sw_triangles = 0;
        s_dense_triangles = 0;
        s_levels = 0;
        s_occluded = 0;
        s_kept_software = 0;
    }
    if (thread < CULL_ITEMS)
    {
        s_drawn[thread] = 0;
        s_software[thread] = 0;
        s_rejected[thread] = 0;
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
    uint2 first = lookback_exclusive(cluster_status(f), ticket, drawn, thread);
    uint below = (1u << lane) - 1u;
    uint index = first.x + before.x + countbits(s_drawn[local] & below);
    uint sw_index = first.y + before.y + countbits(s_software[local] & below); // software clusters before this one
    uint hw_index = index - sw_index; // for a hardware-drawn cluster, the hardware-drawn ones before it
    // Pass 1 lists the clusters it leaves to pass 2 in the same order, through a run of status
    // words of its own after the two passes' (issue #92). Without a previous pyramid it draws
    // nothing and leaves everything: pass 2 then walks the work items again, as it would.
    bool lists = f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_PREV_PYRAMID) != 0;
    uint rejected = 0;     // this workgroup's
    uint rejects_first = 0; // the workgroups' before it
    uint reject_index = 0; // this thread's slot in `Frame::rejects`
    if (lists)
    {
        uint rejected_before = 0;
        for (uint k = 0; k < CULL_ITEMS; k++)
        {
            uint n = countbits(s_rejected[k]);
            rejected_before += k < local ? n : 0u;
            rejected += n;
        }
        GroupMemoryBarrierWithGroupSync(); // every thread is done with the first look-back's shared words
        rejects_first = lookback_exclusive(f[0].cluster_lookback + 2 * f[0].cluster_groups, ticket, uint2(rejected, 0), thread).x;
        reject_index = rejects_first + rejected_before + countbits(s_rejected[local] & below);
    }
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
        bool knows = false;
        if (ends_here)
        {
            listed = uint2(capacity, first.y + s_kept_software);
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
            uint hardware = listed.x - listed.y;
            write_grid(args, hardware, hardware);
            write_grid(args + 4, listed.y, listed.y);
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
    f[0].raster[hw_index] = slot;
    if (f[0].draws != nullptr)
    {
        // VkDrawIndexedIndirectCommand: the cluster's triangles straight from its page (the
        // pool is the index buffer, one-byte indices), its first vertex in the pool as the
        // vertex offset, one instance, and its list slot as the first instance.
        uint* draw = f[0].draws + hw_index * 5;
        draw[0] = v.triangle_count * 3;
        draw[1] = 1;
        draw[2] = v.payload + v.vertex_count * PAGED_VERTEX_BYTES;
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
// stride 1; a workgroup shares a large instance's, `append_instance_work`).
uint instance_items(Frame* f, uint instance_index, uint base, bool write, uint tag, uint lane = 0, uint stride = 1)
{
    Instance* inst = f[0].instances + instance_index;
    Mesh* mesh = f[0].meshes + inst[0].mesh;
    float scale = length(inst[0].model[0].xyz);
    float dist = length(inst[0].center - f[0].camera_pos);
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
groupshared uint2 s_heavy[64]; // (tagged instance, first slot) of the workgroup's instances with many work items
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
// each group of an instance is appended at most once.
void append_instance_work(Frame* f, uint64_t* status, uint ticket, uint groups, uint thread,
                          uint instance_index, bool visible, uint tag, bool second)
{
    uint count = 0;
    uint roots = 0;
    if (visible)
    {
        Instance* inst = f[0].instances + instance_index;
        Mesh* mesh = f[0].meshes + inst[0].mesh;
        float dist = length(inst[0].center - f[0].camera_pos);
        roots = instance_roots(f, mesh, length(inst[0].model[0].xyz), dist);
        count = roots > 0 ? 0u : instance_items(f, instance_index, 0, false, 0u);
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
    if (count > HEAVY_ITEMS)
    {
        uint k;
        InterlockedAdd(s_heavy_count, 1, k);
        s_heavy[k] = uint2(instance_index | tag, slot); // the tag is 0 or INSTANCE_HIDDEN_BIT
    }
    else if (count > 0)
        instance_items(f, instance_index, slot, true, tag);
    GroupMemoryBarrierWithGroupSync();
    for (uint h = 0; h < s_heavy_count; h++)
    {
        uint tagged = s_heavy[h].x;
        instance_items(f, tagged & ~INSTANCE_HIDDEN_BIT, s_heavy[h].y, true, tagged & INSTANCE_HIDDEN_BIT, thread, 64);
    }
    if (roots > 0)
    {
        Mesh* mesh = f[0].meshes + f[0].instances[instance_index].mesh;
        for (uint k = 0; k < roots; k++)
        {
            uint entry = base.y + first.y + s_root_counts[thread] + k;
            if (entry < f[0].work_capacity)
                f[0].roots[entry] = uint2(instance_index | tag, mesh[0].roots[k]);
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
    if (in_cell && instance_index < f[0].instance_count)
    {
        Instance* inst = f[0].instances + instance_index;
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        visible = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
    }
    uint in_frustum = WaveActiveCountBits(visible);
    if (WaveIsFirstLane() && in_frustum > 0)
        InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], in_frustum);
    // With FLAG_HIDDEN_INSTANCES, the instances the previous pyramid hides are counted (the
    // renderer's auto mode decides from them); with FLAG_INSTANCE_OCCLUSION they are also
    // deferred.
    bool hidden = false;
    if (visible && f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_HIDDEN_INSTANCES) != 0)
    {
        Instance* inst = f[0].instances + instance_index;
        hidden = !visible_last_frame(f, inst[0].center, inst[0].radius);
    }
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
        float4 sphere = f[0].cells[cell];
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        bool in_view = !frustum || sphere_in_frustum(f, sphere.xyz, sphere.w);
        if (in_view && f[0].pass == PASS_PREVIOUSLY_VISIBLE && (f[0].flags & FLAG_INSTANCE_OCCLUSION) != 0)
            deferred = !visible_last_frame(f, sphere.xyz, sphere.w);
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
        float4 sphere = f[0].cells[cell];
        open = (f[0].flags & FLAG_SHOW_CULLED) != 0 || occlusion_visible(f, sphere.xyz, sphere.w);
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
    float4* cells;
    uint count;
    uint pad;
};

[[vk::push_constant]]
CellBoundsPush cell_bounds_push;

groupshared float3 s_bounds_min[64];
groupshared float3 s_bounds_max[64];
groupshared float s_bounds_reach[64];

// Once, after the instances are written: a workgroup per cell of 64 instances, the sphere
// around their spheres (centred on the box around them; a tree reduction, so the same table
// gives the same bits).
[shader("compute")]
[numthreads(64, 1, 1)]
void cell_bounds_main(uint3 group : SV_GroupID, uint thread : SV_GroupThreadID)
{
    CellBoundsPush p = cell_bounds_push;
    uint index = group.x * 64 + thread;
    bool valid = index < p.count;
    float3 c = 0.0;
    float r = 0.0;
    if (valid)
    {
        c = p.instances[index].center;
        r = p.instances[index].radius;
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
        p.cells[group.x] = float4(centre, s_bounds_reach[0]);
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
            Instance* inst = f[0].instances + instance_index;
            bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
            candidate = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
        }
    }
    bool visible = false;
    bool hidden = false;
    if (candidate)
    {
        Instance* inst = f[0].instances + instance_index;
        visible = occlusion_visible(f, inst[0].center, inst[0].radius);
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
    if (index >= pass_args(f)[3])
    {
        SetMeshOutputCounts(0, 0); // the partial last row of the grid
        return;
    }
    uint slot = f[0].raster[index];
    uint2 record = f[0].visible[slot];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Instance inst = f[0].instances[record.x];

    SetMeshOutputCounts(m.vertex_count, m.triangle_count);
    uint base = payload_base(f, m);

    if (thread < m.vertex_count)
    {
        PagedVertex v = paged_vertex(f, base, thread);
        float4 world = mul(inst.model, float4(v.position, 1));
        verts[thread].position = mul(f[0].view_proj, world);
    }

    for (uint t = thread; t < m.triangle_count; t += MESH_GROUP_SIZE)
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
    PagedVertex v = paged_vertex(f, 0, vertex_index);
    float4 world = mul(inst.model, float4(v.position, 1));
    FallbackVertex o;
    o.position = mul(f[0].view_proj, world);
    o.id_base = slot << VISIBILITY_TRIANGLE_BITS;
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
    float3 center = mul(inst.model, float4(m.center, 1)).xyz;
    float3 c = mul(f[0].cull_view, float4(center, 1)).xyz;
    float2 size = float2(float(f[0].target_width), float(f[0].target_height));
    float4 rect = float4(0.0, 0.0, size); // pixels; the whole target should the projection fail
    float4 aabb;
    if (project_sphere(c, m.radius * length(inst.model[0].xyz), f[0].znear, f[0].p00, f[0].p11, aabb))
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
        PagedVertex v = paged_vertex(f, base, thread);
        float4 world = mul(inst.model, float4(v.position, 1));
        float4 clip = mul(f[0].view_proj, world);
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

// The sky reflected by a dielectric (issue #49), in the resolve's units (a white Lambertian
// surface facing the sun): the sky-view table in the mirror direction on a smooth surface,
// fading into the sky's irradiance `irradiance` as it gets rough (the table has no blurred
// levels), weighted by Schlick's Fresnel with 4 % at normal incidence and the roughness
// bounding its rise at grazing angles. GTAO's visibility `ao` (or -1) occludes it as
// Lagarde and de Rousiers do (Moving Frostbite to PBR, 2014). `fresnel` is the share the
// surface reflects, which its diffuse part loses. `sky_share` scales the table's light in the
// mirror direction `r`: the share of it the probes find there (issue #68), else 1.
float3 sky_reflection(float4* sky, float3 irradiance, float3 n, float3 view, float3 r, float3 sky_share, float specular_power, float ao, float f0, out float fresnel)
{
    float roughness = sqrt(sqrt(2.0 / (specular_power + 2.0))); // RenderLayer::roughness_for_power, inverted
    float nv = saturate(dot(n, view));
    fresnel = f0 + (max(1.0 - roughness, f0) - f0) * pow(1.0 - nv, 5.0);
    float4 up_table = sky[SH_COEFFICIENTS];
    float3 forward = sky[SH_COEFFICIENTS + 1].xyz;
    // Luminance per unit of sun illuminance, times π: the resolve's units.
    float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, forward, r) * 3.14159265 * sky_share;
    float blend = saturate(2.0 * roughness * roughness);
    float3 environment = lerp(mirror, irradiance, blend);
    float occlusion = ao >= 0.0 ? saturate(pow(nv + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao) : 1.0;
    // For the mirror ray of the smooth rows (issue #52): its direction, and the weight of what
    // it meets in place of `mirror`.
    g_reflection_request = float4(r, fresnel * occlusion * (1.0 - blend));
    return fresnel * occlusion * environment;
}

float3 sun_light(Frame* f, float3 albedo, float3 n, float3 object_position, float specular_power, float specular_weight, float shadow, float4* sky, float ao, float3 view, float reflectance, float3 world, ProbeField* probes, bool traced, out float ndl)
{
    float3 light = normalize(f[0].sun_dir);
    // The sun is far, so a half vector from the light and the camera-to-surface direction
    // approximated by the object's outward normal is close enough for a highlight this soft.
    ndl = dot(n, light);
    float diffuse = max(ndl, 0.0) * shadow;
    float3 highlight_view = normalize(f[0].camera_pos - (object_position + 0.0)); // object space ≈ world offset
    float3 half_vector = normalize(light + highlight_view);
    float specular = pow(max(dot(n, half_vector), 0.0), specular_power) * specular_weight * (ndl > 0.0 ? shadow : 0.0);
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
        if (probes != nullptr && reflects && !traced)
        {
            // The sky's reflection dims as the probes' light does (issue #68): their light for
            // a normal along the mirror direction over the open sky's there, per channel, up
            // to 1. Lazarov rescaled reflection probes so at the normal (Black Ops II,
            // SIGGRAPH 2013); along the mirror, a street's ground sees the buildings, not the
            // sky above it. A mirror ray sees for itself.
            float3 open_r = sh_irradiance(sky, r);
            float3 toward_r;
            irradiance = probe_irradiance_mirror(probes, world, n, view, irradiance, r, open_r, toward_r);
            sky_share = saturate(toward_r / max(open_r, 1e-4));
        }
        else if (probes != nullptr)
            irradiance = probe_irradiance(probes, world, n, view, irradiance);
        float3 ambient = ao >= 0.0 ? irradiance * occlusion_bounce(ao, albedo) : irradiance;
        if (!reflects)
            return albedo * (diffuse * sun + ambient) + specular * sun;
        float fresnel;
        float3 reflection = sky_reflection(sky, irradiance, n, view, r, sky_share, specular_power, ao, reflectance, fresnel);
        return albedo * (diffuse * sun + ambient) * (1.0 - fresnel) + specular * sun + reflection;
    }
    // Space: hard sun, faint fill from the nebula side, a little wrap so shadows are not black.
    float wrap = saturate((ndl + 0.3) / 1.3) * 0.08;
    float3 fill = float3(0.10, 0.12, 0.18) * 0.25 * (0.5 + 0.5 * n.y);
    // Both stand for light from around the surface: the occlusion takes its share (issue #55).
    if (ao >= 0.0)
        return albedo * (diffuse * sun + (wrap + fill) * occlusion_bounce(ao, albedo)) + specular * sun;
    return albedo * (diffuse * sun + wrap + fill) + specular * sun;
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

static const uint MATERIAL_FLAG_HEX_TILING = 1; // Material::flags, forge_render::material

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
struct Surface
{
    float3 n;               // world-space normal, interpolated
    float3 view;            // unit, world space, from the surface towards the camera (issue #49)
    float3 transmitted;     // the sunlight crossing the ice to this point and leaving towards the camera, per channel (issues #59, #61), else 0
    float3 object_normal;   // object-space normal, interpolated
    float3 object_position; // object-space position
    float3 dpdx;            // its change per pixel to the right and down
    float3 dpdy;
    float4x4 model;
    uint instance_id;
    uint meshlet_index;
    uint lod_level;
    float shadow;           // 1 in the sun, 0 where a shadow ray met something (issue #45)
    float4* sky;            // the sky's irradiance coefficients (sh.slang), null in space (issue #47)
    float ao;               // how much of the sky the surface sees (issue #48); -1: no occlusion computed
    float3 world;           // world-space position
    ProbeField* probes;     // the diffuse light's probes (probes.slang, issue #53), or null: the sky's irradiance unoccluded
    bool traced;            // a mirror ray replaces the sky's reflection where it meets something (issues #50, #52)
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
        float3 p = s.object_position * mat.inv_texture_scale;
        float3 dpdx = s.dpdx * mat.inv_texture_scale;
        float3 dpdy = s.dpdy * mat.inv_texture_scale;
        if ((mat.flags & MATERIAL_FLAG_HEX_TILING) != 0)
        {
            // Each instance its own place in the texture (a few dozen repeats away), so the
            // objects of one mesh stop sharing a pattern; then the hex tiles (issue #66).
            p += hash_color(s.instance_id ^ 0x9E3779B9u) * 61.0;
            float broad = 0.65 * value_noise3(p * 0.137) + 0.35 * value_noise3(p * 0.41 + 17.0);
            float3 texel, bent;
            hex_triplanar(mat, s.object_normal, w, p, dpdx, dpdy, texel, bent);
            if (mat.albedo_texture != TEXTURE_NONE)
                albedo *= texel * (0.7 + 0.6 * broad);
            if (mat.normal_texture != TEXTURE_NONE)
                n = normalize(mul(s.model, float4(bent, 0)).xyz);
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
            n = normalize(mul(s.model, float4(bent, 0)).xyz);
        }
    }
}

float3 shade_standard(Frame* f, Material mat, Surface s)
{
    float3 albedo;
    float3 n = s.n;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
        standard_surface(mat, s, albedo, n);
    float ndl;
    return sun_light(f, albedo, n, s.object_position, mat.specular_power, mat.specular, s.shadow, s.sky, s.ao, s.view, mat.reflectance, s.world, s.probes, s.traced, ndl) + mat.emissive;
}

// Ground in layers (issue #42): the row at `row` names a layer map (one layer id per texel,
// over `1 / inv_texture_scale` metres of the object's x and z, centred on its origin); layer k
// is the standard row `row + 1 + k`. The two layers weighing most among the four texels
// around the pixel are shaded and blended by their bilinear weights.
float3 shade_layered(Frame* f, uint row, Material mat, Surface s)
{
    float3 albedo;
    float3 n = s.n;
    float specular_power = mat.specular_power;
    float specular = mat.specular;
    float3 emissive = mat.emissive;
    float reflectance = mat.reflectance;
    if (!debug_albedo(f, s.meshlet_index, s.lod_level, albedo))
    {
        Texture2D<uint> map = g_sampled_images_u32[NonUniformResourceIndex(mat.albedo_texture)];
        uint2 size;
        map.GetDimensions(size.x, size.y);
        float2 texel = (s.object_position.xz * mat.inv_texture_scale + 0.5) * float2(size) - 0.5;
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
        uint first = ids[heaviest], second = first;
        float w_first = total[heaviest], w_second = 0.0;
        [unroll]
        for (uint k = 0; k < 4; k++)
        {
            if (ids[k] != first && total[k] > w_second)
            {
                second = ids[k];
                w_second = total[k];
            }
        }
        Material a = f[0].materials[row + 1 + first];
        float3 albedo_a, n_a;
        standard_surface(a, s, albedo_a, n_a);
        albedo = albedo_a;
        n = n_a;
        specular_power = a.specular_power;
        specular = a.specular;
        reflectance = a.reflectance;
        emissive = a.emissive;
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
            emissive = lerp(a.emissive, b.emissive, blend);
        }
    }
    float ndl;
    return sun_light(f, albedo, n, s.object_position, specular_power, specular, s.shadow, s.sky, s.ao, s.view, reflectance, s.world, s.probes, s.traced, ndl) + emissive;
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
    float3 color = sun_light(f, albedo, s.n, s.object_position, mat.specular_power, mat.specular, s.shadow, s.sky, s.ao, s.view, mat.reflectance, s.world, s.probes, s.traced, ndl);
    if (any(s.transmitted > 0.0))
    {
        // The sunlight that crossed the ice (issues #59, #61), tinted by it: the sun glows
        // through the thin edges of a backlit rock.
        color += ICE_GLOW * albedo * f[0].sun_color.rgb * s.transmitted;
    }
    return color + mat.scatter * saturate(1.0 - abs(ndl)) + mat.emissive; // rim scatter
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
    uint use_background; // write `background` to empty pixels (else leave them for the sky)
    uint tile_capacity;
    float4 background;
    float4* sky;         // the sky's irradiance (sh.slang), or null: space's constant fill (issue #47)
    uint ao_image;       // sampled index of the ambient occlusion (gtao.slang, issue #48), or 0xFFFFFFFF
    uint request_image;  // storage index of the mirror rays' requests (issue #52), or 0xFFFFFFFF
    ProbeField* probes;  // the diffuse light's probes (probes.slang, issue #53), or null
};

[[vk::push_constant]]
ResolvePush resolve_push;

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

// 0 when something lies between `world` and the sun, else 1: one ray, the first hit ends it.
// With a sun disc (`Frame::sun_angular_radius`, issue #54) the ray aims at a point of it,
// uniform over the disc from `jitter` (two values in [0, 1): `noise.slang`'s, which repeat with
// TAA's jitter so TAA averages the points into a penumbra); (0, 0) is the disc's centre.
float sun_shadow(Frame* f, float3 world, float3 n, float2 jitter)
{
    float3 sun = normalize(f[0].sun_dir);
    if (dot(n, sun) <= 0.0)
        return 1.0; // facing away: the sun does not light it anyway
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
    ray.Origin = world + (n + sun) * SHADOW_BIAS;
    ray.Direction = direction;
    ray.TMin = 0.0;
    ray.TMax = 50000.0;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER | RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, ray);
    q.Proceed();
    return q.CommittedStatus() == COMMITTED_TRIANGLE_HIT ? 0.0 : 1.0;
}

// Rows at least this smooth (Blinn-Phong exponent; roughness 0.42 and below) trace their
// mirror ray (issue #50): the city's glass. Rougher rows keep the sky, blurred towards its
// irradiance (`sky_reflection`).
static const float RAY_REFLECTION_POWER = 60.0;
// How far a mirror ray travels before a hit counts, metres: the traced surfaces are cuts of
// the meshes' DAGs and may stand up to about a metre in front of the drawn ones (the towers';
// most props much less), which would otherwise hide a window behind its own facade.
static const float REFLECTION_START = 1.5;

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
    float3 face = mul(inst.model, float4(cross(p[1] - p[0], p[2] - p[0]), 0.0)).xyz;
    return dot(face, direction) > 0.0;
}

RayHit ray_hit(Frame* f, uint instance, uint primitive, float2 b, float3 origin, float3 direction, float t)
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
    float3 face = mul(inst.model, float4(cross(p[1] - p[0], p[2] - p[0]), 0.0)).xyz;
    RayHit hit;
    hit.n = dot(face, face) > 1e-20 ? normalize(face) : -direction;
    hit.back = dot(hit.n, direction) > 0.0;
    if (hit.back)
        hit.n = -hit.n; // the side the ray came from
    hit.position = origin + direction * t;
    uint row = inst.material + rs[0].sections[tri];
    Material mat = f[0].materials[row];
    if (mat.class_index == MATERIAL_CLASS_LAYERED)
    {
        // The layer under the hit, nearest texel (`shade_layered` blends two).
        Texture2D<uint> map = g_sampled_images_u32[NonUniformResourceIndex(mat.albedo_texture)];
        uint2 size;
        map.GetDimensions(size.x, size.y);
        float2 texel = (object_position.xz * mat.inv_texture_scale + 0.5) * float2(size);
        int2 at = clamp(int2(texel), int2(0, 0), int2(size) - 1);
        mat = f[0].materials[row + 1 + map.Load(int3(at, 0))];
    }
    hit.albedo = lerp(mat.color_a, mat.color_b, hash_color(inst.id).x);
    if (mat.albedo_texture != TEXTURE_NONE)
        hit.albedo *= sample_image_level(mat.albedo_texture, SAMPLER_LINEAR_REPEAT, float2(0.5, 0.5), 16.0).rgb;
    hit.emissive = mat.emissive;
    return hit;
}

// The light `hit` sends back along the ray that met it (travelling along `direction`), in the
// resolve's units: the sun through a shadow ray, and the sky's irradiance or, with probes,
// what they measured there (issue #53: the light bounces again, one bounce a frame).
float3 hit_radiance(Frame* f, float4* sky, ProbeField* probes, RayHit hit, float3 direction)
{
    float3 sun = normalize(f[0].sun_dir);
    float ndl = max(dot(hit.n, sun), 0.0);
    float shadow = ndl > 0.0 ? sun_shadow(f, hit.position, hit.n, float2(0.0)) : 0.0;
    float3 ambient = sh_irradiance(sky, hit.n);
    if (probes != nullptr)
        ambient = probe_irradiance(probes, hit.position, hit.n, -direction, ambient);
    float3 light = ndl * shadow * f[0].sun_color.rgb + ambient;
    return hit.albedo * light + hit.emissive;
}

// What the mirror ray from `origin` along `direction` meets (issue #50): the light leaving the
// cut's triangle towards the ray, in the resolve's units, with w = 1; w = 0 on a miss.
float4 trace_reflection(Frame* f, float4* sky, ProbeField* probes, float3 origin, float3 direction)
{
    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = direction;
    ray.TMin = REFLECTION_START;
    ray.TMax = 50000.0;
    RayQuery<RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, ray);
    while (q.Proceed())
    {
    }
    if (q.CommittedStatus() != COMMITTED_TRIANGLE_HIT)
        return float4(0.0);
    RayHit hit = ray_hit(f, q.CommittedInstanceID(), q.CommittedPrimitiveIndex(), q.CommittedTriangleBarycentrics(), origin, direction, q.CommittedRayT());
    return float4(hit_radiance(f, sky, probes, hit, direction), 1.0);
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
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, ray);
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
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER | RAY_FLAG_FORCE_OPAQUE> s;
    s.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, shadow);
    s.Proceed();
    if (s.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
        return float3(0.0);
    float3 to_camera = normalize(f[0].camera_pos - world);
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
        && (f[0].flags & FLAG_SHOW_GI) == 0;
}

// Shades the pixel at `pixel` whose (non-empty) visibility id is `vis`, when its material is
// of class `class_index` (a compile-time constant at every call); `SHADOWS` compiles the
// shadow ray in (devices with ray queries). Returns the class it has.
uint shade_pixel<let SHADOWS : int>(uint class_index, uint2 pixel, uint vis)
{
    Frame* f = resolve_push.frame;
    uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    uint meshlet_index = record.y & MESHLET_INDEX_MASK;
    Meshlet m = f[0].meshlets[meshlet_index];
    // The instance names its first section's row; the others follow it (issue #41).
    uint row = inst.material + triangle_section(m, tri);
    Material mat = f[0].materials[row];
    if (mat.class_index != class_index)
        return mat.class_index | (wants_mirror_ray<SHADOWS>(f, mat) ? REFLECT_BIT : 0u);
    bool culled_debug = (record.y & CULLED_BIT) != 0;
    bool software = (record.y & SOFTWARE_BIT) != 0;
    uint base = payload_base(f, m);
    PagedVertex v0 = paged_vertex(f, base, paged_index(f, base, m, tri, 0));
    PagedVertex v1 = paged_vertex(f, base, paged_index(f, base, m, tri, 1));
    PagedVertex v2 = paged_vertex(f, base, paged_index(f, base, m, tri, 2));
    float4 c0 = mul(f[0].view_proj, mul(inst.model, float4(v0.position, 1)));
    float4 c1 = mul(f[0].view_proj, mul(inst.model, float4(v1.position, 1)));
    float4 c2 = mul(f[0].view_proj, mul(inst.model, float4(v2.position, 1)));
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
    float3 o0 = decode_normal(v0.normal);
    float3 o1 = decode_normal(v1.normal);
    float3 o2 = decode_normal(v2.normal);
    float3 n0 = normalize(mul(inst.model, float4(o0, 0)).xyz);
    float3 n1 = normalize(mul(inst.model, float4(o1, 0)).xyz);
    float3 n2 = normalize(mul(inst.model, float4(o2, 0)).xyz);
    Surface s;
    s.n = normalize(b.lambda.x * n0 + b.lambda.y * n1 + b.lambda.z * n2);
    s.object_normal = normalize(b.lambda.x * o0 + b.lambda.y * o1 + b.lambda.z * o2);
    s.object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
    s.dpdx = b.ddx.x * v0.position + b.ddx.y * v1.position + b.ddx.z * v2.position;
    s.dpdy = b.ddy.x * v0.position + b.ddy.y * v1.position + b.ddy.z * v2.position;
    s.model = inst.model;
    s.instance_id = inst.id;
    s.meshlet_index = meshlet_index;
    s.lod_level = m.lod_level;
    s.shadow = 1.0;
    s.sky = resolve_push.sky;
    s.ao = resolve_push.ao_image != 0xFFFFFFFFu
        ? g_sampled_images_r[NonUniformResourceIndex(resolve_push.ao_image)].Load(int3(pixel, 0))
        : -1.0;
    s.view = s.sky != nullptr ? normalize(f[0].camera_pos - mul(inst.model, float4(s.object_position, 1.0)).xyz) : float3(0.0, 1.0, 0.0);
    s.world = mul(inst.model, float4(s.object_position, 1.0)).xyz;
    s.probes = resolve_push.probes; // read under a sky only
    bool mirror_ray = wants_mirror_ray<SHADOWS>(f, mat);
    s.traced = mirror_ray && resolve_push.request_image != 0xFFFFFFFFu;
    if (SHADOWS != 0 && f[0].tlas != 0 && (f[0].flags & FLAG_SHADOWS) != 0)
        s.shadow = sun_shadow(f, mul(inst.model, float4(s.object_position, 1.0)).xyz, s.n, sun_disc_point(pixel, f[0].noise_frame));
    s.transmitted = float3(0.0);
    if (SHADOWS != 0 && class_index == MATERIAL_CLASS_ICE && f[0].tlas != 0 && (f[0].flags & FLAG_TRANSLUCENCY) != 0)
        s.transmitted = ice_transmission(f, record.x, mul(inst.model, float4(s.object_position, 1.0)).xyz, mat.scattering);
    // The classes work in units of a white Lambertian surface facing the sun; the frame
    // stores luminance (cd/m²) times the exposure.
    float3 color = class_index == MATERIAL_CLASS_ICE ? shade_ice(f, mat, s)
        : class_index == MATERIAL_CLASS_LAYERED ? shade_layered(f, row, mat, s)
        : shade_standard(f, mat, s);
    if (culled_debug)
        color = lerp(color, float3(1.0, 0.0, 0.0), 0.8); // a culling error (FLAG_SHOW_CULLED)
    if (software && (f[0].flags & FLAG_SHOW_RASTER) != 0)
        color = lerp(color, float3(0.1, 0.9, 0.3), 0.6); // the software rasteriser's pixels
    if ((f[0].flags & FLAG_SHOW_AO) != 0 && s.ao >= 0.0)
        color = float3(s.ao * 0.25); // the occlusion, as a light grey surface in the sun
    if ((f[0].flags & FLAG_SHOW_GI) != 0 && s.sky != nullptr)
    {
        // The light the sky and the city send a white surface, without the sun's own and the
        // occlusion: the probes' where they reach, the open sky's elsewhere.
        color = sh_irradiance(s.sky, s.n);
        if (s.probes != nullptr)
            color = probe_irradiance(s.probes, s.world, s.n, s.view, color);
    }
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
        else if (resolve_push.use_background != 0)
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

// The mirror rays of the smooth rows (issue #52), in a pass of their own over the tiles the
// standard pass listed, so the resolve does not carry the ray's code and registers. The
// resolve shaded the pixel with the sky in the mirror direction and stored that direction and
// the weight W of what a ray meets there instead. This pass traces the ray and, on a hit, adds
// W × (hit − sky): the same sum as tracing inside the resolve.
[shader("compute")]
[numthreads(8, 8, 1)]
void reflections_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    uint tile_index = group.y * TILE_GROUPS_X + group.x;
    if (tile_index >= resolve_push.class_args[REFLECTION_LIST * 4 + 3])
        return;
    uint tile = resolve_push.tiles[REFLECTION_LIST * resolve_push.tile_capacity + tile_index];
    uint2 pixel = uint2(tile & 0xFFFFu, tile >> 16) * 8 + thread.xy;
    if (pixel.x >= resolve_push.width || pixel.y >= resolve_push.height)
        return;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY)
        return;
    Frame* f = resolve_push.frame;
    uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    uint2 record = f[0].visible[slot];
    Instance inst = f[0].instances[record.x];
    Meshlet m = f[0].meshlets[record.y & MESHLET_INDEX_MASK];
    Material mat = f[0].materials[inst.material + triangle_section(m, tri)];
    if (!wants_mirror_ray<1>(f, mat))
        return;
    float4 request = g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.request_image)][pixel];
    if (request.w <= 0.0)
        return;
    // The surface at the pixel centre, as `shade_pixel` rebuilt it: its world position and
    // interpolated normal start the ray.
    uint base = payload_base(f, m);
    PagedVertex v0 = paged_vertex(f, base, paged_index(f, base, m, tri, 0));
    PagedVertex v1 = paged_vertex(f, base, paged_index(f, base, m, tri, 1));
    PagedVertex v2 = paged_vertex(f, base, paged_index(f, base, m, tri, 2));
    float4 c0 = mul(f[0].view_proj, mul(inst.model, float4(v0.position, 1)));
    float4 c1 = mul(f[0].view_proj, mul(inst.model, float4(v1.position, 1)));
    float4 c2 = mul(f[0].view_proj, mul(inst.model, float4(v2.position, 1)));
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
    float3 n = normalize(b.lambda.x * normalize(mul(inst.model, float4(decode_normal(v0.normal), 0)).xyz)
        + b.lambda.y * normalize(mul(inst.model, float4(decode_normal(v1.normal), 0)).xyz)
        + b.lambda.z * normalize(mul(inst.model, float4(decode_normal(v2.normal), 0)).xyz));
    float3 object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
    float3 world = mul(inst.model, float4(object_position, 1.0)).xyz;
    float4* sky = resolve_push.sky;
    float4 hit = trace_reflection(f, sky, resolve_push.probes, world + n * 0.05, request.xyz);
    if (hit.w <= 0.0)
        return;
    float4 up_table = sky[SH_COEFFICIENTS];
    float3 mirror = sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, sky[SH_COEFFICIENTS + 1].xyz, request.xyz) * 3.14159265;
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)];
    float3 added = request.w * (hit.xyz - mirror) * (f[0].sun_illuminance * f[0].exposure / 3.14159265);
    color[pixel] = float4(color[pixel].rgb + added, 1.0);
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
    uint pad;
};

[[vk::push_constant]]
ProbeTracePush probe_push;

[shader("compute")]
[numthreads(PROBE_TRACE_GROUP, 1, 1)]
void probe_trace_main(uint3 group : SV_GroupID, uint thread : SV_GroupThreadID)
{
    ProbeField* field = probe_push.field;
    uint probe = group.z * 65535u + group.y;
    uint ray = group.x * PROBE_TRACE_GROUP + thread;
    if (probe >= probe_push.probe_count || ray >= field[0].rays)
        return;
    ProbeSlot slot = probe_slot(field, probe);
    float4 data = slot.fresh ? float4(0.0, 0.0, 0.0, PROBE_ACTIVE) : field[0].probe_data[probe];
    // A settled probe (probes.slang) needs no fixed rays; an inactive one no others.
    bool fixed = ray < PROBE_FIXED_RAYS;
    bool settled = !slot.fresh && probe_age(data.w) >= PROBE_SETTLE_UPDATES;
    if (fixed ? settled : probe_state(data.w) == PROBE_INACTIVE)
        return;
    Frame* f = probe_push.frame;
    float4* sky = probe_push.sky;
    float3 origin = slot.position + data.xyz;
    float3 direction = probe_ray_direction(field, ray);
    RayDesc desc;
    desc.Origin = origin;
    desc.Direction = direction;
    desc.TMin = 0.0;
    desc.TMax = fixed ? PROBE_FIXED_REACH * slot.spacing : PROBE_MISS_DISTANCE;
    RayQuery<RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, desc);
    while (q.Proceed())
    {
    }
    float3 radiance = float3(0.0);
    float distance = PROBE_MISS_DISTANCE;
    if (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
    {
        float t = q.CommittedRayT();
        if (fixed)
        {
            distance = ray_back_face(f, q.CommittedInstanceID(), q.CommittedPrimitiveIndex(), direction) ? -t : t;
        }
        else
        {
            RayHit hit = ray_hit(f, q.CommittedInstanceID(), q.CommittedPrimitiveIndex(), q.CommittedTriangleBarycentrics(), origin, direction, t);
            distance = hit.back ? -t : t;
            if (!hit.back)
                radiance = hit_radiance(f, sky, field, hit, direction);
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

// ------------------------------------------------------------ ray tracing ---

// The top-level acceleration structure's records (issue #45, crates/forge-render/src/raytrace.rs):
// a thread per instance writes its VkAccelerationStructureInstanceKHR, 64 bytes: the model's
// top three rows, the instance index (24 bits) and a full mask, no SBT offset, back faces
// traced too, and the address of its mesh's bottom-level structure.
struct TlasPush
{
    Instance* instances;
    uint64_t* blas;   // per mesh
    uint* records;    // 16 words per instance
    uint count;
    uint pad;
};

[[vk::push_constant]]
TlasPush tlas_push;

[shader("compute")]
[numthreads(64, 1, 1)]
void tlas_instances_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= tlas_push.count)
        return;
    Instance inst = tlas_push.instances[id.x];
    uint* r = tlas_push.records + id.x * 16;
    [unroll]
    for (uint row = 0; row < 3; row++)
    {
        [unroll]
        for (uint column = 0; column < 4; column++)
            r[row * 4 + column] = asuint(inst.model[row][column]);
    }
    r[12] = (id.x & 0xFFFFFFu) | (0xFFu << 24);
    r[13] = 0x1u << 24; // VK_GEOMETRY_INSTANCE_TRIANGLE_FACING_CULL_DISABLE_BIT
    uint64_t blas = tlas_push.blas[inst.mesh];
    r[14] = uint(blas);
    r[15] = uint(blas >> 32);
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
    public uint pad;
    public uint building_meshes[16];
    public uint rock_meshes[8];
    public float3 origin;       // the scene's offset from the world's origin (issue #93)
    public float pad2;
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
    // The scene's offset from the world's origin (issue #93): the ground was sampled in the
    // scene's frame; the world position is rounded to an f32 here, whose spacing grows with the
    // offset (1 m at 10 000 km).
    float3 world = position + p[0].origin;
    float cy = cos(yaw), sy = sin(yaw), ct = cos(tilt), st = sin(tilt);
    // R = Ry(yaw) × Rx(tilt), rows.
    float3 r0 = float3(cy, sy * st, sy * ct);
    float3 r1 = float3(0.0, ct, -st);
    float3 r2 = float3(-sy, cy * st, cy * ct);
    float4x4 model = float4x4(
        float4(r0 * scale, world.x),
        float4(r1 * scale, world.y),
        float4(r2 * scale, world.z),
        float4(0.0, 0.0, 0.0, 1.0));
    Mesh* m = p[0].meshes + mesh;
    Instance inst;
    inst.model = model;
    inst.center = mul(model, float4(m[0].center, 1.0)).xyz;
    inst.radius = m[0].radius * scale;
    inst.mesh = mesh;
    inst.id = p[0].first + slot;
    inst.material = m[0].material;
    inst.pad = 0u;
    p[0].instances[p[0].first + slot] = inst;
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
    // Rocks: uniform over the four bands of hills around the city (north and south span the
    // whole side, east and west the city's depth), any turn, a slight tilt, sunk a little.
    float4 r = random4(p, slot, 4u);
    float4 q = random4(p, slot, 5u);
    uint mesh = p[0].rock_meshes[pcg4d(uint4(slot, p[0].seed, 6u, 0u)).x % p[0].rock_mesh_count];
    float h = p[0].half_size, c = p[0].city_half;
    float ns = 2.0 * h * (h - c), ew = 2.0 * c * (h - c); // areas of one north/south and one east/west band
    float pick = r.x * 2.0 * (ns + ew);
    float2 at;
    if (pick < 2.0 * ns)
        at = float2(lerp(-h, h, r.y), lerp(c, h, r.z) * (pick < ns ? 1.0 : -1.0));
    else
        at = float2(lerp(c, h, r.y) * (pick < 2.0 * ns + ew ? 1.0 : -1.0), lerp(-c, c, r.z));
    float scale = lerp(0.5, 1.6, q.x * q.x);
    float radius = p[0].meshes[mesh].radius * scale;
    place(p, slot, mesh, float3(at.x, ground(p, at.x, at.y) - 0.15 * radius, at.y), r.w * 6.2831853, (q.y - 0.5) * 0.4, scale);
}
