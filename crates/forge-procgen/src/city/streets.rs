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
        let (forward, fwd_stop, fwd_bridges) =
            trace_half(field, site, grid, params, spacing, start, minor, 1.0, id);
        let (backward, back_stop, back_bridges) =
            trace_half(field, site, grid, params, spacing, start, minor, -1.0, id);
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
