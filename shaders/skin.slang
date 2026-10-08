// The skin pass (#165, `crate::skin` in forge-render): moves the vertices of the skinned meshes'
// clusters every frame, before the culls read them. One workgroup per cluster, one thread per
// vertex: linear blend skinning (four joints a vertex) of the bind-pose position and normal
// (`SkinVertex`, crates/forge-geom/src/skin.rs) by this frame's joint matrices, written over the
// cluster's vertices in the pool of cluster pages (a skinned mesh's pages are all roots,
// pinned: each cluster keeps its place in the pool). With rays, the positions also go to the
// ray tracing's copy, which the mesh's bottom-level structure is then refitted from. And the
// position the frame before's matrices gave goes to `previous`, which the motion vectors read.
// A displaced mesh (#185's deformable ground: one joint, which places it) is first raised by
// its height field, this frame's and the frame before's, its normal the field's slopes.

// Mirrors `SkinVertex` in crates/forge-geom/src/skin.rs (32 bytes).
struct SkinVertex
{
    float3 position;   // bind pose
    uint normal;       // bind pose, octahedral (forge_geom::page::encode_normal)
    uint2 joints;      // four 16-bit joint indices, low half first
    uint2 weights;     // four 16-bit unsigned normalised weights, summing to 65 535
};

// Mirrors `GpuSkinCluster` in crates/forge-render/src/skin.rs (32 bytes).
struct SkinCluster
{
    uint pool;          // bytes into the pool where its payload starts
    uint first;         // its first vertex in `vertices` and `previous`
    uint vertex_count;
    uint ray;           // its first vertex in the ray tracing's positions (0xFFFFFFFF: none)
    uint joints;        // its mesh's first matrix in `joints`
    uint field;         // its mesh's height field in `fields` (0xFFFFFFFF: none, #185)
    uint2 pad;
};

// Mirrors `GpuHeightField` in crates/forge-render/src/skin.rs (32 bytes): a grid of heights over
// a displaced mesh's bind-pose x and z (#185's deformable ground).
struct HeightField
{
    float2 origin;  // where its first height lies, (x, z)
    float cell;     // metres between two heights
    uint first;     // its first height in a frame's heights
    uint2 size;     // heights along x and z
    uint slopes;    // its first slope in a frame's heights, x then z a point, added to its own (#197; 0xFFFFFFFF: none)
    uint pad;
};

// Mirrors `SkinPush` in crates/forge-render/src/skin.rs.
struct SkinPush
{
    SkinCluster* clusters;
    SkinVertex* vertices;
    float4* joints;          // this frame's matrices, three rows each (object space)
    float4* previous_joints; // the frame before's
    uint* pool;
    float* rays;             // the ray tracing's positions, three floats a vertex
    float* previous;         // out: the frame before's skinned positions, three floats a vertex
    HeightField* fields;     // the displaced meshes' height fields (#185)
    float* heights;          // this frame's heights, every field's in turn, row by row along x
    float* previous_heights; // the frame before's
    uint cluster_count;
    uint has_rays;
};

[[vk::push_constant]]
SkinPush push;

float3 decode_normal(uint e)
{
    float2 o = clamp(float2(float(int(e << 16) >> 16), float(int(e) >> 16)) / 32767.0, -1.0, 1.0);
    float3 n = float3(o, 1.0 - abs(o.x) - abs(o.y));
    float t = max(-n.z, 0.0);
    n.x += n.x >= 0.0 ? -t : t;
    n.y += n.y >= 0.0 ? -t : t;
    return normalize(n);
}

// forge_geom::page::encode_normal.
uint encode_normal(float3 n)
{
    float l1 = abs(n.x) + abs(n.y) + abs(n.z);
    if (l1 == 0.0 || isinf(l1) || isnan(l1))
        return 0;
    float2 o = n.xy / l1;
    if (n.z < 0.0)
    {
        float2 s = float2(o.x >= 0.0 ? 1.0 : -1.0, o.y >= 0.0 ? 1.0 : -1.0);
        o = (1.0 - abs(o.yx)) * s;
    }
    int2 q = int2(round(clamp(o, -1.0, 1.0) * 32767.0));
    return (uint(q.x) & 0xFFFFu) | ((uint(q.y) & 0xFFFFu) << 16);
}

// The weighted sum of a vertex's joints' matrices (three rows).
struct Blend
{
    float4 r0;
    float4 r1;
    float4 r2;
};

Blend blend(float4* matrices, uint base, uint4 joints, float4 weights)
{
    Blend b;
    b.r0 = float4(0.0);
    b.r1 = float4(0.0);
    b.r2 = float4(0.0);
    for (uint k = 0; k < 4; k++)
    {
        if (weights[k] > 0.0)
        {
            uint j = 3 * (base + joints[k]);
            b.r0 += weights[k] * matrices[j];
            b.r1 += weights[k] * matrices[j + 1];
            b.r2 += weights[k] * matrices[j + 2];
        }
    }
    return b;
}

float3 move_point(Blend b, float3 p)
{
    float4 h = float4(p, 1.0);
    return float3(dot(b.r0, h), dot(b.r1, h), dot(b.r2, h));
}

float3 turn_normal(Blend b, float3 n)
{
    return normalize(float3(dot(b.r0.xyz, n), dot(b.r1.xyz, n), dot(b.r2.xyz, n)));
}

// Field `h`'s height at grid point `g`, clamped to the grid.
float height_at(float* heights, HeightField h, int2 g)
{
    uint2 c = uint2(clamp(g, int2(0), int2(h.size) - 1));
    return heights[h.first + c.y * h.size.x + c.x];
}

// Field `h`'s height under bind-pose (x, z) `p`, between its four nearest points.
float field_height(float* heights, HeightField h, float2 p)
{
    float2 g = (p - h.origin) / h.cell;
    int2 g0 = int2(floor(g));
    float2 t = g - float2(g0);
    float a = lerp(height_at(heights, h, g0), height_at(heights, h, g0 + int2(1, 0)), t.x);
    float b = lerp(height_at(heights, h, g0 + int2(0, 1)), height_at(heights, h, g0 + int2(1, 1)), t.x);
    return lerp(a, b, t.y);
}

// Field `h`'s added slopes (x, z) at grid point `g`, clamped to the grid (#197).
float2 slope_at(float* heights, HeightField h, int2 g)
{
    uint2 c = uint2(clamp(g, int2(0), int2(h.size) - 1));
    uint i = h.slopes + 2 * (c.y * h.size.x + c.x);
    return float2(heights[i], heights[i + 1]);
}

// The normal of field `h`'s surface at bind-pose (x, z) `p`: its slopes across a cell either
// side, plus its added slopes there when it has them (a ground window's, #197: the ground's
// smooth normals in place of its facets'), between its four nearest points.
float3 field_normal(float* heights, HeightField h, float2 p)
{
    float2 dx = float2(h.cell, 0.0);
    float2 dz = float2(0.0, h.cell);
    float sx = (field_height(heights, h, p + dx) - field_height(heights, h, p - dx)) / (2.0 * h.cell);
    float sz = (field_height(heights, h, p + dz) - field_height(heights, h, p - dz)) / (2.0 * h.cell);
    if (h.slopes != 0xFFFFFFFFu)
    {
        float2 g = (p - h.origin) / h.cell;
        int2 g0 = int2(floor(g));
        float2 t = g - float2(g0);
        float2 a = lerp(slope_at(heights, h, g0), slope_at(heights, h, g0 + int2(1, 0)), t.x);
        float2 b = lerp(slope_at(heights, h, g0 + int2(0, 1)), slope_at(heights, h, g0 + int2(1, 1)), t.x);
        float2 added = lerp(a, b, t.y);
        sx += added.x;
        sz += added.y;
    }
    return normalize(float3(-sx, 1.0, -sz));
}

[shader("compute")]
[numthreads(64, 1, 1)]
void skin_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    uint c_index = group.x;
    if (c_index >= push.cluster_count)
        return;
    SkinCluster c = push.clusters[c_index];
    uint i = thread.x;
    if (i >= c.vertex_count)
        return;
    SkinVertex v = push.vertices[c.first + i];
    uint4 joints = uint4(v.joints.x & 0xFFFFu, v.joints.x >> 16, v.joints.y & 0xFFFFu, v.joints.y >> 16);
    float4 weights = float4(v.weights.x & 0xFFFFu, v.weights.x >> 16, v.weights.y & 0xFFFFu, v.weights.y >> 16) / 65535.0;
    uint base = c.joints;

    // A displaced mesh (#185): raised by its field before its joint places it, its normal
    // the field's.
    float3 bind = v.position;
    float3 bind_normal = decode_normal(v.normal);
    float3 bind_before = v.position;
    if (c.field != 0xFFFFFFFFu)
    {
        HeightField h = push.fields[c.field];
        bind.y += field_height(push.heights, h, v.position.xz);
        bind_normal = field_normal(push.heights, h, v.position.xz);
        bind_before.y += field_height(push.previous_heights, h, v.position.xz);
    }

    Blend now = blend(push.joints, base, joints, weights);
    float3 p = move_point(now, bind);
    float3 n = turn_normal(now, bind_normal);
    uint at = c.pool / 4 + i * 4;
    push.pool[at] = asuint(p.x);
    push.pool[at + 1] = asuint(p.y);
    push.pool[at + 2] = asuint(p.z);
    push.pool[at + 3] = encode_normal(n);
    if (push.has_rays != 0 && c.ray != 0xFFFFFFFFu)
    {
        uint r = 3 * (c.ray + i);
        push.rays[r] = p.x;
        push.rays[r + 1] = p.y;
        push.rays[r + 2] = p.z;
    }
    float3 before = move_point(blend(push.previous_joints, base, joints, weights), bind_before);
    uint q = 3 * (c.first + i);
    push.previous[q] = before.x;
    push.previous[q + 1] = before.y;
    push.previous[q + 2] = before.z;
}
