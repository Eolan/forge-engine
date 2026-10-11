//! Mass models: each lot's building as a footprint and a height, the first stage of D-039's
//! grammar ("a mass model fitted to its lot"). A lot's building fills the strip between the
//! building line (the block's outline moved in by [`STREET_HALF`]) and the courtyard (a
//! building's depth further in), measured from the street it faces. Its floors come from its
//! district's range, capped by its street's profile: the tallest on avenues, lower on shared
//! streets, the lowest on pedestrian lanes (heights that follow street widths,
//! `city-generation.md` §15).

use forge_core::SplitMix64;
use glam::DVec2;

use super::blocks::{BlockUse, STREET_HALF};
use super::geom::{self, PointGrid};
use super::{CityPlan, Profile};

/// What a roof carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Roof {
    /// Planted.
    Green,
    /// Solar panels.
    Solar,
    /// A terrace people use.
    Terrace,
}

/// A building's mass.
#[derive(Clone, Debug, PartialEq)]
pub struct Mass {
    /// Its lot's index.
    pub lot: u32,
    /// Its footprint, metres on the plan (positive area).
    pub footprint: Vec<DVec2>,
    /// Its floors.
    pub floors: u8,
    /// Its height to the roof, metres: a 4.2 m ground floor and 3.2 m upper floors.
    pub height: f64,
    /// What its roof carries.
    pub roof: Roof,
}

/// A ground floor's height, metres.
pub const GROUND_FLOOR: f64 = 4.2;
/// An upper floor's height, metres.
pub const FLOOR: f64 = 3.2;

/// The masses of every lot on a street.
pub fn masses(plan: &CityPlan, green_roofs: f64, seed: u64) -> Vec<Mass> {
    // The streets' points, to find the profile a lot faces.
    let mut streets = PointGrid::new(plan.size, 50.0);
    for (i, e) in plan.roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 10.0) {
            streets.insert(p, i as u32);
        }
    }
    let mut out = Vec::new();
    for (index, lot) in plan.lots.iter().enumerate() {
        if !lot.frontage {
            continue;
        }
        let block = &plan.blocks[lot.block as usize];
        if block.use_ != BlockUse::Built {
            continue;
        }
        let district = &plan.districts[block.district];
        let (lo, hi) = district.floors;
        if hi == 0 {
            continue;
        }
        let Some((a, t)) = frontage(&lot.polygon, &block.polygon) else {
            continue;
        };
        // The lot inside the building line: cut by each block edge it touches, moved in by half
        // a street (an inset of the whole outline fails on curved, concave blocks). Less the
        // courtyard, the pieces close round the block's corners.
        let mut inside = lot.polygon.clone();
        let n = block.polygon.len();
        for i in 0..n {
            if inside.len() < 3 {
                break;
            }
            let (p, q) = (block.polygon[i], block.polygon[(i + 1) % n]);
            let edge = (q - p).normalize_or_zero();
            let touches = lot
                .polygon
                .iter()
                .any(|&v| geom::distance_to_segment(v, p, q) < 0.5);
            if edge == DVec2::ZERO || !touches {
                continue;
            }
            let inward = DVec2::new(-edge.y, edge.x);
            inside = geom::clip_half_plane(&inside, p + inward * STREET_HALF, edge);
        }
        let pieces = match (&block.courtyard, district.kind.building_depth()) {
            (Some(courtyard), _) => outside_of(&inside, &geom::convex_hull(courtyard)),
            // No courtyard fits: a strip of a building's depth along the street it fronts.
            (None, Some(depth)) => {
                let inward = DVec2::new(-t.y, t.x);
                vec![geom::clip_half_plane(
                    &inside,
                    a + inward * (STREET_HALF + depth),
                    -t,
                )]
            }
            (None, None) => vec![inside],
        };
        let pieces: Vec<Vec<DVec2>> = pieces
            .into_iter()
            .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() >= 20.0)
            .map(|p| {
                if geom::signed_area(&p) < 0.0 {
                    p.into_iter().rev().collect()
                } else {
                    p
                }
            })
            .collect();
        if pieces.is_empty() {
            continue;
        }
        // The profile of the street the lot faces.
        let front = a + t * 1.0;
        let profile = streets
            .nearest(front, 60.0)
            .map(|(_, e)| plan.roads.edges[e as usize].profile)
            .unwrap_or(Profile::Shared);
        let mut rng = SplitMix64::new(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let span = u64::from(hi.saturating_sub(lo)) + 1;
        let drawn = lo + rng.below(span) as u8;
        let cap = match profile {
            Profile::TramAvenue | Profile::Avenue => hi,
            Profile::Shared => lo + (hi - lo) * 2 / 3,
            Profile::Pedestrian | Profile::Path => lo + (hi - lo) / 3,
        };
        let floors = drawn.min(cap).max(lo.max(1));
        let height = GROUND_FLOOR + f64::from(floors - 1) * FLOOR;
        let r = rng.next_f64();
        let roof = if r < green_roofs {
            Roof::Green
        } else if r < green_roofs + (1.0 - green_roofs) * 0.6 {
            Roof::Solar
        } else {
            Roof::Terrace
        };
        for footprint in pieces {
            out.push(Mass {
                lot: index as u32,
                footprint,
                floors,
                height,
                roof,
            });
        }
    }
    out
}

/// A convex-enough polygon less a convex hole: the pieces outside each of the hole's edges in
/// turn.
fn outside_of(polygon: &[DVec2], hole: &[DVec2]) -> Vec<Vec<DVec2>> {
    let mut rest = polygon.to_vec();
    let mut pieces = Vec::new();
    let n = hole.len();
    for i in 0..n {
        if rest.len() < 3 {
            break;
        }
        let (p, q) = (hole[i], hole[(i + 1) % n]);
        let t = (q - p).normalize_or_zero();
        let out = geom::clip_half_plane(&rest, p, -t);
        if out.len() >= 3 {
            pieces.push(out);
        }
        rest = geom::clip_half_plane(&rest, p, t);
    }
    pieces
}

/// The block edge a lot fronts on: its start and unit direction, the edge sharing the longest
/// stretch with the lot's outline.
fn frontage(lot: &[DVec2], block: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let n = block.len();
    let m = lot.len();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..n {
        let (a, b) = (block[i], block[(i + 1) % n]);
        let mut shared = 0.0;
        for j in 0..m {
            let (p, q) = (lot[j], lot[(j + 1) % m]);
            if geom::distance_to_segment(p, a, b) < 0.5 && geom::distance_to_segment(q, a, b) < 0.5
            {
                shared += p.distance(q);
            }
        }
        if shared > 1.0 && best.is_none_or(|(s, _)| shared > s) {
            best = Some((shared, i));
        }
    }
    let (_, i) = best?;
    let (a, b) = (block[i], block[(i + 1) % n]);
    Some((a, (b - a).normalize_or_zero()))
}

#[cfg(test)]
mod tests {
    use super::super::{example, plan};
    use super::*;

    #[test]
    fn masses_stand_between_the_building_line_and_the_courtyard() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, 0.5, 9);
        assert!(
            masses.len() > city.lots.len() / 2,
            "{} of {}",
            masses.len(),
            city.lots.len()
        );
        for m in masses.iter().take(2000) {
            let lot = &city.lots[m.lot as usize];
            let block = &city.blocks[lot.block as usize];
            assert!(m.height >= GROUND_FLOOR);
            assert!(geom::signed_area(&m.footprint) > 0.0);
            let c = geom::centroid(&m.footprint);
            assert!(
                geom::contains(&block.polygon, c),
                "a mass outside its block"
            );
            if let Some(courtyard) = &block.courtyard {
                assert!(!geom::contains(courtyard, c), "a mass in its courtyard");
            }
        }
        let green = masses.iter().filter(|m| m.roof == Roof::Green).count();
        assert!(green * 3 > masses.len(), "about half the roofs are green");
    }
}
