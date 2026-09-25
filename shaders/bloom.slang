// Bloom (issue #44), after Jimenez, "Next Generation Post Processing in Call of Duty:
// Advanced Warfare" (SIGGRAPH 2014): the pre-exposed HDR image is filtered down a chain of
// half-size levels with a 13-tap filter (five overlapping 2×2 boxes), the first step
// weighting each box against fireflies (Karis average), then back up with a 3×3 tent, each
// level adding the one below. The TAA resolve blends the top level into the image it shows
// (crates/forge-render/src/bloom.rs).
import bindless;

struct Push
{
    uint src;         // sampled-image index read
    uint dst;         // storage-image index written (the level's own view)
    uint2 dst_size;
    float2 texel;     // one texel of the level read, in uv
    float src_level;  // the mip of `src` read
    uint karis;       // 1 on the first step down
};

[[vk::push_constant]]
Push push;

float luma(float3 c)
{
    return dot(c, float3(0.2126, 0.7152, 0.0722));
}

// Bilinear within `src_level` only: the downsample writes the level after it, which a linear
// mip filter could fetch (issue #72).
float3 tap(float2 uv)
{
    return g_sampled_images[NonUniformResourceIndex(push.src)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP_NEAREST_MIP], uv, push.src_level).rgb;
}

// The mean of a 2×2 box, weighted against fireflies on the first step (1 / (1 + luma)).
float3 box(float3 a, float3 b, float3 c, float3 d, out float weight)
{
    float3 mean = (a + b + c + d) * 0.25;
    weight = push.karis != 0 ? 1.0 / (1.0 + luma(mean)) : 1.0;
    return mean;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void downsample_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.dst_size.x || id.y >= push.dst_size.y)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(push.dst_size);
    float2 t = push.texel;
    float3 a = tap(uv + t * float2(-2.0, -2.0));
    float3 b = tap(uv + t * float2(0.0, -2.0));
    float3 c = tap(uv + t * float2(2.0, -2.0));
    float3 d = tap(uv + t * float2(-1.0, -1.0));
    float3 e = tap(uv + t * float2(1.0, -1.0));
    float3 f = tap(uv + t * float2(-2.0, 0.0));
    float3 g = tap(uv);
    float3 h = tap(uv + t * float2(2.0, 0.0));
    float3 i = tap(uv + t * float2(-1.0, 1.0));
    float3 j = tap(uv + t * float2(1.0, 1.0));
    float3 k = tap(uv + t * float2(-2.0, 2.0));
    float3 l = tap(uv + t * float2(0.0, 2.0));
    float3 m = tap(uv + t * float2(2.0, 2.0));
    float w0, w1, w2, w3, w4;
    float3 centre = box(d, e, i, j, w0);
    float3 top_left = box(a, b, f, g, w1);
    float3 top_right = box(b, c, g, h, w2);
    float3 bottom_left = box(f, g, k, l, w3);
    float3 bottom_right = box(g, h, l, m, w4);
    w0 *= 0.5;
    w1 *= 0.125;
    w2 *= 0.125;
    w3 *= 0.125;
    w4 *= 0.125;
    float3 result = (centre * w0 + top_left * w1 + top_right * w2 + bottom_left * w3 + bottom_right * w4) / (w0 + w1 + w2 + w3 + w4);
    g_storage_images_rgba16f[NonUniformResourceIndex(push.dst)][id.xy] = float4(result, 1.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void upsample_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.dst_size.x || id.y >= push.dst_size.y)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(push.dst_size);
    float2 t = push.texel;
    float3 sum = tap(uv + t * float2(-1.0, -1.0)) + 2.0 * tap(uv + t * float2(0.0, -1.0)) + tap(uv + t * float2(1.0, -1.0))
        + 2.0 * tap(uv + t * float2(-1.0, 0.0)) + 4.0 * tap(uv) + 2.0 * tap(uv + t * float2(1.0, 0.0))
        + tap(uv + t * float2(-1.0, 1.0)) + 2.0 * tap(uv + t * float2(0.0, 1.0)) + tap(uv + t * float2(1.0, 1.0));
    RWTexture2D<float4> level = g_storage_images_rgba16f[NonUniformResourceIndex(push.dst)];
    level[id.xy] = float4(level[id.xy].rgb + sum / 16.0, 1.0);
}
