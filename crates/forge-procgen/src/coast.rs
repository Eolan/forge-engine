//! The signed distance to the coast, metres, positive inland and negative at sea: the field
//! the water's shore work keys on (`docs/research/water.md` §2 and §5: wave damping, the
//! shore trains' direction `−∇d` and phase `d`, the foam line, the wet band). An exact
//! Euclidean distance transform (Felzenszwalb & Huttenlocher 2012: the lower envelope of
//! parabolas, one pass along the rows and one along the columns), rows in parallel on the job
//! system, the same bytes with any number of workers (D-016).

use forge_task::TaskPool;

use crate::field::Field2;

/// A value standing for "no site" in the squared-distance passes.
const FAR: f64 = 1.0e30;

/// The 1-D squared distance transform of `f` into `d` (Felzenszwalb & Huttenlocher's lower
/// envelope), `v` and `z` scratch of `f.len()` and `f.len() + 1`.
fn transform_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0;
    v[0] = 0;
    z[0] = -FAR;
    z[1] = FAR;
    for q in 1..n {
        let fq = f[q] + (q * q) as f64;
        loop {
            let p = v[k];
            let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                if s > z[k] {
                    k += 1;
                } else {
                    // The new parabola replaces the whole envelope so far.
                    k = 0;
                }
                v[k] = q;
                z[k] = s;
                z[k + 1] = FAR;
                break;
            }
        }
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f64 - p as f64;
        *out = dq * dq + f[p];
    }
}

/// The squared distance, in cells, from every cell to the nearest cell where `site` holds:
/// rows then columns, each direction's rows in parallel.
fn squared_distance(size: usize, site: impl Fn(usize) -> bool + Sync, pool: &TaskPool) -> Vec<f64> {
    let n = size;
    let mut rows = vec![0.0_f64; n * n];
    pool.par_chunks_mut(&mut rows, n, |y, row| {
        let f: Vec<f64> = (0..n)
            .map(|x| if site(y * n + x) { 0.0 } else { FAR })
            .collect();
        let (mut v, mut z) = (vec![0; n], vec![0.0; n + 1]);
        transform_1d(&f, row, &mut v, &mut z);
    });
    // The columns: each task takes a column out, transforms it, and writes it into the
    // transposed result, which is transposed back by rows.
    let mut columns = vec![0.0_f64; n * n];
    pool.par_chunks_mut(&mut columns, n, |x, out| {
        let f: Vec<f64> = (0..n).map(|y| rows[y * n + x]).collect();
        let (mut v, mut z) = (vec![0; n], vec![0.0; n + 1]);
        transform_1d(&f, out, &mut v, &mut z);
    });
    pool.par_chunks_mut(&mut rows, n, |y, row| {
        for (x, d) in row.iter_mut().enumerate() {
            *d = columns[x * n + y];
        }
    });
    rows
}

/// The distance, metres, from every sample of a `size × size` field `spacing` apart to the
/// nearest sample where `site` holds (#106: how far the ground stands from the water).
pub fn site_distance(
    size: u32,
    spacing: f64,
    site: impl Fn(usize) -> bool + Sync,
    pool: &TaskPool,
) -> Field2<f32> {
    let n = size as usize;
    let squared = squared_distance(n, site, pool);
    let mut distance = Field2::new(size, spacing);
    pool.par_chunks_mut(&mut distance.data, n, |y, row| {
        for (x, d) in row.iter_mut().enumerate() {
            *d = (squared[y * n + x].sqrt() * spacing) as f32;
        }
    });
    distance
}

/// The signed distance to the coast, metres: for a land cell (above `sea_level`) the distance
/// to the nearest sea cell, for a sea cell minus the distance to the nearest land cell, both
/// less half a cell so the coast line itself is at zero. A field without sea is all `+∞`-like
/// large values, one without land all negative.
pub fn coast_distance(height: &Field2<f32>, sea_level: f32, pool: &TaskPool) -> Field2<f32> {
    let n = height.size as usize;
    let land = |i: usize| height.data[i] > sea_level;
    let to_sea = squared_distance(n, |i| !land(i), pool);
    let to_land = squared_distance(n, land, pool);
    let half = 0.5 * height.spacing;
    let mut distance = Field2::new(height.size, height.spacing);
    pool.par_chunks_mut(&mut distance.data, n, |y, row| {
        for (x, d) in row.iter_mut().enumerate() {
            let i = y * n + x;
            *d = if land(i) {
                (to_sea[i].sqrt() * height.spacing - half) as f32
            } else {
                -(to_land[i].sqrt() * height.spacing - half) as f32
            };
        }
    });
    distance
}

/// The sea floor under a field whose sea stands flat at `sea_level` (the island's, the
/// erosion's base level): each sea sample lowered to `depth × (1 − e^(d / width))` under the
/// level, `d` its signed distance to the coast (`coast`, negative at sea), so the floor leaves
/// the shore at a slope of `depth / width` and levels off at `depth`. Land samples keep their
/// height. The water's shore work reads the floor's depth (`docs/research/water.md` §2, §5), and
/// a sea drawn as a plane at the level then meets the ground between the samples, along the
/// coast, instead of along the grid's edges where the flat sea met the first land sample (#96).
pub fn sea_floor(
    height: &mut Field2<f32>,
    coast: &Field2<f32>,
    sea_level: f32,
    depth: f32,
    width: f32,
) {
    assert_eq!(height.size, coast.size, "the coast distance of this field");
    for (h, &d) in height.data.iter_mut().zip(&coast.data) {
        if *h <= sea_level {
            *h = sea_level - depth * (1.0 - forge_core::dmath::exp(d.min(0.0) / width));
        }
    }
}

/// Smooths the ground where it stands within `band` metres of `sea_level` (#106): `passes` of a
/// 3 × 3 binomial filter (¼, ½, ¼ each way) over those samples, the others kept. Where eroded
/// land at the sea's level meets [`sea_floor`]'s first metres, the samples alternate between the
/// two along a coast that runs across the grid, and the sea's level traced through them steps
/// with the samples; filtered, it runs smooth.
pub fn smooth_shore(height: &mut Field2<f32>, sea_level: f32, band: f32, passes: u32) {
    let n = height.size as usize;
    let near: Vec<bool> = height
        .data
        .iter()
        .map(|&h| (h - sea_level).abs() <= band)
        .collect();
    for _ in 0..passes {
        let before = height.data.clone();
        let at = |x: usize, y: usize| before[y.min(n - 1) * n + x.min(n - 1)];
        for (i, h) in height.data.iter_mut().enumerate() {
            if !near[i] {
                continue;
            }
            let (x, y) = (i % n, i / n);
            let (xl, yl) = (x.saturating_sub(1), y.saturating_sub(1));
            let row = |y: usize| 0.25 * at(xl, y) + 0.5 * at(x, y) + 0.25 * at(x + 1, y);
            *h = 0.25 * row(yl) + 0.5 * row(y) + 0.25 * row(y + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_task::PoolConfig;

    #[test]
    fn a_coast_across_the_grid_comes_out_smooth_and_the_rest_stays() {
        // Land at 0.1 m and sea floor at −0.3 m, split along a diagonal staircase; a hill far
        // from the sea's level.
        let mut field = Field2::from_fn(32, 8.0, |x, y| {
            if x + y < 30 {
                0.1
            } else if x > 28 && y > 28 {
                20.0
            } else {
                -0.3
            }
        });
        smooth_shore(&mut field, 0.0, 2.0, 4);
        // The hill is untouched; across the coast the ground now falls through intermediate
        // heights instead of jumping.
        assert_eq!(field.get(30, 30), 20.0);
        let step = field.get(15, 14) - field.get(15, 15);
        assert!(step > 0.0 && step < 0.4, "{step}");
        assert!(field.get(10, 10) > 0.09 && field.get(20, 20) < -0.29);
    }

    #[test]
    fn the_sea_floor_falls_away_from_the_coast_and_the_land_keeps_its_height() {
        // A 20-sample strip of land in a sea, 10 m samples.
        let mut field = Field2::from_fn(
            65,
            10.0,
            |x, _| if (22..42).contains(&x) { 5.0 } else { 0.0 },
        );
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let coast = coast_distance(&field, 0.0, &pool);
        sea_floor(&mut field, &coast, 0.0, 60.0, 150.0);
        let row: Vec<f32> = (0..65).map(|x| field.get(x, 32)).collect();
        assert!(row[22..42].iter().all(|&h| h == 5.0), "the land: {row:?}");
        // Deeper with every sample away from the strip, towards 60 m.
        for pair in row[..22].windows(2) {
            assert!(pair[0] < pair[1] && pair[1] < 0.0, "{row:?}");
        }
        assert!(
            row[21] > -5.0 && row[0] < -45.0 && row[0] > -60.0,
            "{row:?}"
        );
    }

    #[test]
    fn the_coast_distance_of_a_disc_is_its_radius_less_the_distance_to_the_centre() {
        let (n, r) = (129_u32, 40.0_f32);
        let disc = Field2::from_fn(n, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 64.0, y as f32 - 64.0);
            if (dx * dx + dy * dy).sqrt() < r {
                5.0
            } else {
                -5.0
            }
        });
        let serial = TaskPool::new(PoolConfig::with_workers(0));
        let parallel = TaskPool::new(PoolConfig::with_workers(3));
        let d = coast_distance(&disc, 0.0, &serial);
        assert_eq!(coast_distance(&disc, 0.0, &parallel), d);
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 - 64.0, y as f32 - 64.0);
                let expected = (r - (dx * dx + dy * dy).sqrt()) * 10.0;
                let got = d.get(x, y);
                assert!(
                    (got - expected).abs() < 12.0,
                    "({x}, {y}): {got} vs {expected}"
                );
                assert_eq!(got > 0.0, disc.get(x, y) > 0.0);
            }
        }
        // The centre is the farthest inland; the corner the farthest at sea.
        let (lo, hi) = d.min_max();
        assert!((hi - (r * 10.0 - 5.0)).abs() < 12.0, "{hi}");
        assert!(lo < -400.0);
    }
}
