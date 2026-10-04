//! The night sky over the ground (D-046, issue #164): where the sun, the Moon and the stars are
//! at a time of the day, which of the two lights the renderer's sun fields carry, and the stars
//! the sky's compose draws (`shaders/sky.slang`, `night_sky`).
//!
//! - **The day's cycle:** `t` runs 0..2: 0 sunrise, 0.5 noon, 1 sunset (`--day`'s day, #57),
//!   1.5 midnight. The sun keeps `--day`'s path and goes on under the northern horizon.
//! - **The Moon** follows the same path `2 × age` behind the sun (age 0.5, full: opposite it).
//!   Its illuminance above the air is the sun's scaled by their magnitudes, with the Moon's
//!   measured phase law (Allen's: `-12.73 + 0.026 α + 4e-9 α⁴`, α in degrees).
//! - **The key light:** the sun while it is above 3° under the horizon (the clouds still see it
//!   at sunset), then the Moon: its direction, colour and illuminance go where the sun's did,
//!   so its shadows are traced and the probes see it as they saw the sun's.
//! - **The reference illuminance:** the sky's tables are stored per unit of an illuminance that
//!   follows the scene's light, from the sun's 128 klux down to the Moon's tenths of a lux, so
//!   a night's sky keeps fp16's precision. The lights are weighed against it.
//! - **The stars** turn about the celestial pole once a cycle. They are a list (procedural, or
//!   a catalogue's) binned on a cube of cells round the sky; the compose looks up its pixel's
//!   cell and draws each star there as a Gaussian about a pixel wide, keeping its energy.

use std::sync::Arc;

use forge_gpu::{Buffer, Device, MemoryCategory, Result, vk};
use glam::{Mat3, Vec3};

use crate::sky::SkyNight;

/// The Moon's angular radius seen from the Earth, radians (its mean distance).
pub const MOON_ANGULAR_RADIUS: f32 = 0.00452;
/// The sun's apparent visual magnitude.
const SUN_MAGNITUDE: f32 = -26.74;
/// The full Moon's (Allen's phase law at α = 0).
const MOON_FULL_MAGNITUDE: f32 = -12.73;
/// Airglow and the unresolved stars at the zenith of a clear moonless sky, cd/m² (about
/// 22 mag per square arcsecond; research/night-sky.md).
pub const AIRGLOW_ZENITH: f32 = 2e-4;
/// The lowest reference illuminance, lux: a clear moonless night's (about 0.002 lux on the
/// ground).
const REFERENCE_FLOOR: f32 = 1e-3;
/// The sun stays the key light down to this elevation, degrees.
const KEY_SUN_LOWEST: f32 = -3.0;
/// Stars brighter than this are drawn (the catalogue's limit).
pub const FAINTEST_STAR: f32 = 6.5;
/// The star cells' side, per face of the cube round the sky (`STAR_CELLS` in sky.slang).
pub const STAR_CELLS: u32 = 64;
/// How far round a star its cells reach, radians: more than three of the widest Gaussian the
/// compose draws (it clamps its width to a third of this).
pub const STAR_REACH: f32 = 0.008;

/// Where the night's bodies are for a place on the planet.
#[derive(Clone, Copy, Debug)]
pub struct Celestial {
    /// The place's latitude, radians: the celestial pole's height over the northern horizon.
    pub latitude: f32,
    /// The Moon's age in lunations: 0 new, 0.25 first quarter, 0.5 full.
    pub moon_age: f32,
}

impl Default for Celestial {
    fn default() -> Self {
        Self {
            // A Mediterranean coast (the owner's towns, #91).
            latitude: 43.5_f32.to_radians(),
            // A waxing gibbous: high in the evening and most of the night, its phase visible.
            moon_age: 0.4,
        }
    }
}

/// The sky at one time of the cycle.
#[derive(Clone, Copy, Debug)]
pub struct SkyAt {
    /// Towards the sun (world: +X east, +Y up, +Z south).
    pub sun: Vec3,
    /// Towards the Moon.
    pub moon: Vec3,
    /// The Moon's illuminance above the air, lux.
    pub moon_illuminance: f32,
    /// The stars' frame: celestial (equatorial, +Z the north pole) from world.
    pub stars_from_world: Mat3,
}

/// `--day`'s path (#57) at `t` (0..2): 4° under the eastern horizon at 0, 70° in the south at
/// 0.5, the west at 1, and 78° under the northern horizon at 1.5.
pub fn path(t: f32) -> Vec3 {
    let pi = std::f32::consts::PI;
    let elevation = (-4.0_f32 + 74.0 * (pi * t).sin()).to_radians();
    let azimuth = pi * t;
    Vec3::new(
        elevation.cos() * azimuth.cos(),
        elevation.sin(),
        elevation.cos() * azimuth.sin(),
    )
}

/// The Moon's illuminance above the air at phase angle `phase` (radians, 0 full), for a sun of
/// `sun_illuminance` lux.
pub fn moon_illuminance(phase: f32, sun_illuminance: f32) -> f32 {
    let a = phase.to_degrees().abs();
    let magnitude = MOON_FULL_MAGNITUDE + 0.026 * a + 4e-9 * a.powi(4);
    sun_illuminance * 10f32.powf(-0.4 * (magnitude - SUN_MAGNITUDE))
}

impl Celestial {
    /// The sky at `t` of the cycle (0 sunrise, 0.5 noon, 1 sunset, 1.5 midnight), for a sun of
    /// `sun_illuminance` lux above the air.
    pub fn at(&self, t: f32, sun_illuminance: f32) -> SkyAt {
        let sun = path(t);
        let moon = path(t - 2.0 * self.moon_age);
        // The phase angle: the sun and the Earth seen from the Moon.
        let phase = std::f32::consts::PI - sun.dot(moon).clamp(-1.0, 1.0).acos();
        // The celestial pole over the northern horizon (−Z), the sky turning westwards about it
        // once a cycle (half a turn by day, half by night).
        let pole = Vec3::new(0.0, self.latitude.sin(), -self.latitude.cos());
        let east = Vec3::X;
        let base = Mat3::from_cols(east, pole.cross(east), pole);
        let turn = Mat3::from_axis_angle(pole, -std::f32::consts::PI * t);
        let world_from_stars = turn * base;
        SkyAt {
            sun,
            moon,
            moon_illuminance: moon_illuminance(phase, sun_illuminance),
            stars_from_world: world_from_stars.transpose(),
        }
    }
}

/// How much of a light at `elevation` (radians) the sky still sees: 1 above the horizon, then
/// a decade per 3° (twilight's ground illuminance falls about so). Only scales the reference.
fn twilight(elevation: f32) -> f32 {
    let degrees = elevation.to_degrees();
    if degrees >= 0.0 {
        1.0
    } else {
        10f32.powf(degrees / 3.0)
    }
}

/// The two lights of a frame and the illuminance the sky's tables are per unit of.
#[derive(Clone, Copy, Debug)]
pub struct NightLights {
    /// The key light's direction: what the renderer's `sun_dir` takes.
    pub key: Vec3,
    /// The key is the sun (else the Moon).
    pub key_is_sun: bool,
    /// The key's angular radius (soft shadows).
    pub key_radius: f32,
    /// What the tables are per unit of, lux: the renderer's `sun_illuminance`.
    pub reference: f32,
    /// The key's illuminance above the air over the reference.
    pub key_weight: f32,
    /// The other light (the sun's twilight under the Moon; the Moon under the sun).
    pub second: Vec3,
    /// Its illuminance over the reference, 0 when it adds nothing the tables could hold.
    pub second_weight: f32,
    /// The sun's illuminance over the reference (its disc).
    pub sun_weight: f32,
    /// The Moon's.
    pub moon_weight: f32,
}

impl NightLights {
    /// The lights of `sky` for a sun of `sun_illuminance` lux.
    pub fn new(sky: &SkyAt, sun_illuminance: f32) -> Self {
        let sun_up = sky.sun.y.asin();
        let moon_up = sky.moon.y.asin();
        let reference = (sun_illuminance * twilight(sun_up))
            .max(sky.moon_illuminance * twilight(moon_up))
            .max(REFERENCE_FLOOR);
        let key_is_sun = sun_up.to_degrees() > KEY_SUN_LOWEST;
        let (key, key_e, second, second_e) = if key_is_sun {
            (sky.sun, sun_illuminance, sky.moon, sky.moon_illuminance)
        } else {
            (sky.moon, sky.moon_illuminance, sky.sun, sun_illuminance)
        };
        // The sun's twilight ends with astronomical twilight: faded out from 12° to 20° under the
        // horizon. Deeper, the multiple-scattering table's residue at such angles (tiny, but
        // weighed by a million against the Moon) would outshine the moonlit sky.
        let fade = if key_is_sun {
            1.0
        } else {
            ((sun_up.to_degrees() + 20.0) / 8.0).clamp(0.0, 1.0)
        };
        let second_weight = second_e / reference * fade;
        Self {
            key,
            key_is_sun,
            key_radius: if key_is_sun {
                crate::starfield::SUN_ANGULAR_RADIUS_1AU
            } else {
                MOON_ANGULAR_RADIUS
            },
            reference,
            key_weight: key_e / reference,
            second,
            // Under the noon sun the Moon's light is far below what fp16 tables keep.
            second_weight: if second_weight > 1e-4 {
                second_weight
            } else {
                0.0
            },
            sun_weight: sun_illuminance / reference,
            moon_weight: sky.moon_illuminance / reference,
        }
    }
}

/// The share of the Lommel–Seeliger law `μ₀ / (μ₀ + μ)` a disc lit at phase angle `phase`
/// shows on average (over the disc's area): what sets the disc's luminance from the Moon's
/// measured illuminance.
pub fn lommel_seeliger_mean(phase: f32) -> f32 {
    // The disc's frame: towards the viewer +Z, the sun in the XZ plane.
    let sun = Vec3::new(phase.sin(), 0.0, phase.cos());
    const N: usize = 64;
    let mut sum = 0.0;
    for j in 0..N {
        for i in 0..N {
            let x = (i as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let y = (j as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let r2 = x * x + y * y;
            if r2 >= 1.0 {
                continue;
            }
            let normal = Vec3::new(x, y, (1.0 - r2).sqrt());
            let mu0 = normal.dot(sun);
            if mu0 > 0.0 {
                sum += mu0 / (mu0 + normal.z);
            }
        }
    }
    // Over the disc's area, π in these units: each sample covers (2/N)².
    sum * (2.0 / N as f32).powi(2) / std::f32::consts::PI
}

/// One star as the compose reads it (`Star` in sky.slang).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuStar {
    /// Its direction, celestial frame; its illuminance above the air (lux).
    pub direction: [f32; 4],
    /// Its colour, linear Rec.709 of unit luminance.
    pub color: [f32; 4],
}

/// A star of a catalogue or of the procedural sky.
#[derive(Clone, Copy, Debug)]
pub struct Star {
    /// Right ascension, radians.
    pub right_ascension: f32,
    /// Declination, radians.
    pub declination: f32,
    /// Apparent visual magnitude.
    pub magnitude: f32,
    /// Colour index B − V.
    pub b_v: f32,
}

impl Star {
    /// Its direction in the celestial frame (+Z the north pole, +X the vernal equinox).
    pub fn direction(&self) -> Vec3 {
        let (sd, cd) = self.declination.sin_cos();
        let (sa, ca) = self.right_ascension.sin_cos();
        Vec3::new(cd * ca, cd * sa, sd)
    }

    /// Its illuminance above the air, for a sun of `sun_illuminance` lux.
    pub fn illuminance(&self, sun_illuminance: f32) -> f32 {
        sun_illuminance * 10f32.powf(-0.4 * (self.magnitude - SUN_MAGNITUDE))
    }
}

/// A star's colour from its B − V: its temperature (Ballesteros 2012), then a black body's
/// colour (Wyman et al. 2013's fit of the CIE 1931 observer), linear Rec.709 of unit
/// luminance, half-way to white (stars look paler than their spectra).
pub fn star_color(b_v: f32) -> Vec3 {
    let b_v = b_v.clamp(-0.4, 2.0);
    let t = 4600.0 * (1.0 / (0.92 * b_v + 1.7) + 1.0 / (0.92 * b_v + 0.62));
    let lobe = |l: f32, mu: f32, s1: f32, s2: f32| {
        let s = if l < mu { s1 } else { s2 };
        (-0.5 * ((l - mu) / s).powi(2)).exp()
    };
    let mut xyz = Vec3::ZERO;
    let mut l = 380.0_f32;
    while l <= 780.0 {
        let metres = l * 1e-9;
        // Planck's law up to a constant.
        let planck = 1.0 / (metres.powi(5) * ((1.438_777e-2 / (metres * t)).exp() - 1.0));
        let x = 1.056 * lobe(l, 599.8, 37.9, 31.0) + 0.362 * lobe(l, 442.0, 16.0, 26.7)
            - 0.065 * lobe(l, 501.1, 20.4, 26.2);
        let y = 0.821 * lobe(l, 568.8, 46.9, 40.5) + 0.286 * lobe(l, 530.9, 16.3, 31.1);
        let z = 1.217 * lobe(l, 437.0, 11.8, 36.0) + 0.681 * lobe(l, 459.0, 26.0, 13.8);
        xyz += Vec3::new(x, y, z) * planck;
        l += 5.0;
    }
    let rgb = Vec3::new(
        3.2406 * xyz.x - 1.5372 * xyz.y - 0.4986 * xyz.z,
        -0.9689 * xyz.x + 1.8758 * xyz.y + 0.0415 * xyz.z,
        0.0557 * xyz.x - 0.2040 * xyz.y + 1.0570 * xyz.z,
    )
    .max(Vec3::ZERO);
    let luminance = rgb.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-30);
    Vec3::ONE.lerp(rgb / luminance, 0.5)
}

/// Galactic latitude of a celestial direction (J2000 equatorial to galactic, the IAU rotation).
pub fn galactic_latitude(direction: Vec3) -> f32 {
    let z = -0.867_666 * direction.x - 0.198_076 * direction.y + 0.455_984 * direction.z;
    z.clamp(-1.0, 1.0).asin()
}

/// A procedural sky of stars to magnitude 6.5, as many as the real one (about 9 000): the
/// counts brighter than each magnitude as the sky's (log₁₀ N ≈ 0.5 m + 0.7), denser towards
/// the galactic plane, colours spread round a sun-like B − V. Deterministic for `seed`.
pub fn procedural_stars(seed: u64) -> Vec<Star> {
    let mut state = seed;
    let mut next = || {
        // splitmix64
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        ((z >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    };
    let total = 10f32.powf(0.5 * FAINTEST_STAR + 0.7);
    let count = total as usize;
    let mut stars = Vec::with_capacity(count);
    while stars.len() < count {
        let z = 1.0 - 2.0 * next();
        let phi = 2.0 * std::f32::consts::PI * next();
        let declination = z.asin();
        let star = Star {
            right_ascension: phi,
            declination,
            magnitude: 0.0,
            b_v: 0.0,
        };
        // Twice as dense on the galactic plane as away from it.
        let b = galactic_latitude(star.direction());
        if next() * 3.0 > 1.0 + 2.0 * (-(b / 0.26).powi(2)).exp() {
            continue;
        }
        let rank = (stars.len() + 1) as f32;
        let magnitude = ((rank.log10() - 0.7) / 0.5).min(FAINTEST_STAR);
        // B − V: a sum of uniforms round 0.65, from hot blue stars to cool red ones.
        let b_v = 0.65 + (next() + next() + next() - 1.5) * 0.9;
        stars.push(Star {
            magnitude,
            b_v,
            ..star
        });
    }
    stars
}

/// The cube cell of a direction (`star_cell` in sky.slang: the same arithmetic).
pub fn star_cell(d: Vec3) -> u32 {
    let a = d.abs();
    let (face, u, v) = if a.x >= a.y && a.x >= a.z {
        (if d.x > 0.0 { 0 } else { 1 }, d.y / a.x, d.z / a.x)
    } else if a.y >= a.z {
        (if d.y > 0.0 { 2 } else { 3 }, d.x / a.y, d.z / a.y)
    } else {
        (if d.z > 0.0 { 4 } else { 5 }, d.x / a.z, d.y / a.z)
    };
    let n = STAR_CELLS;
    let cell = |c: f32| (((c + 1.0) * 0.5 * n as f32) as u32).min(n - 1);
    face * n * n + cell(v) * n + cell(u)
}

/// The stars for the GPU, and their cells: `STAR_CELLS² × 6 + 1` offsets, then the stars'
/// indices cell by cell. A star sits in every cell within `STAR_REACH` of it.
pub fn bin_stars(stars: &[Star], sun_illuminance: f32) -> (Vec<GpuStar>, Vec<u32>) {
    let cells = (6 * STAR_CELLS * STAR_CELLS) as usize;
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); cells];
    let mut gpu = Vec::with_capacity(stars.len());
    for (i, star) in stars.iter().enumerate() {
        let d = star.direction();
        let c = star_color(star.b_v);
        gpu.push(GpuStar {
            direction: [d.x, d.y, d.z, star.illuminance(sun_illuminance)],
            color: [c.x, c.y, c.z, 0.0],
        });
        let (t1, t2) = d.any_orthonormal_pair();
        for du in [-1.0, 0.0, 1.0] {
            for dv in [-1.0, 0.0, 1.0] {
                let cell = star_cell((d + (t1 * du + t2 * dv) * STAR_REACH).normalize()) as usize;
                if lists[cell].last() != Some(&(i as u32)) && !lists[cell].contains(&(i as u32)) {
                    lists[cell].push(i as u32);
                }
            }
        }
    }
    let mut table = Vec::with_capacity(cells + 1 + stars.len() * 2);
    let mut offset = 0u32;
    for list in &lists {
        table.push(offset);
        offset += list.len() as u32;
    }
    table.push(offset);
    for list in &lists {
        table.extend_from_slice(list);
    }
    (gpu, table)
}

/// How the night is drawn: the owner's choices of D-046 behind flags.
#[derive(Clone, Copy, Debug)]
pub struct NightSettings {
    /// The Moon's light as a soft unshadowed fill in the sky's light (`--moon-fill`) instead
    /// of a key light with traced shadows.
    pub moon_fill: bool,
    /// The stars and the Milky Way brightened over their physical luminance: as an eye sees
    /// them rather than the night's capped exposure.
    pub star_gain: f32,
    /// Draw the Milky Way.
    pub milky_way: bool,
}

impl Default for NightSettings {
    fn default() -> Self {
        Self {
            moon_fill: false,
            // As a dark-adapted eye sees them rather than a camera.
            star_gain: 16.0,
            milky_way: true,
        }
    }
}

/// The night's GPU data: the stars, binned (`bin_stars`).
pub struct NightSky {
    procedural: (Buffer, Buffer),
}

impl NightSky {
    /// Bins and uploads the procedural sky's stars for a sun of `sun_illuminance` lux.
    pub fn new(device: &Arc<Device>, sun_illuminance: f32) -> Result<Self> {
        let (stars, cells) = bin_stars(&procedural_stars(0x5747_4152), sun_illuminance);
        let upload = |stars: &[GpuStar], cells: &[u32], name: &str| -> Result<(Buffer, Buffer)> {
            let usage = vk::BufferUsageFlags::STORAGE_BUFFER;
            Ok((
                device.create_buffer_with_data(stars, usage, MemoryCategory::Frame, name)?,
                device.create_buffer_with_data(cells, usage, MemoryCategory::Frame, name)?,
            ))
        };
        Ok(Self {
            procedural: upload(&stars, &cells, "night stars")?,
        })
    }

    /// The sky's night for `sky` and its `lights`, with a pixel of `pixel_angle` radians at
    /// the view's centre.
    pub fn sky(
        &self,
        sky: &SkyAt,
        lights: &NightLights,
        pixel_angle: f32,
        settings: NightSettings,
    ) -> SkyNight {
        let reference = lights.reference;
        // The Moon's disc: its luminance per unit Lommel–Seeliger share such that the disc
        // gives the Moon's measured illuminance (night.rs's `moon_illuminance`).
        let phase = std::f32::consts::PI - sky.sun.dot(sky.moon).clamp(-1.0, 1.0).acos();
        let solid_angle = std::f32::consts::PI * MOON_ANGULAR_RADIUS * MOON_ANGULAR_RADIUS;
        let share = lommel_seeliger_mean(phase);
        let moon_disc = if share > 1e-5 {
            sky.moon_illuminance / (solid_angle * share) / reference
        } else {
            0.0
        };
        // Stars and airglow only once the sun is down: by day they add nothing visible.
        let dark = sky.sun.y < 0.0;
        SkyNight {
            key_weight: lights.key_weight,
            sun: sky.sun,
            sun_weight: lights.sun_weight,
            second: lights.second,
            second_weight: lights.second_weight,
            moon: sky.moon,
            moon_disc,
            moon_fill: if settings.moon_fill && !lights.key_is_sun {
                lights.moon_weight
            } else {
                0.0
            },
            airglow: if dark {
                AIRGLOW_ZENITH / reference
            } else {
                0.0
            },
            star_gain: if dark {
                settings.star_gain / reference
            } else {
                0.0
            },
            stars_from_world: sky.stars_from_world,
            pixel_angle,
            stars: (self.procedural.0.address(), self.procedural.1.address()),
            milky_way: settings.milky_way,
            moon_albedo: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUN: f32 = 128_000.0;

    #[test]
    fn the_full_moon_lights_the_ground_as_measured() {
        // Research: 0.05–0.3 lux on the ground from a high full Moon, at most about 0.36.
        let full = moon_illuminance(0.0, SUN);
        assert!((0.25..0.36).contains(&full), "{full}");
        // A quarter Moon is about a tenth of the full one.
        let quarter = moon_illuminance(std::f32::consts::FRAC_PI_2, SUN) / full;
        assert!((0.06..0.14).contains(&quarter), "{quarter}");
    }

    #[test]
    fn the_sun_crosses_the_south_by_day_and_the_north_by_night() {
        assert!(path(0.5).y > 0.9 && path(0.5).z > 0.0);
        let midnight = path(1.5);
        assert!(midnight.y < -0.9 && midnight.z < 0.0, "{midnight}");
        // Continuous through sunset and sunrise.
        assert!(path(1.0).abs_diff_eq(path(1.0 + 1e-4), 1e-3));
        assert!(path(0.0).abs_diff_eq(path(2.0), 1e-4));
    }

    #[test]
    fn a_full_moon_rises_as_the_sun_sets() {
        let sky = Celestial {
            moon_age: 0.5,
            ..Celestial::default()
        }
        .at(1.0, SUN);
        assert!(sky.moon.dot(sky.sun) < -0.99, "{}", sky.moon);
        let midnight = Celestial {
            moon_age: 0.5,
            ..Celestial::default()
        }
        .at(1.5, SUN);
        assert!(midnight.moon.y > 0.9);
    }

    #[test]
    fn the_stars_turn_about_the_pole() {
        let c = Celestial::default();
        let pole = Vec3::new(0.0, c.latitude.sin(), -c.latitude.cos());
        for t in [0.0, 0.3, 1.2] {
            let world_from_stars = c.at(t, SUN).stars_from_world.transpose();
            assert!((world_from_stars * Vec3::Z).abs_diff_eq(pole, 1e-5));
            // A proper rotation: the constellations are not mirrored.
            assert!((world_from_stars.determinant() - 1.0).abs() < 1e-5);
        }
        // Westwards: a star on the celestial equator in the south moves towards −X.
        let at = |t| c.at(t, SUN).stars_from_world.transpose();
        let south = Vec3::new(0.0, c.latitude.cos(), c.latitude.sin());
        let star = at(0.5).transpose() * south;
        let later = at(0.51) * star;
        assert!(later.x < 0.0, "{later}");
    }

    #[test]
    fn the_key_is_the_sun_by_day_and_the_moon_by_night() {
        let c = Celestial::default();
        let noon = NightLights::new(&c.at(0.5, SUN), SUN);
        assert!(noon.key_is_sun);
        assert_eq!(noon.reference, SUN);
        assert_eq!(noon.key_weight, 1.0);
        assert_eq!(noon.second_weight, 0.0);
        let midnight = NightLights::new(&c.at(1.5, SUN), SUN);
        assert!(!midnight.key_is_sun);
        assert!(midnight.reference < 1.0, "{}", midnight.reference);
        assert!((midnight.key_weight - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_lommel_seeliger_disc_is_half_bright_when_full() {
        assert!((lommel_seeliger_mean(0.0) - 0.5).abs() < 0.01);
        assert!(lommel_seeliger_mean(std::f32::consts::FRAC_PI_2) < 0.3);
        assert!(lommel_seeliger_mean(3.1) < 0.01);
    }

    #[test]
    fn the_procedural_sky_counts_stars_like_the_real_one() {
        let stars = procedural_stars(7);
        assert!((8_000..10_000).contains(&stars.len()), "{}", stars.len());
        let bright = stars.iter().filter(|s| s.magnitude < 1.0).count();
        assert!((10..25).contains(&bright), "{bright}");
        let first = stars.iter().map(|s| s.magnitude).fold(f32::MAX, f32::min);
        assert!((-1.6..-1.2).contains(&first), "{first}");
        assert_eq!(procedural_stars(7)[100].magnitude, stars[100].magnitude);
    }

    #[test]
    fn hot_stars_are_blue_and_cool_ones_red() {
        let hot = star_color(-0.3);
        let cool = star_color(1.6);
        assert!(hot.z > hot.x && cool.x > cool.z, "{hot} {cool}");
        let sun = star_color(0.65);
        assert!((sun.dot(Vec3::new(0.2126, 0.7152, 0.0722)) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn every_star_is_in_its_own_cell() {
        let stars = procedural_stars(3);
        let (gpu, table) = bin_stars(&stars, SUN);
        let cells = (6 * STAR_CELLS * STAR_CELLS) as usize;
        assert_eq!(table[cells] as usize + cells + 1, table.len());
        for (i, star) in gpu.iter().enumerate() {
            let d = Vec3::from_slice(&star.direction[..3]);
            let cell = star_cell(d) as usize;
            let list =
                &table[cells + 1 + table[cell] as usize..cells + 1 + table[cell + 1] as usize];
            assert!(list.contains(&(i as u32)));
        }
    }
}
