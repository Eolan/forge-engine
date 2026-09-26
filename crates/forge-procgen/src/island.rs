//! Stages 1 and 2 of the terrain pipeline (`docs/research/terrain-genesis.md`): the island's
//! mask and the fields the erosion runs on, an uplift rate, a hardness and a rainfall; and the
//! whole island as one call, [`generate_island`], cached on disk by [`cached_island`].

use std::io;
use std::path::Path;

use forge_core::Seed;
use forge_core::dmath::exp;
use forge_task::TaskPool;

use crate::erosion::{Erosion, ErosionParams, step};
use crate::field::Field2;
use crate::flow::{Flow, drain};
use crate::noise::{fbm, ridged};

/// The eight compass steps the wind can take, `(dx, dy)` in the field's grid (`x` along the
/// columns, `y` along the rows): 0 is +x, then clockwise in image terms.
pub const WIND_STEPS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Steps between two refreshes of the orographic rain from the current relief.
pub const RAIN_EVERY: u32 = 10;

/// The prevailing wind, for the orographic rain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    /// Where it blows to: an index into [`WIND_STEPS`].
    pub towards: u8,
    /// How far the rain departs from flat: 0 leaves it flat, 1 is the model as it is.
    pub contrast: f64,
}

impl Wind {
    /// The wind that blows from a compass point (`n`, `ne`, `e`, `se`, `s`, `sw`, `w`, `nw`,
    /// north being the top of the previews, `−y`), or `None` for anything else.
    pub fn from_compass(from: &str, contrast: f64) -> Option<Self> {
        let towards = match from.trim().to_ascii_lowercase().as_str() {
            "w" => 0,
            "nw" => 1,
            "n" => 2,
            "ne" => 3,
            "e" => 4,
            "se" => 5,
            "s" => 6,
            "sw" => 7,
            _ => return None,
        };
        Some(Self { towards, contrast })
    }
}

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
    /// The prevailing wind; `None` rains the same everywhere.
    pub wind: Option<Wind>,
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
            wind: None,
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

/// The orographic rain over `height` for `wind`, a factor around 1 on the drainage: moisture
/// rides the wind from the upwind edge, fills up over the sea (5 km to saturate), rains out as
/// the ground rises under it (400 m of climb empties it) and a little on every flat kilometre
/// (halving over 40 km), so the windward slopes are wet and the lee is dry. The raw rain is
/// scaled to a mean of 1 over the land, then pulled towards or away from 1 by the wind's
/// `contrast`, and kept between 0.05 and 10. A sequential walk of the wind's lines, the same
/// on every machine (D-016).
pub fn orographic_rain(height: &Field2<f32>, sea_level: f32, wind: Wind) -> Field2<f32> {
    let n = height.size as i32;
    let (dx, dy) = WIND_STEPS[usize::from(wind.towards) % 8];
    let step_len = height.spacing
        * if dx != 0 && dy != 0 {
            std::f64::consts::SQRT_2
        } else {
            1.0
        };
    let mut raw = Field2::new(height.size, height.spacing);
    for y in 0..n {
        for x in 0..n {
            // Start where no upwind neighbour lies inside the field.
            let (ux, uy) = (x - dx, y - dy);
            if ux >= 0 && uy >= 0 && ux < n && uy < n {
                continue;
            }
            let mut moisture = 1.0_f64;
            let (mut cx, mut cy) = (x, y);
            let mut previous = f64::from(height.get(x as u32, y as u32));
            while cx >= 0 && cy >= 0 && cx < n && cy < n {
                let h = f64::from(height.get(cx as u32, cy as u32));
                if h <= f64::from(sea_level) {
                    moisture = (moisture + step_len / 5000.0).min(1.0);
                } else {
                    let rise = (h - previous).max(0.0);
                    let rain = (moisture * (1.0 - exp(-rise / 400.0))
                        + moisture * step_len / 40_000.0)
                        .min(moisture);
                    moisture -= rain;
                    raw.set(cx as u32, cy as u32, (rain / step_len) as f32);
                }
                previous = h;
                cx += dx;
                cy += dy;
            }
        }
    }
    let (sum, land) = raw
        .data
        .iter()
        .zip(&height.data)
        .filter(|(_, h)| **h > sea_level)
        .fold((0.0_f64, 0_usize), |(s, c), (r, _)| {
            (s + f64::from(*r), c + 1)
        });
    let mean = if land > 0 { sum / land as f64 } else { 1.0 };
    raw.map(|r| {
        if mean > 0.0 {
            (1.0 + wind.contrast * (f64::from(r) / mean - 1.0)).clamp(0.05, 10.0) as f32
        } else {
            1.0
        }
    })
}

/// Refreshes `rain` from the relief at `step` when the island has a wind and the step is a
/// refresh step (every [`RAIN_EVERY`], the first included); returns whether it did.
pub fn refresh_rain(
    p: &IslandParams,
    height: &Field2<f32>,
    sea_level: f32,
    step: u32,
    rain: &mut Field2<f32>,
) -> bool {
    match p.wind {
        Some(wind) if step.is_multiple_of(RAIN_EVERY) => {
            *rain = orographic_rain(height, sea_level, wind);
            true
        }
        _ => false,
    }
}

/// The island's heightfield: stages 1–3 from a flat sea, and the last step's flow. The
/// erosion runs on `pool`; the result is the same with any number of workers. With a wind,
/// the rain follows the relief ([`refresh_rain`]).
pub fn generate_island(
    p: &IslandParams,
    erosion: &ErosionParams,
    pool: &TaskPool,
) -> (Field2<f32>, Flow) {
    let fields = island_fields(p);
    let mut height = fields.shape.map(|_| 0.0_f32);
    let mut rain = fields.rain;
    let mut run = Erosion::new();
    for s in 0..erosion.steps {
        refresh_rain(p, &height, erosion.sea_level, s, &mut rain);
        step(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &rain,
            erosion,
            pool,
            &mut run,
        );
    }
    let flow = if erosion.steps == 0 {
        drain(&height, erosion.sea_level, pool)
    } else {
        run.take_flow()
    };
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
/// spacing, then the heights as little-endian `f32`), so the erosion is paid once.
/// Returns the field and whether it came from the file.
pub fn cached_island(
    dir: &Path,
    p: &IslandParams,
    erosion: &ErosionParams,
    pool: &TaskPool,
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
    let (height, _) = generate_island(p, erosion, pool);
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
    fn a_west_wind_rains_on_the_west_and_leaves_the_mean_at_one() {
        let p = IslandParams {
            size: 65,
            ..IslandParams::island_16km(Seed::new(3), 256.0)
        };
        let pool = TaskPool::new(forge_task::PoolConfig::with_workers(0));
        let erosion = ErosionParams {
            steps: 30,
            ..ErosionParams::island()
        };
        let (height, _) = generate_island(&p, &erosion, &pool);
        let west = Wind {
            towards: 0,
            contrast: 1.0,
        };
        let rain = orographic_rain(&height, 0.0, west);
        let half = |from: u32, to: u32| {
            let (mut s, mut c) = (0.0, 0);
            for y in 0..65 {
                for x in from..to {
                    if height.get(x, y) > 0.0 {
                        s += f64::from(rain.get(x, y));
                        c += 1;
                    }
                }
            }
            (s, c)
        };
        let (ws, wc) = half(0, 32);
        let (es, ec) = half(32, 65);
        assert!(ws / wc as f64 > es / ec as f64, "west {ws} east {es}");
        assert!(((ws + es) / (wc + ec) as f64 - 1.0).abs() < 1e-3);
        assert!(rain.data.iter().all(|&r| (0.05..=10.0).contains(&r)));
        // No contrast, no change; the same wind, the same rain; a windy island erodes to
        // another field than a calm one, deterministically.
        let flat = orographic_rain(
            &height,
            0.0,
            Wind {
                contrast: 0.0,
                ..west
            },
        );
        assert!(flat.data.iter().all(|&r| (r - 1.0).abs() < 1e-6));
        assert_eq!(orographic_rain(&height, 0.0, west), rain);
        let windy = IslandParams {
            wind: Some(west),
            ..p
        };
        let (blown, _) = generate_island(&windy, &erosion, &pool);
        assert_ne!(blown, height);
        assert_eq!(generate_island(&windy, &erosion, &pool).0, blown);
        assert_ne!(island_key(&p, &erosion), island_key(&windy, &erosion));
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
        let pool = TaskPool::new(forge_task::PoolConfig::with_workers(0));
        let (first, from_cache) = cached_island(&dir, &p, &erosion, &pool).unwrap();
        assert!(!from_cache);
        let (second, from_cache) = cached_island(&dir, &p, &erosion, &pool).unwrap();
        assert!(from_cache);
        assert_eq!(first, second);
        assert_eq!(first, generate_island(&p, &erosion, &pool).0);
        // Other parameters, another file.
        let other = ErosionParams {
            steps: 21,
            ..erosion
        };
        assert!(!cached_island(&dir, &p, &other, &pool).unwrap().1);
        assert_ne!(island_key(&p, &erosion), island_key(&p, &other));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
