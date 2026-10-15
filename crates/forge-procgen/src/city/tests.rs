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
    use super::structure::{OpeningKind, PIER, SHUTTER_BESIDE};
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
                        let reach = o.width + SHUTTER_BESIDE;
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
            on_path += usize::from(probes.iter().any(|&q| inside_any(mine.iter().copied(), q)));
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
