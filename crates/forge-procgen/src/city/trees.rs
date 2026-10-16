//! Trees of the plan (D-059's step 3, its first part): rows along the streets by profile, trees
//! scattered in the parks and a few in each courtyard (`city-generation.md` §10: street trees
//! every 8–10 m, the 3-30-300 rule's "three trees from every home"). Positions, heights and
//! crowns only; the species and their models come with the vegetation work.

use forge_core::SplitMix64;
use glam::DVec2;

use super::blocks::BlockUse;
use super::geom::{self, PointGrid};
use super::{CityPlan, DistrictKind, LAND, Profile, Site};

/// Where a tree stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// In a row along a street.
    Street,
    /// In a park.
    Park,
    /// In a courtyard.
    Courtyard,
    /// In the countryside's forest beyond the districts' reach.
    Forest,
    /// In a house's garden: the village's olives (#266).
    Garden,
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
    /// Its species, an index into `forge_flora::species::mediterranean()`, where the grower
    /// placed it (the forest, D-063); `None` for the one model.
    pub species: Option<u16>,
}

/// A street-workout park in each park of a hectare or more (the owner, 2026-10-11: workout
/// equipment, no playgrounds): its middle, the park's inside point, and its long axis, the
/// park's longest edge's.
pub fn workout_sites(plan: &CityPlan) -> Vec<(DVec2, DVec2)> {
    plan.blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Park && b.area >= 10_000.0)
        .map(|b| (b.centre, geom::oriented_extent(&b.polygon).0))
        .collect()
}

/// An avenue's kerb rows' slot spacing, metres: trees and lamps alternate along each row, so a
/// row has a tree every 16 m and a lamp halfway between (the owner, 2026-10-11: lamps and trees
/// in conflict, a bit too many trees; one of each in turn).
pub const KERB_SLOT: f64 = 8.0;

/// How far a street tree's crown keeps from the building line, metres: clear of the windows
/// (the owner, 2026-10-11: a canopy a few centimetres from the windows).
pub const CROWN_CLEAR: f64 = 2.5;

/// A kerb row: its offset from the centreline, and whether its slot `k` holds a lamp.
pub type KerbRow = (f64, fn(usize) -> bool);

/// An avenue's kerb rows (`None` for other profiles): each row's offset from the centreline (0.8
/// m into the pavement from the kerb), its slots every [`KERB_SLOT`] from half a slot in,
/// and on each row whether the slot `k` holds a lamp: odd slots on the left row, even ones on
/// the right, so the two sides stagger.
pub fn kerb_rows(profile: Profile) -> Option<[KerbRow; 2]> {
    match profile {
        Profile::TramAvenue | Profile::Avenue => {
            let kerb = (profile.width() * 0.5 - profile.pavement() + 0.8).max(0.0);
            Some([(-kerb, |k| k % 2 == 1), (kerb, |k| k % 2 == 0)])
        }
        _ => None,
    }
}

/// The plan's trees.
pub fn trees(site: &Site, plan: &CityPlan, seed: u64) -> Vec<Tree> {
    let mut out = Vec::new();
    let mut rng = SplitMix64::new(seed);
    // Junctions: no tree within 10 m of one.
    let mut placed = PointGrid::new(plan.size, 20.0);
    let mut junctions = PointGrid::new(plan.size, 20.0);
    for (i, n) in plan.roads.nodes.iter().enumerate() {
        junctions.insert(n.pos, i as u32);
    }
    for e in &plan.roads.edges {
        if e.bridge {
            continue;
        }
        // Rows: their offsets from the centreline and the spacing, by profile; and the trees'
        // heights and crowns: trimmed street trees on the avenues, smaller ones on shared streets,
        // small ones in the lanes. An avenue's rows stand by the kerb, 0.8 m into each
        // pavement (the owner, 2026-10-11: 2.5 m from the facades their crowns went into the
        // buildings; then, 4 m off, a canopy at the windows, which no one living there would
        // want); a shared street's and a lane's in their middle, off the walkers' lines.
        let half = e.profile.width() * 0.5;
        // An avenue's rows by the kerb, a tree in every other slot (the lamps take the rest).
        let lamp_free: fn(usize) -> bool = |_| false;
        let (rows, spacing, height, crown): (Vec<KerbRow>, f64, _, _) = match kerb_rows(e.profile) {
            Some(rows) => (rows.to_vec(), KERB_SLOT, (9.0, 12.0), (2.4, 3.2)),
            None => match e.profile {
                Profile::Shared => (vec![(0.0, lamp_free)], 14.0, (7.0, 10.0), (2.4, 3.4)),
                Profile::Pedestrian => (vec![(0.0, lamp_free)], 18.0, (5.0, 7.0), (1.8, 2.5)),
                _ => (Vec::new(), 0.0, (0.0, 0.0), (0.0, 0.0)),
            },
        };
        let length = e.length();
        for &(offset, lamp) in &rows {
            // The crown keeps [`CROWN_CLEAR`] from the building line.
            let room = (half - offset.abs() - CROWN_CLEAR).max(1.0);
            let mut s = spacing * 0.5;
            let mut slot = 0;
            while s < length {
                let p = geom::point_at(&e.points, s);
                let t = geom::tangent_at(&e.points, s);
                let q = p + DVec2::new(-t.y, t.x) * offset;
                s += spacing;
                slot += 1;
                if lamp(slot - 1)
                    || junctions.any_within(q, 10.0, |_| false)
                    || site.water_at(q) != LAND
                    // Never on a carriageway, nor beside another street's tree: two kerb rows
                    // met at a junction's corner, two trees in one spot (the owner).
                    || plan.ground.on_carriageway(q)
                    || placed.any_within(q, 6.0, |_| false)
                {
                    continue;
                }
                placed.insert(q, 0);
                out.push(Tree {
                    pos: q,
                    height: rng.range_f64(height.0, height.1),
                    crown: rng.range_f64(crown.0, crown.1).min(room),
                    setting: Setting::Street,
                    species: None,
                });
            }
        }
    }
    // Parks: a jittered grid, a little over half filled, glades left open (two thirds were a
    // bit many, the owner found).
    for b in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Park) {
        scatter(
            site,
            &b.polygon,
            13.0,
            0.55,
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
    // The open land, the reserves (the hills, the wetlands) and whatever lies outside every block:
    // a loose wood on a jittered grid of 16 m, under half filled, 8 m clear of every street. It
    // was bare ground the owner took for paving.
    let mut ways = PointGrid::new(plan.size, 40.0);
    for e in &plan.roads.edges {
        for p in geom::resample(&e.points, 6.0) {
            ways.insert(p, 0);
        }
    }
    // The blocks by the cells of 100 m their boxes touch.
    let cell = 100.0;
    let side = (plan.size / cell).ceil() as usize + 1;
    let mut by_cell: Vec<Vec<u32>> = vec![Vec::new(); side * side];
    let clampi = |v: f64| ((v / cell).floor().max(0.0) as usize).min(side - 1);
    for (i, b) in plan.blocks.iter().enumerate() {
        let (lo, hi) = b.polygon.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        for cy in clampi(lo.y)..=clampi(hi.y) {
            for cx in clampi(lo.x)..=clampi(hi.x) {
                by_cell[cy * side + cx].push(i as u32);
            }
        }
    }
    let in_block = |p: DVec2| {
        by_cell[clampi(p.y) * side + clampi(p.x)]
            .iter()
            .any(|&i| geom::contains(&plan.blocks[i as usize].polygon, p))
    };
    let step = 16.0;
    let mut y = step * 0.5;
    while y < plan.size {
        let mut x = step * 0.5;
        while x < plan.size {
            let p = DVec2::new(
                x + rng.range_f64(-0.35, 0.35) * step,
                y + rng.range_f64(-0.35, 0.35) * step,
            );
            let keep = rng.next_f64() < 0.45;
            x += step;
            let reserve = plan
                .district_at(p)
                .is_some_and(|d| d.kind == DistrictKind::Reserve);
            if !keep
                || site.water_at(p) != LAND
                || ways.any_within(p, 8.0, |_| false)
                || (!reserve && in_block(p))
            {
                continue;
            }
            out.push(Tree {
                pos: p,
                height: rng.range_f64(8.0, 16.0),
                crown: rng.range_f64(3.0, 5.5),
                setting: Setting::Park,
                species: None,
            });
        }
        y += step;
    }
    // Clear of the landmarks that stand on quays: 45 m round their quay's land point.
    let held: Vec<DVec2> = plan
        .landmarks
        .iter()
        .filter_map(|l| l.square)
        .filter(|&i| plan.blocks[i].use_ == BlockUse::Quay)
        .map(|i| plan.blocks[i].centre)
        .collect();
    out.retain(|t| held.iter().all(|c| c.distance(t.pos) > 45.0));
    // Clear of the workout parks: 11 m round each (each is 14 by 10 m).
    let workouts = workout_sites(plan);
    out.retain(|t| workouts.iter().all(|(c, _)| c.distance(t.pos) > 11.0));
    // Squares of 0.15 ha or more: a row of plane trees 3 m inside their building lines, every
    // 10 m, the middle left open; those of 0.5 ha or more a second row 11 m further in, an
    // alley of planes round them (a Provençal cours).
    for (index, b) in plan
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.use_ == BlockUse::Square && b.area >= 1500.0)
    {
        let Some(outer) = geom::inset(&b.buildable, 3.0) else {
            continue;
        };
        let mut rings = vec![outer.clone()];
        // A landmark's square keeps its outer row only: the building stands inside it.
        let holds_landmark = plan.landmarks.iter().any(|l| l.square == Some(index));
        if b.area >= 5000.0 && !holds_landmark {
            // The second row: the first drawn 11 m in towards the middle (a square is near
            // convex; an inset's corner checks refuse many).
            let c = geom::centroid(&outer);
            let reach = outer.iter().map(|p| p.distance(c)).sum::<f64>() / outer.len() as f64;
            if reach > 22.0 {
                let k = 1.0 - 11.0 / reach;
                rings.push(outer.iter().map(|&p| c + (p - c) * k).collect());
            }
        }
        for ring in rings {
            let mut closed = ring.clone();
            closed.push(ring[0]);
            for p in geom::resample(&closed, 10.0) {
                if site.water_at(p) != LAND || junctions.any_within(p, 6.0, |_| false) {
                    continue;
                }
                out.push(Tree {
                    pos: p,
                    height: rng.range_f64(10.0, 14.0),
                    crown: rng.range_f64(3.2, 4.2),
                    setting: Setting::Street,
                    species: None,
                });
            }
        }
    }
    // Courtyards: a looser grid.
    for b in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Built) {
        if let Some(c) = &b.courtyard {
            scatter(site, c, 16.0, 0.5, Setting::Courtyard, &mut rng, &mut out);
        }
    }
    // The parks', the quays' and the courtyards' trees off the roads and paths (a metre and a
    // half clear of their edges), and every tree off a river's bank (4 m clear of its water):
    // the owner saw trees on the water, the roads and the paths (2026-10-11).
    let mut ways: Vec<(f64, PointGrid)> = Vec::new();
    for e in &plan.roads.edges {
        let half = e.profile.width() * 0.5;
        let k = match ways.iter().position(|(h, _)| (*h - half).abs() < 0.01) {
            Some(k) => k,
            None => {
                ways.push((half, PointGrid::new(plan.size, 20.0)));
                ways.len() - 1
            }
        };
        for p in geom::resample(&e.points, 2.0) {
            ways[k].1.insert(p, 0);
        }
    }
    // The rivers' centrelines every 2 m, with the reach a tree keeps from each.
    let banks: Vec<(f64, PointGrid)> = site
        .rivers
        .iter()
        .map(|r| {
            let mut grid = PointGrid::new(plan.size, 40.0);
            for p in geom::resample(&r.centreline, 2.0) {
                grid.insert(p, 0);
            }
            (r.width * 0.5 + 4.0, grid)
        })
        .collect();
    out.retain(|t| {
        let near = |grids: &[(f64, PointGrid)], extra: f64| {
            grids
                .iter()
                .any(|(reach, grid)| grid.any_within(t.pos, reach + extra, |_| false))
        };
        // And 2 m from any water, the sea's and the lakes' too: on a quay's edge a trunk stood
        // over the water in its reflection (the owner, 2026-10-16: "tree above water").
        let dry = (0..8).all(|k| {
            let a = f64::from(k) * std::f64::consts::FRAC_PI_4;
            site.water_at(t.pos + DVec2::new(a.cos(), a.sin()) * 2.0) == LAND
        });
        !(t.setting != Setting::Street && near(&ways, 1.5)) && !near(&banks, 0.0) && dry
    });
    // And none where a lamp stands, whichever street's it is: the lamps take precedence.
    let mut lamps = PointGrid::new(plan.size, 20.0);
    for l in super::lamps::lamps(site, plan) {
        lamps.insert(l.pos, 0);
    }
    out.retain(|t| !lamps.any_within(t.pos, 3.0, |_| false));
    // None on a taxi depot's deck or by its driveway, nor in a tram depot's yard or by its link.
    out.retain(|t| {
        plan.depots
            .iter()
            .all(|d| !d.on_driveway(t.pos, 4.0) && !d.on_deck(t.pos, 3.0))
            && plan
                .tram_depots
                .iter()
                .all(|d| !d.on_link(t.pos, 3.0) && !d.on_yard(t.pos, 3.0))
    });
    // The countryside's forest (D-062: the first map's village in a local forest; D-063): on the
    // land beyond every district's reach, the Mediterranean canopy's wild species by their
    // habitats on the site's ground (`forge_flora::place::place`: the Aleppo pines on the dry
    // sunny slopes, the holm oaks in the valleys and the shade), placed once for the whole
    // region, never round the player; 12 m clear of the roads' centrelines and off the blocks.
    // `forest` stays the plan's switch; each tree keeps its species for the grower.
    if plan.forest.is_some() {
        let mut roads = PointGrid::new(plan.size, 20.0);
        for e in &plan.roads.edges {
            for p in geom::resample(&e.points, 5.0) {
                roads.insert(p, 0);
            }
        }
        let all = forge_flora::species::mediterranean();
        let wild: Vec<forge_flora::species::Species> = all
            .iter()
            .filter(|s| s.habitat.as_ref().is_some())
            .cloned()
            .collect();
        let ground = forge_flora::place::Ground::new(
            site.ground.size as usize,
            site.ground.spacing,
            site.ground.data.clone(),
        );
        let mut fields = forge_flora::place::Fields::of(&ground);
        // On flat ground the flow's wetness index reads every sample as a valley floor, deep
        // and moist, and the holm oak took the whole forest. There moisture, soil and the sun's
        // warmth follow the distance from the water instead, an assumption of this map: deep
        // moist alluvium by the river, the thin dry soil of a limestone plateau 300 m from it,
        // where the pines and the garrigue grow.
        let far = water_distance(site);
        let (wn, ws) = (site.water.size as usize, site.water.spacing);
        for j in 0..fields.side {
            for i in 0..fields.side {
                let s = &mut fields.sites[j * fields.side + i];
                if s.slope >= 3.0 {
                    continue;
                }
                let (x, y) = (i as f64 * fields.cell, j as f64 * fields.cell);
                let (wi, wj) = (
                    ((x / ws).round() as usize).min(wn - 1),
                    ((y / ws).round() as usize).min(wn - 1),
                );
                let t = (f64::from(far[wj * wn + wi]) / 300.0).clamp(0.0, 1.0);
                s.moisture = 0.8 - 0.6 * t;
                s.soil = 0.9 - 0.7 * t;
                s.heat = 0.5 + 0.12 * t;
            }
        }
        let keep_out = |p: DVec2| {
            p.x < 0.0
                || p.y < 0.0
                || p.x >= plan.size
                || p.y >= plan.size
                || site.water_at(p) != LAND
                || super::district_index(&plan.district_field, p).is_some()
                || roads.nearest(p, 12.0).is_some()
                || plan
                    .blocks
                    .iter()
                    .any(|b| b.centre.distance(p) < 400.0 && geom::contains(&b.polygon, p))
        };
        let placed =
            forge_flora::place::place(&ground, &fields, &wild, &[], rng.next_u64(), &keep_out);
        for plant in placed.plants {
            let id = &wild[usize::from(plant.species)].id;
            out.push(Tree {
                pos: plant.pos,
                height: f64::from(plant.height),
                crown: f64::from(plant.crown),
                setting: Setting::Forest,
                species: all.iter().position(|s| &s.id == id).map(|i| i as u16),
            });
        }
    }
    // The planted trees' species (#257, D-063): planes along the avenues and the shared streets
    // and round the squares, limes along the lanes and in the courts with olives, umbrella pines
    // within 30 m of the water, the parks a mix of planes, limes, umbrella pines and holm oaks.
    // Only where the plan grows a forest (the village's maps): the 4 km city's thousands of
    // street and park trees as grown meshes took 23–34 ms of GPU where the one model took 4.4–4.9
    // (the sandbox's day check); the city keeps its one model until D-013's far tiers.
    let species_here = plan.forest.is_some();
    let all = forge_flora::species::mediterranean();
    let id = |name: &str| all.iter().position(|s| s.id == name).map(|i| i as u16);
    let (plane, lime, pine, oak, olive) = (
        id("london-plane"),
        id("common-lime"),
        id("umbrella-pine"),
        id("holm-oak"),
        id("olive"),
    );
    let far = water_distance(site);
    let (wn, ws) = (site.water.size as usize, site.water.spacing);
    let by_water = |p: DVec2| {
        let (i, j) = (
            ((p.x / ws).round().max(0.0) as usize).min(wn - 1),
            ((p.y / ws).round().max(0.0) as usize).min(wn - 1),
        );
        far[j * wn + i] < 30.0
    };
    let mut roads = PointGrid::new(plan.size, 30.0);
    for (i, e) in plan.roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 8.0) {
            roads.insert(p, i as u32);
        }
    }
    for t in out
        .iter_mut()
        .filter(|t| species_here && t.species.is_none())
    {
        let hash = (t.pos.x.to_bits() ^ t.pos.y.to_bits().rotate_left(23))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            >> 60;
        t.species = match t.setting {
            Setting::Street if by_water(t.pos) => pine,
            Setting::Street => match roads
                .nearest(t.pos, 30.0)
                .map(|(_, e)| plan.roads.edges[e as usize].profile)
            {
                Some(Profile::Pedestrian) => lime,
                _ => plane,
            },
            Setting::Park => [plane, lime, pine, oak][(hash % 4) as usize],
            Setting::Courtyard => [lime, olive][(hash % 2) as usize],
            Setting::Forest | Setting::Garden => t.species,
        };
    }
    // The trees the plan's own generators planted: the village's gardens' olives.
    out.extend(plan.planted.iter().copied());
    out
}

/// Each cell of the site's water field's distance from the nearest water, metres (a two-pass
/// chamfer: 1 along a row, √2 across a corner).
fn water_distance(site: &Site) -> Vec<f32> {
    let n = site.water.size as usize;
    let s = site.water.spacing as f32;
    let mut d: Vec<f32> = (0..n * n)
        .map(|k| {
            if site.water.data[k] != LAND {
                0.0
            } else {
                f32::MAX
            }
        })
        .collect();
    let diag = s * std::f32::consts::SQRT_2;
    let steps = [(-1i64, 0i64, s), (0, -1, s), (-1, -1, diag), (1, -1, diag)];
    for pass in 0..2 {
        let order: Vec<usize> = if pass == 0 {
            (0..n * n).collect()
        } else {
            (0..n * n).rev().collect()
        };
        let sign = if pass == 0 { 1 } else { -1 };
        for k in order {
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            for &(di, dj, cost) in &steps {
                let (a, b) = (i + di * sign, j + dj * sign);
                if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                    continue;
                }
                let other = d[(b as usize) * n + a as usize];
                if other + cost < d[k] {
                    d[k] = other + cost;
                }
            }
        }
    }
    d
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
                species: None,
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
    fn trees_keep_off_the_ways_the_banks_and_the_lamps() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let trees = trees(&site, &city, 3);
        let lamps = super::super::lamps::lamps(&site, &city);
        // No tree where a lamp stands: an avenue's kerb rows take them in turn.
        for l in &lamps {
            assert!(
                trees.iter().all(|t| t.pos.distance(l.pos) > 3.0),
                "a tree on {}",
                l.pos
            );
        }
        assert!(lamps.iter().any(|l| l.double));
        for t in &trees {
            for r in &site.rivers {
                let line = &r.centreline;
                let d = line
                    .windows(2)
                    .map(|w| geom::distance_to_segment(t.pos, w[0], w[1]))
                    .fold(f64::MAX, f64::min);
                assert!(d > r.width * 0.5 + 3.0, "a tree on the river at {}", t.pos);
            }
            if t.setting == Setting::Street {
                continue;
            }
            for e in &city.roads.edges {
                let d = e
                    .points
                    .windows(2)
                    .map(|w| geom::distance_to_segment(t.pos, w[0], w[1]))
                    .fold(f64::MAX, f64::min);
                assert!(d > e.profile.width() * 0.5, "a tree on a road at {}", t.pos);
            }
        }
    }

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
