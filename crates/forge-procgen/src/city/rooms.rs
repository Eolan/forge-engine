//! Houses thought from their rooms (`research/parcels-and-rooms.md` §4; the owner, 2026-10-16:
//! "a house or structure should be thought by thinking about the rooms, the real space inside
//! and the minimum required to make it a liveable space, depending on how rich it is"). A
//! household's programme, the rooms it needs and how big, comes from its size and its means;
//! the plan lays them on a lot of a given width, storey by storey: a hall with the stair along
//! one party wall the whole depth, the rooms in a column beside it from the front to the back,
//! the day rooms on the ground floor and the bedrooms above. The footprint's depth and the
//! storeys come out of the rooms, not the rooms out of a footprint.

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
/// The hall's width, metres: the stair (1.0 m) and a corridor beside it a wheelchair turns in.
pub const HALL: f64 = 2.2;
/// A room's depth at least and at most, metres: a room is wider than a corridor and no deeper
/// than daylight reaches from its window.
const DEPTH: (f64, f64) = (2.6, 6.5);

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
    /// Its extent along the street, metres from the lot's left corner.
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
    /// Its rooms by storey, each storey's from the front to the back.
    pub storeys: Vec<Vec<Placed>>,
}

impl Plan {
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
/// width) and at most `max_depth` deep: beside the hall, each room takes the full width left
/// and the depth its area asks (within [`DEPTH`]), from the front to the back; when a storey is
/// full the next begins, the day rooms first on the ground floor, the bedrooms and bathrooms
/// above. Every storey's last room stretches to the house's depth, so the storeys stack. A
/// room's area may grow for it, never shrink under what was asked.
pub fn plan(programme: &[Room], width: f64, max_depth: f64) -> Plan {
    let inner = (width - HALL).max(2.4);
    let mut rooms: Vec<Room> = programme.to_vec();
    // The day rooms first, in their order; then the rest.
    rooms.sort_by_key(|r| !r.kind.by_day());
    let mut storeys: Vec<Vec<Placed>> = vec![Vec::new()];
    let mut y = 0.0;
    let mut night = false;
    for r in rooms {
        let wanted = (r.area / inner).clamp(DEPTH.0, DEPTH.1);
        // The bedrooms go upstairs once the day rooms take half the ground floor's depth; a
        // small household keeps to one floor.
        let upstairs = !night && !r.kind.by_day() && storeys.len() == 1 && y >= max_depth * 0.5;
        night |= !r.kind.by_day();
        if (y + wanted > max_depth + 1e-9 || upstairs) && y > 0.0 {
            storeys.push(Vec::new());
            y = 0.0;
        }
        let depth = wanted.min(max_depth);
        let storey = storeys.len() - 1;
        storeys[storey].push(Placed {
            kind: r.kind,
            storey,
            x: (HALL, width),
            y: (y, y + depth),
        });
        y += depth;
    }
    let depth = storeys
        .iter()
        .filter_map(|s| s.last().map(|r| r.y.1))
        .fold(0.0, f64::max);
    for s in &mut storeys {
        if let Some(last) = s.last_mut() {
            last.y.1 = depth;
        }
    }
    Plan {
        width,
        depth,
        storeys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(plan: &Plan, programme: &[Room], max_depth: f64) {
        assert!(plan.depth <= max_depth + 1e-9, "depth {}", plan.depth);
        assert_eq!(
            plan.storeys.iter().map(Vec::len).sum::<usize>(),
            programme.len(),
            "every room placed"
        );
        for (k, storey) in plan.storeys.iter().enumerate() {
            let mut y = 0.0;
            for r in storey {
                assert_eq!(r.storey, k);
                assert!((r.y.0 - y).abs() < 1e-9, "rooms in a column, no gap");
                assert!(r.x.0 >= HALL - 1e-9 && r.x.1 <= plan.width + 1e-9);
                y = r.y.1;
            }
            assert!((y - plan.depth).abs() < 1e-9, "the storey stacks");
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

    #[test]
    fn a_poor_household_keeps_the_minimums() {
        let p = programme(3, Means::Poor);
        let plan = plan(&p, 5.0, 10.0);
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
