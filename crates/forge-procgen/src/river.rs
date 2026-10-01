//! The rivers' water (issue #105, D-038's rivers; `docs/research/water.md` §3 and its
//! recommendation's step 3): each river of stage 4 ([`crate::hydrology`]) as a ribbon of
//! points the GPU draws as a level surface in the channel [`crate::channel`] carves for it.
//! - The D8 course runs from cell centre to cell centre in 45° turns. A binomial filter and
//!   Chaikin's corner cutting (1974) smooth it, its ends kept, and it is resampled every
//!   [`RibbonParams::step`] metres.
//! - The width is [`hydrology::width`] of the catchment; the depth `0.4 (A / km²)^⅜` m (the
//!   downstream hydraulic geometry of Leopold & Maddock 1953, `w ∝ Q^0.5` and `d ∝ Q^0.4`, the
//!   exponent taken as ⅜); the speed Chézy's `C √(d S)` over the water surface's slope `S`.
//! - The water is level across. Its level at a point is the lowest the ground stands there, in
//!   the middle and on either bank ([`smooth_height`]: the field's samples through a cubic),
//!   less a freeboard; then the running minimum from the head, so it only falls, and a fall
//!   steeper than [`RibbonParams::max_fall`] is spread upstream. Through a lake it is the
//!   lake's level, and it never goes below the sea's.
//! - A tributary's water ends at its river's level, and it gives way to that river's water
//!   where it enters its channel; a river gives way to a lake inside it and to the sea where
//!   its level reaches the sea's.
//! - In a bend the ribbon's half width stays under a share of the bend's radius, so its inner
//!   edge never folds over itself.
//!
//! Everything is `f64` arithmetic with `sqrt` only, in the rivers' order (D-016).

use forge_task::TaskPool;

use crate::field::Field2;
use crate::hydrology::{self, Lakes, Mouth, Rivers};

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
    /// Passes pulling the smoothed course back to its valley's floor.
    pub settling: u32,
    /// How far across the course a settling pass looks for the valley's floor, metres.
    pub settle_reach: f64,
    /// Metres over which a river grows from its spring (from `spring` of its width and depth),
    /// its water fading in over the first fifth of them.
    pub head_fade: f64,
    /// The shares of its width and its depth a river has at its spring.
    pub spring: (f64, f64),
    /// Chézy's coefficient, m^½/s: the speed is `C √(d S)`.
    pub chezy: f64,
    /// The speed's range, m/s: from a pool's to a steep stream's.
    pub speed: (f64, f64),
    /// The drawn half width's largest share of a bend's radius.
    pub bend: f64,
    /// How far the ribbon reaches past the water's edge, under the banks: `a + b × half
    /// width`, metres.
    pub tuck: (f64, f64),
    /// How far the water stands under the lowest of its banks: `a + b × width`, metres.
    pub freeboard: (f64, f64),
    /// The water surface's steepest fall, m/m: a steeper step lowers the water upstream.
    pub max_fall: f64,
    /// The smallest lake, m², whose level a river takes through it (and where it is not drawn).
    pub lake_area: f64,
    /// The estuary (D-041): the metres over the sea under which a river widens towards its
    /// mouth, and by how many of its widths at the sea's level (1: twice as wide), shallowing by
    /// two fifths, so it crosses the beach as a river mouth rather than a canal.
    pub estuary: (f64, f64),
}

impl Default for RibbonParams {
    /// Points 4 m apart, four passes of the filter and three of corner cutting, four settling
    /// passes looking 8 m either side, 40 m of growth from the spring, a stream's roughness
    /// (C = 15), 0.3 to 3 m/s, the drawn half width under 0.8 of a bend's radius, 0.5 m + 10 %
    /// of the half width under the banks, the water 0.05 m + 4 % of the width under them, a fall
    /// of 60 % at most, the lakes of a hectare, twice as wide at the sea from 1.5 m over it.
    fn default() -> Self {
        Self {
            step: 4.0,
            relaxing: 4,
            smoothing: 3,
            settling: 4,
            settle_reach: 8.0,
            head_fade: 40.0,
            spring: (1.0 / 6.0, 1.0 / 3.0),
            chezy: 15.0,
            speed: (0.3, 3.0),
            bend: 0.8,
            tuck: (0.5, 0.1),
            freeboard: (0.05, 0.04),
            max_fall: 0.6,
            lake_area: 10_000.0,
            estuary: (1.5, 1.0),
        }
    }
}

/// A point of a ribbon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonPoint {
    /// Metres in the field's frame (`x` along columns, `y` along rows), on the smoothed course.
    pub position: [f32; 2],
    /// The water's level, metres: level across the river, never rising downstream.
    pub level: f32,
    /// Downstream, unit.
    pub direction: [f32; 2],
    /// Half the river's width at its level, metres: where the water meets the banks.
    pub half_width: f32,
    /// Half the ribbon's width, metres: past the water's edge, under the banks.
    pub reach: f32,
    /// The water's depth in the middle, metres.
    pub depth: f32,
    /// The lowest the ground stands on the banks here before the channel is carved, metres.
    pub bank: f32,
    /// The water's speed, m/s.
    pub speed: f32,
    /// The water surface's slope downstream.
    pub slope: f32,
    /// How much of the river is drawn here, 0..1: it fades in from its head, and out into a
    /// lake, into the river it joins, and into the sea.
    pub fade: f32,
    /// Per vertex across, from `position − side × reach` to `position + side × reach` with
    /// `side = (−direction.y, direction.x)`: the highest the ground stands under the quads
    /// around the vertex, metres ([`rest_on`]), where the water lies when it is far away.
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
    /// The points where it enters a lake of [`RibbonParams::lake_area`] or more: the first of
    /// each run of points under a lake, where the lake's water takes the river on.
    pub lake_entries: Vec<u32>,
}

/// The depth of a river with `area_m2` of catchment, metres: 0.4 m at a square kilometre, 0.95 m
/// at ten.
pub fn depth(area_m2: f64) -> f64 {
    let x = area_m2 * 1e-6;
    0.4 * (x * x * x).sqrt().sqrt().sqrt()
}

/// The ribbons of `rivers` (traced over `height`, whose `lakes` they cross), ordered by the
/// catchment at their mouths, the smallest first: a tributary comes before the river it joins.
/// Each runs from its head to where it ends: the junction's point on the larger river, or the
/// outlet's sample. Their `ground` is left at the levels; [`rest_on`] sets it.
pub fn ribbons(
    height: &Field2<f32>,
    rivers: &Rivers,
    lakes: &Lakes,
    params: &RibbonParams,
) -> Vec<Ribbon> {
    let spacing = height.spacing;
    let cell_area = spacing * spacing;
    let mut ribbons: Vec<Ribbon> = rivers
        .rivers
        .iter()
        .enumerate()
        .filter_map(|(index, river)| {
            // The course as (x, y, height, catchment in m²).
            let mut course: Vec<[f64; 4]> = river
                .points
                .iter()
                .zip(&river.area)
                .map(|(p, &a)| {
                    [
                        f64::from(p[0]),
                        f64::from(p[1]),
                        f64::from(p[2]),
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
            course.push([end[0], end[1], end[2], mouth_area]);
            for _ in 0..params.relaxing {
                course = relax(&course);
            }
            for _ in 0..params.smoothing {
                course = chaikin(&course);
            }
            let mut samples = resample(&course, params.step);
            for _ in 0..params.settling {
                samples = settle(&samples, height, params.settle_reach);
            }
            if params.settling > 0 {
                samples = resample(&samples, params.step);
            }
            (samples.len() >= 2).then(|| Ribbon {
                river: index as u32,
                mouth_area,
                points: ribbon_points(&samples, params),
                lake_entries: Vec::new(),
            })
        })
        .collect();
    // The levels, the largest river first: a tributary's water ends at its river's.
    ribbons.sort_by(|a, b| {
        b.mouth_area
            .total_cmp(&a.mouth_area)
            .then(a.river.cmp(&b.river))
    });
    let lake_level = |x: f64, y: f64| -> Option<f64> {
        let last = f64::from(height.size - 1);
        let (i, j) = (
            (x / spacing).round().clamp(0.0, last) as u32,
            (y / spacing).round().clamp(0.0, last) as u32,
        );
        let id = *lakes.lake_of.get(height.index(i, j))?;
        let lake = lakes.lakes.get(id as usize)?;
        (lake.area(spacing) >= params.lake_area).then_some(f64::from(lake.level))
    };
    let mut done: Vec<Option<usize>> = vec![None; rivers.rivers.len()];
    for r in 0..ribbons.len() {
        let joins = match rivers.rivers[ribbons[r].river as usize].mouth {
            Mouth::Junction { river: into, point } => done[into as usize].map(|m| {
                let p = rivers.rivers[into as usize].points[point as usize];
                (m, [f64::from(p[0]), f64::from(p[1])])
            }),
            Mouth::Outlet(_) => None,
        };
        let main = joins.map(|(m, at)| (&ribbons[m].points, at));
        let points = levels(&ribbons[r].points, height, &lake_level, main, params);
        let under = |p: &RibbonPoint| {
            lake_level(f64::from(p.position[0]), f64::from(p.position[1])).is_some()
        };
        ribbons[r].lake_entries = (0..points.len())
            .filter(|&k| under(&points[k]) && (k == 0 || !under(&points[k - 1])))
            .map(|k| k as u32)
            .collect();
        ribbons[r].points = points;
        done[ribbons[r].river as usize] = Some(r);
    }
    // The estuaries: under `estuary.0` metres over the sea the rivers widen and shallow
    // towards their mouths (after the levels, which the narrower course set).
    let (over, widen) = params.estuary;
    if widen > 0.0 {
        for p in ribbons.iter_mut().flat_map(|r| r.points.iter_mut()) {
            let e = 1.0 - smoothstep(0.0, over, f64::from(p.level));
            if e > 0.0 {
                let half = f64::from(p.half_width) * (1.0 + widen * e);
                p.half_width = half as f32;
                p.reach = (half + affine(params.tuck, half)) as f32;
                p.depth = (f64::from(p.depth) * (1.0 - 0.4 * e)) as f32;
            }
        }
    }
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

/// One pass pulling a smoothed course back to its valley's floor, which the corner cutting
/// leaves in the tight bends of a narrow valley: each inner point moves half way to the lowest
/// ground across the course within `reach` metres (on [`smooth_height`], a move costing
/// `0.02 m` per square metre of it, so a point leaves a flat floor only for a clearly lower
/// one), then a pass of the binomial filter keeps the course smooth. The ends stay.
fn settle(course: &[[f64; 4]], height: &Field2<f32>, reach: f64) -> Vec<[f64; 4]> {
    let n = course.len();
    let mut moved = course.to_vec();
    let steps = reach.ceil() as i32;
    for k in 1..n.saturating_sub(1) {
        let (dx, dy) = (
            course[k + 1][0] - course[k - 1][0],
            course[k + 1][1] - course[k - 1][1],
        );
        let length = (dx * dx + dy * dy).sqrt();
        if length == 0.0 {
            continue;
        }
        let (nx, ny) = (-dy / length, dx / length);
        let mut best = (f64::MAX, 0.0);
        for o in -steps..=steps {
            let o = f64::from(o) * reach / f64::from(steps);
            let h =
                smooth_height(height, course[k][0] + nx * o, course[k][1] + ny * o) + 0.02 * o * o;
            if h < best.0 {
                best = (h, o);
            }
        }
        moved[k][0] += nx * 0.5 * best.1;
        moved[k][1] += ny * 0.5 * best.1;
    }
    relax(&moved)
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

/// `(a + b × half width)` of a pair of parameters.
fn affine(p: (f64, f64), x: f64) -> f64 {
    p.0 + p.1 * x
}

/// The share of its full size a river has `arc` metres from its spring, growing from `at_spring`
/// over [`RibbonParams::head_fade`], smoothly.
fn spring(at_spring: f64, arc: f64, params: &RibbonParams) -> f64 {
    at_spring + (1.0 - at_spring) * smoothstep(0.0, params.head_fade, arc)
}

/// The ribbon's points from the resampled course: position, direction, widths, depth and the
/// head's fade (the levels come after, [`levels`]). From its spring a river grows over
/// `head_fade` metres from `spring` of its width and depth, and its water fades
/// in over the first fifth of that.
fn ribbon_points(samples: &[[f64; 4]], params: &RibbonParams) -> Vec<RibbonPoint> {
    let n = samples.len();
    let mut arc = vec![0.0; n];
    for k in 1..n {
        arc[k] = arc[k - 1] + distance(samples[k - 1], samples[k]);
    }
    let mut half: Vec<f64> = samples
        .iter()
        .zip(&arc)
        .map(|(s, &a)| 0.5 * hydrology::width(s[3]) * spring(params.spring.0, a, params))
        .collect();
    // In a bend, the ribbon (the water and its tuck under the banks) under `bend` of its
    // radius; and into and out of it gradually, a quarter of a metre a metre at most.
    for k in 1..n - 1 {
        let bend = curvature(samples[k - 1], samples[k], samples[k + 1]);
        if bend > 0.0 {
            let reach = params.bend / bend;
            let fits = (reach - params.tuck.0) / (1.0 + params.tuck.1);
            half[k] = half[k].min(fits.max(0.25));
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
            RibbonPoint {
                position: [s[0] as f32, s[1] as f32],
                level: 0.0,
                direction: [(dx / length) as f32, (dy / length) as f32],
                half_width: half[k] as f32,
                reach: (half[k] + affine(params.tuck, half[k])) as f32,
                depth: (depth(s[3]) * spring(params.spring.1, arc[k], params)) as f32,
                bank: 0.0,
                speed: 0.0,
                slope: 0.0,
                fade: smoothstep(0.0, 0.2 * params.head_fade, arc[k]) as f32,
                ground: [0.0; ACROSS + 1],
            }
        })
        .collect()
}

/// `smoothstep(e0, e1, x)`.
pub(crate) fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A point `across` metres to the left of `p` (seen downstream) and `along` metres down.
pub(crate) fn offset(p: &RibbonPoint, along: f64, across: f64) -> [f64; 2] {
    let (dx, dy) = (f64::from(p.direction[0]), f64::from(p.direction[1]));
    [
        f64::from(p.position[0]) + dx * along - dy * across,
        f64::from(p.position[1]) + dy * along + dx * across,
    ]
}

/// The distance from `q` to the segment `a`–`b`.
pub(crate) fn segment_distance(q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let length = ex * ex + ey * ey;
    let t = if length > 0.0 {
        (((q[0] - a[0]) * ex + (q[1] - a[1]) * ey) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (q[0] - a[0] - ex * t, q[1] - a[1] - ey * t);
    ((dx * dx + dy * dy).sqrt(), t)
}

/// The points with their levels, banks, speeds, slopes and fades; `main` is the river this one
/// joins (its points, with their levels, and the junction).
fn levels(
    points: &[RibbonPoint],
    height: &Field2<f32>,
    lake_level: &dyn Fn(f64, f64) -> Option<f64>,
    main: Option<(&Vec<RibbonPoint>, [f64; 2])>,
    params: &RibbonParams,
) -> Vec<RibbonPoint> {
    let n = points.len();
    // A tributary ends on its river's smoothed course (the junction's D8 point may be off it).
    let mut points = points.to_vec();
    if let Some((main, at)) = main {
        let m = nearest(main, at);
        let (lo, hi) = (m.saturating_sub(2), (m + 2).min(main.len() - 1));
        let mut best = (f64::MAX, at);
        for j in lo..hi {
            let (a, b) = (main[j].position, main[j + 1].position);
            let (a, b) = (
                [f64::from(a[0]), f64::from(a[1])],
                [f64::from(b[0]), f64::from(b[1])],
            );
            let (d, t) = segment_distance(at, a, b);
            if d < best.0 {
                best = (d, [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        points[n - 1].position = [best.1[0] as f32, best.1[1] as f32];
    }
    let points = &points[..];
    let mut arc = vec![0.0; n];
    for k in 1..n {
        let (a, b) = (points[k - 1].position, points[k].position);
        let (dx, dy) = (f64::from(b[0] - a[0]), f64::from(b[1] - a[1]));
        arc[k] = arc[k - 1] + (dx * dx + dy * dy).sqrt();
    }
    // Per point: what the water may reach (the lowest ground in the middle and on the banks,
    // a little up and down the course, less the freeboard), the lowest bank, and a lake.
    let mut target = vec![0.0; n];
    let mut bank = vec![0.0; n];
    let mut in_lake = vec![false; n];
    for (k, p) in points.iter().enumerate() {
        let (half, reach) = (f64::from(p.half_width), f64::from(p.reach));
        let lake = lake_level(f64::from(p.position[0]), f64::from(p.position[1]));
        let mut lowest_bank = f64::MAX;
        let mut lowest = f64::MAX;
        for along in [-2.0, 0.0, 2.0] {
            for across in [-reach, -half, 0.0, half, reach] {
                let q = offset(p, along, across);
                let h = smooth_height(height, q[0], q[1]);
                lowest = lowest.min(h);
                if across != 0.0 {
                    lowest_bank = lowest_bank.min(h);
                }
            }
        }
        let freeboard = affine(params.freeboard, 2.0 * half);
        (target[k], bank[k], in_lake[k]) = match lake {
            Some(level) => (level, level, true),
            None => (lowest - freeboard, lowest_bank, false),
        };
    }
    // The level: under its target, falling only, the steep steps spread upstream; twice, with
    // a smoothing between.
    let mut level = target.clone();
    for pass in 0..3 {
        if pass > 0 {
            let before = level.clone();
            for k in 1..n - 1 {
                level[k] = 0.25 * before[k - 1] + 0.5 * before[k] + 0.25 * before[k + 1];
            }
            for k in 0..n {
                level[k] = level[k].min(target[k]);
            }
        }
        for k in 1..n {
            level[k] = level[k].min(level[k - 1]);
        }
        for k in (0..n - 1).rev() {
            level[k] = level[k].min(level[k + 1] + params.max_fall * (arc[k + 1] - arc[k]));
        }
    }
    // Never under the sea, nor under the river it joins, where it meets it.
    let joined = main.map(|(main, at)| {
        let m = nearest(main, at);
        (main, m, f64::from(main[m].level))
    });
    for l in &mut level {
        *l = l.max(0.0);
        if let Some((_, _, floor)) = joined {
            *l = l.max(floor);
        }
    }
    // How far each point is from the water of the river it joins, less that water's reach.
    let into_main: Vec<f64> = match joined {
        Some((main, m, _)) => points
            .iter()
            .map(|p| {
                let q = [f64::from(p.position[0]), f64::from(p.position[1])];
                let (lo, hi) = (m.saturating_sub(24), (m + 24).min(main.len() - 1));
                (lo..hi)
                    .map(|j| {
                        let (a, b) = (main[j].position, main[j + 1].position);
                        let (d, t) = segment_distance(
                            q,
                            [f64::from(a[0]), f64::from(a[1])],
                            [f64::from(b[0]), f64::from(b[1])],
                        );
                        let reach = f64::from(main[j].reach)
                            + (f64::from(main[j + 1].reach) - f64::from(main[j].reach)) * t;
                        d - reach
                    })
                    .fold(f64::MAX, f64::min)
            })
            .collect(),
        None => vec![f64::MAX; n],
    };
    // The lakes' points, and how many points away the nearest one is.
    let mut from_lake = vec![u32::MAX; n];
    for k in 0..n {
        if in_lake[k] {
            from_lake[k] = 0;
        } else if k > 0 {
            from_lake[k] = from_lake[k - 1].saturating_add(1);
        }
    }
    for k in (0..n.saturating_sub(1)).rev() {
        from_lake[k] = from_lake[k].min(from_lake[k + 1].saturating_add(1));
    }
    (0..n)
        .map(|k| {
            let p = points[k];
            // The water surface's slope over three points either way.
            let (up, down) = (k.saturating_sub(3), (k + 3).min(n - 1));
            let run = arc[down] - arc[up];
            let slope = if run > 0.0 {
                ((level[up] - level[down]) / run).max(0.0)
            } else {
                0.0
            };
            let depth = f64::from(p.depth);
            let speed =
                (params.chezy * (depth * slope).sqrt()).clamp(params.speed.0, params.speed.1);
            let fade = f64::from(p.fade)
                * smoothstep(0.0, 3.0, f64::from(from_lake[k].min(3)))
                * smoothstep(0.0, 4.0, into_main[k])
                * smoothstep(0.0, 0.3, level[k]);
            RibbonPoint {
                level: level[k] as f32,
                bank: bank[k] as f32,
                speed: speed as f32,
                slope: slope as f32,
                fade: fade as f32,
                ground: [level[k] as f32; ACROSS + 1],
                ..p
            }
        })
        .collect()
}

/// Where a ribbon meets the sea: its first point whose level has come down to the sea's (within
/// 5 cm), from which its channel is the sea's water; none for a river that ends above it.
pub fn sea_mouth(points: &[RibbonPoint]) -> Option<usize> {
    points.iter().position(|p| p.level <= 0.05)
}

/// The index of the point of `points` nearest `at`.
fn nearest(points: &[RibbonPoint], at: [f64; 2]) -> usize {
    let mut best = (f64::MAX, 0);
    for (k, p) in points.iter().enumerate() {
        let (dx, dy) = (
            f64::from(p.position[0]) - at[0],
            f64::from(p.position[1]) - at[1],
        );
        let d = dx * dx + dy * dy;
        if d < best.0 {
            best = (d, k);
        }
    }
    best.1
}

/// The height of `height` at (x, y) metres in its frame, as the island's mesh draws its coarse
/// cells: each split along its (i + 1, j) – (i, j + 1) diagonal, as
/// `forge_geom::city::heightfield_mesh` splits it (and `ground_height` in `water.slang` reads
/// it).
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

/// The height of `height` at (x, y) through a Catmull–Rom cubic of its 4 × 4 nearest samples
/// (the samples clamped at the border): the ground smoothed between the samples, which the
/// channels' cells are drawn on ([`crate::channel`]) and the levels read.
pub fn smooth_height(height: &Field2<f32>, x: f64, y: f64) -> f64 {
    let last = f64::from(height.size - 2);
    let (gx, gy) = (x / height.spacing, y / height.spacing);
    let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
    let (tx, ty) = ((gx - cx).clamp(0.0, 1.0), (gy - cy).clamp(0.0, 1.0));
    let weights = |t: f64| {
        let (t2, t3) = (t * t, t * t * t);
        [
            0.5 * (-t3 + 2.0 * t2 - t),
            0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
            0.5 * (-3.0 * t3 + 4.0 * t2 + t),
            0.5 * (t3 - t2),
        ]
    };
    let (wx, wy) = (weights(tx), weights(ty));
    let top = i64::from(height.size - 1);
    let mut sum = 0.0;
    for (b, wyb) in wy.iter().enumerate() {
        let j = (cy as i64 + b as i64 - 1).clamp(0, top) as u32;
        let mut row = 0.0;
        for (a, wxa) in wx.iter().enumerate() {
            let i = (cx as i64 + a as i64 - 1).clamp(0, top) as u32;
            row += wxa * f64::from(height.get(i, j));
        }
        sum += wyb * row;
    }
    sum
}

/// Sets each point's `ground`: the highest `surface` (the ground as drawn, x and y in the
/// field's frame) stands under the ribbon's quads around each vertex, from samples at most
/// [`GROUND_SAMPLES`] metres apart over each quad. Far away the water lies on it, where the
/// ground's coarser levels of detail may have filled its channel.
pub fn rest_on(
    ribbons: &mut [Ribbon],
    surface: &(dyn Fn(f64, f64) -> f64 + Sync),
    pool: &TaskPool,
) {
    pool.par_chunks_mut(ribbons, 1, |_, chunk| {
        let points = &mut chunk[0].points;
        let vertex = |p: &RibbonPoint, j: usize| -> [f64; 2] {
            let across = (j as f64 / ACROSS as f64) * 2.0 - 1.0;
            offset(p, 0.0, across * f64::from(p.reach))
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
                let side = |a: [f64; 2], b: [f64; 2]| {
                    distance([a[0], a[1], 0.0, 0.0], [b[0], b[1], 0.0, 0.0])
                };
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
                        highest = highest.max(surface(at(0), at(1)));
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
    });
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
    fn a_valley_gives_a_ribbon_down_it_whose_water_falls_under_its_banks() {
        // A V-shaped valley along x = 100 m falling towards y = 0, the border and the outlet.
        let valley = Field2::from_fn(21, 10.0, |x, y| {
            2.0 * (x as f32 - 10.0).abs() + y as f32 + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 15);
        let params = RibbonParams::default();
        let ribbons = ribbons(&valley, &rivers, &Lakes::default(), &params);
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
            // Downstream is −y, the river grows, and its water only falls.
            assert!(w[0].direction[1] < -0.999);
            assert!(w[1].half_width >= w[0].half_width && w[1].depth >= w[0].depth);
            assert!(w[1].level <= w[0].level);
        }
        assert!((length(points) - rivers.rivers[0].length() - 10.0).abs() < 1e-2);
        // Its size from its catchment, its speed from the valley's 10 % fall.
        let last = points[points.len() - 1];
        let area = ribbon.mouth_area;
        assert!((f64::from(last.half_width) - 0.5 * hydrology::width(area)).abs() < 1e-4);
        assert!((f64::from(last.depth) - depth(area)).abs() < 1e-6);
        let mid = points[points.len() / 2];
        assert!((mid.slope - 0.1).abs() < 1e-3, "{}", mid.slope);
        let chezy = 15.0 * (mid.depth * mid.slope).sqrt();
        assert!((mid.speed - chezy.clamp(0.3, 3.0)).abs() < 1e-5);
        // The water stands its freeboard under the valley's floor (the V's bottom is the
        // lowest ground), and under both banks.
        let floor = smooth_height(&valley, 100.0, f64::from(mid.position[1]));
        let freeboard = 0.05 + 0.04 * 2.0 * f64::from(mid.half_width);
        assert!(
            (f64::from(mid.level) - (floor - freeboard)).abs() < 0.25,
            "{} against {}",
            mid.level,
            floor - freeboard
        );
        assert!(mid.bank > mid.level);
        // It grows from its spring over its first 40 m, its water fading in over the first 8 m.
        assert_eq!(points[0].fade, 0.0);
        assert!(points[1].fade > 0.0 && points[1].fade < 1.0);
        assert_eq!(points[2].fade, 1.0);
        assert!(points[0].half_width < 0.2 * points[10].half_width);
        assert!(points[0].depth < 0.4 * points[10].depth);
        assert!(points[points.len() / 2].fade == 1.0);
        assert!((depth(1.0e6) - 0.4).abs() < 1e-12);
        assert!((depth(1.0e7) / depth(1.0e6) - 10f64.powf(0.375)).abs() < 1e-9);
        // The drawn ground meets the field at its samples, and the cubic goes through them.
        assert_eq!(
            drawn_height(&valley, 30.0, 70.0),
            f64::from(valley.get(3, 7))
        );
        assert!((smooth_height(&valley, 30.0, 70.0) - f64::from(valley.get(3, 7))).abs() < 1e-9);
    }

    #[test]
    fn a_tributary_ends_at_its_river_level_and_bends_stay_open() {
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
        let params = RibbonParams::default();
        let ribbons = ribbons(&fork, &rivers, &Lakes::default(), &params);
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
            // A tributary ends on its river's course near the junction's point, at that river's
            // level or over it, and is not drawn in its channel.
            let Mouth::Junction { river, point } = rivers.rivers[ribbon.river as usize].mouth
            else {
                panic!("a tributary ends at a junction");
            };
            let at = rivers.rivers[river as usize].points[point as usize];
            let end = ribbon.points[ribbon.points.len() - 1];
            let on_course = trunk
                .points
                .windows(2)
                .map(|w| {
                    let f = |p: [f32; 2]| [f64::from(p[0]), f64::from(p[1])];
                    segment_distance(f(end.position), f(w[0].position), f(w[1].position)).0
                })
                .fold(f64::MAX, f64::min);
            assert!(on_course < 1e-3, "{on_course}");
            let (dx, dy) = (end.position[0] - at[0], end.position[1] - at[1]);
            assert!((dx * dx + dy * dy).sqrt() < 10.0);
            let m = nearest(&trunk.points, [f64::from(at[0]), f64::from(at[1])]);
            assert!(end.level >= trunk.points[m].level);
            assert_eq!(end.fade, 0.0);
            // The D8 course turns by 45° where the branch meets the trunk: smoothed, the
            // ribbon's reach stays under 0.8 of the radius there, its width changing gradually.
            for w in ribbon.points.windows(3) {
                let a = [w[0].position[0], w[0].position[1]].map(f64::from);
                let b = [w[1].position[0], w[1].position[1]].map(f64::from);
                let c = [w[2].position[0], w[2].position[1]].map(f64::from);
                let bend = curvature(
                    [a[0], a[1], 0.0, 0.0],
                    [b[0], b[1], 0.0, 0.0],
                    [c[0], c[1], 0.0, 0.0],
                );
                assert!(f64::from(w[1].reach) * bend <= 0.8 + 1e-3 || w[1].half_width <= 0.25);
                assert!((w[1].half_width - w[0].half_width).abs() <= 0.25 * 4.4 + 1e-3);
            }
        }
        // Deterministic.
        assert_eq!(
            super::ribbons(&fork, &rivers, &Lakes::default(), &params),
            ribbons
        );
    }
}
