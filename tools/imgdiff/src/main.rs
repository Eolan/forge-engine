//! `imgdiff a.png b.png [--out diff.png] [--tolerance N]`: the golden-image check used by the
//! demos. Exit code 1 when more than `--max-different` pixels differ by more than `tolerance`.
//!
//! For looking at a difference: `--report N` prints the first N differing pixels with both
//! colours; `--crop x,y,w,h --zoom K --crops out.png` writes the crop of both images and of
//! the diff side by side, enlarged K times without filtering.

#![forbid(unsafe_code)]

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
    Ok(if different > args.max_different {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
