//! Stage 5 of the terrain pipeline, its first form: the eroded field amplified ×2 and eroded
//! again at the finer spacing (`docs/research/terrain-genesis.md`, "Recommendation for Forge",
//! stage 5; after Schott et al. 2024, which alternates erosion with ×2 upsampling so that each
//! finer level erodes under the drainage the coarser one fixed).
//!
//! The field is upsampled by Catmull-Rom, given a little fractal detail on the land, then
//! eroded for a few iterations by two explicit operators, both local:
//! - **incision** along the fine grid's own steepest descent (D8), by the stream-power law with
//!   the catchment the coarse field's drainage gives, bilinear: the coarse level fixes where
//!   the water goes, the fine level cuts its channels;
//! - **talus**, the slope above the angle of repose relaxed towards it, symmetric between each
//!   pair of neighbours so that it moves material without losing any.
//!
//! Both read the field as the previous iteration left it (Jacobi), and a sample depends only
//! on its eight neighbours per iteration. So the field is worked in tiles, in parallel, each
//! with a halo of `iterations + 2` samples, and the result is the same to the bit as one
//! untiled run, with any number of workers (D-016): the planet's tiles can be amplified alone,
//! at streaming time, and still agree across their borders.

use forge_core::Seed;
use forge_task::TaskPool;

use crate::field::Field2;
use crate::noise;

/// How the field is amplified.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmplifyParams {
    /// Seed of the detail noise.
    pub seed: Seed,
    /// Metres of detail added to the land before the erosion (a fractal sum of gradient noise).
    pub roughness: f32,
    /// Metres across the detail's largest features.
    pub wavelength: f64,
    /// Iterations of the detail erosion.
    pub iterations: u32,
    /// Stream-power erodibility of one iteration, per metre of catchment to the `area_exponent`
    /// (the metres cut from a sample: `incision · A^m · S`, `A` in m², `S` its slope).
    pub incision: f32,
    /// The exponent `m` of the catchment (0.5, as the coarse erosion's).
    pub area_exponent: f32,
    /// The steepest stable slope, rise over run: steeper ground slides towards it.
    pub talus: f32,
    /// The share of a slope's excess over `talus` moved each iteration (at most 1).
    pub talus_rate: f32,
    /// The share of a sample's difference to the mean of its four axis neighbours removed each
    /// iteration (linear diffusion; 0 for none).
    pub diffusion: f32,
    /// Samples a side of the tiles the fine field is worked in (without their halo).
    pub tile: u32,
}

impl AmplifyParams {
    /// The island's detail at half its spacing: half a metre of roughness over 60 m, twenty
    /// iterations, a talus of 0.9 (42°).
    pub fn island(seed: Seed) -> Self {
        Self {
            seed,
            roughness: 0.5,
            wavelength: 60.0,
            iterations: 20,
            incision: 0.0006,
            area_exponent: 0.5,
            talus: 0.9,
            talus_rate: 0.25,
            diffusion: 0.1,
            tile: 512,
        }
    }
}

/// Neighbour offsets, the axes first, in a fixed order (the D8 tie-break).
const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// `coarse` amplified to half its spacing (`2 (n − 1) + 1` samples a side), `area` its drainage
/// area in cells (`Flow::area`, the catchment the incision reads), the sea at and below
/// `sea_level` left as upsampled (the base level).
pub fn amplify(
    coarse: &Field2<f32>,
    area: &[u32],
    sea_level: f32,
    params: &AmplifyParams,
    pool: &TaskPool,
) -> Field2<f32> {
    assert_eq!(
        area.len(),
        coarse.len(),
        "one drainage area per coarse sample"
    );
    let size = 2 * (coarse.size - 1) + 1;
    let spacing = coarse.spacing * 0.5;
    let tile = params.tile.max(1);
    let tiles = size.div_ceil(tile);
    let halo = params.iterations + 2;
    let mut pieces: Vec<Vec<f32>> = vec![Vec::new(); (tiles * tiles) as usize];
    let n = coarse.size as usize;
    // The heights filtered by [1, 2, 1] / 4 before the cubic: the eroded field carries a faint
    // checkerboard at its sample scale, which Horn's gradient ignores but the cubic turned into
    // a wave of four fine samples, a hatching over the whole hillshade. The sea keeps its
    // samples, so the coast stays where it was.
    let heights: Vec<f32> = binomial(&coarse.data, n)
        .into_iter()
        .zip(&coarse.data)
        .map(|(s, &h)| if h <= sea_level { h } else { s })
        .collect();
    // The catchment in m², filtered twice: a channel of the coarse drainage is one sample wide,
    // and cut at that width the fine field showed cracks rather than valleys.
    let cell = (coarse.spacing * coarse.spacing) as f32;
    let area_m2: Vec<f32> = area.iter().map(|&a| a as f32 * cell).collect();
    let catchment = binomial(&binomial(&area_m2, n), n);
    let context = Context {
        heights: &heights,
        catchment: &catchment,
        coarse_size: coarse.size,
        sea_level,
        params,
        size,
        spacing,
    };
    pool.par_map_into(&mut pieces, 1, |t| {
        let (tx, ty) = (t as u32 % tiles, t as u32 / tiles);
        context.tile(tx * tile, ty * tile, tile, halo)
    });
    // The tiles into the field.
    let mut fine = Field2::new(size, spacing);
    for (t, piece) in pieces.iter().enumerate() {
        let (x0, y0) = ((t as u32 % tiles) * tile, (t as u32 / tiles) * tile);
        let width = tile.min(size - x0) as usize;
        for (row, line) in piece.chunks_exact(width).enumerate() {
            let start = fine.index(x0, y0 + row as u32);
            fine.data[start..start + width].copy_from_slice(line);
        }
    }
    fine
}

/// `data` (`n × n`, row-major) filtered by [1, 2, 1] / 4 along the rows, then the columns, the
/// border samples clamped.
fn binomial(data: &[f32], n: usize) -> Vec<f32> {
    let pass = |src: &[f32], step: usize, stride: usize| -> Vec<f32> {
        let mut out = vec![0.0_f32; n * n];
        for line in 0..n {
            for k in 0..n {
                let at = |k: usize| src[line * stride + k * step];
                let (lo, hi) = (k.saturating_sub(1), (k + 1).min(n - 1));
                out[line * stride + k * step] = 0.25 * at(lo) + 0.5 * at(k) + 0.25 * at(hi);
            }
        }
        out
    };
    pass(&pass(data, 1, n), n, 1)
}

/// What every tile reads.
struct Context<'a> {
    /// The coarse heights, filtered (`binomial`), `coarse_size` a side.
    heights: &'a [f32],
    /// The coarse catchment, m², filtered twice.
    catchment: &'a [f32],
    coarse_size: u32,
    sea_level: f32,
    params: &'a AmplifyParams,
    /// Samples a side of the fine field, and their spacing.
    size: u32,
    spacing: f64,
}

impl Context<'_> {
    /// The coarse field by Catmull-Rom at fine sample `(x, y)` (half coarse samples).
    fn upsampled(&self, x: u32, y: u32) -> f32 {
        let n = self.coarse_size;
        let last = n as i64 - 1;
        let at = |i: i64, j: i64| {
            self.heights[(j.clamp(0, last) as u32 * n + i.clamp(0, last) as u32) as usize]
        };
        // On a coarse sample, the sample itself; half-way, the cubic's midpoint weights.
        let weights = |v: u32| -> ([i64; 4], [f32; 4]) {
            let base = i64::from(v / 2);
            if v.is_multiple_of(2) {
                ([base - 1, base, base + 1, base + 2], [0.0, 1.0, 0.0, 0.0])
            } else {
                (
                    [base - 1, base, base + 1, base + 2],
                    [-0.0625, 0.5625, 0.5625, -0.0625],
                )
            }
        };
        let ((xi, xw), (yi, yw)) = (weights(x), weights(y));
        let mut sum = 0.0;
        for (j, wy) in yi.iter().zip(yw) {
            if wy == 0.0 {
                continue;
            }
            let mut row = 0.0;
            for (i, wx) in xi.iter().zip(xw) {
                if wx != 0.0 {
                    row += wx * at(*i, *j);
                }
            }
            sum += wy * row;
        }
        sum
    }

    /// The coarse catchment at fine sample `(x, y)`, m², bilinear.
    fn catchment(&self, x: u32, y: u32) -> f32 {
        let n = self.coarse_size;
        let at = |i: u32, j: u32| self.catchment[(j.min(n - 1) * n + i.min(n - 1)) as usize];
        let (i, j) = (x / 2, y / 2);
        let (fx, fy) = ((x % 2) as f32 * 0.5, (y % 2) as f32 * 0.5);
        let top = at(i, j) + (at(i + 1, j) - at(i, j)) * fx;
        let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * fx;
        top + (bottom - top) * fy
    }

    /// The fine samples of the tile at `(x0, y0)`, `tile` a side (clipped to the field), row by
    /// row, after the erosion worked a region `halo` samples wider.
    fn tile(&self, x0: u32, y0: u32, tile: u32, halo: u32) -> Vec<f32> {
        let p = self.params;
        let (rx0, ry0) = (x0.saturating_sub(halo), y0.saturating_sub(halo));
        let (rx1, ry1) = (
            (x0 + tile + halo).min(self.size),
            (y0 + tile + halo).min(self.size),
        );
        let (w, h) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
        let seed = p.seed.derive_str("amplify detail").rng().next_u64();
        let mut height = vec![0.0_f32; w * h];
        let mut fixed = vec![false; w * h];
        let mut catchment = vec![0.0_f32; w * h];
        for ry in 0..h {
            for rx in 0..w {
                let (x, y) = (rx0 + rx as u32, ry0 + ry as u32);
                let i = ry * w + rx;
                let up = self.upsampled(x, y);
                // The sea and the field's border stay as upsampled: the base level.
                fixed[i] = up <= self.sea_level
                    || x == 0
                    || y == 0
                    || x == self.size - 1
                    || y == self.size - 1;
                height[i] = if fixed[i] {
                    up
                } else {
                    let (mx, my) = (f64::from(x) * self.spacing, f64::from(y) * self.spacing);
                    let detail =
                        noise::fbm(seed, mx / p.wavelength, my / p.wavelength, 4, 2.0, 0.5) as f32;
                    (up + p.roughness * detail).max(self.sea_level + 0.01)
                };
                catchment[i] = self.catchment(x, y).max(1.0).powf(p.area_exponent);
            }
        }
        let spacing = self.spacing as f32;
        let distance = |dx: i32, dy: i32| {
            if dx != 0 && dy != 0 {
                spacing * std::f32::consts::SQRT_2
            } else {
                spacing
            }
        };
        let mut next = height.clone();
        for _ in 0..p.iterations {
            // The region's own border keeps its value: its error reaches one sample further in
            // each iteration, and the halo is wider than the iterations.
            for ry in 1..h - 1 {
                for rx in 1..w - 1 {
                    let i = ry * w + rx;
                    if fixed[i] {
                        continue;
                    }
                    let here = height[i];
                    let (mut steepest, mut drop_to) = (0.0_f32, here);
                    let mut change = 0.0_f32;
                    for &(dx, dy) in &NEIGHBOURS {
                        let j = (ry as i32 + dy) as usize * w + (rx as i32 + dx) as usize;
                        let there = height[j];
                        let d = distance(dx, dy);
                        let slope = (here - there) / d;
                        if slope > steepest {
                            steepest = slope;
                            drop_to = there;
                        }
                        // Talus, both ways: what slides out of here, and what slides in.
                        let limit = p.talus * d;
                        let rate = p.talus_rate / 8.0;
                        change -= rate * (here - there - limit).max(0.0);
                        change += rate * (there - here - limit).max(0.0);
                    }
                    // Linear diffusion over the four axis neighbours: it rounds what the
                    // steepest descent leaves square.
                    let axes =
                        (height[i + 1] + height[i - 1] + height[i + w] + height[i - w]) * 0.25;
                    change += p.diffusion * (axes - here);
                    // Incision towards the steepest neighbour, never below half the drop.
                    let cut = (p.incision * catchment[i] * steepest).min(0.5 * (here - drop_to));
                    next[i] = here + change - cut;
                }
            }
            std::mem::swap(&mut height, &mut next);
            next.copy_from_slice(&height);
        }
        // The tile's own samples.
        let (tw, th) = (
            (tile.min(self.size - x0)) as usize,
            (tile.min(self.size - y0)) as usize,
        );
        let mut out = Vec::with_capacity(tw * th);
        for y in 0..th {
            let start = (y0 as usize - ry0 as usize + y) * w + (x0 - rx0) as usize;
            out.extend_from_slice(&height[start..start + tw]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_task::PoolConfig;

    /// A cone rising from a sea to 120 m, 8 m samples, with a drainage area growing downhill.
    fn cone() -> (Field2<f32>, Vec<u32>) {
        let n = 65_u32;
        let field = Field2::from_fn(n, 8.0, |x, y| {
            let (dx, dy) = (x as f32 - 32.0, y as f32 - 32.0);
            (120.0 - 5.0 * (dx * dx + dy * dy).sqrt()).max(0.0)
        });
        let area = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f32 - 32.0, (i / n) as f32 - 32.0);
                1 + (x * x + y * y).sqrt() as u32 * 3
            })
            .collect();
        (field, area)
    }

    #[test]
    fn the_tiles_give_the_untiled_field_to_the_bit_with_any_workers() {
        let (field, area) = cone();
        let serial = TaskPool::new(PoolConfig::with_workers(0));
        let parallel = TaskPool::new(PoolConfig::with_workers(3));
        let params = AmplifyParams {
            iterations: 6,
            tile: 1000,
            ..AmplifyParams::island(Seed::new(7))
        };
        let whole = amplify(&field, &area, 0.0, &params, &serial);
        assert_eq!(whole.size, 129);
        assert_eq!(whole.spacing, 4.0);
        let tiled = AmplifyParams { tile: 16, ..params };
        assert_eq!(amplify(&field, &area, 0.0, &tiled, &serial), whole);
        assert_eq!(amplify(&field, &area, 0.0, &tiled, &parallel), whole);
    }

    #[test]
    fn the_sea_stays_and_the_land_keeps_its_shape() {
        let (field, area) = cone();
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let params = AmplifyParams::island(Seed::new(7));
        let fine = amplify(&field, &area, 0.0, &params, &pool);
        // The coarse samples come back within a few metres, but at the cone's sharp apex, which
        // the filter and the diffusion round off; the sea exactly.
        for y in (0..fine.size).step_by(2) {
            for x in (0..fine.size).step_by(2) {
                let (c, f) = (field.get(x / 2, y / 2), fine.get(x, y));
                if c <= 0.0 {
                    assert_eq!(f, c, "the sea at ({x}, {y})");
                } else {
                    let apex = (x as i32 - 64).abs().max((y as i32 - 64).abs()) <= 6;
                    assert!(apex || (f - c).abs() < 4.0, "({x}, {y}): {f} against {c}");
                }
            }
        }
        // Upsampled without the erosion, a coarse sample comes back but for the filter.
        let still = AmplifyParams {
            roughness: 0.0,
            iterations: 0,
            ..params
        };
        let plain = amplify(&field, &area, 0.0, &still, &pool);
        assert!((plain.get(64, 40) - field.get(32, 20)).abs() < 0.5);
    }

    #[test]
    fn talus_lays_a_cliff_down_towards_the_angle_of_repose() {
        // A 60 m cliff on 4 m fine samples, no incision, no detail.
        let n = 33_u32;
        let field = Field2::from_fn(n, 8.0, |x, _| if x < 16 { 80.0 } else { 20.0 });
        let area = vec![1_u32; (n * n) as usize];
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let params = AmplifyParams {
            roughness: 0.0,
            incision: 0.0,
            iterations: 200,
            talus: 0.5,
            talus_rate: 0.5,
            ..AmplifyParams::island(Seed::new(1))
        };
        let steepest = |f: &Field2<f32>| {
            (1..f.size - 1)
                .map(|x| (f.get(x, f.size / 2) - f.get(x + 1, f.size / 2)).abs() / f.spacing as f32)
                .fold(0.0_f32, f32::max)
        };
        let before = amplify(
            &field,
            &area,
            0.0,
            &AmplifyParams {
                iterations: 0,
                ..params
            },
            &pool,
        );
        let after = amplify(&field, &area, 0.0, &params, &pool);
        assert!(steepest(&before) > 2.0);
        assert!(steepest(&after) < 1.0, "{}", steepest(&after));
        // Talus moves the material without losing it (the border rows aside).
        let sum = |f: &Field2<f32>| f.data.iter().map(|&h| f64::from(h)).sum::<f64>();
        assert!((sum(&after) - sum(&before)).abs() / sum(&before) < 1e-3);
    }
}
