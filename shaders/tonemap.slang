// Display transforms: scene-referred, pre-exposed linear colour in, display-referred linear
// colour in [0, 1] out (an sRGB swapchain encodes on write; `display_output` encodes when the
// target is UNORM). Chosen at run time by index, never baked into lighting. CPU mirrors with
// tests: crates/forge-render/src/display.rs (keep the two in step).
module tonemap;

public static const uint TONEMAP_AGX = 0;
public static const uint TONEMAP_ACES = 1;
public static const uint TONEMAP_PBR_NEUTRAL = 2;

// AgX (Sobotka; Blender 4.0's default view), in the minimal real-time form of Wrensch 2023:
// inset matrix, log2 encoding over [-12.47, +4.03] stops (0.18 · 2^-10 to 0.18 · 2^6.5), the
// default contrast sigmoid fitted by a 6th-order polynomial, outset matrix, 2.2 display
// decode. Matrices written as rows so no matrix-layout convention is involved.
float3 agx_contrast(float3 x)
{
    float3 x2 = x * x;
    float3 x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

float3 agx(float3 color)
{
    const float min_ev = -12.47393;
    const float max_ev = 4.026069;
    float3 c = max(color, 0.0);
    float3 v = float3(
        dot(c, float3(0.842479062253094, 0.0784335999999992, 0.0792237451477643)),
        dot(c, float3(0.0423282422610123, 0.878468636469772, 0.0791661274605434)),
        dot(c, float3(0.0423756549057051, 0.0784336, 0.879142973793104)));
    v = clamp(log2(max(v, 1e-10)), min_ev, max_ev);
    v = agx_contrast((v - min_ev) / (max_ev - min_ev));
    v = float3(
        dot(v, float3(1.19687900512017, -0.0980208811401368, -0.0990297440797205)),
        dot(v, float3(-0.0528968517574562, 1.15190312990417, -0.0989611768448433)),
        dot(v, float3(-0.0529716355144438, -0.0980434501171241, 1.15107367264116)));
    return pow(max(v, 0.0), 2.2);
}

// ACES 1.x reference rendering + sRGB output transform, as fitted by Stephen Hill (BakingLab,
// MIT): sRGB → AP1 with the RRT saturation, a rational fit of RRT + ODT, back to sRGB.
// (ACES 2.0's output transform is not implemented; this is the look games shipped as "ACES".)
float3 aces(float3 color)
{
    float3 c = max(color, 0.0);
    float3 v = float3(
        dot(c, float3(0.59719, 0.35458, 0.04823)),
        dot(c, float3(0.07600, 0.90834, 0.01566)),
        dot(c, float3(0.02840, 0.13383, 0.83777)));
    float3 a = v * (v + 0.0245786) - 0.000090537;
    float3 b = v * (0.983729 * v + 0.4329510) + 0.238081;
    v = a / b;
    v = float3(
        dot(v, float3(1.60475, -0.53108, -0.07367)),
        dot(v, float3(-0.10208, 1.10813, -0.00605)),
        dot(v, float3(-0.00327, -0.07276, 1.07602)));
    return saturate(v);
}

// Khronos PBR Neutral (2024): colours up to about 0.76 come out as they went in (less a small
// offset in the blacks), brighter ones are compressed towards white with little hue shift.
float3 pbr_neutral(float3 color)
{
    const float start_compression = 0.8 - 0.04;
    const float desaturation = 0.15;
    float3 c = max(color, 0.0);
    float lowest = min(c.r, min(c.g, c.b));
    float offset = lowest < 0.08 ? lowest - 6.25 * lowest * lowest : 0.04;
    c -= offset;
    float peak = max(c.r, max(c.g, c.b));
    if (peak < start_compression)
        return c;
    const float d = 1.0 - start_compression;
    float new_peak = 1.0 - d * d / (peak + d - start_compression);
    c *= new_peak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return lerp(c, float3(new_peak, new_peak, new_peak), g);
}

public float3 tonemap(float3 color, uint curve)
{
    if (curve == TONEMAP_ACES)
        return aces(color);
    if (curve == TONEMAP_PBR_NEUTRAL)
        return pbr_neutral(color);
    return agx(color);
}

public float3 encode_srgb(float3 linear)
{
    float3 low = linear * 12.92;
    float3 high = 1.055 * pow(linear, 1.0 / 2.4) - 0.055;
    return select(linear <= 0.0031308, low, high);
}

// The display value of a pre-exposed HDR colour: the curve, then the sRGB encoding when the
// target does not do it on write.
public float3 display_output(float3 color, uint curve, bool encode)
{
    float3 display = saturate(tonemap(color, curve));
    return encode ? encode_srgb(display) : display;
}
