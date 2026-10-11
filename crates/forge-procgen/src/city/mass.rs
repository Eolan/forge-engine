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
    /// Pitched and tiled, hipped from every side: the old towns' terracotta.
    Tiled,
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
    /// Each storey's floor to floor, ground first, metres: by its district's era
    /// ([`super::structure::Era::storeys`]).
    pub storeys: Vec<f64>,
    /// Its height to the roof, metres: its storeys' sum.
    pub height: f64,
    /// What its roof carries.
    pub roof: Roof,
    /// The width of the street it fronts, metres.
    pub street_width: f64,
    /// Whether it looks across its street onto a square, a park or a quay.
    pub open: bool,
}

impl Mass {
    /// Whether its ground floor is shops, cafés and bars: on the avenues (18 m or wider), on
    /// the lanes (8 m or narrower), and facing a square, a park or a quay (the owner: large
    /// squares want shops, restaurants and cafés round them, above all by the river and the
    /// sea).
    pub fn lively(&self) -> bool {
        self.open || self.street_width >= 18.0 || self.street_width <= 8.0
    }

    /// Its ground storey's floor to floor, metres.
    pub fn ground(&self) -> f64 {
        self.storeys.first().copied().unwrap_or(self.height)
    }

    /// Its upper storeys' mean floor to floor, metres (its ground storey's if it has none).
    pub fn upper(&self) -> f64 {
        if self.storeys.len() < 2 {
            self.ground()
        } else {
            (self.height - self.ground()) / (self.storeys.len() - 1) as f64
        }
    }
}

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
    // The squares, parks and quays, by their centres: who looks onto them.
    let mut open_blocks = PointGrid::new(plan.size, 100.0);
    for (i, b) in plan.blocks.iter().enumerate() {
        if matches!(b.use_, BlockUse::Square | BlockUse::Park | BlockUse::Quay) {
            open_blocks.insert(b.centre, i as u32);
        }
    }
    let gaps = open_gaps(plan, seed);
    let mut out = Vec::new();
    for (index, lot) in plan.lots.iter().enumerate() {
        if !lot.frontage || gaps.contains(&index) {
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
        let pieces = match district.building_depth() {
            Some(depth) => {
                // The strips, the first `extra` metres deeper, with their fronts' indices.
                let strips = |extra: f64| {
                    let mut rest = inside.clone();
                    let mut strips = Vec::new();
                    for (k, &(p, q, half, _)) in fronts.iter().enumerate() {
                        if rest.len() < 3 {
                            break;
                        }
                        let edge = (q - p).normalize_or_zero();
                        let deep = if k == 0 { depth + extra } else { depth };
                        let line = p + DVec2::new(-edge.y, edge.x) * (half + deep);
                        let near = geom::clip_half_plane(&rest, line, -edge);
                        if near.len() >= 3 {
                            strips.push((k, near));
                        }
                        rest = geom::clip_half_plane(&rest, line, edge);
                    }
                    strips
                };
                // A corner lot a little deeper than the first strip leaves a sliver along its
                // second street: the first strip takes it in instead.
                let first = strips(0.0);
                let sliver = first
                    .iter()
                    .filter(|(k, s)| {
                        *k > 0 && geom::signed_area(s).abs() >= 20.0 && thickness(s) < MIN_THICKNESS
                    })
                    .map(|(_, s)| thickness(s))
                    .fold(0.0, f64::max);
                let chosen = if sliver > 0.0 {
                    strips(sliver + 0.5)
                } else {
                    first
                };
                chosen.into_iter().map(|(_, s)| s).collect()
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
        // No wedge to a point: acute corners cut square (the owner saw an impossible building at a
        // block's sharp corner).
        let pieces = absorb_thin(
            pieces
                .iter()
                .map(|p| blunt(p))
                .filter(|p| p.len() >= 3 && geom::signed_area(p) >= 20.0)
                .collect(),
        );
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
        // Across the street from the middle of the lot's front: a square, a park or a quay?
        let open = {
            let c = geom::centroid(&lot.polygon);
            let on = a + t * (c - a).dot(t);
            let inward = DVec2::new(-t.y, t.x);
            let outward = if geom::contains(&block.polygon, on + inward * 2.0) {
                -inward
            } else {
                inward
            };
            let across = on + outward * (street_width + 8.0);
            open_blocks.any_within(across, 1500.0, |b| {
                !geom::contains(&plan.blocks[b as usize].polygon, across)
            })
        };
        // Its storeys by its district's era (D-060): shops' or homes' ground storey, then the
        // uppers; the bible's ratio caps how many fit, and the height is their sum.
        let era = super::structure::Era::of(district.kind);
        let shops = open || street_width >= 18.0 || street_width <= 8.0;
        let cap = bible.max_floors(street_width, era, shops);
        let floors = drawn.min(cap).max(1);
        let storeys = era.storeys(floors, shops);
        let height = storeys.iter().sum();
        let r = rng.next_f64();
        let tiled = rng.next_f64() < district.tiled_share();
        let roof = if tiled {
            Roof::Tiled
        } else if r < green_roofs {
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
                storeys: storeys.clone(),
                height,
                roof,
                street_width,
                open,
            });
        }
    }
    out
}

/// Under this run (metres) an open-block district opens its corners too.
const OPEN_CORNERS: f64 = 60.0;

/// Open blocks (`DistrictSpec::gaps`): the lots left as gardens between the buildings, so light,
/// views and people reach the courtyards (the owner found the long rows of big buildings too
/// close, 2026-10-11). Each block's street lots in order round its outline: a lot between its
/// corners (one street front) becomes a gap at the district's share, and wherever the front
/// has run its district's [`super::DistrictSpec::run_max`] since the last gap; never two side by
/// side. Where that run is under [`OPEN_CORNERS`], a corner lot may be a gap too: separate
/// buildings round a shared green (the owner: open and organic, not oppressive).
fn open_gaps(plan: &CityPlan, seed: u64) -> std::collections::HashSet<usize> {
    let mut by_block: std::collections::HashMap<u32, Vec<usize>> = Default::default();
    for (i, lot) in plan.lots.iter().enumerate() {
        if lot.frontage {
            by_block.entry(lot.block).or_default().push(i);
        }
    }
    let mut gaps = std::collections::HashSet::new();
    let mut blocks: Vec<_> = by_block.into_iter().collect();
    blocks.sort_by_key(|(b, _)| *b);
    for (b, lots) in blocks {
        let block = &plan.blocks[b as usize];
        let district = &plan.districts[block.district];
        let (share, run_max) = (district.gap_share(), district.run_max());
        let corners = run_max < OPEN_CORNERS;
        if share <= 0.0 || block.use_ != BlockUse::Built {
            continue;
        }
        let outline = &block.buildable;
        let n = outline.len();
        // Where each edge starts along the outline.
        let mut start = Vec::with_capacity(n);
        let mut s = 0.0;
        for i in 0..n {
            start.push(s);
            s += outline[i].distance(outline[(i + 1) % n]);
        }
        // Each lot: where its front lies along the outline, its length, its street edges.
        let mut fronts: Vec<(f64, f64, usize, usize)> = lots
            .iter()
            .filter_map(|&l| {
                let poly = &plan.lots[l].polygon;
                let shared: Vec<(usize, f64)> = (0..n)
                    .map(|i| (i, shared_length(poly, outline[i], outline[(i + 1) % n])))
                    .filter(|&(_, len)| len >= 3.0)
                    .collect();
                let &(edge, length) = shared.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
                let (a, q) = (outline[edge], outline[(edge + 1) % n]);
                let t = (q - a).normalize_or_zero();
                let along = start[edge] + (geom::centroid(poly) - a).dot(t);
                Some((along, length, shared.len(), l))
            })
            .collect();
        fronts.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut run = 0.0;
        let mut last = false;
        for (_, length, edges, l) in fronts {
            let draw =
                SplitMix64::new(seed ^ (l as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)).next_f64();
            if (edges == 1 || corners) && !last && (draw < share || run >= run_max) {
                gaps.insert(l);
                run = 0.0;
                last = true;
            } else {
                run += length;
                last = false;
            }
        }
    }
    gaps
}

/// The thinnest a building stands, metres.
const MIN_THICKNESS: f64 = 4.0;

/// A piece's thickness: the lesser of its extents along and across its longest edge.
fn thickness(piece: &[DVec2]) -> f64 {
    let (_, extent, _) = geom::oriented_extent(piece);
    extent.x.min(extent.y)
}

/// The sharpest corner a building keeps, radians: 55°.
const SHARPEST: f64 = 55.0 * std::f64::consts::PI / 180.0;
/// How wide an acute corner's cut is, metres: a room's width.
const BLUNT: f64 = 6.0;

/// A footprint (positive area) with each corner sharper than [`SHARPEST`] cut square to its
/// bisector where the wedge is [`BLUNT`] wide: a chamfered corner, not a wedge to a point.
fn blunt(polygon: &[DVec2]) -> Vec<DVec2> {
    let mut out = polygon.to_vec();
    for _ in 0..4 {
        let n = out.len();
        if n < 3 {
            break;
        }
        let acute = (0..n).find_map(|i| {
            let (prev, v, next) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
            let (a, b) = (
                (prev - v).normalize_or_zero(),
                (next - v).normalize_or_zero(),
            );
            let convex = geom::cross(v - prev, next - v) > 0.0;
            let angle = a.angle_to(b).abs();
            (convex && angle > 1e-3 && angle < SHARPEST).then_some((v, a, b, angle))
        });
        let Some((v, a, b, angle)) = acute else {
            break;
        };
        let inward = (a + b).normalize_or_zero();
        let cut = v + inward * (BLUNT * 0.5 / (angle * 0.5).tan());
        // Keep what lies beyond the cut, away from the corner.
        out = geom::clip_half_plane(&out, cut, DVec2::new(inward.y, -inward.x));
    }
    out
}

/// A lot's strips with each thinner than [`MIN_THICKNESS`] (what is left along a second street
/// past the first strip, at a corner) joined to another of them where the two make a convex
/// piece (their hull adds under 3 %), or dropped.
fn absorb_thin(mut pieces: Vec<Vec<DVec2>>) -> Vec<Vec<DVec2>> {
    let mut i = 0;
    while i < pieces.len() {
        if thickness(&pieces[i]) >= MIN_THICKNESS {
            i += 1;
            continue;
        }
        let thin = pieces.remove(i);
        let area = geom::signed_area(&thin);
        let hull = |other: &[DVec2]| {
            let points: Vec<DVec2> = other.iter().chain(&thin).copied().collect();
            let hull = geom::convex_hull(&points);
            (geom::signed_area(&hull) <= (geom::signed_area(other) + area) * 1.03).then_some(hull)
        };
        if let Some((j, joined)) = (0..pieces.len()).find_map(|j| hull(&pieces[j]).map(|h| (j, h)))
        {
            pieces[j] = joined;
        }
    }
    pieces
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
    fn open_blocks_leave_gaps_between_their_corners() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let mut params = example::params(size);
        let closed = plan(&site, &params);
        for d in &mut params.districts {
            d.gaps = Some(0.1);
        }
        let open = plan(&site, &params);
        let gaps = open_gaps(&open, 9);
        assert!(!gaps.is_empty());
        // Gaps fall between a block's corners: a lot with one street front.
        for &l in &gaps {
            let lot = &open.lots[l];
            let outline = &open.blocks[lot.block as usize].buildable;
            let n = outline.len();
            let fronts = (0..n)
                .filter(|&i| shared_length(&lot.polygon, outline[i], outline[(i + 1) % n]) >= 3.0)
                .count();
            assert_eq!(fronts, 1);
        }
        let style = StyleBible::default();
        let (with, without) = (
            masses(&open, &style, 9).len(),
            masses(&closed, &style, 9).len(),
        );
        assert!(with < without, "{with} masses with gaps, {without} without");
        assert_eq!(closed.lots.len(), open.lots.len());
    }

    #[test]
    fn the_buildings_round_the_squares_and_quays_open_onto_them() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let open: Vec<_> = masses.iter().filter(|m| m.open).collect();
        let share = open.len() as f64 / masses.len() as f64;
        assert!((0.02..0.4).contains(&share), "{share:.3} open");
        // Each within its street's width and a little of a square, a park or a quay.
        let near = |m: &Mass| {
            let c = geom::centroid(&m.footprint);
            city.blocks.iter().any(|b| {
                matches!(b.use_, BlockUse::Square | BlockUse::Park | BlockUse::Quay)
                    && (0..b.polygon.len()).any(|i| {
                        let (p, q) = (b.polygon[i], b.polygon[(i + 1) % b.polygon.len()]);
                        geom::distance_to_segment(c, p, q) < m.street_width + 40.0
                    })
            })
        };
        assert!(open.iter().all(|m| near(m)));
        assert!(open.iter().all(|m| m.lively()));
    }

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
            assert!((m.height - m.storeys.iter().sum::<f64>()).abs() < 1e-9 && m.height >= 3.0);
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
        // No sliver of a building: what is left past a corner strip joins it.
        let thinnest = masses
            .iter()
            .map(|m| thickness(&m.footprint))
            .fold(f64::MAX, f64::min);
        assert!(thinnest >= MIN_THICKNESS, "a mass {thinnest:.1} m thick");
        let green = masses.iter().filter(|m| m.roof == Roof::Green).count();
        assert!(green * 3 > masses.len(), "about half the roofs are green");
    }

    #[test]
    fn no_building_comes_to_a_point() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let mut sharp = 0;
        for m in &masses {
            let n = m.footprint.len();
            for i in 0..n {
                let (prev, v, next) = (
                    m.footprint[(i + n - 1) % n],
                    m.footprint[i],
                    m.footprint[(i + 1) % n],
                );
                let (a, b) = (
                    (prev - v).normalize_or_zero(),
                    (next - v).normalize_or_zero(),
                );
                if geom::cross(v - prev, next - v) > 0.0 && a.angle_to(b).abs() < SHARPEST - 1e-6 {
                    sharp += 1;
                }
            }
        }
        assert_eq!(sharp, 0, "{sharp} corners sharper than 55°");
    }
}
