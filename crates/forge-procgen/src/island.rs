//! Stages 1 and 2 of the terrain pipeline (`docs/research/terrain-genesis.md`): the island's
//! mask and the fields the erosion runs on, an uplift rate, a hardness and a rainfall; and the
//! whole island as one call, [`generate_island`], cached on disk by [`cached_island`].

use std::f64::consts::{PI, TAU};
use std::io;
use std::path::Path;

use forge_core::Seed;
use forge_core::dmath::{atan2, cos, exp, sin_cos};
use forge_core::hash::{hash_cell2, unit_f32};
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
    /// The coastal plain (D-041, #112): the share of the radius inland over which the uplift
    /// stays at `plain_uplift` of its full rate before the hills rise; 0 has no plain, the
    /// hills rising from the coast. Rivers cross the plain at a gentle slope to the sea.
    pub plain: f64,
    /// The uplift on the plain, as a share of the rate the hills start from.
    pub plain_uplift: f64,
    /// How far the plain's width wanders along the coast, as a share of it (0 the same all
    /// round; 1 from none to twice as wide), over the coast's own scale.
    pub plain_wander: f64,
    /// The large basins (D-041, #123): how many trunk valleys the uplift is lowered along,
    /// each draining a sector of the island; 0 lifts a dome, rivers running out on every side.
    pub basins: u32,
    /// How much of the hills' uplift the trunks' lines lose, a share of it: their sectors'
    /// ridges keep it all.
    pub basin_depth: f64,
    /// How far the trunks' lines turn as they run inland, radians at most.
    pub basin_turn: f64,
    /// The lakes placed on purpose (D-040): a bowl on each trunk's line, partway inland, that
    /// loses this share of the uplift at its centre; 0 places none.
    pub basin_lakes: f64,
    /// The bowls' radius, metres.
    pub basin_lake_radius: f64,
    /// The prevailing wind; `None` rains the same everywhere.
    pub wind: Option<Wind>,
    /// After the erosion, the depressions smaller than this (m²) fill to their spill level
    /// and the larger ones stay as lakes (the lake rule, issue #97,
    /// [`crate::fill_small_depressions`]); 0 keeps every one.
    pub lake_min_area_m2: f64,
    /// After the lake rule, the alluvium's grade (#123, [`crate::flow::grade_to_the_sea`]):
    /// every land sample at least this many metres over the sea per metre of its way down to
    /// it, so a large river's lower course falls to its mouth instead of lying at the sea's
    /// level; 0 leaves the erosion's field.
    pub grade: f64,
}

impl IslandParams {
    /// A 16 km island at `spacing` metres: `size` samples a side to cover it. A coastal plain
    /// over a quarter of the radius, from none to wide along the coast, at 3 % of the hills'
    /// uplift (D-041): its rivers reach the sea at 1–5 % where they fell at 9–24 % from hills
    /// rising straight out of the sea. Three trunk valleys losing 85 % of the hills' uplift
    /// along lines that turn by up to 0.3 rad, a lake's bowl on each, and the lower courses
    /// graded at 0.3 % to the sea (D-041's scale, #123): basins of 23, 18 and 12 km² on seed 7
    /// where the dome's largest were 11, 11 and 9.
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
            plain: 0.25,
            plain_uplift: 0.03,
            plain_wander: 3.0,
            basins: 3,
            basin_depth: 0.85,
            basin_turn: 0.3,
            basin_lakes: 0.85,
            basin_lake_radius: 550.0,
            wind: None,
            lake_min_area_m2: 50_000.0,
            grade: 0.003,
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

/// The island's heightfield: stages 1–3 from a flat sea, then the lake rule
/// ([`IslandParams::lake_min_area_m2`]), and its flow. The erosion runs on `pool`; the result
/// is the same with any number of workers. With a wind, the rain follows the relief
/// ([`refresh_rain`]).
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
    let fill = if p.lake_min_area_m2 > 0.0 {
        crate::fill_small_depressions(&mut height, erosion.sea_level, p.lake_min_area_m2)
    } else {
        crate::DepressionFill::default()
    };
    let flow = if erosion.steps == 0 || fill.filled > 0 {
        drain(&height, erosion.sea_level, pool)
    } else {
        run.take_flow()
    };
    if p.grade > 0.0
        && crate::flow::grade_to_the_sea(&mut height, &flow, erosion.sea_level, p.grade) > 0
    {
        let flow = drain(&height, erosion.sea_level, pool);
        return (height, flow);
    }
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

/// The angles of the large basins' trunks (D-041, #123), radians in `[0, τ)`, ascending:
/// [`IslandParams::basins`] of them spread evenly from a seeded start, each moved by up to a
/// fifth of the spacing so the basins differ in size.
fn trunk_angles(p: &IslandParams, seed: u64) -> Vec<f64> {
    if p.basins == 0 {
        return Vec::new();
    }
    let draw = |i: i32| f64::from(unit_f32(hash_cell2(seed, i, 0)));
    let gap = TAU / f64::from(p.basins);
    let start = TAU * draw(-1);
    let mut angles: Vec<f64> = (0..p.basins as i32)
        .map(|i| (start + gap * (f64::from(i) + 0.4 * (draw(i) - 0.5))).rem_euclid(TAU))
        .collect();
    angles.sort_by(f64::total_cmp);
    angles
}

/// How far the trunks' lines have turned at `(mx, my)` (metres in the field's frame), radians.
fn trunk_turn(p: &IslandParams, seed: u64, mx: f64, my: f64) -> f64 {
    let period = p.coast_scale_km * 1000.0;
    p.basin_turn * fbm(seed, mx / period, my / period, 3, 2.0, 0.5)
}

/// The lakes' bowls (D-040), `(x, y)` metres in the field's frame and the unit direction out
/// from the heart there, along the valley: none without [`IslandParams::basin_lakes`], else
/// one on each trunk's turned line, a seeded 2.3–3.3 km from the heart (inside the hills,
/// below the trunks' heads).
fn lake_bowls(p: &IslandParams, trunks: &[f64], seed: u64) -> Vec<[f64; 4]> {
    if p.basin_lakes <= 0.0 {
        return Vec::new();
    }
    let half = 0.5 * p.extent();
    trunks
        .iter()
        .enumerate()
        .map(|(i, &angle)| {
            let draw = f64::from(unit_f32(hash_cell2(seed, i as i32, 1)));
            let r = half * (0.28 + 0.12 * draw);
            // The line's angle at that radius: a few fixed-point steps, the turn changing
            // slowly along the circle.
            let mut at = angle;
            let mut point = [0.0; 4];
            for _ in 0..8 {
                let (sin, cos) = sin_cos(at);
                point = [half + r * cos, half + r * sin, cos, sin];
                at = angle - trunk_turn(p, seed, point[0], point[1]);
            }
            point
        })
        .collect()
}

/// Where `angle` stands across its basin: 0 on the nearest trunk's line, 1 on the ridge
/// halfway to the next trunk, linear in the angle in between.
fn across_sector(trunks: &[f64], angle: f64) -> f64 {
    let a = angle.rem_euclid(TAU);
    let n = trunks.len();
    let next = trunks.iter().position(|&t| t > a).unwrap_or(n);
    let before = if next == 0 {
        trunks[n - 1] - TAU
    } else {
        trunks[next - 1]
    };
    let after = if next == n {
        trunks[0] + TAU
    } else {
        trunks[next]
    };
    let ridge = 0.5 * (before + after);
    if a < ridge {
        (a - before) / (ridge - before)
    } else {
        (after - a) / (after - ridge)
    }
}

/// The rock's hardness at (`x`, `y`) metres in the field's frame, as [`island_fields`] samples
/// it: 1 give or take a half, in patches about 1.8 km across.
pub fn island_hardness(p: &IslandParams, x: f64, y: f64) -> f32 {
    let n = fbm(lattice(p.seed, 3), x / 1800.0, y / 1800.0, 3, 2.0, 0.5);
    (1.0 + 0.5 * n) as f32
}

/// Stages 1 and 2: the mask, then the uplift, hardness and rain fields.
pub fn island_fields(p: &IslandParams) -> IslandFields {
    let half = 0.5 * p.extent();
    let coast_seed = lattice(p.seed, 1);
    let ridge_seed = lattice(p.seed, 2);
    let plain_seed = lattice(p.seed, 4);
    let basin_seed = lattice(p.seed, 5);
    let trunks = trunk_angles(p, basin_seed);
    let lakes = lake_bowls(p, &trunks, basin_seed);
    let lake_seed = lattice(p.seed, 6);
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
        // The large basins: the hills' uplift lowered towards each trunk's line, whole on the
        // ridges between them, fading out near the heart so the trunks' heads share one massif
        // (the coastal plain loses part of it, below).
        let (basins, mute) = if trunks.is_empty() {
            (1.0, 0.0)
        } else {
            let (cx, cy) = (mx - half, my - half);
            let d = (cx * cx + cy * cy).sqrt() / half;
            let turn = trunk_turn(p, basin_seed, mx, my);
            let s = across_sector(&trunks, atan2(cy, cx) + turn);
            let valley = 0.5 * (1.0 + cos(PI * s));
            // Each bowl half again as long down its valley as its radius and two thirds as
            // wide, its edge ragged by a quarter.
            let bowls = lakes.iter().fold(1.0, |b, l| {
                let (dx, dy) = (mx - l[0], my - l[1]);
                let along = (dx * l[2] + dy * l[3]) / 1.5;
                let across = (dy * l[2] - dx * l[3]) / 0.67;
                let ragged = 1.0 + 0.25 * fbm(lake_seed, mx / 400.0, my / 400.0, 2, 2.0, 0.5);
                let r = (along * along + across * across).sqrt() / (p.basin_lake_radius * ragged);
                b * (1.0 - p.basin_lakes * (1.0 - crate::river::smoothstep(0.3, 1.0, r)))
            });
            let lowered = 1.0 - p.basin_depth * crate::river::smoothstep(0.08, 0.25, d) * valley;
            (lowered * bowls, 0.5 * valley)
        };
        // Inland the shape reaches about `radius`; a square root lifts the coast's foothills,
        // from the coast or from the inner edge of the coastal plain.
        let t = (s / p.radius).min(1.0);
        let inland = if p.plain > 0.0 {
            let wander = fbm(
                plain_seed,
                mx / coast_period,
                my / coast_period,
                3,
                2.0,
                0.5,
            );
            let plain = (p.plain * (1.0 + p.plain_wander * wander)).clamp(0.0, 0.9);
            // The foothills: the square root eased in over the first eighth of the rise, so
            // the hills leave the plain on a slope rather than a wall.
            let x = ((t - plain) / (1.0 - plain)).max(0.0);
            let hills = x.sqrt() * crate::river::smoothstep(0.0, 0.125, x);
            // The plain loses seven tenths of what the hills lose: enough for the trunks to
            // gather their flanks across it, not so much that their last kilometres sink to
            // the sea's level (the whole of it left 6.6 km² of land under 2.5 m, 0.6 before).
            let share = 0.7;
            p.plain_uplift * (1.0 - share * (1.0 - basins))
                + (1.0 - p.plain_uplift) * hills * basins
        } else {
            t.sqrt() * basins
        };
        // The ridges calmed by half along the trunks, so their crests do not split a basin.
        let ridges = ridges + mute * (0.5 - ridges);
        (p.uplift * inland * (0.35 + 0.65 * ridges)) as f32
    });
    let hardness = Field2::from_fn(p.size, p.spacing, |x, y| {
        island_hardness(p, f64::from(x) * p.spacing, f64::from(y) * p.spacing)
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
    fn three_trunks_gather_the_island_into_three_large_basins_with_lakes_in_their_valleys() {
        let pool = TaskPool::new(forge_task::PoolConfig::with_workers(4));
        let erosion = ErosionParams::island();
        const SPACING: f64 = 16.0;
        let trunks = IslandParams::island_16km(Seed::new(7), SPACING);
        let dome = IslandParams {
            basins: 0,
            ..trunks
        };
        let km2 = |n: u32| f64::from(n) * SPACING * SPACING * 1e-6;
        let largest = |p: &IslandParams| {
            let (height, flow) = generate_island(p, &erosion, &pool);
            let basins: Vec<f64> = flow.basins().iter().take(3).map(|&(_, n)| km2(n)).collect();
            (height, basins)
        };
        let (_, before) = largest(&dome);
        let (height, after) = largest(&trunks);
        // The dome's three largest basins are 8–12 km²; the trunks' 12–23.
        assert!(
            after.iter().all(|&a| a > 10.0) && after[0] > 20.0,
            "{after:?} km² (the dome's {before:?})"
        );
        let sum = |b: &[f64]| b.iter().sum::<f64>();
        assert!(
            sum(&after) > 1.5 * sum(&before),
            "{after:?} against {before:?} km²"
        );
        // At least two of the three bowls hold water a metre deep or more.
        let filled = crate::priority_flood(&height, erosion.sea_level);
        let bowls = lake_bowls(
            &trunks,
            &trunk_angles(&trunks, lattice(trunks.seed, 5)),
            lattice(trunks.seed, 5),
        );
        assert_eq!(bowls.len(), 3);
        let wet = bowls
            .iter()
            .filter(|b| {
                let reach = (trunks.basin_lake_radius / SPACING) as i32;
                let (bx, by) = ((b[0] / SPACING) as i32, (b[1] / SPACING) as i32);
                (-reach..=reach).any(|dy| {
                    (-reach..=reach).any(|dx| {
                        let (x, y) = ((bx + dx) as u32, (by + dy) as u32);
                        filled.get(x, y) - height.get(x, y) > 1.0
                    })
                })
            })
            .count();
        assert!(wet >= 2, "{wet} bowls hold a lake");
        // No trunks, no change: the dome is the island of before.
        assert!(trunk_angles(&dome, 0).is_empty());
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
