//! `sharpness capture.png --edge x,y,w,h [--edge ...]`: how sharp a capture's edges are (issue
//! #159), by the slanted-edge method (ISO 12233's e-SFR, after Burns 2000).
//!
//! Each `--edge` is a rectangle of the image holding one straight edge between a dark and a light
//! side, a few degrees off the pixel grid. The edge's position along each row (or column, for an
//! edge nearer horizontal) is the centroid of the row's gradient; a line fitted through them gives
//! the edge exactly. Every pixel's distance from that line, binned at a quarter of a pixel, gives
//! the edge's profile four times finer than the pixels (the edge spread function): the slant puts
//! the pixels at every phase of the edge. Its derivative is the line spread function, whose
//! Fourier transform's magnitude is the modulation transfer function, the contrast left at each
//! spatial frequency.
//!
//! Printed per edge: the angle, the levels either side, the 10–90 % rise in pixels, the MTF50
//! (the frequency, in cycles per pixel, where half the contrast is left) and the MTF at 0.25 and
//! 0.5 cycles per pixel. An ideal pixel, the scene's light averaged over its square and nothing
//! else, rises over 0.8 pixel and has an MTF50 of 0.60; a softer image has a longer rise and a
//! lower MTF50. The 8-bit values are taken back to linear light through sRGB's curve first
//! (`--encoded` keeps them as they are).
//!
//! `--rcas STOPS` first sharpens the capture as AMD's FidelityFX RCAS would (robust contrast-adaptive
//! sharpening, from FSR 1, MIT): a preview of a sharpening pass before the engine has one, 0 stops
//! the strongest, each stop half as strong. `--write PATH` saves the image measured.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Measure the sharpness of edges in a PNG capture")]
struct Args {
    /// The capture.
    image: PathBuf,
    /// A rectangle holding one slanted edge: x,y,w,h in pixels (repeatable).
    #[arg(long, value_parser = parse_rect, required = true)]
    edge: Vec<[u32; 4]>,
    /// Measure the 8-bit values as they are, not taken back to linear light.
    #[arg(long)]
    encoded: bool,
    /// Print the MTF every this many cycles per pixel, up to 1.
    #[arg(long)]
    curve: Option<f64>,
    /// Sharpen the capture first as FidelityFX RCAS would, this many stops below its strongest.
    #[arg(long)]
    rcas: Option<f64>,
    /// Save the image measured (after `--rcas`).
    #[arg(long)]
    write: Option<PathBuf>,
}

fn parse_rect(s: &str) -> std::result::Result<[u32; 4], String> {
    let v: Vec<u32> = s
        .split(',')
        .map(|p| p.trim().parse::<u32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    v.try_into()
        .map_err(|_| format!("expected x,y,w,h, got `{s}`"))
}

/// Bins a pixel of the profile: this many a pixel.
const OVERSAMPLE: f64 = 4.0;

/// One edge's measure.
#[derive(Debug, Clone)]
struct Measure {
    /// The edge's angle off the nearer pixel axis, degrees.
    angle: f64,
    /// The profile's mean level on its dark and light sides.
    dark: f64,
    light: f64,
    /// 10–90 % rise, pixels.
    rise: f64,
    /// The MTF every 0.005 cycle per pixel, from 0 to 1.
    mtf: Vec<(f64, f64)>,
}

impl Measure {
    /// The MTF at `f` cycles per pixel, linear between the samples.
    fn at(&self, f: f64) -> f64 {
        for w in self.mtf.windows(2) {
            let ((f0, m0), (f1, m1)) = (w[0], w[1]);
            if f >= f0 && f <= f1 {
                return m0 + (m1 - m0) * (f - f0) / (f1 - f0);
            }
        }
        self.mtf.last().map_or(0.0, |&(_, m)| m)
    }

    /// The first frequency where the MTF falls to `level`.
    fn falls_to(&self, level: f64) -> Option<f64> {
        self.mtf.windows(2).find_map(|w| {
            let ((f0, m0), (f1, m1)) = (w[0], w[1]);
            (m0 >= level && m1 < level).then(|| f0 + (f1 - f0) * (m0 - level) / (m0 - m1))
        })
    }
}

/// An image of light values, row-major.
struct Plane {
    width: usize,
    height: usize,
    values: Vec<f64>,
}

impl Plane {
    fn at(&self, x: usize, y: usize) -> f64 {
        self.values[y * self.width + x]
    }

    /// The rectangle `r`, transposed when `transpose` (so the edge always crosses the rows).
    fn crop(&self, r: [u32; 4], transpose: bool) -> Plane {
        let [x0, y0, w, h] = r.map(|v| v as usize);
        let mut values = Vec::with_capacity(w * h);
        if transpose {
            for x in x0..x0 + w {
                for y in y0..y0 + h {
                    values.push(self.at(x, y));
                }
            }
            Plane {
                width: h,
                height: w,
                values,
            }
        } else {
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    values.push(self.at(x, y));
                }
            }
            Plane {
                width: w,
                height: h,
                values,
            }
        }
    }
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// An 8-bit capture's colours, as encoded, from 0 to 1, row-major.
struct Rgb {
    width: usize,
    height: usize,
    pixels: Vec<[f64; 3]>,
}

fn load(path: &PathBuf) -> Result<Rgb> {
    let image = image::open(path)
        .with_context(|| format!("reading {}", path.display()))?
        .to_rgb8();
    Ok(Rgb {
        width: image.width() as usize,
        height: image.height() as usize,
        pixels: image
            .pixels()
            .map(|p| p.0.map(|v| f64::from(v) / 255.0))
            .collect(),
    })
}

/// The colours' luminance (Rec. 709 weights), linear unless `encoded`.
fn luminance(image: &Rgb, encoded: bool) -> Plane {
    Plane {
        width: image.width,
        height: image.height,
        values: image
            .pixels
            .iter()
            .map(|p| {
                let c = p.map(|v| if encoded { v } else { srgb_to_linear(v) });
                0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
            })
            .collect(),
    }
}

/// FidelityFX RCAS (FSR 1's `FsrRcasF`, without its noise detection) on the encoded colours:
/// each pixel sharpened by its four neighbours, as strongly as the cross's darkest and brightest
/// let it without clipping, at most RCAS's limit, `stops` below its strongest.
fn rcas(image: &Rgb, stops: f64) -> Rgb {
    const LIMIT: f64 = 0.25 - 1.0 / 16.0;
    let scale = (-stops).exp2();
    let (w, h) = (image.width, image.height);
    let at = |x: usize, y: usize| image.pixels[y * w + x];
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let e = at(x, y);
            let b = at(x, y.saturating_sub(1));
            let d = at(x.saturating_sub(1), y);
            let f = at((x + 1).min(w - 1), y);
            let hh = at(x, (y + 1).min(h - 1));
            let mut lobe = f64::NEG_INFINITY;
            for c in 0..3 {
                let mn = b[c].min(d[c]).min(f[c]).min(hh[c]);
                let mx = b[c].max(d[c]).max(f[c]).max(hh[c]);
                let hit_min = mn.min(e[c]) / (4.0 * mx).max(1e-9);
                let hit_max = (1.0 - mx.max(e[c])) / (4.0 * mn - 4.0).min(-1e-9);
                lobe = lobe.max((-hit_min).max(hit_max));
            }
            let lobe = lobe.clamp(-LIMIT, 0.0) * scale;
            let r = 1.0 / (4.0 * lobe + 1.0);
            pixels.push(std::array::from_fn(|c| {
                ((lobe * (b[c] + d[c] + f[c] + hh[c]) + e[c]) * r).clamp(0.0, 1.0)
            }));
        }
    }
    Rgb {
        width: w,
        height: h,
        pixels,
    }
}

fn save(image: &Rgb, path: &PathBuf) -> Result<()> {
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|p| p.map(|v| (v * 255.0).round() as u8))
        .collect();
    image::RgbImage::from_raw(image.width as u32, image.height as u32, bytes)
        .context("an image of the wrong size")?
        .save(path)
        .with_context(|| format!("writing {}", path.display()))
}

/// Where the edge crosses each row: the centroid of the row's gradient, or `None` for a row with
/// no gradient.
fn crossings(p: &Plane) -> Vec<Option<f64>> {
    (0..p.height)
        .map(|y| {
            let (mut sum, mut weight) = (0.0, 0.0);
            for x in 1..p.width - 1 {
                let g = (p.at(x + 1, y) - p.at(x - 1, y)).abs();
                sum += g * x as f64;
                weight += g;
            }
            (weight > 1e-9).then(|| sum / weight)
        })
        .collect()
}

/// The least-squares line `x = a + b y` through the rows' crossings.
fn fit(points: &[Option<f64>]) -> Option<(f64, f64)> {
    let pts: Vec<(f64, f64)> = points
        .iter()
        .enumerate()
        .filter_map(|(y, x)| x.map(|x| (y as f64, x)))
        .collect();
    let n = pts.len() as f64;
    if n < 3.0 {
        return None;
    }
    let (sy, sx) = pts
        .iter()
        .fold((0.0, 0.0), |(sy, sx), &(y, x)| (sy + y, sx + x));
    let (my, mx) = (sy / n, sx / n);
    let (syy, syx) = pts.iter().fold((0.0, 0.0), |(syy, syx), &(y, x)| {
        (syy + (y - my) * (y - my), syx + (y - my) * (x - mx))
    });
    if syy <= 0.0 {
        return None;
    }
    let b = syx / syy;
    Some((mx - b * my, b))
}

/// Measures the edge in `p`, which crosses its rows (the image's or, transposed, its columns').
fn measure_plane(p: &Plane) -> Result<Measure> {
    // A first fit, then a second through the crossings of a gradient windowed round the first
    // line (the far side of the rectangle may hold another edge's gradient).
    let (mut a, mut b) = fit(&crossings(p)).context("no edge in the rectangle")?;
    let half_window = 6.0_f64.max(0.15 * p.width as f64);
    let windowed: Vec<Option<f64>> = (0..p.height)
        .map(|y| {
            let centre = a + b * y as f64;
            let (mut sum, mut weight) = (0.0, 0.0);
            for x in 1..p.width - 1 {
                if (x as f64 - centre).abs() > half_window {
                    continue;
                }
                let g = (p.at(x + 1, y) - p.at(x - 1, y)).abs();
                sum += g * x as f64;
                weight += g;
            }
            (weight > 1e-9).then(|| sum / weight)
        })
        .collect();
    if let Some(refit) = fit(&windowed) {
        (a, b) = refit;
    }
    let angle = b.atan().to_degrees();
    if angle.abs() < 1.0 {
        bail!(
            "the edge is {angle:.2}° off the pixel grid: at least 1° is needed (2 to 10 is best)"
        );
    }
    // Distances from the line along its normal, binned.
    let cos = 1.0 / (1.0 + b * b).sqrt();
    // The profile over most of the rectangle's width (it holds the one edge).
    let reach = 0.45 * p.width as f64;
    let bins = (2.0 * reach * OVERSAMPLE).ceil() as usize;
    let mut sum = vec![0.0; bins];
    let mut count = vec![0u32; bins];
    for y in 0..p.height {
        for x in 0..p.width {
            let d = (x as f64 - (a + b * y as f64)) * cos;
            let k = ((d + reach) * OVERSAMPLE).floor();
            if k >= 0.0 && (k as usize) < bins {
                sum[k as usize] += p.at(x, y);
                count[k as usize] += 1;
            }
        }
    }
    let mut esf: Vec<Option<f64>> = sum
        .iter()
        .zip(&count)
        .map(|(&s, &c)| (c > 0).then(|| s / f64::from(c)))
        .collect();
    // Empty bins (a steep slant leaves few) from their neighbours.
    for k in 0..bins {
        if esf[k].is_none() {
            let before = (0..k).rev().find_map(|j| esf[j].map(|v| (j, v)));
            let after = (k + 1..bins).find_map(|j| esf[j].map(|v| (j, v)));
            esf[k] = match (before, after) {
                (Some((j0, v0)), Some((j1, v1))) => {
                    Some(v0 + (v1 - v0) * (k - j0) as f64 / (j1 - j0) as f64)
                }
                (Some((_, v)), None) | (None, Some((_, v))) => Some(v),
                (None, None) => None,
            };
        }
    }
    let mut esf: Vec<f64> = esf
        .into_iter()
        .collect::<Option<_>>()
        .context("an empty profile")?;
    // Dark to light, left to right.
    if esf[0] > esf[bins - 1] {
        esf.reverse();
    }
    let side = (bins / 8).max(1);
    let dark = esf[..side].iter().sum::<f64>() / side as f64;
    let light = esf[bins - side..].iter().sum::<f64>() / side as f64;
    if light - dark < 1e-4 {
        bail!("no contrast across the edge");
    }
    let level = |t: f64| dark + t * (light - dark);
    let crossing = |t: f64| -> Option<f64> {
        let target = level(t);
        esf.windows(2).enumerate().find_map(|(k, w)| {
            (w[0] < target && w[1] >= target).then(|| k as f64 + (target - w[0]) / (w[1] - w[0]))
        })
    };
    let rise = match (crossing(0.1), crossing(0.9)) {
        (Some(lo), Some(hi)) => (hi - lo) / OVERSAMPLE,
        _ => f64::NAN,
    };
    // The line spread function, windowed round its peak (Hamming), and its spectrum.
    let mut lsf = vec![0.0; bins];
    for k in 1..bins - 1 {
        lsf[k] = 0.5 * (esf[k + 1] - esf[k - 1]);
    }
    let peak = lsf
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(k, _)| k) as f64;
    let span = (bins as f64 - 1.0).max(1.0);
    for (k, v) in lsf.iter_mut().enumerate() {
        let t = ((k as f64 - peak) / span).clamp(-0.5, 0.5);
        *v *= 0.54 + 0.46 * (2.0 * std::f64::consts::PI * t).cos();
    }
    // Its transform at every 0.005 cycle per pixel up to 1 (the bins' own Nyquist is 2).
    let step = 1.0 / OVERSAMPLE;
    let dc: f64 = lsf.iter().sum();
    let mut mtf = Vec::new();
    for j in 0..=200 {
        let f = f64::from(j) * 0.005;
        let (mut re, mut im) = (0.0, 0.0);
        for (k, v) in lsf.iter().enumerate() {
            let phase = 2.0 * std::f64::consts::PI * f * k as f64 * step;
            re += v * phase.cos();
            im -= v * phase.sin();
        }
        // The central difference's own response, sin(2πfΔ) / (2πfΔ), divided out.
        let x = 2.0 * std::f64::consts::PI * f * step;
        let derivative = if x > 1e-9 { x.sin() / x } else { 1.0 };
        mtf.push((f, (re * re + im * im).sqrt() / dc.abs() / derivative));
    }
    Ok(Measure {
        angle,
        dark,
        light,
        rise,
        mtf,
    })
}

/// Measures the edge in rectangle `r` of `image`.
fn measure(image: &Plane, r: [u32; 4]) -> Result<Measure> {
    let [x, y, w, h] = r;
    if x + w > image.width as u32 || y + h > image.height as u32 || w < 8 || h < 8 {
        bail!(
            "the rectangle {x},{y},{w},{h} is not inside the {}×{} image (or is under 8 pixels)",
            image.width,
            image.height
        );
    }
    // Rows cross a near-vertical edge: its gradient is mostly along x.
    let plane = image.crop(r, false);
    let (mut gx, mut gy) = (0.0, 0.0);
    for yy in 1..plane.height - 1 {
        for xx in 1..plane.width - 1 {
            gx += (plane.at(xx + 1, yy) - plane.at(xx - 1, yy)).abs();
            gy += (plane.at(xx, yy + 1) - plane.at(xx, yy - 1)).abs();
        }
    }
    if gx >= gy {
        measure_plane(&plane)
    } else {
        measure_plane(&image.crop(r, true))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sharpness: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let mut rgb = load(&args.image)?;
    if let Some(stops) = args.rcas {
        rgb = rcas(&rgb, stops);
    }
    if let Some(path) = &args.write {
        save(&rgb, path)?;
    }
    let image = luminance(&rgb, args.encoded);
    println!(
        "{}: an ideal pixel rises over 0.80 px, MTF50 0.60 c/px, MTF 0.90 at 0.25 and 0.64 at 0.5",
        args.image.display()
    );
    let mut failed = false;
    for r in &args.edge {
        let [x, y, w, h] = r;
        match measure(&image, *r) {
            Ok(m) => {
                println!(
                    "edge {x},{y},{w},{h}: {:+.1}°, levels {:.3}–{:.3}, rise {:.2} px, MTF50 {} c/px, MTF {:.2} at 0.25, {:.2} at 0.5",
                    m.angle,
                    m.dark,
                    m.light,
                    m.rise,
                    m.falls_to(0.5)
                        .map_or_else(|| "over 1".to_owned(), |f| format!("{f:.3}")),
                    m.at(0.25),
                    m.at(0.5)
                );
                if let Some(step) = args.curve {
                    let mut f = 0.0;
                    while f <= 1.0 + 1e-9 {
                        println!("  {f:.3} {:.3}", m.at(f));
                        f += step;
                    }
                }
            }
            Err(e) => {
                println!("edge {x},{y},{w},{h}: {e:#}");
                failed = true;
            }
        }
    }
    if failed {
        bail!("some edges could not be measured");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slanted edge rendered as the ideal pixel sees it: the light share of each pixel's
    /// square (16 × 16 points), then blurred by a Gaussian of `sigma` pixels.
    fn edge(size: usize, degrees: f64, sigma: f64) -> Plane {
        let (s, c) = degrees.to_radians().sin_cos();
        let centre = 0.5 * size as f64;
        let n = 16;
        let mut values = vec![0.0; size * size];
        for y in 0..size {
            for x in 0..size {
                let mut lit = 0;
                for j in 0..n {
                    for i in 0..n {
                        let px = x as f64 + (i as f64 + 0.5) / n as f64 - centre;
                        let py = y as f64 + (j as f64 + 0.5) / n as f64 - centre;
                        if px * c - py * s > 0.0 {
                            lit += 1;
                        }
                    }
                }
                values[y * size + x] = 0.05 + 0.85 * f64::from(lit) / f64::from(n * n);
            }
        }
        if sigma > 0.0 {
            let r = (4.0 * sigma).ceil() as i64;
            let kernel: Vec<f64> = (-r..=r)
                .map(|k| (-(k * k) as f64 / (2.0 * sigma * sigma)).exp())
                .collect();
            let total: f64 = kernel.iter().sum();
            let blur = |values: &[f64], horizontal: bool| -> Vec<f64> {
                let mut out = vec![0.0; size * size];
                for y in 0..size {
                    for x in 0..size {
                        let mut acc = 0.0;
                        for (k, w) in (-r..=r).zip(&kernel) {
                            let (xx, yy) = if horizontal {
                                ((x as i64 + k).clamp(0, size as i64 - 1) as usize, y)
                            } else {
                                (x, (y as i64 + k).clamp(0, size as i64 - 1) as usize)
                            };
                            acc += w * values[yy * size + xx];
                        }
                        out[y * size + x] = acc / total;
                    }
                }
                out
            };
            values = blur(&blur(&values, true), false);
        }
        Plane {
            width: size,
            height: size,
            values,
        }
    }

    #[test]
    fn an_ideal_pixel_has_the_box_filters_mtf() {
        let m = measure(&edge(64, 5.0, 0.0), [8, 8, 48, 48]).unwrap();
        assert!((m.angle.abs() - 5.0).abs() < 0.2, "angle {}", m.angle);
        let mtf50 = m.falls_to(0.5).unwrap();
        assert!((mtf50 - 0.603).abs() < 0.03, "MTF50 {mtf50}");
        assert!((m.at(0.25) - 0.90).abs() < 0.03, "MTF(0.25) {}", m.at(0.25));
        assert!((m.rise - 0.8).abs() < 0.15, "rise {}", m.rise);
    }

    #[test]
    fn a_gaussian_blur_lowers_the_mtf50_as_its_transform_says() {
        // The box's sinc times the Gaussian's exp(-2π²σ²f²): at σ = 1 pixel, 0.5 near 0.18.
        let m = measure(&edge(96, 6.0, 1.0), [8, 8, 80, 80]).unwrap();
        let sigma: f64 = 1.0;
        let expected = (1..2000)
            .map(|k| k as f64 / 2000.0)
            .find(|&f| {
                let x = std::f64::consts::PI * f;
                x.sin() / x * (-2.0 * x * x * sigma * sigma).exp() < 0.5
            })
            .unwrap();
        let mtf50 = m.falls_to(0.5).unwrap();
        assert!(
            (mtf50 - expected).abs() < 0.02,
            "MTF50 {mtf50}, expected {expected}"
        );
        assert!(m.rise > 2.0, "rise {}", m.rise);
    }

    #[test]
    fn a_near_horizontal_edge_is_measured_across_the_columns() {
        let m = measure(&edge(64, 84.0, 0.0), [8, 8, 48, 48]).unwrap();
        let mtf50 = m.falls_to(0.5).unwrap();
        assert!((mtf50 - 0.603).abs() < 0.03, "MTF50 {mtf50}");
    }
}
