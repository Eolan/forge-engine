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
//! The cells within reach are found by sampling the disc at half a cell's spacing and asking
//! the partition for each sample's cell, which crosses the cube sphere's face borders without
//! a neighbour table: a cell with its centre inside the reach holds at least one sample.

use std::collections::BTreeMap;

use glam::DVec3;

use crate::cell_id::CellId;
use crate::partition::Partition;

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
}

impl Default for StreamPolicy {
    fn default() -> Self {
        Self {
            finest: 12,
            coarsest: 4,
            rings: 2.5,
            hysteresis: 0.25,
        }
    }
}

impl StreamPolicy {
    /// A level's reach, metres.
    pub fn reach<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        self.rings * partition.cell_size(level)
    }

    /// How far a loaded cell's centre may lie before it unloads, metres: the farthest a wanted
    /// centre can be (a sample on the reach's edge lands in a cell whose centre is up to half a
    /// diagonal beyond), times `1 + hysteresis`.
    pub fn keep_within<P: Partition>(&self, partition: &P, level: u8) -> f64 {
        let size = partition.cell_size(level);
        (self.reach(partition, level) + size * std::f64::consts::FRAC_1_SQRT_2)
            * (1.0 + self.hysteresis)
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
            let spacing = 0.5 * size;
            let steps = (reach / spacing).ceil() as i32;
            let mut cells: Vec<CellId> = Vec::new();
            for j in -steps..=steps {
                for i in -steps..=steps {
                    let east = f64::from(i) * spacing;
                    let north = f64::from(j) * spacing;
                    if east * east + north * north > reach * reach {
                        continue;
                    }
                    cells.push(partition.cell_of(partition.step(viewer, east, north), level));
                }
            }
            cells.sort_unstable();
            cells.dedup();
            for cell in cells {
                let distance = (partition.cell_center(cell) - viewer).length();
                wants.push(CellWant {
                    cell,
                    distance,
                    priority: distance / size,
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
}

/// What a plan asks of the loaded set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// Cells to load, best first.
    pub load: Vec<CellId>,
    /// Cells to unload, in id order.
    pub unload: Vec<CellId>,
}

/// The loaded cells, following successive plans with hysteresis.
#[derive(Clone, Debug, Default)]
pub struct Residency {
    loaded: BTreeMap<CellId, ()>,
}

impl Residency {
    /// Nothing loaded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `plan` for the viewer at `viewer`: the wanted cells not yet loaded are to load,
    /// and the loaded cells the plan no longer wants are to unload once they lie beyond
    /// [`StreamPolicy::keep_within`]. Both are applied to the set at once: the caller streams
    /// them in its own time.
    pub fn update<P: Partition>(
        &mut self,
        partition: &P,
        viewer: DVec3,
        plan: &StreamPlan,
        policy: &StreamPolicy,
    ) -> Changes {
        let mut changes = Changes::default();
        for want in &plan.wants {
            if !self.loaded.contains_key(&want.cell) {
                changes.load.push(want.cell);
            }
        }
        for &cell in self.loaded.keys() {
            if plan.wants(cell) {
                continue;
            }
            let distance = (partition.cell_center(cell) - viewer).length();
            let keep_within = policy.keep_within(partition, cell.level());
            if distance > keep_within
                || cell.level() > policy.finest
                || cell.level() < policy.coarsest
            {
                changes.unload.push(cell);
            }
        }
        for &cell in &changes.load {
            self.loaded.insert(cell, ());
        }
        for cell in &changes.unload {
            self.loaded.remove(cell);
        }
        changes
    }

    /// Whether `cell` is loaded.
    pub fn is_loaded(&self, cell: CellId) -> bool {
        self.loaded.contains_key(&cell)
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
}
