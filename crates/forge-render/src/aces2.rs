//! ACES 2.0's output transform (issue #76), the fourth tone curve: scene-referred linear
//! Rec.709 in, display-linear Rec.709 for a 100-nit SDR display out.
//!
//! A port of OpenColorIO 2.5.2's ACES 2.0 fixed function
//! (`src/OpenColorIO/ops/fixedfunction/ACES2/`, BSD-3-Clause, Copyright Contributors to the
//! OpenColorIO Project), which implements the Academy's CTL (`aces-aswf/aces-core` v2.0,
//! Apache-2.0). Its stages:
//! 1. RGB to JMh through a simplified Hellwig 2022 appearance model (fixed viewing
//!    conditions, custom cone primaries).
//! 2. The tonescale (Siragusano's Michaelis–Menten curve with a flare toe) on the achromatic
//!    response: at 100 nits, 0.18 gives 10 nits.
//! 3. Chroma compression of M: expansion in the shadows, compression in the highlights,
//!    bounded by a reach table (the path to white).
//! 4. Gamut compression of J and M towards a focus point, against a cusp table and a per-hue
//!    upper-hull gamma.
//! 5. Back to RGB in the limiting primaries.
//!
//! The per-hue tables are built on the CPU exactly as OCIO builds them. The arithmetic
//! follows OCIO in `f32` and in the same order, so OCIO's own test values apply (the tests
//! below). On the GPU, `tonemap.slang` samples a table [`bake`] evaluates over a 65³ grid
//! once; `aces2.slang` also mirrors the transform per pixel, as the reference.
//!
//! OpenColorIO's licence, which this port keeps:
//!
//! Copyright Contributors to the OpenColorIO Project.
//!
//! Redistribution and use in source and binary forms, with or without modification, are
//! permitted provided that the following conditions are met:
//!
//! * Redistributions of source code must retain the above copyright notice, this list of
//!   conditions and the following disclaimer.
//! * Redistributions in binary form must reproduce the above copyright notice, this list of
//!   conditions and the following disclaimer in the documentation and/or other materials
//!   provided with the distribution.
//! * Neither the name of the copyright holder nor the names of its contributors may be used
//!   to endorse or promote products derived from this software without specific prior
//!   written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS
//! OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF
//! MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE
//! COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE
//! GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED
//! AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED
//! OF THE POSSIBILITY OF SUCH DAMAGE.
//!
//! SPDX-License-Identifier: BSD-3-Clause

use std::f32::consts::PI;
use std::sync::OnceLock;
use std::thread;

type F2 = [f32; 2];
type F3 = [f32; 3];
/// Row-major: `mul3(v, m)` is `m · v`, as OCIO's `mult_f3_f33`.
type M33 = [f32; 9];
type D33 = [[f64; 3]; 3];

/// Chromaticities (x, y) of a colour space's red, green and blue primaries and its white.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    /// Red (x, y).
    pub red: [f64; 2],
    /// Green (x, y).
    pub green: [f64; 2],
    /// Blue (x, y).
    pub blue: [f64; 2],
    /// White (x, y).
    pub white: [f64; 2],
}

/// ACES AP0 (SMPTE ST 2065-1): ACES2065-1, the transform's input space.
pub const AP0: Primaries = Primaries {
    red: [0.7347, 0.2653],
    green: [0.0, 1.0],
    blue: [0.0001, -0.0770],
    white: [0.32168, 0.33767],
};
/// ACES AP1 (ACEScg): where the input is clamped, and the reach gamut.
pub const AP1: Primaries = Primaries {
    red: [0.713, 0.293],
    green: [0.165, 0.830],
    blue: [0.128, 0.044],
    white: [0.32168, 0.33767],
};
/// Rec.709 / sRGB, D65: Forge's working and display primaries.
pub const REC709: Primaries = Primaries {
    red: [0.64, 0.33],
    green: [0.30, 0.60],
    blue: [0.15, 0.06],
    white: [0.3127, 0.3290],
};
/// Display P3 with a D65 white.
pub const P3_D65: Primaries = Primaries {
    red: [0.680, 0.320],
    green: [0.265, 0.690],
    blue: [0.150, 0.060],
    white: [0.3127, 0.3290],
};
/// The appearance model's cone primaries.
const CAM16: Primaries = Primaries {
    red: [0.8336, 0.1735],
    green: [2.3854, -1.4659],
    blue: [0.087, -0.125],
    white: [0.333, 0.333],
};

// The appearance model (`Common.h`).
const REFERENCE_LUMINANCE: f32 = 100.0;
const L_A: f32 = 100.0;
const Y_B: f32 = 20.0;
const SURROUND: F3 = [0.9, 0.59, 0.9];
const J_SCALE: f32 = 100.0;
const CAM_NL_Y_REFERENCE: f32 = 100.0;
const CAM_NL_OFFSET: f32 = 0.2713 * CAM_NL_Y_REFERENCE;
const CAM_NL_SCALE: f32 = 4.0 * CAM_NL_Y_REFERENCE;
// Chroma compression.
const CHROMA_COMPRESS: f32 = 2.4;
const CHROMA_COMPRESS_FACT: f32 = 3.3;
const CHROMA_EXPAND: f32 = 1.3;
const CHROMA_EXPAND_FACT: f32 = 0.69;
const CHROMA_EXPAND_THR: f32 = 0.5;
// Gamut compression.
const SMOOTH_CUSPS: f32 = 0.12;
const SMOOTH_M: f32 = 0.27;
const CUSP_MID_BLEND: f32 = 1.3;
const FOCUS_GAIN_BLEND: f32 = 0.3;
const FOCUS_DISTANCE: f32 = 1.35;
const FOCUS_DISTANCE_SCALING: f32 = 1.75;
const COMPRESSION_THRESHOLD: f32 = 0.75;
// Table generation.
const HUE_LIMIT: f32 = 360.0;
const GAMMA_MINIMUM: f32 = 0.0;
const GAMMA_MAXIMUM: f32 = 5.0;
const GAMMA_SEARCH_STEP: f32 = 0.4;
const GAMMA_ACCURACY: f32 = 1e-5;
const CUSP_CORNER_COUNT: usize = 6;
const TOTAL_CORNER_COUNT: usize = CUSP_CORNER_COUNT + 2;
const MAX_SORTED_CORNERS: usize = 2 * CUSP_CORNER_COUNT;
const REACH_CUSP_TOLERANCE: f32 = 1e-3;
const DISPLAY_CUSP_TOLERANCE: f32 = 1e-7;

// The per-hue tables: 360 nominal entries, one wrap entry below and two above.
const NOMINAL_SIZE: usize = 360;
const TOTAL_SIZE: usize = NOMINAL_SIZE + 3;
const LOWER_WRAP: usize = 0;
const FIRST_NOMINAL: usize = 1;
const UPPER_WRAP: usize = FIRST_NOMINAL + NOMINAL_SIZE;
const LAST_NOMINAL: usize = UPPER_WRAP - 1;

// `std::min` and `std::max`, NaN behaviour included.
fn min_f(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

fn max_f(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}

fn lerpf(a: f32, b: f32, t: f32) -> f32 {
    (b - a) * t + a
}

fn midpoint(a: f32, b: f32) -> f32 {
    (a + b) / 2.0
}

// ---- Matrices ------------------------------------------------------------------------------

fn d_mul(a: &D33, b: &D33) -> D33 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn d_apply(m: &D33, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn d_inverse(m: &D33) -> D33 {
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let cofactor = [
        [c(1, 2, 1, 2), -c(0, 2, 1, 2), c(0, 1, 1, 2)],
        [-c(1, 2, 0, 2), c(0, 2, 0, 2), -c(0, 1, 0, 2)],
        [c(1, 2, 0, 1), -c(0, 2, 0, 1), c(0, 1, 0, 1)],
    ];
    let det = m[0][0] * cofactor[0][0] + m[0][1] * cofactor[1][0] + m[0][2] * cofactor[2][0];
    cofactor.map(|row| row.map(|v| v / det))
}

fn to_m33(m: &D33) -> M33 {
    std::array::from_fn(|i| m[i / 3][i % 3] as f32)
}

fn from_m33(m: &M33) -> D33 {
    std::array::from_fn(|i| std::array::from_fn(|j| f64::from(m[i * 3 + j])))
}

/// RGB to XYZ (Y of white = 1), OCIO's `rgb2xyz_from_xy`.
fn rgb_to_xyz(p: &Primaries) -> D33 {
    let [r, g, b, w] = [p.red, p.green, p.blue, p.white];
    let m = [
        [r[0], g[0], b[0]],
        [r[1], g[1], b[1]],
        [1.0 - r[0] - r[1], 1.0 - g[0] - g[1], 1.0 - b[0] - b[1]],
    ];
    let inv = d_inverse(&m);
    let white = [w[0] / w[1], 1.0, (1.0 - w[0] - w[1]) / w[1]];
    let gains: [f64; 3] =
        std::array::from_fn(|i| white[0] * inv[i][0] + white[1] * inv[i][1] + white[2] * inv[i][2]);
    std::array::from_fn(|j| std::array::from_fn(|i| gains[i] * m[j][i]))
}

/// RGB in `src` to RGB in `dst`, with Bradford adaptation between their whites when asked
/// (OCIO's `build_conversion_matrix`).
fn conversion(src: &Primaries, dst: &Primaries, bradford: bool) -> D33 {
    let src_to_xyz = rgb_to_xyz(src);
    let dst_to_xyz = rgb_to_xyz(dst);
    let xyz_to_dst = d_inverse(&dst_to_xyz);
    if src.white == dst.white || !bradford {
        return d_mul(&xyz_to_dst, &src_to_xyz);
    }
    const BRADFORD: D33 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let src_white = d_apply(&BRADFORD, d_apply(&src_to_xyz, [1.0; 3]));
    let dst_white = d_apply(&BRADFORD, d_apply(&dst_to_xyz, [1.0; 3]));
    let mut scale = [[0.0; 3]; 3];
    for i in 0..3 {
        scale[i][i] = dst_white[i] / src_white[i];
    }
    let adapt = d_mul(&d_inverse(&BRADFORD), &d_mul(&scale, &BRADFORD));
    d_mul(&xyz_to_dst, &d_mul(&adapt, &src_to_xyz))
}

fn mul3(v: F3, m: &M33) -> F3 {
    [
        v[0] * m[0] + v[1] * m[1] + v[2] * m[2],
        v[0] * m[3] + v[1] * m[4] + v[2] * m[5],
        v[0] * m[6] + v[1] * m[7] + v[2] * m[8],
    ]
}

fn mul33(a: &M33, b: &M33) -> M33 {
    std::array::from_fn(|i| {
        let (row, col) = (i / 3, i % 3);
        a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col]
    })
}

fn diag(s: F3) -> M33 {
    [s[0], 0.0, 0.0, 0.0, s[1], 0.0, 0.0, 0.0, s[2]]
}

fn invert(m: &M33) -> M33 {
    to_m33(&d_inverse(&from_m33(m)))
}

// ---- The appearance model ------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct JmhParams {
    rgb_to_cam16_c: M33,
    cam16_c_to_rgb: M33,
    cone_response_to_aab: M33,
    aab_to_cone_response: M33,
    f_l_n: f32,
    cz: f32,
    inv_cz: f32,
    a_w_j: f32,
    inv_a_w_j: f32,
}

fn cone_compress_fwd_abs(rc: f32) -> f32 {
    let f_l_y = rc.powf(0.42);
    f_l_y / (CAM_NL_OFFSET + f_l_y)
}

fn cone_compress_inv_abs(ra: f32) -> f32 {
    let ra_lim = min_f(ra, 0.99);
    let f_l_y = (CAM_NL_OFFSET * ra_lim) / (1.0 - ra_lim);
    f_l_y.powf(1.0 / 0.42)
}

fn cone_compress_fwd(v: f32) -> f32 {
    cone_compress_fwd_abs(v.abs()).copysign(v)
}

fn cone_compress_inv(v: f32) -> f32 {
    cone_compress_inv_abs(v.abs()).copysign(v)
}

fn achromatic_n_to_j(a: f32, cz: f32) -> f32 {
    J_SCALE * a.powf(cz)
}

fn j_to_achromatic_n(j: f32, inv_cz: f32) -> f32 {
    (j * (1.0 / J_SCALE)).powf(inv_cz)
}

fn a_to_y(a: f32, p: &JmhParams) -> f32 {
    cone_compress_inv_abs(p.a_w_j * a) / p.f_l_n
}

fn y_to_j_abs(abs_y: f32, p: &JmhParams) -> f32 {
    let ra = cone_compress_fwd_abs(abs_y * p.f_l_n);
    achromatic_n_to_j(ra * p.inv_a_w_j, p.cz)
}

fn y_to_j(y: f32, p: &JmhParams) -> f32 {
    y_to_j_abs(y.abs(), p).copysign(y)
}

fn model_gamma() -> f32 {
    SURROUND[1] * (1.48 + (Y_B / REFERENCE_LUMINANCE).sqrt())
}

fn to_radians(v: f32) -> f32 {
    PI * v / 180.0
}

/// Degrees from `atan2`'s radians, wrapped once into [0, 360].
fn from_radians(v: f32) -> f32 {
    let y = 180.0 * v / PI;
    if y < 0.0 { y + HUE_LIMIT } else { y }
}

impl JmhParams {
    fn new(prims: &Primaries) -> Self {
        let base: M33 = [
            2.0,
            1.0,
            1.0 / 20.0,
            1.0,
            -12.0 / 11.0,
            1.0 / 11.0,
            1.0 / 9.0,
            1.0 / 9.0,
            -2.0 / 9.0,
        ];
        let matrix_16 = to_m33(&d_inverse(&rgb_to_xyz(&CAM16)));
        let rgb_to_xyz_m = to_m33(&rgb_to_xyz(prims));
        let xyz_w = mul3([REFERENCE_LUMINANCE; 3], &rgb_to_xyz_m);
        let y_w = xyz_w[1];
        let rgb_w = mul3(xyz_w, &matrix_16);

        // The viewing conditions.
        const K: f32 = 1.0 / (5.0 * L_A + 1.0);
        const K4: f32 = K * K * K * K;
        let f_l = 0.2 * K4 * (5.0 * L_A) + 0.1 * (1.0 - K4).powf(2.0) * (5.0 * L_A).powf(1.0 / 3.0);
        let f_l_n = f_l / REFERENCE_LUMINANCE;
        let cz = model_gamma();
        let inv_cz = 1.0 / cz;

        let d_rgb = [0, 1, 2].map(|i| f_l_n * y_w / rgb_w[i]);
        let rgb_wc = [0, 1, 2].map(|i| d_rgb[i] * rgb_w[i]);
        let rgb_aw = rgb_wc.map(cone_compress_fwd);

        let cone_to_aab = mul33(&diag([CAM_NL_SCALE; 3]), &base);
        let a_w =
            cone_to_aab[0] * rgb_aw[0] + cone_to_aab[1] * rgb_aw[1] + cone_to_aab[2] * rgb_aw[2];
        let a_w_j = cone_compress_fwd_abs(f_l);

        // The CAM16 responses are prescaled for the chromatic adaptation.
        let rgb_to_cam16 = mul33(
            &mul33(&matrix_16, &rgb_to_xyz_m),
            &diag([REFERENCE_LUMINANCE; 3]),
        );
        let rgb_to_cam16_c = mul33(&diag(d_rgb), &rgb_to_cam16);
        let c = cone_to_aab;
        let s = SURROUND[2];
        let cone_response_to_aab = [
            c[0] / a_w,
            c[1] / a_w,
            c[2] / a_w,
            c[3] * 43.0 * s,
            c[4] * 43.0 * s,
            c[5] * 43.0 * s,
            c[6] * 43.0 * s,
            c[7] * 43.0 * s,
            c[8] * 43.0 * s,
        ];
        Self {
            rgb_to_cam16_c,
            cam16_c_to_rgb: invert(&rgb_to_cam16_c),
            cone_response_to_aab,
            aab_to_cone_response: invert(&cone_response_to_aab),
            f_l_n,
            cz,
            inv_cz,
            a_w_j,
            inv_a_w_j: 1.0 / a_w_j,
        }
    }
}

fn rgb_to_aab(rgb: F3, p: &JmhParams) -> F3 {
    let rgb_m = mul3(rgb, &p.rgb_to_cam16_c);
    mul3(rgb_m.map(cone_compress_fwd), &p.cone_response_to_aab)
}

fn aab_to_jmh(aab: F3, p: &JmhParams) -> F3 {
    if aab[0] <= 0.0 {
        return [0.0; 3];
    }
    let j = achromatic_n_to_j(aab[0], p.cz);
    let m = (aab[1] * aab[1] + aab[2] * aab[2]).sqrt();
    let h = from_radians(aab[2].atan2(aab[1]));
    [j, m, h]
}

fn rgb_to_jmh(rgb: F3, p: &JmhParams) -> F3 {
    aab_to_jmh(rgb_to_aab(rgb, p), p)
}

fn jmh_to_aab_cs(jmh: F3, cos_hr: f32, sin_hr: f32, p: &JmhParams) -> F3 {
    [
        j_to_achromatic_n(jmh[0], p.inv_cz),
        jmh[1] * cos_hr,
        jmh[1] * sin_hr,
    ]
}

fn jmh_to_aab(jmh: F3, p: &JmhParams) -> F3 {
    let h_rad = to_radians(jmh[2]);
    jmh_to_aab_cs(jmh, h_rad.cos(), h_rad.sin(), p)
}

fn aab_to_rgb(aab: F3, p: &JmhParams) -> F3 {
    let rgb_a = mul3(aab, &p.aab_to_cone_response);
    mul3(rgb_a.map(cone_compress_inv), &p.cam16_c_to_rgb)
}

fn jmh_to_rgb(jmh: F3, p: &JmhParams) -> F3 {
    aab_to_rgb(jmh_to_aab(jmh, p), p)
}

// ---- Tonescale and chroma compression ------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct ToneScale {
    n: f32,
    n_r: f32,
    g: f32,
    t_1: f32,
    c_t: f32,
    s_2: f32,
    m_2: f32,
    forward_limit: f32,
    log_peak: f32,
}

impl ToneScale {
    fn new(peak_luminance: f32) -> Self {
        let n = peak_luminance;
        let n_r = 100.0_f32; // normalised white in nits
        let g = 1.15_f32; // surround / contrast
        let c = 0.18_f32; // anchor for 18 % grey
        let c_d = 10.013_f32; // output luminance of 18 % grey (nits)
        let w_g = 0.14_f32; // change in grey between peak luminances
        let t_1 = 0.04_f32; // shadow toe, flare compensation
        let r_hit_min = 128.0_f32; // scene-referred value "hitting the roof"
        let r_hit_max = 896.0_f32;

        let r_hit =
            r_hit_min + (r_hit_max - r_hit_min) * ((n / n_r).ln() / (10000.0_f32 / 100.0).ln());
        let m_0 = n / n_r;
        let m_1 = 0.5 * (m_0 + (m_0 * (m_0 + 4.0 * t_1)).sqrt());
        let u = ((r_hit / m_1) / ((r_hit / m_1) + 1.0)).powf(g);
        let m = m_1 / u;
        let w_i = (n / 100.0).ln() / 2.0_f32.ln();
        let c_t = c_d / n_r * (1.0 + w_i * w_g);
        let g_ip = 0.5 * (c_t + (c_t * (c_t + 4.0 * t_1)).sqrt());
        let g_ipp2 = -(m_1 * (g_ip / m).powf(1.0 / g)) / ((g_ip / m).powf(1.0 / g) - 1.0);
        let w_2 = c / g_ipp2;
        let s_2 = w_2 * m_1 * REFERENCE_LUMINANCE;
        let u_2 = ((r_hit / m_1) / ((r_hit / m_1) + w_2)).powf(g);
        let m_2 = m_1 / u_2;
        Self {
            n,
            n_r,
            g,
            t_1,
            c_t,
            s_2,
            m_2,
            forward_limit: 8.0 * r_hit,
            log_peak: (n / n_r).log10(),
        }
    }

    fn apply(&self, y_in: f32) -> f32 {
        let f = self.m_2 * (y_in / (y_in + self.s_2)).powf(self.g);
        max_f(0.0, f * f / (f + self.t_1)) * self.n_r
    }

    /// J after the tonescale, from the achromatic response.
    fn a_to_j(&self, a: f32, p: &JmhParams) -> f32 {
        let y_out = self.apply(a_to_y(a, p));
        y_to_j_abs(y_out, p).copysign(a)
    }
}

#[derive(Clone, Copy, Debug)]
struct ChromaCompress {
    sat: f32,
    sat_thr: f32,
    compr: f32,
    scale: f32,
}

impl ChromaCompress {
    fn new(peak_luminance: f32, ts: &ToneScale) -> Self {
        Self {
            sat: max_f(
                0.2,
                CHROMA_EXPAND - (CHROMA_EXPAND * CHROMA_EXPAND_FACT) * ts.log_peak,
            ),
            sat_thr: CHROMA_EXPAND_THR / ts.n,
            compr: CHROMA_COMPRESS + (CHROMA_COMPRESS * CHROMA_COMPRESS_FACT) * ts.log_peak,
            scale: (0.03379 * peak_luminance).powf(0.30596) - 0.45135,
        }
    }
}

fn chroma_compress_norm(cos_hr1: f32, sin_hr1: f32, scale: f32) -> f32 {
    let cos_hr2 = 2.0 * cos_hr1 * cos_hr1 - 1.0;
    let sin_hr2 = 2.0 * cos_hr1 * sin_hr1;
    let cos_hr3 = 4.0 * cos_hr1 * cos_hr1 * cos_hr1 - 3.0 * cos_hr1;
    let sin_hr3 = 3.0 * sin_hr1 - 4.0 * sin_hr1 * sin_hr1 * sin_hr1;
    let m = 11.34072 * cos_hr1
        + 16.46899 * cos_hr2
        + 7.88380 * cos_hr3
        + 14.66441 * sin_hr1
        + -6.37224 * sin_hr2
        + 9.19364 * sin_hr3
        + 77.12896;
    m * scale
}

fn toe_fwd(x: f32, limit: f32, k1_in: f32, k2_in: f32) -> f32 {
    if x > limit {
        return x;
    }
    let k2 = max_f(k2_in, 0.001);
    let k1 = (k1_in * k1_in + k2 * k2).sqrt();
    let k3 = (limit + k1) / (limit + k2);
    let minus_b = k3 * x - k1;
    let minus_ac = k2 * k3 * x;
    0.5 * (minus_b + (minus_b * minus_b + 4.0 * minus_ac).sqrt())
}

/// The shared compression parameters resolved at one hue.
#[derive(Clone, Copy, Debug)]
struct Resolved {
    limit_j_max: f32,
    model_gamma_inv: f32,
    reach_max_m: f32,
}

fn chroma_compress_fwd(jmh: F3, j_ts: f32, m_norm: f32, r: &Resolved, c: &ChromaCompress) -> F3 {
    let [j, m, h] = jmh;
    let mut m_cp = m;
    if m != 0.0 {
        let n_j = j_ts / r.limit_j_max;
        let sn_j = max_f(0.0, 1.0 - n_j);
        let limit = n_j.powf(r.model_gamma_inv) * r.reach_max_m / m_norm;
        m_cp = m * (j_ts / j).powf(r.model_gamma_inv);
        m_cp /= m_norm;
        m_cp = limit
            - toe_fwd(
                limit - m_cp,
                limit - 0.001,
                sn_j * c.sat,
                (n_j * n_j + c.sat_thr).sqrt(),
            );
        m_cp = toe_fwd(m_cp, limit, n_j * c.compr, sn_j);
        m_cp *= m_norm;
    }
    [j_ts, m_cp, h]
}

// ---- Gamut compression ---------------------------------------------------------------------

#[derive(Clone, Debug)]
struct GamutCompress {
    mid_j: f32,
    focus_dist: f32,
    lower_hull_gamma_inv: f32,
    search_range: [i32; 2],
    hue_table: [f32; TOTAL_SIZE],
    /// Per hue: the cusp's J and M, then the upper hull's inverse gamma.
    cusp_table: [F3; TOTAL_SIZE],
}

/// The hue-dependent parameters of the gamut compression.
#[derive(Clone, Copy, Debug)]
struct HueParams {
    gamma_bottom_inv: f32,
    jm_cusp: F2,
    gamma_top_inv: f32,
    focus_j: f32,
    analytical_threshold: f32,
}

fn get_focus_gain(j: f32, analytical_threshold: f32, limit_j_max: f32, focus_dist: f32) -> f32 {
    let mut gain = limit_j_max * focus_dist;
    if j > analytical_threshold {
        // An approximate inverse above the threshold, where J enters the calculation.
        let mut adjustment =
            ((limit_j_max - analytical_threshold) / max_f(0.0001, limit_j_max - j)).log10();
        adjustment = adjustment * adjustment + 1.0;
        gain *= adjustment;
    }
    gain
}

fn solve_j_intersect(j: f32, m: f32, focus_j: f32, max_j: f32, slope_gain: f32) -> f32 {
    let m_scaled = m / slope_gain;
    let a = m_scaled / focus_j;
    if j < focus_j {
        let b = 1.0 - m_scaled;
        let c = -j;
        let det = b * b - 4.0 * a * c;
        -2.0 * c / (b + det.sqrt())
    } else {
        let b = -(1.0 + m_scaled + max_j * a);
        let c = max_j * m_scaled + j;
        let det = b * b - 4.0 * a * c;
        -2.0 * c / (b - det.sqrt())
    }
}

/// A smooth minimum about the scaled reference (a cubic polynomial).
fn smin_scaled(a: f32, b: f32, scale_reference: f32) -> f32 {
    let s_scaled = SMOOTH_CUSPS * scale_reference;
    let h = max_f(s_scaled - (a - b).abs(), 0.0) / s_scaled;
    min_f(a, b) - h * h * h * s_scaled * (1.0 / 6.0)
}

fn compression_vector_slope(
    intersect_j: f32,
    focus_j: f32,
    limit_j_max: f32,
    slope_gain: f32,
) -> f32 {
    let direction_scaler = if intersect_j < focus_j {
        intersect_j
    } else {
        limit_j_max - intersect_j
    };
    direction_scaler * (intersect_j - focus_j) / (focus_j * slope_gain)
}

/// Where the line J = slope · M + J_axis_intersect meets the boundary
/// J = J_max · (M / M_max)^(1 / inv_gamma), approximately.
fn line_boundary_m(
    j_axis_intersect: f32,
    slope: f32,
    inv_gamma: f32,
    j_max: f32,
    m_max: f32,
    j_intersection_reference: f32,
) -> f32 {
    let normalised_j = j_axis_intersect / j_intersection_reference;
    let shifted_intersection = j_intersection_reference * normalised_j.powf(inv_gamma);
    shifted_intersection * m_max / (j_max - slope * m_max)
}

fn gamut_boundary_m(
    jm_cusp: F2,
    j_max: f32,
    gamma_top_inv: f32,
    gamma_bottom_inv: f32,
    j_intersect_source: f32,
    slope: f32,
    j_intersect_cusp: f32,
) -> f32 {
    let lower = line_boundary_m(
        j_intersect_source,
        slope,
        gamma_bottom_inv,
        jm_cusp[0],
        jm_cusp[1],
        j_intersect_cusp,
    );
    // The upper hull is flipped and so zeroed at J_max, with the slope negated.
    let upper = line_boundary_m(
        j_max - j_intersect_source,
        -slope,
        gamma_top_inv,
        j_max - jm_cusp[0],
        jm_cusp[1],
        j_max - j_intersect_cusp,
    );
    smin_scaled(lower, upper, jm_cusp[1])
}

fn remap_m(m: f32, gamut_boundary_m: f32, reach_boundary_m: f32) -> f32 {
    let boundary_ratio = gamut_boundary_m / reach_boundary_m;
    let proportion = max_f(boundary_ratio, COMPRESSION_THRESHOLD);
    let threshold = proportion * gamut_boundary_m;
    if m <= threshold || proportion >= 1.0 {
        return m;
    }
    // Place the threshold at zero.
    let m_offset = m - threshold;
    let gamut_offset = gamut_boundary_m - threshold;
    let reach_offset = reach_boundary_m - threshold;
    let scale = reach_offset / ((reach_offset / gamut_offset) - 1.0);
    let nd = m_offset / scale;
    threshold + scale * nd / (1.0 + nd)
}

fn compress_gamut(jmh: F3, jx: f32, r: &Resolved, g: &GamutCompress, hp: &HueParams) -> F3 {
    let [j, m, h] = jmh;
    let slope_gain = get_focus_gain(jx, hp.analytical_threshold, r.limit_j_max, g.focus_dist);
    let j_intersect_source = solve_j_intersect(j, m, hp.focus_j, r.limit_j_max, slope_gain);
    let gamut_slope =
        compression_vector_slope(j_intersect_source, hp.focus_j, r.limit_j_max, slope_gain);
    let j_intersect_cusp = solve_j_intersect(
        hp.jm_cusp[0],
        hp.jm_cusp[1],
        hp.focus_j,
        r.limit_j_max,
        slope_gain,
    );
    let boundary = gamut_boundary_m(
        hp.jm_cusp,
        r.limit_j_max,
        hp.gamma_top_inv,
        hp.gamma_bottom_inv,
        j_intersect_source,
        gamut_slope,
        j_intersect_cusp,
    );
    if boundary <= 0.0 {
        return [j, 0.0, h];
    }
    let reach_boundary = line_boundary_m(
        j_intersect_source,
        gamut_slope,
        r.model_gamma_inv,
        r.limit_j_max,
        r.reach_max_m,
        r.limit_j_max,
    );
    let remapped = remap_m(m, boundary, reach_boundary);
    [j_intersect_source + remapped * gamut_slope, remapped, h]
}

fn focus_j(cusp_j: f32, mid_j: f32, limit_j_max: f32) -> f32 {
    lerpf(
        cusp_j,
        mid_j,
        min_f(1.0, CUSP_MID_BLEND - (cusp_j / limit_j_max)),
    )
}

fn hue_position(hue: f32) -> usize {
    hue as u32 as usize
}

/// The upper index of the hue table's interval that holds `h`.
fn lookup_hue_interval(h: f32, hues: &[f32; TOTAL_SIZE], range: [i32; 2]) -> usize {
    let mut i = (FIRST_NOMINAL + hue_position(h)) as i32;
    let mut i_lo = (LOWER_WRAP as i32).max(i + range[0]);
    let mut i_hi = (UPPER_WRAP as i32).min(i + range[1]);
    while i_lo + 1 < i_hi {
        if h > hues[i as usize] {
            i_lo = i;
        } else {
            i_hi = i;
        }
        i = (i_lo + i_hi) / 2;
    }
    i_hi.max(1) as usize
}

impl GamutCompress {
    fn hue_params(&self, hue: f32, r: &Resolved) -> HueParams {
        let i_hi = lookup_hue_interval(hue, &self.hue_table, self.search_range);
        let t =
            (hue - self.hue_table[i_hi - 1]) / (self.hue_table[i_hi] - self.hue_table[i_hi - 1]);
        let (lo, hi) = (self.cusp_table[i_hi - 1], self.cusp_table[i_hi]);
        let cusp = [0, 1, 2].map(|k| lerpf(lo[k], hi[k], t));
        HueParams {
            gamma_bottom_inv: self.lower_hull_gamma_inv,
            jm_cusp: [cusp[0], cusp[1]],
            gamma_top_inv: cusp[2],
            focus_j: focus_j(cusp[0], self.mid_j, r.limit_j_max),
            analytical_threshold: lerpf(cusp[0], r.limit_j_max, FOCUS_GAIN_BLEND),
        }
    }

    fn apply(&self, jmh: F3, r: &Resolved) -> F3 {
        let [j, m, h] = jmh;
        if j <= 0.0 {
            return [0.0, 0.0, h];
        }
        // M only is compressed; above the expected maximum it is mapped to 0.
        if m <= 0.0 || j > r.limit_j_max {
            return [j, 0.0, h];
        }
        compress_gamut(jmh, j, r, self, &self.hue_params(h, r))
    }
}

// ---- Table generation ----------------------------------------------------------------------

fn unit_cube_cusp_corner(corner: usize) -> F3 {
    // Generated in the order R, Y, G, C, B, M so that the hues rotate in order.
    let on = |k: usize| {
        if (corner + k) % CUSP_CORNER_COUNT < 3 {
            1.0
        } else {
            0.0
        }
    };
    [on(1), on(5), on(3)]
}

/// Rotates the six corners so that the lowest hue is at [1], and closes the cycle.
fn cycle_corners<T: Copy>(
    temp: &[T; CUSP_CORNER_COUNT],
    min_index: usize,
    out: &mut [T; TOTAL_CORNER_COUNT],
) {
    for i in 0..CUSP_CORNER_COUNT {
        out[i + 1] = temp[(i + min_index) % CUSP_CORNER_COUNT];
    }
    out[0] = out[CUSP_CORNER_COUNT];
    out[CUSP_CORNER_COUNT + 1] = out[1];
}

fn min_hue_index(corners: &[F3; CUSP_CORNER_COUNT]) -> usize {
    let mut min_index = 0;
    for (i, c) in corners.iter().enumerate() {
        if c[2] < corners[min_index][2] {
            min_index = i;
        }
    }
    min_index
}

fn limiting_cusp_corners(
    p: &JmhParams,
    peak_luminance: f32,
) -> ([F3; TOTAL_CORNER_COUNT], [F3; TOTAL_CORNER_COUNT]) {
    let temp_rgb: [F3; CUSP_CORNER_COUNT] = std::array::from_fn(|i| {
        unit_cube_cusp_corner(i).map(|v| peak_luminance / REFERENCE_LUMINANCE * v)
    });
    let temp_jmh = temp_rgb.map(|rgb| rgb_to_jmh(rgb, p));
    let min_index = min_hue_index(&temp_jmh);
    let (mut rgb, mut jmh) = (
        [[0.0; 3]; TOTAL_CORNER_COUNT],
        [[0.0; 3]; TOTAL_CORNER_COUNT],
    );
    cycle_corners(&temp_rgb, min_index, &mut rgb);
    cycle_corners(&temp_jmh, min_index, &mut jmh);
    // Wrapped hues fall outside [0, 360) to stay monotonic.
    jmh[0][2] -= HUE_LIMIT;
    jmh[CUSP_CORNER_COUNT + 1][2] += HUE_LIMIT;
    (rgb, jmh)
}

/// The JMh of each corner scaled until its J reaches `limit_j` (searched on the achromatic
/// response, which avoids the non-linear transform).
fn reach_corners(p: &JmhParams, limit_j: f32, maximum_source: f32) -> [F3; TOTAL_CORNER_COUNT] {
    let limit_a = j_to_achromatic_n(limit_j, p.inv_cz);
    let temp: [F3; CUSP_CORNER_COUNT] = std::array::from_fn(|i| {
        let rgb_vector = unit_cube_cusp_corner(i);
        let (mut lower, mut upper) = (0.0_f32, maximum_source);
        while (upper - lower) > REACH_CUSP_TOLERANCE {
            let test = midpoint(lower, upper);
            let a = rgb_to_aab(rgb_vector.map(|v| test * v), p)[0];
            if a < limit_a {
                lower = test;
            } else {
                upper = test;
            }
            if a == limit_a {
                break;
            }
        }
        rgb_to_jmh(rgb_vector.map(|v| upper * v), p)
    });
    let min_index = min_hue_index(&temp);
    let mut jmh = [[0.0; 3]; TOTAL_CORNER_COUNT];
    cycle_corners(&temp, min_index, &mut jmh);
    jmh[0][2] -= HUE_LIMIT;
    jmh[CUSP_CORNER_COUNT + 1][2] += HUE_LIMIT;
    jmh
}

/// Merges the two sorted corner lists into their unique hues; returns how many.
fn sorted_cube_hues(
    sorted: &mut [f32; MAX_SORTED_CORNERS],
    reach: &[F3; TOTAL_CORNER_COUNT],
    display: &[F3; TOTAL_CORNER_COUNT],
) -> usize {
    let (mut idx, mut reach_idx, mut display_idx) = (0, 1, 1);
    while reach_idx < CUSP_CORNER_COUNT + 1 || display_idx < CUSP_CORNER_COUNT + 1 {
        let reach_hue = reach[reach_idx][2];
        let display_hue = display[display_idx][2];
        if reach_hue == display_hue {
            sorted[idx] = reach_hue;
            reach_idx += 1;
            display_idx += 1;
        } else if reach_hue < display_hue {
            sorted[idx] = reach_hue;
            reach_idx += 1;
        } else {
            sorted[idx] = display_hue;
            display_idx += 1;
        }
        idx += 1;
    }
    idx
}

fn hue_sample_interval(
    samples: u32,
    lower: f32,
    upper: f32,
    table: &mut [f32; TOTAL_SIZE],
    base: usize,
) {
    let delta = (upper - lower) / samples as f32;
    for i in 0..samples as usize {
        table[base + i] = lower + i as f32 * delta;
    }
}

/// Hues as uniform as possible, with the corners of the limiting gamut and of the reach
/// sampled exactly.
fn build_hue_table(
    table: &mut [f32; TOTAL_SIZE],
    sorted: &[f32; MAX_SORTED_CORNERS],
    unique: usize,
) {
    let ideal_spacing = NOMINAL_SIZE as f32 / HUE_LIMIT;
    let last = NOMINAL_SIZE as u32 - 1;
    let mut samples_count = [0_u32; 2 * CUSP_CORNER_COUNT + 2];
    let mut last_idx = u32::MAX;
    let mut min_index = if sorted[0] == 0.0 { 0 } else { 1 };
    for hue_idx in 0..unique {
        let rounded = (sorted[hue_idx] * ideal_spacing).round() as u32;
        let mut nominal_idx = rounded.max(min_index).min(last);
        if last_idx == nominal_idx {
            // The last two hues would sample at the same index: move one of them.
            if hue_idx > 1
                && samples_count[hue_idx - 2] != samples_count[hue_idx - 1].wrapping_sub(1)
            {
                samples_count[hue_idx - 1] = samples_count[hue_idx - 1].wrapping_sub(1);
            } else {
                nominal_idx += 1;
            }
        }
        samples_count[hue_idx] = nominal_idx.min(last);
        last_idx = nominal_idx;
        min_index = nominal_idx;
    }

    let mut total_samples = 0_u32;
    hue_sample_interval(
        samples_count[0],
        0.0,
        sorted[0],
        table,
        total_samples as usize + 1,
    );
    total_samples += samples_count[0];
    let mut i = 1;
    while i != unique {
        let samples = samples_count[i].wrapping_sub(samples_count[i - 1]);
        hue_sample_interval(
            samples,
            sorted[i - 1],
            sorted[i],
            table,
            total_samples as usize + 1,
        );
        total_samples = total_samples.wrapping_add(samples);
        i += 1;
    }
    hue_sample_interval(
        NOMINAL_SIZE as u32 - total_samples,
        sorted[i - 1],
        HUE_LIMIT,
        table,
        total_samples as usize + 1,
    );
    table[LOWER_WRAP] = table[LAST_NOMINAL] - HUE_LIMIT;
    table[UPPER_WRAP] = table[FIRST_NOMINAL] + HUE_LIMIT;
    table[UPPER_WRAP + 1] = table[FIRST_NOMINAL + 1] + HUE_LIMIT;
}

/// The cusp of the limiting gamut at `hue`: a binary search along the edge of the RGB cube
/// between the two corners around it.
fn display_cusp_for_hue(
    hue: f32,
    rgb_corners: &[F3; TOTAL_CORNER_COUNT],
    jmh_corners: &[F3; TOTAL_CORNER_COUNT],
    p: &JmhParams,
    previous: &mut F2,
) -> F2 {
    let mut upper_corner = 1;
    for (i, corner) in jmh_corners.iter().enumerate().skip(1) {
        if corner[2] > hue {
            upper_corner = i;
            break;
        }
    }
    let lower_corner = upper_corner - 1;
    if jmh_corners[lower_corner][2] == hue {
        return [jmh_corners[lower_corner][0], jmh_corners[lower_corner][1]];
    }
    let (cusp_lower, cusp_upper) = (rgb_corners[lower_corner], rgb_corners[upper_corner]);
    let lerp = |t: f32| [0, 1, 2].map(|k| lerpf(cusp_lower[k], cusp_upper[k], t));
    // Still on the same edge: start from where the last search ended.
    let mut lower_t = if upper_corner as f32 == previous[0] {
        previous[1]
    } else {
        0.0
    };
    let mut upper_t = 1.0_f32;
    while (upper_t - lower_t) > DISPLAY_CUSP_TOLERANCE {
        let sample_t = midpoint(lower_t, upper_t);
        let jmh = rgb_to_jmh(lerp(sample_t), p);
        if jmh[2] < jmh_corners[lower_corner][2] {
            upper_t = sample_t;
        } else if jmh[2] >= jmh_corners[upper_corner][2] {
            lower_t = sample_t;
        } else if jmh[2] > hue {
            upper_t = sample_t;
        } else {
            lower_t = sample_t;
        }
    }
    let sample_t = midpoint(lower_t, upper_t);
    let jmh = rgb_to_jmh(lerp(sample_t), p);
    *previous = [upper_corner as f32, sample_t];
    [jmh[0], jmh[1]]
}

fn build_cusp_table(
    hue_table: &[f32; TOTAL_SIZE],
    rgb_corners: &[F3; TOTAL_CORNER_COUNT],
    jmh_corners: &[F3; TOTAL_CORNER_COUNT],
    p: &JmhParams,
) -> [F3; TOTAL_SIZE] {
    let mut previous = [0.0_f32; 2];
    let mut table = [[0.0_f32; 3]; TOTAL_SIZE];
    for i in FIRST_NOMINAL..UPPER_WRAP {
        let hue = hue_table[i];
        let jm = display_cusp_for_hue(hue, rgb_corners, jmh_corners, p, &mut previous);
        table[i] = [jm[0], jm[1] * (1.0 + SMOOTH_M * SMOOTH_CUSPS), hue];
    }
    table[LOWER_WRAP] = [
        table[LAST_NOMINAL][0],
        table[LAST_NOMINAL][1],
        hue_table[LOWER_WRAP],
    ];
    table[UPPER_WRAP] = [
        table[FIRST_NOMINAL][0],
        table[FIRST_NOMINAL][1],
        hue_table[UPPER_WRAP],
    ];
    table[UPPER_WRAP + 1] = [
        table[FIRST_NOMINAL + 1][0],
        table[FIRST_NOMINAL + 1][1],
        hue_table[UPPER_WRAP + 1],
    ];
    table
}

/// Per whole degree of hue: the largest M at `limit_j_max` that stays inside the reach gamut.
fn reach_m_table(p: &JmhParams, limit_j_max: f32) -> [f32; TOTAL_SIZE] {
    let mut table = [0.0_f32; TOTAL_SIZE];
    let below_zero = |rgb: F3| rgb[0] < 0.0 || rgb[1] < 0.0 || rgb[2] < 0.0;
    for i in 0..NOMINAL_SIZE {
        let hue = i as f32;
        const SEARCH_RANGE: f32 = 50.0;
        const SEARCH_MAXIMUM: f32 = 1300.0;
        let mut low = 0.0_f32;
        let mut high = low + SEARCH_RANGE;
        let mut outside = false;
        while !outside && high < SEARCH_MAXIMUM {
            outside = below_zero(jmh_to_rgb([limit_j_max, high, hue], p));
            if !outside {
                low = high;
                high += SEARCH_RANGE;
            }
        }
        while high - low > 1e-2 {
            let sample_m = (high + low) / 2.0;
            if below_zero(jmh_to_rgb([limit_j_max, sample_m, hue], p)) {
                high = sample_m;
            } else {
                low = sample_m;
            }
        }
        table[i + FIRST_NOMINAL] = high;
    }
    table[LOWER_WRAP] = table[LAST_NOMINAL];
    table[UPPER_WRAP] = table[FIRST_NOMINAL];
    table[UPPER_WRAP + 1] = table[FIRST_NOMINAL + 1];
    table
}

fn reach_m_from_table(h: f32, table: &[f32; TOTAL_SIZE]) -> f32 {
    let base = hue_position(h);
    let t = h - base as f32;
    let i_lo = base + FIRST_NOMINAL;
    lerpf(table[i_lo], table[i_lo + 1], t)
}

/// One of the five points on which a candidate upper-hull gamma is tried.
#[derive(Clone, Copy)]
struct GammaTest {
    hue: f32,
    j_intersect_source: f32,
    slope: f32,
    j_intersect_cusp: f32,
}

fn gamma_tests(
    jm_cusp: F2,
    hue: f32,
    limit_j_max: f32,
    mid_j: f32,
    focus_dist: f32,
) -> [GammaTest; 5] {
    let positions = [0.01_f32, 0.1, 0.5, 0.8, 0.99];
    let analytical_threshold = lerpf(jm_cusp[0], limit_j_max, FOCUS_GAIN_BLEND);
    let focus = focus_j(jm_cusp[0], mid_j, limit_j_max);
    positions.map(|position| {
        let test_j = lerpf(jm_cusp[0], limit_j_max, position);
        let slope_gain = get_focus_gain(test_j, analytical_threshold, limit_j_max, focus_dist);
        let j_intersect_source =
            solve_j_intersect(test_j, jm_cusp[1], focus, limit_j_max, slope_gain);
        GammaTest {
            hue,
            j_intersect_source,
            slope: compression_vector_slope(j_intersect_source, focus, limit_j_max, slope_gain),
            j_intersect_cusp: solve_j_intersect(
                jm_cusp[0],
                jm_cusp[1],
                focus,
                limit_j_max,
                slope_gain,
            ),
        }
    })
}

/// Whether every test point's boundary, with this upper-hull gamma, lies outside the
/// limiting gamut's top shell.
fn gamma_fits(
    jm_cusp: F2,
    tests: &[GammaTest; 5],
    top_gamma_inv: f32,
    peak_luminance: f32,
    limit_j_max: f32,
    lower_hull_gamma_inv: f32,
    p: &JmhParams,
) -> bool {
    let luminance_limit = peak_luminance / REFERENCE_LUMINANCE;
    tests.iter().all(|t| {
        let m = gamut_boundary_m(
            jm_cusp,
            limit_j_max,
            top_gamma_inv,
            lower_hull_gamma_inv,
            t.j_intersect_source,
            t.slope,
            t.j_intersect_cusp,
        );
        let j = t.j_intersect_source + t.slope * m;
        let rgb = jmh_to_rgb([j, m, t.hue], p);
        rgb[0] > luminance_limit || rgb[1] > luminance_limit || rgb[2] > luminance_limit
    })
}

#[allow(clippy::too_many_arguments)]
fn upper_hull_gamma(
    hue_table: &[f32; TOTAL_SIZE],
    cusp_table: &mut [F3; TOTAL_SIZE],
    peak_luminance: f32,
    limit_j_max: f32,
    mid_j: f32,
    focus_dist: f32,
    lower_hull_gamma_inv: f32,
    p: &JmhParams,
) {
    for i in FIRST_NOMINAL..UPPER_WRAP {
        let hue = hue_table[i];
        let jm_cusp = [cusp_table[i][0], cusp_table[i][1]];
        let tests = gamma_tests(jm_cusp, hue, limit_j_max, mid_j, focus_dist);
        let fits = |gamma: f32| {
            gamma_fits(
                jm_cusp,
                &tests,
                1.0 / gamma,
                peak_luminance,
                limit_j_max,
                lower_hull_gamma_inv,
                p,
            )
        };
        let mut low = GAMMA_MINIMUM;
        let mut high = low + GAMMA_SEARCH_STEP;
        let mut outside = false;
        while !outside && high < GAMMA_MAXIMUM {
            if fits(high) {
                outside = true;
            } else {
                low = high;
                high += GAMMA_SEARCH_STEP;
            }
        }
        while (high - low) > GAMMA_ACCURACY {
            let test_gamma = midpoint(high, low);
            if fits(test_gamma) {
                high = test_gamma;
            } else {
                low = test_gamma;
            }
        }
        cusp_table[i][2] = 1.0 / high;
    }
    cusp_table[LOWER_WRAP][2] = cusp_table[LAST_NOMINAL][2];
    cusp_table[UPPER_WRAP][2] = cusp_table[FIRST_NOMINAL][2];
    cusp_table[UPPER_WRAP + 1][2] = cusp_table[FIRST_NOMINAL + 1][2];
}

/// How far the table's hues stray from uniform: the binary search's window.
fn hue_search_range(cusp_table: &[F3; TOTAL_SIZE]) -> [i32; 2] {
    let (lower_padding, upper_padding) = (0, 1);
    let mut range = [lower_padding, upper_padding];
    for (i, entry) in cusp_table
        .iter()
        .enumerate()
        .take(UPPER_WRAP)
        .skip(FIRST_NOMINAL)
    {
        let pos = (FIRST_NOMINAL + hue_position(entry[2])) as i32;
        let delta = i as i32 - pos;
        range[0] = range[0].min(delta + lower_padding);
        range[1] = range[1].max(delta + upper_padding);
    }
    range
}

// ---- The transform -------------------------------------------------------------------------

/// ACES 2.0's output transform for one display: ACES2065-1 in, linear RGB in the limiting
/// primaries out, 1.0 being 100 nits.
#[derive(Clone, Debug)]
pub struct OutputTransform {
    p_in: JmhParams,
    p_out: JmhParams,
    tonescale: ToneScale,
    limit_j_max: f32,
    model_gamma_inv: f32,
    reach_m: [f32; TOTAL_SIZE],
    chroma: ChromaCompress,
    gamut: GamutCompress,
}

impl OutputTransform {
    /// Builds the tables for a display of `peak_luminance` nits whose gamut is `limiting`.
    /// The limiting chromaticities are rounded to `f32` first, as OCIO passes them.
    pub fn new(peak_luminance: f32, limiting: &Primaries) -> Self {
        let round = |xy: [f64; 2]| xy.map(|v| f64::from(v as f32));
        let limiting = Primaries {
            red: round(limiting.red),
            green: round(limiting.green),
            blue: round(limiting.blue),
            white: round(limiting.white),
        };
        let p_in = JmhParams::new(&AP0);
        let p_out = JmhParams::new(&limiting);
        let reach = JmhParams::new(&AP1);
        let tonescale = ToneScale::new(peak_luminance);
        let limit_j_max = y_to_j(peak_luminance, &p_in);
        let model_gamma_inv = 1.0 / model_gamma();
        let reach_m = reach_m_table(&reach, limit_j_max);
        let chroma = ChromaCompress::new(peak_luminance, &tonescale);

        let mid_j = y_to_j(tonescale.c_t * REFERENCE_LUMINANCE, &p_in);
        let focus_dist =
            FOCUS_DISTANCE + FOCUS_DISTANCE * FOCUS_DISTANCE_SCALING * tonescale.log_peak;
        let lower_hull_gamma_inv = 1.0 / (1.14 + 0.07 * tonescale.log_peak);
        let reach_jmh = reach_corners(&reach, limit_j_max, tonescale.forward_limit);
        let (limiting_rgb, limiting_jmh) = limiting_cusp_corners(&p_out, peak_luminance);
        let mut sorted = [0.0_f32; MAX_SORTED_CORNERS];
        let unique = sorted_cube_hues(&mut sorted, &reach_jmh, &limiting_jmh);
        let mut hue_table = [0.0_f32; TOTAL_SIZE];
        build_hue_table(&mut hue_table, &sorted, unique);
        let mut cusp_table = build_cusp_table(&hue_table, &limiting_rgb, &limiting_jmh, &p_out);
        let search_range = hue_search_range(&cusp_table);
        upper_hull_gamma(
            &hue_table,
            &mut cusp_table,
            peak_luminance,
            limit_j_max,
            mid_j,
            focus_dist,
            lower_hull_gamma_inv,
            &p_out,
        );
        Self {
            p_in,
            p_out,
            tonescale,
            limit_j_max,
            model_gamma_inv,
            reach_m,
            chroma,
            gamut: GamutCompress {
                mid_j,
                focus_dist,
                lower_hull_gamma_inv,
                search_range,
                hue_table,
                cusp_table,
            },
        }
    }

    /// The transform of one ACES2065-1 colour (OCIO's `Renderer_ACES_OutputTransform20::fwd`).
    pub fn apply(&self, rgb: F3) -> F3 {
        let aab = rgb_to_aab(rgb, &self.p_in);
        let jmh = aab_to_jmh(aab, &self.p_in);
        let resolved = Resolved {
            limit_j_max: self.limit_j_max,
            model_gamma_inv: self.model_gamma_inv,
            reach_max_m: reach_m_from_table(jmh[2], &self.reach_m),
        };
        let h_rad = to_radians(jmh[2]);
        let (cos_hr, sin_hr) = (h_rad.cos(), h_rad.sin());
        let m_norm = chroma_compress_norm(cos_hr, sin_hr, self.chroma.scale);
        let j_ts = self.tonescale.a_to_j(aab[0], &self.p_in);
        let tonemapped = chroma_compress_fwd(jmh, j_ts, m_norm, &resolved, &self.chroma);
        let compressed = self.gamut.apply(tonemapped, &resolved);
        aab_to_rgb(
            jmh_to_aab_cs(compressed, cos_hr, sin_hr, &self.p_out),
            &self.p_out,
        )
    }

    /// The largest input the transform expects, in AP1 (the clamp before it).
    pub fn forward_limit(&self) -> f32 {
        self.tonescale.forward_limit
    }
}

/// The SDR output as Forge uses it: linear Rec.709 scene colour in, linear Rec.709 display
/// colour in [0, 1] out (100 nits, Rec.709 limiting). The chain of OCIO's
/// "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0": the input clamped to
/// [0, forward limit] in AP1, the transform, the output clamped to [0, 1].
#[derive(Clone, Debug)]
pub struct Sdr {
    transform: OutputTransform,
    rec709_to_ap1: M33,
    ap1_to_ap0: M33,
}

impl Sdr {
    /// Builds the tables (a few milliseconds).
    pub fn new() -> Self {
        Self {
            transform: OutputTransform::new(100.0, &REC709),
            rec709_to_ap1: to_m33(&conversion(&REC709, &AP1, true)),
            ap1_to_ap0: to_m33(&conversion(&AP1, &AP0, false)),
        }
    }

    /// The display colour of a scene colour.
    pub fn apply(&self, rec709: F3) -> F3 {
        let limit = self.transform.forward_limit();
        let ap1 = mul3(rec709, &self.rec709_to_ap1).map(|v| min_f(max_f(v, 0.0), limit));
        let out = self.transform.apply(mul3(ap1, &self.ap1_to_ap0));
        out.map(|v| min_f(max_f(v, 0.0), 1.0))
    }
}

impl Default for Sdr {
    fn default() -> Self {
        Self::new()
    }
}

/// Float4 rows of `Aces2Params` in `aces2.slang`: six matrices of three rows, six rows of
/// scalars, then the hue table and the reach table.
pub const GPU_PARAM_ROWS: usize = 6 * 3 + 6 + 2 * TOTAL_SIZE;

impl Sdr {
    /// The parameters and tables `aces2_analytic` reads, as `Aces2Params` lays them out.
    pub fn gpu_params(&self) -> Vec<[f32; 4]> {
        let t = &self.transform;
        let (p_in, p_out) = (&t.p_in, &t.p_out);
        debug_assert!(
            p_in.cz == p_out.cz && p_in.a_w_j == p_out.a_w_j && p_in.f_l_n == p_out.f_l_n
        );
        let mut rows = Vec::with_capacity(GPU_PARAM_ROWS);
        for m in [
            &self.rec709_to_ap1,
            &self.ap1_to_ap0,
            &p_in.rgb_to_cam16_c,
            &p_in.cone_response_to_aab,
            &p_out.aab_to_cone_response,
            &p_out.cam16_c_to_rgb,
        ] {
            for r in 0..3 {
                rows.push([m[r * 3], m[r * 3 + 1], m[r * 3 + 2], 0.0]);
            }
        }
        let (ts, c, g) = (&t.tonescale, &t.chroma, &t.gamut);
        rows.push([p_in.f_l_n, p_in.cz, p_in.inv_cz, p_in.a_w_j]);
        rows.push([
            p_in.inv_a_w_j,
            t.limit_j_max,
            t.model_gamma_inv,
            ts.forward_limit,
        ]);
        rows.push([ts.n_r, ts.g, ts.t_1, ts.s_2]);
        rows.push([ts.m_2, g.mid_j, g.focus_dist, g.lower_hull_gamma_inv]);
        rows.push([c.sat, c.sat_thr, c.compr, c.scale]);
        rows.push([
            f32::from_bits(g.search_range[0] as u32),
            f32::from_bits(g.search_range[1] as u32),
            0.0,
            0.0,
        ]);
        for i in 0..TOTAL_SIZE {
            let cusp = g.cusp_table[i];
            rows.push([g.hue_table[i], cusp[0], cusp[1], cusp[2]]);
        }
        for &reach in &t.reach_m {
            rows.push([reach, 0.0, 0.0, 0.0]);
        }
        debug_assert_eq!(rows.len(), GPU_PARAM_ROWS);
        rows
    }
}

/// The table `tonemap.slang` samples, baked once per process: RGBA16F texels of a
/// `LUT_SIZE`² × `LUT_SIZE` image ([`lut_texels`]).
pub fn shared_lut_texels() -> &'static [u8] {
    static TEXELS: OnceLock<Vec<u8>> = OnceLock::new();
    TEXELS.get_or_init(|| lut_texels(&bake(sdr(), LUT_SIZE), LUT_SIZE))
}

/// The shared SDR transform, built on first use.
pub fn sdr() -> &'static Sdr {
    static SDR: OnceLock<Sdr> = OnceLock::new();
    SDR.get_or_init(Sdr::new)
}

// ---- The baked table -----------------------------------------------------------------------

/// The table's grid: `LUT_SIZE`³ points.
pub const LUT_SIZE: u32 = 65;
/// The shaper's offset: the grid is uniform in log2(x + ε), so that 0 is a grid point and the
/// shadows get as many points as the highlights.
pub const LUT_EPSILON: f32 = 1.0 / 1024.0;
/// The largest input the grid covers (per channel, linear Rec.709); above it the table
/// clamps. The transform clamps at 1024 in AP1 anyway.
pub const LUT_MAX: f32 = 1024.0;

/// A scene value's position on the grid, in [0, 1].
pub fn lut_shaper(x: f32) -> f32 {
    let lo = LUT_EPSILON.log2();
    let hi = (LUT_MAX + LUT_EPSILON).log2();
    (((x.clamp(0.0, LUT_MAX) + LUT_EPSILON).log2() - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// The inverse of [`lut_shaper`].
pub fn lut_shaper_inverse(s: f32) -> f32 {
    let lo = LUT_EPSILON.log2();
    let hi = (LUT_MAX + LUT_EPSILON).log2();
    ((lo + s * (hi - lo)).exp2() - LUT_EPSILON).max(0.0)
}

fn encode_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// The transform over the grid, sRGB-encoded (interpolating encoded values is closer to
/// uniform in lightness), `size`³ RGB triples with red fastest, then green, then blue.
pub fn bake(sdr: &Sdr, size: u32) -> Vec<F3> {
    let n = size as usize;
    let mut out = vec![[0.0_f32; 3]; n * n * n];
    let axis: Vec<f32> = (0..n)
        .map(|i| lut_shaper_inverse(i as f32 / (n - 1) as f32))
        .collect();
    let threads = thread::available_parallelism().map_or(1, |t| t.get());
    let slices_per_thread = n.div_ceil(threads);
    thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(slices_per_thread * n * n).enumerate() {
            let axis = &axis;
            scope.spawn(move || {
                for (i, texel) in chunk.iter_mut().enumerate() {
                    let index = chunk_index * slices_per_thread * n * n + i;
                    let (r, g, b) = (index % n, index / n % n, index / (n * n));
                    let display = sdr.apply([axis[r], axis[g], axis[b]]);
                    *texel = display.map(|v| {
                        let e = encode_srgb(v);
                        if e.is_finite() { e } else { 0.0 }
                    });
                }
            });
        }
    });
    out
}

/// The table sampled as `tonemap.slang` does, on the CPU: trilinear in the shaper's space,
/// sRGB-decoded. For measuring the table against the transform.
pub fn sample(table: &[F3], size: u32, rec709: F3) -> F3 {
    let n = size as usize;
    let u = rec709.map(|x| lut_shaper(x.max(0.0)) * (n - 1) as f32);
    let i0 = u.map(|v| (v.floor() as usize).min(n - 2));
    let f = [0, 1, 2].map(|k| u[k] - i0[k] as f32);
    let at = |r: usize, g: usize, b: usize| table[r + n * (g + n * b)];
    let mut out = [0.0_f32; 3];
    for corner in 0..8 {
        let (dr, dg, db) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
        let w = [(dr, 0), (dg, 1), (db, 2)]
            .iter()
            .map(|&(d, k)| if d == 1 { f[k] } else { 1.0 - f[k] })
            .product::<f32>();
        let v = at(i0[0] + dr, i0[1] + dg, i0[2] + db);
        for k in 0..3 {
            out[k] += w * v[k];
        }
    }
    out.map(decode_srgb)
}

/// `count` scene colours for measuring the table (deterministic): every other one spread from
/// 2^-14 to 2^10 per channel with a tenth of the channels at 0, the rest uniform in [0, 4].
pub fn test_colours(count: usize) -> Vec<F3> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    (0..count)
        .map(|i| {
            if i % 2 == 0 {
                [(); 3].map(|_| {
                    if random() < 0.1 {
                        0.0
                    } else {
                        (random() * 24.0 - 14.0).exp2()
                    }
                })
            } else {
                [(); 3].map(|_| random() * 4.0)
            }
        })
        .collect()
}

/// A display colour's largest channel difference from another, in 8-bit sRGB codes.
pub fn code_difference(a: F3, b: F3) -> f32 {
    (0..3)
        .map(|k| (encode_srgb(a[k]) - encode_srgb(b[k])).abs() * 255.0)
        .fold(0.0, f32::max)
}

/// The table sampled with tetrahedral interpolation (four texels of the cell's six
/// tetrahedra), sRGB-decoded.
pub fn sample_tetrahedral(table: &[F3], size: u32, rec709: F3) -> F3 {
    let n = size as usize;
    let u = rec709.map(|x| lut_shaper(x.max(0.0)) * (n - 1) as f32);
    let i0 = u.map(|v| (v.floor() as usize).min(n - 2));
    let f = [0, 1, 2].map(|k| u[k] - i0[k] as f32);
    let at = |d: [usize; 3]| table[(i0[0] + d[0]) + n * ((i0[1] + d[1]) + n * (i0[2] + d[2]))];
    // Order the fractions: walk from the cell's origin to its far corner one axis at a time.
    let mut axes = [0, 1, 2];
    axes.sort_by(|&a, &b| f[b].total_cmp(&f[a]));
    let mut corner = [0, 0, 0];
    let mut previous = at(corner);
    let mut out = previous.map(|v| v * (1.0 - f[axes[0]]));
    for (step, &axis) in axes.iter().enumerate() {
        corner[axis] = 1;
        let next = at(corner);
        let weight = f[axis] - if step < 2 { f[axes[step + 1]] } else { 0.0 };
        for k in 0..3 {
            out[k] += weight * next[k];
        }
        previous = next;
    }
    let _ = previous;
    out.map(decode_srgb)
}

fn decode_srgb(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// `f32` to IEEE half, rounded to nearest even (the table is uploaded as RGBA16F).
pub fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // Subnormal: shift the implicit bit in, round to nearest even.
        let m = mantissa | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half = 1 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut h = (m >> shift) as u16;
        if rest > half || (rest == half && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = ((e as u32) << 10 | (mantissa >> 13)) as u16;
    let rest = mantissa & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && (h & 1) == 1) {
        h += 1; // may carry into the exponent, which is still correct
    }
    sign | h
}

/// The baked table as RGBA16F texels for a 2D image `size`² wide and `size` high: slice b
/// (blue) occupies columns [b · size, (b + 1) · size), red along x within it, green along y.
pub fn lut_texels(table: &[F3], size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut bytes = Vec::with_capacity(n * n * n * 8);
    for g in 0..n {
        for b in 0..n {
            for r in 0..n {
                let v = table[r + n * (g + n * b)];
                for c in [v[0], v[1], v[2], 1.0] {
                    bytes.extend_from_slice(&f32_to_f16(c).to_le_bytes());
                }
            }
        }
    }
    bytes
}

#[cfg(test)]
#[allow(clippy::excessive_precision)] // OCIO's test values, digit for digit
mod tests {
    use super::*;

    /// OCIO's tolerance check: absolute below 1, relative above.
    fn close(actual: F3, expected: F3, tolerance: f32) -> bool {
        (0..3).all(|k| (actual[k] - expected[k]).abs() / expected[k].abs().max(1.0) <= tolerance)
    }

    #[test]
    fn the_transform_matches_ocio_at_1000_nits_in_p3() {
        // OCIO 2.5.2, tests/cpu/ops/fixedfunction/FixedFunctionOpCPU_tests.cpp,
        // `aces_output_transform_20`: ACES2065-1 in, P3-D65 out, 1000 nits, 1e-5.
        let input: [F3; 35] = [
            [2.781808965, 0.179178253, -0.022103530],
            [3.344523751, 3.617862727, -0.006002689],
            [0.562714786, 3.438684474, 0.016100841],
            [1.218191035, 3.820821747, 4.022103530],
            [0.655476249, 0.382137273, 4.006002689],
            [3.437285214, 0.561315526, 3.983899159],
            [0.110000000, 0.020000000, 0.040000000],
            [0.710000000, 0.510000000, 0.810000000],
            [0.430000000, 0.820000000, 0.710000000],
            [0.118770000, 0.087090000, 0.058950000],
            [0.400020000, 0.319160000, 0.237360000],
            [0.184760000, 0.203980000, 0.313110000],
            [0.109010000, 0.135110000, 0.064930000],
            [0.266840000, 0.246040000, 0.409320000],
            [0.322830000, 0.462080000, 0.406060000],
            [0.386050000, 0.227430000, 0.057770000],
            [0.138220000, 0.130370000, 0.337030000],
            [0.302020000, 0.137520000, 0.127580000],
            [0.093100000, 0.063470000, 0.135250000],
            [0.348760000, 0.436540000, 0.106130000],
            [0.486550000, 0.366850000, 0.080610000],
            [0.087320000, 0.074430000, 0.272740000],
            [0.153660000, 0.256920000, 0.090710000],
            [0.217420000, 0.070700000, 0.051300000],
            [0.589190000, 0.539430000, 0.091570000],
            [0.309040000, 0.148180000, 0.274260000],
            [0.149010000, 0.233780000, 0.359390000],
            [0.866530000, 0.867920000, 0.858180000],
            [0.573560000, 0.572560000, 0.571690000],
            [0.353460000, 0.353370000, 0.353910000],
            [0.202530000, 0.202430000, 0.202870000],
            [0.094670000, 0.095200000, 0.096370000],
            [0.037450000, 0.037660000, 0.038950000],
            [0.180000000, 0.180000000, 0.180000000],
            [0.977840000, 0.977840000, 0.977840000],
        ];
        let expected: [F3; 35] = [
            [4.966013432, -0.033002287, 0.041583523],
            [3.969460726, 3.825797558, -0.056160748],
            [-0.075460039, 3.689072609, 0.270235062],
            [-0.095436633, 3.650521517, 3.459975719],
            [-0.028881177, 0.196473420, 2.796123743],
            [4.900828362, -0.064385533, 3.838270903],
            [0.096890487, -0.001135427, 0.018971475],
            [0.809613585, 0.479857147, 0.814239979],
            [0.107417941, 0.920530438, 0.726379037],
            [0.115475342, 0.050812997, 0.030212998],
            [0.484880149, 0.301042914, 0.226769030],
            [0.098463453, 0.160814837, 0.277010798],
            [0.071130276, 0.107334509, 0.035097614],
            [0.207111374, 0.198474824, 0.375326097],
            [0.195447117, 0.481112540, 0.393299103],
            [0.571913302, 0.196873263, 0.041634843],
            [0.045791976, 0.069875412, 0.291233569],
            [0.424848884, 0.083199054, 0.102153927],
            [0.059589352, 0.022219239, 0.091246955],
            [0.360364884, 0.478741497, 0.086726815],
            [0.695661962, 0.371994466, 0.068298057],
            [0.011806240, 0.021665439, 0.199594870],
            [0.076526135, 0.256237596, 0.060564563],
            [0.300064713, 0.023416281, 0.030360531],
            [0.805483222, 0.596904039, 0.082996234],
            [0.388385385, 0.079899333, 0.245818958],
            [0.010951802, 0.196106046, 0.307181537],
            [0.921020269, 0.921707630, 0.912857533],
            [0.590191603, 0.588424563, 0.587825298],
            [0.337743223, 0.337686002, 0.338155240],
            [0.169266403, 0.169178575, 0.169557154],
            [0.058346011, 0.059387885, 0.060296256],
            [0.012581199, 0.012947144, 0.013654212],
            [0.145115077, 0.145115703, 0.145115480],
            [1.041565537, 1.041566610, 1.041566253],
        ];
        let transform = OutputTransform::new(1000.0, &P3_D65);
        for (i, (&input, &expected)) in input.iter().zip(&expected).enumerate() {
            let actual = transform.apply(input);
            assert!(
                close(actual, expected, 1e-5),
                "sample {i}: {actual:?} against {expected:?}"
            );
        }
    }

    #[test]
    fn the_transform_matches_ocio_at_100_nits_in_rec709() {
        // `aces_ot_20_edge_cases`: black, and a hue at exactly 360° (OCIO bug #2220), 1e-4.
        let transform = OutputTransform::new(100.0, &REC709);
        assert_eq!(transform.apply([0.0; 3]), [0.0; 3]);
        let actual = transform.apply([0.742242277, 0.0931933373, 0.321542144]);
        let expected = [0.74736571311951, -0.0019352473318577, 0.19451357424259];
        assert!(
            close(actual, expected, 1e-4),
            "{actual:?} against {expected:?}"
        );
    }

    #[test]
    fn the_sdr_chain_matches_ocio_s_builtin() {
        // tests/cpu/transforms/BuiltinTransform_tests.cpp: "ACES-OUTPUT - ACES2065-1_to_CIE-
        // XYZ-D65 - SDR-100nit-REC709_2.0", ACES2065-1 (0.5, 0.4, 0.3) to XYZ-D65, 1e-4.
        let ap0_to_ap1 = to_m33(&conversion(&AP0, &AP1, false));
        let ap1_to_ap0 = to_m33(&conversion(&AP1, &AP0, false));
        let transform = OutputTransform::new(100.0, &REC709);
        let limit = transform.forward_limit();
        let ap1 = mul3([0.5, 0.4, 0.3], &ap0_to_ap1).map(|v| v.clamp(0.0, limit));
        let rec709 = transform
            .apply(mul3(ap1, &ap1_to_ap0))
            .map(|v| v.clamp(0.0, 1.0));
        let xyz = mul3(rec709, &to_m33(&rgb_to_xyz(&REC709)));
        let expected = [0.26260215, 0.25207460, 0.20617345];
        assert!(close(xyz, expected, 1e-4), "{xyz:?} against {expected:?}");
    }

    #[test]
    fn mid_grey_and_white_land_where_aces_2_puts_them() {
        let sdr = sdr();
        // 0.18 gives 10.013 nits of 100.
        let grey = sdr.apply([0.18; 3]);
        assert!((grey[1] - 0.10013).abs() < 2e-4, "{grey:?}");
        assert!(
            grey.iter().all(|&v| (v - grey[1]).abs() < 1e-4),
            "grey stays grey: {grey:?}"
        );
        assert_eq!(sdr.apply([0.0; 3]), [0.0; 3]);
        let white = sdr.apply([1.0e4; 3]);
        assert!(white.iter().all(|&v| v > 0.999), "{white:?}");
    }

    /// `cargo test --release -p forge-render lut_error -- --ignored --nocapture`: how far the
    /// sampled table is from the transform, in 8-bit sRGB codes, for a few grid sizes.
    #[test]
    #[ignore]
    fn lut_error_report() {
        let sdr = sdr();
        let colours = test_colours(400_000);
        let exact: Vec<F3> = colours.iter().map(|&c| sdr.apply(c)).collect();
        type Sampler = fn(&[F3], u32, F3) -> F3;
        let samplers: [(&str, Sampler); 2] =
            [("trilinear", sample), ("tetrahedral", sample_tetrahedral)];
        for (name, sample_fn) in samplers {
            for size in [33, 65, 129] {
                let start = std::time::Instant::now();
                let table = bake(sdr, size);
                let bake_ms = start.elapsed().as_secs_f64() * 1e3;
                let mut errors: Vec<f32> = colours
                    .iter()
                    .zip(&exact)
                    .map(|(&c, &e)| code_difference(sample_fn(&table, size, c), e))
                    .collect();
                let worst = colours[errors
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .unwrap()
                    .0];
                errors.sort_unstable_by(f32::total_cmp);
                let at =
                    |p: f64| errors[((p * errors.len() as f64) as usize).min(errors.len() - 1)];
                let over_1 = errors.iter().filter(|&&e| e > 1.0).count();
                println!(
                    "{name} {size}³: bake {bake_ms:.0} ms, 8-bit codes: p50 {:.3}, p99 {:.3}, p99.9 {:.3}, max {:.2} at {worst:?}, over 1 code: {over_1} of {}",
                    at(0.5),
                    at(0.99),
                    at(0.999),
                    errors[errors.len() - 1],
                    errors.len()
                );
            }
        }
    }

    #[test]
    fn half_floats_round_to_nearest() {
        assert_eq!(f32_to_f16(0.0), 0);
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(-2.0), 0xc000);
        assert_eq!(f32_to_f16(0.5), 0x3800);
        assert_eq!(f32_to_f16(65504.0), 0x7bff);
        assert_eq!(f32_to_f16(1.0e6), 0x7c00);
        assert_eq!(
            f32_to_f16(1.0 + 1.0 / 2048.0),
            0x3c00,
            "a tie rounds to even"
        );
        assert_eq!(
            f32_to_f16(1.0 + 3.0 / 2048.0),
            0x3c02,
            "a tie rounds to even"
        );
        assert_eq!(f32_to_f16(6.0e-8), 0x0001, "the smallest subnormal");
    }

    #[test]
    fn the_shaper_inverts() {
        for i in 0..=64 {
            let s = i as f32 / 64.0;
            assert!((lut_shaper(lut_shaper_inverse(s)) - s).abs() < 1e-5);
        }
        assert_eq!(lut_shaper_inverse(0.0), 0.0);
        assert_eq!(lut_shaper(0.0), 0.0);
    }
}
