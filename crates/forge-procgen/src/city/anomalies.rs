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
    /// An opening its module does not fill (D-066's holes and fills): its stretch, its sill or its
    /// head 2 cm off the structure's, or no module in it but at a passage. The insides open the
    /// wall round the structure's opening and the probes' regions take its reveal, so a misfit
    /// shows (a balcony's door was walled up to its window's sill from inside).
    OpeningMisfit,
    /// An outside door whose threshold, the metre before it across its width, meets a garden
    /// wall, a standing piece of the gardens (a hedge, a planter, a bed, a bench, a bike rack)
    /// or another house (D-066's threshold socket: a door meets walkable ground outside). A
    /// garden wall ran half a metre before a court door, the path to it ending at the wall.
    DoorBlocked,
    /// Two garden walls ending together at a corner sharper than [`SHARP_CORNER`] degrees: a V in a
    /// garden, its point an angle no one can mow or walk into, where a lot's side runs aslant
    /// (the owner, 2026-10-29: "walls issue"; the owner's rule of 2026-10-16: no diagonal lots).
    SharpWallCorner,
    /// A bedroom with no doorway of its own onto the hall or a landing (`RoomKind::Landing`),
    /// entered through another room (the owner, 2026-10-29: "two bedroom should not be linked
    /// together except if there is another door to a corridor for each. Having to have to go
    /// through a bedroom to reach the second one is very unusual").
    BedroomThroughRoom,
}

impl Kind {
    /// Every kind, in order.
    pub const ALL: [Kind; 21] = [
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
        Kind::OpeningMisfit,
        Kind::DoorBlocked,
        Kind::SharpWallCorner,
        Kind::BedroomThroughRoom,
    ];
}

/// The sharpest corner two garden walls may make, degrees ([`Kind::SharpWallCorner`]): a lot's
/// corners are square, a street's bend may make them 70° or more.
pub const SHARP_CORNER: f64 = 70.0;

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
    doors(v, structures, &mut out);
    for (i, (m, s)) in v.masses.iter().zip(structures).enumerate() {
        blank_fronts(i, m, s, &mut out);
        walks(i, m, s, &mut out);
    }
    out
}

/// The outside doors of the village's houses whose threshold meets something standing
/// ([`Kind::DoorBlocked`]): points 0.3, 0.6 and 1 m before the door, at its middle and 15 cm in
/// from its jambs, inside a garden wall, a standing piece of the gardens or another house.
fn doors(v: &Village, structures: &[Structure], out: &mut Vec<Anomaly>) {
    let pieces = super::gardens::gardens(&v.plan, &v.masses, structures);
    let walls: Vec<Vec<DVec2>> = v.plan.walls.iter().map(|w| w.ring()).collect();
    for (i, s) in structures.iter().enumerate() {
        for o in s.openings.iter().filter(|o| {
            o.storey == 0 && matches!(o.kind, OpeningKind::Door | OpeningKind::Shopfront)
        }) {
            let w = &s.walls[o.wall];
            let t = (w.b - w.a).normalize_or_zero();
            let outward = DVec2::new(t.y, -t.x);
            let (lo, hi) = o.span();
            let mut blocked = None;
            for along in [lo + 0.15, (lo + hi) * 0.5, hi - 0.15] {
                for off in [0.3, 0.6, 1.0] {
                    let p = w.a + t * along + outward * off;
                    if walls.iter().any(|r| geom::contains(r, p)) {
                        blocked = Some("a garden wall");
                    } else if pieces
                        .iter()
                        .any(|g| g.kind.standing() && geom::contains(&g.outline, p))
                    {
                        blocked = Some("a piece of the gardens");
                    } else if v
                        .masses
                        .iter()
                        .enumerate()
                        .any(|(j, m)| j != i && geom::contains(&m.footprint, p))
                    {
                        blocked = Some("another house");
                    }
                }
            }
            if let Some(what) = blocked {
                let at = w.a + t * o.along;
                out.push(Anomaly {
                    kind: Kind::DoorBlocked,
                    at,
                    height: 1.0,
                    from: at + outward * 4.0,
                    note: format!(
                        "house {i}: its {:?} on its {:?} wall opens onto {what}",
                        o.kind, w.role
                    ),
                });
            }
        }
    }
}

/// The openings of `structures` their modules do not fill ([`Kind::OpeningMisfit`]), `fills`
/// what [`facades::fills`] found in each (after [`facades::fit_windows`]); each seen from 4 m
/// out, at its storey.
pub fn openings(structures: &[Structure], fills: &[Vec<Option<facades::Fill>>]) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for (i, (s, f)) in structures.iter().zip(fills).enumerate() {
        for (o, fill) in s.openings.iter().zip(f) {
            let (lo, hi) = o.span();
            let wrong = match fill {
                None => o.kind != OpeningKind::Passage,
                Some(x) => {
                    (x.from - lo).abs() > 0.02
                        || (x.to - hi).abs() > 0.02
                        || (x.bottom - o.sill).abs() > 0.02
                        || (x.top - o.head).abs() > 0.02
                }
            };
            if !wrong {
                continue;
            }
            let w = &s.walls[o.wall];
            let t = (w.b - w.a).normalize_or_zero();
            let at = w.a + t * o.along;
            let level = s.storeys.get(o.storey).map_or(0.0, |st| st.level);
            out.push(Anomaly {
                kind: Kind::OpeningMisfit,
                at,
                height: level + (o.sill + o.head) * 0.5,
                from: at + DVec2::new(t.y, -t.x) * 4.0,
                note: format!(
                    "house {i}: its {:?} on storey {} from {lo:.2} to {hi:.2}, sill {:.2}, head {:.2}, filled {}",
                    o.kind,
                    o.storey,
                    o.sill,
                    o.head,
                    fill.map_or("by nothing".to_owned(), |x| format!(
                        "from {:.2} to {:.2}, {:.2} to {:.2}",
                        x.from, x.to, x.bottom, x.top
                    ))
                ),
            });
        }
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
            // Its length, which a wall's end may stop against (#308): 0.8 m of body, as its ends,
            // within the shrubs' crowns (0.55–0.7 m about the line, `village`).
            let t = (q - p).normalize_or_zero();
            out.push(Socket {
                kind: SocketKind::HedgeSide,
                piece,
                at: (p + q) * 0.5,
                level: 0.0,
                normal: DVec2::new(t.y, -t.x),
                extent: Extent::Rect {
                    width: p.distance(q),
                    height: 1.0,
                },
                thickness: 0.8,
                need: Need::Optional,
            });
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
    // The bedrooms entered through another room than the hall or a landing, each looked at from
    // that room's middle.
    let storeys = inner.rooms.iter().map(|r| r.storey + 1).max().unwrap_or(0);
    for k in 0..storeys {
        let from = interior::entered_from(inner, k);
        let circulation = |p: usize| p == 0 || inner.rooms[p - 1].kind == RoomKind::Landing;
        for (b, r) in inner.rooms.iter().enumerate() {
            if r.storey != k || r.kind != RoomKind::Bedroom || from[b + 1].is_some_and(circulation)
            {
                continue;
            }
            let middle = geom::centroid(&r.outline);
            let (through, seen_from) = match from[b + 1] {
                Some(p) => {
                    let room = &inner.rooms[p - 1];
                    (
                        format!("its {:?}", room.kind),
                        geom::centroid(&room.outline),
                    )
                }
                None => ("no doorway".to_owned(), middle),
            };
            out.push(Anomaly {
                kind: Kind::BedroomThroughRoom,
                at: middle,
                height: level(k) + 1.5,
                from: seen_from,
                note: format!("house {i}: a bedroom on storey {k} entered through {through}"),
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
        // A stub (`walls::stubs`): short, one end on a house's wall, square to it (within 6°),
        // joining a garden wall that ran beside the house to it (#308).
        let stub = w.length() <= walls::GATE + walls::THICKNESS
            && v.masses.iter().any(|m| {
                let n = m.footprint.len();
                (0..n).any(|k| {
                    let (p, q) = (m.footprint[k], m.footprint[(k + 1) % n]);
                    let d = (q - p).normalize_or_zero();
                    [w.a, w.b].iter().any(|&e| {
                        geom::distance_to_segment(e, p, q) < walls::THICKNESS * 0.5 + 0.06
                    }) && t.dot(d).abs() < 0.105
                })
            });
        if !on_line && !stub && !w.cut {
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
    // Two walls ending together (their ends within 0.3 m) at a corner sharper than
    // SHARP_CORNER, each pair once; looked at from outside the V, along its halving line. Two
    // street walls are left out: where they meet, the block's own corner is that sharp (the road
    // plan's), the V the check is for being garden walls on lot lines run aslant (#363, #308).
    let walls = &v.plan.walls;
    let cos_sharp = SHARP_CORNER.to_radians().cos();
    for (i, w) in walls.iter().enumerate() {
        for o in &walls[i + 1..] {
            if w.inside.is_some() && o.inside.is_some() {
                continue;
            }
            for (c, far) in [(w.a, w.b), (w.b, w.a)] {
                let Some(other_far) = [(o.a, o.b), (o.b, o.a)]
                    .into_iter()
                    .find(|&(e, _)| e.distance(c) < 0.3)
                    .map(|(_, f)| f)
                else {
                    continue;
                };
                let (u, t) = (
                    (far - c).normalize_or_zero(),
                    (other_far - c).normalize_or_zero(),
                );
                if u.dot(t) > cos_sharp {
                    let angle = u.dot(t).clamp(-1.0, 1.0).acos().to_degrees();
                    let outward = -(u + t).normalize_or_zero();
                    out.push(Anomaly {
                        kind: Kind::SharpWallCorner,
                        at: c,
                        height: w.height.min(o.height),
                        from: c + outward * 6.0,
                        note: format!("two garden walls meet at {angle:.0}°"),
                    });
                }
            }
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
    use super::super::{example, lots, structure, village};
    use super::*;

    /// The villages' layout rules (D-083's fifth step, #308), held at zero over the street plans of
    /// many seeds at 400 to 900 m, not counted at one: the lots and the court fill each block
    /// without overlapping, every lot is near a rectangle, the court is nowhere narrower than
    /// [`lots::COURT_WIDTH`], every garden wall stands on a street, every gate is open, no hedge
    /// crosses a wall, no
    /// hedge ends loose (on nothing: a wall, a house or another hedge), and no wall's end meets
    /// nothing (the sockets' check).
    #[test]
    fn the_village_layout_rules_hold_over_many_plans() {
        let mut broken: Vec<String> = Vec::new();
        for seed in 1..=10u64 {
            for size in [400.0, 500.0, 600.0, 700.0, 800.0, 900.0] {
                broken.extend(layout_rules(size, seed));
            }
        }
        assert!(
            broken.is_empty(),
            "{} broken:\n{}",
            broken.len(),
            broken.join("\n")
        );
    }

    /// The layout rules' breaches in the village `size` across whose streets seed `seed` draws
    /// (seed 1: the plan as `village::params` gives it).
    fn layout_rules(size: f64, seed: u64) -> Vec<String> {
        let site = example::site(size, 10.0);
        let mut params = village::params(size);
        if seed != 1 {
            params.seed = forge_core::Seed::new(20_261_016 + seed);
        }
        let v = village::village(&site, &params, 1);
        let mut out = Vec::new();
        let at = |p: DVec2| format!("{size} m seed {seed} at ({:.1}, {:.1})", p.x, p.y);
        for block in &v.blocks {
            let outline = geom::simplify_closed(&v.plan.blocks[block.block].buildable, 0.6);
            let area = geom::signed_area(&outline).abs();
            let filled: f64 = block
                .lots
                .iter()
                .chain(std::iter::once(&block.court))
                .map(|l| geom::signed_area(l).abs())
                .sum();
            if (filled - area).abs() > 1.0 {
                out.push(format!(
                    "{}: lots and court {filled:.1} m² of the block's {area:.1}",
                    at(geom::centroid(&outline))
                ));
            }
            for lot in &block.lots {
                if lots::squareness(lot) < lots::SQUARE {
                    out.push(format!(
                        "{}: a lot {:.2} of its rectangle",
                        at(geom::centroid(lot)),
                        lots::squareness(lot)
                    ));
                }
            }
            if block.court.len() >= 3 {
                let thin = lots::thin_area(&block.court, lots::COURT_WIDTH);
                let corners = lots::corners_area(&block.court, lots::COURT_WIDTH);
                if thin > corners + 12.0 {
                    out.push(format!(
                        "{}: the court thinner than {} m over {:.0} m²",
                        at(geom::centroid(&block.court)),
                        lots::COURT_WIDTH,
                        thin - corners
                    ));
                }
            }
            let on_street = |p: DVec2| {
                let n = outline.len();
                (0..n).any(|k| geom::distance_to_segment(p, outline[k], outline[(k + 1) % n]) < 0.2)
            };
            for w in v.plan.walls.iter().filter(|w| {
                let mid = (w.a + w.b) * 0.5;
                geom::contains(&outline, mid) || on_street(mid)
            }) {
                if !w.cut && !on_street((w.a + w.b) * 0.5) {
                    out.push(format!("{}: a wall off the street", at((w.a + w.b) * 0.5)));
                }
            }
            let footprints: Vec<&[DVec2]> = block.houses.iter().map(|h| &h.footprint[..]).collect();
            for (i, &(p, q)) in block.hedges.iter().enumerate() {
                for w in &v.plan.walls {
                    if let Some((t, u)) = geom::segment_intersection(p, q, w.a, w.b)
                        && t > 0.02
                        && t < 0.98
                        && u > 0.02
                        && u < 0.98
                    {
                        out.push(format!("{}: a hedge crossing a wall", at(p + (q - p) * t)));
                    }
                }
                for end in [p, q] {
                    let walled = v
                        .plan
                        .walls
                        .iter()
                        .any(|w| geom::distance_to_segment(end, w.a, w.b) < 0.5);
                    let housed = footprints.iter().any(|fp| {
                        let n = fp.len();
                        (0..n).any(|k| geom::distance_to_segment(end, fp[k], fp[(k + 1) % n]) < 0.5)
                    });
                    let hedged =
                        block.hedges.iter().enumerate().any(|(j, &(a, b))| {
                            j != i && geom::distance_to_segment(end, a, b) < 0.5
                        });
                    if !(walled || housed || hedged || on_street(end)) {
                        out.push(format!("{}: a hedge ending loose", at(end)));
                    }
                }
            }
        }
        // Every gate open: no wall across it.
        for &g in v.blocks.iter().flat_map(|b| &b.gates) {
            if v.plan
                .walls
                .iter()
                .any(|w| geom::distance_to_segment(g, w.a, w.b) < walls::THICKNESS * 0.5)
            {
                out.push(format!("{}: a gate walled across", at(g)));
            }
        }
        let structures = structure::structures(&v.plan, &v.masses);
        let mut found = Vec::new();
        unmatched(&v, &structures, &mut found);
        for a in found.iter().filter(|a| a.kind == Kind::FreeWallEnd) {
            out.push(format!("{}: {}", at(a.at), a.note));
        }
        out
    }

    /// No two houses of a village share ground, at 400 to 900 m (#308: a house deepened to
    /// adjoin the side behind it, or swung onto a neighbour's wall, must stop at it): each pair
    /// whose boxes meet sampled every 10 cm, points within 2 cm of either outline left out (two
    /// houses sharing a party wall touch along it).
    #[test]
    fn the_villages_houses_never_overlap() {
        let mut shared = Vec::new();
        for size in [400.0, 500.0, 600.0, 700.0, 800.0, 900.0] {
            let site = example::site(size, 10.0);
            let v = village::village(&site, &village::params(size), 1);
            let boxes: Vec<(DVec2, DVec2)> = v
                .masses
                .iter()
                .map(|m| {
                    m.footprint.iter().fold(
                        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                        |(lo, hi), &p| (lo.min(p), hi.max(p)),
                    )
                })
                .collect();
            let off_outline = |fp: &[DVec2], p: DVec2| {
                let n = fp.len();
                (0..n).all(|k| geom::distance_to_segment(p, fp[k], fp[(k + 1) % n]) > 0.02)
            };
            for i in 0..v.masses.len() {
                for j in i + 1..v.masses.len() {
                    let (lo, hi) = (boxes[i].0.max(boxes[j].0), boxes[i].1.min(boxes[j].1));
                    if lo.x >= hi.x || lo.y >= hi.y {
                        continue;
                    }
                    let (a, b) = (&v.masses[i].footprint, &v.masses[j].footprint);
                    let mut inside = 0;
                    let mut y = lo.y + 0.05;
                    while y < hi.y {
                        let mut x = lo.x + 0.05;
                        while x < hi.x {
                            let p = DVec2::new(x, y);
                            if geom::contains(a, p)
                                && geom::contains(b, p)
                                && off_outline(a, p)
                                && off_outline(b, p)
                            {
                                inside += 1;
                            }
                            x += 0.1;
                        }
                        y += 0.1;
                    }
                    if inside > 0 {
                        shared.push(format!(
                            "{size} m: houses {i} and {j} share about {:.2} m²",
                            f64::from(inside) * 0.01
                        ));
                    }
                }
            }
        }
        assert!(shared.is_empty(), "{shared:?}");
    }

    /// No bedroom of a village is entered through another room, at 400 to 900 m and over several
    /// seeds (their households differ): each has a doorway of its own onto its hall (the owner,
    /// 2026-10-29: "two bedroom should not be linked together except if there is another door to a
    /// corridor for each").
    #[test]
    fn no_bedroom_is_entered_through_another_room() {
        let mut found = Vec::new();
        for seed in 1..=4 {
            for size in [400.0, 500.0, 600.0, 700.0, 800.0, 900.0] {
                let site = example::site(size, 10.0);
                let v = village::village(&site, &village::params(size), seed);
                let structures = structure::structures(&v.plan, &v.masses);
                for (i, (m, s)) in v.masses.iter().zip(&structures).enumerate() {
                    if let Some(inner) = interior::house(m, s) {
                        let mut mine = Vec::new();
                        rooms(i, m, s, &inner, &mut mine);
                        mine.retain(|a| a.kind == Kind::BedroomThroughRoom);
                        if !mine.is_empty()
                            && let Some(plan) = &m.rooms
                        {
                            for (k, st) in plan.storeys.iter().enumerate() {
                                let rooms: Vec<String> = st
                                    .iter()
                                    .map(|r| {
                                        format!(
                                            "{:?} x {:.1}-{:.1} y {:.1}-{:.1}",
                                            r.kind, r.x.0, r.x.1, r.y.0, r.y.1
                                        )
                                    })
                                    .collect();
                                println!(
                                    "seed {seed}, {size} m, house {i} ({:.1} × {:.1} m), storey {k}: {rooms:?}",
                                    plan.width, plan.depth
                                );
                            }
                        }
                        found.extend(
                            mine.iter()
                                .map(|a| format!("seed {seed}, {size} m: {}", a.note)),
                        );
                    }
                }
            }
        }
        assert!(found.is_empty(), "{} {found:#?}", found.len());
    }

    /// The villages' anomalies, by kind, never more than when the check came in (2026-10-16, at
    /// 600 m; the smaller villages since they have houses, the larger since 2026-10-22, #308):
    /// each fix lowers its bound.
    #[test]
    fn the_villages_anomalies_never_grow() {
        let mut over = Vec::new();
        for size in [400.0, 500.0, 600.0, 700.0, 800.0, 900.0] {
            over.extend(over_bounds(size));
        }
        assert!(over.is_empty(), "{over:?}");
    }

    /// The anomalies of the village `size` across over their bounds.
    fn over_bounds(size: f64) -> Vec<String> {
        let site = example::site(size, 10.0);
        let v = village::village(&site, &village::params(size), 1);
        let mut structures = structure::structures(&v.plan, &v.masses);
        // Dressed as `city-plan` dresses it (the Mediterranean rules, 0.9 m over the roofs, seed
        // 12), its windows fitted to their modules, then its openings checked against them.
        let grammar = crate::grammar::Grammar::parse(facades::MEDITERRANEAN).expect("the rules");
        let every: Vec<usize> = (0..v.masses.len()).collect();
        let dress = |s: &[Structure]| facades::dress(&grammar, s, &v.masses, &every, 0.9, 12).0;
        let fills = facades::fills(&grammar, &structures, &dress(&structures));
        let before = openings(&structures, &fills);
        let fitted = facades::fit_windows(&mut structures, &fills);
        println!("{} openings misfit, {fitted} windows fitted", before.len());
        let mut all = village(&v, &structures);
        all.extend(openings(
            &structures,
            &facades::fills(&grammar, &structures, &dress(&structures)),
        ));
        let bounds: [(Kind, usize); 21] = [
            (Kind::RoomOutsideWalls, 0),
            (Kind::PartitionInOpening, 0),
            (Kind::UnlitRoom, 0),
            (Kind::Sliver, 0),
            (Kind::EaveStep, 0),
            (Kind::CrookedWall, 0),
            (Kind::MisplacedPlant, 0),
            (Kind::FreeWallEnd, 0),
            (Kind::LonePartyWall, 0),
            (Kind::RidgeStep, 0),
            (Kind::FlatRoof, 0),
            (Kind::StairOffWall, 0),
            (Kind::BlankStreetWall, 0),
            (Kind::UnreachedRoom, 0),
            (Kind::UnreachedStair, 0),
            (Kind::UnreachedDoor, 0),
            (Kind::DoorSwing, 0),
            (Kind::OpeningMisfit, 0),
            (Kind::DoorBlocked, 0),
            (Kind::SharpWallCorner, 0),
            (Kind::BedroomThroughRoom, 0),
        ];
        // The other sizes' own, where they had more when they came in (#308: at 700 to 900 m,
        // 2026-10-22, after the stairs, the rows' corners, the backs, the walls' corners and the
        // hedges' sides and the rows' ridges: 77 down to 11; 2026-10-23, a storey's lit rooms
        // packed front and back, a narrow end house's hall on its party side, a blank street wall's
        // window from the room behind its bend: 7). Then #285 (2026-10-23) widened the houses' halls
        // to 1.40 m and their lots with them, and the villages were laid again: 600 m kept at none,
        // 700 to 900 m at 13, those new #308's next; a garden door slid off a partition: 12;
        // a blank street wall at a bend took its window from a room 1.7 m off: 11. On 2026-10-24
        // the ground left between the strips and the lots too small for a house went to their
        // neighbours (the owner: open lawns on the street between walled gardens); two corners
        // it walled anew, one at 400 m and two at 700 m, end a wall short of its neighbour, and the
        // 700 m village keeps two small closed pockets: #308 next.
        // The sharp wall corners counted from 2026-10-29 (#363): 3 at 600 m, 5 to 8 in the bigger
        // villages, where lots run aslant at the streets' bends; cut where two walls alone meet
        // (`walls::chamfer_sharp_corners`), 2 and 4 to 6 are left, a hedge or a third wall at
        // their points; the free ends at 400, 800 and 900 m went with them.
        // The leftover ground's slivers half a metre to a metre wide, run out along a neighbour's
        // edge, go to that neighbour (`village::sliver_along`): the 600 m village's V gone, 1 left.
        // At a T (one of the two walls running on through the point in line with a third or a
        // hedge) the slanting wall is cut short and closed onto the line (`walls::
        // chamfer_sharp_corners`): none left at 600 m, 3 and 2 at 700 and 800 m.
        // The "corners" of 0° at 800 and 900 m were one wall twice (a lot line's piece between two
        // houses' corners and the court, from both its lots): kept once, and such a stub left
        // meeting nothing dropped. 700 m keeps 3: two walls and two hedges at one point, and a Y.
        // On 2026-10-30 the lots were cut again per street side (D-083's fifth step, `lots::cut`)
        // and the walls stand only along the streets: no free wall end, sharp wall corner, sliver
        // or lone party wall left at 400 to 700 m, nor at 900 m. The houses laid on the new lots
        // left these, the next work on the houses: rooms without a window in a corner house (500,
        // 600 and 900 m) and a study at 800 m, a ridge step at 700 and 900 m, a door unreached at
        // 700 m, and at 800 m two houses 2.8 m apart, their party walls each without its neighbour.
        let since: [(f64, Kind, usize); 9] = [
            (500.0, Kind::UnlitRoom, 1),
            (600.0, Kind::UnlitRoom, 4),
            (700.0, Kind::RidgeStep, 1),
            (700.0, Kind::UnreachedDoor, 1),
            (800.0, Kind::UnlitRoom, 1),
            (800.0, Kind::Sliver, 1),
            (800.0, Kind::LonePartyWall, 2),
            (900.0, Kind::UnlitRoom, 4),
            (900.0, Kind::RidgeStep, 1),
        ];
        let mut over = Vec::new();
        for (kind, bound) in bounds {
            let bound = since
                .iter()
                .find(|&&(s, k, _)| s == size && k == kind)
                .map_or(bound, |&(_, _, b)| b);
            let n = count(&all, kind);
            println!("{size} m, {kind:?}: {n} ({bound} at most)");
            if n > bound {
                over.push(format!("{size} m: {n} {kind:?}, {bound} at most"));
            }
        }
        over
    }
}

#[cfg(test)]
mod digests {
    use super::*;
    use std::hash::{Hash, Hasher};

    /// A digest of each size's masses, structures and garden walls (#308), to tell whether a
    /// change to the generator kept a village as it was: run before and after,
    /// `cargo test --release -p forge-procgen --lib village_digests -- --ignored --nocapture`.
    /// The 600 m village, the first map's, stays the same unless the owner agrees.
    #[test]
    #[ignore]
    fn village_digests() {
        for size in [400.0, 500.0, 600.0, 700.0, 800.0, 900.0] {
            let site = crate::city::example::site(size, 10.0);
            let v = crate::city::village::village(&site, &crate::city::village::params(size), 1);
            let structures = structure::structures(&v.plan, &v.masses);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", v.masses).hash(&mut h);
            format!("{:?}", structures).hash(&mut h);
            // The garden walls, hedges and gates too, which no mass holds.
            format!("{:?}", v.plan.walls).hash(&mut h);
            for b in &v.blocks {
                format!("{:?}{:?}{:?}", b.lots, b.hedges, b.gates).hash(&mut h);
            }
            println!("digest {size}: {:016x}", h.finish());
        }
    }

    /// Each size's garden walls, hedges and gates drawn on the plan with the lots and the houses,
    /// to look at their layout from above (#308): FORGE_PLAN_DIR names the folder for
    /// `plan-<size>.png`, 5 px a metre. Street walls dark red, walls between gardens orange, cuts
    /// magenta, hedges green, gates blue, houses grey, courts sand.
    #[test]
    #[ignore]
    fn village_plans() {
        let Ok(dir) = std::env::var("FORGE_PLAN_DIR") else {
            return;
        };
        let sizes: Vec<f64> = std::env::var("FORGE_PLAN_SIZES")
            .ok()
            .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
            .unwrap_or_else(|| vec![600.0]);
        // FORGE_PLAN_SEED=n: the streets of the layout test's plan `n` (1: the villages' own).
        let seed: u64 = std::env::var("FORGE_PLAN_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        for size in sizes {
            let site = crate::city::example::site(size, 10.0);
            let mut params = crate::city::village::params(size);
            if seed != 1 {
                params.seed = forge_core::Seed::new(20_261_016 + seed);
            }
            let v = crate::city::village::village(&site, &params, 1);
            let mut points: Vec<DVec2> = v.plan.walls.iter().flat_map(|w| [w.a, w.b]).collect();
            for b in &v.blocks {
                points.extend(b.lots.iter().flatten());
            }
            let (lo, hi) = points.iter().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(lo, hi), &p| (lo.min(p), hi.max(p)),
            );
            let (lo, hi) = (lo - 5.0, hi + 5.0);
            // FORGE_PLAN_AT=x,y,r: the square of r metres about (x, y) on the plan (as the
            // layout test prints its places), drawn at 20 px a metre.
            let focus: Option<Vec<f64>> = std::env::var("FORGE_PLAN_AT")
                .ok()
                .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect());
            let (lo, hi, scale) = match focus.as_deref() {
                Some(&[x, y, r]) => (DVec2::new(x - r, y - r), DVec2::new(x + r, y + r), 20.0),
                _ => (lo, hi, 5.0),
            };
            #[allow(non_snake_case)]
            let PX: f64 = scale;
            let (w, h) = (((hi.x - lo.x) * PX) as u32, ((hi.y - lo.y) * PX) as u32);
            let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([250, 250, 245]));
            let to = |p: DVec2| ((p.x - lo.x) * PX, (hi.y - p.y) * PX);
            let fill = |img: &mut image::RgbImage, poly: &[DVec2], c: [u8; 3]| {
                if poly.len() < 3 {
                    return;
                }
                let (a, b) = poly.iter().fold(
                    (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                    |(a, b), &p| (a.min(p), b.max(p)),
                );
                let (x0, y1) = to(a);
                let (x1, y0) = to(b);
                for y in (y0.max(0.0) as u32)..(y1.min(h as f64 - 1.0) as u32) {
                    for x in (x0.max(0.0) as u32)..(x1.min(w as f64 - 1.0) as u32) {
                        let p = DVec2::new(
                            lo.x + (f64::from(x) + 0.5) / PX,
                            hi.y - (f64::from(y) + 0.5) / PX,
                        );
                        if geom::contains(poly, p) {
                            img.put_pixel(x, y, image::Rgb(c));
                        }
                    }
                }
            };
            let line = |img: &mut image::RgbImage, a: DVec2, b: DVec2, width: f64, c: [u8; 3]| {
                let (ax, ay) = to(a);
                let (bx, by) = to(b);
                let steps = ((bx - ax).hypot(by - ay) * 2.0).ceil().max(1.0) as u32;
                let r = (width * 0.5).max(0.5);
                for s in 0..=steps {
                    let t = f64::from(s) / f64::from(steps);
                    let (cx, cy) = (ax + (bx - ax) * t, ay + (by - ay) * t);
                    for y in (cy - r).floor() as i64..=(cy + r).ceil() as i64 {
                        for x in (cx - r).floor() as i64..=(cx + r).ceil() as i64 {
                            let inside = (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) <= r;
                            if inside && x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                                img.put_pixel(x as u32, y as u32, image::Rgb(c));
                            }
                        }
                    }
                }
            };
            for b in &v.blocks {
                fill(&mut img, &b.court, [236, 226, 196]);
                if std::env::var("FORGE_PLAN_LOT_LINES").is_ok() {
                    for (k, lot) in b.lots.iter().enumerate() {
                        let h = (k as u32).wrapping_mul(2_654_435_761) ^ (b.block as u32 * 97);
                        let c = [
                            150 + (h & 0x5f) as u8,
                            150 + ((h >> 8) & 0x5f) as u8,
                            150 + ((h >> 16) & 0x5f) as u8,
                        ];
                        fill(&mut img, lot, c);
                    }
                }
                for house in &b.houses {
                    fill(&mut img, &house.footprint, [170, 170, 170]);
                }
                for lot in &b.lots {
                    for k in 0..lot.len() {
                        line(
                            &mut img,
                            lot[k],
                            lot[(k + 1) % lot.len()],
                            1.0,
                            [200, 200, 215],
                        );
                    }
                }
            }
            for b in &v.blocks {
                for &(a, c) in &b.hedges {
                    line(&mut img, a, c, 4.0, [40, 150, 50]);
                }
            }
            for wall in &v.plan.walls {
                let c = if wall.cut {
                    [220, 0, 200]
                } else if wall.inside.is_some() {
                    [140, 20, 20]
                } else {
                    [240, 120, 0]
                };
                line(
                    &mut img,
                    wall.a,
                    wall.b,
                    crate::city::walls::THICKNESS * PX,
                    c,
                );
            }
            for b in &v.blocks {
                for &g in &b.gates {
                    line(&mut img, g, g, 4.0, [30, 80, 230]);
                }
            }
            for b in &v.blocks {
                let lots: f64 = b.lots.iter().map(|l| geom::signed_area(l).abs()).sum();
                let court = geom::signed_area(&b.court).abs();
                println!(
                    "{size} block {} depth {} lots {} ({lots:.0} m2) court {court:.0} m2 ({:.0} %) centre ({:.0}, {:.0}) on the plan, houses {}",
                    b.block,
                    b.ty.depth,
                    b.lots.len(),
                    100.0 * court / (court + lots).max(1.0),
                    geom::centroid(&v.plan.blocks[b.block].buildable).x,
                    geom::centroid(&v.plan.blocks[b.block].buildable).y,
                    b.houses.len()
                );
            }
            if std::env::var("FORGE_PLAN_LOTS").is_ok() {
                for b in &v.blocks {
                    for (k, l) in b.lots.iter().enumerate() {
                        let housed = b.houses.iter().any(|h| h.lot == *l);
                        let c = geom::centroid(l);
                        println!(
                            "  block {} lot {k} area {:.0} housed {housed} centre ({:.0},{:.0}) corners {}",
                            b.block,
                            geom::signed_area(l).abs(),
                            c.x - lo.x,
                            hi.y - c.y,
                            l.len()
                        );
                    }
                }
            }
            if std::env::var("FORGE_PLAN_OUTLINES").is_ok() {
                for b in &v.blocks {
                    let o = geom::simplify_closed(&v.plan.blocks[b.block].buildable, 0.6);
                    let n = o.len();
                    let turns: Vec<String> = (0..n)
                        .map(|i| {
                            let a = (o[i] - o[(i + n - 1) % n]).normalize_or_zero();
                            let c = (o[(i + 1) % n] - o[i]).normalize_or_zero();
                            format!(
                                "({:.0},{:.0}) turn {:.0} next {:.1} m",
                                o[i].x - lo.x,
                                hi.y - o[i].y,
                                a.angle_to(c).to_degrees(),
                                o[i].distance(o[(i + 1) % n])
                            )
                        })
                        .collect();
                    println!("  outline {}: {}", b.block, turns.join("; "));
                }
            }
            if std::env::var("FORGE_PLAN_LOT_LINES").is_ok() {
                for b in &v.blocks {
                    for lot in &b.lots {
                        for k in 0..lot.len() {
                            line(
                                &mut img,
                                lot[k],
                                lot[(k + 1) % lot.len()],
                                1.0,
                                [30, 30, 220],
                            );
                        }
                    }
                }
            }
            img.save(format!("{dir}/plan-{size}.png")).unwrap();
        }
    }
}
