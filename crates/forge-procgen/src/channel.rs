//! The rivers' channels carved into the ground (issue #105, D-038's rivers): the bed the water
//! of [`crate::river`] stands in, on cells drawn finer than the field's.
//! - Across a river the channel is a parabola from the level at the water's edge down to the
//!   depth in the middle; past the edge the bank rises `a x + b x²` until it meets the ground,
//!   which it only ever lowers. The level, the depth and the half width run linearly along each
//!   segment of the ribbon, and where two rivers' channels meet the lower one wins.
//! - Where a tributary meets its river, the corners the two channels' banks make either side are
//!   rounded ([`crate::Corner`], [`corner_ground`], #119): a bank rising from a circle's arc, a
//!   shallow bed between the arc and the old corner, which blends into the rivers' beds.
//! - Into and out of a lake ([`crate::Ribbon::lake_runs`]: where the lake's water stands over
//!   the ground) the channel shoals and its banks flatten into the shore, a mouth; in the lake
//!   it runs on as far and fades out, so it ends in no hollow wherever the lake's edge lies
//!   (#120). In front of a mouth into a lake the river's delta raises the lake's floor into a fan
//!   ([`crate::Delta`]), which the channel is then cut across.
//! - Around them, within [`ChannelParams::margin`] of the water's edge, the ground is the
//!   field's samples through a cubic ([`crate::river::smooth_height`]) rather than the 8 m
//!   cells' planes, blended back to those planes by the margin's end, so the valleys the
//!   rivers run in are smooth and the cells around them unchanged.
//! - The cells that reach that far ([`Channels::refined`]) are drawn in `split × split` quads
//!   (`forge_geom::city::refined_heightfield_mesh`), at the heights of [`Channels::height_at`].
//! - So are the lakes' shores (the cells of a lake's mask that span its level, #105) and the
//!   coast's contours (the cells the sea's level or the sand's top cross, #106), and a cell more
//!   around them, on the cubic but not carved: its weight is 1 at a sample whose cells are all
//!   refined and 0 at the others, so it is 0 all along the refined region's outline.
//!
//! Everything is a pure function of the point, `f64` with no transcendental function (D-016).

use forge_core::hash::{hash_cell3, unit_f32};
use forge_task::TaskPool;

use std::ops::RangeInclusive;

use crate::field::Field2;
use crate::lake::LakeWater;
use crate::river::{
    Bar, Corner, Delta, Ribbon, Step, corner_samples, cross, drawn_height, lip_shift, offset,
    segment_distance, smooth_height, smoothstep, sub,
};

/// The deepest a channel's bed lies under its water by a bar ([`Bar`], #127), metres: how far
/// down its flank may raise the ground.
const BAR_DEEPEST: f64 = 3.0;

/// How the channels are carved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelParams {
    /// Metres past the water's edge the carve and the smoothed ground reach.
    pub margin: f64,
    /// The bank's rise past the water's edge in a straight reach: `a x + b x²` metres at `x`
    /// metres out. In a bend `a` falls to a fifth on the inner bank (a point bar) and grows by
    /// three fifths on the outer (a cut bank).
    pub bank: (f64, f64),
    /// How readily the banks take a bend's shape: the course's curvature times the half width
    /// plus 4 m, times this, is the bend's share (1 for a full point bar and cut bank).
    pub bend: f64,
    /// Over how many metres, plus how many of its widths, a channel shoals into a lake and out
    /// of it, to a fifth of its depth at the lake's edge, its banks flattening to three tenths
    /// of their rise; and over as many it runs on into the lake, fading out.
    pub shoal: (f64, f64),
    /// The metres over the sea under which the banks flatten towards a river's mouth, to
    /// three tenths of their rise at the sea's level: a beach, not a cut (D-041's estuary).
    pub beach: f64,
    /// Quads a side of a cell drawn finer.
    pub split: u32,
    /// Metres either side of a lake's level over which its shore's cells are drawn finer.
    pub shore: f64,
    /// Heights whose contours, where the ground crosses them, are drawn on finer cells: the
    /// coast's (the sea's level, the top of the sand).
    pub coast: [f64; 2],
    /// Whether a river's channel runs on through the lakes it crosses (no: their beds silt up,
    /// and it fades out past the lake's edge).
    pub carve_lakes: bool,
    /// Metres over a lake's level the ground rises to across the shallow arm an outlet carries
    /// the lake's water out along ([`crate::LakeWater::arm`], #120): a sill the river's channel
    /// is cut through, its water then trimmed off the arm (`crate::trim_outlets`). `None` leaves
    /// the arm flooded.
    pub sill: Option<f64>,
}

impl Default for ChannelParams {
    /// 8 m past the water, a bank rising by 0.3 m a metre and more in a straight reach, the
    /// bend's share full at a radius of twice the half width plus 4 m, a channel shoaling over
    /// 8 m and three widths into a lake, the banks flattening under 1.5 m over the sea, cells of
    /// 8 m drawn in quads of 1 m, a lake's shore within
    /// a metre of its level, the coast's cells crossing the sea's level and the sand's top
    /// (2.5 m, the island's layer rule), no channel through a lake, the outlets' arms risen 0.2 m
    /// over their lakes.
    fn default() -> Self {
        Self {
            margin: 8.0,
            bank: (0.3, 0.06),
            bend: 2.0,
            shoal: (8.0, 3.0),
            beach: 1.5,
            split: 8,
            shore: 1.0,
            coast: [0.0, 2.5],
            carve_lakes: false,
            sill: Some(0.2),
        }
    }
}

/// A segment of a ribbon, what its channel needs: its ends, and at each end the level, the
/// depth and the half width.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    a: [f64; 2],
    b: [f64; 2],
    level: [f64; 2],
    depth: [f64; 2],
    half: [f64; 2],
    /// How sharply the course bends at each end, −1..1, positive to its left (seen
    /// downstream): the inner bank of a bend is a gentle point bar, the outer a cut bank.
    bend: [f64; 2],
    /// The share of their rise the banks keep at each end: less at a lake's mouth.
    mouth: [f64; 2],
    /// How much of the carve is kept at each end: 1 but in a lake, where it fades out.
    keep: [f64; 2],
    /// The level the banks rise from at each end: the water's before the steps (#122), over a
    /// pool's.
    banks: [f64; 2],
    /// Below a step's lip (#122), the river's direction at the segment's start: it carves
    /// nothing upstream of its start's line, where its lower water would cut the step and the
    /// banks beside the water above away.
    below: Option<[f64; 2]>,
    /// From a step's lip to its pool's first segment (#122), the step's line's bow: the segment
    /// carves as if each point lay that far upstream, as the water bows over it.
    bowed: Option<Bowed>,
}

/// A step's bowed line, as the segments of its fall see it ([`crate::river::lip_shift`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bowed {
    /// The step's lip.
    at: [f64; 2],
    /// The river's direction there.
    down: [f64; 2],
    /// The ribbon's half width there, which `lip_shift` measures across in.
    reach: f64,
    /// Its bow (`RibbonPoint::lip`).
    lip: [f32; 2],
}

/// An outlet's sill (#120): the lake's level and the samples of its shallow arm, in the layout of
/// its mask ([`crate::LakeWater::arm`]).
#[derive(Clone, Debug, PartialEq)]
struct Sill {
    level: f64,
    first: [u32; 2],
    size: [u32; 2],
    arm: Vec<bool>,
}

impl Sill {
    /// How much of the arm is at `q`, 0..1: its samples, bilinearly.
    fn weight(&self, q: [f64; 2], spacing: f64) -> f64 {
        let (gx, gy) = (
            q[0] / spacing - f64::from(self.first[0]),
            q[1] / spacing - f64::from(self.first[1]),
        );
        let (i, j) = (gx.floor(), gy.floor());
        let (tx, ty) = (gx - i, gy - j);
        let at = |di: f64, dj: f64| {
            let (x, y) = (i + di, j + dj);
            if x < 0.0 || y < 0.0 || x >= f64::from(self.size[0]) || y >= f64::from(self.size[1]) {
                return 0.0;
            }
            f64::from(u8::from(
                self.arm[y as usize * self.size[0] as usize + x as usize],
            ))
        };
        let top = at(0.0, 0.0) + (at(1.0, 0.0) - at(0.0, 0.0)) * tx;
        let bottom = at(0.0, 1.0) + (at(1.0, 1.0) - at(0.0, 1.0)) * tx;
        top + (bottom - top) * ty
    }
}

/// A confluence's rounded corner as the channels carve it ([`corner_ground`]): the corner, and
/// the rise a metre out of the banks it touches, the tributary's and the river's.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fillet {
    corner: Corner,
    rise: [f64; 2],
}

impl Segment {
    fn at(v: [f64; 2], t: f64) -> f64 {
        v[0] + (v[1] - v[0]) * t
    }

    /// The bank's rise a metre out at `q` beside the segment, `t` along it, from `a` in a
    /// straight reach: less on a bend's inner bank (a point bar), more on its outer (a cut bank).
    fn rise(&self, q: [f64; 2], t: f64, a: f64) -> f64 {
        // Which side of the course the point is on (positive to its left, seen downstream),
        // against the way the course bends there.
        let side = (self.b[0] - self.a[0]) * (q[1] - self.a[1])
            - (self.b[1] - self.a[1]) * (q[0] - self.a[0]);
        let bend = Segment::at(self.bend, t) * side.signum();
        if bend > 0.0 {
            a * (1.0 - 0.8 * bend)
        } else {
            a * (1.0 - 0.6 * bend)
        }
    }
}

/// The channels of a field's rivers.
#[derive(Clone, Debug, PartialEq)]
pub struct Channels {
    params: ChannelParams,
    spacing: f64,
    /// Cells a side.
    side: u32,
    segments: Vec<Segment>,
    /// Per cell, where its segments start in `list` (one more entry than cells).
    start: Vec<u32>,
    list: Vec<u32>,
    /// The confluences' rounded corners ([`Corner`]), and per cell where its corners start in
    /// `corner_list`.
    corners: Vec<Fillet>,
    corner_start: Vec<u32>,
    corner_list: Vec<u32>,
    /// The deltas' fans on the lakes' floors ([`Delta`], #120), and per cell where its fans
    /// start in `fan_list`.
    fans: Vec<Delta>,
    fan_start: Vec<u32>,
    fan_list: Vec<u32>,
    /// The outlets' sills (#120), and per cell where its sills start in `sill_list`.
    sills: Vec<Sill>,
    sill_start: Vec<u32>,
    sill_list: Vec<u32>,
    /// The bars in the large mouths ([`Bar`], #127), and per cell where its bars start in
    /// `bar_list`.
    bars: Vec<Bar>,
    bar_start: Vec<u32>,
    bar_list: Vec<u32>,
    refined: Vec<u32>,
    /// Per sample, whether every cell around it is refined: the smoothed ground's weight there
    /// (bilinear between the samples, so it is 0 all along the refined region's outline).
    inner: Vec<bool>,
}

impl Channels {
    /// The channels of `ribbons` (made over `height`, with their levels), and the shores of
    /// `lakes`, whose cells are refined and smoothed but not carved.
    pub fn new(
        height: &Field2<f32>,
        ribbons: &[Ribbon],
        lakes: &[LakeWater],
        params: &ChannelParams,
    ) -> Self {
        let spacing = height.spacing;
        let side = height.size - 1;
        let mut segments: Vec<Segment> = Vec::new();
        for r in ribbons {
            let p = &r.points;
            let n = p.len();
            let f = |v: f32| f64::from(v);
            let at = |k: usize| [f(p[k].position[0]), f(p[k].position[1])];
            let mut arc = vec![0.0; n];
            for k in 1..n {
                let (a, b) = (at(k - 1), at(k));
                arc[k] = arc[k - 1] + (b[0] - a[0]).hypot(b[1] - a[1]);
            }
            // The bend at each point: the signed curvature over the points either side, times
            // the river's half width and a few metres (a stream turns sharply in a tight bend).
            let bend: Vec<f64> = (0..n)
                .map(|k| {
                    let (a, b, c) = (at(k.saturating_sub(2)), at(k), at((k + 2).min(n - 1)));
                    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                    let sides = (b[0] - a[0]).hypot(b[1] - a[1])
                        * (c[0] - b[0]).hypot(c[1] - b[1])
                        * (c[0] - a[0]).hypot(c[1] - a[1]);
                    let curvature = if sides > 0.0 {
                        2.0 * cross / sides
                    } else {
                        0.0
                    };
                    (curvature * (f(p[k].half_width) + 4.0) * params.bend).clamp(-1.0, 1.0)
                })
                .collect();
            // Into and out of a lake the channel shoals over `shoal` of its widths, so its bed
            // meets the lake's shallows without a step, and its banks flatten into the shore;
            // in the lake it runs on as far and fades out (its bed is the lake's: a river's
            // channel silts up there). The metres along the course to the nearest point in a
            // lake, and in a lake to the nearest point out of it, each way.
            let under: Vec<bool> = (0..n).map(|k| r.in_lake(k)).collect();
            let (mut to_lake, mut to_shore) = (vec![f64::MAX; n], vec![f64::MAX; n]);
            for k in 0..n {
                let step = if k > 0 { arc[k] - arc[k - 1] } else { 0.0 };
                (to_lake[k], to_shore[k]) = match (under[k], k) {
                    (true, 0) => (0.0, f64::MAX),
                    (true, _) => (0.0, to_shore[k - 1] + step),
                    (false, 0) => (f64::MAX, 0.0),
                    (false, _) => (to_lake[k - 1] + step, 0.0),
                };
            }
            for k in (0..n.saturating_sub(1)).rev() {
                let step = arc[k + 1] - arc[k];
                to_lake[k] = to_lake[k].min(to_lake[k + 1] + step);
                to_shore[k] = to_shore[k].min(to_shore[k + 1] + step);
            }
            let reach = |k: usize| params.shoal.1 * 2.0 * f(p[k].half_width) + params.shoal.0;
            let near = |k: usize| smoothstep(0.0, reach(k), to_lake[k]);
            let depth = |k: usize| f(p[k].depth) * (0.2 + 0.8 * near(k));
            let mouth = |k: usize| 0.3 + 0.7 * near(k);
            let keep = |k: usize| {
                if params.carve_lakes {
                    1.0
                } else {
                    1.0 - smoothstep(0.0, reach(k), to_shore[k])
                }
            };
            // Below each step, each segment from just past the lip to the half width and the
            // margin past the foot carves nothing upstream of its own start: the fall is steep,
            // and a lower segment's reach back cut the step away and the banks beside the water
            // above it. Where two segments meet they carve alike, so nothing jumps there.
            let mut below: Vec<Option<[f64; 2]>> = vec![None; n];
            for step in &r.steps {
                let (lip, foot) = (step.lip as usize, step.foot as usize);
                for k in lip + 1..n {
                    if k > foot && arc[k] - arc[foot] > f(p[k].reach) + params.margin + spacing {
                        break;
                    }
                    below[k] = Some([f(p[k].direction[0]), f(p[k].direction[1])]);
                }
            }
            // The step's bow, over its fall and its pool's first segment, whose start it bows.
            let mut bowed: Vec<Option<Bowed>> = vec![None; n];
            for step in &r.steps {
                let lip = step.lip as usize;
                let line = Bowed {
                    at: at(lip),
                    down: [f(p[lip].direction[0]), f(p[lip].direction[1])],
                    reach: f(p[lip].reach),
                    lip: p[lip].lip,
                };
                bowed[lip..=(step.foot as usize).min(n - 1)].fill(Some(line));
            }
            for k in 0..n.saturating_sub(1) {
                if keep(k) == 0.0 && keep(k + 1) == 0.0 {
                    continue;
                }
                segments.push(Segment {
                    a: at(k),
                    b: at(k + 1),
                    level: [f(p[k].level), f(p[k + 1].level)],
                    depth: [depth(k), depth(k + 1)],
                    half: [f(p[k].half_width), f(p[k + 1].half_width)],
                    bend: [bend[k], bend[k + 1]],
                    mouth: [mouth(k), mouth(k + 1)],
                    keep: [keep(k), keep(k + 1)],
                    banks: [f(p[k].unstepped), f(p[k + 1].unstepped)],
                    below: below[k],
                    bowed: bowed[k],
                });
            }
        }
        // The cells each segment reaches: its box grown by its widest half width, the margin
        // and a cell, so a point on a cell's edge finds it from either side.
        let cells_of = |s: &Segment| {
            let grow = s.half[0].max(s.half[1]) + params.margin + spacing;
            let last = i64::from(side) - 1;
            let lo = |v: f64| (((v - grow) / spacing).floor() as i64).clamp(0, last) as u32;
            let hi = |v: f64| (((v + grow) / spacing).floor() as i64).clamp(0, last) as u32;
            (
                lo(s.a[0].min(s.b[0]))..=hi(s.a[0].max(s.b[0])),
                lo(s.a[1].min(s.b[1]))..=hi(s.a[1].max(s.b[1])),
            )
        };
        let count = (side as usize) * (side as usize);
        let (start, list) = bucket(side, &segments.iter().map(cells_of).collect::<Vec<_>>());
        // The corners' cells: around their water and their arc, as far as the margin past it
        // and their bed's blend past the old edges.
        let corners: Vec<Corner> = ribbons
            .iter()
            .flat_map(|r| r.corners.iter().copied())
            .collect();
        let corner_cells = |c: &Corner, grow: f64| {
            let samples = corner_samples(c);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &samples {
                for i in 0..2 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
            let grow = grow + params.margin.max(c.blend);
            let last = i64::from(side) - 1;
            let cell = |v: f64| ((v / spacing).floor() as i64).clamp(0, last) as u32;
            (
                cell(lo[0] - grow)..=cell(hi[0] + grow),
                cell(lo[1] - grow)..=cell(hi[1] + grow),
            )
        };
        let (corner_start, corner_list) = bucket(
            side,
            &corners
                .iter()
                .map(|c| corner_cells(c, spacing))
                .collect::<Vec<_>>(),
        );
        // The deltas' fans: the cells within their reach of their apexes.
        let fans: Vec<Delta> = ribbons
            .iter()
            .flat_map(|r| r.deltas.iter().copied())
            .collect();
        let fan_cells = |f: &Delta| {
            let grow = f.reach();
            let last = i64::from(side) - 1;
            let cell = |v: f64| ((v / spacing).floor() as i64).clamp(0, last) as u32;
            (
                cell(f.apex[0] - grow)..=cell(f.apex[0] + grow),
                cell(f.apex[1] - grow)..=cell(f.apex[1] + grow),
            )
        };
        let (fan_start, fan_list) = bucket(side, &fans.iter().map(fan_cells).collect::<Vec<_>>());
        // The outlets' arms, as their lakes' masks, and the cells a sample of one reaches.
        let sills: Vec<Sill> = ribbons
            .iter()
            .flat_map(|r| r.outlets.iter())
            .filter_map(|o| {
                let lake = lakes.get(o.lake as usize)?;
                let arm = lake.arm(height, o);
                let any = arm.iter().any(|&a| a);
                any.then_some(Sill {
                    level: o.level,
                    first: lake.first,
                    size: lake.size,
                    arm,
                })
            })
            .collect();
        let sill_cells = |s: &Sill| {
            let last = side - 1;
            (
                s.first[0].saturating_sub(1).min(last)..=(s.first[0] + s.size[0]).min(last),
                s.first[1].saturating_sub(1).min(last)..=(s.first[1] + s.size[1]).min(last),
            )
        };
        let (sill_start, sill_list) =
            bucket(side, &sills.iter().map(sill_cells).collect::<Vec<_>>());
        // The mouths' bars: the cells within their reach of their middles, their flanks falling
        // as deep as a channel's bed can be.
        let bars: Vec<Bar> = ribbons
            .iter()
            .flat_map(|r| r.bars.iter().copied())
            .collect();
        let bar_cells = |b: &Bar| {
            let grow = b.reach(BAR_DEEPEST);
            let last = i64::from(side) - 1;
            let cell = |v: f64| ((v / spacing).floor() as i64).clamp(0, last) as u32;
            (
                cell(b.centre[0] - grow)..=cell(b.centre[0] + grow),
                cell(b.centre[1] - grow)..=cell(b.centre[1] + grow),
            )
        };
        let (bar_start, bar_list) = bucket(side, &bars.iter().map(bar_cells).collect::<Vec<_>>());
        // Each corner's bank rises as the rivers' banks it touches do, half a metre out of
        // their water there: no step where it meets them, and no steeper.
        let corners: Vec<Fillet> = corners
            .into_iter()
            .map(|corner| {
                let rise = corner.touches.map(|touch| {
                    let toward = sub(corner.centre, touch);
                    let length = (toward[0] * toward[0] + toward[1] * toward[1]).sqrt();
                    let q = [
                        touch[0] + toward[0] * 0.5 / length,
                        touch[1] + toward[1] * 0.5 / length,
                    ];
                    let last = f64::from(side - 1);
                    let c = (q[1] / spacing).floor().clamp(0.0, last) as usize * side as usize
                        + (q[0] / spacing).floor().clamp(0.0, last) as usize;
                    list[start[c] as usize..start[c + 1] as usize]
                        .iter()
                        .map(|&s| {
                            let s = &segments[s as usize];
                            let (r, t) = segment_distance(q, s.a, s.b);
                            (r - Segment::at(s.half, t), s.rise(q, t, params.bank.0))
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map_or(params.bank.0, |(_, rise)| rise)
                });
                Fillet { corner, rise }
            })
            .collect();
        // A cell is drawn finer when a point of it may be within the margin of a river's water:
        // its centre within that plus half its diagonal.
        let half_diagonal = 0.5 * spacing * std::f64::consts::SQRT_2;
        let mut is_refined: Vec<bool> = (0..count as u32)
            .map(|c| {
                let (x, y) = (c % side, c / side);
                let centre = [
                    (f64::from(x) + 0.5) * spacing,
                    (f64::from(y) + 0.5) * spacing,
                ];
                list[start[c as usize] as usize..start[c as usize + 1] as usize]
                    .iter()
                    .any(|&s| {
                        let s = &segments[s as usize];
                        let (r, t) = segment_distance(centre, s.a, s.b);
                        r <= Segment::at(s.half, t) + params.margin + half_diagonal
                    })
            })
            .collect();
        for c in &corners {
            let (xs, ys) = corner_cells(&c.corner, 0.0);
            for y in ys {
                for x in xs.clone() {
                    is_refined[(y * side + x) as usize] = true;
                }
            }
        }
        // The cells a fan raises the ground in (sampled every half a cell), and a cell more all
        // round, so it never raises the outline the coarser cells share.
        let side_us = side as usize;
        let mut fanned = vec![false; count];
        for f in &fans {
            let (xs, ys) = fan_cells(f);
            for y in ys {
                for x in xs.clone() {
                    let raised = (0..3).any(|v| {
                        (0..3).any(|u| {
                            let q = [
                                (f64::from(x) + 0.5 * f64::from(u)) * spacing,
                                (f64::from(y) + 0.5 * f64::from(v)) * spacing,
                            ];
                            f.surface(q)
                                .is_some_and(|(z, _)| z > drawn_height(height, q[0], q[1]) + 0.01)
                        })
                    });
                    fanned[y as usize * side_us + x as usize] |= raised;
                }
            }
        }
        // And the cells a bar may raise the ground in.
        for b in &bars {
            let (xs, ys) = bar_cells(b);
            for y in ys {
                for x in xs.clone() {
                    let centre = [
                        (f64::from(x) + 0.5) * spacing,
                        (f64::from(y) + 0.5) * spacing,
                    ];
                    if b.outside(centre) <= half_diagonal + BAR_DEEPEST / b.slopes.1.max(1e-3) {
                        fanned[y as usize * side_us + x as usize] = true;
                    }
                }
            }
        }
        // The cells of the outlets' arms, and a cell more all round.
        for sill in &sills {
            for (k, _) in sill.arm.iter().enumerate().filter(|(_, a)| **a) {
                let (i, j) = (
                    sill.first[0] as usize + k % sill.size[0] as usize,
                    sill.first[1] as usize + k / sill.size[0] as usize,
                );
                for (cx, cy) in [
                    (i, j),
                    (i.wrapping_sub(1), j),
                    (i, j.wrapping_sub(1)),
                    (i.wrapping_sub(1), j.wrapping_sub(1)),
                ] {
                    if cx < side_us && cy < side_us {
                        fanned[cy * side_us + cx] = true;
                    }
                }
            }
        }
        for c in (0..count).filter(|&c| fanned[c]) {
            let (x, y) = (c % side_us, c / side_us);
            for dy in 0..3 {
                for dx in 0..3 {
                    let (nx, ny) = ((x + dx).wrapping_sub(1), (y + dy).wrapping_sub(1));
                    if nx < side_us && ny < side_us {
                        is_refined[ny * side_us + nx] = true;
                    }
                }
            }
        }
        // The lakes' shores: the cells of a lake's mask whose ground spans its level within
        // `shore`, and a cell more all round, where the smoothed ground takes over.
        let mut shore = vec![false; count];
        for lake in lakes {
            let level = f64::from(lake.level);
            for j in lake.first[1]..(lake.first[1] + lake.size[1]).min(side) {
                for i in lake.first[0]..(lake.first[0] + lake.size[0]).min(side) {
                    let corners = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
                    if !corners.iter().any(|&(x, y)| lake.covers(x, y)) {
                        continue;
                    }
                    let h = corners.map(|(x, y)| f64::from(height.get(x, y)));
                    let (lo, hi) = (
                        h.iter().copied().fold(f64::MAX, f64::min),
                        h.iter().copied().fold(f64::MIN, f64::max),
                    );
                    if lo <= level + params.shore && hi >= level - params.shore {
                        shore[j as usize * side_us + i as usize] = true;
                    }
                }
            }
        }
        // The coast's contours: the cells whose ground crosses one of them.
        for (c, cell) in shore.iter_mut().enumerate() {
            let (i, j) = ((c % side_us) as u32, (c / side_us) as u32);
            let h = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)]
                .map(|(x, y)| f64::from(height.get(x, y)));
            let (lo, hi) = (
                h.iter().copied().fold(f64::MAX, f64::min),
                h.iter().copied().fold(f64::MIN, f64::max),
            );
            if params.coast.iter().any(|&level| lo <= level && hi >= level) {
                *cell = true;
            }
        }
        for c in (0..count).filter(|&c| shore[c]) {
            let (x, y) = (c % side_us, c / side_us);
            for dy in 0..3 {
                for dx in 0..3 {
                    let (nx, ny) = ((x + dx).wrapping_sub(1), (y + dy).wrapping_sub(1));
                    if nx < side_us && ny < side_us {
                        is_refined[ny * side_us + nx] = true;
                    }
                }
            }
        }
        let refined = (0..count as u32)
            .filter(|&c| is_refined[c as usize])
            .collect();
        let n = side_us + 1;
        let inner = (0..n * n)
            .map(|s| {
                let (x, y) = (s % n, s / n);
                let cell = |cx: usize, cy: usize| {
                    cx < side_us && cy < side_us && is_refined[cy * side_us + cx]
                };
                x > 0
                    && y > 0
                    && cell(x - 1, y - 1)
                    && cell(x, y - 1)
                    && cell(x - 1, y)
                    && cell(x, y)
            })
            .collect();
        Self {
            params: *params,
            spacing,
            side,
            segments,
            start,
            list,
            corners,
            corner_start,
            corner_list,
            fans,
            fan_start,
            fan_list,
            sills,
            sill_start,
            sill_list,
            bars,
            bar_start,
            bar_list,
            refined,
            inner,
        }
    }

    /// `base` at `q` in cell `c`, raised over the outlets' arms to their sills.
    fn silled(&self, c: usize, q: [f64; 2], base: f64) -> f64 {
        let Some(rise) = self.params.sill else {
            return base;
        };
        let mut out = base;
        for &s in &self.sill_list[self.sill_start[c] as usize..self.sill_start[c + 1] as usize] {
            let sill = &self.sills[s as usize];
            let top = sill.level + rise;
            let w = sill.weight(q, self.spacing);
            if top > base && w > 0.0 {
                out = out.max(base + (top - base) * w);
            }
        }
        out
    }

    /// `ground` at `q` in cell `c`, raised by the mouths' bars that reach it: their sand over the
    /// water, their flanks under it down to the channels' beds.
    fn barred(&self, c: usize, q: [f64; 2], ground: f64) -> f64 {
        self.bar_list[self.bar_start[c] as usize..self.bar_start[c + 1] as usize]
            .iter()
            .map(|&b| self.bars[b as usize].surface(q))
            .fold(ground, f64::max)
    }

    /// `base` at `q` in cell `c`, raised by the deltas' fans that reach it.
    fn fanned(&self, c: usize, q: [f64; 2], base: f64) -> f64 {
        let mut out = base;
        for &f in &self.fan_list[self.fan_start[c] as usize..self.fan_start[c + 1] as usize] {
            if let Some((z, keep)) = self.fans[f as usize].surface(q)
                && z > base
            {
                out = out.max(base + (z - base) * keep);
            }
        }
        out
    }

    /// The smoothed ground's weight from the refined cells at (x, y): the samples' `inner`,
    /// bilinearly.
    fn inner_weight(&self, x: f64, y: f64) -> f64 {
        let last = f64::from(self.side - 1);
        let (gx, gy) = (x / self.spacing, y / self.spacing);
        let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
        let (tx, ty) = ((gx - cx).clamp(0.0, 1.0), (gy - cy).clamp(0.0, 1.0));
        let n = self.side as usize + 1;
        let at = |i: usize, j: usize| f64::from(u8::from(self.inner[j * n + i]));
        let (i, j) = (cx as usize, cy as usize);
        let top = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
        let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
        top + (bottom - top) * ty
    }

    /// The cells drawn finer, `j × (size − 1) + i` for the cell whose first corner is sample
    /// `(i, j)`, ascending.
    pub fn refined(&self) -> &[u32] {
        &self.refined
    }

    /// The parameters.
    pub fn params(&self) -> &ChannelParams {
        &self.params
    }

    /// The ground at (x, y) metres in the field's frame with the channels carved: the drawn
    /// cells' planes away from the rivers; near them the cubic, under which the channels are
    /// cut.
    pub fn height_at(&self, height: &Field2<f32>, x: f64, y: f64) -> f64 {
        self.carved(height, x, y, false)
    }

    /// [`Channels::height_at`] on the field's cubic everywhere, not only near the rivers: the
    /// ground drawn finer than the field's cells (#106), which has no cell's plane to meet.
    pub fn cubic_height_at(&self, height: &Field2<f32>, x: f64, y: f64) -> f64 {
        self.carved(height, x, y, true)
    }

    /// The ground at (x, y) with the channels carved, on the cubic everywhere (`cubic`) or
    /// only near the rivers and in the refined cells.
    fn carved(&self, height: &Field2<f32>, x: f64, y: f64, cubic: bool) -> f64 {
        let last = f64::from(self.side - 1);
        let (cx, cy) = (
            (x / self.spacing).floor().clamp(0.0, last) as usize,
            (y / self.spacing).floor().clamp(0.0, last) as usize,
        );
        let c = cy * self.side as usize + cx;
        let segments = &self.list[self.start[c] as usize..self.start[c + 1] as usize];
        let margin = self.params.margin;
        let base = if cubic {
            smooth_height(height, x, y)
        } else {
            let drawn = drawn_height(height, x, y);
            // How far out of each segment's water the point is, and its weights: the cubic's,
            // full within a metre of the water and gone by the margin (and full inside the
            // refined cells, gone at their outline), and the carve's, gone over the margin's
            // last 3 m.
            let mut smooth = self.inner_weight(x, y);
            for &s in segments {
                let s = &self.segments[s as usize];
                let (r, t) = segment_distance([x, y], s.a, s.b);
                let out = r - Segment::at(s.half, t);
                if out < margin {
                    smooth = smooth.max(1.0 - smoothstep(1.0, margin, out));
                }
            }
            if smooth == 0.0 {
                let ground = self.silled(c, [x, y], self.fanned(c, [x, y], drawn));
                return self.barred(c, [x, y], ground);
            }
            drawn + (smooth_height(height, x, y) - drawn) * smooth
        };
        // The deltas' fans on the lakes' floors, under the channels cut across them.
        let base = self.silled(c, [x, y], self.fanned(c, [x, y], base));
        let (a, b) = self.params.bank;
        let mut carved = base;
        for &s in segments {
            let s = &self.segments[s as usize];
            // Over a step's fall, the point as far upstream as the step's line bows there.
            let [x, y] = s.bowed.map_or([x, y], |b| {
                let side = [-b.down[1], b.down[0]];
                let across = ((x - b.at[0]) * side[0] + (y - b.at[1]) * side[1]) / b.reach;
                let shift = lip_shift(b.lip, across);
                [x - b.down[0] * shift, y - b.down[1] * shift]
            });
            let (r, t) = segment_distance([x, y], s.a, s.b);
            let half = Segment::at(s.half, t);
            let out = r - half;
            if out >= margin {
                continue;
            }
            if let Some(down) = s.below
                && (x - s.a[0]) * down[0] + (y - s.a[1]) * down[1] < 0.0
            {
                continue;
            }
            let level = Segment::at(s.level, t);
            let channel = if out <= 0.0 {
                let u = r / half.max(1e-6);
                level - Segment::at(s.depth, t) * (1.0 - u * u)
            } else {
                let a = s.rise([x, y], t, a);
                // Towards the sea the banks flatten into the beach, and into a lake's shore.
                let beach = (0.3 + 0.7 * smoothstep(0.0, self.params.beach, level))
                    .min(Segment::at(s.mouth, t));
                // Over a pool lower than the water was (#122) the bank climbs back to the level
                // it rose from, over a metre and as many as it climbs: the banks run on down the
                // valley evenly past the steps.
                let lift = (Segment::at(s.banks, t) - level).max(0.0);
                level
                    + lift * smoothstep(0.0, lift.max(1.0), out)
                    + beach * (a * out + b * out * out)
            };
            let weight = (1.0 - smoothstep(margin - 3.0, margin, out)) * Segment::at(s.keep, t);
            carved = carved.min(base + (channel.min(base) - base) * weight);
        }
        let corners =
            &self.corner_list[self.corner_start[c] as usize..self.corner_start[c + 1] as usize];
        for &k in corners {
            if let Some((ground, weight)) =
                corner_ground(&self.corners[k as usize], [x, y], &self.params)
            {
                carved = carved.min(base + (ground.min(base) - base) * weight);
            }
        }
        // The mouths' bars over the channels cut round them.
        self.barred(c, [x, y], carved)
    }

    /// The heights of the refined cells' fine vertices, in the layout
    /// `forge_geom::city::HeightfieldDetail` takes: per cell of [`Channels::refined`],
    /// `(split + 1)²` of them row-major along +y.
    pub fn detail(&self, height: &Field2<f32>, pool: &TaskPool) -> Vec<f32> {
        let k = self.params.split.max(1) as usize;
        let per = (k + 1) * (k + 1);
        let fine = self.spacing / k as f64;
        let mut out = vec![0.0f32; self.refined.len() * per];
        pool.par_chunks_mut(&mut out, per * 64, |chunk_index, chunk| {
            for (n, cell) in chunk.chunks_exact_mut(per).enumerate() {
                let c = self.refined[chunk_index * 64 + n] as usize;
                let (i, j) = (c % self.side as usize, c / self.side as usize);
                for v in 0..=k {
                    for u in 0..=k {
                        let x = (i * k + u) as f64 * fine;
                        let y = (j * k + v) as f64 * fine;
                        cell[v * (k + 1) + u] = self.height_at(height, x, y) as f32;
                    }
                }
            }
        });
        out
    }

    /// The ground drawn `factor` times finer than `height` (#106): its samples at
    /// [`Channels::cubic_height_at`], and the refined cells as the fine cells inside them,
    /// each drawn in `split / factor` quads a side (none when `factor` is the split). `factor`
    /// divides the split.
    pub fn fine(&self, height: &Field2<f32>, factor: u32, pool: &TaskPool) -> FineGround {
        assert!(
            factor >= 1 && self.params.split.is_multiple_of(factor),
            "a factor of the split {}",
            self.params.split
        );
        let size = (height.size - 1) * factor + 1;
        let spacing = height.spacing / f64::from(factor);
        let mut fine = Field2::new(size, spacing);
        pool.par_chunks_mut(&mut fine.data, size as usize, |j, row| {
            let y = j as f64 * spacing;
            for (i, h) in row.iter_mut().enumerate() {
                *h = self.cubic_height_at(height, i as f64 * spacing, y) as f32;
            }
        });
        let split = self.params.split / factor;
        let f = factor as usize;
        let (side, fine_side) = (self.side as usize, (size - 1) as usize);
        let mut cells: Vec<u32> = if split > 1 {
            self.refined
                .iter()
                .flat_map(|&c| {
                    let (i, j) = (c as usize % side, c as usize / side);
                    (0..f * f).map(move |s| ((j * f + s / f) * fine_side + i * f + s % f) as u32)
                })
                .collect()
        } else {
            Vec::new()
        };
        cells.sort_unstable();
        let k = split.max(1) as usize;
        let per = (k + 1) * (k + 1);
        let step = spacing / k as f64;
        let mut heights = vec![0.0f32; cells.len() * per];
        pool.par_chunks_mut(&mut heights, per * 256, |chunk_index, chunk| {
            for (n, cell) in chunk.chunks_exact_mut(per).enumerate() {
                let c = cells[chunk_index * 256 + n] as usize;
                let (i, j) = (c % fine_side, c / fine_side);
                for v in 0..=k {
                    for u in 0..=k {
                        let x = (i * k + u) as f64 * step;
                        let y = (j * k + v) as f64 * step;
                        cell[v * (k + 1) + u] = self.cubic_height_at(height, x, y) as f32;
                    }
                }
            }
        });
        FineGround {
            height: fine,
            split,
            cells,
            heights,
        }
    }
}

/// Per cell of a `side × side` grid, the boxes (their cells, inclusive) covering it: where its
/// run starts in the list (one more entry than cells), and the list of the boxes' indices.
fn bucket(side: u32, boxes: &[(RangeInclusive<u32>, RangeInclusive<u32>)]) -> (Vec<u32>, Vec<u32>) {
    let count = (side as usize) * (side as usize);
    let mut start = vec![0u32; count + 1];
    for (xs, ys) in boxes {
        for y in ys.clone() {
            for x in xs.clone() {
                start[(y * side + x) as usize + 1] += 1;
            }
        }
    }
    for c in 0..count {
        start[c + 1] += start[c];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; start[count] as usize];
    for (index, (xs, ys)) in boxes.iter().enumerate() {
        for y in ys.clone() {
            for x in xs.clone() {
                let c = (y * side + x) as usize;
                list[fill[c] as usize] = index as u32;
                fill[c] += 1;
            }
        }
    }
    (start, list)
}

/// The ground a confluence's rounded corner ([`Corner`], #119) carves at `q`, and the share of it
/// kept, or none away from it. In the cone from the circle's centre through where it touches
/// the two edges:
/// - inside the circle the bank rises from the arc as the banks it touches do there, so it meets
///   them along the cone's sides and lies under them between;
/// - past the arc the water's bed falls at [`Corner::slope`] to [`Corner::deepest`], and over
///   [`Corner::blend`] past the old edges (the lines from the tip through where the arc touches
///   them) it rises back to the level, under the rivers' beds by then: their corner is gone
///   under the water, with no step at the cone's sides or past the blend.
///
/// The level and the bank's rise run from the tributary's where the arc touches its edge to the
/// river's where it touches that.
fn corner_ground(f: &Fillet, q: [f64; 2], params: &ChannelParams) -> Option<(f64, f64)> {
    let c = &f.corner;
    let (a, b, v) = (
        sub(c.touches[0], c.centre),
        sub(c.touches[1], c.centre),
        sub(q, c.centre),
    );
    let turn = cross(a, b).signum();
    let (wa, wb) = (cross(a, v) * turn, cross(v, b) * turn);
    if wa < 0.0 || wb < 0.0 {
        return None;
    }
    let w = if wa + wb > 0.0 { wa / (wa + wb) } else { 0.5 };
    let level = c.level[0] + (c.level[1] - c.level[0]) * w;
    let out = c.radius - (v[0] * v[0] + v[1] * v[1]).sqrt();
    let margin = params.margin;
    if out >= margin {
        return None;
    }
    let beach = 0.3 + 0.7 * smoothstep(0.0, params.beach, level);
    if out >= 0.0 {
        let (a, b) = (f.rise[0] + (f.rise[1] - f.rise[0]) * w, params.bank.1);
        let weight = 1.0 - smoothstep(margin - 3.0, margin, out);
        return Some((level + beach * (a * out + b * out * out), weight));
    }
    // How far past each old edge `q` is, away from the centre.
    let past = |touch: [f64; 2]| {
        let edge = sub(touch, c.tip);
        let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt().max(1e-9);
        let away = -cross(edge, sub(c.centre, c.tip)).signum();
        cross(edge, sub(q, c.tip)) * away / length
    };
    let (pa, pb) = (past(c.touches[0]), past(c.touches[1]));
    if pa >= c.blend || pb >= c.blend {
        return None;
    }
    let keep = (1.0 - smoothstep(0.0, c.blend, pa)) * (1.0 - smoothstep(0.0, c.blend, pb));
    let depth = (c.slope * -out).min(c.deepest) * keep;
    Some((level - depth, 1.0))
}

/// The ground drawn finer than its field (#106, [`Channels::fine`]): the fine samples, and its
/// refined cells in the layout `forge_geom::city::HeightfieldDetail` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct FineGround {
    /// The fine samples.
    pub height: Field2<f32>,
    /// Quads a side of a refined fine cell (1: none is refined).
    pub split: u32,
    /// The refined fine cells, `j × (size − 1) + i`, ascending.
    pub cells: Vec<u32>,
    /// Per refined cell in that order, `(split + 1)²` heights row-major along +y.
    pub heights: Vec<f32>,
}

/// A stone in a river's channel (#105): a boulder standing on the bed, most of them breaking the
/// water, which flows around them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stone {
    /// Metres in the field's frame.
    pub position: [f64; 2],
    /// The bed's height under it, metres: what it stands on.
    pub bed: f64,
    /// Its mean radius, metres.
    pub radius: f64,
    /// The water's level around it, metres.
    pub level: f64,
    /// Its turn about the vertical, 0..1 of a turn.
    pub turn: f64,
    /// Random bits: which of the boulders it is.
    pub pick: u32,
    /// The ribbon it stands in, and the point of it the stone is past.
    pub ribbon: u32,
    /// The point of that ribbon it is past (between it and the next).
    pub point: u32,
}

impl Stone {
    /// The radius of its outline at the water's level, taking it as `forge_geom::city::boulder`
    /// shapes a boulder (a sphere squashed to 0.75 of its height, its centre 0.65 of its radius
    /// up): 0 when it stands under the water or clear of it.
    pub fn waterline(&self) -> f64 {
        let t = (self.level - self.bed - 0.65 * self.radius) / (0.75 * self.radius);
        if t.abs() < 1.0 {
            self.radius * (1.0 - t * t).sqrt()
        } else {
            0.0
        }
    }
}

/// The stones in the rivers' channels: past each point of a ribbon drawn in full (not fading),
/// a stone with a chance of 6 %, rising to over a third where the water falls 15 % (the rapids),
/// at a random place across the middle 70 % of the water and along to the next point. One in four
/// is a boulder 0.25 to 0.85 m in radius, at least 0.8 of the water's depth there, so it breaks
/// the surface; the rest are pebbles and cobbles 0.08 to 0.38 m, mostly small, on the bed (#133:
/// the water breaks the big chunks first, the owner's note). Each is at most 0.45 of the half
/// width, so the river flows past them. On three steps' lips in five
/// (#122), one or two boulders about as tall as the step ([`lip_stones`]). Every draw is a hash
/// of `seed`, the ribbon and the point (the point's index before the steps; a lip's index,
/// negated, for its stones) (D-016).
pub fn stones(
    ribbons: &[Ribbon],
    channels: &Channels,
    height: &Field2<f32>,
    seed: u64,
) -> Vec<Stone> {
    let mut out = Vec::new();
    for (r, ribbon) in ribbons.iter().enumerate() {
        let mut steps = ribbon.steps.iter().peekable();
        for (k, pair) in ribbon.points.windows(2).enumerate() {
            let (p, next) = (pair[0], pair[1]);
            while let Some(step) = steps.next_if(|s| s.lip as usize <= k) {
                if step.lip as usize == k {
                    out.extend(lip_stones(r, k, step, ribbon, channels, height, seed));
                }
            }
            if p.fade < 0.9 || next.fade < 0.9 || p.key == u32::MAX {
                continue;
            }
            let key = p.key as i32;
            let draw = |salt: i32| f64::from(unit_f32(hash_cell3(seed, r as i32, key, salt)));
            let fall = f64::from(p.grade);
            if draw(0) >= 0.06 + 0.3 * smoothstep(0.03, 0.15, fall) {
                continue;
            }
            let half = f64::from(p.half_width);
            let across = (draw(1) - 0.5) * 1.4 * half;
            let (dx, dy) = (
                f64::from(next.position[0] - p.position[0]),
                f64::from(next.position[1] - p.position[1]),
            );
            let t = draw(2);
            let q = offset(&p, t * (dx * dx + dy * dy).sqrt(), across);
            let u = across / half;
            let depth = f64::from(p.depth) * (1.0 - u * u);
            let radius = if draw(6) < 0.25 {
                (0.25 + 0.6 * draw(3)).max(0.8 * depth)
            } else {
                0.08 + 0.3 * draw(3) * draw(3)
            }
            .min(0.45 * half);
            out.push(Stone {
                position: q,
                bed: channels.height_at(height, q[0], q[1]),
                radius,
                level: f64::from(p.level) + f64::from(next.level - p.level) * t,
                turn: draw(4),
                pick: (draw(5) * 65536.0) as u32,
                ribbon: r as u32,
                point: k as u32,
            });
        }
    }
    out
}

/// The stones on a step's lip ([`stones`], #122), as steps form on keystones (Zimmermann &
/// Church): none on two lips in five, one on nearly half, two on the rest (a full row of them
/// was far too many rocks in the water, the owner's look). Each is 0.8 to 1.1 of the step's
/// height across (its drop and its pool's scour; Forge's choice), half a metre at least and 0.9
/// of the half width at most, somewhere across the middle 80 % of the water on the bowed lip.
fn lip_stones(
    r: usize,
    k: usize,
    step: &Step,
    ribbon: &Ribbon,
    channels: &Channels,
    height: &Field2<f32>,
    seed: u64,
) -> Vec<Stone> {
    let p = ribbon.points[k];
    let foot = ribbon.points[step.foot as usize];
    let draw = |salt: i32| f64::from(unit_f32(hash_cell3(seed, r as i32, -1 - k as i32, salt)));
    let (half, reach) = (f64::from(p.half_width), f64::from(p.reach));
    let tall = step.drop + f64::from(foot.depth - p.depth).max(0.0);
    let across_each = (tall * (0.8 + 0.3 * draw(0))).max(0.5).min(0.9 * half);
    let count = match draw(1) {
        d if d < 0.4 => 0,
        d if d < 0.85 => 1,
        _ => 2,
    };
    (0..count)
        .map(|i| {
            // Two take a half of the water each.
            let slot = (f64::from(i) + 0.1 + 0.8 * draw(10 + i)) / f64::from(count);
            let across = (slot - 0.5) * 1.6 * half;
            let bow = lip_shift(p.lip, across / reach);
            let q = offset(&p, bow + (draw(20 + i) - 0.5) * 0.3, across);
            let radius = (0.5 * across_each * (0.8 + 0.4 * draw(30 + i)))
                .max(0.6 * f64::from(p.depth))
                .min(0.45 * half);
            Stone {
                position: q,
                bed: channels.height_at(height, q[0], q[1]),
                radius,
                level: f64::from(p.level),
                turn: draw(40 + i),
                pick: (draw(50 + i) * 65536.0) as u32,
                ribbon: r as u32,
                point: k as u32,
            }
        })
        .collect()
}

/// The stones beside the steeper rivers' water (#118): on the gravel of their floors, past each
/// point drawn in full a stone with a chance of up to two fifths where the water falls 6 % or
/// more (none under 2.5 %), on either side, half a metre to three metres past the water's edge:
/// pebbles and cobbles 0.08 to 0.33 m in radius, mostly small (#133: a big stone there would
/// have rolled into the water). They stand clear of the water. Every draw is a hash of `seed`, the
/// ribbon and the point (D-016).
pub fn bank_stones(
    ribbons: &[Ribbon],
    channels: &Channels,
    height: &Field2<f32>,
    seed: u64,
) -> Vec<Stone> {
    let mut out = Vec::new();
    for (r, ribbon) in ribbons.iter().enumerate() {
        for (k, pair) in ribbon.points.windows(2).enumerate() {
            let (p, next) = (pair[0], pair[1]);
            if p.fade < 0.9 || next.fade < 0.9 || p.key == u32::MAX {
                continue;
            }
            let key = p.key as i32;
            let draw = |salt: i32| f64::from(unit_f32(hash_cell3(seed, r as i32, key, salt)));
            if draw(0) >= 0.4 * smoothstep(0.025, 0.06, f64::from(p.grade)) {
                continue;
            }
            let side = if draw(1) < 0.5 { -1.0 } else { 1.0 };
            let across = side * (f64::from(p.half_width) + 0.5 + 2.5 * draw(2));
            let (dx, dy) = (
                f64::from(next.position[0] - p.position[0]),
                f64::from(next.position[1] - p.position[1]),
            );
            let t = draw(3);
            let q = offset(&p, t * (dx * dx + dy * dy).sqrt(), across);
            out.push(Stone {
                position: q,
                bed: channels.height_at(height, q[0], q[1]),
                radius: 0.08 + 0.25 * draw(4) * draw(4),
                level: f64::from(p.level) + f64::from(next.level - p.level) * t,
                turn: draw(5),
                pick: (draw(6) * 65536.0) as u32,
                ribbon: r as u32,
                point: k as u32,
            });
        }
    }
    out
}

/// Paints each river's bed into `layers` as `layer`: the texels whose centre lies within the
/// water's half width and `beyond` metres more of the course. Returns the texels painted.
pub fn paint_beds(layers: &mut Field2<u8>, ribbons: &[Ribbon], layer: u8, beyond: f64) -> usize {
    let cell = layers.spacing;
    let last = i64::from(layers.size) - 1;
    let mut painted = 0;
    for ribbon in ribbons {
        for w in ribbon.points.windows(2) {
            let f = |v: f32| f64::from(v);
            let (a, b) = (
                [f(w[0].position[0]), f(w[0].position[1])],
                [f(w[1].position[0]), f(w[1].position[1])],
            );
            let grow = f(w[0].half_width.max(w[1].half_width)) + beyond;
            let lo = |v: f64| (((v - grow) / cell).floor() as i64).clamp(0, last);
            let hi = |v: f64| (((v + grow) / cell).ceil() as i64).clamp(0, last);
            for ty in lo(a[1].min(b[1]))..=hi(a[1].max(b[1])) {
                for tx in lo(a[0].min(b[0]))..=hi(a[0].max(b[0])) {
                    let p = [(tx as f64 + 0.5) * cell, (ty as f64 + 0.5) * cell];
                    let (r, t) = segment_distance(p, a, b);
                    let half = f(w[0].half_width) + f(w[1].half_width - w[0].half_width) * t;
                    if r <= half + beyond {
                        let texel =
                            &mut layers.data[(ty as u32 * layers.size + tx as u32) as usize];
                        if *texel != layer {
                            *texel = layer;
                            painted += 1;
                        }
                    }
                }
            }
        }
    }
    painted
}

/// Paints the tops of the rivers' deltas' fans ([`Delta`], #120) into `layers` (over the field's
/// square) as `layer`, the sand a river lays on a lake's floor in front of its mouth: every texel
/// a sixth of the fan's half length or more inside its top, in front of its mouth, whose `ground`
/// (x, y in the field's frame: the ground as drawn) is the fan's or the channel cut across it,
/// not the shore's shallows standing over it. The texels' blend then fades the sand out by the
/// top's outline, where the front drops off. Returns the texels painted.
pub fn paint_fans(
    layers: &mut Field2<u8>,
    ribbons: &[Ribbon],
    ground: &dyn Fn(f64, f64) -> f64,
    layer: u8,
) -> usize {
    let cell = layers.spacing;
    let last = i64::from(layers.size) - 1;
    let mut painted = 0;
    for f in ribbons.iter().flat_map(|r| r.deltas.iter()) {
        let grow = f.length.max(f.half_width) + f.wander;
        let lo = |v: f64| (((v - grow) / cell).floor() as i64).clamp(0, last);
        let hi = |v: f64| (((v + grow) / cell).ceil() as i64).clamp(0, last);
        for ty in lo(f.apex[1])..=hi(f.apex[1]) {
            for tx in lo(f.apex[0])..=hi(f.apex[0]) {
                let q = [(tx as f64 + 0.5) * cell, (ty as f64 + 0.5) * cell];
                let along = (q[0] - f.apex[0]) * f.down[0] + (q[1] - f.apex[1]) * f.down[1];
                let fan = f
                    .surface(q)
                    .is_some_and(|(z, _)| ground(q[0], q[1]) <= z + 0.05);
                let inside = f.outside(q) < -f.length / 12.0;
                if along > -f.half_width && inside && fan {
                    let texel = &mut layers.data[(ty as u32 * layers.size + tx as u32) as usize];
                    if *texel != layer {
                        *texel = layer;
                        painted += 1;
                    }
                }
            }
        }
    }
    painted
}

/// Paints the mouths' bars ([`Bar`], #127) into `layers` (over the field's square) as `layer`,
/// their sand: every texel whose centre lies inside a bar's outline, or within its flank's first
/// metre under the water. Returns the texels painted.
pub fn paint_bars(layers: &mut Field2<u8>, ribbons: &[Ribbon], layer: u8) -> usize {
    let cell = layers.spacing;
    let last = i64::from(layers.size) - 1;
    let mut painted = 0;
    for b in ribbons.iter().flat_map(|r| r.bars.iter()) {
        let grow = b.reach(0.0) + 1.0;
        let lo = |v: f64| (((v - grow) / cell).floor() as i64).clamp(0, last);
        let hi = |v: f64| (((v + grow) / cell).ceil() as i64).clamp(0, last);
        for ty in lo(b.centre[1])..=hi(b.centre[1]) {
            for tx in lo(b.centre[0])..=hi(b.centre[0]) {
                let q = [(tx as f64 + 0.5) * cell, (ty as f64 + 0.5) * cell];
                if b.outside(q) < 1.0 / b.slopes.1.max(1e-3) {
                    let texel = &mut layers.data[(ty as u32 * layers.size + tx as u32) as usize];
                    if *texel != layer {
                        *texel = layer;
                        painted += 1;
                    }
                }
            }
        }
    }
    painted
}

/// Paints the rivers' banks into `layers` (over the field's square): every texel of one of the
/// layers of `from` whose centre is within `reach.0 + reach.1 × width` metres of a river's
/// water becomes `to` (D-041's riparian strip: the lush grass along the banks, wider along a
/// wider river). Returns the texels painted.
pub fn paint_banks(
    layers: &mut Field2<u8>,
    ribbons: &[Ribbon],
    from: &[u8],
    to: u8,
    reach: (f64, f64),
) -> usize {
    let cell = layers.spacing;
    let last = i64::from(layers.size) - 1;
    let mut painted = 0;
    for ribbon in ribbons {
        for w in ribbon.points.windows(2) {
            let f = |v: f32| f64::from(v);
            let (a, b) = (
                [f(w[0].position[0]), f(w[0].position[1])],
                [f(w[1].position[0]), f(w[1].position[1])],
            );
            let half = |t: f64| f(w[0].half_width) + f(w[1].half_width - w[0].half_width) * t;
            let strip = |t: f64| half(t) + reach.0 + reach.1 * 2.0 * half(t);
            let grow = strip(0.0).max(strip(1.0));
            let lo = |v: f64| (((v - grow) / cell).floor() as i64).clamp(0, last);
            let hi = |v: f64| (((v + grow) / cell).ceil() as i64).clamp(0, last);
            for ty in lo(a[1].min(b[1]))..=hi(a[1].max(b[1])) {
                for tx in lo(a[0].min(b[0]))..=hi(a[0].max(b[0])) {
                    let p = [(tx as f64 + 0.5) * cell, (ty as f64 + 0.5) * cell];
                    let (r, t) = segment_distance(p, a, b);
                    if r <= strip(t) {
                        let texel =
                            &mut layers.data[(ty as u32 * layers.size + tx as u32) as usize];
                        if *texel != to && from.contains(texel) {
                            *texel = to;
                            painted += 1;
                        }
                    }
                }
            }
        }
    }
    painted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::drain;
    use crate::hydrology::trace_rivers;
    use crate::river::{RibbonParams, ribbons};
    use forge_task::PoolConfig;

    #[test]
    fn a_channel_holds_its_level_water_and_leaves_the_ground_away_from_it() {
        // The valley of `river`'s test: a V along x = 100 m falling towards y = 0.
        let valley = Field2::from_fn(21, 10.0, |x, y| {
            2.0 * (x as f32 - 10.0).abs() + y as f32 + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 15);
        let ribbons = ribbons(&valley, &rivers, &[], &RibbonParams::default());
        let channels = Channels::new(&valley, &ribbons, &[], &ChannelParams::default());
        // The cells along the valley's floor are refined, those on its far sides are not.
        assert!(!channels.refined().is_empty());
        for &c in channels.refined() {
            let x = c % 20;
            assert!((8..=11).contains(&x), "cell {c}");
        }
        let mid = ribbons[0].points[ribbons[0].points.len() / 2];
        let (x, y) = (f64::from(mid.position[0]), f64::from(mid.position[1]));
        let (level, half) = (f64::from(mid.level), f64::from(mid.half_width));
        // The bed is the depth under the level in the middle, the level at the water's edge;
        // the banks stand over it, and far out the ground is the field's.
        let bed = channels.height_at(&valley, x, y);
        assert!((bed - (level - f64::from(mid.depth))).abs() < 1e-3, "{bed}");
        for side in [-1.0, 1.0] {
            let edge = channels.height_at(&valley, x + side * half, y);
            assert!((edge - level).abs() < 1e-3, "{edge} against {level}");
            let bank = channels.height_at(&valley, x + side * (half + 1.0), y);
            assert!(bank > level + 0.3);
            let far = x + side * 40.0;
            assert_eq!(
                channels.height_at(&valley, far, y),
                drawn_height(&valley, far, y)
            );
        }
        // Stones stand on the bed within the middle of the water, no wider than 0.45 of its
        // half width, the same every time; the valley's 10 % fall gives a fair few.
        let stones = stones(&ribbons, &channels, &valley, 7);
        assert!(stones.len() >= 3, "{} stones", stones.len());
        assert_eq!(stones, super::stones(&ribbons, &channels, &valley, 7));
        for s in &stones {
            let p = ribbons[0].points[s.point as usize];
            let (r, _) = segment_distance(
                s.position,
                [f64::from(p.position[0]), f64::from(p.position[1])],
                {
                    let q = ribbons[0].points[s.point as usize + 1].position;
                    [f64::from(q[0]), f64::from(q[1])]
                },
            );
            assert!(r <= 0.7 * f64::from(p.half_width) + 1e-6);
            assert!(s.radius <= 0.45 * f64::from(p.half_width) + 1e-9);
            assert_eq!(
                s.bed,
                channels.height_at(&valley, s.position[0], s.position[1])
            );
        }
        // Beside the water of the 10 % fall, stones on the floor half a metre to three metres
        // past its edge, the same every time.
        let banked = bank_stones(&ribbons, &channels, &valley, 7);
        assert!(!banked.is_empty());
        assert_eq!(banked, super::bank_stones(&ribbons, &channels, &valley, 7));
        for s in &banked {
            let p = ribbons[0].points[s.point as usize];
            let q = [f64::from(p.position[0]), f64::from(p.position[1])];
            let along = [f64::from(p.direction[0]), f64::from(p.direction[1])];
            let (dx, dy) = (s.position[0] - q[0], s.position[1] - q[1]);
            let across = (dx * along[1] - dy * along[0]).abs();
            let half = f64::from(p.half_width);
            assert!(
                across >= half + 0.5 - 1e-6 && across <= half + 3.0 + 1e-6,
                "{across}"
            );
            assert!((0.08..=0.33).contains(&s.radius));
        }
        // The fine heights match the carve, and they are the field's on the refined region's
        // outline (where the coarse cells meet it).
        let detail = channels.detail(&valley, &pool);
        let k = 8usize;
        assert_eq!(detail.len(), channels.refined().len() * (k + 1) * (k + 1));
        let refined = |i: i64, j: i64| {
            (0..20).contains(&i)
                && (0..20).contains(&j)
                && channels
                    .refined()
                    .binary_search(&((j * 20 + i) as u32))
                    .is_ok()
        };
        for (s, &c) in channels.refined().iter().enumerate() {
            let (i, j) = (i64::from(c % 20), i64::from(c / 20));
            for (u, v) in [(0, 4), (8, 4), (4, 0), (4, 8)] {
                let neighbour = match (u, v) {
                    (0, _) => (i - 1, j),
                    (8, _) => (i + 1, j),
                    (_, 0) => (i, j - 1),
                    _ => (i, j + 1),
                };
                let (fx, fy) = ((i * 8 + u) as f64, (j * 8 + v) as f64);
                let h = detail[s * 81 + v as usize * 9 + u as usize];
                assert_eq!(h, channels.height_at(&valley, fx * 1.25, fy * 1.25) as f32);
                let inside = (0..20).contains(&neighbour.0) && (0..20).contains(&neighbour.1);
                if inside && !refined(neighbour.0, neighbour.1) {
                    let edge = drawn_height(&valley, fx * 1.25, fy * 1.25) as f32;
                    assert!((h - edge).abs() < 1e-4, "{h} against {edge}");
                }
            }
        }
    }

    #[test]
    fn a_river_hands_over_to_a_lake_where_its_water_stands_and_takes_it_back_past_the_lip() {
        // The valley again, 40 samples long, with a bowl 8 m deep at its middle: a lake whose
        // level is the floor's lip downstream (15 m at y = 140 m), the river running through it.
        let valley = Field2::from_fn(41, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 20.0, y as f32 - 20.0);
            let bowl = 8.0 * (1.0 - (dx * dx + dy * dy).sqrt() / 6.0).max(0.0);
            2.0 * dx.abs() + y as f32 + 1.0 - bowl
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let filled = crate::flow::priority_flood(&valley, 0.0);
        let lakes = crate::hydrology::trace_lakes(&valley, &filled, &flow, 0.5);
        let waters = crate::lake::lake_waters(&valley, &filled, &lakes, 0.0);
        assert_eq!(waters.len(), 1);
        let level = waters[0].level;
        let rivers = trace_rivers(&valley, &flow, 15);
        let ribbons = ribbons(&valley, &rivers, &waters, &RibbonParams::default());
        let ribbon = ribbons.last().expect("the valley's river");
        let p = &ribbon.points;
        // Wherever the lake's water stands over the ground the river's level is the lake's (a
        // centimetre over it), never under it upstream, and past the lip it falls, never over
        // the ground across it. One run in the lake, where its water stands half as deep as the
        // river.
        let lake_level = level + 0.01;
        let mut wet = Vec::new();
        for (k, point) in p.iter().enumerate() {
            let x = f64::from(point.position[0]) / 10.0;
            let y = f64::from(point.position[1]) / 10.0;
            let (i, j) = (x.round() as u32, y.round() as u32);
            if waters[0].stands_at(&valley, i, j) {
                wet.push(k);
                assert!((point.level - lake_level).abs() < 1e-4, "{k}");
            }
            if ribbon.in_lake(k) {
                assert!(level - valley.get(i, j) >= 0.5 * point.depth, "{k}");
            }
        }
        assert_eq!(ribbon.lake_runs.len(), 1, "{:?}", ribbon.lake_runs);
        let [first, last] = ribbon.lake_runs[0].map(|k| k as usize);
        assert!(first > 10 && last > first + 6 && last + 10 < p.len());
        let lip = wet[wet.len() - 1];
        assert!(p[..first].iter().all(|q| q.level >= lake_level - 1e-4));
        assert!(p[last..=lip].iter().all(|q| q.level >= lake_level - 1e-4));
        // Past the lip it keeps the lake's level as far as the lake's water is drawn at all (the
        // mask, softened over a sample, covers a corner of the point's cell, the outlet's arm
        // trimmed off) and a point more: under it, the lake's plane hid the river (#120).
        assert_eq!(ribbon.outlets.len(), 1);
        let outlet = &ribbon.outlets[0];
        let drawn = |q: &crate::RibbonPoint| {
            let [x, y] = [q.position[0], q.position[1]].map(|v| (f64::from(v) / 10.0).floor());
            [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                .iter()
                .any(|&(di, dj)| {
                    let (i, j) = ((x + di) as u32, (y + dj) as u32);
                    waters[0].covers(i, j) && !waters[0].in_arm(&valley, outlet, i, j)
                })
        };
        let reached = lip + 1 + p[lip + 1..].iter().take_while(|q| drawn(q)).count();
        assert!(reached > lip + 1);
        assert!(
            p[lip..=reached]
                .iter()
                .all(|q| q.level >= lake_level - 1e-4)
        );
        for q in &p[reached + 1..reached + 6] {
            let [x, y] = [q.position[0], q.position[1]].map(f64::from);
            assert!(f64::from(q.level) <= smooth_height(&valley, x, y).max(f64::from(level)));
        }
        assert!(p[reached + 4].level < level);
        // Its water whole up to the lake and fading over it, gone three points in; coming out,
        // whole again past the run.
        assert_eq!(p[first - 1].fade, 1.0);
        assert!(p[first].fade > 0.5 && p[first].fade < 1.0);
        assert_eq!(p[first + 3].fade, 0.0);
        assert!(p[last].fade > 0.5 && p[last].fade < 1.0);
        assert_eq!(p[last + 1].fade, 1.0);
        // Its channel holds its water up to the lake, and runs on into it fading out: deep in
        // the lake the ground is the field's, uncarved.
        let channels = Channels::new(&valley, &ribbons, &waters, &ChannelParams::default());
        let at = |k: usize| [p[k].position[0], p[k].position[1]].map(f64::from);
        let [x, y] = at(first - 1);
        assert!(channels.cubic_height_at(&valley, x, y) < f64::from(p[first - 1].level));
        let [x, y] = at((first + last) / 2);
        assert_eq!(
            channels.cubic_height_at(&valley, x, y),
            smooth_height(&valley, x, y)
        );
    }

    #[test]
    fn a_river_runs_into_a_lake_through_a_delta_easing_flat_widening_and_laying_a_fan() {
        // The bowl's valley of the test above: the river runs in from y = 400 m, towards y = 0.
        let valley = Field2::from_fn(41, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 20.0, y as f32 - 20.0);
            let bowl = 8.0 * (1.0 - (dx * dx + dy * dy).sqrt() / 6.0).max(0.0);
            2.0 * dx.abs() + y as f32 + 1.0 - bowl
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let filled = crate::flow::priority_flood(&valley, 0.0);
        let lakes = crate::hydrology::trace_lakes(&valley, &filled, &flow, 0.5);
        let waters = crate::lake::lake_waters(&valley, &filled, &lakes, 0.0);
        let level = f64::from(waters[0].level);
        let rivers = trace_rivers(&valley, &flow, 15);
        let delta = crate::river::DeltaParams::default();
        let with = RibbonParams {
            delta: Some(delta),
            ..RibbonParams::default()
        };
        let plain = ribbons(&valley, &rivers, &waters, &RibbonParams::default());
        let deltas = ribbons(&valley, &rivers, &waters, &with);
        let (before, after) = (
            &plain.last().expect("the river").points,
            &deltas.last().expect("the river").points,
        );
        assert_eq!(before.len(), after.len());
        // One delta, at the first point where the lake's water stands, before its run.
        let ribbon = deltas.last().expect("the river");
        assert_eq!(ribbon.deltas.len(), 1);
        let d = ribbon.deltas[0];
        let mouth = after
            .iter()
            .position(|q| {
                let [x, y] = [q.position[0], q.position[1]].map(|v| f64::from(v) / 10.0);
                waters[0].stands_at(&valley, x.round() as u32, y.round() as u32)
            })
            .expect("the lake");
        assert!(mouth < ribbon.lake_runs[0][0] as usize + 1);
        assert_eq!(d.apex, [0, 1].map(|i| f64::from(after[mouth].position[i])));
        assert_eq!(d.level, level);
        assert!(d.down[1] < -0.9, "{:?}", d.down);
        // The water eases flat to the lake's level over the reach before it, `L + (z − L)(2t −
        // t²)` at `t` of the reach up from the mouth: only ever lowered, still only falling, and
        // unchanged beyond.
        let width = 2.0 * f64::from(before[mouth].half_width);
        let reach = delta.reach.0 + delta.reach.1 * width;
        let arc = |k: usize| {
            (k..mouth)
                .map(|j| {
                    let (a, b) = (after[j].position, after[j + 1].position);
                    f64::from(b[0] - a[0]).hypot(f64::from(b[1] - a[1]))
                })
                .sum::<f64>()
        };
        let lake = f64::from(after[mouth].level);
        let mut eased = 0;
        for k in 0..mouth {
            let t = arc(k) / reach;
            let was = f64::from(before[k].level);
            let wanted = if t < 1.0 {
                eased += 1;
                lake + (was - lake) * (2.0 * t - t * t)
            } else {
                was
            };
            assert!((f64::from(after[k].level) - wanted).abs() < 1e-4, "{k}");
            assert!(after[k].level <= before[k].level, "{k}");
            assert!(after[k + 1].level <= after[k].level, "{k}");
        }
        assert!(eased >= 3, "{eased}");
        assert!(after[mouth - 3].level < before[mouth - 3].level - 0.01);
        // Twice as wide and two fifths shallower at the mouth, as wide as before past the reach.
        let ratio = after[mouth].half_width / before[mouth].half_width;
        assert!((ratio - 2.0).abs() < 1e-4, "{ratio}");
        assert!((after[mouth].depth / before[mouth].depth - 0.6).abs() < 1e-4);
        let far = (0..mouth)
            .rev()
            .find(|&k| arc(k) > reach)
            .expect("the reach");
        assert_eq!(after[far].half_width, before[far].half_width);
        // The fan: in front of the mouth its top stands under the lake's water by 0.3 m and
        // deeper away from it, its front falls to the lake's floor, and the ground is never
        // raised over the lake's level less its top's water, nor anywhere out of the lake.
        let channels = Channels::new(&valley, &deltas, &waters, &ChannelParams::default());
        let ground = |q: [f64; 2]| channels.cubic_height_at(&valley, q[0], q[1]);
        let ahead = |along: f64, across: f64| {
            [
                d.apex[0] + d.down[0] * along - d.down[1] * across,
                d.apex[1] + d.down[1] * along + d.down[0] * across,
            ]
        };
        let mut on_top = 0;
        for i in 0..=40 {
            for j in -20..=20 {
                let q = ahead(f64::from(i) * 0.1 * d.length, f64::from(j) * 0.1 * d.length);
                let h = ground(q);
                let base = smooth_height(&valley, q[0], q[1]);
                assert!(h <= base.max(level - d.top.0) + 1e-9, "{q:?}: {h} {base}");
                if let Some((z, _)) = d.surface(q)
                    && z > base + 0.05
                    && d.outside(q) < -1.0
                {
                    on_top += 1;
                    assert!(
                        h >= z - 1e-6 && h <= level - d.top.0 + 1e-9,
                        "{q:?}: {h} {z}"
                    );
                }
            }
        }
        assert!(on_top >= 20, "{on_top}");
        let near = d.surface(ahead(0.1 * d.length, 0.0)).expect("the top").0;
        let end = d.surface(ahead(0.9 * d.length, 0.0)).expect("the top").0;
        assert!(near > end + 0.5 * d.top.1, "{near} {end}");
        // Its cells are drawn finer, and its top is painted.
        let cell = |q: [f64; 2]| ((q[1] / 10.0).floor() * 40.0 + (q[0] / 10.0).floor()) as u32;
        assert!(
            channels
                .refined()
                .binary_search(&cell(ahead(0.5 * d.length, 0.0)))
                .is_ok()
        );
        let mut layers = Field2::new(200, 2.0);
        // The streams down the bowl's sides have small deltas of their own.
        let painted = paint_fans(&mut layers, &deltas, &|x, y| ground([x, y]), 3);
        let all: Vec<_> = deltas.iter().flat_map(|r| r.deltas.iter()).collect();
        assert!(painted > 0 && all.len() > 1);
        let mut on_main = 0;
        for (t, &l) in layers.data.iter().enumerate() {
            if l == 3 {
                let q = [(t % 200) as f64 * 2.0 + 1.0, (t / 200) as f64 * 2.0 + 1.0];
                assert!(ground(q) < level - 0.2, "{q:?}");
                assert!(all.iter().any(|f| f.outside(q) < 0.0), "{q:?}");
                on_main += usize::from(d.outside(q) < 0.0);
            }
        }
        assert!(on_main > 0);
    }

    #[test]
    fn an_outlet_s_flooded_flat_rises_into_a_sill_and_loses_the_lake_s_water() {
        // The bowl's valley, its floor 40 m wide, with a flat 5 cm under the lake's level past its
        // lip (y = 90 to 130 m) and a sill at y = 80 m that holds the lake at 15.2 m.
        let valley = Field2::from_fn(41, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 20.0, y as f32 - 20.0);
            let bowl = 8.0 * (1.0 - (dx * dx + dy * dy).sqrt() / 6.0).max(0.0);
            let floor = match y {
                9..=13 => 15.15,
                8 => 15.2,
                _ => y as f32 + 1.0,
            };
            0.5 * (dx.abs() - 2.0).max(0.0) + floor - bowl
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let filled = crate::flow::priority_flood(&valley, 0.0);
        let lakes = crate::hydrology::trace_lakes(&valley, &filled, &flow, 0.5);
        let mut waters = crate::lake::lake_waters(&valley, &filled, &lakes, 0.0);
        assert_eq!(waters.len(), 1);
        let level = f64::from(waters[0].level);
        assert!((level - 15.2).abs() < 0.06, "{level}");
        let rivers = trace_rivers(&valley, &flow, 15);
        let ribbons = ribbons(&valley, &rivers, &waters, &RibbonParams::default());
        let outlets: Vec<_> = ribbons.iter().flat_map(|r| r.outlets.iter()).collect();
        assert_eq!(outlets.len(), 1);
        let outlet = *outlets[0];
        assert!(outlet.at[1] > 130.0 && outlet.down[1] < -0.9, "{outlet:?}");
        // The arm: the flat's samples past the outlet, none of the bowl's.
        let arm = waters[0].arm(&valley, &outlet);
        let sample = |k: usize| {
            let (w, first) = (waters[0].size[0] as usize, waters[0].first);
            (first[0] as usize + k % w, first[1] as usize + k / w)
        };
        let armed: Vec<(usize, usize)> = (0..arm.len()).filter(|&k| arm[k]).map(sample).collect();
        assert!(
            armed.contains(&(20, 11)) && armed.contains(&(20, 10)),
            "{armed:?}"
        );
        assert!(armed.iter().all(|&(_, y)| y <= 13));
        // With the sill the flat beside the river's water stands over the lake's level, and the
        // lake's water is trimmed off it; without, the flat is under it. The flat is five samples wide.
        let course = |y: f64| {
            let p = ribbons
                .iter()
                .filter(|r| !r.outlets.is_empty())
                .flat_map(|r| r.points.iter())
                .min_by(|a, b| {
                    (f64::from(a.position[1]) - y)
                        .abs()
                        .total_cmp(&(f64::from(b.position[1]) - y).abs())
                })
                .expect("a point");
            (f64::from(p.position[0]), f64::from(p.half_width))
        };
        let beside = |channels: &Channels, y: f64| {
            let (x, half) = course(y);
            [-1.0, 1.0].map(|s| channels.cubic_height_at(&valley, x + s * (half + 1.5), y))
        };
        let sill = Channels::new(&valley, &ribbons, &waters, &ChannelParams::default());
        let flooded = Channels::new(
            &valley,
            &ribbons,
            &waters,
            &ChannelParams {
                sill: None,
                ..ChannelParams::default()
            },
        );
        for y in [100.0, 110.0, 120.0] {
            assert!(beside(&sill, y).iter().all(|&h| h > level), "{y}");
            assert!(beside(&flooded, y).iter().any(|&h| h < level), "{y}");
            let (x, _) = course(y);
            assert!(
                sill.cubic_height_at(&valley, x, y) < level,
                "{y}: the channel"
            );
        }
        let trimmed = crate::lake::trim_outlets(&mut waters, &valley, &ribbons);
        assert_eq!(trimmed, armed.len());
        assert!(!waters[0].covers(20, 11) && waters[0].covers(20, 20));
    }

    #[test]
    fn a_confluence_s_corners_are_rounded_under_water_without_a_step() {
        // Two valleys meeting in a Y (as `river`'s test), its rivers the island's width.
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
        // Without the steps (#122): the fork falls 10 %, and a step's lip is a step.
        let params = RibbonParams {
            steps: None,
            ..RibbonParams::island()
        };
        let ribbons = ribbons(&fork, &rivers, &[], &params);
        let channels = Channels::new(&fork, &ribbons, &[], &ChannelParams::default());
        let corners: Vec<&Corner> = ribbons.iter().flat_map(|r| &r.corners).collect();
        assert!(corners.len() >= 2, "{}", corners.len());
        let height = |p: [f64; 2]| channels.height_at(&fork, p[0], p[1]);
        for c in corners {
            // The corner the edges made is under its water, at the level between theirs (a
            // steep tributary stands over its river there), and the arc is the water's edge: half
            // way along it the ground is at that level or under it, and it rises into the bank.
            let (a, b) = (sub(c.touches[0], c.centre), sub(c.touches[1], c.centre));
            let (la, lb) = (
                (a[0] * a[0] + a[1] * a[1]).sqrt(),
                (b[0] * b[0] + b[1] * b[1]).sqrt(),
            );
            let mid = [a[0] / la + b[0] / lb, a[1] / la + b[1] / lb];
            let length = (mid[0] * mid[0] + mid[1] * mid[1]).sqrt();
            let towards = |r: f64| {
                [
                    c.centre[0] + mid[0] / length * r,
                    c.centre[1] + mid[1] / length * r,
                ]
            };
            let level = 0.5 * (c.level[0] + c.level[1]);
            let (edge, bank) = (height(towards(c.radius)), height(towards(c.radius - 1.0)));
            assert!(height(c.tip) < level - 0.02, "{c:?}");
            assert!(
                edge <= level + 0.01 && bank > edge + 0.15,
                "{edge} {bank} {c:?}"
            );
            // The rivers' water covers its tip and its arc.
            for q in corner_samples(c) {
                let covered = ribbons.iter().any(|r| {
                    r.points.windows(2).any(|w| {
                        let f = |p: [f32; 2]| [f64::from(p[0]), f64::from(p[1])];
                        let (d, t) = segment_distance(q, f(w[0].position), f(w[1].position));
                        d <= f64::from(w[0].cover + (w[1].cover - w[0].cover) * t as f32)
                    })
                });
                assert!(covered, "{q:?} of {c:?}");
            }
            // No step anywhere around it: never steeper than 2 between points 2 cm apart.
            let (centre, reach) = (c.tip, c.radius.max(8.0) + 8.0);
            for j in 0..=(2.0 * reach / 0.25) as u32 {
                for i in 0..=(2.0 * reach / 0.25) as u32 {
                    let p = [
                        centre[0] - reach + f64::from(i) * 0.25,
                        centre[1] - reach + f64::from(j) * 0.25,
                    ];
                    let h = height(p);
                    for q in [[p[0] + 0.02, p[1]], [p[0], p[1] + 0.02]] {
                        assert!((height(q) - h).abs() <= 0.04, "{p:?} {q:?} near {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_large_river_s_mouth_splits_round_bars_of_sand_over_its_water() {
        // A broad valley falling 0.1 m a sample towards the sea at y = 0, its sides rising 0.5 m
        // a sample: its river, thirty times nature's width, is 64 m wide at its mouth.
        let valley = Field2::from_fn(41, 10.0, |x, y| {
            0.1 * y as f32 - 0.5 + 0.5 * (x as f32 - 20.0).abs()
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 15);
        let plain = RibbonParams {
            regional: Some((30.0, 1.0)),
            ..RibbonParams::default()
        };
        let bars = crate::river::BarParams::default();
        let barred = RibbonParams {
            bars: Some(bars),
            ..plain
        };
        let without = ribbons(&valley, &rivers, &[], &plain);
        let with = ribbons(&valley, &rivers, &[], &barred);
        assert_eq!(with, ribbons(&valley, &rivers, &[], &barred));
        let (ribbon, before) = (
            with.last().expect("the valley's river"),
            without.last().expect("the valley's river"),
        );
        assert!(before.bars.is_empty());
        // Two bars: one per 20 m of the mouth's width, two at most.
        let m = crate::river::sea_mouth(&ribbon.points).expect("a mouth at the sea");
        let width = 2.0 * f64::from(ribbon.points[m].half_width);
        assert!(width > 3.0 * bars.per, "{width}");
        assert_eq!(ribbon.bars.len(), 2);
        // The river runs as it did, but wider over the bars' length by their breadths, a
        // parabola along it, ending a third of its width short of the mouth.
        let breadth: f64 = ribbon.bars.iter().map(|b| b.half[1]).sum();
        let length = bars.length * width;
        let mut arc = vec![0.0; ribbon.points.len()];
        for k in 1..arc.len() {
            let (a, b) = (ribbon.points[k - 1].position, ribbon.points[k].position);
            arc[k] = arc[k - 1] + f64::from((b[0] - a[0]).hypot(b[1] - a[1]));
        }
        let middle = arc[m] - bars.gap * width - 0.5 * length;
        for (k, (p, q)) in ribbon.points.iter().zip(&before.points).enumerate() {
            let s = (arc[k] - middle) / (0.5 * length);
            let wider = if s.abs() < 1.0 {
                breadth * (1.0 - s * s)
            } else {
                0.0
            };
            let grown = f64::from(p.half_width) - f64::from(q.half_width);
            assert!((grown - wider).abs() < 1e-3, "{k}: {grown} against {wider}");
            assert_eq!((p.position, p.level), (q.position, q.level));
        }
        // Each bar stands short of the mouth, its crest 0.3 m over the water, the channels
        // either side of it under the water.
        let channels = Channels::new(&valley, &with, &[], &ChannelParams::default());
        let mouth = ribbon.points[m].position[1];
        for b in &ribbon.bars {
            let tail = b.centre[1] + b.down[1] * b.reach(0.0);
            assert!(tail > f64::from(mouth), "{tail} past the mouth at {mouth}");
            let crest = channels.height_at(&valley, b.centre[0], b.centre[1]);
            assert!(
                (crest - b.level_at(b.centre) - bars.top).abs() < 1e-6,
                "{crest}"
            );
            let side = [-b.down[1], b.down[0]];
            for s in [-1.0, 1.0] {
                let out = s * (b.half[1] + b.wander + 2.0);
                let q = [b.centre[0] + side[0] * out, b.centre[1] + side[1] * out];
                let ground = channels.height_at(&valley, q[0], q[1]);
                assert!(ground < b.level_at(q), "{ground} at {q:?}");
            }
        }
        // Their sand, painted inside their outlines and down their flanks' first metre.
        let mut layers = Field2::from_fn(81, 5.0, |_, _| 0u8);
        let painted = paint_bars(&mut layers, &with, 1);
        assert!(painted > 100, "{painted}");
        for (t, _) in layers.data.iter().enumerate().filter(|(_, l)| **l == 1) {
            let q = [
                (f64::from(t as u32 % 81) + 0.5) * 5.0,
                (f64::from(t as u32 / 81) + 0.5) * 5.0,
            ];
            let inside = ribbon
                .bars
                .iter()
                .map(|b| b.outside(q))
                .fold(f64::MAX, f64::min);
            assert!(inside < 3.0, "{q:?}");
        }
        // Their spans across the ribbon's points, which keep the water far away off their sand:
        // each from end to end of the bar's outline over the line across, a point where the bar
        // does not cross it.
        let spans = crate::river::bar_spans(ribbon);
        assert_eq!(spans.len(), ribbon.points.len());
        let mut crossed = [0; 2];
        for (p, span) in ribbon.points.iter().zip(&spans) {
            let at = [f64::from(p.position[0]), f64::from(p.position[1])];
            let side = [-f64::from(p.direction[1]), f64::from(p.direction[0])];
            let reach = f64::from(p.reach);
            for (slot, b) in ribbon.bars.iter().enumerate() {
                let (from, to) = (f64::from(span[2 * slot]), f64::from(span[2 * slot + 1]));
                assert!(-reach <= from && from <= to && to <= reach, "{span:?}");
                let q = |across: f64| [at[0] + side[0] * across, at[1] + side[1] * across];
                if to > from {
                    crossed[slot] += 1;
                    assert!(b.outside(q(from)) < 0.0 && b.outside(q(to)) < 0.0);
                }
                for s in 0..=(20.0 * reach) as i32 {
                    let across = -reach + 0.1 * f64::from(s);
                    if b.outside(q(across)) < 0.0 {
                        assert!(from - 0.25 <= across && across <= to + 0.25, "{across}");
                    }
                }
            }
        }
        // Along most of each bar's length, at points 4 m apart.
        for (n, b) in crossed.iter().zip(&ribbon.bars) {
            assert!(
                f64::from(*n) * 4.0 > 1.5 * b.half[0],
                "{n} points over {b:?}"
            );
        }
    }
}

#[cfg(test)]
mod step_tests {
    use super::*;
    use crate::flow::drain;
    use crate::hydrology::trace_rivers;
    use crate::river::{RibbonParams, ribbons};
    use forge_task::PoolConfig;

    #[test]
    fn a_steep_river_stands_in_pools_between_steps_its_bed_holding_them() {
        // A V along x = 200 m falling 10 % towards y = 0, the island's rivers in it.
        let valley = Field2::from_fn(41, 10.0, |x, y| {
            2.0 * (x as f32 - 20.0).abs() + y as f32 + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 30);
        let params = RibbonParams::island();
        let steps = params.steps.expect("the island's rivers have steps");
        let ribbons = ribbons(&valley, &rivers, &[], &params);
        let r = ribbons.last().expect("the valley's river");
        assert!(r.steps.len() >= 10, "{} steps", r.steps.len());
        assert!(
            r.steps
                .iter()
                .any(|s| r.points[s.lip as usize].lip[0] > 0.1)
        );
        let p = &r.points;
        for (i, s) in r.steps.iter().enumerate() {
            let (lip, foot) = (p[s.lip as usize], p[s.foot as usize]);
            // Each drops the valley's fall over its spacing, a few widths apart, and the pool
            // below stands level from the foot to the next lip.
            assert!(((lip.level - foot.level) as f64 - s.drop).abs() < 1e-4);
            // (The last flattens into the valley's foot, where the field ends.)
            let fall = s.drop / s.spacing;
            assert!(fall <= 0.11, "{s:?}");
            if i + 1 < r.steps.len() {
                assert!((fall - 0.1).abs() < 0.01, "{s:?}");
            }
            assert!(
                s.spacing >= 0.5 * steps.least - 1e-9
                    && s.spacing <= 1.5 * (steps.spacing.0 * s.width).max(steps.least),
                "{s:?}"
            );
            let next = r.steps.get(i + 1).map_or(p.len(), |n| n.lip as usize + 1);
            for q in &p[s.foot as usize..next] {
                if q.unstepped >= foot.level {
                    assert_eq!(q.level, foot.level);
                }
            }
            // Its line bows downstream, and the pool below starts past the bow, so its first quads
            // never fold.
            let bow = f64::from(lip.lip[0] + lip.lip[1].abs());
            assert_eq!(lip.lip, foot.lip);
            let after = p[s.foot as usize + 1].position;
            let gap = f64::from(
                ((after[0] - foot.position[0]).powi(2) + (after[1] - foot.position[1]).powi(2))
                    .sqrt(),
            );
            assert!(gap > bow, "{gap} {bow}");
            // White at the foot, gone by the next lip.
            assert_eq!(foot.foam, 1.0);
            if let Some(n) = r.steps.get(i + 1) {
                assert_eq!(p[n.lip as usize].foam, 0.0);
            }
        }
        // The bed holds the water: past its edge the ground is never under it, in the middle
        // well under it, along every segment of the stepped reach.
        let channels = Channels::new(&valley, &ribbons, &[], &ChannelParams::default());
        let height = |q: [f64; 2]| channels.height_at(&valley, q[0], q[1]);
        let (first, last) = (
            r.steps[0].lip as usize,
            r.steps[r.steps.len() - 1].foot as usize,
        );
        // Where the water is, as the GPU draws it: each point's vertex across moved downstream by
        // its step's bow, the quads between straight.
        let water = |k: usize, t: f64, across: f64| {
            let at = |q: &crate::RibbonPoint| {
                let bow = lip_shift(q.lip, across / f64::from(q.reach));
                crate::river::offset(q, bow, across)
            };
            let (a, b) = (at(&p[k]), at(&p[k + 1]));
            let level = f64::from(p[k].level) + f64::from(p[k + 1].level - p[k].level) * t;
            ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], level)
        };
        for k in first..last {
            for t in [0.0, 0.25, 0.5, 0.75] {
                let mix = |x: f32, y: f32| f64::from(x) + f64::from(y - x) * t;
                let half = mix(p[k].half_width, p[k + 1].half_width);
                let depth = mix(p[k].depth, p[k + 1].depth);
                let (q, level) = water(k, t, 0.0);
                let middle = height(q);
                assert!(middle <= level - 0.5 * depth, "{k} {t}: {middle} {level}");
                for side in [-1.0, 1.0] {
                    let (q, level) = water(k, t, side * (half + 0.3));
                    let edge = height(q);
                    assert!(edge >= level - 0.02, "{k} {t}: {edge} under {level}");
                }
            }
        }
        // Out on the floor the banks run on down the valley past the lips, without a cliff.
        for s in &r.steps {
            let lip = p[s.lip as usize];
            for side in [-1.0, 1.0] {
                let out = side * (f64::from(lip.half_width) + 4.0);
                let line: Vec<f64> = (-8..=8)
                    .map(|i| height(crate::river::offset(&lip, f64::from(i) * 0.25, out)))
                    .collect();
                for pair in line.windows(2) {
                    assert!((pair[1] - pair[0]).abs() <= 0.1, "{line:?}");
                }
            }
        }
    }
}
