//! The island's walker (issue #196): the lab's walking character (#139) on the island, Enter
//! from the fly camera putting it on the ground below and Enter again taking the camera back. It
//! walks on Jolt height fields of the ground as the tiles draw it, their refined cells included
//! (the coast's, the channels' and the lakes' shores), cut in tiles round it as it goes, and
//! ticks at 60 Hz. Its capsule and visor are the scene's last two movers, parked under the
//! island while nobody walks.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Shape, World, WorldDesc};
use forge_procgen::Field2;
use forge_render::MoverTransform;
use forge_sim::TICK;
use glam::{DVec2, DVec3, Quat, Vec2, Vec3};

use crate::DrawnGround;
use crate::lab::materials::{ISLAND_ROWS, SOLE, pair, traction};
use crate::lab::walk::Player;

/// A tile's samples a side: 63 cells, 32 of Jolt's blocks of two each way (a power of two, as
/// its range tree wants, #194).
const SAMPLES: u32 = 64;
/// How far round the walker its ground is kept, metres each way: the tiles that square touches.
const REACH: f64 = 24.0;
/// Where the walker's capsule and visor wait while nobody walks: far under the island.
const PARKED: Vec3 = Vec3::new(0.0, -5000.0, 0.0);
/// Ticks run at most in a frame; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;

/// How far either foot comes down from the walker's way, metres (#197).
const FOOT_APART: f64 = 0.1;

/// A footfall (#197): where a foot came down, the scene's (x, z), and the way it pointed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Footfall {
    pub at: DVec2,
    pub heading: Vec2,
}

/// The walker, its world and its ground.
pub(crate) struct Walker {
    world: World,
    ground: Arc<DrawnGround>,
    /// Metres between the tiles' samples: the drawn ground's finest, its refined cells'.
    fine: f64,
    /// Half the field's extent: the scene's frame is the field's less this.
    half: f64,
    player: Player,
    character: CharacterId,
    /// The ground's tiles as Jolt holds them, by their first sample's index over `SAMPLES − 1`.
    tiles: BTreeMap<(u32, u32), BodyId>,
    /// Seconds not yet ticked.
    owed: f64,
    /// Its feet at the last two ticks.
    feet: [DVec3; 2],
    /// Milliseconds the ticks took since the last title, the tiles' cuts among them, and the
    /// most its feet stood off the drawn ground while on it (metres).
    tick_ms: Vec<f64>,
    cut_ms: Vec<f64>,
    off: f64,
    /// Its footfalls since they were last taken (#197), and its stride.
    footfalls: Vec<Footfall>,
    stride: Stride,
    /// The island's layer map (`island_layer`), whose rows grip its boots (#206); without one,
    /// any ground grips at once. And the layer under its feet at the last tick.
    layers: Option<Arc<Field2<u8>>>,
    underfoot: Option<u8>,
}

/// A walker's stride (#197; the lab's materials scene's too, #205): the metres walked since the
/// last footfall, and whether the next is the left foot's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stride {
    pub walked: f64,
    pub left: bool,
}

impl Default for Stride {
    fn default() -> Self {
        Self {
            walked: 0.0,
            left: true,
        }
    }
}

impl Stride {
    /// Its feet's way on firm ground over a tick of `dt` seconds, from `from` to `to`: a
    /// footfall every stride, left and right of its way in turn, the stride longer the faster it
    /// goes (0.9 m walking, 1.4 m running).
    pub(crate) fn step(&mut self, from: DVec3, to: DVec3, dt: f64) -> Option<Footfall> {
        let way = DVec2::new(to.x - from.x, to.z - from.z);
        let along = way.length();
        if along < 1e-6 {
            return None;
        }
        self.walked += along;
        let pace = along / dt;
        let stride = 0.45 + 0.15 * pace;
        if self.walked < stride {
            return None;
        }
        self.walked -= stride;
        let heading = way / along;
        // Its right, along the ground, seen from above (+y): −z ahead, +x right.
        let right = DVec2::new(-heading.y, heading.x);
        let side = if self.left { -FOOT_APART } else { FOOT_APART };
        self.left = !self.left;
        Some(Footfall {
            at: DVec2::new(to.x, to.z) + right * side,
            heading: heading.as_vec2(),
        })
    }
}

impl Walker {
    /// A walker standing on the ground under `at` (the scene's x and z), gripped by the rows of
    /// `layers`' ground (#206).
    pub(crate) fn new(
        ground: Arc<DrawnGround>,
        at: Vec3,
        layers: Option<Arc<Field2<u8>>>,
    ) -> Result<Self> {
        let fine = ground.spacing / f64::from(ground.detail.split.max(1));
        let half = 0.5 * f64::from(ground.size - 1) * ground.spacing;
        let mut world = World::new(&WorldDesc::default());
        let (x, z) = (f64::from(at.x), f64::from(at.z));
        let y = ground.surface_at(x + half, z + half);
        let feet = DVec3::new(x, y, z);
        let character = world.add_character(&CharacterDesc {
            position: feet,
            ..CharacterDesc::default()
        });
        let mut walker = Self {
            world,
            ground,
            fine,
            half,
            player: Player {
                character: Some(character),
                ..Player::default()
            },
            character,
            tiles: BTreeMap::new(),
            owed: 0.0,
            feet: [feet; 2],
            tick_ms: Vec::new(),
            cut_ms: Vec::new(),
            off: 0.0,
            footfalls: Vec::new(),
            stride: Stride::default(),
            layers,
            underfoot: None,
        };
        walker.keep_tiles(feet)?;
        tracing::info!(
            at = %format_args!("{x:.1},{y:.2},{z:.1}"),
            tiles = walker.tiles.len(),
            cut_ms = %format_args!("{:.2}", walker.cut_ms.iter().sum::<f64>()),
            "the walker on the island's ground (#196)"
        );
        Ok(walker)
    }

    /// The ground as drawn at the scene's (x, z).
    pub(crate) fn ground_at(&self, x: f64, z: f64) -> f64 {
        self.ground.surface_at(x + self.half, z + self.half)
    }

    /// Its walk from now on: m/s along the ground, world x and z.
    pub(crate) fn walk(&mut self, walk: [f32; 2]) {
        self.player.walk = walk;
    }

    /// A jump at the next tick, if on firm ground.
    pub(crate) fn jump(&mut self) {
        self.player.jump = true;
    }

    /// Runs the ticks `dt` seconds owe (one a frame with `fixed`).
    pub(crate) fn advance(&mut self, dt: f64, fixed: bool) {
        let tick = f64::from(TICK);
        let ticks = if fixed {
            self.owed = 0.0;
            1
        } else {
            self.owed += dt;
            let due = (self.owed / tick).floor();
            self.owed -= due * tick;
            (due as u32).min(CATCH_UP)
        };
        for _ in 0..ticks {
            if let Err(e) = self.tick() {
                tracing::warn!("the walker's ground: {e}");
            }
        }
    }

    /// One tick: its ground's tiles round it, then the step.
    fn tick(&mut self) -> Result<()> {
        let start = Instant::now();
        let feet = self.world.character(self.character).position;
        self.keep_tiles(feet)?;
        // Its boots grip the ground as drawn under them (#206): the row's against the sole.
        self.underfoot = self.layers.as_ref().map(|layers| {
            let height = self.ground_at(feet.x, feet.z) as f32;
            crate::island_sand::drawn_layer(layers, DVec2::new(feet.x, feet.z), height)
        });
        let grip = self
            .underfoot
            .and_then(|layer| ISLAND_ROWS.get(usize::from(layer)))
            .map(|row| traction(&row.physics, TICK));
        self.player.tick(&mut self.world, None, 0, TICK, |_| grip);
        let s = self.world.character(self.character);
        if s.ground == forge_physics::Ground::Firm {
            let off = s.position.y - self.ground_at(s.position.x, s.position.z);
            if off.abs() > self.off.abs() {
                self.off = off;
            }
        }
        if s.ground == forge_physics::Ground::Firm {
            self.step(self.feet[1], s.position);
        }
        self.feet = [self.feet[1], s.position];
        self.tick_ms.push(start.elapsed().as_secs_f64() * 1e3);
        Ok(())
    }

    /// Its feet's way on firm ground over a tick, from `from` to `to` (#197): its stride's
    /// footfalls.
    fn step(&mut self, from: DVec3, to: DVec3) {
        if let Some(fall) = self.stride.step(from, to, f64::from(TICK)) {
            self.footfalls.push(fall);
        }
    }

    /// Its footfalls since the last call (#197).
    pub(crate) fn take_footfalls(&mut self) -> Vec<Footfall> {
        std::mem::take(&mut self.footfalls)
    }

    /// Its feet as the last tick left them, (x, z).
    pub(crate) fn feet_xz(&self) -> DVec2 {
        DVec2::new(self.feet[1].x, self.feet[1].z)
    }

    /// Cuts the tiles of ground within `REACH` of `feet` that Jolt lacks, and drops the others.
    fn keep_tiles(&mut self, feet: DVec3) -> Result<()> {
        let cells = f64::from(SAMPLES - 1);
        let last = (f64::from(self.ground.size - 1) * self.ground.spacing / self.fine / cells)
            .ceil()
            .max(1.0)
            - 1.0;
        let span = |c: f64| {
            let low = ((c + self.half - REACH) / self.fine / cells)
                .floor()
                .clamp(0.0, last);
            let high = ((c + self.half + REACH) / self.fine / cells)
                .floor()
                .clamp(0.0, last);
            low as u32..=high as u32
        };
        let wanted: Vec<(u32, u32)> = span(feet.z)
            .flat_map(|tz| span(feet.x).map(move |tx| (tx, tz)))
            .collect();
        let gone: Vec<(u32, u32)> = self
            .tiles
            .keys()
            .filter(|t| !wanted.contains(t))
            .copied()
            .collect();
        for t in gone {
            if let Some(body) = self.tiles.remove(&t) {
                self.world.remove_body(body);
            }
        }
        let mut cut = false;
        for t in wanted {
            if self.tiles.contains_key(&t) {
                continue;
            }
            let start = Instant::now();
            let (shape, position) = self.tile(t)?;
            let body = self.world.add_body(&BodyDesc {
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                ..BodyDesc::fixed(&shape, position)
            })?;
            self.tiles.insert(t, body);
            self.cut_ms.push(start.elapsed().as_secs_f64() * 1e3);
            cut = true;
        }
        if cut {
            self.world.optimize_broad_phase();
        }
        Ok(())
    }

    /// Tile `(tx, tz)` as a fixed body's shape, and where the body stands. Jolt splits a quad
    /// along its (x, y)–(x + 1, y + 1) diagonal and the drawn ground along (i + 1, j)–(i, j + 1),
    /// so the field is laid a quarter turn about +y (its local x along the scene's −z, its local
    /// z along +x): its quads then split as the ground's do. A tile's samples a metre apart lie
    /// on the drawn triangles (a coarse cell's diagonal runs through their corners), so Jolt's
    /// ground is the drawn ground, to its height's compression.
    fn tile(&self, (tx, tz): (u32, u32)) -> Result<(Shape, DVec3)> {
        let n = SAMPLES;
        let (i0, j0) = (tx * (n - 1), tz * (n - 1));
        let scene = |i: u32| f64::from(i) * self.fine - self.half;
        // Sample (a, b) of the field, local x = a and z = b, stands at the ground's fine
        // sample (i0 + b, j0 + n − 1 − a).
        let samples: Vec<f32> = (0..n * n)
            .map(|k| {
                let (a, b) = (k % n, k / n);
                self.ground_at(scene(i0 + b), scene(j0 + n - 1 - a)) as f32
            })
            .collect();
        let fine = self.fine as f32;
        let shape = Shape::height_field(&samples, n, Vec3::ZERO, Vec3::new(fine, 1.0, fine))?;
        Ok((shape, DVec3::new(scene(i0), 0.0, scene(j0 + n - 1))))
    }

    /// Its feet as drawn: between the last two ticks by the time not yet ticked.
    pub(crate) fn feet(&self) -> DVec3 {
        let t = (self.owed / f64::from(TICK)).clamp(0.0, 1.0);
        self.feet[0].lerp(self.feet[1], t)
    }

    /// Its capsule's and its visor's movers, turned where it last walked.
    pub(crate) fn transforms(&self) -> [MoverTransform; 2] {
        let at = MoverTransform {
            position: self.feet().as_vec3(),
            rotation: self.player.rotation(),
            scale: 1.0,
        };
        [at, at]
    }

    /// The movers while nobody walks.
    pub(crate) fn parked() -> [MoverTransform; 2] {
        [MoverTransform {
            position: PARKED,
            rotation: Quat::IDENTITY,
            scale: 1.0,
        }; 2]
    }

    /// The title's part: its ticks' time since the last title, its tiles, the cuts' time, and
    /// the most its feet stood off the drawn ground.
    pub(crate) fn title(&mut self) -> String {
        let n = self.tick_ms.len().max(1) as f64;
        let mean = self.tick_ms.iter().sum::<f64>() / n;
        let max = self.tick_ms.iter().copied().fold(0.0, f64::max);
        let cut = self.cut_ms.iter().copied().fold(0.0, f64::max);
        // The ground under its feet and the boots' grip on it (#206).
        let on = self
            .underfoot
            .and_then(|layer| ISLAND_ROWS.get(usize::from(layer)))
            .map_or_else(String::new, |row| {
                let mu = pair(SOLE.dynamic_friction, row.physics.dynamic_friction);
                format!(", on {} (μ {mu:.2})", row.name)
            });
        let title = format!(
            "walker {mean:.3} ms a tick (max {max:.3}), {} tiles, cut in {cut:.2} ms at most, feet {:+.1} cm off the ground{on}",
            self.tiles.len(),
            100.0 * self.off
        );
        self.tick_ms.clear();
        self.cut_ms.clear();
        self.off = 0.0;
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_geom::city::HeightfieldDetail;

    /// A field of 33 × 33 samples 2 m apart, rough (each sample its own height, so the cells'
    /// diagonals fold), with a block of refined cells split in two.
    fn rough() -> Arc<DrawnGround> {
        let size = 33u32;
        let height = |i: u32, j: u32| {
            let h = (i.wrapping_mul(7919) ^ j.wrapping_mul(104_729)) % 97;
            0.003 * h as f32 + 0.2 * i as f32
        };
        let heights: Vec<f32> = (0..size * size)
            .map(|k| height(k % size, k / size))
            .collect();
        let split = 2;
        let side = size - 1;
        let mut cells = Vec::new();
        let mut fine = Vec::new();
        for j in 10..14 {
            for i in 12..18 {
                cells.push(j * side + i);
                for v in 0..=split {
                    for u in 0..=split {
                        // On the edges, on the coarse edges (as the island's are: a coarse
                        // neighbour's triangles meet them); inside, bumped off the cell's plane.
                        let [a, b, c, d] =
                            [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(di, dj)| height(i + di, j + dj));
                        let (s, t) = (u as f32 / split as f32, v as f32 / split as f32);
                        let along = if s + t <= 1.0 {
                            a + s * (b - a) + t * (c - a)
                        } else {
                            d + (1.0 - s) * (c - d) + (1.0 - t) * (b - d)
                        };
                        let edge = u == 0 || u == split || v == 0 || v == split;
                        let h = if edge { along } else { along - 0.15 };
                        fine.push(h);
                    }
                }
            }
        }
        Arc::new(DrawnGround {
            size,
            spacing: 2.0,
            heights: heights.into(),
            detail: Arc::new(HeightfieldDetail {
                split,
                cells,
                heights: fine,
            }),
        })
    }

    /// A flat field of 33 × 33 samples 2 m apart, `height` metres over the sea.
    fn flat(height: f32) -> Arc<DrawnGround> {
        let size = 33u32;
        Arc::new(DrawnGround {
            size,
            spacing: 2.0,
            heights: vec![height; (size * size) as usize].into(),
            detail: Arc::new(HeightfieldDetail {
                split: 1,
                cells: Vec::new(),
                heights: Vec::new(),
            }),
        })
    }

    #[test]
    fn the_walker_speeds_up_as_its_grip_on_the_layer_allows() {
        use crate::island_layer;
        let ground = flat(20.0);
        // Its speed along x ten ticks after it starts walking 3 m/s from rest, and a second
        // later, on a layer map all of one layer.
        let speed_up = |layer: u8| {
            let layers = Arc::new(Field2::from_fn(64, 1.0, |_, _| layer));
            let mut walker = Walker::new(ground.clone(), Vec3::ZERO, Some(layers)).unwrap();
            for _ in 0..10 {
                walker.advance(0.0, true);
            }
            walker.walk([3.0, 0.0]);
            for _ in 0..10 {
                walker.advance(0.0, true);
            }
            let ten = (walker.feet[1].x - walker.feet[0].x) / f64::from(TICK);
            for _ in 0..60 {
                walker.advance(0.0, true);
            }
            let later = (walker.feet[1].x - walker.feet[0].x) / f64::from(TICK);
            (walker.underfoot, ten, later)
        };
        let mut speeds = Vec::new();
        for layer in [
            island_layer::ROCK,
            island_layer::GRASS,
            island_layer::LAKEBED,
        ] {
            let (underfoot, ten, later) = speed_up(layer);
            assert_eq!(
                underfoot,
                Some(layer),
                "the walker reads the layer under it"
            );
            let row = &ISLAND_ROWS[usize::from(layer)];
            let expected = f64::from(pair(SOLE.dynamic_friction, row.physics.dynamic_friction))
                * 9.81
                * 10.0
                * f64::from(TICK);
            assert!(
                (ten - expected).abs() < 0.1 * expected,
                "on {}: {ten:.3} m/s after ten ticks, the row allows {expected:.3}",
                row.name
            );
            assert!(
                (later - 3.0).abs() < 0.05,
                "on {}: {later:.3} m/s",
                row.name
            );
            speeds.push(ten);
        }
        assert!(speeds[0] > speeds[1] && speeds[1] > speeds[2], "{speeds:?}");
        // Without a layer map, any ground grips at once, as before.
        let mut walker = Walker::new(ground.clone(), Vec3::ZERO, None).unwrap();
        for _ in 0..10 {
            walker.advance(0.0, true);
        }
        walker.walk([3.0, 0.0]);
        for _ in 0..2 {
            walker.advance(0.0, true);
        }
        let at_once = (walker.feet[1].x - walker.feet[0].x) / f64::from(TICK);
        assert!((at_once - 3.0).abs() < 0.05, "{at_once:.3} m/s");
    }

    #[test]
    fn the_island_rows_follow_the_layers_ids() {
        use crate::island_layer;
        let name = |layer: u8| ISLAND_ROWS[usize::from(layer)].name;
        assert_eq!(name(island_layer::GRASS), "grass");
        assert_eq!(name(island_layer::SAND), "dry sand");
        assert_eq!(name(island_layer::ROCK), "granite");
        assert_eq!(name(island_layer::LAKEBED), "mud");
        assert_eq!(name(island_layer::LAKE_SAND), "silty sand");
        assert_eq!(name(island_layer::LIMESTONE), "limestone");
        assert_eq!(name(island_layer::GRUS), "grus");
        // The beach's dry sand is the materials table's sand row.
        let sand = &ISLAND_ROWS[usize::from(island_layer::SAND)].physics;
        assert_eq!(
            sand.dynamic_friction,
            crate::lab::MATERIAL_ROWS[2].physics.dynamic_friction
        );
    }

    #[test]
    fn the_walkers_ground_is_the_drawn_ground_and_it_walks_on_it() {
        let ground = rough();
        let mut walker = Walker::new(ground.clone(), Vec3::new(-2.5, 0.0, -9.3), None).unwrap();
        // Rays down onto Jolt's tiles at points all over the field, the refined cells among
        // them, meet the drawn ground within its compression (a few millimetres).
        let mut worst: f64 = 0.0;
        for k in 0..400u32 {
            let (x, z) = (
                -30.0 + 0.151 * f64::from(k * 37 % 400),
                -30.0 + 0.149 * f64::from(k * 91 % 400),
            );
            let top = 100.0;
            let hit = walker
                .world
                .cast_ray(DVec3::new(x, top, z), Vec3::new(0.0, -200.0, 0.0))
                .expect("the ground under a ray");
            let y = top - 200.0 * f64::from(hit.fraction);
            worst = worst.max((y - walker.ground_at(x, z)).abs());
        }
        assert!(worst < 0.01, "Jolt's ground {worst} m off the drawn");
        // Walking 3 m/s along +x for two seconds: it goes on, standing on the ground.
        walker.walk([3.0, 0.0]);
        for _ in 0..120 {
            walker.advance(0.0, true);
        }
        let feet = walker.feet();
        assert!(feet.x > 2.0, "at {feet}");
        assert!(
            walker.off.abs() < 0.05,
            "feet {} m off the ground",
            walker.off
        );
    }
}
