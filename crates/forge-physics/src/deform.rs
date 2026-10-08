//! A deformable ground (#185, D-007's deformable layer): snow, sand or mud as a layer of loose
//! material over a hard base, kept as a grid of its thickness, which what stands on it presses
//! into. A pad (a paw, a boot, a wheel's patch) sinks as deep as its pressure over the
//! material's stiffness, never through the base; of what it pushes out, the material's packing
//! share is pressed into the ground under it and the rest heaped in a rim round the print;
//! then the ground about the print slumps wherever it is steeper than the material stands
//! (sand to its angle of repose, snow holding steep walls).
//!
//! As in the buoyancy, no transcendental function, and every pass in a fixed order (the slump
//! moves material between neighbours from a copy of the heights, the same whatever the order),
//! so a layer pressed by the same pads holds the same bits everywhere (D-016).

use glam::{DVec2, Vec2};

/// A soft material's rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Soft {
    /// Its thickness over the base untouched, metres.
    pub depth: f32,
    /// What the hardest press leaves over the base, metres: packed snow, a film of sand.
    pub least: f32,
    /// How firm it is: the pressure that would sink a pad a metre, Pa/m. A pad sinks its
    /// pressure over this.
    pub stiffness: f32,
    /// Of what a pad pushes out, the share packed into the ground under it; the rest is heaped
    /// in a rim round the print.
    pub packing: f32,
    /// The steepest slope it stands at, rise over run; past it, it slumps.
    pub repose: f32,
}

impl Soft {
    /// Fresh snow over hard ground: 6 cm that a dog's paw goes through (it would sink 50 cm
    /// under its 100 kPa), packed under it with little thrown aside, its walls standing at 72°.
    pub const SNOW: Self = Self {
        depth: 0.06,
        least: 0.004,
        stiffness: 2.0e5,
        packing: 0.85,
        repose: 3.0,
    };

    /// Mud (#186): 5 cm of wet soil over firm ground, a dog's paw sinking 4 cm under its 100
    /// kPa. The water in it does not compress, so nearly all a pad pushes out rises round the
    /// print; sticky, it holds its walls at 56°.
    pub const MUD: Self = Self {
        depth: 0.05,
        least: 0.004,
        stiffness: 2.5e6,
        packing: 0.05,
        repose: 1.5,
    };

    /// Sand a little damp, as on a beach over the swash: 3 cm loose over firm sand, a dog's paw
    /// sinking 2.5 cm under its 100 kPa, most of what it moves heaped round the print, the water
    /// between its grains holding its walls at 45° where dry sand would slump to 33°.
    pub const SAND: Self = Self {
        depth: 0.03,
        least: 0.004,
        stiffness: 4.0e6,
        packing: 0.25,
        repose: 1.0,
    };
}

/// What presses: a pad, an ellipse coming down on the layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    /// Its middle, (x, z), metres.
    pub at: DVec2,
    /// The way it points along the ground, (x, z), unit.
    pub heading: Vec2,
    /// Its half sizes across and along its heading, metres.
    pub size: Vec2,
    /// Its pressure, Pa.
    pub pressure: f32,
    /// How far behind it along its heading it was at the step's start, metres: a rolling wheel's
    /// travel, all of which it presses; 0 for a footfall.
    pub sweep: f32,
    /// The radius of the wheel it is, metres: its sole along its heading is the wheel's round,
    /// reaching ahead as far as the wheel meets the material, so the wheel rests in its rut
    /// rather than on the lip before it (#187). 0 for a foot.
    pub wheel: f32,
}

/// How much higher a pad's sole is at its edge than at its middle, as a share of its smaller
/// half size: a paw's or a boot's rounded sole.
const ROUND: f32 = 0.4;
/// How far the rim reaches past the pad's edge, as a share of its half sizes.
const RIM: f32 = 1.2;
/// A wheel's rim reaches farther, as a share of its half width: it moves a long stretch of
/// material at once, which would otherwise stand in walls beside its rut (#187).
const WHEEL_RIM: f32 = 3.0;
/// How far from its border the layer thins to nothing, metres (a bed's edge).
const BEVEL: f32 = 0.06;
/// Passes of the slump after each press at most (it stops once settled: no move over
/// `SETTLED` metres), and the share of a slope's excess each moves.
const SLUMP_PASSES: u32 = 32;
const SETTLED: f32 = 1e-4;
/// Passes of the smoothing after the slump, and the share of a difference between neighbours
/// each moves: the millimetres a rim heaped a step at a time leaves, which a low sun stripes
/// (#189).
const SMOOTH_PASSES: u32 = 3;
const SMOOTH_RATE: f32 = 0.12;
const SLUMP_RATE: f32 = 0.25;

/// A layer of soft material over a hard base: its thickness at the points of a grid.
#[derive(Clone, Debug)]
pub struct Layer {
    soft: Soft,
    /// Where its first point lies, (x, z), metres.
    origin: DVec2,
    /// Metres between two points.
    cell: f32,
    /// Points along x and z.
    size: [u32; 2],
    /// The thickness at each point, metres over the base, row by row along x.
    heights: Vec<f32>,
    /// The slump's copy of the heights.
    scratch: Vec<f32>,
    /// The points changed since [`Layer::take_changed`], as boxes (lowest and highest x, z).
    changed: Vec<([u32; 2], [u32; 2])>,
}

/// Two layers are equal when their material, grid and heights are (the slump's copy aside).
impl PartialEq for Layer {
    fn eq(&self, other: &Self) -> bool {
        self.soft == other.soft
            && self.origin == other.origin
            && self.cell == other.cell
            && self.size == other.size
            && self.heights == other.heights
    }
}

impl Layer {
    /// An untouched layer of `soft` over `size` points `cell` metres apart from `origin`: its
    /// depth everywhere but within `BEVEL` of its border, where it thins smoothly to nothing.
    pub fn new(soft: Soft, origin: DVec2, cell: f32, size: [u32; 2]) -> Self {
        Self::with_bevel(soft, origin, cell, size, BEVEL)
    }

    /// As [`Layer::new`], thinning to nothing over the last `bevel` metres to its border
    /// instead (a deep puddle's gentle edge, #187).
    pub fn with_bevel(soft: Soft, origin: DVec2, cell: f32, size: [u32; 2], bevel: f32) -> Self {
        let mut heights = Vec::with_capacity((size[0] * size[1]) as usize);
        for z in 0..size[1] {
            for x in 0..size[0] {
                let edge = x.min(z).min(size[0] - 1 - x).min(size[1] - 1 - z) as f32 * cell;
                let t = (edge / bevel.max(1e-6)).min(1.0);
                heights.push(soft.depth * t * t * (3.0 - 2.0 * t));
            }
        }
        Self {
            soft,
            origin,
            cell,
            size,
            scratch: heights.clone(),
            heights,
            changed: Vec::new(),
        }
    }

    /// Its material.
    pub fn soft(&self) -> Soft {
        self.soft
    }

    /// Points along x and z.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Where its first point lies, (x, z).
    pub fn origin(&self) -> DVec2 {
        self.origin
    }

    /// Metres between two points.
    pub fn cell(&self) -> f32 {
        self.cell
    }

    /// The thickness at each point, row by row along x.
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// Back to saved heights ([`Layer::heights`]).
    ///
    /// # Panics
    ///
    /// When `heights` is not a height per point.
    pub fn set_heights(&mut self, heights: &[f32]) {
        self.heights.copy_from_slice(heights);
        self.mark([0, 0], [self.size[0] - 1, self.size[1] - 1]);
    }

    /// The points changed since the last call (by a press or [`Layer::set_heights`]), as boxes
    /// from their lowest to their highest (x, z): one for the presses that touch each other (a
    /// wheel's, step after step), so far apart ones (a car's four wheels) stay apart. What a
    /// collision shape made of the layer needs again (#187).
    pub fn take_changed(&mut self) -> Vec<([u32; 2], [u32; 2])> {
        std::mem::take(&mut self.changed)
    }

    /// Adds the points from `low` to `high` to those changed: into a box it touches, else as a
    /// box of its own.
    fn mark(&mut self, low: [u32; 2], high: [u32; 2]) {
        let touches = |(l, h): &([u32; 2], [u32; 2])| {
            (0..2).all(|k| low[k] <= h[k] + 1 && l[k] <= high[k] + 1)
        };
        match self.changed.iter_mut().find(|b| touches(b)) {
            Some((l, h)) => {
                *l = [l[0].min(low[0]), l[1].min(low[1])];
                *h = [h[0].max(high[0]), h[1].max(high[1])];
            }
            None => self.changed.push((low, high)),
        }
    }

    /// Whether (`x`, `z`) is over the layer.
    pub fn covers(&self, at: DVec2) -> bool {
        let g = (at - self.origin) / f64::from(self.cell);
        g.x >= 0.0
            && g.y >= 0.0
            && g.x <= f64::from(self.size[0] - 1)
            && g.y <= f64::from(self.size[1] - 1)
    }

    /// The thickness under (`x`, `z`), between the four nearest points (the border's beyond
    /// the layer).
    pub fn height_at(&self, at: DVec2) -> f32 {
        let g = ((at - self.origin) / f64::from(self.cell)).as_vec2();
        let max = Vec2::new((self.size[0] - 1) as f32, (self.size[1] - 1) as f32);
        let g = g.clamp(Vec2::ZERO, max);
        let g0 = g.floor().min(max - 1.0).max(Vec2::ZERO);
        let t = g - g0;
        let (x, z) = (g0.x as u32, g0.y as u32);
        let h = |dx: u32, dz: u32| {
            self.heights[((z + dz).min(self.size[1] - 1) * self.size[0]
                + (x + dx).min(self.size[0] - 1)) as usize]
        };
        let a = h(0, 0) + (h(1, 0) - h(0, 0)) * t.x;
        let b = h(0, 1) + (h(1, 1) - h(0, 1)) * t.x;
        a + (b - a) * t.y
    }

    /// The material over the layer's points, m³.
    pub fn volume(&self) -> f64 {
        let area = f64::from(self.cell) * f64::from(self.cell);
        self.heights.iter().map(|&h| f64::from(h)).sum::<f64>() * area
    }
    /// Presses `pad` into the layer: under it the ground sinks to the pad's sole (as deep as
    /// its pressure sinks it below the untouched layer, rounded up towards its edge,
    /// never closer to the base than the material's least); the rim takes what is not packed;
    /// then the print and its rim slump. A pad that swept (a rolling wheel) presses the whole
    /// stretch it rolled over, and heaps its rim beside it only. The volume it pushed out, m³
    /// (none off the layer).
    pub fn press(&mut self, pad: Pad) -> f32 {
        if !self.covers(pad.at) {
            return 0.0;
        }
        let soft = self.soft;
        // As deep as the pressure sinks it below the untouched layer, wherever it lands: a pad
        // pressing again where it stands sinks no further (#187).
        let level = (soft.depth - pad.pressure / soft.stiffness).max(soft.least);
        let heading = pad.heading.normalize_or(Vec2::Y);
        let across = Vec2::new(-heading.y, heading.x);
        let wheel = pad.wheel.max(0.0);
        // A wheel meets the material along its round as far ahead as the round rises to the
        // untouched layer.
        let sunk = soft.depth - level;
        let arc = if wheel > 0.0 {
            (2.0 * wheel * sunk - sunk * sunk).max(0.0).sqrt()
        } else {
            0.0
        };
        let (a, b) = (pad.size.x.max(1e-4), pad.size.y.max(arc).max(1e-4));
        let sweep = pad.sweep.max(0.0);
        let round = ROUND * a.min(pad.size.y.max(1e-4));
        let rim_reach = if wheel > 0.0 { WHEEL_RIM } else { RIM };
        // The points the rim reaches, and a cell more for the slump: across the pad and along it
        // (a wheel heaps beside its stretch only), turned onto x and z.
        let across_reach = (1.0 + rim_reach) * a + 2.0 * self.cell;
        let along_reach = if wheel > 0.0 {
            b
        } else {
            (1.0 + rim_reach) * b
        } + sweep
            + 2.0 * self.cell;
        let reach = Vec2::new(
            heading.x.abs() * along_reach + heading.y.abs() * across_reach,
            heading.y.abs() * along_reach + heading.x.abs() * across_reach,
        );
        let (low, high) = self.span(pad.at, reach);
        // At point (x, z): the pad's elliptic distance, 1 on its edge, the stretch it swept
        // over counted as its middle; how much of a rim it takes beside that stretch (all
        // points for a pad that did not sweep); and how far it is before or behind the stretch.
        let shape = |x: u32, z: u32| {
            let d = (self.origin + DVec2::new(f64::from(x), f64::from(z)) * f64::from(self.cell)
                - pad.at)
                .as_vec2();
            let along = d.dot(heading);
            // How much of a rim the point takes along the stretch: all of it beside the stretch,
            // fading over a stretch's length past either end, so the rims of a wheel's steps
            // blend into one berm rather than a ridge a step (#189).
            let beside = if sweep == 0.0 {
                1.0
            } else {
                let past = (-along - sweep).max(along).max(0.0);
                (1.0 - past / sweep).max(0.0)
            };
            let along = if along > 0.0 {
                along
            } else {
                (along + sweep).min(0.0)
            };
            let u = d.dot(across) / a;
            let v = along / b;
            // A wheel's patch is a rectangle, as wide as its tread all along; a foot's an ellipse.
            let r2 = if wheel > 0.0 {
                (u * u).max(v * v)
            } else {
                u * u + v * v
            };
            (r2, beside, along)
        };
        let r2 = |x: u32, z: u32| {
            let (r2, beside, _) = shape(x, z);
            (r2, beside)
        };
        let mut pushed = 0.0f32;
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let (r2, _, along) = shape(x, z);
                if r2 < 1.0 {
                    // A foot's sole rounded towards its edge; a wheel's flat across its tread
                    // (the tyre's cylinder rests on all of it) and along its heading its round.
                    let sole = if wheel > 0.0 {
                        let off = along.abs().min(wheel);
                        level + wheel - (wheel * wheel - off * off).sqrt()
                    } else {
                        level + round * r2
                    };
                    let k = self.index(x, z);
                    if self.heights[k] > sole {
                        pushed += self.heights[k] - sole;
                        self.heights[k] = sole;
                    }
                }
            }
        }
        // The rim: what is not packed, heaped in a ring past the pad's edge, highest halfway
        // out.
        let bump = |(r2, beside): (f32, f32)| {
            let t = (r2.sqrt() - 1.0) / rim_reach;
            if (0.0..1.0).contains(&t) {
                let w = t * (1.0 - t);
                w * w * beside
            } else {
                0.0
            }
        };
        let mut weights = 0.0f32;
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                weights += bump(r2(x, z));
            }
        }
        let rim = (1.0 - soft.packing) * pushed;
        if weights > 0.0 {
            for z in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let w = bump(r2(x, z));
                    if w > 0.0 {
                        let k = self.index(x, z);
                        self.heights[k] += rim * w / weights;
                    }
                }
            }
        }
        // A wheel's rim, heaped a step at a time, smoothed but for the print itself (a foot's
        // rims are of one press, and stay as they are).
        if wheel > 0.0 {
            let width = (high[0] - low[0] + 1) as usize;
            let print: Vec<bool> = (low[1]..=high[1])
                .flat_map(|z| (low[0]..=high[0]).map(move |x| (x, z)))
                .map(|(x, z)| r2(x, z).0 < 1.0)
                .collect();
            self.smooth(low, high, |x, z| {
                print[(z - low[1]) as usize * width + (x - low[0]) as usize]
            });
        }
        // Then the slump: the smoothing heaps against the print's walls, which it leaves alone.
        self.slump(low, high);
        self.mark(low, high);
        pushed * self.cell * self.cell
    }

    /// The points within `reach` metres of `at` (x and z), clamped to the layer.
    fn span(&self, at: DVec2, reach: Vec2) -> ([u32; 2], [u32; 2]) {
        let g = (at - self.origin) / f64::from(self.cell);
        let r = (reach / self.cell).as_dvec2();
        let clamp = |v: f64, n: u32| v.clamp(0.0, f64::from(n - 1)) as u32;
        (
            [
                clamp((g.x - r.x).floor(), self.size[0]),
                clamp((g.y - r.y).floor(), self.size[1]),
            ],
            [
                clamp((g.x + r.x).ceil(), self.size[0]),
                clamp((g.y + r.y).ceil(), self.size[1]),
            ],
        )
    }

    fn index(&self, x: u32, z: u32) -> usize {
        (z * self.size[0] + x) as usize
    }

    /// Smooths the points from `low` to `high` but those of the print (`print`): each pass, a
    /// share of the difference between each point and its next along x and along z moves
    /// across, worked out from the heights before the pass, so no material is made or lost.
    fn smooth(&mut self, low: [u32; 2], high: [u32; 2], print: impl Fn(u32, u32) -> bool) {
        for _ in 0..SMOOTH_PASSES {
            for z in low[1]..=high[1] {
                let row = self.index(low[0], z);
                let end = self.index(high[0], z);
                self.scratch[row..=end].copy_from_slice(&self.heights[row..=end]);
            }
            for z in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    if print(x, z) {
                        continue;
                    }
                    let k = self.index(x, z);
                    for (next, inside) in [
                        (k + 1, x < high[0] && !print(x + 1, z)),
                        (k + self.size[0] as usize, z < high[1] && !print(x, z + 1)),
                    ] {
                        if inside {
                            let flow = SMOOTH_RATE * (self.scratch[k] - self.scratch[next]);
                            self.heights[k] -= flow;
                            self.heights[next] += flow;
                        }
                    }
                }
            }
        }
    }

    /// Slumps the points from `low` to `high`: each pass, between each point and its next
    /// along x and along z, a share of the drop past the material's repose moves down, every
    /// move worked out from the heights before the pass; until no move is over `SETTLED`.
    fn slump(&mut self, low: [u32; 2], high: [u32; 2]) {
        let limit = self.soft.repose * self.cell;
        for _ in 0..SLUMP_PASSES {
            let mut largest = 0.0f32;
            for z in low[1]..=high[1] {
                let row = self.index(low[0], z);
                let end = self.index(high[0], z);
                self.scratch[row..=end].copy_from_slice(&self.heights[row..=end]);
            }
            for z in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let k = self.index(x, z);
                    for (next, inside) in [
                        (k + 1, x < high[0]),
                        (k + self.size[0] as usize, z < high[1]),
                    ] {
                        if !inside {
                            continue;
                        }
                        let drop = self.scratch[k] - self.scratch[next];
                        let excess = drop.abs() - limit;
                        if excess > 0.0 {
                            let flow = SLUMP_RATE * excess * drop.signum();
                            self.heights[k] -= flow;
                            self.heights[next] += flow;
                            largest = largest.max(flow.abs());
                        }
                    }
                }
            }
            if largest <= SETTLED {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer 1 m square of 1 cm cells about the origin.
    fn bed(soft: Soft) -> Layer {
        Layer::new(soft, DVec2::new(-0.5, -0.5), 0.01, [101, 101])
    }

    /// A dog's paw: 3 cm across, 4 cm along, pointing +z, at 100 kPa.
    fn paw(at: DVec2) -> Pad {
        Pad {
            at,
            heading: Vec2::Y,
            size: Vec2::new(0.015, 0.02),
            pressure: 1.0e5,
            sweep: 0.0,
            wheel: 0.0,
        }
    }

    #[test]
    fn an_untouched_layer_is_its_depth_thinning_to_nothing_at_its_border() {
        let layer = bed(Soft::SNOW);
        assert_eq!(layer.height_at(DVec2::ZERO), Soft::SNOW.depth);
        assert_eq!(layer.height_at(DVec2::new(-0.5, 0.2)), 0.0);
        assert_eq!(
            layer.height_at(DVec2::new(0.47, 0.0)),
            Soft::SNOW.depth * 0.5
        );
        assert!(!layer.covers(DVec2::new(0.6, 0.0)));
    }

    #[test]
    fn a_paw_sinks_by_its_pressure_and_never_through_the_base() {
        // Sand: 100 kPa over 4 MPa/m, 2.5 cm, less what pours back in from the walls.
        let mut sand = bed(Soft::SAND);
        sand.press(paw(DVec2::ZERO));
        let sunk = Soft::SAND.depth - sand.height_at(DVec2::ZERO);
        assert!((0.014..=0.025).contains(&sunk), "{sunk}");
        // Snow: 50 cm deep under that pressure, so down to its least.
        let mut snow = bed(Soft::SNOW);
        snow.press(paw(DVec2::ZERO));
        assert!((snow.height_at(DVec2::ZERO) - Soft::SNOW.least).abs() < 1e-4);
        // Off the layer, nothing.
        assert_eq!(snow.press(paw(DVec2::new(2.0, 0.0))), 0.0);
    }

    #[test]
    fn sand_heaps_what_it_does_not_pack_and_snow_packs_most() {
        let mut rims = Vec::new();
        for soft in [Soft::SAND, Soft::SNOW, Soft::MUD] {
            let mut layer = bed(soft);
            let before = layer.volume();
            let pushed = layer.press(paw(DVec2::ZERO));
            assert!(pushed > 0.0);
            // What is lost is what was packed; the slump moves the rest about.
            let lost = before - layer.volume();
            let packed = f64::from(soft.packing * pushed);
            assert!(
                (lost - packed).abs() < 1e-3 * f64::from(pushed),
                "{lost} {packed}"
            );
            // The highest the ground round the print stands over the untouched layer.
            let rim = (0..64)
                .map(|k| {
                    let turn = f64::from(k) / 64.0 * std::f64::consts::TAU;
                    let at = DVec2::new(0.024 * turn.cos(), 0.032 * turn.sin());
                    layer.height_at(at) - soft.depth
                })
                .fold(f32::MIN, f32::max);
            rims.push(rim);
        }
        // Sand's rim stands a few millimetres; snow's is low for how deep its print is (5.6 cm
        // against 1.5).
        assert!(rims[0] > 0.002, "{rims:?}");
        assert!(rims[1] / 0.056 < 0.5 * rims[0] / 0.015, "{rims:?}");
        // Mud, which packs nearly nothing, heaps the highest rim.
        assert!(rims[2] > rims[0], "{rims:?}");
    }

    #[test]
    fn sand_slumps_to_its_repose_and_snow_holds_steeper_walls() {
        let steepest = |layer: &Layer| {
            let h = layer.heights();
            let n = layer.size()[0] as usize;
            let mut most = 0.0f32;
            for z in 30..70 {
                for x in 30..70 {
                    let k = z * n + x;
                    most = most
                        .max((h[k] - h[k + 1]).abs())
                        .max((h[k] - h[k + n]).abs());
                }
            }
            most / layer.cell()
        };
        let mut sand = bed(Soft::SAND);
        let mut snow = bed(Soft::SNOW);
        for k in 0..4 {
            let at = DVec2::new(0.0, -0.1 + 0.06 * f64::from(k));
            sand.press(paw(at));
            snow.press(paw(at));
        }
        let (sand_slope, snow_slope) = (steepest(&sand), steepest(&snow));
        assert!(sand_slope < 1.2 * Soft::SAND.repose, "{sand_slope}");
        assert!(snow_slope > 1.5 * sand_slope, "{snow_slope} {sand_slope}");
    }

    #[test]
    fn a_pad_pressing_again_where_it_stands_sinks_no_further_and_marks_what_changed() {
        // Deep soft mud: a wheel's 135 kPa sinks 13.5 cm into 15 cm.
        let deep = Soft {
            depth: 0.15,
            stiffness: 1.0e6,
            ..Soft::MUD
        };
        let mut layer = Layer::with_bevel(deep, DVec2::new(-0.5, -0.5), 0.01, [101, 101], 0.3);
        assert!(layer.take_changed().is_empty());
        let wheel = Pad {
            at: DVec2::ZERO,
            heading: Vec2::Y,
            size: Vec2::new(0.1, 0.07),
            pressure: 1.35e5,
            sweep: 0.0,
            wheel: 0.0,
        };
        layer.press(wheel);
        let once = layer.height_at(DVec2::ZERO);
        assert!((once - 0.015).abs() < 0.002, "{once}");
        let [(low, high)] = layer.take_changed()[..] else {
            panic!("one box changed");
        };
        assert!(
            low[0] < 40 && high[0] > 60 && low[1] < 40 && high[1] > 60,
            "{low:?} {high:?}"
        );
        for _ in 0..60 {
            layer.press(wheel);
        }
        assert!((layer.height_at(DVec2::ZERO) - once).abs() < 0.001);
        // Its gentle edge: 30 cm to its full depth.
        assert!(layer.height_at(DVec2::new(-0.35, 0.0)) < 0.5 * deep.depth);
        assert_eq!(layer.height_at(DVec2::new(-0.19, -0.19)), deep.depth);
    }

    #[test]
    fn a_rolling_wheel_ploughs_one_unbroken_rut_with_berms_beside_it() {
        // A tyre's patch, 20 by 14 cm at 150 kPa, rolling 5 cm a step along +z through mud.
        let mut mud = bed(Soft::MUD);
        for k in 0..12 {
            mud.press(Pad {
                at: DVec2::new(0.0, -0.25 + 0.05 * f64::from(k)),
                heading: Vec2::Y,
                size: Vec2::new(0.1, 0.07),
                pressure: 1.5e5,
                sweep: 0.05,
                wheel: 0.31,
            });
        }
        // Along its middle the rut keeps one depth: no ridge left between two steps.
        let along: Vec<f32> = (0..30)
            .map(|k| mud.height_at(DVec2::new(0.0, -0.2 + 0.01 * f64::from(k))))
            .collect();
        let (low, high) = along
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(high < Soft::MUD.depth - 0.02, "{along:?}");
        assert!(high - low < 0.004, "{along:?}");
        // Its berms stand beside it, over the untouched mud.
        let berm = (10..20)
            .map(|k| mud.height_at(DVec2::new(0.01 * f64::from(k), 0.0)))
            .fold(f32::MIN, f32::max);
        assert!(berm > Soft::MUD.depth + 0.005, "{berm}");
    }

    #[test]
    fn the_same_pads_leave_the_same_bits() {
        let pads: Vec<Pad> = (0..20)
            .map(|k| Pad {
                heading: Vec2::new(0.3, 1.0).normalize(),
                ..paw(DVec2::new(
                    0.013 * f64::from(k) - 0.1,
                    0.021 * f64::from(k) - 0.2,
                ))
            })
            .collect();
        let run = || {
            let mut layer = bed(Soft::SAND);
            for &pad in &pads {
                layer.press(pad);
            }
            layer
        };
        let (a, b) = (run(), run());
        assert!(
            a.heights()
                .iter()
                .zip(b.heights())
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
        // Back to saved heights.
        let mut c = bed(Soft::SAND);
        c.set_heights(a.heights());
        assert_eq!(c, a);
    }
}
