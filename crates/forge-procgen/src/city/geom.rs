//! Plane geometry for the city plan: points in metres on the plan (`x` east, `y` south, the
//! frame's top-left corner at the origin), segments, polylines and simple polygons. Only
//! IEEE-exact operations (`+ − × ÷ sqrt`), so every machine draws the same plan (D-016).

use glam::DVec2;

/// The z of the cross product `a × b`.
#[inline]
pub fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// Where segments `p0 p1` and `q0 q1` cross, as the parameters `(t, u)` along each in `[0, 1]`;
/// `None` when they are parallel or miss.
pub fn segment_intersection(p0: DVec2, p1: DVec2, q0: DVec2, q1: DVec2) -> Option<(f64, f64)> {
    let r = p1 - p0;
    let s = q1 - q0;
    let denom = cross(r, s);
    if denom.abs() < 1e-12 {
        return None;
    }
    let qp = q0 - p0;
    let t = cross(qp, s) / denom;
    let u = cross(qp, r) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// The point of segment `a b` closest to `p`, as its parameter in `[0, 1]`.
pub fn closest_on_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 0.0 {
        return 0.0;
    }
    ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// The distance from `p` to segment `a b`.
pub fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let t = closest_on_segment(p, a, b);
    p.distance(a + (b - a) * t)
}

/// A polyline's length.
pub fn polyline_length(points: &[DVec2]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The point at distance `s` along a polyline (clamped to its ends).
pub fn point_at(points: &[DVec2], s: f64) -> DVec2 {
    let mut left = s.max(0.0);
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if left <= len && len > 0.0 {
            return w[0] + (w[1] - w[0]) * (left / len);
        }
        left -= len;
    }
    *points.last().unwrap_or(&DVec2::ZERO)
}

/// The unit tangent at distance `s` along a polyline.
pub fn tangent_at(points: &[DVec2], s: f64) -> DVec2 {
    let mut left = s.max(0.0);
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if left <= len && len > 0.0 {
            return (w[1] - w[0]) / len;
        }
        left -= len;
    }
    match points {
        [.., a, b] => (*b - *a).normalize_or_zero(),
        _ => DVec2::X,
    }
}

/// A polyline with each corner rounded into an arc of `radius` metres, sampled every `step`
/// metres or so: a tram, a train or a car does not turn at a point. Where the segments either
/// side are short, the arc shrinks so that it never takes more than 45 % of either, and two
/// arcs never overlap. Turns under a degree are left as they are.
pub fn fillet(points: &[DVec2], radius: f64, step: f64) -> Vec<DVec2> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for i in 1..n - 1 {
        let (a, v, b) = (points[i - 1], points[i], points[i + 1]);
        let (din, dout) = ((v - a).normalize_or_zero(), (b - v).normalize_or_zero());
        let turn = din.angle_to(dout).abs();
        if turn < 1f64.to_radians() || din == DVec2::ZERO || dout == DVec2::ZERO {
            out.push(v);
            continue;
        }
        // The tangent length for the radius, shortened to fit the segments.
        let half = (turn * 0.5).tan();
        let room = 0.45 * a.distance(v).min(v.distance(b));
        let t = (radius * half).min(room);
        let r = t / half;
        let (start, end) = (v - din * t, v + dout * t);
        // The arc's centre lies on the inside of the turn, `r` from both tangent points.
        let inward = DVec2::new(-din.y, din.x) * din.perp_dot(dout).signum();
        let centre = start + inward * r;
        let (a0, a1) = ((start - centre).to_angle(), (end - centre).to_angle());
        let mut sweep = a1 - a0;
        if sweep > std::f64::consts::PI {
            sweep -= std::f64::consts::TAU;
        } else if sweep < -std::f64::consts::PI {
            sweep += std::f64::consts::TAU;
        }
        let k = ((r * sweep.abs()) / step).ceil().max(1.0) as usize;
        for j in 0..=k {
            let angle = a0 + sweep * j as f64 / k as f64;
            out.push(centre + DVec2::from_angle(angle) * r);
        }
    }
    out.push(points[n - 1]);
    out
}

/// A polyline resampled every `step` metres (its last point kept).
pub fn resample(points: &[DVec2], step: f64) -> Vec<DVec2> {
    let length = polyline_length(points);
    if points.len() < 2 || length <= 0.0 {
        return points.to_vec();
    }
    let n = (length / step).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| point_at(points, length * i as f64 / n as f64))
        .collect()
}

/// Douglas–Peucker simplification to within `tolerance` metres, ends kept.
pub fn simplify(points: &[DVec2], tolerance: f64) -> Vec<DVec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let mut worst = 0.0;
        let mut index = first;
        for (i, &p) in points.iter().enumerate().take(last).skip(first + 1) {
            let d = distance_to_segment(p, points[first], points[last]);
            if d > worst {
                worst = d;
                index = i;
            }
        }
        if worst > tolerance {
            keep[index] = true;
            stack.push((first, index));
            stack.push((index, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(&p, k)| k.then_some(p))
        .collect()
}

/// Douglas–Peucker on a closed polygon: split at its first vertex and the vertex farthest from
/// it, each half simplified, the duplicate joints dropped.
pub fn simplify_closed(polygon: &[DVec2], tolerance: f64) -> Vec<DVec2> {
    let n = polygon.len();
    if n < 4 {
        return polygon.to_vec();
    }
    let far = (1..n)
        .max_by(|&a, &b| {
            polygon[a]
                .distance_squared(polygon[0])
                .partial_cmp(&polygon[b].distance_squared(polygon[0]))
                .expect("finite")
        })
        .expect("n ≥ 4");
    let first = simplify(&polygon[..=far], tolerance);
    let mut second_chain: Vec<DVec2> = polygon[far..].to_vec();
    second_chain.push(polygon[0]);
    let second = simplify(&second_chain, tolerance);
    let mut out = first;
    out.extend_from_slice(&second[1..second.len() - 1]);
    out
}

/// A closed polygon's signed area (positive when its vertices turn counter-clockwise in a
/// right-handed frame; on the plan, whose `y` points south, that is clockwise on the map).
pub fn signed_area(polygon: &[DVec2]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| cross(polygon[i], polygon[(i + 1) % n]))
        .sum::<f64>()
        * 0.5
}

/// A closed polygon's centroid (its vertices' mean if it has no area).
pub fn centroid(polygon: &[DVec2]) -> DVec2 {
    let n = polygon.len();
    let area = signed_area(polygon);
    if area.abs() < 1e-9 {
        return polygon.iter().copied().sum::<DVec2>() / n.max(1) as f64;
    }
    let mut c = DVec2::ZERO;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        c += (a + b) * cross(a, b);
    }
    c / (6.0 * area)
}

/// Whether `p` lies inside a closed polygon (even-odd rule).
pub fn contains(polygon: &[DVec2], p: DVec2) -> bool {
    let n = polygon.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The part of a polygon on the side of the line through `origin` along `direction` where
/// `cross(direction, p − origin) ≥ 0` (Sutherland–Hodgman against one half-plane). Exact for
/// convex polygons; a concave one may come back with a degenerate bridge, harmless for lots.
pub fn clip_half_plane(polygon: &[DVec2], origin: DVec2, direction: DVec2) -> Vec<DVec2> {
    let side = |p: DVec2| cross(direction, p - origin);
    let n = polygon.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            out.push(a + (b - a) * (sa / (sa - sb)));
        }
    }
    out
}

/// A simple polygon's triangles by ear clipping, as index triples into it, each turning the way
/// the polygon does. Exact operations only; a polygon whose ears run out (a self-touching
/// outline) gives the triangles found so far.
pub fn triangulate(polygon: &[DVec2]) -> Vec<[usize; 3]> {
    let n = polygon.len();
    if n < 3 {
        return Vec::new();
    }
    let sign = if signed_area(polygon) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let mut ring: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut misses = 0;
    let mut i = 0;
    while ring.len() > 3 && misses < ring.len() {
        let m = ring.len();
        let (ia, ib, ic) = (ring[(i + m - 1) % m], ring[i % m], ring[(i + 1) % m]);
        let (a, b, c) = (polygon[ia], polygon[ib], polygon[ic]);
        let convex = cross(b - a, c - b) * sign > 0.0;
        let empty = convex
            && ring.iter().all(|&j| {
                if j == ia || j == ib || j == ic {
                    return true;
                }
                let p = polygon[j];
                // Outside (or on an edge of) triangle a b c.
                let d1 = cross(b - a, p - a) * sign;
                let d2 = cross(c - b, p - b) * sign;
                let d3 = cross(a - c, p - c) * sign;
                d1 < 0.0 || d2 < 0.0 || d3 < 0.0
            });
        if empty {
            out.push([ia, ib, ic]);
            ring.remove(i % m);
            misses = 0;
            if i >= ring.len() {
                i = 0;
            }
        } else {
            i = (i + 1) % m;
            misses += 1;
        }
    }
    if ring.len() == 3 {
        out.push([ring[0], ring[1], ring[2]]);
    }
    out
}

/// The convex hull of a set of points (Andrew's monotone chain), positive area, no collinear
/// points.
pub fn convex_hull(points: &[DVec2]) -> Vec<DVec2> {
    let mut p = points.to_vec();
    p.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .expect("finite")
            .then(a.y.partial_cmp(&b.y).expect("finite"))
    });
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut hull: Vec<DVec2> = Vec::with_capacity(2 * p.len());
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in iter {
            while hull.len() >= start + 2
                && cross(
                    hull[hull.len() - 1] - hull[hull.len() - 2],
                    q - hull[hull.len() - 2],
                ) <= 0.0
            {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    hull
}

/// A polygon moved `d` metres inwards on every side (mitred corners), for a polygon of positive
/// signed area. `None` when the result folds over itself or comes closer than `d` to the
/// outline somewhere (a block too thin or too concave for it).
pub fn inset(polygon: &[DVec2], d: f64) -> Option<Vec<DVec2>> {
    inset_by(polygon, &vec![d; polygon.len()])
}

/// A polygon moved inwards by `d[i]` metres along its edge `i` (from vertex `i` to `i + 1`):
/// a block inside the building lines of streets of different widths. As [`inset`] otherwise;
/// where two neighbouring edges are nearly parallel the corner is the mean of their moved ends.
pub fn inset_by(polygon: &[DVec2], d: &[f64]) -> Option<Vec<DVec2>> {
    let n = polygon.len();
    if n < 3 || d.len() != n || signed_area(polygon) <= 0.0 {
        return None;
    }
    // Each edge's line moved inwards (to its left, the inside of a positive polygon), with the
    // vertex it starts from.
    let lines: Vec<(DVec2, DVec2, f64, DVec2)> = (0..n)
        .filter_map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            let t = (b - a).normalize_or_zero();
            (t != DVec2::ZERO).then(|| {
                let normal = DVec2::new(-t.y, t.x);
                (a + normal * d[i], t, d[i], a)
            })
        })
        .collect();
    let m = lines.len();
    let mut out = Vec::with_capacity(m);
    for i in 0..m {
        let (p, t, dp, _) = lines[(i + m - 1) % m];
        let (q, u, dq, corner) = lines[i];
        let denom = cross(t, u);
        let mean = (p + t * (corner - p).dot(t) + q) * 0.5;
        if denom.abs() < 1e-6 {
            out.push(mean);
            continue;
        }
        let s = cross(q - p, u) / denom;
        let mitre = p + t * s;
        // A long mitre where the edges nearly line up: their moved ends' mean instead.
        out.push(if mitre.distance(corner) > 3.0 * dp.max(dq) {
            mean
        } else {
            mitre
        });
    }
    valid_inset(polygon, d, &out).then_some(out)
}

/// As [`inset_by`], dropping each edge the move turns over (a short edge between two that move
/// further in: the straight skeleton's edge event) until none does.
pub fn inset_by_collapsing(polygon: &[DVec2], d: &[f64]) -> Option<Vec<DVec2>> {
    let n = polygon.len();
    if n < 3 || d.len() != n || signed_area(polygon) <= 0.0 {
        return None;
    }
    let mut kept: Vec<usize> = (0..n)
        .filter(|&i| polygon[i] != polygon[(i + 1) % n])
        .collect();
    loop {
        let m = kept.len();
        if m < 3 {
            return None;
        }
        // Each kept edge's line moved inwards, its direction, and where it starts.
        let lines: Vec<(DVec2, DVec2, DVec2)> = kept
            .iter()
            .map(|&i| {
                let (a, b) = (polygon[i], polygon[(i + 1) % n]);
                let t = (b - a).normalize();
                (a + DVec2::new(-t.y, t.x) * d[i], t, a)
            })
            .collect();
        let out: Vec<DVec2> = (0..m)
            .map(|k| {
                let prev = (k + m - 1) % m;
                let ((p, t, _), (q, u, corner)) = (lines[prev], lines[k]);
                let mean = (p + t * (corner - p).dot(t) + q) * 0.5;
                let denom = cross(t, u);
                if denom.abs() < 1e-6 {
                    return mean;
                }
                let mitre = p + t * (cross(q - p, u) / denom);
                // As inset_by: a long mitre between neighbours nearly in line, their mean.
                let (i, j) = (kept[prev], kept[k]);
                let neighbours = (i + 1) % n == j;
                if neighbours && mitre.distance(corner) > 3.0 * d[i].max(d[j]) {
                    mean
                } else {
                    mitre
                }
            })
            .collect();
        let turned = (0..m)
            .map(|k| ((out[(k + 1) % m] - out[k]).dot(lines[k].1), k))
            .filter(|&(along, _)| along < 0.0)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match turned {
            Some((_, k)) => {
                kept.remove(k);
            }
            None => return valid_inset(polygon, d, &out).then_some(out),
        }
    }
}

/// Whether `out` is a fair inset of `polygon` by `d`: positive and smaller, every corner inside
/// the outline and at least ~the smallest inset from it, and no two non-adjacent edges crossing.
fn valid_inset(polygon: &[DVec2], d: &[f64], out: &[DVec2]) -> bool {
    let n = polygon.len();
    let m = out.len();
    let area = signed_area(out);
    if area <= 0.0 || area > signed_area(polygon) {
        return false;
    }
    let least = d.iter().copied().fold(f64::MAX, f64::min);
    let ok = out.iter().all(|&v| {
        contains(polygon, v)
            && (0..n)
                .all(|i| distance_to_segment(v, polygon[i], polygon[(i + 1) % n]) > least * 0.8)
    });
    // And no two non-adjacent edges cross.
    let crossing = (0..m).any(|i| {
        ((i + 2)..m).any(|j| {
            (i + 1) % m != j
                && (j + 1) % m != i
                && segment_intersection(out[i], out[(i + 1) % m], out[j], out[(j + 1) % m])
                    .is_some()
        })
    });
    ok && !crossing
}

/// A polygon's oriented box by its longest edge: the unit axis along that edge and the extents
/// `(along, across)` of the polygon in that frame, with the frame's centre.
pub fn oriented_extent(polygon: &[DVec2]) -> (DVec2, DVec2, DVec2) {
    let n = polygon.len();
    let mut axis = DVec2::X;
    let mut longest = 0.0;
    for i in 0..n {
        let e = polygon[(i + 1) % n] - polygon[i];
        let len = e.length();
        if len > longest {
            longest = len;
            axis = e / len;
        }
    }
    let normal = DVec2::new(-axis.y, axis.x);
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for &p in polygon {
        let q = DVec2::new(p.dot(axis), p.dot(normal));
        lo = lo.min(q);
        hi = hi.max(q);
    }
    let mid = (lo + hi) * 0.5;
    (axis, hi - lo, axis * mid.x + normal * mid.y)
}

/// A uniform grid of buckets over the frame for nearest-point queries, filled in a fixed order
/// so its answers never depend on hashing (D-016).
#[derive(Clone, Debug)]
pub struct PointGrid {
    cell: f64,
    side: usize,
    buckets: Vec<Vec<(DVec2, u32)>>,
}

impl PointGrid {
    /// An empty grid over a square frame of `size` metres with buckets of `cell` metres.
    pub fn new(size: f64, cell: f64) -> Self {
        let side = (size / cell).ceil().max(1.0) as usize + 1;
        Self {
            cell,
            side,
            buckets: vec![Vec::new(); side * side],
        }
    }

    fn bucket(&self, p: DVec2) -> Option<usize> {
        let x = (p.x / self.cell).floor();
        let y = (p.y / self.cell).floor();
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.side && (y as usize) < self.side)
            .then(|| y as usize * self.side + x as usize)
    }

    /// Adds a point with a tag (the polyline it belongs to, say).
    pub fn insert(&mut self, p: DVec2, tag: u32) {
        if let Some(b) = self.bucket(p) {
            self.buckets[b].push((p, tag));
        }
    }

    /// Whether a point lies within `radius` of `p`, skipping those whose tag `skip` refuses.
    pub fn any_within(&self, p: DVec2, radius: f64, skip: impl Fn(u32) -> bool) -> bool {
        self.visit(p, radius, |q, tag| {
            !skip(tag) && q.distance_squared(p) < radius * radius
        })
    }

    /// The nearest point within `radius` of `p` with its tag.
    pub fn nearest(&self, p: DVec2, radius: f64) -> Option<(DVec2, u32)> {
        let mut best: Option<(f64, DVec2, u32)> = None;
        self.visit(p, radius, |q, tag| {
            let d = q.distance_squared(p);
            if d < radius * radius && best.is_none_or(|(bd, ..)| d < bd) {
                best = Some((d, q, tag));
            }
            false
        });
        best.map(|(_, q, tag)| (q, tag))
    }

    /// Calls `f` on every point within the buckets that cover `radius` around `p`, in a fixed
    /// order.
    pub fn for_each_within(&self, p: DVec2, radius: f64, mut f: impl FnMut(DVec2, u32)) {
        self.visit(p, radius, |q, tag| {
            f(q, tag);
            false
        });
    }

    /// Calls `f` on every point in the buckets within `radius` of `p` until it returns true.
    fn visit(&self, p: DVec2, radius: f64, mut f: impl FnMut(DVec2, u32) -> bool) -> bool {
        let r = (radius / self.cell).ceil() as i64;
        let cx = (p.x / self.cell).floor() as i64;
        let cy = (p.y / self.cell).floor() as i64;
        for y in (cy - r).max(0)..=(cy + r).min(self.side as i64 - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.side as i64 - 1) {
                for &(q, tag) in &self.buckets[y as usize * self.side + x as usize] {
                    if f(q, tag) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_round_into_arcs_of_the_radius() {
        // A right angle between two 100 m legs, then a short dog-leg.
        let line = [
            DVec2::new(0.0, 0.0),
            DVec2::new(100.0, 0.0),
            DVec2::new(100.0, 100.0),
            DVec2::new(104.0, 104.0),
        ];
        let round = fillet(&line, 18.0, 2.0);
        assert_eq!(round[0], line[0]);
        assert_eq!(round[round.len() - 1], line[3]);
        // No turn sharper than an 18 m radius allows over the step, but at the short leg.
        for w in round.windows(3) {
            let (a, b) = (w[1] - w[0], w[2] - w[1]);
            if a.length() < 0.1 || b.length() < 0.1 || w[1].x > 99.0 && w[1].y > 90.0 {
                continue;
            }
            let turn = a.angle_to(b).abs();
            let reach = (a.length() + b.length()) * 0.5;
            assert!(
                turn <= reach / 18.0 * 1.2 + 1e-6,
                "{turn} over {reach} m at {}",
                w[1]
            );
        }
        // The corner is cut by r(√2 − 1) at its bisector.
        let nearest = round
            .iter()
            .map(|p| p.distance(DVec2::new(100.0, 0.0)))
            .fold(f64::MAX, f64::min);
        assert!(
            (nearest - 18.0 * (2f64.sqrt() - 1.0)).abs() < 0.2,
            "{nearest}"
        );
    }

    #[test]
    fn a_short_edge_between_wide_streets_collapses() {
        // A 60 × 40 block cut at one corner by a 6 m edge; the two long sides round that corner
        // move in 14 m, the rest 4 m. The short edge turns over and goes, the long sides meet.
        let block = [
            DVec2::new(0.0, 0.0),
            DVec2::new(60.0, 0.0),
            DVec2::new(60.0, 36.0),
            DVec2::new(56.0, 40.0),
            DVec2::new(0.0, 40.0),
        ];
        let d = [4.0, 14.0, 14.0, 14.0, 4.0];
        let inside = inset_by_collapsing(&block, &d).expect("an inset");
        assert_eq!(inside.len(), 4, "{inside:?}");
        let corner = inside
            .iter()
            .copied()
            .max_by(|a, b| (a.x + a.y).total_cmp(&(b.x + b.y)))
            .expect("corners");
        assert!(corner.distance(DVec2::new(46.0, 26.0)) < 0.5, "{corner:?}");
    }

    #[test]
    fn segments_cross_where_expected() {
        let hit = segment_intersection(
            DVec2::new(0.0, 0.0),
            DVec2::new(10.0, 10.0),
            DVec2::new(0.0, 10.0),
            DVec2::new(10.0, 0.0),
        );
        let (t, u) = hit.expect("the diagonals cross");
        assert!((t - 0.5).abs() < 1e-12 && (u - 0.5).abs() < 1e-12);
        assert!(
            segment_intersection(
                DVec2::ZERO,
                DVec2::X,
                DVec2::new(0.0, 1.0),
                DVec2::new(1.0, 1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn a_square_s_area_centroid_and_inside() {
        let square = [
            DVec2::new(0.0, 0.0),
            DVec2::new(4.0, 0.0),
            DVec2::new(4.0, 4.0),
            DVec2::new(0.0, 4.0),
        ];
        assert_eq!(signed_area(&square), 16.0);
        assert_eq!(centroid(&square), DVec2::new(2.0, 2.0));
        assert!(contains(&square, DVec2::new(1.0, 3.0)));
        assert!(!contains(&square, DVec2::new(5.0, 3.0)));
        let half = clip_half_plane(&square, DVec2::new(2.0, 0.0), DVec2::new(0.0, 1.0));
        assert!((signed_area(&half).abs() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let line: Vec<DVec2> = (0..=10)
            .map(|i| DVec2::new(i as f64, 0.0))
            .chain((1..=10).map(|i| DVec2::new(10.0, i as f64)))
            .collect();
        let simple = simplify(&line, 0.01);
        assert_eq!(
            simple,
            vec![DVec2::ZERO, DVec2::new(10.0, 0.0), DVec2::new(10.0, 10.0)]
        );
    }

    #[test]
    fn an_l_shape_is_cut_into_four_triangles_covering_it() {
        let l = [
            DVec2::new(0.0, 0.0),
            DVec2::new(4.0, 0.0),
            DVec2::new(4.0, 2.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(2.0, 4.0),
            DVec2::new(0.0, 4.0),
        ];
        let tris = triangulate(&l);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris
            .iter()
            .map(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]))
            .sum();
        assert!((area - 12.0).abs() < 1e-9, "{area}");
        assert!(
            tris.iter()
                .all(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]) > 0.0)
        );
    }

    #[test]
    fn the_grid_finds_its_nearest_point() {
        let mut grid = PointGrid::new(100.0, 10.0);
        grid.insert(DVec2::new(50.0, 50.0), 1);
        grid.insert(DVec2::new(58.0, 50.0), 2);
        assert_eq!(
            grid.nearest(DVec2::new(56.0, 50.0), 5.0).map(|n| n.1),
            Some(2)
        );
        assert!(grid.any_within(DVec2::new(52.0, 50.0), 3.0, |_| false));
        assert!(!grid.any_within(DVec2::new(52.0, 50.0), 3.0, |tag| tag == 1));
    }
}
