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
