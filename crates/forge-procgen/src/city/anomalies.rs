//! The anomalies of a generated village (the owner, 2026-10-16: "if it does not snap
//! everywhere, it means something is wrong, we need a way to detect anomalies for building and
//! probably roads, walls and other things"): the plan checked against the rules of
//! `docs/research/house-layouts.md` §8, each breach with where it stands and where to look at it
//! from, so the village's test counts them (never more than when the check came in) and the demo
//! prints a view of each (`city-plan --village 600 --anomalies`).

use glam::DVec2;

use super::geom;
use super::interior::{self, Interior};
use super::mass::Mass;
use super::rooms::RoomKind;
use super::structure::{OpeningKind, Structure};
use super::village::Village;

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
}

impl Kind {
    /// Every kind, in order.
    pub const ALL: [Kind; 7] = [
        Kind::RoomOutsideWalls,
        Kind::PartitionInOpening,
        Kind::UnlitRoom,
        Kind::Sliver,
        Kind::EaveStep,
        Kind::CrookedWall,
        Kind::MisplacedPlant,
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
    out
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
        let bounds: [(Kind, usize); 7] = [
            (Kind::RoomOutsideWalls, 45),
            (Kind::PartitionInOpening, 7),
            (Kind::UnlitRoom, 183),
            (Kind::Sliver, 19),
            (Kind::EaveStep, 0),
            (Kind::CrookedWall, 0),
            (Kind::MisplacedPlant, 1),
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
