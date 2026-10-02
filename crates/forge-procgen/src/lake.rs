//! The lakes' water (issue #105, D-038's lakes; `docs/research/water.md` §4 and its
//! recommendation's step 4): each lake of stage 4 ([`crate::hydrology::trace_lakes`]) as a
//! level plane the GPU draws (`water.slang`), clipped by a mask of the samples its water may
//! stand over: the priority flood's depression at the lake's level, grown by a sample where the
//! ground rises through the level, so it draws the shore between the samples. Past the outlet,
//! where the ground falls away under the level, the mask stops and the river takes the water on
//! (#120). The rivers through a lake take its level ([`crate::river`]).
//!
//! Everything is a pure function of the fields, in index order (D-016).

use std::collections::{HashSet, VecDeque};

use crate::field::Field2;
use crate::hydrology::Lakes;
use crate::river::{Outlet, Ribbon};

/// How far the flood may stand from a lake's level and still be the lake's depression, metres
/// (the priority flood's ε steps across a flat).
const LEVEL_TOLERANCE: f32 = 0.05;

/// A lake's water.
#[derive(Clone, Debug, PartialEq)]
pub struct LakeWater {
    /// The lake's index in [`Lakes::lakes`].
    pub lake: u32,
    /// The water's level, metres.
    pub level: f32,
    /// Its deepest point under the level, metres.
    pub depth: f32,
    /// The sample the water leaves by, column and row of the field ([`crate::Lake::outlet`]).
    pub outlet: [u32; 2],
    /// The mask's first sample, column and row of the field.
    pub first: [u32; 2],
    /// The mask's samples along the columns and the rows.
    pub size: [u32; 2],
    /// Per sample of the mask, row-major: whether the lake's water may stand there.
    pub mask: Vec<bool>,
}

impl LakeWater {
    /// Whether the water may stand at sample `(x, y)` of the field.
    pub fn covers(&self, x: u32, y: u32) -> bool {
        let (i, j) = (x.wrapping_sub(self.first[0]), y.wrapping_sub(self.first[1]));
        i < self.size[0] && j < self.size[1] && self.mask[(j * self.size[0] + i) as usize]
    }

    /// Whether the water stands over sample `(x, y)` of `height`, the field it was traced over:
    /// the mask covers it and the ground there is under the level.
    pub fn stands_at(&self, height: &Field2<f32>, x: u32, y: u32) -> bool {
        self.covers(x, y) && height.get(x, y) < self.level
    }

    /// The samples of the mask, in its layout, that are the shallow arm `outlet` carries the
    /// lake's water out along (#120): more than [`ARM_KEEP`] metres past it down the river, within
    /// [`ARM_SIDE`] metres of the river's water either side, the lake's water under [`ARM_DEEP`]
    /// metres deep over them. A flat valley floor at the lake's level past its outlet floods into
    /// such an arm, centimetres of water round the river.
    pub fn arm(&self, height: &Field2<f32>, outlet: &Outlet) -> Vec<bool> {
        (0..self.mask.len())
            .map(|k| {
                let (i, j) = (k as u32 % self.size[0], k as u32 / self.size[0]);
                self.mask[k] && self.in_arm(height, outlet, self.first[0] + i, self.first[1] + j)
            })
            .collect()
    }

    /// Whether sample `(x, y)` of `height` lies where `outlet`'s shallow arm would be
    /// ([`LakeWater::arm`]), whether the mask covers it or not.
    pub fn in_arm(&self, height: &Field2<f32>, outlet: &Outlet, x: u32, y: u32) -> bool {
        let spacing = height.spacing;
        let (dx, dy) = (
            f64::from(x) * spacing - outlet.at[0],
            f64::from(y) * spacing - outlet.at[1],
        );
        let along = dx * outlet.down[0] + dy * outlet.down[1];
        let across = (dx * outlet.down[1] - dy * outlet.down[0]).abs();
        along > ARM_KEEP
            && across <= outlet.half_width + ARM_SIDE
            && f64::from(self.level - height.get(x, y)) < ARM_DEEP
    }
}

/// Metres past an outlet down the river from which the lake's shallow arm is the river's
/// ([`LakeWater::arm`]): the river's water is whole from a point past the outlet, and the lake's
/// fades out over a sample from here.
pub const ARM_KEEP: f64 = 8.0;

/// Metres either side of the river's water an outlet's arm reaches.
pub const ARM_SIDE: f64 = 40.0;

/// The deepest an outlet's arm is, metres: deeper water past the outlet is the lake's still.
pub const ARM_DEEP: f64 = 0.75;

/// Trims each lake's water off the shallow arms its outlets carry it out along
/// ([`LakeWater::arm`], #120), past the outlets of `ribbons` (made with `lakes`); the ground
/// there rises over the lake's level ([`crate::Channels`]). Returns the samples trimmed.
pub fn trim_outlets(lakes: &mut [LakeWater], height: &Field2<f32>, ribbons: &[Ribbon]) -> usize {
    let mut trimmed = 0;
    for outlet in ribbons.iter().flat_map(|r| r.outlets.iter()) {
        let lake = &mut lakes[outlet.lake as usize];
        let arm = lake.arm(height, outlet);
        for (m, a) in lake.mask.iter_mut().zip(arm) {
            if a && *m {
                *m = false;
                trimmed += 1;
            }
        }
    }
    trimmed
}

/// The water of every lake of `lakes` with `min_area` m² or more, traced over `height` from its
/// priority flood `filled`: from the lake's samples, every 4-neighbour where the flood stands
/// at the lake's level (its shallow margins, which the lakes' depth threshold leaves out, and
/// the rim), then a sample more all round where the ground stands at the level or over it (the
/// shore), not past the outlet, where it falls away under it.
pub fn lake_waters(
    height: &Field2<f32>,
    filled: &Field2<f32>,
    lakes: &Lakes,
    min_area: f64,
) -> Vec<LakeWater> {
    let n = height.size as usize;
    lakes
        .lakes
        .iter()
        .enumerate()
        .filter(|(_, lake)| lake.area(height.spacing) >= min_area)
        .map(|(index, lake)| {
            let level = lake.level;
            let at_level = |i: usize| (filled.data[i] - level).abs() <= LEVEL_TOLERANCE;
            let mut region: HashSet<usize> = lake.cells.iter().map(|&c| c as usize).collect();
            let mut queue: VecDeque<usize> = lake.cells.iter().map(|&c| c as usize).collect();
            while let Some(i) = queue.pop_front() {
                let (x, y) = (i % n, i / n);
                let neighbours = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < n).then(|| i + 1),
                    (y > 0).then(|| i - n),
                    (y + 1 < n).then(|| i + n),
                ];
                for j in neighbours.into_iter().flatten() {
                    if !region.contains(&j) && at_level(j) {
                        region.insert(j);
                        queue.push_back(j);
                    }
                }
            }
            // The box of the region and a sample more all round, clamped to the field.
            let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
            for &i in &region {
                let (x, y) = (i % n, i / n);
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
            let lo = [lo[0].saturating_sub(1), lo[1].saturating_sub(1)];
            let hi = [(hi[0] + 1).min(n - 1), (hi[1] + 1).min(n - 1)];
            let size = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1];
            let mut mask = vec![false; size[0] * size[1]];
            for &i in &region {
                let (x, y) = (i % n, i / n);
                for dy in 0..3 {
                    for dx in 0..3 {
                        let (mx, my) = ((x + dx).wrapping_sub(1), (y + dy).wrapping_sub(1));
                        if !(lo[0]..=hi[0]).contains(&mx) || !(lo[1]..=hi[1]).contains(&my) {
                            continue;
                        }
                        let m = my * n + mx;
                        if region.contains(&m) || height.data[m] >= level {
                            mask[(my - lo[1]) * size[0] + (mx - lo[0])] = true;
                        }
                    }
                }
            }
            LakeWater {
                lake: index as u32,
                level,
                depth: lake.depth,
                outlet: {
                    let (x, y) = height.coords(lake.outlet as usize);
                    [x, y]
                },
                first: [lo[0] as u32, lo[1] as u32],
                size: [size[0] as u32, size[1] as u32],
                mask,
            }
        })
        .collect()
}

/// Paints the lakes' beds into `layers` (over the same square as `height`) as `layer`: every
/// texel whose nearest sample a lake's water covers and whose `ground` (x, y in the field's
/// frame: the ground as drawn) stands under its level. Returns the texels painted.
pub fn paint_lake_beds(
    layers: &mut Field2<u8>,
    height: &Field2<f32>,
    ground: &dyn Fn(f64, f64) -> f64,
    lakes: &[LakeWater],
    layer: u8,
) -> usize {
    let mut painted = 0;
    let last = f64::from(height.size - 1);
    for ty in 0..layers.size {
        for tx in 0..layers.size {
            let (x, y) = (
                (f64::from(tx) + 0.5) * layers.spacing,
                (f64::from(ty) + 0.5) * layers.spacing,
            );
            let (sx, sy) = (
                (x / height.spacing).round().clamp(0.0, last) as u32,
                (y / height.spacing).round().clamp(0.0, last) as u32,
            );
            let under = lakes
                .iter()
                .any(|l| l.covers(sx, sy) && ground(x, y) < f64::from(l.level));
            let texel = &mut layers.data[(ty * layers.size + tx) as usize];
            if under && *texel != layer {
                *texel = layer;
                painted += 1;
            }
        }
    }
    painted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{drain, priority_flood};
    use crate::hydrology::trace_lakes;
    use forge_task::{PoolConfig, TaskPool};

    #[test]
    fn a_basin_s_water_covers_its_depression_and_its_shore_but_not_past_its_outlet() {
        // A bowl 2 m deep in a plane falling towards x = 0 (the outlet): its lip at x = 13.
        let bowl = Field2::from_fn(24, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 12.0, y as f32 - 12.0);
            let r = (dx * dx + dy * dy).sqrt();
            0.1 * x as f32 + 10.0 - 2.0 * (1.0 - (r / 4.0).min(1.0))
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&bowl, 0.0, &pool);
        let filled = priority_flood(&bowl, 0.0);
        let lakes = trace_lakes(&bowl, &filled, &flow, 0.5);
        let waters = lake_waters(&bowl, &filled, &lakes, 0.0);
        assert_eq!(waters.len(), lakes.lakes.len());
        assert!(!waters.is_empty());
        let water = &waters[0];
        let lake = &lakes.lakes[water.lake as usize];
        // Every sample of the lake is covered, the bowl's centre too, and the shore a sample past
        // the water, where the ground rises through the level. The plane's far side and the
        // outlet's slope are not, not even the sample past the lip (#120): the lip at (8, 12)
        // stands at the level, (7, 12) under it.
        for &c in &lake.cells {
            let (x, y) = bowl.coords(c as usize);
            assert!(water.covers(x, y) && water.stands_at(&bowl, x, y));
        }
        assert!(water.covers(12, 12));
        assert!(water.covers(12, 16) && !water.stands_at(&bowl, 12, 16));
        assert!(water.covers(8, 12) && !water.covers(7, 12));
        assert!(!water.covers(23, 12) && !water.covers(0, 12));
        // Every covered sample is within a sample of one where the flood stands at the level.
        for y in 0..24 {
            for x in 0..24 {
                if water.covers(x, y) {
                    let near = (-1i32..=1).any(|dy| {
                        (-1i32..=1).any(|dx| {
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            (0..24).contains(&nx)
                                && (0..24).contains(&ny)
                                && (filled.get(nx as u32, ny as u32) - water.level).abs()
                                    <= LEVEL_TOLERANCE
                        })
                    });
                    assert!(near, "({x}, {y})");
                }
            }
        }
    }
}
