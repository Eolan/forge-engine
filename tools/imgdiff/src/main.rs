//! `imgdiff a.png b.png [--out diff.png] [--tolerance N]`: the golden-image check used by the
//! demos. Exit code 1 when more than `--max-different` pixels differ by more than `tolerance`.
//!
//! For looking at a difference: `--report N` prints the first N differing pixels with both
//! colours; `--crop x,y,w,h --zoom K --crops out.png` writes the crop of both images and of
//! the diff side by side, enlarged K times without filtering.
//!
//! For motion: `--then next_a.png next_b.png` counts the pixels whose change to the next frame
//! differs between the two sequences (LOD pops against a full-detail reference, issue #65).
//!
//! Whether a person would see the difference: when any pixel differs, the perceptual error
//! LDR-ꟻLIP ([`flip`], issue #75) prints its mean, weighted quartiles, percentiles and
//! largest value, `a` taken as the reference. `--flip-map` writes its error map. With
//! `--max-flip` or `--max-flip-mean`, the exit code judges ꟻLIP instead of the pixel count.

#![forbid(unsafe_code)]

mod flip;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Compare two PNG images")]
struct Args {
    /// First image.
    a: PathBuf,
    /// Second image.
    b: PathBuf,
    /// Write a diff image (white where pixels differ beyond the tolerance).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Per-channel difference (0–255) below which pixels count as equal.
    #[arg(long, default_value_t = 2)]
    tolerance: u8,
    /// Allowed number of differing pixels before the exit code is 1.
    #[arg(long, default_value_t = 0)]
    max_different: u64,
    /// Print the first N differing pixels (x, y, colour in a, colour in b).
    #[arg(long, default_value_t = 0)]
    report: usize,
    /// Region to write with `--crops`: "x,y,width,height".
    #[arg(long, value_parser = parse_rect)]
    crop: Option<[u32; 4]>,
    /// Enlargement of the crops (nearest neighbour).
    #[arg(long, default_value_t = 4)]
    zoom: u32,
    /// Write the crops of a, b and the diff side by side to this path.
    #[arg(long)]
    crops: Option<PathBuf>,
    /// The pixel "x,y" of `a` whose colour is the background: counts the pixels that show
    /// the background in `b` but not in `a`, and those of them whose eight neighbours in `a`
    /// show none either (inside a surface of `a`: a hole in `b`, not a moved outline). The
    /// cluster-streaming check (issue #36).
    #[arg(long, value_parser = parse_point)]
    background_at: Option<[u32; 2]>,
    /// The next frames of `a` and `b`: counts the pixels whose change from `a` to its next
    /// frame differs from the change from `b` to its next frame by more than `tolerance`. With
    /// `a` drawn with LOD and `b` at full detail along the same path, these are the LOD pops:
    /// the image changed where the reference did not (issue #65).
    /// `--out` then writes these pixels, and `--crops` shows them beside the crops.
    #[arg(long, num_args = 2, value_names = ["NEXT_A", "NEXT_B"])]
    then: Option<Vec<PathBuf>>,
    /// Skip the perceptual error (LDR-ꟻLIP).
    #[arg(long)]
    no_flip: bool,
    /// Pixels per degree of visual angle for ꟻLIP. The default is the reference's: a 0.7 m
    /// wide 3840-pixel monitor seen from 0.7 m, about 67.
    #[arg(long)]
    ppd: Option<f32>,
    /// Write the ꟻLIP error map (magma colours: black is no error, pale yellow the largest).
    #[arg(long)]
    flip_map: Option<PathBuf>,
    /// Judge by ꟻLIP instead of the pixel count: exit code 1 when a pixel's error reaches
    /// this value.
    #[arg(long)]
    max_flip: Option<f32>,
    /// Judge by ꟻLIP instead of the pixel count: exit code 1 when the mean error reaches this
    /// value.
    #[arg(long)]
    max_flip_mean: Option<f32>,
}

fn parse_point(text: &str) -> std::result::Result<[u32; 2], String> {
    let parts: Vec<u32> = text
        .split(',')
        .map(|p| p.trim().parse::<u32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    parts.try_into().map_err(|_| "expected x,y".to_owned())
}

/// Pixels showing `background` in `b` but not in `a`, and those of them whose eight
/// neighbours in `a` show none either.
fn background_only_in_b(
    a: &image::RgbaImage,
    b: &image::RgbaImage,
    background: image::Rgba<u8>,
    inside_at: &mut Vec<(u32, u32)>,
) -> (u64, u64) {
    let (width, height) = a.dimensions();
    let is_background = |image: &image::RgbaImage, x: u32, y: u32| {
        image.get_pixel(x, y).0[..3] == background.0[..3]
    };
    let (mut only_b, mut inside) = (0, 0);
    for y in 0..height {
        for x in 0..width {
            if !is_background(b, x, y) || is_background(a, x, y) {
                continue;
            }
            only_b += 1;
            let mut near_background = false;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    near_background |= is_background(a, nx, ny);
                }
            }
            inside += u64::from(!near_background);
            if !near_background {
                inside_at.push((x, y));
            }
        }
    }
    (only_b, inside)
}

/// Pixels where `a → next_a` and `b → next_b` differ by more than `tolerance` in a channel,
/// marked white in `mask`.
fn changed_differently(
    [a, next_a, b, next_b]: [&image::RgbaImage; 4],
    tolerance: u8,
    mask: &mut image::RgbaImage,
) -> u64 {
    let mut count = 0;
    for (x, y, pa) in a.enumerate_pixels() {
        let (na, pb, nb) = (
            next_a.get_pixel(x, y),
            b.get_pixel(x, y),
            next_b.get_pixel(x, y),
        );
        let err = (0..3)
            .map(|c| {
                let change = |from: u8, to: u8| i16::from(to) - i16::from(from);
                change(pa[c], na[c]).abs_diff(change(pb[c], nb[c]))
            })
            .max()
            .unwrap_or(0);
        let bad = err > u16::from(tolerance);
        count += u64::from(bad);
        mask.put_pixel(
            x,
            y,
            image::Rgba(if bad { [255; 4] } else { [0, 0, 0, 255] }),
        );
    }
    count
}

fn parse_rect(text: &str) -> std::result::Result<[u32; 4], String> {
    let parts: Vec<u32> = text
        .split(',')
        .map(|p| p.trim().parse::<u32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    match parts.as_slice() {
        [x, y, w, h] => Ok([*x, *y, *w, *h]),
        _ => Err("expected x,y,width,height".to_owned()),
    }
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let judge_by_flip = args.max_flip.is_some() || args.max_flip_mean.is_some();
    anyhow::ensure!(
        !(judge_by_flip && args.no_flip),
        "--max-flip and --max-flip-mean need ꟻLIP: drop --no-flip"
    );
    let a = image::open(&args.a)
        .with_context(|| format!("open {}", args.a.display()))?
        .to_rgba8();
    let b = image::open(&args.b)
        .with_context(|| format!("open {}", args.b.display()))?
        .to_rgba8();
    if a.dimensions() != b.dimensions() {
        anyhow::bail!(
            "size mismatch: {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
    }
    let (width, height) = a.dimensions();
    let mut different = 0_u64;
    let mut max_error = 0_u8;
    let mut sum_error = 0_u64;
    let mut darker_in_a = 0_u64;
    let mut reported = 0_usize;
    let mut diff = image::RgbaImage::new(width, height);
    for (x, y, pa) in a.enumerate_pixels() {
        let pb = b.get_pixel(x, y);
        let err = (0..3).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
        max_error = max_error.max(err);
        sum_error += u64::from(err);
        let bad = err > args.tolerance;
        different += u64::from(bad);
        if bad {
            let luma = |p: &image::Rgba<u8>| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
            darker_in_a += u64::from(luma(pa) < luma(pb));
            if reported < args.report {
                println!("({x}, {y}): a = {:?}  b = {:?}", &pa.0[..3], &pb.0[..3]);
                reported += 1;
            }
        }
        diff.put_pixel(
            x,
            y,
            if bad {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 255])
            },
        );
    }
    let total = u64::from(width) * u64::from(height);
    println!(
        "{} vs {}: {different} / {total} pixels differ (> {}), {:.4} %, max channel error {max_error}, mean error {:.4}, darker in a: {darker_in_a}",
        args.a.display(),
        args.b.display(),
        args.tolerance,
        different as f64 * 100.0 / total as f64,
        sum_error as f64 / total as f64
    );
    let mut flip_fails = false;
    if !args.no_flip {
        let ppd = args.ppd.unwrap_or_else(flip::default_ppd);
        // Identical images have no error anywhere: skip the filtering.
        let (errors, s) = if max_error > 0 {
            let errors = flip::error_map(&a, &b, ppd);
            let s = flip::stats(&errors, width);
            (errors, s)
        } else {
            (vec![0.0; total as usize], flip::Stats::default())
        };
        let [p50, p99, p999] = s.percentiles;
        let [q1, q3] = s.weighted_quartiles;
        let [above_1, above_2, above_5] = s.above;
        println!(
            "LDR-FLIP at {ppd:.1} ppd: mean {:.6}, weighted median {:.6}, weighted quartiles {q1:.6} / {q3:.6}, p50 {p50:.6}, p99 {p99:.6}, p99.9 {p999:.6}, max {:.6} at ({}, {}), pixels >= 0.1: {above_1}, >= 0.2: {above_2}, >= 0.5: {above_5}",
            s.mean, s.weighted_median, s.max, s.max_at.0, s.max_at.1
        );
        flip_fails = args.max_flip.is_some_and(|limit| s.max >= limit)
            || args.max_flip_mean.is_some_and(|limit| s.mean >= limit);
        if let Some(path) = &args.flip_map {
            flip::magma_image(&errors, width, height)
                .save(path)
                .with_context(|| format!("write {}", path.display()))?;
        }
    }
    if let Some([x, y]) = args.background_at {
        let background = *a.get_pixel(x.min(width - 1), y.min(height - 1));
        let mut inside_at = Vec::new();
        let (only_b, inside) = background_only_in_b(&a, &b, background, &mut inside_at);
        let (only_a, inside_a) = background_only_in_b(&b, &a, background, &mut Vec::new());
        println!(
            "background {:?}: {only_b} pixels only in b ({inside} inside a's surfaces), {only_a} only in a ({inside_a} inside b's)",
            &background.0[..3]
        );
        for (x, y) in inside_at.iter().take(args.report) {
            println!("inside a's surfaces, background in b: ({x}, {y})");
        }
    }
    if let Some(next) = &args.then {
        let open = |path: &PathBuf| -> Result<image::RgbaImage> {
            let image = image::open(path)
                .with_context(|| format!("open {}", path.display()))?
                .to_rgba8();
            anyhow::ensure!(
                image.dimensions() == a.dimensions(),
                "size mismatch: {}",
                path.display()
            );
            Ok(image)
        };
        let (next_a, next_b) = (open(&next[0])?, open(&next[1])?);
        // `--out` shows these pixels instead of the plain difference.
        let count = changed_differently([&a, &next_a, &b, &next_b], args.tolerance, &mut diff);
        println!(
            "changed differently (> {}): {count} / {total} pixels, {:.4} %",
            args.tolerance,
            count as f64 * 100.0 / total as f64
        );
    }
    if let Some(out) = &args.out {
        diff.save(out)
            .with_context(|| format!("write {}", out.display()))?;
    }
    if let (Some([x, y, w, h]), Some(path)) = (args.crop, &args.crops) {
        let zoom = args.zoom.max(1);
        let w = w.min(width.saturating_sub(x)).max(1);
        let h = h.min(height.saturating_sub(y)).max(1);
        let mut sheet = image::RgbaImage::new((w * zoom + 2) * 3, h * zoom);
        for (column, source) in [&a, &b, &diff].into_iter().enumerate() {
            let x0 = column as u32 * (w * zoom + 2);
            for sy in 0..h * zoom {
                for sx in 0..w * zoom {
                    sheet.put_pixel(x0 + sx, sy, *source.get_pixel(x + sx / zoom, y + sy / zoom));
                }
            }
        }
        sheet
            .save(path)
            .with_context(|| format!("write {}", path.display()))?;
    }
    let fails = if judge_by_flip {
        flip_fails
    } else {
        different > args.max_different
    };
    Ok(if fails {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
