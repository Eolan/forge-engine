// Display transforms: scene-referred, pre-exposed linear colour in, display-referred linear
// colour in [0, 1] out (an sRGB swapchain encodes on write; `display_output` encodes when the
// target is UNORM). Chosen at run time by index, never baked into lighting. CPU mirrors with
// tests: crates/forge-render/src/display.rs (keep the two in step).
module tonemap;

import aces2;
import noise;

public static const uint TONEMAP_AGX = 0;
public static const uint TONEMAP_ACES = 1;
public static const uint TONEMAP_PBR_NEUTRAL = 2;
// ACES 2.0's output transform through its baked table, and per pixel (the reference).
public static const uint TONEMAP_ACES2 = 3;
public static const uint TONEMAP_ACES2_ANALYTIC = 4;
// AgX with its "punchy" look: deeper blacks, more saturated colours (D-045).
public static const uint TONEMAP_AGX_PUNCHY = 5;

// What the ACES 2.0 curves read (crates/forge-render/src/display.rs, `ToneTables`): the
// baked tables' bindless indices and the transform's parameters, SDR and the HDR preset's.
public struct ToneTables
{
    public uint lut;
    public uint hdr_lut;      // the HDR preset's table: the PQ signal, RGBA16 UNORM
    public Aces2Params* aces2;
    public Aces2Params* hdr;  // the HDR preset's parameters (aces2.rs, `Hdr::gpu_params`)
    public float hdr_max;     // the HDR table's top (aces2.rs, `Hdr::lut_max`)
    public uint pad;
};

// How the display image is encoded (display.rs, `OutputEncoding`, issue #94).
public static const uint OUTPUT_SRGB_TARGET = 0; // the target encodes sRGB on write
public static const uint OUTPUT_SRGB = 1;        // a UNORM SDR target: the shader encodes
public static const uint OUTPUT_PQ = 2;          // HDR10: Rec.2100 PQ, Rec.2020 primaries
public static const uint OUTPUT_SCRGB = 3;       // scRGB: linear Rec.709, 1.0 being 80 nits

// The display's side of the output (display.rs, `OutputPush`).
public struct Output
{
    public uint encoding;     // OUTPUT_*
    public float scene_gain;  // HDR: the paper-white offset (2^stops), before ACES 2.0
    public float sdr_white;   // HDR: the nits of an SDR curve's white (the UI's paper white)
    public uint frame;        // the dither's frame
};

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

// With `punchy`, Wrensch's "punchy" look between the sigmoid and the outset (as Filament's
// `AgxLook::PUNCHY`): an ASC CDL of power 1.35, then saturation 1.4 about the luma before it.
float3 agx(float3 color, bool punchy)
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
    if (punchy)
    {
        float luma = dot(v, float3(0.2126, 0.7152, 0.0722));
        v = luma + 1.4 * (pow(max(v, 0.0), 1.35) - luma);
    }
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

public float3 tonemap(float3 color, uint curve, ToneTables tables)
{
    if (curve == TONEMAP_ACES)
        return aces(color);
    if (curve == TONEMAP_PBR_NEUTRAL)
        return pbr_neutral(color);
    if (curve == TONEMAP_ACES2)
        return aces2_lut(max(color, 0.0), tables.lut);
    if (curve == TONEMAP_ACES2_ANALYTIC)
        return aces2_analytic(max(color, 0.0), tables.aces2);
    return agx(color, curve == TONEMAP_AGX_PUNCHY);
}

public float3 encode_srgb(float3 linear)
{
    float3 low = linear * 12.92;
    float3 high = 1.055 * pow(linear, 1.0 / 2.4) - 0.055;
    return select(linear <= 0.0031308, low, high);
}

// SMPTE ST 2084 (PQ), as aces2.rs: nits in, the signal in [0, 1] out, and back.
public float3 pq_encode(float3 nits)
{
    const float m1 = 2610.0 / 16384.0;
    const float m2 = 2523.0 / 4096.0 * 128.0;
    const float c1 = 3424.0 / 4096.0;
    const float c2 = 2413.0 / 4096.0 * 32.0;
    const float c3 = 2392.0 / 4096.0 * 32.0;
    float3 p = pow(saturate(nits / 10000.0), m1);
    return pow((c1 + c2 * p) / (1.0 + c3 * p), m2);
}

public float3 pq_decode(float3 signal)
{
    const float m1 = 2610.0 / 16384.0;
    const float m2 = 2523.0 / 4096.0 * 128.0;
    const float c1 = 3424.0 / 4096.0;
    const float c2 = 2413.0 / 4096.0 * 32.0;
    const float c3 = 2392.0 / 4096.0 * 32.0;
    float3 e = pow(saturate(signal), 1.0 / m2);
    return pow(max(e - c1, 0.0) / (c2 - c3 * e), 1.0 / m1) * 10000.0;
}

// Between Rec.709 and Rec.2020 (both D65; display.rs, `REC709_TO_REC2020`, checked against
// aces2.rs's conversion).
public float3 rec709_to_rec2020(float3 c)
{
    return float3(
        dot(c, float3(0.6274039, 0.32928304, 0.04331307)),
        dot(c, float3(0.06909729, 0.9195404, 0.01136232)),
        dot(c, float3(0.01639144, 0.08801331, 0.89559525)));
}

public float3 rec2020_to_rec709(float3 c)
{
    return float3(
        dot(c, float3(1.660491, -0.5876411, -0.07284986)),
        dot(c, float3(-0.12455047, 1.1328999, -0.00834942)),
        dot(c, float3(-0.01815076, -0.1005789, 1.1187297)));
}

// The HDR signal of a pre-exposed colour: ACES 2.0 through the preset (the scene raised by the
// paper-white offset first), any other curve as its SDR image at the UI's white.
float3 hdr_signal(float3 color, uint curve, Output output, ToneTables tables)
{
    if (curve == TONEMAP_ACES2)
        return aces2_hdr_lut(max(color * output.scene_gain, 0.0), tables.hdr_lut, tables.hdr_max);
    if (curve == TONEMAP_ACES2_ANALYTIC)
        return pq_encode(aces2_analytic_hdr(max(color * output.scene_gain, 0.0), tables.hdr) * 100.0);
    float3 display = saturate(tonemap(color, curve, tables));
    return pq_encode(rec709_to_rec2020(display) * output.sdr_white);
}

// The display value of a pre-exposed HDR colour at `pixel`: the curve, then the encoding.
// SDR: the sRGB encoding when the target does not do it on write. PQ: half a 10-bit code of
// dither, so smooth gradients do not band. scRGB: PQ's luminance back in linear Rec.709.
public float3 display_output(float3 color, uint curve, Output output, ToneTables tables, uint2 pixel)
{
    if (output.encoding < OUTPUT_PQ)
    {
        float3 display = saturate(tonemap(color, curve, tables));
        return output.encoding == OUTPUT_SRGB ? encode_srgb(display) : display;
    }
    float3 signal = hdr_signal(color, curve, output, tables);
    if (output.encoding == OUTPUT_PQ)
    {
        float dither = spatio_temporal_noise(pixel, output.frame).x - 0.5;
        return saturate(signal + dither * (1.0 / 1023.0));
    }
    return rec2020_to_rec709(pq_decode(signal)) / 80.0;
}

public float3 decode_srgb(float3 encoded)
{
    float3 low = encoded / 12.92;
    float3 high = pow((encoded + 0.055) / 1.055, 2.4);
    return select(encoded <= 0.04045, low, high);
}

// The display's signal of a pre-exposed colour, in [0, 1] and even to the eye: an SDR curve's
// image sRGB-encoded, or the PQ signal. A sharpening pass works on it (taa.slang, D-045), then
// `display_from_signal` finishes it as `display_output` would.
public float3 display_signal(float3 color, uint curve, Output output, ToneTables tables)
{
    if (output.encoding < OUTPUT_PQ)
        return encode_srgb(saturate(tonemap(color, curve, tables)));
    return hdr_signal(color, curve, output, tables);
}

public float3 display_from_signal(float3 signal, Output output, uint2 pixel)
{
    if (output.encoding == OUTPUT_SRGB)
        return signal;
    if (output.encoding == OUTPUT_SRGB_TARGET)
        return decode_srgb(signal);
    if (output.encoding == OUTPUT_PQ)
    {
        float dither = spatio_temporal_noise(pixel, output.frame).x - 0.5;
        return saturate(signal + dither * (1.0 / 1023.0));
    }
    return rec2020_to_rec709(pq_decode(signal)) / 80.0;
}
