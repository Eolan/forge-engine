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
//!   (#120).
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
    Corner, Ribbon, Step, corner_samples, cross, drawn_height, lip_shift, offset, segment_distance,
    smooth_height, smoothstep, sub,
};

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
}

impl Default for ChannelParams {
    /// 8 m past the water, a bank rising by 0.3 m a metre and more in a straight reach, the
    /// bend's share full at a radius of twice the half width plus 4 m, a channel shoaling over
    /// 8 m and three widths into a lake, the banks flattening under 1.5 m over the sea, cells of
    /// 8 m drawn in quads of 1 m, a lake's shore within
    /// a metre of its level, the coast's cells crossing the sea's level and the sand's top
    /// (2.5 m, the island's layer rule), no channel through a lake.
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
        // The lakes' shores: the cells of a lake's mask whose ground spans its level within
        // `shore`, and a cell more all round, where the smoothed ground takes over.
        let side_us = side as usize;
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
            refined,
            inner,
        }
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
                return drawn;
            }
            drawn + (smooth_height(height, x, y) - drawn) * smooth
        };
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
        carved
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
/// a stone with a chance of 3 %, rising to a third where the water falls 15 % (the rapids), at a
/// random place across the middle 70 % of the water and along to the next point. Its radius is
/// 0.25 to 0.85 m, at least 0.8 of the water's depth there, so most break the surface, and at
/// most 0.45 of the half width, so the river flows past them. On each step's lip (#122), a row of
/// boulders about as tall as the step with a gap the fall pours through ([`lip_stones`]). Every
/// draw is a hash of `seed`, the ribbon and the point (the point's index before the steps; a
/// lip's index, negated, for its stones) (D-016).
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
            if draw(0) >= 0.03 + 0.3 * smoothstep(0.03, 0.15, fall) {
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
            let radius = (0.25 + 0.6 * draw(3)).max(0.8 * depth).min(0.45 * half);
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

/// The stones on a step's lip ([`stones`], #122): a row of boulders, as steps form on keystones
/// (Zimmermann & Church), each 0.8 to 1.1 of the step's height across (its drop and its pool's
/// scour; Forge's choice), half a metre at least and 0.9 of the half width at most, as many as
/// cover about 55 % of the water's width, in slots across it along the bowed lip. One slot,
/// drawn, stays open, so the fall pours through a gap between rocks rather than over a straight
/// line.
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
    let slots = ((0.55 * 2.0 * half / across_each).round() as i32).clamp(1, 6) + 1;
    let open = (draw(1) * f64::from(slots)).floor() as i32;
    (0..slots)
        .filter(|&i| i != open)
        .map(|i| {
            let slot = (f64::from(i) + 0.25 + 0.5 * draw(10 + i)) / f64::from(slots);
            let across = (slot - 0.5) * 1.9 * half;
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
/// more (none under 2.5 %), on either side, half a metre to three metres past the water's edge,
/// 0.3 to 1.1 m across. They stand clear of the water. Every draw is a hash of `seed`, the
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
                radius: 0.3 + 0.8 * draw(4) * draw(4),
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
            assert!((0.3..=1.1).contains(&s.radius));
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
        for q in &p[lip + 1..lip + 6] {
            let [x, y] = [q.position[0], q.position[1]].map(f64::from);
            assert!(f64::from(q.level) <= smooth_height(&valley, x, y).max(f64::from(level)));
        }
        assert!(p[lip + 4].level < level);
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
