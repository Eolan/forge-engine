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

#[cfg(test)]
mod tests {
    use super::*;
    use forge_task::PoolConfig;

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
