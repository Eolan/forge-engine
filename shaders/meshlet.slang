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
static const uint CULLED_BIT = 0x80000000u; // top bit of a payload / varying meshlet index
static const uint LOD_SHIFT = 24;           // varying meshlet index bits 24-29: LOD level

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
    public uint* group_table;       // instance index of every task group
    public uint total_groups;
    public uint pad_end;
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
    bool first_group = false;
    bool occluded = false;
    bool culled_debug = false; // FLAG_SHOW_CULLED: this meshlet would have been culled
    uint meshlet_index = 0;
    uint instance_index = 0;
    if (group < f[0].total_groups)
    {
        instance_index = f[0].group_table[group];
        Instance* inst = f[0].instances + instance_index;
        Mesh* mesh = f[0].meshes + inst[0].mesh;
        uint local_group = group - inst[0].group_offset;
        uint local_meshlet = local_group * TASK_GROUP_SIZE + thread;
        first_group = local_group == 0;
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
    if (draw)
        s_payload.meshlet_index[slot] = meshlet_index | (culled_debug ? CULLED_BIT : 0u);
    uint occluded_count = WaveActiveCountBits(occluded);
    if (thread == 0)
    {
        s_payload.instance = instance_index;
        if (draw_count > 0)
            InterlockedAdd(f[0].stats[pass == PASS_REMAINDER ? STAT_MESHLETS_PASS2 : STAT_MESHLETS_PASS1], draw_count);
        if (occluded_count > 0)
            InterlockedAdd(f[0].stats[STAT_OCCLUDED], occluded_count);
        if (instance_visible && first_group && pass != PASS_REMAINDER)
            InterlockedAdd(f[0].stats[STAT_INSTANCES_VISIBLE], 1);
    }
    DispatchMesh(draw_count, 1, 1, s_payload);
}

// ---------------------------------------------------------------- mesh stage ---

static const uint MAX_VERTICES = 64;
static const uint MAX_TRIANGLES = 124;
static const uint MESH_GROUP_SIZE = 64;

struct VertexOut
{
    float4 position : SV_Position;
    float3 world_normal : NORMAL;
    float3 object_position : OBJPOS;
    nointerpolation uint meshlet_id : MESHLET_ID;
    nointerpolation uint instance_id : INSTANCE_ID;
};

[shader("mesh")]
[outputtopology("triangle")]
[numthreads(MESH_GROUP_SIZE, 1, 1)]
void mesh_main(
    uint thread : SV_GroupThreadID,
    uint group : SV_GroupID,
    in payload Payload payload,
    out vertices VertexOut verts[MAX_VERTICES],
    out indices uint3 tris[MAX_TRIANGLES])
{
    Frame* f = push.frame;
    uint packed_index = payload.meshlet_index[group];
    uint meshlet_index = packed_index & ~CULLED_BIT;
    Meshlet m = f[0].meshlets[meshlet_index];
    Instance inst = f[0].instances[payload.instance];

    SetMeshOutputCounts(m.vertex_count, m.triangle_count);

    if (thread < m.vertex_count)
    {
        uint vertex_index = f[0].meshlet_vertices[m.vertex_offset + thread];
        Vertex v = f[0].vertices[vertex_index];
        float4 world = mul(inst.model, float4(v.position, 1));
        verts[thread].position = mul(f[0].view_proj, world);
        verts[thread].world_normal = normalize(mul(inst.model, float4(v.normal, 0)).xyz);
        verts[thread].object_position = v.position;
        // Bits 0-23 meshlet, 24-29 LOD level, 31 culled (debug view).
        verts[thread].meshlet_id = (packed_index & (CULLED_BIT | 0x00FFFFFFu)) | (min(m.lod_level, 63u) << LOD_SHIFT);
        verts[thread].instance_id = inst.id;
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
    }
}

// ------------------------------------------------------------ fragment stage ---

[shader("fragment")]
float4 frag_main(VertexOut input, float4 screen : SV_Position) : SV_Target
{
    Frame* f = push.frame;
    float3 n = normalize(input.world_normal);
    float3 light = normalize(f[0].sun_dir);
    float3 instance_hash = hash_color(input.instance_id);
    // A fifth of the rocks are ice: brighter, bluish, with a real specular highlight.
    bool ice = instance_hash.y < 0.2;
    float3 albedo;
    float specular_power = 14.0;
    float specular_weight = 0.06;
    if ((f[0].flags & FLAG_LOD_COLORS) != 0)
    {
        albedo = lod_color((input.meshlet_id >> LOD_SHIFT) & 0x3Fu);
    }
    else if ((f[0].flags & FLAG_MESHLET_COLORS) != 0)
    {
        albedo = hash_color(input.meshlet_id & 0x00FFFFFFu) * 0.6 + 0.3;
    }
    else if (ice)
    {
        float cavity = saturate(0.5 + 0.5 * dot(normalize(input.object_position), n));
        albedo = lerp(float3(0.50, 0.62, 0.78), float3(0.70, 0.82, 0.92), cavity);
        specular_power = 48.0;
        specular_weight = 0.5;
    }
    else
    {
        // Rock: a per-instance tint between grey and rust, slightly darker in the noise cavities.
        float3 tint = lerp(float3(0.42, 0.40, 0.38), float3(0.45, 0.33, 0.25), instance_hash.x);
        float cavity = saturate(0.5 + 0.5 * dot(normalize(input.object_position), n));
        albedo = tint * (0.8 + 0.2 * cavity);
    }
    // View direction from the camera through this pixel is not in the varyings; the sun is
    // far, so a half vector from the light and the camera-to-surface direction approximated by
    // the object's outward normal is close enough for a highlight this soft.
    float ndl = dot(n, light);
    float diffuse = max(ndl, 0.0);
    float3 view = normalize(f[0].camera_pos - (input.object_position + 0.0)); // object space ≈ world offset
    float3 half_vector = normalize(light + view);
    float specular = pow(max(dot(n, half_vector), 0.0), specular_power) * specular_weight * (ndl > 0.0 ? 1.0 : 0.0);
    // Space: hard sun, faint fill from the nebula side, a little wrap so shadows are not black.
    float wrap = saturate((ndl + 0.3) / 1.3) * 0.08;
    float3 fill = float3(0.10, 0.12, 0.18) * 0.25 * (0.5 + 0.5 * n.y);
    float3 sun = float3(1.0, 0.96, 0.9);
    float3 color = albedo * (diffuse * sun + wrap + fill) + specular * sun;
    if (ice)
        color += float3(0.05, 0.10, 0.18) * saturate(1.0 - abs(ndl)) * 0.6; // rim scatter
    if ((input.meshlet_id & CULLED_BIT) != 0)
        color = lerp(color, float3(1.0, 0.0, 0.0), 0.8); // a culling error (FLAG_SHOW_CULLED)
    return float4(color, 1.0);
}
