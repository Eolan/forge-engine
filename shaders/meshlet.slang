// Meshlet rendering: task shader culls instances and meshlets, mesh shader emits triangles.
//
// Every buffer is reached through a device address held in the per-frame `Frame` block, so
// the CPU hands the GPU one pointer (push constant) and the GPU walks the scene from there.
// Layouts are std430 and mirrored by `#[repr(C)]` structs in `forge-geom` / `forge-render`.
//
// Occlusion is two-pass (Haar & Aaltonen 2015, Nanite 2021): pass 1 draws the meshlets that
// were visible last frame; the renderer then builds a hierarchical-Z pyramid from that depth;
// pass 2 tests every remaining meshlet against it, draws the survivors and rewrites the
// visibility bits for next frame (pass-1 meshlets are re-tested too, without drawing).
import bindless;

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

// One mesh: a range of meshlets stored level by level, with per-level bounds so a task group
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
    public uint group_offset; // first task group of this instance
    public uint bit_offset;   // first visibility bit of this instance
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
static const uint CULLED_BIT = 0x80000000u; // top bit of a payload / visible-list meshlet index

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

// The visibility buffer: every drawn pixel holds `visible_slot << 7 | triangle`, where
// `visible_slot` indexes the frame's visible-cluster list (`Frame::visible`, appended by the
// task shader) and `triangle` the cluster's triangle; `VISIBILITY_EMPTY` is the clear value.
public static const uint VISIBILITY_EMPTY = 0xFFFFFFFFu;
public static const uint VISIBILITY_TRIANGLE_BITS = 7;

public struct Frame
{
    public float4x4 view_proj;      // for drawing
    public float4x4 cull_view_proj; // for culling (frozen when FLAG_FREEZE_CULLING)
    public float4x4 cull_view;      // view matrix used for culling
    public float4 planes[6];        // frustum planes of `cull_view_proj`, inward normals
    public float3 camera_pos;       // camera used for culling
    public uint instance_count;
    public uint max_meshlets;       // task groups per instance = ceil(max_meshlets / 32)
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
    public uint* visibility;        // one bit per (instance, local meshlet): visible last frame
    public uint* group_table;       // instance index of every task group (capacity: total_groups)
    public uint total_groups;
    public uint pad_end;
    public uint2* work;             // (instance, local group) per task group, built by `cull_main`
    public uint* indirect;          // VkDrawMeshTasksIndirectCommandEXT: x = work count, y = z = 1
    public uint2* visible;          // slot 0: (count, 0); then (instance, meshlet | CULLED_BIT) per drawn cluster
    public uint visible_capacity;   // slots after the counter
    public float exposure;          // pre-exposure of the frame (crates/forge-render/src/exposure.rs)
    public float sun_illuminance;   // lux; the rocks return albedo × E / π
    public uint pad_exposure;
};

struct Push
{
    Frame* frame;
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

// True when the sphere may be visible against the depth pyramid.
bool occlusion_visible(Frame* f, float3 world_center, float radius)
{
    float3 c = mul(f[0].cull_view, float4(world_center, 1)).xyz;
    float4 aabb;
    if (!project_sphere(c, radius, f[0].znear, f[0].p00, f[0].p11, aabb))
        return true;
    // The depth was drawn with a jittered projection: test where the sphere is in *that* image.
    aabb += f[0].draw_jitter.xyxy;
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
        min(sample_depth_level(f[0].hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.xy, level),
            sample_depth_level(f[0].hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.zy, level)),
        min(sample_depth_level(f[0].hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.xw, level),
            sample_depth_level(f[0].hzb_image, SAMPLER_MIN_REDUCTION_CLAMP, aabb.zw, level)));
    // Reversed-Z, infinite far plane: depth = znear / distance. Nearest point of the sphere:
    float nearest = f[0].znear / (-c.z - radius);
    return nearest >= farthest;
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

// Whether any cluster of task group `local_group` of `inst` can be selected: the group's 32
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

// ---------------------------------------------------------------- task stage ---

static const uint TASK_GROUP_SIZE = 32;

struct Payload
{
    uint meshlet_index[TASK_GROUP_SIZE];
    uint instance;
    uint visible_base; // first slot of this group's clusters in the visible-cluster list
};

groupshared Payload s_payload;

// One task workgroup handles 32 consecutive meshlets of one instance.
[shader("amplification")]
[numthreads(TASK_GROUP_SIZE, 1, 1)]
void task_main(uint thread : SV_GroupThreadID, uint group : SV_GroupID)
{
    Frame* f = push.frame;
    uint pass = f[0].pass;

    bool draw = false;
    bool instance_visible = false;
    bool occluded = false;
    bool culled_debug = false; // FLAG_SHOW_CULLED: this meshlet would have been culled
    uint meshlet_index = 0;
    uint instance_index = 0;
    if (group < f[0].total_groups)
    {
        uint2 item = f[0].work[group]; // written by cull_main: (instance, local group)
        instance_index = item.x;
        uint local_group = item.y;
        Instance* inst = f[0].instances + instance_index;
        Mesh* mesh = f[0].meshes + inst[0].mesh;
        uint local_meshlet = local_group * TASK_GROUP_SIZE + thread;
        bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
        bool show_culled = (f[0].flags & FLAG_SHOW_CULLED) != 0;
        instance_visible = !frustum || sphere_in_frustum(f, inst[0].center, inst[0].radius);
        // Uniform scale is assumed for the bounding spheres: take the length of the first column.
        float scale = length(inst[0].model[0].xyz);
        // Whole-group rejection from the per-level tables before any cluster is read.
        bool possible = instance_visible && group_possible(f, mesh, inst, local_group, scale);
        if (possible && local_meshlet < mesh[0].meshlet_count)
        {
            meshlet_index = mesh[0].meshlet_offset + local_meshlet;
            uint bit_index = inst[0].bit_offset + local_meshlet;
            uint word = bit_index >> 5;
            uint mask = 1u << (bit_index & 31);
            bool was_visible = (f[0].visibility[word] & mask) != 0;

            bool visible = false;
            float3 center = 0;
            float world_radius = 0;
            {
                Meshlet m = f[0].meshlets[meshlet_index];
                center = mul(inst[0].model, float4(m.center, 1)).xyz;
                world_radius = m.radius * scale;
                visible = lod_selected(f, m, inst[0].model, scale);
                visible = visible && (!frustum || sphere_in_frustum(f, center, world_radius));
                // Normal-cone culling (meshoptimizer): culled when
                // dot(normalize(apex - camera), axis) >= cutoff. A cutoff of 1 marks a cone
                // wider than a hemisphere; meshoptimizer then leaves the axis at zero, whose
                // normalisation is NaN and would (silently) cull the meshlet: skip the test.
                if (visible && (f[0].flags & FLAG_CONE_CULLING) != 0 && m.cone_cutoff < 1.0)
                {
                    float3 apex = mul(inst[0].model, float4(m.cone_apex, 1)).xyz;
                    float3 axis = normalize(mul(inst[0].model, float4(m.cone_axis, 0)).xyz);
                    bool cone_visible = dot(normalize(apex - f[0].camera_pos), axis) < m.cone_cutoff;
                    if (!cone_visible && show_culled)
                        culled_debug = true;
                    else
                        visible = cone_visible;
                }
            }

            if (pass == PASS_PREVIOUSLY_VISIBLE)
            {
                draw = was_visible && visible;
            }
            else if (pass == PASS_REMAINDER)
            {
                bool now_visible = visible;
                if (visible && (f[0].flags & FLAG_OCCLUSION_CULLING) != 0)
                {
                    now_visible = occlusion_visible(f, center, world_radius);
                    occluded = !now_visible;
                    if (occluded && show_culled)
                    {
                        culled_debug = true;
                        now_visible = true;
                    }
                }
                draw = now_visible && !was_visible;
                if (now_visible != was_visible)
                {
                    if (now_visible)
                        InterlockedOr(f[0].visibility[word], mask);
                    else
                        InterlockedAnd(f[0].visibility[word], ~mask);
                }
            }
            else
            {
                draw = visible;
            }
        }
    }

    uint draw_count = WaveActiveCountBits(draw);
    uint slot = WavePrefixCountBits(draw);
    // Reserve this group's slots in the visible-cluster list (one atomic per group; the list
    // starts at slot 1, slot 0 holds the counter). A full list drops the clusters and counts
    // them, so the resolve never reads past the end.
    uint visible_base = 0;
    if (thread == 0 && draw_count > 0)
        InterlockedAdd(f[0].visible[0].x, draw_count, visible_base);
    visible_base = WaveReadLaneAt(visible_base, 0);
    if (visible_base + draw_count > f[0].visible_capacity)
    {
        if (thread == 0 && draw_count > 0)
            InterlockedAdd(f[0].stats[STAT_VISIBLE_OVERFLOW], draw_count);
        draw_count = 0;
        draw = false;
    }
    uint packed = meshlet_index | (culled_debug ? CULLED_BIT : 0u);
    if (draw)
    {
        s_payload.meshlet_index[slot] = packed;
        f[0].visible[1 + visible_base + slot] = uint2(instance_index, packed);
    }
    uint occluded_count = WaveActiveCountBits(occluded);
    if (thread == 0)
    {
        s_payload.instance = instance_index;
        s_payload.visible_base = visible_base;
        if (draw_count > 0)
            InterlockedAdd(f[0].stats[pass == PASS_REMAINDER ? STAT_MESHLETS_PASS2 : STAT_MESHLETS_PASS1], draw_count);
        if (occluded_count > 0)
            InterlockedAdd(f[0].stats[STAT_OCCLUDED], occluded_count);
    }
    DispatchMesh(draw_count, 1, 1, s_payload);
}

// ------------------------------------------------------------- cull stage ---

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

// One thread per instance: frustum-cull the instance, find the LOD levels that can hold
// selected clusters at its distance, and append the task groups covering those levels to
// the work list. `indirect[0]` counts them and is the x of the indirect mesh-task draws;
// each group is appended at most once, so the list never exceeds `total_groups`.
[shader("compute")]
[numthreads(64, 1, 1)]
void cull_main(uint3 id : SV_DispatchThreadID)
{
    Frame* f = push.frame;
    uint instance_index = id.x;
    if (instance_index == 0)
        f[0].visible[0] = uint2(0, 0); // the visible-cluster counter, before the mesh passes
    if (instance_index >= f[0].instance_count)
        return;
    Instance* inst = f[0].instances + instance_index;
    bool frustum = (f[0].flags & FLAG_FRUSTUM_CULLING) != 0;
    if (frustum && !sphere_in_frustum(f, inst[0].center, inst[0].radius))
        return;
    InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], 1);
    Mesh* mesh = f[0].meshes + inst[0].mesh;
    float scale = length(inst[0].model[0].xyz);
    float dist = length(inst[0].center - f[0].camera_pos);
    uint groups = (mesh[0].meshlet_count + TASK_GROUP_SIZE - 1) / TASK_GROUP_SIZE;
    bool all = (f[0].flags & FLAG_GROUP_WINDOW_OFF) != 0;
    bool lod = (f[0].flags & FLAG_LOD) != 0;
    uint level_count = max(mesh[0].level_count, 1u);
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
        for (uint g = first; g <= last; g++)
        {
            uint slot;
            InterlockedAdd(f[0].indirect[0], 1, slot);
            if (slot < f[0].total_groups)
                f[0].work[slot] = uint2(instance_index, g);
        }
        next_group = last + 1;
    }
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

[shader("mesh")]
[outputtopology("triangle")]
[numthreads(MESH_GROUP_SIZE, 1, 1)]
void mesh_main(
    uint thread : SV_GroupThreadID,
    uint group : SV_GroupID,
    in payload Payload payload,
    out vertices VertexOut verts[MAX_VERTICES],
    out indices uint3 tris[MAX_TRIANGLES],
    out primitives PrimitiveOut prims[MAX_TRIANGLES])
{
    Frame* f = push.frame;
    uint meshlet_index = payload.meshlet_index[group] & ~CULLED_BIT;
    Meshlet m = f[0].meshlets[meshlet_index];
    Instance inst = f[0].instances[payload.instance];
    uint visible_slot = payload.visible_base + group;

    SetMeshOutputCounts(m.vertex_count, m.triangle_count);

    if (thread < m.vertex_count)
    {
        uint vertex_index = f[0].meshlet_vertices[m.vertex_offset + thread];
        Vertex v = f[0].vertices[vertex_index];
        float4 world = mul(inst.model, float4(v.position, 1));
        verts[thread].position = mul(f[0].view_proj, world);
    }
    if (thread == 0)
    {
        InterlockedAdd(f[0].stats[STAT_TRIANGLES], m.triangle_count);
        InterlockedAdd(f[0].stats[STAT_LOD_LEVEL_SUM], m.lod_level);
    }

    for (uint t = thread; t < m.triangle_count; t += MESH_GROUP_SIZE)
    {
        uint base = m.triangle_offset + t * 3;
        tris[t] = uint3(
            load_byte(f[0].meshlet_triangles, base),
            load_byte(f[0].meshlet_triangles, base + 1),
            load_byte(f[0].meshlet_triangles, base + 2));
        prims[t].id = (visible_slot << VISIBILITY_TRIANGLE_BITS) | t;
    }
}

// ------------------------------------------------------------ fragment stage ---

// Writes the visibility id; the depth test already decided who owns the pixel.
[shader("fragment")]
uint frag_main(uint id : SV_PrimitiveID) : SV_Target
{
    return id;
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
    uint2 record = f[0].visible[1 + slot];
    uint meshlet_index = record.y & ~CULLED_BIT;
    bool culled_debug = (record.y & CULLED_BIT) != 0;
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
    float3 color = shade(f, n, object_position, inst.id, meshlet_index, m.lod_level, culled_debug)
        * (f[0].sun_illuminance * f[0].exposure / 3.14159265);
    g_storage_images_rgba16f[NonUniformResourceIndex(resolve_push.color_image)][id.xy] = float4(color, 1.0);
}
