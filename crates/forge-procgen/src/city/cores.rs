//! The buildings' stair cores (D-060, step 3; `docs/research/building-structure.md`, §1): behind
//! each street door a hall, a dog-leg stair on one side of it and, from [`LIFT_FROM`] storeys, a
//! lift on the other. The hall is the landing on every storey, where the flats' doors open.
//!
//! Sized by the French rules:
//! - flights 1.00 m between handrails, with risers of 17 cm at most on 28 cm goings;
//! - landings as wide as a flight;
//! - a type 2 lift car (1.10 × 1.40 m in a 1.60 × 1.90 m shaft);
//! - a hall of Ø 1.50 m, where a wheelchair turns.
//!
//! A core climbs on to the roof where the roof is a garden or a terrace, with a head house
//! over it (the owner, 2026-10-11: a garden on the roof means a way up).

use glam::DVec2;

use super::geom;
use super::mass::{Mass, Roof};
use super::structure::{DOOR, LIFT_FROM, MARGIN, Opening, OpeningKind, Storey, Wall, WallRole};

/// A flight's width with its handrails (1.00 m between them), metres.
pub const FLIGHT: f64 = 1.1;
/// The well between a dog-leg's two flights, metres.
pub const WELL: f64 = 0.1;
/// A core's walls, metres.
pub const CORE_WALL: f64 = 0.2;
/// The hall's width, the landing on every storey: Ø 1.50 m to turn a wheelchair, metres.
pub const HALL: f64 = 1.5;
/// A house's corridor beside its straight flight, metres: a wheelchair's passage, the landing at
/// the flight's head as wide.
pub const HOUSE_HALL: f64 = 1.2;
/// A lift's shaft inside, along the wall and inwards, metres: a type 2 car (EN 81-70, 1.10 ×
/// 1.40 m) with its clearances.
pub const SHAFT: (f64, f64) = (1.6, 1.9);
/// The highest riser in housing, metres.
pub const RISER: f64 = 0.17;
/// A step's going, metres.
pub const GOING: f64 = 0.28;
/// A landing's length, as wide as a flight and a little more, metres.
pub const LANDING: f64 = 1.2;
/// A core every this many metres of street front, metres: each serves the flats either side of
/// it, two to four a storey.
pub const SPACING: f64 = 20.0;
/// A floor smaller than this takes the compact core (the lift behind the hall) where it fits,
/// m².
pub const COMPACT_UNDER: f64 = 150.0;
/// The farthest a floor lies from its landing, as the crow flies, metres.
pub const REACH: f64 = 25.0;
/// A wing whose floor is smaller than this, beside another mass of its building, is reached
/// from that one's core, m².
pub const SERVED: f64 = 100.0;

/// A stair core.
#[derive(Clone, Debug, PartialEq)]
pub struct Core {
    /// The wall its door is in, an index into the structure's walls.
    pub wall: usize,
    /// Its door, an index into the structure's openings: a [`OpeningKind::Door`] on the ground
    /// storey, in the middle of the hall.
    pub door: usize,
    /// Its door's stretch along its wall, metres from the wall's `a`.
    pub door_at: (f64, f64),
    /// Its stretch along its wall, metres from the wall's `a`.
    pub span: (f64, f64),
    /// How deep it reaches in from the wall's inner face, its back wall included, metres.
    pub deep: f64,
    /// Its outline on the plan, counter-clockwise, its first edge on the wall's inner face.
    pub outline: [DVec2; 4],
    /// The stair's box inside its walls.
    pub stair: [DVec2; 4],
    /// The hall: the landing on every storey, open at the back to the flats' doors.
    pub hall: [DVec2; 4],
    /// The lift's shaft inside its walls, from [`LIFT_FROM`] storeys.
    pub lift: Option<[DVec2; 4]>,
    /// The risers each storey's stair climbs, ground first, the roof's flight last where it
    /// goes up.
    pub risers: Vec<u32>,
    /// Whether it climbs on to the roof: a garden or a terrace there, a head house over it.
    pub roof: bool,
    /// A house's core: one straight flight along the party wall beside its corridor, the hall
    /// the whole depth, no lift, serving one home a storey. Taken where no dog-leg fits (a 5 m
    /// house holds no hall of 1.5 m beside two flights).
    pub house: bool,
}

impl Core {
    /// Its floor area on every storey, m².
    pub fn area(&self) -> f64 {
        (self.span.1 - self.span.0) * self.deep
    }
}

/// The risers a stair climbs over a storey `height` high, none over 17 cm.
pub fn risers(height: f64) -> u32 {
    (height / RISER - 1e-9).ceil().max(1.0) as u32
}

/// The length a dog-leg needs inside its walls for a storey `height` high, metres: two flights
/// of half the risers each, their goings, a landing at each end.
pub fn stair_length(height: f64) -> f64 {
    let flight = risers(height).div_ceil(2);
    f64::from(flight.saturating_sub(1)) * GOING + 2.0 * LANDING
}

/// The length a house's straight flight needs inside its walls for a storey `height` high,
/// metres: all its goings and the landing at its head (the hall below is its foot).
pub fn flight_length(height: f64) -> f64 {
    f64::from(risers(height).saturating_sub(1)) * GOING + LANDING
}

/// Whether a mass needs cores of its own: two storeys or more, and not a small wing reached
/// from its building's other mass ([`SERVED`]).
pub fn needs_cores(m: &Mass, walls: &[Wall], plate: f64) -> bool {
    m.storeys.len() >= 2 && !(plate < SERVED && walls.iter().any(|w| w.role == WallRole::Inner))
}

/// The cores of a mass: one every [`SPACING`] of its street front, and enough that its floor lies
/// within [`REACH`] of one (one at least). Each stands behind a door on an open ground stretch
/// of its street walls (of its court walls where the street's have no room), nearest its share of
/// the front: on a ground opening's bay first, else anywhere along the stretch; wide (the lift
/// beside the hall) or, on a narrow lot, compact (the lift behind it); inside the walls and clear
/// of the others. The ground openings its door covers go, and the shopfronts its walls stand
/// behind become windows. `inside` is the floor inside the walls; `avoid` the stretches (wall,
/// from, to) a passage into the court keeps clear.
pub fn place(
    m: &Mass,
    storeys: &[Storey],
    walls: &[Wall],
    openings: &mut Vec<Opening>,
    inside: &[DVec2],
    window: f64,
    avoid: &[(usize, f64, f64)],
) -> Vec<Core> {
    let role = if walls.iter().any(|w| w.role == WallRole::Street) {
        WallRole::Street
    } else {
        WallRole::Court
    };
    // Each wall of that role's start along the run of them all.
    let mut start = vec![0.0; walls.len()];
    let mut run = 0.0;
    for (w, wall) in walls.iter().enumerate() {
        if wall.role == role {
            start[w] = run;
            run += wall.length();
        }
    }
    if run <= 0.0 {
        return Vec::new();
    }
    let (_, extent, _) = geom::oriented_extent(&m.footprint);
    let n = ((run / SPACING).round() as usize)
        .max((extent.x / (1.2 * REACH)).ceil() as usize)
        .max(1);
    let deep = storeys
        .iter()
        .map(|s| stair_length(s.height))
        .fold(0.0, f64::max);
    let deep_house = storeys
        .iter()
        .map(|s| flight_length(s.height))
        .fold(0.0, f64::max);
    let lift = storeys.len() >= LIFT_FROM;
    let roof = matches!(m.roof, Roof::Green | Roof::Terrace);
    let mut climb: Vec<u32> = storeys.iter().map(|s| risers(s.height)).collect();
    if roof {
        // The roof's flight: as many as the top storey's, to the head house's floor.
        climb.push(*climb.last().unwrap_or(&1));
    }
    // The point `s` metres along the run of walls of that role.
    let along_run = |s: f64| {
        walls
            .iter()
            .enumerate()
            .filter(|(_, w)| w.role == role)
            .find(|(w, wall)| s <= start[*w] + wall.length() + 1e-9)
            .map_or(walls[0].a, |(w, wall)| {
                let t = (wall.b - wall.a).normalize_or_zero();
                wall.a + t * (s - start[w])
            })
    };
    // Where a door may go: each ground opening's bay, then every half metre of the open ground
    // stretches, as (wall, along, cost over the distance).
    let mut places: Vec<(usize, f64, f64)> = openings
        .iter()
        .filter(|o| o.storey == 0 && matches!(o.kind, OpeningKind::Window | OpeningKind::Shopfront))
        .map(|o| (o.wall, o.along, 0.0))
        .collect();
    for (w, wall) in walls.iter().enumerate() {
        for &(lo, hi) in wall.open.first().into_iter().flatten() {
            let (lo, hi) = (lo + MARGIN + DOOR * 0.5, hi - MARGIN - DOOR * 0.5);
            let steps = ((hi - lo) / 0.5).floor().max(-1.0) as i64;
            for k in 0..=steps {
                places.push((w, lo + k as f64 * 0.5, 3.0));
            }
        }
    }
    places.retain(|&(w, _, _)| matches!(walls[w].role, WallRole::Street | WallRole::Court));
    let mut cores: Vec<Core> = Vec::new();
    for i in 0..n {
        // A house's core stands where its rooms' plan keeps the hall, at the start of its street
        // front by its first corner (`rooms::plan`, the hall from x = 0): the nearest place to
        // it wins, whatever opening it takes.
        let house = m.home.is_some();
        let target = if house {
            party_end(walls, role).unwrap_or_else(|| along_run(0.0))
        } else {
            along_run(run * (i as f64 + 0.5) / n as f64)
        };
        let mut ranked: Vec<(usize, f64, f64)> = places
            .iter()
            .map(|&(w, along, cost)| {
                let wall = &walls[w];
                let p = wall.a + (wall.b - wall.a).normalize_or_zero() * along;
                let other = if wall.role == role { 0.0 } else { 6.0 };
                let cost = if house { 0.0 } else { cost };
                (w, along, p.distance(target) + cost + other)
            })
            .collect();
        ranked.sort_by(|x, y| x.2.total_cmp(&y.2));
        // On a small floor the compact core first: the wide one took a third of a narrow
        // house's floor.
        let small = geom::signed_area(inside).abs() < COMPACT_UNDER;
        let shapes: &[(bool, bool)] = match (lift, small) {
            (true, false) => &[(false, false), (true, false), (false, true), (true, true)],
            (true, true) => &[(false, true), (true, true), (false, false), (true, false)],
            (false, _) => &[(false, false), (true, false)],
        };
        let fits = |c: &Core| {
            probes(&c.outline)
                .iter()
                .all(|&p| geom::contains(inside, p))
                && cores.iter().all(|d| apart(c, d))
                && avoid
                    .iter()
                    .all(|&(w, lo, hi)| c.wall != w || c.span.1 <= lo || c.span.0 >= hi)
        };
        // A house from its rooms takes its straight flight in the hall its plan keeps; others a
        // dog-leg where one fits.
        let found = ranked
            .iter()
            .filter(|_| m.rooms.is_none())
            .find_map(|&(w, along, _)| {
                shapes.iter().find_map(|&(flip, compact)| {
                    let c = layout(&walls[w], w, along, deep, lift, flip, compact, false)?;
                    fits(&c).then_some(c)
                })
            })
            .or_else(|| {
                // No dog-leg anywhere: a house's straight flight, where no lift is due.
                (!lift)
                    .then(|| {
                        ranked.iter().find_map(|&(w, along, _)| {
                            // A house from its rooms keeps its flight on the party wall's side,
                            // towards the nearer end of its front, its corridor on the rooms'
                            // side with their doorways.
                            let near_b = along > walls[w].length() * 0.5;
                            let flips: &[bool] = if m.rooms.is_some() {
                                if near_b { &[true] } else { &[false] }
                            } else {
                                &[false, true]
                            };
                            flips.iter().copied().find_map(|flip| {
                                let c = layout(
                                    &walls[w], w, along, deep_house, false, flip, false, true,
                                )?;
                                if !fits(&c) {
                                    return None;
                                }
                                // Slid against the side wall where the door keeps its margin.
                                let snug = snug(&walls[w], inside, along, flip, c.deep)
                                    .filter(|&a| door_fits(&walls[w], a))
                                    .and_then(|a| {
                                        layout(
                                            &walls[w], w, a, deep_house, false, flip, false, true,
                                        )
                                    })
                                    .filter(|c| fits(c));
                                Some(snug.unwrap_or(c))
                            })
                        })
                    })
                    .flatten()
            });
        if let Some(mut c) = found {
            c.risers = climb.clone();
            c.roof = roof;
            cores.push(c);
        }
    }
    for c in &mut cores {
        let along = (c.door_at.0 + c.door_at.1) * 0.5;
        // The ground openings the door covers go; the shopfronts behind the core become windows.
        openings.retain(|o| {
            !(o.wall == c.wall
                && o.storey == 0
                && o.span().1 > along - DOOR * 0.5 - 0.3
                && o.span().0 < along + DOOR * 0.5 + 0.3)
        });
        for o in openings.iter_mut() {
            let (lo, hi) = o.span();
            if o.wall == c.wall
                && o.storey == 0
                && o.kind == OpeningKind::Shopfront
                && hi > c.span.0
                && lo < c.span.1
            {
                *o = Opening {
                    kind: OpeningKind::Window,
                    width: window,
                    sill: 1.0,
                    head: storeys[0].clear() - 0.4,
                    ..*o
                };
            }
        }
        let template = Opening {
            wall: c.wall,
            storey: 0,
            along,
            width: DOOR,
            sill: 0.0,
            head: 2.2,
            kind: OpeningKind::Door,
        };
        openings.push(door(m, storeys, walls, &template));
    }
    // The doors' indices, once every core's has gone in.
    reindex(&mut cores, openings);
    cores
}

/// Each core's [`Core::door`] found again in `openings`, after a pass that rebuilt or thinned
/// them (`windows::by_room`, the hall's windows moved off its wall): a stale index named a
/// window as the house's front door. `usize::MAX` where its door is gone.
pub fn reindex(cores: &mut [Core], openings: &[Opening]) {
    for c in cores {
        let along = (c.door_at.0 + c.door_at.1) * 0.5;
        c.door = openings
            .iter()
            .position(|o| {
                o.kind == OpeningKind::Door
                    && o.wall == c.wall
                    && o.storey == 0
                    && (o.along - along).abs() < 1e-9
            })
            .unwrap_or(usize::MAX);
    }
}

/// A ground opening `o` made a door: the step-free width, its sill on the ground outside.
pub fn door(m: &Mass, storeys: &[Storey], walls: &[Wall], o: &Opening) -> Opening {
    let (clear, level) = (
        storeys[0].clear(),
        super::structure::outside(m, walls[o.wall].role),
    );
    Opening {
        kind: OpeningKind::Door,
        width: DOOR,
        sill: level,
        head: (clear - 0.4).min(level + 2.6),
        ..*o
    }
}

/// A core's plan behind a door at `along` on `wall`, `deep` inside its walls: from the wall's
/// `a` the stair, the hall round the door, the lift (or, `flip`, the other way round). Wide, the
/// lift's shaft beside the hall; `compact`, behind it at the hall's end, the landing in front of
/// it (4.4 m along the wall instead of 6.0). `None` where it runs off the wall.
#[allow(clippy::too_many_arguments)]
fn layout(
    wall: &Wall,
    w: usize,
    along: f64,
    deep: f64,
    lift: bool,
    flip: bool,
    compact: bool,
    house: bool,
) -> Option<Core> {
    let t = (wall.b - wall.a).normalize_or_zero();
    let inward = DVec2::new(-t.y, t.x);
    let face = wall.thickness;
    let p = |s: f64, d: f64| wall.a + t * s + inward * (face + d);
    let rect = |lo: f64, hi: f64, d0: f64, d1: f64| [p(lo, d0), p(hi, d0), p(hi, d1), p(lo, d1)];
    let sign = if flip { -1.0 } else { 1.0 };
    // Offsets from the door's middle, towards `a` first (or `b` when flipped).
    let at = |x: (f64, f64)| {
        let (a, b) = (along + sign * x.0, along + sign * x.1);
        (a.min(b), a.max(b))
    };
    if house {
        // The corridor behind the door, the straight flight beside it along the party wall: the
        // wall its side, no core wall between ([`snug`] slides it there).
        let hall = (-HOUSE_HALL * 0.5, HOUSE_HALL * 0.5);
        let stair = (hall.0 - FLIGHT, hall.0);
        let (lo, hi) = at((stair.0, hall.1 + CORE_WALL));
        if lo < 0.0 || hi > wall.length() {
            return None;
        }
        let (s, h) = (at(stair), at(hall));
        return Some(Core {
            wall: w,
            door: usize::MAX,
            door_at: (along - DOOR * 0.5, along + DOOR * 0.5),
            span: (lo, hi),
            deep: deep + CORE_WALL,
            outline: rect(lo, hi, 0.0, deep + CORE_WALL),
            stair: rect(s.0, s.1, 0.0, deep),
            hall: rect(h.0, h.1, 0.0, deep + CORE_WALL),
            lift: None,
            risers: Vec::new(),
            roof: false,
            house: true,
        });
    }
    let width = if lift && compact { SHAFT.0 } else { HALL };
    let hall = (-width * 0.5, width * 0.5);
    let stair = (hall.0 - 2.0 * FLIGHT - WELL, hall.0);
    let shaft = (lift && !compact).then_some((hall.1 + CORE_WALL, hall.1 + CORE_WALL + SHAFT.0));
    let end = shaft.map_or(hall.1, |s| s.1) + CORE_WALL;
    let begin = stair.0 - CORE_WALL;
    let (lo, hi) = at((begin, end));
    if lo < 0.0 || hi > wall.length() {
        return None;
    }
    let (s, h) = (at(stair), at(hall));
    let (hall_rect, lift_rect) = if lift && compact {
        (
            rect(h.0, h.1, 0.0, deep - SHAFT.1 - CORE_WALL),
            Some(rect(h.0, h.1, deep - SHAFT.1, deep)),
        )
    } else {
        (
            rect(h.0, h.1, 0.0, deep + CORE_WALL),
            shaft.map(|x| {
                let x = at(x);
                rect(x.0, x.1, 0.0, SHAFT.1)
            }),
        )
    };
    let door_at = (along - DOOR * 0.5, along + DOOR * 0.5);
    Some(Core {
        wall: w,
        door: usize::MAX,
        door_at,
        span: (lo, hi),
        deep: deep + CORE_WALL,
        outline: rect(lo, hi, 0.0, deep + CORE_WALL),
        stair: rect(s.0, s.1, 0.0, deep),
        hall: hall_rect,
        lift: lift_rect,
        risers: Vec::new(),
        roof: false,
        house: false,
    })
}

/// Where a house's door goes on `wall` so its straight flight (towards `a`, or `b` when `flip`)
/// stands against the side wall it is nearest, `deep` into the floor `inside` (the owner,
/// 2026-10-16: "stairs should most likely stuck to the wall, no space between wall and stair"):
/// the floor's edge on the flight's side at the tightest of its depths, the flight's outer side
/// there. `None` where no edge lies on that side of the door at `along`.
pub fn snug(wall: &Wall, inside: &[DVec2], along: f64, flip: bool, deep: f64) -> Option<f64> {
    let t = (wall.b - wall.a).normalize_or_zero();
    let inward = DVec2::new(-t.y, t.x);
    let sign = if flip { -1.0 } else { 1.0 };
    let n = inside.len();
    // Along the wall, signed so the flight's side is the lower: the floor's edge there at a depth
    // `d` in from the wall's inner face.
    let edge = |d: f64| {
        let v = wall.thickness + d;
        (0..n)
            .filter_map(|i| {
                let (p, q) = (inside[i], inside[(i + 1) % n]);
                let (vp, vq) = ((p - wall.a).dot(inward), (q - wall.a).dot(inward));
                ((vp - v) * (vq - v) <= 0.0 && (vp - vq).abs() > 1e-9)
                    .then(|| ((p + (q - p) * ((v - vp) / (vq - vp))) - wall.a).dot(t) * sign)
            })
            .filter(|&x| x < along * sign)
            .reduce(f64::max)
    };
    let mut tightest = f64::MIN;
    for k in 0..=8 {
        tightest = tightest.max(edge(0.02 + (deep - 0.04) * f64::from(k) / 8.0)?);
    }
    Some((tightest + FLIGHT + HOUSE_HALL * 0.5) * sign)
}

/// The end of a house's front (its first footprint edge's wall, of `role`) where its hall goes:
/// the one beside a party wall, so the flight runs along the neighbour and the rooms take the
/// walls with windows; the start where both or neither are. A corner house's hall ran along its
/// side street, a blank wall the whole depth (the owner, 2026-10-16: "an architect would arrange
/// the inside of structure so that windows are on the street side").
fn party_end(walls: &[Wall], role: WallRole) -> Option<DVec2> {
    let front = walls.iter().find(|w| w.edge == 0 && w.role == role)?;
    let at = |p: DVec2, end: fn(&Wall) -> DVec2| {
        walls
            .iter()
            .find(|w| end(w).distance(p) < 0.01)
            .map(|w| w.role)
    };
    let before = at(front.a, |w| w.b);
    let after = at(front.b, |w| w.a);
    Some(
        if before != Some(WallRole::Party) && after == Some(WallRole::Party) {
            front.b
        } else {
            front.a
        },
    )
}

/// Whether a door at `along` on `wall` keeps [`MARGIN`] inside one of its open ground stretches.
fn door_fits(wall: &Wall, along: f64) -> bool {
    wall.open.first().is_some_and(|stretches| {
        stretches.iter().any(|&(lo, hi)| {
            lo + MARGIN + DOOR * 0.5 <= along + 1e-9 && along <= hi - MARGIN - DOOR * 0.5 + 1e-9
        })
    })
}

/// Points just inside a core's outline to test it against the floor: its corners and its sides'
/// middles, 5 cm in.
fn probes(outline: &[DVec2; 4]) -> Vec<DVec2> {
    let c = (outline[0] + outline[1] + outline[2] + outline[3]) * 0.25;
    let mut out = Vec::with_capacity(8);
    for k in 0..4 {
        let (a, b) = (outline[k], outline[(k + 1) % 4]);
        for p in [a, (a + b) * 0.5] {
            out.push(p + (c - p).normalize_or_zero() * 0.05);
        }
    }
    out
}

/// Whether two cores stand a metre apart or more: neither's probes inside the other grown by a
/// metre (rectangles, so corners and middles tell).
fn apart(a: &Core, b: &Core) -> bool {
    let grow = |r: &[DVec2; 4]| -> Vec<DVec2> {
        let c = (r[0] + r[1] + r[2] + r[3]) * 0.25;
        r.iter()
            .map(|&p| p + (p - c).normalize_or_zero() * 1.4)
            .collect()
    };
    let (ga, gb) = (grow(&a.outline), grow(&b.outline));
    !probes(&a.outline).iter().any(|&p| geom::contains(&gb, p))
        && !probes(&b.outline).iter().any(|&p| geom::contains(&ga, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stairs_climb_each_storey_in_two_flights_within_the_rules() {
        for h in [2.8, 2.9, 3.0, 3.2, 3.4, 3.6, 4.2] {
            let r = risers(h);
            assert!(h / f64::from(r) <= RISER + 1e-9, "{h}: {r} risers");
            // Blondel's rule, 2h + g between 0.60 and 0.66 m: a comfortable stair.
            let blondel = 2.0 * h / f64::from(r) + GOING;
            assert!(
                (0.58..=0.66).contains(&blondel),
                "{h}: 2h + g = {blondel:.3}"
            );
            let l = stair_length(h);
            assert!((4.0..=6.0).contains(&l), "{h}: {l:.2} m");
        }
    }
}
