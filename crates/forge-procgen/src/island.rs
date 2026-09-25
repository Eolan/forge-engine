//! Stages 1 and 2 of the terrain pipeline (`docs/research/terrain-genesis.md`): the island's
//! mask and the fields the erosion runs on, an uplift rate, a hardness and a rainfall; and the
//! whole island as one call, [`generate_island`], cached on disk by [`cached_island`].

use std::io;
use std::path::Path;

use forge_core::Seed;

use crate::erosion::{ErosionParams, erode};
use crate::field::Field2;
use crate::flow::Flow;
use crate::noise::{fbm, ridged};

/// What shapes the island.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IslandParams {
    /// Every random choice derives from it.
    pub seed: Seed,
    /// Samples per side of every field.
    pub size: u32,
    /// Metres between samples.
    pub spacing: f64,
    /// The island's mean radius as a share of the half side (0.62: 5 km on a 16 km domain,
    /// the coast wandering 2 km in and out; the domain's edge stays sea, it is the outlet).
    pub radius: f64,
    /// How far the coast wanders in and out, as a share of the half side.
    pub coast_noise: f64,
    /// Kilometres per period of the coast's largest wander.
    pub coast_scale_km: f64,
    /// Uplift at the island's heart, metres per erosion step.
    pub uplift: f64,
    /// Kilometres per period of the ridges' largest wander.
    pub ridge_scale_km: f64,
}

impl IslandParams {
    /// A 16 km island at `spacing` metres: `size` samples a side to cover it.
    pub fn island_16km(seed: Seed, spacing: f64) -> Self {
        Self {
            seed,
            size: (16_384.0 / spacing) as u32 + 1,
            spacing,
            radius: 0.62,
            coast_noise: 0.25,
            coast_scale_km: 5.0,
            uplift: 4.0,
            ridge_scale_km: 3.0,
        }
    }

    /// The side of the domain, metres.
    pub fn extent(&self) -> f64 {
        f64::from(self.size - 1) * self.spacing
    }
}

/// The fields of stages 1 and 2.
#[derive(Clone, Debug)]
pub struct IslandFields {
    /// The island's shape: positive inland, zero on the coast, negative at sea, in shares of
    /// the half side (about `−1..0.4`).
    pub shape: Field2<f32>,
    /// Metres of uplift per erosion step: zero at and beyond the coast, most at the heart.
    pub uplift: Field2<f32>,
    /// How easily the rock erodes, a factor on the erodibility (0.5 hard basalt to 1.5 loose
    /// regolith), in bands and patches.
    pub hardness: Field2<f32>,
    /// Rainfall as a factor on the drainage (1 everywhere for now; orographic later).
    pub rain: Field2<f32>,
}

/// A seed's 64 bits for a noise lattice.
fn lattice(seed: Seed, purpose: u64) -> u64 {
    seed.derive(purpose).rng().next_u64()
}

/// The island's heightfield: stages 1–3 from a flat sea, and the last step's flow.
pub fn generate_island(p: &IslandParams, erosion: &ErosionParams) -> (Field2<f32>, Flow) {
    let fields = island_fields(p);
    let mut height = fields.shape.map(|_| 0.0_f32);
    let flow = erode(
        &mut height,
        &fields.uplift,
        &fields.hardness,
        &fields.rain,
        erosion,
    );
    (height, flow)
}

/// The text that names an island's heightfield: its parameters, and so its cache key.
pub fn island_key(p: &IslandParams, erosion: &ErosionParams) -> String {
    format!("{p:?} {erosion:?}")
}

/// FNV-1a over `bytes` (stable across platforms and runs).
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The island's heightfield from `dir` when it was generated before with the same
/// parameters, else generated and stored there (`island-<key>.f32`: the samples per side, the
/// spacing, then the heights as little-endian `f32`), so the minute of erosion is paid once.
/// Returns the field and whether it came from the file.
pub fn cached_island(
    dir: &Path,
    p: &IslandParams,
    erosion: &ErosionParams,
) -> io::Result<(Field2<f32>, bool)> {
    let key = fnv1a64(island_key(p, erosion).as_bytes());
    let file = dir.join(format!("island-{key:016x}.f32"));
    if let Ok(bytes) = std::fs::read(&file)
        && bytes.len() >= 12
    {
        let size = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes"));
        let spacing = f64::from_le_bytes(bytes[4..12].try_into().expect("8 bytes"));
        let count = (size as usize) * (size as usize);
        if size == p.size && spacing == p.spacing && bytes.len() == 12 + 4 * count {
            let data = bytes[12..]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            return Ok((
                Field2 {
                    size,
                    spacing,
                    data,
                },
                true,
            ));
        }
    }
    let (height, _) = generate_island(p, erosion);
    std::fs::create_dir_all(dir)?;
    let mut bytes = Vec::with_capacity(12 + 4 * height.len());
    bytes.extend_from_slice(&height.size.to_le_bytes());
    bytes.extend_from_slice(&height.spacing.to_le_bytes());
    for h in &height.data {
        bytes.extend_from_slice(&h.to_le_bytes());
    }
    let partial = file.with_extension("f32.part");
    std::fs::write(&partial, &bytes)?;
    std::fs::rename(&partial, &file)?;
    Ok((height, false))
}

/// Stages 1 and 2: the mask, then the uplift, hardness and rain fields.
pub fn island_fields(p: &IslandParams) -> IslandFields {
    let half = 0.5 * p.extent();
    let coast_seed = lattice(p.seed, 1);
    let ridge_seed = lattice(p.seed, 2);
    let hardness_seed = lattice(p.seed, 3);
    let coast_period = p.coast_scale_km * 1000.0;
    let ridge_period = p.ridge_scale_km * 1000.0;
    // Stage 1: a distance-to-centre shape warped by low-frequency noise (Patel 2015).
    let shape = Field2::from_fn(p.size, p.spacing, |x, y| {
        let (mx, my) = (
            f64::from(x) * p.spacing - half,
            f64::from(y) * p.spacing - half,
        );
        let d = (mx * mx + my * my).sqrt() / half;
        let wander = fbm(
            coast_seed,
            mx / coast_period,
            my / coast_period,
            4,
            2.0,
            0.5,
        );
        (p.radius + p.coast_noise * wander - d) as f32
    });
    // Stage 2: uplift from the shape (zero at the coast, rising inland) shaped by ridges;
    // hardness in patches; rain flat.
    let uplift = Field2::from_fn(p.size, p.spacing, |x, y| {
        let s = f64::from(shape.get(x, y));
        if s <= 0.0 {
            return 0.0;
        }
        let (mx, my) = (f64::from(x) * p.spacing, f64::from(y) * p.spacing);
        let ridges = ridged(
            ridge_seed,
            mx / ridge_period,
            my / ridge_period,
            4,
            2.1,
            0.55,
        );
        // Inland the shape reaches about `radius`; a square root lifts the coast's foothills.
        let inland = (s / p.radius).min(1.0).sqrt();
        (p.uplift * inland * (0.35 + 0.65 * ridges)) as f32
    });
    let hardness = Field2::from_fn(p.size, p.spacing, |x, y| {
        let (mx, my) = (f64::from(x) * p.spacing, f64::from(y) * p.spacing);
        let n = fbm(hardness_seed, mx / 1800.0, my / 1800.0, 3, 2.0, 0.5);
        (1.0 + 0.5 * n) as f32
    });
    let rain = Field2::from_fn(p.size, p.spacing, |_, _| 1.0);
    IslandFields {
        shape,
        uplift,
        hardness,
        rain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_island_sits_in_the_sea_with_uplift_inland_only() {
        let p = IslandParams {
            size: 129,
            ..IslandParams::island_16km(Seed::new(7), 128.0)
        };
        let f = island_fields(&p);
        assert_eq!(f.shape.size, 129);
        // The centre is land and the corners are sea.
        assert!(f.shape.get(64, 64) > 0.0);
        assert!(f.shape.get(0, 0) < 0.0 && f.shape.get(128, 128) < 0.0);
        // Uplift only where there is land, most of it well inland.
        let (land, lifted) =
            f.shape
                .data
                .iter()
                .zip(&f.uplift.data)
                .fold((0, 0), |(l, u), (&s, &up)| {
                    assert!(up >= 0.0 && (s > 0.0 || up == 0.0));
                    (l + usize::from(s > 0.0), u + usize::from(up > 0.0))
                });
        assert!(
            land > 129 * 129 / 8 && land < 129 * 129 / 2,
            "{land} land samples"
        );
        assert_eq!(land, lifted);
        assert!(f.uplift.get(64, 64) > 0.3 * p.uplift as f32);
        let (lo, hi) = f.hardness.min_max();
        assert!(lo > 0.4 && hi < 1.6);
        // The same seed gives the same fields, another seed does not.
        assert_eq!(island_fields(&p).shape, f.shape);
        let other = IslandParams {
            seed: Seed::new(8),
            ..p
        };
        assert_ne!(island_fields(&other).shape, f.shape);
    }

    #[test]
    fn the_cached_island_is_the_generated_one() {
        let p = IslandParams {
            size: 33,
            ..IslandParams::island_16km(Seed::new(11), 512.0)
        };
        let erosion = ErosionParams {
            steps: 20,
            ..ErosionParams::island()
        };
        let dir = std::env::temp_dir().join(format!("forge-island-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (first, from_cache) = cached_island(&dir, &p, &erosion).unwrap();
        assert!(!from_cache);
        let (second, from_cache) = cached_island(&dir, &p, &erosion).unwrap();
        assert!(from_cache);
        assert_eq!(first, second);
        assert_eq!(first, generate_island(&p, &erosion).0);
        // Other parameters, another file.
        let other = ErosionParams {
            steps: 21,
            ..erosion
        };
        assert!(!cached_island(&dir, &p, &other).unwrap().1);
        assert_ne!(island_key(&p, &erosion), island_key(&p, &other));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
