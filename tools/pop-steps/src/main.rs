//! `pop-steps DIR [--tile 16] [--min 5] [--region X,Y,W,H] [--from N] [--top 20]`: finds pops in
//! a capture sequence (a demo's `--fixed-step --capture-every N`, frames named `*-NNNNN.png`).
//! Each capture is cut into `tile`-pixel squares and their mean sRGB luma taken; a tile pops at a
//! capture when its step from the capture before is at least `min` levels and beats the steps
//! before and after it by as much (score = |d_k| − max(|d_k−1|, |d_k+1|)): a jump, not the steady
//! change of a moving camera. A tree crossing from its impostor to its shells in one frame scored
//! 5–12 (#363); its dithered crossing scores under 3.
//!
//! Near objects sweeping past (trunks, poles, their shadows) score as pops too, so keep the
//! `--region` to the middle distance on a camera at street level. Captures missing from the
//! sequence (the frame `--capture` writes on its own) break it: no step is measured across a gap.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Finds pops in a capture sequence")]
struct Args {
    /// The directory holding the captures (`*-NNNNN.png`, the number the frame).
    dir: PathBuf,
    /// Only the captures whose name starts with this (default: every `*-NNNNN.png`).
    #[arg(long, default_value = "")]
    prefix: String,
    /// The tiles' size, pixels.
    #[arg(long, default_value_t = 16)]
    tile: u32,
    /// The smallest step and score that count, sRGB levels (0–255).
    #[arg(long, default_value_t = 5.0)]
    min: f32,
    /// Only the tiles inside this rectangle, "x,y,width,height".
    #[arg(long, value_parser = parse_rect)]
    region: Option<[u32; 4]>,
    /// Skip the captures of frames before this one (the first frames settle).
    #[arg(long, default_value_t = 0)]
    from: u32,
    /// How many of the strongest pops to list.
    #[arg(long, default_value_t = 20)]
    top: usize,
}

/// One tile's pop: the capture it lands in, the tile's corner and its step and score.
struct Pop {
    frame: u32,
    x: u32,
    y: u32,
    step: f32,
    score: f32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let tile = args.tile.max(1);
    let mut captures = captures(&args.dir, &args.prefix)?;
    captures.retain(|(frame, _)| *frame >= args.from);
    if captures.len() < 4 {
        bail!(
            "{}: {} captures, at least 4 needed",
            args.dir.display(),
            captures.len()
        );
    }
    // The sequence's step, the commonest gap between captures: no step is measured across others.
    let gaps: Vec<u32> = captures.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let step = *gaps
        .iter()
        .max_by_key(|&&g| gaps.iter().filter(|&&h| h == g).count())
        .expect("gaps");
    let mut size = (0, 0);
    let mut means = Vec::with_capacity(captures.len());
    for (_, path) in &captures {
        let image = image::open(path)
            .with_context(|| format!("open {}", path.display()))?
            .to_rgb8();
        if means.is_empty() {
            size = image.dimensions();
        } else if image.dimensions() != size {
            bail!("{}: {:?}, not {size:?}", path.display(), image.dimensions());
        }
        means.push(tile_means(&image, tile));
    }
    let (nx, ny) = (size.0 / tile, size.1 / tile);
    let [rx, ry, rw, rh] = args.region.unwrap_or([0, 0, size.0, size.1]);
    let inside = |i: u32, j: u32| {
        let (x, y) = (i * tile, j * tile);
        x >= rx && y >= ry && x + tile <= rx + rw && y + tile <= ry + rh
    };
    let frames: Vec<u32> = captures.iter().map(|(f, _)| *f).collect();
    let (mut pops, pairs) = find_pops(&frames, &means, step, nx, tile, args.min, inside);
    for &frame in &frames {
        let count = pops.iter().filter(|p| p.frame == frame).count();
        if count > 0 {
            println!("frame {frame}: {count} tiles");
        }
    }
    pops.sort_by(|p, q| q.score.total_cmp(&p.score));
    println!("-- the strongest (frame, tile corner, step, score):");
    for p in pops.iter().take(args.top) {
        println!(
            "frame {:5}  {:4},{:4}  {:+6.1}  {:5.1}",
            p.frame, p.x, p.y, p.step, p.score
        );
    }
    println!(
        "{} pops of {} levels or more over {pairs} steps of {step} frames ({} captures, {} × {} tiles)",
        pops.len(),
        args.min,
        captures.len(),
        nx,
        ny
    );
    Ok(())
}

/// The pops of a sequence: `frames` its captures' frame numbers, `means` their tiles' mean luma
/// (`nx` tiles a row, `tile` pixels each), `step` the frames between captures; only the tiles
/// `inside` takes, by column and row. And how many steps were measured (none across a gap).
fn find_pops(
    frames: &[u32],
    means: &[Vec<f32>],
    step: u32,
    nx: u32,
    tile: u32,
    min: f32,
    inside: impl Fn(u32, u32) -> bool,
) -> (Vec<Pop>, usize) {
    let mut pops = Vec::new();
    let mut pairs = 0;
    for k in 2..frames.len().saturating_sub(1) {
        if (k - 2..k + 1).any(|i| frames[i + 1] - frames[i] != step) {
            continue;
        }
        pairs += 1;
        let (a, b, c, d) = (&means[k - 2], &means[k - 1], &means[k], &means[k + 1]);
        for q in 0..c.len() {
            let (i, j) = (q as u32 % nx, q as u32 / nx);
            if !inside(i, j) {
                continue;
            }
            let change = c[q] - b[q];
            let score = change.abs() - (b[q] - a[q]).abs().max((d[q] - c[q]).abs());
            if change.abs() >= min && score >= min {
                pops.push(Pop {
                    frame: frames[k],
                    x: i * tile,
                    y: j * tile,
                    step: change,
                    score,
                });
            }
        }
    }
    (pops, pairs)
}

/// The captures in `dir` named `PREFIX…-NNNNN.png`, by frame.
fn captures(dir: &Path, prefix: &str) -> Result<Vec<(u32, PathBuf)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".png") else {
            continue;
        };
        if !stem.starts_with(prefix) {
            continue;
        }
        let Some((_, number)) = stem.rsplit_once('-') else {
            continue;
        };
        if let Ok(frame) = number.parse::<u32>() {
            out.push((frame, path));
        }
    }
    out.sort_by_key(|(frame, _)| *frame);
    Ok(out)
}

/// The mean sRGB luma (Rec. 709 weights on the 8-bit values) of each `tile`-pixel square, rows
/// first; the right and bottom remainders left out.
fn tile_means(image: &image::RgbImage, tile: u32) -> Vec<f32> {
    let (nx, ny) = (image.width() / tile, image.height() / tile);
    let mut sums = vec![0.0f32; (nx * ny) as usize];
    for y in 0..ny * tile {
        for x in 0..nx * tile {
            let [r, g, b] = image.get_pixel(x, y).0;
            let luma = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
            sums[((y / tile) * nx + x / tile) as usize] += luma;
        }
    }
    let area = (tile * tile) as f32;
    sums.iter().map(|s| s / area).collect()
}

fn parse_rect(text: &str) -> std::result::Result<[u32; 4], String> {
    let values: Vec<u32> = text
        .split(',')
        .map(|v| v.trim().parse::<u32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    match values[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok([x, y, w, h]),
        _ => Err("expected x,y,width,height with a non-empty size".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile that jumps in one capture of a smooth sequence is found, the smooth change around
    /// it is not, and no step is measured across a missing capture.
    #[test]
    fn a_jump_among_smooth_steps_is_a_pop() {
        // Two tiles brightening by 2 a capture; the first jumps by 40 at the fourth capture.
        let means: Vec<Vec<f32>> = (0..6)
            .map(|k| {
                let base = 100.0 + 2.0 * k as f32;
                vec![base + if k >= 3 { 40.0 } else { 0.0 }, base]
            })
            .collect();
        let frames = [0, 3, 6, 9, 12, 15];
        let (pops, pairs) = find_pops(&frames, &means, 3, 2, 16, 5.0, |_, _| true);
        assert_eq!(pairs, 3);
        assert_eq!(pops.len(), 1);
        let p = &pops[0];
        assert_eq!((p.frame, p.x, p.y), (9, 0, 0));
        assert!((p.step - 42.0).abs() < 1e-4 && (p.score - 40.0).abs() < 1e-4);
        // The capture of frame 6 missing (`--capture` took it): the jump is not measured.
        let (gappy, pairs) = find_pops(
            &[0, 3, 9, 12, 15],
            &[&means[..2], &means[3..]].concat(),
            3,
            2,
            16,
            5.0,
            |_, _| true,
        );
        assert_eq!((gappy.len(), pairs), (0, 0));
        // Outside the region: nothing.
        let (none, _) = find_pops(&frames, &means, 3, 2, 16, 5.0, |i, _| i == 1);
        assert!(none.is_empty());
    }
}
