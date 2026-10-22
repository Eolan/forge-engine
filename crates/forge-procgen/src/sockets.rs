//! Pieces' sockets (D-066, `docs/research/building-sockets.md`): where a piece may meet another,
//! with its kind, its frame, its extent and its profile, and one rule, [`compatible`], that says
//! whether two sockets meet. Generation places pieces so that their required sockets meet, the
//! anomaly check ([`match_all`]) reports those that do not, and a player's building will snap
//! through the same rule. The owner's: "the shape of the module piece and its type in itself
//! would authorize or reject what's possible to join it to" (2026-10-16).

use glam::DVec2;

/// What a socket is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A garden wall's end: meets another's end (in line or at a corner), a wall's face (a garden
    /// wall's or a house's), a gate's jamb or a hedge's end.
    GardenEnd,
    /// A garden wall's long face: what another's end may butt against.
    GardenFace,
    /// A house's outer wall on the street or its garden: what a garden wall's end may butt
    /// against.
    HouseFace,
    /// A house's party wall: needs a neighbour's party wall on its plane over all its length, or
    /// it stands bare over a sliver.
    PartyFace,
    /// A gate's jamb: the wall's end on either side of its opening.
    GateJamb,
    /// A hedge's end: where a garden wall may stop and the hedge go on.
    HedgeEnd,
    /// A hedge's length: what a garden wall's end may stop against, the hedge's thickness
    /// taking the wall's end in.
    HedgeSide,
}

/// What a socket covers about its origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extent {
    /// A point.
    Point,
    /// A rectangle `width` along `across` (centred) by `height` up from the origin.
    Rect {
        /// Its width, metres.
        width: f64,
        /// Its height, metres.
        height: f64,
    },
}

/// Whether a socket must meet another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// It must: alone, it is an anomaly.
    Required,
    /// It may: an anchor for others.
    Optional,
}

/// A socket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Socket {
    /// What it is for.
    pub kind: Kind,
    /// Its piece, an index the producer chose.
    pub piece: u32,
    /// Its origin, metres: the plan's x and y, and the height over the ground.
    pub at: DVec2,
    /// The height of its origin over the ground, metres.
    pub level: f64,
    /// Its outward normal on the plan, a unit vector.
    pub normal: DVec2,
    /// Its extent about its origin.
    pub extent: Extent,
    /// Its thickness (a wall's), metres.
    pub thickness: f64,
    /// Whether it must meet another.
    pub need: Need,
}

impl Socket {
    /// The direction its extent runs along on the plan: the normal turned left.
    pub fn across(&self) -> DVec2 {
        DVec2::new(-self.normal.y, self.normal.x)
    }
}

/// How close two points must be to meet, metres.
pub const TOLERANCE: f64 = 0.06;

/// Whether `a` meets `b` (`a` the one that needs it): their kinds allowed together and their
/// frames agreeing. A wall's end meets a face it touches: its end face's middle within half its
/// thickness and [`TOLERANCE`] of the face (a wall stopping at another's face, or running on past
/// it at a corner, its end then flush with the other's side); two points meet where they
/// coincide; two faces where they stand on one plane back to back.
pub fn compatible(a: &Socket, b: &Socket) -> bool {
    use Kind::*;
    if a.piece == b.piece {
        return false;
    }
    // Within `reach` of the face's middle line, along its width.
    let near_face = |p: &Socket, f: &Socket, reach: f64| {
        let Extent::Rect { width, .. } = f.extent else {
            return false;
        };
        let half = f.across() * (width * 0.5);
        let (s0, s1) = (f.at - half, f.at + half);
        let t = ((p.at - s0).dot(s1 - s0) / (s1 - s0).length_squared().max(1e-12)).clamp(0.0, 1.0);
        p.at.distance(s0 + (s1 - s0) * t) < reach
    };
    let point_on_face = |p: &Socket, f: &Socket| near_face(p, f, p.thickness * 0.5 + TOLERANCE);
    // Two points meet within the thicker one's half (a gate's jamb is as wide as its opening).
    let points_meet = |p: &Socket, q: &Socket| {
        p.at.distance(q.at) < TOLERANCE + p.thickness.max(q.thickness) * 0.5
    };
    match (a.kind, b.kind) {
        (GardenEnd, GardenEnd) => points_meet(a, b),
        (GardenEnd, GardenFace | HouseFace | PartyFace) => point_on_face(a, b),
        (GardenEnd, GateJamb | HedgeEnd) | (GateJamb | HedgeEnd, GardenEnd) => points_meet(a, b),
        // The wall's end face within the hedge: its middle within both halves of the line.
        (GardenEnd, HedgeSide) => near_face(a, b, (a.thickness + b.thickness) * 0.5 + TOLERANCE),
        (PartyFace, PartyFace) => {
            let (Extent::Rect { width: wa, .. }, Extent::Rect { width: wb, .. }) =
                (a.extent, b.extent)
            else {
                return false;
            };
            // Back to back on one plane, overlapping along it.
            let plane = (b.at - a.at).dot(a.normal).abs() < TOLERANCE;
            let facing = a.normal.dot(b.normal) < -0.98;
            let gap = (b.at - a.at).dot(a.across()).abs();
            plane && facing && gap < (wa + wb) * 0.5 - TOLERANCE
        }
        _ => false,
    }
}

/// How much of party face `a`'s width the faces in `others` that meet it cover, 0 to 1.
pub fn coverage(a: &Socket, others: &[&Socket]) -> f64 {
    let Extent::Rect { width, .. } = a.extent else {
        return 0.0;
    };
    let half = width * 0.5;
    let mut spans: Vec<(f64, f64)> = others
        .iter()
        .filter(|b| compatible(a, b))
        .filter_map(|b| match b.extent {
            Extent::Rect { width: wb, .. } => {
                let c = (b.at - a.at).dot(a.across());
                Some(((c - wb * 0.5).max(-half), (c + wb * 0.5).min(half)))
            }
            Extent::Point => None,
        })
        .filter(|(lo, hi)| hi > lo)
        .collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (mut covered, mut reach) = (0.0, -half);
    for (lo, hi) in spans {
        if hi > reach {
            covered += hi - lo.max(reach);
            reach = hi;
        }
    }
    if width > 0.0 { covered / width } else { 1.0 }
}

/// A required socket that meets nothing (or a party face not covered along its length).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unmatched {
    /// The socket.
    pub socket: Socket,
    /// How much of a face is covered, 0 to 1 (1 for a point that met nothing: 0).
    pub covered: f64,
}

/// Every required socket of `sockets` checked against the others: those that meet nothing, and
/// the party faces covered on under 95 % of their length. A grid of `cell` metres finds the
/// candidates.
pub fn match_all(sockets: &[Socket], cell: f64) -> Vec<Unmatched> {
    let key = |p: DVec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (i, s) in sockets.iter().enumerate() {
        // A face goes in every cell its width crosses.
        let reach = match s.extent {
            Extent::Rect { width, .. } => width * 0.5,
            Extent::Point => 0.0,
        } + s.thickness
            + TOLERANCE;
        let (lo, hi) = (
            key(s.at - DVec2::splat(reach)),
            key(s.at + DVec2::splat(reach)),
        );
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                grid.entry((x, y)).or_default().push(i);
            }
        }
    }
    let mut out = Vec::new();
    for s in sockets.iter().filter(|s| s.need == Need::Required) {
        let reach = match s.extent {
            Extent::Rect { width, .. } => width * 0.5,
            Extent::Point => 0.0,
        } + s.thickness
            + TOLERANCE;
        let (lo, hi) = (
            key(s.at - DVec2::splat(reach)),
            key(s.at + DVec2::splat(reach)),
        );
        let mut near: Vec<usize> = Vec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                if let Some(v) = grid.get(&(x, y)) {
                    near.extend(v);
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        let others: Vec<&Socket> = near.iter().map(|&k| &sockets[k]).collect();
        match s.extent {
            Extent::Rect { .. } => {
                let covered = coverage(s, &others);
                if covered < 0.95 {
                    out.push(Unmatched {
                        socket: *s,
                        covered,
                    });
                }
            }
            Extent::Point => {
                if !others.iter().any(|o| compatible(s, o)) {
                    out.push(Unmatched {
                        socket: *s,
                        covered: 0.0,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(piece: u32, at: DVec2, normal: DVec2) -> Socket {
        Socket {
            kind: Kind::GardenEnd,
            piece,
            at,
            level: 0.0,
            normal,
            extent: Extent::Point,
            thickness: 0.4,
            need: Need::Required,
        }
    }

    fn face(kind: Kind, piece: u32, at: DVec2, normal: DVec2, width: f64) -> Socket {
        Socket {
            kind,
            piece,
            at,
            level: 0.0,
            normal,
            extent: Extent::Rect { width, height: 3.0 },
            thickness: 0.25,
            need: if kind == Kind::PartyFace {
                Need::Required
            } else {
                Need::Optional
            },
        }
    }

    #[test]
    fn a_wall_end_meets_a_house_face_it_butts_against_and_nothing_beside_it() {
        // A house's face along y = 0 facing -y, 10 m wide about x = 5.
        let house = face(Kind::HouseFace, 1, DVec2::new(5.0, 0.0), DVec2::NEG_Y, 10.0);
        // A garden wall running south from it: its end at the face, pointing into it (+y).
        let butting = end(2, DVec2::new(3.0, -0.0), DVec2::Y);
        assert!(compatible(&butting, &house));
        // Past the face's end by more than half a wall, or a metre short of it: no.
        assert!(!compatible(
            &end(2, DVec2::new(11.0, 0.0), DVec2::Y),
            &house
        ));
        assert!(!compatible(
            &end(2, DVec2::new(3.0, -1.0), DVec2::Y),
            &house
        ));
        let unmatched = match_all(
            &[house, butting, end(3, DVec2::new(3.0, -5.0), DVec2::X)],
            10.0,
        );
        assert_eq!(unmatched.len(), 1, "{unmatched:?}");
        assert_eq!(unmatched[0].socket.piece, 3);
    }

    #[test]
    fn a_party_face_is_covered_by_its_neighbours_back_to_back() {
        // Two houses' party faces on x = 0, back to back; a third face 1 m off (a sliver).
        let a = face(Kind::PartyFace, 1, DVec2::new(0.0, 5.0), DVec2::X, 10.0);
        let b = face(Kind::PartyFace, 2, DVec2::new(0.0, 4.0), DVec2::NEG_X, 8.0);
        let c = face(Kind::PartyFace, 3, DVec2::new(1.0, 5.0), DVec2::NEG_X, 10.0);
        let all = [a, b, c];
        let un = match_all(&all, 10.0);
        // `a` is covered over 8 m of its 10 (80 %): unmatched, with its coverage; `b` wholly;
        // `c`, a metre off, not at all.
        let of = |p: u32| un.iter().find(|u| u.socket.piece == p).map(|u| u.covered);
        assert!(of(1).is_some_and(|c| (c - 0.8).abs() < 1e-9), "{un:?}");
        assert_eq!(of(2), None);
        assert_eq!(of(3), Some(0.0));
    }
}
