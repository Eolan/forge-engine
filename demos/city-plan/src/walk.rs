//! The player walking the plan's streets in first person (D-059's step 4). Enter puts a Jolt
//! character on the ground under the camera and the camera at its eyes; WASD walks along the
//! view and Shift runs (`city_blocks::wished_walk`, as on the island); Enter again frees the
//! camera. The masses round it are its walls, each its footprint's convex hull raised to its
//! roof, kept within [`REACH`] as it goes; the ground is one flat slab at the streets' level.

use std::collections::HashMap;

use anyhow::Result;
use city_blocks::character::Player;
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Shape, World, WorldDesc};
use forge_sim::TICK;
use glam::{DVec3, Vec2, Vec3};

/// How far round the walker its walls are kept, metres.
const REACH: f32 = 80.0;
/// Its eyes over its feet, metres.
pub const EYES: f32 = 1.65;
/// The streets' level, metres: what the walker stands on.
const LEVEL: f32 = 0.16;
/// The walls' cells, metres a side.
const CELL: f32 = 50.0;
/// Ticks run at most in a frame; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;

/// The masses as walls to walk against: each footprint (the world's x and z) and its roof's
/// height, found by cells.
pub struct Walls {
    walls: Vec<(Vec<Vec2>, f32)>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Walls {
    /// The walls of `masses`: footprints in the world's (x, z) and roof heights.
    pub fn new(masses: Vec<(Vec<Vec2>, f32)>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, (footprint, _)) in masses.iter().enumerate() {
            let c = footprint.iter().copied().sum::<Vec2>() / footprint.len().max(1) as f32;
            cells.entry(cell(c)).or_default().push(i as u32);
        }
        Self {
            walls: masses,
            cells,
        }
    }

    /// The walls whose footprints' centres lie within `r` of `p`.
    fn near(&self, p: Vec2, r: f32) -> Vec<u32> {
        let (lo, hi) = (cell(p - Vec2::splat(r)), cell(p + Vec2::splat(r)));
        let mut out = Vec::new();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                if let Some(list) = self.cells.get(&(x, z)) {
                    out.extend(list.iter().copied().filter(|&i| {
                        let f = &self.walls[i as usize].0;
                        let c = f.iter().copied().sum::<Vec2>() / f.len().max(1) as f32;
                        c.distance(p) <= r
                    }));
                }
            }
        }
        out
    }
}

/// The cell of a point.
fn cell(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

/// The walker, its world and the walls it keeps.
pub struct Walker {
    world: World,
    player: Player,
    character: CharacterId,
    kept: HashMap<u32, BodyId>,
    /// Seconds not yet ticked.
    owed: f64,
}

impl Walker {
    /// A walker on the ground under `at` (the world's x and z).
    pub fn new(walls: &Walls, at: Vec3) -> Result<Self> {
        let mut world = World::new(&WorldDesc::default());
        let slab = Shape::cuboid(Vec3::new(20_000.0, 1.0, 20_000.0), 0.05, 1000.0)?;
        world.add_body(&BodyDesc::fixed(
            &slab,
            DVec3::new(0.0, f64::from(LEVEL) - 1.0, 0.0),
        ))?;
        let feet = DVec3::new(f64::from(at.x), f64::from(LEVEL) + 0.01, f64::from(at.z));
        let character = world.add_character(&CharacterDesc {
            position: feet,
            ..CharacterDesc::default()
        });
        let mut walker = Self {
            world,
            player: Player {
                character: Some(character),
                ..Player::default()
            },
            character,
            kept: HashMap::new(),
            owed: 0.0,
        };
        walker.keep(walls)?;
        tracing::info!(
            at = %format_args!("{:.1},{:.1}", at.x, at.z),
            walls = walker.kept.len(),
            "the walker in the streets (D-059's step 4)"
        );
        Ok(walker)
    }

    /// Its feet, the world's metres.
    pub fn feet(&self) -> Vec3 {
        self.world.character(self.character).position.as_vec3()
    }

    /// Walks `walk` (m/s along the ground, the world's x and z) for the ticks `dt` seconds owe.
    pub fn advance(&mut self, walls: &Walls, walk: [f32; 2], dt: f64) {
        self.player.walk = walk;
        self.owed += dt;
        let tick = f64::from(TICK);
        let due = (self.owed / tick).floor();
        self.owed -= due * tick;
        for _ in 0..(due as u32).min(CATCH_UP) {
            self.player.tick(&mut self.world, None, 0, TICK, |_| None);
        }
        if let Err(e) = self.keep(walls) {
            tracing::warn!("the walker's walls: {e}");
        }
    }

    /// The walls within reach as bodies, the others dropped.
    fn keep(&mut self, walls: &Walls) -> Result<()> {
        let feet = self.feet();
        let p = Vec2::new(feet.x, feet.z);
        let wanted = walls.near(p, REACH);
        let mut changed = false;
        for &i in &wanted {
            if self.kept.contains_key(&i) {
                continue;
            }
            let (footprint, height) = &walls.walls[i as usize];
            let points: Vec<Vec3> = footprint
                .iter()
                .flat_map(|q| [Vec3::new(q.x, 0.0, q.y), Vec3::new(q.x, *height, q.y)])
                .collect();
            let Ok(shape) = Shape::convex_hull(&points, 0.05, 1000.0) else {
                continue;
            };
            let body = self.world.add_body(&BodyDesc::fixed(&shape, DVec3::ZERO))?;
            self.kept.insert(i, body);
            changed = true;
        }
        let far: Vec<u32> = self
            .kept
            .keys()
            .copied()
            .filter(|i| {
                let f = &walls.walls[*i as usize].0;
                let c = f.iter().copied().sum::<Vec2>() / f.len().max(1) as f32;
                c.distance(p) > REACH * 1.3
            })
            .collect();
        for i in far {
            if let Some(body) = self.kept.remove(&i) {
                self.world.remove_body(body);
                changed = true;
            }
        }
        if changed {
            self.world.optimize_broad_phase();
        }
        Ok(())
    }
}
