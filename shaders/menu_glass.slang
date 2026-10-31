// The frosted glass behind the game's menu (#377, the owner: "a glass gaussian transparency
// effect but almost opaque", "full screen"), driven by crates/forge-app/src/glass.rs: the frame
// brought down to a quarter of its size (each pixel the mean of a 4 x 4 block, so a moving
// picture behind it does not shimmer), blurred by a Gaussian across and then down, and drawn
// back over the whole window under a dark tint, nearly opaque. An sRGB frame is read as linear
// light and written back encoded; in HDR the tint is encoded as the HUD's colours are.
import bindless;
import tonemap;

// Must match GlassPush in glass.rs.
struct GlassPush
{
    uint image;      // sampled image: the pass's source
    uint encoding;   // the overlay's: 0 the display's own, OUTPUT_PQ, OUTPUT_SCRGB
    float ui_white;  // nits of the interface's white in HDR
    uint axis;       // the blur's: 0 across, 1 down
    float2 texel;    // one over the source's size (the window's for the last pass)
    float2 pad;
};

[[vk::push_constant]]
GlassPush glass_push;

// The blur's reach, quarter-size pixels: sigma 4 (16 of the window's), out to three sigmas.
static const int RADIUS = 12;
static const float SIGMA = 4.0;
// The glass: this much the tint, the rest the blurred game.
static const float OPACITY = 0.80;
static const float3 TINT = float3(0.010, 0.012, 0.016);

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

// A quarter-size pixel: the mean of its 4 x 4 block of the frame, read pixel by pixel (the frame's
// format need not filter: HDR10's A2R10G10B10), clamped to the frame at its right and bottom.
[shader("fragment")]
float4 down_main(Varyings input) : SV_Target
{
    int2 base = int2(input.position.xy) * 4;
    int2 last = int2(round(1.0 / glass_push.texel)) - 1;
    float3 sum = 0.0;
    for (int y = 0; y < 4; y++)
        for (int x = 0; x < 4; x++)
        {
            int2 q = min(base + int2(x, y), last);
            sum += g_sampled_images[NonUniformResourceIndex(glass_push.image)].Load(int3(q, 0)).rgb;
        }
    return float4(sum / 16.0, 1.0);
}

// A Gaussian along `axis`, its taps clamped to the image (no darkened rim).
[shader("fragment")]
float4 blur_main(Varyings input) : SV_Target
{
    int2 p = int2(input.position.xy);
    int2 step = glass_push.axis == 0 ? int2(1, 0) : int2(0, 1);
    int2 last = int2(round(1.0 / glass_push.texel)) - 1;
    float3 sum = 0.0;
    float weight = 0.0;
    for (int k = -RADIUS; k <= RADIUS; k++)
    {
        float w = exp(-0.5 * float(k * k) / (SIGMA * SIGMA));
        int2 q = clamp(p + step * k, int2(0, 0), last);
        sum += w * g_sampled_images[NonUniformResourceIndex(glass_push.image)].Load(int3(q, 0)).rgb;
        weight += w;
    }
    return float4(sum / weight, 1.0);
}

// The glass over the window: the blurred frame, mostly the tint, mixed as the eye sees it. A PQ
// frame is read as its codes, perceptual already; an sRGB or scRGB one as linear light, where 20 %
// of a bright sky still showed mid-grey: mixed under a 2.2 gamma instead, relative to the
// interface's white.
[shader("fragment")]
float4 glass_main(Varyings input) : SV_Target
{
    float2 uv = input.position.xy * glass_push.texel;
    float3 blurred = sample_image_level(glass_push.image, SAMPLER_LINEAR_CLAMP, uv, 0.0).rgb;
    if (glass_push.encoding == OUTPUT_PQ)
    {
        float3 tint = pq_encode(rec709_to_rec2020(TINT) * glass_push.ui_white);
        return float4(lerp(blurred, tint, OPACITY), 1.0);
    }
    float white = glass_push.encoding == OUTPUT_SCRGB ? glass_push.ui_white / 80.0 : 1.0;
    float3 seen = pow(max(blurred / white, 0.0), 1.0 / 2.2);
    float3 mixed = lerp(seen, pow(TINT, 1.0 / 2.2), OPACITY);
    return float4(pow(mixed, 2.2) * white, 1.0);
}
