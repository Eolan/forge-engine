//! Stage 3 of the terrain pipeline: uplift against the stream-power law, solved implicitly in
//! the downstream-first order (Braun & Willett 2013, `docs/research/terrain-genesis.md` §1),
//! with hillslope diffusion between the channels. Each step: the uplift rises, the water is
//! routed with the depressions carved through their passes ([`crate::flow::drain`]), every
//! cell lowers towards its receiver by `Δt · K · A^m / Δx` of the difference (the implicit
//! update with `n = 1`, unconditionally stable), then an explicit diffusion sweep smooths the
//! slopes. The height keeps its depressions: a cell below its receiver rises towards it by the
//! same rule, which is sediment settling in a lake, so lakes exist, then fill. A hundred and
//! fifty steps from a flat island give ridges, valleys and a drainage network.
//!
//! The work runs on a [`TaskPool`]: rows in parallel for the uplift, the D8 receivers and
//! the diffusion, the stack's segments (whole drainage trees) in parallel for the incision;
//! the arithmetic per cell is the same in any order, so the result does not depend on the
//! number of threads (D-016).

use std::time::{Duration, Instant};

use forge_task::TaskPool;

use crate::field::Field2;
use crate::flow::{Flow, drain};

/// The erosion's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErosionParams {
    /// Erodibility, per step: how fast a channel of unit drainage area cuts down (the
    /// stream-power `K` with `Δt` folded in).
    pub k: f64,
    /// The area exponent `m` of the stream-power law (0.5 with `n = 1`: a concavity of 0.5).
    pub m: f64,
    /// Hillslope diffusion, m² per step (an explicit sweep is stable below `spacing² / 4`).
    pub diffusion: f64,
    /// Steps to run.
    pub steps: u32,
    /// The sea, metres: cells at or below it are outlets and never rise.
    pub sea_level: f32,
}

impl ErosionParams {
    /// Values that carve a 16 km island in 150 steps: a river with a square kilometre of
    /// catchment lowers by half its drop to the next cell per step (`f = K √A / Δx ≈ 0.5` at
    /// 32 m), a ridge cell by a fiftieth; mountains of several hundred metres with valleys
    /// cut to the coast.
    pub fn island() -> Self {
        Self {
            k: 0.02,
            m: 0.5,
            diffusion: 15.0,
            steps: 150,
            sea_level: 0.0,
        }
    }
}

/// Where a step's time went (`genesis` prints the sum over a run).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepTimings {
    /// Adding the uplift.
    pub uplift: Duration,
    /// Routing the water, depressions included.
    pub drain: Duration,
    /// The implicit stream-power update along the stack.
    pub incise: Duration,
    /// The diffusion sweep.
    pub diffuse: Duration,
}

impl std::ops::AddAssign for StepTimings {
    fn add_assign(&mut self, o: Self) {
        self.uplift += o.uplift;
        self.drain += o.drain;
        self.incise += o.incise;
        self.diffuse += o.diffuse;
    }
}

/// One erosion step over `height` (modified in place): uplift, drain, incise, diffuse.
/// Returns the flow of the step.
pub fn step(
    height: &mut Field2<f32>,
    uplift: &Field2<f32>,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
    pool: &TaskPool,
) -> Flow {
    step_timed(height, uplift, hardness, rain, p, pool).0
}

/// [`step`], with where its time went.
pub fn step_timed(
    height: &mut Field2<f32>,
    uplift: &Field2<f32>,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
    pool: &TaskPool,
) -> (Flow, StepTimings) {
    let n = height.size as usize;
    let mut t = StepTimings::default();
    let start = Instant::now();
    pool.par_chunks_mut(&mut height.data, n, |row, chunk| {
        for (h, &u) in chunk.iter_mut().zip(&uplift.data[row * n..row * n + n]) {
            if *h > p.sea_level || u > 0.0 {
                *h += u;
            }
        }
    });
    t.uplift = start.elapsed();
    // The water is routed with the depressions carved; the height keeps them, and their
    // cells rise towards their receivers above (sediment settling in the lake).
    let start = Instant::now();
    let flow = drain(height, p.sea_level, pool);
    t.drain = start.elapsed();
    let start = Instant::now();
    incise(height, &flow, hardness, rain, p, pool);
    t.incise = start.elapsed();
    let start = Instant::now();
    if p.diffusion > 0.0 {
        diffuse(height, p.diffusion, p.sea_level, pool);
    }
    t.diffuse = start.elapsed();
    (flow, t)
}

/// The implicit stream-power update, the stack's segments (whole trees) on their own tasks:
/// a cell's new height needs its receiver's, an outlet's or one earlier in the segment.
fn incise(
    height: &mut Field2<f32>,
    flow: &Flow,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
    pool: &TaskPool,
) {
    let n = height.size as usize;
    let cell_area = (height.spacing * height.spacing) as f32;
    let power = |drainage: f64| -> f64 {
        if p.m == 0.5 {
            drainage.sqrt()
        } else {
            forge_core::dmath::powf(drainage, p.m)
        }
    };
    let old: &Field2<f32> = height;
    let mut new = vec![0.0_f32; flow.stack.len()];
    flow.par_segments(pool, &mut new, |_, first, slice| {
        for k in 0..slice.len() {
            let i = flow.stack[first + k] as usize;
            let r = flow.receiver[i] as usize;
            let receiver_height = if flow.is_outlet(r) {
                old.data[r]
            } else {
                slice[flow.position[r] as usize - first]
            };
            let drainage =
                power(f64::from(flow.area[i]) * f64::from(cell_area) * f64::from(rain.data[i]));
            let f =
                (p.k * f64::from(hardness.data[i]) * drainage / f64::from(flow.distance[i])) as f32;
            // Implicit: the receiver is already at its new height.
            slice[k] = ((old.data[i] + f * receiver_height) / (1.0 + f)).max(p.sea_level);
        }
    });
    pool.par_chunks_mut(&mut height.data, n, |row, chunk| {
        for (x, h) in chunk.iter_mut().enumerate() {
            let position = flow.position[row * n + x];
            if position != u32::MAX {
                *h = new[position as usize];
            }
        }
    });
}

/// One explicit diffusion sweep: `h += D · ∇²h`, on land, the border left as it is.
fn diffuse(height: &mut Field2<f32>, diffusion: f64, sea_level: f32, pool: &TaskPool) {
    let n = height.size as usize;
    let factor = (diffusion / (height.spacing * height.spacing)).min(0.24) as f32;
    let before = height.data.clone();
    pool.par_chunks_mut(&mut height.data, n, |y, row| {
        if y == 0 || y + 1 == n {
            return;
        }
        for (x, out) in row.iter_mut().enumerate().skip(1).take(n - 2) {
            let i = y * n + x;
            let h = before[i];
            if h <= sea_level {
                continue;
            }
            let laplacian = before[i - 1] + before[i + 1] + before[i - n] + before[i + n] - 4.0 * h;
            *out = (h + factor * laplacian).max(sea_level);
        }
    });
}

/// `p.steps` steps of [`step`]; returns the last step's flow.
pub fn erode(
    height: &mut Field2<f32>,
    uplift: &Field2<f32>,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
    pool: &TaskPool,
) -> Flow {
    let mut flow = drain(height, p.sea_level, pool);
    for _ in 0..p.steps {
        flow = step(height, uplift, hardness, rain, p, pool);
    }
    flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::{IslandParams, island_fields};
    use forge_core::Seed;
    use forge_task::PoolConfig;

    #[test]
    fn uplift_against_erosion_raises_mountains_that_drain_to_the_sea() {
        let params = IslandParams {
            size: 65,
            ..IslandParams::island_16km(Seed::new(3), 256.0)
        };
        let fields = island_fields(&params);
        let mut height = fields.shape.map(|_| 0.0_f32);
        let erosion = ErosionParams {
            steps: 60,
            ..ErosionParams::island()
        };
        let serial = TaskPool::new(PoolConfig::with_workers(0));
        let flow = erode(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion,
            &serial,
        );
        let (lo, hi) = height.min_max();
        assert_eq!(lo, 0.0);
        // The uplift alone would give 60 steps of it at the heart; erosion takes part of it.
        let lifted = 60.0 * params.uplift as f32;
        assert!(hi > 0.2 * lifted && hi < lifted, "{hi} of {lifted}");
        // Every land cell drains to the sea, and the biggest rivers reach the coast.
        let mut best = (0, 0_u32);
        for i in 0..height.len() {
            if height.data[i] > 0.0 {
                let mut c = i;
                while !flow.is_outlet(c) {
                    c = flow.receiver[c] as usize;
                }
                assert!(
                    height.data[c] <= erosion.sea_level
                        || height.on_border(height.coords(c).0, height.coords(c).1)
                );
            }
            if flow.area[i] > best.1 && !flow.is_outlet(i) {
                best = (i, flow.area[i]);
            }
        }
        assert!(best.1 > 20, "the largest river drains {} cells", best.1);
        // Deterministic, and the same with three workers as with none.
        let parallel = TaskPool::new(PoolConfig::with_workers(3));
        let mut again = fields.shape.map(|_| 0.0_f32);
        erode(
            &mut again,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion,
            &parallel,
        );
        assert_eq!(again, height);
    }
}
