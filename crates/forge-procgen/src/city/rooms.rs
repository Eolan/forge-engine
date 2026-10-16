//! Houses thought from their rooms (`research/parcels-and-rooms.md` §4; the owner, 2026-10-16:
//! "a house or structure should be thought by thinking about the rooms, the real space inside
//! and the minimum required to make it a liveable space, depending on how rich it is"). A
//! household's programme, the rooms it needs and how big, comes from its size and its means;
//! the plan lays them on a lot of a given width, storey by storey: a hall with the stair along
//! one party wall the whole depth, the rooms in a column beside it from the front to the back,
//! the day rooms on the ground floor and the bedrooms above. The footprint's depth and the
//! storeys come out of the rooms, not the rooms out of a footprint.

use super::cores::{CORE_WALL, FLIGHT, HOUSE_HALL};

/// How well off a household is: it grows the programme in rooms before it grows them in size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Means {
    /// One or two rooms, the kitchen in the living room, at the minimums.
    Poor,
    /// A living room, a kitchen, a bedroom a couple and one a child or two.
    Modest,
    /// A bedroom a person, a dining room, a study, two bathrooms.
    Comfortable,
    /// Reception rooms, suites with their bathrooms, service rooms.
    Rich,
}

/// What a room is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    /// The living room; the kitchen too where there is no other.
    Living,
    /// The kitchen.
    Kitchen,
    /// The dining room.
    Dining,
    /// A bedroom.
    Bedroom,
    /// A bathroom.
    Bathroom,
    /// A study.
    Study,
    /// A reception room.
    Salon,
    /// A service room: a laundry, a pantry, a maid's room.
    Service,
    /// A store room.
    Store,
}

impl RoomKind {
    /// Whether the room belongs to the day: on the ground floor, before the bedrooms.
    fn by_day(self) -> bool {
        matches!(
            self,
            Self::Living | Self::Kitchen | Self::Dining | Self::Salon | Self::Service | Self::Store
        )
    }
}

/// A room wanted: its kind and its floor area, m².
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// What it is for.
    pub kind: RoomKind,
    /// Its floor area wanted, m².
    pub area: f64,
}

/// France's decency minimum for the main room, m² (decree 2002-120), and for a room let on its
/// own: the floor under every room here.
pub const ROOM_MIN: f64 = 9.0;
/// The hall's width from the party wall's middle, metres: half the party wall ([`WALL`]) and the
/// house's core (`cores`), its straight flight with handrails (1.1 m) and the corridor beside it a
/// wheelchair passes (1.2 m) between walls of 0.2 m, from the front to the back on every storey.
pub const HALL: f64 = WALL + 2.0 * CORE_WALL + FLIGHT + HOUSE_HALL;
/// Half a party wall, metres (an old stone house's, 0.5 m, `structure`): the plan's width runs
/// between the party walls' middles, so the hall starts and the rooms end this far in.
pub const WALL: f64 = 0.25;
/// A room's depth at least and at most, metres: a room is wider than a corridor and no deeper
/// than daylight reaches from its window.
pub const DEPTH: (f64, f64) = (2.6, 6.5);

/// The rooms a household of `people` needs by its `means`: the day rooms first, then the
/// bedrooms and bathrooms, each with its area (the research's table by means).
pub fn programme(people: u32, means: Means) -> Vec<Room> {
    let people = people.max(1);
    let couples = u32::from(people >= 2);
    let others = people - 2 * couples;
    let room = |kind, area: f64| Room { kind, area };
    let mut rooms = Vec::new();
    match means {
        Means::Poor => {
            // The kitchen in the living room; one bedroom a couple, one for every two others.
            rooms.push(room(RoomKind::Living, 12.0));
            for _ in 0..(couples + others.div_ceil(2)).max(1) {
                rooms.push(room(RoomKind::Bedroom, ROOM_MIN));
            }
            rooms.push(room(RoomKind::Bathroom, 3.0));
        }
        Means::Modest => {
            rooms.push(room(RoomKind::Living, 18.0));
            rooms.push(room(RoomKind::Kitchen, 8.0));
            for _ in 0..couples {
                rooms.push(room(RoomKind::Bedroom, 13.0));
            }
            for _ in 0..others.div_ceil(2) {
                rooms.push(room(RoomKind::Bedroom, 10.0));
            }
            rooms.push(room(RoomKind::Bathroom, 4.0));
        }
        Means::Comfortable => {
            rooms.push(room(RoomKind::Living, 30.0));
            rooms.push(room(RoomKind::Kitchen, 12.0));
            rooms.push(room(RoomKind::Dining, 14.0));
            rooms.push(room(RoomKind::Study, 10.0));
            for _ in 0..couples {
                rooms.push(room(RoomKind::Bedroom, 16.0));
            }
            for _ in 0..others {
                rooms.push(room(RoomKind::Bedroom, 12.0));
            }
            rooms.push(room(RoomKind::Bathroom, 5.0));
            rooms.push(room(RoomKind::Bathroom, 5.0));
        }
        Means::Rich => {
            rooms.push(room(RoomKind::Salon, 50.0));
            rooms.push(room(RoomKind::Dining, 25.0));
            rooms.push(room(RoomKind::Kitchen, 18.0));
            rooms.push(room(RoomKind::Study, 15.0));
            rooms.push(room(RoomKind::Service, 10.0));
            rooms.push(room(RoomKind::Store, 8.0));
            // A suite a couple and a suite a person: a bedroom with its bathroom.
            for _ in 0..(couples + others) {
                rooms.push(room(RoomKind::Bedroom, 20.0));
                rooms.push(room(RoomKind::Bathroom, 8.0));
            }
            rooms.push(room(RoomKind::Service, 10.0));
        }
    }
    rooms
}

/// A room laid out: its kind, its storey, and its rectangle on the lot, metres from the lot's
/// street-side left corner, `x` along the street and `y` into the depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// What it is for.
    pub kind: RoomKind,
    /// Its storey, 0 the ground floor.
    pub storey: usize,
    /// Its extent along the street, metres from the lot's left corner (the hall takes the first
    /// [`HALL`]).
    pub x: (f64, f64),
    /// Its extent into the depth, metres from the street.
    pub y: (f64, f64),
}

/// A house's plan: its width along the street, the depth its rooms took, and its rooms by
/// storey. The hall and the stair run from `x = 0` to [`HALL`] the whole depth on every storey.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Its width along the street, metres.
    pub width: f64,
    /// The depth its rooms took, metres.
    pub depth: f64,
    /// Its rooms by storey, each storey's in bands from the front to the back, side by side
    /// across each band.
    pub storeys: Vec<Vec<Placed>>,
}

impl Plan {
    /// The plan made `depth` deep where it is shallower: every storey's last band stretched to
    /// it, its rooms the larger (a house deep enough for its straight flight, [`super::houses`]).
    pub fn deepen(&mut self, depth: f64) {
        if depth <= self.depth {
            return;
        }
        let old = self.depth;
        for r in self.storeys.iter_mut().flatten() {
            if (r.y.1 - old).abs() < 1e-9 {
                r.y.1 = depth;
            }
        }
        self.depth = depth;
    }

    /// The rooms' floor area over all the storeys, m² (the hall left out).
    pub fn rooms_area(&self) -> f64 {
        self.storeys
            .iter()
            .flatten()
            .map(|r| (r.x.1 - r.x.0) * (r.y.1 - r.y.0))
            .sum()
    }
}

/// The rooms of `programme` laid on a lot `width` wide (at least [`HALL`] plus a room's least
/// width) and at most `max_depth` deep: beside the hall, in bands from the front to the back,
/// each band as deep as its first room is square (within [`DEPTH`]) and its rooms side by side
/// across the width left, the largest first and the small ones filling the bands' ends; when a
/// storey is full the next begins, the day rooms first on the ground floor, the bedrooms and
/// bathrooms above. A band's rooms share its width and every storey's last band stretches to
/// the house's depth, so the bands tile each storey and the storeys stack. A room's area may
/// grow for it, never shrink under what was asked (but a room wider than the house, which
/// takes the whole width at the deepest a room goes).
pub fn plan(programme: &[Room], width: f64, max_depth: f64) -> Plan {
    // The rooms end at the far party wall's half.
    let end = width - WALL;
    let inner = (end - HALL).max(2.4);
    let mut rooms: Vec<Room> = programme.to_vec();
    // The day rooms first, the largest first; the night rooms in the programme's order, each
    // bathroom after the bedrooms it serves (a suite's beside its bedroom): sorted by area, the
    // bathrooms all came last and a storey of them went on top.
    rooms.sort_by(|a, b| {
        b.kind
            .by_day()
            .cmp(&a.kind.by_day())
            .then(if a.kind.by_day() {
                b.area.total_cmp(&a.area)
            } else {
                std::cmp::Ordering::Equal
            })
    });
    // The bedrooms go upstairs together once the day rooms take half the ground floor's depth; a
    // small household keeps to one floor. The first of them decides: a bathroom alone went up
    // after a bedroom stayed down, a floor for 3 m².
    let mut night = false;
    let upstairs = |r: &Room, storeys: usize, y: f64, night: bool| {
        !r.kind.by_day() && !night && storeys == 1 && y >= max_depth * 0.5
    };
    let mut storeys: Vec<Vec<Placed>> = vec![Vec::new()];
    // The asked areas of the current storey's rooms, in their order.
    let mut asked: Vec<f64> = Vec::new();
    // A band's depth from its first room's area: as deep as the room is square, or as its area
    // across the width left asks, within [`DEPTH`] and the plan's depth.
    let band = |area: f64| {
        let d = area.sqrt().clamp(DEPTH.0, DEPTH.1);
        let d = if area / d > inner {
            (area / inner).clamp(DEPTH.0, DEPTH.1)
        } else {
            d
        };
        d.min(max_depth)
    };
    let mut y = 0.0;
    let mut i = 0;
    // Every pass places a band or moves one room (at most once a storey): far fewer than this.
    let mut passes = 0usize;
    while i < rooms.len() {
        passes += 1;
        assert!(
            passes <= 64 * (programme.len() + 2),
            "rooms::plan does not end: {} rooms, {} storeys",
            rooms.len(),
            storeys.len()
        );
        let first = rooms[i];
        let d = band(first.area);
        let up = upstairs(&first, storeys.len(), y, night);
        // Only small rooms left past the storey's depth (a bathroom, a store): the storey's bands
        // give up the depth they spare for them rather than a floor holding a bathroom alone, its
        // street front blank (the owner, 2026-10-16).
        let small = rooms[i..]
            .iter()
            .all(|r| matches!(r.kind, RoomKind::Bathroom | RoomKind::Store));
        let k = storeys.len() - 1;
        if y > 0.0
            && !up
            && small
            && y + d > max_depth + 1e-9
            && squeeze(&mut storeys[k], &asked, y + d - max_depth)
        {
            y = max_depth - d;
        }
        if y > 0.0 && (y + d > max_depth + 1e-9 || up) {
            arrange(storeys.last_mut().expect("a storey"));
            storeys.push(Vec::new());
            asked.clear();
            y = 0.0;
            // Its band's depth, as the band will take it.
            let company = band(ROOM_MIN);
            if small && company + d <= max_depth + 1e-9 {
                // Still a storey for them alone: a room with a window keeps them company at the
                // front (the owner, 2026-10-16: "it can be an office, a guest room, or an utility
                // room (although they tend to be downstairs)"), an office where the household has
                // none, else a guest room. Only where both fit the depth, so the next band places
                // the bathroom: on a shallow lot it never fitted behind the office, and an office
                // went in again and again until the memory ran out (Windows down twice, 2026-10-16).
                let kind = if rooms.iter().any(|r| r.kind == RoomKind::Study) {
                    RoomKind::Bedroom
                } else {
                    RoomKind::Study
                };
                rooms.insert(
                    i,
                    Room {
                        kind,
                        area: ROOM_MIN,
                    },
                );
                continue;
            }
            // A storey's front band holds a room with a window, not a store or a bathroom: the
            // next room that is comes first.
            if let Some(j) = rooms[i..]
                .iter()
                .position(|r| !matches!(r.kind, RoomKind::Bathroom | RoomKind::Store))
                .filter(|&j| j > 0)
            {
                let r = rooms.remove(i + j);
                rooms.insert(i, r);
                continue;
            }
        }
        let storey = storeys.len() - 1;
        let start = storeys[storey].len();
        let mut x = HALL;
        while i < rooms.len() {
            let r = rooms[i];
            let w = (r.area / d).max(2.4);
            if x > HALL && (x + w > end + 1e-9 || upstairs(&r, storeys.len(), y, night)) {
                break;
            }
            let w = w.min(end - x);
            storeys[storey].push(Placed {
                kind: r.kind,
                storey,
                x: (x, x + w),
                y: (y, y + d),
            });
            asked.push(r.area);
            night |= !r.kind.by_day();
            x += w;
            i += 1;
        }
        // The band's rooms share what is left of its width, in proportion.
        let used = x - HALL;
        if used < inner - 1e-9 {
            let scale = inner / used;
            let mut at = HALL;
            for r in &mut storeys[storey][start..] {
                let w = (r.x.1 - r.x.0) * scale;
                r.x = (at, at + w);
                at += w;
            }
        }
        if let Some(last) = storeys[storey].last_mut() {
            last.x.1 = end;
        }
        y += d;
    }
    arrange(storeys.last_mut().expect("a storey"));
    let depth = storeys
        .iter()
        .filter_map(|s| s.last().map(|r| r.y.1))
        .fold(0.0, f64::max);
    for s in &mut storeys {
        if let Some(end) = s.last().map(|r| r.y.1) {
            for r in s.iter_mut().filter(|r| (r.y.1 - end).abs() < 1e-9) {
                r.y.1 = depth;
            }
        }
    }
    Plan {
        width,
        depth,
        storeys,
    }
}

/// A storey's bands (`placed`) in their order from the front: a band with a room that wants a
/// window first and another last, against the street and the back walls, the bands of bathrooms,
/// stores and service rooms between them, where a row house's middle has no outside wall
/// (`research/house-layouts.md` rule 14: a bedroom in the middle band had no window). The rooms
/// then in that order, front to back.
fn arrange(placed: &mut [Placed]) {
    let dark = |k: RoomKind| matches!(k, RoomKind::Bathroom | RoomKind::Store | RoomKind::Service);
    // Each band's start, end and whether a room of it wants a window, from the front.
    let mut bands: Vec<(f64, f64, bool)> = Vec::new();
    for r in placed.iter() {
        match bands.iter_mut().find(|b| (b.0 - r.y.0).abs() < 1e-9) {
            Some(b) => b.2 |= !dark(r.kind),
            None => bands.push((r.y.0, r.y.1, !dark(r.kind))),
        }
    }
    bands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let lit: Vec<usize> = (0..bands.len()).filter(|&b| bands[b].2).collect();
    let mut order: Vec<usize> = Vec::with_capacity(bands.len());
    order.extend(lit.first());
    order.extend((0..bands.len()).filter(|&b| !bands[b].2));
    if lit.len() > 2 {
        order.extend(&lit[1..lit.len() - 1]);
    }
    if lit.len() > 1 {
        order.extend(lit.last());
    }
    let mut at = 0.0;
    let moved: Vec<(f64, f64, f64)> = order
        .iter()
        .map(|&b| {
            let (y0, y1, _) = bands[b];
            at += y1 - y0;
            (y0, at - (y1 - y0), at)
        })
        .collect();
    for r in placed.iter_mut() {
        if let Some(&(_, s, e)) = moved.iter().find(|m| (m.0 - r.y.0).abs() < 1e-9) {
            r.y = (s, e);
        }
    }
    placed.sort_by(|a, b| a.y.0.total_cmp(&b.y.0).then(a.x.0.total_cmp(&b.x.0)));
}

/// A storey's bands (`placed`, the rooms that asked `asked`) made `by` metres shallower in all,
/// each giving up depth in proportion to what it spares: none under [`DEPTH`]'s least, none of
/// its rooms under what it asked or [`ROOM_MIN`], whichever is less. Whether they could.
fn squeeze(placed: &mut [Placed], asked: &[f64], by: f64) -> bool {
    // The bands' starts, in order, and what each spares.
    let mut bands: Vec<(f64, f64, f64)> = Vec::new();
    for (r, &a) in placed.iter().zip(asked) {
        let least = (a.min(ROOM_MIN) / (r.x.1 - r.x.0).max(1e-6)).max(DEPTH.0);
        match bands.iter_mut().find(|b| (b.0 - r.y.0).abs() < 1e-9) {
            Some(b) => b.2 = b.2.max(least),
            None => bands.push((r.y.0, r.y.1, least)),
        }
    }
    let spare: f64 = bands.iter().map(|b| (b.1 - b.0 - b.2).max(0.0)).sum();
    if spare < by - 1e-9 || bands.is_empty() {
        return false;
    }
    // Each band's new start and end, from the front.
    let mut at = 0.0;
    let new: Vec<(f64, f64, f64)> = bands
        .iter()
        .map(|b| {
            let give = (b.1 - b.0 - b.2).max(0.0) / spare * by;
            let start = at;
            at += b.1 - b.0 - give;
            (b.0, start, at)
        })
        .collect();
    for r in placed.iter_mut() {
        if let Some(&(_, s, e)) = new.iter().find(|n| (n.0 - r.y.0).abs() < 1e-9) {
            r.y = (s, e);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(plan: &Plan, programme: &[Room], max_depth: f64) {
        assert!(plan.depth <= max_depth + 1e-9, "depth {}", plan.depth);
        // Every room placed, an office or a guest room more at most beside a storey's bathroom.
        let placed = plan.storeys.iter().map(Vec::len).sum::<usize>();
        assert!(
            placed >= programme.len() && placed <= programme.len() + plan.storeys.len(),
            "every room placed: {placed} of {}",
            programme.len()
        );
        // Each storey's rooms tile it beside the hall, none over another.
        for (k, storey) in plan.storeys.iter().enumerate() {
            let mut area = 0.0;
            for (a, r) in storey.iter().enumerate() {
                assert_eq!(r.storey, k);
                assert!(
                    r.x.0 >= HALL - 1e-9
                        && r.x.1 <= plan.width - WALL + 1e-9
                        && r.y.0 >= -1e-9
                        && r.y.1 <= plan.depth + 1e-9,
                    "a room off the house: {r:?}"
                );
                area += (r.x.1 - r.x.0) * (r.y.1 - r.y.0);
                for o in &storey[..a] {
                    let apart = r.x.1 <= o.x.0 + 1e-9
                        || o.x.1 <= r.x.0 + 1e-9
                        || r.y.1 <= o.y.0 + 1e-9
                        || o.y.1 <= r.y.0 + 1e-9;
                    assert!(apart, "rooms over each other: {r:?} {o:?}");
                }
            }
            let floor = (plan.width - WALL - HALL) * plan.depth;
            assert!(
                (area - floor).abs() < 1e-6,
                "storey {k} tiled: {area:.2} of {floor:.2} m²"
            );
        }
        // No room under the minimum, none under what it asked.
        let mut asked: Vec<f64> = programme.iter().map(|r| r.area).collect();
        asked.sort_by(f64::total_cmp);
        let mut got: Vec<f64> = plan
            .storeys
            .iter()
            .flatten()
            .map(|r| (r.x.1 - r.x.0) * (r.y.1 - r.y.0))
            .collect();
        got.sort_by(f64::total_cmp);
        for (a, g) in asked.iter().zip(&got) {
            assert!(
                g + 1e-6 >= a.min(ROOM_MIN) || *a < ROOM_MIN,
                "{g:.1} m² for {a:.1}"
            );
        }
    }

    #[test]
    fn a_modest_family_of_four_fits_a_village_lot_on_two_storeys() {
        let p = programme(4, Means::Modest);
        assert_eq!(
            p.iter().filter(|r| r.kind == RoomKind::Bedroom).count(),
            2,
            "a couple's bedroom and the children's"
        );
        let plan = plan(&p, 6.0, 12.0);
        check(&plan, &p, 12.0);
        assert!(
            (2..=3).contains(&plan.storeys.len()),
            "{} storeys",
            plan.storeys.len()
        );
        assert!(plan.storeys[0].iter().all(|r| r.kind.by_day()));
        assert!(plan.rooms_area() >= 50.0, "{:.0} m²", plan.rooms_area());
    }

    /// No storey holds only bathrooms or stores (the owner, 2026-10-16: a floor for a bathroom
    /// left its street front blank).
    #[test]
    fn no_storey_is_a_bathroom_alone() {
        let mut alone = Vec::new();
        for means in [Means::Poor, Means::Modest, Means::Comfortable, Means::Rich] {
            for people in 1..=6 {
                for width in [5.6, 6.0, 7.0, 8.5, 10.0, 12.0] {
                    for max_depth in [8.0, 10.0, 12.0] {
                        let p = programme(people, means);
                        let plan = plan(&p, width, max_depth);
                        check(&plan, &p, max_depth);
                        let lone = plan.storeys.iter().any(|s| {
                            s.iter()
                                .all(|r| matches!(r.kind, RoomKind::Bathroom | RoomKind::Store))
                        });
                        if lone {
                            alone.push(format!("{means:?} {people} on {width} × {max_depth}"));
                        }
                    }
                }
            }
        }
        assert!(alone.is_empty(), "{} {alone:?}", alone.len());
    }

    /// Every plan ends with every room placed, on lots as shallow as a room (the houses' lots
    /// cap the depth): an office added for a bathroom that never fitted behind it looped until
    /// the memory ran out (2026-10-16).
    #[test]
    fn every_plan_ends_on_shallow_lots() {
        for means in [Means::Poor, Means::Modest, Means::Comfortable, Means::Rich] {
            for people in 1..=6 {
                for width in [5.6, 6.0, 8.5, 12.0] {
                    for max_depth in [2.6, 3.0, 4.0, 5.0, 5.5, 6.0, 7.0] {
                        let p = programme(people, means);
                        let plan = plan(&p, width, max_depth);
                        let placed = plan.storeys.iter().map(Vec::len).sum::<usize>();
                        assert!(placed >= p.len() && placed <= p.len() + plan.storeys.len());
                        assert!(plan.depth <= max_depth + 1e-9);
                    }
                }
            }
        }
    }

    #[test]
    fn a_poor_household_keeps_the_minimums() {
        let p = programme(3, Means::Poor);
        let plan = plan(&p, 6.0, 10.0);
        check(&plan, &p, 10.0);
        for r in plan.storeys.iter().flatten() {
            let area = (r.x.1 - r.x.0) * (r.y.1 - r.y.0);
            if matches!(r.kind, RoomKind::Living | RoomKind::Bedroom) {
                assert!(area >= ROOM_MIN - 1e-9, "{area:.1} m² {:?}", r.kind);
            }
        }
    }

    #[test]
    fn a_rich_house_takes_a_wide_lot_and_its_suites_go_upstairs() {
        let p = programme(5, Means::Rich);
        let plan = plan(&p, 14.0, 16.0);
        check(&plan, &p, 16.0);
        assert!(plan.storeys.len() >= 2);
        assert!(plan.rooms_area() >= 250.0, "{:.0} m²", plan.rooms_area());
        let upstairs = plan.storeys[1..].iter().flatten();
        assert!(upstairs.clone().any(|r| r.kind == RoomKind::Bedroom));
    }
}
