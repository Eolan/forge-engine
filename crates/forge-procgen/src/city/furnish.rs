//! A house's rooms furnished by what they are for (D-062, #266: the first map's props). Each
//! room takes its pieces in order — a bedroom its bed and wardrobe, a living room its sofa and
//! low table, a kitchen its counter and table with chairs, a bathroom its bath, basin and
//! toilet — each against a wall facing into the room, clear of the doorways and of the openings
//! down to the floor, the tall ones clear of the windows, none over another; a piece that finds
//! no place is left out. The measures are a room's usual furniture's (a double bed 1.60 by
//! 2.05 m, a kitchen's worktop 0.62 m deep).

use glam::DVec2;

use super::geom;
use super::interior::{DOORWAY, Interior};
use super::rooms::RoomKind;
use super::structure::{Structure, WallRole};

/// A piece of furniture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Piece {
    /// A double bed, its head to the wall.
    Bed,
    /// A single bed.
    SingleBed,
    /// A wardrobe.
    Wardrobe,
    /// A sofa for three.
    Sofa,
    /// A low table before a sofa.
    LowTable,
    /// A kitchen's worktop on its cupboards, its sink and its hob.
    Counter,
    /// A table for four.
    Table,
    /// A chair.
    Chair,
    /// A bath.
    Bath,
    /// A wash basin on its pedestal.
    Basin,
    /// A toilet.
    Toilet,
    /// A writing desk.
    Desk,
    /// Open shelves.
    Shelves,
}

impl Piece {
    /// Every piece.
    pub const ALL: [Piece; 13] = [
        Piece::Bed,
        Piece::SingleBed,
        Piece::Wardrobe,
        Piece::Sofa,
        Piece::LowTable,
        Piece::Counter,
        Piece::Table,
        Piece::Chair,
        Piece::Bath,
        Piece::Basin,
        Piece::Toilet,
        Piece::Desk,
        Piece::Shelves,
    ];

    /// Its footprint: its width along its back and its depth out from it, metres.
    pub fn size(self) -> (f64, f64) {
        match self {
            Piece::Bed => (1.6, 2.05),
            Piece::SingleBed => (0.95, 2.05),
            Piece::Wardrobe => (1.2, 0.62),
            Piece::Sofa => (2.0, 0.9),
            Piece::LowTable => (1.0, 0.55),
            Piece::Counter => (2.4, 0.62),
            Piece::Table => (1.4, 0.85),
            Piece::Chair => (0.45, 0.5),
            Piece::Bath => (1.7, 0.72),
            Piece::Basin => (0.6, 0.48),
            Piece::Toilet => (0.4, 0.68),
            Piece::Desk => (1.2, 0.62),
            Piece::Shelves => (1.0, 0.36),
        }
    }

    /// Its height, metres.
    pub fn height(self) -> f64 {
        match self {
            Piece::Bed | Piece::SingleBed => 0.55,
            Piece::Wardrobe => 2.0,
            Piece::Sofa => 0.85,
            Piece::LowTable => 0.42,
            Piece::Counter => 0.9,
            Piece::Table | Piece::Desk => 0.75,
            Piece::Chair => 0.9,
            Piece::Bath => 0.58,
            Piece::Basin => 0.85,
            Piece::Toilet => 0.75,
            Piece::Shelves => 1.8,
        }
    }

    /// Whether it stands higher than a window's sill: kept clear of the windows.
    fn tall(self) -> bool {
        self.height() > 0.95
    }
}

/// A piece placed in a house: what it is, its storey, its footprint's middle on the floor and
/// the way it faces (its front, away from its back), a unit vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// What it is.
    pub piece: Piece,
    /// Its storey, 0 the ground floor.
    pub storey: usize,
    /// Its footprint's middle, metres on the plan.
    pub centre: DVec2,
    /// The way it faces.
    pub facing: DVec2,
}

impl Placed {
    /// Its footprint, anticlockwise.
    pub fn outline(&self) -> [DVec2; 4] {
        let (w, d) = self.piece.size();
        let side = DVec2::new(self.facing.y, -self.facing.x) * (w * 0.5);
        let front = self.facing * (d * 0.5);
        let c = self.centre;
        [
            c - side - front,
            c + side - front,
            c + side + front,
            c - side + front,
        ]
    }
}

/// A rectangle in a room's frame: `x` along its first edge, `y` into it.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: (f64, f64),
    y: (f64, f64),
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.x.0 < o.x.1 - 1e-6
            && o.x.0 < self.x.1 - 1e-6
            && self.y.0 < o.y.1 - 1e-6
            && o.y.0 < self.y.1 - 1e-6
    }
}

/// The pieces a room of `kind` (`area` m²) wants, in order; with `cooks`, the room that keeps the
/// kitchen's where the house has no kitchen of its own, its counter first.
fn wants(kind: RoomKind, area: f64, cooks: bool) -> Vec<Piece> {
    match kind {
        RoomKind::Bedroom if area >= 11.0 => vec![Piece::Bed, Piece::Wardrobe, Piece::Desk],
        RoomKind::Bedroom => vec![Piece::SingleBed, Piece::Wardrobe],
        RoomKind::Living if cooks => {
            vec![Piece::Counter, Piece::Sofa, Piece::Table, Piece::Shelves]
        }
        RoomKind::Dining if cooks => vec![Piece::Counter, Piece::Table, Piece::Shelves],
        RoomKind::Living => vec![Piece::Sofa, Piece::Shelves, Piece::Table],
        RoomKind::Kitchen => vec![Piece::Counter, Piece::Table],
        RoomKind::Dining => vec![Piece::Table, Piece::Shelves],
        RoomKind::Bathroom => vec![Piece::Bath, Piece::Basin, Piece::Toilet],
        RoomKind::Study => vec![Piece::Desk, Piece::Shelves, Piece::Shelves],
        RoomKind::Salon => vec![Piece::Sofa, Piece::Sofa, Piece::Shelves],
        RoomKind::Service | RoomKind::Store => vec![Piece::Shelves, Piece::Shelves],
    }
}

/// The furniture of the house `interior` lays inside the structure `s`: each room's pieces
/// ([`wants`]) against its walls, with the chairs at the tables and desks and a low table before
/// each sofa.
pub fn furnish(s: &Structure, interior: &Interior) -> Vec<Placed> {
    // Where the house has no kitchen of its own, the room that keeps the kitchen's counter: the
    // dining room (a kitchen-diner, `rooms::rebalance`), else the living room.
    let has = |kind: RoomKind| interior.rooms.iter().any(|r| r.kind == kind);
    let (kitchen, dining) = (has(RoomKind::Kitchen), has(RoomKind::Dining));
    let cooks = |kind: RoomKind| {
        !kitchen
            && match kind {
                RoomKind::Dining => true,
                RoomKind::Living => !dining,
                _ => false,
            }
    };
    let mut out = Vec::new();
    for room in &interior.rooms {
        // The room's rectangle every depth of it shares: a room against a slanted side wall
        // (`interior`: a house on its lot's side lines) is a trapezoid, and its pieces keep to
        // the part square to the front.
        let r = room.outline;
        let ex = (r[1] - r[0]).normalize_or_zero();
        let ey = DVec2::new(-ex.y, ex.x);
        let u = |p: DVec2| (p - r[0]).dot(ex);
        let (u0, u1) = (u(r[0]).max(u(r[3])), u(r[1]).min(u(r[2])));
        let d = (r[3] - r[0]).dot(ey);
        let o0 = r[0] + ex * u0;
        let (w, d) = ((u1 - u0).max(0.0), d.max(0.0));
        if w < 1.2 || d < 1.2 {
            continue;
        }
        let local = |p: DVec2| ((p - o0).dot(ex), (p - o0).dot(ey));
        let world = |x: f64, y: f64| o0 + ex * x + ey * y;
        // What must stay clear: before each doorway into the room and each opening to the floor,
        // 1.1 m deep; the windows along the walls, for the tall pieces.
        let mut keep: Vec<Rect> = Vec::new();
        let mut windows: Vec<Rect> = Vec::new();
        let near_edge = |x: f64, y: f64| -> Option<usize> {
            let t = 0.35;
            if y.abs() < t {
                Some(0)
            } else if (x - w).abs() < t {
                Some(1)
            } else if (y - d).abs() < t {
                Some(2)
            } else if x.abs() < t {
                Some(3)
            } else {
                None
            }
        };
        // A stretch from `a` to `b` along edge `e`, reaching `deep` into the room.
        let zone = |e: usize, a: f64, b: f64, deep: f64| -> Rect {
            let (lo, hi) = (a.min(b), a.max(b));
            match e {
                0 => Rect {
                    x: (lo, hi),
                    y: (0.0, deep),
                },
                1 => Rect {
                    x: (w - deep, w),
                    y: (lo, hi),
                },
                2 => Rect {
                    x: (lo, hi),
                    y: (d - deep, d),
                },
                _ => Rect {
                    x: (0.0, deep),
                    y: (lo, hi),
                },
            }
        };
        let along = |e: usize, x: f64, y: f64| if e.is_multiple_of(2) { x } else { y };
        for p in interior
            .partitions
            .iter()
            .filter(|p| p.storey == room.storey)
        {
            let t = (p.b - p.a).normalize_or_zero();
            for &(d0, d1) in &p.doors {
                let (pa, pb) = (p.a + t * d0, p.a + t * d1);
                let (ax, ay) = local(pa);
                let (bx, by) = local(pb);
                let (mx, my) = ((ax + bx) * 0.5, (ay + by) * 0.5);
                if mx < -0.4 || mx > w + 0.4 || my < -0.4 || my > d + 0.4 {
                    continue;
                }
                if let Some(e) = near_edge(mx, my) {
                    let (a, b) = (along(e, ax, ay), along(e, bx, by));
                    keep.push(zone(e, a - 0.15, b + 0.15, DOORWAY + 0.2));
                }
            }
        }
        for op in s.openings.iter().filter(|op| op.storey == room.storey) {
            let wl = &s.walls[op.wall];
            if !matches!(wl.role, WallRole::Street | WallRole::Court) {
                continue;
            }
            let t = (wl.b - wl.a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x) * wl.thickness;
            let (pa, pb) = (
                wl.a + t * op.span().0 + inward,
                wl.a + t * op.span().1 + inward,
            );
            let (ax, ay) = local(pa);
            let (bx, by) = local(pb);
            let (mx, my) = ((ax + bx) * 0.5, (ay + by) * 0.5);
            let Some(e) = near_edge(mx, my) else {
                continue;
            };
            let (a, b) = (along(e, ax, ay), along(e, bx, by));
            if a.max(b) < 0.0 || a.min(b) > if e.is_multiple_of(2) { w } else { d } {
                continue;
            }
            if op.sill < 0.3 {
                keep.push(zone(e, a - 0.1, b + 0.1, 1.1));
            } else {
                windows.push(zone(e, a - 0.1, b + 0.1, 0.8));
            }
        }
        let mut placed: Vec<(Rect, Piece)> = Vec::new();
        // Whether a piece fits at `r`: inside the room, clear of what must stay clear and of the
        // pieces placed (and of the passage left before each).
        let fits = |r: &Rect, piece: Piece, placed: &[(Rect, Piece)], keep: &[Rect]| {
            r.x.0 >= 0.02
                && r.y.0 >= 0.02
                && r.x.1 <= w - 0.02
                && r.y.1 <= d - 0.02
                && keep.iter().all(|k| !k.overlaps(r))
                && (!piece.tall() || windows.iter().all(|k| !k.overlaps(r)))
                && placed.iter().all(|(q, _)| !q.overlaps(r))
        };
        // A piece against edge `e` at `s` along it: its rectangle and its centre and facing; a
        // bed `sideways` lies along the wall, its head towards the nearer corner.
        let against = |piece: Piece, e: usize, s: f64, sideways: bool| -> (Rect, DVec2, DVec2) {
            let (pw, pd) = piece.size();
            let len = if e.is_multiple_of(2) { w } else { d };
            let (along_w, out_d) = if sideways { (pd, pw) } else { (pw, pd) };
            let (lo, hi) = (s - along_w * 0.5, s + along_w * 0.5);
            let (r, inward, dir) = match e {
                0 => (
                    Rect {
                        x: (lo, hi),
                        y: (0.02, 0.02 + out_d),
                    },
                    ey,
                    ex,
                ),
                1 => (
                    Rect {
                        x: (w - 0.02 - out_d, w - 0.02),
                        y: (lo, hi),
                    },
                    -ex,
                    ey,
                ),
                2 => (
                    Rect {
                        x: (lo, hi),
                        y: (d - 0.02 - out_d, d - 0.02),
                    },
                    -ey,
                    ex,
                ),
                _ => (
                    Rect {
                        x: (0.02, 0.02 + out_d),
                        y: (lo, hi),
                    },
                    ex,
                    ey,
                ),
            };
            let facing = if !sideways {
                inward
            } else if s < len * 0.5 {
                dir
            } else {
                -dir
            };
            let centre = world((r.x.0 + r.x.1) * 0.5, (r.y.0 + r.y.1) * 0.5);
            (r, centre, facing)
        };
        // The rectangle of a piece of `size` centred at room point (cx, cy), facing along an
        // axis (`across`: its width along the room's y).
        let rect_at = |cx: f64, cy: f64, size: (f64, f64), across: bool| {
            let (hw, hd) = if across {
                (size.1 * 0.5, size.0 * 0.5)
            } else {
                (size.0 * 0.5, size.1 * 0.5)
            };
            Rect {
                x: (cx - hw, cx + hw),
                y: (cy - hd, cy + hd),
            }
        };
        for wanted in wants(room.kind, w * d, cooks(room.kind)) {
            // A double bed head to the wall, then along it, then a single bed.
            let variants: Vec<(Piece, bool)> = match wanted {
                Piece::Bed => vec![
                    (Piece::Bed, false),
                    (Piece::Bed, true),
                    (Piece::SingleBed, false),
                    (Piece::SingleBed, true),
                ],
                Piece::SingleBed => vec![(Piece::SingleBed, false), (Piece::SingleBed, true)],
                p => vec![(p, false)],
            };
            // Each edge, the longer first, and along it from the middle outwards (a bed along a
            // wall from the corners inwards).
            let mut edges = [0usize, 1, 2, 3];
            edges.sort_by(|&a, &b| {
                let len = |e: usize| if e.is_multiple_of(2) { w } else { d };
                len(b).total_cmp(&len(a))
            });
            let mut found = None;
            'search: for &(piece, sideways) in &variants {
                let (pw, pd) = piece.size();
                let along_w = if sideways { pd } else { pw };
                for &e in &edges {
                    let len = if e.is_multiple_of(2) { w } else { d };
                    if len < along_w + 0.04 {
                        continue;
                    }
                    let steps = ((len - along_w) / 0.1).floor() as i64;
                    let middle = len * 0.5;
                    let mut tries: Vec<f64> = (0..=steps)
                        .map(|k| along_w * 0.5 + 0.02 + k as f64 * 0.1)
                        .filter(|&s| s + along_w * 0.5 <= len - 0.02)
                        .collect();
                    if sideways {
                        tries.sort_by(|a, b| (b - middle).abs().total_cmp(&(a - middle).abs()));
                    } else {
                        tries.sort_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()));
                    }
                    for s in tries {
                        let (r, centre, facing) = against(piece, e, s, sideways);
                        // A passage of 0.6 m kept before every piece but the low ones.
                        let pad = if piece.height() > 0.6 { 0.6 } else { 0.0 };
                        let front = match e {
                            0 => Rect {
                                x: r.x,
                                y: (r.y.1, r.y.1 + pad),
                            },
                            1 => Rect {
                                x: (r.x.0 - pad, r.x.0),
                                y: r.y,
                            },
                            2 => Rect {
                                x: r.x,
                                y: (r.y.0 - pad, r.y.0),
                            },
                            _ => Rect {
                                x: (r.x.1, r.x.1 + pad),
                                y: r.y,
                            },
                        };
                        if fits(&r, piece, &placed, &keep)
                            && placed.iter().all(|(q, _)| !q.overlaps(&front))
                        {
                            found = Some((r, centre, facing, e, piece));
                            break 'search;
                        }
                    }
                }
            }
            let Some((r, centre, facing, e, piece)) = found else {
                continue;
            };
            placed.push((r, piece));
            out.push(Placed {
                piece,
                storey: room.storey,
                centre,
                facing,
            });
            // What goes with it: the chairs at a table or a desk, a low table before a sofa.
            let (cx, cy) = ((r.x.0 + r.x.1) * 0.5, (r.y.0 + r.y.1) * 0.5);
            let (fx, fy) = (facing.dot(ex), facing.dot(ey));
            let across = !e.is_multiple_of(2);
            let (pw, pd) = piece.size();
            let mut companions: Vec<(Piece, f64, f64, DVec2)> = Vec::new();
            match piece {
                Piece::Table => {
                    // Two chairs before it, facing it.
                    let side = if across { ey } else { ex };
                    for k in [-0.25, 0.25] {
                        let off = pd * 0.5 + 0.3;
                        let (sx, sy) = (side.dot(ex) * k * pw, side.dot(ey) * k * pw);
                        companions.push((
                            Piece::Chair,
                            cx + fx * off + sx,
                            cy + fy * off + sy,
                            -facing,
                        ));
                    }
                }
                Piece::Desk => {
                    let off = pd * 0.5 + 0.3;
                    companions.push((Piece::Chair, cx + fx * off, cy + fy * off, -facing));
                }
                Piece::Sofa => {
                    let off = pd * 0.5 + 0.45 + Piece::LowTable.size().1 * 0.5;
                    companions.push((Piece::LowTable, cx + fx * off, cy + fy * off, facing));
                }
                _ => {}
            }
            for (c, x, y, f) in companions {
                let r = rect_at(x, y, c.size(), across);
                if fits(&r, c, &placed, &keep) {
                    placed.push((r, c));
                    out.push(Placed {
                        piece: c,
                        storey: room.storey,
                        centre: world(x, y),
                        facing: f,
                    });
                }
            }
        }
    }
    out
}

/// What a home's lamp is (D-065 step 2, #270).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LampKind {
    /// A shade hanging from a room's ceiling over its middle.
    Pendant,
    /// A light flush with the hall's ceiling, one a storey, clear of the flight.
    Ceiling,
    /// A shade on the wall over a bed's head or a sofa's back, to read by.
    Sconce,
}

/// A home's lamp: what it is, where its bulb is, how bright and how warm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HomeLamp {
    /// What it is.
    pub kind: LampKind,
    /// Its storey, 0 the ground floor.
    pub storey: usize,
    /// Its bulb's middle, metres on the plan.
    pub at: DVec2,
    /// Its bulb's height over its storey's floor, metres.
    pub height: f64,
    /// Its globe's radius, metres: the frosted glass round its bulb that glows.
    pub radius: f64,
    /// A sconce's way out from its wall, a unit vector; zero for the others.
    pub facing: DVec2,
    /// Its room, an index into the interior's rooms; `None` for the hall.
    pub room: Option<usize>,
    /// Its luminous intensity, candelas, the same every way (its lumens over 4π).
    pub candela: f64,
    /// Its colour temperature, kelvin.
    pub kelvin: f64,
}

/// A pendant's drop from the ceiling to its bulb, metres, at most.
const PENDANT_DROP: f64 = 0.5;
/// The lowest a pendant's bulb hangs, metres: its globe over a doorway's head (2.1 m).
const PENDANT_LOWEST: f64 = 2.25;
/// A pendant's and the hall's light's globe, metres in radius; the hall's hugs its ceiling.
pub const GLOBE: f64 = 0.09;
/// A sconce's globe, metres in radius, its middle 10 cm out from its wall.
pub const SCONCE_GLOBE: f64 = 0.06;

/// The lamps of the house `interior` in the structure `s` among its furniture `pieces` (as
/// `walk::furnished` leaves them):
/// - a pendant over each room's middle, as bright as the room is large (70 lm a square metre,
///   from 450 to 1 600 lm, a third more and whiter in a kitchen or a bathroom);
/// - a light on the hall's ceiling on each storey with rooms, clear of the flight;
/// - a sconce over each bed's head and each sofa's back that stands against a wall.
pub fn lamps(s: &Structure, interior: &Interior, pieces: &[Placed]) -> Vec<HomeLamp> {
    let candela = |lumens: f64| lumens / (4.0 * std::f64::consts::PI);
    let clear = |k: usize| s.storeys.get(k).map_or(2.6, |st| st.clear());
    let room_of = |k: usize, p: DVec2| {
        interior
            .rooms
            .iter()
            .position(|r| r.storey == k && geom::contains(&r.outline, p))
    };
    let mut out = Vec::new();
    for (i, r) in interior.rooms.iter().enumerate() {
        let area = geom::signed_area(&r.outline).abs();
        let (lumens, kelvin) = match r.kind {
            RoomKind::Kitchen | RoomKind::Bathroom => ((93.0 * area).clamp(600.0, 2100.0), 3000.0),
            _ => ((70.0 * area).clamp(450.0, 1600.0), 2700.0),
        };
        let c = clear(r.storey);
        out.push(HomeLamp {
            kind: LampKind::Pendant,
            storey: r.storey,
            at: geom::centroid(&r.outline),
            height: (c - PENDANT_DROP).max(PENDANT_LOWEST).min(c - GLOBE - 0.03),
            radius: GLOBE,
            facing: DVec2::ZERO,
            room: Some(i),
            candela: candela(lumens),
            kelvin,
        });
    }
    // The hall's light on each storey with rooms: along the hall from its first edge's middle to
    // its third's, where it stands farthest from the flight's box.
    let hall = interior.hall;
    let (front, back) = ((hall[0] + hall[1]) * 0.5, (hall[2] + hall[3]) * 0.5);
    let clearance = |p: DVec2| match interior.stair {
        Some(b) if geom::contains(&b, p) => -1.0,
        Some(b) => (0..4)
            .map(|k| geom::distance_to_segment(p, b[k], b[(k + 1) % 4]))
            .fold(f64::MAX, f64::min),
        None => (p - (front + back) * 0.5).length() * -1e-3,
    };
    let at = (3..=17)
        .map(|t| front.lerp(back, t as f64 * 0.05))
        .fold((DVec2::ZERO, f64::MIN), |best, p| {
            let c = clearance(p);
            if c > best.1 { (p, c) } else { best }
        })
        .0;
    let mut storeys: Vec<usize> = interior.rooms.iter().map(|r| r.storey).collect();
    storeys.sort_unstable();
    storeys.dedup();
    for k in storeys {
        out.push(HomeLamp {
            kind: LampKind::Ceiling,
            storey: k,
            at,
            height: clear(k) - GLOBE - 0.03,
            radius: GLOBE,
            facing: DVec2::ZERO,
            room: None,
            candela: candela(500.0),
            kelvin: 3000.0,
        });
    }
    // The sconces: on the wall the piece's back stands against (2 cm off its room's outline),
    // 10 cm out from it, over its middle.
    for p in pieces {
        let (lumens, kelvin, height) = match p.piece {
            Piece::Bed | Piece::SingleBed => (250.0, 2400.0, 1.15),
            Piece::Sofa => (400.0, 2700.0, 1.45),
            _ => continue,
        };
        let Some(i) = room_of(p.storey, p.centre) else {
            continue;
        };
        let outline = &interior.rooms[i].outline;
        let back = p.centre - p.facing * (p.piece.size().1 * 0.5);
        let to_wall = (0..4)
            .map(|k| geom::distance_to_segment(back, outline[k], outline[(k + 1) % 4]))
            .fold(f64::MAX, f64::min);
        if to_wall > 0.08 {
            continue;
        }
        out.push(HomeLamp {
            kind: LampKind::Sconce,
            storey: p.storey,
            at: back + p.facing * 0.1,
            height,
            radius: SCONCE_GLOBE,
            facing: p.facing,
            room: Some(i),
            candela: candela(lumens),
            kelvin,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{example, geom, interior, structure, village};

    #[test]
    fn the_villages_rooms_are_furnished_clear_of_their_doorways() {
        let v = village::village(&example::site(600.0, 10.0), &village::params(600.0), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        let mut count: std::collections::BTreeMap<String, usize> = Default::default();
        let (mut bedrooms, mut beds, mut baths, mut toilets) = (0, 0, 0, 0);
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let pieces = furnish(s, &inner);
            for p in &pieces {
                *count.entry(format!("{:?}", p.piece)).or_default() += 1;
            }
            // Each piece within a room of its storey, none over another.
            for (i, p) in pieces.iter().enumerate() {
                let shape = p.outline();
                let middle = p.centre;
                assert!(
                    inner
                        .rooms
                        .iter()
                        .any(|r| r.storey == p.storey && geom::contains(&r.outline, middle)),
                    "a {:?} outside the rooms",
                    p.piece
                );
                for q in &pieces[i + 1..] {
                    if q.storey != p.storey {
                        continue;
                    }
                    let mut part = shape.to_vec();
                    let other = q.outline();
                    for k in 0..4 {
                        part =
                            geom::clip_half_plane(&part, other[k], other[(k + 1) % 4] - other[k]);
                    }
                    let shared = if part.len() < 3 {
                        0.0
                    } else {
                        geom::signed_area(&part).abs()
                    };
                    assert!(shared < 1e-3, "a {:?} over a {:?}", p.piece, q.piece);
                }
                // None in a doorway.
                for part in inner.partitions.iter().filter(|w| w.storey == p.storey) {
                    let t = (part.b - part.a).normalize_or_zero();
                    for &(d0, d1) in &part.doors {
                        let mid = part.a + t * ((d0 + d1) * 0.5);
                        assert!(!geom::contains(&shape, mid), "a {:?} in a doorway", p.piece);
                    }
                }
            }
            for r in &inner.rooms {
                let has = |k: Piece| {
                    pieces.iter().any(|p| {
                        p.piece == k && p.storey == r.storey && geom::contains(&r.outline, p.centre)
                    })
                };
                if r.kind == RoomKind::Bedroom {
                    bedrooms += 1;
                    beds += usize::from(has(Piece::Bed) || has(Piece::SingleBed));
                }
                if r.kind == RoomKind::Bathroom {
                    baths += 1;
                    toilets += usize::from(has(Piece::Toilet));
                }
            }
        }
        println!("furniture: {count:?}");
        println!(
            "{beds} of {bedrooms} bedrooms with a bed, {toilets} of {baths} bathrooms with a toilet"
        );
        assert!(
            beds * 10 >= bedrooms * 9,
            "{beds} beds for {bedrooms} bedrooms"
        );
        assert!(
            toilets * 10 >= baths * 9,
            "{toilets} toilets for {baths} bathrooms"
        );
    }

    #[test]
    fn the_village_homes_lamps_hang_in_their_rooms_over_heads_and_clear_of_the_flight() {
        let v = village::village(&example::site(600.0, 10.0), &village::params(600.0), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        let (mut rooms, mut pendants, mut halls, mut sconces, mut beds) = (0, 0, 0, 0, 0);
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let pieces = crate::city::walk::furnished(m, s, &inner);
            beds += pieces
                .iter()
                .filter(|p| matches!(p.piece, Piece::Bed | Piece::SingleBed))
                .count();
            rooms += inner.rooms.len();
            for l in lamps(s, &inner, &pieces) {
                let clear = s.storeys[l.storey].clear();
                assert!(
                    l.height + l.radius < clear - 0.02,
                    "a {:?} in the ceiling",
                    l.kind
                );
                assert!(
                    l.candela > 15.0 && l.candela < 200.0,
                    "a {:?} of {} cd",
                    l.kind,
                    l.candela
                );
                match l.room {
                    Some(i) => {
                        let r = &inner.rooms[i];
                        assert_eq!(r.storey, l.storey);
                        assert!(
                            geom::contains(&r.outline, l.at),
                            "a {:?} out of its room",
                            l.kind
                        );
                    }
                    None => {
                        assert!(
                            geom::contains(&inner.hall, l.at),
                            "the hall's light out of it"
                        );
                        if let Some(b) = inner.stair {
                            assert!(
                                !geom::contains(&b, l.at),
                                "the hall's light over the flight"
                            );
                        }
                    }
                }
                match l.kind {
                    LampKind::Pendant => {
                        pendants += 1;
                        // Over a walker's head and a doorway's, unless the ceiling is lower.
                        assert!(l.height >= PENDANT_LOWEST.min(clear - 0.1) - 1e-9);
                    }
                    LampKind::Ceiling => halls += 1,
                    LampKind::Sconce => sconces += 1,
                }
            }
        }
        println!("{pendants} pendants, {halls} hall lights, {sconces} sconces ({beds} beds)");
        assert_eq!(pendants, rooms, "a pendant a room");
        assert!(halls > 0);
        assert!(sconces * 2 >= beds, "{sconces} sconces for {beds} beds");
    }
}
