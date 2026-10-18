//! Lamps (D-065 step 1, #270): small sphere lights the resolve lights the scene with, one shadow
//! ray each towards a point of the sphere that changes every frame, so TAA averages their
//! penumbrae as it did the sun's single ray (#54).
//!
//! A scene gives its lamps each frame ([`crate::MeshletScene::set_lamps`]): where each is, its
//! radius, its luminous intensity in candelas and its colour temperature in kelvin. The
//! [`MAX_LAMPS`] nearest the camera whose light reaches it are kept ([`nearest`]); each pixel of
//! the resolve takes every one whose [`reach`] it is in (`lamps_at` in `meshlet.slang`). D-065's
//! clustered lists (Olsson, Billeter and Assarsson 2012) come when a view holds more lamps than
//! a village's few rooms; the probes lit by the lamps after.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// The most lamps a frame lights with.
pub const MAX_LAMPS: usize = 64;
/// The illuminance a lamp's light is let go below, lux: its reach. A candle's at about 1 m.
pub const LEAST_LUX: f32 = 0.05;
/// The most regions a lamp lights ([`Lamp::regions`]).
pub const LAMP_REGIONS: usize = 8;
/// An empty slot of [`Lamp::regions`] (`REGION_NONE` in `probes.slang`).
pub const NO_REGION: u32 = u32::MAX;
/// The regions of a lamp that lights every surface its rays reach: a scene without regions.
pub const EVERY_REGION: [u32; LAMP_REGIONS] = [NO_REGION; LAMP_REGIONS];
/// The key the lamps of [`EVERY_REGION`] are listed under in [`lists`] (`REGION_ANY` in
/// `meshlet.slang`).
pub const REGION_ANY: u32 = u32::MAX - 1;
/// The most entries of [`lists`]: every lamp in every region it lights.
pub const MAX_LIST: usize = MAX_LAMPS * LAMP_REGIONS;

/// A lamp, in the frame [`crate::MeshletScene::set_lamps`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    /// Its bulb's middle, metres.
    pub position: Vec3,
    /// Its bulb's radius, metres: the size of its shadows' penumbrae.
    pub radius: f32,
    /// Its luminous intensity, candelas, the same every way (a bare bulb: lumens / 4π).
    pub candela: f32,
    /// Its colour temperature, kelvin (a warm bulb 2700, daylight 6500).
    pub kelvin: f32,
    /// The probes' regions (D-068, [`crate::RegionVolume`]) whose surfaces it lights: its room's
    /// and those it shines into through a doorway, [`NO_REGION`] in the slots left over. A
    /// surface elsewhere takes nothing of it, without a shadow ray: the house's rooms are its
    /// light lists. All slots empty ([`EVERY_REGION`]): every surface.
    pub regions: [u32; LAMP_REGIONS],
}

/// Mirrors `Lamp` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuLamp {
    /// xyz: the bulb's middle in the scene's frame; w: its radius.
    pub position_radius: [f32; 4],
    /// rgb: its intensity per channel, candelas (its colour of luminance 1 times its candelas);
    /// w: its reach, metres.
    pub colour_reach: [f32; 4],
    /// [`Lamp::regions`].
    pub regions: [u32; LAMP_REGIONS],
}

const _: () = assert!(std::mem::size_of::<GpuLamp>() == 64);

/// How far a lamp's light counts, metres: where it falls to [`LEAST_LUX`].
pub fn reach(candela: f32) -> f32 {
    (candela.max(0.0) / LEAST_LUX).sqrt()
}

/// A blackbody's colour at `kelvin`, linear Rec. 709 of luminance 1: Tanner Helland's fit of
/// the CIE 1964 10° colour-matching functions' blackbody locus (sRGB-encoded, so decoded here).
pub fn kelvin_rgb(kelvin: f32) -> Vec3 {
    let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    let decode = |c: f32| {
        let c = (c / 255.0).clamp(0.0, 1.0);
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let rgb = Vec3::new(decode(r), decode(g), decode(b));
    rgb / rgb.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
}

/// The colour temperature the lamps' colours are balanced for, kelvin: as a camera set for
/// tungsten light sees them, and the eye adapted to a lit room. At the sun's white point a 2 700 K
/// bulb turned a white room deep orange (2026-10-18); balanced, it stays warm. The sun and the
/// sky keep their colours.
pub const WHITE_BALANCE: f32 = 3200.0;

/// A lamp's colour at `kelvin` as [`WHITE_BALANCE`] sees it ([`kelvin_rgb`] over the balance's),
/// linear Rec. 709 of luminance 1.
pub fn lamp_rgb(kelvin: f32) -> Vec3 {
    let c = kelvin_rgb(kelvin) / kelvin_rgb(WHITE_BALANCE);
    c / c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
}

/// The lamps a frame lights with: those whose light reaches within [`reach`] of the camera at
/// `camera` (scene frame), the nearest first, [`MAX_LAMPS`] at most.
pub fn nearest(lamps: &[Lamp], camera: Vec3) -> Vec<GpuLamp> {
    let mut near: Vec<(f32, &Lamp)> = lamps
        .iter()
        .filter(|l| l.candela > 0.0)
        .map(|l| (l.position.distance(camera), l))
        .filter(|(d, l)| *d < reach(l.candela) + 50.0)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter()
        .take(MAX_LAMPS)
        .map(|(_, l)| {
            let c = lamp_rgb(l.kelvin) * l.candela;
            GpuLamp {
                position_radius: [
                    l.position.x,
                    l.position.y,
                    l.position.z,
                    l.radius.max(0.005),
                ],
                colour_reach: [c.x, c.y, c.z, reach(l.candela)],
                regions: l.regions,
            }
        })
        .collect()
}

/// The frame's light lists (D-065): for each region its `kept` lamps light, the lamps' indices, as
/// pairs (region, lamp) sorted by region, then lamp; the lamps of [`EVERY_REGION`] under
/// [`REGION_ANY`]. A pixel finds its region's run by a binary search (`lamp_run` in
/// `meshlet.slang`) and loops over its few lamps, not over the frame's 64 (0.5 ms in a hall).
pub fn lists(kept: &[GpuLamp]) -> Vec<[u32; 2]> {
    let mut out = Vec::new();
    for (i, l) in kept.iter().enumerate() {
        let i = i as u32;
        if l.regions.iter().all(|&r| r == NO_REGION) {
            out.push([REGION_ANY, i]);
        } else {
            out.extend(
                l.regions
                    .iter()
                    .filter(|&&r| r != NO_REGION)
                    .map(|&r| [r, i]),
            );
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_lists_hold_each_lamp_under_each_of_its_regions() {
        let lamp = |regions: [u32; LAMP_REGIONS]| GpuLamp {
            regions,
            ..GpuLamp::default()
        };
        let mut two = EVERY_REGION;
        two[0] = 7;
        two[1] = 3;
        two[2] = 7;
        let mut one = EVERY_REGION;
        one[0] = 3;
        let pairs = lists(&[lamp(two), lamp(EVERY_REGION), lamp(one)]);
        assert_eq!(pairs, vec![[3, 0], [3, 2], [7, 0], [REGION_ANY, 1]]);
    }

    #[test]
    fn a_warm_bulb_is_orange_and_daylight_white_both_of_luminance_one() {
        let warm = kelvin_rgb(2700.0);
        let day = kelvin_rgb(6500.0);
        let luminance = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        assert!((luminance(warm) - 1.0).abs() < 1e-4);
        assert!((luminance(day) - 1.0).abs() < 1e-4);
        assert!(warm.x > warm.y && warm.y > warm.z, "{warm}");
        // Daylight within a few percent of white.
        assert!((day.x - day.z).abs() < 0.1, "{day}");
        // Balanced for tungsten: its own temperature white, a warm bulb warm but its blue kept
        // (half its red, where the sun's white point leaves a tenth).
        let balanced = lamp_rgb(WHITE_BALANCE);
        assert!(
            (balanced - Vec3::ONE).abs().max_element() < 1e-4,
            "{balanced}"
        );
        let warm = lamp_rgb(2700.0);
        assert!((luminance(warm) - 1.0).abs() < 1e-4);
        assert!(warm.z / warm.x > 0.4 && kelvin_rgb(2700.0).z / kelvin_rgb(2700.0).x < 0.12);
    }

    #[test]
    fn the_frame_keeps_the_nearest_lamps_that_reach_the_camera() {
        let bulb = |x: f32, candela: f32| Lamp {
            position: Vec3::new(x, 2.0, 0.0),
            radius: 0.05,
            candela,
            kelvin: 2700.0,
            regions: EVERY_REGION,
        };
        // 100 cd reaches about 45 m.
        assert!((reach(100.0) - 44.72).abs() < 0.01);
        // A row a metre apart: those within the reach and 50 m of the camera (95 of them) count,
        // the 64 nearest are kept.
        let lamps: Vec<Lamp> = (0..100)
            .map(|i| bulb(i as f32, 100.0))
            .chain([bulb(5000.0, 100.0), bulb(1.0, 0.0)])
            .collect();
        let kept = nearest(&lamps, Vec3::ZERO);
        assert_eq!(kept.len(), MAX_LAMPS);
        assert_eq!(kept[0].position_radius[0], 0.0);
        assert!(
            kept.windows(2)
                .all(|w| w[0].position_radius[0] <= w[1].position_radius[0])
        );
    }
}
