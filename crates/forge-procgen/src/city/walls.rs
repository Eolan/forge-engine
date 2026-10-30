//! The walls round a village's gardens (#266). The villages of the cadastre research
//! (`docs/research/cadastre-villages-and-towns.md`) wall their gardens along the streets and lanes
//! wherever no house stands, low on a lotissement's front, over which the house is seen; between
//! the gardens, and between a garden and the common court, hedges (the owner, 2026-10-29: walls
//! only along the streets). Each garden keeps a gate on its street side, level with the street
//! (the owner's rule: no step where people walk).

use glam::DVec2;

use super::geom;

/// A garden wall's thickness, metres: rubble stone under a lime render.
pub const THICKNESS: f64 = 0.4;
/// A garden wall's height along a street or a lane, metres: over a walker's eyes.
pub const STREET: f64 = 1.8;
/// A lotissement's front wall, metres: low, the house seen over it from the street.
pub const FRONT: f64 = 1.0;
/// A gate's width, metres: a wheelchair's passage.
pub const GATE: f64 = 1.2;
/// How far a gate's side stays from a house its wall runs to, metres: past the bed or the
/// planter along the house's wall.
const FROM_HOUSE: f64 = 1.5;
/// How close a point lies to a house's outline or to the block's to count as on it, metres.
const ON: f64 = 0.08;
/// The shortest stretch of wall kept, metres.
const SHORTEST: f64 = 0.3;
/// The shortest piece of hedge kept, metres: one shrub. Dropped like a wall's piece, a jog of
/// half a metre in a lot's back left the hedge after it ending in the air.
const HEDGE_SHORTEST: f64 = 0.05;
/// How near a hedge's end must come to a wall or another hedge to meet it, metres: a
/// wall's thickness.
const JOIN: f64 = THICKNESS;
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
    /// A short wall closing a garden from a street wall's end to a house standing back from the
    /// street ([`garden_walls`]): off the street and the lots' lines by design.
    pub cut: bool,
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
            cut: false,
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

/// A stretch of a lot's outline where a hedge stands.
struct Stretch {
    a: DVec2,
    b: DVec2,
    /// The lot it closes.
    #[allow(dead_code)]
    owner: usize,
}

/// The walls and hedges of a block cut into `lots` within its `outline` (anticlockwise, as
/// `lots::cut` cuts them), the houses' `footprints` standing on the lots and `housed[k]` saying
/// whether lot `k` has its house (D-083's fifth step; the owner, 2026-10-29: walls only along the
/// streets and lanes, hedges inside, "the wall placements are over complicated"):
///
/// - **Walls** stand along the block's outline wherever a housed lot fronts the street and no
///   house stands on it or before it (nearer than [`NEAR`]), `front` high ([`STREET`], or a
///   lotissement's [`FRONT`]), their thickness on the gardens' side. Each runs along one edge of
///   the outline, and where two meet at a bend or a corner their ends are mitred on the bisector:
///   one line round the block, no joint to settle. A wall's end in front of a house standing back
///   from the street takes a short wall square to the house.
/// - **Gates:** each lot with a wall keeps a gate in its longest stretch (its front's first),
///   before its house on its front, clear of the stretch's ends and of a house beside it.
/// - **Hedges** run along every edge between two lots and between a lot and the court, where no
///   house stands, once; an end left in the air before a house joins it.
///
/// The court is the block's common ground, open to the lanes, and so are the lots without a
/// house: walled all round they were yards of walls with nothing in them (the owner, 2026-10-16).
pub fn garden_walls(
    outline: &[DVec2],
    lots: &[Vec<DVec2>],
    footprints: &[Vec<DVec2>],
    housed: &[bool],
    front: f64,
) -> GardenWalls {
    let n = outline.len();
    let mut out = GardenWalls::default();
    if n < 3 {
        return out;
    }
    let is_housed = |k: usize| housed.get(k).copied().unwrap_or(true);
    let origin = |e: usize| outline[e];
    let length = |e: usize| outline[e].distance(outline[(e + 1) % n]);
    let along = |e: usize| (outline[(e + 1) % n] - outline[e]).normalize_or_zero();
    let inward = |e: usize| {
        let t = along(e);
        DVec2::new(-t.y, t.x)
    };
    let on_edge = |e: usize, p: DVec2| {
        geom::distance_to_segment(p, outline[e], outline[(e + 1) % n]) < 2.0 * ON
    };
    // Each edge's frontages: where a housed lot runs along it, as distances along the edge, the
    // lot, and whether that is its front.
    let mut frontages: Vec<Vec<(f64, f64, usize, bool)>> = vec![Vec::new(); n];
    for (k, lot) in lots.iter().enumerate().filter(|&(k, _)| is_housed(k)) {
        // The corners other lots put in line on its sides left out: its whole side at once.
        let whole = geom::clean(lot, 1e-4);
        let m = whole.len();
        for i in 0..m {
            let (p, q) = (whole[i], whole[(i + 1) % m]);
            let Some(e) =
                (0..n).find(|&e| on_edge(e, p) && on_edge(e, q) && on_edge(e, (p + q) * 0.5))
            else {
                continue;
            };
            let (s0, s1) = ((p - origin(e)).dot(along(e)), (q - origin(e)).dot(along(e)));
            frontages[e].push((s0.min(s1), s0.max(s1), k, p.distance(lot[0]) < 1e-6));
        }
    }
    // Each edge's stretches no house stands on or before: the frontages, joined where two lots'
    // meet, less the houses' cover.
    let mut stretches: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n];
    for e in 0..n {
        let mut spans: Vec<(f64, f64)> = frontages[e].iter().map(|&(a, b, _, _)| (a, b)).collect();
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut joined: Vec<(f64, f64)> = Vec::new();
        for (a, b) in spans {
            match joined.last_mut() {
                Some(last) if a <= last.1 + 1e-6 => last.1 = last.1.max(b),
                _ => joined.push((a, b)),
            }
        }
        let cover = house_cover(origin(e), along(e), inward(e), length(e), footprints);
        stretches[e] = subtract(&joined, &cover)
            .into_iter()
            .filter(|&(a, b)| b - a >= SHORTEST)
            .collect();
    }
    // The gates: each lot's in its longest stretch of wall within its own frontage, its front's
    // first, before its house on its front, clear of the stretch's ends (by half a gate, a
    // joint and the shortest wall) and of a house at either end (by FROM_HOUSE).
    let half = GATE * 0.5 + JOIN + SHORTEST;
    let mut gates: Vec<Vec<f64>> = vec![Vec::new(); n];
    for k in (0..lots.len()).filter(|&k| is_housed(k)) {
        let mut candidates: Vec<(usize, f64, f64, bool)> = Vec::new();
        for e in 0..n {
            for &(f0, f1, owner, is_front) in &frontages[e] {
                if owner != k {
                    continue;
                }
                for &(a, b) in &stretches[e] {
                    let (a, b) = (a.max(f0), b.min(f1));
                    if b - a >= GATE + 2.0 * (JOIN + SHORTEST) {
                        candidates.push((e, a, b, is_front));
                    }
                }
            }
        }
        candidates.sort_by(|x, y| {
            (y.3, y.2 - y.1)
                .partial_cmp(&(x.3, x.2 - x.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some(&(e, a, b, is_front)) = candidates.first() else {
            continue;
        };
        let at_point = |s: f64| origin(e) + along(e) * s;
        // Before the house on a front: its middle, along the street.
        let wanted = if is_front {
            footprints
                .iter()
                .map(|fp| geom::centroid(fp))
                .min_by(|x, y| {
                    x.distance(at_point((a + b) * 0.5))
                        .total_cmp(&y.distance(at_point((a + b) * 0.5)))
                })
                .map_or((a + b) * 0.5, |c| (c - origin(e)).dot(along(e)))
        } else {
            (a + b) * 0.5
        };
        let by_house = |s: f64| {
            footprints
                .iter()
                .any(|fp| boundary_distance(fp, at_point(s)) < NEAR + JOIN)
        };
        let (lo, hi) = (
            a + half + if by_house(a) { FROM_HOUSE } else { 0.0 },
            b - half - if by_house(b) { FROM_HOUSE } else { 0.0 },
        );
        let (lo, hi) = if lo <= hi {
            (lo, hi)
        } else {
            (a + half, b - half)
        };
        let g = wanted.clamp(lo, hi);
        gates[e].push(g);
        out.gates.push(at_point(g));
    }
    // The walls: each stretch less its gates, one wall a piece, their ends mitred where two
    // pieces meet at the outline's corners.
    let mut pieces: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n];
    for e in 0..n {
        // In order along the edge (`subtract` takes them so): the lots' gates came lot by lot,
        // and a corner lot's gate on its side street came before its neighbours' there.
        let mut openings: Vec<(f64, f64)> = gates[e]
            .iter()
            .map(|&g| (g - GATE * 0.5, g + GATE * 0.5))
            .collect();
        openings.sort_by(|x, y| x.0.total_cmp(&y.0));
        pieces[e] = subtract(&stretches[e], &openings)
            .into_iter()
            .filter(|&(a, b)| b - a >= SHORTEST)
            .collect();
    }
    // The bisector of the two edges' squares at corner `v` (the start of edge `v`), when a piece
    // runs to it on both sides.
    let mitre = |v: usize| {
        let h = (v + n - 1) % n;
        let ends_there = pieces[h].last().is_some_and(|p| p.1 > length(h) - 1e-6);
        let starts_there = pieces[v].first().is_some_and(|p| p.0 < 1e-6);
        (ends_there && starts_there).then(|| (inward(h) + inward(v)).normalize_or_zero())
    };
    for (e, run) in pieces.iter().enumerate() {
        let square = inward(e);
        for &(a, b) in run {
            let (pa, pb) = (origin(e) + along(e) * a, origin(e) + along(e) * b);
            let mut wall = Wall::new(pa, pb, front, Some((pa + pb) * 0.5 + square));
            if a < 1e-6
                && let Some(m) = mitre(e)
            {
                wall.ends[0] = m;
            }
            if b > length(e) - 1e-6
                && let Some(m) = mitre((e + 1) % n)
            {
                wall.ends[1] = m;
            }
            out.walls.push(wall);
        }
    }
    // The hedges: the housed lots' edges inside the block, each once (the lots share their
    // corners on these edges, so a line two lots meet on comes twice, reversed), where no house
    // stands nor runs along it nearer than NEAR.
    let mut seen: Vec<(DVec2, DVec2)> = Vec::new();
    let mut hedges: Vec<Stretch> = Vec::new();
    for (k, lot) in lots.iter().enumerate().filter(|&(k, _)| is_housed(k)) {
        let m = lot.len();
        for i in 0..m {
            let (p, q) = (lot[i], lot[(i + 1) % m]);
            let street =
                (0..n).any(|e| on_edge(e, p) && on_edge(e, q) && on_edge(e, (p + q) * 0.5));
            let twice = seen.iter().any(|&(u, v)| {
                (u.distance(q) < 1e-6 && v.distance(p) < 1e-6)
                    || (u.distance(p) < 1e-6 && v.distance(q) < 1e-6)
            });
            if street || twice || p.distance(q) < HEDGE_SHORTEST {
                continue;
            }
            seen.push((p, q));
            hedges.extend(
                open_runs(p, q, footprints)
                    .into_iter()
                    .map(|(a, b)| Stretch { a, b, owner: k }),
            );
        }
    }
    // A wall's end before a house standing back from the street (nearer than NEAR, no wall
    // before it): a short wall from the end to the house, square to its wall, closing the
    // garden. A wall stopping beside a house's corner on the street line needs none, nor one
    // whose end a hedge meets (the lot's side running in from the street).
    let mut joins = Vec::new();
    for (i, w) in out.walls.iter().enumerate() {
        for end in [w.a, w.b] {
            let walled = out
                .walls
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && geom::distance_to_segment(end, o.a, o.b) < JOIN);
            if walled
                || footprints
                    .iter()
                    .any(|fp| boundary_distance(fp, end) < THICKNESS * 0.5 + 0.05)
                || out
                    .gates
                    .iter()
                    .any(|&g| g.distance(end) < GATE * 0.5 + 0.05)
                || hedges
                    .iter()
                    .any(|h| geom::distance_to_segment(end, h.a, h.b) < JOIN)
            {
                continue;
            }
            if let Some(foot) =
                foot_on_house(end, footprints).filter(|f| f.distance(end) < NEAR + JOIN)
            {
                joins.push(Wall {
                    cut: true,
                    ..Wall::new(end, foot, front, None)
                });
            }
        }
    }
    out.walls
        .extend(joins.into_iter().filter(|w| w.length() >= SHORTEST * 0.5));
    let joined = stubs(&hedges, &out.walls, footprints);
    out.hedges = hedges.iter().chain(&joined).map(|s| (s.a, s.b)).collect();
    out
}

/// What of the edge from `origin` along `along` (to its `inward` side, `length` long) the houses
/// stand on or before: where its wall's band would run into a house, and where a house's wall
/// runs along it nearer than [`NEAR`] (the house's own wall closes the garden there), as
/// distances along it, joined.
fn house_cover(
    origin: DVec2,
    along: DVec2,
    inward: DVec2,
    length: f64,
    footprints: &[Vec<DVec2>],
) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for fp in footprints {
        let m = fp.len();
        // The band's three lines (its two faces 2 cm in, and its middle) through the house.
        for d in [0.02, THICKNESS * 0.5, THICKNESS - 0.02] {
            let line = origin + inward * d;
            let mut cuts: Vec<f64> = Vec::new();
            for i in 0..m {
                let (u, v) = (fp[i], fp[(i + 1) % m]);
                let (su, sv) = ((u - line).perp_dot(along), (v - line).perp_dot(along));
                if (su > 0.0) != (sv > 0.0) {
                    let p = u + (v - u) * (su / (su - sv));
                    cuts.push((p - line).dot(along));
                }
            }
            cuts.sort_by(f64::total_cmp);
            for pair in cuts.chunks(2) {
                if let [a, b] = *pair {
                    spans.push((a, b));
                }
            }
        }
        // A house wall along the edge, nearer than NEAR.
        for i in 0..m {
            let (u, v) = (fp[i], fp[(i + 1) % m]);
            let e = (v - u).normalize_or_zero();
            let (du, dv) = ((u - origin).dot(inward), (v - origin).dot(inward));
            if e.perp_dot(along).abs() < 0.17 && du.max(dv) < NEAR && du.min(dv) > -ON {
                let (a, b) = ((u - origin).dot(along), (v - origin).dot(along));
                spans.push((a.min(b), a.max(b)));
            }
        }
    }
    let mut spans: Vec<(f64, f64)> = spans
        .into_iter()
        .map(|(a, b)| (a.max(0.0), b.min(length)))
        .filter(|&(a, b)| b > a)
        .collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut joined: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match joined.last_mut() {
            Some(last) if a <= last.1 + 1e-6 => last.1 = last.1.max(b),
            _ => joined.push((a, b)),
        }
    }
    joined
}

/// `spans` (sorted, apart) less `holes` (sorted, apart), as distances along one line.
fn subtract(spans: &[(f64, f64)], holes: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(a, b) in spans {
        let mut start = a;
        for &(h0, h1) in holes {
            if h1 <= start || h0 >= b {
                continue;
            }
            if h0 > start {
                out.push((start, h0));
            }
            start = start.max(h1);
        }
        if b > start {
            out.push((start, b));
        }
    }
    out
}

/// The pieces of the segment `p q` no house stands on and no house's wall runs along nearer
/// than [`NEAR`] (where the house's own wall closes the garden), each [`HEDGE_SHORTEST`] at least.
fn open_runs(p: DVec2, q: DVec2, footprints: &[Vec<DVec2>]) -> Vec<(DVec2, DVec2)> {
    let length = p.distance(q);
    let along = (q - p) / length;
    // Where a piece may begin or end: where a house's outline crosses or touches the segment, and
    // the feet of its corners near it.
    let mut ts = vec![0.0, 1.0];
    for fp in footprints {
        let m = fp.len();
        for i in 0..m {
            let (u, v) = (fp[i], fp[(i + 1) % m]);
            if let Some((t, _)) = geom::segment_intersection(p, q, u, v) {
                ts.push(t);
            }
            for w in [u, v] {
                if geom::distance_to_segment(w, p, q) < NEAR {
                    ts.push(geom::closest_on_segment(w, p, q));
                }
            }
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
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
        if housed || footprints.iter().any(|fp| hugs(fp, mid, along)) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if (run.1 - t0).abs() < 1e-9 => run.1 = t1,
            _ => runs.push((t0, t1)),
        }
    }
    runs.into_iter()
        .filter(|&(t0, t1)| (t1 - t0) * length >= HEDGE_SHORTEST)
        .map(|(t0, t1)| (p + (q - p) * t0, p + (q - p) * t1))
        .collect()
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

/// The point of the houses' walls nearest `p`.
fn foot_on_house(p: DVec2, footprints: &[Vec<DVec2>]) -> Option<DVec2> {
    footprints
        .iter()
        .flat_map(|fp| {
            let m = fp.len();
            (0..m).map(move |k| {
                let (u, v) = (fp[k], fp[(k + 1) % m]);
                u + (v - u) * geom::closest_on_segment(p, u, v)
            })
        })
        .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
}

/// The hedges joining to a house the hedges' ends a hedge along its wall left free ([`hugs`]):
/// each end that meets no other hedge, no wall and no house, and stands within [`NEAR`] of a
/// house, from the end to the house's nearest wall. An end counts as on the house only within
/// half a thickness and 5 cm (#308).
fn stubs(hedges: &[Stretch], walls: &[Wall], footprints: &[Vec<DVec2>]) -> Vec<Stretch> {
    let mut out = Vec::new();
    for (i, s) in hedges.iter().enumerate() {
        for end in [s.a, s.b] {
            let met = hedges
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && geom::distance_to_segment(end, o.a, o.b) < JOIN)
                || walls
                    .iter()
                    .any(|w| geom::distance_to_segment(end, w.a, w.b) < JOIN)
                || footprints
                    .iter()
                    .any(|fp| boundary_distance(fp, end) < THICKNESS * 0.5 + 0.05);
            if met {
                continue;
            }
            if let Some(foot) =
                foot_on_house(end, footprints).filter(|f| f.distance(end) < NEAR + JOIN)
            {
                out.push(Stretch {
                    a: end,
                    b: foot,
                    owner: s.owner,
                });
            }
        }
    }
    out
}

/// The area two convex rings share, square metres (the walls' rings are).
#[cfg(test)]
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
        let (lots, _, _) =
            parcels::strips_by(&block, &street, &[40.0, 0.0, 0.0, 0.0], &ty, |_| 9.0)
                .expect("lanières");
        let footprints: Vec<Vec<DVec2>> = lots.iter().map(|l| house(l, 10.0)).collect();
        let g = garden_walls(&block, &lots, &footprints, &vec![true; lots.len()], STREET);
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
        // a hedge; the gardens' backs on the court, 90 m, a hedge; no wall inside the block.
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
        let lines = (lots.len() - 1) as f64 * 30.0 + 90.0;
        assert!(
            (hedges - lines).abs() < 1.0,
            "{hedges:.1} m of hedge between gardens, {lines:.1} expected"
        );
        assert!(party == 0.0, "{party:.1} m of wall between the gardens");
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
        let g = garden_walls(&lot, lots, &[fp], &[true], FRONT);
        let front: Vec<&Wall> = g
            .walls
            .iter()
            .filter(|w| w.a.y.abs() < 1e-9 && w.b.y.abs() < 1e-9)
            .collect();
        assert!(front.iter().all(|w| w.height == FRONT));
        let length: f64 = front.iter().map(|w| w.length()).sum();
        // The whole front less the gate, mitred onto the side walls at the corners.
        assert!((length - (20.0 - GATE)).abs() < 1e-6, "{length}");
        assert_eq!(g.gates.len(), 1);
        assert!(
            (g.gates[0] - DVec2::new(8.0, 0.0)).length() < 1e-6,
            "{}",
            g.gates[0]
        );
    }
}
