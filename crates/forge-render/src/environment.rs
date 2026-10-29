//! An environment map as the sky (D-084): a Radiance HDR photograph of a place, its
//! equirectangular layout read in Falcor's convention (`u` from `atan2(x, −z)`, `v` from the
//! zenith down), for scenes authored under one (D-083: the Bistro under HDRI Haven's
//! San Giuseppe bridge).
//!
//! The sky's passes sample it in place of the atmosphere (`sky.slang`): the sky-view table,
//! whence the sky's irradiance and the resolve's reflections, and the backdrop at full
//! resolution; the air's haze is left out. Stored as RGB9E5 (four bytes a texel, an exponent
//! shared by the three channels) with its mips, box-filtered in linear light.

use std::path::Path;
use std::sync::Arc;

use forge_gpu::{Device, GpuError, Image, ImageDesc, Result, SampledImageId, vk};

/// An environment map on the GPU, visible to every shader through the bindless set. Released
/// when dropped.
pub struct EnvironmentMap {
    device: Arc<Device>,
    _image: Image,
    sampled: SampledImageId,
    /// Level 0's width and height, texels.
    pub size: [u32; 2],
    /// The mean of its luminance over the sphere (each texel weighed by its solid angle), its sun
    /// left out.
    pub mean_luminance: f32,
    /// The photograph's sun, taken out of the map ([`take_sun`]), when it has one.
    pub sun: Option<EnvironmentSun>,
}

/// An environment map's sun, taken out of it to be the scene's sun: drawn by the renderer's
/// own sun with its shadows, not by the map's few bright texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentSun {
    /// Towards it, world axes (the map's layout, Falcor's).
    pub direction: [f32; 3],
    /// Its irradiance on a surface facing it, the map's units, linear RGB.
    pub irradiance: [f32; 3],
}

impl EnvironmentSun {
    /// The luminance of its irradiance.
    pub fn illuminance(&self) -> f32 {
        let [r, g, b] = self.irradiance;
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

/// How many times the map's mean luminance a texel must hold to be part of its sun.
const SUN_OVER_MEAN: f32 = 40.0;

/// The world direction of the centre of texel `(x, y)` of a `w` × `h` map, in Falcor's layout:
/// `u` from `atan2(x, −z)` (the sky shader's `environment_luminance`), `v` from the zenith down.
fn texel_direction(x: u32, y: u32, w: u32, h: u32) -> [f32; 3] {
    let phi = ((f64::from(x) + 0.5) / f64::from(w) * 2.0 - 1.0) * std::f64::consts::PI;
    let theta = (f64::from(y) + 0.5) / f64::from(h) * std::f64::consts::PI;
    [
        (phi.sin() * theta.sin()) as f32,
        theta.cos() as f32,
        (-phi.cos() * theta.sin()) as f32,
    ]
}

/// The map's sun taken out of `texels`: the texels round the brightest one (within a 64th of the
/// map's width) that hold over [`SUN_OVER_MEAN`] times the mean, their light summed by solid
/// angle and their direction weighed by it, each then held to that bound so the sky keeps no
/// second sun. `None` where nothing stands so far over the mean.
pub fn take_sun(texels: &mut [[f32; 3]], w: u32, h: u32) -> Option<EnvironmentSun> {
    let luminance = |t: [f32; 3]| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    let bound = SUN_OVER_MEAN * mean_luminance(texels, w, h);
    let (peak, _) = texels
        .iter()
        .enumerate()
        .map(|(i, &t)| (i, luminance(t)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let (px, py) = ((peak as u32) % w, (peak as u32) / w);
    if luminance(texels[peak]) <= bound {
        return None;
    }
    let reach = (w / 64).max(1) as i64;
    let (mut irradiance, mut direction) = ([0.0_f64; 3], [0.0_f64; 3]);
    for dy in -reach..=reach {
        let y = i64::from(py) + dy;
        if !(0..i64::from(h)).contains(&y) {
            continue;
        }
        let theta = (y as f64 + 0.5) / f64::from(h) * std::f64::consts::PI;
        let solid_angle = (2.0 * std::f64::consts::PI / f64::from(w))
            * (std::f64::consts::PI / f64::from(h))
            * theta.sin();
        for dx in -reach..=reach {
            let x = (i64::from(px) + dx).rem_euclid(i64::from(w)) as u32;
            let i = (y as u32 * w + x) as usize;
            let l = luminance(texels[i]);
            if l <= bound {
                continue;
            }
            let d = texel_direction(x, y as u32, w, h);
            for k in 0..3 {
                irradiance[k] += f64::from(texels[i][k]) * solid_angle;
                direction[k] += f64::from(d[k]) * f64::from(l) * solid_angle;
            }
            let keep = bound / l;
            texels[i] = texels[i].map(|c| c * keep);
        }
    }
    let length = direction
        .iter()
        .map(|c| c * c)
        .sum::<f64>()
        .sqrt()
        .max(1e-12);
    Some(EnvironmentSun {
        direction: direction.map(|c| (c / length) as f32),
        irradiance: irradiance.map(|c| c as f32),
    })
}

impl EnvironmentMap {
    /// Reads the Radiance HDR file at `path` and uploads it with its mips.
    pub fn load(device: &Arc<Device>, path: &Path) -> Result<Self> {
        let image = image::open(path)
            .map_err(|e| {
                GpuError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                ))
            })?
            .to_rgb32f();
        let (w, h) = image.dimensions();
        let texels: Vec<[f32; 3]> = image.pixels().map(|p| p.0).collect();
        Self::from_texels(device, path.to_string_lossy().as_ref(), &texels, w, h)
    }

    /// `texels` (`w` × `h` linear RGB, rows from the zenith down) uploaded with their mips, its
    /// sun taken out first ([`take_sun`]).
    pub fn from_texels(
        device: &Arc<Device>,
        name: &str,
        texels: &[[f32; 3]],
        w: u32,
        h: u32,
    ) -> Result<Self> {
        let mut texels = texels.to_vec();
        let sun = take_sun(&mut texels, w, h);
        let mean_luminance = mean_luminance(&texels, w, h);
        let mut levels: Vec<Vec<u8>> = Vec::new();
        let (mut current, mut size) = (texels, [w, h]);
        loop {
            levels.push(
                current
                    .iter()
                    .flat_map(|&[r, g, b]| pack_rgb9e5(r, g, b).to_le_bytes())
                    .collect(),
            );
            if size == [1, 1] {
                break;
            }
            (current, size) = halve(&current, size);
        }
        let level_refs: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
        let image = device.create_image_with_mips(
            ImageDesc {
                width: w,
                height: h,
                format: vk::Format::E5B9G9R9_UFLOAT_PACK32,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: level_refs.len() as u32,
                name,
            },
            &level_refs,
        )?;
        let sampled =
            device.register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        Ok(Self {
            device: device.clone(),
            _image: image,
            sampled,
            size: [w, h],
            mean_luminance,
            sun,
        })
    }

    /// Its bindless index.
    pub fn sampled(&self) -> u32 {
        self.sampled.0
    }
}

impl Drop for EnvironmentMap {
    fn drop(&mut self) {
        self.device.release_sampled_image(self.sampled);
    }
}

/// `w` × `h` texels halved in each direction that is over one (2 × 2, 2 × 1 or 1 × 2 averaged).
fn halve(texels: &[[f32; 3]], [w, h]: [u32; 2]) -> (Vec<[f32; 3]>, [u32; 2]) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let (sx, sy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
    let mut out = Vec::with_capacity((nw * nh) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0.0_f32; 3];
            for dy in 0..sy {
                for dx in 0..sx {
                    let t = texels[((y * sy + dy) * w + x * sx + dx) as usize];
                    for k in 0..3 {
                        sum[k] += t[k];
                    }
                }
            }
            let n = (sx * sy) as f32;
            out.push(sum.map(|s| s / n));
        }
    }
    (out, [nw, nh])
}

/// The mean luminance of an equirectangular map over the sphere: each row weighed by the
/// solid angle its texels cover (the sine of their polar angle).
fn mean_luminance(texels: &[[f32; 3]], w: u32, h: u32) -> f32 {
    let (mut sum, mut weight) = (0.0_f64, 0.0_f64);
    for y in 0..h {
        let polar = (f64::from(y) + 0.5) / f64::from(h) * std::f64::consts::PI;
        let s = polar.sin();
        for x in 0..w {
            let [r, g, b] = texels[(y * w + x) as usize];
            sum += s * f64::from(0.2126 * r + 0.7152 * g + 0.0722 * b);
            weight += s;
        }
    }
    (sum / weight.max(1e-12)) as f32
}

/// Three non-negative floats as RGB9E5 (`E5B9G9R9_UFLOAT_PACK32`): nine bits of mantissa each
/// and a five-bit exponent they share, rounded to nearest (Vulkan's packing rule).
pub fn pack_rgb9e5(r: f32, g: f32, b: f32) -> u32 {
    const MANTISSA: i32 = 9;
    const BIAS: i32 = 15;
    const MAX_EXPONENT: i32 = 31;
    let max_value =
        ((1 << MANTISSA) - 1) as f32 / (1 << MANTISSA) as f32 * 2f32.powi(MAX_EXPONENT - BIAS);
    let [r, g, b] = [r, g, b].map(|c| {
        if c.is_finite() {
            c.clamp(0.0, max_value)
        } else {
            0.0
        }
    });
    let largest = r.max(g).max(b);
    if largest <= 0.0 {
        return 0;
    }
    let mut exponent = (largest.log2().floor() as i32).max(-BIAS - 1) + 1 + BIAS;
    let mut unit = 2f32.powi(exponent - BIAS - MANTISSA);
    if (largest / unit + 0.5).floor() as i32 == 1 << MANTISSA {
        exponent += 1;
        unit *= 2.0;
    }
    let m = |c: f32| ((c / unit + 0.5).floor() as u32).min((1 << MANTISSA) - 1);
    m(r) | (m(g) << 9) | (m(b) << 18) | ((exponent as u32) << 27)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unpacks RGB9E5 as the GPU does.
    fn unpack(p: u32) -> [f32; 3] {
        let e = (p >> 27) as i32 - 15 - 9;
        let s = 2f32.powi(e);
        [p & 511, (p >> 9) & 511, (p >> 18) & 511].map(|m| m as f32 * s)
    }

    #[test]
    fn rgb9e5_keeps_three_digits_over_the_range() {
        for c in [
            [0.0, 0.0, 0.0],
            [1.0, 0.5, 0.25],
            [874.0, 600.0, 300.0],
            [0.003, 0.002, 0.001],
            [1e-9, 0.0, 0.0],
        ] {
            let back = unpack(pack_rgb9e5(c[0], c[1], c[2]));
            let largest = c.iter().copied().fold(0.0_f32, f32::max);
            for k in 0..3 {
                // Each channel to half a unit of the shared exponent's step: under 1/512 of the
                // largest channel, or half the format's smallest step (2^-24) near black.
                assert!(
                    (back[k] - c[k]).abs() <= largest / 512.0 + 3.0e-8,
                    "{c:?} -> {back:?}"
                );
            }
        }
        // Negative and non-finite values are black.
        assert_eq!(pack_rgb9e5(-1.0, f32::NAN, f32::INFINITY), 0);
    }

    #[test]
    fn the_sun_comes_out_of_the_map_with_its_light_and_direction() {
        // A grey sky of 1 and a 2 × 2 sun of 5 000 a quarter of the way round and a third down.
        let (w, h) = (256, 128);
        let mut texels = vec![[1.0_f32; 3]; (w * h) as usize];
        for (x, y) in [(64, 42), (65, 42), (64, 43), (65, 43)] {
            texels[(y * w + x) as usize] = [5000.0, 4000.0, 2000.0];
        }
        let bound = SUN_OVER_MEAN * mean_luminance(&texels, w, h);
        let sun = take_sun(&mut texels, w, h).expect("a sun");
        // Falcor's layout at u = 0.25, v = 1/3: atan2(x, -z) = -90°, 60° from the zenith.
        let d = sun.direction;
        assert!(
            (d[0] + 0.866).abs() < 0.02 && (d[1] - 0.5).abs() < 0.03 && d[2].abs() < 0.03,
            "{d:?}"
        );
        // Its light: four texels' radiance by their solid angle.
        let texel = (2.0 * std::f64::consts::PI / 256.0) * (std::f64::consts::PI / 128.0);
        let expected = 4.0 * 5000.0 * texel * (60_f64.to_radians()).sin();
        assert!(
            (f64::from(sun.irradiance[0]) - expected).abs() < 0.03 * expected,
            "{:?} vs {expected}",
            sun.irradiance
        );
        // The map keeps no sun: every texel's luminance within the bound it was taken at.
        let luminance = |t: &[f32; 3]| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
        assert!(
            texels.iter().all(|t| luminance(t) <= bound * 1.0001),
            "a texel kept its sun"
        );
        // A sky with nothing far over its mean has none.
        let mut flat = vec![[1.0_f32; 3]; (w * h) as usize];
        assert!(take_sun(&mut flat, w, h).is_none());
    }

    #[test]
    fn mips_halve_down_to_one_texel_and_keep_the_mean() {
        let texels = vec![[1.0, 2.0, 3.0]; 8 * 4];
        let (half, size) = halve(&texels, [8, 4]);
        assert_eq!((size, half.len()), ([4, 2], 8));
        assert!(half.iter().all(|t| *t == [1.0, 2.0, 3.0]));
        let (half, size) = halve(&[[0.0; 3], [2.0; 3]], [2, 1]);
        assert_eq!((size, half), ([1, 1], vec![[1.0; 3]]));
        // A uniform sky's mean is its value.
        let mean = mean_luminance(&vec![[1.0, 1.0, 1.0]; 16 * 8], 16, 8);
        assert!((mean - 1.0).abs() < 1e-5, "{mean}");
    }
}
