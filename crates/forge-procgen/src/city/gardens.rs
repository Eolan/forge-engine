//! What the homes keep round them on the ground (D-060's ground floors, the owner's note of
//! 2026-10-15; `research/ground-floors.md` §4.6–4.8):
//! - the paths across each green court, from the gaps between the buildings, the passages and
//!   the court doors, joined into one network round the court's middle;
//! - a shared kitchen garden of raised beds at the middle of each large court, a paved ring
//!   round it where the paths meet;
//! - the court-side ground-floor flats (the *rez-de-jardin*): about half with a private garden
//!   behind hedges, a gate onto the court, the others with a terrace, each opening onto its
//!   terrace through a glazed bay;
//! - planted beds before the homes on the streets, broken at the doors, and low hedges along
//!   the gaps between the buildings.
//!
//! All of it at the walking level and step-free: the beds raised for a seat, the paths level
//! and wide enough for a wheelchair, a gate in every garden's hedge.

use std::collections::{HashMap, HashSet};

use glam::DVec2;

use super::geom::{self, PointGrid};
use super::mass::Mass;
use super::structure::{Opening, OpeningKind, PIER, Structure, Wall, WallRole};
use super::{BlockUse, CityPlan};

/// What a piece of the homes' ground is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A path across a court's green.
    Path,
    /// A paved terrace before a court-side flat, flush with its floor.
    Terrace,
    /// A garden's lawn.
    Lawn,
    /// A kitchen garden's paved ground, its beds on it.
    Plot,
    /// A planted bed before a home on the street.
    Planter,
    /// A hedge: between neighbours' gardens and terraces, and lower along a garden's court side.
    Hedge,
    /// A kitchen garden's raised bed.
    Bed,
    /// A bench on the green beside a court's middle, facing it: its outline's first edge is its
    /// front, [`BENCH`] long.
    Bench,
    /// A row of bike stands on a gap's forecourt, along its hedge: its outline's first edge
    /// runs along the row, [`BIKE_RACK`] long.
    BikeRack,
}

impl Kind {
    /// Whether it is paved ground at the walking level, one surface with the block's forecourt.
    pub fn paved(self) -> bool {
        matches!(self, Kind::Path | Kind::Terrace | Kind::Plot)
    }

    /// Whether it stands over the ground, something people walk round.
    pub fn standing(self) -> bool {
        matches!(
            self,
            Kind::Planter | Kind::Hedge | Kind::Bed | Kind::Bench | Kind::BikeRack
        )
    }
}

/// A piece of the homes' ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// What it is.
    pub kind: Kind,
    /// The block it belongs to.
    pub block: u32,
    /// Its outline on the plan, counter-clockwise.
    pub outline: Vec<DVec2>,
    /// Its top over the walking level, metres (0 for the paved ground and the lawns).
    pub height: f64,
}

/// A court-side ground-floor flat: its stretch of a court wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flat {
    /// Its building, an index into the masses.
    pub mass: u32,
    /// Its wall, an index into the structure's walls.
    pub wall: usize,
    /// Its stretch along the wall, metres from the wall's `a`.
    pub from: f64,
    /// The stretch's other end.
    pub to: f64,
    /// Whether it has a garden; else a terrace.
    pub garden: bool,
}

/// A built block's paved forecourt along its streets, metres deep from the building line: the
/// pavement it meets, carried into the gaps between the buildings.
pub const FORECOURT: f64 = 2.0;
/// A court-side flat's width along its wall, metres: one garden or terrace each.
pub const FLAT: f64 = 8.0;
/// A garden's depth out from its wall, metres: 36 m² for a flat, Home Select's 25–40.
pub const GARDEN: f64 = 4.5;
/// A garden's terrace, metres deep: room for a table before the glazed bay.
const TERRACE: f64 = 2.5;
/// A flat without a garden: its terrace, metres deep (1.5 at least).
const PLAIN_TERRACE: f64 = 1.8;
/// The share of the court-side flats with a garden (40–60 %).
const GARDENED: f64 = 0.5;
/// A hedge's width, and its heights between neighbours (under 2 m) and along the court, metres.
const HEDGE: (f64, f64, f64) = (0.5, 1.4, 0.8);
/// A garden gate's width, metres (0.9 m clear at least).
const GATE: f64 = 1.2;
/// A glazed sliding bay onto a terrace, metres wide.
pub const BAY: f64 = 2.4;
/// A street planter: its depth out from the facade and its height, metres. The footway keeps
/// 2.6 m on an avenue, and the walkers pass 3 m out (`crowd`).
pub const PLANTER: (f64, f64) = (0.9, 0.45);
/// The narrowest street that gets planters, metres between the building lines: on the paths'
/// 4 m the walkers pass a metre from the walls.
const PLANTER_STREET: f64 = 8.0;
/// The room left clear before a door or a passage, either side of it, metres.
const DOOR_CLEAR: f64 = 0.8;
/// A path's half width from a gap between the buildings or a passage (2.4 m), and from a court
/// door (1.8 m: a wheelchair and someone walking pass), metres.
const PATH_HALF: (f64, f64) = (1.2, 0.9);
/// A kitchen garden's raised bed: its width (reached from both sides), length and height for a
/// seated gardener, metres (Mably's accessible garden: 0.6 m seated, 1.2 m from both sides).
const BED: (f64, f64, f64) = (1.2, 4.0, 0.7);
/// The paths between and round the beds, metres (1.2–1.4).
const BED_PATH: f64 = 1.4;
/// The smallest court that gets a kitchen garden, m².
const KITCHEN_COURT: f64 = 600.0;
/// The paved ring round a kitchen garden, metres wide, where the court's paths meet.
const RING: f64 = 1.8;
/// The round paved place where a court without a kitchen garden has its paths meet, metres
/// across.
const PLAZA: f64 = 7.0;
/// Past this angle between the way to the nearest path and the way to the hub, a path goes to
/// the hub instead of joining, degrees.
const JOIN_ANGLE: f64 = 75.0;
/// A bench: its length and depth (`forge_geom::furniture::bench`), and how far its front stands
/// off the paving it faces, metres.
pub const BENCH: (f64, f64, f64) = (1.8, 0.5, 0.1);
/// The most benches round a court's middle, and the least room between two of them, metres.
const BENCHES: (usize, f64) = (4, 3.0);
/// A row of bike stands (`forge_geom::furniture::bike_rack`): its length, depth and height,
/// metres.
pub const BIKE_RACK: (f64, f64, f64) = (2.7, 0.3, 0.8);

/// A court's middle ([`court_middle`]): its pieces, its rim's segments, and its benches' places
/// with the way each would face.
type Middle = (Vec<Piece>, Vec<(DVec2, DVec2)>, Vec<(DVec2, DVec2)>);

/// The court of block `b` when it is a green one: a built block's, outside a taxi or a tram
/// depot.
fn green_court(plan: &CityPlan, b: u32) -> Option<&[DVec2]> {
    let block = &plan.blocks[b as usize];
    if block.use_ != BlockUse::Built
        || plan.depots.iter().any(|d| d.block == b)
        || plan.tram_depots.iter().any(|d| d.block == b)
    {
        return None;
    }
    block.courtyard.as_deref()
}

/// A wall's direction and its outward normal (the side its mass is not on).
fn frame(m: &Mass, wall: &Wall) -> (DVec2, DVec2) {
    let t = (wall.b - wall.a).normalize_or_zero();
    let n = DVec2::new(t.y, -t.x);
    let mid = (wall.a + wall.b) * 0.5;
    if geom::contains(&m.footprint, mid + n * 0.3) {
        (t, -n)
    } else {
        (t, n)
    }
}

/// A rectangle along a wall: from `s0` to `s1` along it, `d0` to `d1` out from it.
fn rect(wall: &Wall, t: DVec2, n: DVec2, (s0, s1): (f64, f64), (d0, d1): (f64, f64)) -> Vec<DVec2> {
    let (p, q) = (wall.a + t * s0, wall.a + t * s1);
    ccw(vec![p + n * d0, q + n * d0, q + n * d1, p + n * d1])
}

/// An outline turned counter-clockwise.
fn ccw(mut outline: Vec<DVec2>) -> Vec<DVec2> {
    if geom::signed_area(&outline) < 0.0 {
        outline.reverse();
    }
    outline
}

/// A polyline moved `d` metres to its left.
fn offset(points: &[DVec2], d: f64) -> Vec<DVec2> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let prev = if i > 0 {
                points[i] - points[i - 1]
            } else {
                points[1] - points[0]
            };
            let next = if i + 1 < n {
                points[i + 1] - points[i]
            } else {
                prev
            };
            let t = (prev.normalize_or_zero() + next.normalize_or_zero()).normalize_or_zero();
            points[i] + DVec2::new(-t.y, t.x) * d
        })
        .collect()
}

/// A path's outline along `line`, `half` metres either side, its points every 2 m.
fn path(line: &[DVec2], half: f64) -> Option<Vec<DVec2>> {
    let line = geom::resample(line, 2.0);
    if line.len() < 2 {
        return None;
    }
    let mut outline = offset(&line, half);
    outline.extend(offset(&line, -half).into_iter().rev());
    Some(outline)
}

/// Where a path comes into a court: its mouth (a gap's on the street's middle, a door's or a
/// passage's on its wall), where it has left the private gardens behind, and its half width.
struct Entrance {
    mouth: DVec2,
    knee: DVec2,
    half: f64,
}

/// The nearest point to `p` on any of `segments`.
fn nearest_on(segments: &[(DVec2, DVec2)], p: DVec2) -> Option<DVec2> {
    segments
        .iter()
        .map(|&(a, b)| a + (b - a) * geom::closest_on_segment(p, a, b))
        .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
}

/// The paths across each green court (the owner's photos of open blocks: a shared green with
/// paths through it), as one network round its middle (the owner: close paths join first,
/// round the garden, like a hub):
/// - its middle: a shared kitchen garden of raised beds with a paved ring round it, in the
///   courts of [`KITCHEN_COURT`] or more where one fits, else a small round paved place;
/// - a path in from each gap between the buildings (a street lot left open) and each passage,
///   2.4 m wide, and from each court door, 1.8 m: straight out past the private gardens, then to
///   the nearest point of the network laid so far, the nearest entrances first, so that close
///   paths join instead of running side by side; to the middle itself where joining would turn
///   them away from it.
pub fn court_paths(plan: &CityPlan, masses: &[Mass], structures: &[Structure]) -> Vec<Piece> {
    let built: HashSet<u32> = masses.iter().map(|m| m.lot).collect();
    let mut entrances: std::collections::BTreeMap<u32, Vec<Entrance>> = Default::default();
    for (i, lot) in plan.lots.iter().enumerate() {
        let Some(court) = green_court(plan, lot.block) else {
            continue;
        };
        if !lot.frontage || built.contains(&(i as u32)) {
            continue;
        }
        // From the block's edge nearest the gap's middle, into the court.
        let block = &plan.blocks[lot.block as usize];
        let middle = geom::centroid(&lot.polygon);
        let n = block.polygon.len();
        let from = (0..n)
            .map(|k| {
                let (a, b) = (block.polygon[k], block.polygon[(k + 1) % n]);
                a + (b - a) * geom::closest_on_segment(middle, a, b)
            })
            .min_by(|p, q| p.distance(middle).total_cmp(&q.distance(middle)));
        let Some(mouth) = from else {
            continue;
        };
        let hub = geom::centroid(court);
        let way = (hub - mouth).normalize_or_zero();
        let reach = mouth.distance(hub);
        let knee = (0..reach as usize)
            .map(|k| mouth + way * k as f64)
            .find(|&p| geom::contains(court, p))
            .unwrap_or(hub);
        entrances.entry(lot.block).or_default().push(Entrance {
            mouth,
            knee,
            half: PATH_HALF.0,
        });
    }
    for (m, s) in masses.iter().zip(structures) {
        let block = plan.lots[m.lot as usize].block;
        if green_court(plan, block).is_none() {
            continue;
        }
        for o in &s.openings {
            let wall = &s.walls[o.wall];
            let half = match o.kind {
                OpeningKind::Passage => PATH_HALF.0,
                OpeningKind::Door => PATH_HALF.1,
                _ => continue,
            };
            if o.storey != 0 || wall.role != WallRole::Court {
                continue;
            }
            let (t, n) = frame(m, wall);
            let mouth = wall.a + t * o.along;
            entrances.entry(block).or_default().push(Entrance {
                mouth,
                knee: mouth + n * (GARDEN + 1.0),
                half,
            });
        }
    }
    let mut out = Vec::new();
    // Each block's buildings, which its benches keep out of (a wing can reach into the court).
    let mut buildings: HashMap<u32, Vec<&[DVec2]>> = HashMap::new();
    for m in masses {
        let block = plan.lots[m.lot as usize].block;
        buildings.entry(block).or_default().push(&m.footprint);
    }
    for (b, mut list) in entrances {
        let Some(court) = green_court(plan, b) else {
            continue;
        };
        let built_here = buildings.get(&b).map_or(&[][..], Vec::as_slice);
        let (pieces, rim, seats) = court_middle(court, b);
        let first = out.len();
        out.extend(pieces);
        let hub = geom::centroid(court);
        let rim = if rim.is_empty() {
            vec![(hub, hub)]
        } else {
            rim
        };
        // The nearest first: the further ones join them.
        let to_rim =
            |e: &Entrance| nearest_on(&rim, e.knee).map_or(f64::MAX, |p| p.distance(e.knee));
        list.sort_by(|x, y| to_rim(x).total_cmp(&to_rim(y)));
        let mut network = rim.clone();
        for e in &list {
            let (Some(join), Some(middle)) =
                (nearest_on(&network, e.knee), nearest_on(&rim, e.knee))
            else {
                continue;
            };
            let (a, c) = (
                (join - e.knee).normalize_or_zero(),
                (middle - e.knee).normalize_or_zero(),
            );
            let to = if a.dot(c) >= JOIN_ANGLE.to_radians().cos() {
                join
            } else {
                middle
            };
            let mut line = vec![e.mouth];
            if e.knee.distance(e.mouth) > 0.5 && e.knee.distance(to) > 1.0 {
                line.push(e.knee);
            }
            line.push(to);
            if let Some(outline) = path(&line, e.half) {
                out.push(Piece {
                    kind: Kind::Path,
                    block: b,
                    outline,
                    height: 0.0,
                });
            }
            // The rest join this path where it has left the gardens.
            network.push((e.knee, to));
        }
        // Benches on the green beside the middle, facing it, where no path arrives; spread
        // round it, the first candidates first.
        let mut placed: Vec<DVec2> = Vec::new();
        for (at, facing) in seats {
            let outline = bench(at, facing);
            let clear = outline.iter().all(|&p| geom::contains(court, p))
                && out[first..]
                    .iter()
                    .all(|p| !geom::overlaps(&p.outline, &outline))
                && built_here.iter().all(|f| !geom::overlaps(f, &outline));
            if clear
                && placed.len() < BENCHES.0
                && placed.iter().all(|q| q.distance(at) >= BENCHES.1)
            {
                placed.push(at);
                out.push(Piece {
                    kind: Kind::Bench,
                    block: b,
                    outline,
                    height: 0.85,
                });
            }
        }
    }
    out
}

/// A bench's footprint standing at `at`, facing `facing` (a unit vector): counter-clockwise,
/// its front edge first.
fn bench(at: DVec2, facing: DVec2) -> Vec<DVec2> {
    let (long, deep, _) = BENCH;
    let along = DVec2::new(-facing.y, facing.x) * (long * 0.5);
    let front = facing * (deep * 0.5);
    vec![
        at + front - along,
        at + front + along,
        at - front + along,
        at - front - along,
    ]
}

/// A green court's middle and the rim its paths join: in a court of [`KITCHEN_COURT`] or more,
/// a shared kitchen garden (raised beds in rows on paved ground with paths between them, at
/// seated height for a wheelchair) in a paved ring [`RING`] wide, as near the court's middle
/// as fits, else on its sunniest side (north, −y on the plan, catches the most sun), clear of
/// the private gardens; else a round paved place [`PLAZA`] across at the middle. Its pieces,
/// the rim's segments, and the places a bench could stand with the way it would face, the
/// likeliest first.
fn court_middle(court: &[DVec2], b: u32) -> Middle {
    let (bw, bl, bh) = BED;
    let hub = geom::centroid(court);
    let mut pieces = Vec::new();
    let inner = geom::inset(court, GARDEN + 1.0);
    if geom::signed_area(court).abs() >= KITCHEN_COURT
        && let Some(inner) = &inner
    {
        let (u, _, _) = geom::oriented_extent(court);
        let v = DVec2::new(-u.y, u.x);
        // The middle first, then eight ways out of it, the sunniest first.
        let mut ways = [
            u,
            -u,
            v,
            -v,
            (u + v).normalize(),
            (u - v).normalize(),
            (v - u).normalize(),
            (-u - v).normalize(),
        ];
        ways.sort_by(|p, q| p.y.total_cmp(&q.y));
        let placed = [(4_usize, 2_usize), (2, 2)]
            .into_iter()
            .find_map(|(cols, rows)| {
                let wide = cols as f64 * bw + (cols + 1) as f64 * BED_PATH;
                let deep = rows as f64 * bl + (rows + 1) as f64 * BED_PATH;
                std::iter::once(hub)
                    .chain(
                        ways.iter()
                            .flat_map(|&w| (1..20).map(move |k| hub + w * k as f64 * 2.0)),
                    )
                    .find_map(|centre| {
                        let corner = move |x: f64, y: f64| {
                            centre + u * (x - wide * 0.5) + v * (y - deep * 0.5)
                        };
                        let fits = [
                            corner(-RING, -RING),
                            corner(wide + RING, -RING),
                            corner(wide + RING, deep + RING),
                            corner(-RING, deep + RING),
                        ]
                        .iter()
                        .all(|&p| geom::contains(inner, p));
                        fits.then_some((corner, cols, rows, wide, deep))
                    })
            });
        if let Some((corner, cols, rows, wide, deep)) = placed {
            let quad = |x0: f64, y0: f64, x1: f64, y1: f64| {
                ccw(vec![
                    corner(x0, y0),
                    corner(x1, y0),
                    corner(x1, y1),
                    corner(x0, y1),
                ])
            };
            pieces.push(Piece {
                kind: Kind::Plot,
                block: b,
                outline: quad(0.0, 0.0, wide, deep),
                height: 0.0,
            });
            // The ring: its four sides round the plot.
            for outline in [
                quad(-RING, -RING, wide + RING, 0.0),
                quad(-RING, deep, wide + RING, deep + RING),
                quad(-RING, 0.0, 0.0, deep),
                quad(wide, 0.0, wide + RING, deep),
            ] {
                pieces.push(Piece {
                    kind: Kind::Path,
                    block: b,
                    outline,
                    height: 0.0,
                });
            }
            for r in 0..rows {
                for c in 0..cols {
                    let x = BED_PATH + c as f64 * (bw + BED_PATH);
                    let y = BED_PATH + r as f64 * (bl + BED_PATH);
                    pieces.push(Piece {
                        kind: Kind::Bed,
                        block: b,
                        outline: quad(x, y, x + bw, y + bl),
                        height: bh,
                    });
                }
            }
            let h = RING * 0.5;
            let rim = [
                corner(-h, -h),
                corner(wide + h, -h),
                corner(wide + h, deep + h),
                corner(-h, deep + h),
            ];
            let rim = (0..4).map(|k| (rim[k], rim[(k + 1) % 4])).collect();
            // Seats on the green past the ring, facing the beds: the long sides' middles, then
            // the ends, then the long sides' quarters.
            let (_, deep_bench, off) = BENCH;
            let out_of = RING + off + deep_bench * 0.5;
            let seats = [
                (corner(wide * 0.5, -out_of), v),
                (corner(wide * 0.5, deep + out_of), -v),
                (corner(-out_of, deep * 0.5), u),
                (corner(wide + out_of, deep * 0.5), -u),
                (corner(wide * 0.2, -out_of), v),
                (corner(wide * 0.8, deep + out_of), -v),
                (corner(wide * 0.8, -out_of), v),
                (corner(wide * 0.2, deep + out_of), -v),
            ];
            return (pieces, rim, seats.to_vec());
        }
    }
    if !geom::contains(court, hub) {
        return (pieces, Vec::new(), Vec::new());
    }
    // A round paved place, a twelve-sided one (its corners from √3, exact on every machine,
    // D-016); the paths end a metre into it.
    let c = 3f64.sqrt() * 0.5;
    let corners = [
        (1.0, 0.0),
        (c, 0.5),
        (0.5, c),
        (0.0, 1.0),
        (-0.5, c),
        (-c, 0.5),
        (-1.0, 0.0),
        (-c, -0.5),
        (-0.5, -c),
        (0.0, -1.0),
        (0.5, -c),
        (c, -0.5),
    ];
    let ring = |r: f64| -> Vec<DVec2> {
        corners
            .iter()
            .map(|&(x, y)| hub + DVec2::new(x, y) * r)
            .collect()
    };
    pieces.push(Piece {
        kind: Kind::Path,
        block: b,
        outline: ccw(ring(PLAZA * 0.5)),
        height: 0.0,
    });
    let rim = ring(PLAZA * 0.5 - 1.0);
    let rim = (0..12).map(|k| (rim[k], rim[(k + 1) % 12])).collect();
    // Seats on the green past the place's sides, facing its middle, a quarter turn apart first.
    let (_, deep_bench, off) = BENCH;
    let seats = [0, 6, 3, 9, 1, 7, 4, 10, 2, 8, 5, 11]
        .into_iter()
        .map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 12]);
            let out = DVec2::new(a.0 + b.0, a.1 + b.1).normalize();
            let side = PLAZA * 0.5 * DVec2::new(a.0 + b.0, a.1 + b.1).length() * 0.5;
            (hub + out * (side + off + deep_bench * 0.5), -out)
        })
        .collect();
    (pieces, rim, seats)
}

/// The court-side ground-floor flats with a garden or a terrace before them.
pub fn flats(plan: &CityPlan, masses: &[Mass], structures: &[Structure]) -> Vec<Flat> {
    let paths = court_paths(plan, masses, structures);
    flats_beside(plan, masses, structures, &paths)
}

/// The flats along the green courts' walls, `FLAT` metres each centred on each open stretch of
/// the ground storey, where the garden's ground lies on the court or the building's own lot,
/// clear of every building, of the `paths` and of the doors and passages.
fn flats_beside(
    plan: &CityPlan,
    masses: &[Mass],
    structures: &[Structure],
    paths: &[Piece],
) -> Vec<Flat> {
    // The buildings' outlines every 8 m: a point inside one lies within 30 m of its samples.
    let mut near = PointGrid::new(plan.size, 30.0);
    for (k, m) in masses.iter().enumerate() {
        let mut ring = m.footprint.clone();
        ring.push(m.footprint[0]);
        for p in geom::resample(&ring, 8.0) {
            near.insert(p, k as u32);
        }
    }
    let built = |p: DVec2| {
        let mut inside = false;
        near.for_each_within(p, 30.0, |_, k| {
            inside = inside || geom::contains(&masses[k as usize].footprint, p);
        });
        inside
    };
    let mut by_block: HashMap<u32, Vec<&Piece>> = HashMap::new();
    for p in paths {
        by_block.entry(p.block).or_default().push(p);
    }
    let mut out = Vec::new();
    for (i, (m, s)) in masses.iter().zip(structures).enumerate() {
        let lot = &plan.lots[m.lot as usize];
        let Some(court) = green_court(plan, lot.block) else {
            continue;
        };
        let paths = by_block.get(&lot.block).map_or(&[][..], Vec::as_slice);
        for (w, wall) in s.walls.iter().enumerate() {
            if wall.role != WallRole::Court {
                continue;
            }
            let (t, n) = frame(m, wall);
            let mut doors: Vec<(f64, f64)> = s
                .openings
                .iter()
                .filter(|o| {
                    o.wall == w
                        && o.storey == 0
                        && matches!(o.kind, OpeningKind::Door | OpeningKind::Passage)
                })
                .map(Opening::span)
                .collect();
            doors.sort_by(|a, b| a.0.total_cmp(&b.0));
            // The ground storey's open stretches, cut round its doors and passages.
            let mut runs = Vec::new();
            for &(lo, hi) in wall.open.first().map_or(&[][..], Vec::as_slice) {
                let mut from = lo;
                for &(a, b) in doors.iter().filter(|&&(a, b)| b > lo && a < hi) {
                    runs.push((from, a - DOOR_CLEAR));
                    from = from.max(b + DOOR_CLEAR);
                }
                runs.push((from, hi));
            }
            for (k, &(lo, hi)) in runs.iter().enumerate() {
                // A pier's room off the run's ends, where a neighbour's wall may meet it.
                let count = ((hi - lo - 2.0 * PIER) / FLAT).floor().max(0.0) as usize;
                for f in 0..count {
                    let from = lo + (hi - lo - count as f64 * FLAT) * 0.5 + f as f64 * FLAT;
                    let to = from + FLAT;
                    let ground = rect(wall, t, n, (from, to), (0.0, GARDEN));
                    let at = |s: f64, d: f64| wall.a + t * s + n * d;
                    let open = [at(from, GARDEN), at(to, GARDEN)]
                        .iter()
                        .all(|&p| geom::contains(court, p) || geom::contains(&lot.polygon, p));
                    let free = [
                        from + 0.05,
                        from + 0.6,
                        (from + to) * 0.5,
                        to - 0.6,
                        to - 0.05,
                    ]
                    .iter()
                    .flat_map(|&s| [at(s, 0.05), at(s, GARDEN * 0.5), at(s, GARDEN)])
                    .all(|p| !built(p));
                    let clear = paths.iter().all(|p| !geom::overlaps(&ground, &p.outline));
                    if !(open && free && clear) {
                        continue;
                    }
                    let key = (u64::from(m.lot) << 40)
                        ^ ((i as u64) << 20)
                        ^ ((w as u64) << 12)
                        ^ ((k as u64) << 6)
                        ^ f as u64;
                    out.push(Flat {
                        mass: i as u32,
                        wall: w,
                        from,
                        to,
                        garden: unit(key) < GARDENED,
                    });
                }
            }
        }
    }
    out
}

/// Each flat's ground-floor window nearest its middle made a glazed bay onto its terrace: to
/// the floor (its threshold flush, 2 cm at most for a wheelchair), [`BAY`] wide where its
/// neighbours leave a pier.
pub fn glaze(structures: &mut [Structure], flats: &[Flat]) {
    for f in flats {
        let s = &mut structures[f.mass as usize];
        let middle = (f.from + f.to) * 0.5;
        let nearest = s
            .openings
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                o.wall == f.wall
                    && o.storey == 0
                    && o.kind == OpeningKind::Window
                    && o.along > f.from
                    && o.along < f.to
            })
            .min_by(|x, y| {
                (x.1.along - middle)
                    .abs()
                    .total_cmp(&(y.1.along - middle).abs())
            })
            .map(|(k, _)| k);
        let Some(k) = nearest else {
            continue;
        };
        let o = s.openings[k];
        let mut half = (BAY * 0.5)
            .min(o.along - f.from - PIER)
            .min(f.to - PIER - o.along);
        for (j, q) in s.openings.iter().enumerate() {
            if j != k && q.wall == f.wall && q.storey == 0 {
                half = half.min((q.along - o.along).abs() - q.width * 0.5 - PIER);
            }
        }
        // Widths by the 10 cm, so the kit shares their meshes.
        let width = ((half * 2.0 * 10.0).floor() / 10.0).max(o.width);
        s.openings[k] = Opening {
            width,
            sill: 0.0,
            ..o
        };
    }
}

/// Everything the homes keep on the ground round `masses` and their `structures` on `plan`.
pub fn gardens(plan: &CityPlan, masses: &[Mass], structures: &[Structure]) -> Vec<Piece> {
    let mut out = court_paths(plan, masses, structures);
    let flats = flats_beside(plan, masses, structures, &out);
    for f in &flats {
        let m = &masses[f.mass as usize];
        let block = plan.lots[m.lot as usize].block;
        let wall = &structures[f.mass as usize].walls[f.wall];
        let (t, n) = frame(m, wall);
        let mut push = |kind: Kind, along: (f64, f64), out_of: (f64, f64), height: f64| {
            out.push(Piece {
                kind,
                block,
                outline: rect(wall, t, n, along, out_of),
                height,
            });
        };
        let (from, to) = (f.from, f.to);
        let (hw, high, low) = HEDGE;
        if f.garden {
            // The terrace before the bay, the lawn beyond, a paved way across it to the gate.
            let gate = (from + to) * 0.5;
            let way = (gate - GATE * 0.5, gate + GATE * 0.5);
            push(Kind::Terrace, (from + hw, to - hw), (0.0, TERRACE), 0.0);
            push(Kind::Terrace, way, (TERRACE, GARDEN), 0.0);
            push(Kind::Lawn, (from + hw, way.0), (TERRACE, GARDEN - hw), 0.0);
            push(Kind::Lawn, (way.1, to - hw), (TERRACE, GARDEN - hw), 0.0);
            // Hedges between the neighbours, and a low one along the court with the gate in
            // its middle.
            push(Kind::Hedge, (from, from + hw), (0.0, GARDEN), high);
            push(Kind::Hedge, (to - hw, to), (0.0, GARDEN), high);
            push(Kind::Hedge, (from + hw, way.0), (GARDEN - hw, GARDEN), low);
            push(Kind::Hedge, (way.1, to - hw), (GARDEN - hw, GARDEN), low);
        } else {
            // A terrace, screened from the neighbours' by a short hedge at either end.
            push(
                Kind::Terrace,
                (from + 1.0, to - 1.0),
                (0.0, PLAIN_TERRACE),
                0.0,
            );
            push(
                Kind::Hedge,
                (from + 0.5, from + 1.0),
                (0.0, PLAIN_TERRACE),
                1.2,
            );
            push(Kind::Hedge, (to - 1.0, to - 0.5), (0.0, PLAIN_TERRACE), 1.2);
        }
    }
    planters(plan, masses, structures, &mut out);
    gap_hedges(plan, masses, &mut out);
    out
}

/// A low hedge along each gap between the buildings on a green court's streets (a street lot
/// left open), just behind its paved forecourt, broken a metre either side of the court's
/// paths: the gaps showed a plain lawn (the owner: a gap would "match the sidewalk or have a
/// decoration"; Paris's residents asked for low planting rather than fences,
/// `research/ground-floors.md` §3).
fn gap_hedges(plan: &CityPlan, masses: &[Mass], out: &mut Vec<Piece>) {
    let built: HashSet<u32> = masses.iter().map(|m| m.lot).collect();
    let mut streets = PointGrid::new(plan.size, 20.0);
    for (k, e) in plan.roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 2.0) {
            streets.insert(p, k as u32);
        }
    }
    // The buildings' outlines every 8 m: a point inside one lies within 30 m of its samples.
    let mut near = PointGrid::new(plan.size, 30.0);
    for (k, m) in masses.iter().enumerate() {
        let mut ring = m.footprint.clone();
        ring.push(m.footprint[0]);
        for p in geom::resample(&ring, 8.0) {
            near.insert(p, k as u32);
        }
    }
    let in_building = |p: DVec2| {
        let mut inside = false;
        near.for_each_within(p, 30.0, |_, k| {
            inside = inside || geom::contains(&masses[k as usize].footprint, p);
        });
        inside
    };
    let mut paths: HashMap<u32, Vec<Vec<DVec2>>> = HashMap::new();
    for p in out.iter().filter(|p| p.kind == Kind::Path) {
        paths.entry(p.block).or_default().push(p.outline.clone());
    }
    let (hw, _, low) = HEDGE;
    let mut hedges = Vec::new();
    for (i, lot) in plan.lots.iter().enumerate() {
        if green_court(plan, lot.block).is_none() || !lot.frontage || built.contains(&(i as u32)) {
            continue;
        }
        let mut racked = false;
        let mine = paths.get(&lot.block).map_or(&[][..], Vec::as_slice);
        let n = lot.polygon.len();
        for k in 0..n {
            let (a, b) = (lot.polygon[k], lot.polygon[(k + 1) % n]);
            let length = a.distance(b);
            let mid = (a + b) * 0.5;
            // Its sides on a street's building line: half the street's width from its middle.
            let Some((_, e)) = streets.nearest(mid, 40.0) else {
                continue;
            };
            let street = &plan.roads.edges[e as usize];
            let off = street
                .points
                .windows(2)
                .map(|w| geom::distance_to_segment(mid, w[0], w[1]))
                .fold(f64::MAX, f64::min);
            if length < 4.0 || (off - street.profile.width() * 0.5).abs() > 1.5 {
                continue;
            }
            let t = (b - a) / length;
            let mut inward = DVec2::new(-t.y, t.x);
            if !geom::contains(&lot.polygon, mid + inward * 0.5) {
                inward = -inward;
            }
            let d = FORECOURT + hw * 0.5 + 0.1;
            // Every half metre along the line, where it lies in the lot, half a metre clear of
            // the buildings and a metre clear of the paths.
            let clear = |p: DVec2| {
                geom::contains(&lot.polygon, p)
                    && [p, p + t * 0.5, p - t * 0.5, p + inward * 0.5]
                        .iter()
                        .all(|&q| !in_building(q))
                    && mine
                        .iter()
                        .all(|o| [p, p + t, p - t].iter().all(|&q| !geom::contains(o, q)))
            };
            let steps = (length / 0.5).floor() as usize;
            let mut from: Option<f64> = None;
            for j in 0..=steps {
                let s = j as f64 * 0.5;
                let good = j < steps && clear(a + t * s + inward * d);
                match (from, good) {
                    (None, true) => from = Some(s),
                    (Some(s0), false) => {
                        if s - s0 >= 2.0 {
                            let side = |s: f64, off: f64| a + t * s + inward * (d + off);
                            // A gap's first stretch of hedge long enough gets a bike rack on
                            // the forecourt before it, for the residents and their visitors.
                            let (long, deep, _) = BIKE_RACK;
                            if !racked && s - 0.5 - s0 >= long + 1.0 {
                                racked = true;
                                let mid = (s0 + s - 0.5) * 0.5;
                                let front = -(hw * 0.5 + 0.1 + 0.05 + deep);
                                hedges.push(Piece {
                                    kind: Kind::BikeRack,
                                    block: lot.block,
                                    outline: ccw(vec![
                                        side(mid - long * 0.5, front),
                                        side(mid + long * 0.5, front),
                                        side(mid + long * 0.5, front + deep),
                                        side(mid - long * 0.5, front + deep),
                                    ]),
                                    height: BIKE_RACK.2,
                                });
                            }
                            hedges.push(Piece {
                                kind: Kind::Hedge,
                                block: lot.block,
                                outline: ccw(vec![
                                    side(s0, -hw * 0.5),
                                    side(s - 0.5, -hw * 0.5),
                                    side(s - 0.5, hw * 0.5),
                                    side(s0, hw * 0.5),
                                ]),
                                height: low,
                            });
                        }
                        from = None;
                    }
                    _ => {}
                }
            }
        }
    }
    out.extend(hedges);
}

/// Planted beds before the homes on the streets 8 m wide or more, against the facade on the
/// ground storey's open stretches, broken before each door and passage; none before a shop.
fn planters(plan: &CityPlan, masses: &[Mass], structures: &[Structure], out: &mut Vec<Piece>) {
    let mut streets = PointGrid::new(plan.size, 20.0);
    for (k, e) in plan.roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 2.0) {
            streets.insert(p, k as u32);
        }
    }
    let (deep, height) = PLANTER;
    // The courts' paths, by block: a door's path out of a building's end ran over the planter
    // round its corner.
    let mut paths: HashMap<u32, Vec<Vec<DVec2>>> = HashMap::new();
    for p in out.iter().filter(|p| p.kind == Kind::Path) {
        paths.entry(p.block).or_default().push(p.outline.clone());
    }
    let mut planted = Vec::new();
    for (m, s) in masses.iter().zip(structures) {
        if m.active {
            continue;
        }
        let block = plan.lots[m.lot as usize].block;
        for (w, wall) in s.walls.iter().enumerate() {
            if wall.role != WallRole::Street {
                continue;
            }
            let (t, n) = frame(m, wall);
            let mid = (wall.a + wall.b) * 0.5;
            let wide = streets.nearest(mid + n * 0.5, 40.0).is_some_and(|(_, e)| {
                plan.roads.edges[e as usize].profile.width() >= PLANTER_STREET
            });
            if !wide {
                continue;
            }
            let mut gaps: Vec<(f64, f64)> = s
                .openings
                .iter()
                .filter(|o| {
                    o.wall == w
                        && o.storey == 0
                        && matches!(o.kind, OpeningKind::Door | OpeningKind::Passage)
                })
                .map(|o| (o.span().0 - DOOR_CLEAR, o.span().1 + DOOR_CLEAR))
                .collect();
            gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(lo, hi) in wall.open.first().map_or(&[][..], Vec::as_slice) {
                let (start, end) = (lo + 0.6, hi - 0.6);
                let mut from = start;
                for (a, b) in gaps.iter().copied().chain([(end, f64::INFINITY)]) {
                    let a = a.min(end);
                    let outline = rect(wall, t, n, (from, a), (0.05, deep));
                    let clear = paths
                        .get(&block)
                        .is_none_or(|ps| ps.iter().all(|p| !geom::overlaps(&outline, p)));
                    if a - from >= 2.0 && clear {
                        planted.push(Piece {
                            kind: Kind::Planter,
                            block,
                            outline,
                            height,
                        });
                    }
                    from = from.max(b);
                    if from >= end {
                        break;
                    }
                }
            }
        }
    }
    out.extend(planted);
}

/// A number in 0..1 from `key`.
fn unit(key: u64) -> f64 {
    let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}
