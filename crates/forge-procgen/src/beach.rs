//! Stage 6's beaches (#128, D-041's materials as rules): the beach band the slope rule paints
//! sand ([`crate::slope_layers`]) split along the coast: shingle on the headlands and under
//! steep land (the waves' energy leaves the pebbles there and takes the sand away), pale sand in
//! the bays and wherever a river reaches the sea (its sediment). On an island whose hard rock is
//! volcanic, black sand where it is hardest (the hardness field of stage 2, the dark rock the
//! waves grind); the owner's island has none, its hard rock being no basalt. The dark beaches
//! run a few metres out under the sea, so the waterline does not show a pale floor beside them.
//!
//! The rule reads its fields on a coarse grid, each blurred over a few hundred metres, so a
//! beach keeps its type along a stretch of coast; a little noise lets the stretches' ends wander
//! rather than cut across the beach in a straight line.

use crate::field::Field2;
use crate::noise;

/// How the beaches are split ([`paint_beaches`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeachRule {
    /// Metres a cell of the coarse grid the fields are read on.
    pub cell: f64,
    /// Metres around a point over which the share of sea tells a headland (much sea around it)
    /// from a bay (little).
    pub headland: f64,
    /// Metres around a point over which the land's height behind the beach is taken.
    pub backshore: f64,
    /// Metres over which the hardness is smoothed along the coast.
    pub stretch: f64,
    /// The share of the beaches, by their texels, that are black sand: the hardest rock's.
    pub black: f64,
    /// The share that are shingle: the roughest of the rest.
    pub shingle: f64,
    /// Metres from a river's mouth within which a beach stays pale sand.
    pub mouth: f64,
    /// Metres out under the sea the shingle and the black sand reach.
    pub under_sea: f64,
    /// How far either side of a type's threshold (in standard deviations of its score) the
    /// types mix in patches, so a stretch fades into the next.
    pub mixing: f64,
    /// The seed of the noise that lets the stretches' ends wander.
    pub seed: u64,
}

impl Default for BeachRule {
    /// A coarse grid of 32 m; headlands over 300 m, the land behind over 150 m, the hardness
    /// over 200 m; a sixth of the beaches black sand and a quarter shingle; pale sand within
    /// 400 m of a mouth; the dark beaches 12 m out under the sea; the types mixing over 0.8 of
    /// a standard deviation either side of their thresholds.
    fn default() -> Self {
        Self {
            cell: 32.0,
            headland: 300.0,
            backshore: 150.0,
            stretch: 200.0,
            black: 0.16,
            shingle: 0.25,
            mouth: 400.0,
            under_sea: 12.0,
            mixing: 0.8,
            seed: 0x6265_6163_6865_7321,
        }
    }
}

/// The layers [`paint_beaches`] reads and paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeachLayers {
    /// The beach band's sand, which the rule splits.
    pub sand: u8,
    /// The sea floor, which the dark beaches run out over.
    pub sea: u8,
    /// Pebbles.
    pub shingle: u8,
    /// Black volcanic sand, for an island whose hard rock is volcanic; `None`, none, whatever
    /// [`BeachRule::black`] says (the owner's island: its hard rock is no basalt).
    pub black: Option<u8>,
}

/// What [`paint_beaches`] painted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BeachStats {
    /// Metres of coast (beach texels on the sea, times their side) of pale sand, shingle and
    /// black sand.
    pub coast_m: [f64; 3],
    /// Texels of pale sand, shingle and black sand on the land.
    pub texels: [usize; 3],
    /// Texels of the sea floor turned to shingle or black sand.
    pub under_sea: usize,
}

/// Splits the beaches of `layers` (`ids.sand`; a layer map over `height`'s extent, rows along
/// +y) into pale sand, shingle and black sand by `rule`, over `height` (the sea at 0 m) and
/// `hardness` (stage 2's at a point in the same frame, read on the rule's coarse grid), keeping
/// pale sand around `mouths` (metres in the field's frame: where the rivers reach the sea).
pub fn paint_beaches(
    layers: &mut Field2<u8>,
    height: &Field2<f32>,
    hardness: &dyn Fn(f64, f64) -> f32,
    mouths: &[[f64; 2]],
    ids: BeachLayers,
    rule: &BeachRule,
) -> BeachStats {
    // The coarse fields: the share of sea, the land's mean height and the hardness.
    let extent = height.extent();
    let n = (extent / rule.cell).ceil().max(1.0) as usize;
    let centre = |c: usize| (c as f64 + 0.5) * rule.cell;
    let mut sea = vec![0.0; n * n];
    let mut land = vec![0.0; n * n];
    let mut hard = vec![0.0; n * n];
    for j in 0..n {
        for i in 0..n {
            let (mut wet, mut up) = (0.0, 0.0);
            for s in 0..4 {
                for t in 0..4 {
                    let x = (i as f64 + (f64::from(s) + 0.5) / 4.0) * rule.cell;
                    let y = (j as f64 + (f64::from(t) + 0.5) / 4.0) * rule.cell;
                    let h = f64::from(height.sample(x, y));
                    if h <= 0.0 {
                        wet += 1.0 / 16.0;
                    } else {
                        up += h / 16.0;
                    }
                }
            }
            sea[j * n + i] = wet;
            land[j * n + i] = up;
            hard[j * n + i] = f64::from(hardness(centre(i), centre(j)));
        }
    }
    let cells = |metres: f64| (metres / rule.cell).round().max(1.0) as usize;
    let sea_share = blur(&sea, n, cells(rule.headland));
    let land_sum = blur(&land, n, cells(rule.backshore));
    let dry = blur(
        &sea.iter().map(|w| 1.0 - w).collect::<Vec<_>>(),
        n,
        cells(rule.backshore),
    );
    let behind: Vec<f64> = land_sum
        .iter()
        .zip(&dry)
        .map(|(h, d)| h / d.max(0.05))
        .collect();
    let hard = blur(&hard, n, cells(rule.stretch));
    let read = |field: &[f64], x: f64, y: f64| {
        let last = (n - 1) as f64;
        let gx = (x / rule.cell - 0.5).clamp(0.0, last);
        let gy = (y / rule.cell - 0.5).clamp(0.0, last);
        let (i, j) = (gx.floor() as usize, gy.floor() as usize);
        let (i1, j1) = ((i + 1).min(n - 1), (j + 1).min(n - 1));
        let (fx, fy) = (gx - i as f64, gy - j as f64);
        let top = field[j * n + i] + (field[j * n + i1] - field[j * n + i]) * fx;
        let bottom = field[j1 * n + i] + (field[j1 * n + i1] - field[j1 * n + i]) * fx;
        top + (bottom - top) * fy
    };
    // The beach's texels and their scores.
    let texel = layers.spacing;
    let size = layers.size as usize;
    let at = |t: usize| {
        (
            ((t % size) as f64 + 0.5) * texel,
            ((t / size) as f64 + 0.5) * texel,
        )
    };
    let beach: Vec<usize> = (0..layers.data.len())
        .filter(|&t| layers.data[t] == ids.sand)
        .collect();
    let near_mouth = |x: f64, y: f64| {
        mouths
            .iter()
            .any(|m| (m[0] - x).hypot(m[1] - y) < rule.mouth)
    };
    let wander =
        |x: f64, y: f64, k: u64| noise::fbm(rule.seed ^ k, x / 120.0, y / 120.0, 2, 2.0, 0.5);
    let (mut hardest, mut roughest) = (Vec::new(), Vec::new());
    let mut free = Vec::with_capacity(beach.len());
    for &t in &beach {
        let (x, y) = at(t);
        free.push(!near_mouth(x, y));
        hardest.push(read(&hard, x, y));
        roughest.push((read(&sea_share, x, y), read(&behind, x, y)));
    }
    // Each score in standard deviations over the beaches, the wander added.
    let z = |values: &[f64]| {
        let count = values.len().max(1) as f64;
        let mean = values.iter().sum::<f64>() / count;
        let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / count;
        let sd = var.sqrt().max(1e-9);
        values.iter().map(|v| (v - mean) / sd).collect::<Vec<f64>>()
    };
    let hard_z = z(&hardest);
    let head_z = z(&roughest.iter().map(|r| r.0).collect::<Vec<_>>());
    let back_z = z(&roughest.iter().map(|r| r.1).collect::<Vec<_>>());
    let scores: Vec<(f64, f64)> = beach
        .iter()
        .enumerate()
        .map(|(k, &t)| {
            let (x, y) = at(t);
            (
                hard_z[k] + 0.35 * wander(x, y, 1),
                0.6 * head_z[k] + 0.4 * back_z[k] + 0.35 * wander(x, y, 2),
            )
        })
        .collect();
    // Black sand, where there is any: the hardest share of the beaches away from the mouths;
    // shingle: the roughest share of the rest.
    let black = ids.black.filter(|_| rule.black > 0.0);
    let black_at = if black.is_some() {
        quantile(
            scores
                .iter()
                .zip(&free)
                .filter(|(_, f)| **f)
                .map(|(s, _)| s.0)
                .collect(),
            1.0 - rule.black,
        )
    } else {
        f64::INFINITY
    };
    let rest = if black.is_some() {
        (1.0 - rule.black).max(1e-9)
    } else {
        1.0
    };
    let shingle_at = quantile(
        scores
            .iter()
            .zip(&free)
            .filter(|(s, f)| **f && s.0 < black_at)
            .map(|(s, _)| s.1)
            .collect(),
        1.0 - rule.shingle / rest,
    );
    // Near a threshold the types mix in patches a few metres wide, so a stretch fades into the
    // next over tens of metres instead of ending in a line across the beach.
    let mixed = |score: f64, at: f64, x: f64, y: f64, k: u64| {
        let share = smoothstep(at - rule.mixing, at + rule.mixing, score);
        let patch = 0.5 + 0.5 * noise::fbm(rule.seed ^ k, x / 10.0, y / 10.0, 2, 2.0, 0.5);
        patch < share
    };
    let kinds = [Some(ids.sand), Some(ids.shingle), black];
    let mut stats = BeachStats::default();
    for (k, &t) in beach.iter().enumerate() {
        let (hard, rough) = scores[k];
        let (x, y) = at(t);
        let kind = if !free[k] {
            0
        } else if black.is_some() && mixed(hard, black_at, x, y, 3) {
            2
        } else if mixed(rough, shingle_at, x, y, 4) {
            1
        } else {
            0
        };
        layers.data[t] = kinds[kind].expect("a type painted");
        stats.texels[kind] += 1;
    }
    // The coast's length by type: the beach's texels beside the sea.
    let neighbours = |t: usize| {
        let (x, y) = (t % size, t / size);
        [
            (x > 0).then(|| t - 1),
            (x + 1 < size).then(|| t + 1),
            (y > 0).then(|| t - size),
            (y + 1 < size).then(|| t + size),
        ]
    };
    for &t in &beach {
        if neighbours(t)
            .into_iter()
            .flatten()
            .any(|u| layers.data[u] == ids.sea)
        {
            let kind = kinds
                .iter()
                .position(|&l| l == Some(layers.data[t]))
                .expect("a beach's layer");
            stats.coast_m[kind] += texel;
        }
    }
    // The dark beaches out under the sea, a texel a step from the last step's, black sand
    // before shingle.
    let steps = (rule.under_sea / texel).round() as usize;
    let darks: Vec<u8> = black.into_iter().chain([ids.shingle]).collect();
    let mut fronts: Vec<Vec<usize>> = darks
        .iter()
        .map(|&dark| {
            beach
                .iter()
                .copied()
                .filter(|&t| layers.data[t] == dark)
                .collect()
        })
        .collect();
    for _ in 0..steps {
        for (front, &dark) in fronts.iter_mut().zip(&darks) {
            let mut next = Vec::new();
            for &t in front.iter() {
                for u in neighbours(t).into_iter().flatten() {
                    if layers.data[u] == ids.sea {
                        layers.data[u] = dark;
                        stats.under_sea += 1;
                        next.push(u);
                    }
                }
            }
            *front = next;
        }
    }
    stats
}

/// How far the salt water keeps the grass off the land beside it ([`paint_salt`], #199): the
/// spray and the floods of the sea and of the rivers' tidal reaches, where only sand and the
/// plants that bear salt hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SaltRule {
    /// Metres from the salt water within which it reaches.
    pub reach: f64,
    /// Metres over the sea the sand rises to at the water, falling to the beach's own top
    /// (`sand_below` of [`paint_salt`]) at the reach.
    pub above: f64,
    /// How much of that rise wanders, over noise 30 m wide, so its line does not keep to one
    /// height.
    pub wander: f64,
    /// The seed of that noise.
    pub seed: u64,
}

impl Default for SaltRule {
    /// Within 40 m of the salt water, the sand up to 5 m over the sea at the water, the rise
    /// wandering by a third.
    fn default() -> Self {
        Self {
            reach: 40.0,
            above: 5.0,
            wander: 0.35,
            seed: 0x7361_6c74_2d6b_6565,
        }
    }
}

/// Turns the grasses of `layers` (any of `grasses`) beside the salt water to `sand` (#199): the
/// texels within `rule.reach` of salt water whose ground (`ground` at their centre, metres over
/// the sea, the rivers' channels carved) lies under the sand's rise there. The salt water is the
/// ground under the sea's level: the sea and the rivers' reaches it fills (`sea` texels, or
/// ground at or under 0 m). Over the reach the rise falls from `rule.above` to `sand_below`, the
/// beach's own top. Returns the texels turned.
pub fn paint_salt(
    layers: &mut Field2<u8>,
    ground: &dyn Fn(f64, f64) -> f64,
    grasses: &[u8],
    sea: u8,
    sand: u8,
    sand_below: f64,
    rule: &SaltRule,
) -> usize {
    let size = layers.size as usize;
    let texel = layers.spacing;
    let centre = |t: usize| {
        (
            ((t % size) as f64 + 0.5) * texel,
            ((t / size) as f64 + 0.5) * texel,
        )
    };
    // The distance to the salt water, in texels, by a two-pass chamfer (3-4) up to the reach.
    let far = (rule.reach / texel).ceil() as u32 * 3 + 3;
    let mut distance: Vec<u32> = (0..size * size)
        .map(|t| {
            let (x, y) = centre(t);
            if layers.data[t] == sea || ground(x, y) <= 0.0 {
                0
            } else {
                far
            }
        })
        .collect();
    for y in 0..size {
        for x in 0..size {
            let t = y * size + x;
            let mut d = distance[t];
            if x > 0 {
                d = d.min(distance[t - 1] + 3);
            }
            if y > 0 {
                d = d.min(distance[t - size] + 3);
                if x > 0 {
                    d = d.min(distance[t - size - 1] + 4);
                }
                if x + 1 < size {
                    d = d.min(distance[t - size + 1] + 4);
                }
            }
            distance[t] = d.min(far);
        }
    }
    for y in (0..size).rev() {
        for x in (0..size).rev() {
            let t = y * size + x;
            let mut d = distance[t];
            if x + 1 < size {
                d = d.min(distance[t + 1] + 3);
            }
            if y + 1 < size {
                d = d.min(distance[t + size] + 3);
                if x + 1 < size {
                    d = d.min(distance[t + size + 1] + 4);
                }
                if x > 0 {
                    d = d.min(distance[t + size - 1] + 4);
                }
            }
            distance[t] = d;
        }
    }
    let mut turned = 0;
    for (t, &d) in distance.iter().enumerate() {
        if d >= far || !grasses.contains(&layers.data[t]) {
            continue;
        }
        let metres = f64::from(d) / 3.0 * texel;
        if metres > rule.reach {
            continue;
        }
        let (x, y) = centre(t);
        let near = 1.0 - metres / rule.reach;
        let wander = 1.0 + rule.wander * noise::fbm(rule.seed, x / 30.0, y / 30.0, 2, 2.0, 0.5);
        let top = sand_below + (rule.above - sand_below).max(0.0) * near * near * wander;
        if ground(x, y) < top {
            layers.data[t] = sand;
            turned += 1;
        }
    }
    turned
}

/// `values` blurred by a box `reach` cells either way, along the rows then the columns (an
/// `n × n` grid; the box shrinks at the edges).
pub(crate) fn blur(values: &[f64], n: usize, reach: usize) -> Vec<f64> {
    let pass = |src: &[f64], along_rows: bool| {
        let mut out = vec![0.0; n * n];
        for a in 0..n {
            let index = |b: usize| if along_rows { a * n + b } else { b * n + a };
            let mut prefix = vec![0.0; n + 1];
            for b in 0..n {
                prefix[b + 1] = prefix[b] + src[index(b)];
            }
            for b in 0..n {
                let (lo, hi) = (b.saturating_sub(reach), (b + reach).min(n - 1));
                out[index(b)] = (prefix[hi + 1] - prefix[lo]) / (hi - lo + 1) as f64;
            }
        }
        out
    };
    pass(&pass(values, true), false)
}

/// Hermite's smoothstep of `x` from `a` to `b`.
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The value `share` of the way up `values` (0: the least, 1: the most); infinite for none.
fn quantile(mut values: Vec<f64>, share: f64) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    values.sort_by(f64::total_cmp);
    let k = ((values.len() - 1) as f64 * share.clamp(0.0, 1.0)).round() as usize;
    values[k]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_salt_water_keeps_the_grass_off_its_banks_but_not_off_the_land_above() {
        // The sea under y = 400 m of a 1 km square at 4 m; the land rising 1 in 10 from it, and
        // a plateau at 6 m from x = 600 m. Grass (0) on the land, the sea floor (2) under it.
        let ground = |x: f64, y: f64| {
            if y < 400.0 {
                -1.0
            } else if x >= 600.0 {
                6.0
            } else {
                0.1 * (y - 400.0)
            }
        };
        let mut layers = Field2::from_fn(256, 4.0, |i, j| {
            let (x, y) = ((f64::from(i) + 0.5) * 4.0, (f64::from(j) + 0.5) * 4.0);
            if ground(x, y) <= 0.0 { 2 } else { 0 }
        });
        let rule = SaltRule {
            wander: 0.0,
            ..SaltRule::default()
        };
        let turned = paint_salt(&mut layers, &ground, &[0], 2, 1, 2.0, &rule);
        assert!(turned > 0);
        let at = |x: f64, y: f64| layers.get((x / 4.0) as u32, (y / 4.0) as u32);
        // On the slope: sand 12 m from the water (1 m up, under the rise's 3.5 m), grass 28 m
        // from it (2.6 m up, over its 2.3 m) and beyond the reach.
        assert_eq!(at(300.0, 410.0), 1);
        assert_eq!(at(300.0, 426.0), 0);
        assert_eq!(at(300.0, 470.0), 0);
        // The plateau at 6 m stays grass even beside the water, and the sea stays the sea.
        assert_eq!(at(800.0, 402.0), 0);
        assert_eq!(at(300.0, 300.0), 2);
    }

    /// The type of the beach at `x` (layers 1, 3 or 4): the most texels of its column's band.
    fn kind_in(layers: &Field2<u8>, x: f64) -> usize {
        let i = (x / 4.0) as u32;
        let mut count = [0; 5];
        for j in 0..1024 {
            count[usize::from(layers.get(i, j))] += 1;
        }
        [1, 3, 4]
            .into_iter()
            .max_by_key(|&l| count[l])
            .expect("a beach")
    }

    #[test]
    fn a_coast_turns_to_shingle_on_its_headland_black_where_hard_and_sand_in_its_bay() {
        // A coast along y = 1 km over a 4 km square at 8 m: land to the north (lower y), the
        // sea to the south, a headland 400 m out at x = 1 km, a bay 300 m in at x = 3 km, the
        // rock hardest around x = 2 km, a river's mouth at x = 3.6 km.
        let coast = |x: f64| {
            1000.0 + 400.0 * (-((x - 1000.0) / 250.0).powi(2)).exp()
                - 300.0 * (-((x - 3000.0) / 250.0).powi(2)).exp()
        };
        let height = Field2::from_fn(513, 8.0, |i, j| {
            let (x, y) = (f64::from(i) * 8.0, f64::from(j) * 8.0);
            ((coast(x) - y) * 0.02) as f32
        });
        let hardness = Field2::from_fn(513, 8.0, |i, _| {
            let x = f64::from(i) * 8.0;
            (1.0 + 0.5 * (-((x - 2000.0) / 300.0).powi(2)).exp()) as f32
        });
        let ids = BeachLayers {
            sand: 1,
            sea: 2,
            shingle: 3,
            black: Some(4),
        };
        // Sand on the first 2 m over the sea, grass over it (the slope rule's shore).
        let fresh = || {
            Field2::from_fn(1024, 4.0, |i, j| {
                let h = height.sample((f64::from(i) + 0.5) * 4.0, (f64::from(j) + 0.5) * 4.0);
                if h <= 0.0 {
                    2
                } else if h < 2.0 {
                    1
                } else {
                    0
                }
            })
        };
        let mut layers = fresh();
        let rule = BeachRule::default();
        let stats = paint_beaches(
            &mut layers,
            &height,
            &|x, y| hardness.sample(x, y),
            &[[3600.0, coast(3600.0)]],
            ids,
            &rule,
        );
        assert_eq!(kind_in(&layers, 1000.0), 3, "the headland's shingle");
        assert_eq!(kind_in(&layers, 2000.0), 4, "the hard rock's black sand");
        assert_eq!(kind_in(&layers, 3000.0), 1, "the bay's sand");
        assert_eq!(kind_in(&layers, 3600.0), 1, "the mouth's sand");
        // About a sixth black and a quarter shingle, the rest sand.
        let total: usize = stats.texels.iter().sum();
        let share = |k: usize| stats.texels[k] as f64 / total as f64;
        assert!((share(2) - 0.16).abs() < 0.05, "{stats:?}");
        assert!((share(1) - 0.25).abs() < 0.08, "{stats:?}");
        // The dark beaches run three texels (12 m) out under the sea and no farther: every dark
        // texel at sea is three steps or fewer from the land.
        assert!(stats.under_sea > 0);
        let land = |i: i64, j: i64| {
            (0..1024).contains(&i) && (0..1024).contains(&j) && {
                let (x, y) = ((i as f64 + 0.5) * 4.0, (j as f64 + 0.5) * 4.0);
                height.sample(x, y) > 0.0
            }
        };
        for i in 0..1024_i64 {
            for j in 0..1024_i64 {
                let l = layers.get(i as u32, j as u32);
                if (l == 3 || l == 4) && !land(i, j) {
                    let near = (-3..=3_i64).any(|di: i64| {
                        (-3..=3_i64).any(|dj: i64| di.abs() + dj.abs() <= 3 && land(i + di, j + dj))
                    });
                    assert!(near, "{i} {j}");
                }
            }
        }
        // Without black sand (an island whose hard rock is no basalt): none, the hard rock's
        // beach pale sand or shingle, and a quarter of the beaches shingle.
        let mut layers = fresh();
        let stats = paint_beaches(
            &mut layers,
            &height,
            &|x, y| hardness.sample(x, y),
            &[[3600.0, coast(3600.0)]],
            BeachLayers { black: None, ..ids },
            &rule,
        );
        assert_eq!(stats.texels[2], 0);
        assert!(!layers.data.contains(&4));
        let total: usize = stats.texels.iter().sum();
        let shingle = stats.texels[1] as f64 / total as f64;
        assert!((shingle - 0.25).abs() < 0.08, "{stats:?}");
        assert_eq!(kind_in(&layers, 1000.0), 3, "the headland's shingle");
    }
}
