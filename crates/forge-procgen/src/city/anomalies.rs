//! The anomalies of a generated village (the owner, 2026-10-16: "if it does not snap
//! everywhere, it means something is wrong, we need a way to detect anomalies for building and
//! probably roads, walls and other things"): the plan checked against the rules of
//! `docs/research/house-layouts.md` §8, each breach with where it stands and where to look at it
//! from, so the village's test counts them (never more than when the check came in) and the demo
//! prints a view of each (`city-plan --village 600 --anomalies`).

use glam::DVec2;

use super::facades;
use super::geom;
use super::interior::{self, Interior};
use super::mass::{Mass, Roof};
use super::roofs;
use super::rooms::RoomKind;
use super::structure::{self, OpeningKind, Structure, WallRole};
use super::village::Village;
use super::walk;
use super::walls;
use crate::sockets::{self, Extent, Kind as SocketKind, Need, Socket};

/// What is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A room reaches past its house's walls: its plan's rectangle laid on a footprint its lot
    /// cut (the owner's "a lot of mess here": beds on the neighbour's roof).
    RoomOutsideWalls,
    /// A partition meets an outside wall within a window or a door (rule 13).
    PartitionInOpening,
    /// A bedroom, living room, dining room, kitchen, study or salon with no window (rule 14).
    UnlitRoom,
    /// Two houses of a block facing each other less than 3 m apart without touching (rule 1).
    Sliver,
    /// Two houses sharing a party wall, their eaves apart by less than 0.8 m but not equal (rule
    /// 3).
    EaveStep,
    /// A garden wall off the line of every lot's and court's edge near it: a crooked piece.
    CrookedWall,
    /// A planted tree or shrub inside a house or a garden wall.
    MisplacedPlant,
    /// A garden wall's end meeting nothing (D-066's sockets): not another wall, a house, a gate's
    /// jamb nor a hedge; it stands free, a pier in a garden.
    FreeWallEnd,
    /// A house's party wall with no neighbour's back to back against it anywhere along it (D-066's
    /// sockets): a blank wall facing a gap.
    LonePartyWall,
    /// Two houses back to back on a party wall, their eaves at one height but their walls along
    /// it of different lengths (their depths): their ridges stand at two heights where a row of
    /// one height shares one roof (rule 2; the owner, 2026-10-16: "they would probably share their
    /// roof structure in the middle").
    RidgeStep,
    /// A tiled house whose footprint takes no pitched roof: the kit gives it a flat one, grey
    /// in a row of tiles, its neighbours' gables bare over it.
    FlatRoof,
    /// A house's straight flight standing off the side wall it runs along (the owner,
    /// 2026-10-16: "stairs should most likely stuck to the wall, no space between wall and
    /// stair").
    StairOffWall,
    /// A storey of a house whose street wall has [`BLANK`] or more of open front and not one
    /// opening on it (the owner, 2026-10-16: "an architect would arrange the inside of structure
    /// so that windows are on the street side instead of showing a blank wall").
    BlankStreetWall,
    /// A room no walker reaches from the front door ([`walk`]; the owner, 2026-10-17: "using the
    /// nav mesh to test navigations inside the houses, and check if there are no anomaly or
    /// obstacles").
    UnreachedRoom,
    /// A flight whose foot no walker reaches on its storey.
    UnreachedStair,
    /// An outside door of the ground floor no walker reaches from the front door, or the front
    /// door's inside blocked.
    UnreachedDoor,
    /// A door whose leaf swings into furniture or a flight (the owner, 2026-10-16: "stairs start
    /// behind the door (the door prevents access to the stairs)").
    DoorSwing,
}

impl Kind {
    /// Every kind, in order.
    pub const ALL: [Kind; 17] = [
        Kind::RoomOutsideWalls,
        Kind::PartitionInOpening,
        Kind::UnlitRoom,
        Kind::Sliver,
        Kind::EaveStep,
        Kind::CrookedWall,
        Kind::MisplacedPlant,
        Kind::FreeWallEnd,
        Kind::LonePartyWall,
        Kind::RidgeStep,
        Kind::FlatRoof,
        Kind::StairOffWall,
        Kind::BlankStreetWall,
        Kind::UnreachedRoom,
        Kind::UnreachedStair,
        Kind::UnreachedDoor,
        Kind::DoorSwing,
    ];
}

/// An anomaly.
#[derive(Clone, Debug, PartialEq)]
pub struct Anomaly {
    /// What is wrong.
    pub kind: Kind,
    /// Where, metres on the plan.
    pub at: DVec2,
    /// How high over the ground, metres.
    pub height: f64,
    /// Where to look at it from, metres on the plan.
    pub from: DVec2,
    /// What it is, for the log.
    pub note: String,
}

/// How far two houses stand apart at least when they do not touch, metres (Italy's Civil Code,
/// art. 873: "united or adjoining", or 3 m apart).
pub const GAP: f64 = 3.0;
/// The least step between two attached houses' eaves, metres: less reads as a modelling error.
pub const EAVE_STEP: f64 = 0.8;
/// The longest open street front a storey keeps without an opening, metres.
pub const BLANK: f64 = 3.0;
/// The farthest a house's flight stands off its side wall, metres.
pub const STAIR_GAP: f64 = 0.05;

/// The anomalies of village `v`, `structures` its masses' (in their order).
pub fn village(v: &Village, structures: &[Structure]) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for (i, (m, s)) in v.masses.iter().zip(structures).enumerate() {
        if let Some(inner) = interior::house(m, s) {
            rooms(i, m, s, &inner, &mut out);
        }
    }
    neighbours(v, &mut out);
    garden_walls(v, &mut out);
    plants(v, &mut out);
    unmatched(v, structures, &mut out);
    flat_roofs(v, &mut out);
    for (i, (m, s)) in v.masses.iter().zip(structures).enumerate() {
        blank_fronts(i, m, s, &mut out);
        walks(i, m, s, &mut out);
    }
    out
}

/// What a walker through house `i` does not reach ([`walk::house`]), each seen from 3 m off
/// inside, at its storey.
fn walks(i: usize, m: &Mass, s: &Structure, out: &mut Vec<Anomaly>) {
    let Some(w) = walk::house(m, s) else {
        return;
    };
    let centre = geom::centroid(&m.footprint);
    let level = |k: usize| s.storeys.get(k).map_or(0.0, |st| st.level);
    let inside = |p: DVec2| p + (centre - p).normalize_or_zero() * 3.0;
    for &(kind, k, p) in &w.rooms {
        out.push(Anomaly {
            kind: Kind::UnreachedRoom,
            at: p,
            height: level(k) + 1.5,
            from: p,
            note: format!("house {i}: its {kind:?} on storey {k} unreached from the front door"),
        });
    }
    for &(k, p) in &w.stairs {
        out.push(Anomaly {
            kind: Kind::UnreachedStair,
            at: p,
            height: level(k) + 1.5,
            from: inside(p),
            note: format!("house {i}: the flight from storey {k} unreached at its foot"),
        });
    }
    for &p in &w.doors {
        out.push(Anomaly {
            kind: Kind::UnreachedDoor,
            at: p,
            height: level(0) + 1.5,
            from: inside(p),
            note: format!("house {i}: a door unreached from the front door"),
        });
    }
    for &p in &w.swings {
        out.push(Anomaly {
            kind: Kind::DoorSwing,
            at: p,
            height: level(0) + 1.5,
            from: inside(p),
            note: format!("house {i}: a door's leaf swings into furniture or a flight"),
        });
    }
}

/// Each storey of house `i` whose street wall has [`BLANK`] or more of open front and no opening
/// on it.
fn blank_fronts(i: usize, m: &Mass, s: &Structure, out: &mut Vec<Anomaly>) {
    let inner = interior::house(m, s);
    for (w, wall) in s.walls.iter().enumerate() {
        if wall.role != WallRole::Street {
            continue;
        }
        let t = (wall.b - wall.a).normalize_or_zero();
        let outward = DVec2::new(t.y, -t.x);
        // What stands behind it on storey `k`: the rooms with a corner on its inner face, the hall.
        let on_face = |p: &DVec2| ((*p - wall.a).dot(-outward) - wall.thickness).abs() < 0.25;
        let behind = |k: usize| -> String {
            let Some(inner) = &inner else {
                return String::new();
            };
            let mut names: Vec<String> = inner
                .rooms
                .iter()
                .filter(|r| r.storey == k && r.outline.iter().filter(|p| on_face(p)).count() >= 2)
                .map(|r| format!("{:?}", r.kind))
                .collect();
            if inner.hall.iter().filter(|p| on_face(p)).count() >= 2 {
                names.push("hall".into());
            }
            format!(" behind: {}", names.join(", "))
        };
        for (k, stretches) in wall.open.iter().enumerate() {
            let open: f64 = stretches.iter().map(|&(a, b)| b - a).sum();
            let opened = s.openings.iter().any(|o| o.wall == w && o.storey == k);
            if open < BLANK || opened {
                continue;
            }
            let (a, b) = stretches
                .iter()
                .copied()
                .max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
                .unwrap_or((0.0, 0.0));
            let mid = wall.a + t * ((a + b) * 0.5);
            out.push(Anomaly {
                kind: Kind::BlankStreetWall,
                at: mid,
                height: s.storeys.get(k).map_or(0.0, |st| st.level) + 1.5,
                from: mid + outward * 10.0,
                note: format!(
                    "house {i}: storey {k} of {}, {open:.1} m of street front without an opening{}",
                    s.storeys.len(),
                    behind(k)
                ),
            });
        }
    }
}

/// The village's pieces' sockets (D-066): each house's walls as faces (its party walls needing a
/// neighbour's, the others anchors), each garden wall's two ends (needing a match) and two faces,
/// each gate's jambs and each hedge's ends. Pieces are numbered houses first, then garden walls,
/// gates and hedges.
pub fn village_sockets(v: &Village, structures: &[Structure]) -> Vec<Socket> {
    let mut out = Vec::new();
    let mut piece = 0u32;
    for (m, s) in v.masses.iter().zip(structures) {
        for w in &s.walls {
            let kind = match w.role {
                WallRole::Party => SocketKind::PartyFace,
                WallRole::Street | WallRole::Court => SocketKind::HouseFace,
                WallRole::Inner => continue,
            };
            if w.length() < 0.5 {
                continue;
            }
            let t = (w.b - w.a).normalize_or_zero();
            out.push(Socket {
                kind,
                piece,
                at: (w.a + w.b) * 0.5,
                level: 0.0,
                normal: DVec2::new(t.y, -t.x),
                extent: Extent::Rect {
                    width: w.length(),
                    height: m.height,
                },
                thickness: w.thickness,
                need: if kind == SocketKind::PartyFace {
                    Need::Required
                } else {
                    Need::Optional
                },
            });
        }
        piece += 1;
    }
    for w in &v.plan.walls {
        let r = w.ring();
        let t = (w.b - w.a).normalize_or_zero();
        for (at, normal) in [((r[0] + r[3]) * 0.5, -t), ((r[1] + r[2]) * 0.5, t)] {
            out.push(Socket {
                kind: SocketKind::GardenEnd,
                piece,
                at,
                level: 0.0,
                normal,
                extent: Extent::Point,
                thickness: walls::THICKNESS,
                need: Need::Required,
            });
        }
        for (p, q, normal) in [
            (r[0], r[1], DVec2::new(t.y, -t.x)),
            (r[3], r[2], DVec2::new(-t.y, t.x)),
        ] {
            out.push(Socket {
                kind: SocketKind::GardenFace,
                piece,
                at: (p + q) * 0.5,
                level: 0.0,
                normal,
                extent: Extent::Rect {
                    width: p.distance(q),
                    height: w.height,
                },
                thickness: walls::THICKNESS,
                need: Need::Optional,
            });
        }
        piece += 1;
    }
    for block in &v.blocks {
        // A gate's jambs: the walls' ends half a gate either side of its middle.
        for &g in &block.gates {
            out.push(Socket {
                kind: SocketKind::GateJamb,
                piece,
                at: g,
                level: 0.0,
                normal: DVec2::X,
                extent: Extent::Point,
                thickness: walls::GATE + 0.3,
                need: Need::Optional,
            });
            piece += 1;
        }
        for &(p, q) in &block.hedges {
            for (at, normal) in [
                (p, (p - q).normalize_or_zero()),
                (q, (q - p).normalize_or_zero()),
            ] {
                out.push(Socket {
                    kind: SocketKind::HedgeEnd,
                    piece,
                    at,
                    level: 0.0,
                    normal,
                    extent: Extent::Point,
                    thickness: 0.8,
                    need: Need::Optional,
                });
            }
            piece += 1;
        }
    }
    out
}

/// The village's required sockets that meet nothing: garden walls' free ends, and party walls
/// with no neighbour's back to back anywhere along them (one covered in part is a deeper house's
/// beside a shallower one's, blank over the garden as it should be).
fn unmatched(v: &Village, structures: &[Structure], out: &mut Vec<Anomaly>) {
    let sockets = village_sockets(v, structures);
    for u in sockets::match_all(&sockets, 10.0) {
        let s = u.socket;
        match s.kind {
            SocketKind::GardenEnd => out.push(Anomaly {
                kind: Kind::FreeWallEnd,
                at: s.at,
                height: 1.0,
                from: s.at + s.normal * 5.0 + s.across() * 2.0,
                note: "a garden wall's end meeting nothing".into(),
            }),
            SocketKind::PartyFace if u.covered < 0.05 => out.push(Anomaly {
                kind: Kind::LonePartyWall,
                at: s.at,
                height: 2.0,
                from: s.at + s.normal * 8.0,
                note: format!(
                    "house {}: a party wall with no neighbour against it",
                    s.piece
                ),
            }),
            _ => {}
        }
    }
    // Each pair of party walls back to back, once: one eave height, walls of different lengths.
    let party: Vec<&Socket> = sockets
        .iter()
        .filter(|s| s.kind == SocketKind::PartyFace)
        .collect();
    for (i, a) in party.iter().enumerate() {
        for b in &party[i + 1..] {
            if a.at.distance(b.at) > 30.0 || !sockets::compatible(a, b) {
                continue;
            }
            let (
                Extent::Rect {
                    width: la,
                    height: ha,
                },
                Extent::Rect {
                    width: lb,
                    height: hb,
                },
            ) = (a.extent, b.extent)
            else {
                continue;
            };
            if (ha - hb).abs() < 0.05 && (la - lb).abs() > 0.3 {
                let at = (a.at + b.at) * 0.5;
                out.push(Anomaly {
                    kind: Kind::RidgeStep,
                    at,
                    height: ha,
                    from: at + a.normal * 6.0 + a.across() * 6.0,
                    note: format!(
                        "houses {} and {}: one eave height, {la:.1} and {lb:.1} m deep",
                        a.piece, b.piece
                    ),
                });
            }
        }
    }
}

/// The tiled houses whose footprint takes no pitched roof (`roofs::takes_pitched`).
fn flat_roofs(v: &Village, out: &mut Vec<Anomaly>) {
    let edges = facades::edge_kinds(&v.plan, &v.masses);
    let tall = roofs::tall(&v.plan, &v.masses);
    for (i, m) in v.masses.iter().enumerate() {
        if m.roof == Roof::Tiled && !roofs::takes_pitched(&m.footprint, &edges[i], &tall[i]) {
            let c = geom::centroid(&m.footprint);
            out.push(Anomaly {
                kind: Kind::FlatRoof,
                at: c,
                height: m.height,
                from: c + DVec2::new(8.0, 8.0),
                note: format!(
                    "house {i}: a footprint of {} corners takes no pitched roof",
                    m.footprint.len()
                ),
            });
        }
    }
}

/// The anomalies of `kind` in `all`.
pub fn count(all: &[Anomaly], kind: Kind) -> usize {
    all.iter().filter(|a| a.kind == kind).count()
}

/// How far `p` lies from `polygon`'s outline, metres.
fn to_outline(polygon: &[DVec2], p: DVec2) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| geom::distance_to_segment(p, polygon[i], polygon[(i + 1) % n]))
        .fold(f64::MAX, f64::min)
}

/// Whether `p` lies inside `polygon` or within `slack` of its outline.
fn within(polygon: &[DVec2], p: DVec2, slack: f64) -> bool {
    geom::contains(polygon, p) || to_outline(polygon, p) < slack
}

/// House `i`'s rooms: past its walls, a partition in an opening, a habitable room unlit.
fn rooms(i: usize, m: &Mass, s: &Structure, inner: &Interior, out: &mut Vec<Anomaly>) {
    let centre = geom::centroid(&m.footprint);
    let level = |k: usize| s.storeys.get(k).map_or(0.0, |st| st.level);
    let outward = |p: DVec2, by: f64| p + (p - centre).normalize_or_zero() * by;
    for r in &inner.rooms {
        if let Some(&c) = r.outline.iter().find(|&&c| !within(&m.footprint, c, 0.05)) {
            out.push(Anomaly {
                kind: Kind::RoomOutsideWalls,
                at: c,
                height: level(r.storey) + 1.0,
                from: outward(c, 8.0),
                note: format!(
                    "house {i}: its {:?} on storey {} past its walls",
                    r.kind, r.storey
                ),
            });
        }
    }
    for (p, k) in interior::openings_hit(s, inner) {
        // Seen from inside, 3 m off, as the walker meets it.
        out.push(Anomaly {
            kind: Kind::PartitionInOpening,
            at: p,
            height: level(k) + 1.5,
            from: p + (centre - p).normalize_or_zero() * 3.0,
            note: format!("house {i}: a partition in an opening on storey {k}"),
        });
    }
    // The flight's long sides: the nearer one to the floor's edge stands on it.
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    if let (Some(st), Some(floor)) = (inner.stair, structure::floor(m, &thickness)) {
        let n = floor.len();
        let gap = |p: DVec2| {
            (0..n)
                .map(|k| geom::distance_to_segment(p, floor[k], floor[(k + 1) % n]))
                .fold(f64::MAX, f64::min)
        };
        // Each long side's nearest approach to the floor's edge, from nine points along it: a
        // flight against a slanted side wall touches it at one end, the wedge beside it unavoidable.
        let side = |a: DVec2, b: DVec2| {
            (0..=8)
                .map(|k| {
                    let p = a + (b - a) * (f64::from(k) / 8.0);
                    (gap(p), p)
                })
                .min_by(|x, y| x.0.total_cmp(&y.0))
                .unwrap_or((f64::MAX, a))
        };
        let (d, p) = [side(st[0], st[3]), side(st[1], st[2])]
            .into_iter()
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .unwrap_or((0.0, centre));
        if d > STAIR_GAP {
            out.push(Anomaly {
                kind: Kind::StairOffWall,
                at: p,
                height: level(0) + 1.5,
                from: p + (centre - p).normalize_or_zero() * 3.0,
                note: format!("house {i}: its flight {d:.2} m off its side wall"),
            });
        }
    }
    // Each window's middle on its wall's inner face, with its storey.
    let windows: Vec<(DVec2, usize)> = s
        .openings
        .iter()
        .filter(|o| matches!(o.kind, OpeningKind::Window | OpeningKind::Shopfront))
        .map(|o| {
            let w = &s.walls[o.wall];
            let t = (w.b - w.a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x);
            let (lo, hi) = o.span();
            (w.a + t * ((lo + hi) * 0.5) + inward * w.thickness, o.storey)
        })
        .collect();
    for r in &inner.rooms {
        let habitable = matches!(
            r.kind,
            RoomKind::Living
                | RoomKind::Kitchen
                | RoomKind::Dining
                | RoomKind::Bedroom
                | RoomKind::Study
                | RoomKind::Salon
        );
        let lit = windows
            .iter()
            .any(|&(p, k)| k == r.storey && within(&r.outline, p, 0.4));
        if habitable && !lit {
            let middle = geom::centroid(&r.outline);
            out.push(Anomaly {
                kind: Kind::UnlitRoom,
                at: middle,
                height: level(r.storey) + 1.5,
                from: middle,
                note: format!(
                    "house {i}: its {:?} on storey {} has no window",
                    r.kind, r.storey
                ),
            });
        }
    }
}

/// Each block's houses two by two: facing each other less than [`GAP`] apart without touching, or
/// touching with their eaves less than [`EAVE_STEP`] apart but not equal.
fn neighbours(v: &Village, out: &mut Vec<Anomaly>) {
    let mut first = 0;
    for block in &v.blocks {
        let range = first..first + block.houses.len();
        first = range.end;
        let masses = &v.masses[range.clone()];
        for (a, ma) in masses.iter().enumerate() {
            for (b, mb) in masses.iter().enumerate().skip(a + 1) {
                let (i, j) = (range.start + a, range.start + b);
                let (fa, fb) = (&ma.footprint, &mb.footprint);
                // Along each of `fa`'s edges at five points: how far out `fb` stands straight
                // ahead (within 20° of the edge's outward normal).
                let n = fa.len();
                let mut gap: Option<(DVec2, DVec2, DVec2)> = None;
                let mut touching: Option<DVec2> = None;
                for e in 0..n {
                    let (p, q) = (fa[e], fa[(e + 1) % n]);
                    let t = (q - p).normalize_or_zero();
                    let out_dir = DVec2::new(t.y, -t.x);
                    for s in [0.1, 0.3, 0.5, 0.7, 0.9] {
                        let at = p + (q - p) * s;
                        let ahead = at + out_dir * GAP;
                        let hit = (0..fb.len())
                            .filter_map(|k| {
                                geom::segment_intersection(at, ahead, fb[k], fb[(k + 1) % fb.len()])
                            })
                            .map(|(u, _)| u * GAP)
                            .fold(f64::MAX, f64::min);
                        if hit < 0.05 || to_outline(fb, at) < 0.05 {
                            touching.get_or_insert(at);
                        } else if hit < GAP && gap.is_none() {
                            gap = Some((at, at + out_dir * hit, t));
                        }
                    }
                }
                if let Some((p, q, along)) = gap {
                    let mid = (p + q) * 0.5;
                    // Looking along the gap from whichever end is clear of both houses.
                    let from = [mid + along * 8.0, mid - along * 8.0]
                        .into_iter()
                        .find(|&f| !geom::contains(fa, f) && !geom::contains(fb, f))
                        .unwrap_or(mid + along * 8.0);
                    out.push(Anomaly {
                        kind: Kind::Sliver,
                        at: mid,
                        height: 1.5,
                        from,
                        note: format!(
                            "houses {i} and {j}: {:.2} m apart, neither touching nor {GAP} m apart",
                            p.distance(q)
                        ),
                    });
                }
                let step = (ma.height - mb.height).abs();
                if let Some(at) = touching.filter(|_| step > 0.05 && step < EAVE_STEP) {
                    out.push(Anomaly {
                        kind: Kind::EaveStep,
                        at,
                        height: ma.height.min(mb.height),
                        from: at + (at - geom::centroid(fa)).normalize_or_zero() * 10.0,
                        note: format!("houses {i} and {j}: eaves {step:.2} m apart"),
                    });
                }
            }
        }
    }
}

/// The garden walls off every lot's and court's edge near them (within 6° and a metre).
fn garden_walls(v: &Village, out: &mut Vec<Anomaly>) {
    let mut edges: Vec<(DVec2, DVec2)> = Vec::new();
    for block in &v.blocks {
        for ring in block.lots.iter().chain(std::iter::once(&block.court)) {
            let n = ring.len();
            edges.extend((0..n).map(|k| (ring[k], ring[(k + 1) % n])));
        }
    }
    for w in &v.plan.walls {
        if w.length() < 0.3 {
            continue;
        }
        let t = (w.b - w.a).normalize_or_zero();
        let mid = (w.a + w.b) * 0.5;
        let on_line = edges.iter().any(|&(p, q)| {
            let d = (q - p).normalize_or_zero();
            geom::distance_to_segment(mid, p, q) < 1.0 && t.perp_dot(d).abs() < 0.105
        });
        if !on_line {
            out.push(Anomaly {
                kind: Kind::CrookedWall,
                at: mid,
                height: w.height,
                from: mid + DVec2::new(-t.y, t.x) * 6.0,
                note: format!(
                    "a garden wall {:.2} m long off every lot line near it",
                    w.length()
                ),
            });
        }
    }
}

/// The planted trees and shrubs inside a house or a garden wall.
fn plants(v: &Village, out: &mut Vec<Anomaly>) {
    let rings: Vec<Vec<DVec2>> = v.plan.walls.iter().map(|w| w.ring()).collect();
    for t in &v.plan.planted {
        let housed = v.masses.iter().any(|m| geom::contains(&m.footprint, t.pos));
        let walled = rings.iter().any(|r| geom::contains(r, t.pos));
        if housed || walled {
            out.push(Anomaly {
                kind: Kind::MisplacedPlant,
                at: t.pos,
                height: t.height * 0.5,
                from: t.pos + DVec2::new(5.0, 5.0),
                note: format!(
                    "a {:.1} m plant {}",
                    t.height,
                    if housed { "in a house" } else { "in a wall" }
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{example, structure, village};
    use super::*;

    /// The 600 m village's anomalies, by kind, never more than when the check came in
    /// (2026-10-16): each fix lowers its bound.
    #[test]
    fn the_villages_anomalies_never_grow() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village::village(&site, &village::params(size), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        let all = village(&v, &structures);
        let bounds: [(Kind, usize); 17] = [
            (Kind::RoomOutsideWalls, 0),
            (Kind::PartitionInOpening, 0),
            (Kind::UnlitRoom, 20),
            (Kind::Sliver, 0),
            (Kind::EaveStep, 0),
            (Kind::CrookedWall, 0),
            (Kind::MisplacedPlant, 0),
            (Kind::FreeWallEnd, 4),
            (Kind::LonePartyWall, 0),
            (Kind::RidgeStep, 0),
            (Kind::FlatRoof, 0),
            (Kind::StairOffWall, 0),
            (Kind::BlankStreetWall, 3),
            (Kind::UnreachedRoom, 0),
            (Kind::UnreachedStair, 0),
            (Kind::UnreachedDoor, 0),
            (Kind::DoorSwing, 0),
        ];
        let mut over = Vec::new();
        for (kind, bound) in bounds {
            let n = count(&all, kind);
            println!("{kind:?}: {n} ({bound} at most)");
            if n > bound {
                over.push(format!("{n} {kind:?}, {bound} at most"));
            }
        }
        assert!(over.is_empty(), "{over:?}");
    }
}
