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
