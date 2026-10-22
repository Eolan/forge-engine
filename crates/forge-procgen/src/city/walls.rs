//! The walls round a village's gardens (#266). The villages of the cadastre research
//! (`docs/research/cadastre-villages-and-towns.md`) close every garden and court with a stone
//! wall wherever no house stands on its outline: high along the streets and lanes, lower between
//! neighbours, low on a lotissement's front, over which the house is seen. Each garden keeps a
//! gate on its street side, level with the street (the owner's rule: no step where people walk).

use forge_core::SplitMix64;
use glam::DVec2;

use super::geom;

/// A garden wall's thickness, metres: rubble stone under a lime render.
pub const THICKNESS: f64 = 0.4;
/// A garden wall's height along a street or a lane, metres: over a walker's eyes.
pub const STREET: f64 = 1.8;
/// A wall's height between two neighbours' gardens or courts, metres.
pub const PARTY: f64 = 1.5;
/// A lotissement's front wall, metres: low, the house seen over it from the street.
pub const FRONT: f64 = 1.0;
/// A gate's width, metres: a wheelchair's passage.
pub const GATE: f64 = 1.2;
/// How close a stretch lies to a house's outline or to a wall already kept to count as on it,
/// metres (the court stands 5 cm in from the lanes, `parcels::strips_by`).
const ON: f64 = 0.08;
/// The shortest stretch of wall kept, metres.
const SHORTEST: f64 = 0.3;
/// How near a wall's end must come to another wall for the two to be joined, metres: a
/// wall's thickness, the lots' corners from two strips falling a little apart.
const JOIN: f64 = THICKNESS;
/// How far apart the ends of two walls nearly in line may stand to be joined, metres: a slot
/// narrower than a gate is no way through, and showed as a dark strip in the wall.
const IN_LINE_JOIN: f64 = 0.75;
/// Two walls ending together at a corner between these angles apart are mitred (`joints`), as
/// cosines (75° to 36°): butted, the one ranking above ran on past the other's far face, its end
/// out in the open meeting nothing (#308). From 75° up the overrun stays within the other's
/// face; under 36° a mitre leans past the 0.3 of square [`Wall::ring`] keeps, and would part.
const MITRE: (f64, f64) = (0.26, 0.81);
/// And obtuse corners, 100° to 162° (where walls in line meet on their own mitre): butted, two
/// walls whose thickness both took the garden's side left their ends' faces 0.27 m apart at the
/// corner (#308).
const MITRE_OBTUSE: (f64, f64) = (-0.95, -0.17);
/// How near a stretch running along a house's wall may come to it and still be walled, metres:
/// nearer, the house's own wall bounds the garden. A lot line half a metre behind a row's backs
/// stood a garden wall in a slot before their court doors, the paths to them ending at it.
const NEAR: f64 = GATE;

/// A garden wall: its line on the plan, its height and the side its thickness takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    /// Its line's start, metres on the plan.
    pub a: DVec2,
    /// Its line's end.
    pub b: DVec2,
    /// Its height over the street, metres.
    pub height: f64,
    /// A point on the side its thickness takes: the garden's, so a wall along the street stays
    /// off it; `None` between two gardens, the wall centred on their line.
    pub inside: Option<DVec2>,
    /// The way its end faces run at `a` and at `b`, unit vectors to its left: square to it, or
    /// mitred where it meets a wall nearly in line with it (where a lane bends).
    pub ends: [DVec2; 2],
}

impl Wall {
    /// A wall from `a` to `b`, its ends square.
    pub fn new(a: DVec2, b: DVec2, height: f64, inside: Option<DVec2>) -> Self {
        let t = (b - a).normalize_or_zero();
        let square = DVec2::new(-t.y, t.x);
        Wall {
            a,
            b,
            height,
            inside,
            ends: [square, square],
        }
    }

    /// Its length, metres.
    pub fn length(&self) -> f64 {
        self.a.distance(self.b)
    }

    /// How far its thickness reaches to the right and to the left of its line from `a` to `b`,
    /// metres: all of it on the garden's side, or half each side between two gardens.
    pub fn band(&self) -> (f64, f64) {
        let t = (self.b - self.a).normalize_or_zero();
        let left = DVec2::new(-t.y, t.x);
        match self.inside {
            Some(p) if left.dot(p - self.a) >= 0.0 => (0.0, THICKNESS),
            Some(_) => (THICKNESS, 0.0),
            None => (THICKNESS * 0.5, THICKNESS * 0.5),
        }
    }

    /// Its ground plan: its line widened by its band between its end faces, anticlockwise.
    pub fn ring(&self) -> Vec<DVec2> {
        let t = (self.b - self.a).normalize_or_zero();
        let left = DVec2::new(-t.y, t.x);
        let (r, l) = self.band();
        // Where the band's edge `offset` to the left crosses the end face through `p`.
        let at = |p: DVec2, end: DVec2, offset: f64| p + end * (offset / end.dot(left).max(0.3));
        let [ea, eb] = self.ends;
        vec![
            at(self.a, ea, -r),
            at(self.b, eb, -r),
            at(self.b, eb, l),
            at(self.a, ea, l),
        ]
    }
}

/// A block's garden walls, the gates left in them and the hedges between its gardens.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GardenWalls {
    /// The walls.
    pub walls: Vec<Wall>,
    /// Each gate's middle, metres on the plan.
    pub gates: Vec<DVec2>,
    /// The hedges: each line on the plan, from one end to the other.
    pub hedges: Vec<(DVec2, DVec2)>,
}

/// A stretch of a lot's outline where a wall or a hedge stands.
struct Stretch {
    a: DVec2,
    b: DVec2,
    /// The lot it closes.
    owner: usize,
    /// On its lot's front, the street side.
    front: bool,
    /// On the block's outline: along a street or a lane.
    street: bool,
    /// A hedge, not a wall.
    hedge: bool,
}

/// Of the stretches between two gardens, how many in [`HEDGE_SHARE`]'s denominator are walls;
/// the others hedges (the owner, 2026-10-16: "sometimes it's better to have plants (hedges)
/// separating the gardens").
const HEDGE_SHARE: (u64, u64) = (2, 5);

/// The walls and hedges of a block cut into `lots` round `court` within its `outline`, the
/// houses' `footprints` standing on the lots and `housed[k]` saying whether lot `k` has its
/// house: each stretch of a housed lot's outline that no house stands on, once. Along the block's
/// outline a wall [`STREET`] high (on a lot's front `front` high: [`FRONT`] for a lotissement's);
/// between two gardens a wall [`PARTY`] high or, for [`HEDGE_SHARE`] of them, a hedge; a garden's
/// side on the court a hedge. The court is the block's common ground, open to the lanes and to
/// the lots without a house, which stay open ground too: walled all round, the courts and the
/// empty lots were yards of walls with nothing in them (the owner, 2026-10-16: "why is this place
/// full of surrounding walls?", "the middle area should not exist or be common"). Each lot with
/// a wall along the street keeps a gate in its longest one, before its house when that wall is
/// on its front. The lots and the court wind anticlockwise, as [`super::parcels::strips_by`] cuts
/// them.
pub fn garden_walls(
    outline: &[DVec2],
    lots: &[Vec<DVec2>],
    court: &[DVec2],
    footprints: &[Vec<DVec2>],
    housed: &[bool],
    gardens: &[Vec<DVec2>],
    front: f64,
) -> GardenWalls {
    let mut edges: Vec<(DVec2, DVec2, usize, bool)> = Vec::new();
    for (k, lot) in lots.iter().enumerate() {
        if !housed.get(k).copied().unwrap_or(true) {
            continue;
        }
        let n = lot.len();
        for i in 0..n {
            edges.push((lot[i], lot[(i + 1) % n], k, i == 0));
        }
    }
    let on_outline = |p: DVec2| boundary_distance(outline, p) < 2.0 * ON;
    // Whether the ground just right of a stretch (outside its lot) is the court's.
    let on_court = |p: DVec2, along: DVec2| {
        court.len() >= 3 && geom::contains(court, p + DVec2::new(along.y, -along.x) * 0.3)
    };
    let mut kept: Vec<(DVec2, DVec2)> = Vec::new();
    let mut stretches: Vec<Stretch> = Vec::new();
    for (p, q, owner, is_front) in edges {
        let length = p.distance(q);
        if length < SHORTEST {
            continue;
        }
        let along = (q - p) / length;
        // Where a stretch may begin or end along p q: where a house's outline crosses it or
        // touches it, and where a wall kept before ends on it.
        let mut ts = vec![0.0, 1.0];
        for fp in footprints {
            let m = fp.len();
            for i in 0..m {
                let (u, v) = (fp[i], fp[(i + 1) % m]);
                if let Some((t, _)) = geom::segment_intersection(p, q, u, v) {
                    ts.push(t);
                }
                // Its corners on the stretch, and those near it, where a stretch along the
                // house's wall begins and ends (`hugs`).
                for w in [u, v] {
                    if geom::distance_to_segment(w, p, q) < NEAR {
                        ts.push(geom::closest_on_segment(w, p, q));
                    }
                }
            }
        }
        for &(u, v) in &kept {
            for w in [u, v] {
                if geom::distance_to_segment(w, p, q) < THICKNESS {
                    ts.push(geom::closest_on_segment(w, p, q));
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        // The pieces between them no house stands on and no kept wall runs along (within its
        // thickness and within 10°: two lots back to back across a sliver of court keep one
        // wall), joined.
        let mut runs: Vec<(f64, f64)> = Vec::new();
        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if (t1 - t0) * length < 1e-3 {
                continue;
            }
            let mid = p + (q - p) * ((t0 + t1) * 0.5);
            let housed = footprints
                .iter()
                .any(|fp| geom::contains(fp, mid) || boundary_distance(fp, mid) < ON);
            let walled = kept.iter().any(|&(u, v)| {
                let d = (v - u).normalize_or_zero();
                d.perp_dot(along).abs() < 0.17 && geom::distance_to_segment(mid, u, v) < THICKNESS
            });
            let hugging = footprints.iter().any(|fp| hugs(fp, mid, along));
            if housed || walled || hugging {
                continue;
            }
            match runs.last_mut() {
                Some(run) if (run.1 - t0).abs() < 1e-9 => run.1 = t1,
                _ => runs.push((t0, t1)),
            }
        }
        for (t0, t1) in runs {
            if (t1 - t0) * length < SHORTEST {
                continue;
            }
            let (a, b) = (p + (q - p) * t0, p + (q - p) * t1);
            kept.push((a, b));
            let mid = (a + b) * 0.5;
            let street = on_outline(a) && on_outline(b) && on_outline(mid);
            // Between two gardens a wall or a hedge, by a hash of where the stretch lies (the
            // same whichever of the two lots meets it first).
            let cell = (mid * 4.0).floor();
            let hash = SplitMix64::new(
                (cell.x as i64 as u64) << 32 ^ (cell.y as i64 as u64 & 0xFFFF_FFFF),
            )
            .next_u64();
            // Only between two gardens (a garden and the court): across a corner lot's paved
            // front a hedge crossed the paving (the owner, 2026-10-16: "positions of the hedges
            // don't make sense").
            let left = DVec2::new(-along.y, along.x) * 0.3;
            let green = |p: DVec2| gardens.iter().any(|g| g.len() >= 3 && geom::contains(g, p));
            let between = green(mid + left) && (green(mid - left) || on_court(mid, along));
            let hedge = !street
                && between
                && (on_court(mid, along) || hash % HEDGE_SHARE.1 < HEDGE_SHARE.0);
            stretches.push(Stretch {
                a,
                b,
                owner,
                front: is_front,
                street,
                hedge,
            });
        }
    }
    // The ends a stretch along a house's wall left free, joined to the house.
    let joined = stubs(&stretches, footprints);
    stretches.extend(joined);
    // The gates: each lot's in its longest wall along the street, its front's first.
    let mut gate_at: Vec<Option<f64>> = vec![None; stretches.len()];
    for k in 0..lots.len() {
        let best = stretches
            .iter()
            .enumerate()
            .filter(|(_, s)| s.owner == k && s.street && !s.hedge)
            .filter(|(_, s)| s.a.distance(s.b) >= GATE + 2.0 * (JOIN + SHORTEST))
            .max_by(|(_, x), (_, y)| {
                (x.front, x.a.distance(x.b))
                    .partial_cmp(&(y.front, y.a.distance(y.b)))
                    .unwrap()
            });
        if let Some((i, s)) = best {
            let length = s.a.distance(s.b);
            let half = GATE * 0.5 + JOIN + SHORTEST;
            // Before the house on a front: the nearest footprint's middle, along the stretch.
            let mid = (s.a + s.b) * 0.5;
            let at = if s.front {
                footprints
                    .iter()
                    .map(|fp| geom::centroid(fp))
                    .min_by(|x, y| x.distance(mid).total_cmp(&y.distance(mid)))
                    .map_or(length * 0.5, |c| {
                        geom::closest_on_segment(c, s.a, s.b) * length
                    })
            } else {
                length * 0.5
            };
            gate_at[i] = Some(at.clamp(half, length - half));
        }
    }
    let mut out = GardenWalls::default();
    for (s, gate) in stretches.iter().zip(gate_at) {
        if s.hedge {
            out.hedges.push((s.a, s.b));
            continue;
        }
        let height = match (s.street, s.front) {
            (true, true) => front,
            (true, false) => STREET,
            (false, _) => PARTY,
        };
        let along = (s.b - s.a).normalize_or_zero();
        // The lots and the court wind anticlockwise: their inside is to the left.
        let inside = s
            .street
            .then(|| (s.a + s.b) * 0.5 + DVec2::new(-along.y, along.x));
        let mut push = |a: DVec2, b: DVec2| {
            if a.distance(b) >= SHORTEST {
                out.walls.push(Wall::new(a, b, height, inside));
            }
        };
        match gate {
            Some(at) => {
                push(s.a, s.a + along * (at - GATE * 0.5));
                push(s.a + along * (at + GATE * 0.5), s.b);
                out.gates.push(s.a + along * at);
            }
            None => push(s.a, s.b),
        }
    }
    joints(&mut out.walls);
    out
}

/// Whether point `mid` of a stretch along `along` stands before a wall of house `fp` parallel to
/// it (within 10°), nearer than [`NEAR`].
fn hugs(fp: &[DVec2], mid: DVec2, along: DVec2) -> bool {
    let m = fp.len();
    (0..m).any(|i| {
        let (u, v) = (fp[i], fp[(i + 1) % m]);
        let length = u.distance(v);
        if length < 1e-6 {
            return false;
        }
        let e = (v - u) / length;
        let s = (mid - u).dot(e);
        e.perp_dot(along).abs() < 0.17
            && s > 0.0
            && s < length
            && (mid - u).perp_dot(e).abs() < NEAR
    })
}

/// The stubs joining to a house the stretches' ends a stretch along its wall left free
/// ([`hugs`]): each end that meets no other stretch and stands within [`NEAR`] of a house, from
/// the end to the house's nearest wall, square to it. An end counts as on the house only within
/// half a thickness and 5 cm: within a whole thickness, a wall centred on a lot line 0.33 m
/// behind a house's back stopped beside it, its face 13 cm off (#308).
fn stubs(stretches: &[Stretch], footprints: &[Vec<DVec2>]) -> Vec<Stretch> {
    let mut out = Vec::new();
    for (i, s) in stretches.iter().enumerate() {
        for end in [s.a, s.b] {
            let met = stretches
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && geom::distance_to_segment(end, o.a, o.b) < JOIN)
                || footprints
                    .iter()
                    .any(|fp| boundary_distance(fp, end) < THICKNESS * 0.5 + 0.05);
            if met {
                continue;
            }
            let foot = footprints
                .iter()
                .flat_map(|fp| {
                    let m = fp.len();
                    (0..m).map(move |k| {
                        let (u, v) = (fp[k], fp[(k + 1) % m]);
                        u + (v - u) * geom::closest_on_segment(end, u, v)
                    })
                })
                .min_by(|x, y| x.distance(end).total_cmp(&y.distance(end)));
            if let Some(foot) = foot.filter(|f| f.distance(end) < NEAR + JOIN) {
                out.push(Stretch {
                    a: end,
                    b: foot,
                    owner: s.owner,
                    front: false,
                    street: false,
                    hedge: s.hedge,
                });
            }
        }
    }
    out
}

/// A wall's direction from `a` to `b`.
fn dir(w: &Wall) -> DVec2 {
    (w.b - w.a).normalize_or_zero()
}

/// How far along `u` from `c`, a point by `w`'s line, `w`'s thickness reaches, metres.
fn reach(w: &Wall, c: DVec2, u: DVec2) -> f64 {
    let t = dir(w);
    let normal = DVec2::new(-t.y, t.x);
    let (s, at) = (normal.dot(u), normal.dot(c - w.a));
    let (right, left) = w.band();
    let r = if s > 1e-6 {
        (left - at) / s
    } else if s < -1e-6 {
        (at + right) / -s
    } else {
        0.0
    };
    r.max(0.0)
}

/// How far `w` steps back from its end `c`, going `into` it, for both its faces (and its line)
/// to clear `other`'s thickness.
fn clearing(w: &Wall, c: DVec2, into: DVec2, other: &Wall) -> f64 {
    let t = dir(w);
    let normal = DVec2::new(-t.y, t.x);
    let (right, left) = w.band();
    [c + normal * left, c - normal * right, c]
        .iter()
        .map(|&p| reach(other, p, into))
        .fold(0.0, f64::max)
}

/// The mitre where wall `w`'s end `c` meets wall `o` at a corner: the face through the corners
/// their faces make, inner with inner and outer with outer (whichever side each one's thickness
/// takes), as where `w`'s line crosses it and the face's direction to `w`'s left. `None` where
/// the face leans past the 0.3 of square [`Wall::ring`] keeps, or crosses the line far off.
fn corner_mitre(w: &Wall, c: DVec2, o: &Wall) -> Option<(DVec2, DVec2)> {
    // A wall's band edges (left, right) as lines (a point, the direction).
    let edges = |x: &Wall| {
        let t = dir(x);
        let normal = DVec2::new(-t.y, t.x);
        let (right, left) = x.band();
        ((x.a + normal * left, t), (x.a - normal * right, t))
    };
    let meet = |(p, u): (DVec2, DVec2), (q, v): (DVec2, DVec2)| {
        let d = geom::cross(u, v);
        (d.abs() > 1e-9).then(|| p + u * (geom::cross(q - p, v) / d))
    };
    let (t, to) = (dir(w), dir(o));
    let (normal, o_normal) = (DVec2::new(-t.y, t.x), DVec2::new(-to.y, to.x));
    // The inner sides: each one's side towards the other, by the way each runs from the corner
    // (a short wall's middle, its end off the other's line, stood on the wrong side near 162°).
    let away = |x: &Wall, d: DVec2| {
        if x.a.distance(c) < x.b.distance(c) {
            d
        } else {
            -d
        }
    };
    let o_on_left = normal.dot(away(o, to)) > 0.0;
    let w_on_left = o_normal.dot(away(w, t)) > 0.0;
    let ((wl, wr), (ol, or)) = (edges(w), edges(o));
    let (w_in, w_out) = if o_on_left { (wl, wr) } else { (wr, wl) };
    let (o_in, o_out) = if w_on_left { (ol, or) } else { (or, ol) };
    let (inner, outer) = (meet(w_in, o_in)?, meet(w_out, o_out)?);
    let face = (outer - inner).normalize_or_zero();
    let at = meet((w.a, t), (inner, face))?;
    let face = if face.dot(normal) < 0.0 { -face } else { face };
    (face.dot(normal) >= 0.3 && at.distance(c) < 2.0 * THICKNESS).then_some((at, face))
}

/// Where walls meet, one butts against the other so that none overlaps another: two walls of a
/// height overlapping would put their tops on one plane, the flicker the owner sees
/// (`forge-geometry-pitfalls`). A wall ending within a wall's thickness of another: stops at the
/// face of one passing through; meets one in line (within 17°, its end within `IN_LINE_JOIN`) at
/// the middle of their two ends, their end faces mitred on the bisector; stops past the farthest
/// face of walls in line with each other crossing there; at a corner, clears the walls ranking above it (higher, then
/// longer, then first), or runs on past their far faces when it ranks above them all. Then
/// [`settle`] clears what is left.
fn joints(walls: &mut Vec<Wall>) {
    let n = walls.len();
    // Within about 17° of each other: a bend, not a corner.
    let in_line = |x: &Wall, y: &Wall| dir(x).perp_dot(dir(y)).abs() <= 0.3;
    // Each wall's move at `a` and at `b`: shorter by so much, or longer when negative; and its
    // mitred end faces.
    let mut moves = vec![(0.0, 0.0); n];
    let mut mitres: Vec<[Option<DVec2>; 2]> = vec![[None, None]; n];
    // Where an end moves to meet a wall in line with it: the middle of the two ends.
    let mut snaps: Vec<[Option<DVec2>; 2]> = vec![[None, None]; n];
    // The wall in line each end meets, whose direction its mitre takes once both have moved.
    let mut partners: Vec<[Option<usize>; 2]> = vec![[None, None]; n];
    // The end of `p` nearer `c`.
    let nearer = |p: &Wall, c: DVec2| {
        if p.a.distance(c) < p.b.distance(c) {
            p.a
        } else {
            p.b
        }
    };
    // The one wall at wall `i`'s end `c` (going `into` it), when it ends there too at a corner
    // [`MITRE`] sharp: its index, its end and the way into it.
    let corner_of = |i: usize, c: DVec2, into: DVec2| {
        let mut at = (0..n)
            .filter(|&j| j != i && geom::distance_to_segment(c, walls[j].a, walls[j].b) < JOIN);
        let (Some(j), None) = (at.next(), at.next()) else {
            return None;
        };
        let o = &walls[j];
        let near = nearer(o, c);
        let away = if near == o.a { dir(o) } else { -dir(o) };
        (near.distance(c) < JOIN
            && [MITRE, MITRE_OBTUSE]
                .iter()
                .any(|r| (r.0..r.1).contains(&into.dot(away))))
        .then_some((j, near, away))
    };
    // Each end's mitre at such a corner, both walls each other's only one there: the face's
    // direction to the wall's left.
    let mut corners: Vec<[Option<DVec2>; 2]> = vec![[None, None]; n];
    for i in 0..n {
        for end in [false, true] {
            let w = &walls[i];
            // The end, and the way from it into the wall.
            let (c, into) = if end { (w.b, -dir(w)) } else { (w.a, dir(w)) };
            // Both walls take the mitre or neither: one mitred against the other butted left the
            // corner open.
            let mitred = corner_of(i, c, into)
                .filter(|&(j, near, away)| corner_of(j, near, away).is_some_and(|k| k.0 == i))
                .filter(|&(j, near, _)| corner_mitre(&walls[j], near, w).is_some())
                .and_then(|(j, _, _)| corner_mitre(w, c, &walls[j]));
            if let Some((at, face)) = mitred {
                snaps[i][end as usize] = Some(at);
                corners[i][end as usize] = Some(face);
                continue;
            }
            let touching =
                |j: usize| j != i && geom::distance_to_segment(c, walls[j].a, walls[j].b) < JOIN;
            let crossing: Vec<usize> = (0..n)
                .filter(|&j| touching(j) && dir(&walls[j]).perp_dot(into).abs() > 0.3)
                .collect();
            let interior =
                |j: usize| walls[j].a.distance(c) > JOIN && walls[j].b.distance(c) > JOIN;
            // A stop clears the other wall with both its faces, not only its line: at a corner
            // off the square one face reaches further in.
            let t = dir(w);
            let farthest = |js: &[usize]| {
                js.iter()
                    .map(|&j| clearing(w, c, into, &walls[j]))
                    .fold(0.0, f64::max)
            };
            // In order: a wall passing through the end, which this one stops at even when a wall
            // in line with it ends there too (the two stop on either side); a wall in line ending
            // here, the two then meeting on their bisector, mitred; walls in line with each
            // other crossing here, this one stopping past the farthest of their faces; at a
            // corner, the walls ranking above this one (higher, then longer, then first), which
            // it clears, or, ranking above them all, it runs on past their far faces.
            let passing: Vec<usize> = crossing.iter().copied().filter(|&j| interior(j)).collect();
            // A wall in line whose nearer end stands within `IN_LINE_JOIN` of this one, and
            // within a wall's thickness of its line.
            let meets = |j: usize| {
                let near = nearer(&walls[j], c);
                j != i
                    && in_line(w, &walls[j])
                    && near.distance(c) < IN_LINE_JOIN
                    && (near - c).perp_dot(t).abs() < THICKNESS
            };
            let mv = if !passing.is_empty() {
                farthest(&passing)
            } else if let Some(j) = (0..n).find(|&j| meets(j)) {
                // The two ends meet at the middle of them, on one line: each moved along its own
                // line halfway, two walls 5 cm off each other's line (a court 5 cm in from the
                // lane) kept a 5 cm step, a dark upright strip in the wall (the owner,
                // 2026-10-16), and ends more than a thickness apart a slot. Mitred below, on
                // the walls' directions once moved.
                snaps[i][end as usize] = Some((c + nearer(&walls[j], c)) * 0.5);
                partners[i][end as usize] = Some(j);
                0.0
            } else if (0..n).any(|j| touching(j) && in_line(w, &walls[j])) {
                // In line, but its nearer end too far to meet: it stays as it is.
                continue;
            } else if crossing.is_empty() {
                continue;
            } else {
                let through: Vec<usize> = crossing
                    .iter()
                    .copied()
                    .filter(|&j| {
                        crossing
                            .iter()
                            .any(|&k| k != j && in_line(&walls[j], &walls[k]))
                    })
                    .collect();
                let rank = |k: usize| (walls[k].height, walls[k].length());
                // Whether wall `k`, clearing `other` at its end here, keeps [`SHORTEST`].
                let fits = |k: usize, other: usize| {
                    let x = &walls[k];
                    let near = nearer(x, c);
                    let into = if near == x.a { dir(x) } else { -dir(x) };
                    x.length() - clearing(x, near, into, &walls[other]) >= SHORTEST
                };
                // Whether `x` ranks above `y` here: higher, then longer, then first, but where two
                // walls meet alone, one that would go clearing the other while the other would not
                // runs on instead (a stub from a wall's end to a house it ran beside went, and the
                // slit stayed). Where more meet, that could go round in a circle, every wall
                // stepping back and the junction left open: the order alone.
                let pair = crossing.len() == 1;
                let over = |x: usize, y: usize| {
                    if pair && !fits(y, x) && fits(x, y) {
                        false
                    } else if pair && !fits(x, y) && fits(y, x) {
                        true
                    } else {
                        rank(x) > rank(y) || (rank(x) == rank(y) && x < y)
                    }
                };
                let above: Vec<usize> = crossing.iter().copied().filter(|&j| over(j, i)).collect();
                if !through.is_empty() {
                    farthest(&through)
                } else if !above.is_empty() {
                    farthest(&above)
                } else {
                    -crossing
                        .iter()
                        .map(|&j| reach(&walls[j], c, -into))
                        .fold(0.0, f64::max)
                }
            };
            if end {
                moves[i].1 = mv;
            } else {
                moves[i].0 = mv;
            }
        }
    }
    // Each wall's ends once moved; a wall reversed or too short goes.
    let placed: Vec<Option<(DVec2, DVec2)>> = walls
        .iter()
        .zip(&moves)
        .zip(&snaps)
        .map(|((w, &(at_a, at_b)), &[sa, sb])| {
            let t = dir(w);
            let (a, b) = (sa.unwrap_or(w.a + t * at_a), sb.unwrap_or(w.b - t * at_b));
            (a.distance(b) >= SHORTEST && (b - a).dot(t) > 0.0).then_some((a, b))
        })
        .collect();
    // An end that met a wall in line, mitred on the bisector of the two as they now run (on
    // their directions before the move, the two overlapped a hair, and `settle` opened the slot
    // again).
    for i in 0..n {
        for end in 0..2 {
            let Some(j) = partners[i][end] else {
                continue;
            };
            let (Some((a, b)), Some((pa, pb))) = (placed[i], placed[j]) else {
                continue;
            };
            let (t, d) = ((b - a).normalize_or_zero(), (pb - pa).normalize_or_zero());
            let d = if d.dot(t) < 0.0 { -d } else { d };
            let mean = (t + d).normalize_or_zero();
            mitres[i][end] = Some(DVec2::new(-mean.y, mean.x));
        }
        for end in 0..2 {
            if corners[i][end].is_some() {
                mitres[i][end] = corners[i][end];
            }
        }
    }
    let mut kept = Vec::with_capacity(n);
    for ((w, [ma, mb]), place) in walls.drain(..).zip(mitres).zip(placed) {
        let Some((a, b)) = place else {
            continue;
        };
        kept.push(Wall {
            a,
            b,
            ends: [ma.unwrap_or(w.ends[0]), mb.unwrap_or(w.ends[1])],
            ..w
        });
    }
    *walls = kept;
    settle(walls);
    fill_slots(walls);
}

/// Where two walls nearly in line still end apart (a junction of four lots' walls, one stopping
/// at a third wall's face and its partner at another's), a piece of wall along their line fills
/// the slot between them, its end faces theirs: a slot showed as a dark upright strip (the
/// owner, 2026-10-16). Only where nothing else stands in the slot, and only when the piece
/// shares no ground with another wall.
fn fill_slots(walls: &mut Vec<Wall>) {
    let n = walls.len();
    let mut pieces = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let (w, x) = (&walls[i], &walls[j]);
            if dir(w).perp_dot(dir(x)).abs() > 0.3 || w.height != x.height {
                continue;
            }
            // The facing ends: the pair of ends nearest each other.
            let ends = [(0, 0), (0, 1), (1, 0), (1, 1)].map(|(e, f)| {
                let p = if e == 0 { w.a } else { w.b };
                let q = if f == 0 { x.a } else { x.b };
                (p.distance(q), e, f, p, q)
            });
            let &(gap, e, f, p, q) = ends
                .iter()
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("four pairs");
            let t = dir(w);
            if !(0.005..IN_LINE_JOIN).contains(&gap) || (q - p).perp_dot(t).abs() >= THICKNESS {
                continue;
            }
            // The slot runs from `w`'s end on, away from `w`.
            let out = if e == 0 { -t } else { t };
            if (q - p).dot(out) <= 0.0 {
                continue;
            }
            let mid = (p + q) * 0.5;
            if walls
                .iter()
                .any(|o| geom::contains(&o.ring(), mid) || o.a.distance(mid) < 1e-3)
            {
                continue;
            }
            // The piece from `p` to `q`, its end faces `w`'s at `p` and `x`'s at `q`, each
            // turned to the piece's left.
            let mut piece = Wall::new(p, q, w.height, w.inside.or(x.inside));
            let left = {
                let u = (q - p).normalize_or_zero();
                DVec2::new(-u.y, u.x)
            };
            let facing = |v: DVec2| if v.dot(left) < 0.0 { -v } else { v };
            piece.ends = [facing(w.ends[e]), facing(x.ends[f])];
            let ring = piece.ring();
            let shared = walls
                .iter()
                .chain(&pieces)
                .map(|o| overlap(&ring, &o.ring()))
                .fold(0.0, f64::max);
            if shared > 1e-3 {
                continue;
            }
            // A corner's sliver shared with a wall crossing there (a few square centimetres):
            // the piece a centimetre lower, so the two tops never lie on one plane.
            if shared > 1e-6 {
                piece.height -= 0.01;
            }
            pieces.push(piece);
        }
    }
    walls.extend(pieces);
}

/// What the joints leave sharing ground (two corners a little apart, each joined on its own):
/// of two walls, the one ranking lower (lower, then shorter, then later) steps back from its end
/// nearer the other until both its faces clear it, and goes when too little of it is left.
fn settle(walls: &mut Vec<Wall>) {
    let rank = |w: &Wall| (w.height, w.length());
    let near = |x: &Wall, y: &Wall| {
        let lo = |w: &Wall| w.a.min(w.b) - DVec2::splat(THICKNESS);
        let hi = |w: &Wall| w.a.max(w.b) + DVec2::splat(THICKNESS);
        lo(x).cmple(hi(y)).all() && lo(y).cmple(hi(x)).all()
    };
    for _ in 0..4 {
        let mut moved = false;
        let n = walls.len();
        for i in 0..n {
            for j in 0..n {
                let (w, o) = (&walls[i], &walls[j]);
                let below = rank(w) < rank(o) || (rank(w) == rank(o) && i > j);
                if i == j || !below || !near(w, o) || overlap(&w.ring(), &o.ring()) < 1e-4 {
                    continue;
                }
                let at_b = geom::distance_to_segment(w.b, o.a, o.b)
                    < geom::distance_to_segment(w.a, o.a, o.b);
                let (c, into) = if at_b { (w.b, -dir(w)) } else { (w.a, dir(w)) };
                let back = clearing(w, c, into, o);
                if back < 1e-9 {
                    continue;
                }
                let t = dir(w);
                let w = &mut walls[i];
                if at_b {
                    w.b -= t * back.min(w.length());
                } else {
                    w.a += t * back.min(w.length());
                }
                moved = true;
            }
        }
        walls.retain(|w| w.length() >= SHORTEST);
        if !moved {
            break;
        }
    }
}

/// The area two convex rings share, square metres (the walls' rings are).
pub(crate) fn overlap(x: &[DVec2], y: &[DVec2]) -> f64 {
    let mut part = x.to_vec();
    let n = y.len();
    for i in 0..n {
        if part.len() < 3 {
            return 0.0;
        }
        part = geom::clip_half_plane(&part, y[i], y[(i + 1) % n] - y[i]);
    }
    if part.len() < 3 {
        0.0
    } else {
        geom::signed_area(&part).abs()
    }
}

/// The distance from `p` to `polygon`'s outline, metres.
fn boundary_distance(polygon: &[DVec2], p: DVec2) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| geom::distance_to_segment(p, polygon[i], polygon[(i + 1) % n]))
        .fold(f64::MAX, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::parcels::{self, ParcelType};

    fn rectangle(w: f64, h: f64) -> Vec<DVec2> {
        vec![
            DVec2::ZERO,
            DVec2::new(w, 0.0),
            DVec2::new(w, h),
            DVec2::new(0.0, h),
        ]
    }

    /// The part of `lot` within `depth` of its street side: a house the lot's width.
    fn house(lot: &[DVec2], depth: f64) -> Vec<DVec2> {
        let (a, b) = (lot[0], lot[1]);
        let t = (b - a).normalize();
        let inward = DVec2::new(-t.y, t.x);
        geom::clean(&geom::clip_half_plane(lot, a + inward * depth, -t), 1e-6)
    }

    #[test]
    fn lanieres_are_walled_where_no_house_stands_with_a_gate_on_the_lane() {
        // Lanières along the bottom, 40 m deep on a block 90 by 70; the houses 10 m deep.
        let block = rectangle(90.0, 70.0);
        let ty = ParcelType {
            depth: 40.0,
            ..ParcelType::FAUBOURG
        };
        let street = [true, false, false, false];
        let (lots, court, _) =
            parcels::strips_by(&block, &street, &[40.0, 0.0, 0.0, 0.0], &ty, |_| 9.0)
                .expect("lanières");
        let footprints: Vec<Vec<DVec2>> = lots.iter().map(|l| house(l, 10.0)).collect();
        // The gardens behind the houses.
        let gardens: Vec<Vec<DVec2>> = lots
            .iter()
            .map(|l| {
                let t = (l[1] - l[0]).normalize();
                let inward = DVec2::new(-t.y, t.x);
                geom::clean(&geom::clip_half_plane(l, l[0] + inward * 10.0, t), 1e-6)
            })
            .collect();
        let g = garden_walls(
            &block,
            &lots,
            &court,
            &footprints,
            &vec![true; lots.len()],
            &gardens,
            STREET,
        );
        // No wall along a house nor through one.
        for w in &g.walls {
            let mid = (w.a + w.b) * 0.5;
            assert!(
                footprints
                    .iter()
                    .all(|fp| !geom::contains(fp, mid) && boundary_distance(fp, mid) > ON),
                "a wall on a house at {mid}"
            );
        }
        // Between the gardens: each party line from the house's back to the court, 30 m, once,
        // a wall or a hedge; the gardens' backs on the court, 90 m, a hedge.
        let party: f64 = g
            .walls
            .iter()
            .filter(|w| w.inside.is_none())
            .map(Wall::length)
            .sum();
        let hedges: f64 = g.hedges.iter().map(|(a, b)| a.distance(*b)).sum();
        let back: f64 = g
            .hedges
            .iter()
            .filter(|(a, b)| (a.y - 40.0).abs() < 0.1 && (b.y - 40.0).abs() < 0.1)
            .map(|(a, b)| a.distance(*b))
            .sum();
        assert!(
            (back - 90.0).abs() < 1.0,
            "{back:.1} m of hedge on the court"
        );
        // Less what the joints take from the walls: each stops at the lanes' walls.
        let lines = (lots.len() - 1) as f64 * 30.0 + 90.0;
        assert!(
            (party + hedges - lines).abs() < 3.0,
            "{party:.1} m of wall and {hedges:.1} of hedge between gardens, {lines:.1} expected"
        );
        assert!(
            party > 0.0 && hedges > back,
            "walls and hedges between the gardens"
        );
        // Along the lanes: the two corner gardens' sides (30 m each), less their gates; the
        // court, the block's common ground, open to the lanes.
        let street: f64 = g
            .walls
            .iter()
            .filter(|w| w.inside.is_some())
            .map(Wall::length)
            .sum();
        let gates = g.gates.len() as f64;
        assert_eq!(g.gates.len(), 2, "gates at {:?}", g.gates);
        let expected = 2.0 * 30.0 - gates * GATE;
        assert!(
            (street - expected).abs() < 2.0,
            "{street:.1} m along the lanes, {expected:.1} expected"
        );
        // No two walls share ground: their tops would lie on one plane.
        for (i, x) in g.walls.iter().enumerate() {
            for y in &g.walls[i + 1..] {
                let shared = overlap(&x.ring(), &y.ring());
                assert!(shared < 1e-3, "walls share {shared:.4} m²: {x:?} {y:?}");
            }
        }
        assert!(
            g.walls
                .iter()
                .all(|w| w.inside.is_none() || w.height == STREET)
        );
    }

    #[test]
    fn a_lotissements_front_is_low_with_a_gate_before_the_house() {
        // One lot 20 by 35, its house 10 m wide set 5 m back and 3 m off the sides.
        let lot = rectangle(20.0, 35.0);
        let fp = vec![
            DVec2::new(3.0, 5.0),
            DVec2::new(13.0, 5.0),
            DVec2::new(13.0, 15.0),
            DVec2::new(3.0, 15.0),
        ];
        let lots = std::slice::from_ref(&lot);
        let g = garden_walls(&lot, lots, &[], &[fp], &[true], lots, FRONT);
        let front: Vec<&Wall> = g
            .walls
            .iter()
            .filter(|w| w.a.y.abs() < 1e-9 && w.b.y.abs() < 1e-9)
            .collect();
        assert!(front.iter().all(|w| w.height == FRONT));
        let length: f64 = front.iter().map(|w| w.length()).sum();
        // Less the side walls' thickness at each corner: they are higher and run on.
        assert!(
            (length - (20.0 - GATE - 2.0 * THICKNESS)).abs() < 1e-6,
            "{length}"
        );
        assert_eq!(g.gates.len(), 1);
        assert!(
            (g.gates[0] - DVec2::new(8.0, 0.0)).length() < 1e-6,
            "{}",
            g.gates[0]
        );
    }
}
