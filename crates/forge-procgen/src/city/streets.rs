//! Streets traced as streamlines of the tensor field (Chen et al. 2008, §4): seeds taken in a
//! fixed order, each traced both ways with a midpoint step along its family's eigenvector until
//! it leaves the frame, meets water it may not bridge, comes within `dtest` of a street of its
//! family or reaches its length; then each end that stopped short is joined to the nearest
//! street ahead, which makes the T-junctions. Main roads are traced first and may bridge the
//! rivers; streets follow at their district's spacing and treat the main roads as their own.

use std::collections::VecDeque;

use forge_core::SplitMix64;
use glam::DVec2;

use super::geom::PointGrid;
use super::tensor::TensorField;
use super::{LAND, RIVER, RoadClass, Site};

/// A traced street before the graph is built.
#[derive(Clone, Debug)]
pub struct Streamline {
    /// Its points, metres on the plan.
    pub points: Vec<DVec2>,
    /// The tensor field's minor family (else the major one).
    pub minor: bool,
    /// Its class.
    pub class: RoadClass,
    /// Point index ranges `[start, end]` that cross water: bridges.
    pub bridges: Vec<(usize, usize)>,
}

/// One level of tracing.
#[derive(Clone, Copy, Debug)]
pub struct TraceParams {
    /// The class the level's streets get.
    pub class: RoadClass,
    /// A street stops this fraction of the separation short of a parallel one.
    pub test_ratio: f64,
    /// The integration step, metres.
    pub step: f64,
    /// The longest street, metres.
    pub max_length: f64,
    /// The shortest street kept, metres.
    pub min_length: f64,
    /// How far an end that stopped short may reach to join a street, as a fraction of the
    /// separation.
    pub join_ratio: f64,
    /// The longest bridge over a river, metres (0: none).
    pub max_bridge: f64,
    /// Seeds drawn at random once the queue is empty, before the level gives up after this many
    /// failures in a row.
    pub random_tries: u32,
}

/// Why a half-line stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// It left the frame or met water: nothing to join.
    Edge,
    /// It came near a parallel street, met a no-street zone, or ran out of length: join it.
    Short,
}

/// Traces one level of streets. `spacing(p)` is the separation at `p` (`None` where the level
/// draws no street); `existing` holds the levels traced before, which count as streets of
/// their family and as join targets.
pub fn trace_level(
    field: &TensorField,
    site: &Site,
    existing: &[Streamline],
    params: &TraceParams,
    spacing: &dyn Fn(DVec2) -> Option<f64>,
    seeds: &[DVec2],
    seed: u64,
    sites: &[BridgeSite],
) -> Vec<Streamline> {
    let size = field.size();
    let mut families = [PointGrid::new(size, 40.0), PointGrid::new(size, 40.0)];
    let mut all = PointGrid::new(size, 40.0);
    const OLD: u32 = u32::MAX;
    for line in existing {
        for &p in &line.points {
            families[usize::from(line.minor)].insert(p, OLD);
            all.insert(p, OLD);
        }
    }
    let mut queue: VecDeque<(DVec2, bool)> = seeds
        .iter()
        .flat_map(|&p| [(p, false), (p, true)])
        .collect();
    let mut rng = SplitMix64::new(seed);
    let mut lines: Vec<Streamline> = Vec::new();
    let mut failures = 0;
    loop {
        let (start, minor) = match queue.pop_front() {
            Some(s) => s,
            None => {
                if failures >= params.random_tries {
                    break;
                }
                failures += 1;
                (
                    DVec2::new(rng.range_f64(0.0, size), rng.range_f64(0.0, size)),
                    rng.next_bool(),
                )
            }
        };
        let Some(dsep) = spacing(start) else { continue };
        if site.water_at(start) != LAND
            || families[usize::from(minor)].any_within(start, dsep, |_| false)
        {
            continue;
        }
        let id = lines.len() as u32;
        let grid = &families[usize::from(minor)];
        let (forward, fwd_stop, fwd_bridges) = trace_half(
            field, site, grid, params, spacing, start, minor, 1.0, id, sites,
        );
        let (backward, back_stop, back_bridges) = trace_half(
            field, site, grid, params, spacing, start, minor, -1.0, id, sites,
        );
        let mut points: Vec<DVec2> = backward.iter().rev().copied().collect();
        let offset = points.len();
        points.extend_from_slice(&forward[1..]);
        let mut bridges: Vec<(usize, usize)> = back_bridges
            .iter()
            .map(|&(a, b)| (offset - 1 - b, offset - 1 - a))
            .collect();
        bridges.extend(
            fwd_bridges
                .iter()
                .map(|&(a, b)| (a + offset - 1, b + offset - 1)),
        );
        let mut line = Streamline {
            points,
            minor,
            class: params.class,
            bridges,
        };
        if super::geom::polyline_length(&line.points) < params.min_length {
            continue;
        }
        failures = 0;
        // Join the ends that stopped short to the nearest street ahead.
        let join = |end: DVec2, before: DVec2, line_points: &[DVec2]| -> Option<DVec2> {
            let dir = (end - before).normalize_or_zero();
            let reach = spacing(end).unwrap_or(0.0).max(params.step * 4.0) * params.join_ratio;
            let target = nearest_ahead(&all, line_points, end, dir, reach)?;
            let crossing = site.segment_crosses_water(end, target);
            (!crossing).then_some(target)
        };
        if back_stop == Stop::Short
            && line.points.len() >= 2
            && let Some(q) = join(line.points[0], line.points[1], &line.points)
        {
            line.points.insert(0, q);
            for b in &mut line.bridges {
                b.0 += 1;
                b.1 += 1;
            }
        }
        let n = line.points.len();
        if fwd_stop == Stop::Short
            && n >= 2
            && let Some(q) = join(line.points[n - 1], line.points[n - 2], &line.points)
        {
            line.points.push(q);
        }
        for &p in &line.points {
            families[usize::from(minor)].insert(p, id);
            all.insert(p, id);
        }
        // New seeds: the other family from points along this street, this family one
        // separation off to each side.
        let length = super::geom::polyline_length(&line.points);
        let mut s = 0.0;
        while s < length {
            let p = super::geom::point_at(&line.points, s);
            let d = spacing(p).unwrap_or(params.max_length);
            let t = super::geom::tangent_at(&line.points, s);
            let n = DVec2::new(-t.y, t.x);
            queue.push_back((p, !minor));
            queue.push_back((p + n * d, minor));
            queue.push_back((p - n * d, minor));
            s += d;
        }
        lines.push(line);
    }
    lines
}

/// One way of a street from `start`: its points, why it stopped and its bridges' point ranges.
#[allow(clippy::too_many_arguments)]
fn trace_half(
    field: &TensorField,
    site: &Site,
    family: &PointGrid,
    params: &TraceParams,
    spacing: &dyn Fn(DVec2) -> Option<f64>,
    start: DVec2,
    minor: bool,
    sign: f64,
    id: u32,
    sites: &[BridgeSite],
) -> (Vec<DVec2>, Stop, Vec<(usize, usize)>) {
    let mut points = vec![start];
    let mut bridges = Vec::new();
    let Some(d0) = field.direction(start, minor) else {
        return (points, Stop::Edge, bridges);
    };
    let mut dir = d0 * sign;
    let mut length = 0.0;
    let size = field.size();
    let h = params.step;
    let aligned = |p: DVec2, prev: DVec2| -> Option<DVec2> {
        let d = field.direction(p, minor)?;
        Some(if d.dot(prev) < 0.0 { -d } else { d })
    };
    loop {
        let p = *points.last().expect("never empty");
        let Some(d1) = aligned(p, dir) else {
            return (points, Stop::Short, bridges);
        };
        let Some(d2) = aligned(p + d1 * (h * 0.5), d1) else {
            return (points, Stop::Short, bridges);
        };
        let q = p + d2 * h;
        dir = d2;
        if q.x < 0.0 || q.y < 0.0 || q.x > size || q.y > size {
            return (points, Stop::Edge, bridges);
        }
        let water = site.water_at(q);
        if water != LAND {
            // Bridge a river if land comes back within reach, straight ahead.
            // With bridge sites, a road crosses only at one: along the bank to its near end,
            // over to its far end, and on from there the bridge's way.
            if water == RIVER && params.max_bridge > 0.0 && !sites.is_empty() {
                if let Some((near, far)) = bridge_ahead(sites, p, dir) {
                    if near.distance(p) > 0.5 {
                        points.push(near);
                    }
                    let first = points.len() - 1;
                    points.push(far);
                    bridges.push((first, points.len() - 1));
                    length += p.distance(near) + near.distance(far);
                    dir = (far - near).normalize_or_zero();
                    continue;
                }
                return (points, Stop::Edge, bridges);
            }
            if water == RIVER && params.max_bridge > 0.0 {
                let mut far = q;
                let mut crossed = h;
                while crossed <= params.max_bridge && site.water_at(far) == RIVER {
                    far += dir * h;
                    crossed += h;
                }
                if site.water_at(far) == LAND && crossed <= params.max_bridge {
                    let first = points.len() - 1;
                    points.push(far);
                    bridges.push((first, points.len() - 1));
                    length += crossed;
                    continue;
                }
            }
            return (points, Stop::Edge, bridges);
        }
        let Some(dsep) = spacing(q) else {
            return (points, Stop::Short, bridges);
        };
        if family.any_within(q, dsep * params.test_ratio, |tag| tag == id) {
            return (points, Stop::Short, bridges);
        }
        // A loop closes on its own start.
        if length > 4.0 * dsep && q.distance(start) < h * 1.5 {
            points.push(start);
            return (points, Stop::Edge, bridges);
        }
        points.push(q);
        length += h;
        if length > params.max_length {
            return (points, Stop::Short, bridges);
        }
    }
}

/// The nearest street point within `reach` of `end`, ahead of it (within 70° of `dir`), on any
/// street but the joining one's own last stretch.
fn nearest_ahead(
    all: &PointGrid,
    own: &[DVec2],
    end: DVec2,
    dir: DVec2,
    reach: f64,
) -> Option<DVec2> {
    let mut best: Option<(f64, DVec2)> = None;
    let cos_limit = 0.34;
    // Candidates: every point of the grid near the end; the own street's points are skipped by
    // position (they are behind or beside the end).
    let mut probe = |q: DVec2| {
        let v = q - end;
        let d = v.length();
        if d < 1e-6 || d > reach || v.dot(dir) < cos_limit * d {
            return;
        }
        if own.iter().any(|&o| o.distance_squared(q) < 1e-6) {
            return;
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, q));
        }
    };
    all.for_each_within(end, reach, |q, _| probe(q));
    best.map(|(_, q)| q)
}

/// Where a river is bridged: a crossing chosen before any road is traced (D-061), its two ends
/// on the banks. Main roads cross only here. Left to the streamlines, two roads a few dozen
/// metres apart each threw a bridge, sometimes across each other over the water (the owner:
/// "bridges are still a mess").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeSite {
    /// The end on one bank.
    pub a: DVec2,
    /// The end on the other.
    pub b: DVec2,
}

/// The bridge sites along the site's rivers: one every `spacing` metres along each river inside
/// the frame where `allowed` holds at both ends (the built districts), each square to the river
/// and at the narrowest place within 80 m of its mark, no longer than `max_span`.
pub fn bridge_sites(
    site: &Site,
    spacing: f64,
    max_span: f64,
    allowed: &dyn Fn(DVec2) -> bool,
) -> Vec<BridgeSite> {
    let mut out = Vec::new();
    let inside = |p: DVec2| p.x >= 0.0 && p.y >= 0.0 && p.x <= site.size && p.y <= site.size;
    for river in &site.rivers {
        let line = &river.centreline;
        let length = super::geom::polyline_length(line);
        // From the centreline at `s`, square to it, out to the land on either side.
        let span = |s: f64| -> Option<(DVec2, DVec2)> {
            let c = super::geom::point_at(line, s);
            let t = super::geom::tangent_at(line, s);
            let n = DVec2::new(-t.y, t.x);
            let bank = |side: f64| {
                let mut d = 0.0;
                while d <= max_span {
                    let p = c + n * (d * side);
                    match site.water_at(p) {
                        LAND => return Some(p + n * (2.0 * side)),
                        RIVER => d += 2.0,
                        _ => return None,
                    }
                }
                None
            };
            let (a, b) = (bank(1.0)?, bank(-1.0)?);
            (inside(a) && inside(b) && a.distance(b) <= max_span && allowed(a) && allowed(b))
                .then_some((a, b))
        };
        let mut s = spacing * 0.5;
        while s < length {
            let best = (-8..=8)
                .filter_map(|k| {
                    let at = s + f64::from(k) * 10.0;
                    span(at).map(|(a, b)| (a.distance(b) + 0.2 * (at - s).abs(), a, b))
                })
                .min_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((_, a, b)) = best {
                out.push(BridgeSite { a, b });
            }
            s += spacing;
        }
    }
    out
}

/// A road over each bridge site no traced road took: from each end straight on inland along the
/// bridge's axis to the first road of `lines` it meets within 300 m, on land, a T-junction there.
/// A site that meets none on either side keeps no bridge (the boats cross there). Joined to the
/// nearest road point instead, a bridge landed on a turn along the quay (the owner).
pub fn connect_sites(site: &Site, sites: &[BridgeSite], lines: &[Streamline]) -> Vec<Streamline> {
    let used = |s: &BridgeSite| {
        lines.iter().any(|l| {
            l.points.iter().any(|p| p.distance(s.a) < 1.0)
                && l.points.iter().any(|p| p.distance(s.b) < 1.0)
        })
    };
    // The first road the ray from `from` along `way` meets, before any water.
    let ahead = |from: DVec2, way: DVec2| -> Option<DVec2> {
        let to = from + way * 300.0;
        let mut best: Option<(f64, DVec2)> = None;
        for line in lines {
            for w in line.points.windows(2) {
                if let Some((t, _)) = super::geom::segment_intersection(from, to, w[0], w[1])
                    && t * 300.0 > 1.0
                    && best.is_none_or(|(b, _)| t < b)
                {
                    best = Some((t, from + (to - from) * t));
                }
            }
        }
        best.map(|(_, q)| q)
            .filter(|&q| !site.segment_crosses_water(from + way * 0.5, q))
    };
    let mut out = Vec::new();
    for s in sites.iter().filter(|s| !used(s)) {
        let way = (s.b - s.a).normalize_or_zero();
        if let (Some(qa), Some(qb)) = (ahead(s.a, -way), ahead(s.b, way)) {
            out.push(Streamline {
                points: vec![qa, s.a, s.b, qb],
                minor: false,
                class: RoadClass::Main,
                bridges: vec![(1, 2)],
            });
        }
    }
    out
}

/// The bridge site a street reaching the river at `p`, heading `dir`, takes: the nearest whose
/// near end lies within 60 m, 12 m off its axis at most and which runs within 25° of the
/// street's way, as its near and far ends.
fn bridge_ahead(sites: &[BridgeSite], p: DVec2, dir: DVec2) -> Option<(DVec2, DVec2)> {
    sites
        .iter()
        .filter_map(|s| {
            let (near, far) = if s.a.distance(p) <= s.b.distance(p) {
                (s.a, s.b)
            } else {
                (s.b, s.a)
            };
            let way = (far - near).normalize_or_zero();
            // Arriving near its axis and nearly along it: a bridge lands straight, not on a
            // turn along the quay (the owner saw a bridge land on an L).
            let lateral = super::geom::cross(way, p - near).abs();
            (near.distance(p) <= 60.0 && lateral <= 12.0 && way.dot(dir) > 0.9).then_some((
                near.distance(p),
                near,
                far,
            ))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, near, far)| (near, far))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::tensor::BoundaryInfluence;
    use crate::city::{DistrictSpec, Pattern};

    #[test]
    fn a_grid_traces_parallel_streets_at_its_spacing() {
        let site = Site::flat(1000.0, 10.0);
        let district = DistrictSpec::test(DVec2::splat(500.0), 2000.0, Pattern::Grid, 0.0);
        let field = TensorField::bake(
            &site,
            &[district],
            BoundaryInfluence {
                strength: 0.0,
                reach: 100.0,
            },
            1,
            20.0,
        );
        let params = TraceParams {
            class: RoadClass::Street,
            test_ratio: 0.5,
            step: 5.0,
            max_length: 2000.0,
            min_length: 20.0,
            join_ratio: 0.8,
            max_bridge: 0.0,
            random_tries: 20,
        };
        let lines = trace_level(
            &field,
            &site,
            &[],
            &params,
            &|_| Some(100.0),
            &[DVec2::splat(500.0)],
            7,
            &[],
        );
        let horizontal: Vec<&Streamline> = lines.iter().filter(|l| !l.minor).collect();
        let vertical: Vec<&Streamline> = lines.iter().filter(|l| l.minor).collect();
        assert!(
            horizontal.len() >= 8 && vertical.len() >= 8,
            "{} {}",
            horizontal.len(),
            vertical.len()
        );
        for line in &horizontal {
            let dy = line.points.last().unwrap().y - line.points[0].y;
            assert!(dy.abs() < 1.0, "an east–west street drifted by {dy} m");
        }
    }
}
