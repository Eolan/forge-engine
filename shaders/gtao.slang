// Ground-truth ambient occlusion (issue #48): how much of the sky a pixel's surface sees past
// the depth around it. Jimenez, Wu, Pesce, Jarabo, "Practical Real-Time Strategies for
// Accurate Indirect Occlusion", SIGGRAPH 2016, as Intel's XeGTAO implements it: this file
// follows XeGTAO's structure and constants (the depth prefilter, the slice and step loops,
// the spatio-temporal noise). XeGTAO is Copyright (C) 2016-2021, Intel Corporation, under
// the MIT licence (shaders/third-party/XeGTAO-LICENSE.txt).
//
// Four passes, driven by crates/forge-render/src/gtao.rs:
// - `prefilter0_main`: the distance along the view axis of every pixel, from the reversed-Z
//   depth (level 0 of a chain);
// - `prefilter_main`: levels 1..DEPTH_MIPS-1, each a 2×2 average weighted towards the near
//   samples, so a far sample of a thin occluder does not vanish;
// - `gtao_main`: per pixel, SLICES directions around it, STEPS depth samples each way along
//   each; the highest horizon on either side bounds the visible arc, integrated against the
//   normal (rebuilt from the depth);
// - `denoise_main`: a 3×3 blur that does not cross depth edges. The noise changes every
//   frame, and TAA averages what is left.
//
// View space here is XeGTAO's: x right, y up, z the distance forward (the engine's world is
// right-handed with -Z forward; nothing here needs it).
import bindless;
import noise;

struct Push
{
    float2 ndc_to_view_mul;  // view (x, y) = (uv * mul + add) * z, uv the screen position (y down)
    float2 ndc_to_view_add;
    float2 pixel_size;       // 1 / the full-resolution size
    uint2 size;              // the size of the image this pass writes
    float near;              // z = near / depth (reversed-Z, infinite far plane)
    float radius;            // the effect radius, metres
    uint frame;              // the noise's frame index
    uint src;                // sampled image: the depth (level 0), a level of the chain, the raw AO
    uint src_level;
    uint dst;                // storage image (r32f)
    uint depth_mips;         // sampled image: the distance chain (gtao_main, denoise_main)
    uint pad;
};

[[vk::push_constant]]
Push push;

static const uint DEPTH_MIPS = 5;
static const uint SLICES = 3;
static const uint STEPS = 3;               // per side
static const float PI = 3.14159265;
static const float PI_HALF = 1.57079633;
static const float SKY_DISTANCE = 1e6;     // where the depth is empty
static const float RADIUS_MULTIPLIER = 1.457;
static const float FALLOFF_RANGE = 0.615;  // the share of the radius over which occluders fade
static const float SAMPLE_DISTRIBUTION_POWER = 2.0;
static const float DEPTH_MIP_SAMPLING_OFFSET = 3.30;
static const float FINAL_VALUE_POWER = 2.2;
static const float PIXEL_TOO_CLOSE = 1.3;  // pixels: the first sample steps at least this far

float3 view_position(float2 uv, float z)
{
    return float3((uv * push.ndc_to_view_mul + push.ndc_to_view_add) * z, z);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void prefilter0_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size.x || id.y >= push.size.y)
        return;
    float depth = g_sampled_images_r[NonUniformResourceIndex(push.src)].Load(int3(id.xy, 0));
    float z = depth > 0.0 ? push.near / depth : SKY_DISTANCE;
    g_storage_images_r32f[NonUniformResourceIndex(push.dst)][id.xy] = min(z, SKY_DISTANCE);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void prefilter_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size.x || id.y >= push.size.y)
        return;
    Texture2D<float> src = g_sampled_images_r[NonUniformResourceIndex(push.src)];
    uint2 src_size;
    uint levels;
    src.GetDimensions(push.src_level, src_size.x, src_size.y, levels);
    float z[4];
    [unroll]
    for (uint k = 0; k < 4; k++)
    {
        int2 at = min(int2(id.xy * 2 + uint2(k & 1u, k >> 1)), int2(src_size) - 1);
        z[k] = src.Load(int3(at, int(push.src_level)));
    }
    float far = max(max(z[0], z[1]), max(z[2], z[3]));
    // XeGTAO's weights: samples well in front of the farthest one count fully, samples near it
    // fade, so an occluder's edge survives the averaging.
    float effect = 0.75 * push.radius * RADIUS_MULTIPLIER;
    float falloff_range = FALLOFF_RANGE * effect;
    float falloff_from = effect * (1.0 - FALLOFF_RANGE);
    float falloff_mul = -1.0 / falloff_range;
    float falloff_add = falloff_from / falloff_range + 1.0;
    float sum = 0.0, weights = 0.0;
    [unroll]
    for (uint k = 0; k < 4; k++)
    {
        float w = saturate((far - z[k]) * falloff_mul + falloff_add);
        sum += w * z[k];
        weights += w;
    }
    g_storage_images_r32f[NonUniformResourceIndex(push.dst)][id.xy] = sum / max(weights, 1e-6);
}

// How much the pixel's depth agrees with each neighbour (left, right, top, bottom): 1 on the
// same surface, 0 across an edge. A slope through the pixel is not an edge.
float4 depth_edges(float center, float left, float right, float top, float bottom)
{
    float4 edges = float4(left, right, top, bottom) - center;
    float slope_lr = (edges.y - edges.x) * 0.5;
    float slope_tb = (edges.w - edges.z) * 0.5;
    float4 adjusted = edges + float4(slope_lr, -slope_lr, slope_tb, -slope_tb);
    edges = min(abs(edges), abs(adjusted));
    return saturate(1.25 - edges / (center * 0.011));
}

// The view-space normal from the four neighbours' positions, each quadrant's cross product
// counted where both of its edges hold.
float3 depth_normal(float4 edges, float3 center, float3 left, float3 right, float3 top, float3 bottom)
{
    float4 accepted = saturate(float4(edges.x * edges.z, edges.z * edges.y, edges.y * edges.w, edges.w * edges.x) + 0.01);
    float3 l = normalize(left - center);
    float3 r = normalize(right - center);
    float3 t = normalize(top - center);
    float3 b = normalize(bottom - center);
    float3 n = accepted.x * cross(l, t) + accepted.y * cross(t, r) + accepted.z * cross(r, b) + accepted.w * cross(b, l);
    return normalize(n);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void gtao_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size.x || id.y >= push.size.y)
        return;
    RWTexture2D<float> dst = g_storage_images_r32f[NonUniformResourceIndex(push.dst)];
    Texture2D<float> chain = g_sampled_images_r[NonUniformResourceIndex(push.depth_mips)];
    SamplerState point_clamp = g_samplers[SAMPLER_NEAREST_CLAMP];
    int2 pixel = int2(id.xy);
    int2 last = int2(push.size) - 1;
    float center_z = chain.Load(int3(pixel, 0));
    if (center_z >= SKY_DISTANCE * 0.5)
    {
        dst[id.xy] = 1.0;
        return;
    }
    float left_z = chain.Load(int3(max(pixel - int2(1, 0), int2(0, 0)), 0));
    float right_z = chain.Load(int3(min(pixel + int2(1, 0), last), 0));
    float top_z = chain.Load(int3(max(pixel - int2(0, 1), int2(0, 0)), 0));
    float bottom_z = chain.Load(int3(min(pixel + int2(0, 1), last), 0));
    float2 uv = (float2(id.xy) + 0.5) * push.pixel_size;
    float3 position = view_position(uv, center_z);
    float4 edges = depth_edges(center_z, left_z, right_z, top_z, bottom_z);
    float3 normal = depth_normal(edges, position,
        view_position(uv - float2(push.pixel_size.x, 0.0), left_z),
        view_position(uv + float2(push.pixel_size.x, 0.0), right_z),
        view_position(uv - float2(0.0, push.pixel_size.y), top_z),
        view_position(uv + float2(0.0, push.pixel_size.y), bottom_z));
    // Nudge the position towards the camera against self-occlusion from depth precision.
    position *= 0.99920;
    float3 view = normalize(-position);

    float effect = push.radius * RADIUS_MULTIPLIER;
    float falloff_range = FALLOFF_RANGE * effect;
    float falloff_from = effect * (1.0 - FALLOFF_RANGE);
    float falloff_mul = -1.0 / falloff_range;
    float falloff_add = falloff_from / falloff_range + 1.0;
    // The radius in pixels at this depth.
    float pixel_view_size = position.z * abs(push.ndc_to_view_mul.x) * push.pixel_size.x;
    float screen_radius = effect / pixel_view_size;
    float min_s = PIXEL_TOO_CLOSE / screen_radius;
    float2 jitter = spatio_temporal_noise(id.xy, push.frame);

    float visibility = 0.0;
    for (uint slice = 0; slice < SLICES; slice++)
    {
        float phi = (float(slice) + jitter.x) / float(SLICES) * PI;
        float cos_phi = cos(phi);
        float sin_phi = sin(phi);
        // The slice's direction on screen (y down), in pixels, and in view space (y up).
        float2 omega = float2(cos_phi, -sin_phi) * screen_radius;
        float3 direction = float3(cos_phi, sin_phi, 0.0);
        float3 ortho_direction = direction - dot(direction, view) * view;
        float3 axis = normalize(cross(ortho_direction, view));
        float3 projected_normal = normal - axis * dot(normal, axis);
        float sign_normal = sign(dot(ortho_direction, projected_normal));
        float projected_length = length(projected_normal);
        float cos_normal = saturate(dot(projected_normal, view) / max(projected_length, 1e-6));
        float n = sign_normal * acos(cos_normal);
        float low_horizon0 = cos(n + PI_HALF);
        float low_horizon1 = cos(n - PI_HALF);
        float horizon0 = low_horizon0;
        float horizon1 = low_horizon1;
        for (uint j = 0; j < STEPS; j++)
        {
            float step_noise = frac(jitter.y + float(slice + j * STEPS) * 0.6180339887);
            float s = (float(j) + step_noise) / float(STEPS);
            s = pow(s, SAMPLE_DISTRIBUTION_POWER) + min_s;
            float2 offset = s * omega;
            float level = clamp(log2(length(offset)) - DEPTH_MIP_SAMPLING_OFFSET, 0.0, float(DEPTH_MIPS - 1));
            // Whole pixels, so the sample reads the texel it names.
            offset = round(offset) * push.pixel_size;
            float2 uv0 = uv + offset;
            float2 uv1 = uv - offset;
            float3 delta0 = view_position(uv0, chain.SampleLevel(point_clamp, uv0, level)) - position;
            float3 delta1 = view_position(uv1, chain.SampleLevel(point_clamp, uv1, level)) - position;
            float distance0 = length(delta0);
            float distance1 = length(delta1);
            float weight0 = saturate(distance0 * falloff_mul + falloff_add);
            float weight1 = saturate(distance1 * falloff_mul + falloff_add);
            float cos0 = lerp(low_horizon0, dot(delta0 / max(distance0, 1e-6), view), weight0);
            float cos1 = lerp(low_horizon1, dot(delta1 / max(distance1, 1e-6), view), weight1);
            horizon0 = max(horizon0, cos0);
            horizon1 = max(horizon1, cos1);
        }
        // The visible arc between the two horizons, clamped to the hemisphere around the
        // projected normal, integrated against the cosine.
        projected_length = lerp(projected_length, 1.0, 0.05);
        float h0 = -acos(clamp(horizon1, -1.0, 1.0));
        float h1 = acos(clamp(horizon0, -1.0, 1.0));
        h0 = n + clamp(h0 - n, -PI_HALF, PI_HALF);
        h1 = n + clamp(h1 - n, -PI_HALF, PI_HALF);
        float arc0 = (cos_normal + 2.0 * h0 * sin(n) - cos(2.0 * h0 - n)) / 4.0;
        float arc1 = (cos_normal + 2.0 * h1 * sin(n) - cos(2.0 * h1 - n)) / 4.0;
        visibility += projected_length * (arc0 + arc1);
    }
    visibility /= float(SLICES);
    visibility = pow(saturate(visibility), FINAL_VALUE_POWER);
    // Fade out where the radius covers under a few pixels (far away): nothing to resolve.
    visibility = lerp(1.0, visibility, saturate((screen_radius - PIXEL_TOO_CLOSE) / 4.0));
    dst[id.xy] = max(visibility, 0.03);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void denoise_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.size.x || id.y >= push.size.y)
        return;
    Texture2D<float> ao = g_sampled_images_r[NonUniformResourceIndex(push.src)];
    Texture2D<float> chain = g_sampled_images_r[NonUniformResourceIndex(push.depth_mips)];
    int2 pixel = int2(id.xy);
    int2 last = int2(push.size) - 1;
    float center_z = chain.Load(int3(pixel, 0));
    float sum = 0.0, weights = 0.0;
    [unroll]
    for (int y = -1; y <= 1; y++)
    {
        [unroll]
        for (int x = -1; x <= 1; x++)
        {
            int2 at = clamp(pixel + int2(x, y), int2(0, 0), last);
            float z = chain.Load(int3(at, 0));
            // Same surface: depths within about 2 % (the edge test's scale); corners count half.
            float w = saturate(1.25 - abs(z - center_z) / (center_z * 0.022));
            w *= (x == 0 && y == 0) ? 1.0 : (x == 0 || y == 0) ? 0.75 : 0.5;
            sum += w * ao.Load(int3(at, 0));
            weights += w;
        }
    }
    g_storage_images_r32f[NonUniformResourceIndex(push.dst)][id.xy] = sum / max(weights, 1e-6);
}
