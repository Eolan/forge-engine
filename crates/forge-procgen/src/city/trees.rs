//! Trees of the plan (D-059's step 3, its first part): rows along the streets by profile, trees
//! scattered in the parks and a few in each courtyard (`city-generation.md` §10: street trees
//! every 8–10 m, the 3-30-300 rule's "three trees from every home"). Positions, heights and
//! crowns only; the species and their models come with the vegetation work.

use forge_core::SplitMix64;
use glam::DVec2;

use super::blocks::BlockUse;
use super::geom::{self, PointGrid};
use super::{CityPlan, LAND, Profile, Site};

/// Where a tree stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// In a row along a street.
    Street,
    /// In a park.
    Park,
    /// In a courtyard.
    Courtyard,
}

/// A tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    /// Where its trunk stands, metres on the plan.
    pub pos: DVec2,
    /// Its height, metres.
    pub height: f64,
    /// Its crown's radius, metres.
    pub crown: f64,
    /// Where it stands.
    pub setting: Setting,
}

/// The plan's trees.
pub fn trees(site: &Site, plan: &CityPlan, seed: u64) -> Vec<Tree> {
    let mut out = Vec::new();
    let mut rng = SplitMix64::new(seed);
    // Junctions: no tree within 10 m of one.
    let mut junctions = PointGrid::new(plan.size, 20.0);
    for (i, n) in plan.roads.nodes.iter().enumerate() {
        junctions.insert(n.pos, i as u32);
    }
    for e in &plan.roads.edges {
        if e.bridge {
            continue;
        }
        // Rows: offsets from the centreline and the spacing, by profile.
        let (rows, spacing): (&[f64], f64) = match e.profile {
            Profile::TramAvenue => (&[-0.5, 0.5], 9.0),
            Profile::Avenue => (&[-0.5, 0.5], 9.0),
            Profile::Shared => (&[0.5], 11.0),
            Profile::Pedestrian => (&[0.0], 18.0),
            Profile::Path => (&[], 0.0),
        };
        let half = e.profile.width() * 0.5;
        let length = e.length();
        for &side in rows {
            // Inside the pavement's edge: 2.5 m from the building line.
            let offset = side * 2.0 * (half - 2.5).max(0.0);
            let mut s = spacing * 0.5;
            while s < length {
                let p = geom::point_at(&e.points, s);
                let t = geom::tangent_at(&e.points, s);
                let q = p + DVec2::new(-t.y, t.x) * offset;
                s += spacing;
                if junctions.any_within(q, 10.0, |_| false) || site.water_at(q) != LAND {
                    continue;
                }
                out.push(Tree {
                    pos: q,
                    height: rng.range_f64(11.0, 16.0),
                    crown: rng.range_f64(3.5, 5.0),
                    setting: Setting::Street,
                });
            }
        }
    }
    // Parks: a jittered grid, two thirds filled, glades left open.
    for b in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Park) {
        scatter(
            site,
            &b.polygon,
            13.0,
            0.66,
            Setting::Park,
            &mut rng,
            &mut out,
        );
    }
    // Quays: the land between the streets and a river, a promenade under trees.
    for b in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Quay) {
        scatter(
            site,
            &b.polygon,
            14.0,
            0.45,
            Setting::Park,
            &mut rng,
            &mut out,
        );
    }
    // Courtyards: a looser grid.
    for b in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Built) {
        if let Some(c) = &b.courtyard {
            scatter(site, c, 16.0, 0.5, Setting::Courtyard, &mut rng, &mut out);
        }
    }
    out
}

/// Trees on a jittered grid of `step` metres inside a polygon, each cell filled with probability
/// `fill`, at least 4 m in from its edge.
fn scatter(
    site: &Site,
    polygon: &[DVec2],
    step: f64,
    fill: f64,
    setting: Setting,
    rng: &mut SplitMix64,
    out: &mut Vec<Tree>,
) {
    let (lo, hi) = polygon.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let n = polygon.len();
    let mut y = lo.y + step * 0.5;
    while y < hi.y {
        let mut x = lo.x + step * 0.5;
        while x < hi.x {
            let p = DVec2::new(
                x + rng.range_f64(-0.35, 0.35) * step,
                y + rng.range_f64(-0.35, 0.35) * step,
            );
            let keep = rng.next_f64() < fill;
            x += step;
            if !keep || !geom::contains(polygon, p) || site.water_at(p) != LAND {
                continue;
            }
            let edge = (0..n)
                .map(|i| geom::distance_to_segment(p, polygon[i], polygon[(i + 1) % n]))
                .fold(f64::MAX, f64::min);
            if edge < 4.0 {
                continue;
            }
            let (height, crown) = match setting {
                Setting::Park => (rng.range_f64(10.0, 22.0), rng.range_f64(4.0, 7.0)),
                _ => (rng.range_f64(7.0, 12.0), rng.range_f64(3.0, 4.5)),
            };
            out.push(Tree {
                pos: p,
                height,
                crown,
                setting,
            });
        }
        y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{example, plan};
    use super::*;

    #[test]
    fn trees_line_the_streets_and_fill_the_parks_on_land() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let trees = trees(&site, &city, 5);
        let street = trees
            .iter()
            .filter(|t| t.setting == Setting::Street)
            .count();
        let park = trees.iter().filter(|t| t.setting == Setting::Park).count();
        assert!(street > 10_000, "{street} street trees");
        assert!(park > 1_000, "{park} park trees");
        assert!(trees.iter().all(|t| site.water_at(t.pos) == LAND));
        // About one street tree per 10 m of street.
        let km = city.stats.main_km + city.stats.street_km;
        assert!((street as f64 / km) > 60.0, "{} a km", street as f64 / km);
    }
}
