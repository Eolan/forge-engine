// ACES 2.0's output transform for a 100-nit SDR Rec.709 display (issue #76), two ways:
// - `aces2_lut`: the transform baked over a 65³ grid on the CPU (crates/forge-render/src/
//   aces2.rs, `bake`), sampled trilinearly. The default: two texture fetches.
// - `aces2_analytic`: the transform per pixel, a line-by-line mirror of the Rust port of
//   OpenColorIO 2.5.2's ACES2 code (BSD-3-Clause, Copyright Contributors to the OpenColorIO
//   Project), with its per-hue tables in a buffer. The reference the table is measured
//   against (`--tonemap aces2-analytic`).
// Keep both in step with aces2.rs.
//
// OpenColorIO's licence, which this port keeps:
//
// Copyright Contributors to the OpenColorIO Project.
//
// Redistribution and use in source and binary forms, with or without modification, are
// permitted provided that the following conditions are met:
//
// * Redistributions of source code must retain the above copyright notice, this list of
//   conditions and the following disclaimer.
// * Redistributions in binary form must reproduce the above copyright notice, this list of
//   conditions and the following disclaimer in the documentation and/or other materials
//   provided with the distribution.
// * Neither the name of the copyright holder nor the names of its contributors may be used
//   to endorse or promote products derived from this software without specific prior
//   written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS
// OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE
// COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE
// GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED
// AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
// NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED
// OF THE POSSIBILITY OF SUCH DAMAGE.
//
// SPDX-License-Identifier: BSD-3-Clause
module aces2;

import bindless;

// The table's grid and shaper (aces2.rs: LUT_SIZE, LUT_EPSILON, LUT_MAX).
static const float LUT_SIZE = 65.0;
static const float LUT_EPSILON = 1.0 / 1024.0;
static const float LUT_MAX = 1024.0;

// The transform's parameters and tables (aces2.rs, `gpu_params`): float4 rows.
public struct Aces2Params
{
    float4 rec709_to_ap1[3];
    float4 ap1_to_ap0[3];
    float4 in_rgb_to_cam16_c[3];
    float4 in_cone_to_aab[3];
    float4 out_aab_to_cone[3];
    float4 out_cam16_c_to_rgb[3];
    float4 model;      // F_L / 100, cz, 1 / cz, A_w_J
    float4 model2;     // 1 / A_w_J, J of the peak, 1 / model gamma, forward limit
    float4 tonescale;  // n_r, g, t_1, s_2
    float4 tonescale2; // m_2, mid J, focus distance, 1 / lower hull gamma
    float4 chroma;     // sat, sat_thr, compr, chroma compress scale
    int4 search;       // the hue search's window (lo, hi)
    float4 table[363]; // per hue entry: hue, cusp J, cusp M, 1 / upper hull gamma
    float4 reach[363]; // per whole degree: reach M (x)
};

static const float PI = 3.14159265358979;
static const float HUE_LIMIT = 360.0;
static const float J_SCALE = 100.0;
static const float CAM_NL_OFFSET = 27.13;
static const float SMOOTH_CUSPS = 0.12;
static const float CUSP_MID_BLEND = 1.3;
static const float FOCUS_GAIN_BLEND = 0.3;
static const float COMPRESSION_THRESHOLD = 0.75;
static const int LOWER_WRAP = 0;
static const int FIRST_NOMINAL = 1;
static const int UPPER_WRAP = 361;

float min_f(float a, float b) { return b < a ? b : a; }
float max_f(float a, float b) { return a < b ? b : a; }
float lerpf(float a, float b, float t) { return (b - a) * t + a; }
float copy_sign(float magnitude, float sign)
{
    return asfloat((asuint(magnitude) & 0x7fffffffu) | (asuint(sign) & 0x80000000u));
}

float3 rows(float4 r0, float4 r1, float4 r2, float3 v)
{
    return float3(
        v.x * r0.x + v.y * r0.y + v.z * r0.z,
        v.x * r1.x + v.y * r1.y + v.z * r1.z,
        v.x * r2.x + v.y * r2.y + v.z * r2.z);
}

float cone_fwd_abs(float rc)
{
    float f_l_y = pow(rc, 0.42);
    return f_l_y / (CAM_NL_OFFSET + f_l_y);
}

float cone_inv_abs(float ra)
{
    float ra_lim = min_f(ra, 0.99);
    float f_l_y = (CAM_NL_OFFSET * ra_lim) / (1.0 - ra_lim);
    return pow(f_l_y, 1.0 / 0.42);
}

float cone_fwd(float v) { return copy_sign(cone_fwd_abs(abs(v)), v); }
float cone_inv(float v) { return copy_sign(cone_inv_abs(abs(v)), v); }

float from_radians(float v)
{
    float y = 180.0 * v / PI;
    return y < 0.0 ? y + HUE_LIMIT : y;
}

float to_radians(float v) { return PI * v / 180.0; }

// The tonescale on the achromatic response, to J.
float tonescale_a_to_j(float a, Aces2Params* p)
{
    float4 model = p[0].model;
    float4 model2 = p[0].model2;
    float4 ts = p[0].tonescale;
    float m_2 = p[0].tonescale2.x;
    float y_in = cone_inv_abs(model.w * a) / model.x;
    float f = m_2 * pow(y_in / (y_in + ts.w), ts.y);
    float y_out = max_f(0.0, f * f / (f + ts.z)) * ts.x;
    float ra = cone_fwd_abs(y_out * model.x);
    float j = J_SCALE * pow(ra * model2.x, model.y);
    return copy_sign(j, a);
}

float chroma_compress_norm(float cos_hr1, float sin_hr1, float scale)
{
    float cos_hr2 = 2.0 * cos_hr1 * cos_hr1 - 1.0;
    float sin_hr2 = 2.0 * cos_hr1 * sin_hr1;
    float cos_hr3 = 4.0 * cos_hr1 * cos_hr1 * cos_hr1 - 3.0 * cos_hr1;
    float sin_hr3 = 3.0 * sin_hr1 - 4.0 * sin_hr1 * sin_hr1 * sin_hr1;
    float m = 11.34072 * cos_hr1 + 16.46899 * cos_hr2 + 7.88380 * cos_hr3 + 14.66441 * sin_hr1
        + -6.37224 * sin_hr2 + 9.19364 * sin_hr3 + 77.12896;
    return m * scale;
}

float toe_fwd(float x, float limit, float k1_in, float k2_in)
{
    if (x > limit)
        return x;
    float k2 = max_f(k2_in, 0.001);
    float k1 = sqrt(k1_in * k1_in + k2 * k2);
    float k3 = (limit + k1) / (limit + k2);
    float minus_b = k3 * x - k1;
    float minus_ac = k2 * k3 * x;
    return 0.5 * (minus_b + sqrt(minus_b * minus_b + 4.0 * minus_ac));
}

float3 chroma_compress(float3 jmh, float j_ts, float m_norm, float reach_max_m, Aces2Params* p)
{
    float limit_j_max = p[0].model2.y;
    float model_gamma_inv = p[0].model2.z;
    float4 c = p[0].chroma;
    float m_cp = jmh.y;
    if (jmh.y != 0.0)
    {
        float n_j = j_ts / limit_j_max;
        float sn_j = max_f(0.0, 1.0 - n_j);
        float limit = pow(n_j, model_gamma_inv) * reach_max_m / m_norm;
        m_cp = jmh.y * pow(j_ts / jmh.x, model_gamma_inv);
        m_cp = m_cp / m_norm;
        m_cp = limit - toe_fwd(limit - m_cp, limit - 0.001, sn_j * c.x, sqrt(n_j * n_j + c.y));
        m_cp = toe_fwd(m_cp, limit, n_j * c.z, sn_j);
        m_cp = m_cp * m_norm;
    }
    return float3(j_ts, m_cp, jmh.z);
}

float get_focus_gain(float j, float analytical_threshold, float limit_j_max, float focus_dist)
{
    float gain = limit_j_max * focus_dist;
    if (j > analytical_threshold)
    {
        float adjustment = log10((limit_j_max - analytical_threshold) / max_f(0.0001, limit_j_max - j));
        adjustment = adjustment * adjustment + 1.0;
        gain = gain * adjustment;
    }
    return gain;
}

float solve_j_intersect(float j, float m, float focus_j, float max_j, float slope_gain)
{
    float m_scaled = m / slope_gain;
    float a = m_scaled / focus_j;
    if (j < focus_j)
    {
        float b = 1.0 - m_scaled;
        float c = -j;
        float det = b * b - 4.0 * a * c;
        return -2.0 * c / (b + sqrt(det));
    }
    float b = -(1.0 + m_scaled + max_j * a);
    float c = max_j * m_scaled + j;
    float det = b * b - 4.0 * a * c;
    return -2.0 * c / (b - sqrt(det));
}

float smin_scaled(float a, float b, float scale_reference)
{
    float s_scaled = SMOOTH_CUSPS * scale_reference;
    float h = max_f(s_scaled - abs(a - b), 0.0) / s_scaled;
    return min_f(a, b) - h * h * h * s_scaled * (1.0 / 6.0);
}

float compression_vector_slope(float intersect_j, float focus_j, float limit_j_max, float slope_gain)
{
    float direction_scaler = intersect_j < focus_j ? intersect_j : limit_j_max - intersect_j;
    return direction_scaler * (intersect_j - focus_j) / (focus_j * slope_gain);
}

float line_boundary_m(float j_axis_intersect, float slope, float inv_gamma, float j_max, float m_max, float j_reference)
{
    float normalised_j = j_axis_intersect / j_reference;
    float shifted_intersection = j_reference * pow(normalised_j, inv_gamma);
    return shifted_intersection * m_max / (j_max - slope * m_max);
}

float gamut_boundary_m(float2 jm_cusp, float j_max, float gamma_top_inv, float gamma_bottom_inv,
                       float j_intersect_source, float slope, float j_intersect_cusp)
{
    float lower = line_boundary_m(j_intersect_source, slope, gamma_bottom_inv, jm_cusp.x, jm_cusp.y, j_intersect_cusp);
    float upper = line_boundary_m(j_max - j_intersect_source, -slope, gamma_top_inv, j_max - jm_cusp.x, jm_cusp.y,
                                  j_max - j_intersect_cusp);
    return smin_scaled(lower, upper, jm_cusp.y);
}

float remap_m(float m, float gamut_boundary, float reach_boundary)
{
    float boundary_ratio = gamut_boundary / reach_boundary;
    float proportion = max_f(boundary_ratio, COMPRESSION_THRESHOLD);
    float threshold = proportion * gamut_boundary;
    if (m <= threshold || proportion >= 1.0)
        return m;
    float m_offset = m - threshold;
    float gamut_offset = gamut_boundary - threshold;
    float reach_offset = reach_boundary - threshold;
    float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0);
    float nd = m_offset / scale;
    return threshold + scale * nd / (1.0 + nd);
}

// The upper index of the hue table's interval that holds `h`.
int lookup_hue_interval(float h, Aces2Params* p)
{
    int2 range = p[0].search.xy;
    int i = FIRST_NOMINAL + int(uint(h));
    int i_lo = max(LOWER_WRAP, i + range.x);
    int i_hi = min(UPPER_WRAP, i + range.y);
    while (i_lo + 1 < i_hi)
    {
        if (h > p[0].table[i].x)
            i_lo = i;
        else
            i_hi = i;
        i = (i_lo + i_hi) / 2;
    }
    return max(1, i_hi);
}

float3 gamut_compress(float3 jmh, float reach_max_m, Aces2Params* p)
{
    float limit_j_max = p[0].model2.y;
    if (jmh.x <= 0.0)
        return float3(0.0, 0.0, jmh.z);
    if (jmh.y <= 0.0 || jmh.x > limit_j_max)
        return float3(jmh.x, 0.0, jmh.z);

    // The hue's parameters.
    int i_hi = lookup_hue_interval(jmh.z, p);
    float4 lo = p[0].table[i_hi - 1];
    float4 hi = p[0].table[i_hi];
    float t = (jmh.z - lo.x) / (hi.x - lo.x);
    float2 jm_cusp = float2(lerpf(lo.y, hi.y, t), lerpf(lo.z, hi.z, t));
    float gamma_top_inv = lerpf(lo.w, hi.w, t);
    float mid_j = p[0].tonescale2.y;
    float focus_dist = p[0].tonescale2.z;
    float gamma_bottom_inv = p[0].tonescale2.w;
    float focus_j = lerpf(jm_cusp.x, mid_j, min_f(1.0, CUSP_MID_BLEND - (jm_cusp.x / limit_j_max)));
    float analytical_threshold = lerpf(jm_cusp.x, limit_j_max, FOCUS_GAIN_BLEND);

    float slope_gain = get_focus_gain(jmh.x, analytical_threshold, limit_j_max, focus_dist);
    float j_intersect_source = solve_j_intersect(jmh.x, jmh.y, focus_j, limit_j_max, slope_gain);
    float gamut_slope = compression_vector_slope(j_intersect_source, focus_j, limit_j_max, slope_gain);
    float j_intersect_cusp = solve_j_intersect(jm_cusp.x, jm_cusp.y, focus_j, limit_j_max, slope_gain);
    float boundary = gamut_boundary_m(jm_cusp, limit_j_max, gamma_top_inv, gamma_bottom_inv, j_intersect_source,
                                      gamut_slope, j_intersect_cusp);
    if (boundary <= 0.0)
        return float3(jmh.x, 0.0, jmh.z);
    float reach_boundary = line_boundary_m(j_intersect_source, gamut_slope, p[0].model2.z, limit_j_max, reach_max_m,
                                           limit_j_max);
    float remapped = remap_m(jmh.y, boundary, reach_boundary);
    return float3(j_intersect_source + remapped * gamut_slope, remapped, jmh.z);
}

// Linear Rec.709 scene colour in, linear Rec.709 display colour in [0, 1] out.
public float3 aces2_analytic(float3 rec709, Aces2Params* p)
{
    float4 model = p[0].model;
    float4 model2 = p[0].model2;
    float limit = model2.w;
    float3 ap1 = rows(p[0].rec709_to_ap1[0], p[0].rec709_to_ap1[1], p[0].rec709_to_ap1[2], rec709);
    ap1 = float3(min_f(max_f(ap1.x, 0.0), limit), min_f(max_f(ap1.y, 0.0), limit), min_f(max_f(ap1.z, 0.0), limit));
    float3 rgb = rows(p[0].ap1_to_ap0[0], p[0].ap1_to_ap0[1], p[0].ap1_to_ap0[2], ap1);

    // RGB to Aab to JMh.
    float3 rgb_m = rows(p[0].in_rgb_to_cam16_c[0], p[0].in_rgb_to_cam16_c[1], p[0].in_rgb_to_cam16_c[2], rgb);
    float3 rgb_a = float3(cone_fwd(rgb_m.x), cone_fwd(rgb_m.y), cone_fwd(rgb_m.z));
    float3 aab = rows(p[0].in_cone_to_aab[0], p[0].in_cone_to_aab[1], p[0].in_cone_to_aab[2], rgb_a);
    float3 jmh = float3(0.0, 0.0, 0.0);
    if (aab.x > 0.0)
        jmh = float3(J_SCALE * pow(aab.x, model.y), sqrt(aab.y * aab.y + aab.z * aab.z), from_radians(atan2(aab.z, aab.y)));

    // The reach at this hue (whole degrees, linear between them).
    uint base = uint(jmh.z);
    float reach_max_m = lerpf(p[0].reach[base + FIRST_NOMINAL].x, p[0].reach[base + FIRST_NOMINAL + 1].x, jmh.z - float(base));

    float h_rad = to_radians(jmh.z);
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    float m_norm = chroma_compress_norm(cos_hr, sin_hr, p[0].chroma.w);
    float j_ts = tonescale_a_to_j(aab.x, p);
    float3 tonemapped = chroma_compress(jmh, j_ts, m_norm, reach_max_m, p);
    float3 compressed = gamut_compress(tonemapped, reach_max_m, p);

    // JMh to Aab to RGB, in the limiting primaries.
    float a_out = pow(compressed.x * (1.0 / J_SCALE), model.z);
    float3 aab_out = float3(a_out, compressed.y * cos_hr, compressed.y * sin_hr);
    float3 cone = rows(p[0].out_aab_to_cone[0], p[0].out_aab_to_cone[1], p[0].out_aab_to_cone[2], aab_out);
    float3 rgb_out = rows(p[0].out_cam16_c_to_rgb[0], p[0].out_cam16_c_to_rgb[1], p[0].out_cam16_c_to_rgb[2],
                          float3(cone_inv(cone.x), cone_inv(cone.y), cone_inv(cone.z)));
    return float3(min_f(max_f(rgb_out.x, 0.0), 1.0), min_f(max_f(rgb_out.y, 0.0), 1.0), min_f(max_f(rgb_out.z, 0.0), 1.0));
}

float3 decode_srgb(float3 encoded)
{
    float3 low = encoded / 12.92;
    float3 high = pow((encoded + 0.055) / 1.055, 2.4);
    return select(encoded <= 0.04045, low, high);
}

// The baked table: a 2D image of LUT_SIZE slices of LUT_SIZE² texels side by side (slice =
// blue, red along x, green along y), sRGB-encoded. Trilinear: the sampler interpolates red
// and green within two slices, the shader blends the slices.
public float3 aces2_lut(float3 rec709, uint lut)
{
    float lo = log2(LUT_EPSILON);
    float hi = log2(LUT_MAX + LUT_EPSILON);
    float3 shaped = saturate((log2(clamp(rec709, 0.0, LUT_MAX) + LUT_EPSILON) - lo) / (hi - lo));
    float3 u = shaped * (LUT_SIZE - 1.0);
    float slice = min(floor(u.z), LUT_SIZE - 2.0);
    float f = u.z - slice;
    float width = LUT_SIZE * LUT_SIZE;
    float2 uv = float2((u.x + 0.5 + slice * LUT_SIZE) / width, (u.y + 0.5) / LUT_SIZE);
    float3 c0 = sample_image_level(lut, SAMPLER_LINEAR_CLAMP, uv, 0.0).rgb;
    float3 c1 = sample_image_level(lut, SAMPLER_LINEAR_CLAMP, uv + float2(LUT_SIZE / width, 0.0), 0.0).rgb;
    return decode_srgb(lerp(c0, c1, f));
}
