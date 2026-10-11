//! Transit on the plan: tram lines routed through their waypoints by the shortest path that
//! prefers main roads (costs in whole millimetres, so ties break the same everywhere), stops at
//! a spacing snapped to nearby junctions, boat crossings where the rivers run long between
//! bridges, and the street profiles that follow (`city-generation.md` §12).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::DVec2;

use super::geom;
use super::graph::RoadGraph;
use super::{
    DistrictKind, DistrictSpec, LAND, Profile, RoadClass, Site, TramLineSpec, district_index,
};
use crate::field::Field2;

/// The radius a tram line's corners are rounded to, metres: about a low-floor tram's tightest in
/// street running (18–25 m), the tighter so the curve stays in the junction's carriageway where
/// two avenues meet.
pub const TRAM_RADIUS: f64 = 18.0;

/// A routed tram line.
#[derive(Clone, Debug, PartialEq)]
pub struct TramLine {
    /// Its name.
    pub name: String,
    /// Its colour on the map.
    pub colour: [u8; 3],
    /// The road edges it runs on, in order.
    pub edges: Vec<u32>,
    /// Its route, metres on the plan.
    pub points: Vec<DVec2>,
    /// Its stops.
    pub stops: Vec<DVec2>,
}

/// A boat crossing of a river.
#[derive(Clone, Debug, PartialEq)]
pub struct BoatCrossing {
    /// The river's index in the site.
    pub river: usize,
    /// One landing.
    pub a: DVec2,
    /// The other landing.
    pub b: DVec2,
}

/// The nodes that a main road touches.
fn main_nodes(graph: &RoadGraph) -> Vec<bool> {
    let mut main = vec![false; graph.nodes.len()];
    for e in &graph.edges {
        if e.class == RoadClass::Main {
            main[e.a as usize] = true;
            main[e.b as usize] = true;
        }
    }
    main
}

/// The main-road node nearest `p`.
pub fn nearest_main_node(graph: &RoadGraph, p: DVec2) -> Option<DVec2> {
    let main = main_nodes(graph);
    nearest_node(graph, p, &main).map(|i| graph.nodes[i].pos)
}

/// The nearest node to `p` among those `allowed` marks (or any node if none is).
fn nearest_node(graph: &RoadGraph, p: DVec2, allowed: &[bool]) -> Option<usize> {
    let any = allowed.iter().any(|&a| a);
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !any || allowed[*i])
        .min_by(|(i, a), (j, b)| {
            a.pos
                .distance_squared(p)
                .partial_cmp(&b.pos.distance_squared(p))
                .expect("finite")
                .then(i.cmp(j))
        })
        .map(|(i, _)| i)
}

/// The shortest path from `from` to `to` as edge indices, main roads cheaper than streets.
fn shortest_path(
    graph: &RoadGraph,
    adjacency: &[Vec<(u32, u32, u64)>],
    from: usize,
    to: usize,
) -> Option<Vec<u32>> {
    let n = graph.nodes.len();
    let mut dist = vec![u64::MAX; n];
    let mut via: Vec<Option<(u32, u32)>> = vec![None; n];
    let mut heap = BinaryHeap::new();
    dist[from] = 0;
    heap.push(Reverse((0_u64, from as u32)));
    while let Some(Reverse((d, v))) = heap.pop() {
        let v = v as usize;
        if d > dist[v] {
            continue;
        }
        if v == to {
            break;
        }
        for &(w, e, cost) in &adjacency[v] {
            let nd = d + cost;
            if nd < dist[w as usize] {
                dist[w as usize] = nd;
                via[w as usize] = Some((v as u32, e));
                heap.push(Reverse((nd, w)));
            }
        }
    }
    if dist[to] == u64::MAX {
        return None;
    }
    let mut edges = Vec::new();
    let mut v = to;
    while v != from {
        let (prev, e) = via[v]?;
        edges.push(e);
        v = prev as usize;
    }
    edges.reverse();
    Some(edges)
}

/// Routes each tram line through its waypoints and places its stops.
pub fn route_trams(graph: &RoadGraph, specs: &[TramLineSpec], stop_spacing: f64) -> Vec<TramLine> {
    let mut adjacency: Vec<Vec<(u32, u32, u64)>> = vec![Vec::new(); graph.nodes.len()];
    for (i, e) in graph.edges.iter().enumerate() {
        let factor = if e.class == RoadClass::Main { 1.0 } else { 3.0 };
        let cost = (e.length() * factor * 1000.0).round() as u64;
        adjacency[e.a as usize].push((e.b, i as u32, cost));
        adjacency[e.b as usize].push((e.a, i as u32, cost));
    }
    let main = main_nodes(graph);
    let mut lines = Vec::new();
    for spec in specs {
        // Each waypoint to the nearest main-road node, or to the nearest node of any street
        // when no main road comes within 150 m.
        let any = vec![false; graph.nodes.len()];
        let nodes: Vec<usize> = spec
            .waypoints
            .iter()
            .filter_map(|&p| {
                let on_main = nearest_node(graph, p, &main)?;
                if graph.nodes[on_main].pos.distance(p) <= 150.0 {
                    Some(on_main)
                } else {
                    nearest_node(graph, p, &any)
                }
            })
            .collect();
        let mut edges = Vec::new();
        let mut node_path = vec![];
        for pair in nodes.windows(2) {
            if pair[0] == pair[1] {
                continue;
            }
            if let Some(path) = shortest_path(graph, &adjacency, pair[0], pair[1]) {
                edges.extend(path);
            }
        }
        // The route's points, each edge walked from the node the last one ended on.
        let mut points: Vec<DVec2> = Vec::new();
        let mut at = nodes.first().copied().unwrap_or(0) as u32;
        node_path.push(at);
        for &e in &edges {
            let edge = &graph.edges[e as usize];
            let forward = edge.a == at;
            let pts: Vec<DVec2> = if forward {
                edge.points.clone()
            } else {
                edge.points.iter().rev().copied().collect()
            };
            if points.is_empty() {
                points.extend(pts);
            } else {
                points.extend_from_slice(&pts[1..]);
            }
            at = if forward { edge.b } else { edge.a };
            node_path.push(at);
        }
        // Stops every `stop_spacing`, moved to a junction of the route within 80 m.
        let length = geom::polyline_length(&points);
        let count = (length / stop_spacing).round().max(1.0) as usize;
        let mut stops: Vec<DVec2> = Vec::new();
        for k in 0..=count {
            let p = geom::point_at(&points, length * k as f64 / count as f64);
            let junction = node_path
                .iter()
                .map(|&v| graph.nodes[v as usize].pos)
                .filter(|q| q.distance(p) < 80.0)
                .min_by(|a, b| {
                    a.distance_squared(p)
                        .partial_cmp(&b.distance_squared(p))
                        .expect("finite")
                });
            let stop = junction.unwrap_or(p);
            if stops
                .last()
                .is_none_or(|&last| last.distance(stop) > stop_spacing * 0.4)
            {
                stops.push(stop);
            }
        }
        if points.len() >= 2 {
            lines.push(TramLine {
                name: spec.name.clone(),
                colour: spec.colour,
                edges,
                // Simplified, then each corner rounded: trams turn on arcs, not at a point (the
                // owner saw the tracks turn at right angles in a junction).
                points: geom::fillet(&geom::simplify(&points, 0.5), TRAM_RADIUS, 2.0),
                stops,
            });
        }
    }
    lines
}

/// Lines added until `target` of the built blocks have a stop within `catchment` metres (or
/// `max_lines` are added). Each new line serves the largest cluster of blocks left without a
/// stop: it runs along the cluster's long axis (the principal direction of the blocks within
/// 1.5 km) between its farthest blocks, from the nearest existing stop. Pure and in a fixed
/// order, like the rest of the plan.
pub fn cover(
    graph: &RoadGraph,
    centres: &[DVec2],
    lines: &[TramLine],
    catchment: f64,
    stop_spacing: f64,
    target: f64,
    max_lines: usize,
) -> Vec<TramLine> {
    const PALETTE: [[u8; 3]; 6] = [
        [96, 96, 96],
        [160, 110, 60],
        [60, 130, 130],
        [130, 60, 90],
        [90, 120, 40],
        [70, 70, 150],
    ];
    let mut added: Vec<TramLine> = Vec::new();
    let mut tried: Vec<DVec2> = Vec::new();
    for k in 0..max_lines * 3 {
        if added.len() >= max_lines {
            break;
        }
        let stops: Vec<DVec2> = lines
            .iter()
            .chain(added.iter())
            .flat_map(|l| l.stops.iter().copied())
            .collect();
        let uncovered: Vec<DVec2> = centres
            .iter()
            .copied()
            .filter(|c| !stops.iter().any(|s| s.distance(*c) <= catchment))
            .collect();
        let share = 1.0 - uncovered.len() as f64 / centres.len().max(1) as f64;
        if share >= target || uncovered.is_empty() {
            break;
        }
        // The densest uncovered block not tried yet: the most uncovered neighbours within
        // 600 m.
        let Some(&seed) = uncovered
            .iter()
            .filter(|c| !tried.iter().any(|t| t.distance(**c) <= 300.0))
            .max_by_key(|c| {
                uncovered
                    .iter()
                    .filter(|o| o.distance(**c) <= 600.0)
                    .count()
            })
        else {
            break;
        };
        tried.push(seed);
        let cluster: Vec<DVec2> = uncovered
            .iter()
            .copied()
            .filter(|c| c.distance(seed) <= 1500.0)
            .collect();
        let mean = cluster.iter().copied().sum::<DVec2>() / cluster.len() as f64;
        // The principal axis of the cluster (the covariance's larger eigenvector).
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for c in &cluster {
            let d = *c - mean;
            sxx += d.x * d.x;
            sxy += d.x * d.y;
            syy += d.y * d.y;
        }
        let angle = 0.5 * forge_core::dmath::atan2(2.0 * sxy, sxx - syy);
        let (s, c) = forge_core::dmath::sin_cos(angle);
        let axis = DVec2::new(c, s);
        let along = |p: DVec2| (p - mean).dot(axis);
        let (lo, hi) = cluster.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(along(*p)), hi.max(along(*p)))
        });
        let (a, b) = (mean + axis * lo, mean + axis * hi);
        // From the existing stop nearest either end.
        let nearest = |p: DVec2| {
            stops.iter().copied().min_by(|x, y| {
                x.distance_squared(p)
                    .partial_cmp(&y.distance_squared(p))
                    .expect("finite")
            })
        };
        let waypoints = match (nearest(a), nearest(b)) {
            (Some(sa), Some(sb)) if sa.distance(a) <= sb.distance(b) => vec![sa, a, b],
            (_, Some(sb)) => vec![sb, b, a],
            _ => vec![a, b],
        };
        let spec = TramLineSpec {
            name: format!("auto {}", k + 1),
            colour: PALETTE[k % PALETTE.len()],
            waypoints,
        };
        let routed = route_trams(graph, std::slice::from_ref(&spec), stop_spacing);
        let Some(line) = routed.into_iter().next() else {
            continue;
        };
        // A line must serve at least a few blocks that had no stop.
        let gain = uncovered
            .iter()
            .filter(|c| line.stops.iter().any(|s| s.distance(**c) <= catchment))
            .count();
        if gain < 5 {
            continue;
        }
        let mut line = line;
        line.name = format!("auto {}", added.len() + 1);
        line.colour = PALETTE[added.len() % PALETTE.len()];
        added.push(line);
    }
    added
}

/// Boat crossings where a river runs more than `gap` metres without a bridge, evenly spaced,
/// both landings on land.
pub fn boat_crossings(site: &Site, graph: &RoadGraph, gap: f64) -> Vec<BoatCrossing> {
    let mut crossings = Vec::new();
    for (r, river) in site.rivers.iter().enumerate() {
        let line = &river.centreline;
        let inside = |p: DVec2| p.x >= 0.0 && p.y >= 0.0 && p.x <= site.size && p.y <= site.size;
        // The stretch of the centreline inside the frame, as distances along it.
        let length = geom::polyline_length(line);
        let mut s = 0.0;
        let (mut first, mut last) = (None, 0.0);
        while s <= length {
            if inside(geom::point_at(line, s)) {
                first.get_or_insert(s);
                last = s;
            }
            s += 10.0;
        }
        let Some(first) = first else { continue };
        // Where the bridges cross the centreline.
        let mut marks = vec![first, last];
        for e in graph.edges.iter().filter(|e| e.bridge) {
            for w in e.points.windows(2) {
                let mut along = 0.0;
                for c in line.windows(2) {
                    if let Some((_, u)) = geom::segment_intersection(w[0], w[1], c[0], c[1]) {
                        marks.push(along + c[0].distance(c[1]) * u);
                    }
                    along += c[0].distance(c[1]);
                }
            }
        }
        marks.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        for pair in marks.windows(2) {
            let span = pair[1] - pair[0];
            let k = (span / gap).floor() as usize;
            for j in 0..k {
                let s = pair[0] + span * (j + 1) as f64 / (k + 1) as f64;
                let p = geom::point_at(line, s);
                let t = geom::tangent_at(line, s);
                let n = DVec2::new(-t.y, t.x) * (river.width * 0.5 + 12.0);
                let (a, b) = (p + n, p - n);
                if site.water_at(a) == LAND && site.water_at(b) == LAND {
                    crossings.push(BoatCrossing { river: r, a, b });
                }
            }
        }
    }
    crossings
}

/// The street profiles: tram avenues where the trams run, pedestrian lanes in the old town,
/// paths in the parks, shared streets elsewhere.
pub fn assign_profiles(
    graph: &mut RoadGraph,
    trams: &[TramLine],
    districts_field: &Field2<u8>,
    districts: &[DistrictSpec],
) {
    let mut tram = vec![false; graph.edges.len()];
    for line in trams {
        for &e in &line.edges {
            tram[e as usize] = true;
        }
    }
    for (i, e) in graph.edges.iter_mut().enumerate() {
        let mid = geom::point_at(&e.points, e.length() * 0.5);
        let kind = district_index(districts_field, mid).map(|d| districts[d].kind);
        e.profile = match (e.class, tram[i], kind) {
            (_, true, _) => Profile::TramAvenue,
            (RoadClass::Main, false, _) => Profile::Avenue,
            (RoadClass::Street, false, Some(DistrictKind::OldTown)) => Profile::Pedestrian,
            (RoadClass::Street, false, Some(k)) if k.is_green() => Profile::Path,
            (RoadClass::Street, false, _) => Profile::Shared,
        };
    }
}
