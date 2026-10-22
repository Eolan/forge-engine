//! Which cells a viewer needs, and the loads and unloads that follow the viewer. A plan lists,
//! for every level from the finest to the coarsest, the cells within that level's reach of the
//! viewer (`rings` cells of that level), so the fine levels form a small disc around the viewer
//! and each coarser level a wider one: the clipmap of cells of World Partition and its HLOD
//! proxies (`docs/research/large-worlds.md` §5). Every level keeps its cells over the whole
//! disc rather than an annulus: whoever draws picks the finest resident cell over a point, so a
//! coarse proxy stands in until its fine cells arrive and nothing ever shows a hole.
//! [`Residency`] turns successive plans into loads, nearest and finest first, and unloads with
//! hysteresis, so a viewer on a cell's border does not thrash it.
//!
//! **Proxies** (D-077): with [`StreamPolicy::proxy_rings`] past `rings`, a level's cells beyond
//! its reach and within the proxies' are wanted as their proxy ([`Detail::Proxy`]), one merged
//! mesh each: a cell swaps between its content and its proxy as the viewer comes and goes, and
//! its content stays until past the hysteresis, as an unload would.
//!
//! The cells within reach are found by sampling the disc at half a cell's spacing and asking
//! the partition for each sample's cell, which crosses the cube sphere's face borders without
//! a neighbour table: a cell with its centre inside the reach holds at least one sample.

use std::collections::{BTreeMap, HashMap};

use glam::DVec3;

use crate::cell_id::CellId;
use crate::partition::Partition;

/// How much of a cell is in (D-077).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Detail {
    /// Its content, within its level's reach.
    Full,
    /// What stands in for it beyond its level's reach, within the proxies'.
    Proxy,
}

/// How far a viewer's interest reaches at each level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamPolicy {
    /// The finest level wanted around the viewer.
    pub finest: u8,
    /// The coarsest level wanted (loaded to its reach too).
    pub coarsest: u8,
    /// A level's reach around the viewer, in cells of that level (2.5: the viewer's cell and
    /// two rings, about twenty cells a level).
    pub rings: f64,
    /// A loaded cell is kept until its centre lies beyond `(1 + hysteresis)` times the
    /// farthest a wanted centre of its level can be (the reach plus half a cell's diagonal).
    pub hysteresis: f64,
    /// The proxies' reach, in cells of each level (D-077): a cell beyond `rings` and within this
    /// is wanted as its proxy. At `rings` or under, there are no proxies.
    pub proxy_rings: f64,
}

impl Default for StreamPolicy {
    fn default() -> Self {
        Self {
            finest: 12,
            coarsest: 4,
            rings: 2.5,
            hysteresis: 0.25,
            proxy_rings: 0.0,
        }
    }
}

impl StreamPolicy {
    /// A level's reach, metres.
    pub fn reach<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        self.rings * partition.cell_size(level)
    }

    /// Whether the cells past the reach are wanted as proxies.
    pub fn proxies(&self) -> bool {
        self.proxy_rings > self.rings
    }

    /// A level's proxies' reach, metres: its reach when there are no proxies.
    pub fn proxy_reach<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        self.rings.max(self.proxy_rings) * partition.cell_size(level)
    }

    /// How far a loaded cell's centre may lie before it unloads, metres: the farthest a wanted
    /// centre can be (a sample on the reach's edge lands in a cell whose centre is up to half a
    /// diagonal beyond), times `1 + hysteresis`. With proxies, before its content gives way to
    /// its proxy.
    pub fn keep_within<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        self.keep(partition, level, self.reach(partition, level))
    }

    /// As [`Self::keep_within`], for a cell's proxy.
    pub fn keep_proxy_within<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        self.keep(partition, level, self.proxy_reach(partition, level))
    }

    fn keep<P: Partition>(&self, partition: &P, level: u8, reach: f64) -> f64 {
        let size = partition.cell_size(level);
        (reach + size * std::f64::consts::FRAC_1_SQRT_2) * (1.0 + self.hysteresis)
    }
}

/// A cell a viewer wants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellWant {
    /// The cell.
    pub cell: CellId,
    /// From the viewer to the cell's centre, metres.
    pub distance: f64,
    /// Lower first: the distance in cells of its level, so the near, fine cells lead and the
    /// far, coarse ones follow.
    pub priority: f64,
    /// Its content or its proxy.
    pub detail: Detail,
}

/// The cells a viewer wants, every level, sorted by priority.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamPlan {
    /// The wanted cells, best first.
    pub wants: Vec<CellWant>,
}

impl StreamPlan {
    /// The plan around `viewer` (metres in the partition's frame) under `policy`.
    pub fn around<P: Partition>(partition: &P, viewer: DVec3, policy: &StreamPolicy) -> Self {
        assert!(
            policy.finest >= policy.coarsest,
            "finest is the deeper level"
        );
        let mut wants = Vec::new();
        for level in policy.coarsest..=policy.finest {
            let size = partition.cell_size(level);
            let reach = policy.reach(partition, level);
            let outer = policy.proxy_reach(partition, level);
            let spacing = 0.5 * size;
            let steps = (outer / spacing).ceil() as i32;
            // Each cell's nearest sample: within the reach its content, past it its proxy.
            let mut cells: BTreeMap<CellId, f64> = BTreeMap::new();
            for j in -steps..=steps {
                for i in -steps..=steps {
                    let east = f64::from(i) * spacing;
                    let north = f64::from(j) * spacing;
                    let out = east * east + north * north;
                    if out > outer * outer {
                        continue;
                    }
                    let cell = partition.cell_of(partition.step(viewer, east, north), level);
                    let nearest = cells.entry(cell).or_insert(f64::INFINITY);
                    *nearest = nearest.min(out);
                }
            }
            for (cell, nearest) in cells {
                let distance = (partition.cell_center(cell) - viewer).length();
                wants.push(CellWant {
                    cell,
                    distance,
                    priority: distance / size,
                    detail: if nearest <= reach * reach {
                        Detail::Full
                    } else {
                        Detail::Proxy
                    },
                });
            }
        }
        wants.sort_by(|a, b| {
            a.priority
                .total_cmp(&b.priority)
                .then_with(|| a.cell.cmp(&b.cell))
        });
        Self { wants }
    }

    /// The wanted cells of `level`.
    pub fn at_level(&self, level: u8) -> impl Iterator<Item = &CellWant> {
        self.wants.iter().filter(move |w| w.cell.level() == level)
    }

    /// Whether `cell` is wanted.
    pub fn wants(&self, cell: CellId) -> bool {
        self.wants.iter().any(|w| w.cell == cell)
    }

    /// How `cell` is wanted, if it is.
    pub fn detail(&self, cell: CellId) -> Option<Detail> {
        self.wants.iter().find(|w| w.cell == cell).map(|w| w.detail)
    }
}

/// What a plan asks of the loaded set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// Cells to bring in, best first, each with the detail it takes; a cell already in at the
    /// other detail swaps to it (D-077).
    pub load: Vec<(CellId, Detail)>,
    /// Cells to unload, in id order.
    pub unload: Vec<CellId>,
}

/// The loaded cells, following successive plans with hysteresis.
#[derive(Clone, Debug, Default)]
pub struct Residency {
    loaded: BTreeMap<CellId, Detail>,
}

impl Residency {
    /// Nothing loaded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `plan` for the viewer at `viewer`: the wanted cells not yet loaded are to load,
    /// a proxy wanted as its content swaps to it at once, and a loaded cell wanted no more as it
    /// is gives way once it lies beyond [`StreamPolicy::keep_within`] (its content to its proxy
    /// where proxies are wanted) or [`StreamPolicy::keep_proxy_within`] (a proxy, to nothing).
    /// Both are applied to the set at once: the caller streams them in its own time.
    pub fn update<P: Partition>(
        &mut self,
        partition: &P,
        viewer: DVec3,
        plan: &StreamPlan,
        policy: &StreamPolicy,
    ) -> Changes {
        let mut changes = Changes::default();
        let wanted: HashMap<CellId, Detail> =
            plan.wants.iter().map(|w| (w.cell, w.detail)).collect();
        for want in &plan.wants {
            match self.loaded.get(&want.cell) {
                None => changes.load.push((want.cell, want.detail)),
                Some(Detail::Proxy) if want.detail == Detail::Full => {
                    changes.load.push((want.cell, Detail::Full));
                }
                Some(_) => {}
            }
        }
        for (&cell, &detail) in &self.loaded {
            let level = cell.level();
            if level > policy.finest || level < policy.coarsest {
                changes.unload.push(cell);
                continue;
            }
            let want = wanted.get(&cell).copied();
            if want == Some(Detail::Full) || want == Some(detail) {
                continue;
            }
            let distance = (partition.cell_center(cell) - viewer).length();
            let near_proxy = want == Some(Detail::Proxy)
                || distance <= policy.keep_proxy_within(partition, level);
            match detail {
                Detail::Full if distance <= policy.keep_within(partition, level) => {}
                Detail::Full if policy.proxies() && near_proxy => {
                    changes.load.push((cell, Detail::Proxy));
                }
                Detail::Proxy if near_proxy => {}
                _ => changes.unload.push(cell),
            }
        }
        for &(cell, detail) in &changes.load {
            self.loaded.insert(cell, detail);
        }
        for cell in &changes.unload {
            self.loaded.remove(cell);
        }
        changes
    }

    /// Whether `cell` is loaded, at either detail.
    pub fn is_loaded(&self, cell: CellId) -> bool {
        self.loaded.contains_key(&cell)
    }

    /// How `cell` is loaded, if it is.
    pub fn detail(&self, cell: CellId) -> Option<Detail> {
        self.loaded.get(&cell).copied()
    }

    /// How many cells are loaded.
    pub fn len(&self) -> usize {
        self.loaded.len()
    }

    /// Whether nothing is loaded.
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// The loaded cells, in id order.
    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.loaded.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::{CubeSphere, Face, FlatGrid};

    fn policy() -> StreamPolicy {
        StreamPolicy {
            finest: 6,
            coarsest: 3,
            rings: 2.5,
            hysteresis: 0.25,
            proxy_rings: 0.0,
        }
    }

    #[test]
    fn a_plan_holds_the_viewer_at_every_level_within_a_bounded_count() {
        let grid = FlatGrid {
            root_size: 65_536.0,
        };
        let viewer = DVec3::new(1234.5, 10.0, -987.0);
        let plan = StreamPlan::around(&grid, viewer, &policy());
        for level in 3..=6 {
            let own = grid.cell_of(viewer, level);
            assert!(plan.wants(own), "level {level}");
            let count = plan.at_level(level).count();
            // A disc of 2.5 cells holds about 20 cells; never more than the square around it.
            assert!((13..=36).contains(&count), "level {level}: {count} cells");
            // Every wanted cell's centre lies within the reach plus half a diagonal.
            let size = grid.cell_size(level);
            for want in plan.at_level(level) {
                assert!(
                    want.distance <= 2.5 * size + size * std::f64::consts::FRAC_1_SQRT_2 + 1e-6
                );
                assert_eq!(want.detail, Detail::Full);
            }
        }
        // Best first: a cell the viewer stands in leads (coarse before fine at equal
        // distance in cells, so the proxies arrive first), and the order is by priority.
        let first = plan.wants[0].cell;
        assert_eq!(first, grid.cell_of(viewer, first.level()));
        assert!(
            plan.wants
                .windows(2)
                .all(|w| w[0].priority <= w[1].priority)
        );
    }

    #[test]
    fn a_viewer_near_a_face_border_wants_cells_on_both_faces() {
        let planet = CubeSphere {
            radius: 1_500_000.0,
        };
        // Just on +X's side of the border with +Y.
        let viewer = planet.surface_point(DVec3::new(1.0, 0.99, 0.0), 0.0);
        let plan = StreamPlan::around(&planet, viewer, &policy());
        let faces: std::collections::BTreeSet<u8> =
            plan.at_level(6).map(|w| w.cell.face()).collect();
        assert!(
            faces.contains(&Face::PosX.index()) && faces.contains(&Face::PosY.index()),
            "{faces:?}"
        );
        assert!(plan.wants(planet.cell_of(viewer, 6)));
    }

    #[test]
    fn residency_loads_what_is_new_and_unloads_with_hysteresis() {
        let grid = FlatGrid {
            root_size: 65_536.0,
        };
        let policy = policy();
        let mut residency = Residency::new();
        let start = DVec3::new(100.0, 0.0, 100.0);
        let first = residency.update(
            &grid,
            start,
            &StreamPlan::around(&grid, start, &policy),
            &policy,
        );
        assert!(first.unload.is_empty());
        assert_eq!(first.load.len(), residency.len());
        assert!(residency.is_loaded(grid.cell_of(start, 6)));
        // One finest cell east: a strip loads, nothing unloads yet (hysteresis).
        let size = grid.cell_size(6);
        let moved = start + DVec3::new(size, 0.0, 0.0);
        let second = residency.update(
            &grid,
            moved,
            &StreamPlan::around(&grid, moved, &policy),
            &policy,
        );
        assert!(
            !second.load.is_empty() && second.load.len() < 20,
            "{}",
            second.load.len()
        );
        assert!(second.unload.is_empty(), "{:?}", second.unload);
        // Far away: everything old unloads, a new set loads.
        let far = start + DVec3::new(50.0 * size, 0.0, 0.0);
        let before: Vec<CellId> = residency.cells().collect();
        let third = residency.update(
            &grid,
            far,
            &StreamPlan::around(&grid, far, &policy),
            &policy,
        );
        for cell in before {
            if cell.level() == 6 {
                assert!(third.unload.contains(&cell));
            }
        }
        assert!(residency.is_loaded(grid.cell_of(far, 6)));
        assert!(!residency.is_loaded(grid.cell_of(start, 6)));
    }

    /// D-077: content near, proxies farther, swapped as the viewer goes by, each way with its
    /// hysteresis, and never a cell both loaded and unloaded.
    #[test]
    fn cells_swap_between_content_and_proxy_as_the_viewer_goes_by() {
        let grid = FlatGrid {
            root_size: 65_536.0,
        };
        let policy = StreamPolicy {
            finest: 6,
            coarsest: 6,
            rings: 1.5,
            hysteresis: 0.25,
            proxy_rings: 5.0,
        };
        let size = grid.cell_size(6);
        let start = DVec3::new(0.5 * size, 0.0, 0.5 * size);
        let plan = StreamPlan::around(&grid, start, &policy);
        let full = plan
            .wants
            .iter()
            .filter(|w| w.detail == Detail::Full)
            .count();
        let proxies = plan.wants.len() - full;
        // A disc of 1.5 cells, about 9 cells; out to 5, about 80 more.
        assert!((5..=16).contains(&full), "{full}");
        assert!((50..=120).contains(&proxies), "{proxies}");
        for want in &plan.wants {
            let reach = if want.detail == Detail::Full {
                1.5
            } else {
                5.0
            };
            assert!(want.distance <= (reach + std::f64::consts::FRAC_1_SQRT_2) * size + 1e-6);
        }
        let mut residency = Residency::new();
        let first = residency.update(&grid, start, &plan, &policy);
        assert_eq!(first.load.len(), plan.wants.len());
        let own = grid.cell_of(start, 6);
        assert_eq!(residency.detail(own), Some(Detail::Full));
        // Walk east a cell at a time: the cells ahead swap to their content as they come
        // within the reach, those behind to their proxy once past the hysteresis.
        let behind = own;
        let mut swapped_back = None;
        for step in 1..=8 {
            let at = start + DVec3::new(f64::from(step) * size, 0.0, 0.0);
            let plan = StreamPlan::around(&grid, at, &policy);
            let changes = residency.update(&grid, at, &plan, &policy);
            for &(cell, _) in &changes.load {
                assert!(!changes.unload.contains(&cell), "{cell:?} both ways");
            }
            // Every cell the plan wants as its content has it.
            for want in plan.wants.iter().filter(|w| w.detail == Detail::Full) {
                assert_eq!(residency.detail(want.cell), Some(Detail::Full));
            }
            let ahead = grid.cell_of(at, 6);
            assert_eq!(residency.detail(ahead), Some(Detail::Full));
            if swapped_back.is_none() && residency.detail(behind) == Some(Detail::Proxy) {
                swapped_back = Some(step);
            }
        }
        // The start's cell gives way to its proxy past the hysteresis: farther than the 1.5
        // cells it stops being wanted at.
        let back = swapped_back.expect("the start's cell swapped to its proxy");
        assert!((3..=4).contains(&back), "{back}");
        // Far away: everything unloads, the proxies too.
        let far = start + DVec3::new(100.0 * size, 0.0, 0.0);
        let before: Vec<CellId> = residency.cells().collect();
        let away = residency.update(
            &grid,
            far,
            &StreamPlan::around(&grid, far, &policy),
            &policy,
        );
        for cell in before {
            assert!(away.unload.contains(&cell), "{cell:?}");
        }
    }
}
