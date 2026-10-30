//! The village's lots, cut per street side (D-083's fifth step, #308). The owner's review of
//! 2026-10-29: lots "close to rectangles", "a single straight hedge" where two rows meet, square
//! corners, one street's lots taking a block's corner up to the other street and the other's
//! starting at their back line, the rest of the block a common court of some width. The strips
//! cut edge by edge and mended after (leftovers absorbed, slivers moved, courts dissolved) made a
//! new case of every fix; here the shapes come from a few rules, so the bad ones cannot occur:
//!
//! - **Sides.** The block's outline splits into street sides at its corners (a turn of
//!   [`CORNER`] or more); a side bends less within it. A side shorter than [`ARC`] between two
//!   corners is a rounded corner's arc, and keeps no lots.
//! - **Lot lines** run square to their side (to its direction smoothed over a few metres where it
//!   bends), each at a cut along the street, the widths the households want.
//! - **One back line a side:** the side offset by the depth, its lots' backs on it, so a row's
//!   backs never step.
//! - **Corners:** of two sides meeting at a corner, the longer takes it: its end lot runs round
//!   the corner to the other street, and the other's lots start at its back line.
//! - **The court** is what is left: one piece, a share of the block and [`COURT_WIDTH`] wide
//!   everywhere, else the caller cuts shallower, or lays the two longest facing sides back to
//!   back, both rows running to one straight line between them ([`Back::Spine`]).

use glam::DVec2;

use super::geom;
use super::ground;
use super::parcels::ParcelType;

/// The cosine of the least turn of a block's outline that makes a corner (30°): a smaller turn
/// is a bend within a side.
const CORNER_COS: f64 = 0.866_025_403_784_438_6;
/// The shortest side between two corners that keeps lots of its own, metres: shorter, it is a
/// rounded corner's arc, and the side that takes the corner runs round it.
pub const ARC: f64 = 8.0;
/// The narrowest a court may be anywhere, metres: an olive's crown with a path either side.
pub const COURT_WIDTH: f64 = 8.0;
/// The least share of its block a court takes, else the block is cut shallower or back to back.
pub const COURT_SHARE: f64 = 0.15;
/// How near a bend of the street a cut moves onto it, metres: a lot's front then bends at its
/// corner, not in its middle.
const SNAP: f64 = 3.5;
/// How many times a side starved of lots may change who takes its corners.
const PASSES: usize = 4;
/// How far before and after a point the outline's way is taken to find a rounded corner, metres.
const ROUND: f64 = 6.0;
/// The share of the type's least width a lot keeps when its cut moves onto a bend of its street.
const BEND_WIDTH: f64 = 0.85;
/// How far a back line runs on past its side's ends, metres: past any block.
const FAR: f64 = 1000.0;

/// Where a block's lots end at the back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Back {
    /// At this depth from their street, the rest of the block the court.
    Depth(f64),
    /// Two facing sides back to back, their lots running to one straight line between them; the
    /// other sides' lots, if any, at this depth. The pairs facing each other (within 60° of
    /// opposite) ranked by their shorter side's length, longest first: which one.
    Spine(f64, usize),
}

/// A block cut into lots.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The lots, anticlockwise, each its street front first (from its first vertex to its
    /// second).
    pub lots: Vec<Vec<DVec2>>,
    /// Each lot's key: the household it was cut for.
    pub keys: Vec<usize>,
    /// Each lot's street side, counted from the block's first corner.
    pub sides: Vec<usize>,
    /// The court, anticlockwise; empty where the rows meet back to back.
    pub court: Vec<DVec2>,
    /// The ground no lot and not the court holds, m² (a fault: none by construction).
    pub left: f64,
    /// The line the rows meet on, where they meet back to back.
    pub spine: Option<(DVec2, DVec2)>,
}

impl Layout {
    /// Whether its court is one the village keeps: at least [`COURT_SHARE`] of the block's
    /// `area`, and nowhere narrower than [`COURT_WIDTH`].
    pub fn court_fits(&self, area: f64) -> bool {
        self.court.len() >= 3
            && geom::signed_area(&self.court).abs() >= COURT_SHARE * area
            && court_is_wide(&self.court)
            && self.left < 1.0
    }

    /// Whether the rows met back to back leave no court, or one nowhere narrower than
    /// [`COURT_WIDTH`], and nothing else of the block.
    pub fn back_to_back_fits(&self) -> bool {
        self.left < 1.0 && (self.court.len() < 3 || court_is_wide(&self.court))
    }
}

/// Whether a court is nowhere narrower than [`COURT_WIDTH`]: what of it lies nearer its outline
/// than half that, from every point at least that far in, is no more than its corners make
/// (and 12 m², the sampling's error).
pub fn court_is_wide(court: &[DVec2]) -> bool {
    thin_area(court, COURT_WIDTH) < corners_area(court, COURT_WIDTH) + 12.0
}

/// A street side: the block's outline from one corner to the next.
#[derive(Clone, Debug)]
struct Side {
    /// Its first corner, an index into the outline.
    first: usize,
    /// Its last corner.
    last: usize,
    /// Its points along the street, from corner to corner.
    front: Vec<DVec2>,
    /// Its length, metres.
    length: f64,
    /// Whether it is cut into lots.
    lots: bool,
}

/// An open polyline with its points' distances along it.
#[derive(Clone, Debug)]
struct Line {
    points: Vec<DVec2>,
    at: Vec<f64>,
}

impl Line {
    fn new(points: Vec<DVec2>) -> Self {
        let mut at = Vec::with_capacity(points.len());
        let mut s = 0.0;
        for (k, p) in points.iter().enumerate() {
            if k > 0 {
                s += points[k - 1].distance(*p);
            }
            at.push(s);
        }
        Line { points, at }
    }

    fn length(&self) -> f64 {
        self.at.last().copied().unwrap_or(0.0)
    }

    fn point(&self, s: f64) -> DVec2 {
        geom::point_at(&self.points, s)
    }

    /// Its points strictly between distances `from` and `to`, in the order from one to the other.
    fn between(&self, from: f64, to: f64) -> Vec<DVec2> {
        let (lo, hi) = (from.min(to), from.max(to));
        let mut out: Vec<DVec2> = self
            .points
            .iter()
            .zip(&self.at)
            .filter(|&(_, &s)| s > lo + 1e-6 && s < hi - 1e-6)
            .map(|(&p, _)| p)
            .collect();
        if from > to {
            out.reverse();
        }
        out
    }

    /// Where the ray from `origin` along `dir` first meets it: its distance along the ray and
    /// along the line.
    fn hit(&self, origin: DVec2, dir: DVec2) -> Option<(f64, f64)> {
        let far = origin + dir * (2.0 * FAR);
        let mut best: Option<(f64, f64)> = None;
        for k in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[k], self.points[k + 1]);
            if let Some((t, u)) = crossing(origin, far, a, b) {
                let along = t * 2.0 * FAR;
                if along > 1e-6 && best.is_none_or(|(x, _)| along < x) {
                    best = Some((along, self.at[k] + u * a.distance(b)));
                }
            }
        }
        best
    }

    /// Where it crosses `other`: each crossing's distance along this line and along the other.
    fn crossings(&self, other: &Line) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for k in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[k], self.points[k + 1]);
            for j in 0..other.points.len().saturating_sub(1) {
                let (c, d) = (other.points[j], other.points[j + 1]);
                if let Some((t, u)) = crossing(a, b, c, d) {
                    out.push((
                        self.at[k] + t * a.distance(b),
                        other.at[j] + u * c.distance(d),
                    ));
                }
            }
        }
        out.sort_by(|x, y| x.0.total_cmp(&y.0));
        // A crossing on a shared point, found on both segments, once.
        out.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-6);
        out
    }

    /// Its point nearest `p`, as its distance along it.
    fn nearest(&self, p: DVec2) -> f64 {
        let mut best = (f64::MAX, 0.0);
        for k in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[k], self.points[k + 1]);
            let t = geom::closest_on_segment(p, a, b);
            let d = p.distance(a + (b - a) * t);
            if d < best.0 {
                best = (d, self.at[k] + t * a.distance(b));
            }
        }
        best.1
    }
}

/// Where segments `p0 p1` and `q0 q1` cross, as their parameters, a crossing on a segment's end
/// within 1e-9 counted (a lot line aimed at a back line's corner fell between its two segments).
fn crossing(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> Option<(f64, f64)> {
    let (r, s) = (p1 - p0, q1 - q0);
    let denom = geom::cross(r, s);
    if denom.abs() < 1e-12 {
        return None;
    }
    let qp = q0 - p0;
    let (t, u) = (geom::cross(qp, s) / denom, geom::cross(qp, r) / denom);
    const E: f64 = 1e-9;
    ((-E..=1.0 + E).contains(&t) && (-E..=1.0 + E).contains(&u))
        .then_some((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
}

/// `front` offset by `depth` to its left (inside an anticlockwise block), its corners mitred,
/// run on [`FAR`] past both ends.
fn back_line(front: &[DVec2], depth: f64) -> Line {
    let m = front.len();
    let dir = |k: usize| (front[k + 1] - front[k]).normalize_or_zero();
    let left = |t: DVec2| DVec2::new(-t.y, t.x);
    // Its ends run on in line (no point at the side's own ends).
    let mut points = vec![front[0] + left(dir(0)) * depth - dir(0) * FAR];
    for (k, &bend) in front.iter().enumerate().take(m - 1).skip(1) {
        let (t0, t1) = (dir(k - 1), dir(k));
        // The two offset lines meet on the bisector, depth / cos(half the turn) from the bend.
        let bisector = (left(t0) + left(t1)).normalize_or_zero();
        let cos_half = bisector.dot(left(t0)).max(0.2);
        points.push(bend + bisector * (depth / cos_half));
    }
    points.push(front[m - 1] + left(dir(m - 2)) * depth + dir(m - 2) * FAR);
    // A bend's mitre past the next one (an edge shorter than the mitres): the points out of order
    // dropped.
    let mut kept: Vec<DVec2> = Vec::with_capacity(points.len());
    for p in points {
        while kept.len() >= 2 {
            let (a, b) = (kept[kept.len() - 2], kept[kept.len() - 1]);
            if (b - a).dot(p - b) < 0.0 {
                kept.pop();
            } else {
                break;
            }
        }
        kept.push(p);
    }
    Line::new(kept)
}

/// The straight line two facing sides' rows meet on: where the distances to the two sides' chords
/// stand in proportion to their depths (halfway between them for equal depths), run along the
/// first side's way.
fn spine_line(a: &[DVec2], c: &[DVec2]) -> Option<Line> {
    let chord = |s: &[DVec2]| {
        let (p, q) = (s[0], s[s.len() - 1]);
        let t = (q - p).normalize_or_zero();
        (p, DVec2::new(-t.y, t.x))
    };
    let ((pa, na), (pc, nc)) = (chord(a), chord(c));
    // Equal depths: {x : na·(x − pa) = nc·(x − pc)}.
    let g = na - nc;
    if g.length() < 1e-6 {
        return None;
    }
    let k = na.dot(pa) - nc.dot(pc);
    let origin = g * (k / g.length_squared());
    let mut along = DVec2::new(-g.y, g.x).normalize();
    if along.dot(a[a.len() - 1] - a[0]) < 0.0 {
        along = -along;
    }
    Some(Line::new(vec![origin - along * FAR, origin + along * FAR]))
}

/// The inward direction of a side's lot line at distance `s` along its `front`: square to the
/// street there, halfway between the two edges' squares at a bend. The houses are laid square to
/// their lot's front, and their depth measured so: lines leaning a few degrees off it (averaged
/// over a bend) cut the houses short at their lots' sides.
fn square_at(front: &Line, s: f64) -> DVec2 {
    let left = |t: DVec2| DVec2::new(-t.y, t.x);
    let edge = |k: usize| (front.points[k + 1] - front.points[k]).normalize_or_zero();
    let last = front.points.len() - 2;
    for k in 1..=last {
        if (front.at[k] - s).abs() < 1e-6 {
            return (left(edge(k - 1)) + left(edge(k))).normalize_or_zero();
        }
    }
    let k = (0..=last).find(|&k| s < front.at[k + 1]).unwrap_or(last);
    left(edge(k))
}

/// How near a cut must come to a bend of `front` at its point `k` to move onto it, metres: lines
/// square to the street either side of a bend that turns inward close in before the back (at
/// `depth`), their lot a wedge.
fn snap_zone(front: &Line, k: usize, depth: f64) -> f64 {
    let p = &front.points;
    let (u, v) = (
        (p[k] - p[k - 1]).normalize_or_zero(),
        (p[k + 1] - p[k]).normalize_or_zero(),
    );
    let (sin, cos) = (geom::cross(u, v), u.dot(v));
    if sin <= 0.0 {
        return SNAP;
    }
    // tan(half the turn) · depth: where the two edges' squares from the bend's sides meet the back.
    SNAP.max(depth * sin / (1.0 + cos) + 0.5)
}

/// A block `outline` (anticlockwise, simplified) cut into lots on its sides whose edges are
/// streets (`street[e]` for edge `e`, from vertex `e` to the next), their backs at `back`, each
/// lot as wide as `width` asks for its key (within the type's widths: the last of a side takes
/// what is left). `None` where a side's lines cannot be laid (a back line out of reach) or the
/// lots would overlap.
pub fn cut(
    outline: &[DVec2],
    street: &[bool],
    back: Back,
    ty: &ParcelType,
    width: impl FnMut(usize) -> f64,
) -> Option<Layout> {
    cut_or_why(outline, street, back, ty, width).ok()
}

/// [`cut`], or why it cannot be cut.
pub fn cut_or_why(
    outline: &[DVec2],
    street: &[bool],
    back: Back,
    ty: &ParcelType,
    mut width: impl FnMut(usize) -> f64,
) -> Result<Layout, &'static str> {
    let n = outline.len();
    if n < 3 || street.len() != n || geom::signed_area(outline) <= 0.0 {
        return Err("not a block");
    }
    let dir = |e: usize| (outline[(e + 1) % n] - outline[e]).normalize_or_zero();
    // The outline's way over [`ROUND`] metres before and after corner `v`: a lane's rounded
    // corner turns a little at each of its points, and one side running round it fanned its lots
    // out into wedges and L's.
    let walk = |v: usize, ahead: bool| {
        let (mut p, mut left, mut i) = (outline[v], ROUND, v);
        for _ in 0..n {
            let q = if ahead {
                outline[(i + 1) % n]
            } else {
                outline[(i + n - 1) % n]
            };
            let d = p.distance(q);
            if d >= left {
                return p + (q - p) * (left / d.max(1e-9));
            }
            left -= d;
            p = q;
            i = if ahead { (i + 1) % n } else { (i + n - 1) % n };
        }
        p
    };
    let turned = |v: usize| {
        let (before, after) = (
            (outline[v] - walk(v, false)).normalize_or_zero(),
            (walk(v, true) - outline[v]).normalize_or_zero(),
        );
        before.dot(after) < CORNER_COS
    };
    let sharp = |v: usize| {
        let h = (v + n - 1) % n;
        dir(h).dot(dir(v)) < CORNER_COS || street[h] != street[v]
    };
    // A corner's point, or the first or last point of a run of points that turn together (a
    // rounded corner, whose edges between make a short side, its arc).
    let corner = |v: usize| {
        let (h, j) = ((v + n - 1) % n, (v + 1) % n);
        sharp(v) || (turned(v) && !(turned(h) && turned(j)))
    };
    let firsts: Vec<usize> = (0..n).filter(|&v| corner(v)).collect();
    if firsts.is_empty() {
        return Err("no corner");
    }
    let depth = match back {
        Back::Depth(d) | Back::Spine(d, _) => d,
    };
    let sides: Vec<Side> = firsts
        .iter()
        .enumerate()
        .map(|(k, &first)| {
            let last = firsts[(k + 1) % firsts.len()];
            let count = (last + n - first - 1) % n + 1;
            let front: Vec<DVec2> = (0..=count).map(|i| outline[(first + i) % n]).collect();
            let length = geom::polyline_length(&front);
            // A side between two corners each turning under 60°, shorter than the depth, is a
            // corner cut off (a lane's chamfer): its lots were wedges between the two streets.
            let gentle = |v: usize| dir((v + n - 1) % n).dot(dir(v)) > 0.5;
            let chamfer = gentle(first) && gentle(last) && length < depth;
            Side {
                first,
                last,
                front,
                length,
                lots: street[first] && length >= ARC.max(ty.width.0) && !chamfer,
            }
        })
        .collect();
    let m = sides.len();
    let arc = |k: usize| !sides[k].lots && street[sides[k].first] && sides[k].length < depth;
    // Two sides meeting at a corner (or round an arc): the next side with lots after each, where
    // the outline turns left (a convex corner) between them.
    let turning = |from: usize, to: usize| {
        let (a, b) = (&sides[from], &sides[to]);
        let ta = (a.front[a.front.len() - 1] - a.front[a.front.len() - 2]).normalize_or_zero();
        let tb = (b.front[1] - b.front[0]).normalize_or_zero();
        geom::cross(ta, tb) > 0.0
    };
    let next_of = |k: usize, cut: &[bool]| -> Option<usize> {
        let j = (k + 1) % m;
        if cut[j] {
            return Some(j);
        }
        let i = (k + 2) % m;
        (arc(j) && cut[i] && i != k).then_some(i)
    };
    // The spine's two sides: the longest facing pair (their chords within 60° of opposite).
    let chord = |s: &Side| (s.front[s.front.len() - 1] - s.front[0]).normalize_or_zero();
    let pair = if let Back::Spine(_, rank) = back {
        let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
        for a in (0..m).filter(|&a| sides[a].lots) {
            for c in (a + 1..m).filter(|&c| sides[c].lots) {
                if chord(&sides[a]).dot(chord(&sides[c])) < -0.5 {
                    pairs.push((a, c, sides[a].length.min(sides[c].length)));
                }
            }
        }
        pairs.sort_by(|x, y| y.2.total_cmp(&x.2).then(x.0.cmp(&y.0)).then(x.1.cmp(&y.1)));
        let &(a, c, _) = pairs.get(rank).ok_or("no facing sides")?;
        Some((a, c))
    } else {
        None
    };
    let spine = pair.and_then(|(a, c)| spine_line(&sides[a].front, &sides[c].front));
    if pair.is_some() && spine.is_none() {
        return Err("no spine");
    }
    let in_pair = |k: usize| pair.is_some_and(|(a, c)| k == a || k == c);
    // Each side's back line: its offset, or the spine.
    let backs: Vec<Line> = (0..m)
        .map(|k| match (&spine, pair) {
            (Some(line), Some((a, _))) if in_pair(k) => {
                if k == a {
                    line.clone()
                } else {
                    Line::new(line.points.iter().rev().copied().collect())
                }
            }
            _ => back_line(&sides[k].front, depth),
        })
        .collect();
    let fronts: Vec<Line> = sides.iter().map(|s| Line::new(s.front.clone())).collect();
    // Where a corner's loser `l` and its taker `t` part (at `l`'s first corner, or its last): the
    // taker's back line, from where it crosses `l`'s street to where `l`'s own back line meets
    // it. That crossing's distance along `l`'s street, the meeting point, its distance along
    // `l`'s back line, and the taker's back line's points between the two.
    let meet = |l: usize, t: usize, first: bool| -> Option<(f64, DVec2, f64, Vec<DVec2>)> {
        let c = backs[l].crossings(&backs[t]);
        let &(u, u_t) = if first { c.first()? } else { c.last()? };
        let x = fronts[l].crossings(&backs[t]);
        let &(s, s_t) = if first { x.first()? } else { x.last()? };
        let along = if first {
            backs[t].between(u_t, s_t)
        } else {
            backs[t].between(s_t, u_t)
        };
        Some((s, backs[l].point(u), u, along))
    };
    // Who takes each corner: `end_to[k]` the side that took side k's last corner, `start_to[k]`
    // its first's. The spine's sides take theirs; else the longer side.
    let mut on: Vec<bool> = sides.iter().map(|s| s.lots).collect();
    let mut end_to: Vec<Option<usize>> = vec![None; m];
    let mut start_to: Vec<Option<usize>> = vec![None; m];
    let mut ranges: Vec<Option<(f64, f64)>> = vec![None; m];
    let mut flipped: Vec<(usize, usize)> = Vec::new();
    for pass in 0..=PASSES {
        end_to = vec![None; m];
        start_to = vec![None; m];
        for k in (0..m).filter(|&k| on[k]) {
            let Some(j) = next_of(k, &on) else {
                continue;
            };
            if j == k || !turning(k, j) {
                continue;
            }
            let k_takes = match (in_pair(k), in_pair(j)) {
                (true, false) => true,
                (false, true) => false,
                _ => sides[k].length >= sides[j].length,
            };
            let k_takes = k_takes != flipped.contains(&(k, j));
            if k_takes {
                start_to[j] = Some(k);
            } else {
                end_to[k] = Some(j);
            }
        }
        // Each side's free stretch: from where the back line of the side that took its first
        // corner crosses its street, to where the one that took its last does.
        for k in 0..m {
            ranges[k] = None;
            if !on[k] {
                continue;
            }
            let s0 = match start_to[k] {
                Some(t) => meet(k, t, true).map(|c| c.0),
                None => Some(0.0),
            };
            let s1 = match end_to[k] {
                Some(t) => meet(k, t, false).map(|c| c.0),
                None => Some(fronts[k].length()),
            };
            if let (Some(s0), Some(s1)) = (s0, s1) {
                ranges[k] = Some((s0, s1));
            }
        }
        // A side left less than a lot between two corners it lost takes one of them, the one
        // whose taker is longer, unless that starves the taker; else it keeps no lots. The last
        // pass only lays the corners and the stretches out for what the others decided.
        if pass == PASSES {
            break;
        }
        let mut changed = false;
        for k in 0..m {
            if !on[k] {
                continue;
            }
            let free = ranges[k].map_or(0.0, |(s0, s1)| s1 - s0);
            if free >= ty.width.0 {
                continue;
            }
            let lost: Vec<(usize, usize)> =
                [start_to[k].map(|t| (t, k)), end_to[k].map(|t| (k, t))]
                    .into_iter()
                    .flatten()
                    .filter(|&(a, b)| !flipped.contains(&(a, b)) && !in_pair(a) && !in_pair(b))
                    .collect();
            let other = |&(a, b): &(usize, usize)| if a == k { b } else { a };
            if let Some(&corner) = lost
                .iter()
                .max_by(|x, y| sides[other(x)].length.total_cmp(&sides[other(y)].length))
                && sides[other(&corner)].length - depth >= 2.0 * ty.width.0
            {
                flipped.push(corner);
            } else {
                on[k] = false;
            }
            changed = true;
            break;
        }
        if !changed {
            break;
        }
    }
    let mut lots = Vec::new();
    let mut keys = Vec::new();
    let mut side_of = Vec::new();
    for k in 0..m {
        if !on[k] {
            continue;
        }
        let Some((s0, s1)) = ranges[k] else {
            continue;
        };
        if s1 - s0 < ty.width.0 {
            continue;
        }
        let front = &fronts[k];
        let back = &backs[k];
        // Where its first and last lots' backs end: on the back line of the side that took the
        // corner, or where its own meets the block.
        let ends = (
            match start_to[k] {
                Some(t) => meet(k, t, true).map(|(_, y, _, _)| y),
                None => round(outline, sides[k].first, back, false).map(|(h, _)| h[0]),
            },
            match end_to[k] {
                Some(t) => meet(k, t, false).map(|(_, y, _, _)| y),
                None => round(outline, sides[k].last, back, true).map(|(t, _)| t[t.len() - 1]),
            },
        );
        let (Some(y0), Some(y1)) = ends else {
            return Err("the back never meets the block");
        };
        // The cuts: a width drawn each time, the last lot taking what is left (no less than a
        // lot); each moved onto a bend of the street within SNAP where its lots keep their least
        // width. Measured from whichever of an end lot's front and back is the shorter: where a
        // corner is off square, the end lot's side runs aslant and its back or front narrows.
        let (lo, hi) = (s0.max(front.nearest(y0)), s1.min(front.nearest(y1)));
        let mut cuts = vec![s0];
        let mut s = lo;
        loop {
            let w = width(lots.len() + cuts.len() - 1).max(0.5);
            if s + w + ty.width.0 > hi {
                break;
            }
            s += w;
            cuts.push(s);
        }
        cuts.push(s1);
        for c in 1..cuts.len() - 1 {
            let bend = (1..front.at.len() - 1)
                .filter(|&b| (front.at[b] - cuts[c]).abs() < snap_zone(front, b, depth))
                .map(|b| front.at[b])
                .min_by(|x, y| (x - cuts[c]).abs().total_cmp(&(y - cuts[c]).abs()));
            // A lot may come out a little narrower than the type's least: left past the bend, the
            // cut left the next lot a stub of street a few decimetres long beyond it.
            let least = ty.width.0 * BEND_WIDTH;
            if let Some(b) = bend
                && b - cuts[c - 1] >= least
                && cuts[c + 1] - b >= least
            {
                cuts[c] = b;
            }
        }
        // Each cut's line from the street to the back line: where it meets it, along the back.
        let mut meets = Vec::with_capacity(cuts.len());
        for &c in &cuts {
            let p = front.point(c);
            let (_, u) = back
                .hit(p, square_at(front, c))
                .ok_or("a lot line misses its back")?;
            meets.push((p, u));
        }
        for j in 0..cuts.len() - 1 {
            let (c0, c1) = (cuts[j], cuts[j + 1]);
            // Its front, start to end.
            let mut ring: Vec<DVec2> = vec![front.point(c0)];
            ring.extend(front.between(c0, c1));
            ring.push(front.point(c1));
            let f = ring.len();
            // From its front's end round to the back line.
            let u_end = if j + 2 < cuts.len() {
                ring.push(back.point(meets[j + 1].1));
                meets[j + 1].1
            } else if let Some(t) = end_to[k] {
                // Up the taker's back line to where this one meets it.
                let (_, y, u, along) = meet(k, t, false).ok_or("two backs do not meet")?;
                ring.extend(along);
                ring.push(y);
                u
            } else {
                // Round the corner along the block to where the back line meets it.
                let (tail, u) = round(outline, sides[k].last, back, true)
                    .ok_or("the back never meets the block")?;
                ring.extend(tail);
                u
            };
            let u_start = if j > 0 {
                meets[j].1
            } else if let Some(t) = start_to[k] {
                // Along its back to where it meets the taker's, then down that to the street.
                let (_, y, u, along) = meet(k, t, true).ok_or("two backs do not meet")?;
                ring.extend(back.between(u_end, u));
                ring.push(y);
                ring.extend(along);
                lots.push(finish(ring, f));
                keys.push(lots.len() - 1);
                side_of.push(k);
                continue;
            } else {
                let (head, u) = round(outline, sides[k].first, back, false)
                    .ok_or("the back never meets the block")?;
                ring.extend(back.between(u_end, u));
                ring.push(back.point(u));
                ring.extend(head);
                lots.push(finish(ring, f));
                keys.push(lots.len() - 1);
                side_of.push(k);
                continue;
            };
            ring.extend(back.between(u_end, u_start));
            ring.push(back.point(u_start));
            lots.push(finish(ring, f));
            keys.push(lots.len() - 1);
            side_of.push(k);
        }
    }
    // A corner two lots share, found twice (where a side's back line crosses the street, and
    // where the outline walked round the corner meets it), made one point: 1e-8 apart, the two
    // lots' union came out in two pieces.
    let mut seen: Vec<DVec2> = Vec::new();
    for lot in &mut lots {
        for p in lot.iter_mut() {
            match seen.iter().find(|q| q.distance(*p) < 1e-5) {
                Some(&q) => *p = q,
                None => seen.push(*p),
            }
        }
        *lot = geom::clean(lot, 1e-6);
    }
    if lots
        .iter()
        .any(|l| l.len() < 3 || geom::signed_area(l) <= 1.0)
    {
        return Err("a lot of no area");
    }
    if lots.iter().any(|l| !simple(l)) {
        return Err("a lot crossing itself");
    }
    if lots.iter().any(|l| squareness(l) < SQUARE) {
        return Err("a lot far from a rectangle");
    }
    // The court: the block less its lots; the lots never overlap.
    let area = geom::signed_area(outline);
    let lots_area: f64 = lots.iter().map(|l| geom::signed_area(l)).sum();
    let mut pieces: Vec<Vec<DVec2>> = ground::difference(outline, &lots)
        .into_iter()
        .filter_map(|shape| shape.into_iter().next())
        .map(|p| geom::clean(&p, 1e-6))
        .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() > 0.05)
        .collect();
    let rest: f64 = pieces.iter().map(|p| geom::signed_area(p).abs()).sum();
    if lots_area + rest > area + 1.0 {
        return Err("the lots overlap");
    }
    pieces.sort_by(|a, b| {
        geom::signed_area(b)
            .abs()
            .total_cmp(&geom::signed_area(a).abs())
    });
    let court = pieces.first().cloned().unwrap_or_default();
    let mut court = if geom::signed_area(&court).abs() >= 1.0 {
        court
    } else {
        Vec::new()
    };
    if geom::signed_area(&court) < 0.0 {
        court.reverse();
    }
    let left = rest - geom::signed_area(&court).abs();
    let spine = spine.map(|l| (l.points[0], l.points[1]));
    Ok(Layout {
        lots,
        keys,
        sides: side_of,
        court,
        left,
        spine,
    })
}

/// Whether a closed polygon's edges meet only where they follow each other.
pub fn simple(polygon: &[DVec2]) -> bool {
    let n = polygon.len();
    (0..n).all(|i| {
        (i + 2..n).all(|j| {
            if (j + 1) % n == i {
                return true;
            }
            geom::segment_intersection(
                polygon[i],
                polygon[(i + 1) % n],
                polygon[j],
                polygon[(j + 1) % n],
            )
            .is_none()
        })
    })
}

/// The least share of the rectangle round a lot (square to its front) that the lot fills: a
/// triangle fills half, a lot whose side runs 30° off square about 0.85.
pub const SQUARE: f64 = 0.65;

/// How much of the tightest rectangle round `lot` it fills, the rectangle square to one of its
/// edges: 1 for a rectangle, about 0.9 for a lot whose sides lean 10° at a bend of the street.
pub fn squareness(lot: &[DVec2]) -> f64 {
    let area = geom::signed_area(lot).abs();
    let m = lot.len();
    (0..m)
        .map(|k| {
            let t = (lot[(k + 1) % m] - lot[k]).normalize_or_zero();
            let n = DVec2::new(-t.y, t.x);
            let (lo, hi) = lot.iter().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(lo, hi), &p| {
                    let q = DVec2::new(p.dot(t), p.dot(n));
                    (lo.min(q), hi.max(q))
                },
            );
            let boxed = (hi.x - lo.x) * (hi.y - lo.y);
            if boxed <= 0.0 { 0.0 } else { area / boxed }
        })
        .fold(0.0, f64::max)
}

/// A lot's `ring` (its first `front` points along the street) cleaned and turned to start at its
/// longest stretch of street: a house is laid along its lot's first edge, and a front bent near
/// its start left it no room.
fn finish(mut ring: Vec<DVec2>, front: usize) -> Vec<DVec2> {
    if let Some(k) = (0..front.saturating_sub(1))
        .max_by(|&a, &b| {
            ring[a]
                .distance(ring[a + 1])
                .total_cmp(&ring[b].distance(ring[b + 1]))
                .then(b.cmp(&a))
        })
        .filter(|&k| k > 0)
    {
        ring.rotate_left(k);
    }
    geom::clean(&ring, 1e-6)
}

/// The block's outline from corner `from` round to where `back` meets it: forward (`ahead`, the
/// points after the corner up to the meeting, which ends the list) or backward (the meeting first,
/// then the points up to the corner, excluded); and the meeting's distance along `back`.
fn round(outline: &[DVec2], from: usize, back: &Line, ahead: bool) -> Option<(Vec<DVec2>, f64)> {
    let n = outline.len();
    let mut passed = Vec::new();
    for i in 0..n {
        let (a, b) = if ahead {
            (outline[(from + i) % n], outline[(from + i + 1) % n])
        } else {
            (
                outline[(from + n - i) % n],
                outline[(from + 2 * n - i - 1) % n],
            )
        };
        let edge = Line::new(vec![a, b]);
        if let Some(&(t, u)) = edge.crossings(back).first() {
            let x = edge.point(t);
            if ahead {
                passed.push(x);
            } else {
                passed.push(x);
                passed.reverse();
            }
            return Some((passed, u));
        }
        passed.push(b);
    }
    None
}

/// What [`thin_area`] finds of an anticlockwise polygon `width` wide everywhere: at each corner
/// turning by under 120° (60° or more inside), the corner's point beyond the circle of
/// `width / 2` set into it, r²(tan φ − φ) for half the turn φ. Nothing at a sharper corner, whose
/// tip counts as thin.
pub fn corners_area(polygon: &[DVec2], width: f64) -> f64 {
    let n = polygon.len();
    let r = width * 0.5;
    (0..n)
        .map(|i| {
            let u = (polygon[i] - polygon[(i + n - 1) % n]).normalize_or_zero();
            let v = (polygon[(i + 1) % n] - polygon[i]).normalize_or_zero();
            let (sin, cos) = (geom::cross(u, v), u.dot(v));
            if sin <= 0.0 || cos < -0.5 {
                return 0.0;
            }
            let tan_half = sin / (1.0 + cos);
            r * r * (tan_half - forge_core::dmath::atan(tan_half))
        })
        .sum()
}

/// How much of a polygon lies nearer than `width / 2` to its outline from every point at least
/// that far in, m²: the parts thinner than `width` (a court's slivers and sharp tips), sampled
/// every half metre.
pub fn thin_area(polygon: &[DVec2], width: f64) -> f64 {
    let n = polygon.len();
    if n < 3 {
        return 0.0;
    }
    let r = width * 0.5;
    let (lo, hi) = polygon.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    const STEP: f64 = 0.5;
    let to_edge = |p: DVec2| {
        (0..n)
            .map(|k| geom::distance_to_segment(p, polygon[k], polygon[(k + 1) % n]))
            .fold(f64::MAX, f64::min)
    };
    let (cols, rows) = (
        ((hi.x - lo.x) / STEP).ceil() as usize + 1,
        ((hi.y - lo.y) / STEP).ceil() as usize + 1,
    );
    let mut inside = Vec::new();
    let mut core = std::collections::HashMap::<(i64, i64), Vec<DVec2>>::new();
    for y in 0..rows {
        for x in 0..cols {
            let p = lo + DVec2::new(x as f64 + 0.5, y as f64 + 0.5) * STEP;
            if !geom::contains(polygon, p) {
                continue;
            }
            inside.push(p);
            if to_edge(p) >= r {
                let cell = ((p.x / r).floor() as i64, (p.y / r).floor() as i64);
                core.entry(cell).or_default().push(p);
            }
        }
    }
    let reach = r + STEP;
    let thin = inside
        .iter()
        .filter(|&&p| {
            let (cx, cy) = ((p.x / r).floor() as i64, (p.y / r).floor() as i64);
            !(cx - 2..=cx + 2).any(|x| {
                (cy - 2..=cy + 2).any(|y| {
                    core.get(&(x, y))
                        .is_some_and(|ps| ps.iter().any(|q| q.distance(p) <= reach))
                })
            })
        })
        .count();
    thin as f64 * STEP * STEP
}
