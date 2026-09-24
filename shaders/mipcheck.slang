// The mip check (issue #20, `forge_render::mipcheck`): does a texture sampled with the
// visibility resolve's reconstructed derivatives pick the level a fragment shader picks?
//
// A ground quad seen at a grazing angle is drawn twice. The reference rasterises it with a
// fragment shader that samples with the hardware's own derivatives (`Sample`, 2×2 quads). The
// analytic pass does what the resolve does: for each pixel, the perspective-correct
// barycentrics of the triangle under it and their derivatives (`barycentrics.slang`), then
// `SampleGrad`. The texture is a mip ramp (level k holds k / 16), sampled trilinearly, so each
// sample is the level of detail the sampler chose. A compare pass reduces the difference.
import bindless;
import barycentrics;

struct Params
{
    float4x4 view_proj;
    float4 corners[4];   // the quad's corners in world space (w = 1), a fan 0-1-2, 0-2-3
    uint ramp;           // sampled-image index of the ramp
    uint reference;      // sampled-image index of the reference's target
    uint analytic;       // storage-image index of the analytic pass's target
    uint width;
    uint height;
    float uv_scale;      // texture repeats per metre
    uint analytic_sampled; // the analytic target's sampled-image index (the compare reads it)
    uint pad;
    uint* result;        // max difference (float bits, levels), pixels compared, sum of differences (1/1024 levels)
};

struct Push
{
    Params* params;
};

[[vk::push_constant]]
Push push_constants;

static const uint FAN[6] = { 0, 1, 2, 0, 2, 3 };

float2 uv_of(float3 world)
{
    return world.xz * push_constants.params[0].uv_scale;
}

struct VertexOut
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

[shader("vertex")]
VertexOut reference_vert(uint vertex : SV_VertexID)
{
    float4 corner = push_constants.params[0].corners[FAN[vertex]];
    VertexOut o;
    o.position = mul(push_constants.params[0].view_proj, corner);
    o.uv = uv_of(corner.xyz);
    return o;
}

// The level of detail with the hardware's derivatives; alpha 1 marks a covered pixel.
[shader("fragment")]
float4 reference_frag(VertexOut v) : SV_Target0
{
    float level = sample_image(push_constants.params[0].ramp, SAMPLER_LINEAR_REPEAT, v.uv).r * 16.0;
    return float4(level, 0.0, 0.0, 1.0);
}

// The same with the resolve's derivatives, a thread per pixel.
[shader("compute")]
[numthreads(8, 8, 1)]
void analytic_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push_constants.params[0].width || id.y >= push_constants.params[0].height)
        return;
    float2 size = float2(float(push_constants.params[0].width), float(push_constants.params[0].height));
    float2 ndc = float2((float(id.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(id.y) + 0.5) / size.y * 2.0);
    float4 out_value = float4(0.0);
    [unroll]
    for (uint t = 0; t < 2; t++)
    {
        float4 w0 = push_constants.params[0].corners[FAN[t * 3 + 0]];
        float4 w1 = push_constants.params[0].corners[FAN[t * 3 + 1]];
        float4 w2 = push_constants.params[0].corners[FAN[t * 3 + 2]];
        Barycentrics b = barycentrics(mul(push_constants.params[0].view_proj, w0), mul(push_constants.params[0].view_proj, w1), mul(push_constants.params[0].view_proj, w2), ndc, size);
        if (all(b.lambda >= 0.0))
        {
            float2 u0 = uv_of(w0.xyz), u1 = uv_of(w1.xyz), u2 = uv_of(w2.xyz);
            float2 uv = b.lambda.x * u0 + b.lambda.y * u1 + b.lambda.z * u2;
            float2 dx = b.ddx.x * u0 + b.ddx.y * u1 + b.ddx.z * u2;
            float2 dy = b.ddy.x * u0 + b.ddy.y * u1 + b.ddy.z * u2;
            float level = sample_image_grad(push_constants.params[0].ramp, SAMPLER_LINEAR_REPEAT, uv, dx, dy).r * 16.0;
            out_value = float4(level, 0.0, 0.0, 1.0);
        }
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(push_constants.params[0].analytic)][id.xy] = out_value;
}

// Where both passes covered the pixel: the largest difference, the count and the sum.
[shader("compute")]
[numthreads(8, 8, 1)]
void compare_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push_constants.params[0].width || id.y >= push_constants.params[0].height)
        return;
    float4 a = g_sampled_images[NonUniformResourceIndex(push_constants.params[0].reference)].Load(int3(id.xy, 0));
    float4 b = g_sampled_images[NonUniformResourceIndex(push_constants.params[0].analytic_sampled)].Load(int3(id.xy, 0));
    if (a.a < 0.5 || b.a < 0.5)
        return;
    float difference = abs(a.r - b.r);
    InterlockedMax(push_constants.params[0].result[0], asuint(difference));
    InterlockedAdd(push_constants.params[0].result[1], 1u);
    InterlockedAdd(push_constants.params[0].result[2], uint(difference * 1024.0));
}
