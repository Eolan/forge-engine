//! Houses on their lots (D-062's first map): for each lot a strip gives
//! ([`super::parcels::strips`]), a household's programme laid out by [`super::rooms::plan`]
//! to the lot's width and the strip's depth; the house's footprint runs from the street to its
//! plan's depth, cut to the lot where a corner lot narrows, and the rest of the lot is its
//! garden. The lots fit the houses and the houses their rooms, the owner's way round.

use glam::DVec2;

use super::geom;
use super::parcels::{self, ParcelType};
use super::rooms::{self, Means, Plan};

/// A house on its lot.
#[derive(Clone, Debug, PartialEq)]
pub struct House {
    /// Its lot, its street side first (as [`parcels::strips`] lays it).
    pub lot: Vec<DVec2>,
    /// Its plan: the rooms by storey on the lot's width.
    pub plan: Plan,
    /// Its footprint: the lot's width from the street to the plan's depth, within the lot.
    pub footprint: Vec<DVec2>,
    /// The household: its people and its means.
    pub household: (u32, Means),
}

/// A margin the house keeps from the strip's back, metres: a yard behind at least.
pub const YARD: f64 = 1.5;

/// How deep a house's plan may go on a lot of type `ty`: the strip less the yard, a hall and
/// one room's depth at least.
pub fn plan_depth(ty: &ParcelType) -> f64 {
    (ty.depth - YARD).max(rooms::HALL + 2.6)
}

/// A house on each of `lots`, the household of lot `k` from `household(k)`.
pub fn houses(
    lots: &[Vec<DVec2>],
    ty: &ParcelType,
    household: impl Fn(usize) -> (u32, Means),
) -> Vec<House> {
    lots.iter()
        .enumerate()
        .filter(|(_, lot)| lot.len() >= 3)
        .filter_map(|(k, lot)| {
            let (people, means) = household(k);
            let width = parcels::frontage(lot);
            let plan = rooms::plan(&rooms::programme(people, means), width, plan_depth(ty));
            let (a, b) = (lot[0], lot[1]);
            let t = (b - a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x) * plan.depth;
            let mut footprint = vec![a, b, b + inward, a + inward];
            // Within the lot: a corner lot narrows towards the back.
            let n = lot.len();
            for i in 0..n {
                footprint = geom::clip_half_plane(&footprint, lot[i], lot[(i + 1) % n] - lot[i]);
                if footprint.len() < 3 {
                    break;
                }
            }
            // A corner wedge too narrow for a house (its footprint cut to nothing) stays a
            // garden.
            // The clips repeat a corner they pass through: a repeated corner made a wall of no
            // length, an inner one, and the house got no stair.
            let footprint = geom::clean(&footprint, 1e-6);
            if footprint.len() < 3 || geom::signed_area(&footprint).abs() < 4.0 {
                return None;
            }
            Some(House {
                lot: lot.clone(),
                plan,
                footprint,
                household: (people, means),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modest_houses_fill_a_village_block_within_their_lots() {
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let ty = ParcelType::VILLAGE;
        let (lots, court) = parcels::strips(&block, &[true; 4], &ty, 5).expect("strips");
        let houses = houses(&lots, &ty, |k| (2 + (k % 3) as u32, Means::Modest));
        assert_eq!(houses.len(), lots.len());
        for h in &houses {
            assert!(h.footprint.len() >= 3, "a footprint");
            let area = geom::signed_area(&h.footprint).abs();
            let lot_area = geom::signed_area(&h.lot).abs();
            assert!(
                area > 20.0 && area <= lot_area + 1e-6,
                "{area:.0} of {lot_area:.0} m²"
            );
            // Every footprint corner on or inside its lot.
            for &p in &h.footprint {
                let n = h.lot.len();
                let inside = geom::contains(&h.lot, p)
                    || (0..n)
                        .any(|i| geom::distance_to_segment(p, h.lot[i], h.lot[(i + 1) % n]) < 1e-6);
                assert!(inside, "a footprint corner {p} off its lot");
            }
            assert!(h.plan.depth <= ty.depth - YARD + 1e-9);
            assert!(!h.plan.storeys.is_empty());
        }
        assert!(geom::signed_area(&court).abs() > 1000.0);
        if let Ok(path) = std::env::var("HOUSES_PNG") {
            super::super::map::write_houses(
                &block,
                &houses,
                &court,
                std::path::Path::new(&path),
                0.1,
            )
            .expect("the drawing");
        }
    }
}
