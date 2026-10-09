//! `physics-lab --lab tug` (issue #149): one of Phase 3's advanced tests, a tug-of-war on one
//! sled. Two ropes run from its sides to the two teams; each team pulls along its rope as its
//! player's last `Pull` command says, the left team for player 0 (this player), the right for
//! player 1 (the bot with `--net`). The pulls are held in the world's state like the other
//! controls, so a pull made on one machine moves the sled on every other, late by the link: the
//! test of a body two players fight over, which a client must correct whenever the other
//! changes their pull (D-010's reconciliation).

use anyhow::Result;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, World};
use glam::{DVec3, Mat4, Quat, Vec3};

/// The sled: its half sizes (a crate's), its mass, kg, and its friction on the floor.
pub(super) const SLED_HALF: f32 = 0.35;
const SLED_MASS: f32 = 200.0;
const SLED_FRICTION: f32 = 0.4;
/// A team's pull at full strength, N, and what each holds at the start (half: the sled stays).
const FULL_PULL: f32 = 2000.0;
pub(super) const HOLD: f32 = 0.5;
/// How far each team's rope runs out from the sled, metres, and the lines either side the sled
/// must cross to win, metres from the middle.
const ROPE: f32 = 10.0;
pub(super) const WIN_X: f64 = 3.0;

/// The scene's props: a rope (a cord along x) and a line on the floor (along z).
pub(super) fn props() -> Vec<PropSpec> {
    vec![
        PropSpec {
            name: "lab-rope".to_owned(),
            kind: PropKind::Block(Block {
                half: [0.5 * ROPE, 0.02, 0.02],
                radius: 0.015,
                segments: 2,
            }),
        },
        PropSpec {
            name: "lab-line".to_owned(),
            kind: PropKind::Block(Block {
                half: [0.05, 0.005, 3.0],
                radius: 0.0,
                segments: 2,
            }),
        },
    ]
}

/// What the scene puts in the world: the lines (drawn) and the sled.
pub(super) struct Site {
    pub statics: Vec<(usize, Mat4)>,
    pub sled: BodyId,
}

/// Builds the scene into `world`: the middle line and the two winning lines drawn with `line`,
/// the sled on the middle one.
pub(super) fn build(world: &mut World, line: usize) -> Result<Site> {
    let statics = [-WIN_X, 0.0, WIN_X]
        .into_iter()
        .map(|x| {
            (
                line,
                Mat4::from_translation(Vec3::new(x as f32, 0.005, 0.0)),
            )
        })
        .collect();
    let shape = Shape::cuboid(Vec3::splat(SLED_HALF), 0.025, 100.0)?;
    let sled = world.add_body(&BodyDesc {
        mass: Some(SLED_MASS),
        friction: SLED_FRICTION,
        allow_sleep: false,
        ..BodyDesc::dynamic(&shape, DVec3::new(0.0, f64::from(SLED_HALF), 0.0))
    })?;
    Ok(Site { statics, sled })
}

/// Where each team's rope leaves the sled: the middle of its side, in the world.
fn ends(t: &Transform) -> [DVec3; 2] {
    [-1.0_f32, 1.0]
        .map(|side| t.position + (t.rotation * Vec3::new(side * SLED_HALF, 0.0, 0.0)).as_dvec3())
}

/// The teams' pulls for the coming step, each `pulls[k]` of full strength (0 to 1), along its rope:
/// level, out from its side of the sled. Once the sled is over a line the game is won and the
/// teams let go.
pub(super) fn pull(world: &mut World, sled: BodyId, pulls: [f32; 2]) {
    let mut t = Vec::new();
    world.transforms(&[sled], &mut t);
    if t[0].position.x.abs() > WIN_X {
        return;
    }
    let [left, right] = ends(&t[0]);
    let strength = |k: usize| pulls[k].clamp(0.0, 1.0) * FULL_PULL;
    world.push(
        &[sled, sled],
        &[
            (Vec3::NEG_X * strength(0), left, Vec3::ZERO),
            (Vec3::X * strength(1), right, Vec3::ZERO),
        ],
    );
}

/// The two ropes for the movers, after the bodies: each from its side of the sled out towards
/// its team.
pub(super) fn ropes(world: &World, sled: BodyId, out: &mut Vec<Transform>) {
    let mut t = Vec::new();
    world.transforms(&[sled], &mut t);
    for (side, end) in [-1.0, 1.0].into_iter().zip(ends(&t[0])) {
        out.push(Transform {
            position: end + DVec3::new(side * f64::from(0.5 * ROPE), 0.0, 0.0),
            rotation: Quat::IDENTITY,
        });
    }
}

/// The team that has won, once the sled's middle is over its line: 0 the left, 1 the right.
pub(super) fn winner(world: &World, sled: BodyId) -> Option<usize> {
    let mut t = Vec::new();
    world.transforms(&[sled], &mut t);
    let x = t[0].position.x;
    (x.abs() > WIN_X).then_some(usize::from(x > 0.0))
}
