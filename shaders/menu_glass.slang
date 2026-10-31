// The frosted glass behind the game's menu (#377, the owner: "a glass gaussian transparency
// effect but almost opaque", "full screen"), driven by crates/forge-app/src/glass.rs: the frame
// brought down to a quarter of its size (each pixel the mean of a 4 x 4 block, so a moving
// picture behind it does not shimmer), blurred by a Gaussian across and then down, and drawn
// back over the whole window under a dark tint, nearly opaque. An sRGB frame is read as linear
// light and written back encoded; in HDR the tint is encoded as the HUD's colours are. The menu's
// Carbon look (crates/forge-app/src/menu/carbon.rs) tints it as the owner's streaming overlays'
// page: two glows and a vignette.
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
    uint theme;      // the menu's look: 0 base, THEME_CARBON
    float pad;
};

static const uint THEME_CARBON = 1;

[[vk::push_constant]]
GlassPush glass_push;

// The blur's reach, quarter-size pixels: sigma 4 (16 of the window's), out to three sigmas.
static const int RADIUS = 12;
static const float SIGMA = 4.0;
// The glass: this much the tint, the rest the blurred game.
static const float OPACITY = 0.80;
static const float3 TINT = float3(0.010, 0.012, 0.016);
// Carbon's colours as a page has them, sRGB codes: the dark, the violet and the ion blue.
static const float3 VOID = float3(10.0, 11.0, 14.0) / 255.0;
static const float3 NEBULA = float3(139.0, 124.0, 246.0) / 255.0;
static const float3 ION = float3(79.0, 216.0, 255.0) / 255.0;

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

// A page's sRGB code as seen here: decoded to light, then under the mix's 2.2 gamma.
float3 page(float3 code)
{
    return pow(decode_srgb(code), 1.0 / 2.2);
}

// How far a page's radial gradient centred at `centre` (of the window `size`) has gone at `p`:
// 0 at its centre, 1 at the window's farthest corner (CSS's `farthest-corner`).
float reach(float2 p, float2 size, float2 centre)
{
    float2 c = centre * size;
    return length(p - c) / length(max(c, size - c));
}

// Carbon's page over the glass at pixel `p`, as seen: the dark (the glass's tint), a violet glow
// at the top right, an ion blue one at the bottom left, and a vignette (without the page's grid:
// the owner); dithered, so the glows' long ramps do not band in 8 bits.
float3 carbon(float3 seen, float2 p, float2 size)
{
    float3 dark = page(VOID);
    float3 c = lerp(seen, dark, OPACITY);
    c = lerp(c, page(NEBULA), 0.18 * saturate(1.0 - reach(p, size, float2(0.80, 0.14)) / 0.46));
    c = lerp(c, page(ION), 0.13 * saturate(1.0 - reach(p, size, float2(0.12, 0.90)) / 0.44));
    float2 e = (p - float2(0.5, 0.42) * size) / (float2(1.2, 0.92) * size);
    c = lerp(c, dark, 0.42 * saturate((length(e) - 0.58) / 0.42));
    // Interleaved gradient noise (Jimenez 2014), half a code each way.
    float dither = frac(52.9829189 * frac(dot(p, float2(0.06711056, 0.00583715))));
    return c + (dither - 0.5) / 255.0;
}

// The glass over the window: the blurred frame, mostly the tint, mixed as the eye sees it. A PQ
// frame is read as its codes, perceptual already; an sRGB or scRGB one as linear light, where 20 %
// of a bright sky still showed mid-grey: mixed under a 2.2 gamma instead, relative to the
// interface's white. Carbon's page is drawn as seen in every encoding (a PQ frame decoded first).
[shader("fragment")]
float4 glass_main(Varyings input) : SV_Target
{
    float2 uv = input.position.xy * glass_push.texel;
    float3 blurred = sample_image_level(glass_push.image, SAMPLER_LINEAR_CLAMP, uv, 0.0).rgb;
    float white = glass_push.encoding == OUTPUT_SCRGB ? glass_push.ui_white / 80.0 : 1.0;
    if (glass_push.theme == THEME_CARBON)
    {
        float2 size = 1.0 / glass_push.texel;
        float3 light = glass_push.encoding == OUTPUT_PQ
            ? max(rec2020_to_rec709(pq_decode(blurred)), 0.0) / glass_push.ui_white
            : max(blurred / white, 0.0);
        float3 seen = carbon(pow(light, 1.0 / 2.2), input.position.xy, size);
        float3 back = pow(max(seen, 0.0), 2.2);
        if (glass_push.encoding == OUTPUT_PQ)
            return float4(pq_encode(rec709_to_rec2020(back) * glass_push.ui_white), 1.0);
        return float4(back * white, 1.0);
    }
    if (glass_push.encoding == OUTPUT_PQ)
    {
        float3 tint = pq_encode(rec709_to_rec2020(TINT) * glass_push.ui_white);
        return float4(lerp(blurred, tint, OPACITY), 1.0);
    }
    float3 seen = pow(max(blurred / white, 0.0), 1.0 / 2.2);
    float3 mixed = lerp(seen, pow(TINT, 1.0 / 2.2), OPACITY);
    return float4(pow(mixed, 2.2) * white, 1.0);
}
