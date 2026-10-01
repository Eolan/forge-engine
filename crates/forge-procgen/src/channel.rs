//! The rivers' channels carved into the ground (issue #105, D-038's rivers): the bed the water
//! of [`crate::river`] stands in, on cells drawn finer than the field's.
//! - Across a river the channel is a parabola from the level at the water's edge down to the
//!   depth in the middle; past the edge the bank rises `a x + b x²` until it meets the ground,
//!   which it only ever lowers. The level, the depth and the half width run linearly along each
//!   segment of the ribbon, and where two rivers' channels meet the lower one wins.
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

use crate::field::Field2;
use crate::lake::LakeWater;
use crate::river::{Ribbon, drawn_height, offset, segment_distance, smooth_height, smoothstep};

/// How the channels are carved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelParams {
    /// Metres past the water's edge the carve and the smoothed ground reach.
    pub margin: f64,
    /// The bank's rise past the water's edge: `a x + b x²` metres at `x` metres out.
    pub bank: (f64, f64),
    /// Quads a side of a cell drawn finer.
    pub split: u32,
    /// Metres either side of a lake's level over which its shore's cells are drawn finer.
    pub shore: f64,
    /// Heights whose contours, where the ground crosses them, are drawn on finer cells: the
    /// coast's (the sea's level, the top of the sand).
    pub coast: [f64; 2],
    /// Whether a river's channel runs on through the lakes it crosses (no: their beds silt up).
    pub carve_lakes: bool,
}

impl Default for ChannelParams {
    /// 8 m past the water, a bank rising by half a metre a metre and more, cells of 8 m drawn
    /// in quads of 1 m, a lake's shore within a metre of its level, the coast's cells crossing the
    /// sea's level and the sand's top (2.5 m, the island's layer rule), no channel through a lake.
    fn default() -> Self {
        Self {
            margin: 8.0,
            bank: (0.5, 0.1),
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
}

impl Segment {
    fn at(v: [f64; 2], t: f64) -> f64 {
        v[0] + (v[1] - v[0]) * t
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
        // No channel through a lake: its bed is the lake's (a river's channel silts up there).
        let last = f64::from(height.size - 1);
        let under_lake = |p: [f32; 2]| {
            let (x, y) = (
                (f64::from(p[0]) / spacing).round().clamp(0.0, last) as u32,
                (f64::from(p[1]) / spacing).round().clamp(0.0, last) as u32,
            );
            lakes.iter().any(|l| l.covers(x, y))
        };
        let segments: Vec<Segment> = ribbons
            .iter()
            .flat_map(|r| r.points.windows(2))
            .filter(|w| {
                params.carve_lakes || !(under_lake(w[0].position) && under_lake(w[1].position))
            })
            .map(|w| {
                let f = |v: f32| f64::from(v);
                Segment {
                    a: [f(w[0].position[0]), f(w[0].position[1])],
                    b: [f(w[1].position[0]), f(w[1].position[1])],
                    level: [f(w[0].level), f(w[1].level)],
                    depth: [f(w[0].depth), f(w[1].depth)],
                    half: [f(w[0].half_width), f(w[1].half_width)],
                }
            })
            .collect();
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
        let mut start = vec![0u32; count + 1];
        for s in &segments {
            let (xs, ys) = cells_of(s);
            for y in ys {
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
        for (index, s) in segments.iter().enumerate() {
            let (xs, ys) = cells_of(s);
            for y in ys {
                for x in xs.clone() {
                    let c = (y * side + x) as usize;
                    list[fill[c] as usize] = index as u32;
                    fill[c] += 1;
                }
            }
        }
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
        let last = f64::from(self.side - 1);
        let (cx, cy) = (
            (x / self.spacing).floor().clamp(0.0, last) as usize,
            (y / self.spacing).floor().clamp(0.0, last) as usize,
        );
        let c = cy * self.side as usize + cx;
        let segments = &self.list[self.start[c] as usize..self.start[c + 1] as usize];
        let drawn = drawn_height(height, x, y);
        let margin = self.params.margin;
        // How far out of each segment's water the point is, and its weights: the cubic's, full
        // within a metre of the water and gone by the margin (and full inside the refined
        // cells, gone at their outline), and the carve's, gone over the margin's last 3 m.
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
        let base = drawn + (smooth_height(height, x, y) - drawn) * smooth;
        let (a, b) = self.params.bank;
        let mut carved = base;
        for &s in segments {
            let s = &self.segments[s as usize];
            let (r, t) = segment_distance([x, y], s.a, s.b);
            let half = Segment::at(s.half, t);
            let out = r - half;
            if out >= margin {
                continue;
            }
            let level = Segment::at(s.level, t);
            let channel = if out <= 0.0 {
                let u = r / half.max(1e-6);
                level - Segment::at(s.depth, t) * (1.0 - u * u)
            } else {
                level + a * out + b * out * out
            };
            let weight = 1.0 - smoothstep(margin - 3.0, margin, out);
            carved = carved.min(base + (channel.min(base) - base) * weight);
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
/// most 0.45 of the half width, so the river flows past them. Every draw is a hash of `seed`,
/// the ribbon and the point (D-016).
pub fn stones(
    ribbons: &[Ribbon],
    channels: &Channels,
    height: &Field2<f32>,
    seed: u64,
) -> Vec<Stone> {
    let mut out = Vec::new();
    for (r, ribbon) in ribbons.iter().enumerate() {
        for (k, pair) in ribbon.points.windows(2).enumerate() {
            let (p, next) = (pair[0], pair[1]);
            if p.fade < 0.9 || next.fade < 0.9 {
                continue;
            }
            let draw = |salt: i32| f64::from(unit_f32(hash_cell3(seed, r as i32, k as i32, salt)));
            let fall = f64::from(p.slope);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::drain;
    use crate::hydrology::{Lakes, trace_rivers};
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
        let ribbons = ribbons(
            &valley,
            &rivers,
            &Lakes::default(),
            &RibbonParams::default(),
        );
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
}
