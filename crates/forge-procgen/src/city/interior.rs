//! A house's insides from its rooms (D-062, #266; the owner, 2026-10-16: "a house or structure
//! should be thought by thinking about the rooms"). The plan's rooms ([`super::rooms::plan`]) are
//! laid inside the structure's walls beside its core, the hall with the straight flight the plan
//! keeps at the start of the front ([`super::cores`] puts a house's core there): partitions
//! between the bands and between the rooms of a band, a doorway from the hall into each band's
//! first room and from each room into the next, so every room of a storey is reached from the
//! hall, and the flight climbing from the street wall into the house.

use glam::DVec2;

use super::cores::CORE_WALL;
use super::mass::Mass;
use super::rooms::{HALL, RoomKind, WALL};
use super::structure::{self, Structure};

/// A partition's thickness, metres: plastered brick on edge.
pub const PARTITION: f64 = 0.1;
/// A doorway's clear width, metres: a wheelchair's (0.90 m, the owner's rule).
pub const DOORWAY: f64 = 0.9;
/// A doorway's head over its floor, metres.
pub const DOORWAY_HEAD: f64 = 2.1;
/// How far a doorway keeps from a partition's ends, metres: room for its frame.
const JAMB: f64 = 0.15;

/// A wall inside a house: its line, its thickness and its doorways.
#[derive(Clone, Debug, PartialEq)]
pub struct Partition {
    /// Its line's start, metres on the plan.
    pub a: DVec2,
    /// Its line's end.
    pub b: DVec2,
    /// Its thickness, centred on its line, metres.
    pub thickness: f64,
    /// Its storey, 0 the ground floor.
    pub storey: usize,
    /// Its doorways, metres along it from `a`, each [`DOORWAY`] wide.
    pub doors: Vec<(f64, f64)>,
}

impl Partition {
    /// Its length, metres.
    pub fn length(&self) -> f64 {
        self.a.distance(self.b)
    }
}

/// A room inside the walls.
#[derive(Clone, Debug, PartialEq)]
pub struct InRoom {
    /// What it is for.
    pub kind: RoomKind,
    /// Its storey, 0 the ground floor.
    pub storey: usize,
    /// Its outline, anticlockwise, its first edge on the street's side.
    pub outline: [DVec2; 4],
}

/// A house's insides.
#[derive(Clone, Debug, PartialEq)]
pub struct Interior {
    /// The rooms, storey by storey.
    pub rooms: Vec<InRoom>,
    /// The partitions, storey by storey: the hall's wall with a doorway into each band, the
    /// walls between the bands, and between the rooms of a band with a doorway each.
    pub partitions: Vec<Partition>,
    /// The hall on every storey, from the street wall to the back wall: the core's stair and
    /// corridor and, behind the core, the corridor on to the back.
    pub hall: [DVec2; 4],
    /// The flight's box (the core's stair), the same on every storey but the top; `None` in a house
    /// of one storey.
    pub stair: Option<[DVec2; 4]>,
    /// The way the flight climbs: from the street wall into the house.
    pub climb: DVec2,
    /// The risers each storey's flight climbs, ground first.
    pub risers: Vec<u32>,
}

/// The insides of house `m` (one with its rooms' plan, [`Mass::rooms`]) whose structure is `s`:
/// `None` without a plan, a house's core, or a floor inside the walls, or where the core does not
/// stand at the start of the front where the plan keeps the hall. The plan's rooms beside the
/// hall are stretched to the floor between the core and the far wall, front to back.
pub fn house(m: &Mass, s: &Structure) -> Option<Interior> {
    let plan = m.rooms.as_ref()?;
    // The hall: the house core's span along its wall, or, in a house of one storey with no core,
    // the plan's hall along the street wall on its first edge.
    let core = s.cores.iter().find(|c| c.house);
    if core.is_none() && !s.cores.is_empty() {
        return None;
    }
    let w = match core {
        Some(c) => c.wall,
        None => s
            .walls
            .iter()
            .position(|w| w.edge == 0 && w.role == structure::WallRole::Street)?,
    };
    let wall = &s.walls[w];
    let t = (wall.b - wall.a).normalize_or_zero();
    let n = DVec2::new(-t.y, t.x);
    let o = wall.a;
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    let inside = structure::floor(m, &thickness)?;
    let span = |f: &dyn Fn(DVec2) -> f64| {
        inside
            .iter()
            .map(|&p| f(p))
            .fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
    };
    let (umin, umax) = span(&|p| (p - o).dot(t));
    let (vmin, vmax) = span(&|p| (p - o).dot(n));
    let (lo, hi) = core.map_or((umin, HALL), |c| c.span);
    // The hall at the start of the wall, as the plan keeps it, or at its end where the core stood
    // by the far corner: the plan's rooms then run the other way. A core in the middle of the
    // front leaves no plan to follow.
    let mirrored = lo - umin > 1.0 && umax - hi <= 1.0;
    let (room0, room1) = if mirrored { (umin, lo) } else { (hi, umax) };
    if (!mirrored && lo - umin > 1.0)
        || room1 - room0 < 1.8
        || vmax - vmin < 2.6
        || plan.width <= HALL + WALL
    {
        return None;
    }
    let at = |u: f64, v: f64| o + t * u + n * v;
    let rect = |u0: f64, u1: f64, v0: f64, v1: f64| {
        let (a, b) = (u0.min(u1), u0.max(u1));
        [at(a, v0), at(b, v0), at(b, v1), at(a, v1)]
    };
    // The plan's metres to the floor's: its rooms beside the hall stretched to the far wall, its
    // depth to the back wall.
    let x = |px: f64| {
        let f = (px - HALL) / (plan.width - WALL - HALL);
        if mirrored {
            room1 - f * (room1 - room0)
        } else {
            room0 + f * (room1 - room0)
        }
    };
    let y = |py: f64| vmin + py / plan.depth.max(1e-6) * (vmax - vmin);
    // A doorway `DOORWAY` wide about `mid` on a stretch from `from` to `to`, clear of its ends.
    let door = |from: f64, to: f64, mid: f64| -> Option<(f64, f64)> {
        let (lo, hi) = (from + JAMB + DOORWAY * 0.5, to - JAMB - DOORWAY * 0.5);
        (lo <= hi).then(|| {
            let m = mid.clamp(lo, hi);
            (m - DOORWAY * 0.5, m + DOORWAY * 0.5)
        })
    };
    let mut rooms = Vec::new();
    let mut partitions = Vec::new();
    let hall_line = if mirrored {
        lo + CORE_WALL * 0.5
    } else {
        hi - CORE_WALL * 0.5
    };
    // The openings of the outside walls on the plan's axes: each its storey, its span along the
    // front and where its wall stands into the depth, so a partition keeps off them.
    let openings: Vec<(usize, f64, f64, f64)> = s
        .openings
        .iter()
        .filter_map(|op| {
            let wl = &s.walls[op.wall];
            if !matches!(
                wl.role,
                structure::WallRole::Street | structure::WallRole::Court
            ) {
                return None;
            }
            let wt = (wl.b - wl.a).normalize_or_zero();
            let (p0, p1) = (wl.a + wt * op.span().0, wl.a + wt * op.span().1);
            let (u0, u1) = ((p0 - o).dot(t), (p1 - o).dot(t));
            Some((
                op.storey,
                u0.min(u1),
                u0.max(u1),
                ((p0 + p1) * 0.5 - o).dot(n),
            ))
        })
        .collect();
    // A partition at `u` meeting the wall that stands about `v` on storey `k`, moved off an
    // opening it would stand in (a wall behind a window, the owner's item 26 the other way
    // round): to the nearer side of the opening, half a partition and 15 cm clear, or the other
    // side, keeping a metre from its neighbours `lo` and `hi`.
    let clear = |k: usize, u: f64, v: f64, lo: f64, hi: f64| -> f64 {
        let margin = PARTITION * 0.5 + 0.15;
        let fits = |c: f64| c > lo + 1.0 && c < hi - 1.0;
        let mut u = u;
        for &(sk, a, b, wv) in &openings {
            if sk != k || (wv - v).abs() > 1.0 || u <= a - margin || u >= b + margin {
                continue;
            }
            let (left, right) = (a - margin, b + margin);
            let (near, far) = if u - left <= right - u {
                (left, right)
            } else {
                (right, left)
            };
            if fits(near) {
                u = near;
            } else if fits(far) {
                u = far;
            }
        }
        u
    };
    for (k, storey) in plan.storeys.iter().enumerate() {
        // The storey's bands, front to back, each its rooms left to right.
        let mut bands: Vec<(f64, f64, Vec<usize>)> = Vec::new();
        for (i, r) in storey.iter().enumerate() {
            match bands
                .iter_mut()
                .find(|b| (b.0 - r.y.0).abs() < 1e-6 && (b.1 - r.y.1).abs() < 1e-6)
            {
                Some(b) => b.2.push(i),
                None => bands.push((r.y.0, r.y.1, vec![i])),
            }
        }
        bands.sort_by(|p, q| p.0.total_cmp(&q.0));
        for b in &mut bands {
            b.2.sort_by(|&i, &j| storey[i].x.0.total_cmp(&storey[j].x.0));
        }
        // The hall's wall the whole depth, a doorway into each band's first room.
        let doors: Vec<(f64, f64)> = bands
            .iter()
            .filter_map(|(y0, y1, _)| {
                let (v0, v1) = (y(*y0) - vmin, y(*y1) - vmin);
                door(v0, v1, (v0 + v1) * 0.5)
            })
            .collect();
        partitions.push(Partition {
            a: at(hall_line, vmin),
            b: at(hall_line, vmax),
            thickness: CORE_WALL,
            storey: k,
            doors,
        });
        // Between the bands, wall to wall.
        for pair in bands.windows(2) {
            let v = y(pair[0].1);
            partitions.push(Partition {
                a: at(room0, v),
                b: at(room1, v),
                thickness: PARTITION,
                storey: k,
                doors: Vec::new(),
            });
        }
        // Each band's rooms between their boundaries, those on the front or the back wall moved
        // off its openings; between two rooms a doorway in the middle.
        for (y0, y1, ids) in &bands {
            let mut us: Vec<f64> = vec![x(storey[ids[0]].x.0)];
            us.extend(ids.iter().map(|&i| x(storey[i].x.1)));
            for j in 1..us.len() - 1 {
                let (lo, hi) = (us[j - 1].min(us[j + 1]), us[j - 1].max(us[j + 1]));
                if *y0 < 1e-9 {
                    us[j] = clear(k, us[j], 0.0, lo, hi);
                }
                if (*y1 - plan.depth).abs() < 1e-9 {
                    us[j] = clear(k, us[j], vmax, lo, hi);
                }
            }
            let (v0, v1) = (y(*y0), y(*y1));
            for (j, &i) in ids.iter().enumerate() {
                rooms.push(InRoom {
                    kind: storey[i].kind,
                    storey: k,
                    outline: rect(us[j], us[j + 1], v0, v1),
                });
            }
            let len = v1 - v0;
            for &u in &us[1..us.len() - 1] {
                partitions.push(Partition {
                    a: at(u, v0),
                    b: at(u, v1),
                    thickness: PARTITION,
                    storey: k,
                    doors: door(0.0, len, len * 0.5).into_iter().collect(),
                });
            }
        }
    }
    Some(Interior {
        rooms,
        partitions,
        hall: if mirrored {
            rect(lo, umax, vmin, vmax)
        } else {
            rect(umin, hi, vmin, vmax)
        },
        stair: core.map(|c| c.stair),
        climb: n,
        risers: core.map_or_else(Vec::new, |c| c.risers.clone()),
    })
}

/// How many of the partitions' ends meet an outside wall of `s` within one of its openings: a
/// wall behind a window or a door (one flush with a jamb, within a centimetre, is not).
pub fn at_openings(s: &Structure, interior: &Interior) -> usize {
    let mut count = 0;
    for p in &interior.partitions {
        for end in [p.a, p.b] {
            let hit = s.openings.iter().any(|op| {
                let w = &s.walls[op.wall];
                if op.storey != p.storey
                    || !matches!(
                        w.role,
                        structure::WallRole::Street | structure::WallRole::Court
                    )
                {
                    return false;
                }
                let t = (w.b - w.a).normalize_or_zero();
                let n = DVec2::new(-t.y, t.x);
                let (along, depth) = ((end - w.a).dot(t), (end - w.a).dot(n));
                let half = p.thickness * 0.5;
                (depth - w.thickness).abs() < 0.1
                    && along > op.span().0 - half + 0.01
                    && along < op.span().1 + half - 0.01
            });
            count += usize::from(hit);
        }
    }
    count
}

/// Whether every room of each storey of `interior` is reached from its hall through doorways.
pub fn reachable(interior: &Interior) -> bool {
    let storeys = interior
        .rooms
        .iter()
        .map(|r| r.storey + 1)
        .max()
        .unwrap_or(0);
    (0..storeys).all(|k| {
        let rooms: Vec<&InRoom> = interior.rooms.iter().filter(|r| r.storey == k).collect();
        // Which region a point lies in: the hall (0) or a room (1 + its index).
        let region = |p: DVec2| {
            if super::geom::contains(&interior.hall, p) {
                Some(0)
            } else {
                rooms
                    .iter()
                    .position(|r| super::geom::contains(&r.outline, p))
                    .map(|i| i + 1)
            }
        };
        let mut links: Vec<(usize, usize)> = Vec::new();
        for p in interior.partitions.iter().filter(|p| p.storey == k) {
            let along = (p.b - p.a).normalize_or_zero();
            let normal = DVec2::new(-along.y, along.x);
            for &(d0, d1) in &p.doors {
                let mid = p.a + along * ((d0 + d1) * 0.5);
                let off = p.thickness * 0.5 + 0.2;
                if let (Some(i), Some(j)) = (region(mid + normal * off), region(mid - normal * off))
                {
                    links.push((i, j));
                }
            }
        }
        let mut seen = vec![false; rooms.len() + 1];
        seen[0] = true;
        let mut changed = true;
        while changed {
            changed = false;
            for &(i, j) in &links {
                if seen[i] != seen[j] {
                    seen[i] = true;
                    seen[j] = true;
                    changed = true;
                }
            }
        }
        seen.iter().all(|&s| s)
    })
}
