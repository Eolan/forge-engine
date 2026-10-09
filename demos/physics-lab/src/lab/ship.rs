//! The space lab's spaceship (the owner's ask of 2026-10-03, after #150): a small sci-fi
//! fighter-shuttle modelled in Blender (`assets/blender/ship.py`, read through glTF) in place of
//! the rocket. Its three engines push it along its nose at the throttle, their flames sliding out
//! of the nozzles as it opens; jets turn it on the stick (the aeroplane's controls: the stick
//! pitches and rolls it, the rudder yaws it) at the rate the stick asks, a flight computer's way,
//! and hold it still when the stick is centred. They push no linear momentum: what the ship
//! gains, its engines gave it.

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use forge_geom::city::{Imported, PropKind, PropSpec};
use forge_geom::model::{Model, load_glb};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, World};
use glam::{DVec3, Quat, Vec3};

use super::fly::Pilot;

/// Its mass, kg.
pub(super) const MASS: f32 = 4000.0;
/// Its three engines' push together at full throttle, N (1.5 g), along its nose (−z).
pub(super) const THRUST: f32 = 60_000.0;
/// The jets' most torque, N·m: pitch, yaw and roll.
const TURN: Vec3 = Vec3::new(80_000.0, 80_000.0, 60_000.0);
/// The flight computer: the stick asks for a rate of turn about each axis, rad/s at full stick
/// (none when centred), and the jets push towards it, this many N·m per rad/s it is off (at most
/// `TURN`).
const RATE: Vec3 = Vec3::new(0.8, 0.8, 1.4);
const ASSIST: f32 = 150_000.0;
/// How far a flame slides out of its nozzle at full throttle, metres: its length and the depth
/// of the nozzle's throat in front of its mouth, so a closed throttle hides it whole.
const FLAME: f32 = 4.0;
const THROAT: f32 = 1.0;
/// How much its meshes' normals weigh in their simplification: the chase camera stays on it, as
/// on the rocket (#157).
const SMOOTH_NORMALS: f32 = 8.0;
/// Its engines' nozzles, as the model names their mouths.
const NOZZLES: [&str; 3] = ["nozzle-left", "nozzle-middle", "nozzle-right"];
/// The flames: one per engine, after the bodies.
pub(super) const FLAMES: u32 = NOZZLES.len() as u32;

/// The ship's model, read once from `assets/models/ship.glb`, and its cache key.
pub(super) fn ship_model() -> &'static (Model, String) {
    static MODEL: OnceLock<(Model, String)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/ship.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the ship's model {}: {e}", path.display()));
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, format!("assets/models/ship.glb#{digest:016x}"))
    })
}

/// The scene's props: the ship and an engine's flame.
pub(super) fn props() -> Vec<PropSpec> {
    let (model, key) = ship_model();
    let mesh = |name: &str| {
        model
            .mesh(name)
            .unwrap_or_else(|| panic!("the ship's model has no {name}"))
    };
    vec![
        PropSpec {
            name: "lab-ship".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} ship"),
                mesh: Arc::new(mesh("ship").mesh.clone()),
                normal_weight: Some(SMOOTH_NORMALS),
            }),
        },
        PropSpec {
            name: "lab-ship-flame".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} flame"),
                mesh: Arc::new(mesh("ship-flame").mesh.clone()),
                normal_weight: Some(SMOOTH_NORMALS),
            }),
        },
    ]
}

/// Adds the ship at `at`, turned by `rotation`, nothing damped: its body, the hull of its
/// model's shell, its weight at its origin.
pub(super) fn add(world: &mut World, at: DVec3, rotation: Quat) -> Result<BodyId> {
    let (model, _) = ship_model();
    let shell = model
        .mesh("ship-shell")
        .unwrap_or_else(|| panic!("the ship's model has no ship-shell"));
    let points: Vec<Vec3> = shell
        .mesh
        .positions
        .iter()
        .map(|&p| Vec3::from(p))
        .collect();
    let shape = Shape::convex_hull(&points, 0.03, 100.0)?.with_center_of_mass_at(Vec3::ZERO)?;
    Ok(world.add_body(&BodyDesc {
        rotation,
        mass: Some(MASS),
        friction: 0.5,
        allow_sleep: false,
        linear_damping: 0.0,
        angular_damping: 0.0,
        ..BodyDesc::dynamic(&shape, at)
    })?)
}

/// The engines' push and the jets' for the coming step, from the pilot's throttle and stick.
pub(super) fn tick(world: &mut World, ship: BodyId, pilot: &Pilot) {
    let (mut t, mut v) = (Vec::new(), Vec::new());
    world.transforms(&[ship], &mut t);
    world.velocities(&[ship], &mut v);
    let (t, v) = (t[0], v[0]);
    let thrust = t.rotation * Vec3::NEG_Z * (pilot.throttle.clamp(0.0, 1.0) * THRUST);
    // In its own frame (x right, y up, −z its nose): pushing the stick (+elevator) pitches the
    // nose down, the rudder right yaws it right, the stick right (+aileron) rolls it right.
    let stick = Vec3::new(
        -pilot.elevator.clamp(-1.0, 1.0),
        -pilot.rudder.clamp(-1.0, 1.0),
        -pilot.aileron.clamp(-1.0, 1.0),
    );
    let spin = t.rotation.inverse() * v.angular;
    let off = stick * RATE - spin;
    let torque = t.rotation * (off * ASSIST).clamp(-TURN, TURN);
    world.push(&[ship], &[(thrust, t.position, torque)]);
}

/// The flames for the movers, after the bodies: one at each nozzle, slid out by the throttle.
pub(super) fn flames_at(world: &World, ship: BodyId, throttle: f32, out: &mut Vec<Transform>) {
    let (model, _) = ship_model();
    let mut t = Vec::new();
    world.transforms(&[ship], &mut t);
    // Forward into the nozzle by what the throttle holds back: closed, the flame lies whole in
    // front of the throat.
    let back = (1.0 - throttle.clamp(0.0, 1.0)) * (FLAME + THROAT);
    for name in NOZZLES {
        let mouth = model
            .point(name)
            .unwrap_or_else(|| panic!("the ship's model has no {name}"));
        let at = mouth + Vec3::NEG_Z * back;
        out.push(Transform {
            position: t[0].position + (t[0].rotation * at).as_dvec3(),
            rotation: t[0].rotation,
        });
    }
}
