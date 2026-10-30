//! A house's insides from its rooms (D-062, #266; the owner, 2026-10-16: "a house or structure
//! should be thought by thinking about the rooms"). The plan's rooms ([`super::rooms::plan`]) are
//! laid inside the structure's walls beside its core, the hall with the straight flight the plan
//! keeps at the start of the front ([`super::cores`] puts a house's core there): partitions
//! between the bands and between the rooms of a band, a doorway from the hall into each band's
//! first room and from each room into the next, so every room of a storey is reached from the
//! hall, and the flight climbing from the back of the hall towards the street wall. A storey laid
//! round a landing (`rooms::RoomKind::Landing`) has the hall open on to it and a doorway from it
//! into each room off the hall, none between its rooms: no bedroom is entered through another.

use glam::DVec2;

use super::cores::CORE_WALL;
use super::mass::Mass;
use super::rooms::{HALL, RoomKind, WALL};
use super::structure::{self, Structure};

/// A partition's thickness, metres: plastered brick on edge.
pub const PARTITION: f64 = 0.1;
/// A doorway's clear width, metres: a wheelchair's (0.90 m, the owner's rule).
pub const DOORWAY: f64 = 0.93;
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
    /// The flight's box (the core's stair), its foot's corners first, the same on every storey but
    /// the top; `None` in a house of one storey.
    pub stair: Option<[DVec2; 4]>,
    /// The way the flight climbs: from the back of the hall towards the street wall, its landing
    /// at the head by the front, so the front door opens beside free floor (the owner, 2026-10-16:
    /// "stairs start behind the door (the door prevents access to the stairs)").
    pub climb: DVec2,
    /// The risers each storey's flight climbs, ground first.
    pub risers: Vec<u32>,
    /// Whether the flights turn a quarter at their foot towards the corridor beside them, over
    /// their first four steps, where less than [`FOOT_LANDING`] of floor stands before their first
    /// riser (the owner, 2026-10-30: stepped on from the side, "not enough space"; a quarter turn
    /// chosen over deeper houses).
    pub turn: bool,
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
    // The floor's extent along the front at a depth `v`: where a line square to the front there
    // crosses the floor's outline. A house on its lot's side lines (`houses`: a row where a lane
    // bends) stands on a trapezoid, its side walls slanted.
    let n_in = inside.len();
    let u_at = |v: f64| -> (f64, f64) {
        let v = v.clamp(vmin + 0.01, vmax - 0.01);
        let mut ext = (f64::MAX, f64::MIN);
        for i in 0..n_in {
            let (p, q) = (inside[i], inside[(i + 1) % n_in]);
            let (vp, vq) = ((p - o).dot(n), (q - o).dot(n));
            if (vp - v) * (vq - v) <= 0.0 && (vp - vq).abs() > 1e-9 {
                let u = ((p + (q - p) * ((v - vp) / (vq - vp))) - o).dot(t);
                ext = (ext.0.min(u), ext.1.max(u));
            }
        }
        if ext.0 > ext.1 { (umin, umax) } else { ext }
    };
    // Its extent every depth from `v0` to `v1` shares: a band's rooms keep within it, square to
    // the front, and only the room against a slanted side wall takes the slant.
    let shared = |v0: f64, v1: f64| {
        (0..=8).map(|k| v0 + (v1 - v0) * f64::from(k) / 8.0).fold(
            (f64::MIN, f64::MAX),
            |(l, r), v| {
                let (a, b) = u_at(v);
                (l.max(a), r.min(b))
            },
        )
    };
    // Without a core the plan's hall from the front's first corner, or from its far one where the
    // plan is turned over (`structure::hall_far`).
    let (lo, hi) = match core {
        Some(c) => c.span,
        None if structure::hall_far(m, &s.walls) => (wall.length() - HALL, umax),
        None => (umin, HALL),
    };
    // The hall at the start of the wall, as the plan keeps it, or at its end where the core stood
    // by the far corner: the plan's rooms then run the other way. A core in the middle of the
    // front leaves no plan to follow.
    let mirrored = lo - umin > 1.0 && umax - hi <= 1.0;
    // The rooms' span along the front over the depths `v0..v1`: from the hall's wall to the far
    // side wall where every one of those depths reaches.
    let span_over = |v0: f64, v1: f64| {
        let (l, r) = shared(v0, v1);
        if mirrored { (l, lo) } else { (hi, r) }
    };
    // The far side wall at a depth: where the band's partitions and its last room reach.
    let far = |v: f64| if mirrored { u_at(v).0 } else { u_at(v).1 };
    let (room0, room1) = span_over(vmin, vmin + (vmax - vmin) * 0.25);
    // The hall a metre wide at least at the front and the back: a side wall slanted across it (a
    // lot closing in behind the core) twisted its outline.
    let hall_width = |v: f64| {
        let (l, r) = u_at(v);
        if mirrored { r - lo } else { hi - l }
    };
    if (!mirrored && lo - umin > 1.0)
        || room1 - room0 < 1.8
        || vmax - vmin < 2.6
        || plan.width <= HALL + WALL
        || hall_width(vmin) < 1.0
        || hall_width(vmax) < 1.0
    {
        return None;
    }
    let at = |u: f64, v: f64| o + t * u + n * v;
    let rect = |u0: f64, u1: f64, v0: f64, v1: f64| {
        let (a, b) = (u0.min(u1), u0.max(u1));
        [at(a, v0), at(b, v0), at(b, v1), at(a, v1)]
    };
    // The plan's metres to the floor's within the span `(r0, r1)` of the band: its rooms beside the
    // hall stretched to the far wall, its depth to the back wall.
    let x = |px: f64, (r0, r1): (f64, f64)| {
        let f = (px - HALL) / (plan.width - WALL - HALL);
        if mirrored {
            r1 - f * (r1 - r0)
        } else {
            r0 + f * (r1 - r0)
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
    // side, keeping `room` metres from its neighbours `lo` and `hi` (a metre, or a doorway's width
    // beside a landing).
    let clear = |k: usize, u: f64, v: f64, lo: f64, hi: f64, room: f64| -> f64 {
        let margin = PARTITION * 0.5 + 0.15;
        let fits = |c: f64| c > lo + room && c < hi - room;
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
        // The storey's landing, where it has one (`rooms::plan`): the hall open on to it, and
        // every room beside it but the first of its band (on the hall) a doorway on to it.
        let landing = bands
            .iter()
            .position(|b| b.2.iter().any(|&i| storey[i].kind == RoomKind::Landing));
        // The plan's depths on the floor: in proportion, but a landing [`rooms::LANDING`] clear
        // between its partitions, the bands round it sharing the rest (in proportion, the walls'
        // thickness took a fifth off a corridor 1.2 m wide on the plan).
        let y = |py: f64| -> f64 {
            let Some(l) = landing else {
                return y(py);
            };
            let (l0, l1) = (bands[l].0, bands[l].1);
            let floor = vmax - vmin;
            let keep = (super::rooms::LANDING + PARTITION).min(floor * 0.5);
            let scale = (floor - keep) / (plan.depth - (l1 - l0)).max(1e-6);
            if py <= l0 {
                vmin + py * scale
            } else if py <= l1 {
                vmin + l0 * scale + (py - l0) / (l1 - l0).max(1e-6) * keep
            } else {
                vmin + l0 * scale + keep + (py - l1) * scale
            }
        };
        // A partition's least distance from its neighbours where it moves off a window: a
        // doorway's width with its jambs beside a landing, each room's own doorway on to it.
        let room = if landing.is_some() {
            DOORWAY + 2.0 * JAMB + 0.02
        } else {
            1.0
        };
        // Each band's partitions across the front from the hall's to the far end, those on the
        // front or the back wall moved off its openings.
        let mut cuts: Vec<Vec<f64>> = Vec::with_capacity(bands.len());
        for (y0, y1, ids) in &bands {
            let sp = span_over(y(*y0), y(*y1));
            // A wedge whose side walls close in behind the hall leaves a band no room: no insides.
            if sp.1 - sp.0 < 1.2 {
                return None;
            }
            let mut us: Vec<f64> = vec![x(storey[ids[0]].x.0, sp)];
            us.extend(ids.iter().map(|&i| x(storey[i].x.1, sp)));
            for j in 1..us.len() - 1 {
                let (lo, hi) = (us[j - 1].min(us[j + 1]), us[j - 1].max(us[j + 1]));
                if *y0 < 1e-9 {
                    us[j] = clear(k, us[j], 0.0, lo, hi, room);
                }
                if (*y1 - plan.depth).abs() < 1e-9 {
                    us[j] = clear(k, us[j], vmax, lo, hi, room);
                }
            }
            cuts.push(us);
        }
        // The hall's wall the whole depth, a doorway into each band's first room, open across
        // the landing between the partitions either side of it.
        let doors: Vec<(f64, f64)> = bands
            .iter()
            .enumerate()
            .filter_map(|(b, (y0, y1, _))| {
                let (v0, v1) = (y(*y0) - vmin, y(*y1) - vmin);
                if landing == Some(b) {
                    Some((v0 + PARTITION * 0.5, v1 - PARTITION * 0.5))
                } else {
                    door(v0, v1, (v0 + v1) * 0.5)
                }
            })
            .collect();
        partitions.push(Partition {
            a: at(hall_line, vmin),
            b: at(hall_line, vmax),
            thickness: CORE_WALL,
            storey: k,
            doors,
        });
        // Between the bands, wall to wall: from the hall's wall to the far side wall where it
        // stands at that depth; beside the landing, a doorway into each room of the other band
        // but its first, in the middle of the room's stretch along it. Which rooms of each band
        // have theirs.
        let mut landed: Vec<Vec<bool>> = bands.iter().map(|b| vec![false; b.2.len()]).collect();
        for (b, pair) in bands.windows(2).enumerate() {
            let v = y(pair[0].1);
            let (u0, u1) = if mirrored {
                (far(v), room1)
            } else {
                (room0, far(v))
            };
            let beside = match landing {
                Some(l) if l == b => Some(b + 1),
                Some(l) if l == b + 1 => Some(b),
                _ => None,
            };
            let mut doors: Vec<(f64, f64)> = Vec::new();
            if let Some(o) = beside {
                let us = &cuts[o];
                for j in 1..bands[o].2.len() {
                    // The last room reaches the far side wall.
                    let end = if j + 2 < us.len() { us[j + 1] } else { far(v) };
                    let (lo, hi) = (us[j].min(end) - u0, us[j].max(end) - u0);
                    if let Some(d) = door(lo, hi, (lo + hi) * 0.5) {
                        doors.push(d);
                        landed[o][j] = true;
                    }
                }
                // Along the partition from its start: a mirrored plan's rooms run the other way.
                doors.sort_by(|p, q| p.0.total_cmp(&q.0));
            }
            partitions.push(Partition {
                a: at(u0, v),
                b: at(u1, v),
                thickness: PARTITION,
                storey: k,
                doors,
            });
        }
        // Each band's rooms between their boundaries; between two rooms a doorway in the middle,
        // but into a room with its own on to the landing.
        for (((y0, y1, ids), us), landed) in bands.iter().zip(&cuts).zip(&landed) {
            let (v0, v1) = (y(*y0), y(*y1));
            for (j, &i) in ids.iter().enumerate() {
                // The last room reaches the far side wall where it stands at either end.
                let outline = if j + 2 < us.len() {
                    rect(us[j], us[j + 1], v0, v1)
                } else if mirrored {
                    [
                        at(far(v0), v0),
                        at(us[j], v0),
                        at(us[j], v1),
                        at(far(v1), v1),
                    ]
                } else {
                    [
                        at(us[j], v0),
                        at(far(v0), v0),
                        at(far(v1), v1),
                        at(us[j], v1),
                    ]
                };
                rooms.push(InRoom {
                    kind: storey[i].kind,
                    storey: k,
                    outline,
                });
            }
            let len = v1 - v0;
            for (j, &u) in us.iter().enumerate().take(us.len() - 1).skip(1) {
                partitions.push(Partition {
                    a: at(u, v0),
                    b: at(u, v1),
                    thickness: PARTITION,
                    storey: k,
                    doors: door(0.0, len, len * 0.5)
                        .filter(|_| !landed[j])
                        .into_iter()
                        .collect(),
                });
            }
        }
    }
    Some(Interior {
        rooms,
        partitions,
        // To its side wall where it stands at the front and at the back.
        hall: if mirrored {
            [
                at(lo, vmin),
                at(u_at(vmin).1, vmin),
                at(u_at(vmax).1, vmax),
                at(lo, vmax),
            ]
        } else {
            [
                at(u_at(vmin).0, vmin),
                at(hi, vmin),
                at(hi, vmax),
                at(u_at(vmax).0, vmax),
            ]
        },
        stair: core.map(|c| [c.stair[2], c.stair[3], c.stair[0], c.stair[1]]),
        climb: -n,
        risers: core.map_or_else(Vec::new, |c| c.risers.clone()),
        // The floor before the flights' foot, back from its middle against the climb (`n`) to the
        // floor's edge.
        turn: core.is_some_and(|c| {
            let foot = (c.stair[2] + c.stair[3]) * 0.5;
            let n_in = inside.len();
            let room = (0..n_in)
                .filter_map(|i| {
                    super::geom::segment_intersection(
                        foot,
                        foot + n * 20.0,
                        inside[i],
                        inside[(i + 1) % n_in],
                    )
                })
                .map(|(t, _)| t * 20.0)
                .fold(f64::MAX, f64::min);
            room < FOOT_LANDING
        }),
    })
}

/// The floor a flight's foot wants before its first riser, metres: as deep as the flight is wide
/// (a stair's landing at least its width). Where less stands there, the flight turns a quarter at
/// its foot towards the corridor beside it ([`Interior::turn`]).
pub const FOOT_LANDING: f64 = super::cores::FLIGHT;

/// How many of the partitions' ends meet an outside wall of `s` within one of its openings: a
/// wall behind a window or a door (one flush with a jamb, within a centimetre, is not).
pub fn at_openings(s: &Structure, interior: &Interior) -> usize {
    openings_hit(s, interior).len()
}

/// The partitions' ends that meet an outside wall of `s` within one of its openings, each with
/// its storey ([`at_openings`] counts them).
pub fn openings_hit(s: &Structure, interior: &Interior) -> Vec<(DVec2, usize)> {
    let mut out = Vec::new();
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
            if hit {
                out.push((end, p.storey));
            }
        }
    }
    out
}

/// The doorways of storey `k` of `interior`, each as the two places it joins: 0 the hall, `1 + i`
/// the room `interior.rooms[i]`.
pub fn doorways(interior: &Interior, k: usize) -> Vec<(usize, usize)> {
    // Which place a point lies in.
    let region = |p: DVec2| {
        if super::geom::contains(&interior.hall, p) {
            Some(0)
        } else {
            interior
                .rooms
                .iter()
                .position(|r| r.storey == k && super::geom::contains(&r.outline, p))
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
            if let (Some(i), Some(j)) = (region(mid + normal * off), region(mid - normal * off)) {
                links.push((i, j));
            }
        }
    }
    links
}

/// The way in to each place of storey `k` from its hall through [`doorways`]: the place it is
/// entered from (0 the hall, `1 + i` the room `interior.rooms[i]`), by place as numbered there;
/// `None` for the hall itself, for the rooms of other storeys and for a room no doorway reaches.
pub fn entered_from(interior: &Interior, k: usize) -> Vec<Option<usize>> {
    let links = doorways(interior, k);
    let mut from: Vec<Option<usize>> = vec![None; interior.rooms.len() + 1];
    let mut seen = vec![false; interior.rooms.len() + 1];
    seen[0] = true;
    // Breadth first: a room is entered from the place nearest the hall it opens onto.
    let mut front = vec![0];
    while !front.is_empty() {
        let mut next = Vec::new();
        for &at in &front {
            for &(i, j) in &links {
                let other = if i == at {
                    j
                } else if j == at {
                    i
                } else {
                    continue;
                };
                if !seen[other] {
                    seen[other] = true;
                    from[other] = Some(at);
                    next.push(other);
                }
            }
        }
        front = next;
    }
    from
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
        let from = entered_from(interior, k);
        interior
            .rooms
            .iter()
            .enumerate()
            .all(|(i, r)| r.storey != k || from[i + 1].is_some())
    })
}
