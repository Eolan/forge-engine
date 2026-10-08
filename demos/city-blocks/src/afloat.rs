//! The island's barrels afloat (#177, Phase 3's step 3 on the island): `--movers` puts Jolt
//! bodies on the island's water, pushed by it as the sea lab's are (#138,
//! `forge_physics::buoyancy`).
//!
//! Under each barrel the water is a plane fitted where it floats: in a river, its level along
//! the course and its current down it, strongest mid-channel; on a lake, still water at its
//! level; past the mouths, the sea, still at 0 m. The ground near the water is cut from the
//! island's field into Jolt height fields, so the banks, the bars and the beds stop them.
//!
//! Most barrels are carried down the four largest rivers and start again at their river's head
//! when they strand, stick or reach the sea; one in ten is moored on a rope. With two or more,
//! the last is towed round a circle on the largest lake by a line, and one more is dropped into
//! that lake again and again (#107's splashes). The world ticks at 60 Hz on the sea's clock and
//! is drawn between its last two ticks.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_physics::buoyancy::{Fluid, Hull, Water, push};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World, WorldDesc};
use forge_procgen::Field2;
use forge_render::{
    MAX_FLOATERS, MAX_WAKES, MoverTransform, SplashSource, WaterFloater, WaterLake,
    WaterRiverPoint, WaterWake,
};
use forge_task::TaskPool;
use glam::{DVec3, Quat, Vec2, Vec3};

use super::{BARREL_LENGTH, BARREL_RADIUS};

/// The physics' tick, seconds: the labs' 60 Hz.
const TICK: f64 = 1.0 / 60.0;
/// The most ticks a frame catches up; past that the barrels skip ahead to the sea's clock.
const CATCH_UP: u64 = 8;
/// A barrel's mass, kg: a drum partly full, 40 % of it under the water.
const BARREL_MASS: f32 = 100.0;
/// The rivers that carry barrels: the largest.
const BARREL_RIVERS: usize = 4;
/// One barrel in this many on a river is moored where it starts, the stream running past it.
const BARREL_MOORED: u32 = 10;
/// Where a barrel starts on a river: water at least this deep, metres, and drawn whole.
const START_DEPTH: f32 = 0.6;
/// A moored barrel's rope: this much longer than the water is deep at its anchor, metres.
const ROPE_SLACK: f32 = 1.5;
/// The towed barrel: its speed round its circle, m/s, the circle's radius at most, metres, and
/// its line's stiffness, N/m, and damping, N·s/m.
const TOW_SPEED: f32 = 2.5;
const TOW_RADIUS: f32 = 20.0;
const TOW_STIFFNESS: f32 = 600.0;
const TOW_DAMPING: f32 = 500.0;
/// The dropped barrel: every period it hangs a while this far over its floating level, falls,
/// bobs, and is lifted out again from the given second.
const DROP_PERIOD: f64 = 10.0;
const DROP_HEIGHT: f32 = 3.0;
const DROP_HANG: f64 = 2.0;
const DROP_LIFT: f64 = 6.5;
/// A carried barrel starts again at its river's head after this long on dry ground, out at sea
/// or barely moving (slower than `STUCK_SPEED`, m/s), seconds.
const STRANDED: f32 = 10.0;
const AT_SEA: f32 = 30.0;
const STUCK: f32 = 30.0;
const STUCK_SPEED: f32 = 0.05;
/// A barrel meeting the water at least this fast downwards, m/s, splashes.
const SPLASH_SPEED: f32 = 2.0;
/// The ground's tiles: samples a side (63 of the field's cells), and how far past a river's
/// reach or a lake's shore they cover, metres.
const TILE: u32 = 64;
const TILE_MARGIN: f32 = 8.0;
/// The side of the cells the rivers' segments are filed in, metres.
const CELL: f32 = 32.0;
/// The ground kept to tell the sea from dry land: every this many of the field's samples.
const GROUND_STEP: u32 = 4;
/// A river's current across it: `CURRENT_MIDDLE` times its speed in the middle, falling by
/// `CURRENT_FALL` of it to the banks (a parabola whose mean across is the speed).
const CURRENT_MIDDLE: f32 = 1.2;
const CURRENT_FALL: f32 = 0.6;
/// Fresh water: the sea's drag, its density 1000 kg/m³.
const FRESH: Fluid = Fluid {
    density: 1000.0,
    ..Fluid::SEA
};

/// What water stands at a point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// River `river`, `fade` of it drawn there: under a half, the still water of a lake or a
    /// river it runs into.
    River {
        river: u32,
        fade: f32,
    },
    Lake,
    Sea,
    /// No water: dry ground.
    Dry,
}

/// The water round a point, as a plane: what a barrel's hull meets in a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LocalWater {
    kind: Kind,
    /// Where the plane was fitted, world x and z, its level there, metres, and how much it rises
    /// a metre along x and z.
    at: [f64; 2],
    level: f64,
    rise: [f64; 2],
    /// The water's velocity, m/s.
    current: Vec3,
}

impl Water for LocalWater {
    fn height(&self, x: f64, z: f64) -> f64 {
        self.level + self.rise[0] * (x - self.at[0]) + self.rise[1] * (z - self.at[1])
    }

    fn current(&self, _x: f64, _z: f64) -> Vec3 {
        self.current
    }
}

impl LocalWater {
    /// Still water at `level`.
    fn still(kind: Kind, at: [f64; 2], level: f64) -> Self {
        Self {
            kind,
            at,
            level,
            rise: [0.0; 2],
            current: Vec3::ZERO,
        }
    }

    /// Whether it stands still: a lake, the sea, or a river faded into a lake.
    fn is_still(&self) -> bool {
        match self.kind {
            Kind::Lake | Kind::Sea => true,
            Kind::River { fade, .. } => fade < 0.5,
            Kind::Dry => false,
        }
    }

    /// The fluid it is.
    fn fluid(&self) -> Fluid {
        if self.kind == Kind::Sea {
            Fluid::SEA
        } else {
            FRESH
        }
    }
}

/// The island's water as what floats meets it ([`Waters::at`]).
struct Waters {
    rivers: Vec<Vec<WaterRiverPoint>>,
    lakes: Vec<WaterLake>,
    /// The lakes' masks' spacing, metres.
    spacing: f32,
    /// Per cell of `CELL` metres, the segments of the rivers whose reach meets it: the river and
    /// its first point.
    cells: HashMap<(i32, i32), Vec<(u32, u32)>>,
    /// The ground every `GROUND_STEP` samples, and the sea frame's x and z of its first.
    ground: Field2<f32>,
    corner: f32,
}

impl Waters {
    fn new(height: &Field2<f32>, rivers: Vec<Vec<WaterRiverPoint>>, lakes: Vec<WaterLake>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        for (r, river) in rivers.iter().enumerate() {
            for (i, pair) in river.windows(2).enumerate() {
                let reach = pair[0].reach.max(pair[1].reach);
                let (a, b) = (Vec2::from(pair[0].position), Vec2::from(pair[1].position));
                let (low, high) = (a.min(b) - reach, a.max(b) + reach);
                let (x0, z0) = ((low.x / CELL).floor() as i32, (low.y / CELL).floor() as i32);
                let (x1, z1) = (
                    (high.x / CELL).floor() as i32,
                    (high.y / CELL).floor() as i32,
                );
                for z in z0..=z1 {
                    for x in x0..=x1 {
                        cells.entry((x, z)).or_default().push((r as u32, i as u32));
                    }
                }
            }
        }
        let size = (height.size - 1) / GROUND_STEP + 1;
        let ground = Field2::from_fn(size, height.spacing * f64::from(GROUND_STEP), |x, y| {
            height.get(
                (x * GROUND_STEP).min(height.size - 1),
                (y * GROUND_STEP).min(height.size - 1),
            )
        });
        Self {
            rivers,
            lakes,
            spacing: height.spacing as f32,
            cells,
            ground,
            corner: -(0.5 * height.extent()) as f32,
        }
    }

    /// The water at world (x, z), fitted there.
    fn at(&self, x: f64, z: f64) -> LocalWater {
        let p = Vec2::new(x as f32, z as f32);
        let here = [x, z];
        // The river whose middle it is nearest, in its half widths.
        let mut best: Option<(f32, u32, u32, f32, f32)> = None;
        let cell = ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32);
        for &(r, i) in self.cells.get(&cell).map_or(&[][..], |c| &c[..]) {
            let river = &self.rivers[r as usize];
            let (a, b) = (&river[i as usize], &river[i as usize + 1]);
            let (pa, pb) = (Vec2::from(a.position), Vec2::from(b.position));
            let ab = pb - pa;
            let t = ((p - pa).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let off = p.distance(pa + ab * t);
            if off > a.reach + (b.reach - a.reach) * t {
                continue;
            }
            let half = (a.half_width + (b.half_width - a.half_width) * t).max(0.1);
            let ratio = off / half;
            if best.is_none_or(|b| ratio < b.0) {
                best = Some((ratio, r, i, t, half));
            }
        }
        let river = best.map(|(ratio, r, i, t, _)| (ratio, self.river_water(here, ratio, r, i, t)));
        let lake = self
            .lake_at(p)
            .map(|level| LocalWater::still(Kind::Lake, here, level));
        // In a river's water, unless it has faded into a lake there.
        if let Some((ratio, w)) = river
            && ratio <= 1.0
            && (!w.is_still() || lake.is_none())
        {
            return w;
        }
        if let Some(lake) = lake {
            return lake;
        }
        // Over a river's bank: its level, which the ground stands above.
        if let Some((_, w)) = river {
            return w;
        }
        let ground = self
            .ground
            .sample(f64::from(p.x - self.corner), f64::from(p.y - self.corner));
        if ground < 0.0 {
            LocalWater::still(Kind::Sea, here, 0.0)
        } else {
            // Nothing to float in: far under the ground.
            LocalWater::still(Kind::Dry, here, -1.0e3)
        }
    }

    /// River `r`'s water at `t` along its segment from point `i`, `ratio` of its half width off
    /// its middle.
    fn river_water(&self, here: [f64; 2], ratio: f32, r: u32, i: u32, t: f32) -> LocalWater {
        let river = &self.rivers[r as usize];
        let (a, b) = (&river[i as usize], &river[i as usize + 1]);
        let level = a.level + (b.level - a.level) * t;
        let fade = a.fade + (b.fade - a.fade) * t;
        let down = Vec2::from(a.direction)
            .lerp(Vec2::from(b.direction), t)
            .normalize_or(Vec2::X);
        // The level falls along the course as the points' levels do.
        let along = Vec2::from(b.position) - Vec2::from(a.position);
        let length = along.length().max(1e-3);
        let rise = (b.level - a.level) / length * (along / length);
        let speed = a.speed + (b.speed - a.speed) * t;
        let across = CURRENT_MIDDLE - CURRENT_FALL * ratio.min(1.0).powi(2);
        // Into a lake, a river it joins or the sea, its current fades with its water.
        let flowing = smoothstep(0.25, 0.75, fade);
        let current = down * (speed * across * flowing);
        LocalWater {
            kind: Kind::River { river: r, fade },
            at: here,
            level: f64::from(level),
            rise: [f64::from(rise.x), f64::from(rise.y)],
            current: Vec3::new(current.x, 0.0, current.y),
        }
    }

    /// The level of the lake whose mask holds the sample nearest `p`.
    fn lake_at(&self, p: Vec2) -> Option<f64> {
        self.lakes.iter().find_map(|lake| {
            let s = (p - Vec2::from(lake.origin)) / self.spacing;
            let (x, z) = (s.x.round(), s.y.round());
            let [w, h] = lake.size;
            if x < 0.0 || z < 0.0 || x >= w as f32 || z >= h as f32 {
                return None;
            }
            lake.mask[z as usize * w as usize + x as usize].then_some(f64::from(lake.level))
        })
    }
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The tiles of the field (its samples `(TILE - 1) * (x, z)` on, `TILE` a side) that the water
/// may carry a barrel over: those within `TILE_MARGIN` of a river's reach or a lake's water.
fn ground_tiles(
    height: &Field2<f32>,
    rivers: &[Vec<WaterRiverPoint>],
    lakes: &[WaterLake],
) -> BTreeSet<(u32, u32)> {
    let half = (0.5 * height.extent()) as f32;
    let side = (TILE - 1) as f32 * height.spacing as f32;
    let last = (height.size - 2) / (TILE - 1);
    let mut tiles = BTreeSet::new();
    let mut cover = |x: f32, z: f32, r: f32| {
        let span = |c: f32| {
            let low = ((c + half - r) / side).floor().max(0.0) as u32;
            let high = (((c + half + r) / side).floor().max(0.0) as u32).min(last);
            low.min(last)..=high
        };
        for tz in span(z) {
            for tx in span(x) {
                tiles.insert((tx, tz));
            }
        }
    };
    for p in rivers.iter().flatten() {
        cover(p.position[0], p.position[1], p.reach + TILE_MARGIN);
    }
    let spacing = height.spacing as f32;
    for lake in lakes {
        let w = lake.size[0] as usize;
        for (i, _) in lake.mask.iter().enumerate().filter(|(_, wet)| **wet) {
            let at = Vec2::from(lake.origin) + Vec2::new((i % w) as f32, (i / w) as f32) * spacing;
            cover(at.x, at.y, TILE_MARGIN);
        }
    }
    tiles
}

/// A height field of tile (`tx`, `tz`) as a fixed body's shape, and where the body stands.
fn tile_shape(height: &Field2<f32>, (tx, tz): (u32, u32)) -> Result<(Shape, DVec3)> {
    let (x0, z0) = (tx * (TILE - 1), tz * (TILE - 1));
    let last = height.size - 1;
    let samples: Vec<f32> = (0..TILE * TILE)
        .map(|i| height.get((x0 + i % TILE).min(last), (z0 + i / TILE).min(last)))
        .collect();
    let spacing = height.spacing as f32;
    let shape = Shape::height_field(&samples, TILE, Vec3::ZERO, Vec3::new(spacing, 1.0, spacing))?;
    let half = 0.5 * height.extent();
    let at = DVec3::new(
        f64::from(x0) * height.spacing - half,
        0.0,
        f64::from(z0) * height.spacing - half,
    );
    Ok((shape, at))
}

/// A river's course as the barrels start on it: the metres along it to each point, and the
/// first and the last point deep enough and drawn whole.
struct Course {
    river: usize,
    along: Vec<f32>,
    head: usize,
    end: usize,
}

impl Course {
    fn new(river: usize, points: &[WaterRiverPoint]) -> Option<Self> {
        let deep = |p: &WaterRiverPoint| p.depth >= START_DEPTH && p.fade >= 0.9;
        let head = points.iter().position(deep)?;
        let end = points.iter().rposition(deep)?;
        let mut along = vec![0.0_f32];
        for pair in points.windows(2) {
            let step = Vec2::from(pair[0].position).distance(Vec2::from(pair[1].position));
            along.push(along.last().unwrap() + step);
        }
        Some(Self {
            river,
            along,
            head,
            end,
        })
    }

    /// The point `share` of the way from its head to its end, moved on to the next deep enough.
    fn point(&self, points: &[WaterRiverPoint], share: f32) -> usize {
        let at = self.along[self.head] + share * (self.along[self.end] - self.along[self.head]);
        let i = self
            .along
            .partition_point(|&a| a < at)
            .clamp(self.head, self.end);
        (i..=self.end)
            .find(|&i| points[i].depth >= START_DEPTH && points[i].fade >= 0.9)
            .unwrap_or(self.head)
    }
}

/// What a barrel does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    /// Carried down course `course`.
    Carried { course: usize },
    /// Moored on a rope where it started on course `course`.
    Moored { course: usize },
    /// Towed round the lake's circle.
    Towed,
    /// Dropped into the lake again and again.
    Dropped,
}

/// What floats (#177's logs and crates beside the barrels): a metal drum, a log, a wooden crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Float {
    Barrel,
    Log,
    Crate,
}

/// A log's radius and length, and a crate's half side, metres (the sea lab's, #138).
pub(crate) const LOG_RADIUS: f32 = 0.2;
pub(crate) const LOG_LENGTH: f32 = 3.0;
pub(crate) const CRATE_HALF: f32 = 0.35;

impl Float {
    /// Which of `count` (`--movers`) floater `k` is: one in eight a log, one in eight a crate,
    /// the rest barrels, the towed one and the dropped one barrels too. By the count alone, so
    /// the scene reserves each prop's movers before the rivers are known.
    pub(crate) fn of(k: u32, count: u32) -> Self {
        if count > 1 && k >= count - 1 {
            return Self::Barrel;
        }
        match k % 8 {
            3 => Self::Log,
            6 => Self::Crate,
            _ => Self::Barrel,
        }
    }

    /// The order the movers' props come in the table.
    const ALL: [Float; 3] = [Float::Barrel, Float::Log, Float::Crate];

    /// Its body's origin to its middle, in its frame: a barrel's and a log's origin is the
    /// middle of their bottom (their lathe's axis +y), a crate's its middle.
    fn up(self) -> Vec3 {
        match self {
            Self::Barrel => Vec3::Y * (0.5 * BARREL_LENGTH),
            Self::Log => Vec3::Y * (0.5 * LOG_LENGTH),
            Self::Crate => Vec3::ZERO,
        }
    }

    /// Half its length lying across its way (the water's floater and wake waterline), and half
    /// its height lying there, metres.
    fn half_length(self) -> f32 {
        match self {
            Self::Barrel => 0.5 * BARREL_LENGTH,
            Self::Log => 0.5 * LOG_LENGTH,
            Self::Crate => CRATE_HALF,
        }
    }

    fn half_height(self) -> f32 {
        match self {
            Self::Barrel => BARREL_RADIUS,
            Self::Log => LOG_RADIUS,
            Self::Crate => CRATE_HALF,
        }
    }

    /// Kilograms: the drum partly full (40 % under), wood (64 % under), a crate's planks and
    /// air (35 % under).
    fn mass(self) -> f32 {
        match self {
            Self::Barrel => BARREL_MASS,
            Self::Log => 240.0,
            Self::Crate => 120.0,
        }
    }

    /// Its volume, m³.
    fn volume(self) -> f32 {
        let pi = std::f32::consts::PI;
        match self {
            Self::Barrel => pi * BARREL_RADIUS * BARREL_RADIUS * BARREL_LENGTH,
            Self::Log => pi * LOG_RADIUS * LOG_RADIUS * LOG_LENGTH,
            Self::Crate => 8.0 * CRATE_HALF * CRATE_HALF * CRATE_HALF,
        }
    }

    /// Its body's shape, its origin as [`Float::up`] has it.
    fn shape(self) -> Result<Shape> {
        let up = |h: f32| Vec3::new(0.0, h, 0.0);
        Ok(match self {
            Self::Barrel => Shape::cylinder(0.5 * BARREL_LENGTH, BARREL_RADIUS, 0.03, 300.0)?
                .offset(up(0.5 * BARREL_LENGTH), Quat::IDENTITY)?,
            Self::Log => Shape::cylinder(0.5 * LOG_LENGTH, LOG_RADIUS, 0.03, 600.0)?
                .offset(up(0.5 * LOG_LENGTH), Quat::IDENTITY)?,
            Self::Crate => Shape::cuboid(Vec3::splat(CRATE_HALF), 0.02, 400.0)?,
        })
    }

    /// Its hull as the water pushes it, in its body's frame.
    fn hull(self) -> Hull {
        match self {
            Self::Barrel => Hull::cylinder(BARREL_RADIUS, BARREL_LENGTH, 0.0, 12, 2),
            Self::Log => Hull::cylinder(LOG_RADIUS, LOG_LENGTH, 0.0, 12, 6),
            Self::Crate => Hull::cuboid(Vec3::splat(CRATE_HALF), 2),
        }
    }
}

/// The island's log and crate (#177's), after the barrel ([`super::barrel_prop`]): the sea
/// lab's, a log a lathe along +y from its bottom, a crate a block about its middle.
pub(crate) fn props() -> [PropSpec; 2] {
    [
        PropSpec {
            name: "island-log".to_owned(),
            kind: PropKind::Lathe(Lathe {
                profile: vec![
                    (0.0, 0.0),
                    (LOG_RADIUS - 0.03, 0.0),
                    (LOG_RADIUS, 0.03),
                    (LOG_RADIUS * 0.97, LOG_LENGTH * 0.5),
                    (LOG_RADIUS, LOG_LENGTH - 0.03),
                    (LOG_RADIUS - 0.03, LOG_LENGTH),
                    (0.0, LOG_LENGTH),
                ],
                around: 40,
                along: 48,
                flutes: 0,
                flute_depth: 0.0,
                flute_span: (0.0, 0.0),
            }),
        },
        PropSpec {
            name: "island-crate".to_owned(),
            kind: PropKind::Block(Block {
                half: [CRATE_HALF; 3],
                radius: 0.025,
                segments: 4,
            }),
        },
    ]
}

/// A barrel (or a log, or a crate): its body, what it is and does, and how long it has been
/// stranded, at sea or stuck, and how many times it started again.
struct Barrel {
    body: BodyId,
    float: Float,
    role: Role,
    dry: f32,
    sea: f32,
    slow: f32,
    starts: u32,
}

/// A barrel meeting the water fast: what [`SplashSource::Impact`] takes, for a second after.
#[derive(Clone, Copy, Debug)]
struct Meeting {
    float: Float,
    position: Vec3,
    velocity: Vec3,
    time: f64,
    seed: u32,
}

/// The barrels of `--movers` afloat on the island's water (the module's notes).
pub(crate) struct Barrels {
    world: World,
    waters: Waters,
    /// Per [`Float`] (its `ALL` order): the hull the water pushes and the body's shape.
    hulls: [Hull; 3],
    shapes: [Shape; 3],
    /// The floaters as the movers' table lists them: the barrels, then the logs, then the crates.
    order: Vec<usize>,
    courses: Vec<Course>,
    barrels: Vec<Barrel>,
    /// `--movers`: the barrels on the rivers and the towed one, without the dropped one.
    count: u32,
    /// The towed barrel's circle: its centre (world x, z), radius and the lake's level.
    towed: Option<(Vec2, f32, f32)>,
    /// Where the dropped barrel falls (world x, z) and the level of the water it falls into.
    dropped: Option<(Vec2, f32)>,
    /// Where the dropped barrel was as its lift began.
    lift_from: Option<Transform>,
    /// Ticks done.
    tick: u64,
    /// Each barrel at the last two ticks, its velocity at the last and the water under it.
    previous: Vec<Transform>,
    current: Vec<Transform>,
    velocities: Vec<Velocity>,
    water: Vec<LocalWater>,
    /// Where the frame stands between the last two ticks: 0 at the one before, 1 at the last.
    blend: f32,
    meetings: Vec<Meeting>,
    /// The ground's tiles made.
    pub(crate) tiles: usize,
    /// The ticks' milliseconds since the last title, and over the run.
    tick_ms: Vec<f64>,
    run_tick_ms: Vec<f64>,
    pool: Arc<TaskPool>,
}

impl Drop for Barrels {
    fn drop(&mut self) {
        let mut ticks = std::mem::take(&mut self.run_tick_ms);
        if ticks.is_empty() {
            return;
        }
        let mean = ticks.iter().sum::<f64>() / ticks.len() as f64;
        tracing::info!(
            barrels = self.barrels.len(),
            ticks = ticks.len(),
            mean = format!("{mean:.3}"),
            p99 = format!("{:.3}", super::percentile(&mut ticks, 0.99)),
            max = format!("{:.3}", super::percentile(&mut ticks, 1.0)),
            starts = self.barrels.iter().map(|b| b.starts).sum::<u32>(),
            digest = format!("{:#018x}", self.digest()),
            "the barrels' physics ticks (ms) over the run"
        );
    }
}

impl Barrels {
    /// `count` barrels on the island of field `height`, its `rivers` (the tributaries first, as
    /// the water uploads them: the largest last) and its `lakes`.
    pub(crate) fn new(
        height: &Field2<f32>,
        rivers: Vec<Vec<WaterRiverPoint>>,
        lakes: Vec<WaterLake>,
        count: u32,
    ) -> Result<Self> {
        let spacing = height.spacing as f32;
        let courses: Vec<Course> = (0..rivers.len())
            .rev()
            .filter_map(|r| Course::new(r, &rivers[r]))
            .take(BARREL_RIVERS)
            .collect();
        anyhow::ensure!(!courses.is_empty(), "no river deep enough for a barrel");
        // The ground under the rivers a barrel may float down (as wide as the narrowest that
        // carries them, or wider) and under the lakes.
        let narrowest = courses
            .iter()
            .map(|c| widest(&rivers[c.river]))
            .fold(f32::INFINITY, f32::min);
        let carrying: Vec<Vec<WaterRiverPoint>> = rivers
            .iter()
            .filter(|r| widest(r) >= narrowest)
            .cloned()
            .collect();
        let tile_set = ground_tiles(height, &carrying, &lakes);
        let towed = (count > 1)
            .then(|| {
                lakes
                    .iter()
                    .max_by_key(|l| l.mask.iter().filter(|&&m| m).count())
            })
            .flatten()
            .map(|lake| {
                let (centre, room) = lake_middle(lake, spacing);
                (centre, (0.6 * room).min(TOW_RADIUS), lake.level)
            });
        let dropped = (count > 1)
            .then(|| towed.map_or((Vec2::ZERO, -1000.0), |(centre, _, level)| (centre, level)));
        let waters = Waters::new(height, rivers, lakes);
        let movers: u32 = Self::movers(count).iter().sum();
        // Room for every barrel touching a few others and the ground: 10 000 packed along four
        // rivers overflowed Jolt's default 32 768 contacts.
        let defaults = WorldDesc::default();
        let mut world = World::new(&WorldDesc {
            max_bodies: movers + tile_set.len() as u32 + 16,
            max_body_pairs: defaults.max_body_pairs.max(16 * movers),
            max_contact_constraints: defaults.max_contact_constraints.max(8 * movers),
            ..defaults
        });
        for &tile in &tile_set {
            let (shape, at) = tile_shape(height, tile)?;
            world.add_body(&BodyDesc {
                friction: 0.6,
                ..BodyDesc::fixed(&shape, at)
            })?;
        }
        world.optimize_broad_phase();
        let floats: Vec<Float> = (0..movers).map(|k| Float::of(k, count)).collect();
        let mut order: Vec<usize> = (0..floats.len()).collect();
        order.sort_by_key(|&n| Float::ALL.iter().position(|&f| f == floats[n]));
        let mut this = Self {
            world,
            waters,
            hulls: Float::ALL.map(Float::hull),
            shapes: [
                Float::Barrel.shape()?,
                Float::Log.shape()?,
                Float::Crate.shape()?,
            ],
            order,
            courses,
            barrels: Vec::with_capacity(movers as usize),
            count,
            towed,
            dropped,
            lift_from: None,
            tick: 0,
            previous: Vec::new(),
            current: Vec::new(),
            velocities: Vec::new(),
            water: Vec::new(),
            blend: 1.0,
            meetings: Vec::new(),
            tiles: tile_set.len(),
            tick_ms: Vec::new(),
            run_tick_ms: Vec::new(),
            pool: Arc::new(TaskPool::client()),
        };
        for k in 0..movers {
            let (role, float) = (this.role(k), floats[k as usize]);
            let (transform, velocity) = this.start(k, role, float, 0);
            let shape = &this.shapes[float as usize];
            let body = this.world.add_body(&BodyDesc {
                rotation: transform.rotation,
                linear_velocity: velocity,
                friction: 0.5,
                restitution: 0.1,
                mass: Some(float.mass()),
                ..BodyDesc::dynamic(shape, transform.position)
            })?;
            if let Role::Moored { course } = role {
                let centre = middle(transform, float);
                let points = &this.waters.rivers[this.courses[course].river];
                let i = this.start_point(k, course);
                let bed = f64::from(points[i].level - points[i].depth);
                let anchor = DVec3::new(centre.x, bed, centre.z);
                let rope = points[i].depth + ROPE_SLACK;
                this.world
                    .join_distance(None, body, anchor, centre, (0.0, rope));
            }
            this.barrels.push(Barrel {
                body,
                float,
                role,
                dry: 0.0,
                sea: 0.0,
                slow: 0.0,
                starts: 0,
            });
        }
        this.read_state();
        this.previous = this.current.clone();
        // The water under each before the first tick: a frame may come before it.
        this.water = this
            .current
            .iter()
            .zip(&this.barrels)
            .map(|(&t, b)| {
                let m = middle(t, b.float);
                this.waters.at(m.x, m.z)
            })
            .collect();
        Ok(this)
    }

    /// The movers the floaters take in the table, a prop each in [`Float`]'s order: the
    /// `--movers` count and the dropped barrel, as [`Float::of`] makes them.
    pub(crate) fn movers(count: u32) -> [u32; 3] {
        let total = count + u32::from(count > 1);
        Float::ALL.map(|f| (0..total).filter(|&k| Float::of(k, count) == f).count() as u32)
    }

    /// The rivers that carry barrels.
    pub(crate) fn rivers(&self) -> usize {
        self.courses.len()
    }

    /// The towed barrel's circle's radius, metres (0 without one).
    pub(crate) fn tow_radius(&self) -> f32 {
        self.towed.map_or(0.0, |t| t.1)
    }

    /// What barrel `k` does: the last of `count` towed (with two or more), the one after it
    /// dropped, one in `BARREL_MOORED` on each river moored, the rest carried.
    fn role(&self, k: u32) -> Role {
        if self.dropped.is_some() && k == self.count {
            return Role::Dropped;
        }
        if self.towed.is_some() && k == self.count - 1 {
            return Role::Towed;
        }
        let rivers = self.courses.len() as u32;
        let course = (k % rivers) as usize;
        if (k / rivers) % BARREL_MOORED == BARREL_MOORED / 2 {
            Role::Moored { course }
        } else {
            Role::Carried { course }
        }
    }

    /// The point of its course river barrel `k` starts at: its share of the river's barrels
    /// spread along the course.
    fn start_point(&self, k: u32, course: usize) -> usize {
        let rivers = self.courses.len() as u32;
        let on_rivers = self.count - u32::from(self.towed.is_some());
        let per_river = on_rivers.div_ceil(rivers).max(1);
        let share = (k / rivers) as f32 / per_river as f32;
        let c = &self.courses[course];
        c.point(&self.waters.rivers[c.river], share)
    }

    /// Where floater `k` of `role`, a `float`, starts for the `starts`-th time, and its velocity.
    fn start(&self, k: u32, role: Role, float: Float, starts: u32) -> (Transform, Vec3) {
        match role {
            Role::Carried { course } | Role::Moored { course } => {
                let c = &self.courses[course];
                let points = &self.waters.rivers[c.river];
                let i = if starts == 0 {
                    self.start_point(k, course)
                } else {
                    c.head
                };
                let p = &points[i];
                let down = Vec2::from(p.direction).normalize_or(Vec2::X);
                let across = Vec2::new(-down.y, down.x);
                let off = (unit(hash(k, starts)) - 0.5) * 0.7 * p.half_width;
                let flat = Vec2::from(p.position) + off * across;
                let water = self.waters.at(f64::from(flat.x), f64::from(flat.y));
                let velocity = if matches!(role, Role::Moored { .. }) {
                    Vec3::ZERO
                } else {
                    water.current
                };
                (
                    lying(
                        Vec3::new(flat.x, p.level - 0.05, flat.y),
                        Vec3::new(across.x, 0.0, across.y),
                        float,
                    ),
                    velocity,
                )
            }
            Role::Towed => {
                let (centre, radius, level) = self.towed.expect("a towed barrel's circle");
                let (at, velocity) = tow_target(centre, radius, 0.0);
                let ahead = velocity.normalize_or(Vec2::X);
                (
                    lying(
                        Vec3::new(at.x, level - 0.05, at.y),
                        Vec3::new(-ahead.y, 0.0, ahead.x),
                        float,
                    ),
                    Vec3::new(velocity.x, 0.0, velocity.y),
                )
            }
            Role::Dropped => (self.hanging(), Vec3::ZERO),
        }
    }

    /// The dropped barrel hanging over its lake, its axis along x.
    fn hanging(&self) -> Transform {
        let (centre, level) = self.dropped.expect("a dropped barrel's lake");
        lying(
            Vec3::new(centre.x, level - 0.05 + DROP_HEIGHT, centre.y),
            Vec3::X,
            Float::Barrel,
        )
    }

    /// Steps the barrels up to the sea's clock `time` (seconds), and sets where the frame stands
    /// between the last two ticks.
    pub(crate) fn advance(&mut self, time: f64) {
        let due = (time / TICK + 1e-6).floor().max(0.0) as u64;
        if due > self.tick + CATCH_UP {
            self.tick = due - CATCH_UP;
        }
        while self.tick < due {
            self.step();
        }
        let last = (self.tick as f64 - 1.0) * TICK;
        self.blend = ((time - last) / TICK).clamp(0.0, 1.0) as f32;
    }

    /// One tick: the water's push on each barrel, the towed one's line, the dropped one held or
    /// lifted, Jolt's step, then the barrels that splash and those that start again.
    fn step(&mut self) {
        let start = std::time::Instant::now();
        let time = self.tick as f64 * TICK;
        let bodies: Vec<BodyId> = self.barrels.iter().map(|b| b.body).collect();
        let mut centres = Vec::new();
        self.world.centers_of_mass(&bodies, &mut centres);
        self.water = centres.iter().map(|c| self.waters.at(c.x, c.z)).collect();
        // The dropped barrel follows its cycle; the water pushes it only while it is let go.
        let held = self.drop_control(time);
        let mut pushes = vec![None; bodies.len()];
        {
            let (hulls, current, velocities, water, barrels) = (
                &self.hulls,
                &self.current,
                &self.velocities,
                &self.water,
                &self.barrels,
            );
            let centres = &centres;
            self.pool.scope(|scope| {
                for (k, out) in pushes.chunks_mut(32).enumerate() {
                    scope.spawn(move |_| {
                        for (i, slot) in out.iter_mut().enumerate() {
                            let n = k * 32 + i;
                            if held && barrels[n].role == Role::Dropped {
                                continue;
                            }
                            let p = push(
                                &hulls[barrels[n].float as usize],
                                current[n],
                                velocities[n],
                                centres[n],
                                &water[n],
                                &water[n].fluid(),
                            );
                            if p.wetted > 0.0 {
                                *slot = Some((p.force, current[n].position, p.torque));
                            }
                        }
                    });
                }
            });
        }
        let (pushed, forces): (Vec<BodyId>, Vec<(Vec3, DVec3, Vec3)>) = bodies
            .iter()
            .zip(&pushes)
            .filter_map(|(&b, p)| p.map(|p| (b, p)))
            .unzip();
        self.world.push(&pushed, &forces);
        // The towed barrel's line pulls it towards its place on the circle.
        if let Some((centre, radius, _)) = self.towed
            && let Some(n) = self.barrels.iter().position(|b| b.role == Role::Towed)
        {
            let (target, speed) = tow_target(centre, radius, time);
            let at = Vec2::new(centres[n].x as f32, centres[n].z as f32);
            let v = self.velocities[n].linear;
            let pull = TOW_STIFFNESS * (target - at) + TOW_DAMPING * (speed - Vec2::new(v.x, v.z));
            self.world
                .add_force(self.barrels[n].body, Vec3::new(pull.x, 0.0, pull.y));
        }
        self.world
            .step(TICK as f32, 1)
            .expect("the barrels' world steps");
        self.tick += 1;
        self.previous = std::mem::take(&mut self.current);
        self.read_state();
        self.after_step();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        self.tick_ms.push(ms);
        self.run_tick_ms.push(ms);
    }

    /// The title's part: the ticks' time since the last title.
    pub(crate) fn title(&mut self) -> String {
        let n = self.tick_ms.len().max(1) as f64;
        let mean = self.tick_ms.iter().sum::<f64>() / n;
        let max = self.tick_ms.iter().copied().fold(0.0, f64::max);
        self.tick_ms.clear();
        format!(
            "{} barrels afloat, {mean:.2} ms a tick (max {max:.2})",
            self.barrels.len()
        )
    }

    /// The dropped barrel at `time`: held over the lake, let go, or lifted back; whether the
    /// water should leave it alone this tick.
    fn drop_control(&mut self, time: f64) -> bool {
        let Some(n) = self.barrels.iter().position(|b| b.role == Role::Dropped) else {
            return false;
        };
        let body = self.barrels[n].body;
        let into = time.rem_euclid(DROP_PERIOD);
        let hang = self.hanging();
        // Without a lake it waits far under the ground, held.
        if into < DROP_HANG || self.towed.is_none() {
            self.lift_from = None;
            self.world.set_transform(body, hang);
            self.world.set_velocity(body, Velocity::default());
            true
        } else if into < DROP_LIFT {
            false
        } else {
            // Lifted along a smooth step from where it floats to where it hangs.
            let from = *self.lift_from.get_or_insert(self.current[n]);
            let span = DROP_PERIOD - DROP_LIFT;
            let w = ((into - DROP_LIFT) / span) as f32;
            let s = w * w * (3.0 - 2.0 * w);
            let rate = 6.0 * w * (1.0 - w) / span as f32;
            let position = from.position.lerp(hang.position, f64::from(s));
            let rotation = from.rotation.slerp(hang.rotation, s);
            self.world
                .set_transform(body, Transform { position, rotation });
            self.world.set_velocity(
                body,
                Velocity {
                    linear: (hang.position - from.position).as_vec3() * rate,
                    angular: Vec3::ZERO,
                },
            );
            true
        }
    }

    /// Reads every barrel's transform and velocity after a step.
    fn read_state(&mut self) {
        let bodies: Vec<BodyId> = self.barrels.iter().map(|b| b.body).collect();
        self.world.transforms(&bodies, &mut self.current);
        self.world.velocities(&bodies, &mut self.velocities);
    }

    /// After a step: the barrels that met the water fast, and the carried ones that start again.
    fn after_step(&mut self) {
        let time = self.tick as f64 * TICK;
        self.meetings.retain(|m| time - m.time <= 1.0);
        let dt = TICK as f32;
        for n in 0..self.barrels.len() {
            let float = self.barrels[n].float;
            let (before, now) = (
                middle(self.previous[n], float),
                middle(self.current[n], float),
            );
            let water = self.water[n];
            let level = water.height(now.x, now.z);
            let v = self.velocities[n].linear;
            let r = f64::from(float.half_height());
            if water.kind != Kind::Dry
                && before.y - r > level
                && now.y - r <= level
                && v.y <= -SPLASH_SPEED
            {
                self.meetings.push(Meeting {
                    float,
                    position: Vec3::new(now.x as f32, level as f32, now.z as f32),
                    velocity: v,
                    time,
                    seed: hash(n as u32, self.tick as u32),
                });
            }
            let k = n as u32;
            let barrel = &mut self.barrels[n];
            let Role::Carried { .. } = barrel.role else {
                continue;
            };
            barrel.dry = if water.kind == Kind::Dry {
                barrel.dry + dt
            } else {
                0.0
            };
            barrel.sea = if water.kind == Kind::Sea {
                barrel.sea + dt
            } else {
                0.0
            };
            let flat = Vec2::new(v.x, v.z).length();
            barrel.slow = if flat < STUCK_SPEED {
                barrel.slow + dt
            } else {
                0.0
            };
            if barrel.dry > STRANDED || barrel.sea > AT_SEA || barrel.slow > STUCK || now.y < -100.0
            {
                barrel.starts += 1;
                (barrel.dry, barrel.sea, barrel.slow) = (0.0, 0.0, 0.0);
                let (role, starts, body) = (barrel.role, barrel.starts, barrel.body);
                let (transform, velocity) = self.start(k, role, float, starts);
                self.world.set_transform(body, transform);
                self.world.set_velocity(
                    body,
                    Velocity {
                        linear: velocity,
                        angular: Vec3::ZERO,
                    },
                );
                // Drawn where it starts, not swept across the island.
                self.previous[n] = transform;
                self.current[n] = transform;
            }
        }
    }

    /// Barrel `n` as drawn: between its last two ticks.
    fn drawn(&self, n: usize) -> Transform {
        let (a, b) = (self.previous[n], self.current[n]);
        Transform {
            position: a.position.lerp(b.position, f64::from(self.blend)),
            rotation: a.rotation.slerp(b.rotation, self.blend),
        }
    }

    /// Their transforms as drawn, relative to the scene's origin (the sea's frame), in the
    /// movers' table's order: the barrels, then the logs, then the crates.
    pub(crate) fn transforms(&self) -> Vec<MoverTransform> {
        self.order
            .iter()
            .map(|&n| {
                let t = self.drawn(n);
                MoverTransform {
                    position: t.position.as_vec3(),
                    rotation: t.rotation,
                    scale: 1.0,
                }
            })
            .collect()
    }

    /// The barrels nearest `camera` (world x and z) as the rivers' water sees them (#107):
    /// their outline at the water's level, about a circle half their length across, and their
    /// velocity.
    pub(crate) fn floaters(&self, camera: Vec2) -> Vec<WaterFloater> {
        let mut near: Vec<(f32, WaterFloater)> = (0..self.barrels.len())
            .filter(|&n| self.water[n].kind != Kind::Dry)
            .map(|n| {
                let float = self.barrels[n].float;
                let centre = middle(self.drawn(n), float).as_vec3();
                let flat = Vec2::new(centre.x, centre.z);
                let v = self.velocities[n].linear;
                let floater = WaterFloater {
                    position: flat.to_array(),
                    waterline: float.half_length(),
                    velocity: [v.x, v.z],
                };
                (flat.distance_squared(camera), floater)
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter()
            .take(MAX_FLOATERS)
            .map(|(_, f)| f)
            .collect()
    }

    /// The barrels in still water nearest `camera` (world x and z), afloat or going in, making
    /// waves (#107).
    pub(crate) fn wakes(&self, camera: Vec2) -> Vec<WaterWake> {
        let mut near: Vec<(f32, WaterWake)> = (0..self.barrels.len())
            .filter(|&n| self.water[n].is_still())
            .filter_map(|n| {
                let float = self.barrels[n].float;
                let centre = middle(self.drawn(n), float);
                let level = self.water[n].height(centre.x, centre.z);
                // In the water, or just over it.
                let over = (centre.y - level) as f32;
                (-1.0..=float.half_height() + 0.05)
                    .contains(&over)
                    .then(|| {
                        let flat = Vec2::new(centre.x as f32, centre.z as f32);
                        let v = self.velocities[n].linear;
                        let wake = WaterWake {
                            position: flat.to_array(),
                            waterline: float.half_length(),
                            velocity: [v.x, v.z],
                            rise: v.y,
                        };
                        (flat.distance_squared(camera), wake)
                    })
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter().take(MAX_WAKES).map(|(_, w)| w).collect()
    }

    /// Where the barrels make the water splash at the sea's `time` (#107): those that met it
    /// fast in the last second, the drops running off the dropped barrel as it is lifted out,
    /// and the towed barrel's bow.
    pub(crate) fn splashes(&self, out: &mut Vec<SplashSource>) {
        for m in &self.meetings {
            // Lying across its fall: the circle of its outline's area.
            let (along, up) = (m.float.half_length(), m.float.half_height());
            out.push(SplashSource::Impact {
                position: m.position,
                velocity: m.velocity,
                radius: (4.0 * along * up / std::f32::consts::PI).sqrt(),
                density: m.float.mass() / (m.float.volume() * FRESH.density),
                time: m.time as f32,
                seed: m.seed,
            });
        }
        for (n, barrel) in self.barrels.iter().enumerate() {
            match barrel.role {
                Role::Dropped => {
                    // Out of the water and rising: drops run off its underside, fewer as it
                    // climbs.
                    let centre = middle(self.drawn(n), barrel.float).as_vec3();
                    let level = self.water[n].level as f32;
                    let above = centre.y - BARREL_RADIUS - level;
                    let rise = self.velocities[n].linear.y;
                    if self.water[n].kind != Kind::Dry && above > 0.0 && rise > 0.0 {
                        out.push(SplashSource::Drip {
                            position: centre - Vec3::new(0.0, BARREL_RADIUS, 0.0),
                            spread: self.drawn(n).rotation
                                * Vec3::new(0.0, 0.5 * BARREL_LENGTH, 0.0),
                            velocity: Vec3::new(0.0, rise, 0.0),
                            level,
                            rate: 60.0 * (-above / 0.5).exp(),
                            seed: 0xd419 ^ (self.tick / (DROP_PERIOD / TICK) as u64) as u32,
                        });
                    }
                }
                Role::Towed => {
                    let centre = middle(self.drawn(n), barrel.float).as_vec3();
                    let v = self.velocities[n].linear;
                    let ahead = Vec2::new(v.x, v.z).normalize_or_zero();
                    out.push(SplashSource::Bow {
                        bow: Vec3::new(centre.x, centre.y + 0.05, centre.z)
                            + BARREL_RADIUS * Vec3::new(ahead.x, 0.0, ahead.y),
                        velocity: Vec2::new(v.x, v.z),
                        beam: BARREL_LENGTH,
                        length: 2.0 * BARREL_RADIUS,
                        seed: 0x70ed,
                    });
                }
                _ => {}
            }
        }
    }

    /// A view of barrel `k` as it starts, from `side_m` to its side and `up_m` over it, looking
    /// `ahead_m` down its way; `time` seconds on along its way at the stream's (or the line's)
    /// speed: `x,y,z,yaw,pitch` for `--view`.
    pub(crate) fn view_of(
        &self,
        k: u32,
        time: f32,
        side_m: f32,
        up_m: f32,
        ahead_m: f32,
    ) -> String {
        let role = self.role(k);
        let (mut centre, velocity) = match role {
            Role::Towed => {
                let (centre, radius, level) = self.towed.expect("a towed barrel's circle");
                let (at, v) = tow_target(centre, radius, f64::from(time));
                (Vec3::new(at.x, level, at.y), Vec3::new(v.x, 0.0, v.y))
            }
            _ => {
                let float = Float::of(k, self.count);
                let (t, v) = self.start(k, role, float, 0);
                (middle(t, float).as_vec3(), v)
            }
        };
        if role != Role::Towed {
            centre += velocity * time;
        }
        let down = Vec2::new(velocity.x, velocity.z).normalize_or(Vec2::X);
        let side = Vec3::new(-down.y, 0.0, down.x);
        let down = Vec3::new(down.x, 0.0, down.y);
        let eye = centre + side_m * side + Vec3::new(0.0, up_m, 0.0);
        let look = (centre + ahead_m * down - eye).normalize();
        format!(
            "{:.1},{:.2},{:.1},{:.1},{:.1}",
            eye.x,
            eye.y,
            eye.z,
            (-look.x).atan2(-look.z).to_degrees(),
            look.y.asin().to_degrees()
        )
    }

    /// The first moored barrel the stream runs past at `speed` m/s or more.
    pub(crate) fn moored_in(&self, speed: f32) -> Option<u32> {
        (0..self.count).find(|&k| match self.role(k) {
            Role::Moored { course } => {
                let c = &self.courses[course];
                self.waters.rivers[c.river][self.start_point(k, course)].speed >= speed
            }
            _ => false,
        })
    }

    /// Whether there is a towed barrel, and the dropped one's lake: where it falls (world x, z)
    /// and its level.
    pub(crate) fn towing(&self) -> bool {
        self.towed.is_some()
    }

    pub(crate) fn drop_at(&self) -> Option<(Vec2, f32)> {
        self.dropped
    }

    /// The barrels' digest after the last tick: every body's place and rotation to the bit.
    pub(crate) fn digest(&self) -> u64 {
        forge_physics::state_hash(&self.current)
    }
}

/// A `float` lying with its middle at `centre` and its length along `axis` (a crate with a side
/// along it, upright): its body's transform.
fn lying(centre: Vec3, axis: Vec3, float: Float) -> Transform {
    let axis = axis.normalize_or(Vec3::X);
    let rotation = match float {
        Float::Crate => Quat::from_rotation_arc(Vec3::X, axis),
        _ => Quat::from_rotation_arc(Vec3::Y, axis),
    };
    Transform {
        position: (centre - rotation * float.up()).as_dvec3(),
        rotation,
    }
}

/// The middle of a `float` whose body stands at `t`.
fn middle(t: Transform, float: Float) -> DVec3 {
    t.position + (t.rotation * float.up()).as_dvec3()
}

/// Where the towed barrel's line pulls towards at `time` seconds on its circle, and that
/// place's velocity.
fn tow_target(centre: Vec2, radius: f32, time: f64) -> (Vec2, Vec2) {
    let angle = (f64::from(TOW_SPEED / radius) * time).rem_euclid(std::f64::consts::TAU) as f32;
    let out = Vec2::new(angle.cos(), angle.sin());
    (centre + radius * out, TOW_SPEED * Vec2::new(-out.y, out.x))
}

/// The point of `lake` farthest from its shore (world x and z) and how far that is, metres: the
/// mask's samples (`spacing` apart) by their distance in samples to the nearest dry one, eight
/// neighbours a step.
fn lake_middle(lake: &WaterLake, spacing: f32) -> (Vec2, f32) {
    let [w, h] = lake.size.map(|s| s as usize);
    let mut steps = vec![u32::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for (i, &wet) in lake.mask.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        if !wet || x == 0 || y == 0 || x == w - 1 || y == h - 1 {
            steps[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let n = ny as usize * w + nx as usize;
            if steps[n] == u32::MAX {
                steps[n] = steps[i] + 1;
                queue.push_back(n);
            }
        }
    }
    let (best, &most) = steps.iter().enumerate().max_by_key(|&(_, s)| *s).unwrap();
    let at = Vec2::new((best % w) as f32, (best / w) as f32) * spacing + Vec2::from(lake.origin);
    (at, most as f32 * spacing)
}

/// The widest a river's water is, metres.
fn widest(points: &[WaterRiverPoint]) -> f32 {
    points.iter().map(|p| p.half_width).fold(0.0, f32::max)
}

fn hash(a: u32, b: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9e37_79b9) ^ b.wrapping_mul(0x85eb_ca6b) ^ 0xb5ad_4ece;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn unit(x: u32) -> f32 {
    (x >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The river's level, metres.
    const LEVEL: f32 = 0.5;

    /// A straight river running east at 1 m/s across a field 256 m wide: its bed 1 m under the
    /// sea's level and 12 m wide, its banks rising 1 in 1 to the ground 2.5 m up, its water
    /// 1.5 m deep and 15 m wide. It fades in over its first points and out over its last.
    fn river() -> (Field2<f32>, Vec<Vec<WaterRiverPoint>>) {
        let field = Field2::from_fn(129, 2.0, |_, z| {
            let off = (z as f32 * 2.0 - 128.0).abs();
            (-1.0 + (off - 6.0).max(0.0)).min(2.5)
        });
        let last = 60;
        let points = (0..=last)
            .map(|i| WaterRiverPoint {
                position: [-120.0 + 4.0 * i as f32, 0.0],
                level: LEVEL,
                direction: [1.0, 0.0],
                half_width: 7.5,
                cover: 7.5,
                reach: 10.0,
                depth: 1.5,
                bank: 2.5,
                speed: 1.0,
                slope: 0.0,
                foam: 0.0,
                lip: [0.0; 2],
                fade: (i.min(last - i) as f32 / 3.0).min(1.0),
                ground: Default::default(),
                bars: [0.0; 4],
            })
            .collect();
        (field, vec![points])
    }

    /// Thirteen barrels (twelve on the river, one of them moored, and the dropped one, held
    /// since there is no lake) ten seconds on: their places, and the digest.
    fn run() -> (Vec<Transform>, Vec<Transform>, u64) {
        let (field, rivers) = river();
        let mut barrels = Barrels::new(&field, rivers, Vec::new(), 12).expect("the barrels");
        let start = barrels.current.clone();
        // A frame a tick, as `--fixed-step` runs.
        for frame in 1..=600 {
            barrels.advance(f64::from(frame) / 60.0);
        }
        assert_eq!(barrels.tick, 600);
        (start, barrels.current.clone(), barrels.digest())
    }

    #[test]
    fn barrels_drift_down_a_river_afloat_and_a_moored_one_holds() {
        let (start, end, _) = run();
        for k in 0..12 {
            let float = Float::of(k as u32, 12);
            let (a, b) = (middle(start[k], float), middle(end[k], float));
            assert!(
                (b.y - f64::from(LEVEL)).abs() < 0.25,
                "{float:?} {k} afloat: its middle at {:.3}",
                b.y
            );
            let moved = b.x - a.x;
            if k == 5 {
                // Moored: carried until its rope of 3 m holds it.
                assert!(
                    moved > 0.3 && moved < 3.2,
                    "the moored barrel moved {moved:.2} m"
                );
            } else {
                // Carried at about the stream's speed, faster mid-channel.
                assert!(moved > 6.0 && moved < 13.0, "barrel {k} moved {moved:.2} m");
            }
            assert!(
                b.z.abs() < 7.0,
                "barrel {k} stayed in the river: z {:.2}",
                b.z
            );
        }
    }

    #[test]
    fn a_frame_before_the_first_tick_draws_them_where_they_start() {
        // A frame with a variable step may come before the first tick.
        let (field, rivers) = river();
        let mut barrels = Barrels::new(&field, rivers, Vec::new(), 12).expect("the barrels");
        barrels.advance(0.5 / 60.0);
        assert_eq!(barrels.tick, 0);
        assert_eq!(barrels.transforms().len(), 13);
        assert!(barrels.floaters(Vec2::ZERO).len() >= 12);
        barrels.wakes(Vec2::ZERO);
        barrels.splashes(&mut Vec::new());
    }

    #[test]
    fn one_floater_in_eight_is_a_log_and_one_a_crate_the_towed_and_dropped_barrels() {
        // Twelve, and the dropped one: logs at 3, crates at 6; 11 is the towed barrel.
        assert_eq!(Barrels::movers(12), [11, 1, 1]);
        assert_eq!(Barrels::movers(1), [1, 0, 0]);
        assert_eq!(Barrels::movers(100), [77, 12, 12]);
        assert_eq!(Float::of(99, 100), Float::Barrel);
    }

    #[test]
    fn the_same_run_twice_gives_the_same_digest() {
        assert_eq!(run().2, run().2);
    }
}
