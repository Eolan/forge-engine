//! Houses on their lots (D-062's first map): for each lot a strip gives
//! ([`super::parcels::strips`]), a household's programme laid out by [`super::rooms::plan`]
//! to the lot's width and the strip's depth; the house's footprint runs from the street to its
//! plan's depth, cut to the lot where a corner lot narrows, and the rest of the lot is its
//! garden. The lots fit the houses and the houses their rooms, the owner's way round.

use glam::DVec2;

use super::geom;
use super::parcels::{self, ParcelType};
use super::rooms::{self, Means, Plan};

/// A house on its lot.
#[derive(Clone, Debug, PartialEq)]
pub struct House {
    /// Its lot, its street side first (as [`parcels::strips`] lays it).
    pub lot: Vec<DVec2>,
    /// Its plan: the rooms by storey on the lot's width.
    pub plan: Plan,
    /// Its footprint: the lot's width from the street to the plan's depth, within the lot.
    pub footprint: Vec<DVec2>,
    /// The household: its people and its means.
    pub household: (u32, Means),
}

/// The deepest a house goes, metres: two rooms deep with windows front and back
/// (`research/house-layouts.md` §1.2, rule 16: a room lit from one side works to about 6 m).
/// Deeper, the faubourg's houses ran 18 m back on 6 m fronts, the "stretched out rectangles" the
/// owner saw (2026-10-16), each hipped front to back instead of a row under one ridge.
pub const MAX_DEPTH: f64 = 12.0;

/// How deep a house's plan may go on a lot of type `ty`: the strip less the setback and the
/// yard, [`MAX_DEPTH`] at most, a hall and one room's depth at least.
pub fn plan_depth(ty: &ParcelType) -> f64 {
    (ty.depth - ty.setback - ty.yard).clamp(rooms::HALL + 2.6, MAX_DEPTH)
}

/// The least stretch of a lot's front a room deep that makes a house of its own, metres: the hall
/// with its stair, one room and a party wall (about 5.6 m); less,
/// the lot (a corner's wedge, a strip's remainder) joins its neighbour along the street.
pub const MIN_FRONT: f64 = rooms::HALL + 2.4 + rooms::WALL;

/// How near its lot's corner a row house's front may start or end and take the corner, metres:
/// the lot then reaches a room deep within this of it (a side line up to about 10° off square).
const CORNER: f64 = 0.5;

/// `lots` (each its street side first, `keys` the households they were cut for) with each lot
/// whose front is a room deep along under [`MIN_FRONT`] joined to its neighbour along the same
/// street, the two sharing a side and their fronts in line: the neighbour's household builds on
/// both (the owner, 2026-10-16: "house on top right looks too small … I would have make the
/// house it is stuck to bigger and remove this one"). A small lot with no such neighbour stays.
pub fn merge_small(
    lots: &[Vec<DVec2>],
    keys: &[usize],
    ty: &ParcelType,
) -> (Vec<Vec<DVec2>>, Vec<usize>) {
    let mut lots: Vec<Option<Vec<DVec2>>> = lots.iter().cloned().map(Some).collect();
    let small = |lot: &[DVec2]| {
        if lot.len() < 3 {
            return false;
        }
        let (a, b) = (lot[0], lot[1]);
        let t = (b - a).normalize_or_zero();
        let inward = DVec2::new(-t.y, t.x);
        let a = a + inward * ty.setback;
        front_stretch(lot, a, t, inward, rooms::DEPTH.0)
            .is_none_or(|(s0, s1)| s1 - s0 - 2.0 * ty.sides < MIN_FRONT)
    };
    let front = |lot: &[DVec2]| (lot[1] - lot[0]).normalize_or_zero();
    for k in 0..lots.len() {
        let Some(lot) = lots[k].clone().filter(|l| small(l)) else {
            continue;
        };
        // The neighbour beside it on the street: sharing its front's end or start, the two fronts
        // within 15° (a small lot is mostly a strip's end at a lane's bend, the front bending
        // there by 6–9°). The merged lot keeps the neighbour's front first, so its house is laid
        // along it and takes the small lot's ground behind its bent front.
        let found = (0..lots.len()).find_map(|j| {
            let other = lots[j].as_ref().filter(|_| j != k)?;
            if other.len() < 3 || front(other).dot(front(&lot)) < 0.966 {
                return None;
            }
            let merged = if other[1].distance(lot[0]) < 0.01 {
                merge_adjacent(other, &lot)?
            } else if other[0].distance(lot[1]) < 0.01 {
                merge_adjacent(&lot, other)?
            } else {
                return None;
            };
            let start = merged.iter().position(|p| p.distance(other[0]) < 0.01)?;
            let mut merged = merged;
            merged.rotate_left(start);
            Some((j, merged))
        });
        if let Some((j, merged)) = found {
            lots[j] = Some(merged);
            lots[k] = None;
        }
    }
    lots.into_iter()
        .zip(keys.iter().copied())
        .filter_map(|(lot, key)| lot.map(|l| (l, key)))
        .unzip()
}

/// The shortest edge a footprint keeps, metres: the facades give a shorter one no face
/// (`facades::edge_kinds`), its wall an inner one, and a house with an inner wall counts as a
/// wing another mass's core serves: it got no stair (#308).
const STUB: f64 = 0.5;

/// `fp` without its corners that turn by under 3° or close an edge under 15 cm: a footprint cut
/// from a lot keeps the lot's points along its sides, and a roof whose edges nearly line up, or
/// one turns over at once, will not pitch (the kit then flattens it, grey in a row of tiles).
/// An edge under [`STUB`] (a lot's corner clipped off at the back) goes too, its neighbours
/// run on to where they meet.
pub fn tidy(fp: &[DVec2]) -> Vec<DVec2> {
    let mut out = fp.to_vec();
    let mut changed = true;
    while changed && out.len() > 3 {
        changed = false;
        let n = out.len();
        for i in 0..n {
            let (p, c, q) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
            let (u, v) = (c - p, q - c);
            let turn = u.normalize_or_zero().perp_dot(v.normalize_or_zero()).abs();
            if u.length() < 0.15 || turn < 0.052 && u.dot(v) > 0.0 {
                out.remove(i);
                changed = true;
                break;
            }
        }
        if changed {
            continue;
        }
        for i in 0..n {
            let (p, c, q, r) = (
                out[(i + n - 1) % n],
                out[i],
                out[(i + 1) % n],
                out[(i + 2) % n],
            );
            if c.distance(q) >= STUB {
                continue;
            }
            // Where the edges either side meet, if near; else the stub's start goes.
            let (d1, d2) = (c - p, r - q);
            let denom = geom::cross(d1, d2);
            let meet = (denom.abs() > 1e-9)
                .then(|| p + d1 * (geom::cross(q - p, d2) / denom))
                .filter(|x| x.distance(c) < STUB && x.distance(q) < STUB);
            if let Some(x) = meet {
                out[(i + 1) % n] = x;
            }
            out.remove(i);
            changed = true;
            break;
        }
    }
    out
}

/// Two lots side by side along one street, `left`'s front ending where `right`'s starts and the
/// two sharing that side: one lot, its front from `left`'s start to `right`'s end first. `None`
/// where they do not share the side.
fn merge_adjacent(left: &[DVec2], right: &[DVec2]) -> Option<Vec<DVec2>> {
    let (n, m) = (left.len(), right.len());
    // `left`'s side from its front's end, `right`'s from its front's start, back along it.
    let near = |p: DVec2, q: DVec2| p.distance(q) < 0.01;
    let li = (0..n).find(|&i| near(left[i], right[0]))?;
    let rj = (0..m).find(|&j| near(right[j], left[(li + 1) % n]))?;
    // From left's start round to the shared front corner, then right's front and on round to
    // the shared side's far end, then left's on from there.
    let mut out: Vec<DVec2> = Vec::new();
    out.extend((0..=li).map(|i| left[i]));
    let mut j = 1;
    while j != rj {
        out.push(right[j % m]);
        j = (j + 1) % m;
    }
    out.extend(((li + 1)..n).map(|i| left[i]));
    // The shared corners in line with their neighbours go (the front's join first).
    let mut i = 0;
    while i < out.len() && out.len() > 3 {
        let (p, c, q) = (
            out[(i + out.len() - 1) % out.len()],
            out[i],
            out[(i + 1) % out.len()],
        );
        if (c - p).perp_dot(q - c).abs() < 1e-4 * (c - p).length() * (q - c).length() {
            out.remove(i);
        } else {
            i += 1;
        }
    }
    let out = geom::clean(&out, 1e-6);
    (out.len() >= 3 && geom::signed_area(&out) > 1.0).then_some(out)
}

/// A house on each of `lots`, the household of lot `k` from `household(k)`. A house of two storeys
/// or more is made `stair` deep at least where its type allows: its straight flight's depth
/// (`cores::flight_length` of its ground storey, the core's back wall, the front and back walls).
pub fn houses(
    lots: &[Vec<DVec2>],
    ty: &ParcelType,
    household: impl Fn(usize) -> (u32, Means),
    stair: f64,
) -> Vec<House> {
    // Each lot's house drafted: its front's stretch and its plan.
    let mut drafts: Vec<Option<Draft>> = lots
        .iter()
        .enumerate()
        .map(|(k, lot)| {
            if lot.len() < 3 {
                return None;
            }
            let (people, means) = household(k);
            let (a, b) = (lot[0], lot[1]);
            let t = (b - a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x);
            // The house's front line: the street line, or the type's setback behind it.
            let a = a + inward * ty.setback;
            // The house takes the stretch of the front where the lot is at least a room deep:
            // a corner lot's bisector cuts the strip to a wedge, and a house across the whole
            // front had a window at its end with the roof right behind it and no room (the
            // owner, 2026-10-16). The rest of the front stays garden, as do the type's sides.
            let (s0, s1) = front_stretch(lot, a, t, inward, rooms::DEPTH.0)?;
            // A row house (no side gardens) whose stretch starts or ends within [`CORNER`] of its
            // lot's corner takes the corner: a side line a degree off square left the house a
            // quarter metre short of it, and the houses either side stood apart by a slit (#308).
            // The lot's depth is still measured along the stretch as found: at the corner itself
            // a side off square leaves it a metre deep, and the house went.
            let found = (s0 + ty.sides, s1 - ty.sides);
            let frontage = parcels::frontage(lot);
            let (s0, s1) = if ty.sides > 0.0 {
                (s0, s1)
            } else {
                (
                    if s0 <= CORNER { 0.0 } else { s0 },
                    if s1 >= frontage - CORNER {
                        frontage
                    } else {
                        s1
                    },
                )
            };
            let (mut s0, mut s1) = (s0 + ty.sides, s1 - ty.sides);
            if s1 - s0 < MIN_FRONT {
                return None;
            }
            // A front far wider than the household's rooms want (a corner's two lots made one,
            // `parcels::strips_by`): the house as wide as they want, against its row neighbour,
            // the rest a garden round the corner. Across the whole front, it was a long thin house
            // (the owner, 2026-10-16: "not sure long houses like that are viable").
            let area: f64 = rooms::programme(people, means).iter().map(|r| r.area).sum();
            let wanted = (rooms::HALL + rooms::WALL + area / (2.0 * plan_depth(ty)))
                .clamp(ty.width.0, ty.width.1.max(16.0));
            // The neighbour whose front starts where this one's ends: a narrower house keeps to
            // that end, against it.
            let joined_at_end = lots
                .iter()
                .enumerate()
                .any(|(j, o)| j != k && o.len() >= 2 && o[0].distance(lot[1]) < 0.01);
            let narrow = |s0: &mut f64, s1: &mut f64, w: f64| {
                if joined_at_end {
                    *s0 = *s1 - w;
                } else {
                    *s1 = *s0 + w;
                }
            };
            if s1 - s0 > wanted * 1.5 {
                narrow(&mut s0, &mut s1, wanted);
            }
            let width = s1 - s0;
            // As deep as the lot lets it along its stretch at most: a plan deeper than its lot
            // left the lot's lines for a rectangle, a sliver beside its neighbour.
            let room = least_depth(lot, a, t, inward, s0.max(found.0), s1.min(found.1))
                .min(plan_depth(ty));
            let mut plan = rooms::plan(
                &rooms::programme(people, means),
                width,
                room.max(rooms::DEPTH.0),
            );
            // Closer to a square than a stretched rectangle (the owner, 2026-10-16: "house
            // surfaces are most likely to have something closer to a square shape than
            // stretched out rectangle"): a plan over 1.6 times as wide as deep is laid again
            // narrower, where its storeys stay as they were, it keeps within the lot's depth and
            // leaves a garden of 3 m at least beside it (houses touch or stand 3 m apart).
            if plan.width > 1.6 * plan.depth {
                // From 3 m narrower down to the least front, half a metre at a time: the plan
                // nearest 1.2 wide to 1 deep among those that keep its storeys and the lot's depth.
                let mut best: Option<(f64, rooms::Plan)> = None;
                let mut w = width - 3.0;
                while w >= MIN_FRONT {
                    let again = rooms::plan(
                        &rooms::programme(people, means),
                        w,
                        room.max(rooms::DEPTH.0),
                    );
                    if again.storeys.len() == plan.storeys.len() && again.depth <= room + 1e-6 {
                        let off = (w / again.depth.max(1e-6) - 1.2).abs();
                        if best.as_ref().is_none_or(|(o, _)| off < *o) {
                            best = Some((off, again));
                        }
                    }
                    w -= 0.5;
                }
                if let Some((off, again)) = best
                    && off < (plan.width / plan.depth.max(1e-6) - 1.2).abs()
                {
                    narrow(&mut s0, &mut s1, again.width);
                    plan = again;
                }
            }
            if plan.storeys.len() > 1 {
                // A house of storeys needs its flight's depth on its lot: deeper than the lot
                // along its front, its footprint ran past the lot, and an L-shaped corner lot's
                // inner side cut it to a sliver 0.9 m deep. The lot stays a garden. On a strip
                // whose type leaves a plan shallower than the flight (a faubourg's shallowest,
                // its garden given up) it still takes the flight's depth where the lot has it:
                // kept to the type's, the house got no stair and its upper rooms no way up (#308).
                let lot_room = least_depth(lot, a, t, inward, s0.max(found.0), s1.min(found.1));
                if stair.min(plan_depth(ty)) > room.max(rooms::DEPTH.0) + 0.05
                    || stair > lot_room.max(rooms::DEPTH.0) + 0.05
                {
                    return None;
                }
                plan.deepen(stair);
            }
            // A front cut short (a corner's wedge) with a household's rooms piled over three
            // storeys is no village house: the lot stays garden.
            if plan.storeys.len() > 3 {
                return None;
            }
            Some(Draft {
                a,
                t,
                s0,
                s1,
                plan,
                room,
                people,
                means,
            })
        })
        .collect();
    // The rows (D-066: a house's party walls meet its neighbours'): a lot whose house runs to its
    // front's end and the lot beside it, sharing that corner, whose house starts at its front's
    // start. Each row takes the depth of its deepest house where each lot allows it, so their
    // backs line up and houses of a height share one ridge: each its own depth, the backs stepped
    // and the ridges with them (the owner, 2026-10-16: "house should be stuck together and
    // probably share their roof structures").
    let reaches = |k: usize, start: bool| {
        drafts[k].as_ref().is_some_and(|d| {
            let width = parcels::frontage(&lots[k]);
            if start {
                d.s0 < 0.05
            } else {
                d.s1 > width - 0.05
            }
        })
    };
    let mut row: Vec<usize> = (0..lots.len()).collect();
    fn root(row: &mut [usize], k: usize) -> usize {
        let mut r = k;
        while row[r] != r {
            r = row[r];
        }
        row[k] = r;
        r
    }
    for i in 0..lots.len() {
        for j in 0..lots.len() {
            if i != j
                && lots[i].len() >= 3
                && lots[j].len() >= 3
                && lots[i][1].distance(lots[j][0]) < 0.01
                && reaches(i, false)
                && reaches(j, true)
            {
                let (ri, rj) = (root(&mut row, i), root(&mut row, j));
                row[ri] = rj;
            }
        }
    }
    let mut deepest: std::collections::HashMap<usize, f64> = Default::default();
    for (k, d) in drafts.iter().enumerate() {
        if let Some(d) = d {
            let r = root(&mut row, k);
            let e = deepest.entry(r).or_insert(0.0);
            *e = e.max(d.plan.depth);
        }
    }
    for (k, d) in drafts.iter_mut().enumerate() {
        let r = root(&mut row, k);
        if let Some(d) = d {
            let depth = deepest[&r].min(d.room);
            d.plan.deepen(depth);
        }
    }
    // The backs: a house whose back faces another house under [`BACK_GAP`] away is made
    // shallower, both when the two backs face each other, until the gap is a light court of that
    // width (Italy's Civil Code: buildings "united or adjoining", or 3 m apart; the owner,
    // 2026-10-16: "there are either adjoining properties or there should be a legal distance
    // between structures"). Its rooms are laid again on the shallower depth; a house that would
    // then lose its stair's depth or take a fourth storey adjoins instead where its back faces
    // another's side and its lot reaches that far: deepened to it (#308: a house 1.5 m short of
    // its corner neighbour's side, its back wall blank against the slit).
    let mut prints: Vec<Option<Vec<DVec2>>> = lots
        .iter()
        .zip(&drafts)
        .map(|(lot, d)| d.as_ref().and_then(|d| footprint_of(lot, d)))
        .collect();
    let mut shrink = vec![0.0f64; lots.len()];
    // The gap to a side straight behind, where the house could adjoin instead.
    let mut reach = vec![0.0f64; lots.len()];
    for i in 0..lots.len() {
        let (Some(d), Some(_)) = (&drafts[i], &prints[i]) else {
            continue;
        };
        let back = DVec2::new(-d.t.y, d.t.x);
        let gap = (0..lots.len())
            .filter(|&j| j != i)
            .filter_map(|j| prints[j].as_ref().map(|p| (j, p)))
            .filter_map(|(j, p)| {
                let g = (1..8)
                    .map(|k| {
                        let s = d.s0 + (d.s1 - d.s0) * f64::from(k) / 8.0;
                        let from = d.a + d.t * s + back * d.plan.depth;
                        ray_to(p, from, back, BACK_GAP)
                    })
                    .fold(f64::MAX, f64::min);
                (g > 0.05 && g < BACK_GAP).then_some((j, g))
            })
            .min_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((j, g)) = gap {
            // Halved when the other's back faces this one's.
            let facing = drafts[j].as_ref().is_some_and(|o| {
                let other_back = DVec2::new(-o.t.y, o.t.x);
                other_back.dot(back) < -0.9
            });
            let want = (BACK_GAP - g) * if facing { 0.5 } else { 1.0 };
            shrink[i] = shrink[i].max(want);
            if !facing {
                reach[i] = g;
            }
        }
    }
    for (k, (lot, d)) in lots.iter().zip(drafts.iter_mut()).enumerate() {
        let Some(d) = d.as_mut().filter(|_| shrink[k] > 0.0) else {
            continue;
        };
        let depth = d.plan.depth - shrink[k];
        let mut plan = rooms::plan(
            &rooms::programme(d.people, d.means),
            d.s1 - d.s0,
            depth.min(plan_depth(ty)),
        );
        let shrunk = if plan.storeys.len() > 1 && depth < stair {
            false
        } else {
            if plan.storeys.len() > 1 {
                plan.deepen(stair.min(depth));
            }
            plan.storeys.len() <= 3 && plan.depth <= depth + 1e-6
        };
        if shrunk {
            d.plan = plan;
            prints[k] = footprint_of(lot, d);
            continue;
        }
        // To the other's side, the lot's line between them: no deeper than the lot.
        let wanted = d.plan.depth + reach[k];
        let least = least_depth(lot, d.a, d.t, DVec2::new(-d.t.y, d.t.x), d.s0, d.s1);
        let deeper = wanted.min(least);
        if reach[k] > 0.0 && deeper <= MAX_DEPTH + 0.05 && least >= wanted - 0.05 {
            d.plan.deepen(deeper);
            prints[k] = footprint_of(lot, d);
        }
    }
    // Walls closer than a metre brought together (`snap`), those a degree or two apart swung
    // together (`swing`).
    snap(&mut prints);
    swing(&mut prints);
    lots.iter()
        .zip(drafts)
        .zip(prints)
        .filter_map(|((lot, draft), footprint)| {
            let d = draft?;
            Some(House {
                lot: lot.clone(),
                plan: d.plan,
                footprint: footprint?,
                household: (d.people, d.means),
            })
        })
        .collect()
}

/// Two houses' walls facing each other closer than a metre without touching: the first's wall
/// brought onto the other's, so the two adjoin (houses are "united or adjoining", or 3 m apart,
/// `research/house-layouts.md` rule 1). Two houses 10 cm apart each kept a party wall facing the
/// gap. Walls facing each other along half a metre at least, within 0.5°.
fn snap(prints: &mut [Option<Vec<DVec2>>]) {
    let n = prints.len();
    for i in 0..n {
        for j in i + 1..n {
            let (Some(p), Some(q)) = (prints[i].clone(), prints[j].clone()) else {
                continue;
            };
            let (pn, qn) = (p.len(), q.len());
            for e in 0..pn {
                let (a0, a1) = (p[e], p[(e + 1) % pn]);
                let ta = (a1 - a0).normalize_or_zero();
                let out = DVec2::new(ta.y, -ta.x);
                let length = a0.distance(a1);
                let moved = (0..qn).find_map(|f| {
                    let (b0, b1) = (q[f], q[(f + 1) % qn]);
                    let tb = (b1 - b0).normalize_or_zero();
                    let d = (b0 - a0).dot(out);
                    let (u0, u1) = ((b0 - a0).dot(ta), (b1 - a0).dot(ta));
                    let overlap = u0.max(u1).min(length) - u0.min(u1).max(0.0);
                    (ta.dot(tb) < -0.99996 && d > 0.005 && d < 1.0 && overlap > 0.5).then_some(d)
                });
                if let Some(d) = moved {
                    let mut p = p.clone();
                    p[e] += out * d;
                    p[(e + 1) % pn] += out * d;
                    prints[i] = Some(p);
                    break;
                }
                // Sharing a corner, their sides from it opening in a V (a row where the street
                // bends, each house square to its own front: two houses touching at the front and
                // a metre apart at the back): the first's side swung onto the other's, its far
                // corner slid along its own back wall.
                let Some(f) = (0..qn).find(|&f| q[(f + 1) % qn].distance(a0) < 0.01) else {
                    continue;
                };
                let b = q[f];
                let (u, w) = ((a1 - a0).normalize_or_zero(), (b - a0).normalize_or_zero());
                let angle = u.angle_to(w).abs();
                if !(0.003..0.26).contains(&angle) {
                    continue;
                }
                // Where the back wall's line (a1 → next) meets the other's side (a0 → b).
                let next = p[(e + 2) % pn];
                let (d1, d2) = (next - a1, b - a0);
                let denom = geom::cross(d1, d2);
                if denom.abs() > 1e-9 {
                    let s = geom::cross(a0 - a1, d2) / denom;
                    if s.abs() < 1.0 {
                        let mut p = p.clone();
                        p[(e + 1) % pn] = a1 + d1 * s;
                        prints[i] = Some(p);
                        break;
                    }
                }
            }
        }
    }
}

/// Two houses' walls facing each other within 0.5° to 3°, closer than a metre at both ends of one
/// without touching (a back square to its front against a side on a lot line a degree off, #308):
/// that one's ends brought straight out onto the other's line, the wall swung onto it. Each
/// house of a pair is tried as the one that moves: a side running past the other's back reaches
/// past its line at one end, while the back lies within the side's.
fn swing(prints: &mut [Option<Vec<DVec2>>]) {
    let n = prints.len();
    for i in 0..n {
        for j in 0..n {
            let (Some(p), Some(q)) = (prints[i].clone(), prints[j].clone()) else {
                continue;
            };
            if i == j {
                continue;
            }
            let (pn, qn) = (p.len(), q.len());
            for e in 0..pn {
                let (a0, a1) = (p[e], p[(e + 1) % pn]);
                let ta = (a1 - a0).normalize_or_zero();
                let out = DVec2::new(ta.y, -ta.x);
                let length = a0.distance(a1);
                let swung = (0..qn).find_map(|f| {
                    let (b0, b1) = (q[f], q[(f + 1) % qn]);
                    let tb = (b1 - b0).normalize_or_zero();
                    if !(-0.99996..-0.99863).contains(&ta.dot(tb)) {
                        return None;
                    }
                    let denom = geom::cross(out, tb);
                    let to = |a: DVec2| geom::cross(b0 - a, tb) / denom;
                    let (d0, d1) = (to(a0), to(a1));
                    let (u0, u1) = ((b0 - a0).dot(ta), (b1 - a0).dot(ta));
                    let overlap = u0.max(u1).min(length) - u0.min(u1).max(0.0);
                    ((-0.005..1.0).contains(&d0)
                        && (-0.005..1.0).contains(&d1)
                        && d0.max(d1) > 0.005
                        && overlap > 0.5)
                        .then_some((d0.max(0.0), d1.max(0.0)))
                });
                if let Some((d0, d1)) = swung {
                    let mut p = p.clone();
                    p[e] += out * d0;
                    p[(e + 1) % pn] += out * d1;
                    prints[i] = Some(p);
                    break;
                }
            }
        }
    }
}

/// The gap a house's back keeps from the next house straight behind it, metres: a light court
/// under which two houses must touch instead (`research/house-layouts.md` rule 1).
pub const BACK_GAP: f64 = 3.0;

/// How far from `from` along `dir` the ray first meets `polygon`'s outline, up to `reach`;
/// `f64::MAX` past it.
fn ray_to(polygon: &[DVec2], from: DVec2, dir: DVec2, reach: f64) -> f64 {
    let n = polygon.len();
    let far = from + dir * reach;
    (0..n)
        .filter_map(|i| geom::segment_intersection(from, far, polygon[i], polygon[(i + 1) % n]))
        .map(|(u, _)| u * reach)
        .fold(f64::MAX, f64::min)
}

/// House `d`'s footprint on `lot`. A house across its lot's whole front keeps to the lot's side
/// lines: its party walls on them, so the houses of a row share their walls where a lane bends and
/// the lots fan (square to its front, two houses met at the street and parted behind it, a sliver
/// with a garden wall in it; the owner, 2026-10-16: "there are either adjoining properties or
/// there should be a legal distance between structures"). Its rooms stay the plan's rectangle
/// within it. Otherwise the plan's rectangle, cut where a corner lot narrows; `None` where nothing
/// is left of it.
fn footprint_of(lot: &[DVec2], d: &Draft) -> Option<Vec<DVec2>> {
    let (a, t, s0, s1) = (d.a, d.t, d.s0, d.s1);
    let unit_in = DVec2::new(-t.y, t.x);
    let width = parcels::frontage(lot);
    if s0 < 0.05 && s1 > width - 0.05 && d.plan.depth <= d.room + 0.05 {
        let band = geom::clip_half_plane(lot, a + unit_in * d.plan.depth, -t);
        let mut band = geom::clip_half_plane(&band, a, t);
        // Where the street bends and the lot's front goes on past its frontage (the next edge
        // within 30° of it and 3 m or more), the house keeps to its frontage, the rest a garden:
        // its plan's rooms stop at the frontage, and the wall past the bend had nothing behind it,
        // blank on every storey (the village's house 23, 4.4 m on three). Shorter, it keeps it,
        // so no gap under 3 m opens to a neighbour.
        let n = lot.len();
        let goes_on = |p: DVec2, q: DVec2, dir: DVec2| {
            p.distance(q) >= 3.0 && (q - p).normalize_or_zero().dot(dir) > 30f64.to_radians().cos()
        };
        if n > 3 && goes_on(lot[1], lot[2], t) {
            band = geom::clip_half_plane(&band, a + t * s1, unit_in);
        }
        if n > 3 && goes_on(lot[0], lot[n - 1], -t) {
            band = geom::clip_half_plane(&band, a + t * s0, -unit_in);
        }
        let footprint = tidy(&geom::clean(&band, 1e-6));
        if footprint.len() >= 3 && geom::signed_area(&footprint).abs() >= 4.0 {
            return Some(footprint);
        }
    }
    let (a, b) = (a + t * s0, a + t * s1);
    let inward = unit_in * d.plan.depth;
    // An end on its lot's corner runs its side along the lot's side line (0.17° to 15° off
    // square, as `snap` swings a V) to the back: square to the front, a house narrower than its
    // lot or deeper than it lets stood in a V with its row neighbour at a bend, a slit opening to
    // a metre at the back (#308).
    let n = lot.len();
    let side = |corner: DVec2, next: DVec2| {
        let dir = (next - corner).normalize_or_zero();
        let square = dir.dot(unit_in);
        if square > 15f64.to_radians().cos() && square < 0.003f64.cos() {
            dir * (d.plan.depth / square)
        } else {
            inward
        }
    };
    let back_a = if n >= 3 && a.distance(lot[0]) < 0.05 {
        a + side(lot[0], lot[n - 1])
    } else {
        a + inward
    };
    let back_b = if n >= 3 && b.distance(lot[1]) < 0.05 {
        b + side(lot[1], lot[2])
    } else {
        b + inward
    };
    let mut footprint = vec![a, b, back_b, back_a];
    // Within the lot: a corner lot narrows towards the back. The cut is by each side's half
    // plane, as for a convex lot; a footprint already inside the lot keeps whole (a corner's two
    // lots made one, `parcels::strips_by`, are an L, and its inner corner's side cut the house
    // to 0.9 m). Within a centimetre of its outline counts: a back corner a millimetre past a
    // lot's back line sent the house through every side's clip, and a stub of the outline's
    // cut its back corner off (#308).
    let inside = footprint.iter().all(|&p| {
        geom::contains(lot, p)
            || (0..lot.len())
                .any(|i| geom::distance_to_segment(p, lot[i], lot[(i + 1) % lot.len()]) < 0.01)
    });
    let n = if inside { 0 } else { lot.len() };
    for i in 0..n {
        footprint = geom::clip_half_plane(&footprint, lot[i], lot[(i + 1) % n] - lot[i]);
        if footprint.len() < 3 {
            break;
        }
    }
    // A corner wedge too narrow for a house (its footprint cut to nothing) stays a garden. The
    // clips repeat a corner they pass through: a repeated corner made a wall of no length, an
    // inner one, and the house got no stair.
    let footprint = tidy(&geom::clean(&footprint, 1e-6));
    (footprint.len() >= 3 && geom::signed_area(&footprint).abs() >= 4.0).then_some(footprint)
}

/// A lot's house before its row is known: its front line from `a` along `t`, the stretch
/// `s0..s1` of it the house takes, its plan, how deep the lot lets it go along that stretch, and
/// its household.
struct Draft {
    a: DVec2,
    t: DVec2,
    s0: f64,
    s1: f64,
    plan: rooms::Plan,
    room: f64,
    people: u32,
    means: Means,
}

/// How deep `lot` reaches in along `inward` from the front point `a + t s`, metres: 0 where that
/// point is not on the lot's front.
fn deep_at(lot: &[DVec2], a: DVec2, t: DVec2, inward: DVec2, s: f64) -> f64 {
    let n = lot.len();
    let p = a + t * s;
    if !geom::contains(lot, p + inward * 0.3) {
        return 0.0;
    }
    let far = p + inward * 200.0;
    (0..n)
        .filter_map(|i| geom::segment_intersection(p, far, lot[i], lot[(i + 1) % n]))
        .map(|(u, _)| u * 200.0)
        .filter(|&d| d > 0.05)
        .fold(f64::MAX, f64::min)
}

/// The least depth of `lot` along its front from `s0` to `s1`, sampled every quarter metre and
/// 2 cm in from both ends.
fn least_depth(lot: &[DVec2], a: DVec2, t: DVec2, inward: DVec2, s0: f64, s1: f64) -> f64 {
    let (s0, s1) = (s0 + 0.02, (s1 - 0.02).max(s0 + 0.02));
    let steps = ((s1 - s0) / 0.25).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|k| deep_at(lot, a, t, inward, s0 + (s1 - s0) * k as f64 / steps as f64))
        .fold(f64::MAX, f64::min)
}

/// The stretch `(from, to)` along the front `a + t s` of `lot` where the lot reaches at least
/// `depth` metres in along `inward`, sampled every quarter metre (one stretch: along a convex
/// lot's front the depth rises then falls); `None` where it never does.
fn front_stretch(
    lot: &[DVec2],
    a: DVec2,
    t: DVec2,
    inward: DVec2,
    depth: f64,
) -> Option<(f64, f64)> {
    let width = parcels::frontage(lot);
    let deep_at = |s: f64| deep_at(lot, a, t, inward, s);
    let steps = (width / 0.25).ceil().max(1.0) as usize;
    let (mut from, mut to) = (None, None);
    for k in 0..=steps {
        let s = (k as f64 * 0.25).min(width);
        // Probed 2 cm in from the lot's corners: on a corner the probe fell on the side line,
        // outside, and every house of a row started a quarter metre in from its lot's side, a
        // sliver between each two with a garden wall in it (the anomaly check's first find).
        if deep_at(s.clamp(0.02, (width - 0.02).max(0.02))) >= depth {
            from.get_or_insert(s);
            to = Some(s);
        } else if from.is_some() {
            break;
        }
    }
    Some((from?, to?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modest_houses_fill_a_village_block_within_their_lots() {
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let ty = ParcelType::VILLAGE;
        let (lots, court) = parcels::strips(&block, &[true; 4], &ty, 5).expect("strips");
        let houses = houses(&lots, &ty, |k| (2 + (k % 3) as u32, Means::Modest), 8.3);
        // A corner wedge may keep no house.
        assert!(houses.len() + 2 >= lots.len() && houses.len() <= lots.len());
        for h in &houses {
            assert!(h.footprint.len() >= 3, "a footprint");
            let area = geom::signed_area(&h.footprint).abs();
            let lot_area = geom::signed_area(&h.lot).abs();
            assert!(
                area > 20.0 && area <= lot_area + 1e-6,
                "{area:.0} of {lot_area:.0} m²"
            );
            // Every footprint corner on or inside its lot.
            for &p in &h.footprint {
                let n = h.lot.len();
                let inside = geom::contains(&h.lot, p)
                    || (0..n)
                        .any(|i| geom::distance_to_segment(p, h.lot[i], h.lot[(i + 1) % n]) < 1e-6);
                assert!(inside, "a footprint corner {p} off its lot");
            }
            assert!(h.plan.depth <= ty.depth - ty.yard + 1e-9);
            assert!(!h.plan.storeys.is_empty());
        }
        assert!(geom::signed_area(&court).abs() > 1000.0);
        if let Ok(path) = std::env::var("HOUSES_PNG") {
            super::super::map::write_houses(
                &block,
                &houses,
                &court,
                std::path::Path::new(&path),
                0.1,
            )
            .expect("the drawing");
        }
    }
}
