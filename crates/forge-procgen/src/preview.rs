//! PNG previews of the pipeline's stages: the height in 16 bits, a hillshade, the drainage on
//! a log scale, the basins in colours, and an overview with the sea, hypsometric tints, rivers
//! and lakes. How a stage is looked at before the GPU draws it (and how a cloud session looks
//! at it at all).

use std::path::Path;

use image::{ImageBuffer, Luma, Rgb};

use crate::field::Field2;
use crate::flow::Flow;
use crate::hydrology::{Lakes, Rivers};

/// The height as a 16-bit grey PNG, black the lowest sample and white the highest.
pub fn write_height(height: &Field2<f32>, path: &Path) -> image::ImageResult<()> {
    let (lo, hi) = height.min_max();
    let scale = if hi > lo { 65535.0 / (hi - lo) } else { 0.0 };
    let image = ImageBuffer::from_fn(height.size, height.size, |x, y| {
        Luma([((height.get(x, y) - lo) * scale)
            .round()
            .clamp(0.0, 65535.0) as u16])
    });
    image.save(path)
}

/// Lambert shading of the surface by a sun at `azimuth` degrees (0 north, clockwise) and
/// `elevation` degrees over the horizon, in `[0, 1]`.
fn shade(height: &Field2<f32>, x: u32, y: u32, sun: [f32; 3]) -> f32 {
    let (dx, dy) = height.gradient(x, y);
    // The normal of z = h(x, y) with y rows along +z: (−∂h/∂x, 1, −∂h/∂y).
    let (nx, ny, nz) = (-dx, 1.0, -dy);
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    ((nx * sun[0] + ny * sun[1] + nz * sun[2]) / len).clamp(0.0, 1.0)
}

/// The sun's direction from its azimuth and elevation, degrees.
fn sun_direction(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    [el.cos() * az.sin(), el.sin(), -el.cos() * az.cos()]
}

/// The hillshade as an 8-bit grey PNG, the sun from the north-west at 45°.
pub fn write_hillshade(height: &Field2<f32>, path: &Path) -> image::ImageResult<()> {
    let sun = sun_direction(315.0, 45.0);
    let image = ImageBuffer::from_fn(height.size, height.size, |x, y| {
        Luma([(shade(height, x, y, sun) * 255.0) as u8])
    });
    image.save(path)
}

/// The drainage area on a log scale as an 8-bit grey PNG: white where the rivers are.
pub fn write_flow(flow: &Flow, size: u32, path: &Path) -> image::ImageResult<()> {
    let max = flow.area.iter().copied().max().unwrap_or(1).max(1) as f32;
    let image = ImageBuffer::from_fn(size, size, |x, y| {
        let a = flow.area[(y * size + x) as usize] as f32;
        Luma([(a.ln() / max.ln() * 255.0) as u8])
    });
    image.save(path)
}

/// The network: the hillshade in grey, the sea dark, the lakes a flat blue, the rivers by
/// Strahler order from a pale blue (first order) to a deep one (fourth and above).
pub fn write_network(
    height: &Field2<f32>,
    rivers: &Rivers,
    lakes: &Lakes,
    sea_level: f32,
    path: &Path,
) -> image::ImageResult<()> {
    let sun = sun_direction(315.0, 45.0);
    let image = ImageBuffer::from_fn(height.size, height.size, |x, y| {
        let i = height.index(x, y);
        if height.get(x, y) <= sea_level {
            return Rgb([26, 62, 118]);
        }
        if lakes.lake_of[i] != u32::MAX {
            return Rgb([70, 130, 190]);
        }
        match rivers.order[i] {
            0 => {
                let g = (60.0 + 195.0 * shade(height, x, y, sun)) as u8;
                Rgb([g, g, g])
            }
            1 => Rgb([120, 170, 230]),
            2 => Rgb([70, 130, 220]),
            3 => Rgb([30, 90, 200]),
            _ => Rgb([10, 50, 160]),
        }
    });
    image.save(path)
}

/// The basins: the land hillshaded in the colour of the outlet it drains to, the eight largest
/// basins each in a hue of their own (largest first: red, orange, yellow, green, cyan, blue,
/// violet, pink) and the rest grey, rivers where more than `river_cells` cells drain through
/// a sample darker.
pub fn write_basins(
    height: &Field2<f32>,
    flow: &Flow,
    sea_level: f32,
    river_cells: u32,
    path: &Path,
) -> image::ImageResult<()> {
    const HUES: [[f32; 3]; 8] = [
        [230.0, 70.0, 60.0],
        [240.0, 150.0, 50.0],
        [230.0, 210.0, 60.0],
        [90.0, 190.0, 80.0],
        [70.0, 200.0, 200.0],
        [80.0, 120.0, 230.0],
        [160.0, 100.0, 220.0],
        [230.0, 120.0, 190.0],
    ];
    let outlet = flow.outlets();
    let basins = flow.basins();
    let mut hue = vec![u8::MAX; outlet.len()];
    for (k, &(o, _)) in basins.iter().take(HUES.len()).enumerate() {
        hue[o as usize] = k as u8;
    }
    let sun = sun_direction(315.0, 45.0);
    let image = ImageBuffer::from_fn(height.size, height.size, |x, y| {
        let i = height.index(x, y);
        if height.get(x, y) <= sea_level {
            return Rgb([26, 62, 118]);
        }
        let tint = match hue[outlet[i] as usize] {
            u8::MAX => [150.0, 150.0, 150.0],
            k => HUES[usize::from(k)],
        };
        let light = (0.4 + 0.6 * shade(height, x, y, sun))
            * if flow.area[i] > river_cells {
                0.35
            } else {
                1.0
            };
        Rgb([
            (tint[0] * light) as u8,
            (tint[1] * light) as u8,
            (tint[2] * light) as u8,
        ])
    });
    image.save(path)
}

/// The overview: the sea in blue, land hillshaded under a tint from green lowlands to grey
/// peaks, rivers in blue where more than `river_cells` cells drain through a sample, lakes
/// (the flood's fill over the eroded field, where `lake_depth` is given) in a lighter blue.
pub fn write_overview(
    height: &Field2<f32>,
    flow: &Flow,
    sea_level: f32,
    river_cells: u32,
    lake_depth: Option<&Field2<f32>>,
    path: &Path,
) -> image::ImageResult<()> {
    let sun = sun_direction(315.0, 45.0);
    let (_, hi) = height.min_max();
    let top = (hi - sea_level).max(1.0);
    let image = ImageBuffer::from_fn(height.size, height.size, |x, y| {
        let h = height.get(x, y);
        let i = height.index(x, y);
        if h <= sea_level {
            return Rgb([26, 62, 118]);
        }
        if lake_depth.is_some_and(|d| d.get(x, y) > 0.5) {
            return Rgb([70, 130, 190]);
        }
        if flow.area[i] > river_cells {
            return Rgb([50, 110, 200]);
        }
        let t = ((h - sea_level) / top).clamp(0.0, 1.0);
        // Green at the shore, ochre in the hills, grey on the peaks.
        let tint = if t < 0.4 {
            let k = t / 0.4;
            [90.0 + 100.0 * k, 140.0 + 40.0 * k, 60.0 + 30.0 * k]
        } else {
            let k = (t - 0.4) / 0.6;
            [190.0 - 30.0 * k, 180.0 - 20.0 * k, 90.0 + 90.0 * k]
        };
        let light = 0.35 + 0.65 * shade(height, x, y, sun);
        Rgb([
            (tint[0] * light) as u8,
            (tint[1] * light) as u8,
            (tint[2] * light) as u8,
        ])
    });
    image.save(path)
}
