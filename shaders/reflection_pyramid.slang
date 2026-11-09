// The frame's two pyramids for the screen's reflections (D-108, step 3), a level at a time, each
// level half the one before it, rounded down. The nearest depth, which `screen_reflection` walks;
// and the colour the class passes left, whose levels the rough rows read.
import bindless;

struct Push
{
    uint src_image;  // the pyramid's sampled index, or the frame's depth or colour for level 0
    uint src_level;  // the level it reads, or FROM_THE_FRAME
    uint dst_image;  // storage index of the level being written
    uint dst_width;
    uint dst_height;
    uint src_width;
    uint src_height;
    uint depth_image; // the colour's level 0: where nothing was drawn holds nothing to read
};

[[vk::push_constant]]
Push push;

static const uint FROM_THE_FRAME = 0xFFFFFFFFu;

// The texels of the level before that texel `id` stands for: two a side, and where the level
// before was odd, the last column and row take the one left over.
void children(uint2 id, out uint2 first, out uint2 last)
{
    first = id * 2u;
    last = uint2(id.x + 1u >= push.dst_width ? push.src_width - 1u : first.x + 1u,
                 id.y + 1u >= push.dst_height ? push.src_height - 1u : first.y + 1u);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void reflection_depth_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.dst_width || id.y >= push.dst_height)
        return;
    RWTexture2D<float> out_level = g_storage_images_r32f[NonUniformResourceIndex(push.dst_image)];
    Texture2D<float> source = g_sampled_images_r[NonUniformResourceIndex(push.src_image)];
    if (push.src_level == FROM_THE_FRAME)
    {
        out_level[id.xy] = source.Load(int3(id.xy, 0));
        return;
    }
    uint2 first;
    uint2 last;
    children(id.xy, first, last);
    // Reversed-Z: the greatest is the nearest.
    float nearest = 0.0;
    for (uint y = first.y; y <= last.y; y++)
    {
        for (uint x = first.x; x <= last.x; x++)
            nearest = max(nearest, source.Load(int3(x, y, push.src_level)));
    }
    out_level[id.xy] = nearest;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void reflection_colour_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.dst_width || id.y >= push.dst_height)
        return;
    RWTexture2D<float4> out_level = g_storage_images_rgba16f[NonUniformResourceIndex(push.dst_image)];
    Texture2D<float4> source = g_sampled_images[NonUniformResourceIndex(push.src_image)];
    if (push.src_level == FROM_THE_FRAME)
    {
        // The sky is drawn later: until then those pixels hold nothing. Alpha says how much of a
        // texel is there, and the levels carry the colour by it.
        float there = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)].Load(int3(id.xy, 0)) > 0.0 ? 1.0 : 0.0;
        out_level[id.xy] = float4(source.Load(int3(id.xy, 0)).rgb * there, there);
        return;
    }
    uint2 first;
    uint2 last;
    children(id.xy, first, last);
    float4 sum = float4(0.0);
    uint count = 0u;
    for (uint y = first.y; y <= last.y; y++)
    {
        for (uint x = first.x; x <= last.x; x++)
        {
            sum += source.Load(int3(x, y, push.src_level));
            count += 1u;
        }
    }
    out_level[id.xy] = sum / float(count);
}


// ------------------------------------------------------------- a reflection cube's mips ---

struct CubePush
{
    uint src_image;
    uint dst_image;
    uint level;
    uint side;  // texels of the tile at this level
    uint2 tile;
    uint2 pad;
};

[[vk::push_constant]]
CubePush cube_push;

static const uint CUBE_TILES = 8u;
static const uint CUBE_TILE_TEXELS = 256u;
static const float CUBE_ROUGHEST = 0.42;
static const float CUBE_LAST_LEVEL = 4.0;

float not_zero(float v)
{
    return v >= 0.0 ? 1.0 : -1.0;
}

float3 tile_direction(float2 p)
{
    float h = 1.0 - abs(p.x) - abs(p.y);
    float2 q = h >= 0.0 ? p : (1.0 - abs(p.yx)) * float2(not_zero(p.x), not_zero(p.y));
    return normalize(float3(q.x, h, q.y));
}

float2 tile_place(float3 d)
{
    float2 p = d.xz / (abs(d.x) + abs(d.y) + abs(d.z));
    if (d.y < 0.0)
        p = (1.0 - abs(p.yx)) * float2(not_zero(p.x), not_zero(p.y));
    return p * 0.5 + float2(0.5);
}

float radical_inverse(uint bits)
{
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

// A level of a cube's tile from its level 0: the mirror direction's GGX lobe, its whole, at the
// level's roughness, the viewer along the normal.
[shader("compute")]
[numthreads(8, 8, 1)]
void cube_prefilter_main(uint3 id : SV_DispatchThreadID)
{
    CubePush p = cube_push;
    if (id.x >= p.side || id.y >= p.side)
        return;
    float inset = 1.5 / float(p.side);
    float2 in_tile = (float2(id.xy) + float2(0.5)) / float(p.side);
    float2 o = saturate((in_tile - float2(inset)) / (1.0 - 2.0 * inset));
    float3 n = tile_direction(o * 2.0 - float2(1.0));
    float3 up = abs(n.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0);
    float3 t = normalize(cross(up, n));
    float3 b = cross(n, t);
    float rough = CUBE_ROUGHEST * float(p.level) / CUBE_LAST_LEVEL;
    float a = rough * rough;
    float a2 = a * a;
    Texture2D<float4> source = g_sampled_images[NonUniformResourceIndex(p.src_image)];
    float source_inset = 1.5 / float(CUBE_TILE_TEXELS);
    float3 sum = float3(0.0);
    float weight = 0.0;
    const uint samples = 96u;
    for (uint k = 0u; k < samples; k++)
    {
        float u = float(k) / float(samples);
        float v = radical_inverse(k);
        float phi = 6.2831853 * u;
        float cos2 = (1.0 - v) / (1.0 + (a2 - 1.0) * v);
        float cos_t = sqrt(cos2);
        float sin_t = sqrt(max(1.0 - cos2, 0.0));
        float3 h = t * (cos(phi) * sin_t) + b * (sin(phi) * sin_t) + n * cos_t;
        float3 l = 2.0 * dot(n, h) * h - n;
        float nl = dot(n, l);
        if (nl <= 0.0)
            continue;
        float2 place = float2(source_inset) + tile_place(l) * (1.0 - 2.0 * source_inset);
        float2 uv = (float2(p.tile) + place) / float(CUBE_TILES);
        sum += source.SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], uv, 0.0).rgb * nl;
        weight += nl;
    }
    uint2 texel = p.tile * p.side + id.xy;
    g_storage_images_rgba16f[NonUniformResourceIndex(p.dst_image)][texel] = float4(sum / max(weight, 1e-4), 1.0);
}
