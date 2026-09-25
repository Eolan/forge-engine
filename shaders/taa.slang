// Temporal anti-aliasing, driven by crates/forge-render/src/taa.rs (ported from the previous
// project's temporal.slang, itself after Karis 2014, Jimenez 2016 and Playdead 2016).
//
// Every frame is drawn with its projection shifted by a sub-pixel jitter. The motion pass says
// where each pixel was the frame before (static scene: depth and the two cameras suffice); the
// resolve fetches the history there, clips it to what this frame's neighbourhood allows and
// blends it with this frame's samples. The scene is pre-exposed: the history, stored at the
// previous frame's exposure, is rescaled by `history_scale` before it is compared. The second
// output is the display image, through the tone curve of `tonemap.slang`.
import bindless;
import tonemap;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vert_main(uint id : SV_VertexID)
{
    Varyings o;
    o.position = float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    return o;
}

// ---- Motion vectors ---------------------------------------------------------------------------

struct MotionPush
{
    float4x4 previous_from_current; // previous unjittered clip from this frame's unjittered clip
    float jitter_x;                 // this frame's jitter in pixels, x right, y down
    float jitter_y;
    uint depth;
    uint width;
    uint height;
};

[[vk::push_constant]]
MotionPush motion_push;

[shader("fragment")]
float4 motion_main(Varyings input) : SV_Target
{
    float2 size = float2(float(motion_push.width), float(motion_push.height));
    int2 pixel = int2(input.position.xy);
    float depth = g_sampled_images_r[NonUniformResourceIndex(motion_push.depth)].Load(int3(pixel, 0));
    // The jittered projection moved the scene by the jitter: this pixel shows what sits that far
    // the other way in the unjittered view.
    float2 uv = (float2(pixel) + 0.5 - float2(motion_push.jitter_x, motion_push.jitter_y)) / size;
    float4 previous = mul(motion_push.previous_from_current, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
    if (previous.w <= 1e-9)
        return float4(4.0, 4.0, 0.0, 1.0);
    float2 ndc = previous.xy / previous.w;
    float2 previous_uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return float4(previous_uv - uv, 0.0, 1.0);
}

// ---- Resolve ----------------------------------------------------------------------------------

struct ResolvePush
{
    uint color;
    uint motion;
    uint depth;
    uint history;
    uint width;
    uint height;
    float jitter_x;
    float jitter_y;
    float blend; // how much of the result may come from this frame; 1 discards the history
    float history_scale; // this frame's exposure over the history's
    uint curve;          // TONEMAP_* of tonemap.slang
    uint encode_srgb;    // 1 when the display target is UNORM
    uint bloom;          // the bloom chain's top level (sampled index), 0xFFFFFFFF for none (bloom.slang)
    float bloom_strength; // how much of the shown image is bloom
};

// Levels the bloom chain's top level sums (crates/forge-render/src/bloom.rs, LEVELS).
static const float BLOOM_LEVELS = 6.0;

[[vk::push_constant]]
ResolvePush resolve_push;

float luma(float3 color)
{
    return dot(color, float3(0.2126, 0.7152, 0.0722));
}

// Bright values are compressed before they are averaged and expanded after (Karis 2014).
float3 compress(float3 color)
{
    return color / (1.0 + luma(color));
}

float3 expand(float3 color)
{
    return color / max(1.0 - luma(color), 1e-4);
}

float3 to_ycocg(float3 c)
{
    return float3(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

float3 from_ycocg(float3 c)
{
    return float3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

float3 load_color(int2 pixel)
{
    return max(g_sampled_images[NonUniformResourceIndex(resolve_push.color)].Load(int3(pixel, 0)).rgb, 0.0);
}

float3 history_at(float2 uv)
{
    return g_sampled_images[NonUniformResourceIndex(resolve_push.history)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], uv, 0.0).rgb;
}

// Catmull-Rom from five bilinear taps (Jimenez 2016): a history that does not blur more every frame.
float3 sample_history(float2 uv, float2 size)
{
    float2 position = uv * size;
    float2 centre = floor(position - 0.5) + 0.5;
    float2 f = position - centre;
    float2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    float2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    float2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    float2 w3 = f * f * (-0.5 + 0.5 * f);
    float2 w12 = w1 + w2;
    float2 t0 = (centre - 1.0) / size;
    float2 t3 = (centre + 2.0) / size;
    float2 t12 = (centre + w2 / w12) / size;
    float3 sum = history_at(float2(t12.x, t0.y)) * (w12.x * w0.y)
        + history_at(float2(t0.x, t12.y)) * (w0.x * w12.y)
        + history_at(t12) * (w12.x * w12.y)
        + history_at(float2(t3.x, t12.y)) * (w3.x * w12.y)
        + history_at(float2(t12.x, t3.y)) * (w12.x * w3.y);
    float weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(sum / weight, 0.0);
}

// Moves `value` towards the box centre until it lies inside (Playdead 2016): keeps the hue.
float3 clip_to_box(float3 value, float3 low, float3 high)
{
    float3 centre = 0.5 * (low + high);
    float3 extent = 0.5 * (high - low) + 1e-5;
    float3 offset = value - centre;
    float3 units = abs(offset / extent);
    float largest = max(units.x, max(units.y, units.z));
    return largest > 1.0 ? centre + offset / largest : value;
}

// The resolved colour goes to two targets at once: the next history (pre-exposed HDR) and the
// display image through the tone curve, so no separate display pass is needed afterwards.
struct ResolveOut
{
    float4 history : SV_Target0;
    float4 output : SV_Target1;
};

[shader("fragment")]
ResolveOut resolve_main(Varyings input)
{
    float3 color = resolve_color(input);
    ResolveOut o;
    o.history = float4(color, 1.0);
    // Bloom goes into the image shown, not into the history.
    float3 shown = color;
    if (resolve_push.bloom != 0xFFFFFFFFu)
    {
        float2 uv = input.position.xy / float2(float(resolve_push.width), float(resolve_push.height));
        float3 bloom = sample_image_level(resolve_push.bloom, SAMPLER_LINEAR_CLAMP, uv, 0.0).rgb / BLOOM_LEVELS;
        shown = lerp(color, bloom, resolve_push.bloom_strength);
    }
    o.output = float4(display_output(shown, resolve_push.curve, resolve_push.encode_srgb != 0), 1.0);
    return o;
}

float3 resolve_color(Varyings input)
{
    float2 size = float2(float(resolve_push.width), float(resolve_push.height));
    float2 uv = input.position.xy / size;
    float2 jitter = float2(resolve_push.jitter_x, resolve_push.jitter_y);

    // The render pixel whose sample lands nearest to this output pixel: pixel i's sample shows
    // the unjittered scene at i + 0.5 - jitter.
    float2 position = uv * size;
    int2 nearest = int2(floor(position + jitter));
    int2 last_pixel = int2(size) - 1;
    float sharpness = 4.58;
    float3 sum = 0.0;
    float weights = 0.0;
    float3 moment1 = 0.0;
    float3 moment2 = 0.0;
    float nearest_weight = 0.0;
    float closest_depth = -1.0;
    int2 closest = clamp(nearest, int2(0, 0), last_pixel);
    for (int dy = -1; dy <= 1; dy++)
    {
        for (int dx = -1; dx <= 1; dx++)
        {
            int2 pixel = clamp(nearest + int2(dx, dy), int2(0, 0), last_pixel);
            float3 color = compress(load_color(pixel));
            float2 offset = float2(pixel) + 0.5 - jitter - position;
            float weight = exp(-sharpness * dot(offset, offset));
            sum += color * weight;
            weights += weight;
            nearest_weight = max(nearest_weight, weight);
            float3 ycocg = to_ycocg(color);
            moment1 += ycocg;
            moment2 += ycocg * ycocg;
            // Motion from the nearest surface around: edges move with what is in front.
            float depth = g_sampled_images_r[NonUniformResourceIndex(resolve_push.depth)].Load(int3(pixel, 0));
            if (depth > closest_depth)
            {
                closest_depth = depth;
                closest = pixel;
            }
        }
    }
    float3 current = sum / max(weights, 1e-5);
    if (resolve_push.blend >= 1.0)
        return expand(current);

    float2 motion = g_sampled_images[NonUniformResourceIndex(resolve_push.motion)].Load(int3(closest, 0)).rg;
    float2 history_uv = uv + motion;
    if (any(history_uv < 0.0) || any(history_uv > 1.0))
        return expand(current);

    // Clip the history to the mean ± 1.25 standard deviations of the nine samples around.
    float3 mean = moment1 / 9.0;
    float3 deviation = sqrt(abs(moment2 / 9.0 - mean * mean));
    float3 history = compress(sample_history(history_uv, size) * resolve_push.history_scale);
    history = from_ycocg(clip_to_box(to_ycocg(history), mean - 1.25 * deviation, mean + 1.25 * deviation));

    float alpha = saturate(resolve_push.blend * lerp(0.4, 1.0, nearest_weight));
    alpha = max(alpha, saturate(length(motion * size) / 48.0));
    float3 result = expand(lerp(history, current, alpha));
    if (any(isnan(result)))
        result = expand(current);
    return result;
}
