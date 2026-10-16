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

/// How deep a house's plan may go on a lot of type `ty`: the strip less the setback and the
/// yard, a hall and one room's depth at least.
pub fn plan_depth(ty: &ParcelType) -> f64 {
    (ty.depth - ty.setback - ty.yard).max(rooms::HALL + 2.6)
}

/// A house on each of `lots`, the household of lot `k` from `household(k)`. A house of two storeys
/// or more is made `stair` deep at least where its type allows: its straight flight's depth
/// (`cores::flight_length` of its ground storey, the core's back wall, the front and back walls).
pub fn houses(
    lots: &[Vec<DVec2>],
    ty: &ParcelType,
    household: impl Fn(usize) -> (u32, Means),
    stair: f64,
) -> Vec<House> {
    lots.iter()
        .enumerate()
        .filter(|(_, lot)| lot.len() >= 3)
        .filter_map(|(k, lot)| {
            let (people, means) = household(k);
            let (a, b) = (lot[0], lot[1]);
            let t = (b - a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x);
            // The house's front line: the street line, or the type's setback behind it.
            let a = a + inward * ty.setback;
            // The house takes the stretch of the front where the lot is at least a room deep:
            // a corner lot's bisector cuts the strip to a wedge, and a house across the whole
            // front had a window at its end with the roof right behind it and no room (the
            // owner, 2026-10-16). The rest of the front stays garden, as do the type's sides.
            let (s0, s1) = front_stretch(lot, a, t, inward, rooms::DEPTH.0)?;
            let (s0, s1) = (s0 + ty.sides, s1 - ty.sides);
            let width = s1 - s0;
            if width < rooms::HALL + 2.4 + rooms::WALL {
                return None;
            }
            let mut plan = rooms::plan(&rooms::programme(people, means), width, plan_depth(ty));
            if plan.storeys.len() > 1 {
                plan.deepen(stair.min(plan_depth(ty)));
            }
            // A front cut short (a corner's wedge) with a household's rooms piled over three
            // storeys is no village house: the lot stays garden.
            if plan.storeys.len() > 3 {
                return None;
            }
            let (a, b) = (a + t * s0, a + t * s1);
            let inward = inward * plan.depth;
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

/// The stretch `(from, to)` along the front `a + t s` of `lot` where the lot reaches at least
/// `depth` metres in along `inward`, sampled every quarter metre (one stretch: along a convex
/// lot's front the depth rises then falls); `None` where it never does.
fn front_stretch(
    lot: &[DVec2],
    a: DVec2,
    t: DVec2,
    inward: DVec2,
    depth: f64,
) -> Option<(f64, f64)> {
    let width = parcels::frontage(lot);
    let n = lot.len();
    let deep_at = |s: f64| {
        let p = a + t * s;
        if !geom::contains(lot, p + inward * 0.3) {
            return 0.0;
        }
        let far = p + inward * 200.0;
        (0..n)
            .filter_map(|i| geom::segment_intersection(p, far, lot[i], lot[(i + 1) % n]))
            .map(|(u, _)| u * 200.0)
            .filter(|&d| d > 0.05)
            .fold(f64::MAX, f64::min)
    };
    let steps = (width / 0.25).ceil().max(1.0) as usize;
    let (mut from, mut to) = (None, None);
    for k in 0..=steps {
        let s = (k as f64 * 0.25).min(width);
        // Probed 2 cm in from the lot's corners: on a corner the probe fell on the side line,
        // outside, and every house of a row started a quarter metre in from its lot's side, a
        // sliver between each two with a garden wall in it (the anomaly check's first find).
        if deep_at(s.clamp(0.02, (width - 0.02).max(0.02))) >= depth {
            from.get_or_insert(s);
            to = Some(s);
        } else if from.is_some() {
            break;
        }
    }
    Some((from?, to?))
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
        let houses = houses(&lots, &ty, |k| (2 + (k % 3) as u32, Means::Modest), 8.3);
        // A corner wedge may keep no house.
        assert!(houses.len() + 2 >= lots.len() && houses.len() <= lots.len());
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
            assert!(h.plan.depth <= ty.depth - ty.yard + 1e-9);
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
