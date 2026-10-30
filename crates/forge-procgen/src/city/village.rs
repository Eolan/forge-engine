//! The first map's village (D-062, #266): a site a few hundred metres across planned as the
//! city is (streets, blocks, parks), each built block cut into strips of village lots
//! ([`super::parcels::strips`]) and a house from its rooms put on each
//! ([`super::houses::houses`]), the households drawn by the seed: so many people, so well off.
//! A block too thin or concave for the strips keeps no houses here; the map says how many.
//! Each house is also one of the city's masses on its own lot, so the structures, the facades
//! and the kit draw it as they draw the city's buildings.

use forge_core::{Seed, SplitMix64};
use glam::DVec2;

use super::blocks::{self, BlockUse, Lot};
use super::cores;
use super::geom::{self, PointGrid};
use super::houses::{self, House};
use super::lots;
use super::mass::{Mass, Roof};
use super::parcels::ParcelType;
use super::rooms::{self, Means};
use super::structure::Era;
use super::trees::{Setting, Tree};
use super::units;
use super::walls;
use super::{
    BoundaryInfluence, CityParams, CityPlan, DistrictKind, DistrictSpec, Pattern, RoadClass, Site,
    plan,
};

/// A built block's lots, court and houses.
#[derive(Clone, Debug, PartialEq)]
pub struct VillageBlock {
    /// The block, an index into the plan's.
    pub block: usize,
    /// Its lots, each its street side first.
    pub lots: Vec<Vec<DVec2>>,
    /// Its court, the block inset by the strips' depth.
    pub court: Vec<DVec2>,
    /// Its houses, one a lot.
    pub houses: Vec<House>,
    /// Its lots' type: its ring's, at the depth the block took.
    pub ty: ParcelType,
    /// The gates in its garden walls (their walls are the plan's, [`CityPlan::walls`]).
    pub gates: Vec<DVec2>,
    /// The hedges between its gardens, each a line on the plan (`walls::GardenWalls::hedges`).
    pub hedges: Vec<(DVec2, DVec2)>,
}

/// The village: its plan and its blocks' houses.
#[derive(Clone, Debug)]
pub struct Village {
    /// The site's plan: streets, blocks, parks; its lots the houses' (appended to the city's
    /// own, which no mass uses).
    pub plan: CityPlan,
    /// The built blocks that took strips, with their houses.
    pub blocks: Vec<VillageBlock>,
    /// The built blocks that took no strips (too thin or concave for the depth).
    pub without: usize,
    /// The houses as the city's masses, one a house in the order of [`Village::houses`], each
    /// on its own lot of the plan: what the structures, the facades and the kit read.
    pub masses: Vec<Mass>,
}

impl Village {
    /// All the houses.
    pub fn houses(&self) -> impl Iterator<Item = &House> {
        self.blocks.iter().flat_map(|b| b.houses.iter())
    }
}

/// A household drawn for lot `k` of block `b`: one to five people, most of modest means, a few
/// poor, a third comfortable, one in twenty rich.
fn household(seed: u64, b: usize, k: usize) -> (u32, Means) {
    let mut rng =
        SplitMix64::new(seed ^ (b as u64 * 0x9E37_79B9_7F4A_7C15) ^ (k as u64 * 0xC2B2_AE35));
    let people = 1 + (rng.range_f64(0.0, 1.0) * 5.0) as u32;
    let draw = rng.range_f64(0.0, 1.0);
    let means = if draw < 0.15 {
        Means::Poor
    } else if draw < 0.65 {
        Means::Modest
    } else if draw < 0.95 {
        Means::Comfortable
    } else {
        Means::Rich
    };
    (people, means)
}

/// A block's cut, as [`parcels::strips_with`] returns it, with its type.
type Cut = (ParcelType, Vec<Vec<DVec2>>, Vec<DVec2>, Vec<usize>);

/// The household of lot `key` of block `b`: the village's rich one (five people, rich) where
/// `rich` names that lot, else the seed's draw.
fn household_of(seed: u64, b: usize, key: usize, rich: Option<(usize, usize)>) -> (u32, Means) {
    if rich == Some((b, key)) {
        (5, Means::Rich)
    } else {
        household(seed, b, key)
    }
}

/// The village's own parameters for a site `size` across (D-062, #266): one village district
/// on the site's middle with lanes that wander, one main road, no trams, boats nor landmarks, a
/// green within 200 m of every block, the land beyond the village left to the forest.
pub fn params(size: f64) -> CityParams {
    let at = |x: f64, y: f64| DVec2::new(x * size, y * size);
    CityParams {
        seed: Seed::new(20_261_016),
        districts: vec![DistrictSpec {
            name: "village".into(),
            kind: DistrictKind::Village,
            anchor: at(0.5, 0.5),
            radius: 0.2 * size,
            pattern: Pattern::Organic(1.2),
            grid_angle: 0.3,
            street_spacing: 80.0,
            lot_area: 200.0,
            floors: (1, 3),
            depth: None,
            tiled_roofs: Some(1.0),
            gaps: None,
            run: None,
        }],
        main_spacing: size,
        boundary: BoundaryInfluence {
            strength: 1.5,
            reach: 0.03 * size,
        },
        max_bridge: 80.0,
        centre: at(0.5, 0.5),
        tram_lines: Vec::new(),
        stop_spacing: 450.0,
        stop_target: 0.0,
        max_extra_lines: 0,
        boat_gap: f64::INFINITY,
        landmarks: Vec::new(),
        park_reach: 200.0,
        park_min_area: 2_000.0,
        park_share_cap: 0.15,
        forest_above: f32::MAX,
        forest_slope: f32::MAX,
        reach: Some(1.6),
        max_block: None,
        forest: Some(7.0),
    }
}

/// The village on `site` by `params`: the plan, then each built block's strips and houses, and
/// the houses as masses.
pub fn village(site: &Site, params: &CityParams, seed: u64) -> Village {
    let mut city = plan(site, params);
    // Old stone throughout, whatever the example's districts: the structures and the facades
    // take a building's era from its lot's district.
    for d in &mut city.districts {
        d.kind = DistrictKind::Village;
    }
    let streets = blocks::street_grid(&city.roads, city.size);
    // A house of two storeys or more is deep enough for its straight flight (`cores`): the
    // ground storey's flight and the core's back wall inside the front and back walls.
    let era = Era::OldStone.table();
    let stair = cores::flight_length(Era::OldStone.storeys(2, false)[0])
        + cores::CORE_WALL
        + era.facade
        + era.rear;
    // The rings (`research/cadastre-villages-and-towns.md` §2): the core round the centre,
    // the faubourgs about it, the lotissements beyond, by a block's distance from the centre
    // against the village district's radius.
    let radius = params
        .districts
        .first()
        .map_or(params.centre.x.max(1.0), |d| d.radius);
    // A block's cut: its ring's type and depth (a thin block takes a shallower strip; one too
    // thin for the last keeps no houses), each lot as wide as its household's rooms need on
    // two storeys at that depth (the owner's way round: the rooms first, the lot cut to the
    // house), never narrower than the type's least nor wider than its most. `rich` names the
    // lot (block, key) whose household is the village's rich one.
    let cut_block = |b: usize, rich: Option<(usize, usize)>| {
        let block = &city.blocks[b];
        // The traced lanes give the block short edges along their curves, and each edge took a
        // strip of its own with one narrow lot: the outline simplified within 0.6 m first.
        let outline = geom::simplify_closed(&block.buildable, 0.6);
        let n = outline.len();
        let away = block.centre.distance(params.centre) / radius;
        // A faubourg's block along the road out takes its strips on that side only, deep, and
        // leaves its lanes the gardens' ends: the lanières of the research's §2, 6–12 m wide and
        // 25–45 m deep. One with no side on the road out is cut on all its sides.
        let on_main: Vec<bool> = (0..n)
            .map(|i| {
                let mid = (outline[i] + outline[(i + 1) % n]) * 0.5;
                streets
                    .nearest(mid, 30.0)
                    .is_some_and(|(_, e)| city.roads.edges[e as usize].class == RoadClass::Main)
            })
            .collect();
        let lanieres = (0.9..1.35).contains(&away) && on_main.iter().any(|&m| m);
        let (ty, depths): (ParcelType, &[f64]) = if away < 0.9 {
            (ParcelType::CORE, &[12.0, 10.0, 8.0])
        } else if lanieres {
            (ParcelType::FAUBOURG, &[40.0, 32.0, 25.0, 18.0])
        } else if away < 1.35 {
            (ParcelType::FAUBOURG, &[30.0, 22.0, 16.0, 12.0])
        } else {
            (ParcelType::LOTISSEMENT, &[35.0, 28.0, 22.0])
        };
        let street = if lanieres { on_main } else { vec![true; n] };
        let block_area = geom::signed_area(&outline).abs().max(1.0);
        // Cut at `depth`, its backs at `back`: a cut shallower than the deepest that cut (`first`)
        // keeps that one's houses, its yard the shallower.
        let cut_at = |depth: f64, back: lots::Back, first: Option<&Cut>| {
            let mut ty = ParcelType { depth, ..ty };
            if let Some(f) = first {
                ty.yard = (depth - ty.setback - houses::plan_depth(&f.0))
                    .clamp(LEAST_YARD.min(ty.yard), ty.yard);
            }
            let max_depth = houses::plan_depth(&ty);
            let width = |k: usize| {
                let (people, means) = household_of(seed, b, k, rich);
                let area: f64 = rooms::programme(people, means).iter().map(|r| r.area).sum();
                (rooms::HALL + rooms::WALL + area / (2.0 * max_depth) + 2.0 * ty.sides)
                    .clamp(ty.width.0, ty.width.1.max(16.0))
            };
            lots::cut(&outline, &street, back, &ty, width).map(|layout| (ty, layout))
        };
        // The deepest cut that leaves the block a court of its own (`lots::Layout::court_fits`);
        // deeper, the rows met round a scrap of court (the owner, 2026-10-29: "shallower lots,
        // bigger court"). Where none does, the block's two longest facing sides back to back on
        // one straight line ("it could be a single straight hedge").
        let mut first: Option<Cut> = None;
        for &depth in depths {
            let Some((ty, layout)) = cut_at(depth, lots::Back::Depth(depth), first.as_ref()) else {
                continue;
            };
            let fits = layout.court_fits(block_area);
            let cut = (ty, layout.lots, layout.court, layout.keys);
            if fits {
                return Some(cut);
            }
            first.get_or_insert(cut);
        }
        // Each pair of facing sides in turn, the longest first, until one leaves no court or a
        // wide one.
        for &depth in depths {
            for rank in 0..4 {
                if let Some((ty, layout)) =
                    cut_at(depth, lots::Back::Spine(depth, rank), first.as_ref())
                    && layout.back_to_back_fits()
                {
                    return Some((ty, layout.lots, layout.court, layout.keys));
                }
            }
        }
        // No cut fits the rules: the block keeps no lots and stays open ground (a thin court's
        // wedge or a tip left between the rows is the bad shape the rules rule out).
        None
    };
    let built: Vec<usize> = (0..city.blocks.len())
        .filter(|&b| city.blocks[b].use_ == BlockUse::Built)
        .collect();
    let mut cuts: Vec<Option<Cut>> = built.iter().map(|&b| cut_block(b, None)).collect();
    // The rich house on the square (the research's §5): the lot whose front lies nearest the
    // square takes the village's rich household, and its block is cut again for its width.
    let square = city
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Square)
        .min_by(|a, b| {
            a.centre
                .distance(params.centre)
                .total_cmp(&b.centre.distance(params.centre))
        })
        .map(|b| b.polygon.clone());
    let rich = square.and_then(|square| {
        let m = square.len();
        let to_square = |p: DVec2| {
            (0..m)
                .map(|i| geom::distance_to_segment(p, square[i], square[(i + 1) % m]))
                .fold(f64::MAX, f64::min)
        };
        built
            .iter()
            .zip(&cuts)
            .filter_map(|(&b, cut)| cut.as_ref().map(|c| (b, c)))
            .flat_map(|(b, (_, lots, _, keys))| {
                lots.iter()
                    .zip(keys)
                    .map(move |(lot, &key)| ((b, key), to_square((lot[0] + lot[1]) * 0.5)))
            })
            .filter(|&(_, d)| d < 25.0)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(at, _)| at)
    });
    if let Some((b, _)) = rich {
        let i = built.iter().position(|&x| x == b).unwrap_or(0);
        cuts[i] = cut_block(b, rich);
    }
    // The olive, the gardens' tree, among the grower's species.
    let olive = forge_flora::species::mediterranean()
        .iter()
        .position(|s| s.id == "olive")
        .map(|i| i as u16);
    // The kermes oak, the hedges' shrub.
    let kermes = forge_flora::species::mediterranean()
        .iter()
        .position(|s| s.id == "kermes-oak")
        .map(|i| i as u16);
    let mut blocks = Vec::new();
    let mut masses = Vec::new();
    let mut without = 0;
    for (&b, cut) in built.iter().zip(cuts) {
        let Some((ty, lots, court, keys)) = cut else {
            without += 1;
            continue;
        };
        // A lot too narrow for a house of its own joins its neighbour along the street.
        let (lots, keys) = houses::merge_small(&lots, &keys, &ty);
        // Each lot's corners on another's edges put into them (`conform`).
        let lots = conform(&lots);
        let mut houses =
            houses::houses(&lots, &ty, |k| household_of(seed, b, keys[k], rich), stair);
        // A lot that took no house (its household's rooms wanted more than it holds) joins the
        // neighbour along its street whose house it borders: open, it was a lawn on the street
        // between two walled gardens (the owner, 2026-10-24).
        let lots = join_empty(lots, &mut houses);
        // A block where no house stands keeps no lots: its ground stays open.
        if houses.is_empty() {
            without += 1;
            continue;
        }
        // The empty lots left (no neighbour to join as a rectangle) go to the court they border,
        // the block's common ground.
        let (lots, court) = empty_to_court(lots, court, &houses);
        let outline = geom::simplify_closed(&city.blocks[b].buildable, 0.6);
        // And again where a lot joined its neighbour.
        let conformed = conform(&lots);
        for h in &mut houses {
            if let Some(k) = lots.iter().position(|lot| *lot == h.lot) {
                h.lot.clone_from(&conformed[k]);
            }
        }
        let lots = conformed;
        for h in &houses {
            masses.push(mass(&mut city, b, h, &streets));
        }
        // The walls round its gardens and its court wherever no house stands, a lotissement's
        // front low: on the outline the strips were cut from.
        let footprints: Vec<Vec<DVec2>> = houses.iter().map(|h| h.footprint.clone()).collect();
        let front = if ty.setback > 0.0 {
            walls::FRONT
        } else {
            walls::STREET
        };
        let housed: Vec<bool> = lots
            .iter()
            .map(|lot| houses.iter().any(|h| h.lot == *lot))
            .collect();
        let garden = walls::garden_walls(&outline, &lots, &footprints, &housed, front);
        // The room a crown at `p` has (the owner, 2026-10-25: "some bushes in the village are
        // going through the walls (even some houses here and there)"): to the garden walls past
        // their whole thickness, either side of their line, and to the houses' walls.
        let lines: Vec<(DVec2, DVec2)> = garden.walls.iter().map(|w| (w.a, w.b)).collect();
        let room = |p: DVec2| {
            let walled = lines
                .iter()
                .map(|&(a, b)| geom::distance_to_segment(p, a, b) - walls::THICKNESS);
            let housed = footprints.iter().flat_map(|f| {
                (0..f.len()).map(move |k| geom::distance_to_segment(p, f[k], f[(k + 1) % f.len()]))
            });
            walled.chain(housed).fold(f64::INFINITY, f64::min)
        };
        // A crown of `crown` metres at `p`, narrowed to its room ([`CROWN_CLEAR`], and the
        // stretch a drawn crown may take, [`CROWN_STRETCH`]).
        let fit = |crown: f64, p: DVec2| crown.min((room(p) - CROWN_CLEAR) / CROWN_STRETCH);
        city.walls.extend(garden.walls);
        // The hedges: kermes oaks clipped to a row, 0.9 m apart, 1.4 to 1.8 m high. A hedge runs
        // from a street wall to a house or another wall: its end plants are narrowed to the room
        // they have, and left out under [`HEDGE_LEAST`] (they came through the street walls).
        for (h, &(p, q)) in garden
            .hedges
            .iter()
            .enumerate()
            .filter(|_| kermes.is_some())
        {
            let count = (p.distance(q) / 0.9).round().max(1.0) as usize;
            let mut rng =
                SplitMix64::new(seed ^ ((b as u64) << 20) ^ ((h as u64) << 8) ^ 0x004E_D6E5);
            for i in 0..count {
                let pos = p + (q - p) * ((i as f64 + 0.5) / count as f64);
                let height = rng.range_f64(1.4, 1.8);
                let crown = fit(rng.range_f64(0.55, 0.7), pos);
                if crown < HEDGE_LEAST {
                    continue;
                }
                city.planted.push(Tree {
                    pos,
                    height,
                    crown,
                    setting: Setting::Garden,
                    species: kermes,
                });
            }
        }
        // An olive of a drawn height and crown at `p`, its crown narrowed to its room and its
        // height with it; none under [`OLIVE_LEAST`].
        let olive_at = |p: DVec2, height: f64, crown: f64| {
            let fitted = fit(crown, p);
            (fitted >= OLIVE_LEAST).then(|| Tree {
                pos: p,
                height: height * fitted.min(crown) / crown,
                crown: fitted.min(crown),
                setting: Setting::Garden,
                species: olive,
            })
        };
        // The court, the block's common ground: an olive grove, a tree every 8 m on a grid 3 m
        // in from its edges (the owner, 2026-10-16: the middle "should not exist or be common").
        if court.len() >= 3 && geom::signed_area(&court).abs() > 60.0 {
            let (axis, extent, middle) = geom::oriented_extent(&court);
            let side = DVec2::new(-axis.y, axis.x);
            let mut rng = SplitMix64::new(seed ^ ((b as u64) << 24) ^ 0x000C_07A7);
            let (nx, ny) = (
                (extent.x * 2.0 / 8.0).floor() as i32,
                (extent.y * 2.0 / 8.0).floor() as i32,
            );
            for i in -nx / 2..=nx / 2 {
                for j in -ny / 2..=ny / 2 {
                    let p = middle + axis * (f64::from(i) * 8.0) + side * (f64::from(j) * 8.0);
                    let inside = geom::contains(&court, p)
                        && (0..court.len()).all(|k| {
                            geom::distance_to_segment(p, court[k], court[(k + 1) % court.len()])
                                > 3.0
                        });
                    if inside {
                        let (height, crown) = (rng.range_f64(4.5, 7.0), rng.range_f64(2.0, 3.2));
                        city.planted.extend(olive_at(p, height, crown));
                    }
                }
            }
        }
        // The gardens' olives (`trees::Setting::Garden`): behind each house where 30 m² or more
        // stay 1.5 m clear of it, one at the garden's middle, two along a garden of 150 m² or
        // more; a lot without a house is all garden.
        for (k, lot) in lots.iter().enumerate() {
            let t = (lot[1] - lot[0]).normalize_or_zero();
            let n = DVec2::new(-t.y, t.x);
            let ground = match houses.iter().find(|h| h.lot == *lot) {
                Some(h) => {
                    let back = h
                        .footprint
                        .iter()
                        .map(|&p| (p - lot[0]).dot(n))
                        .fold(0.0, f64::max)
                        + 1.5;
                    geom::clean(
                        &geom::clip_half_plane(lot, lot[0] + n * back, DVec2::new(n.y, -n.x)),
                        1e-6,
                    )
                }
                None => lot.clone(),
            };
            if ground.len() < 3 || geom::signed_area(&ground).abs() < 30.0 {
                continue;
            }
            let area = geom::signed_area(&ground).abs();
            let middle = geom::centroid(&ground);
            let (axis, extent, _) = geom::oriented_extent(&ground);
            let spots: Vec<DVec2> = if area >= 150.0 {
                vec![
                    middle - axis * extent.x * 0.25,
                    middle + axis * extent.x * 0.25,
                ]
            } else {
                vec![middle]
            };
            let mut rng = SplitMix64::new(seed ^ ((b as u64) << 20) ^ (k as u64) ^ 0x0717_7EE5);
            for p in spots.into_iter().filter(|&p| geom::contains(&ground, p)) {
                let (height, crown) = (rng.range_f64(4.5, 7.0), rng.range_f64(2.0, 3.2));
                city.planted.extend(olive_at(p, height, crown));
            }
        }
        blocks.push(VillageBlock {
            block: b,
            lots,
            court,
            houses,
            ty,
            gates: garden.gates,
            hedges: garden.hedges,
        });
    }
    // The garden walls' ends meeting nothing (D-066's join pass, `anomalies`' free wall ends):
    // each carried on along its line, 2 m at most, to the first house or garden wall it meets.
    // A free end stood as a pier in a garden, a step short of the house beside it.
    let near = |poly: &[DVec2], p: DVec2, tol: f64| {
        geom::contains(poly, p)
            || (0..poly.len())
                .any(|i| geom::distance_to_segment(p, poly[i], poly[(i + 1) % poly.len()]) < tol)
    };
    // Two blocks' walls whose ends nearly meet at a bend (each block's walls are joined among
    // themselves only: two free ends 0.35 m apart where two blocks' gardens met at 134°) meet
    // where their lines cross, their end faces mitred on the bisector as `walls::joints` mitres
    // two walls in line. Bends under 80° only, the two ends within a wall's thickness and the
    // crossing within a metre of both.
    let end_at = |w: &walls::Wall, e: usize| if e == 0 { w.a } else { w.b };
    let far_of = |w: &walls::Wall, e: usize| if e == 0 { w.b } else { w.a };
    for k in 0..city.walls.len() {
        for ek in 0..2 {
            let p = end_at(&city.walls[k], ek);
            let met_by = |j: usize| j != k && near(&city.walls[j].ring(), p, 0.05);
            if masses.iter().any(|m| near(&m.footprint, p, 0.05))
                || (0..city.walls.len()).any(met_by)
            {
                continue;
            }
            let pair = (0..city.walls.len()).filter(|&j| j != k).find_map(|j| {
                let ej =
                    (0..2).find(|&e| end_at(&city.walls[j], e).distance(p) < walls::THICKNESS)?;
                let (wk, wj) = (&city.walls[k], &city.walls[j]);
                let q = end_at(wj, ej);
                // The way along the two through the bend: into it along `k`, out along `j`.
                let (into, out) = (
                    (p - far_of(wk, ek)).normalize_or_zero(),
                    (far_of(wj, ej) - q).normalize_or_zero(),
                );
                if into.dot(out) < 80f64.to_radians().cos() || into.perp_dot(out).abs() < 0.3 {
                    return None;
                }
                let (u, _) = geom::segment_intersection(
                    far_of(wk, ek),
                    far_of(wk, ek) + into * 1e3,
                    far_of(wj, ej) - out * 1e3,
                    far_of(wj, ej),
                )?;
                let x = far_of(wk, ek) + into * (u * 1e3);
                (x.distance(p) < 1.0 && x.distance(q) < 1.0).then_some((j, ej, x))
            });
            let Some((j, ej, x)) = pair else {
                continue;
            };
            for (w, e) in [(k, ek), (j, ej)] {
                if e == 0 {
                    city.walls[w].a = x;
                } else {
                    city.walls[w].b = x;
                }
            }
            // `walls::joints`' mitre: each wall's direction and the other's, turned its way.
            let dir = |w: &walls::Wall| (w.b - w.a).normalize_or_zero();
            let (dk, dj) = (dir(&city.walls[k]), dir(&city.walls[j]));
            for (w, e, t, d) in [(k, ek, dk, dj), (j, ej, dj, dk)] {
                let d = if d.dot(t) < 0.0 { -d } else { d };
                let mean = (t + d).normalize_or_zero();
                city.walls[w].ends[e] = DVec2::new(-mean.y, mean.x);
            }
        }
    }
    // A gate's sides are free ends too, and stay where they are: carried on, the wall beyond a
    // gate ran back through it onto the wall before it, two walls on one line (#285).
    let gates: Vec<DVec2> = blocks
        .iter()
        .flat_map(|b| b.gates.iter().copied())
        .collect();
    for k in 0..city.walls.len() {
        for end in 0..2 {
            let w = &city.walls[k];
            let (p, from) = if end == 0 { (w.a, w.b) } else { (w.b, w.a) };
            let dir = (p - from).normalize_or_zero();
            if gates
                .iter()
                .any(|g| g.distance(p) < walls::GATE * 0.5 + 0.05)
            {
                continue;
            }
            let met = masses.iter().any(|m| near(&m.footprint, p, 0.05))
                || city
                    .walls
                    .iter()
                    .enumerate()
                    .any(|(j, o)| j != k && near(&o.ring(), p, 0.05));
            if met {
                continue;
            }
            let far = p + dir * 2.0;
            // The first thing the line meets, and whether the end may stop on it: a house, or
            // another garden wall standing square to this one (met at a slant, the square end's
            // corner ran into it; along one in line, the end ran over it).
            let mut hit: Option<(f64, bool, Option<usize>)> = None;
            let mut try_poly = |poly: &[DVec2], ok: bool, wall: Option<usize>| {
                for i in 0..poly.len() {
                    if let Some((t, _)) =
                        geom::segment_intersection(p, far, poly[i], poly[(i + 1) % poly.len()])
                        && t > 1e-6
                        && hit.is_none_or(|(h, _, _)| t < h)
                    {
                        hit = Some((t, ok, wall));
                    }
                }
            };
            for m in &masses {
                try_poly(&m.footprint, true, None);
            }
            for (j, o) in city.walls.iter().enumerate() {
                if j != k {
                    let square = (o.b - o.a).normalize_or_zero().dot(dir).abs() < 0.2;
                    try_poly(&o.ring(), square, Some(j));
                }
            }
            if let Some((t, true, met)) = hit {
                let q = p + (far - p) * t;
                // Stopped on a wall's face, its end face along it: square, a wall met a few
                // degrees off square ran a corner 2 cm into it, the two tops on one plane there.
                // The wall the line met, not the first whose ring holds the point: at a joint
                // that could be one in line with this wall, its end then turned along itself.
                let face = met.map(|j| (city.walls[j].b - city.walls[j].a).normalize_or_zero());
                let w = &mut city.walls[k];
                if end == 0 {
                    w.a = q;
                } else {
                    w.b = q;
                }
                if let Some(d) = face {
                    let t = (w.b - w.a).normalize_or_zero();
                    let left = DVec2::new(-t.y, t.x);
                    w.ends[end] = if d.dot(left) < 0.0 { -d } else { d };
                }
            }
        }
    }
    // No plant stands in a garden wall: a hedge's shrub planted where a neighbour's wall ran
    // stood through it.
    let rings: Vec<Vec<DVec2>> = city.walls.iter().map(|w| w.ring()).collect();
    city.planted
        .retain(|t| !rings.iter().any(|r| geom::contains(r, t.pos)));
    Village {
        plan: city,
        blocks,
        without,
        masses,
    }
}

/// `lots` less each lot that has no house among `houses` and borders the `court` (a metre of side
/// at least), and the court grown by them: open ground beside the common ground is common ground.
/// Where the union would come out in pieces, the lot stays.
fn empty_to_court(
    lots: Vec<Vec<DVec2>>,
    mut court: Vec<DVec2>,
    houses: &[House],
) -> (Vec<Vec<DVec2>>, Vec<DVec2>) {
    let mut kept = Vec::with_capacity(lots.len());
    for lot in lots {
        if court.len() < 3 || houses.iter().any(|h| h.lot == lot) {
            kept.push(lot);
            continue;
        }
        let touching: f64 = (0..lot.len())
            .filter(|&i| {
                let (p, q) = (lot[i], lot[(i + 1) % lot.len()]);
                let n = court.len();
                (0..n).any(|k| {
                    let (u, v) = (court[k], court[(k + 1) % n]);
                    geom::distance_to_segment(p, u, v) < 0.01
                        && geom::distance_to_segment(q, u, v) < 0.01
                })
            })
            .map(|i| lot[i].distance(lot[(i + 1) % lot.len()]))
            .sum();
        let pieces: Vec<Vec<DVec2>> = super::ground::union_of(&court, &lot)
            .into_iter()
            .filter_map(|shape| shape.into_iter().next())
            .map(|p| geom::clean(&p, 1e-6))
            .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() > 0.05)
            .collect();
        // Never a court narrower than its least anywhere (a lot's width is under it).
        match <[Vec<DVec2>; 1]>::try_from(pieces) {
            Ok([mut one]) if touching > 1.0 && lots::court_is_wide(&one) => {
                if geom::signed_area(&one) < 0.0 {
                    one.reverse();
                }
                court = despike(&one);
            }
            _ => kept.push(lot),
        }
    }
    (kept, court)
}

/// `lots` with each lot that has no house among `houses` joined to the neighbour along its street
/// that has one (the two sharing a side, their fronts within 15°), the longer side shared first;
/// the house's lot follows, its front first as before. A lot with no such neighbour stays.
fn join_empty(mut lots: Vec<Vec<DVec2>>, houses: &mut [House]) -> Vec<Vec<DVec2>> {
    let housed = |lot: &[DVec2], houses: &[House]| houses.iter().position(|h| h.lot == lot);
    let front = |lot: &[DVec2]| (lot[1] - lot[0]).normalize_or_zero();
    // The length of the side two lots share: an edge of one on an edge of the other.
    let shared = |a: &[DVec2], b: &[DVec2]| -> f64 {
        let (n, m) = (a.len(), b.len());
        (0..n)
            .map(|i| {
                let (p, q) = (a[i], a[(i + 1) % n]);
                (0..m)
                    .filter(|&j| {
                        let (u, v) = (b[j], b[(j + 1) % m]);
                        geom::distance_to_segment(p, u, v) < 0.01
                            && geom::distance_to_segment(q, u, v) < 0.01
                    })
                    .map(|_| p.distance(q))
                    .fold(0.0, f64::max)
            })
            .sum()
    };
    // The empty lots that could not join (their union with the neighbour in two pieces).
    let mut stuck: Vec<Vec<DVec2>> = Vec::new();
    // The neighbour an empty lot `k` joins: along its street first, else (a short side's lot,
    // between two corner lots) the one it shares the most of its outline with.
    let partner = |k: usize, lots: &[Vec<DVec2>], houses: &[House], along: bool| {
        (0..lots.len())
            .filter(|&j| j != k && lots[j].len() >= 3)
            .filter(|&j| housed(&lots[j], houses).is_some())
            .filter(|&j| !along || front(&lots[j]).dot(front(&lots[k])) > 0.966)
            .map(|j| (j, shared(&lots[k], &lots[j])))
            .filter(|&(_, s)| s > 1.0)
            .max_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(j, _)| (k, j))
    };
    for _ in 0..2 * lots.len() {
        let empty = |k: &usize| {
            lots[*k].len() >= 3 && housed(&lots[*k], houses).is_none() && !stuck.contains(&lots[*k])
        };
        let Some((k, j)) = (0..lots.len())
            .filter(empty)
            .find_map(|k| partner(k, &lots, houses, true))
            .or_else(|| {
                (0..lots.len())
                    .filter(empty)
                    .find_map(|k| partner(k, &lots, houses, false))
            })
        else {
            break;
        };
        // One piece, or the two do not truly share their side: then the empty lot stays.
        let pieces: Vec<Vec<DVec2>> = super::ground::union_of(&lots[j], &lots[k])
            .into_iter()
            .filter_map(|shape| shape.into_iter().next())
            .map(|p| geom::clean(&p, 1e-6))
            .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() > 0.05)
            .collect();
        let [mut one] = <[Vec<DVec2>; 1]>::try_from(pieces).unwrap_or_else(|_| [Vec::new()]);
        if one.len() < 3 {
            stuck.push(lots[k].clone());
            continue;
        }
        if geom::signed_area(&one) < 0.0 {
            one.reverse();
        }
        // Joined only where the two stay near a rectangle: an L round a block's corner is no
        // village lot (the court takes the empty one instead).
        if lots::squareness(&one) < lots::SQUARE {
            stuck.push(lots[k].clone());
            continue;
        }
        // The union's corners back on the two lots' own: the booleans move them by 1e-8 (their
        // fixed-point grid), and a lot shares its corners with its other neighbours.
        for p in &mut one {
            if let Some(&q) = lots[j]
                .iter()
                .chain(&lots[k])
                .find(|q| q.distance(*p) < 1e-5)
            {
                *p = q;
            }
        }
        // Where the two shared only part of a side, the union ran out to a point and back.
        let mut one = despike(&one);
        // Its front first: the union's edge along the street that holds the housed lot's front
        // (the two fronts made one where the empty lot came first, its old first corner gone).
        let (f0, f1) = (lots[j][0], lots[j][1]);
        let along = (f1 - f0).normalize_or_zero();
        let mid = (f0 + f1) * 0.5;
        let m = one.len();
        let Some(at) = (0..m).find(|&e| {
            let (p, q) = (one[e], one[(e + 1) % m]);
            geom::distance_to_segment(mid, p, q) < 1e-3
                && (q - p).normalize_or_zero().dot(along) > 0.99
        }) else {
            stuck.push(lots[k].clone());
            continue;
        };
        one.rotate_left(at);
        if let Some(h) = housed(&lots[j], houses) {
            houses[h].lot.clone_from(&one);
        }
        lots[j] = one;
        lots.remove(k);
    }
    lots
}

/// `ring` without its spikes (a point the outline turns back at, out and in along one line) and
/// its points in line with their neighbours.
fn despike(ring: &[DVec2]) -> Vec<DVec2> {
    let mut out = geom::clean(ring, 1e-3);
    let mut changed = true;
    while changed && out.len() > 3 {
        changed = false;
        let n = out.len();
        for i in 0..n {
            let (a, b, c) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
            let (u, v) = ((b - a).normalize_or_zero(), (c - b).normalize_or_zero());
            // Back the way it came, or straight on.
            if u.dot(v) < -0.999 || (u.perp_dot(v).abs() < 1e-3 && u.dot(v) > 0.0) {
                out.remove(i);
                changed = true;
                break;
            }
        }
    }
    out
}

/// `lots` with every lot's corners lying on another's edges put into those edges (a row's lots
/// starting on the back line of the side that took the corner, `lots::cut`): the walls and
/// hedges meet there point for point, and two lots joined share their side vertex for vertex
/// (with a corner on the other's edge, their union came out in two pieces).
fn conform(lots: &[Vec<DVec2>]) -> Vec<Vec<DVec2>> {
    let corners: Vec<DVec2> = lots.iter().flatten().copied().collect();
    lots.iter()
        .map(|lot| {
            let n = lot.len();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let (a, b) = (lot[i], lot[(i + 1) % n]);
                out.push(a);
                // Its street side whole: the front its gate and its house go by.
                if i == 0 {
                    continue;
                }
                let length = a.distance(b);
                let mut on: Vec<(f64, DVec2)> = corners
                    .iter()
                    .filter(|&&c| geom::distance_to_segment(c, a, b) < 0.02)
                    .map(|&c| (geom::closest_on_segment(c, a, b) * length, c))
                    .filter(|&(t, _)| t > 0.05 && t < length - 0.05)
                    .collect();
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                on.dedup_by(|x, y| (x.0 - y.0).abs() < 0.05);
                out.extend(on.into_iter().map(|(_, c)| c));
            }
            out
        })
        .collect()
}

/// The shallowest yard behind a house that a cut shallower for its block's court leaves, metres
/// ([`village`]).
const LEAST_YARD: f64 = 4.0;

/// Metres a garden plant's crown keeps from a wall's face or a house's.
const CROWN_CLEAR: f64 = 0.15;

/// How much wider a drawn crown may stand than planted: the village draws each plant stretched
/// by up to 8 % along one axis (city-plan's `stretch`).
const CROWN_STRETCH: f64 = 1.08;

/// The narrowest a hedge's plant may be fitted to its room, metres of crown radius: narrower, it
/// is left out.
const HEDGE_LEAST: f64 = 0.3;

/// The narrowest an olive's crown may be fitted to its room, metres: narrower, none stands there.
const OLIVE_LEAST: f64 = 1.2;

/// House `h` of block `b` as a mass on its lot, appended to the plan's lots: its storeys from
/// its plan at old stone's heights, a tiled roof, homes only.
fn mass(city: &mut CityPlan, b: usize, h: &House, streets: &PointGrid) -> Mass {
    let lot = city.lots.len() as u32;
    city.lots.push(Lot {
        block: b as u32,
        polygon: h.lot.clone(),
        area: geom::signed_area(&h.lot).abs(),
        frontage: true,
    });
    let mut footprint = h.footprint.clone();
    if geom::signed_area(&footprint) < 0.0 {
        footprint.reverse();
    }
    let floors = h.plan.storeys.len().clamp(1, 255) as u8;
    let storeys = Era::OldStone.storeys(floors, false);
    let height = storeys.iter().sum();
    let street_width = 2.0 * blocks::edge_half_width(streets, &city.roads, h.lot[0], h.lot[1]);
    Mass {
        lot,
        footprint,
        floors,
        storeys,
        height,
        roof: Roof::Tiled,
        street_width,
        open: false,
        active: false,
        offices: 0,
        home: Some(
            (f64::from(h.household.0) * units::ROOMS_A_PERSON)
                .ceil()
                .min(255.0) as u8,
        ),
        rooms: Some(h.plan.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::example;
    use super::super::structure::{self, OpeningKind, WallRole};
    use super::*;

    /// The smaller villages keep houses (#308): at 400 m the plan's only block became a park for
    /// itself, and at 500 m the square took the block the park had left.
    #[test]
    fn the_smaller_villages_have_houses() {
        for (size, least) in [(400.0, 9), (500.0, 7)] {
            let v = village(&example::site(size, 10.0), &params(size), 1);
            let uses: Vec<String> = v
                .plan
                .blocks
                .iter()
                .map(|b| format!("{:?}", b.use_))
                .collect();
            let houses = v.houses().count();
            println!("{size} m: {houses} houses, blocks {uses:?}");
            assert!(houses >= least, "{size} m: {houses} houses");
        }
    }

    /// No garden plant's crown reaches into a garden wall or a house (the owner, 2026-10-25:
    /// "some bushes in the village are going through the walls (even some houses here and
    /// there)"): every hedge plant and olive keeps its drawn crown clear of their faces.
    #[test]
    fn the_gardens_crowns_keep_clear_of_the_walls_and_houses() {
        let size = 600.0;
        let v = village(&example::site(size, 10.0), &example::params(size), 1);
        let houses: Vec<&Vec<DVec2>> = v.houses().map(|h| &h.footprint).collect();
        let mut checked = 0;
        for t in v
            .plan
            .planted
            .iter()
            .filter(|t| t.setting == Setting::Garden)
        {
            let wall = v
                .plan
                .walls
                .iter()
                .map(|w| geom::distance_to_segment(t.pos, w.a, w.b) - walls::THICKNESS)
                .fold(f64::INFINITY, f64::min);
            let house = houses
                .iter()
                .flat_map(|f| {
                    (0..f.len())
                        .map(|k| geom::distance_to_segment(t.pos, f[k], f[(k + 1) % f.len()]))
                })
                .fold(f64::INFINITY, f64::min);
            let reach = t.crown * CROWN_STRETCH;
            assert!(
                reach <= wall.min(house) + 1e-9,
                "a crown of {:.2} m at {:?}: {wall:.2} m from a wall's face, {house:.2} from a house",
                t.crown,
                t.pos
            );
            checked += 1;
        }
        assert!(checked > 100, "{checked} garden plants");
    }

    #[test]
    fn the_villages_own_plan_has_lanes_houses_and_no_trams() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village(&site, &params(size), 1);
        let built = v
            .plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Built)
            .count();
        let houses = v.houses().count();
        println!(
            "the village's own plan: {} nodes, {} edges, {} blocks ({built} built), {} with \
             strips, {} without, {houses} houses, {} people",
            v.plan.roads.nodes.len(),
            v.plan.roads.edges.len(),
            v.plan.blocks.len(),
            v.blocks.len(),
            v.without,
            v.houses().map(|h| h.household.0).sum::<u32>()
        );
        let trees = super::super::trees::trees(&site, &v.plan, 1);
        let forest = trees
            .iter()
            .filter(|t| t.setting == super::super::trees::Setting::Forest)
            .count();
        println!("{} trees, {forest} in the forest", trees.len());
        assert!(forest >= 500, "{forest} forest trees");
        let mut by_profile = std::collections::BTreeMap::new();
        for e in &v.plan.roads.edges {
            *by_profile.entry(format!("{:?}", e.profile)).or_insert(0) += 1;
        }
        println!("edges by profile: {by_profile:?}");
        assert!(
            !by_profile.contains_key("Avenue"),
            "an avenue through the village"
        );
        let rings: Vec<String> = v
            .blocks
            .iter()
            .map(|b| {
                format!(
                    "{:.0} m: {} lots, {} houses",
                    b.houses.first().map_or(0.0, |h| h.plan.depth),
                    b.lots.len(),
                    b.houses.len()
                )
            })
            .collect();
        println!("blocks: {rings:?}");
        // The distribution across the area (the owner's question, the cadastre research §2):
        // per block its ring (by the strip's depth), its lots' mean area, the share of lots
        // with land left beside their house (15 m² or more), and the mean land left.
        for b in &v.blocks {
            let ring = if b.ty.setback > 0.0 {
                "lotissement"
            } else if b.ty.yard >= 12.0 {
                "faubourg"
            } else {
                "core"
            };
            let areas: Vec<(f64, f64)> = b
                .houses
                .iter()
                .map(|h| {
                    (
                        geom::signed_area(&h.lot).abs(),
                        geom::signed_area(&h.footprint).abs(),
                    )
                })
                .collect();
            let n = areas.len().max(1) as f64;
            let lot = areas.iter().map(|a| a.0).sum::<f64>() / n;
            let land: Vec<f64> = areas.iter().map(|a| a.0 - a.1).collect();
            let with = land.iter().filter(|&&l| l >= 15.0).count();
            println!(
                "{ring}: {} houses, lots of {lot:.0} m², {with} with land ({:.0} m² left on average)",
                areas.len(),
                land.iter().sum::<f64>() / n
            );
        }
        let rich: Vec<String> = v
            .houses()
            .filter(|h| h.household.1 == Means::Rich)
            .map(|h| {
                format!(
                    "{} people, {:.1} m wide, {} storeys",
                    h.household.0,
                    h.plan.width,
                    h.plan.storeys.len()
                )
            })
            .collect();
        println!("rich houses: {rich:?}");
        assert!(
            v.houses().any(|h| h.household == (5, Means::Rich)),
            "the rich house on the square"
        );
        assert!(v.plan.tram_lines.is_empty() && v.plan.landmarks.is_empty());
        assert!(built >= 4, "{built} built blocks");
        assert!(houses >= 40, "{houses} houses");
        // The houses' insides: their rooms laid beside the hall where the core stands, every
        // room reached from the hall through doorways.
        let structures = structure::structures(&v.plan, &v.masses);
        let insides: Vec<_> = v
            .masses
            .iter()
            .zip(&structures)
            .filter_map(|(m, s)| super::super::interior::house(m, s))
            .collect();
        let partitions: usize = insides.iter().map(|i| i.partitions.len()).sum();
        let doorways: usize = insides
            .iter()
            .flat_map(|i| &i.partitions)
            .map(|p| p.doors.len())
            .sum();
        println!(
            "interiors: {} of {houses} houses, {partitions} partitions, {doorways} doorways",
            insides.len()
        );
        // Most houses: those on a lot narrowing to a wedge behind the hall stay closed (their
        // rooms would stand past their slanted walls), 8 of 77 on 2026-10-16.
        assert!(
            insides.len() * 20 >= houses * 17,
            "{} interiors of {houses}",
            insides.len()
        );
        for (k, i) in insides.iter().enumerate() {
            assert!(
                super::super::interior::reachable(i),
                "house {k}: a room not reached"
            );
        }
        // A garden door at the back of the hall, besides the street's.
        let garden_doors = v
            .masses
            .iter()
            .zip(&structures)
            .filter(|(m, s)| {
                super::super::interior::house(m, s).is_some()
                    && s.openings
                        .iter()
                        .filter(|o| o.storey == 0 && o.kind == OpeningKind::Door)
                        .count()
                        >= 2
            })
            .count();
        println!("{garden_doors} of them with a garden door");
        let against: usize = v
            .masses
            .iter()
            .zip(&structures)
            .filter_map(|(m, s)| {
                super::super::interior::house(m, s)
                    .map(|i| super::super::interior::at_openings(s, &i))
            })
            .sum();
        println!("{against} partition ends against a window or a door");
        assert!(against <= 8, "{against} partition ends against an opening");
        assert!(
            garden_doors * 2 >= insides.len(),
            "{garden_doors} garden doors"
        );
        // The garden walls: along the streets only (D-083's fifth step: hedges between the
        // gardens), but for the short walls closing a garden onto a house standing back.
        let along = |street: bool| -> f64 {
            v.plan
                .walls
                .iter()
                .filter(|w| w.inside.is_some() == street)
                .map(walls::Wall::length)
                .sum()
        };
        let gates: usize = v.blocks.iter().map(|b| b.gates.len()).sum();
        println!(
            "garden walls: {:.0} m along the streets and lanes, {:.0} m closing gardens onto houses, {gates} gates",
            along(true),
            along(false)
        );
        assert!(along(true) > 100.0 && gates >= 5);
        assert!(
            v.plan.walls.iter().all(|w| w.inside.is_some() || w.cut),
            "a wall between gardens"
        );
        // The gardens' olives, behind the houses.
        let olives = v.plan.planted.len();
        println!("{olives} olives in the gardens");
        assert!(olives >= 20, "{olives} olives");
        for (i, w) in v.plan.walls.iter().enumerate() {
            let mid = (w.a + w.b) * 0.5;
            assert!(
                v.houses().all(|h| !geom::contains(&h.footprint, mid)),
                "a garden wall through a house at {mid}"
            );
            // No two walls share ground (their tops would lie on one plane).
            for x in &v.plan.walls[i + 1..] {
                if x.a.distance(w.a) > w.length() + x.length() + 1.0 {
                    continue;
                }
                let shared = walls::overlap(&w.ring(), &x.ring());
                assert!(
                    shared < 1e-3,
                    "garden walls share {shared:.4} m²: {w:?} {x:?}"
                );
            }
        }
        // Walls in line meet without a slot or a step (the owner's sighting of 2026-10-16: a
        // dark upright strip where two garden walls met). For each two walls nearly in line,
        // their facing ends within 0.75 m and within a wall's thickness of each other's line
        // (a jog of a whole thickness is a corner): either their corners meet, or a third wall
        // stands between them.
        let mut slots = Vec::new();
        for (i, w) in v.plan.walls.iter().enumerate() {
            let t = (w.b - w.a).normalize_or_zero();
            for x in &v.plan.walls[i + 1..] {
                let u = (x.b - x.a).normalize_or_zero();
                if t.perp_dot(u).abs() > 0.3 {
                    continue;
                }
                let (gap, e, f) = [(0, 0), (0, 1), (1, 0), (1, 1)]
                    .map(|(e, f)| {
                        let p = if e == 0 { w.a } else { w.b };
                        let q = if f == 0 { x.a } else { x.b };
                        (p.distance(q), e, f)
                    })
                    .into_iter()
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .expect("four pairs");
                let (p, q) = (
                    if e == 0 { w.a } else { w.b },
                    if f == 0 { x.a } else { x.b },
                );
                if gap >= 0.75 || (q - p).perp_dot(t).abs() >= 0.35 {
                    continue;
                }
                // Each wall's two corners at the joint (ring 0 and 3 at `a`, 1 and 2 at `b`).
                let corners = |r: Vec<DVec2>, at_a: bool| {
                    if at_a { [r[0], r[3]] } else { [r[1], r[2]] }
                };
                let (cw, cx) = (corners(w.ring(), e == 0), corners(x.ring(), f == 0));
                let apart = cw
                    .iter()
                    .map(|p| cx.iter().map(|q| p.distance(*q)).fold(f64::MAX, f64::min))
                    .fold(0.0, f64::max);
                let mid = (cw[0] + cw[1] + cx[0] + cx[1]) * 0.25;
                let filled = v.plan.walls.iter().any(|o| {
                    !std::ptr::eq(o, w) && !std::ptr::eq(o, x) && geom::contains(&o.ring(), mid)
                });
                if apart > 0.02 && !filled {
                    slots.push((apart, mid));
                }
            }
        }
        assert!(
            slots.is_empty(),
            "walls in line with a slot or a step between them: {slots:?}"
        );
        if let Ok(path) = std::env::var("VILLAGE_OWN_PNG") {
            super::super::map::write_village(&v, std::path::Path::new(&path), 0.25)
                .expect("the drawing");
        }
    }

    #[test]
    fn the_example_village_has_its_houses_on_strips() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village(&site, &example::params(size), 1);
        let houses = v.houses().count();
        let people: u32 = v.houses().map(|h| h.household.0).sum();
        println!(
            "{} blocks with strips, {} without, {houses} houses, {people} people",
            v.blocks.len(),
            v.without
        );
        assert!(v.blocks.len() >= 4, "{} blocks with strips", v.blocks.len());
        assert!(houses >= 60, "{houses} houses");
        for h in v.houses() {
            assert!(!h.plan.storeys.is_empty() && h.footprint.len() >= 3);
        }
        // The masses and their structure (D-060) as the city's: each on its own lot, old stone,
        // a street wall each, most with a door and the storeyed ones with a stair.
        assert_eq!(v.masses.len(), houses);
        for (m, h) in v.masses.iter().zip(v.houses()) {
            assert_eq!(v.plan.lots[m.lot as usize].polygon, h.lot);
            assert!(geom::signed_area(&m.footprint) > 0.0);
            assert_eq!(usize::from(m.floors), h.plan.storeys.len());
        }
        let structures = structure::structures(&v.plan, &v.masses);
        assert_eq!(structures.len(), houses);
        let (mut doors, mut cores, mut storeyed, mut straight) = (0, 0, 0, 0);
        for s in &structures {
            assert_eq!(s.era, Era::OldStone);
            assert!(
                s.walls.iter().any(|w| w.role == WallRole::Street),
                "mass {} has no street wall",
                s.mass
            );
            doors += usize::from(s.openings.iter().any(|o| o.kind == OpeningKind::Door));
            cores += usize::from(!s.cores.is_empty());
            straight += usize::from(s.cores.iter().any(|c| c.house));
            storeyed += usize::from(s.storeys.len() >= 2);
        }
        let mut by_storeys = std::collections::BTreeMap::new();
        for h in v.houses() {
            *by_storeys.entry(h.plan.storeys.len()).or_insert(0) += 1;
        }
        println!("houses by storeys: {by_storeys:?}");
        println!(
            "{doors} houses with a door, {cores} with a stair core ({straight} with a house's \
             straight flight), {storeyed} storeyed"
        );
        let (mut households, mut residents) = (0, 0);
        for s in &structures {
            let o = units::occupancy(s);
            households += o.households;
            residents += o.residents;
        }
        println!(
            "{households} households and {residents} residents by the structures, {people} by the village"
        );
        assert!(
            households as usize <= houses && households as usize * 10 >= houses * 9,
            "{households} households in {houses} houses"
        );
        assert!(
            cores * 10 >= storeyed * 9,
            "{cores} of {storeyed} storeyed houses have a stair"
        );
        assert!(
            doors * 10 >= houses * 9,
            "{doors} of {houses} houses have a door"
        );
        if let Ok(path) = std::env::var("VILLAGE_PNG") {
            super::super::map::write_village(&v, std::path::Path::new(&path), 0.25)
                .expect("the drawing");
        }
    }
}
