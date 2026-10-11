//! Street lamps of the plan: along each street by its profile, at the pavement's edge, none in
//! the junctions. Where Phase 4's many lights will stand (D-059's step 5); shielded and dimmed
//! when the street is empty, as the style bible's lighting asks.

use glam::DVec2;

use super::geom::{self, PointGrid};
use super::{CityPlan, LAND, Profile, Site};

/// A street lamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    /// Where its pole stands, metres on the plan.
    pub pos: DVec2,
    /// The unit direction its arm reaches, towards the street's centre.
    pub facing: DVec2,
    /// Its height, metres.
    pub height: f64,
}

/// No two lamps closer than this, metres: a street's lamps stop where another's already light.
const APART: f64 = 12.0;

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
        // Sides, spacing and height by profile; staggered on the two sides.
        let (sides, spacing, height): (&[f64], f64, f64) = match e.profile {
            Profile::TramAvenue | Profile::Avenue => (&[-1.0, 1.0], 28.0, 8.0),
            Profile::Shared => (&[1.0], 30.0, 6.0),
            Profile::Pedestrian => (&[1.0], 24.0, 4.5),
            Profile::Path => (&[1.0], 35.0, 4.0),
        };
        let half = e.profile.width() * 0.5;
        let length = e.length();
        for (k, &side) in sides.iter().enumerate() {
            let offset = side * (half - 0.8).max(0.5);
            let mut s = spacing * (0.25 + 0.5 * k as f64);
            while s < length {
                let p = geom::point_at(&e.points, s);
                let t = geom::tangent_at(&e.points, s);
                let normal = DVec2::new(-t.y, t.x);
                let q = p + normal * offset;
                s += spacing;
                let in_junction = junctions.any_within(q, 40.0, |n| {
                    plan.roads.nodes[n as usize].pos.distance(q) >= reach[n as usize] + 4.0
                });
                if in_junction || placed.any_within(q, APART, |_| false) || site.water_at(q) != LAND
                {
                    continue;
                }
                placed.insert(q, out.len() as u32);
                out.push(Lamp {
                    pos: q,
                    facing: -normal * side.signum(),
                    height,
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
