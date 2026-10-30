//! `contact-sheet out.png a.png b.png … [--columns N] [--width W] [--gap G] [--crop X,Y,W,H]
//! [--label A --label B …]`: lays captures out left to right, top to bottom, as thumbnails `W` pixels
//! wide (aspect kept; triangle filter when shrinking, nearest when enlarging so pixels stay
//! visible) separated by `G` pixels of black, optionally cropped first to the same rectangle,
//! each with its label (where it comes from: "Forge", "Falcor") small in its lower left corner.
//! Used for the capture sequences and zoomed details in `docs/demos`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use anyhow::{Context, Result, bail};
use clap::Parser;
use image::{Rgba, RgbaImage, imageops};

/// The labels' font: the overlay's JetBrains Mono (OFL), built in.
const FONT: &[u8] =
    include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf");

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
    /// Crop every input to this rectangle first, "x,y,width,height".
    #[arg(long, value_parser = parse_rect)]
    crop: Option<[u32; 4]>,
    /// A label for each input in order, one `--label` each (an empty one for none): drawn small
    /// in the thumbnail's lower left corner.
    #[arg(long = "label")]
    labels: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.inputs.is_empty() {
        bail!("no input images");
    }
    let columns = args.columns.max(1);
    let mut thumbs = Vec::with_capacity(args.inputs.len());
    for path in &args.inputs {
        let mut image = image::open(path)
            .with_context(|| format!("open {}", path.display()))?
            .to_rgba8();
        if let Some([x, y, w, h]) = args.crop {
            if x + w > image.width() || y + h > image.height() {
                bail!("{}: crop outside the image", path.display());
            }
            image = imageops::crop_imm(&image, x, y, w, h).to_image();
        }
        let filter = if args.width > image.width() {
            imageops::FilterType::Nearest
        } else {
            imageops::FilterType::Triangle
        };
        let height = (u64::from(image.height()) * u64::from(args.width)
            / u64::from(image.width().max(1))) as u32;
        let mut thumb = imageops::resize(&image, args.width, height.max(1), filter);
        if let Some(text) = args
            .labels
            .get(thumbs.len())
            .filter(|t| !t.trim().is_empty())
        {
            label(&mut thumb, text.trim())?;
        }
        thumbs.push(thumb);
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

/// Writes `text` in the lower left corner of `image`: light grey on a translucent dark band, its
/// height a 40th of the image's width (11 pixels at the least).
fn label(image: &mut RgbaImage, text: &str) -> Result<()> {
    let font = FontRef::try_from_slice(FONT).context("the labels' font")?;
    let px = (image.width() as f32 / 40.0).max(11.0);
    let scaled = font.as_scaled(PxScale::from(px));
    let pad = (px / 3.0).round() as i32;
    let width: f32 = text
        .chars()
        .map(|c| scaled.h_advance(scaled.glyph_id(c)))
        .sum();
    let (box_w, box_h) = (
        width.ceil() as i32 + 2 * pad,
        scaled.height().ceil() as i32 + pad,
    );
    let (x0, y0) = (0, image.height() as i32 - box_h);
    let blend = |image: &mut RgbaImage, x: i32, y: i32, value: f32, alpha: f32| {
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            return;
        }
        let p = image.get_pixel_mut(x as u32, y as u32);
        for c in 0..3 {
            p[c] = (f32::from(p[c]) * (1.0 - alpha) + value * alpha).round() as u8;
        }
    };
    for y in y0..y0 + box_h {
        for x in x0..x0 + box_w {
            blend(image, x, y, 0.0, 0.45);
        }
    }
    let mut pen = (x0 + pad) as f32;
    let baseline = (y0 + pad / 2) as f32 + scaled.ascent();
    for c in text.chars() {
        let mut glyph = scaled.scaled_glyph(c);
        glyph.position = point(pen, baseline);
        pen += scaled.h_advance(glyph.id);
        if let Some(outlined) = scaled.font().outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, coverage| {
                let (px, py) = (
                    bounds.min.x as i32 + x as i32,
                    bounds.min.y as i32 + y as i32,
                );
                blend(image, px, py, 235.0, coverage.clamp(0.0, 1.0) * 0.9);
            });
        }
    }
    Ok(())
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
