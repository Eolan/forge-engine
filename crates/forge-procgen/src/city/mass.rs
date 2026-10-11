//! Mass models: each lot's building as a footprint and a height, the first stage of D-039's
//! grammar ("a mass model fitted to its lot"). A lot's building fills the strip between the
//! building line (the block's outline moved in by half the width of each street it fronts) and the courtyard (a
//! building's depth further in), measured from the street it faces. Its floors come from its
//! district's range, capped by its street's profile: the tallest on avenues, lower on shared
//! streets, the lowest on pedestrian lanes (heights that follow street widths,
//! `city-generation.md` §15).

use forge_core::SplitMix64;
use glam::DVec2;

use super::bible::StyleBible;
use super::blocks::{BlockUse, edge_half_width};
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
    /// The width of the street it fronts, metres.
    pub street_width: f64,
}

/// A ground floor's height, metres.
pub const GROUND_FLOOR: f64 = 4.2;
/// An upper floor's height, metres.
pub const FLOOR: f64 = 3.2;

/// The masses of every lot on a street.
pub fn masses(plan: &CityPlan, bible: &StyleBible, seed: u64) -> Vec<Mass> {
    let green_roofs = bible.green_roofs;
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
        let outline = &block.buildable;
        let Some((a, t)) = frontage(&lot.polygon, outline) else {
            continue;
        };
        // The lot inside the building line. Lots are cut from the block's buildable outline,
        // already on the building lines; where that outline fell back to the streets'
        // centrelines, each edge the lot touches cuts it by what is left of half its street.
        let mut inside = lot.polygon.clone();
        let n = outline.len();
        let m = block.polygon.len();
        // The edges the lot fronts for 3 m or more, with what is left of their streets' half
        // widths, longest first.
        let mut fronts: Vec<(DVec2, DVec2, f64, f64)> = Vec::new();
        for i in 0..n {
            if inside.len() < 3 {
                break;
            }
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            let edge = (q - p).normalize_or_zero();
            let touches = lot
                .polygon
                .iter()
                .any(|&v| geom::distance_to_segment(v, p, q) < 0.5);
            if edge == DVec2::ZERO || !touches {
                continue;
            }
            let inward = DVec2::new(-edge.y, edge.x);
            let mid = (p + q) * 0.5;
            let moved = (0..m)
                .map(|j| {
                    geom::distance_to_segment(mid, block.polygon[j], block.polygon[(j + 1) % m])
                })
                .fold(f64::MAX, f64::min);
            let half = (edge_half_width(&streets, &plan.roads, p, q) - moved).max(0.0);
            inside = geom::clip_half_plane(&inside, p + inward * half, edge);
            let shared = shared_length(&lot.polygon, p, q);
            if shared >= 3.0 {
                fronts.push((p, q, half, shared));
            }
        }
        fronts.sort_by(|x, y| y.3.total_cmp(&x.3));
        // Strips of a building's depth along each street the lot fronts, the longest frontage
        // first: a lot between two streets is built on both, a corner lot wraps its corner, and
        // the courtyard (inset by the widest street round the block) stays clear. Subtracting
        // the courtyard's convex hull instead ate the buildings along concave and curved sides.
        let pieces = match district.kind.building_depth() {
            Some(depth) => {
                let mut rest = inside.clone();
                let mut strips = Vec::new();
                for &(p, q, half, _) in &fronts {
                    if rest.len() < 3 {
                        break;
                    }
                    let edge = (q - p).normalize_or_zero();
                    let line = p + DVec2::new(-edge.y, edge.x) * (half + depth);
                    let near = geom::clip_half_plane(&rest, line, -edge);
                    if near.len() >= 3 {
                        strips.push(near);
                    }
                    rest = geom::clip_half_plane(&rest, line, edge);
                }
                strips
            }
            None => vec![inside],
        };
        let pieces: Vec<Vec<DVec2>> = pieces
            .into_iter()
            .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() >= 20.0)
            // Slivers a strip leaves in the courtyard where the street bends.
            .filter(|p| {
                block
                    .courtyard
                    .as_ref()
                    .is_none_or(|c| !geom::contains(c, geom::centroid(p)))
            })
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
        // Heights follow street widths: the bible's ratio caps the floors.
        let street_width = profile.width();
        let cap = bible.max_floors(street_width);
        let floors = drawn.min(cap).max(1);
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
                street_width,
            });
        }
    }
    out
}

/// How much of a lot's outline lies along the block edge from `a` to `b`, metres.
fn shared_length(lot: &[DVec2], a: DVec2, b: DVec2) -> f64 {
    let m = lot.len();
    let mut shared = 0.0;
    for j in 0..m {
        let (p, q) = (lot[j], lot[(j + 1) % m]);
        if geom::distance_to_segment(p, a, b) < 0.5 && geom::distance_to_segment(q, a, b) < 0.5 {
            shared += p.distance(q);
        }
    }
    shared
}

/// The block edge a lot fronts on: its start and unit direction, the edge sharing the longest
/// stretch with the lot's outline.
fn frontage(lot: &[DVec2], block: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let n = block.len();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..n {
        let (a, b) = (block[i], block[(i + 1) % n]);
        let shared = shared_length(lot, a, b);
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
    use crate::city::bible::StyleBible;

    #[test]
    fn masses_stand_between_the_building_line_and_the_courtyard() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
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
