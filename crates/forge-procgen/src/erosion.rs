//! Stage 3 of the terrain pipeline: uplift against the stream-power law, solved implicitly in
//! the downstream-first order (Braun & Willett 2013, `docs/research/terrain-genesis.md` §1),
//! with hillslope diffusion between the channels. Each step: the uplift rises, depressions are
//! flooded and the water routed ([`crate::flow`]), every cell lowers towards its receiver by
//! `Δt · K · A^m / Δx` of the difference (the implicit update with `n = 1`, unconditionally
//! stable), then an explicit diffusion sweep smooths the slopes. The water is routed over the
//! flooded field but the height keeps its depressions: a cell below its receiver rises towards
//! it by the same rule, which is sediment settling in a lake, so lakes exist, then fill. A
//! hundred and fifty steps from a flat island give ridges, valleys and a drainage network.

use crate::field::Field2;
use crate::flow::{Flow, priority_flood, route};

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

/// One erosion step over `height` (modified in place): uplift, flood, route, incise, diffuse.
/// Returns the flow of the step.
pub fn step(
    height: &mut Field2<f32>,
    uplift: &Field2<f32>,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
) -> Flow {
    for (h, &u) in height.data.iter_mut().zip(&uplift.data) {
        if *h > p.sea_level || u > 0.0 {
            *h += u;
        }
    }
    // The water is routed over the flooded field; the height keeps its depressions, whose
    // cells rise towards their receivers below (sediment settling in the lake).
    let filled = priority_flood(height, p.sea_level);
    let flow = route(&filled, p.sea_level);
    let cell_area = (height.spacing * height.spacing) as f32;
    for &c in &flow.stack {
        let i = c as usize;
        if flow.is_outlet(i) {
            continue;
        }
        let r = flow.receiver[i] as usize;
        let drainage =
            (f64::from(flow.area[i]) * f64::from(cell_area) * f64::from(rain.data[i])).powf(p.m);
        let f = (p.k * f64::from(hardness.data[i]) * drainage / f64::from(flow.distance[i])) as f32;
        // Implicit: the receiver is already at its new height.
        height.data[i] = (height.data[i] + f * height.data[r]) / (1.0 + f);
        if height.data[i] < p.sea_level {
            height.data[i] = p.sea_level;
        }
    }
    if p.diffusion > 0.0 {
        diffuse(height, p.diffusion, p.sea_level);
    }
    flow
}

/// One explicit diffusion sweep: `h += D · ∇²h`, on land, the border left as it is.
fn diffuse(height: &mut Field2<f32>, diffusion: f64, sea_level: f32) {
    let n = height.size;
    let factor = (diffusion / (height.spacing * height.spacing)).min(0.24) as f32;
    let before = height.data.clone();
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            let i = height.index(x, y);
            let h = before[i];
            if h <= sea_level {
                continue;
            }
            let laplacian =
                before[i - 1] + before[i + 1] + before[i - n as usize] + before[i + n as usize]
                    - 4.0 * h;
            height.data[i] = (h + factor * laplacian).max(sea_level);
        }
    }
}

/// `p.steps` steps of [`step`]; returns the last step's flow.
pub fn erode(
    height: &mut Field2<f32>,
    uplift: &Field2<f32>,
    hardness: &Field2<f32>,
    rain: &Field2<f32>,
    p: &ErosionParams,
) -> Flow {
    let mut flow = route(&priority_flood(height, p.sea_level), p.sea_level);
    for _ in 0..p.steps {
        flow = step(height, uplift, hardness, rain, p);
    }
    flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::{IslandParams, island_fields};
    use forge_core::Seed;

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
        let flow = erode(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion,
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
        // Deterministic.
        let mut again = fields.shape.map(|_| 0.0_f32);
        erode(
            &mut again,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion,
        );
        assert_eq!(again, height);
    }
}
