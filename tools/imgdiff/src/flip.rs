//! LDR-ꟻLIP, the perceptual error between two sRGB images (issue #75): a port of NVIDIA's
//! reference implementation, `FLIP.h` v1.7 (<https://github.com/NVlabs/flip>, commit
//! b475eb4).
//!
//! Andersson, Nilsson, Akenine-Möller, Oskarsson, Åström and Fairchild, "ꟻLIP: A Difference
//! Evaluator for Alternating Images", *Proceedings of the ACM on Computer Graphics and
//! Interactive Techniques* 3(2), High Performance Graphics 2020.
//!
//! Both images are filtered as an observer sees them at `ppd` pixels per degree: a colour
//! difference after contrast-sensitivity filters in YCxCz (Hunt-adjusted HyAB in CIELab), then
//! raised by the difference in edges and points of the luminance. Each pixel's error is in
//! [0, 1], 0 where nobody would see a difference. The arithmetic follows the reference step by
//! step in `f32`, in the same order, so the maps and statistics agree with its tool.
//!
//! The reference's licence, which this port keeps:
//!
//! Copyright (c) 2020-2025, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without modification, are
//! permitted provided that the following conditions are met:
//!
//! 1. Redistributions of source code must retain the above copyright notice, this list of
//!    conditions and the following disclaimer.
//! 2. Redistributions in binary form must reproduce the above copyright notice, this list of
//!    conditions and the following disclaimer in the documentation and/or other materials
//!    provided with the distribution.
//! 3. Neither the name of the copyright holder nor the names of its contributors may be used
//!    to endorse or promote products derived from this software without specific prior
//!    written permission.
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
use std::thread;

type Color = [f32; 3];

const PI_SQ: f32 = PI * PI;

// The ꟻLIP constants (`FLIPConstants`).
const QC: f32 = 0.7;
const PC: f32 = 0.4;
const PT: f32 = 0.95;
const W: f32 = 0.082;
const QF: f32 = 0.5;

// The contrast sensitivity functions, as sums of Gaussians (`GaussianConstants`).
const A1: Color = [1.0, 1.0, 34.1];
const B1: Color = [0.0047, 0.0053, 0.04];
const A2: Color = [0.0, 0.0, 13.5];
const B2: Color = [1.0e-5, 1.0e-5, 0.025];

// D65, and its inverse. The literals are the reference's, digit for digit.
#[allow(clippy::excessive_precision)]
const ILLUMINANT: Color = [0.950428545, 1.000000000, 1.088900371];
#[allow(clippy::excessive_precision)]
const INV_ILLUMINANT: Color = [1.052156925, 1.000000000, 0.918357670];

/// The reference's default viewing condition: a 0.7 m wide monitor of 3840 pixels, seen from
/// 0.7 m. About 67 pixels per degree.
pub fn default_ppd() -> f32 {
    let (distance, resolution, monitor_width) = (0.7_f32, 3840.0_f32, 0.7_f32);
    distance * (resolution / monitor_width) * (PI / 180.0)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn mul(a: Color, b: Color) -> Color {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

#[allow(clippy::excessive_precision)]
fn linear_rgb_to_xyz(c: Color) -> Color {
    let a11 = 10_135_552.0_f32 / 24_577_794.0;
    let a12 = 8_788_810.0_f32 / 24_577_794.0;
    let a13 = 4_435_075.0_f32 / 24_577_794.0;
    let a21 = 2_613_072.0_f32 / 12_288_897.0;
    let a22 = 8_788_810.0_f32 / 12_288_897.0;
    let a23 = 887_015.0_f32 / 12_288_897.0;
    let a31 = 1_425_312.0_f32 / 73_733_382.0;
    let a32 = 8_788_810.0_f32 / 73_733_382.0;
    let a33 = 70_074_185.0_f32 / 73_733_382.0;
    [
        a11 * c[0] + a12 * c[1] + a13 * c[2],
        a21 * c[0] + a22 * c[1] + a23 * c[2],
        a31 * c[0] + a32 * c[1] + a33 * c[2],
    ]
}

#[allow(clippy::excessive_precision)]
fn xyz_to_linear_rgb(c: Color) -> Color {
    let (a11, a12, a13) = (3.241003275_f32, -1.537398934_f32, -0.498615861_f32);
    let (a21, a22, a23) = (-0.969224334_f32, 1.875930071_f32, 0.041554224_f32);
    let (a31, a32, a33) = (0.055639423_f32, -0.204011202_f32, 1.057148933_f32);
    [
        a11 * c[0] + a12 * c[1] + a13 * c[2],
        a21 * c[0] + a22 * c[1] + a23 * c[2],
        a31 * c[0] + a32 * c[1] + a33 * c[2],
    ]
}

fn xyz_to_lab(c: Color) -> Color {
    let delta = 6.0_f32 / 29.0;
    let delta_square = delta * delta;
    let delta_cube = delta * delta_square;
    let factor = 1.0 / (3.0 * delta_square);
    let term = 4.0_f32 / 29.0;
    let f = |v: f32| {
        if v > delta_cube {
            v.powf(1.0 / 3.0)
        } else {
            factor * v + term
        }
    };
    let c = mul(c, INV_ILLUMINANT);
    let (x, y, z) = (f(c[0]), f(c[1]), f(c[2]));
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

fn xyz_to_ycxcz(c: Color) -> Color {
    let c = mul(c, INV_ILLUMINANT);
    [
        116.0 * c[1] - 16.0,
        500.0 * (c[0] - c[1]),
        200.0 * (c[1] - c[2]),
    ]
}

fn ycxcz_to_xyz(c: Color) -> Color {
    let y = (c[0] + 16.0) / 116.0;
    let cx = c[1] / 500.0;
    let cz = c[2] / 200.0;
    mul([y + cx, y, y - cz], ILLUMINANT)
}

fn hunt(luminance: f32, chrominance: f32) -> f32 {
    0.01 * luminance * chrominance
}

/// CIELab with Hunt-adjusted chroma.
fn lab_hunt(lab: Color) -> Color {
    [lab[0], hunt(lab[0], lab[1]), hunt(lab[0], lab[2])]
}

fn hyab(a: Color, b: Color) -> f32 {
    let (da, db) = (a[1] - b[1], a[2] - b[2]);
    (a[0] - b[0]).abs() + (da * da + db * db).sqrt()
}

/// The largest colour difference: green against blue.
fn max_distance() -> f32 {
    let lab = |rgb: Color| lab_hunt(xyz_to_lab(linear_rgb_to_xyz(rgb)));
    hyab(lab([0.0, 1.0, 0.0]), lab([0.0, 0.0, 1.0])).powf(QC)
}

fn clamp01(v: f32) -> f32 {
    let v = if v > 0.0 { v } else { 0.0 };
    if v > 1.0 { 1.0 } else { v }
}

fn gaussian(x2: f32, a: f32, b: f32) -> f32 {
    a * (PI / b).sqrt() * (-PI_SQ * x2 / b).exp()
}

fn gaussian_sqrt(x2: f32, a: f32, b: f32) -> f32 {
    (a * (PI / b).sqrt()).sqrt() * (-PI_SQ * x2 / b).exp()
}

/// The separated contrast-sensitivity filters: (Y, Cx) and the two Gaussians of Cz.
fn spatial_filters(ppd: f32) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let max_scale = [B1[0], B1[1], B1[2], B2[0], B2[1], B2[2]]
        .into_iter()
        .fold(0.0_f32, f32::max);
    let radius = (3.0 * (max_scale / (2.0 * PI_SQ)).sqrt() * ppd).ceil() as i32;
    let delta_x = 1.0 / ppd;
    let (mut ycx, mut cz) = (Vec::new(), Vec::new());
    let (mut sum_ycx, mut sum_cz) = ([0.0_f32; 2], [0.0_f32; 2]);
    for x in 0..2 * radius + 1 {
        let ix = (x - radius) as f32 * delta_x;
        let ix2 = ix * ix;
        let value_ycx = [gaussian(ix2, A1[0], B1[0]), gaussian(ix2, A1[1], B1[1])];
        let value_cz = [
            gaussian_sqrt(ix2, A1[2], B1[2]),
            gaussian_sqrt(ix2, A2[2], B2[2]),
        ];
        sum_ycx = [sum_ycx[0] + value_ycx[0], sum_ycx[1] + value_ycx[1]];
        sum_cz = [sum_cz[0] + value_cz[0], sum_cz[1] + value_cz[1]];
        ycx.push(value_ycx);
        cz.push(value_cz);
    }
    let norm_ycx = [1.0 / sum_ycx[0], 1.0 / sum_ycx[1]];
    let norm_cz = 1.0 / (sum_cz[0] * sum_cz[0] + sum_cz[1] * sum_cz[1]).sqrt();
    for (w_ycx, w_cz) in ycx.iter_mut().zip(&mut cz) {
        *w_ycx = [w_ycx[0] * norm_ycx[0], w_ycx[1] * norm_ycx[1]];
        *w_cz = [w_cz[0] * norm_cz, w_cz[1] * norm_cz];
    }
    (ycx, cz)
}

/// The separated edge and point filters: a Gaussian, its first and its second derivative.
fn feature_filter(ppd: f32) -> Vec<Color> {
    let std_dev = 0.5 * W * ppd;
    let radius = (3.0 * std_dev).ceil() as i32;
    let mut filter = Vec::new();
    let (mut g_sum, mut dg_negative, mut dg_positive, mut ddg_negative, mut ddg_positive) =
        (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
    for x in 0..2 * radius + 1 {
        let xx = (x - radius) as f32;
        let g = (-(xx * xx) / (2.0 * std_dev * std_dev)).exp();
        g_sum += g;
        let dg = -xx * g;
        if dg > 0.0 {
            dg_positive += dg;
        } else {
            dg_negative -= dg;
        }
        let ddg = (xx * xx / (std_dev * std_dev) - 1.0) * g;
        if ddg > 0.0 {
            ddg_positive += ddg;
        } else {
            ddg_negative -= ddg;
        }
        filter.push([g, dg, ddg]);
    }
    // Positive weights sum to 1, negative ones to −1.
    let by_sign =
        |v: f32, positive: f32, negative: f32| v / if v > 0.0 { positive } else { negative };
    for p in &mut filter {
        *p = [
            p[0] / g_sum,
            by_sign(p[1], dg_positive, dg_negative),
            by_sign(p[2], ddg_positive, ddg_negative),
        ];
    }
    filter
}

/// Calls `f(y, row)` for every row of `out`, spread over the CPU's threads.
fn for_rows<T: Send>(out: &mut [T], width: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let height = out.len() / width;
    let rows_per_thread = height.div_ceil(threads).max(1);
    thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(rows_per_thread * width).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (row_index, row) in chunk.chunks_mut(width).enumerate() {
                    f(chunk_index * rows_per_thread + row_index, row);
                }
            });
        }
    });
}

/// An 8-bit sRGB image in YCxCz, the way the reference's tool loads a PNG.
fn to_ycxcz(image: &image::RgbaImage) -> Vec<Color> {
    image
        .pixels()
        .map(|p| {
            let c = [0, 1, 2].map(|k| clamp01(srgb_to_linear(f32::from(p[k]) / 255.0)));
            xyz_to_ycxcz(linear_rgb_to_xyz(c))
        })
        .collect()
}

/// The LDR-ꟻLIP error of every pixel, row by row; `reference` and `test` have the same size.
pub fn error_map(reference: &image::RgbaImage, test: &image::RgbaImage, ppd: f32) -> Vec<f32> {
    let (width, height) = reference.dimensions();
    let (w, h) = (width as usize, height as usize);
    let images = [to_ycxcz(reference), to_ycxcz(test)];

    // The colour difference: filter both images horizontally, then vertically, and compare.
    let (filter_ycx, filter_cz) = spatial_filters(ppd);
    let half = (filter_ycx.len() / 2) as isize;
    // Per pixel and image: (Y, Cx) filtered, and Cz through both Gaussians.
    let mut rows = vec![[[0.0_f32; 4]; 2]; w * h];
    for_rows(&mut rows, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            for (image, out) in images.iter().zip(out.iter_mut()) {
                for ix in -half..=half {
                    let xx = (x as isize + ix).clamp(0, w as isize - 1) as usize;
                    let (wy, wc) = (
                        filter_ycx[(ix + half) as usize],
                        filter_cz[(ix + half) as usize],
                    );
                    let c = image[y * w + xx];
                    out[0] += wy[0] * c[0];
                    out[1] += wy[1] * c[1];
                    out[2] += wc[0] * c[2];
                    out[3] += wc[1] * c[2];
                }
            }
        }
    });
    let cmax = max_distance();
    let pccmax = PC * cmax;
    let mut errors = vec![0.0_f32; w * h];
    for_rows(&mut errors, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut filtered = [[0.0_f32; 4]; 2];
            for iy in -half..=half {
                let yy = (y as isize + iy).clamp(0, h as isize - 1) as usize;
                let (wy, wc) = (
                    filter_ycx[(iy + half) as usize],
                    filter_cz[(iy + half) as usize],
                );
                for (filtered, source) in filtered.iter_mut().zip(&rows[yy * w + x]) {
                    filtered[0] += wy[0] * source[0];
                    filtered[1] += wy[1] * source[1];
                    filtered[2] += wc[0] * source[2];
                    filtered[3] += wc[1] * source[3];
                }
            }
            let [reference, test] = filtered.map(|f| {
                let rgb = xyz_to_linear_rgb(ycxcz_to_xyz([f[0], f[1], f[2] + f[3]])).map(clamp01);
                lab_hunt(xyz_to_lab(linear_rgb_to_xyz(rgb)))
            });
            let difference = hyab(reference, test).powf(QC);
            // Up to pccmax maps to [0, PT], the rest to (PT, 1].
            *out = if difference < pccmax {
                difference * (PT / pccmax)
            } else {
                PT + ((difference - pccmax) / (cmax - pccmax)) * (1.0 - PT)
            };
        }
    });

    // The feature difference, on the luminance normalised to [0, 1].
    let filter = feature_filter(ppd);
    let half = (filter.len() / 2) as isize;
    let (one_over_116, sixteen_over_116) = (1.0_f32 / 116.0, 16.0_f32 / 116.0);
    // Per pixel and image: the first and second x-derivatives, and the Gaussian.
    let mut rows = vec![[[0.0_f32; 3]; 2]; w * h];
    for_rows(&mut rows, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            for (image, out) in images.iter().zip(out.iter_mut()) {
                for ix in -half..=half {
                    let xx = (x as isize + ix).clamp(0, w as isize - 1) as usize;
                    let weights = filter[(ix + half) as usize];
                    let normalized = image[y * w + xx][0] * one_over_116 + sixteen_over_116;
                    out[0] += weights[1] * normalized;
                    out[1] += weights[2] * normalized;
                    out[2] += weights[0] * normalized;
                }
            }
        }
    });
    let normalization = 1.0 / 2.0_f32.sqrt();
    for_rows(&mut errors, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            // (dx, ddx, dy, ddy) per image.
            let mut d = [[0.0_f32; 4]; 2];
            for iy in -half..=half {
                let yy = (y as isize + iy).clamp(0, h as isize - 1) as usize;
                let weights = filter[(iy + half) as usize];
                for (d, source) in d.iter_mut().zip(&rows[yy * w + x]) {
                    d[0] += weights[0] * source[0];
                    d[1] += weights[0] * source[1];
                    d[2] += weights[1] * source[2];
                    d[3] += weights[2] * source[2];
                }
            }
            let edge = d.map(|d| (d[0] * d[0] + d[2] * d[2]).sqrt());
            let point = d.map(|d| (d[1] * d[1] + d[3] * d[3]).sqrt());
            let edge_difference = (edge[0] - edge[1]).abs();
            let point_difference = (point[0] - point[1]).abs();
            let larger = if edge_difference > point_difference {
                edge_difference
            } else {
                point_difference
            };
            let feature_difference = (normalization * larger).powf(QF);
            *out = out.powf(1.0 - feature_difference);
        }
    });
    errors
}

/// The pooled error, as the reference's tool prints it, and the percentiles D-017 asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    /// The mean error: ꟻLIP's usual single number.
    pub mean: f32,
    /// The error below which half of the summed error lies (the reference's weighted median).
    pub weighted_median: f32,
    /// The weighted first and third quartiles.
    pub weighted_quartiles: [f32; 2],
    /// Percentiles of the pixels (nearest rank): p50, p99, p99.9.
    pub percentiles: [f32; 3],
    /// The largest error.
    pub max: f32,
    /// The first pixel (x, y) with the largest error.
    pub max_at: (u32, u32),
    /// The pixels whose error is at least 0.1, 0.2 and 0.5.
    pub above: [u64; 3],
}

/// The statistics of an error map `width` pixels wide. The sums run in `f32` in the reference's
/// order, so the mean and the weighted quartiles match its printout.
pub fn stats(errors: &[f32], width: u32) -> Stats {
    let mut sum = 0.0_f32;
    let (mut max, mut max_index) = (f32::MIN_POSITIVE, 0);
    for (i, &e) in errors.iter().enumerate() {
        sum += e;
        if e > max {
            max = e;
            max_index = i;
        }
    }
    let mut sorted = errors.to_vec();
    sorted.sort_unstable_by(f32::total_cmp);
    let weighted = |percent: f32| {
        let mut running = 0.0_f32;
        for &e in &sorted {
            running += e;
            if running > percent * sum {
                return e;
            }
        }
        0.0
    };
    let rank =
        |p: f64| sorted[((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1];
    let above = [0.1, 0.2, 0.5].map(|t| errors.iter().filter(|&&e| e >= t).count() as u64);
    Stats {
        mean: sum / errors.len() as f32,
        weighted_median: weighted(0.5),
        weighted_quartiles: [weighted(0.25), weighted(0.75)],
        percentiles: [rank(0.5), rank(0.99), rank(0.999)],
        max,
        max_at: (
            (max_index % width as usize) as u32,
            (max_index / width as usize) as u32,
        ),
        above,
    }
}

/// The error map in the magma colour map, as the reference's tool writes it.
pub fn magma_image(errors: &[f32], width: u32, height: u32) -> image::RgbImage {
    let to_u8 = |c: f32| (255.0 * clamp01(c) + 0.5) as u8;
    image::RgbImage::from_fn(width, height, |x, y| {
        let e = errors[(y * width + x) as usize];
        let c = MAGMA[(e * 255.0 + 0.5) as usize % MAGMA.len()];
        image::Rgb(c.map(to_u8))
    })
}

/// Matplotlib's magma colour map (CC0), as `FLIP.h` holds it.
#[rustfmt::skip]
const MAGMA: [Color; 256] = [
    [0.001462, 0.000466, 0.013866], [0.002258, 0.001295, 0.018331], [0.003279, 0.002305, 0.023708], [0.004512, 0.003490, 0.029965],
    [0.005950, 0.004843, 0.037130], [0.007588, 0.006356, 0.044973], [0.009426, 0.008022, 0.052844], [0.011465, 0.009828, 0.060750],
    [0.013708, 0.011771, 0.068667], [0.016156, 0.013840, 0.076603], [0.018815, 0.016026, 0.084584], [0.021692, 0.018320, 0.092610],
    [0.024792, 0.020715, 0.100676], [0.028123, 0.023201, 0.108787], [0.031696, 0.025765, 0.116965], [0.035520, 0.028397, 0.125209],
    [0.039608, 0.031090, 0.133515], [0.043830, 0.033830, 0.141886], [0.048062, 0.036607, 0.150327], [0.052320, 0.039407, 0.158841],
    [0.056615, 0.042160, 0.167446], [0.060949, 0.044794, 0.176129], [0.065330, 0.047318, 0.184892], [0.069764, 0.049726, 0.193735],
    [0.074257, 0.052017, 0.202660], [0.078815, 0.054184, 0.211667], [0.083446, 0.056225, 0.220755], [0.088155, 0.058133, 0.229922],
    [0.092949, 0.059904, 0.239164], [0.097833, 0.061531, 0.248477], [0.102815, 0.063010, 0.257854], [0.107899, 0.064335, 0.267289],
    [0.113094, 0.065492, 0.276784], [0.118405, 0.066479, 0.286321], [0.123833, 0.067295, 0.295879], [0.129380, 0.067935, 0.305443],
    [0.135053, 0.068391, 0.315000], [0.140858, 0.068654, 0.324538], [0.146785, 0.068738, 0.334011], [0.152839, 0.068637, 0.343404],
    [0.159018, 0.068354, 0.352688], [0.165308, 0.067911, 0.361816], [0.171713, 0.067305, 0.370771], [0.178212, 0.066576, 0.379497],
    [0.184801, 0.065732, 0.387973], [0.191460, 0.064818, 0.396152], [0.198177, 0.063862, 0.404009], [0.204935, 0.062907, 0.411514],
    [0.211718, 0.061992, 0.418647], [0.218512, 0.061158, 0.425392], [0.225302, 0.060445, 0.431742], [0.232077, 0.059889, 0.437695],
    [0.238826, 0.059517, 0.443256], [0.245543, 0.059352, 0.448436], [0.252220, 0.059415, 0.453248], [0.258857, 0.059706, 0.457710],
    [0.265447, 0.060237, 0.461840], [0.271994, 0.060994, 0.465660], [0.278493, 0.061978, 0.469190], [0.284951, 0.063168, 0.472451],
    [0.291366, 0.064553, 0.475462], [0.297740, 0.066117, 0.478243], [0.304081, 0.067835, 0.480812], [0.310382, 0.069702, 0.483186],
    [0.316654, 0.071690, 0.485380], [0.322899, 0.073782, 0.487408], [0.329114, 0.075972, 0.489287], [0.335308, 0.078236, 0.491024],
    [0.341482, 0.080564, 0.492631], [0.347636, 0.082946, 0.494121], [0.353773, 0.085373, 0.495501], [0.359898, 0.087831, 0.496778],
    [0.366012, 0.090314, 0.497960], [0.372116, 0.092816, 0.499053], [0.378211, 0.095332, 0.500067], [0.384299, 0.097855, 0.501002],
    [0.390384, 0.100379, 0.501864], [0.396467, 0.102902, 0.502658], [0.402548, 0.105420, 0.503386], [0.408629, 0.107930, 0.504052],
    [0.414709, 0.110431, 0.504662], [0.420791, 0.112920, 0.505215], [0.426877, 0.115395, 0.505714], [0.432967, 0.117855, 0.506160],
    [0.439062, 0.120298, 0.506555], [0.445163, 0.122724, 0.506901], [0.451271, 0.125132, 0.507198], [0.457386, 0.127522, 0.507448],
    [0.463508, 0.129893, 0.507652], [0.469640, 0.132245, 0.507809], [0.475780, 0.134577, 0.507921], [0.481929, 0.136891, 0.507989],
    [0.488088, 0.139186, 0.508011], [0.494258, 0.141462, 0.507988], [0.500438, 0.143719, 0.507920], [0.506629, 0.145958, 0.507806],
    [0.512831, 0.148179, 0.507648], [0.519045, 0.150383, 0.507443], [0.525270, 0.152569, 0.507192], [0.531507, 0.154739, 0.506895],
    [0.537755, 0.156894, 0.506551], [0.544015, 0.159033, 0.506159], [0.550287, 0.161158, 0.505719], [0.556571, 0.163269, 0.505230],
    [0.562866, 0.165368, 0.504692], [0.569172, 0.167454, 0.504105], [0.575490, 0.169530, 0.503466], [0.581819, 0.171596, 0.502777],
    [0.588158, 0.173652, 0.502035], [0.594508, 0.175701, 0.501241], [0.600868, 0.177743, 0.500394], [0.607238, 0.179779, 0.499492],
    [0.613617, 0.181811, 0.498536], [0.620005, 0.183840, 0.497524], [0.626401, 0.185867, 0.496456], [0.632805, 0.187893, 0.495332],
    [0.639216, 0.189921, 0.494150], [0.645633, 0.191952, 0.492910], [0.652056, 0.193986, 0.491611], [0.658483, 0.196027, 0.490253],
    [0.664915, 0.198075, 0.488836], [0.671349, 0.200133, 0.487358], [0.677786, 0.202203, 0.485819], [0.684224, 0.204286, 0.484219],
    [0.690661, 0.206384, 0.482558], [0.697098, 0.208501, 0.480835], [0.703532, 0.210638, 0.479049], [0.709962, 0.212797, 0.477201],
    [0.716387, 0.214982, 0.475290], [0.722805, 0.217194, 0.473316], [0.729216, 0.219437, 0.471279], [0.735616, 0.221713, 0.469180],
    [0.742004, 0.224025, 0.467018], [0.748378, 0.226377, 0.464794], [0.754737, 0.228772, 0.462509], [0.761077, 0.231214, 0.460162],
    [0.767398, 0.233705, 0.457755], [0.773695, 0.236249, 0.455289], [0.779968, 0.238851, 0.452765], [0.786212, 0.241514, 0.450184],
    [0.792427, 0.244242, 0.447543], [0.798608, 0.247040, 0.444848], [0.804752, 0.249911, 0.442102], [0.810855, 0.252861, 0.439305],
    [0.816914, 0.255895, 0.436461], [0.822926, 0.259016, 0.433573], [0.828886, 0.262229, 0.430644], [0.834791, 0.265540, 0.427671],
    [0.840636, 0.268953, 0.424666], [0.846416, 0.272473, 0.421631], [0.852126, 0.276106, 0.418573], [0.857763, 0.279857, 0.415496],
    [0.863320, 0.283729, 0.412403], [0.868793, 0.287728, 0.409303], [0.874176, 0.291859, 0.406205], [0.879464, 0.296125, 0.403118],
    [0.884651, 0.300530, 0.400047], [0.889731, 0.305079, 0.397002], [0.894700, 0.309773, 0.393995], [0.899552, 0.314616, 0.391037],
    [0.904281, 0.319610, 0.388137], [0.908884, 0.324755, 0.385308], [0.913354, 0.330052, 0.382563], [0.917689, 0.335500, 0.379915],
    [0.921884, 0.341098, 0.377376], [0.925937, 0.346844, 0.374959], [0.929845, 0.352734, 0.372677], [0.933606, 0.358764, 0.370541],
    [0.937221, 0.364929, 0.368567], [0.940687, 0.371224, 0.366762], [0.944006, 0.377643, 0.365136], [0.947180, 0.384178, 0.363701],
    [0.950210, 0.390820, 0.362468], [0.953099, 0.397563, 0.361438], [0.955849, 0.404400, 0.360619], [0.958464, 0.411324, 0.360014],
    [0.960949, 0.418323, 0.359630], [0.963310, 0.425390, 0.359469], [0.965549, 0.432519, 0.359529], [0.967671, 0.439703, 0.359810],
    [0.969680, 0.446936, 0.360311], [0.971582, 0.454210, 0.361030], [0.973381, 0.461520, 0.361965], [0.975082, 0.468861, 0.363111],
    [0.976690, 0.476226, 0.364466], [0.978210, 0.483612, 0.366025], [0.979645, 0.491014, 0.367783], [0.981000, 0.498428, 0.369734],
    [0.982279, 0.505851, 0.371874], [0.983485, 0.513280, 0.374198], [0.984622, 0.520713, 0.376698], [0.985693, 0.528148, 0.379371],
    [0.986700, 0.535582, 0.382210], [0.987646, 0.543015, 0.385210], [0.988533, 0.550446, 0.388365], [0.989363, 0.557873, 0.391671],
    [0.990138, 0.565296, 0.395122], [0.990871, 0.572706, 0.398714], [0.991558, 0.580107, 0.402441], [0.992196, 0.587502, 0.406299],
    [0.992785, 0.594891, 0.410283], [0.993326, 0.602275, 0.414390], [0.993834, 0.609644, 0.418613], [0.994309, 0.616999, 0.422950],
    [0.994738, 0.624350, 0.427397], [0.995122, 0.631696, 0.431951], [0.995480, 0.639027, 0.436607], [0.995810, 0.646344, 0.441361],
    [0.996096, 0.653659, 0.446213], [0.996341, 0.660969, 0.451160], [0.996580, 0.668256, 0.456192], [0.996775, 0.675541, 0.461314],
    [0.996925, 0.682828, 0.466526], [0.997077, 0.690088, 0.471811], [0.997186, 0.697349, 0.477182], [0.997254, 0.704611, 0.482635],
    [0.997325, 0.711848, 0.488154], [0.997351, 0.719089, 0.493755], [0.997351, 0.726324, 0.499428], [0.997341, 0.733545, 0.505167],
    [0.997285, 0.740772, 0.510983], [0.997228, 0.747981, 0.516859], [0.997138, 0.755190, 0.522806], [0.997019, 0.762398, 0.528821],
    [0.996898, 0.769591, 0.534892], [0.996727, 0.776795, 0.541039], [0.996571, 0.783977, 0.547233], [0.996369, 0.791167, 0.553499],
    [0.996162, 0.798348, 0.559820], [0.995932, 0.805527, 0.566202], [0.995680, 0.812706, 0.572645], [0.995424, 0.819875, 0.579140],
    [0.995131, 0.827052, 0.585701], [0.994851, 0.834213, 0.592307], [0.994524, 0.841387, 0.598983], [0.994222, 0.848540, 0.605696],
    [0.993866, 0.855711, 0.612482], [0.993545, 0.862859, 0.619299], [0.993170, 0.870024, 0.626189], [0.992831, 0.877168, 0.633109],
    [0.992440, 0.884330, 0.640099], [0.992089, 0.891470, 0.647116], [0.991688, 0.898627, 0.654202], [0.991332, 0.905763, 0.661309],
    [0.990930, 0.912915, 0.668481], [0.990570, 0.920049, 0.675675], [0.990175, 0.927196, 0.682926], [0.989815, 0.934329, 0.690198],
    [0.989434, 0.941470, 0.697519], [0.989077, 0.948604, 0.704863], [0.988717, 0.955742, 0.712242], [0.988367, 0.962878, 0.719649],
    [0.988033, 0.970012, 0.727077], [0.987691, 0.977154, 0.734536], [0.987387, 0.984288, 0.742002], [0.987053, 0.991438, 0.749504],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_no_error() {
        let a = image::RgbaImage::from_fn(40, 30, |x, y| {
            image::Rgba([(x * 6) as u8, (y * 8) as u8, ((x + y) * 3) as u8, 255])
        });
        let errors = error_map(&a, &a, default_ppd());
        assert!(errors.iter().all(|&e| e == 0.0));
    }

    #[test]
    fn a_black_square_on_white_is_a_large_error_and_the_rest_stays_small() {
        let white = image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]));
        let mut test = white.clone();
        for y in 28..36 {
            for x in 28..36 {
                test.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let errors = error_map(&white, &test, default_ppd());
        let s = stats(&errors, 64);
        assert!(s.max > 0.9, "{s:?}");
        assert!(
            (28..36).contains(&s.max_at.0) && (28..36).contains(&s.max_at.1),
            "{s:?}"
        );
        assert!(errors[0] < 0.01, "a far corner: {}", errors[0]);
    }

    #[test]
    fn the_filters_are_normalised_like_the_reference() {
        let (ycx, cz) = spatial_filters(default_ppd());
        assert_eq!(ycx.len(), 21, "radius 10 at 67 pixels per degree");
        let sum: f32 = ycx.iter().map(|w| w[0]).sum();
        assert!((sum - 1.0).abs() < 1e-5);
        let (s0, s1) = cz.iter().fold((0.0, 0.0), |(a, b), w| (a + w[0], b + w[1]));
        assert!((s0 * s0 + s1 * s1 - 1.0_f32).abs() < 1e-5);
        let feature = feature_filter(default_ppd());
        assert_eq!(feature.len(), 19, "radius 9 at 67 pixels per degree");
        let positive: f32 = feature.iter().map(|w| w[1].max(0.0)).sum();
        assert!((positive - 1.0).abs() < 1e-5);
    }
}
