//! Street lamps of the plan: along each street by its profile, between the trees by an avenue's
//! kerb and at the pavement's edge elsewhere, none in the junctions. Where Phase 4's many lights
//! will stand (D-059's step 5); shielded and dimmed when the street is empty, as the style
//! bible's lighting asks.

use glam::DVec2;

use super::geom::{self, PointGrid};
use super::{CityPlan, LAND, Profile, Site, trees};

/// A street lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    /// Where its pole stands, metres on the plan.
    pub pos: DVec2,
    /// The unit direction its arm reaches, towards the street's centre.
    pub facing: DVec2,
    /// Its height, metres.
    pub height: f64,
    /// Whether a second arm reaches back over the pavement (an avenue's kerb lamps).
    pub double: bool,
}

/// A row of lamps along a street: its offset from the centreline, its spacing, the lamps' height,
/// and on an avenue whether its kerb slot `k` holds a lamp.
type LampRow = (f64, f64, f64, Option<fn(usize) -> bool>);

/// No two lamps closer than this, metres: a street's lamps stop where another's already light.
/// Across an avenue's 6 m carriageway the two kerb rows' staggered lamps stand 11 m apart.
const APART: f64 = 10.0;

/// The plan's street lamps.
pub fn lamps(site: &Site, plan: &CityPlan) -> Vec<Lamp> {
    let mut out = Vec::new();
    // A junction keeps clear as far as its widest street's half width and a little: an
    // avenue's lamps stand 11 m off its centreline, beyond any fixed radius round the node, and
    // two streets' lamps met in its corners, side by side (the owner's walk, 2026-10-11).
    let mut reach = vec![0.0f64; plan.roads.nodes.len()];
    for e in &plan.roads.edges {
        for n in [e.a, e.b] {
            let r = &mut reach[n as usize];
            *r = r.max(e.profile.width() * 0.5);
        }
    }
    let mut junctions = PointGrid::new(plan.size, 40.0);
    for (i, n) in plan.roads.nodes.iter().enumerate() {
        junctions.insert(n.pos, i as u32);
    }
    let mut placed = PointGrid::new(plan.size, APART);
    for e in &plan.roads.edges {
        let half = e.profile.width() * 0.5;
        let length = e.length();
        // An avenue's lamps stand in its kerb rows' free slots, between the trees, lighting the
        // carriageway and the pavement from two arms (the owner, 2026-10-11: by the road, one
        // of each in turn with the trees). Other streets: one side, at the pavement's edge.
        let rows: Vec<LampRow> = match trees::kerb_rows(e.profile) {
            Some(rows) => rows
                .iter()
                .map(|&(offset, lamp)| (offset, trees::KERB_SLOT, 8.0, Some(lamp)))
                .collect(),
            None => {
                let (spacing, height) = match e.profile {
                    Profile::Shared => (30.0, 6.0),
                    Profile::Pedestrian => (24.0, 4.5),
                    _ => (35.0, 4.0),
                };
                vec![((half - 0.8).max(0.5), spacing, height, None)]
            }
        };
        for (offset, spacing, height, slots) in rows {
            let mut s = if slots.is_some() {
                spacing * 0.5
            } else {
                spacing * 0.25
            };
            let mut slot = 0;
            while s < length {
                let p = geom::point_at(&e.points, s);
                let t = geom::tangent_at(&e.points, s);
                let normal = DVec2::new(-t.y, t.x);
                let q = p + normal * offset;
                s += spacing;
                slot += 1;
                if slots.is_some_and(|lamp| !lamp(slot - 1)) {
                    continue;
                }
                let in_junction = junctions.any_within(q, 40.0, |n| {
                    plan.roads.nodes[n as usize].pos.distance(q) >= reach[n as usize] + 4.0
                });
                // Not in a taxi stand's lay-by either: it is carriageway.
                if in_junction
                    || placed.any_within(q, APART, |_| false)
                    || site.water_at(q) != LAND
                    || plan.ground.on_carriageway(q)
                {
                    continue;
                }
                placed.insert(q, out.len() as u32);
                out.push(Lamp {
                    pos: q,
                    facing: -normal * offset.signum(),
                    height,
                    double: slots.is_some(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{example, plan};
    use super::*;

    #[test]
    fn lamps_line_the_streets_on_land() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let lamps = lamps(&site, &city);
        let km = city.stats.main_km + city.stats.street_km;
        assert!(
            lamps.len() as f64 / km > 25.0,
            "{} a km",
            lamps.len() as f64 / km
        );
        assert!(lamps.iter().all(|l| site.water_at(l.pos) == LAND));
        // Never two side by side, nor in a junction's corners.
        for (i, a) in lamps.iter().enumerate() {
            assert!(
                lamps[i + 1..]
                    .iter()
                    .all(|b| a.pos.distance(b.pos) >= APART),
                "two lamps within {APART} m at {}",
                a.pos
            );
        }
        let mut reach = vec![0.0f64; city.roads.nodes.len()];
        for e in &city.roads.edges {
            for n in [e.a, e.b] {
                reach[n as usize] = reach[n as usize].max(e.profile.width() * 0.5);
            }
        }
        for (n, r) in city.roads.nodes.iter().zip(&reach) {
            assert!(lamps.iter().all(|l| l.pos.distance(n.pos) >= r + 4.0));
        }
    }
}
