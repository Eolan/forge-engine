//! The rivers' surfaces (issue #105, D-038's rivers; `docs/research/water.md` §3 and its
//! recommendation's step 3): each river of stage 4 ([`crate::hydrology`]) as a ribbon of
//! points the GPU draws over the ground (`water.slang`).
//! - The D8 course runs from cell centre to cell centre in 45° turns. Chaikin's corner cutting
//!   (1974) smooths it, its ends kept, and it is resampled every [`RibbonParams::step`] metres.
//! - The width is [`hydrology::width`] of the catchment; the depth `0.3 (A / km²)^⅜` m (the
//!   downstream hydraulic geometry of Leopold & Maddock 1953, `w ∝ Q^0.5` and `d ∝ Q^0.4`, the
//!   exponent taken as ⅜); the speed Chézy's `C √(d S)` over the smoothed bed's slope `S`.
//! - In a bend the half width stays under a share of the bend's radius, so the ribbon's inner
//!   edge never folds over itself.
//! - The ribbon rests on the ground as the island's mesh draws it: each of its vertices, [`ACROSS`]
//!   quads across and a quad per step along, stands at the highest the drawn ground reaches
//!   under the quads around it, so no crease of the ground pokes through.
//!
//! Everything is `f64` arithmetic with `sqrt` only, in the rivers' order (D-016).

use crate::field::Field2;
use crate::hydrology::{self, Mouth, Rivers};

/// Quads across a ribbon (`RIVER_ACROSS` in `water.slang`).
pub const ACROSS: usize = 4;

/// Metres between the samples of the ground under a quad of a ribbon.
const GROUND_SAMPLES: f64 = 0.5;

/// How the ribbons are made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonParams {
    /// Metres between a ribbon's points.
    pub step: f64,
    /// Passes of a binomial filter (¼, ½, ¼) over the D8 course's points, its ends kept: the
    /// grid's stair steps straighten and its right-angled turns open into bends.
    pub relaxing: u32,
    /// Passes of Chaikin's corner cutting after it.
    pub smoothing: u32,
    /// Metres over which a river fades in from its head.
    pub head_fade: f64,
    /// Chézy's coefficient, m^½/s: the speed is `C √(d S)`.
    pub chezy: f64,
    /// The speed's range, m/s: from a pool's to a steep stream's.
    pub speed: (f64, f64),
    /// The half width's largest share of a bend's radius.
    pub bend: f64,
}

impl Default for RibbonParams {
    /// Points 4 m apart, four passes of the filter and three of corner cutting, 40 m of fade at
    /// the head, a stream's roughness (C = 15), 0.3 to 3 m/s, the half width under 0.8 of a
    /// bend's radius.
    fn default() -> Self {
        Self {
            step: 4.0,
            relaxing: 4,
            smoothing: 3,
            head_fade: 40.0,
            chezy: 15.0,
            speed: (0.3, 3.0),
            bend: 0.8,
        }
    }
}

/// A point of a ribbon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonPoint {
    /// Metres in the field's frame (`x` along columns, `y` along rows), on the smoothed course.
    pub position: [f32; 2],
    /// The smoothed bed's height, metres (the sea's level where the course runs below it).
    pub height: f32,
    /// Downstream, unit.
    pub direction: [f32; 2],
    /// Half the river's width, metres.
    pub half_width: f32,
    /// The water's depth in the middle, metres.
    pub depth: f32,
    /// The water's speed, m/s.
    pub speed: f32,
    /// The bed's slope downstream (0 where the course climbs: out of a lake).
    pub slope: f32,
    /// How much of the river is drawn here, 0..1: it fades in from its head.
    pub fade: f32,
    /// Per vertex across, from `position − side × half_width` to `position + side ×
    /// half_width` with `side = (−direction.y, direction.x)`: the highest the drawn ground
    /// stands under the quads around the vertex, metres.
    pub ground: [f32; ACROSS + 1],
}

/// One river's ribbon, head first.
#[derive(Clone, Debug, PartialEq)]
pub struct Ribbon {
    /// The river's index in [`Rivers::rivers`].
    pub river: u32,
    /// The catchment at its mouth, m².
    pub mouth_area: f64,
    /// The points, [`RibbonParams::step`] metres apart along the smoothed course (the last one
    /// may be closer).
    pub points: Vec<RibbonPoint>,
}

/// The depth of a river with `area_m2` of catchment, metres: 0.3 m at a square kilometre, 0.7 m
/// at ten.
pub fn depth(area_m2: f64) -> f64 {
    let x = area_m2 * 1e-6;
    0.3 * (x * x * x).sqrt().sqrt().sqrt()
}

/// The ribbons of `rivers` (traced over `height`), ordered by the catchment at their mouths, the
/// smallest first: a tributary comes before the river it joins, which is drawn over its end.
/// Each runs from its head to where it ends: the junction's point on the larger river, or the
/// outlet's sample.
pub fn ribbons(height: &Field2<f32>, rivers: &Rivers, params: &RibbonParams) -> Vec<Ribbon> {
    let spacing = height.spacing;
    let cell_area = spacing * spacing;
    let mut ribbons: Vec<Ribbon> = rivers
        .rivers
        .iter()
        .enumerate()
        .filter_map(|(index, river)| {
            // The course as (x, y, height, catchment in m²); below the sea it runs level.
            let mut course: Vec<[f64; 4]> = river
                .points
                .iter()
                .zip(&river.area)
                .map(|(p, &a)| {
                    [
                        f64::from(p[0]),
                        f64::from(p[1]),
                        f64::from(p[2]).max(0.0),
                        f64::from(a) * cell_area,
                    ]
                })
                .collect();
            let mouth_area = course.last()?[3];
            let end = match river.mouth {
                Mouth::Junction { river: into, point } => {
                    let p = rivers.rivers[into as usize].points[point as usize];
                    [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
                }
                Mouth::Outlet(cell) => {
                    let (x, y) = height.coords(cell as usize);
                    [
                        f64::from(x) * spacing,
                        f64::from(y) * spacing,
                        f64::from(height.data[cell as usize]),
                    ]
                }
            };
            course.push([end[0], end[1], end[2].max(0.0), mouth_area]);
            for _ in 0..params.relaxing {
                course = relax(&course);
            }
            for _ in 0..params.smoothing {
                course = chaikin(&course);
            }
            let samples = resample(&course, params.step);
            (samples.len() >= 2).then(|| {
                let mut points = ribbon_points(&samples, params);
                rest_on(&mut points, height);
                Ribbon {
                    river: index as u32,
                    mouth_area,
                    points,
                }
            })
        })
        .collect();
    ribbons.sort_by(|a, b| {
        a.mouth_area
            .total_cmp(&b.mouth_area)
            .then(a.river.cmp(&b.river))
    });
    ribbons
}

/// `a + (b − a) t`, every component.
fn mix(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// The distance from `a` to `b` in x and y.
fn distance(a: [f64; 4], b: [f64; 4]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dx * dx + dy * dy).sqrt()
}

/// One pass of the binomial filter over the course's inner points, the ends kept.
fn relax(course: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let mut out = course.to_vec();
    for k in 1..course.len().saturating_sub(1) {
        out[k] = std::array::from_fn(|i| {
            0.25 * course[k - 1][i] + 0.5 * course[k][i] + 0.25 * course[k + 1][i]
        });
    }
    out
}

/// One pass of Chaikin's corner cutting, the ends kept: each segment gives its points at a
/// quarter and three quarters of its length.
fn chaikin(course: &[[f64; 4]]) -> Vec<[f64; 4]> {
    if course.len() < 3 {
        return course.to_vec();
    }
    let mut out = Vec::with_capacity(2 * course.len());
    out.push(course[0]);
    for pair in course.windows(2) {
        out.push(mix(pair[0], pair[1], 0.25));
        out.push(mix(pair[0], pair[1], 0.75));
    }
    out.push(course[course.len() - 1]);
    out
}

/// Points every `step` metres along `course`, from its first to its last point, which is kept
/// (in place of a sample within a tenth of a step of it).
fn resample(course: &[[f64; 4]], step: f64) -> Vec<[f64; 4]> {
    let mut out = vec![course[0]];
    let mut next = step;
    let mut walked = 0.0;
    for pair in course.windows(2) {
        let length = distance(pair[0], pair[1]);
        while next <= walked + length {
            out.push(mix(pair[0], pair[1], (next - walked) / length));
            next += step;
        }
        walked += length;
    }
    let last = course[course.len() - 1];
    if walked - (next - step) > 0.1 * step {
        out.push(last);
    } else if out.len() > 1 {
        let end = out.len() - 1;
        out[end] = last;
    }
    out
}

/// The curvature of the circle through three points, 1/m (Menger's: four times the triangle's
/// area over the product of its sides); 0 when they are in line.
fn curvature(a: [f64; 4], b: [f64; 4], c: [f64; 4]) -> f64 {
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let sides = distance(a, b) * distance(b, c) * distance(a, c);
    if sides > 0.0 {
        2.0 * cross.abs() / sides
    } else {
        0.0
    }
}

/// The ribbon's points from the resampled course.
fn ribbon_points(samples: &[[f64; 4]], params: &RibbonParams) -> Vec<RibbonPoint> {
    let n = samples.len();
    let mut arc = vec![0.0; n];
    for k in 1..n {
        arc[k] = arc[k - 1] + distance(samples[k - 1], samples[k]);
    }
    let mut half: Vec<f64> = samples
        .iter()
        .map(|s| 0.5 * hydrology::width(s[3]))
        .collect();
    // In a bend, under `bend` of its radius; and into and out of it gradually, a quarter of a
    // metre a metre at most.
    for k in 1..n - 1 {
        let bend = curvature(samples[k - 1], samples[k], samples[k + 1]);
        if bend > 0.0 {
            half[k] = half[k].min(params.bend / bend);
        }
    }
    for k in 1..n {
        half[k] = half[k].min(half[k - 1] + 0.25 * (arc[k] - arc[k - 1]));
    }
    for k in (0..n - 1).rev() {
        half[k] = half[k].min(half[k + 1] + 0.25 * (arc[k + 1] - arc[k]));
    }
    (0..n)
        .map(|k| {
            let s = samples[k];
            let (before, after) = (samples[k.saturating_sub(1)], samples[(k + 1).min(n - 1)]);
            let (dx, dy) = (after[0] - before[0], after[1] - before[1]);
            let length = (dx * dx + dy * dy).sqrt().max(1e-9);
            // The bed's slope over three points either way.
            let (up, down) = (k.saturating_sub(3), (k + 3).min(n - 1));
            let run = arc[down] - arc[up];
            let slope = if run > 0.0 {
                ((samples[up][2] - samples[down][2]) / run).max(0.0)
            } else {
                0.0
            };
            let depth = depth(s[3]);
            let speed =
                (params.chezy * (depth * slope).sqrt()).clamp(params.speed.0, params.speed.1);
            let t = (arc[k] / params.head_fade).clamp(0.0, 1.0);
            RibbonPoint {
                position: [s[0] as f32, s[1] as f32],
                height: s[2] as f32,
                direction: [(dx / length) as f32, (dy / length) as f32],
                half_width: half[k] as f32,
                depth: depth as f32,
                speed: speed as f32,
                slope: slope as f32,
                fade: (t * t * (3.0 - 2.0 * t)) as f32,
                ground: [0.0; ACROSS + 1],
            }
        })
        .collect()
}

/// The height of `height` at (x, y) metres in its frame, as the island's mesh draws it: each
/// cell split along its (i + 1, j) – (i, j + 1) diagonal, as `forge_geom::city::heightfield_mesh`
/// splits it (and `ground_height` in `water.slang` reads it).
pub fn drawn_height(height: &Field2<f32>, x: f64, y: f64) -> f64 {
    let last = f64::from(height.size - 2);
    let (gx, gy) = (x / height.spacing, y / height.spacing);
    let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
    let (tx, ty) = ((gx - cx).clamp(0.0, 1.0), (gy - cy).clamp(0.0, 1.0));
    let (i, j) = (cx as u32, cy as u32);
    let a = f64::from(height.get(i, j));
    let b = f64::from(height.get(i + 1, j));
    let c = f64::from(height.get(i, j + 1));
    let d = f64::from(height.get(i + 1, j + 1));
    if tx + ty <= 1.0 {
        a + tx * (b - a) + ty * (c - a)
    } else {
        d + (1.0 - tx) * (c - d) + (1.0 - ty) * (b - d)
    }
}

/// Sets each point's `ground`: the highest the drawn ground stands under the ribbon's quads
/// around each vertex, from samples at most [`GROUND_SAMPLES`] metres apart over each quad.
fn rest_on(points: &mut [RibbonPoint], height: &Field2<f32>) {
    let vertex = |p: &RibbonPoint, j: usize| -> [f64; 2] {
        let across = (j as f64 / ACROSS as f64) * 2.0 - 1.0;
        let reach = across * f64::from(p.half_width);
        [
            f64::from(p.position[0]) - f64::from(p.direction[1]) * reach,
            f64::from(p.position[1]) + f64::from(p.direction[0]) * reach,
        ]
    };
    let n = points.len();
    let mut ground = vec![[f64::MIN; ACROSS + 1]; n];
    for k in 0..n - 1 {
        for j in 0..ACROSS {
            let corners = [
                vertex(&points[k], j),
                vertex(&points[k], j + 1),
                vertex(&points[k + 1], j),
                vertex(&points[k + 1], j + 1),
            ];
            let side =
                |a: [f64; 2], b: [f64; 2]| distance([a[0], a[1], 0.0, 0.0], [b[0], b[1], 0.0, 0.0]);
            let along = side(corners[0], corners[2]).max(side(corners[1], corners[3]));
            let across = side(corners[0], corners[1]).max(side(corners[2], corners[3]));
            let (su, sv) = (
                (across / GROUND_SAMPLES).ceil().max(1.0) as u32,
                (along / GROUND_SAMPLES).ceil().max(1.0) as u32,
            );
            let mut highest = f64::MIN;
            for v in 0..=sv {
                let fv = f64::from(v) / f64::from(sv);
                for u in 0..=su {
                    let fu = f64::from(u) / f64::from(su);
                    let at = |c: usize| {
                        let near = [
                            corners[0][c] + (corners[1][c] - corners[0][c]) * fu,
                            corners[2][c] + (corners[3][c] - corners[2][c]) * fu,
                        ];
                        near[0] + (near[1] - near[0]) * fv
                    };
                    highest = highest.max(drawn_height(height, at(0), at(1)));
                }
            }
            for (kk, jj) in [(k, j), (k, j + 1), (k + 1, j), (k + 1, j + 1)] {
                ground[kk][jj] = ground[kk][jj].max(highest);
            }
        }
    }
    for (p, g) in points.iter_mut().zip(&ground) {
        p.ground = g.map(|h| h as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::drain;
    use crate::hydrology::trace_rivers;
    use forge_task::{PoolConfig, TaskPool};

    fn length(points: &[RibbonPoint]) -> f32 {
        points
            .windows(2)
            .map(|w| {
                let (dx, dy) = (
                    w[1].position[0] - w[0].position[0],
                    w[1].position[1] - w[0].position[1],
                );
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    #[test]
    fn a_valley_gives_a_ribbon_down_it_that_widens_deepens_and_fades_in() {
        // A V-shaped valley along x = 100 m falling towards y = 0, the border and the outlet.
        let valley = Field2::from_fn(21, 10.0, |x, y| {
            2.0 * (x as f32 - 10.0).abs() + y as f32 + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 15);
        let params = RibbonParams::default();
        let ribbons = ribbons(&valley, &rivers, &params);
        assert_eq!(ribbons.len(), 1);
        let ribbon = &ribbons[0];
        let points = &ribbon.points;
        // Down the valley's floor to the outlet on the border, points a step apart.
        assert!(points.iter().all(|p| (p.position[0] - 100.0).abs() < 1e-3));
        assert!((points[points.len() - 1].position[1]).abs() < 1e-3);
        for w in points.windows(2) {
            // A step apart, the last within a tenth of a step more.
            let gap = w[0].position[1] - w[1].position[1];
            assert!(gap > 0.0 && gap <= 4.4 + 1e-3, "{gap}");
            // Downstream is −y, and the river grows.
            assert!(w[0].direction[1] < -0.999);
            assert!(w[1].half_width >= w[0].half_width && w[1].depth >= w[0].depth);
        }
        assert!((length(points) - rivers.rivers[0].length() - 10.0).abs() < 1e-2);
        // Its size from its catchment, its speed from the valley's 10 % fall.
        let last = points[points.len() - 1];
        let area = ribbon.mouth_area;
        assert!((f64::from(last.half_width) - 0.5 * hydrology::width(area)).abs() < 1e-4);
        assert!((f64::from(last.depth) - depth(area)).abs() < 1e-6);
        let mid = points[points.len() / 2];
        assert!((mid.slope - 0.1).abs() < 1e-4, "{}", mid.slope);
        let chezy = 15.0 * (mid.depth * mid.slope).sqrt();
        assert!((mid.speed - chezy.clamp(0.3, 3.0)).abs() < 1e-5);
        // It fades in over its first 40 m.
        assert_eq!(points[0].fade, 0.0);
        assert!(points[5].fade > 0.0 && points[5].fade < 1.0);
        assert_eq!(points[points.len() - 1].fade, 1.0);
        assert!((depth(1.0e6) - 0.3).abs() < 1e-12);
        assert!((depth(1.0e7) / depth(1.0e6) - 10f64.powf(0.375)).abs() < 1e-9);
        // The drawn ground meets the field at its samples.
        assert_eq!(
            drawn_height(&valley, 30.0, 70.0),
            f64::from(valley.get(3, 7))
        );
        // The ribbon rests on the drawn ground: over each quad, its lowest corner is at least
        // the highest the ground stands under it (sampled finer than the ribbon's own samples,
        // within what the valley's slopes rise between those).
        for w in points.windows(2) {
            for j in 0..ACROSS {
                let lowest = [
                    w[0].ground[j],
                    w[0].ground[j + 1],
                    w[1].ground[j],
                    w[1].ground[j + 1],
                ]
                .into_iter()
                .fold(f32::MAX, f32::min);
                for (u, v) in (0..=8).flat_map(|u| (0..=8).map(move |v| (u, v))) {
                    let across = (j as f32 + u as f32 / 8.0) / ACROSS as f32 * 2.0 - 1.0;
                    let t = v as f32 / 8.0;
                    let centre = [
                        w[0].position[0] + (w[1].position[0] - w[0].position[0]) * t,
                        w[0].position[1] + (w[1].position[1] - w[0].position[1]) * t,
                    ];
                    let reach =
                        across * (w[0].half_width + (w[1].half_width - w[0].half_width) * t);
                    let ground = drawn_height(
                        &valley,
                        f64::from(centre[0] - w[0].direction[1] * reach),
                        f64::from(centre[1] + w[0].direction[0] * reach),
                    );
                    assert!(f64::from(lowest) >= ground - 0.1, "{lowest} under {ground}");
                }
            }
        }
    }

    #[test]
    fn a_tributary_ends_on_its_river_and_comes_first_and_bends_stay_open() {
        // Two valleys meeting in a Y (as `hydrology`'s test).
        let fork = Field2::from_fn(41, 10.0, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let spread = (fy - 20.0).max(0.0) * 0.5;
            let branch = (fx - (20.0 - spread))
                .abs()
                .min((fx - (20.0 + spread)).abs());
            2.0 * branch + fy + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&fork, 0.0, &pool);
        let rivers = trace_rivers(&fork, &flow, 30);
        let ribbons = ribbons(&fork, &rivers, &RibbonParams::default());
        assert_eq!(ribbons.len(), rivers.rivers.len());
        // The smallest mouth first; the trunk, to the outlet, last.
        for pair in ribbons.windows(2) {
            assert!(pair[0].mouth_area <= pair[1].mouth_area);
        }
        let trunk = &ribbons[ribbons.len() - 1];
        assert!(matches!(
            rivers.rivers[trunk.river as usize].mouth,
            Mouth::Outlet(_)
        ));
        for ribbon in &ribbons[..ribbons.len() - 1] {
            // A tributary ends on the junction's point of its river.
            let Mouth::Junction { river, point } = rivers.rivers[ribbon.river as usize].mouth
            else {
                panic!("a tributary ends at a junction");
            };
            let at = rivers.rivers[river as usize].points[point as usize];
            let end = ribbon.points[ribbon.points.len() - 1].position;
            assert!((end[0] - at[0]).abs() < 1e-3 && (end[1] - at[1]).abs() < 1e-3);
            // The D8 course turns by 45° where the branch meets the trunk: smoothed, the ribbon's
            // half width stays under 0.8 of the radius there, changing gradually.
            for w in ribbon.points.windows(3) {
                let a = [w[0].position[0], w[0].position[1]].map(f64::from);
                let b = [w[1].position[0], w[1].position[1]].map(f64::from);
                let c = [w[2].position[0], w[2].position[1]].map(f64::from);
                let bend = curvature(
                    [a[0], a[1], 0.0, 0.0],
                    [b[0], b[1], 0.0, 0.0],
                    [c[0], c[1], 0.0, 0.0],
                );
                assert!(f64::from(w[1].half_width) * bend <= 0.8 + 1e-3);
                assert!((w[1].half_width - w[0].half_width).abs() <= 0.25 * 4.4 + 1e-3);
            }
        }
        // Deterministic.
        assert_eq!(
            super::ribbons(&fork, &rivers, &RibbonParams::default()),
            ribbons
        );
    }
}
