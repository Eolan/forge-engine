//! Lots that fit the houses (`research/parcels-and-rooms.md`, D-062's first map): a block's
//! street sides cut into strips of one depth (the block inset as [`geom::inset`] does, mitred
//! at the corners, near enough the straight skeleton's), each strip into lots by cuts square
//! to its street at widths a type draws, the corner lots taking the corners, a strip's end too
//! short for a lot merged into the last one. What a cadastre does, and what the city's lots
//! ([`super::blocks::subdivide`], split across the longer side) do not: party walls square to
//! the street, no slivers, no wedge for no reason (the owner, 2026-10-15). The first map's
//! village takes it first; the city, block by block.

use forge_core::SplitMix64;
use glam::DVec2;

use super::geom;

/// How a block's street sides are cut into lots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParcelType {
    /// The lots' depth from the street, metres: the strip's.
    pub depth: f64,
    /// The lots' widths along the street, metres, drawn between these two.
    pub width: (f64, f64),
    /// How far behind the street line the house stands, metres: none in a core, a front
    /// garden in a lotissement.
    pub setback: f64,
    /// What the house leaves to each side of its lot, metres: none between party walls, a
    /// passage round a free-standing house.
    pub sides: f64,
    /// What the house leaves at the back of its lot at least, metres: a yard, a garden.
    pub yard: f64,
}

impl ParcelType {
    /// A village house's lot: 6 to 8 m wide on a 14 m strip, a yard behind (6 m: the hall with
    /// its stair, `rooms::HALL`, and a room beside it).
    pub const VILLAGE: Self = Self {
        depth: 14.0,
        width: (6.0, 8.0),
        setback: 0.0,
        sides: 0.0,
        yard: 1.5,
    };
    /// A perimeter block's building: 12 to 25 m wide on an 18 m strip.
    pub const BLOCK: Self = Self {
        depth: 18.0,
        width: (12.0, 25.0),
        setback: 0.0,
        sides: 0.0,
        yard: 0.0,
    };
    /// An old core's lot (`research/cadastre-villages-and-towns.md` §2): 6 to 9 m wide on a
    /// 12 m strip (the research's 5 to 10 m; 6 m holds the hall and a room), the house the
    /// whole lot, a court only where the rooms leave one.
    pub const CORE: Self = Self {
        depth: 12.0,
        width: (6.0, 9.0),
        setback: 0.0,
        sides: 0.0,
        yard: 0.0,
    };
    /// A faubourg's lanière: 6 to 12 m wide on a 30 m strip, the house on the street line, a
    /// garden of 12 m at least behind.
    pub const FAUBOURG: Self = Self {
        depth: 30.0,
        width: (6.0, 12.0),
        setback: 0.0,
        sides: 0.0,
        yard: 12.0,
    };
    /// A lotissement's lot: 15 to 25 m wide on a 35 m strip (500 to 900 m²), the house free on
    /// four sides, 5 m back from the street, 3 m off the sides, a garden of 8 m behind.
    pub const LOTISSEMENT: Self = Self {
        depth: 35.0,
        width: (15.0, 25.0),
        setback: 5.0,
        sides: 3.0,
        yard: 8.0,
    };
}

/// A block's lots and its court. The block is a polygon of positive area; `street[i]` says
/// whether its edge `i` (from vertex `i` to `i + 1`) is a street side, the others party sides
/// (a lot there reaches the court). `None` where the block is too thin for a strip of the
/// type's depth all round: the caller falls back on another cut.
pub fn strips(
    block: &[DVec2],
    street: &[bool],
    ty: &ParcelType,
    seed: u64,
) -> Option<(Vec<Vec<DVec2>>, Vec<DVec2>)> {
    let mut rng = SplitMix64::new(seed);
    strips_with(block, street, ty, |_| rng.range_f64(ty.width.0, ty.width.1))
        .map(|(lots, court, _)| (lots, court))
}

/// A block cut into strips: its lots (each its street side first), its court, and each lot's
/// key, the cut it was planned as ([`strips_with`]).
pub type Cut = (Vec<Vec<DVec2>>, Vec<DVec2>, Vec<usize>);

/// [`strips`] with each lot's width asked of `width` by a key counting the cuts along the
/// block's street sides: the lots cut to fit their houses, the owner's way round (the rooms
/// first). The keys come back with the lots: a lot that took a strip's remainder or swallowed
/// a corner sliver keeps the key of its planned part, so the keys skip where lots merged.
pub fn strips_with(
    block: &[DVec2],
    street: &[bool],
    ty: &ParcelType,
    width: impl FnMut(usize) -> f64,
) -> Option<Cut> {
    strips_by(block, street, &vec![ty.depth; block.len()], ty, width)
}

/// [`strips_with`] with a depth for each edge: a faubourg's deep strips along the road out and
/// none along its lanes, whose side stays the gardens' ends (#266). Between two edges of
/// different depths the corner is split along the line from the corner to where the two depth
/// lines meet, so a deep strip's corner lot keeps its depth beside a shallow one; beside an edge
/// of depth 0 it runs to that edge. An edge of depth under 0.5 m takes no strip.
pub fn strips_by(
    block: &[DVec2],
    street: &[bool],
    depths: &[f64],
    ty: &ParcelType,
    mut width: impl FnMut(usize) -> f64,
) -> Option<Cut> {
    let n = block.len();
    if n < 3 || street.len() != n || depths.len() != n || geom::signed_area(block) <= 0.0 {
        return None;
    }
    // The court: the block inset by the depths. A block too thin for it takes no strips; one
    // whose inset loses a corner (a short edge gone, the straight skeleton's edge event) still
    // takes them, each strip bounded by its depth line and its corners' dividers. An edge
    // without a strip moves in by 5 cm: the inset's check wants each corner inside the outline.
    let inset: Vec<f64> = depths.iter().map(|d| d.max(0.05)).collect();
    let court = geom::inset_by_collapsing(block, &inset)
        .filter(|c| c.len() >= 3 && geom::signed_area(c) > 1.0)?;
    // Each edge's inward normal, and each corner's divider: toward where its two edges' depth
    // lines meet; the normals' sum (the bisector, which also halves a reflex corner's inner
    // angle) where the depths are equal or the edges nearly in line.
    let normal = |i: usize| {
        let t = (block[(i + 1) % n] - block[i]).normalize_or_zero();
        DVec2::new(-t.y, t.x)
    };
    let divider = |i: usize| {
        let h = (i + n - 1) % n;
        let (n0, n1) = (normal(h), normal(i));
        let (d0, d1) = (depths[h], depths[i]);
        let det = n0.perp_dot(n1);
        if (d0 - d1).abs() > 1e-6 && det.abs() > 0.1 {
            // x · n0 = d0 and x · n1 = d1.
            let x = DVec2::new(d0 * n1.y - d1 * n0.y, n0.x * d1 - n1.x * d0) / det;
            if x.length() > 1e-6 {
                return x.normalize();
            }
        }
        let d = n0 + n1;
        if d.length() < 1e-6 { n1 } else { d.normalize() }
    };
    // Each corner between two strips that turns by 30° or more: a corner lot of its own, from
    // the corner along both streets to the feet of the court's corner on them, its two inner
    // sides square to their streets, so every party line stands square to its street. Split on
    // the bisector, the corner lots were wedges with party lines on the diagonal and houses
    // turned against each other (the owner, 2026-10-16: "diagonal layouts like in your
    // picture: no"). `(x, on the previous edge, on the next edge)`; `None` at a bend, at a
    // reflex corner, beside a side without a strip, or where the corner lot would take most of
    // an edge, or leave the edge too little between it and the next corner's lot (the larger of
    // the two goes).
    let edge_dir = |i: usize| (block[(i + 1) % n] - block[i]).normalize_or_zero();
    let mut square: Vec<Option<(DVec2, DVec2, DVec2)>> = (0..n)
        .map(|i| {
            let h = (i + n - 1) % n;
            let has_strip = |e: usize| {
                street[e] && depths[e] >= 0.5 && block[e].distance(block[(e + 1) % n]) >= 0.5
            };
            let (th, ti) = (edge_dir(h), edge_dir(i));
            if !has_strip(h) || !has_strip(i) || th.perp_dot(ti) <= 0.0 || th.dot(ti) > 0.866 {
                return None;
            }
            let (n0, n1) = (normal(h), normal(i));
            let det = n0.perp_dot(n1);
            if det.abs() < 1e-6 {
                return None;
            }
            // r · n0 = depth of h and r · n1 = depth of i, r from the corner.
            let (d0, d1) = (depths[h], depths[i]);
            let r = DVec2::new(d0 * n1.y - d1 * n0.y, n0.x * d1 - n1.x * d0) / det;
            let corner = block[i];
            let (along_i, back_h) = (r.dot(ti), -r.dot(th));
            let (len_i, len_h) = (
                corner.distance(block[(i + 1) % n]),
                corner.distance(block[h]),
            );
            // A sharp corner's (under 60°) region is the tip, left open as a green rather than a
            // corner lot (`tips`), and may take more of its edges.
            let sharp = th.dot(ti) < -0.5;
            let most = if sharp { 0.95 } else { 0.8 };
            let fits =
                (0.5..most * len_i).contains(&along_i) && (0.5..most * len_h).contains(&back_h);
            if fits {
                Some((corner + r, corner - th * back_h, corner + ti * along_i))
            } else if sharp && along_i > 0.5 && back_h > 0.5 {
                // A long thin tip, its region past most of an edge: open all the same, up to the
                // edges' ends. Split on the bisector instead, its two lots met on a diagonal wall
                // from the tip into the block (the owner, 2026-10-16: "still the diagonal
                // issues", the faubourg's triangle).
                let (along_i, back_h) = (along_i.min(len_i), back_h.min(len_h));
                Some((corner + r, corner - th * back_h, corner + ti * along_i))
            } else {
                None
            }
        })
        .collect();
    // The sharp corners (under 60°), whose regions stay open: a triangular block's tip split
    // into two wedges on the bisector made long diagonal party lines (the owner, 2026-10-16:
    // "still the diagonal issues"); villages leave the tip a small green or a square.
    let tips: Vec<bool> = (0..n)
        .map(|i| {
            let h = (i + n - 1) % n;
            square[i].is_some() && edge_dir(h).dot(edge_dir(i)) < -0.5
        })
        .collect();
    // Two corner lots on one edge leave a lot's least width between them at least.
    for i in 0..n {
        let j = (i + 1) % n;
        let (Some((_, _, f0)), Some((_, f1, _))) = (square[i], square[j]) else {
            continue;
        };
        let (a, b) = (block[i], block[j]);
        let t = (b - a).normalize_or_zero();
        if (f1 - a).dot(t) - (f0 - a).dot(t) < ty.width.0 {
            // A tip stays open: the other corner's lot goes (two tips on one edge leave it to
            // their greens).
            if tips[i] && tips[j] {
                continue;
            }
            let (take_i, take_j) = ((f0 - a).length(), (b - f1).length());
            if tips[i] != tips[j] {
                if tips[i] {
                    square[j] = None;
                } else {
                    square[i] = None;
                }
            } else if take_i >= take_j {
                square[i] = None;
            } else {
                square[j] = None;
            }
        }
    }
    // Each street side's strip: the block within the depth of its side, between the dividers
    // at the side's two corners, so the first and last lots take the corners; at a square
    // corner, from the corner lot's inner side on.
    let strip_of = |i: usize| -> Option<Vec<DVec2>> {
        let depth = depths[i];
        let (a, b) = (block[i], block[(i + 1) % n]);
        if !street[i] || depth < 0.5 || a.distance(b) < 0.5 {
            return None;
        }
        let t = (b - a).normalize();
        let inward = DVec2::new(-t.y, t.x);
        let mut strip = geom::clip_half_plane(block, a + inward * depth, -t);
        let j = (i + 1) % n;
        for (corner, d, along, sq) in [
            (a, divider(i), t, square[i].map(|(_, _, f)| (f, -inward))),
            (b, divider(j), -t, square[j].map(|(_, f, _)| (f, inward))),
        ] {
            strip = match sq {
                Some((foot, keep)) => geom::clip_half_plane(&strip, foot, keep),
                None => {
                    let keep = if d.perp_dot(along) >= 0.0 { d } else { -d };
                    geom::clip_half_plane(&strip, corner, keep)
                }
            };
        }
        // The clips repeat a corner they pass through.
        let strip = geom::clean(&strip, 1e-6);
        (strip.len() >= 3 && geom::signed_area(&strip) > 1.0).then_some(strip)
    };
    let raw: Vec<Option<Vec<DVec2>>> = (0..n).map(strip_of).collect();
    let mut lots: Vec<Vec<DVec2>> = Vec::new();
    // The street side each lot was cut along.
    let mut sides: Vec<usize> = Vec::new();
    let mut keys: Vec<usize> = Vec::new();
    for i in 0..n {
        let Some(mut strip) = raw[i].clone() else {
            continue;
        };
        let depth = depths[i];
        let j = (i + 1) % n;
        let (a, b) = (block[i], block[j]);
        let length = a.distance(b);
        let t = (b - a) / length;
        let inward = DVec2::new(-t.y, t.x);
        // Where a short edge between two collapses (the straight skeleton's edge event), or the
        // block narrows under two depths, two strips overlap: each then keeps to its side of the
        // line where its depth and the other's stand in proportion, {p : g·p ≤ k}. Strips that
        // only meet are left as they are.
        for o in 0..n {
            let Some(other) = raw[o].as_ref().filter(|_| o != i) else {
                continue;
            };
            let g = inward / depth - normal(o) / depths[o];
            if g.length() < 1e-6 || !overlapping(&strip, other) {
                continue;
            }
            let k = inward.dot(a) / depth - normal(o).dot(block[o]) / depths[o];
            strip =
                geom::clip_half_plane(&strip, g * (k / g.length_squared()), DVec2::new(-g.y, g.x));
        }
        let strip = geom::clean(&strip, 1e-6);
        if strip.len() < 3 || geom::signed_area(&strip) <= 1.0 {
            continue;
        }
        // The cuts along the street, from a square corner's lot on and up to the next: a width
        // drawn each time; the last lot takes what is left when that is short of the least
        // width, so no sliver is cut.
        let start = square[i].map_or(0.0, |(_, _, f)| (f - a).dot(t));
        let end = square[j].map_or(length, |(_, f, _)| (f - a).dot(t));
        let mut cuts = vec![start];
        let mut s = start;
        let mut k = lots.len();
        loop {
            let w = width(k).max(0.5);
            if s + w + ty.width.0 > end {
                break;
            }
            s += w;
            cuts.push(s);
            k += 1;
        }
        cuts.push(end);
        // The keys the lots were cut for, one a cut.
        let first = lots.len();
        let mut ks: Vec<usize> = (first..first + cuts.len() - 1).collect();
        let cut = |cuts: &[f64], ks: &[usize]| -> Vec<(usize, Vec<DVec2>)> {
            let mut out = Vec::new();
            for k in 0..cuts.len() - 1 {
                let (s0, s1) = (cuts[k], cuts[k + 1]);
                let mut lot = strip.clone();
                if k > 0 {
                    // After the cut at s0: the right of the inward line through it.
                    lot = geom::clip_half_plane(&lot, a + t * s0, -inward);
                }
                if k + 2 < cuts.len() {
                    // Before the cut at s1: its left.
                    lot = geom::clip_half_plane(&lot, a + t * s1, inward);
                }
                let lot = geom::clean(&lot, 1e-6);
                if lot.len() >= 3 && geom::signed_area(&lot).abs() > 1.0 {
                    out.push((ks[k], from_the_street(lot, a, b)));
                }
            }
            out
        };
        let mut here = cut(&cuts, &ks);
        // A corner lot cut to a sliver by an acute corner's bisector joins its neighbour: the
        // cut between them goes (a lot under half the least lot's area is a sliver), and the
        // merged lot keeps the key of the part that was planned (the first's, or the last but
        // one's: the last took the remainder).
        let sliver = ty.width.0 * depth * 0.5;
        for _ in 0..2 {
            let m = here.len();
            if m < 2 || cuts.len() < 3 {
                break;
            }
            if geom::signed_area(&here[0].1).abs() < sliver {
                cuts.remove(1);
                ks.remove(1);
            } else if geom::signed_area(&here[m - 1].1).abs() < sliver {
                cuts.remove(cuts.len() - 2);
                ks.pop();
            } else {
                break;
            }
            here = cut(&cuts, &ks);
        }
        for (key, lot) in here {
            keys.push(key);
            sides.push(i);
            lots.push(lot);
        }
    }
    // A corner of 30° or more with no square lot (it did not fit its edges): its two strips' end
    // lots met on its divider, a diagonal party line from the corner into the block, a garden
    // wall on it (the owner, 2026-10-16: "diagonal layouts ... no", "still the diagonal
    // issues"). They make one corner lot, its front the longer street's, the other's key gone.
    for i in 0..n {
        let h = (i + n - 1) % n;
        let (th, ti) = (edge_dir(h), edge_dir(i));
        if square[i].is_some() || th.perp_dot(ti) <= 0.0 || th.dot(ti) > 0.866 {
            continue;
        }
        let corner = block[i];
        let at = |side: usize| {
            (0..lots.len())
                .find(|&k| sides[k] == side && lots[k].iter().any(|p| p.distance(corner) < 0.01))
        };
        let (Some(kh), Some(ki)) = (at(h), at(i)) else {
            continue;
        };
        let Some(one) = joined(&lots[kh], &lots[ki]) else {
            continue;
        };
        // The larger lot's front stays the front: along the longer street, a corner lot thin
        // there took a house a metre and a half deep.
        let (area_h, area_i) = (
            geom::signed_area(&lots[kh]).abs(),
            geom::signed_area(&lots[ki]).abs(),
        );
        let (keep, gone, a, b) = if area_h >= area_i {
            (kh, ki, block[h], corner)
        } else {
            (ki, kh, corner, block[(i + 1) % n])
        };
        // Only where its front is found on that street: turned from some other side, a house
        // stood on a side of the lot 0.9 m deep.
        let one = from_the_street(one, a, b);
        let on = |p: DVec2| geom::distance_to_segment(p, a, b) < 1e-3;
        if !(on(one[0]) && on(one[1])) {
            continue;
        }
        lots[keep] = one;
        lots.remove(gone);
        keys.remove(gone);
        sides.remove(gone);
    }
    // The same at a rounded corner, its turn spread over the arc's short edges (each under 30°):
    // the two strips' end lots stopped at the arc's ends, two triangles on two diagonals from
    // the corner into the block, a garden wall on each (the village's faubourg corner,
    // 2026-10-17). Two lots whose fronts meet across a short convex run of the block's outline
    // (8 m at most, 30° to 150° in all) and whose backs meet within 3 m make one, the arc and
    // the sliver between their diagonals in it.
    let on_edge = |p: DVec2| {
        (0..n).find(|&e| geom::distance_to_segment(p, block[e], block[(e + 1) % n]) < 1e-3)
    };
    let mut k = 0;
    while k < lots.len() {
        let a = lots[k].clone();
        let partner = (0..lots.len())
            .filter(|&j| j != k && sides[j] != sides[k])
            .find_map(|j| {
                let b = &lots[j];
                let (Some(e), Some(f)) = (on_edge(a[1]), on_edge(b[0])) else {
                    return None;
                };
                if e == f || a.len() < 3 || b.len() < 3 {
                    return None;
                }
                let (th, ti) = (
                    (a[1] - a[0]).normalize_or_zero(),
                    (b[1] - b[0]).normalize_or_zero(),
                );
                if th.perp_dot(ti) <= 0.0 || th.dot(ti) > 0.866 || th.dot(ti) < -0.866 {
                    return None;
                }
                // The outline's corners from `a`'s front's end on to `b`'s front's start.
                let mut run = Vec::new();
                let mut v = (e + 1) % n;
                let mut length = a[1].distance(block[v]);
                while v != (f + 1) % n && run.len() < n {
                    if square[v].is_some() {
                        return None;
                    }
                    run.push(block[v]);
                    if v == f {
                        break;
                    }
                    length += block[v].distance(block[(v + 1) % n]);
                    v = (v + 1) % n;
                }
                length += run.last().map_or(0.0, |&l| l.distance(b[0]));
                if length > 8.0 || a[2].distance(b[b.len() - 1]) > 3.0 {
                    return None;
                }
                let mut one: Vec<DVec2> = vec![a[0], a[1]];
                one.extend(&run);
                one.extend(b.iter().copied());
                one.extend(a[2..].iter().copied());
                let one = geom::clean(&one, 1e-6);
                let clear = (0..lots.len())
                    .filter(|&o| o != k && o != j)
                    .all(|o| !overlapping(&one, &lots[o]));
                (geom::signed_area(&one) > 1.0 && clear).then_some((j, one))
            });
        let Some((j, one)) = partner else {
            k += 1;
            continue;
        };
        let (keep, gone, front) =
            if geom::signed_area(&a).abs() >= geom::signed_area(&lots[j]).abs() {
                (k, j, (a[0], a[1]))
            } else {
                (j, k, (lots[j][0], lots[j][1]))
            };
        lots[keep] = from_the_street(one, front.0, front.1);
        lots.remove(gone);
        keys.remove(gone);
        sides.remove(gone);
        k = 0;
    }
    // The square corners' lots, each its front on the corner's next street first, keyed past
    // every strip's lots.
    let mut key = keys.iter().max().map_or(0, |k| k + 1);
    for (i, sq) in square.iter().enumerate() {
        if let Some((x, on_h, on_i)) = sq.filter(|_| !tips[i]) {
            let lot = vec![block[i], on_i, x, on_h];
            if geom::signed_area(&lot) > 1.0 {
                keys.push(key);
                lots.push(lot);
                key += 1;
            }
        }
    }
    Some((lots, court, keys))
}

/// Whether two polygons share more than their outlines: an edge of one crosses an edge of the
/// other, or a corner of one lies within the other.
fn overlapping(p: &[DVec2], q: &[DVec2]) -> bool {
    let within = |poly: &[DVec2], v: DVec2| {
        let m = poly.len();
        geom::contains(poly, v)
            && (0..m).all(|i| geom::distance_to_segment(v, poly[i], poly[(i + 1) % m]) > 1e-3)
    };
    let crossing = (0..p.len()).any(|i| {
        (0..q.len()).any(|j| {
            geom::segment_intersection(p[i], p[(i + 1) % p.len()], q[j], q[(j + 1) % q.len()])
                .is_some_and(|(t, u)| t > 1e-6 && t < 1.0 - 1e-6 && u > 1e-6 && u < 1.0 - 1e-6)
        })
    });
    crossing || p.iter().any(|&v| within(q, v)) || q.iter().any(|&v| within(p, v))
}

/// Two lots sharing a side (`a`'s `p → q`, `b`'s `q → p`, both anticlockwise) as one, the side
/// gone and the corners left in line dropped. `None` where they share none.
fn joined(a: &[DVec2], b: &[DVec2]) -> Option<Vec<DVec2>> {
    let near = |x: DVec2, y: DVec2| x.distance(y) < 0.01;
    let (n, m) = (a.len(), b.len());
    let (i, j) = (0..n).find_map(|i| {
        let (p, q) = (a[i], a[(i + 1) % n]);
        (0..m)
            .find(|&j| near(b[j], q) && near(b[(j + 1) % m], p))
            .map(|j| (i, j))
    })?;
    // `a` from `q` round to `p`, then `b` on from after `p` round to before `q`.
    let mut out: Vec<DVec2> = (0..n).map(|k| a[(i + 1 + k) % n]).collect();
    out.extend((2..m).map(|k| b[(j + k) % m]));
    let mut k = 0;
    while k < out.len() && out.len() > 3 {
        let len = out.len();
        let (p, c, q) = (out[(k + len - 1) % len], out[k], out[(k + 1) % len]);
        if (c - p).perp_dot(q - c).abs() < 1e-4 * (c - p).length() * (q - c).length()
            || p.distance(c) < 1e-6
        {
            out.remove(k);
        } else {
            k += 1;
        }
    }
    let out = geom::clean(&out, 1e-6);
    (out.len() >= 3 && geom::signed_area(&out) > 1.0).then_some(out)
}

/// `lot` turned to start with its street side (its edge along `a b`), so its first two vertices
/// are its frontage and the next edge and the last its party sides; the clip leaves the vertices
/// starting wherever they fell.
fn from_the_street(mut lot: Vec<DVec2>, a: DVec2, b: DVec2) -> Vec<DVec2> {
    let n = lot.len();
    let on = |p: DVec2| geom::distance_to_segment(p, a, b) < 1e-6;
    if let Some(k) = (0..n).find(|&k| on(lot[k]) && on(lot[(k + 1) % n])) {
        lot.rotate_left(k);
    }
    lot
}

/// A lot's width along its street side (the edge from its first to its second vertex, as
/// [`strips`] lays it), metres.
pub fn frontage(lot: &[DVec2]) -> f64 {
    if lot.len() < 2 {
        return 0.0;
    }
    lot[0].distance(lot[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(w: f64, h: f64) -> Vec<DVec2> {
        vec![
            DVec2::ZERO,
            DVec2::new(w, 0.0),
            DVec2::new(w, h),
            DVec2::new(0.0, h),
        ]
    }

    /// The lots and the court cover the block, every lot has its street side on the block's
    /// outline, and no lot is narrower than the type's least width.
    fn check(block: &[DVec2], street: &[bool], ty: &ParcelType, seed: u64) -> usize {
        let (lots, court) = strips(block, street, ty, seed).expect("a block that takes strips");
        let block_area = geom::signed_area(block).abs();
        let covered: f64 = lots.iter().map(|l| geom::signed_area(l).abs()).sum::<f64>()
            + geom::signed_area(&court).abs();
        let party: f64 = (0..block.len())
            .filter(|&i| !street[i])
            .map(|i| block[i].distance(block[(i + 1) % block.len()]) * ty.depth)
            .sum();
        // The party sides' strips are left out; the rest covers the block.
        assert!(
            (block_area - covered - party).abs() < block_area * 0.02 + party * 0.6,
            "block {block_area:.0}, covered {covered:.0}, party strips about {party:.0}"
        );
        let n = block.len();
        for lot in &lots {
            let (a, b) = (lot[0], lot[1]);
            let on = |p: DVec2| {
                (0..n).any(|i| geom::distance_to_segment(p, block[i], block[(i + 1) % n]) < 1e-6)
            };
            assert!(
                on(a) && on(b),
                "a lot's street side off the outline: {a} {b}"
            );
            assert!(
                frontage(lot) >= ty.width.0 - 1e-6,
                "a lot {:.1} m wide under {:.1}",
                frontage(lot),
                ty.width.0
            );
            // The last lot may take a short remainder, a corner lot its sliver neighbour.
            assert!(frontage(lot) <= 2.0 * ty.width.1 + ty.width.0 + 1e-6);
            for k in 0..lot.len() {
                let (p, q, r) = (
                    lot[(k + lot.len() - 1) % lot.len()],
                    lot[k],
                    lot[(k + 1) % lot.len()],
                );
                let (u, v) = ((p - q).normalize(), (r - q).normalize());
                let angle = u.dot(v).clamp(-1.0, 1.0).acos().to_degrees();
                assert!(angle > 30.0, "a lot with a {angle:.0}° corner");
            }
        }
        lots.len()
    }

    #[test]
    fn a_rectangle_cuts_into_strips_of_village_lots() {
        let block = rectangle(100.0, 60.0);
        let n = check(&block, &[true; 4], &ParcelType::VILLAGE, 7);
        // Four corner lots of 14 by 14 m, then about 208 m of street between them at 7 m a lot.
        assert!((28..=40).contains(&n), "{n} lots");
    }

    #[test]
    fn a_trapezoid_keeps_its_party_walls_square_to_the_street() {
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let n = check(&block, &[true, true, true, true], &ParcelType::BLOCK, 11);
        assert!((14..=30).contains(&n), "{n} lots");
        let (lots, _) = strips(&block, &[true; 4], &ParcelType::BLOCK, 11).expect("strips");
        // Each lot's two party sides run square to its street, but the corner lots', which
        // take the corners' bisectors (an acute corner gives a flatiron, as in a town).
        let corner = |lot: &Vec<DVec2>| {
            lot.iter()
                .any(|&p| block.iter().any(|&q| p.distance(q) < 1e-6))
        };
        let mut inner = 0;
        for lot in lots.iter().filter(|l| !corner(l)) {
            let t = (lot[1] - lot[0]).normalize();
            for side in [
                (lot[2] - lot[1]).normalize(),
                (lot[0] - lot[lot.len() - 1]).normalize(),
            ] {
                assert!(
                    t.dot(side).abs() < 0.02,
                    "a party side {:.1}° off the square",
                    t.dot(side).acos().to_degrees() - 90.0
                );
            }
            inner += 1;
        }
        assert!(inner >= 6, "{inner} inner lots");
    }

    /// With `PARCELS_PNG=path`, the trapezoid's lots drawn there for a look.
    #[test]
    fn the_trapezoid_is_drawn_on_request() {
        let Ok(path) = std::env::var("PARCELS_PNG") else {
            return;
        };
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let (lots, court) = strips(&block, &[true; 4], &ParcelType::VILLAGE, 11).expect("strips");
        super::super::map::write_parcels(&block, &lots, &court, std::path::Path::new(&path), 0.1)
            .expect("the drawing");
    }

    #[test]
    fn a_party_side_is_left_to_the_court() {
        let block = rectangle(80.0, 50.0);
        let (lots, _) =
            strips(&block, &[true, false, true, false], &ParcelType::VILLAGE, 3).expect("strips");
        assert!(
            lots.iter()
                .all(|l| l[0].y.abs() < 1e-9 || (l[0].y - 50.0).abs() < 1e-9)
        );
    }

    #[test]
    fn a_block_too_thin_for_the_depth_takes_no_strips() {
        assert!(strips(&rectangle(100.0, 20.0), &[true; 4], &ParcelType::VILLAGE, 1).is_none());
    }

    #[test]
    fn lanieres_take_the_road_outs_side_to_their_full_depth() {
        let block = rectangle(90.0, 70.0);
        let ty = ParcelType {
            depth: 40.0,
            ..ParcelType::FAUBOURG
        };
        // The road out along the bottom only: every lot 40 m deep, the corner ones too.
        let only_bottom = [true, false, false, false];
        let (lots, court, _) =
            strips_by(&block, &only_bottom, &[40.0, 0.0, 0.0, 0.0], &ty, |_| 8.0)
                .expect("lanières");
        let area: f64 = lots.iter().map(|l| geom::signed_area(l).abs()).sum();
        assert!((area - 90.0 * 40.0).abs() < 1.0, "{area:.0} m² of lots");
        for lot in &lots {
            let full = frontage(lot) * 40.0;
            assert!((geom::signed_area(lot).abs() - full).abs() < 1.0);
        }
        assert!((geom::signed_area(&court) - 90.0 * 30.0).abs() < 20.0);
        // Shallow strips on the sides too: each corner splits where its depth lines meet, and
        // the lots and the court still tile the block without overlapping.
        let street = [true, true, false, true];
        let (lots, court, _) = strips_by(&block, &street, &[40.0, 12.0, 0.0, 12.0], &ty, |_| 8.0)
            .expect("lanières and side lots");
        let covered: f64 = lots.iter().map(|l| geom::signed_area(l).abs()).sum::<f64>()
            + geom::signed_area(&court).abs();
        assert!(
            (covered - 90.0 * 70.0).abs() < 20.0,
            "{covered:.0} of 6300 m²"
        );
        let bottom: f64 = lots
            .iter()
            .filter(|l| l[0].y.abs() < 1e-9 && l[1].y.abs() < 1e-9)
            .map(|l| geom::signed_area(l).abs())
            .sum();
        assert!(
            (bottom - 3120.0).abs() < 2.0,
            "{bottom:.0} m² on the road out"
        );
    }
}
