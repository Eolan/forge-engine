//! Interest by cells: which entities a client should hear about.
//!
//! The world's entities are bucketed into square cells on the ground plane (X and Z, D-004's
//! Y up) once a tick; a client's interest is the entities within a radius of its focus, found
//! by visiting only the cells the radius touches. The replicator keeps an entity a client
//! already hears about until it is a margin beyond the radius, so one walking along the edge
//! does not come and go every tick. The cells belong to the server's replication layer, never
//! to the entities: an entity carries no query of its own (Improbable's query-based interest,
//! US 10,579,434 and its family, is the patented alternative; `docs/research/netcode.md`, §9).

use std::collections::HashMap;

use glam::DVec3;

/// The world's entities bucketed by cell.
#[derive(Debug)]
pub struct InterestGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<u32>>,
}

impl InterestGrid {
    /// A grid of cells `cell` metres wide.
    pub fn new(cell: f64) -> Self {
        assert!(cell > 0.0, "cells need a size");
        Self {
            cell,
            cells: HashMap::new(),
        }
    }

    fn key(&self, x: f64, z: f64) -> (i64, i64) {
        (
            (x / self.cell).floor() as i64,
            (z / self.cell).floor() as i64,
        )
    }

    /// Buckets the entities at `positions`, numbered in their order. Cells keep their storage
    /// from tick to tick.
    pub fn rebuild(&mut self, positions: impl Iterator<Item = DVec3>) {
        for v in self.cells.values_mut() {
            v.clear();
        }
        for (k, p) in positions.enumerate() {
            let key = self.key(p.x, p.z);
            self.cells.entry(key).or_default().push(k as u32);
        }
    }

    /// The entities in the cells a disc of `radius` around `center` touches, cell by cell (X,
    /// then Z), each cell's in their order: a superset of those within the radius, in an order
    /// that depends only on the positions.
    pub fn query(&self, center: DVec3, radius: f64, out: &mut Vec<u32>) {
        out.clear();
        let (x0, z0) = self.key(center.x - radius, center.z - radius);
        let (x1, z1) = self.key(center.x + radius, center.z + radius);
        let span = (x1 - x0 + 1) as f64 * (z1 - z0 + 1) as f64;
        if span > self.cells.len() as f64 {
            // A radius wider than the world: visit the cells that exist, in key order.
            let mut keys: Vec<(i64, i64)> = self
                .cells
                .keys()
                .filter(|(x, z)| (x0..=x1).contains(x) && (z0..=z1).contains(z))
                .copied()
                .collect();
            keys.sort_unstable();
            for k in keys {
                out.extend_from_slice(&self.cells[&k]);
            }
            return;
        }
        for x in x0..=x1 {
            for z in z0..=z1 {
                if let Some(v) = self.cells.get(&(x, z)) {
                    out.extend_from_slice(v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::SplitMix64;

    #[test]
    fn a_query_finds_everything_within_its_radius() {
        let mut r = SplitMix64::new(8);
        let points: Vec<DVec3> = (0..5000)
            .map(|_| DVec3::new(r.range_f64(-500.0, 500.0), 0.0, r.range_f64(-500.0, 500.0)))
            .collect();
        let mut grid = InterestGrid::new(32.0);
        grid.rebuild(points.iter().copied());
        let mut out = Vec::new();
        for _ in 0..50 {
            let c = DVec3::new(r.range_f64(-500.0, 500.0), 0.0, r.range_f64(-500.0, 500.0));
            let radius = r.range_f64(10.0, 200.0);
            grid.query(c, radius, &mut out);
            let found: std::collections::BTreeSet<u32> = out.iter().copied().collect();
            for (k, p) in points.iter().enumerate() {
                if p.distance(c) <= radius {
                    assert!(found.contains(&(k as u32)));
                }
            }
            assert!(out.len() < points.len() / 2 || radius > 150.0);
        }
    }
}
