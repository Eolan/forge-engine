use glam::DVec2;

use super::blocks::BlockUse;
use super::{LAND, example, geom, plan};

/// The example city at 4 km, its masks at 10 m.
fn example_plan() -> (super::Site, super::CityPlan) {
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let params = example::params(size);
    let city = plan(&site, &params);
    (site, city)
}

#[test]
fn superblocks_are_cut_by_streets() {
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let mut params = example::params(size);
    let traced = plan(&site, &params);
    params.max_block = Some(1.6);
    let cut = plan(&site, &params);
    // Built blocks longer than 1.6 times their district's spacing, with a little slack.
    let long = |city: &super::CityPlan| {
        city.blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Built)
            .filter(|b| {
                let (_, extent, _) = geom::oriented_extent(&b.polygon);
                extent.x.max(extent.y) > 1.7 * city.districts[b.district].street_spacing
            })
            .count()
    };
    let (before, after) = (long(&traced), long(&cut));
    assert!(before > 0);
    assert!(
        after * 3 <= before,
        "{before} long blocks, {after} once cut"
    );
    assert!(cut.stats.built_blocks > traced.stats.built_blocks);
    // Every street, the cuts too, has a line.
    assert!(cut.roads.edges.iter().all(|e| e.points.len() >= 2));
}

#[test]
fn the_example_city_has_streets_blocks_parks_and_trams() {
    let (site, city) = example_plan();
    let st = &city.stats;
    assert!(st.main_km > 10.0, "{st:?}");
    assert!(st.street_km > 50.0, "{st:?}");
    assert!(st.built_blocks > 100, "{st:?}");
    assert!(st.park_blocks > 0, "{st:?}");
    assert!(st.bridges >= 2, "the main roads bridge the river: {st:?}");
    assert!(st.tram_km > 3.0 && st.stops >= 8, "{st:?}");
    assert!(st.lots > st.built_blocks, "{st:?}");
    for block in &city.blocks {
        assert_eq!(site.water_at(block.centre), LAND, "a block in the water");
        assert!(block.area > 0.0);
    }
    // The 3-30-300 rule's park within 300 m for nearly every built block.
    assert!(st.park_coverage > 0.9, "{st:?}");
}

#[test]
fn the_plan_is_the_same_every_time() {
    let (_, a) = example_plan();
    let (_, b) = example_plan();
    assert_eq!(a.stats.digest, b.stats.digest);
    assert_eq!(a.roads, b.roads);
}

#[test]
fn no_two_roads_cross_except_at_a_node() {
    let (_, city) = example_plan();
    let edges = &city.roads.edges;
    let mut crossings = 0;
    for (i, e) in edges.iter().enumerate() {
        for f in &edges[i + 1..] {
            for w in e.points.windows(2) {
                for v in f.points.windows(2) {
                    if let Some((t, u)) = geom::segment_intersection(w[0], w[1], v[0], v[1]) {
                        let p = w[0].lerp(w[1], t);
                        let at_node = city.roads.nodes.iter().any(|n| n.pos.distance(p) < 2.0);
                        if !at_node && t > 1e-6 && t < 1.0 - 1e-6 && u > 1e-6 && u < 1.0 - 1e-6 {
                            crossings += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(crossings, 0);
}

#[test]
fn courtyards_sit_inside_built_blocks() {
    let (_, city) = example_plan();
    let inner = city.lots.iter().filter(|l| !l.frontage).count();
    assert!(inner > 0);
    for lot in city.lots.iter().take(500) {
        let block = &city.blocks[lot.block as usize];
        assert_eq!(block.use_, BlockUse::Built);
        let c = geom::centroid(&lot.polygon);
        assert!(
            geom::contains(&block.polygon, c) || lot.area < 50.0,
            "a lot outside its block at {c:?}"
        );
    }
    let _ = DVec2::ZERO;
}

#[test]
fn added_lines_bring_stops_to_the_blocks_left_without() {
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let mut params = example::params(size);
    let before = plan(&site, &params).stats.stop_coverage;
    params.stop_target = 0.9;
    params.max_extra_lines = 6;
    let after = plan(&site, &params);
    assert!(
        after.stats.stop_coverage > before + 0.2,
        "{before} → {}",
        after.stats.stop_coverage
    );
    assert!(after.tram_lines.len() > params.tram_lines.len());
}

#[test]
fn the_street_ground_is_one_map_with_every_surface_once() {
    use super::ground;
    let (_, city) = example_plan();

    let map = &city.ground;
    let built = city
        .timings
        .iter()
        .find(|t| t.0 == "street ground")
        .map_or(0.0, |t| t.1 / 1e3);
    let area = |shapes: &ground::Shapes| -> f64 {
        shapes
            .iter()
            .flatten()
            .map(|c| geom::signed_area(c))
            .sum::<f64>()
    };
    let (mut road, mut walk, mut ramps) = (0.0, 0.0, 0.0);
    let mut overlaps = 0;
    for t in &map.tiles {
        let r = area(&t.carriageway) + area(&t.crossings) + area(&t.stripes);
        let w = area(&t.walk);
        let ra: f64 = t.ramps.iter().map(|(_, s)| area(s)).sum();
        road += r;
        walk += w;
        ramps += ra;
        // No point of the walking ground is also carriageway: each walking shape's middle,
        // where it lies in its shape.
        for c in t.walk.iter().flat_map(|s| s.first()) {
            let p = geom::centroid(c);
            if geom::contains(c, p) && ground::inside(&t.walk, p) && map.on_carriageway(p) {
                overlaps += 1;
            }
        }
    }
    let km2 = |a: f64| a / 1e6;
    println!(
        "street ground of the example ({:.1} s): carriageway {:.3} km², walking {:.3} km², \
         ramps {:.0} m², {} crossings, {} dropped kerbs",
        built,
        km2(road),
        km2(walk),
        ramps,
        map.crossings.len(),
        map.ramps.len()
    );
    assert_eq!(overlaps, 0);
    assert!(map.crossings.len() > 100, "{}", map.crossings.len());
    // The streets' area against the sum of their strips, roughly (the junctions overlap).
    let strips: f64 = city
        .roads
        .edges
        .iter()
        .map(|e| e.length() * e.profile.width())
        .sum();
    let total = road + walk + ramps;
    assert!(
        total < strips && total > 0.7 * strips,
        "{total:.0} against {strips:.0}"
    );
}

#[test]
fn the_network_has_one_place_for_each_junction() {
    let (site, city) = example_plan();
    let d = super::graph::defects(&city.roads, &site);
    println!("the example's network: {d:?}");
    // Before the tidy (2026-10-14): 7 close junctions, 7 short streets, 5 sharp meetings, 1
    // doubled street.
    assert_eq!(
        (d.close_junctions, d.short_streets, d.sharp_meetings),
        (0, 0, 0),
        "{d:?}"
    );
    assert_eq!((d.wet_nodes, d.close_bridges), (0, 0), "{d:?}");
    assert!(d.doubled_streets <= 1, "{d:?}");
}

#[test]
fn the_street_ground_triangulates_whole() {
    use super::ground;
    let (_, city) = example_plan();
    let (mut whole, mut short, mut lost) = (0, 0, 0.0);
    for t in &city.ground.tiles {
        for shapes in [&t.walk, &t.carriageway] {
            for polygon in ground::simple_polygons(shapes) {
                let area = geom::signed_area(&polygon).abs();
                let covered: f64 = ground::triangles(&polygon)
                    .iter()
                    .map(|tri| geom::signed_area(tri).abs())
                    .sum();
                if covered < area * 0.999 - 0.01 {
                    short += 1;
                    lost += area - covered;
                } else {
                    whole += 1;
                }
            }
        }
    }
    println!("street ground polygons: {whole} whole, {short} short of {lost:.0} m²");
    assert_eq!(short, 0, "{short} polygons short of {lost:.0} m²");
}
