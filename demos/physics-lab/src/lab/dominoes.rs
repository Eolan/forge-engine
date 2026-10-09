//! `physics-lab --lab dominoes` (issue #146): one of Phase 3's advanced tests, a domino run that
//! ends the same however it is run. 300 wooden dominoes stand on a spiral from 3 m out to 8 m,
//! each 40 cm on from the last (five eighths of its height); Space (or `--release N`) tips the
//! first, and the fall runs the spiral out. A recording of the run replays to the same digests,
//! and `--net` runs it through a server and a client: a test of the solver's determinism over
//! the thousands of contacts the fall makes and breaks in turn (that it does not depend on the
//! workers is the pile's test, #136).

use anyhow::Result;
use forge_core::dmath::sin_cos;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World};
use glam::{DVec3, Quat, Vec3};

/// A domino's half sizes: 8 cm thick, 64 cm tall, 32 cm wide; its wood, kg/m³.
pub(super) const DOMINO_HALF: [f32; 3] = [0.16, 0.32, 0.04];
const DENSITY: f32 = 650.0;
/// How many, and the spiral: its first radius and how far it widens a turn, metres; the
/// dominoes' spacing along it.
const COUNT: usize = 300;
const FIRST_RADIUS: f64 = 3.0;
const WIDENING: f64 = 1.5;
const SPACING: f64 = 0.4;
/// The first one's push, m/s at its top.
const PUSH: f32 = 1.2;

/// The scene's prop: a domino.
pub(super) fn props() -> Vec<PropSpec> {
    vec![PropSpec {
        name: "lab-domino".to_owned(),
        kind: PropKind::Block(Block {
            half: DOMINO_HALF,
            radius: 0.01,
            segments: 2,
        }),
    }]
}

/// The run as the world holds it.
#[derive(Clone, Debug)]
pub(super) struct Run {
    pub dominoes: Vec<BodyId>,
}

/// Where each domino stands: on the spiral, `SPACING` apart along it, turned to face along it,
/// standing on the floor. The angles through `dmath`, so the run is the same bits everywhere.
fn places() -> Vec<Transform> {
    let mut out = Vec::with_capacity(COUNT);
    let mut angle = 0.0_f64;
    for _ in 0..COUNT {
        let radius = FIRST_RADIUS + WIDENING * angle / std::f64::consts::TAU;
        let (s, c) = sin_cos(angle);
        let position = DVec3::new(radius * c, f64::from(DOMINO_HALF[1]), radius * s);
        // Its face across the spiral's way (the tangent at the angle): turned about y so its
        // thin axis (z) lies along the way.
        let (hs, hc) = sin_cos(0.5 * (-angle));
        let rotation = Quat::from_xyzw(0.0, hs as f32, 0.0, hc as f32);
        out.push(Transform { position, rotation });
        // The next a spacing on along the spiral's arc.
        angle += SPACING / radius;
    }
    out
}

/// Builds the run into `world`.
pub(super) fn build(world: &mut World) -> Result<Run> {
    let shape = Shape::cuboid(Vec3::from_array(DOMINO_HALF), 0.005, DENSITY)?;
    let dominoes = places()
        .into_iter()
        .map(|at| {
            world.add_body(&BodyDesc {
                rotation: at.rotation,
                friction: 0.5,
                ..BodyDesc::dynamic(&shape, at.position)
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Run { dominoes })
}

impl Run {
    /// Tips the first domino along the spiral: a spin about its own x at its base's speed.
    pub(super) fn push(&self, world: &mut World) {
        let Some(&first) = self.dominoes.first() else {
            return;
        };
        let mut t = Vec::new();
        world.transforms(&[first], &mut t);
        let along = t[0].rotation * Vec3::Z;
        let axis = t[0].rotation * Vec3::X;
        world.set_velocity(
            first,
            Velocity {
                linear: along * (0.5 * PUSH),
                angular: axis * (PUSH / (2.0 * DOMINO_HALF[1])),
            },
        );
    }

    /// The dominoes down (tipped past 45°).
    pub(super) fn fallen(&self, world: &World) -> usize {
        let mut t = Vec::new();
        world.transforms(&self.dominoes, &mut t);
        t.iter().filter(|t| (t.rotation * Vec3::Y).y < 0.7).count()
    }
}
