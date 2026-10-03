//! `physics-lab --lab rocket` (Phase 3's step 5, issue #148): a rocket on a launch pad. Its engine
//! pushes along its axis from the nozzle, swung a few degrees by the stick (thrust vectoring: the
//! elevator pitches it, the rudder yaws it), and small jets at its nose roll it; four fins at its
//! tail are flying surfaces (`forge_physics::aero`, the aeroplane's) that turn it into the wind,
//! and its body drags. The throttle and the stick are the aeroplane's commands, so a flight
//! records, replays and goes through `--net`.

use std::sync::Arc;

use anyhow::Result;
use forge_geom::city::{Block, Imported, Lathe, PropKind, PropSpec, lathe};
use forge_geom::fracture::Polyhedron;
use forge_geom::procedural::TriMesh;
use forge_physics::aero::{Air, Surface, push};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, World};
use glam::{DVec3, Mat4, Quat, Vec3};

use super::fly::Pilot;

/// The rocket, in its frame (+y along its axis, its origin at its base's centre): its body's
/// radius and height, its nose's tip, and its fins' reach from the axis, height and foot (under
/// the nozzle's bell: it stands on them).
const RADIUS: f32 = 0.6;
const BODY_TOP: f32 = 10.5;
const NOSE_TIP: f32 = 13.5;
const FIN_REACH: f32 = 1.7;
const FIN_HEIGHT: f32 = 2.2;
const FIN_BOTTOM: f32 = -0.8;
/// How much the rocket's normals weigh in its meshes' simplification error, metres per unit of
/// normal change (a hard surface's is 0.5).
const SMOOTH_NORMALS: f32 = 8.0;
/// Its mass, kg (fuelled), and its weight's height on its axis.
pub(super) const MASS: f32 = 3000.0;
const CENTER_OF_MASS: Vec3 = Vec3::new(0.0, 5.0, 0.0);
/// Its engine's push at full throttle, N (1.7 times its weight), where it acts (the nozzle's
/// throat), and how far the stick swings it, as the sine of the angle (about 6°).
const THRUST: f32 = 50_000.0;
const NOZZLE: Vec3 = Vec3::new(0.0, -0.1, 0.0);
const GIMBAL: f32 = 0.1;
/// The roll jets' torque at full aileron, N·m.
const ROLL: f32 = 2000.0;
/// In space, the pitch and yaw jets' torque at full stick, N·m.
const TURN: f32 = 5000.0;
/// Its body's drag along its axis and across it (the drag coefficient times the area, m²).
const AXIAL_DRAG: f32 = 0.35;
const CROSS_DRAG: f32 = 8.0;
/// The launch pad: its half sizes; the rocket stands on it.
pub(super) const PAD_HALF: [f32; 3] = [6.0, 0.5, 6.0];

/// The scene's props: the rocket's body (with its nose and nozzle), a pair of its fins, the pad.
pub(super) fn props() -> Vec<PropSpec> {
    // The nozzle's bell under the base, the body, an ogive nose.
    let profile = vec![
        (0.0, -0.75),
        (0.42, -0.75),
        (0.22, -0.1),
        (RADIUS, 0.0),
        (RADIUS, BODY_TOP),
        (0.56, 11.2),
        (0.46, 12.0),
        (0.3, 12.8),
        (0.12, 13.3),
        (0.0, NOSE_TIP),
    ];
    let along = profile.len() as u32;
    let body = lathe(&Lathe {
        profile,
        around: 48,
        along,
        flutes: 0,
        flute_depth: 0.0,
        flute_span: (0.0, 0.0),
    });
    vec![
        // Its body and fins simplify only where their shading does not change: the chase camera
        // stays on them, and with a hard surface's weight (0.5) the light's line along the body
        // moved a pixel as levels changed while it turned, 108 to 138 px a frame (the owner saw
        // it shimmer in `--lab space`).
        PropSpec {
            name: "lab-rocket".to_owned(),
            kind: PropKind::Imported(Imported {
                key: "lab rocket body, 48 round".to_owned(),
                mesh: Arc::new(body),
                normal_weight: Some(SMOOTH_NORMALS),
            }),
        },
        PropSpec {
            name: "lab-rocket-fins".to_owned(),
            kind: PropKind::Imported(Imported {
                key: "lab rocket fins, swept".to_owned(),
                mesh: Arc::new(fin_pair()),
                normal_weight: Some(SMOOTH_NORMALS),
            }),
        },
        PropSpec {
            name: "lab-pad".to_owned(),
            kind: PropKind::Block(Block {
                half: PAD_HALF,
                radius: 0.05,
                segments: 8,
            }),
        },
    ]
}

/// A pair of fins across the axis, about their middle: a plate 6 cm thick, its leading edges
/// swept down from the top of their root at the body's side to their tips 1.4 m lower.
fn fin_pair() -> TriMesh {
    let half = DVec3::new(f64::from(FIN_REACH), 0.5 * f64::from(FIN_HEIGHT), 0.03);
    let (root, drop) = (f64::from(RADIUS), 1.4);
    let mut fins = Polyhedron::cuboid(half);
    for side in [1.0, -1.0] {
        // Keep what lies under the line from (root, top) to (tip, top − drop).
        let normal = DVec3::new(side * drop, half.x - root, 0.0).normalize();
        let offset = normal.dot(DVec3::new(side * root, half.y, 0.0));
        fins = fins.clip(normal, offset).expect("the fins' roots stay");
    }
    // Drawn in one paint: the cuts are their edges, not broken faces.
    for face in &mut fins.faces {
        face.cut = false;
    }
    fins.mesh(DVec3::ZERO)
}

/// The fins as flying surfaces, in the rocket's frame: each pair across the axis one surface,
/// at the fins' middle (under the weight: they turn the nose into the wind), its chord along the
/// axis.
fn fins() -> [Surface; 2] {
    let fin = |normal: Vec3| Surface {
        at: Vec3::new(0.0, FIN_BOTTOM + 0.5 * FIN_HEIGHT, 0.0),
        normal,
        forward: Vec3::Y,
        area: 2.0 * (FIN_REACH - RADIUS) * FIN_HEIGHT,
        aspect: 1.2,
        rigging: 0.0,
    };
    [fin(Vec3::X), fin(Vec3::Z)]
}

/// What the scene puts in the world: the pad (drawn) and the rocket.
pub(super) struct Site {
    pub statics: Vec<(usize, Mat4)>,
    pub rocket: BodyId,
}

/// Builds the scene into `world`: the pad drawn with `pad`, the rocket standing on it.
pub(super) fn build(world: &mut World, pad: usize) -> Result<Site> {
    let pad_at = DVec3::new(0.0, f64::from(PAD_HALF[1]), 0.0);
    let pad_shape = Shape::cuboid(Vec3::from_array(PAD_HALF), 0.05, 0.0)?;
    world.add_body(&BodyDesc {
        friction: 0.8,
        ..BodyDesc::fixed(&pad_shape, pad_at)
    })?;
    let at = DVec3::new(0.0, f64::from(2.0 * PAD_HALF[1] - FIN_BOTTOM), 0.0);
    let rocket = add(world, at, Quat::IDENTITY, 0.05)?;
    Ok(Site {
        statics: vec![(pad, Mat4::from_translation(pad_at.as_vec3()))],
        rocket,
    })
}

/// Adds the rocket at `at`, turned by `rotation`, losing `angular_damping` of its spin a second:
/// its body.
pub(super) fn add(
    world: &mut World,
    at: DVec3,
    rotation: Quat,
    angular_damping: f32,
) -> Result<BodyId> {
    // Its collision: the hull of its body, its nose and its fins' tips (it stands on them).
    let mut points = Vec::new();
    for k in 0..12 {
        let (s, c) = ring(k);
        points.push(Vec3::new(RADIUS * c, 0.0, RADIUS * s));
        points.push(Vec3::new(RADIUS * c, BODY_TOP, RADIUS * s));
    }
    for (x, z) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        points.push(Vec3::new(FIN_REACH * x, FIN_BOTTOM, FIN_REACH * z));
        points.push(Vec3::new(RADIUS * x, FIN_BOTTOM + FIN_HEIGHT, RADIUS * z));
    }
    points.push(Vec3::new(0.0, NOSE_TIP, 0.0));
    let shape = Shape::convex_hull(&points, 0.03, 100.0)?.with_center_of_mass_at(CENTER_OF_MASS)?;
    Ok(world.add_body(&BodyDesc {
        rotation,
        mass: Some(MASS),
        friction: 0.6,
        allow_sleep: false,
        // Its drag is the air's (its body's and its fins'), not the solver's damping.
        linear_damping: 0.0,
        angular_damping,
        ..BodyDesc::dynamic(&shape, at)
    })?)
}

/// The sine and cosine of the `k`th of twelve angles round a circle, without trigonometry (the
/// rocket's hull is the same bits everywhere): multiples of 30°.
fn ring(k: u32) -> (f32, f32) {
    const HALF_ROOT_3: f32 = 0.866_025_4;
    const SINES: [f32; 12] = [
        0.0,
        0.5,
        HALF_ROOT_3,
        1.0,
        HALF_ROOT_3,
        0.5,
        0.0,
        -0.5,
        -HALF_ROOT_3,
        -1.0,
        -HALF_ROOT_3,
        -0.5,
    ];
    (SINES[k as usize % 12], SINES[(k as usize + 3) % 12])
}

/// The air's push, the engine's and the roll jets' for the coming step, from the pilot's
/// throttle and stick.
pub(super) fn tick(world: &mut World, rocket: BodyId, pilot: &Pilot, space: bool) {
    let (mut t, mut v, mut c) = (Vec::new(), Vec::new(), Vec::new());
    world.transforms(&[rocket], &mut t);
    world.velocities(&[rocket], &mut v);
    world.centers_of_mass(&[rocket], &mut c);
    let (t, v) = (t[0], v[0]);
    let axis = t.rotation * Vec3::Y;
    let (force, mut torque) = if space {
        // No air in space (#150): jets at its nose and tail pitch and yaw it as the stick says,
        // the way the engine's swing would.
        let jets = Vec3::new(
            -pilot.elevator.clamp(-1.0, 1.0),
            0.0,
            -pilot.rudder.clamp(-1.0, 1.0),
        );
        (Vec3::ZERO, t.rotation * (jets * TURN))
    } else {
        let (mut force, torque) = push(&fins(), &[0.0, 0.0], t, v, c[0], &Air::STILL);
        // The body's drag, split along its axis and across it.
        let along = axis * v.linear.dot(axis);
        let across = v.linear - along;
        let q = 0.5 * Air::STILL.density;
        force -= q * (AXIAL_DRAG * along.length() * along + CROSS_DRAG * across.length() * across);
        (force, torque)
    };
    // The engine along its axis, swung by the stick, as for an aeroplane pitched up on its tail:
    // pushing the stick (+elevator) pushes the tail back (+z) and tips the nose downrange (−z),
    // the rudder right pushes the tail left (−x) and yaws the nose right; the roll jets roll it
    // right (its right side, +x, down) about its axis.
    let swing = Vec3::new(
        -pilot.rudder.clamp(-1.0, 1.0) * GIMBAL,
        1.0,
        pilot.elevator.clamp(-1.0, 1.0) * GIMBAL,
    )
    .normalize();
    let thrust = t.rotation * swing * (pilot.throttle.clamp(0.0, 1.0) * THRUST);
    let nozzle = t.position + (t.rotation * NOZZLE).as_dvec3();
    torque += axis * (pilot.aileron.clamp(-1.0, 1.0) * ROLL);
    world.push(
        &[rocket, rocket],
        &[(force, t.position, torque), (thrust, nozzle, Vec3::ZERO)],
    );
}

/// The fins for the movers, after the bodies: two pairs across each other.
pub(super) fn fins_at(world: &World, rocket: BodyId, out: &mut Vec<Transform>) {
    let mut t = Vec::new();
    world.transforms(&[rocket], &mut t);
    let fins = Vec3::new(0.0, FIN_BOTTOM + 0.5 * FIN_HEIGHT, 0.0);
    let middle = t[0].position + (t[0].rotation * fins).as_dvec3();
    let quarter = Quat::from_xyzw(
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    out.push(Transform {
        position: middle,
        rotation: t[0].rotation,
    });
    out.push(Transform {
        position: middle,
        rotation: t[0].rotation * quarter,
    });
}
