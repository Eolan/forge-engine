//! The road graph: the traced streets cut where they cross or meet, their ends snapped into
//! shared nodes, short dead ends pruned, and the blocks found as the graph's bounded faces
//! (each half-edge followed by the next one clockwise round its end node).

use glam::DVec2;

use super::geom::{self, PointGrid};
use super::streets::Streamline;
use super::{LAND, Profile, RoadClass, Site};

/// A crossing, a bend or an end of the road graph.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadNode {
    /// Where it is, metres on the plan.
    pub pos: DVec2,
}

/// A road between two nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadEdge {
    /// Its first node.
    pub a: u32,
    /// Its last node.
    pub b: u32,
    /// Its centreline, from `a` to `b`.
    pub points: Vec<DVec2>,
    /// Main road or street.
    pub class: RoadClass,
    /// Its street profile (who may use it).
    pub profile: Profile,
    /// Whether it crosses water.
    pub bridge: bool,
}

impl RoadEdge {
    /// Its length, metres.
    pub fn length(&self) -> f64 {
        geom::polyline_length(&self.points)
    }
}

/// The road network as a planar graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadGraph {
    /// The nodes.
    pub nodes: Vec<RoadNode>,
    /// The roads.
    pub edges: Vec<RoadEdge>,
}

/// A cut in a polyline: its segment and the parameter along it.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct Cut {
    segment: usize,
    t: f64,
}

/// Builds the planar graph of the traced streets.
pub fn build(site: &Site, lines: &[Streamline], snap: f64, prune: f64) -> RoadGraph {
    let size = site.size;
    // Every segment, bucketed by the cells its box touches.
    let cell = 50.0;
    let side = (size / cell).ceil() as usize + 1;
    let mut buckets: Vec<Vec<(u32, u32)>> = vec![Vec::new(); side * side];
    let clampi = |v: f64| (v / cell).floor().clamp(0.0, (side - 1) as f64) as usize;
    for (li, line) in lines.iter().enumerate() {
        for (si, w) in line.points.windows(2).enumerate() {
            let (lo, hi) = (w[0].min(w[1]), w[0].max(w[1]));
            for y in clampi(lo.y)..=clampi(hi.y) {
                for x in clampi(lo.x)..=clampi(hi.x) {
                    buckets[y * side + x].push((li as u32, si as u32));
                }
            }
        }
    }
    let mut cuts: Vec<Vec<Cut>> = vec![Vec::new(); lines.len()];
    // Crossings: each pair once, in the first bucket they share.
    let mut pairs: Vec<((u32, u32), (u32, u32))> = Vec::new();
    for bucket in &buckets {
        for (i, &s) in bucket.iter().enumerate() {
            for &o in &bucket[i + 1..] {
                let (p, q) = if s < o { (s, o) } else { (o, s) };
                if p.0 == q.0 && q.1.abs_diff(p.1) <= 1 {
                    continue;
                }
                pairs.push((p, q));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let seg = |(l, s): (u32, u32)| {
        let pts = &lines[l as usize].points;
        (pts[s as usize], pts[s as usize + 1])
    };
    for (p, q) in pairs {
        let (p0, p1) = seg(p);
        let (q0, q1) = seg(q);
        if let Some((t, u)) = geom::segment_intersection(p0, p1, q0, q1) {
            cuts[p.0 as usize].push(Cut {
                segment: p.1 as usize,
                t,
            });
            cuts[q.0 as usize].push(Cut {
                segment: q.1 as usize,
                t: u,
            });
        }
    }
    // Ends that touch another street (the joins) cut it there.
    for (li, line) in lines.iter().enumerate() {
        for &end in [line.points[0], *line.points.last().expect("not empty")].iter() {
            let (x, y) = (clampi(end.x), clampi(end.y));
            let mut best: Option<(f64, u32, u32, f64)> = None;
            for &(l, s) in &buckets[y * side + x] {
                if l as usize == li {
                    continue;
                }
                let (a, b) = seg((l, s));
                let t = geom::closest_on_segment(end, a, b);
                let d = end.distance(a + (b - a) * t);
                if d <= snap && best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, l, s, t));
                }
            }
            if let Some((_, l, s, t)) = best {
                cuts[l as usize].push(Cut {
                    segment: s as usize,
                    t,
                });
            }
        }
    }
    // Cut every street into pieces between its cuts.
    let mut pieces: Vec<(Vec<DVec2>, RoadClass)> = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut c = std::mem::take(&mut cuts[li]);
        c.sort_by(|x, y| x.partial_cmp(y).expect("finite"));
        split_line(line, &c, &mut pieces);
    }
    assemble(site, pieces, snap, prune)
}

/// Splits a polyline at its sorted cuts into pieces.
fn split_line(line: &Streamline, cuts: &[Cut], pieces: &mut Vec<(Vec<DVec2>, RoadClass)>) {
    let pts = &line.points;
    let mut current = vec![pts[0]];
    let mut next_vertex = 1;
    for cut in cuts {
        while next_vertex <= cut.segment {
            current.push(pts[next_vertex]);
            next_vertex += 1;
        }
        let p = pts[cut.segment] + (pts[cut.segment + 1] - pts[cut.segment]) * cut.t;
        current.push(p);
        if geom::polyline_length(&current) > 1e-6 {
            pieces.push((std::mem::take(&mut current), line.class));
        }
        current = vec![p];
    }
    while next_vertex < pts.len() {
        current.push(pts[next_vertex]);
        next_vertex += 1;
    }
    if geom::polyline_length(&current) > 1e-6 {
        pieces.push((current, line.class));
    }
}

/// Snaps the pieces' ends into nodes, drops loops and duplicates, prunes short dead ends.
fn assemble(site: &Site, pieces: Vec<(Vec<DVec2>, RoadClass)>, snap: f64, prune: f64) -> RoadGraph {
    let mut graph = RoadGraph::default();
    let mut node_grid = PointGrid::new(site.size, 20.0);
    let mut node_of = |p: DVec2, graph: &mut RoadGraph| -> u32 {
        if let Some((_, id)) = node_grid.nearest(p, snap) {
            return id;
        }
        let id = graph.nodes.len() as u32;
        graph.nodes.push(RoadNode { pos: p });
        node_grid.insert(p, id);
        id
    };
    let mut seen: Vec<(u32, u32, i64)> = Vec::new();
    for (mut points, class) in pieces {
        let a = node_of(points[0], &mut graph);
        let b = node_of(*points.last().expect("not empty"), &mut graph);
        let length = geom::polyline_length(&points);
        if a == b && length < 4.0 * snap {
            continue;
        }
        points[0] = graph.nodes[a as usize].pos;
        let last = points.len() - 1;
        points[last] = graph.nodes[b as usize].pos;
        let key = (a.min(b), a.max(b), (length * 0.5).round() as i64);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let points = geom::simplify(&points, 0.25);
        let mid = geom::point_at(&points, length * 0.5);
        let bridge = site.water_at(mid) != LAND;
        graph.edges.push(RoadEdge {
            a,
            b,
            points,
            class,
            profile: Profile::default_for(class),
            bridge,
        });
    }
    prune_dead_ends(&mut graph, prune);
    graph
}

/// Removes dead ends shorter than `prune` metres until none is left, then the nodes nothing
/// uses, renumbering the rest in order.
fn prune_dead_ends(graph: &mut RoadGraph, prune: f64) {
    loop {
        let mut degree = vec![0_u32; graph.nodes.len()];
        for e in &graph.edges {
            degree[e.a as usize] += 1;
            degree[e.b as usize] += 1;
        }
        let before = graph.edges.len();
        graph.edges.retain(|e| {
            let dead = degree[e.a as usize] == 1 || degree[e.b as usize] == 1;
            !(dead && e.length() < prune)
        });
        if graph.edges.len() == before {
            break;
        }
    }
    let mut used = vec![false; graph.nodes.len()];
    for e in &graph.edges {
        used[e.a as usize] = true;
        used[e.b as usize] = true;
    }
    let mut remap = vec![u32::MAX; graph.nodes.len()];
    let mut nodes = Vec::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        if used[i] {
            remap[i] = nodes.len() as u32;
            nodes.push(n.clone());
        }
    }
    for e in &mut graph.edges {
        e.a = remap[e.a as usize];
        e.b = remap[e.b as usize];
    }
    graph.nodes = nodes;
}

/// A monotonic stand-in for the angle of a direction, in `[0, 4)`, exact (no trigonometry).
fn pseudo_angle(d: DVec2) -> f64 {
    let p = d.x / (d.x.abs() + d.y.abs());
    if d.y >= 0.0 { 1.0 - p } else { 3.0 + p }
}

/// The graph's bounded faces as closed polygons, in a fixed order.
pub fn faces(graph: &RoadGraph) -> Vec<Vec<DVec2>> {
    let n = graph.nodes.len();
    // Half-edge 2e runs a→b, 2e+1 runs b→a. Out-going half-edges per node, sorted by angle.
    let mut out: Vec<Vec<(f64, u32)>> = vec![Vec::new(); n];
    for (i, e) in graph.edges.iter().enumerate() {
        let k = e.points.len();
        let da = e.points[1] - e.points[0];
        let db = e.points[k - 2] - e.points[k - 1];
        out[e.a as usize].push((pseudo_angle(da), 2 * i as u32));
        out[e.b as usize].push((pseudo_angle(db), 2 * i as u32 + 1));
    }
    for list in &mut out {
        list.sort_by(|x, y| x.partial_cmp(y).expect("finite"));
    }
    let origin = |h: u32| {
        let e = &graph.edges[(h / 2) as usize];
        if h.is_multiple_of(2) { e.a } else { e.b }
    };
    // Position of each half-edge in its origin's list.
    let mut slot = vec![0_usize; graph.edges.len() * 2];
    for list in &out {
        for (i, &(_, h)) in list.iter().enumerate() {
            slot[h as usize] = i;
        }
    }
    let next = |h: u32| -> u32 {
        let twin = h ^ 1;
        let v = origin(twin) as usize;
        let list = &out[v];
        let i = slot[twin as usize];
        list[(i + list.len() - 1) % list.len()].1
    };
    let mut visited = vec![false; graph.edges.len() * 2];
    let mut result = Vec::new();
    for start in 0..(graph.edges.len() * 2) as u32 {
        if visited[start as usize] {
            continue;
        }
        let mut polygon: Vec<DVec2> = Vec::new();
        let mut h = start;
        let mut steps = 0;
        loop {
            visited[h as usize] = true;
            let e = &graph.edges[(h / 2) as usize];
            if h.is_multiple_of(2) {
                polygon.extend_from_slice(&e.points[..e.points.len() - 1]);
            } else {
                polygon.extend(e.points[1..].iter().rev());
            }
            h = next(h);
            steps += 1;
            if h == start || steps > graph.edges.len() * 2 {
                break;
            }
        }
        if polygon.len() >= 3 && geom::signed_area(&polygon) > 0.0 {
            result.push(polygon);
        }
    }
    result
}

/// What makes a network hard to build good streets on, counted so it is fixed by number, not by
/// eye (D-061; the owner: "junctions, road intersections and bridges are still a mess").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defects {
    /// Pairs of junctions (three ways or more) closer than [`CLOSE`]: one junction drawn as a
    /// cluster.
    pub close_junctions: usize,
    /// Streets under [`CLOSE`] long between two junctions.
    pub short_streets: usize,
    /// Pairs of ways leaving a node within [`SHARP`] degrees of each other.
    pub sharp_meetings: usize,
    /// Nodes standing on water: a junction on a bridge, or two bridges crossing.
    pub wet_nodes: usize,
    /// Pairs of bridges whose crossings lie within [`BRIDGE_GAP`] of each other.
    pub close_bridges: usize,
    /// Streets running beside another so close that their areas overlap for 30 m or more.
    pub doubled_streets: usize,
}

/// Junctions closer than this are one junction, metres.
pub const CLOSE: f64 = 20.0;
/// Ways meeting at a node closer than this in direction meet too sharply, degrees.
pub const SHARP: f64 = 25.0;
/// Bridges closer than this cross at one place, metres.
pub const BRIDGE_GAP: f64 = 150.0;

/// The middle of each stretch of an edge over water longer than 10 m, sampled every 2 m.
fn wet_middles(site: &Site, points: &[DVec2]) -> Vec<DVec2> {
    let length = geom::polyline_length(points);
    let mut out = Vec::new();
    let mut run: Option<(f64, f64)> = None;
    let mut s = 0.0;
    while s <= length + 1e-9 {
        let wet = site.water_at(geom::point_at(points, s)) != LAND;
        match (wet, run) {
            (true, None) => run = Some((s, s)),
            (true, Some((a, _))) => run = Some((a, s)),
            (false, Some((a, b))) => {
                if b - a > 10.0 {
                    out.push(geom::point_at(points, (a + b) * 0.5));
                }
                run = None;
            }
            (false, None) => {}
        }
        s += 2.0;
    }
    if let Some((a, b)) = run
        && b - a > 10.0
    {
        out.push(geom::point_at(points, (a + b) * 0.5));
    }
    out
}

/// The network's [`Defects`].
pub fn defects(graph: &RoadGraph, site: &Site) -> Defects {
    let n = graph.nodes.len();
    let mut d = Defects::default();
    let mut degree = vec![0_usize; n];
    let mut dirs: Vec<Vec<DVec2>> = vec![Vec::new(); n];
    for e in &graph.edges {
        let k = e.points.len();
        if k < 2 {
            continue;
        }
        degree[e.a as usize] += 1;
        degree[e.b as usize] += 1;
        dirs[e.a as usize].push((e.points[1] - e.points[0]).normalize_or_zero());
        dirs[e.b as usize].push((e.points[k - 2] - e.points[k - 1]).normalize_or_zero());
    }
    let mut grid = PointGrid::new(site.size, 40.0);
    for (i, node) in graph.nodes.iter().enumerate() {
        if degree[i] >= 3 {
            grid.insert(node.pos, i as u32);
        }
        if site.water_at(node.pos) != LAND {
            d.wet_nodes += 1;
        }
        let cos = SHARP.to_radians().cos();
        let list = &dirs[i];
        for a in 0..list.len() {
            for b in a + 1..list.len() {
                if list[a].dot(list[b]) > cos {
                    d.sharp_meetings += 1;
                }
            }
        }
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        if degree[i] >= 3 {
            grid.for_each_within(node.pos, CLOSE, |_, j| {
                if (j as usize) > i && graph.nodes[j as usize].pos.distance(node.pos) < CLOSE {
                    d.close_junctions += 1;
                }
            });
        }
    }
    for e in &graph.edges {
        if degree[e.a as usize] >= 3 && degree[e.b as usize] >= 3 && e.length() < CLOSE {
            d.short_streets += 1;
        }
    }
    // Bridges: each wet stretch's middle, and the pairs of them close together.
    let wet: Vec<(u32, DVec2)> = graph
        .edges
        .iter()
        .enumerate()
        .flat_map(|(k, e)| {
            wet_middles(site, &e.points)
                .into_iter()
                .map(move |p| (k as u32, p))
        })
        .collect();
    for (i, &(ea, pa)) in wet.iter().enumerate() {
        for &(eb, pb) in &wet[i + 1..] {
            if ea != eb && pa.distance(pb) < BRIDGE_GAP {
                d.close_bridges += 1;
            }
        }
    }
    // Doubled streets: samples every 10 m, past 30 m from either end, that lie within both
    // streets' half widths of another street that shares no node with it.
    let mut samples = PointGrid::new(site.size, 40.0);
    for (k, e) in graph.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 5.0) {
            samples.insert(p, k as u32);
        }
    }
    for (k, e) in graph.edges.iter().enumerate() {
        let length = e.length();
        let half = e.profile.width() * 0.5;
        let mut close = 0;
        let mut s = 30.0;
        while s < length - 30.0 {
            let p = geom::point_at(&e.points, s);
            let mut hit = false;
            samples.for_each_within(p, half + 12.0, |q, j| {
                let o = &graph.edges[j as usize];
                let shares = o.a == e.a || o.a == e.b || o.b == e.a || o.b == e.b;
                if j as usize != k && !shares && p.distance(q) < half + o.profile.width() * 0.5 {
                    hit = true;
                }
            });
            if hit {
                close += 1;
            }
            s += 10.0;
        }
        if close >= 3 {
            d.doubled_streets += 1;
        }
    }
    d
}

/// Cleans the network before streets are built on it (D-061), so each junction is one place:
/// - junctions closer than [`CLOSE`], or joined by a street shorter than it, become one, at
///   their main roads' middle (else all of theirs), each street's end eased onto it over its
///   last 30 m;
/// - of two streets between the same junctions, of a pair leaving a node within [`SHARP`]
///   degrees, and of a pair running doubled side by side, the lesser goes (a street before a
///   main road, the shorter first);
/// - then the dead ends shorter than `prune`.
///
/// Returns how many nodes were merged away and streets dropped.
pub fn tidy(graph: &mut RoadGraph, site: &Site, prune: f64) -> (usize, usize) {
    let n = graph.nodes.len();
    let mut degree = vec![0_usize; n];
    for e in &graph.edges {
        degree[e.a as usize] += 1;
        degree[e.b as usize] += 1;
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    // The pairs to join, nearest first: a short street's two ends, and junctions near each other.
    let mut pairs: Vec<(i64, usize, usize)> = Vec::new();
    for e in &graph.edges {
        let (a, b) = (e.a as usize, e.b as usize);
        if a != b && degree[a] >= 3 && degree[b] >= 3 && e.length() < CLOSE {
            pairs.push(((e.length() * 1000.0) as i64, a.min(b), a.max(b)));
        }
    }
    let mut grid = PointGrid::new(site.size, 40.0);
    for (i, node) in graph.nodes.iter().enumerate() {
        if degree[i] >= 3 {
            grid.insert(node.pos, i as u32);
        }
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        if degree[i] < 3 {
            continue;
        }
        grid.for_each_within(node.pos, CLOSE, |q, j| {
            let j = j as usize;
            if j > i && q.distance(node.pos) < CLOSE {
                pairs.push(((q.distance(node.pos) * 1000.0) as i64, i, j));
            }
        });
    }
    pairs.sort_unstable();
    // Each group stays one place: every member within `CLOSE` of its middle. Joined
    // without a bound, junction after junction chained into one across a dense grid, and its
    // streets collapsed.
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    for &(_, a, b) in &pairs {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra == rb {
            continue;
        }
        let all: Vec<usize> = members[ra].iter().chain(&members[rb]).copied().collect();
        let middle = all.iter().map(|&k| graph.nodes[k].pos).sum::<DVec2>() / all.len() as f64;
        if all
            .iter()
            .any(|&k| graph.nodes[k].pos.distance(middle) > CLOSE)
        {
            continue;
        }
        let (keep, gone) = (ra.min(rb), ra.max(rb));
        parent[gone] = keep;
        members[keep] = all;
        members[gone].clear();
    }
    // Each group's place: its main-road members' middle, else all its members'.
    let mut main = vec![false; n];
    for e in &graph.edges {
        if e.class == RoadClass::Main {
            main[e.a as usize] = true;
            main[e.b as usize] = true;
        }
    }
    let roots: Vec<usize> = (0..n).map(|i| root(&mut parent, i)).collect();
    let mut sum = vec![(DVec2::ZERO, 0.0_f64, DVec2::ZERO, 0.0_f64); n];
    for i in 0..n {
        let s = &mut sum[roots[i]];
        s.2 += graph.nodes[i].pos;
        s.3 += 1.0;
        if main[i] {
            s.0 += graph.nodes[i].pos;
            s.1 += 1.0;
        }
    }
    let place = |r: usize| {
        let s = sum[r];
        if s.1 > 0.0 { s.0 / s.1 } else { s.2 / s.3 }
    };
    let merged = (0..n).filter(|&i| roots[i] != i).count();
    for i in 0..n {
        if roots[i] == i {
            graph.nodes[i].pos = place(i);
        }
    }
    let mut edges = Vec::with_capacity(graph.edges.len());
    let mut dropped = 0;
    for mut e in std::mem::take(&mut graph.edges) {
        let (a, b) = (roots[e.a as usize], roots[e.b as usize]);
        let length = e.length();
        if a == b && length < 4.0 * CLOSE {
            dropped += 1;
            continue;
        }
        // Each moved end: the street's first stretch, twice the move and 5 m, cut, and the rest
        // run straight on to the junction's new place. Easing the points along instead folded
        // a street back on itself where its junction moved in along it.
        let (pa, pb) = (graph.nodes[a].pos, graph.nodes[b].pos);
        let cut = |end: DVec2, now: DVec2| {
            let d = end.distance(now);
            if d < 1e-6 {
                0.0
            } else {
                (2.0 * d + 5.0).min(length * 0.45)
            }
        };
        let (ca, cb) = (
            cut(e.points[0], pa),
            cut(*e.points.last().expect("not empty"), pb),
        );
        let mut moved = vec![pa];
        let mut s = 0.0;
        if ca > 0.0 {
            moved.push(geom::point_at(&e.points, ca));
        }
        for w in e.points.windows(2) {
            s += w[0].distance(w[1]);
            if s > ca + 1e-6 && s < length - cb - 1e-6 {
                moved.push(w[1]);
            }
        }
        if cb > 0.0 {
            moved.push(geom::point_at(&e.points, length - cb));
        }
        moved.push(pb);
        // Points the move brought together go: a street needs a direction at each end.
        moved.dedup_by(|p, q| p.distance(*q) < 0.05);
        if moved.len() < 2 || geom::polyline_length(&moved) < 1.0 {
            dropped += 1;
            continue;
        }
        let last = moved.len() - 1;
        moved[last] = graph.nodes[b].pos;
        e.points = moved;
        e.a = a as u32;
        e.b = b as u32;
        edges.push(e);
    }
    graph.edges = edges;
    // The lesser of two: a street before a main road, the shorter first.
    let lesser = |graph: &RoadGraph, x: usize, y: usize| {
        let key = |k: usize| {
            let e = &graph.edges[k];
            (
                e.class == RoadClass::Main,
                (e.length() * 1000.0) as i64,
                std::cmp::Reverse(k),
            )
        };
        if key(x) < key(y) { x } else { y }
    };
    let m = graph.edges.len();
    let mut gone = vec![false; m];
    // Two streets between the same junctions whose middles lie within their half widths of each
    // other: one street drawn twice. Farther apart they hold a block between them.
    let mut by_pair: std::collections::BTreeMap<(u32, u32), Vec<usize>> = Default::default();
    for (k, e) in graph.edges.iter().enumerate() {
        by_pair
            .entry((e.a.min(e.b), e.a.max(e.b)))
            .or_default()
            .push(k);
    }
    for list in by_pair.values() {
        for (i, &x) in list.iter().enumerate() {
            for &y in &list[i + 1..] {
                let (lx, ly) = (graph.edges[x].length(), graph.edges[y].length());
                let (ex, ey) = (&graph.edges[x], &graph.edges[y]);
                let apart = geom::point_at(&ex.points, lx * 0.5)
                    .distance(geom::point_at(&ey.points, ly * 0.5));
                let overlap = (ex.profile.width() + ey.profile.width()) * 0.5;
                if !gone[x] && !gone[y] && lx.max(ly) < 1.5 * lx.min(ly) && apart < overlap {
                    gone[lesser(graph, x, y)] = true;
                }
            }
        }
    }
    // Two ways leaving a node too close in direction.
    let cos = SHARP.to_radians().cos();
    let mut arms: Vec<Vec<(usize, DVec2)>> = vec![Vec::new(); graph.nodes.len()];
    for (k, e) in graph.edges.iter().enumerate() {
        let p = &e.points;
        let q = p.len();
        if q < 2 {
            continue;
        }
        arms[e.a as usize].push((k, (p[1] - p[0]).normalize_or_zero()));
        arms[e.b as usize].push((k, (p[q - 2] - p[q - 1]).normalize_or_zero()));
    }
    for list in &arms {
        for (i, &(x, dx)) in list.iter().enumerate() {
            for &(y, dy) in &list[i + 1..] {
                if x != y && !gone[x] && !gone[y] && dx.dot(dy) > cos {
                    gone[lesser(graph, x, y)] = true;
                }
            }
        }
    }
    // Two streets running doubled side by side, sharing no node.
    let mut samples = PointGrid::new(site.size, 40.0);
    for (k, e) in graph.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 5.0) {
            samples.insert(p, k as u32);
        }
    }
    for k in 0..m {
        if gone[k] {
            continue;
        }
        let e = &graph.edges[k];
        let (length, half) = (e.length(), e.profile.width() * 0.5);
        let mut hits: std::collections::BTreeMap<usize, usize> = Default::default();
        let mut s = 30.0;
        while s < length - 30.0 {
            let p = geom::point_at(&e.points, s);
            let mut seen = Vec::new();
            samples.for_each_within(p, half + 12.0, |q, j| {
                let j = j as usize;
                let o = &graph.edges[j];
                let shares = o.a == e.a || o.a == e.b || o.b == e.a || o.b == e.b;
                if j != k
                    && !gone[j]
                    && !shares
                    && p.distance(q) < half + o.profile.width() * 0.5
                    && !seen.contains(&j)
                {
                    seen.push(j);
                }
            });
            for j in seen {
                *hits.entry(j).or_default() += 1;
            }
            s += 10.0;
        }
        for (j, count) in hits {
            if count >= 3 && !gone[k] && !gone[j] {
                gone[lesser(graph, k, j)] = true;
            }
        }
    }
    let before = graph.edges.len();
    let mut k = 0;
    graph.edges.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
    dropped += before - graph.edges.len();
    let edges_before = graph.edges.len();
    prune_dead_ends(graph, prune);
    dropped += edges_before - graph.edges.len();
    (merged, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[(f64, f64)]) -> Streamline {
        Streamline {
            points: points.iter().map(|&(x, y)| DVec2::new(x, y)).collect(),
            minor: false,
            class: RoadClass::Street,
            bridges: Vec::new(),
        }
    }

    #[test]
    fn a_grid_of_streets_makes_its_blocks() {
        let site = Site::flat(400.0, 10.0);
        let mut lines = Vec::new();
        for i in 0..4 {
            let c = 50.0 + 100.0 * i as f64;
            lines.push(line(&[(20.0, c), (380.0, c)]));
            lines.push(line(&[(c, 20.0), (c, 380.0)]));
        }
        let graph = build(&site, &lines, 1.0, 40.0);
        let blocks = faces(&graph);
        assert_eq!(
            blocks.len(),
            9,
            "a 4 × 4 grid of streets encloses 3 × 3 blocks"
        );
        for b in &blocks {
            assert!((geom::signed_area(b) - 10_000.0).abs() < 1e-6);
        }
        // The stubs past the outer streets are 30 m dead ends, pruned.
        assert_eq!(graph.nodes.len(), 16);
    }

    #[test]
    fn a_street_that_ends_on_another_makes_a_junction() {
        let site = Site::flat(200.0, 10.0);
        let lines = vec![
            line(&[(0.0, 100.0), (200.0, 100.0)]),
            line(&[(100.0, 0.0), (100.0, 100.0)]),
        ];
        let graph = build(&site, &lines, 1.0, 0.0);
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.nodes.len(), 4);
    }
}
