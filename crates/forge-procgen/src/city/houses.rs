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

/// `fp` without its corners that turn by under 3° or close an edge under 15 cm: a footprint cut
/// from a lot keeps the lot's points along its sides, and a roof whose edges nearly line up, or
/// one turns over at once, will not pitch (the kit then flattens it, grey in a row of tiles).
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
            let (s0, s1) = (s0 + ty.sides, s1 - ty.sides);
            let width = s1 - s0;
            if width < MIN_FRONT {
                return None;
            }
            // As deep as the lot lets it along its stretch at most: a plan deeper than its lot
            // left the lot's lines for a rectangle, a sliver beside its neighbour.
            let room = least_depth(lot, a, t, inward, s0, s1).min(plan_depth(ty));
            let mut plan = rooms::plan(
                &rooms::programme(people, means),
                width,
                room.max(rooms::DEPTH.0),
            );
            if plan.storeys.len() > 1 {
                plan.deepen(stair.min(plan_depth(ty)));
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
    // then lose its stair's depth or take a fourth storey stays as it was.
    let mut prints: Vec<Option<Vec<DVec2>>> = lots
        .iter()
        .zip(&drafts)
        .map(|(lot, d)| d.as_ref().and_then(|d| footprint_of(lot, d)))
        .collect();
    let mut shrink = vec![0.0f64; lots.len()];
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
        if plan.storeys.len() > 1 {
            if depth < stair {
                continue;
            }
            plan.deepen(stair.min(depth));
        }
        if plan.storeys.len() > 3 || plan.depth > depth + 1e-6 {
            continue;
        }
        d.plan = plan;
        prints[k] = footprint_of(lot, d);
    }
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
        let band = geom::clip_half_plane(&band, a, t);
        let footprint = tidy(&geom::clean(&band, 1e-6));
        if footprint.len() >= 3 && geom::signed_area(&footprint).abs() >= 4.0 {
            return Some(footprint);
        }
    }
    let (a, b) = (a + t * s0, a + t * s1);
    let inward = unit_in * d.plan.depth;
    let mut footprint = vec![a, b, b + inward, a + inward];
    // Within the lot: a corner lot narrows towards the back.
    let n = lot.len();
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
