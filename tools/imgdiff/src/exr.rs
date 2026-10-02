//! A minimal OpenEXR writer: one part, 32-bit float R, G and B, no compression. For handing
//! the light HDR-ꟻLIP compares to NVIDIA's tool, or to an HDR image viewer (issue #126).
//!
//! The layout (OpenEXR's "File Layout"): the magic number and version, the header's attributes
//! (name, type, size, value) ending with a zero byte, a table of each scanline's offset, then
//! the scanlines: y, the size of the data, and the row of each channel in alphabetical order.

use std::path::Path;

use anyhow::{Context, Result};

use crate::flip::Color;

fn attribute(out: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]) {
    for text in [name, kind] {
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
}

/// The file of an image `width` pixels wide, rows from the top.
pub fn encode(image: &[Color], width: usize) -> Vec<u8> {
    let height = image.len() / width;
    let mut out = Vec::new();
    out.extend_from_slice(&20_000_630_u32.to_le_bytes());
    out.extend_from_slice(&2_u32.to_le_bytes());
    let mut channels = Vec::new();
    for name in ["B", "G", "R"] {
        channels.extend_from_slice(name.as_bytes());
        // FLOAT, linear 0 and three reserved bytes, sampled every pixel.
        channels.push(0);
        channels.extend_from_slice(&2_i32.to_le_bytes());
        channels.extend_from_slice(&[0; 4]);
        channels.extend_from_slice(&1_i32.to_le_bytes());
        channels.extend_from_slice(&1_i32.to_le_bytes());
    }
    channels.push(0);
    attribute(&mut out, "channels", "chlist", &channels);
    attribute(&mut out, "compression", "compression", &[0]);
    let window: Vec<u8> = [0, 0, width as i32 - 1, height as i32 - 1]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    attribute(&mut out, "dataWindow", "box2i", &window);
    attribute(&mut out, "displayWindow", "box2i", &window);
    attribute(&mut out, "lineOrder", "lineOrder", &[0]);
    attribute(
        &mut out,
        "pixelAspectRatio",
        "float",
        &1.0_f32.to_le_bytes(),
    );
    attribute(&mut out, "screenWindowCenter", "v2f", &[0; 8]);
    attribute(
        &mut out,
        "screenWindowWidth",
        "float",
        &1.0_f32.to_le_bytes(),
    );
    out.push(0);
    let row_bytes = width * 3 * 4;
    let first = out.len() + height * 8;
    for y in 0..height {
        out.extend_from_slice(&((first + y * (8 + row_bytes)) as u64).to_le_bytes());
    }
    for (y, row) in image.chunks(width).enumerate() {
        out.extend_from_slice(&(y as i32).to_le_bytes());
        out.extend_from_slice(&(row_bytes as i32).to_le_bytes());
        for channel in [2, 1, 0] {
            for pixel in row {
                out.extend_from_slice(&pixel[channel].to_le_bytes());
            }
        }
    }
    out
}

/// Writes `image`, `width` pixels wide, to `path`.
pub fn write(path: &Path, image: &[Color], width: usize) -> Result<()> {
    std::fs::write(path, encode(image, width)).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_point_at_the_scanlines() {
        let image: Vec<Color> = (0..6).map(|i| [i as f32, 10.0 + i as f32, 0.5]).collect();
        let file = encode(&image, 3);
        let read_u64 = |at: usize| u64::from_le_bytes(file[at..at + 8].try_into().unwrap());
        let read_f32 = |at: usize| f32::from_le_bytes(file[at..at + 4].try_into().unwrap());
        let table = file.len() - 2 * (8 + 3 * 3 * 4) - 2 * 8;
        assert_eq!(file[table - 1], 0, "the header ends with a zero byte");
        let second = read_u64(table + 8) as usize;
        assert_eq!(&file[second..second + 4], &1_i32.to_le_bytes(), "y");
        // B, G then R of the second row (pixels 3, 4 and 5).
        assert_eq!(read_f32(second + 8), 0.5);
        assert_eq!(read_f32(second + 8 + 12), 13.0);
        assert_eq!(read_f32(second + 8 + 24 + 8), 5.0);
    }
}
