//! `contact-sheet out.png a.png b.png … [--columns N] [--width W] [--gap G]`: lays captures
//! out left to right, top to bottom, as thumbnails `W` pixels wide (aspect kept, triangle
//! filter) separated by `G` pixels of black. Used for the capture sequences in `docs/demos`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use image::{Rgba, RgbaImage, imageops};

#[derive(Parser, Debug)]
#[command(about = "Lays PNG captures out on a grid of thumbnails")]
struct Args {
    /// The sheet to write.
    output: PathBuf,
    /// The captures, in reading order.
    inputs: Vec<PathBuf>,
    /// Thumbnails per row.
    #[arg(long, default_value_t = 4)]
    columns: u32,
    /// Width of one thumbnail in pixels.
    #[arg(long, default_value_t = 400)]
    width: u32,
    /// Pixels between thumbnails.
    #[arg(long, default_value_t = 4)]
    gap: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.inputs.is_empty() {
        bail!("no input images");
    }
    let columns = args.columns.max(1);
    let mut thumbs = Vec::with_capacity(args.inputs.len());
    for path in &args.inputs {
        let image = image::open(path)
            .with_context(|| format!("open {}", path.display()))?
            .to_rgba8();
        let height = (u64::from(image.height()) * u64::from(args.width)
            / u64::from(image.width().max(1))) as u32;
        thumbs.push(imageops::resize(
            &image,
            args.width,
            height.max(1),
            imageops::FilterType::Triangle,
        ));
    }
    let cell_height = thumbs.iter().map(RgbaImage::height).max().unwrap_or(1);
    let rows = (thumbs.len() as u32).div_ceil(columns);
    let used_columns = columns.min(thumbs.len() as u32);
    let mut sheet = RgbaImage::from_pixel(
        used_columns * args.width + (used_columns - 1) * args.gap,
        rows * cell_height + (rows - 1) * args.gap,
        Rgba([0, 0, 0, 255]),
    );
    for (i, thumb) in thumbs.iter().enumerate() {
        let (column, row) = (i as u32 % columns, i as u32 / columns);
        imageops::overlay(
            &mut sheet,
            thumb,
            i64::from(column * (args.width + args.gap)),
            i64::from(row * (cell_height + args.gap)),
        );
    }
    sheet
        .save(&args.output)
        .with_context(|| format!("write {}", args.output.display()))?;
    println!(
        "{}: {} images, {} × {}",
        args.output.display(),
        thumbs.len(),
        sheet.width(),
        sheet.height()
    );
    Ok(())
}
