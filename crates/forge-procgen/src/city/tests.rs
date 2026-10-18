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

/// The rules that make the streets, the ground and the buildings hold on other seeds too, not
/// only the one the views were checked on (the owner: "or else it will just go back if we change
/// a param or the seed").
#[test]
fn the_rules_hold_on_other_seeds() {
    use super::ground;
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let mut failures = Vec::new();
    for seed in [0x5EED_0001_u64, 0x5EED_0042, 0xC17E_0003, 0xBEEF_0007] {
        let mut params = example::params(size);
        params.seed = forge_core::Seed::new(seed);
        let city = plan(&site, &params);
        let d = super::graph::defects(&city.roads, &site);
        // The street ground drawn whole.
        let mut short = 0;
        for t in &city.ground.tiles {
            for shapes in [&t.walk, &t.carriageway, &t.crossings] {
                for polygon in ground::simple_polygons(shapes) {
                    let area = geom::signed_area(&polygon).abs();
                    let covered: f64 = ground::triangles(&polygon)
                        .iter()
                        .map(|tri| geom::signed_area(tri).abs())
                        .sum();
                    if covered < area * 0.999 - 0.01 {
                        short += 1;
                    }
                }
            }
        }
        // Everyone reaches their door and their court from the street.
        let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), seed);
        let structures = super::structure::structures(&city, &masses);
        let (doors, courts) = super::structure::unreached(&city, &masses, &structures);
        println!(
            "seed {seed:#x}: {d:?}; {short} polygons short; {} doors and {} courts unreached; {} masses",
            doors.len(),
            courts.len(),
            masses.len()
        );
        let ok = d.close_junctions == 0
            && d.short_streets == 0
            && d.wet_nodes == 0
            && d.close_bridges == 0
            && d.sharp_meetings == 0
            && d.doubled_streets == 0
            && short == 0
            && doors.is_empty()
            && courts.is_empty();
        if !ok {
            failures.push(seed);
        }
    }
    assert!(failures.is_empty(), "the rules fail on seeds {failures:x?}");
}

#[test]
fn taxis_wait_within_a_short_walk_of_every_block() {
    let (site, city) = example_plan();
    let stands = &city.taxi_stands;
    let built: Vec<DVec2> = city
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Built)
        .map(|b| b.centre)
        .collect();
    let near = |p: DVec2| {
        stands
            .iter()
            .map(|s| s.kerb.distance(p))
            .fold(f64::MAX, f64::min)
    };
    let far = built
        .iter()
        .filter(|b| near(**b) > super::taxi::REACH)
        .count();
    println!(
        "{} taxi stands; {far} of {} built blocks farther than {} m from one",
        stands.len(),
        built.len(),
        super::taxi::REACH
    );
    assert!(stands.len() > 10, "{} stands", stands.len());
    // Each on land, apart from the others, on a taxi road's carriageway level.
    for (i, s) in stands.iter().enumerate() {
        assert_eq!(site.water_at(s.kerb), LAND);
        for o in &stands[i + 1..] {
            assert!(s.kerb.distance(o.kerb) >= 199.0);
        }
        let inside = s.kerb + s.out * 1.0;
        assert_eq!(
            city.ground.level_at(inside),
            super::ground::ROAD_LEVEL,
            "lay-by at {inside}"
        );
    }
    assert!(far * 20 <= built.len(), "{far} blocks far from a stand");
}

#[test]
fn shutters_fold_back_clear_of_corners_and_of_each_other() {
    use super::structure::{OpeningKind, PIER};
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let (mut windows, mut at_ends, mut over) = (0, 0, 0);
    for s in &structures {
        for (w, wall) in s.walls.iter().enumerate() {
            for (k, stretches) in wall.open.iter().enumerate() {
                // Each window's span with its shutters folded back either side; the garden flats'
                // glazed bays (to the floor) have none.
                let mut spans: Vec<(f64, f64)> = s
                    .openings
                    .iter()
                    .filter(|o| {
                        o.wall == w
                            && o.storey == k
                            && o.kind == OpeningKind::Window
                            && o.sill > 0.0
                    })
                    .map(|o| {
                        let reach = o.width;
                        (o.along - reach, o.along + reach)
                    })
                    .collect();
                spans.sort_by(|a, b| a.0.total_cmp(&b.0));
                windows += spans.len();
                for &(a, b) in &spans {
                    let inside = stretches
                        .iter()
                        .any(|&(lo, hi)| a >= lo + PIER - 1e-6 && b <= hi - PIER + 1e-6);
                    at_ends += usize::from(!inside);
                }
                over += spans.windows(2).filter(|p| p[1].0 < p[0].1 - 1e-6).count();
            }
        }
    }
    println!(
        "{windows} windows; {at_ends} with a shutter at a stretch's end, {over} over a neighbour's"
    );
    assert!(windows > 1000, "{windows} windows");
    assert_eq!(
        at_ends, 0,
        "{at_ends} windows' shutters within {PIER} m of a corner or an end"
    );
    assert_eq!(over, 0, "{over} windows' shutters over their neighbours'");
}

#[test]
fn taxi_depots_stand_behind_the_building_lines_with_level_footways() {
    use super::ground::{ROAD_LEVEL, WALK_LEVEL};
    let (_, city) = example_plan();
    let depots = &city.depots;
    let industrial = depots
        .iter()
        .filter(|d| {
            city.districts
                .get(city.blocks[d.block as usize].district)
                .is_some_and(|x| x.kind == super::DistrictKind::Industrial)
        })
        .count();
    println!(
        "{} depots for {} stands, {industrial} in industrial districts, {} cabs' room",
        depots.len(),
        city.taxi_stands.len(),
        depots.iter().map(|d| d.capacity()).sum::<usize>()
    );
    assert!(
        depots.len() >= city.taxi_stands.len() / 25,
        "{} depots",
        depots.len()
    );
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    for (i, d) in depots.iter().enumerate() {
        // Its deck off the streets, its block without flats.
        let deck = d.footprint();
        let off: f64 = city
            .ground
            .off_streets(&deck)
            .iter()
            .map(|s| geom::signed_area(&s[0]).abs())
            .sum();
        let area = geom::signed_area(&deck).abs();
        assert!(
            off > area * 0.99,
            "depot {i}: {off:.0} of {area:.0} m² off the streets"
        );
        assert!(
            masses
                .iter()
                .all(|m| city.lots[m.lot as usize].block != d.block),
            "depot {i}: flats in its block"
        );
        // Both driveways, the way in and the way out, up the verge from the carriageway; the
        // footway across them level.
        for kerb in [d.kerb, d.exit_kerb()] {
            let foot = city.ground.level_at(kerb + d.out * 0.2);
            let footway = city.ground.level_at(kerb + d.out * (d.front - 1.0));
            assert!(
                foot < WALK_LEVEL - 0.05,
                "depot {i}: its ramp's foot at {foot:.2}"
            );
            assert!(foot >= ROAD_LEVEL, "depot {i}: below the road");
            assert_eq!(
                footway, WALK_LEVEL,
                "depot {i}: the footway at {footway:.2}"
            );
        }
        for o in &depots[i + 1..] {
            assert!(d.kerb.distance(o.kerb) >= 999.0, "two depots close");
        }
    }
}

#[test]
fn a_city_has_about_as_many_shops_as_a_real_one() {
    // `research/ground-floors.md` §4.1: 26–30 active ground floors per 1 000 people (Paris 28,
    // Marseille 25): here, the shopfronts per 1 000 residents.
    use super::structure::OpeningKind;
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let shopfronts: usize = structures
        .iter()
        .map(|s| {
            s.openings
                .iter()
                .filter(|o| o.kind == OpeningKind::Shopfront)
                .count()
        })
        .sum();
    let people = structures.iter().map(|s| s.net).sum::<f64>() * (2.0 / 3.0) / 32.4;
    let per_thousand = shopfronts as f64 / people * 1000.0;
    println!("{shopfronts} shopfronts for {people:.0} people: {per_thousand:.1} per 1 000");
    assert!(
        (20.0..40.0).contains(&per_thousand),
        "{per_thousand:.1} shopfronts per 1 000 people"
    );
}

/// Whether `p` lies inside any of `outlines` (a box test first).
fn inside_any<'a>(outlines: impl IntoIterator<Item = &'a Vec<DVec2>>, p: DVec2) -> bool {
    outlines.into_iter().any(|o| {
        let (lo, hi) = o.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), &q| (lo.min(q), hi.max(q)),
        );
        p.x >= lo.x && p.y >= lo.y && p.x <= hi.x && p.y <= hi.y && geom::contains(o, p)
    })
}

#[test]
fn court_gardens_keep_clear_of_the_buildings_the_paths_and_the_doors() {
    // `research/ground-floors.md` §4.7–4.8: about half the court-side flats with a garden, the
    // rest with a terrace, each opening onto it through a glazed bay; a kitchen garden in the
    // large courts.
    use super::gardens::{self, Kind};
    use super::structure::{OpeningKind, WallRole};
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let pieces = gardens::gardens(&city, &masses, &structures);
    let flats = gardens::flats(&city, &masses, &structures);
    let count = |k: Kind| pieces.iter().filter(|p| p.kind == k).count();
    let gardened = flats.iter().filter(|f| f.garden).count();
    let bays = flats
        .iter()
        .filter(|f| {
            structures[f.mass as usize].openings.iter().any(|o| {
                o.wall == f.wall
                    && o.storey == 0
                    && o.kind == OpeningKind::Window
                    && o.sill == 0.0
                    && o.along > f.from
                    && o.along < f.to
            })
        })
        .count();
    println!(
        "{} court-side flats, {gardened} with a garden, {bays} with a glazed bay; {} paths, {} \
         hedges, {} kitchen gardens of {} beds, {} planters",
        flats.len(),
        count(Kind::Path),
        count(Kind::Hedge),
        count(Kind::Plot),
        count(Kind::Bed),
        count(Kind::Planter)
    );
    assert!(flats.len() > 200, "{} flats", flats.len());
    let share = gardened as f64 / flats.len() as f64;
    assert!(
        (0.4..0.6).contains(&share),
        "{share:.2} of the flats gardened"
    );
    assert!(
        bays * 10 >= flats.len() * 9,
        "{bays} bays for {} flats",
        flats.len()
    );
    assert!(
        count(Kind::Plot) >= 5,
        "{} kitchen gardens",
        count(Kind::Plot)
    );
    assert_eq!(count(Kind::Bed) % 4, 0);
    // The paths, by block.
    let mut paths: std::collections::HashMap<u32, Vec<&Vec<DVec2>>> = Default::default();
    for p in pieces.iter().filter(|p| p.kind == Kind::Path) {
        paths.entry(p.block).or_default().push(&p.outline);
    }
    let footprints: Vec<Vec<DVec2>> = masses.iter().map(|m| m.footprint.clone()).collect();
    let (mut on_path, mut in_building) = (0, 0);
    for p in pieces.iter().filter(|p| p.kind != Kind::Path) {
        // Its corners a little in, and its middle.
        let middle = geom::centroid(&p.outline);
        let probes: Vec<DVec2> = p
            .outline
            .iter()
            .map(|&q| q + (middle - q).normalize_or_zero() * 0.1)
            .chain([middle])
            .collect();
        if p.kind.standing() {
            let mine = paths.get(&p.block).cloned().unwrap_or_default();
            if let Some(q) = probes
                .iter()
                .find(|&&q| inside_any(mine.iter().copied(), q))
            {
                println!("a {:?} of block {} on a path at {q:.1}", p.kind, p.block);
                on_path += 1;
            }
        }
        if let Some(q) = probes.iter().find(|&&q| inside_any(&footprints, q)) {
            println!(
                "a {:?} of block {} in a building at {q:.1}",
                p.kind, p.block
            );
            in_building += 1;
        }
    }
    assert_eq!(on_path, 0, "{on_path} planters, hedges or beds on a path");
    assert_eq!(in_building, 0, "{in_building} pieces inside a building");
    // Every court door and passage opens onto a path, not into a garden.
    let mut blocked = 0;
    for (m, s) in masses.iter().zip(&structures) {
        let block = city.lots[m.lot as usize].block;
        for o in &s.openings {
            let wall = &s.walls[o.wall];
            if o.storey != 0
                || wall.role != WallRole::Court
                || !matches!(o.kind, OpeningKind::Door | OpeningKind::Passage)
                || !paths.contains_key(&block)
            {
                continue;
            }
            let t = (wall.b - wall.a).normalize_or_zero();
            let mouth = wall.a + t * o.along;
            let n = DVec2::new(t.y, -t.x);
            let n = if geom::contains(&m.footprint, mouth + n * 0.3) {
                -n
            } else {
                n
            };
            let step = mouth + n * 1.0;
            let standing = pieces
                .iter()
                .filter(|p| p.block == block && (p.kind.standing() || p.kind == Kind::Lawn))
                .map(|p| &p.outline);
            blocked += usize::from(inside_any(standing, step));
        }
    }
    assert_eq!(
        blocked, 0,
        "{blocked} court doors or passages into a garden"
    );
}

#[test]
fn tram_depots_stand_behind_the_building_lines_near_the_lines_ends() {
    // The owner: one or more tram depots for maintenance and stabling
    // (`research/transit-and-vehicles.md` §10: about one for three lines, near their ends).
    use super::ground::WALK_LEVEL;
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let mut total = 0;
    for seed in [None, Some(0x5EED_0001_u64), Some(0x5EED_0042)] {
        let mut params = example::params(size);
        if let Some(seed) = seed {
            params.seed = forge_core::Seed::new(seed);
        }
        let city = plan(&site, &params);
        let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
        for (i, d) in city.tram_depots.iter().enumerate() {
            let line = &city.tram_lines[d.line];
            let length = geom::polyline_length(&line.points);
            // Its link leaves near one of the line's ends.
            let start = d.link()[0];
            let along = (0..=(length / 5.0) as usize)
                .map(|k| k as f64 * 5.0)
                .min_by(|&a, &b| {
                    geom::point_at(&line.points, a)
                        .distance(start)
                        .total_cmp(&geom::point_at(&line.points, b).distance(start))
                })
                .unwrap_or(0.0);
            let from_end = along.min(length - along);
            println!(
                "seed {seed:?} depot {i}: line {}, {} tracks of {:.0} m, {} trams, {from_end:.0} m \
                 from the line's end",
                d.line,
                d.tracks,
                d.length,
                d.capacity()
            );
            assert!(d.capacity() >= 6, "{} places", d.capacity());
            assert!(from_end <= 900.0, "{from_end:.0} m from the line's end");
            // No building over its yard, none of the line's stops by its link.
            for m in &masses {
                assert!(
                    !m.footprint.iter().any(|&p| d.on_yard(p, 0.0)),
                    "a building in a depot's yard"
                );
            }
            for &s in &line.stops {
                assert!(!d.on_link(s, 30.0), "a stop on a depot's link");
            }
            // The footway it crosses stays level; the yard is at the walking level.
            let yard = d.yard();
            let c = geom::centroid(&yard);
            assert!(city.ground.level_at(c) == WALK_LEVEL || !city.ground.on_carriageway(c));
            total += 1;
        }
    }
    assert!(total >= 2, "{total} depots in three cities");
}

#[test]
fn court_paths_join_round_the_middle_instead_of_running_side_by_side() {
    // The owner: close paths would first join the paths round the garden, like a hub; separate
    // ones side by side waste the ground. Measured on each path's middle line, past its first
    // 6 m (straight out of its door past the private gardens) and short of its last 3 m (where
    // it joins): the share within 4 m of another path's middle line.
    use super::gardens::{self, Kind};
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let pieces = gardens::gardens(&city, &masses, &structures);
    // A path's middle line from its outline (its left side, then its right side back).
    let middle = |o: &Vec<DVec2>| -> Vec<DVec2> {
        let n = o.len();
        (0..n / 2).map(|i| (o[i] + o[n - 1 - i]) * 0.5).collect()
    };
    let mut lines: std::collections::BTreeMap<u32, Vec<Vec<DVec2>>> = Default::default();
    for p in pieces
        .iter()
        .filter(|p| p.kind == Kind::Path && p.outline.len() >= 6)
    {
        // Not the round places at the middles.
        let c = geom::centroid(&p.outline);
        let r = p.outline[0].distance(c);
        if p.outline.len() == 12 && p.outline.iter().all(|q| (q.distance(c) - r).abs() < 1e-6) {
            continue;
        }
        lines.entry(p.block).or_default().push(middle(&p.outline));
    }
    let (mut near, mut total) = (0, 0);
    for list in lines.values() {
        // Every path's ends: its door and where it joins.
        let ends: Vec<DVec2> = list.iter().flat_map(|l| [l[0], l[l.len() - 1]]).collect();
        for (i, line) in list.iter().enumerate() {
            let length = geom::polyline_length(line);
            let mut along = 0.0;
            for k in 0..line.len() {
                if k > 0 {
                    along += line[k].distance(line[k - 1]);
                }
                let at_junction = ends.iter().any(|e| e.distance(line[k]) < 8.0);
                if along < 6.0 || along > length - 3.0 || at_junction {
                    continue;
                }
                total += 1;
                let close = list.iter().enumerate().any(|(j, other)| {
                    j != i
                        && other
                            .windows(2)
                            .any(|w| geom::distance_to_segment(line[k], w[0], w[1]) < 4.0)
                });
                near += usize::from(close);
            }
        }
    }
    let share = near as f64 / total.max(1) as f64;
    println!("{near} of {total} path samples within 4 m of another path: {share:.3}");
    assert!(total > 1000, "{total} samples");
    assert!(share < 0.05, "{share:.3} of the paths run beside another");
}

#[test]
fn court_benches_face_their_middles_paving_from_the_green() {
    // Benches round the green courts' middles: each on the green (no path under it, the
    // gardens test checks), its front a little off the middle's paving, facing it.
    use super::gardens::{self, Kind};
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let pieces = gardens::gardens(&city, &masses, &structures);
    let benches: Vec<&gardens::Piece> = pieces.iter().filter(|p| p.kind == Kind::Bench).collect();
    let courts: std::collections::BTreeSet<u32> = benches.iter().map(|p| p.block).collect();
    let mut astray = 0;
    for b in &benches {
        let o = &b.outline;
        let front = o[1] - o[0];
        assert!((front.length() - gardens::BENCH.0).abs() < 1e-9);
        // Half a metre before its front's middle: the paving it faces.
        let face = DVec2::new(front.y, -front.x).normalize();
        let ahead = (o[0] + o[1]) * 0.5 + face * 0.5;
        let paved = pieces
            .iter()
            .filter(|p| p.block == b.block && p.kind.paved())
            .any(|p| geom::contains(&p.outline, ahead));
        if !paved {
            println!("a bench of block {} faces no paving at {ahead:.1}", b.block);
            astray += 1;
        }
    }
    println!("{} benches in {} courts", benches.len(), courts.len());
    assert!(courts.len() >= 20, "{} courts with benches", courts.len());
    assert!(
        benches.len() >= 3 * courts.len(),
        "{} benches",
        benches.len()
    );
    assert_eq!(astray, 0, "{astray} benches facing no paving");
}

#[test]
fn open_blocks_gaps_are_hedged_behind_their_forecourts_but_where_their_paths_go_in() {
    // The example with open blocks (`DistrictSpec::gaps`, the sandbox's): each gap's street
    // side gets a low hedge 2 m in from the building line, none across a court path.
    use super::gardens::{self, FORECOURT, Kind};
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let mut params = example::params(size);
    for d in &mut params.districts {
        d.gaps = Some(0.25);
    }
    let city = plan(&site, &params);
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let pieces = gardens::gardens(&city, &masses, &structures);
    let low: Vec<&gardens::Piece> = pieces
        .iter()
        .filter(|p| p.kind == Kind::Hedge && (p.height - 0.8).abs() < 1e-9)
        .collect();
    let built: std::collections::HashSet<u32> = masses.iter().map(|m| m.lot).collect();
    let gap_lots: Vec<usize> = city
        .lots
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            l.frontage
                && !built.contains(&(*i as u32))
                && city.blocks[l.block as usize].courtyard.is_some()
        })
        .map(|(i, _)| i)
        .collect();
    // Those standing in a gap: a hedge's middle inside a gap lot.
    let in_gaps = low
        .iter()
        .filter(|h| {
            let c = geom::centroid(&h.outline);
            gap_lots
                .iter()
                .any(|&i| geom::contains(&city.lots[i].polygon, c))
        })
        .count();
    let footprints: Vec<Vec<DVec2>> = masses.iter().map(|m| m.footprint.clone()).collect();
    let paths: Vec<&Vec<DVec2>> = pieces
        .iter()
        .filter(|p| p.kind == Kind::Path)
        .map(|p| &p.outline)
        .collect();
    let mut wrong = 0;
    for h in &low {
        let c = geom::centroid(&h.outline);
        wrong += usize::from(inside_any(&footprints, c) || inside_any(paths.iter().copied(), c));
    }
    println!(
        "{} gap lots, {in_gaps} hedges in them (forecourt {FORECOURT} m), {wrong} wrong",
        gap_lots.len()
    );
    assert!(gap_lots.len() > 20, "{} gap lots", gap_lots.len());
    assert!(
        in_gaps >= gap_lots.len() / 2,
        "{in_gaps} hedges for {} gaps",
        gap_lots.len()
    );
    assert_eq!(wrong, 0, "{wrong} gap hedges in a building or on a path");
    // A bike rack before a hedge in most gaps: on the forecourt, clear of the buildings and the
    // paths, the footway kept (its back 0.4 m off the hedge's front, the street side 1.6 m and
    // more from the building line).
    let racks: Vec<&gardens::Piece> = pieces.iter().filter(|p| p.kind == Kind::BikeRack).collect();
    let mut astray = 0;
    for r in &racks {
        let c = geom::centroid(&r.outline);
        let gap = gap_lots
            .iter()
            .find(|&&i| geom::contains(&city.lots[i].polygon, c));
        let on_forecourt = gap.is_some_and(|&i| {
            let lot = &city.lots[i].polygon;
            let n = lot.len();
            let edge = (0..n)
                .map(|k| geom::distance_to_segment(c, lot[k], lot[(k + 1) % n]))
                .fold(f64::MAX, f64::min);
            (1.6..FORECOURT).contains(&edge)
        });
        let clear = r
            .outline
            .iter()
            .all(|&q| !inside_any(&footprints, q) && !inside_any(paths.iter().copied(), q));
        if !on_forecourt || !clear {
            println!("a bike rack astray at {c:.1}");
            astray += 1;
        }
    }
    println!("{} bike racks", racks.len());
    assert!(racks.len() >= gap_lots.len() / 3, "{} racks", racks.len());
    assert_eq!(astray, 0, "{astray} racks off their forecourts");
}

#[test]
fn planters_line_the_homes_on_the_streets_and_leave_their_doors_clear() {
    // `research/ground-floors.md` §4.6: the homes on the streets behind a planted strip, never
    // raised; none before a shop, and every door and passage clear.
    use super::gardens::{self, Kind, PLANTER};
    use super::structure::{OpeningKind, WallRole};
    let (_, city) = example_plan();
    let masses = super::mass::masses(&city, &super::bible::StyleBible::default(), 7);
    let structures = super::structure::structures(&city, &masses);
    let planters: Vec<Vec<DVec2>> = gardens::gardens(&city, &masses, &structures)
        .into_iter()
        .filter(|p| p.kind == Kind::Planter)
        .map(|p| p.outline)
        .collect();
    let (mut doors, mut blocked, mut shops, mut before_shops) = (0, 0, 0, 0);
    for (m, s) in masses.iter().zip(&structures) {
        for o in s.openings.iter().filter(|o| o.storey == 0) {
            let wall = &s.walls[o.wall];
            if wall.role != WallRole::Street {
                continue;
            }
            let t = (wall.b - wall.a).normalize_or_zero();
            let n = DVec2::new(t.y, -t.x);
            let mid = wall.a + t * o.along;
            let n = if geom::contains(&m.footprint, mid + n * 0.3) {
                -n
            } else {
                n
            };
            // Half a planter's depth out, before the opening's middle and its edges.
            let probes = [-0.5, 0.0, 0.5].map(|k| mid + t * (o.width * k) + n * (PLANTER.0 * 0.5));
            let hit = probes.iter().any(|&p| inside_any(&planters, p));
            match o.kind {
                OpeningKind::Door | OpeningKind::Passage => {
                    doors += 1;
                    blocked += usize::from(hit);
                }
                OpeningKind::Shopfront => {
                    shops += 1;
                    before_shops += usize::from(hit);
                }
                OpeningKind::Window => {}
            }
        }
    }
    println!(
        "{} planters; {doors} street doors and passages, {blocked} blocked; {shops} shopfronts, \
         {before_shops} behind a planter",
        planters.len()
    );
    assert!(planters.len() > 500, "{} planters", planters.len());
    assert_eq!(blocked, 0, "{blocked} doors or passages behind a planter");
    assert_eq!(
        before_shops, 0,
        "{before_shops} shopfronts behind a planter"
    );
}

#[test]
fn no_street_is_left_with_a_jog() {
    // Every street's line, after the plan's smoothing: no corner over 60° followed within 45 m
    // by one the other way (a traced street joined to the one ahead by a stub; the owner:
    // "one gentle turn would be enough"). The example has them before the pass.
    let size = 4000.0;
    let site = example::site(size, 10.0);
    let city = plan(&site, &example::params(size));
    let left: usize = city
        .roads
        .edges
        .iter()
        .map(|e| super::graph::jogs(&e.points).len())
        .sum();
    assert_eq!(left, 0, "{left} jogs left in the streets");
    assert!(
        city.roads.edges.len() > 1000,
        "{} edges",
        city.roads.edges.len()
    );
}

#[test]
fn a_village_sized_site_plans() {
    // The first map's scale (D-062): a site a few hundred metres across still plans, with its
    // blocks and lots, so the village can start from the same machinery.
    let size = 600.0;
    let site = example::site(size, 10.0);
    let city = plan(&site, &example::params(size));
    let built = city
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Built)
        .count();
    println!(
        "{} nodes, {} edges, {} blocks ({built} built), {} lots",
        city.roads.nodes.len(),
        city.roads.edges.len(),
        city.blocks.len(),
        city.lots.len()
    );
    assert!(built >= 4, "{built} built blocks");
    assert!(city.lots.len() >= 20, "{} lots", city.lots.len());
}

/// The taxis and shuttles (D-061) drive every lane of the taxi roads both ways and turn from each
/// road into every other at each junction, swept as boxes against the kerbs, the bollards, the
/// street trees and the lamps' posts (#276: the owner's "for roads ... rectangular cuboid").
/// - **A lane** is the demo's: 1.5 m right of the centreline (`taxis::LANE`), swept every 0.5 m
///   from 8 m after one end to 8 m before the other (the junctions are the turns').
/// - **A turn** is possible when one of the paths a driver could take is clear: a curve from the
///   lane in to the lane out (a quadratic Bézier through the two lanes' crossing), starting
///   3 to 12 m before the junction, no tighter than the vehicle turns (4.5 m for the cab, 5.5 m
///   for the shuttle, at its middle).
///
/// What is met is counted by place (5 m cells), and the count may not grow: a ratchet, as the
/// village's anomalies.
#[test]
fn taxis_drive_and_turn_clear_of_the_kerbs_bollards_trees_and_lamps() {
    use super::sweep::{self, Cuboid};
    use std::collections::{BTreeSet, HashMap};
    const LANE: f64 = 1.5;
    const CELL: f64 = 10.0;
    const KINDS: [&str; 6] = ["bollard", "trunk", "crown", "lamp", "control", "kerb"];
    let (site, city) = example_plan();
    let trees = super::trees::trees(&site, &city, 5);
    let lamps = super::lamps::lamps(&site, &city);
    // The cab and the shuttle (`forge_geom::vehicles`), their bodies from 0.15 m, and the
    // tightest their middles turn.
    let vehicles = [
        (
            "cab",
            Cuboid {
                half_length: 1.8,
                half_width: 0.9,
                bottom: 0.15,
                top: 1.65,
            },
            4.5,
        ),
        (
            "shuttle",
            Cuboid {
                half_length: 2.5,
                half_width: 1.0,
                bottom: 0.15,
                top: 2.2,
            },
            5.5,
        ),
    ];
    let key = |p: DVec2| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32);
    // The kerbs' segments, in every cell their box covers.
    let kerbs: Vec<[DVec2; 2]> = city
        .ground
        .tiles
        .iter()
        .flat_map(|t| t.kerbs.iter().copied())
        .collect();
    let mut kerb_cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, [a, b]) in kerbs.iter().enumerate() {
        let (lo, hi) = (key(a.min(*b)), key(a.max(*b)));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                kerb_cells.entry((x, y)).or_default().push(i);
            }
        }
    }
    // Upright discs: where, radius, from, to, and what (an index into KINDS).
    let mut discs: Vec<(DVec2, f64, f64, f64, usize)> = Vec::new();
    for &b in &city.ground.bollards {
        discs.push((b, 0.1, 0.0, 1.0, 0));
    }
    for t in trees.iter().filter(|t| t.species.is_none()) {
        discs.push((t.pos, 0.3 * t.crown / 4.2, 0.0, 0.44 * t.height, 1));
        discs.push((t.pos, t.crown, 0.43 * t.height, t.height, 2));
    }
    for l in &lamps {
        discs.push((l.pos, 0.1, 0.0, l.height, 3));
    }
    let driven: Vec<usize> = (0..city.roads.edges.len())
        .filter(|&k| city.roads.edges[k].profile.driven())
        .collect();
    // The control: a post on the first long driven road's lane halfway, which the sweep must
    // meet.
    if let Some(&k) = driven
        .iter()
        .find(|&&k| city.roads.edges[k].length() > 30.0)
    {
        let e = &city.roads.edges[k];
        let s = e.length() * 0.5;
        let (p, t) = (geom::point_at(&e.points, s), geom::tangent_at(&e.points, s));
        discs.push((p + DVec2::new(-t.y, t.x) * LANE, 0.1, 0.0, 1.0, 4));
    }
    let mut disc_cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, d) in discs.iter().enumerate() {
        disc_cells.entry(key(d.0)).or_default().push(i);
    }
    // What a body at `centre` heading along `heading` meets, by kind.
    let meets = |body: &Cuboid, centre: DVec2, heading: DVec2| -> [bool; 6] {
        let (cx, cy) = key(centre);
        let mut hit = [false; 6];
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(near) = kerb_cells.get(&(cx + dx, cy + dy)) {
                    hit[5] |= near.iter().any(|&k| {
                        let [a, b] = kerbs[k];
                        sweep::cuboid_meets_segment(body, centre, heading, a, b)
                    });
                }
                if let Some(near) = disc_cells.get(&(cx + dx, cy + dy)) {
                    for &k in near {
                        let (at, r, y0, y1, kind) = discs[k];
                        hit[kind] |=
                            sweep::cuboid_meets_disc(body, centre, heading, 0.0, at, r, y0, y1);
                    }
                }
            }
        }
        hit
    };
    let place = |p: DVec2| ((p.x / 5.0).floor() as i32, (p.y / 5.0).floor() as i32);
    // Along every lane, both ways, every 0.5 m, clear of the junctions.
    let mut lanes: [BTreeSet<(i32, i32)>; 6] = Default::default();
    let mut km = 0.0;
    for &k in &driven {
        let e = &city.roads.edges[k];
        let length = e.length();
        km += 2.0 * length / 1000.0;
        for forward in [true, false] {
            let mut s = 8.0;
            while s <= length - 8.0 {
                let along = if forward { s } else { length - s };
                let dir = if forward { 1.0 } else { -1.0 };
                for (_, body, _) in &vehicles {
                    // Set by its two ends on the centreline, as a vehicle sits on its path (at a
                    // bend's corner its middle cuts in), then its lane's offset.
                    let end = |d: f64| geom::point_at(&e.points, (along + d).clamp(0.0, length));
                    let (front, back) = (end(dir * body.half_length), end(-dir * body.half_length));
                    let t = (front - back).normalize_or_zero();
                    let p = (front + back) * 0.5 + DVec2::new(-t.y, t.x) * LANE;
                    for (kind, &h) in meets(body, p, t).iter().enumerate() {
                        if h {
                            lanes[kind].insert(place(p));
                        }
                    }
                }
                s += 0.5;
            }
        }
    }
    // Each turn at each node: from every driven road in to every other out.
    let mut ends: HashMap<u32, Vec<(usize, bool)>> = HashMap::new();
    for &k in &driven {
        let e = &city.roads.edges[k];
        ends.entry(e.a).or_default().push((k, true));
        ends.entry(e.b).or_default().push((k, false));
    }
    // A lane's point and the way the vehicle drives `d` metres from road `k`'s end at the node,
    // `out` of it or into it.
    let lane = |k: usize, at_a: bool, d: f64, out: bool| {
        let e = &city.roads.edges[k];
        let length = e.length();
        let d = d.min(length * 0.5);
        let along = if at_a { d } else { length - d };
        let away = if at_a { 1.0 } else { -1.0 };
        let t = geom::tangent_at(&e.points, along) * if out { away } else { -away };
        (
            geom::point_at(&e.points, along) + DVec2::new(-t.y, t.x) * LANE,
            t,
        )
    };
    // Whether `body` turning no tighter than `radius` takes the turn from road `i` into road `o`
    // by one of the paths, and what the last path tried met.
    let turn = |body: &Cuboid, radius: f64, i: (usize, bool), o: (usize, bool)| {
        let mut last = [false; 6];
        for d in [3.0, 4.5, 6.0, 8.0, 10.0, 12.0] {
            let (a, u) = lane(i.0, i.1, d, false);
            let (b, v) = lane(o.0, o.1, d, true);
            let cross = u.x * v.y - u.y * v.x;
            let c = if cross.abs() > 0.05 {
                let t = ((b - a).x * v.y - (b - a).y * v.x) / cross;
                a + u * t.clamp(0.0, 30.0)
            } else {
                (a + b) * 0.5
            };
            let steps = ((a.distance(c) + c.distance(b)) / 0.25).ceil().max(1.0) as usize;
            let mut tight = false;
            let mut hit = [false; 6];
            for j in 0..=steps {
                let t = j as f64 / steps as f64;
                let p = a * (1.0 - t) * (1.0 - t) + c * 2.0 * (1.0 - t) * t + b * t * t;
                let d1 = (c - a) * 2.0 * (1.0 - t) + (b - c) * 2.0 * t;
                let d2 = (a - c * 2.0 + b) * 2.0;
                let speed = d1.length();
                if speed < 1e-6 {
                    tight = true;
                    break;
                }
                if (d1.x * d2.y - d1.y * d2.x).abs() / speed.powi(3) > 1.0 / radius {
                    tight = true;
                    break;
                }
                for (kind, h) in meets(body, p, d1 / speed).iter().enumerate() {
                    hit[kind] |= *h;
                }
            }
            if !tight {
                last = hit;
                if hit.iter().all(|&h| !h) {
                    return (true, hit);
                }
            }
        }
        (false, last)
    };
    let mut turns = 0;
    let mut blocked: [BTreeSet<(i32, i32)>; 2] = Default::default();
    let mut blocked_by = [0_usize; 6];
    let mut first: Vec<(&str, DVec2, usize, usize)> = Vec::new();
    let mut sorted: Vec<_> = ends.into_iter().collect();
    sorted.sort_by_key(|(n, _)| *n);
    for (node, roads) in sorted {
        let at = city.roads.nodes[node as usize].pos;
        for &i in &roads {
            for &o in &roads {
                if i.0 == o.0 {
                    continue;
                }
                turns += 1;
                for (v, (name, body, radius)) in vehicles.iter().enumerate() {
                    let (clear, hit) = turn(body, *radius, i, o);
                    if !clear {
                        blocked[v].insert(place(at));
                        for (kind, &h) in hit.iter().enumerate() {
                            blocked_by[kind] += usize::from(h);
                        }
                        if first.len() < 12 {
                            first.push((name, (at * 10.0).round() / 10.0, i.0, o.0));
                        }
                    }
                }
            }
        }
    }
    let counts: Vec<String> = KINDS
        .iter()
        .zip(&lanes)
        .map(|(k, s)| format!("{} {k}", s.len()))
        .collect();
    println!(
        "{km:.1} km of lanes both ways: met at {}; {turns} turns: the cab cannot take them at {} \
         places, the shuttle at {} (the last paths tried met {blocked_by:?} of {KINDS:?})",
        counts.join(", "),
        blocked[0].len(),
        blocked[1].len()
    );
    println!("the first blocked turns (vehicle, node, road in, road out): {first:?}");
    // Where to look from, over each place (`city-plan --view`, the plan's middle at the origin).
    let views = |s: &BTreeSet<(i32, i32)>| -> Vec<String> {
        s.iter()
            .map(|&(x, y)| {
                let (vx, vz) = (
                    f64::from(x) * 5.0 + 2.5 - 2000.0,
                    f64::from(y) * 5.0 + 2.5 - 2000.0,
                );
                format!("--view={vx:.0},30,{vz:.0},0,-89")
            })
            .collect()
    };
    println!("lanes over a kerb: {:?}", views(&lanes[5]));
    println!("turns no vehicle takes: {:?}", views(&blocked[1]));
    assert!(km > 10.0, "{km}");
    assert!(
        !lanes[4].is_empty(),
        "the control post on a lane was never met"
    );
    // Nothing stands in a lane: no bollard, trunk, crown nor lamp.
    assert!(
        lanes[..4].iter().all(BTreeSet::is_empty),
        "lanes meet bollards at {:?}",
        views(&lanes[0])
    );
    // No lane crosses a kerb: the sharp bends of driven roads' centrelines (35° to 68° at one
    // point) have their inside kerbs rounded as the junctions' corners are. What is left may not
    // grow: the two acute junctions where no curve is wide enough to turn (a three-point turn
    // there).
    assert!(
        lanes[5].is_empty(),
        "lanes over a kerb at {:?}",
        views(&lanes[5])
    );
    assert!(
        blocked.iter().all(|b| b.len() <= 2),
        "turns no vehicle takes at {:?}",
        views(&blocked[1])
    );
}
