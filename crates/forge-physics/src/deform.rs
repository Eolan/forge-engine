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
    /// The tread it presses into the floor of its print, if any (#188): a tyre's.
    pub tread: Option<Tread>,
}

/// A tyre's tread as its print takes it (#188): what of the tyre stands out presses `depth`
/// deeper than its rut's floor. Laid where it rolled, fixed to the ground (a tyre that does not
/// slip leaves its tread where it touched), so a wheel's presses step after step, and the wheel
/// after it in the same rut, press the same tread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tread {
    /// A tractor's lugs across it, angled back from its middle in chevrons (#188).
    Lugs {
        /// How much deeper a lug presses, metres.
        depth: f32,
        /// Half the tread's width, metres: the lugs lie within it.
        half_width: f32,
        /// Metres from one lug to the next along the path.
        pitch: f32,
        /// How far back a lug's arm reaches for each metre from the tread's middle (the
        /// chevron).
        sweep_back: f32,
    },
    /// A road tyre's grooves along it (#193): its ribs press, its grooves leave lines standing
    /// down the rut, the same all along it (so a tyre spinning along its way leaves them too).
    Grooves {
        /// How much deeper a rib presses, metres.
        depth: f32,
        /// Half the tread's width, metres: the ribs and grooves lie within it.
        half_width: f32,
        /// How many grooves, evenly across it.
        count: u32,
        /// A groove's width, metres.
        width: f32,
    },
}

impl Tread {
    /// How deep the tread presses at a point `along` metres along the path (in the world: the
    /// point's position along the heading) and `across` metres from the tread's middle: full
    /// depth where it stands out, nothing between, with smooth sides so a coarse grid takes it
    /// without a step; 0 off the tread.
    pub fn depth_at(&self, along: f32, across: f32) -> f32 {
        let ease = |s: f32| {
            let s = s.clamp(0.0, 1.0);
            s * s * (3.0 - 2.0 * s)
        };
        match *self {
            Self::Lugs {
                depth,
                half_width,
                pitch,
                sweep_back,
            } => {
                if across.abs() > half_width {
                    return 0.0;
                }
                let phase = (along + sweep_back * across.abs()) / pitch;
                let t = phase - phase.floor();
                // A triangle over the pitch, eased: a lug half the pitch wide.
                let tri = 1.0 - (2.0 * t - 1.0).abs();
                depth * ease((tri - 0.3) / 0.4)
            }
            Self::Grooves {
                depth,
                half_width,
                count,
                width,
            } => {
                if across.abs() > half_width {
                    return 0.0;
                }
                if count == 0 {
                    return depth;
                }
                // The nearest groove's middle: `count` of them a spacing apart about the middle.
                let spacing = 2.0 * half_width / (count + 1) as f32;
                let first = -0.5 * (count as f32 - 1.0) * spacing;
                let k = ((across - first) / spacing)
                    .round()
                    .clamp(0.0, (count - 1) as f32);
                let off = (across - first - k * spacing).abs();
                // Nothing over its middle half, rising to the rib over a quarter of its width
                // either side of its edge.
                depth * ease((off - 0.25 * width) / (0.5 * width))
            }
        }
    }
}

/// What digs (#191): a wheel's patch spinning on the layer, its tread running faster than it
/// travels (or slower: a locked wheel sliding), which tears the material from under it and
/// throws it the way its tread slides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dig {
    /// The patch's middle, (x, z), metres.
    pub at: DVec2,
    /// The way the material goes, (x, z), unit: behind a wheel spinning forward.
    pub throw: Vec2,
    /// The patch's half sizes across and along `throw`, metres.
    pub size: Vec2,
    /// How much it tears from under the patch's middle, metres (never closer to the base than
    /// the material's least).
    pub depth: f32,
    /// How far past the patch's edge what it tears lands, metres: a heap highest a third of the
    /// way out, nothing past it.
    pub reach: f32,
    /// The radius of the wheel it is, metres: its hole's floor along `throw` is the wheel's
    /// round, deepest under the patch's middle, so the wheel climbs out of it gradually as from
    /// its trough (#189), not up the 45° walls a flat hole slumps to. 0: flat under the patch.
    pub wheel: f32,
}

/// How much wider than the patch the heap a dig throws spreads, as a share of its half width.
const SPREAD: f32 = 1.5;

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
const SMOOTH_PASSES: u32 = 5;
const SMOOTH_RATE: f32 = 0.12;
/// The smoothing keeps off the rim's inner edge (this share of its width next to the print):
/// heaped against the top of the print's wall, it steepened it, and the slump poured it into
/// the rut's foot behind the wheel, in a sawtooth a low sun dashed (#188).
const SMOOTH_EDGE: f32 = 0.2;
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
    /// What a tread pressed into the thickness's surface (#188), metres (0 or less), drawn with
    /// it but no part of it: the ground a wheel rolls on stays smooth, and the slump leaves it.
    relief: Vec<f32>,
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
            && self.relief == other.relief
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
            relief: vec![0.0; (size[0] * size[1]) as usize],
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

    /// What treads pressed into the surface at each point (0 or less), row by row along x
    /// (#188): drawn with the thickness, no part of it.
    pub fn relief(&self) -> &[f32] {
        &self.relief
    }

    /// Back to a saved relief ([`Layer::relief`]).
    ///
    /// # Panics
    ///
    /// When `relief` is not a value per point.
    pub fn set_relief(&mut self, relief: &[f32]) {
        self.relief.copy_from_slice(relief);
    }

    /// The surface as drawn, each point's thickness and relief, into `out` (cleared first).
    pub fn drawn(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend(self.heights.iter().zip(&self.relief).map(|(h, r)| h + r));
    }

    /// The surface as drawn under (`x`, `z`), as [`Layer::height_at`] its thickness.
    pub fn drawn_at(&self, at: DVec2) -> f32 {
        self.sample(&self.relief, at) + self.height_at(at)
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
        self.sample(&self.heights, at)
    }

    /// `values` (one a point) under (`x`, `z`), between the four nearest points.
    fn sample(&self, values: &[f32], at: DVec2) -> f32 {
        let g = ((at - self.origin) / f64::from(self.cell)).as_vec2();
        let max = Vec2::new((self.size[0] - 1) as f32, (self.size[1] - 1) as f32);
        let g = g.clamp(Vec2::ZERO, max);
        let g0 = g.floor().min(max - 1.0).max(Vec2::ZERO);
        let t = g - g0;
        let (x, z) = (g0.x as u32, g0.y as u32);
        let h = |dx: u32, dz: u32| {
            values[((z + dz).min(self.size[1] - 1) * self.size[0] + (x + dx).min(self.size[0] - 1))
                as usize]
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
                    // The tread's lugs into the relief, not the thickness (#188): the ground a
                    // wheel rolls on stays smooth (the lugs in it snagged the car's wheels) and
                    // the slump leaves them. Never under half the least: at the floor (snow,
                    // sand) a lug would cut through, the floor showing beneath in patches. A
                    // press without a tread wipes what one left.
                    let lugs = pad.tread.map_or(0.0, |t| {
                        let p = self.origin
                            + DVec2::new(f64::from(x), f64::from(z)) * f64::from(self.cell);
                        let d = (p - pad.at).as_vec2();
                        t.depth_at(p.dot(heading.as_dvec2()) as f32, d.dot(across))
                    });
                    self.relief[k] = -lugs.min((self.heights[k] - 0.5 * soft.least).max(0.0));
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
        // A wheel's rim, heaped a step at a time, smoothed where this press heaps it alone: the
        // rut beside it, its walls and the tread's lugs pressed into it (#188) stay as pressed
        // (smoothing all but this press's print wore down the lugs behind it). A foot's rims are
        // of one press, and stay as they are.
        if wheel > 0.0 {
            let width = (high[0] - low[0] + 1) as usize;
            let kept: Vec<bool> = (low[1]..=high[1])
                .flat_map(|z| (low[0]..=high[0]).map(move |x| (x, z)))
                .map(|(x, z)| {
                    let (r2, beside) = r2(x, z);
                    let t = (r2.sqrt() - 1.0) / rim_reach;
                    bump((r2, beside)) == 0.0 || t < SMOOTH_EDGE
                })
                .collect();
            self.smooth(low, high, |x, z| {
                kept[(z - low[1]) as usize * width + (x - low[0]) as usize]
            });
        }
        // Then the slump: the smoothing heaps against the rut's walls, which it leaves alone.
        self.slump(low, high);
        self.mark(low, high);
        pushed * self.cell * self.cell
    }

    /// Digs `dig` into the layer (#191): under its patch's middle the ground sinks by its depth
    /// (to the material's least at most), along it to the wheel's round from there (flat for no
    /// wheel), and all it tears is heaped beyond that the way it is thrown, spread a little wider
    /// than the patch; then the hole and the heap slump. The relief under the patch is left to the press that follows (a spinning
    /// tread smears its lugs). The volume it moved, m³ (none off the layer).
    pub fn dig(&mut self, dig: Dig) -> f32 {
        if !self.covers(dig.at) || dig.depth <= 0.0 {
            return 0.0;
        }
        let throw = dig.throw.normalize_or(Vec2::Y);
        let across = Vec2::new(-throw.y, throw.x);
        let (a, b) = (dig.size.x.max(1e-4), dig.size.y.max(1e-4));
        let reach = dig.reach.max(self.cell);
        // How far along it the hole reaches: the patch, or the wheel's round.
        let wheel = dig.wheel.max(0.0);
        let b = b.max(wheel);
        // Behind the patch as far as the heap lands, before it a cell; across, the heap's spread;
        // and a cell more for the slump. A box about the patch's middle, turned onto x and z.
        let along_reach = b + reach + 2.0 * self.cell;
        let across_reach = SPREAD * a + 2.0 * self.cell;
        let half = Vec2::new(
            throw.x.abs() * along_reach + throw.y.abs() * across_reach,
            throw.y.abs() * along_reach + throw.x.abs() * across_reach,
        );
        let (low, high) = self.span(dig.at, half);
        let local = |x: u32, z: u32| {
            let d = (self.origin + DVec2::new(f64::from(x), f64::from(z)) * f64::from(self.cell)
                - dig.at)
                .as_vec2();
            (d.dot(across), d.dot(throw))
        };
        // The heap: rising from the patch's edge, highest a third of the way out, thinning to
        // nothing `reach` beyond it (thrown clear of the hole, not slumping back into it); and
        // across, from its middle to its spread.
        let heap = |(u, v): (f32, f32)| {
            let t = (v - b) / reach;
            let s = u.abs() / (SPREAD * a);
            if (0.0..1.0).contains(&t) && s < 1.0 {
                t * (1.0 - t) * (1.0 - t) * (1.0 - s * s)
            } else {
                0.0
            }
        };
        let mut weights = 0.0f32;
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                weights += heap(local(x, z));
            }
        }
        // Nowhere on the layer for it to land (the patch at its border): it stays.
        if weights == 0.0 {
            return 0.0;
        }
        let least = self.soft.least;
        // The hole's floor: its depth under the ground at the patch's middle, and from there the
        // wheel's round along it.
        let floor = (self.height_at(dig.at) - dig.depth).max(least);
        let sole = |v: f32| {
            let off = v.abs().min(wheel);
            floor + wheel - (wheel * wheel - off * off).sqrt()
        };
        let mut torn = 0.0f32;
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let (u, v) = local(x, z);
                if u.abs() < a && v.abs() < b {
                    let k = self.index(x, z);
                    let take = (self.heights[k] - sole(v).max(least)).max(0.0);
                    self.heights[k] -= take;
                    torn += take;
                }
            }
        }
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let w = heap(local(x, z));
                if w > 0.0 {
                    let k = self.index(x, z);
                    self.heights[k] += torn * w / weights;
                }
            }
        }
        self.slump(low, high);
        self.mark(low, high);
        torn * self.cell * self.cell
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

    /// Smooths the points from `low` to `high` but those `kept` as they are: each pass, a
    /// share of the difference between each point and its next along x and along z moves
    /// across, worked out from the heights before the pass, so no material is made or lost.
    fn smooth(&mut self, low: [u32; 2], high: [u32; 2], kept: impl Fn(u32, u32) -> bool) {
        for _ in 0..SMOOTH_PASSES {
            for z in low[1]..=high[1] {
                let row = self.index(low[0], z);
                let end = self.index(high[0], z);
                self.scratch[row..=end].copy_from_slice(&self.heights[row..=end]);
            }
            for z in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    if kept(x, z) {
                        continue;
                    }
                    let k = self.index(x, z);
                    for (next, inside) in [
                        (k + 1, x < high[0] && !kept(x + 1, z)),
                        (k + self.size[0] as usize, z < high[1] && !kept(x, z + 1)),
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
            tread: None,
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
            tread: None,
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
                tread: None,
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
    fn a_treaded_wheel_leaves_its_lugs_a_pitch_apart_where_they_touched() {
        let tread = Tread::Lugs {
            depth: 0.006,
            half_width: 0.1,
            pitch: 0.08,
            sweep_back: 0.6,
        };
        let roll = |layer: &mut Layer| {
            for k in 0..14 {
                layer.press(Pad {
                    at: DVec2::new(0.0, -0.3 + 0.05 * f64::from(k)),
                    heading: Vec2::Y,
                    size: Vec2::new(0.12, 0.07),
                    pressure: 1.0e5,
                    sweep: 0.1,
                    wheel: 0.4,
                    tread: Some(tread),
                });
            }
        };
        let mut mud = bed(Soft::MUD);
        roll(&mut mud);
        // Along the rut's middle as drawn: lugs pressed 6 mm into its floor every 8 cm, the
        // ground under them smooth.
        let floor = |layer: &Layer| -> Vec<f32> {
            (0..32)
                .map(|k| layer.drawn_at(DVec2::new(0.0, -0.16 + 0.01 * f64::from(k))))
                .collect()
        };
        let along = floor(&mud);
        let (low, high) = along
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!((high - low - 0.006).abs() < 0.0015, "{along:?}");
        let deepest: Vec<usize> = (1..31)
            .filter(|&k| along[k] < along[k - 1] && along[k] <= along[k + 1])
            .collect();
        // A point either way where the rut starts on a slope.
        assert!(deepest.len() >= 3, "{deepest:?}");
        assert!(
            deepest.windows(2).all(|w| (7..=9).contains(&(w[1] - w[0]))),
            "{deepest:?}"
        );
        // The thickness under them, which a wheel rolls on, smooth.
        let ground: Vec<f32> = (0..32)
            .map(|k| mud.height_at(DVec2::new(0.0, -0.16 + 0.01 * f64::from(k))))
            .collect();
        let (gl, gh) = ground
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(gh - gl < 0.001, "{ground:?}");
        // A second pass in the same rut presses the same lugs.
        roll(&mut mud);
        let moved = along
            .iter()
            .zip(floor(&mud))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(moved < 0.001, "{moved}");
    }

    #[test]
    fn a_road_tyre_leaves_its_grooves_as_lines_down_its_rut() {
        // Three grooves 2.5 cm wide across 20 cm: their middles 5 cm apart.
        let tread = Tread::Grooves {
            depth: 0.005,
            half_width: 0.1,
            count: 3,
            width: 0.025,
        };
        assert_eq!(tread.depth_at(0.0, 0.0), 0.0);
        assert_eq!(tread.depth_at(0.0, 0.05), 0.0);
        assert_eq!(tread.depth_at(0.0, 0.025), 0.005);
        assert_eq!(tread.depth_at(0.0, 0.09), 0.005);
        assert_eq!(tread.depth_at(0.0, 0.11), 0.0);
        let mut mud = bed(Soft::MUD);
        for k in 0..14 {
            mud.press(Pad {
                at: DVec2::new(0.0, -0.3 + 0.05 * f64::from(k)),
                heading: Vec2::Y,
                size: Vec2::new(0.12, 0.07),
                pressure: 1.0e5,
                sweep: 0.1,
                wheel: 0.4,
                tread: Some(tread),
            });
        }
        // Across the rut as drawn: lines standing over the ribs' floor where the grooves were.
        let across = |z: f64| -> Vec<f32> {
            (-10..=10)
                .map(|k| mud.drawn_at(DVec2::new(0.01 * f64::from(k), z)))
                .collect()
        };
        let line = across(0.0);
        // The grooves at −5, 0 and 5 cm (points 5, 10, 15), the ribs 3 cm beside them.
        for (groove, rib) in [(5, 8), (10, 7), (15, 12)] {
            assert!(line[groove] - line[rib] > 0.004, "{line:?}");
        }
        // The same all along it.
        let other = across(0.13);
        let most = line
            .iter()
            .zip(&other)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(most < 0.001, "{most}");
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

    #[test]
    fn a_spinning_wheel_digs_its_hole_and_throws_it_behind() {
        // A wheel standing in deep mud, spinning forward (+z) for a second: 2 mm torn a step
        // from under its 20 cm by 14 cm patch, thrown back over 40 cm.
        let mud = Soft {
            depth: 0.15,
            stiffness: 1.0e6,
            repose: 0.85,
            ..Soft::MUD
        };
        let spin = Dig {
            at: DVec2::ZERO,
            throw: Vec2::NEG_Y,
            size: Vec2::new(0.1, 0.07),
            depth: 0.002,
            reach: 0.4,
            wheel: 0.0,
        };
        let mut layer = bed(mud);
        let before = layer.volume();
        let mut moved = 0.0;
        for _ in 0..60 {
            moved += layer.dig(spin);
        }
        assert!(moved > 0.0);
        // Nothing made or lost: it only moved.
        assert!(
            (layer.volume() - before).abs() < 1e-6,
            "{} {before}",
            layer.volume()
        );
        // A hole under it, deeper the longer it spins, its walls slumping in, never through
        // the base.
        let hole = layer.height_at(DVec2::ZERO);
        assert!(hole < mud.depth - 0.05, "{hole}");
        assert!(hole >= mud.least - 1e-6, "{hole}");
        // The heap behind it, none before it.
        let behind = (10..40)
            .map(|k| layer.height_at(DVec2::new(0.0, -0.01 * f64::from(k))))
            .fold(f32::MIN, f32::max);
        assert!(behind > mud.depth + 0.02, "{behind}");
        let ahead = (10..40)
            .map(|k| layer.height_at(DVec2::new(0.0, 0.01 * f64::from(k))))
            .fold(f32::MIN, f32::max);
        assert!(ahead <= mud.depth + 1e-4, "{ahead}");
        // Off the layer, or not digging, nothing.
        let mut still = bed(mud);
        assert_eq!(still.dig(Dig { depth: 0.0, ..spin }), 0.0);
        assert_eq!(
            still.dig(Dig {
                at: DVec2::new(2.0, 0.0),
                ..spin
            }),
            0.0
        );
        assert_eq!(still, bed(mud));
        // A wheel's (40 cm round): its hole's floor is its round, gentle under its middle, so it
        // can roll out; a flat hole's ends slump to the mud's slope.
        let mut round = bed(mud);
        for _ in 0..30 {
            round.dig(Dig { wheel: 0.4, ..spin });
        }
        let floor = |l: &Layer, v: f64| l.height_at(DVec2::new(0.0, v));
        let rise = floor(&round, 0.06) - floor(&round, 0.0);
        assert!(rise > 0.0 && rise / 0.06 < 0.25, "{rise} over 6 cm");
        let mut flat = bed(mud);
        for _ in 0..30 {
            flat.dig(spin);
        }
        let wall = (floor(&flat, 0.1) - floor(&flat, 0.08)) / 0.02;
        assert!(wall > 0.5, "{wall}");
    }
}
