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

// ---------------------------------------------------------------- shared data ---

public struct Vertex
{
    public float3 position;
    public float pad0;
    public float3 normal;
    public float pad1;
};

// One cluster of at most 64 vertices / 124 triangles (meshoptimizer output).
public struct Meshlet
{
    public float3 center;      // bounding sphere, object space
    public float radius;
    public float3 cone_apex;   // normal cone, object space (meshoptimizer)
    public float cone_cutoff;
    public float3 cone_axis;
    public uint vertex_offset;   // into `meshlet_vertices`
    public uint triangle_offset; // into `meshlet_triangles`, in bytes (3 per triangle)
    public uint vertex_count;
    public uint triangle_count;
    public uint pad;
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
    public uint pad2;
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
};

public struct Instance
{
    public float4x4 model;
    public float3 center; // world-space bounding sphere of the instance
    public float radius;
    public uint mesh;
    public uint id;
    public uint2 pad;
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
public static const uint STAT_WORK_OVERFLOW = 11;   // of those, dropped because the work list was full

// The visibility buffer: every drawn pixel holds `visible_slot << 7 | triangle`, where
// `visible_slot` indexes the frame's visible-cluster list (`Frame::visible`, filled by the
// cluster cull) and `triangle` the cluster's triangle; `VISIBILITY_EMPTY` is the clear value.
// The software rasteriser keeps `depth << 32 | id` per pixel in `Push::vis64` (the depth's
// float bits: reversed-Z, nearer is larger; 0 is empty) until the merge.
public static const uint VISIBILITY_EMPTY = 0xFFFFFFFFu;
public static const uint VISIBILITY_TRIANGLE_BITS = 7;

public struct Frame
{
    public float4x4 view_proj;      // for drawing
    public float4x4 cull_view_proj; // for culling (frozen when FLAG_FREEZE_CULLING)
    public float4x4 cull_view;      // view matrix used for culling
    public float4x4 prev_cull_view; // the previous frame's (with FLAG_PREV_PYRAMID)
    public float4 planes[6];        // frustum planes of `cull_view_proj`, inward normals
    public float3 camera_pos;       // camera used for culling
    public uint instance_count;
    public uint work_capacity;      // slots in `work`
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
    public Vertex* vertices;
    public Meshlet* meshlets;
    public uint* meshlet_vertices;
    public uint* meshlet_triangles; // bytes packed four per uint
    public Mesh* meshes;
    public Instance* instances;
    public uint* stats;
    public uint64_t* cluster_lookback; // status words of the cluster culls' ordered appends: `cluster_groups` per pass
    public uint cluster_groups;     // cluster-cull workgroups `work_capacity` items need
    public uint prev_hzb_image;     // sampled image index of the previous frame's pyramid
    public uint2* work;             // (instance, local group) per work item, built by `cull_main`
    public uint* indirect;          // the cluster cull's grid (x, y, 1, work item count), then the instance cull's ticket
    public uint2* visible;          // (instance, meshlet | flag bits) per drawn cluster, in cull order: pass 1, then pass 2
    public uint visible_capacity;   // slots in `visible`
    public float exposure;          // pre-exposure of the frame (crates/forge-render/src/exposure.rs)
    public float sun_illuminance;   // lux; the rocks return albedo × E / π
    public uint pad_exposure;
    public uint* clusters;          // per pass (pass 1 or the single pass at 0, pass 2 at 12): the hardware draw grid (x, y, 1) and its cluster count, the software raster grid and its count, the merge's VkDrawIndirectCommand; then the culls' tickets (24, 25)
    public uint* draws;             // fallback: a VkDrawIndexedIndirectCommand per hardware-drawn cluster of the pass; null on the mesh path
    public uint64_t* lookback;      // status words of the instance cull's ordered appends
    public uint* raster;            // per pass, the list slots of its clusters: hardware-drawn from the front, software-rasterised from the back
    public uint target_width;       // pixels
    public uint target_height;
    public float sw_raster_area;    // a cluster whose bounding rectangle holds fewer pixels than this per triangle is dense (rasterised in compute with FLAG_SW_RASTER)
    public uint pad_raster;
    public float2 prev_draw_jitter; // the previous frame's `draw_jitter`
    public float prev_p00;          // the previous culling camera's projection scales
    public float prev_p11;
};

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
// this accepts, and pass 2 asks the same question on the same inputs to skip it. The answer
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
bool lod_selected(Frame* f, Meshlet m, float4x4 model, float scale)
{
    if ((f[0].flags & FLAG_LOD) == 0)
        return m.lod_level == 0;
    float3 self_center = mul(model, float4(m.self_center, 1)).xyz;
    float self_px = lod_error_pixels(f, self_center, m.self_radius * scale, m.self_error * scale);
    if (self_px > f[0].lod_threshold)
        return false;
    if (isinf(m.parent_error))
        return true;
    float3 parent_center = mul(model, float4(m.parent_center, 1)).xyz;
    float parent_px = lod_error_pixels(f, parent_center, m.parent_radius * scale, m.parent_error * scale);
    return parent_px > f[0].lod_threshold;
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
bool group_possible(Frame* f, Mesh* mesh, Instance* inst, uint local_group, float scale)
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
    for (uint l = l0; l <= l1; l++)
    {
        float self_far = dist + mesh[0].self_reach_max[l] * scale;
        float parent_near = max(dist - mesh[0].parent_reach_max[l] * scale, f[0].znear);
        float self_min_px = mesh[0].self_error_min[l] * scale * k / self_far;
        float parent_max_px = mesh[0].parent_error_max[l] * scale * k / parent_near;
        if (self_min_px <= t && parent_max_px > t)
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
    bool culled_debug; // FLAG_SHOW_CULLED: drawn although culling rejected it
    bool dense;        // drawn, and dense enough for the software rasteriser (`dense_cluster`)
    bool software;     // dense, and rasterised in compute
    uint instance;
    uint meshlet;      // global meshlet index
    uint triangle_count;
    uint triangle_offset;
    uint vertex_offset;
    uint lod_level;
};

// Largest cluster the software rasteriser takes, in pixels across its bounding rectangle
// (forge_render::meshlet::SW_RASTER_MAX_PX): the rectangle bounds every triangle, which
// keeps the rasteriser's integer edge functions in range (`SW_RASTER_MAX_EXTENT`).
static const float SW_RASTER_MAX_PX = 64.0;

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

// Culls cluster `thread` of work item `item`: LOD selection, frustum and normal cone; with
// occlusion, pass 1 keeps what the previous frame showed and pass 2 what this frame's pyramid
// does not hide among the rest.
ClusterVerdict cull_cluster(Frame* f, uint item, uint thread)
{
    ClusterVerdict v = {};
    uint pass = f[0].pass;
    uint2 work = f[0].work[item]; // written by cull_main: (instance, local group)
    v.instance = work.x;
    uint local_group = work.y;
    Instance* inst = f[0].instances + v.instance;
    Mesh* mesh = f[0].meshes + inst[0].mesh;
    uint local_meshlet = local_group * TASK_GROUP_SIZE + thread;
    bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
    bool show_culled = (f[0].flags & FLAG_SHOW_CULLED) != 0;
    bool instance_visible = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
    // Uniform scale is assumed for the bounding spheres: take the length of the first column.
    float scale = length(inst[0].model[0].xyz);
    // Whole-group rejection from the per-level tables before any cluster is read.
    if (!instance_visible || !group_possible(f, mesh, inst, local_group, scale)
        || local_meshlet >= mesh[0].meshlet_count)
        return v;
    v.meshlet = mesh[0].meshlet_offset + local_meshlet;
    Meshlet m = f[0].meshlets[v.meshlet];
    v.triangle_count = m.triangle_count;
    v.triangle_offset = m.triangle_offset;
    v.vertex_offset = m.vertex_offset;
    v.lod_level = m.lod_level;
    float3 center = mul(inst[0].model, float4(m.center, 1)).xyz;
    float world_radius = m.radius * scale;
    bool visible = lod_selected(f, m, inst[0].model, scale);
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
    }
    else if (pass == PASS_REMAINDER)
    {
        // Pass 1 drew what the previous frame showed; the rest faces this frame's pyramid.
        if (visible && !visible_last_frame(f, center, world_radius))
        {
            v.draw = occlusion_visible(f, center, world_radius);
            v.occluded = !v.draw;
            if (v.occluded && show_culled)
            {
                v.culled_debug = true;
                v.draw = true;
            }
        }
    }
    else
    {
        v.draw = visible;
    }
    v.dense = v.draw && dense_cluster(f, center, world_radius, m.triangle_count);
    v.software = v.dense && (f[0].flags & FLAG_SW_RASTER) != 0;
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
groupshared uint s_kept_software;        // where the list ends: software clusters still kept

// One workgroup per CULL_ITEMS work items: cull their clusters and append the survivors to
// this pass's range of the visible-cluster list in item order, then lane order, and their
// slots to the pass's raster lists (hardware-drawn from the front, software-rasterised from
// the back, each in the same order); on the fallback path also one indexed draw per
// hardware-drawn survivor. The mesh shader and the software rasteriser read the raster
// lists, the fallback's draws carry the slots. The grids are written by the last workgroup,
// or, when the list is full, by the one it ends in. A full list drops what does not fit and
// counts it, so no draw reads past the end.
[shader("compute")]
[numthreads(TASK_GROUP_SIZE * CULL_ITEMS, 1, 1)]
void cluster_cull_main(uint thread : SV_GroupThreadID)
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
    }
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint items = f[0].indirect[3];
    if (ticket * CULL_ITEMS >= items)
        return; // the rest of the grid's last row: the whole workgroup leaves
    uint local = thread / TASK_GROUP_SIZE;
    uint lane = thread % TASK_GROUP_SIZE;
    uint item = ticket * CULL_ITEMS + local;
    ClusterVerdict v = {};
    if (item < items)
        v = cull_cluster(f, item, lane);
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
        }
        if (knows)
        {
            uint hardware = listed.x - listed.y;
            write_grid(args, hardware, hardware);
            write_grid(args + 4, listed.y, listed.y);
            args[8] = listed.y > 0 ? 3u : 0u; // the merge: a full-screen triangle, or nothing
            InterlockedAdd(f[0].stats[second ? STAT_MESHLETS_PASS2 : STAT_MESHLETS_PASS1], listed.x);
            InterlockedAdd(f[0].stats[STAT_SW_CLUSTERS], listed.y);
        }
    }
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
        // VkDrawIndexedIndirectCommand: the cluster's triangles straight from the cooked
        // one-byte lists (the index buffer), its window of `meshlet_vertices` as the vertex
        // offset, one instance, and its list slot as the first instance.
        uint* draw = f[0].draws + hw_index * 5;
        draw[0] = v.triangle_count * 3;
        draw[1] = 1;
        draw[2] = v.triangle_offset;
        draw[3] = v.vertex_offset;
        draw[4] = slot;
    }
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
    return self_min_px <= t && parent_max_px > t;
}

// The work items of a visible instance: its groups of 32 clusters covering the LOD levels
// that can hold selected clusters at its distance, in increasing order, each once. Returns
// how many; with `write`, also writes them to the work list from `base` and clears their
// status words for the cluster culls' look-back.
uint instance_items(Frame* f, uint instance_index, uint base, bool write)
{
    Instance* inst = f[0].instances + instance_index;
    Mesh* mesh = f[0].meshes + inst[0].mesh;
    float scale = length(inst[0].model[0].xyz);
    float dist = length(inst[0].center - f[0].camera_pos);
    uint groups = (mesh[0].meshlet_count + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE;
    bool all = (f[0].flags & FLAG_GROUP_WINDOW_OFF) != 0;
    bool lod = (f[0].flags & FLAG_LOD) != 0;
    uint level_count = max(mesh[0].level_count, 1u);
    uint64_t* status = f[0].cluster_lookback;
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
        for (uint g = first; write && g <= last; g++)
        {
            uint slot = base + count + (g - first);
            if (slot < f[0].work_capacity)
            {
                f[0].work[slot] = uint2(instance_index, g);
                if (slot % CULL_ITEMS == 0)
                {
                    // The first item of a cluster-cull workgroup clears its status words.
                    status[slot / CULL_ITEMS] = 0;
                    status[f[0].cluster_groups + slot / CULL_ITEMS] = 0;
                }
            }
        }
        if (first <= last)
            count += last - first + 1;
        next_group = last + 1;
    }
    return count;
}

groupshared uint s_counts[64];
groupshared uint s_total;

// One thread per instance: frustum-cull it and append its work items to the work list, in
// instance order (see "ordered appends"). The last workgroup writes the cluster cull's grid
// `indirect` = (x, y, 1, count) over the items that fit in `work_capacity` and counts the
// demand; each group of an instance is appended at most once.
[shader("compute")]
[numthreads(64, 1, 1)]
void cull_main(uint thread : SV_GroupThreadID)
{
    Frame* f = push.frame;
    if (thread == 0)
        InterlockedAdd(f[0].indirect[4], 1, s_ticket);
    GroupMemoryBarrierWithGroupSync();
    uint ticket = s_ticket;
    uint instance_index = ticket * 64 + thread;
    bool visible = false;
    if (instance_index < f[0].instance_count)
    {
        Instance* inst = f[0].instances + instance_index;
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        visible = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
    }
    if (visible)
        InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], 1);
    uint count = visible ? instance_items(f, instance_index, 0, false) : 0;
    s_counts[thread] = count;
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        uint total = 0;
        for (uint i = 0; i < 64; i++)
        {
            uint c = s_counts[i];
            s_counts[i] = total; // now this thread's offset in the workgroup's run
            total += c;
        }
        s_total = total;
    }
    GroupMemoryBarrierWithGroupSync();
    uint total = s_total;
    uint first = lookback_exclusive(f[0].lookback, ticket, uint2(total, 0), thread).x;
    if (thread == 0 && ticket == instance_cull_groups(f) - 1)
    {
        // The last workgroup knows the demand: what fits goes to the cluster cull, the rest is
        // counted (the renderer grows the list from it).
        uint demand = first + total;
        uint items = min(demand, f[0].work_capacity);
        write_grid(f[0].indirect, (items + CULL_ITEMS - 1) / CULL_ITEMS, items);
        InterlockedAdd(f[0].stats[STAT_WORK_ITEMS], demand);
        if (demand > items)
            InterlockedAdd(f[0].stats[STAT_WORK_OVERFLOW], demand - items);
    }
    if (count > 0)
        instance_items(f, instance_index, first + s_counts[thread], true);
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

    if (thread < m.vertex_count)
    {
        uint vertex_index = f[0].meshlet_vertices[m.vertex_offset + thread];
        Vertex v = f[0].vertices[vertex_index];
        float4 world = mul(inst.model, float4(v.position, 1));
        verts[thread].position = mul(f[0].view_proj, world);
    }

    for (uint t = thread; t < m.triangle_count; t += MESH_GROUP_SIZE)
    {
        uint base = m.triangle_offset + t * 3;
        tris[t] = uint3(
            load_byte(f[0].meshlet_triangles, base),
            load_byte(f[0].meshlet_triangles, base + 1),
            load_byte(f[0].meshlet_triangles, base + 2));
        prims[t].id = (slot << VISIBILITY_TRIANGLE_BITS) | t;
    }
}

// ---------------------------------------------------------- vertex fallback ---

// Without mesh shaders every cluster is one indexed draw of `vkCmdDrawIndexedIndirectCount`
// (the command `cluster_cull_main` wrote): the index buffer is the cooked one-byte triangle
// lists, so an index plus the draw's vertex offset (the cluster's `vertex_offset`) is a
// position in `meshlet_vertices`, and the first instance is the cluster's slot in the
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
    Vertex v = f[0].vertices[f[0].meshlet_vertices[vertex_index]];
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
// buffer empty at the end of the frame. Drawn only when the pass has software clusters (an
// indirect draw of three vertices or none). Merging only the tiles the software rasteriser
// touched measured slower where it pays (full detail: the marking cost more than the merge).
[shader("vertex")]
float4 merge_vert_main(uint id : SV_VertexID) : SV_Position
{
    return float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
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
    if (thread < m.vertex_count)
    {
        // The mesh shader's transform, then the fixed-function steps: perspective division,
        // the viewport of `set_viewport_full` (y flipped) and the snap to sub-pixels.
        uint vertex_index = f[0].meshlet_vertices[m.vertex_offset + thread];
        Vertex v = f[0].vertices[vertex_index];
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
        uint base = m.triangle_offset + t * 3;
        rasterise_triangle(f,
            load_byte(f[0].meshlet_triangles, base),
            load_byte(f[0].meshlet_triangles, base + 1),
            load_byte(f[0].meshlet_triangles, base + 2),
            id_base | t);
    }
}

// ------------------------------------------------------------------ shading ---

float3 shade(Frame* f, float3 n, float3 object_position, uint instance_id, uint meshlet_index, uint lod_level, bool culled_debug)
{
    float3 light = normalize(f[0].sun_dir);
    float3 instance_hash = hash_color(instance_id);
    // A fifth of the rocks are ice: brighter, bluish, with a real specular highlight.
    bool ice = instance_hash.y < 0.2;
    float3 albedo;
    float specular_power = 14.0;
    float specular_weight = 0.06;
    if ((f[0].flags & FLAG_LOD_COLORS) != 0)
    {
        albedo = lod_color(min(lod_level, 63u));
    }
    else if ((f[0].flags & FLAG_MESHLET_COLORS) != 0)
    {
        albedo = hash_color(meshlet_index & 0x00FFFFFFu) * 0.6 + 0.3;
    }
    else if (ice)
    {
        float cavity = saturate(0.5 + 0.5 * dot(normalize(object_position), n));
        albedo = lerp(float3(0.50, 0.62, 0.78), float3(0.70, 0.82, 0.92), cavity);
        specular_power = 48.0;
        specular_weight = 0.5;
    }
    else
    {
        // Rock: a per-instance tint between grey and rust, slightly darker in the noise cavities.
        float3 tint = lerp(float3(0.42, 0.40, 0.38), float3(0.45, 0.33, 0.25), instance_hash.x);
        float cavity = saturate(0.5 + 0.5 * dot(normalize(object_position), n));
        albedo = tint * (0.8 + 0.2 * cavity);
    }
    // The sun is far, so a half vector from the light and the camera-to-surface direction
    // approximated by the object's outward normal is close enough for a highlight this soft.
    float ndl = dot(n, light);
    float diffuse = max(ndl, 0.0);
    float3 view = normalize(f[0].camera_pos - (object_position + 0.0)); // object space ≈ world offset
    float3 half_vector = normalize(light + view);
    float specular = pow(max(dot(n, half_vector), 0.0), specular_power) * specular_weight * (ndl > 0.0 ? 1.0 : 0.0);
    // Space: hard sun, faint fill from the nebula side, a little wrap so shadows are not black.
    float wrap = saturate((ndl + 0.3) / 1.3) * 0.08;
    float3 fill = float3(0.10, 0.12, 0.18) * 0.25 * (0.5 + 0.5 * n.y);
    float3 sun = float3(1.0, 0.96, 0.9);
    float3 color = albedo * (diffuse * sun + wrap + fill) + specular * sun;
    if (ice)
        color += float3(0.05, 0.10, 0.18) * saturate(1.0 - abs(ndl)) * 0.6; // rim scatter
    if (culled_debug)
        color = lerp(color, float3(1.0, 0.0, 0.0), 0.8); // a culling error (FLAG_SHOW_CULLED)
    return color;
}

// ------------------------------------------------------------------ resolve ---

struct ResolvePush
{
    Frame* frame;
    uint vis_image;      // sampled-image index of the visibility buffer (R32_UINT)
    uint color_image;    // storage-image index of the colour target (RGBA16F)
    uint width;
    uint height;
    uint use_background; // write `background` to empty pixels (else leave them for the sky)
    uint pad;
    float4 background;
};

[[vk::push_constant]]
ResolvePush resolve_push;

struct Barycentrics
{
    float3 lambda; // perspective-correct barycentric coordinates of the pixel centre
    float3 ddx;    // their change per pixel to the right
    float3 ddy;    // their change per pixel down
};

// Perspective-correct barycentrics of a screen point from the three clip-space vertices, with
// analytic screen-space derivatives (Schied & Dachsbacher 2015; the form used by Hable 2021):
// b_i / w_i is affine in screen space, so it is reconstructed from its value at vertex 0 and
// its gradient; the sum gives 1 / w at the pixel, and lambda_i = w * b_i / w_i. Mirrored on
// the CPU in `forge_render::visibility` with a unit test.
Barycentrics barycentrics(float4 c0, float4 c1, float4 c2, float2 ndc, float2 size)
{
    float3 inv_w = 1.0 / float3(c0.w, c1.w, c2.w);
    float2 n0 = c0.xy * inv_w.x;
    float2 n1 = c1.xy * inv_w.y;
    float2 n2 = c2.xy * inv_w.z;
    float inv_det = 1.0 / determinant(float2x2(n2 - n1, n0 - n1));
    float3 dbdx = float3(n1.y - n2.y, n2.y - n0.y, n0.y - n1.y) * inv_det * inv_w;
    float3 dbdy = float3(n2.x - n1.x, n0.x - n2.x, n1.x - n0.x) * inv_det * inv_w;
    float ddx_sum = dbdx.x + dbdx.y + dbdx.z;
    float ddy_sum = dbdy.x + dbdy.y + dbdy.z;
    float2 delta = ndc - n0;
    float interp_inv_w = inv_w.x + delta.x * ddx_sum + delta.y * ddy_sum;
    float interp_w = 1.0 / interp_inv_w;
    Barycentrics b;
    b.lambda.x = interp_w * (inv_w.x + delta.x * dbdx.x + delta.y * dbdy.x);
    b.lambda.y = interp_w * (delta.x * dbdx.y + delta.y * dbdy.y);
    b.lambda.z = interp_w * (delta.x * dbdx.z + delta.y * dbdy.z);
    b.ddx = interp_w * (dbdx - ddx_sum * b.lambda) * (2.0 / size.x);
    b.ddy = interp_w * (dbdy - ddy_sum * b.lambda) * (-2.0 / size.y);
    return b;
}

// One thread per pixel: read the visibility id, fetch the triangle, reconstruct the
// attributes at the pixel centre and shade once.
[shader("compute")]
[numthreads(8, 8, 1)]
void resolve_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= resolve_push.width || id.y >= resolve_push.height)
        return;
    Frame* f = resolve_push.frame;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(resolve_push.vis_image)].Load(int3(id.xy, 0));
    if (vis == VISIBILITY_EMPTY)
    {
        if (resolve_push.use_background != 0)
            g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)][id.xy] = resolve_push.background;
        return;
    }
    uint slot = vis >> VISIBILITY_TRIANGLE_BITS;
    uint tri = vis & ((1u << VISIBILITY_TRIANGLE_BITS) - 1u);
    uint2 record = f[0].visible[slot];
    uint meshlet_index = record.y & MESHLET_INDEX_MASK;
    bool culled_debug = (record.y & CULLED_BIT) != 0;
    bool software = (record.y & SOFTWARE_BIT) != 0;
    Meshlet m = f[0].meshlets[meshlet_index];
    Instance inst = f[0].instances[record.x];
    uint base = m.triangle_offset + tri * 3;
    uint i0 = f[0].meshlet_vertices[m.vertex_offset + load_byte(f[0].meshlet_triangles, base)];
    uint i1 = f[0].meshlet_vertices[m.vertex_offset + load_byte(f[0].meshlet_triangles, base + 1)];
    uint i2 = f[0].meshlet_vertices[m.vertex_offset + load_byte(f[0].meshlet_triangles, base + 2)];
    Vertex v0 = f[0].vertices[i0];
    Vertex v1 = f[0].vertices[i1];
    Vertex v2 = f[0].vertices[i2];
    float4 c0 = mul(f[0].view_proj, mul(inst.model, float4(v0.position, 1)));
    float4 c1 = mul(f[0].view_proj, mul(inst.model, float4(v1.position, 1)));
    float4 c2 = mul(f[0].view_proj, mul(inst.model, float4(v2.position, 1)));
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    float2 ndc = float2((float(id.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(id.y) + 0.5) / size.y * 2.0);
    Barycentrics b = barycentrics(c0, c1, c2, ndc, size);
    float3 n0 = normalize(mul(inst.model, float4(v0.normal, 0)).xyz);
    float3 n1 = normalize(mul(inst.model, float4(v1.normal, 0)).xyz);
    float3 n2 = normalize(mul(inst.model, float4(v2.normal, 0)).xyz);
    float3 n = normalize(b.lambda.x * n0 + b.lambda.y * n1 + b.lambda.z * n2);
    float3 object_position = b.lambda.x * v0.position + b.lambda.y * v1.position + b.lambda.z * v2.position;
    // `shade` works in units of a white Lambertian surface facing the sun; the frame stores
    // luminance (cd/m²) times the exposure.
    float3 color = shade(f, n, object_position, inst.id, meshlet_index, m.lod_level, culled_debug);
    if (software && (f[0].flags & FLAG_SHOW_RASTER) != 0)
        color = lerp(color, float3(0.1, 0.9, 0.3), 0.6); // the software rasteriser's pixels
    color *= f[0].sun_illuminance * f[0].exposure / 3.14159265;
    g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)][id.xy] = float4(color, 1.0);
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
// `tilt` about the turned X axis, scaled by `scale`.
void place(Placement* p, uint slot, uint mesh, float3 position, float yaw, float tilt, float scale)
{
    float cy = cos(yaw), sy = sin(yaw), ct = cos(tilt), st = sin(tilt);
    // R = Ry(yaw) × Rx(tilt), rows.
    float3 r0 = float3(cy, sy * st, sy * ct);
    float3 r1 = float3(0.0, ct, -st);
    float3 r2 = float3(-sy, cy * st, cy * ct);
    float4x4 model = float4x4(
        float4(r0 * scale, position.x),
        float4(r1 * scale, position.y),
        float4(r2 * scale, position.z),
        float4(0.0, 0.0, 0.0, 1.0));
    Mesh* m = p[0].meshes + mesh;
    Instance inst;
    inst.model = model;
    inst.center = mul(model, float4(m[0].center, 1.0)).xyz;
    inst.radius = m[0].radius * scale;
    inst.mesh = mesh;
    inst.id = p[0].first + slot;
    inst.pad = uint2(0u, 0u);
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
