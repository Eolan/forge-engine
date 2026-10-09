//! `physics-lab --lab materials` (issue #203, Phase 3's materials, D-007's one material record):
//! physics reads the material row. Five patches lie along the walker's way, brick, wood, sand,
//! snow and a long sheet of ice, each body taking its friction and restitution from the one
//! table here, which the patches' drawn rows carry too (`CityMaterials`). On each patch a wooden
//! crate waits on a 20° ramp of the patch's material and a ball with no bounce of its own falls
//! from 2 m; the walker crosses them all on its autopilot and stops on the ice, where its feet
//! find little grip. What each does is measured against its rows: the crates' slides, the
//! balls' bounces and the walker's slide, in the log.

use anyhow::Result;
use forge_core::dmath::sin_cos;
use forge_core::material::{MaterialTags, PhysicsLayer};
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::deform::{Layer, Pad, Soft};
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Shape, World};
use glam::{DVec2, DVec3, Mat4, Quat, Vec3};

use super::yard::Bed;
use crate::island_walk::{Footfall, Stride};

/// A material's row as physics reads it: its name (the prop drawn with it), its physical layer
/// and its tags.
pub(crate) struct Row {
    pub name: &'static str,
    pub physics: PhysicsLayer,
    pub tags: MaterialTags,
}

/// The patches' rows, in the order the walker meets them. Friction is each material's against
/// itself: a pair combines by the geometric mean, restitution by the larger (Jolt's rules, the
/// table's `combine` for now). Jolt holds one coefficient a body, so the bodies take the dynamic
/// one; the static stays for the walker's first step and for later.
pub(crate) const ROWS: [Row; 5] = [
    Row {
        name: "lab-mat-brick",
        physics: PhysicsLayer {
            density: 1900.0,
            static_friction: 0.7,
            dynamic_friction: 0.6,
            restitution: 0.4,
        },
        tags: MaterialTags(0),
    },
    Row {
        name: "lab-mat-wood",
        physics: PhysicsLayer {
            density: 600.0,
            static_friction: 0.5,
            dynamic_friction: 0.4,
            restitution: 0.45,
        },
        tags: MaterialTags::FLAMMABLE,
    },
    Row {
        name: "lab-mat-sand",
        physics: PhysicsLayer {
            density: 1600.0,
            static_friction: 0.6,
            dynamic_friction: 0.5,
            restitution: 0.05,
        },
        tags: MaterialTags::DEFORMABLE,
    },
    Row {
        name: "lab-mat-snow",
        physics: PhysicsLayer {
            density: 400.0,
            static_friction: 0.3,
            dynamic_friction: 0.2,
            restitution: 0.1,
        },
        tags: MaterialTags(MaterialTags::DEFORMABLE.0 | MaterialTags::SLIPPERY.0),
    },
    Row {
        // Wet ice near melting, the slipperiest.
        name: "lab-mat-ice",
        physics: PhysicsLayer {
            density: 917.0,
            static_friction: 0.05,
            dynamic_friction: 0.02,
            restitution: 0.6,
        },
        tags: MaterialTags::SLIPPERY,
    },
];

/// The walker's boots: a rubber sole.
pub(crate) const SOLE: PhysicsLayer = PhysicsLayer {
    density: 1100.0,
    static_friction: 0.9,
    dynamic_friction: 0.8,
    restitution: 0.5,
};

/// The crates: planks of the wood row.
const CRATE_WOOD: usize = 1;

/// Gravity, m/s².
const GRAVITY: f32 = 9.81;

/// A patch's half sizes, metres: 4 m across the way, 6 m along it, 4 cm thick.
pub(super) const PATCH_HALF: [f32; 3] = [2.0, 0.02, 3.0];
/// The first patch's middle along z; the ice takes three patches' length.
const FIRST_Z: f64 = -12.0;
const ICE_PATCHES: usize = 3;
/// The ramps: their slope, degrees, and where their middles stand across the way.
const RAMP_DEGREES: f64 = 20.0;
const RAMP_X: f64 = 5.0;
/// How far up its ramp from the middle a crate starts, metres.
const CRATE_UP: f64 = 1.2;
/// The balls: where they fall, across the way, and from how high (their bottoms over the
/// patch), metres.
const BALL_X: f64 = -1.3;
const BALL_DROP: f64 = 2.0;
/// The walker: where it starts, its pace on the autopilot (m/s, the playground's walk) and where
/// it stops walking, on the ice.
const WALKER_START_Z: f64 = -17.0;
pub(super) const PACE: f32 = 3.0;
const STOP_Z: f64 = 12.0;
/// Ticks a second, and the tick at which the crates' slides are measured (half a second in).
const RATE: f64 = 60.0;
const MEASURE_TICK: u64 = 30;

/// The beds the walker's boots press (#205): the sand's and the snow's patches, their half
/// width (a centimetre in from the patch's edges), their points' spacing, metres, and how far
/// in from their edges they thin to nothing, metres. A strip 1.6 m wide along the walker's way
/// drew its 10 cm bevel as a hard line beside it.
const BED_HALF_WIDTH: f64 = 1.98;
const BED_CELL: f32 = 0.02;
const BED_BEVEL: f32 = 0.25;

/// The patches the walker leaves prints in (#205): the sand, the island's dry sand (#197), and
/// the snow, the yard's fresh snow.
const PRINTED: [(usize, Soft); 2] = [(2, crate::island_sand::SOFT), (3, Soft::SNOW)];

/// The beds, untouched (#205): each printed patch a centimetre in from its edges, lying on it
/// as the yard's lie on the floor (its ground just under the patch's top, its edges thinning to
/// nothing there), drawn with the patch's row: loose sand or fresh snow a few centimetres
/// deep. Flush with the patch, its prints sank under the patch's own top, which hid them.
pub(super) fn beds() -> Vec<Bed> {
    let length = f64::from(2.0 * PATCH_HALF[2]);
    let span = length - 2.0 * (f64::from(PATCH_HALF[0]) - BED_HALF_WIDTH);
    let top = 2.0 * PATCH_HALF[1];
    PRINTED
        .iter()
        .map(|&(k, soft)| {
            let low = DVec2::new(-BED_HALF_WIDTH, FIRST_Z + length * k as f64 - 0.5 * span);
            let size = [
                (2.0 * BED_HALF_WIDTH / f64::from(BED_CELL)).round() as u32 + 1,
                (span / f64::from(BED_CELL)).round() as u32 + 1,
            ];
            Bed {
                layer: Layer::with_bevel(soft, low, BED_CELL, size, BED_BEVEL),
                name: ROWS[k].name,
                grip: None,
                base: top - super::yard::UNDER,
            }
        })
        .collect()
}

/// The walker's boot coming down at `fall` (#205): pressed into the bed under it, if any
/// (the island's boot, 70 kg on an ellipse of 5 by 13 cm half sizes). Whether it was.
pub(super) fn press(beds: &mut [Bed], fall: Footfall) -> bool {
    let Some(bed) = beds.iter_mut().find(|b| b.layer.covers(fall.at)) else {
        return false;
    };
    bed.layer.press(Pad {
        at: fall.at,
        heading: fall.heading,
        size: crate::island_sand::FOOT,
        pressure: crate::island_sand::FOOT_PRESSURE,
        sweep: 0.0,
        wheel: 0.0,
        tread: None,
    });
    true
}

/// The pair friction of two rows' coefficients, as Jolt combines them.
pub(crate) fn pair(a: f32, b: f32) -> f32 {
    (a * b).sqrt()
}

/// The patches' props: a block per row, drawn with its row.
pub(super) fn props() -> Vec<PropSpec> {
    ROWS.iter()
        .map(|row| PropSpec {
            name: row.name.to_owned(),
            kind: PropKind::Block(Block {
                half: PATCH_HALF,
                radius: 0.01,
                segments: 4,
            }),
        })
        .collect()
}

/// A rotation about +z by `degrees`, from the deterministic sine: +x rises.
fn about_z(degrees: f64) -> Quat {
    let (s, c) = sin_cos(0.5 * degrees.to_radians());
    Quat::from_xyzw(0.0, 0.0, s as f32, c as f32)
}

/// What the scene puts in the world: its still pieces (the props, from `first`, and their
/// transforms), the crates and the balls, the walker, and what physics reads of the grounds.
pub(super) struct Field {
    pub statics: Vec<(usize, Mat4)>,
    pub crates: Vec<BodyId>,
    pub balls: Vec<BodyId>,
    pub player: CharacterId,
    pub yard: Yard,
}

/// Builds the patches into `world`: `first` is the first patch prop's index, `crate_shape` and
/// `ball_shape` the lab's (the ball's origin at its bottom).
pub(super) fn build(
    world: &mut World,
    first: usize,
    crate_shape: &Shape,
    crate_half: f32,
    ball_shape: &Shape,
) -> Result<Field> {
    let mut statics = Vec::new();
    let mut grounds = Vec::new();
    let mut crates = Vec::new();
    let mut balls = Vec::new();
    let mut crate_start = Vec::new();
    let patch_shape = Shape::cuboid(Vec3::from(PATCH_HALF), 0.01, 0.0)?;
    let top = f64::from(2.0 * PATCH_HALF[1]);
    let length = f64::from(2.0 * PATCH_HALF[2]);
    let turn = about_z(RAMP_DEGREES);
    let (ramp_sin, ramp_cos) = sin_cos(RAMP_DEGREES.to_radians());
    for (k, row) in ROWS.iter().enumerate() {
        let patches = if k + 1 == ROWS.len() { ICE_PATCHES } else { 1 };
        let z0 = FIRST_Z + length * k as f64;
        let ground = BodyDesc {
            friction: row.physics.dynamic_friction,
            restitution: row.physics.restitution,
            ..BodyDesc::fixed(&patch_shape, DVec3::ZERO)
        };
        for p in 0..patches {
            let at = DVec3::new(0.0, 0.5 * top, z0 + length * p as f64);
            grounds.push((
                world.add_body(&BodyDesc {
                    position: at,
                    ..ground
                })?,
                k,
            ));
            statics.push((first + k, Mat4::from_translation(at.as_vec3())));
        }
        // The ramp: the patch's block turned 20° about z, its low end on the floor.
        let half_run = f64::from(PATCH_HALF[0]);
        let ramp_at = DVec3::new(RAMP_X, half_run * ramp_sin + 0.5 * top, z0);
        grounds.push((
            world.add_body(&BodyDesc {
                position: ramp_at,
                rotation: turn,
                ..ground
            })?,
            k,
        ));
        statics.push((
            first + k,
            Mat4::from_rotation_translation(turn, ramp_at.as_vec3()),
        ));
        // Its crate, of the wood row, resting on it up the slope.
        let normal = DVec3::new(-ramp_sin, ramp_cos, 0.0);
        let along = DVec3::new(ramp_cos, ramp_sin, 0.0);
        let crate_at =
            ramp_at + along * CRATE_UP + normal * (0.5 * top + f64::from(crate_half) + 0.005);
        let wood = &ROWS[CRATE_WOOD].physics;
        crate_start.push(crate_at);
        crates.push(world.add_body(&BodyDesc {
            rotation: turn,
            friction: wood.dynamic_friction,
            restitution: wood.restitution,
            ..BodyDesc::dynamic(crate_shape, crate_at)
        })?);
        // Its ball, with no bounce of its own: the patch's restitution is the pair's.
        balls.push(world.add_body(&BodyDesc {
            friction: SOLE.dynamic_friction,
            restitution: 0.0,
            ..BodyDesc::dynamic(ball_shape, DVec3::new(BALL_X, top + BALL_DROP, z0))
        })?);
    }
    let player = world.add_character(&CharacterDesc {
        position: DVec3::new(0.0, 0.0, WALKER_START_Z),
        ..CharacterDesc::default()
    });
    let starts = crates.len();
    Ok(Field {
        statics,
        crates: crates.clone(),
        balls: balls.clone(),
        player,
        yard: Yard {
            stride: Stride::default(),
            grounds,
            crates,
            balls,
            player,
            crate_start,
            crate_measured: Vec::new(),
            bounces: vec![Bounce::Falling; starts],
            stop: None,
            logged: false,
        },
    })
}

/// A ball's first bounce, as it is watched.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bounce {
    Falling,
    /// Up again, the highest its bottom has stood since, metres.
    Rising(f64),
    /// Its apex over the patch, metres.
    Done(f64),
}

/// The scene's state: which row each ground body is, and what is measured (the log's, never
/// the simulation's: a replay does not need it).
pub(crate) struct Yard {
    /// The walker's stride (#205): simulation state, saved and digested with the world.
    pub stride: Stride,
    grounds: Vec<(BodyId, usize)>,
    crates: Vec<BodyId>,
    balls: Vec<BodyId>,
    player: CharacterId,
    crate_start: Vec<DVec3>,
    crate_measured: Vec<DVec3>,
    bounces: Vec<Bounce>,
    /// Where the walker stopped walking and how fast it went then, then where it came to rest.
    stop: Option<(f64, f32, Option<f64>)>,
    logged: bool,
}

impl Yard {
    /// The row of the ground body `body`, if it is one of the patches or ramps.
    pub(crate) fn row(&self, body: BodyId) -> Option<&'static Row> {
        self.grounds
            .iter()
            .find(|(b, _)| *b == body)
            .map(|&(_, k)| &ROWS[k])
    }

    /// The walk the autopilot asks at the walker's position `z`: along +z at the pace until it
    /// stands on the ice, then none.
    pub(crate) fn walk(&self, z: f64) -> [f32; 2] {
        if z < STOP_Z { [0.0, PACE] } else { [0.0, 0.0] }
    }

    /// Watches the crates, the balls and the walker after `ticks` ticks; logs what was measured
    /// once all of it is.
    pub(crate) fn watch(&mut self, world: &World, ticks: u64) {
        let (crates, balls, player, tick) = (&self.crates, &self.balls, self.player, ticks);
        let mut transforms = Vec::new();
        world.transforms(crates, &mut transforms);
        if tick == MEASURE_TICK {
            self.crate_measured = transforms.iter().map(|t| t.position).collect();
        }
        world.transforms(balls, &mut transforms);
        let mut velocities = Vec::new();
        world.velocities(balls, &mut velocities);
        let top = f64::from(2.0 * PATCH_HALF[1]);
        for ((bounce, t), v) in self.bounces.iter_mut().zip(&transforms).zip(&velocities) {
            let y = t.position.y - top;
            *bounce = match *bounce {
                Bounce::Falling if v.linear.y > 0.05 => Bounce::Rising(y),
                Bounce::Rising(high) if v.linear.y <= 0.0 => Bounce::Done(high.max(y)),
                Bounce::Rising(high) => Bounce::Rising(high.max(y)),
                other => other,
            };
        }
        let s = world.character(player);
        match self.stop {
            None if s.position.z >= STOP_Z => {
                self.stop = Some((s.position.z, s.velocity.z, None));
            }
            // Its speed along the ground: on it, the tick's fall stays in its velocity.
            Some((from, speed, None))
                if Vec3::new(s.velocity.x, 0.0, s.velocity.z).length() < 0.01 =>
            {
                self.stop = Some((from, speed, Some(s.position.z)));
            }
            _ => {}
        }
        let bounced = self.bounces.iter().all(|b| matches!(b, Bounce::Done(_)));
        let rested = matches!(self.stop, Some((_, _, Some(_))));
        if !self.logged && bounced && rested && !self.crate_measured.is_empty() {
            self.logged = true;
            self.log();
        }
    }

    /// How far the walker slid on the ice once it stopped walking, and what the rows predict
    /// (v² / 2 μ g, its sole's pair friction with the ice), once it rests.
    pub(crate) fn slide(&self) -> Option<(f64, f64)> {
        let (from, speed, Some(to)) = self.stop? else {
            return None;
        };
        let ice = &ROWS[ROWS.len() - 1].physics;
        let mu = pair(SOLE.dynamic_friction, ice.dynamic_friction);
        Some((to - from, f64::from(speed * speed / (2.0 * mu * GRAVITY))))
    }

    /// How far each crate has travelled from where it started, metres, in the rows' order.
    #[cfg(test)]
    pub(crate) fn crate_travel(&self, world: &World) -> Vec<f64> {
        let mut transforms = Vec::new();
        world.transforms(&self.crates, &mut transforms);
        transforms
            .iter()
            .zip(&self.crate_start)
            .map(|(t, start)| (t.position - *start).length())
            .collect()
    }

    /// The measurements against what the rows predict.
    fn log(&self) {
        let (sin, cos) = sin_cos(RAMP_DEGREES.to_radians());
        let t = MEASURE_TICK as f64 / RATE;
        let wood = ROWS[CRATE_WOOD].physics.dynamic_friction;
        let crates: Vec<String> = ROWS
            .iter()
            .zip(self.crate_start.iter().zip(&self.crate_measured))
            .map(|(row, (a, b))| {
                let mu = f64::from(pair(wood, row.physics.dynamic_friction));
                let predicted = (f64::from(GRAVITY) * (sin - mu * cos)).max(0.0);
                // Down the slope only: not the few millimetres it settled onto the ramp.
                let down = DVec3::new(-cos, -sin, 0.0);
                let measured = 2.0 * (*b - *a).dot(down) / (t * t);
                format!(
                    "{} mu {mu:.3}: {measured:.2} m/s2 against {predicted:.2}",
                    &row.name[8..]
                )
            })
            .collect();
        let balls: Vec<String> = ROWS
            .iter()
            .zip(&self.bounces)
            .map(|(row, bounce)| {
                let e = f64::from(row.physics.restitution);
                let apex = if let Bounce::Done(h) = bounce {
                    *h
                } else {
                    0.0
                };
                format!(
                    "{} e {e:.2}: {apex:.3} m against {:.3}",
                    &row.name[8..],
                    e * e * BALL_DROP
                )
            })
            .collect();
        if let (Some((_, speed, _)), Some((slid, predicted))) = (self.stop, self.slide()) {
            let mu = pair(
                SOLE.dynamic_friction,
                ROWS[ROWS.len() - 1].physics.dynamic_friction,
            );
            tracing::info!(
                crates = %crates.join(", "),
                balls = %balls.join(", "),
                walker = %format_args!(
                    "from {speed:.2} m/s on ice (mu {mu:.3}): slid {slid:.2} m against {predicted:.2}"
                ),
                "the materials, measured against their rows (#203)"
            );
        }
    }
}

/// How far the walker's feet change its speed in a tick of `dt` on the row `ground` (m/s): its
/// sole's pair friction with the ground times g, the most the ground can push it by.
pub(crate) fn traction(ground: &PhysicsLayer, dt: f32) -> f32 {
    pair(SOLE.dynamic_friction, ground.dynamic_friction) * GRAVITY * dt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_hold_a_crate_where_their_friction_beats_the_slope() {
        // tan 20° = 0.364: a wooden crate holds on brick, wood and sand, slides on snow and ice.
        let tan = 0.364_f32;
        let wood = ROWS[CRATE_WOOD].physics.dynamic_friction;
        let holds: Vec<bool> = ROWS
            .iter()
            .map(|r| pair(wood, r.physics.dynamic_friction) > tan)
            .collect();
        assert_eq!(holds, [true, true, true, false, false]);
        // The walker's sole on the ice: a tenth of g, about.
        let ice = &ROWS[4].physics;
        assert!((pair(SOLE.dynamic_friction, ice.dynamic_friction) - 0.126).abs() < 1e-3);
    }
}
