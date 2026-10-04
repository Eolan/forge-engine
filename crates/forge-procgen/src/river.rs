//! The rivers' water (issue #105, D-038's rivers; `docs/research/water.md` §3 and its
//! recommendation's step 3): each river of stage 4 ([`crate::hydrology`]) as a ribbon of
//! points the GPU draws as a level surface in the channel [`crate::channel`] carves for it.
//! - The D8 course runs from cell centre to cell centre in 45° turns. A binomial filter and
//!   Chaikin's corner cutting (1974) smooth it, its ends kept, and it is resampled every
//!   [`RibbonParams::step`] metres.
//! - The width is [`hydrology::width`] of the catchment; the depth `0.4 (A / km²)^⅜` m (the
//!   downstream hydraulic geometry of Leopold & Maddock 1953, `w ∝ Q^0.5` and `d ∝ Q^0.4`, the
//!   exponent taken as ⅜); the speed Chézy's `C √(d S)` over the water surface's slope `S`.
//!   The island sizes them by D-041's regional curves instead ([`RibbonParams::regional`]),
//!   the small ones brooks of nature's size ([`RibbonParams::brooks`], #123).
//! - The water is level across. Its level at a point is the lowest the ground stands there, in
//!   the middle and on either bank ([`smooth_height`]: the field's samples through a cubic),
//!   less a freeboard; then the running minimum from the head, so it only falls, and a fall
//!   steeper than [`RibbonParams::max_fall`] is spread upstream. Where a lake's water stands
//!   over the ground it is the lake's level, never under it upstream, and it never goes below
//!   the sea's.
//! - A tributary's water ends at its river's level, and it gives way to that river's water
//!   where it enters its channel. The corners either side are rounded ([`Corner`], #119), their
//!   water drawn by the nearer river ([`RibbonPoint::cover`]).
//! - A river gives way to the sea where its level reaches the sea's, and to a lake inside its
//!   water: a river running in fades out over the lake's water, one running out fades in over
//!   it, so the two meet wherever the lake's edge lies (#120). Running in, it has a delta
//!   ([`DeltaParams`]): its water eases flat to the lake's level and it widens over its last
//!   reach, and its sand builds a fan on the lake's floor in front of its mouth ([`Delta`]).
//! - In a bend the ribbon's half width stays under a share of the bend's radius, so its inner
//!   edge never folds over itself.
//! - On the steep reaches ([`StepParams`], #122) the water stands in pools and falls from one
//!   to the next over a step, at the level it had at each step's lip; its banks keep rising
//!   from that level as it was ([`RibbonPoint::unstepped`]).
//!
//! Everything is `f64` arithmetic with `sqrt` only, in the rivers' order (D-016).

use forge_core::hash::{hash_cell3, unit_f32};
use forge_task::TaskPool;

use crate::field::Field2;
use crate::hydrology::{self, Mouth, Rivers};
use crate::lake::LakeWater;

/// Quads across a ribbon (`RIVER_ACROSS` in `water.slang`).
pub const ACROSS: usize = 4;

/// Metres a river's water stands over a lake's level where it runs in it, so it draws over the
/// lake's water as it fades (the same depth would fight).
const LAKE_LIFT: f64 = 0.01;

/// Points over which a river's water fades into a lake's.
const LAKE_FADE: f64 = 3.0;

/// How steeply a river's water may fall out of a lake past its lip, m/m: a short ramp, not a
/// step under the lake's water.
const LAKE_OUTFALL: f64 = 0.05;

/// The share of a river's depth a lake's water must stand over the ground to take the river on:
/// in shallower water, a flooded flat, the river runs on in its channel at the lake's level.
const LAKE_TAKES: f64 = 0.5;

/// A river's outlet from a lake as the levels see it: the point, the river's direction there
/// (unit) and its half width, metres.
type FromLake = ([f64; 2], [f64; 2], f64);

/// Metres between the samples of the ground under a quad of a ribbon.
const GROUND_SAMPLES: f64 = 0.5;

/// The seed of the steps' spacings (#122).
const STEP_SEED: u64 = 0x5745_5053_504f_4f4c;

/// The seed of the deltas' fans' outlines (#120).
const DELTA_SEED: u64 = 0x4445_4c54_4146_414e;

/// The seed of the mouths' bars (#127).
const BAR_SEED: u64 = 0x4241_5253_4d4f_5554;

/// Steps and pools on a river's steep reaches (#122, D-041's type A): where its water falls
/// faster than `from`, it stands in pools and drops from each into the next over a step, as a
/// mountain stream does. Montgomery & Buffington (1997) find step-pools from 3 % and cascades
/// over 6.5 %, their pools half a width to four widths apart, closer as the slope steepens;
/// Abrahams, Li & Atkinson (1995) a step's height over its spacing one to two times the slope
/// (H/L/S), the pools' scour making up the difference (`docs/research/rivers.md` §4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepParams {
    /// The water surface's slope over which a reach runs in steps and pools, m/m; it runs on in
    /// them down to three quarters of it.
    pub from: f64,
    /// The steps' spacing in the river's widths: `.0` at `from`, `.1` at `steep` and over.
    pub spacing: (f64, f64),
    /// The slope at which the steps are closest, m/m.
    pub steep: f64,
    /// The least spacing before its jitter, metres.
    pub least: f64,
    /// The highest a step drops, metres: on the steepest reaches the steps come closer.
    pub highest: f64,
    /// How much a spacing varies either way, a share of it.
    pub jitter: f64,
    /// H/L/S: `.0` at `from`, `.1` at `steep` and over (1 would be a plain staircase, no scour).
    pub scour: (f64, f64),
    /// The share of its depth the water keeps over a lip.
    pub lip: f64,
    /// A fall's length along the river: `.0` metres a metre it drops, `.1` metres at least.
    pub fall: (f64, f64),
    /// How far a step's line bows downstream at most, shares of the river's half width: in its
    /// middle (`.0`, an arch) and towards one bank (`.1`, a slant); never upstream, nor over
    /// 1.5 m or three fifths of the pool below.
    pub bow: (f64, f64),
    /// The share of a pool's length its foam reaches down from the fall.
    pub foam: f64,
    /// The pools' speed, a share of the speed the reach's slope gives.
    pub pool_speed: f64,
}

impl Default for StepParams {
    /// From 4 % (D-041's type A), steps a width apart there and 0.4 of one from 15 %, 3 m at
    /// least, half again or half as long (the island's widths are three times nature's: 0.6 to
    /// 4.5 natural widths), 2 m high at most, their lines bowing downstream by up to two fifths
    /// of the half width in the middle and slanting by three fifths; H/L/S 1.8 at 4 %, 1.3 from
    /// 15 % (Abrahams et al.'s one to two);
    /// the water over a lip two fifths of its depth; a fall 0.4 m long a metre it drops and
    /// 0.3 m at least; its foam over a third of the pool, whose water runs at half the reach's
    /// speed.
    fn default() -> Self {
        Self {
            from: 0.04,
            spacing: (1.0, 0.4),
            steep: 0.15,
            least: 3.0,
            highest: 2.0,
            jitter: 0.5,
            scour: (1.8, 1.3),
            lip: 0.4,
            fall: (0.4, 0.3),
            bow: (0.4, 0.6),
            foam: 0.35,
            pool_speed: 0.5,
        }
    }
}

/// A river's delta where it runs into a lake (D-041's lake entry, #120): over its last reach
/// before the lake its water eases flat to the lake's level and its channel widens, and in front
/// of its mouth its sediment builds a fan on the lake's floor ([`Delta`]), a shallow top just
/// under the water that drops off at its front into the lake's depth (`docs/research/rivers.md`,
/// its recommendation's step 7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaParams {
    /// Over how many metres, plus how many of its widths, before the lake's edge the river's
    /// water eases flat to the lake's level and its channel widens.
    pub reach: (f64, f64),
    /// How much wider the river is at the lake's edge (1: twice as wide), growing over the
    /// reach's last stretch as a trumpet does.
    pub flare: f64,
    /// The share of its depth it loses there.
    pub shallow: f64,
    /// The fan's length in front of the mouth, `a + b ×` the river's width there, metres; at
    /// most `.2` of the lake's water ahead of it.
    pub fan: (f64, f64, f64),
    /// The water over the fan's top at its apex, metres, and how much deeper it is at the top's
    /// far end, whatever the fan's length.
    pub top: (f64, f64),
    /// The fan's front, its slope down to the lake's floor, m/m.
    pub front: f64,
    /// How far the fan's outline wanders, a share of its half length.
    pub wander: f64,
}

impl Default for DeltaParams {
    /// Over 8 m and five widths (D-041's five to ten), twice as wide at the lake and two fifths
    /// shallower, as the estuaries; a fan 6 m and three and a half widths long, at most three
    /// fifths of the lake ahead, its top 0.3 m under the water at the mouth and 1.1 m at its far
    /// end, so its sand fades into the lake's colour, its front falling at 0.3 (17°), its
    /// outline wandering by a fifth.
    fn default() -> Self {
        Self {
            reach: (8.0, 5.0),
            flare: 1.0,
            shallow: 0.4,
            fan: (6.0, 3.5, 0.6),
            top: (0.3, 0.8),
            front: 0.3,
            wander: 0.2,
        }
    }
}

/// Where a river runs into a lake (D-041's lake entry, #120): the fan its sediment builds on the
/// lake's floor in front of its mouth. Its top is a lobe from the mouth, the disc whose diameter
/// is its length along the river's direction, joined to a disc of the river's half width round
/// the mouth, its outline wandering; the water stands [`Delta::top`] over it, deeper away from
/// the mouth. Past the top's outline its front falls at [`Delta::front`] until it meets the
/// lake's floor. It only ever raises the ground, under the lake's water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delta {
    /// The mouth: where the lake's water starts to stand over the river's course, metres in the
    /// field's frame.
    pub apex: [f64; 2],
    /// The river's direction there, unit: the fan's axis.
    pub down: [f64; 2],
    /// The lake's level, metres.
    pub level: f64,
    /// The lowest the lake's floor goes, metres: the front reaches no deeper.
    pub floor: f64,
    /// The river's half width at the mouth, metres.
    pub half_width: f64,
    /// The fan's length along its axis, metres.
    pub length: f64,
    /// The water over its top at the mouth, metres, and how much deeper it is at the top's far
    /// end ([`Delta::length`] from the mouth).
    pub top: (f64, f64),
    /// Its front's slope, m/m.
    pub front: f64,
    /// How far its outline wanders, metres.
    pub wander: f64,
    /// The wander's seed.
    pub seed: u64,
}

impl Delta {
    /// How far outside the fan's top `q` is, metres (negative inside), its outline wandering.
    pub fn outside(&self, q: [f64; 2]) -> f64 {
        let half = 0.5 * self.length;
        let centre = [
            self.apex[0] + self.down[0] * half,
            self.apex[1] + self.down[1] * half,
        ];
        let lobe = (q[0] - centre[0]).hypot(q[1] - centre[1]) - half;
        let mouth = (q[0] - self.apex[0]).hypot(q[1] - self.apex[1]) - self.half_width;
        // A single octave over the fan's half length: a few broad lobes, not a ragged edge.
        let scale = half.max(1.0);
        lobe.min(mouth)
            + self.wander * crate::noise::fbm(self.seed, q[0] / scale, q[1] / scale, 1, 2.0, 0.5)
    }

    /// The fan's surface at `q`, metres, and the share of it kept (it fades in over the river's
    /// half width behind the mouth, into its channel), or none where it is under the lake's floor
    /// or behind the mouth.
    pub fn surface(&self, q: [f64; 2]) -> Option<(f64, f64)> {
        let (dx, dy) = (q[0] - self.apex[0], q[1] - self.apex[1]);
        let along = dx * self.down[0] + dy * self.down[1];
        if along <= -self.half_width || dx.hypot(dy) > self.reach() {
            return None;
        }
        let r = dx.hypot(dy) / self.length.max(1e-3);
        let z = self.level - self.top.0 - self.top.1 * r - self.front * self.outside(q).max(0.0);
        (z > self.floor).then(|| (z, smoothstep(-self.half_width, 0.0, along)))
    }

    /// How far from its apex the fan may raise the ground, metres: its length, the wander, and
    /// as far as its front can fall to the lake's floor.
    pub fn reach(&self) -> f64 {
        self.length.max(self.half_width)
            + self.wander
            + (self.level - self.floor).max(0.0) / self.front.max(1e-3)
    }
}

/// The bars in a large river's mouth at the sea (D-041's mouths: "distributaries split around
/// bars where the catchment is large", #127). Over its last reach before the sea one bar of sand,
/// or two side by side, stand a little over the water in its widened channel, and the water runs
/// round them in two or three channels to the sea; the river widens by the bars' breadth there,
/// so each channel keeps its share of the water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarParams {
    /// Metres of the river's width at its mouth per bar: a river this wide or more has one, twice
    /// as wide two.
    pub per: f64,
    /// The most bars in a mouth.
    pub most: u32,
    /// A bar's length, in the river's widths at the mouth.
    pub length: f64,
    /// A bar's half breadth at its middle, a share of the river's width at the mouth.
    pub breadth: f64,
    /// How far up the river from the mouth a bar's downstream tip stands, in its widths.
    pub gap: f64,
    /// Metres the bar's crest stands over the water.
    pub top: f64,
    /// Its slopes, m/m: out of the water to its crest, and under the water down to the bed.
    pub slopes: (f64, f64),
    /// How far its outline wanders, a share of its half breadth.
    pub wander: f64,
}

impl Default for BarParams {
    /// A bar per 20 m of the mouth's width, two at most, each two and a half widths long and
    /// three tenths of the width broad, its tip a third of a width up from the mouth, its crest
    /// 0.3 m over the water on slopes of 1 in 12 out of it and 1 in 3 under it, its outline
    /// wandering by a quarter of its half breadth.
    fn default() -> Self {
        Self {
            per: 20.0,
            most: 2,
            length: 2.5,
            breadth: 0.15,
            gap: 1.0 / 3.0,
            top: 0.3,
            slopes: (1.0 / 12.0, 1.0 / 3.0),
            wander: 0.25,
        }
    }
}

/// The share of a bar's half length its blunt head takes upstream of its widest ([`Bar`]); its
/// tail takes the rest, `2 −` this.
const BAR_HEAD: f64 = 0.6;

/// The power along a bar's tail: under 2 it tapers to a point rather than rounding off.
const BAR_TAIL: f64 = 1.4;

/// A bar of sand in a river's mouth at the sea ([`BarParams`], #127): a teardrop along the river,
/// a blunt head upstream and a long tail tapering downstream, its outline wandering, where the
/// water's edge lies; inside it the sand rises to its crest over
/// the water, outside it falls under the water to the channel's bed. It only ever raises the
/// ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// Its widest point, between its head and its tail, metres in the field's frame.
    pub centre: [f64; 2],
    /// Its long axis, downstream, unit: the river's direction there, turned a little.
    pub down: [f64; 2],
    /// Its half length and half breadth, metres.
    pub half: [f64; 2],
    /// The water's level at its upstream tip and at its downstream one, metres.
    pub level: [f64; 2],
    /// Metres its crest stands over the water.
    pub top: f64,
    /// Its slopes out of the water and under it, m/m.
    pub slopes: (f64, f64),
    /// How far its outline wanders, metres.
    pub wander: f64,
    /// The wander's seed.
    pub seed: u64,
}

impl Bar {
    /// `q` along the bar's axis from its middle, and across it (positive to its left seen
    /// downstream), metres.
    fn local(&self, q: [f64; 2]) -> (f64, f64) {
        let (dx, dy) = (q[0] - self.centre[0], q[1] - self.centre[1]);
        (
            dx * self.down[0] + dy * self.down[1],
            dx * -self.down[1] + dy * self.down[0],
        )
    }

    /// How far outside the bar's outline `q` is, metres (negative inside), its outline wandering:
    /// the teardrop's implicit distance to first order. Upstream of its widest it is a half
    /// ellipse [`BAR_HEAD`] of its half length long, a blunt head; downstream a tail the rest of
    /// its length, tapering to a point ([`BAR_TAIL`]'s power along it).
    pub fn outside(&self, q: [f64; 2]) -> f64 {
        let (u, v) = self.local(q);
        let (a, p) = if u < 0.0 {
            (self.half[0] * BAR_HEAD, 2.0)
        } else {
            (self.half[0] * (2.0 - BAR_HEAD), BAR_TAIL)
        };
        let (a, b) = (a.max(1e-3), self.half[1].max(1e-3));
        let along = u.abs() / a;
        let k = (along.powf(p) + (v / b) * (v / b)).sqrt();
        let gradient = if k > 1e-9 {
            (0.5 * p * along.powf(p - 1.0) / a).hypot(v.abs() / (b * b)) / k
        } else {
            1.0 / b
        };
        // One octave over a third of its half length: a few long bays and spits.
        let scale = (self.half[0] / 3.0).max(1.0);
        (k - 1.0) / gradient.max(1e-9)
            + self.wander * crate::noise::fbm(self.seed, q[0] / scale, q[1] / scale, 1, 2.0, 0.5)
    }

    /// The water's level at `q`, metres: from the upstream tip's to the downstream one's along
    /// the bar.
    pub fn level_at(&self, q: [f64; 2]) -> f64 {
        let (u, _) = self.local(q);
        let t = ((u + BAR_HEAD * self.half[0]) / (2.0 * self.half[0]).max(1e-3)).clamp(0.0, 1.0);
        self.level[0] + (self.level[1] - self.level[0]) * t
    }

    /// The bar's surface at `q`, metres: its crest over the water inside, its flank under it
    /// outside, as far as [`Bar::reach`].
    pub fn surface(&self, q: [f64; 2]) -> f64 {
        let e = self.outside(q);
        let level = self.level_at(q);
        if e < 0.0 {
            level + (self.slopes.0 * -e).min(self.top)
        } else {
            level - self.slopes.1 * e
        }
    }

    /// How far from its middle the bar may raise the ground, metres: its half length, the wander
    /// and its flank down a channel's depth of `deepest` metres.
    pub fn reach(&self, deepest: f64) -> f64 {
        (2.0 - BAR_HEAD) * self.half[0] + self.wander + deepest / self.slopes.1.max(1e-3)
    }
}

/// Metres between the samples across a ribbon point that [`bar_spans`] looks for the bars at.
const BAR_SPAN_STEP: f64 = 0.25;

/// Where the first two of a ribbon's [`Bar`]s stand over its water across each of its points:
/// from and to, metres from the middle along `(−direction.y, direction.x)` (positive to the
/// left seen downstream), the span inside each bar's outline over the ribbon's width. Where a
/// bar does not cross a point, both ends lie where its widest point projects onto the line
/// across, clamped to the ribbon, so a span grows from nothing between points. Far away the
/// water lies on the ground over the ribbon's whole width, and these keep it off the bars' sand.
pub fn bar_spans(ribbon: &Ribbon) -> Vec<[f32; 4]> {
    ribbon
        .points
        .iter()
        .map(|p| {
            let at = position(p);
            let side = [-f64::from(p.direction[1]), f64::from(p.direction[0])];
            let reach = f64::from(p.reach);
            let mut out = [0.0f32; 4];
            for (slot, bar) in ribbon.bars.iter().take(2).enumerate() {
                let (dx, dy) = (bar.centre[0] - at[0], bar.centre[1] - at[1]);
                let middle = (dx * side[0] + dy * side[1]).clamp(-reach, reach);
                let mut span = [middle; 2];
                if dx.hypot(dy) < bar.reach(0.0) + reach {
                    let steps = (2.0 * reach / BAR_SPAN_STEP).ceil() as i32;
                    let mut inside: Option<[f64; 2]> = None;
                    for s in 0..=steps {
                        let across = -reach + f64::from(s) * BAR_SPAN_STEP;
                        let q = [at[0] + side[0] * across, at[1] + side[1] * across];
                        if bar.outside(q) < 0.0 {
                            inside = Some(inside.map_or([across; 2], |i| [i[0], across]));
                        }
                    }
                    if let Some(i) = inside {
                        span = i;
                    }
                }
                out[2 * slot] = span[0] as f32;
                out[2 * slot + 1] = span[1] as f32;
            }
            out
        })
        .collect()
}

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
    /// Over how many metres, plus how many of its widths, a river's water rises to its banks
    /// into a lake and out of it, where the lake's shore holds it (its freeboard gone at the
    /// lake's edge), as its channel shoals there (`ChannelParams::shoal`).
    pub shoal: (f64, f64),
    /// The smallest lake, m², a river runs into (the least area of the lakes' water,
    /// [`crate::lake_waters`], whose level the rivers take).
    pub lake_area: f64,
    /// The estuary (D-041): the metres over the sea under which a river widens towards its
    /// mouth, and by how many of its widths at the sea's level (1: twice as wide), shallowing by
    /// two fifths, so it crosses the beach as a river mouth rather than a canal.
    pub estuary: (f64, f64),
    /// D-041's regional curves with their exaggeration `(k, k_d)`: a river `k · 2.7 (A/km²)^0.37`
    /// metres wide and `k_d · 0.3 (A/km²)^0.21` deep, the width growing downstream at nature's
    /// rate; `None` keeps [`hydrology::width`] and [`depth`].
    pub regional: Option<(f64, f64)>,
    /// The brooks (D-041's scale, #123): under `to` m² of catchment the regional curves'
    /// exaggeration eases down to nature's (`k = k_d = 1` at `from` m² and under), smoothly in
    /// the area's logarithm, so the few large rivers read as rivers and the many small ones as
    /// brooks; `None` exaggerates every river alike.
    pub brooks: Option<(f64, f64)>,
    /// How far along each edge the corners where a tributary meets its river are rounded
    /// ([`Corner`], #119): `a + b ×` the tributary's width, metres.
    pub confluence: (f64, f64),
    /// Steps and pools on the steep reaches (#122); `None` lets the water fall evenly.
    pub steps: Option<StepParams>,
    /// The deltas where rivers run into lakes (#120); `None` runs them in as they come.
    pub delta: Option<DeltaParams>,
    /// The bars in the large rivers' mouths at the sea (#127); `None` leaves them one channel.
    pub bars: Option<BarParams>,
    /// How much deeper a river runs below where a tributary joins it (#119's polish): the share
    /// of its depth added at the deepest, a width downstream, for a tributary as wide as it (less
    /// as the tributary is narrower), easing back over the next two widths; `None` leaves the
    /// bed as it was.
    pub confluence_scour: Option<f64>,
}

impl Default for RibbonParams {
    /// Points 4 m apart, four passes of the filter and three of corner cutting, four settling
    /// passes looking 8 m either side, 40 m of growth from the spring, a stream's roughness
    /// (C = 15), 0.3 to 3 m/s, the drawn half width under 0.8 of a bend's radius, 0.5 m + 10 %
    /// of the half width under the banks, the water 0.05 m + 4 % of the width under them, a fall
    /// of 60 % at most, rising to its banks over 8 m and three widths into a lake, the lakes of a
    /// hectare, twice as wide at the sea from 1.5 m over it, a confluence's corners rounded over
    /// 2 m and a tributary's width along each edge, no steps, no deltas, no bars, no scour.
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
            shoal: (8.0, 3.0),
            lake_area: 10_000.0,
            estuary: (1.5, 1.0),
            regional: None,
            brooks: None,
            confluence: (2.0, 1.0),
            steps: None,
            delta: None,
            bars: None,
            confluence_scour: None,
        }
    }
}

impl RibbonParams {
    /// The island's rivers (D-041): the defaults, sized by the regional curves three times as
    /// wide and one and a half times as deep as nature's (the owner's pick of `k`, 2026-10-01)
    /// from 3 km² of catchment, brooks of nature's size at 0.5 km² (#123), in steps and pools
    /// on their steep reaches, a delta where they run into a lake (#120), bars in their large
    /// mouths at the sea (#127), and running 60 % deeper below where a tributary as wide joins.
    pub fn island() -> Self {
        Self {
            regional: Some((3.0, 1.5)),
            brooks: Some((500_000.0, 3_000_000.0)),
            steps: Some(StepParams::default()),
            delta: Some(DeltaParams::default()),
            bars: Some(BarParams::default()),
            confluence_scour: Some(0.6),
            ..Self::default()
        }
    }
}

/// A river's width and depth with `area_m2` of catchment under `params` (D-041's regional
/// curves, or [`hydrology::width`] and [`depth`]), metres.
fn size(area_m2: f64, params: &RibbonParams) -> (f64, f64) {
    match params.regional {
        Some((k, k_d)) => {
            let km2 = (area_m2 * 1e-6).max(1e-6);
            let (k, k_d) = match params.brooks {
                Some((from, to)) => {
                    let ln = forge_core::dmath::ln::<f64>;
                    let full = smoothstep(ln(from), ln(to), ln(area_m2.max(1.0)));
                    (1.0 + (k - 1.0) * full, 1.0 + (k_d - 1.0) * full)
                }
                None => (k, k_d),
            };
            (
                k * 2.7 * forge_core::dmath::powf(km2, 0.37),
                k_d * 0.3 * forge_core::dmath::powf(km2, 0.21),
            )
        }
        None => (hydrology::width(area_m2), depth(area_m2)),
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
    /// Half the width over which its water is drawn whole, metres: the half width, more where it
    /// fills a confluence's rounded corner ([`Corner`]).
    pub cover: f32,
    /// Half the ribbon's width, metres: past the water's edge, under the banks.
    pub reach: f32,
    /// The water's depth in the middle, metres.
    pub depth: f32,
    /// The lowest the ground stands on the banks here before the channel is carved, metres.
    pub bank: f32,
    /// The water's speed, m/s.
    pub speed: f32,
    /// The water surface's slope downstream: none in a pool, steep over a step's fall.
    pub slope: f32,
    /// The slope of the reach, before its steps (#122): the valley's, which its stones follow.
    pub grade: f32,
    /// The water's level before the steps, metres (its level off them): its banks rise from it.
    pub unstepped: f32,
    /// The white water a step's fall leaves, 0..1: from its lip down, fading across the pool.
    pub foam: f32,
    /// The point's index before the steps, which the draws along the river hash (`u32::MAX` for
    /// a point a step put in), so the rest of the river keeps its stones.
    pub key: u32,
    /// At a step's four points (#122), how far its line bows downstream: in the river's middle,
    /// and towards its left bank (seen downstream; negative, its right), metres ([`lip_shift`]);
    /// none elsewhere.
    pub lip: [f32; 2],
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
    /// The runs of its points in a lake, the first and the last of each: where the lake's water
    /// stands over the ground half as deep as the river or more and takes the river on (the
    /// first is where it runs in, the point past the last where it runs out).
    pub lake_runs: Vec<[u32; 2]>,
    /// The rounded corners where it joins its river, either side of it: none for a river that
    /// ends in the sea or a lake, or meets its river along it.
    pub corners: Vec<Corner>,
    /// The steps of its steep reaches, head first (#122).
    pub steps: Vec<Step>,
    /// Its deltas where it runs into a lake, head first (#120).
    pub deltas: Vec<Delta>,
    /// Where it leaves a lake, head first (#120).
    pub outlets: Vec<Outlet>,
    /// The bars in its mouth at the sea (#127).
    pub bars: Vec<Bar>,
}

/// Where a river leaves a lake (#120): the last point of a run in it, where the lake's water still
/// stands half as deep as the river, past which the river carries the water on. Past it the lake's
/// shallow arm along the river is the river's: its water is trimmed off the arm
/// ([`crate::trim_outlets`]) and the ground there rises over the lake's level, a sill the river's
/// channel is cut through ([`crate::Channels`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outlet {
    /// The lake, its index in the lakes' water the ribbons were made with.
    pub lake: u32,
    /// The point, metres in the field's frame.
    pub at: [f64; 2],
    /// The river's direction there, unit.
    pub down: [f64; 2],
    /// The lake's level, metres.
    pub level: f64,
    /// The river's half width there, metres.
    pub half_width: f64,
}

/// A step of a steep reach (#122): where the water falls from one pool into the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// The point at its lip, where the pool above ends.
    pub lip: u32,
    /// The point at the foot of its fall, where the pool below starts.
    pub foot: u32,
    /// Metres it drops.
    pub drop: f64,
    /// Metres along the river to the next step's lip, or to the end of the reach.
    pub spacing: f64,
    /// The river's width at its lip, metres.
    pub width: f64,
}

/// A rounded corner where a tributary's water meets its river's (#119): the circle touching both
/// edges of the land between them, whose arc the water's edge follows instead of the corner the
/// two edges made. Between the arc and that corner the water stands shallow over a bed that
/// blends into the rivers', and past the arc the bank rises.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// The circle's centre, metres in the field's frame.
    pub centre: [f64; 2],
    /// Its radius, metres.
    pub radius: f64,
    /// Where it touches the tributary's water's edge, and the river's.
    pub touches: [[f64; 2]; 2],
    /// The corner the two edges made.
    pub tip: [f64; 2],
    /// The water's level where it touches each, metres.
    pub level: [f64; 2],
    /// How the bed falls from the arc, m/m: a little less steeply than the gentler of the rivers'
    /// beds from their edges, so it is never under theirs where it blends into them.
    pub slope: f64,
    /// The bed's deepest, metres: half the shallower river's depth.
    pub deepest: f64,
    /// Metres past the old edges into the rivers' water over which its bed blends into theirs.
    pub blend: f64,
}

impl Ribbon {
    /// Whether point `k` is in a lake (one of [`Ribbon::lake_runs`]).
    pub fn in_lake(&self, k: usize) -> bool {
        self.lake_runs
            .iter()
            .any(|r| (r[0] as usize..=r[1] as usize).contains(&k))
    }
}

/// The depth of a river with `area_m2` of catchment, metres: 0.4 m at a square kilometre, 0.95 m
/// at ten.
pub fn depth(area_m2: f64) -> f64 {
    let x = area_m2 * 1e-6;
    0.4 * (x * x * x).sqrt().sqrt().sqrt()
}

/// The ribbons of `rivers` (traced over `height`, whose lakes' `lakes` water they cross),
/// ordered by the catchment at their mouths, the smallest first: a tributary comes before the
/// river it joins. Each runs from its head to where it ends: the junction's point on the larger
/// river, or the outlet's sample. Their `ground` is left at the levels; [`rest_on`] sets it.
pub fn ribbons(
    height: &Field2<f32>,
    rivers: &Rivers,
    lakes: &[LakeWater],
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
                lake_runs: Vec::new(),
                corners: Vec::new(),
                steps: Vec::new(),
                deltas: Vec::new(),
                outlets: Vec::new(),
                bars: Vec::new(),
            })
        })
        .collect();
    // The levels, the largest river first: a tributary's water ends at its river's.
    ribbons.sort_by(|a, b| {
        b.mouth_area
            .total_cmp(&a.mouth_area)
            .then(a.river.cmp(&b.river))
    });
    // The level of the lake whose water stands over the nearest sample, how deep it is there, and
    // how deep it is at its deepest.
    let lake_at = |x: f64, y: f64| -> Option<(f64, f64, f64)> {
        let last = f64::from(height.size - 1);
        let (i, j) = (
            (x / spacing).round().clamp(0.0, last) as u32,
            (y / spacing).round().clamp(0.0, last) as u32,
        );
        lakes.iter().find(|l| l.stands_at(height, i, j)).map(|l| {
            (
                f64::from(l.level),
                f64::from(l.level) - f64::from(height.get(i, j)),
                f64::from(l.depth),
            )
        })
    };
    let lake_level = |x: f64, y: f64| lake_at(x, y).map(|(level, depth, _)| (level, depth));
    let last = f64::from(height.size - 1);
    // Whether a lake's water is drawn at all at a point: its mask covers a corner of the cell of
    // samples round it (`water.slang` softens the mask over a sample), whatever the ground. Past
    // `from`, a river's outlet from it (the point, the river's direction there and its half
    // width), not where the outlet's shallow arm is trimmed off ([`crate::trim_outlets`]).
    let lake_drawn = |x: f64, y: f64, from: Option<FromLake>| -> bool {
        let outlet = from.and_then(|(at, down, half_width)| {
            let (i, j) = (
                (at[0] / spacing).round().clamp(0.0, last) as u32,
                (at[1] / spacing).round().clamp(0.0, last) as u32,
            );
            let lake = lakes.iter().position(|l| l.stands_at(height, i, j))?;
            Some(Outlet {
                lake: lake as u32,
                at,
                down,
                level: f64::from(lakes[lake].level),
                half_width,
            })
        });
        let (i, j) = ((x / spacing).floor(), (y / spacing).floor());
        [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
            .iter()
            .any(|&(di, dj)| {
                let (x, y) = (i + di, j + dj);
                x >= 0.0
                    && y >= 0.0
                    && lakes.iter().enumerate().any(|(index, l)| {
                        let (x, y) = (x as u32, y as u32);
                        l.covers(x, y)
                            && !outlet.is_some_and(|o| {
                                o.lake as usize == index && l.in_arm(height, &o, x, y)
                            })
                    })
            })
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
        let (points, in_lake, mut mouths) = levels(
            &ribbons[r].points,
            height,
            &lake_level,
            &lake_drawn,
            main,
            params,
        );
        let mut runs: Vec<[u32; 2]> = Vec::new();
        for (k, _) in in_lake.iter().enumerate().filter(|(_, l)| **l) {
            match runs.last_mut() {
                Some(run) if run[1] + 1 == k as u32 => run[1] = k as u32,
                _ => runs.push([k as u32; 2]),
            }
        }
        ribbons[r].lake_runs = runs;
        ribbons[r].points = points;
        // The steep reaches' steps and pools, before the tributaries take their levels: one
        // joining in a pool ends at the pool's.
        if let Some(steps) = &params.steps {
            let (points, made, index) = step_pools(
                &ribbons[r].points,
                &ribbons[r].lake_runs,
                ribbons[r].river,
                steps,
                params,
            );
            for run in &mut ribbons[r].lake_runs {
                *run = run.map(|k| index[k as usize]);
            }
            for mouth in &mut mouths {
                *mouth = index[*mouth] as usize;
            }
            ribbons[r].points = points;
            ribbons[r].steps = made;
        }
        if let Some(delta) = &params.delta {
            let seed = hash_cell3(DELTA_SEED, ribbons[r].river as i32, 0, 0);
            let lake_runs = ribbons[r].lake_runs.clone();
            ribbons[r].deltas = deltas(
                &mut ribbons[r].points,
                &mouths,
                &lake_runs,
                &lake_at,
                delta,
                params,
                seed,
            );
        }
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
    // The large mouths' bars, on the estuaries' widths (#127).
    if let Some(bars) = &params.bars {
        for ribbon in &mut ribbons {
            let seed = hash_cell3(BAR_SEED, ribbon.river as i32, 0, 0);
            ribbon.bars = mouth_bars(&mut ribbon.points, bars, params, seed);
        }
    }
    // Below each confluence the river runs deeper for a few widths (#119's polish): the two
    // flows meeting scour a hole, deepest about a width downstream. The level stays.
    if let Some(scour) = params.confluence_scour {
        for r in 0..ribbons.len() {
            let Mouth::Junction { river: into, .. } =
                rivers.rivers[ribbons[r].river as usize].mouth
            else {
                continue;
            };
            let (Some(m), Some(last)) = (done[into as usize], ribbons[r].points.last()) else {
                continue;
            };
            let (at, half) = (last.position.map(f64::from), f64::from(last.half_width));
            let main = &mut ribbons[m].points;
            let n = nearest(main, at);
            let main_half = f64::from(main[n].half_width).max(0.25);
            let share = scour * (half / main_half).min(1.0);
            // Its width, or two of the points' spacings on a brook, so the hole spans a few points.
            let width = (2.0 * main_half).max(2.0 * params.step);
            let mut along = 0.0;
            for k in n..main.len() {
                if k > n {
                    let (a, b) = (main[k - 1].position, main[k].position);
                    along += f64::from((b[0] - a[0]).hypot(b[1] - a[1]));
                }
                if along > 3.0 * width {
                    break;
                }
                let bump =
                    smoothstep(0.0, width, along) * (1.0 - smoothstep(width, 3.0 * width, along));
                main[k].depth = (f64::from(main[k].depth) * (1.0 + share * bump)) as f32;
            }
        }
    }
    // The confluences' rounded corners, on the final widths, and the water drawn over them: each
    // part of a corner by the river whose edge is nearer, and by the river joined too where the
    // tributary's water is fading into it.
    for p in ribbons.iter_mut().flat_map(|r| r.points.iter_mut()) {
        p.cover = p.half_width;
    }
    for r in 0..ribbons.len() {
        let Mouth::Junction { river: into, .. } = rivers.rivers[ribbons[r].river as usize].mouth
        else {
            continue;
        };
        let Some(m) = done[into as usize] else {
            continue;
        };
        let corners = confluence(&ribbons[r], &ribbons[m], params);
        let mut covers: Vec<(usize, usize, f64)> = Vec::new();
        for corner in &corners {
            for q in corner_samples(corner) {
                let (tributary, main) = (&ribbons[r].points, &ribbons[m].points);
                let t = edge_distance(
                    tributary,
                    tributary.len().saturating_sub(64)..tributary.len(),
                    q,
                );
                let n = nearest(main, q);
                let j = edge_distance(main, n.saturating_sub(48)..(n + 49).min(main.len()), q);
                let fading = tributary[t.segment].fade.min(tributary[t.segment + 1].fade) < 0.9;
                for (ribbon, e, near) in [
                    (r, t, t.out <= j.out + 1.0),
                    (m, j, j.out <= t.out + 1.0 || fading),
                ] {
                    if near {
                        let across = e.out + e.half + 0.5;
                        covers.push((ribbon, e.segment, across));
                        covers.push((ribbon, e.segment + 1, across));
                    }
                }
            }
        }
        for (ribbon, k, across) in covers {
            let p = &mut ribbons[ribbon].points[k];
            let cover = f64::from(p.cover).max(across);
            p.cover = cover as f32;
            p.reach = p.reach.max((cover + affine(params.tuck, cover)) as f32);
        }
        ribbons[r].corners = corners;
    }
    // Where each river leaves a lake: the last point of each run in one that the river runs on
    // past.
    let last = f64::from(height.size - 1);
    for ribbon in &mut ribbons {
        let p = &ribbon.points;
        ribbon.outlets = ribbon
            .lake_runs
            .iter()
            .filter(|run| (run[1] as usize) + 1 < p.len())
            .filter_map(|run| {
                let k = run[1] as usize;
                let at = position(&p[k]);
                let (i, j) = (
                    (at[0] / spacing).round().clamp(0.0, last) as u32,
                    (at[1] / spacing).round().clamp(0.0, last) as u32,
                );
                let lake = lakes.iter().position(|l| l.stands_at(height, i, j))?;
                Some(Outlet {
                    lake: lake as u32,
                    at,
                    down: [f64::from(p[k].direction[0]), f64::from(p[k].direction[1])],
                    level: f64::from(lakes[lake].level),
                    half_width: f64::from(p[k].half_width),
                })
            })
            .collect();
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
        .map(|(s, &a)| 0.5 * size(s[3], params).0 * spring(params.spring.0, a, params))
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
                cover: half[k] as f32,
                reach: (half[k] + affine(params.tuck, half[k])) as f32,
                depth: (size(s[3], params).1 * spring(params.spring.1, arc[k], params)) as f32,
                bank: 0.0,
                speed: 0.0,
                slope: 0.0,
                grade: 0.0,
                unstepped: 0.0,
                foam: 0.0,
                key: k as u32,
                lip: [0.0; 2],
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

/// The points with their levels, banks, speeds, slopes and fades, whether each is in a lake, and
/// with [`RibbonParams::delta`], the points where it runs into one (the first where the lake's
/// water stands, before a run in it); `main` is the river this one joins (its points, with their
/// levels, and the junction). `lake_drawn` tells where a lake's water is drawn at all.
fn levels(
    points: &[RibbonPoint],
    height: &Field2<f32>,
    lake_level: &dyn Fn(f64, f64) -> Option<(f64, f64)>,
    lake_drawn: &dyn Fn(f64, f64, Option<FromLake>) -> bool,
    main: Option<(&Vec<RibbonPoint>, [f64; 2])>,
    params: &RibbonParams,
) -> (Vec<RibbonPoint>, Vec<bool>, Vec<usize>) {
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
    // Where a lake's water stands over a point's nearest sample the water is at the lake's
    // level (`wet`); where it stands there [`LAKE_TAKES`] of the river's depth or more the lake
    // takes the river on (`deep`: in its shallows the river runs on in its channel). A few
    // points between two of the same lake's are its too: a flat shore at its level crosses it
    // back and forth.
    let found: Vec<Option<(f64, f64)>> = points
        .iter()
        .map(|p| lake_level(f64::from(p.position[0]), f64::from(p.position[1])))
        .collect();
    let mut wet: Vec<Option<f64>> = found.iter().map(|f| f.map(|(level, _)| level)).collect();
    let mut deep: Vec<Option<f64>> = (0..n)
        .map(|k| {
            found[k]
                .filter(|&(_, d)| d >= LAKE_TAKES * f64::from(points[k].depth))
                .map(|(level, _)| level)
        })
        .collect();
    fill_gaps(&mut wet, LAKE_FADE as usize);
    fill_gaps(&mut deep, LAKE_FADE as usize);
    for k in 0..n {
        wet[k] = wet[k].or(deep[k]);
    }
    let in_lake: Vec<bool> = deep.iter().map(Option::is_some).collect();
    // The metres along the course to the nearest point where a lake's water stands: the water
    // rises to its banks into a lake and out of it, where the lake's shore holds it at the
    // lake's level.
    let mut to_lake = vec![f64::MAX; n];
    for k in 0..n {
        if wet[k].is_some() {
            to_lake[k] = 0.0;
        } else if k > 0 {
            to_lake[k] = to_lake[k - 1] + (arc[k] - arc[k - 1]);
        }
    }
    for k in (0..n.saturating_sub(1)).rev() {
        to_lake[k] = to_lake[k].min(to_lake[k + 1] + (arc[k + 1] - arc[k]));
    }
    // Per point: what the water may reach (the lowest ground in the middle and on the banks,
    // a little up and down the course, less the freeboard), the lowest bank, and a lake.
    let mut target = vec![0.0; n];
    let mut bank = vec![0.0; n];
    let mut ground = vec![0.0; n];
    for (k, p) in points.iter().enumerate() {
        let (half, reach) = (f64::from(p.half_width), f64::from(p.reach));
        let mut lowest_bank = f64::MAX;
        let mut lowest = f64::MAX;
        for along in [-2.0, 0.0, 2.0] {
            for across in [-reach, -half, 0.0, half, reach] {
                let q = offset(p, along, across);
                // Beside a lake, its level holds the water up, not its bed.
                let h = match lake_level(q[0], q[1]) {
                    Some((lake, _)) => smooth_height(height, q[0], q[1]).max(lake),
                    None => smooth_height(height, q[0], q[1]),
                };
                lowest = lowest.min(h);
                if across != 0.0 {
                    lowest_bank = lowest_bank.min(h);
                }
            }
        }
        ground[k] = lowest;
        let shore = smoothstep(0.0, affine(params.shoal, 2.0 * half), to_lake[k]);
        let freeboard = affine(params.freeboard, 2.0 * half) * shore;
        (target[k], bank[k]) = match wet[k] {
            Some(level) => (level + LAKE_LIFT, level),
            None => (lowest - freeboard, lowest_bank),
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
    // Nor, up from a lake, under the lake's level: where the ground by the shore dips under it
    // short of the lake's water, the river stands at the level the lake holds it to (#120).
    let mut lake_floor = f64::MIN;
    for k in (0..n).rev() {
        if wet[k].is_some() {
            lake_floor = target[k];
        }
        level[k] = level[k].max(lake_floor);
    }
    // Out of a lake it keeps the lake's level a sample's spacing past the last point where the
    // lake's water stands, as far as that water may reach (its mask fades out over a sample), so
    // the two meet; then it falls [`LAKE_OUTFALL`] a metre at most until it meets its own level.
    // It keeps it to the point past the last where the lake's water is drawn at all, too (its
    // outlet's arm trimmed off): past the shore the mask still reaches over the channel carved
    // under the level, and a river under the lake's plane there was hidden by it, a dark band
    // across the channel (#120). Never over the lowest ground across it, though, out of the
    // lake's water: past the lip the water spread over the banks falling away beside it.
    let mut held: Option<(f64, f64)> = None;
    let mut from: Option<FromLake> = None;
    let mut drawn_before = false;
    for k in 0..n {
        let p = &points[k];
        if in_lake[k] {
            let down = [f64::from(p.direction[0]), f64::from(p.direction[1])];
            from = Some((position(p), down, f64::from(p.half_width)));
        }
        if wet[k].is_some() {
            held = Some((target[k], arc[k] + height.spacing));
            drawn_before = true;
        } else if let Some((lake_level, until)) = held {
            let half = f64::from(p.half_width);
            let drawn = [-half, 0.0, half].iter().any(|&across| {
                let q = offset(p, 0.0, across);
                lake_drawn(q[0], q[1], from)
            });
            let reached = drawn || drawn_before;
            drawn_before = drawn;
            let until = if reached { until.max(arc[k]) } else { until };
            held = Some((lake_level, until));
            let past = (arc[k] - until).max(0.0);
            let floor = lake_level - LAKE_OUTFALL * past;
            let floor = if reached { floor } else { floor.min(ground[k]) };
            if floor <= level[k] {
                if !reached {
                    held = None;
                }
            } else {
                level[k] = if k > 0 {
                    floor.min(level[k - 1])
                } else {
                    floor
                };
            }
        }
    }
    // Into a lake, with a delta (D-041's lake entry): over its reach before the lake's edge (the
    // first point where the lake's water stands, before a run in it) the water eases flat to the
    // lake's level, `L + (z − L)(2t − t²)` at `t` of the reach up from the edge. It meets the lake
    // with no fall, falls a third faster than it did at most (two thirds of the way up), and is
    // only ever lowered, so it still only falls and stands under its banks. It stops at another
    // lake's water upstream.
    let mut mouths = Vec::new();
    if let Some(delta) = &params.delta {
        let mut k = 1;
        while k < n {
            if wet[k].is_none() || wet[k - 1].is_some() {
                k += 1;
                continue;
            }
            let mut end = k;
            while end + 1 < n && wet[end + 1].is_some() {
                end += 1;
            }
            if in_lake[k..=end].iter().any(|&l| l) {
                mouths.push(k);
                let lake = level[k];
                let reach = affine(delta.reach, 2.0 * f64::from(points[k].half_width));
                for j in (0..k).rev() {
                    let up = arc[k] - arc[j];
                    if up >= reach || wet[j].is_some() {
                        break;
                    }
                    let t = up / reach;
                    level[j] = lake + (level[j] - lake).max(0.0) * (2.0 * t - t * t);
                }
            }
            k = end + 1;
        }
    }
    // How far each point is from the water of the river it joins: from its edge (its half
    // width), in units of how far inside it the tributary's water is gone, 3 m or six tenths of
    // the half width. The water is whole from half that outside the edge, and thins over the
    // main river's water, which is drawn before it (the larger river first). Measured from the
    // main's reach under its banks and drawn first itself, it stopped metres short and its
    // thinning water blended with the dry bed: a band across the junction (#115).
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
                        let half = f64::from(main[j].half_width)
                            + (f64::from(main[j + 1].half_width) - f64::from(main[j].half_width))
                                * t;
                        // Gone 3 m inside, or at six tenths of a narrower river's half width,
                        // so never on its middle, where the tributary's ribbon ends.
                        let inside = (0.6 * half).clamp(0.05, 3.0);
                        (d - half) / inside
                    })
                    .fold(f64::MAX, f64::min)
            })
            .collect(),
        None => vec![f64::MAX; n],
    };
    // How many points into a lake each point is, from the nearest point out of it (0 out of
    // it): the river's water is whole up to the lake's and fades over it, a little over its
    // level, so the two meet wherever the lake's edge lies between the points.
    let mut into_lake = vec![0u32; n];
    for k in 0..n {
        if in_lake[k] {
            into_lake[k] = if k > 0 {
                into_lake[k - 1].saturating_add(1)
            } else {
                u32::MAX
            };
        }
    }
    for k in (0..n.saturating_sub(1)).rev() {
        if in_lake[k] {
            into_lake[k] = into_lake[k].min(into_lake[k + 1].saturating_add(1));
        }
    }
    let points = (0..n)
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
                * (1.0 - smoothstep(0.0, LAKE_FADE, f64::from(into_lake[k])))
                * smoothstep(-1.0, 0.5, into_main[k])
                * smoothstep(0.0, 0.3, level[k]);
            RibbonPoint {
                level: level[k] as f32,
                bank: bank[k] as f32,
                speed: speed as f32,
                slope: slope as f32,
                grade: slope as f32,
                unstepped: level[k] as f32,
                fade: fade as f32,
                ground: [level[k] as f32; ACROSS + 1],
                ..p
            }
        })
        .collect();
    (points, in_lake, mouths)
}

/// With [`DeltaParams`], each mouth of `mouths` (a river's points where it runs into a lake,
/// levelled and stepped) widens the river over the reach before it as a trumpet, `1 + flare (1 −
/// t)²` times as wide and `shallow (1 − t)²` shallower at `t` of the reach up from the lake's edge,
/// and on so into the lake while the river's water fades there; and gives the fan in front of it
/// on the lake's floor ([`Delta`]), its length shortened to `fan.2` of the lake's water ahead of it.
/// `lake_at` gives the lake's level at a point, its depth there and at its deepest.
/// With [`BarParams`], the bars in the mouth of a river at the sea as wide as [`BarParams::per`]
/// or more ([`sea_mouth`]): over its length up the river from the mouth, ending
/// [`BarParams::gap`] widths short of it, the river widens by the bars' breadth, and the bars
/// stand side by side across it, staggered, channels between them and its banks. None for a
/// river that does not reach the sea, is narrower, or is too short.
fn mouth_bars(
    points: &mut [RibbonPoint],
    bars: &BarParams,
    params: &RibbonParams,
    seed: u64,
) -> Vec<Bar> {
    let Some(m) = sea_mouth(points) else {
        return Vec::new();
    };
    let width = 2.0 * f64::from(points[m].half_width);
    let count = ((width / bars.per).floor() as u32).min(bars.most);
    let n = points.len();
    let mut arc = vec![0.0; n];
    for k in 1..n {
        let (a, b) = (position(&points[k - 1]), position(&points[k]));
        arc[k] = arc[k - 1] + (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    let length = bars.length * width;
    let middle = arc[m] - bars.gap * width - 0.5 * length;
    if count == 0 || middle - 0.5 * length < 0.0 {
        return Vec::new();
    }
    // The course's point and the water's level at `s` metres along it.
    let course: Vec<([f64; 2], f64)> = points
        .iter()
        .map(|p| (position(p), f64::from(p.level)))
        .collect();
    let at = |s: f64| -> ([f64; 2], f64) {
        let k = arc.partition_point(|&a| a < s).clamp(1, n - 1);
        let t = ((s - arc[k - 1]) / (arc[k] - arc[k - 1]).max(1e-9)).clamp(0.0, 1.0);
        let ((a, la), (b, lb)) = (course[k - 1], course[k]);
        (
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            la + (lb - la) * t,
        )
    };
    // The bars' axis: the chord over their length.
    let (up, _) = at(middle - 0.5 * length);
    let (down_tip, _) = at(middle + 0.5 * length);
    let chord = [down_tip[0] - up[0], down_tip[1] - up[1]];
    let chord_length = chord[0].hypot(chord[1]).max(1e-9);
    let down = [chord[0] / chord_length, chord[1] / chord_length];
    let side = [-down[1], down[0]];
    // Each bar its own: from three quarters to one and a quarter of the breadth, the shorter by
    // up to two fifths, staggered along the river, and turned off its axis a little.
    let hash = |i: u32, j: i32| f64::from(unit_f32(hash_cell3(seed, i as i32, j, 0)));
    let breadths: Vec<f64> = (0..count)
        .map(|i| bars.breadth * width * (0.75 + 0.5 * hash(i, 3)))
        .collect();
    let breadth: f64 = breadths.iter().sum();
    // The river widens by the bars' breadth over their length, a parabola along it, so each
    // channel round them keeps its share of the water.
    for k in 0..n {
        let s = (arc[k] - middle) / (0.5 * length);
        if s.abs() < 1.0 {
            let p = &mut points[k];
            let half = f64::from(p.half_width) + breadth * (1.0 - s * s);
            p.half_width = half as f32;
            p.reach = (half + affine(params.tuck, half)) as f32;
        }
    }
    // Across the widened river, channels and bars in turn from its right bank: each channel
    // the river's width there before, shared.
    let wide = 2.0 * f64::from(points[arc.partition_point(|&a| a < middle).min(n - 1)].half_width);
    let channel = (wide - 2.0 * breadth) / f64::from(count + 1);
    let mut across = -0.5 * wide;
    (0..count)
        .map(|i| {
            let b = breadths[i as usize];
            across += channel + b;
            let at_bar = across;
            across += b;
            // Its widest point where the river is about widest, and its tail no further down
            // than the widening's end.
            let shift = (hash(i, 1) - 0.5) * 0.2 * length;
            let half_length = (0.5 * length * (0.6 + 0.4 * hash(i, 0)))
                .min((0.5 * length - shift) / (2.0 - BAR_HEAD));
            // Turned so its tail moves across by two fifths of a channel at most.
            let most = (0.4 * channel / ((2.0 - BAR_HEAD) * half_length)).atan();
            let turn = (2.0 * hash(i, 4) - 1.0) * most;
            let (sin, cos) = (turn.sin(), turn.cos());
            let axis = [down[0] * cos - down[1] * sin, down[0] * sin + down[1] * cos];
            // Across the course where it is widest, from the course's point there.
            let (on, _) = at(middle + shift);
            let level = |u: f64| at(middle + shift + u).1;
            Bar {
                centre: [on[0] + side[0] * at_bar, on[1] + side[1] * at_bar],
                down: axis,
                half: [half_length, b],
                level: [
                    level(-BAR_HEAD * half_length),
                    level((2.0 - BAR_HEAD) * half_length),
                ],
                top: bars.top,
                slopes: bars.slopes,
                wander: bars.wander * b,
                seed: hash_cell3(seed, i as i32, 2, 0),
            }
        })
        .collect()
}

fn deltas(
    points: &mut [RibbonPoint],
    mouths: &[usize],
    lake_runs: &[[u32; 2]],
    lake_at: &dyn Fn(f64, f64) -> Option<(f64, f64, f64)>,
    delta: &DeltaParams,
    params: &RibbonParams,
    seed: u64,
) -> Vec<Delta> {
    let n = points.len();
    let mut arc = vec![0.0; n];
    for k in 1..n {
        let (a, b) = (position(&points[k - 1]), position(&points[k]));
        arc[k] = arc[k - 1] + (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    let mut made = Vec::new();
    for (index, &k) in mouths.iter().enumerate() {
        let width = 2.0 * f64::from(points[k].half_width);
        let reach = affine(delta.reach, width);
        // The river's water fades out three points into the run it takes, which starts at the
        // mouth or a few points past it, across the lake's shallow margin.
        let faded = lake_runs
            .iter()
            .find(|r| r[0] as usize >= k)
            .map_or(k, |r| (r[0] as usize + LAKE_FADE as usize).min(n - 1));
        let widen = |p: &mut RibbonPoint, e: f64| {
            let half = f64::from(p.half_width) * (1.0 + delta.flare * e);
            p.half_width = half as f32;
            p.reach = (half + affine(params.tuck, half)) as f32;
            p.depth = (f64::from(p.depth) * (1.0 - delta.shallow * e)) as f32;
        };
        for j in (0..k).rev() {
            let up = arc[k] - arc[j];
            if up >= reach {
                break;
            }
            let t = 1.0 - up / reach;
            widen(&mut points[j], t * t);
        }
        for p in &mut points[k..=faded] {
            widen(p, 1.0);
        }
        // The fan: in front of the mouth, along the river's last few metres' direction, as far
        // as the lake's water stands ahead of it at most.
        let at = position(&points[k]);
        let back = position(&points[k.saturating_sub(2)]);
        let ahead = position(&points[(k + 2).min(n - 1)]);
        let (dx, dy) = (ahead[0] - back[0], ahead[1] - back[1]);
        let length = dx.hypot(dy);
        let Some((level, _, deepest)) = lake_at(at[0], at[1]) else {
            continue;
        };
        if length <= 0.0 {
            continue;
        }
        let down = [dx / length, dy / length];
        let wanted = affine((delta.fan.0, delta.fan.1), width);
        let mut water = 0.0;
        while water < wanted / delta.fan.2
            && lake_at(at[0] + down[0] * water, at[1] + down[1] * water).is_some()
        {
            water += 1.0;
        }
        let half_width = f64::from(points[k].half_width);
        let length = wanted.min(delta.fan.2 * water).max(half_width);
        made.push(Delta {
            apex: at,
            down,
            level,
            floor: level - deepest,
            half_width,
            length,
            top: delta.top,
            front: delta.front,
            wander: delta.wander * 0.5 * length,
            seed: seed ^ index as u64,
        });
    }
    made
}

/// Fills the runs of at most `most` `None`s between two equal values of `v` with that value.
fn fill_gaps(v: &mut [Option<f64>], most: usize) {
    let mut last: Option<usize> = None;
    for k in 0..v.len() {
        let Some(value) = v[k] else { continue };
        if let Some(j) = last
            && k - j > 1
            && k - j - 1 <= most
            && v[j] == Some(value)
        {
            v[j + 1..k].fill(Some(value));
        }
        last = Some(k);
    }
}

/// How far a step's line (#122) lies downstream of its point at `u` of the way across its
/// ribbon (−1 at its right edge seen downstream, 1 at its left: `across / reach`, clamped), for
/// its [`RibbonPoint::lip`]: the arch `a (1 − u²)` and the slant `|s| (1 ± u) / 2` at the
/// ribbon's vertices across, and straight between them, as the GPU draws its quads (`water.slang`
/// bends the step's vertices by it), so the carve under the water bends exactly as the water.
pub fn lip_shift(lip: [f32; 2], u: f64) -> f64 {
    let at = |v: f64| {
        let slant = if lip[1] >= 0.0 { 1.0 + v } else { 1.0 - v };
        f64::from(lip[0]) * (1.0 - v * v) + f64::from(lip[1].abs()) * 0.5 * slant
    };
    let x = (u.clamp(-1.0, 1.0) + 1.0) * 0.5 * ACROSS as f64;
    let i = (x.floor() as usize).min(ACROSS - 1);
    let (v0, v1) = (
        2.0 * i as f64 / ACROSS as f64 - 1.0,
        2.0 * (i + 1) as f64 / ACROSS as f64 - 1.0,
    );
    let (y0, y1) = (at(v0), at(v1));
    y0 + (y1 - y0) * (x - i as f64)
}

/// The point `t` of the way from `a` to `b`, every field between theirs (its direction
/// normalised again), put in by a step: no key.
fn point_between(a: &RibbonPoint, b: &RibbonPoint, t: f64) -> RibbonPoint {
    let t = t as f32;
    let mix = |x: f32, y: f32| x + (y - x) * t;
    let direction = [
        mix(a.direction[0], b.direction[0]),
        mix(a.direction[1], b.direction[1]),
    ];
    let length = (direction[0] * direction[0] + direction[1] * direction[1])
        .sqrt()
        .max(1e-9);
    RibbonPoint {
        position: [
            mix(a.position[0], b.position[0]),
            mix(a.position[1], b.position[1]),
        ],
        level: mix(a.level, b.level),
        direction: [direction[0] / length, direction[1] / length],
        half_width: mix(a.half_width, b.half_width),
        cover: mix(a.cover, b.cover),
        reach: mix(a.reach, b.reach),
        depth: mix(a.depth, b.depth),
        bank: mix(a.bank, b.bank),
        speed: mix(a.speed, b.speed),
        slope: mix(a.slope, b.slope),
        grade: mix(a.grade, b.grade),
        unstepped: mix(a.unstepped, b.unstepped),
        foam: mix(a.foam, b.foam),
        key: u32::MAX,
        lip: [0.0; 2],
        fade: mix(a.fade, b.fade),
        ground: std::array::from_fn(|i| mix(a.ground[i], b.ground[i])),
    }
}

/// Steps and pools on the steep reaches of a ribbon's levelled `points` (#122): where the reach
/// falls `steps.from` or more (and on while it falls three quarters of that), clear of its head,
/// its lakes and its end, the water stands in pools between steps a few of its widths apart,
/// each pool at the level the water had at the next step's lip, so it is never higher than it
/// was. At each lip it falls into the pool below over a short fall, four points (the lip, just
/// past it, just short of the foot, the foot), so the pools are level up to it and the fall
/// steep between. Over a lip the water is shallow, and under the fall the pool is scoured by
/// what H/L/S gives beyond the step's drop, its bed rising to the next lip. White water foams
/// down the fall and across the pool below. Returns the points, the steps, and for each point
/// before, its index now.
fn step_pools(
    points: &[RibbonPoint],
    lake_runs: &[[u32; 2]],
    river: u32,
    steps: &StepParams,
    params: &RibbonParams,
) -> (Vec<RibbonPoint>, Vec<Step>, Vec<u32>) {
    let n = points.len();
    let unchanged = (points.to_vec(), Vec::new(), (0..n as u32).collect());
    if n < 3 {
        return unchanged;
    }
    let mut arc = vec![0.0; n];
    for k in 1..n {
        let (a, b) = (position(&points[k - 1]), position(&points[k]));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        arc[k] = arc[k - 1] + (dx * dx + dy * dy).sqrt();
    }
    let at = |s: f64| {
        let j = arc.partition_point(|&a| a <= s).clamp(1, n - 1);
        let t = (s - arc[j - 1]) / (arc[j] - arc[j - 1]).max(1e-9);
        point_between(&points[j - 1], &points[j], t.clamp(0.0, 1.0))
    };
    // The reaches: clear of a lake by two points, wholly drawn (past the head's fade, short of
    // the river it joins, a lake or the sea), and a metre over the estuary, which widens the
    // river by its level.
    let clear = |k: usize| {
        points[k].fade >= 0.999
            && f64::from(points[k].level) >= params.estuary.0 + 1.0
            && !lake_runs
                .iter()
                .any(|r| k + 2 >= r[0] as usize && k <= r[1] as usize + 2)
    };
    let mut reaches: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < n {
        if clear(k) && f64::from(points[k].grade) >= steps.from {
            let first = k;
            while k + 1 < n && clear(k + 1) && f64::from(points[k + 1].grade) >= 0.75 * steps.from {
                k += 1;
            }
            reaches.push((first, k));
        }
        k += 1;
    }
    // Each reach's lips, a spacing apart, the last pool at least half of its spacing long.
    struct Plan {
        lip: f64,
        next: f64,
        fall: f64,
        drop: f64,
        pool: f64,
        scour: f64,
        width: f64,
        bow: [f32; 2],
    }
    let mut plans: Vec<Plan> = Vec::new();
    for &(first, last) in &reaches {
        let end = arc[last];
        let mut lips: Vec<(f64, f64)> = Vec::new();
        let mut s = arc[first];
        let mut i = 0;
        while s < end {
            let p = at(s);
            let slope = f64::from(p.grade);
            let widths = steps.spacing.0
                + (steps.spacing.1 - steps.spacing.0) * smoothstep(steps.from, steps.steep, slope);
            let draw = unit_f32(hash_cell3(STEP_SEED, river as i32, first as i32, i));
            let jitter = 1.0 + steps.jitter * (2.0 * f64::from(draw) - 1.0);
            let mut spacing = (widths * 2.0 * f64::from(p.half_width)).max(steps.least) * jitter;
            // No higher than `highest`, where the water falls faster than the reach's slope: the
            // spacing whose drop it is (the level only falls, so its drop grows with the spacing).
            let level = f64::from(p.level);
            if level - f64::from(at(s + spacing).level) > steps.highest {
                let (mut lo, mut hi) = ((0.5 * steps.least).min(spacing), spacing);
                if level - f64::from(at(s + lo).level) > steps.highest {
                    hi = lo;
                }
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if level - f64::from(at(s + mid).level) > steps.highest {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                spacing = hi;
            }
            lips.push((s, spacing));
            s += spacing;
            i += 1;
        }
        if let Some(&(lip, spacing)) = lips.last()
            && end - lip < 0.5 * spacing
        {
            lips.pop();
        }
        let level = |s: f64| f64::from(at(s).level);
        for (j, &(lip, _)) in lips.iter().enumerate() {
            let next = lips.get(j + 1).map_or(end, |l| l.0);
            let (top, pool) = (level(lip), level(next));
            let drop = (top - pool).max(0.0);
            let p = at(lip);
            let slope = f64::from(p.grade);
            let hls = steps.scour.0
                + (steps.scour.1 - steps.scour.0) * smoothstep(steps.from, steps.steep, slope);
            let fall = (steps.fall.0 * drop)
                .max(steps.fall.1)
                .min(0.4 * (next - lip));
            // Its line's bow: an arch and a slant towards a bank, drawn, within 1.5 m and three
            // fifths of the pool below.
            let draw = |salt: i32| {
                f64::from(unit_f32(hash_cell3(
                    STEP_SEED,
                    river as i32,
                    plans.len() as i32,
                    salt,
                )))
            };
            let half = f64::from(p.half_width);
            let (arch, slant) = (
                steps.bow.0 * half * draw(1),
                steps.bow.1 * half * (2.0 * draw(2) - 1.0),
            );
            let most = (0.6 * (next - lip - fall)).min(1.5);
            let fit = (most / (arch + slant.abs()).max(1e-9)).min(1.0);
            plans.push(Plan {
                lip,
                next,
                fall,
                drop,
                pool,
                scour: (hls - 1.0).max(0.0) * drop,
                width: 2.0 * f64::from(p.half_width),
                bow: [(arch * fit) as f32, (slant * fit) as f32],
            });
        }
    }
    if plans.is_empty() {
        return unchanged;
    }
    // The points put in, by their place along the river: per step, its lip, just past it, just
    // short of its foot and its foot, each bowed as its line is. A point of the river within a
    // centimetre of one gives way to it, and so does one past the foot by less than the bow and
    // a quarter of a metre, so the pool's first quads, from the bowed foot, never fold.
    let mut made: Vec<(f64, usize)> = Vec::new();
    for (j, plan) in plans.iter().enumerate() {
        let edge = (0.25 * plan.fall).min(0.1);
        for s in [
            plan.lip,
            plan.lip + edge,
            plan.lip + plan.fall - edge,
            plan.lip + plan.fall,
        ] {
            made.push((s, j));
        }
    }
    let mut out: Vec<(f64, RibbonPoint)> = Vec::with_capacity(n + made.len());
    let mut index = vec![0u32; n];
    let mut m = 0;
    let mut clear = f64::MIN;
    for k in 0..n {
        while m < made.len() && made[m].0 <= arc[k] + 0.01 {
            let (s, j) = made[m];
            let mut p = at(s);
            p.lip = plans[j].bow;
            out.push((s, p));
            if m % 4 == 3 {
                let bow = plans[j].bow;
                clear = s + f64::from(bow[0] + bow[1].abs()) + 0.25;
            }
            m += 1;
        }
        if arc[k] <= clear && out.last().is_some_and(|(s, _)| *s < arc[k]) {
            index[k] = (out.len() - 1) as u32;
            continue;
        }
        let replaced = out
            .last()
            .is_some_and(|(s, p)| p.key == u32::MAX && (arc[k] - s).abs() <= 0.01);
        if replaced {
            // It keeps the river's draws there.
            index[k] = (out.len() - 1) as u32;
            if let Some((_, p)) = out.last_mut() {
                p.key = points[k].key;
            }
        } else {
            index[k] = out.len() as u32;
            out.push((arc[k], points[k]));
        }
    }
    // Each point's water over its reach's steps: falling at a lip, level in a pool.
    let (mut lips, mut feet) = (vec![None; plans.len()], vec![None; plans.len()]);
    let mut j = 0;
    for (q, (s, p)) in out.iter_mut().enumerate() {
        while j + 1 < plans.len() && plans[j + 1].lip <= *s + 1e-9 {
            j += 1;
        }
        let plan = &plans[j];
        let local = *s - plan.lip;
        if local < -1e-9 || *s > plan.next + 1e-9 {
            continue;
        }
        let edge = (0.25 * plan.fall).min(0.1);
        let depth = f64::from(p.depth);
        let (lip, foot) = (steps.lip * depth, depth + plan.scour);
        let fall_speed = (2.0 * 9.81 * plan.drop).sqrt().min(params.speed.1);
        let pool_speed = (f64::from(p.speed) * steps.pool_speed).max(params.speed.0);
        let (level, slope, water, foam, speed);
        if local <= 0.5 * edge {
            // The lip: the pool above ends here, shallow.
            (level, slope, water, foam, speed) = (plan.pool + plan.drop, 0.0, lip, 0.0, pool_speed);
            lips[j] = lips[j].or(Some(q as u32));
        } else if local < plan.fall - 0.5 * edge {
            // The fall.
            let t = local / plan.fall;
            (level, slope, water, foam, speed) = (
                plan.pool + plan.drop * (1.0 - t),
                plan.drop / plan.fall,
                lip + (foot - lip) * t,
                0.3 + 0.7 * t,
                fall_speed,
            );
        } else {
            // The pool: deepest under the fall, its bed rising to the next lip, its foam fading
            // across it.
            let length = (plan.next - plan.lip - plan.fall).max(1e-6);
            let t = ((local - plan.fall) / length).clamp(0.0, 1.0);
            let rise = smoothstep(0.0, 1.0, t);
            (level, slope, water, foam, speed) = (
                plan.pool,
                0.0,
                foot + (lip - foot) * rise,
                1.0 - smoothstep(0.0, steps.foam, t),
                pool_speed,
            );
            if local <= plan.fall + 0.5 * edge {
                feet[j] = feet[j].or(Some(q as u32));
            }
        }
        p.level = level as f32;
        p.slope = slope as f32;
        p.depth = water as f32;
        p.foam = foam as f32;
        p.speed = speed as f32;
        // Every point from the lip to the foot bows with the step's line, as the carve does.
        if local <= plan.fall + 0.5 * edge {
            p.lip = plan.bow;
        }
    }
    let made_steps = plans
        .iter()
        .zip(lips.iter().zip(&feet))
        .filter_map(|(plan, (lip, foot))| {
            Some(Step {
                lip: (*lip)?,
                foot: (*foot)?,
                drop: plan.drop,
                spacing: plan.next - plan.lip,
                width: plan.width,
            })
        })
        .collect();
    (out.into_iter().map(|(_, p)| p).collect(), made_steps, index)
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

/// A point's position, metres.
fn position(p: &RibbonPoint) -> [f64; 2] {
    [f64::from(p.position[0]), f64::from(p.position[1])]
}

/// `a × b`, the z of the cross product.
pub(crate) fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// `a · b`.
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// `a + b s`.
fn along(a: [f64; 2], b: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] + b[0] * s, a[1] + b[1] * s]
}

/// `a − b`.
pub(crate) fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// `a` over its length (`a` itself when it has none).
fn unit(a: [f64; 2]) -> [f64; 2] {
    let length = dot(a, a).sqrt();
    if length > 0.0 {
        [a[0] / length, a[1] / length]
    } else {
        a
    }
}

/// How far a point is out of a ribbon's water ([`edge_distance`]).
#[derive(Clone, Copy, Debug)]
struct Edge {
    /// Metres out of the water, negative in it.
    out: f64,
    /// The half width at the nearest point of the course.
    half: f64,
    /// The nearest segment's first point, and how far along it the nearest point is (0..1).
    segment: usize,
    t: f64,
    /// The nearest point of the course.
    on: [f64; 2],
}

impl Edge {
    /// A value of the points, at the nearest point of the course.
    fn at(&self, points: &[RibbonPoint], value: impl Fn(&RibbonPoint) -> f32) -> f64 {
        let (a, b) = (
            f64::from(value(&points[self.segment])),
            f64::from(value(&points[self.segment + 1])),
        );
        a + (b - a) * self.t
    }
}

/// How far `q` is out of the water of the segments between `points[range]` (two or more), its
/// edge at their half widths.
fn edge_distance(points: &[RibbonPoint], range: std::ops::Range<usize>, q: [f64; 2]) -> Edge {
    let mut best = Edge {
        out: f64::MAX,
        half: 0.0,
        segment: range.start,
        t: 0.0,
        on: q,
    };
    for k in range.start..range.end.saturating_sub(1) {
        let (a, b) = (position(&points[k]), position(&points[k + 1]));
        let (d, t) = segment_distance(q, a, b);
        let half = f64::from(points[k].half_width)
            + f64::from(points[k + 1].half_width - points[k].half_width) * t;
        if d - half < best.out {
            best = Edge {
                out: d - half,
                half,
                segment: k,
                t,
                on: along(a, sub(b, a), t),
            };
        }
    }
    best
}

/// The rounded corners either side of where `tributary` meets `main` (#119): on each side the
/// circle touching both rivers' water's edges, their own curves near the junction, whose tangent
/// points are [`RibbonParams::confluence`] from the corner the edges make (were they straight).
/// None where it meets its river in a lake or at the sea, or on a side where the edges meet
/// nearly straight on or along each other.
fn confluence(tributary: &Ribbon, main: &Ribbon, params: &RibbonParams) -> Vec<Corner> {
    let (tp, mp) = (&tributary.points, &main.points);
    let n = tp.len();
    let j = nearest(mp, position(&tp[n - 1]));
    if tributary.in_lake(n - 1) || main.in_lake(j) || mp[j].level <= 0.05 {
        return Vec::new();
    }
    let near = j.saturating_sub(48)..(j + 49).min(mp.len());
    let out_main = |q: [f64; 2]| edge_distance(mp, near.clone(), q);
    let out_tributary = |q: [f64; 2]| edge_distance(tp, n.saturating_sub(64)..n, q);
    // The tributary's last point out of the main's water, where its edges run into the main's.
    let Some(k) = (0..n).rev().find(|&k| out_main(position(&tp[k])).out > 0.0) else {
        return Vec::new();
    };
    let f = |v: [f32; 2]| [f64::from(v[0]), f64::from(v[1])];
    let (t, m) = (f(tp[k].direction), f(mp[j].direction));
    // The main's near edge, facing up the tributary.
    let mut facing = [-m[1], m[0]];
    if dot(facing, t) > 0.0 {
        facing = [-facing[0], -facing[1]];
    }
    if dot(facing, t).abs() < 0.2 {
        return Vec::new();
    }
    let (half, main_half) = (f64::from(tp[k].half_width), f64::from(mp[j].half_width));
    let tangent = params.confluence.0 + params.confluence.1 * 2.0 * half;
    let mut corners = Vec::new();
    for side in [1.0, -1.0] {
        let normal = [-t[1] * side, t[0] * side];
        // The land between the tributary's edge back up it and the main's edge away from it.
        let away = dot(m, normal);
        if away.abs() < 0.1 {
            continue;
        }
        let (up, edge) = ([-t[0], -t[1]], [m[0] * away.signum(), m[1] * away.signum()]);
        let (cos, sin) = (dot(up, edge), cross(up, edge).abs());
        if !(-0.87..=0.97).contains(&cos) {
            continue;
        }
        let sin_half = ((1.0 - cos) / 2.0).sqrt();
        // The corner the straight edges make, and the circle touching them, from which the
        // circle touching the curved edges is found (Newton's method, the edges' distances'
        // gradients the unit vectors from their nearest points). Its radius is then scaled
        // until it touches them the tangent's length from their corner: curving, they may meet
        // more or less sharply than straight.
        let from = along(position(&tp[k]), normal, half);
        let to = along(position(&mp[j]), facing, main_half);
        let x = cross(sub(to, from), m) / cross(t, m);
        let straight = along(from, t, x);
        let mut radius = tangent * sin / (1.0 + cos);
        let mut centre = along(
            straight,
            unit([up[0] + edge[0], up[1] + edge[1]]),
            radius / sin_half,
        );
        let mut best = None;
        for _ in 0..6 {
            let mut found = None;
            for _ in 0..16 {
                let (et, em) = (out_tributary(centre), out_main(centre));
                let (gt, gm) = (unit(sub(centre, et.on)), unit(sub(centre, em.on)));
                let (ft, fm) = (et.out - radius, em.out - radius);
                if ft.abs().max(fm.abs()) < 1e-4 {
                    found = Some((et, em, gt, gm));
                    break;
                }
                let det = cross(gt, gm);
                if det.abs() < 0.05 {
                    break;
                }
                let step = [
                    (fm * gt[1] - ft * gm[1]) / det,
                    (ft * gm[0] - fm * gt[0]) / det,
                ];
                let length = dot(step, step).sqrt();
                centre = along(centre, step, (0.5 * radius / length.max(1e-12)).min(1.0));
            }
            let Some((et, em, gt, gm)) = found else {
                break;
            };
            let touches = [along(centre, gt, -radius), along(centre, gm, -radius)];
            // On this side, each touching the other's water's edge or out of it.
            if dot(sub(centre, position(&tp[k])), normal) <= 0.0
                || out_main(touches[0]).out < -0.05
                || out_tributary(touches[1]).out < -0.05
            {
                break;
            }
            // The corner: from the centre towards both waters to where the first one starts.
            let toward = unit([-(gt[0] + gm[0]), -(gt[1] + gm[1])]);
            let dry = |s: f64| {
                let q = along(centre, toward, s);
                out_tributary(q).out.min(out_main(q).out) > 0.0
            };
            let Some(wet) = (1..=(4.0 * radius / sin_half) as u32).find(|&s| !dry(f64::from(s)))
            else {
                break;
            };
            let (mut lo, mut hi) = (f64::from(wet - 1), f64::from(wet));
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if dry(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let tip = along(centre, toward, lo);
            let length = touches
                .iter()
                .map(|&p| dot(sub(p, tip), sub(p, tip)).sqrt())
                .sum::<f64>()
                / 2.0;
            best = Some((radius, centre, et, em, touches, tip));
            if (length - tangent).abs() < 0.05 * tangent || length <= 0.0 {
                break;
            }
            radius = (radius * (tangent / length).clamp(0.5, 2.0)).min(8.0 * tangent);
        }
        let Some((radius, centre, et, em, touches, tip)) = best else {
            continue;
        };
        let (depth, main_depth) = (et.at(tp, |p| p.depth), em.at(mp, |p| p.depth));
        corners.push(Corner {
            centre,
            radius,
            touches,
            tip,
            level: [et.at(tp, |p| p.level), em.at(mp, |p| p.level)],
            slope: 0.95 * (2.0 * depth / et.half).min(2.0 * main_depth / em.half),
            deepest: 0.5 * depth.min(main_depth),
            blend: 0.7 * et.half.min(em.half),
        });
    }
    corners
}

/// Points over a corner's water ([`Corner`]), which the rivers' water must cover: its tip, its
/// arc and half way between them.
pub(crate) fn corner_samples(corner: &Corner) -> Vec<[f64; 2]> {
    let (a, b) = (
        unit(sub(corner.touches[0], corner.centre)),
        unit(sub(corner.touches[1], corner.centre)),
    );
    let mut samples = vec![corner.tip];
    for i in 0..=8 {
        let w = f64::from(i) / 8.0;
        let d = unit([a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]);
        let p = along(corner.centre, d, corner.radius);
        samples.push(p);
        samples.push([0.5 * (p[0] + corner.tip[0]), 0.5 * (p[1] + corner.tip[1])]);
    }
    samples
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

    #[test]
    fn the_small_rivers_are_brooks_of_natures_size_easing_to_the_islands_by_three_km2() {
        use forge_core::dmath::powf;
        let island = RibbonParams::island();
        let alike = RibbonParams {
            brooks: None,
            ..island
        };
        // At half a square kilometre, nature's regional curves.
        let (w, d) = size(500_000.0, &island);
        assert!((w - 2.7 * powf(0.5, 0.37)).abs() < 1e-9, "{w} m wide");
        assert!((d - 0.3 * powf(0.5, 0.21)).abs() < 1e-9, "{d} m deep");
        // From three, the island's exaggeration whole.
        assert_eq!(size(3e6, &island), size(3e6, &alike));
        assert_eq!(size(2e7, &island), size(2e7, &alike));
        // In between, wider downstream and never wider than the island's rivers.
        let mut last = 0.0;
        for i in 0..=20 {
            let area = 5e5 * powf(6.0, f64::from(i) / 20.0);
            let (w, _) = size(area, &island);
            assert!(
                w > last && w <= size(area, &alike).0 + 1e-9,
                "{w} m at {area} m²"
            );
            last = w;
        }
    }

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
        // Without the estuary first: the river reaches the sea, whose last metres it widens.
        let params = RibbonParams {
            estuary: (1.5, 0.0),
            ..RibbonParams::default()
        };
        let ribbons = ribbons(&valley, &rivers, &[], &params);
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
        // With the estuary, under 1.5 m over the sea the river widens towards its mouth and
        // shallows: twice as wide and three fifths as deep at the sea's level, unchanged above.
        let estuary = super::ribbons(&valley, &rivers, &[], &RibbonParams::default());
        let wide = &estuary[0].points;
        let (outlet, mouth) = (wide[wide.len() - 1], points[points.len() - 1]);
        let e = (1.0 - smoothstep(0.0, 1.5, f64::from(outlet.level))) as f32;
        assert!(outlet.level < 1.5 && e > 0.2, "{} {e}", outlet.level);
        assert!((outlet.half_width - (1.0 + e) * mouth.half_width).abs() < 1e-4);
        assert!((outlet.depth - (1.0 - 0.4 * e) * mouth.depth).abs() < 1e-4);
        assert_eq!(wide[points.len() / 2], points[points.len() / 2]);
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
        let ribbons = ribbons(&fork, &rivers, &[], &params);
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
        assert_eq!(super::ribbons(&fork, &rivers, &[], &params), ribbons);
    }
}
