//! `physics-lab --lab walk` (issue #139, Phase 3's step 4): a playground for a walking
//! character (Jolt's `CharacterVirtual` through `forge-physics`): a flight of stairs, ramps of
//! 20°, 35° and 50°, a platform shuttling to and fro, crates light enough to push and crates too
//! heavy, and a small pyramid of blocks to jump onto. The walk and the jumps are commands of the
//! lab's world, so a session records, replays and goes through `--net`.

use anyhow::Result;
use forge_core::dmath::sin_cos;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Motion, Shape, World};
use glam::{DVec3, Mat4, Quat, Vec3};

pub(crate) use crate::character::{Player, RUN_SPEED, player_props};

/// The stairs: steps, their rise and run, metres, the first's near edge, and their width.
pub(super) const STEPS: u32 = 6;
const RISE: f32 = 0.2;
const RUN: f32 = 0.4;
const STAIRS_X: f32 = 4.0;
const STAIRS_HALF_WIDTH: f32 = 1.5;
/// The ramps: their angles in degrees, their half length and where they stand.
const RAMPS: [f64; 3] = [20.0, 35.0, 50.0];
pub(super) const RAMP_HALF: [f32; 3] = [1.5, 0.1, 3.0];
const RAMPS_Z: f32 = -5.0;
/// The platform: its half sizes and its height (its way and speed: `character::Player::tick`).
pub(super) const PLATFORM_HALF: [f32; 3] = [1.5, 0.15, 1.5];
const PLATFORM_Y: f32 = 0.25;
/// The crates: half their side, metres, and how many of each, light and heavy.
const CRATE_HALF: f32 = 0.35;
const LIGHT_CRATES: u32 = 6;
const HEAVY_CRATES: u32 = 3;

/// The playground's props: a stair's slab, a ramp, the platform, the player and its visor.
pub(super) fn props() -> Vec<PropSpec> {
    let mut props = vec![
        PropSpec {
            name: "lab-slab".to_owned(),
            kind: PropKind::Block(Block {
                half: [0.5 * RUN, 0.5 * RISE, STAIRS_HALF_WIDTH],
                radius: 0.015,
                segments: 4,
            }),
        },
        PropSpec {
            name: "lab-ramp".to_owned(),
            kind: PropKind::Block(Block {
                half: RAMP_HALF,
                radius: 0.03,
                segments: 8,
            }),
        },
        PropSpec {
            name: "lab-platform".to_owned(),
            kind: PropKind::Block(Block {
                half: PLATFORM_HALF,
                radius: 0.03,
                segments: 6,
            }),
        },
    ];
    props.extend(player_props());
    props
}

/// A rotation about +x by `degrees`, from the deterministic sine (the platforms' differ).
fn about_x(degrees: f64) -> Quat {
    let (s, c) = sin_cos(0.5 * degrees.to_radians());
    Quat::from_xyzw(s as f32, 0.0, 0.0, c as f32)
}

/// What the playground puts in the world: its still pieces (shapes kept alive by their bodies,
/// with the props and transforms that draw them), the platform, the crates and the player.
pub(super) struct Playground {
    pub statics: Vec<(usize, Mat4)>,
    pub platform: BodyId,
    pub light: Vec<BodyId>,
    pub heavy: Vec<BodyId>,
    pub blocks: Vec<BodyId>,
    pub player: CharacterId,
}

/// Builds the playground into `world`; `slab`, `ramp` are the props' indices.
pub(super) fn build(
    world: &mut World,
    slab: usize,
    ramp: usize,
    block_shape: &Shape,
    block_half: f32,
) -> Result<Playground> {
    let mut statics = Vec::new();
    // The stairs, rising along +x: step k a stack of k + 1 slabs drawn, one box to collide.
    for k in 0..STEPS {
        let height = RISE * (k + 1) as f32;
        let x = STAIRS_X + RUN * (k as f32 + 0.5);
        let shape = Shape::cuboid(
            Vec3::new(0.5 * RUN, 0.5 * height, STAIRS_HALF_WIDTH),
            0.015,
            0.0,
        )?;
        world.add_body(&BodyDesc::fixed(
            &shape,
            DVec3::new(f64::from(x), f64::from(0.5 * height), 0.0),
        ))?;
        for s in 0..=k {
            let y = RISE * (s as f32 + 0.5);
            statics.push((slab, Mat4::from_translation(Vec3::new(x, y, 0.0))));
        }
    }
    // The ramps along −z, each rising from the floor away from the player.
    let ramp_shape = Shape::cuboid(Vec3::from(RAMP_HALF), 0.03, 0.0)?;
    for (n, &degrees) in RAMPS.iter().enumerate() {
        let (s, c) = sin_cos(degrees.to_radians());
        let turn = about_x(degrees);
        let half = f64::from(RAMP_HALF[2]);
        let at = DVec3::new(
            -6.0 + 6.0 * n as f64,
            s * half,
            f64::from(RAMPS_Z) - c * half,
        );
        world.add_body(&BodyDesc {
            rotation: turn,
            ..BodyDesc::fixed(&ramp_shape, at)
        })?;
        statics.push((ramp, Mat4::from_rotation_translation(turn, at.as_vec3())));
    }
    // The platform, shuttling along x at +z.
    let deck = Shape::cuboid(Vec3::from(PLATFORM_HALF), 0.03, 0.0)?;
    let platform = world.add_body(&BodyDesc {
        motion: Motion::Kinematic,
        friction: 0.8,
        ..BodyDesc::fixed(&deck, DVec3::new(-6.0, f64::from(PLATFORM_Y), 8.0))
    })?;
    // Crates by the stairs: light ones to push, heavy ones that stay.
    let crate_half = Vec3::splat(CRATE_HALF);
    let light_shape = Shape::cuboid(crate_half, 0.025, 100.0)?;
    let heavy_shape = Shape::cuboid(crate_half, 0.025, 900.0)?;
    let mut light = Vec::new();
    for n in 0..LIGHT_CRATES {
        let at = DVec3::new(-2.0 + 0.9 * f64::from(n), f64::from(CRATE_HALF), 3.0);
        light.push(world.add_body(&BodyDesc {
            friction: 0.5,
            ..BodyDesc::dynamic(&light_shape, at)
        })?);
    }
    let mut heavy = Vec::new();
    for n in 0..HEAVY_CRATES {
        let at = DVec3::new(-2.0 + 1.2 * f64::from(n), f64::from(CRATE_HALF), 5.0);
        heavy.push(world.add_body(&BodyDesc {
            friction: 0.7,
            ..BodyDesc::dynamic(&heavy_shape, at)
        })?);
    }
    // A pyramid of four layers to jump up.
    let mut blocks = Vec::new();
    let pitch = f64::from(2.0 * block_half + 0.02);
    for layer in 0..4u32 {
        let side = 4 - layer;
        let offset = |k: u32| (f64::from(k) - f64::from(side - 1) * 0.5) * pitch;
        let y = f64::from(block_half) + f64::from(layer) * f64::from(2.0 * block_half + 0.001);
        for j in 0..side {
            for i in 0..side {
                blocks.push(world.add_body(&BodyDesc {
                    friction: 0.8,
                    ..BodyDesc::dynamic(
                        block_shape,
                        DVec3::new(8.0 + offset(i), y, 7.0 + offset(j)),
                    )
                })?);
            }
        }
    }
    let player = world.add_character(&CharacterDesc {
        position: DVec3::new(0.0, 0.0, 1.0),
        ..CharacterDesc::default()
    });
    Ok(Playground {
        statics,
        platform,
        light,
        heavy,
        blocks,
        player,
    })
}
