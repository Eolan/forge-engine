//! The walking player (#139, #196): its input as the world holds it, its tick through Jolt's
//! `CharacterVirtual` (`forge-physics`) and its props, a capsule and a visor. The physics lab's
//! playground and the island's walker both walk it.

use std::sync::Arc;

use forge_geom::city::{Block, Imported, Lathe, PropKind, PropSpec, block};
use forge_physics::{BodyId, CharacterId, Ground, Velocity, World};
use glam::{Quat, Vec3};

/// The playground's platform: its speed (m/s) and the ticks it runs each way.
const PLATFORM_SPEED: f32 = 2.0;
const PLATFORM_TICKS: u64 = 360;
/// The player's walk, m/s.
pub const WALK_SPEED: f32 = 3.0;
/// The player's run, m/s.
pub const RUN_SPEED: f32 = 6.5;
/// How fast the player leaves the ground jumping, m/s.
const JUMP_SPEED: f32 = 5.0;
/// Gravity, m/s².
const GRAVITY: f32 = 9.81;

/// The player's props, the island's walker's too (#196): its capsule and its visor.
pub fn player_props() -> [PropSpec; 2] {
    // The visor: a band across the face, so one sees where the player looks.
    let mut visor = block(&Block {
        half: [0.16, 0.05, 0.05],
        radius: 0.02,
        segments: 2,
    });
    for p in &mut visor.positions {
        p[1] += 1.55;
        p[2] -= 0.27;
    }
    [
        PropSpec {
            name: "lab-player".to_owned(),
            // A capsule 1.8 m tall and 0.3 m round, standing on its feet.
            kind: PropKind::Lathe(Lathe {
                profile: (0..=24)
                    .map(|k| {
                        let a = std::f32::consts::PI * k as f32 / 24.0;
                        let (s, c) = (a.sin(), a.cos());
                        // The bottom half-sphere, the cylinder, the top half-sphere.
                        if k <= 12 {
                            (0.3 * s, 0.3 - 0.3 * c)
                        } else {
                            (0.3 * s, 1.5 - 0.3 * c)
                        }
                    })
                    .collect(),
                around: 32,
                along: 40,
                flutes: 0,
                flute_depth: 0.0,
                flute_span: (0.0, 0.0),
            }),
        },
        PropSpec {
            name: "lab-visor".to_owned(),
            kind: PropKind::Imported(Imported {
                key: "lab-visor 1".to_owned(),
                mesh: Arc::new(visor),
                normal_weight: None,
            }),
        },
    ]
}

/// The player's input as the world holds it: the walk (m/s along the ground, world x and z)
/// held until the next command, and a jump waiting for the next tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Player {
    /// Its character in the world, once made.
    pub character: Option<CharacterId>,
    /// The walk, m/s along the ground (world x and z), held until the next command.
    pub walk: [f32; 2],
    /// A jump waiting for the next tick.
    pub jump: bool,
    /// Where it last walked (unit, x and z), for its facing.
    pub facing: [f32; 2],
}

impl Player {
    /// One tick: the platform's way, then the player moved as a game moves it (on firm ground
    /// it takes the ground's velocity and may jump; in the air it keeps its fall). `traction`
    /// gives a ground body's grip (#203): the most the feet change the speed on it in the tick,
    /// m/s; a ground without one is gripped at once.
    pub fn tick(
        &mut self,
        world: &mut World,
        platform: Option<BodyId>,
        tick: u64,
        dt: f32,
        traction: impl Fn(BodyId) -> Option<f32>,
    ) {
        if let Some(platform) = platform {
            let way = if (tick / PLATFORM_TICKS).is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            world.set_velocity(
                platform,
                Velocity {
                    linear: Vec3::new(way * PLATFORM_SPEED, 0.0, 0.0),
                    angular: Vec3::ZERO,
                },
            );
        }
        let Some(c) = self.character else {
            return;
        };
        let s = world.character(c);
        let mut v = Vec3::new(self.walk[0], 0.0, self.walk[1]);
        if s.ground == Ground::Firm {
            v += s.ground_velocity;
            // On a ground with a material row, the feet push as hard as it grips: on ice the
            // player starts slowly and slides when it stops (#203).
            if let Some(most) = s.ground_body.and_then(&traction) {
                let now = Vec3::new(s.velocity.x, 0.0, s.velocity.z);
                let wished = Vec3::new(v.x, 0.0, v.z);
                let flat = now + (wished - now).clamp_length_max(most);
                v = Vec3::new(flat.x, v.y, flat.z);
            }
            if self.jump {
                v.y = JUMP_SPEED;
            }
        } else {
            v.y = s.velocity.y;
        }
        v.y -= GRAVITY * dt;
        world.move_character(c, dt, v);
        self.jump = false;
        let flat = Vec3::new(self.walk[0], 0.0, self.walk[1]);
        if flat.length_squared() > 1e-4 {
            let unit = flat.normalize();
            self.facing = [unit.x, unit.z];
        }
    }

    /// Its rotation: −z turned to where it last walked (no trigonometry: the arc between).
    pub fn rotation(&self) -> Quat {
        let f = Vec3::new(self.facing[0], 0.0, self.facing[1]);
        if f.length_squared() < 0.5 {
            return Quat::IDENTITY;
        }
        if f.dot(Vec3::NEG_Z) < -0.9999 {
            return Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);
        }
        Quat::from_rotation_arc(Vec3::NEG_Z, f)
    }
}
