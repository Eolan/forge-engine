//! `physics-lab --lab drive` (issue #140, Phase 3's step 5): a car on the lab's floor (Jolt's
//! `VehicleConstraint` through `forge-physics`, its body and wheels modelled in Blender,
//! `assets/blender/car.py`, read through glTF): a jump ramp, a slalom of barrels, and a wall of
//! crates to drive through. The throttle, the steering and the handbrake are commands of the
//! lab's world, so a drive records, replays and goes through `--net`.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result};
use forge_core::dmath::sin_cos;
use forge_geom::city::{Imported, PropKind, PropSpec};
use forge_geom::model::{Model, load_glb};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, VehicleDesc, VehicleId, World};
use glam::{DVec3, Mat4, Quat, Vec3};

/// The car: its mass, kg, how far under the chassis' centre its weight sits, metres, and its
/// running gear (the model's wheelbase and track).
const CAR_MASS: f32 = 1200.0;
const CAR_WEIGHT_LOW: f32 = 0.25;
const VEHICLE: VehicleDesc = VehicleDesc {
    half_track: 0.74,
    half_wheelbase: 1.225,
    attach_y: 0.52,
    suspension: (0.05, 0.35),
    spring: (1.6, 0.55),
    wheel: (0.31, 0.2),
    max_steer: 0.55,
    engine: (320.0, 6500.0),
    brakes: (1600.0, 4000.0),
};
/// Faster than this ahead, m/s, a throttle astern brakes first.
const BRAKE_ABOVE: f32 = 1.0;
/// The jump: its ramp's angle, degrees, and where it stands.
const JUMP_DEGREES: f64 = 14.0;
const JUMP_Z: f64 = -45.0;
/// The slalom: barrels standing in a line along −z at x = 8, this far apart.
const SLALOM: u32 = 8;
const SLALOM_STEP: f64 = 9.0;
/// The wall: crates in courses, this many a course and this many high, across the way at z.
const WALL_WIDE: u32 = 8;
const WALL_HIGH: u32 = 4;
const WALL_Z: f64 = -95.0;

/// The car's model, read once from `assets/models/car.glb`, and its cache key.
pub(super) fn car_model() -> &'static (Model, String) {
    static MODEL: OnceLock<(Model, String)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/car.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the car's model {}: {e}", path.display()));
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, format!("assets/models/car.glb#{digest:016x}"))
    })
}

/// The car's props: its body and a wheel.
pub(super) fn props() -> Vec<PropSpec> {
    let (model, key) = car_model();
    let mesh = |name: &str| {
        model
            .mesh(name)
            .unwrap_or_else(|| panic!("the car's model has no {name}"))
    };
    vec![
        PropSpec {
            name: "lab-car".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} car"),
                mesh: Arc::new(mesh("car").mesh.clone()),
            }),
        },
        PropSpec {
            name: "lab-wheel".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} wheel"),
                mesh: Arc::new(mesh("car-wheel").mesh.clone()),
            }),
        },
    ]
}

/// What the track puts in the world.
pub(super) struct Track {
    pub statics: Vec<(usize, Mat4)>,
    pub chassis: BodyId,
    pub vehicle: VehicleId,
    pub barrels: Vec<BodyId>,
    pub crates: Vec<BodyId>,
}

/// Builds the track into `world`: the ramp (drawn with `ramp`), the barrels and the crates
/// (their shapes), the car.
pub(super) fn build(
    world: &mut World,
    ramp: usize,
    ramp_half: [f32; 3],
    barrel_shape: &Shape,
    crate_shape: &Shape,
    crate_half: f32,
) -> Result<Track> {
    let mut statics = Vec::new();
    // The jump: a ramp rising along −z, its low end on the floor.
    let (s, c) = sin_cos(JUMP_DEGREES.to_radians());
    let (hs, hc) = sin_cos(0.5 * JUMP_DEGREES.to_radians());
    // Rising towards −z: turned about +x by the angle (its far end up).
    let turn = Quat::from_xyzw(hs as f32, 0.0, 0.0, hc as f32);
    let half = f64::from(ramp_half[2]);
    let at = DVec3::new(
        0.0,
        s * half - f64::from(ramp_half[1]) * c,
        JUMP_Z - c * half,
    );
    let ramp_shape = Shape::cuboid(Vec3::from(ramp_half), 0.03, 0.0)?;
    world.add_body(&BodyDesc {
        rotation: turn,
        ..BodyDesc::fixed(&ramp_shape, at)
    })?;
    statics.push((ramp, Mat4::from_rotation_translation(turn, at.as_vec3())));
    // The slalom's barrels, standing.
    let mut barrels = Vec::new();
    for n in 0..SLALOM {
        let z = -15.0 - SLALOM_STEP * f64::from(n);
        barrels.push(world.add_body(&BodyDesc {
            friction: 0.5,
            mass: Some(25.0),
            ..BodyDesc::dynamic(barrel_shape, DVec3::new(8.0, 0.0, z))
        })?);
    }
    // The wall of crates, in courses, each a hair over the one under it.
    let mut crates = Vec::new();
    let pitch = f64::from(2.0 * crate_half + 0.01);
    for course in 0..WALL_HIGH {
        let shift = if course % 2 == 0 { 0.0 } else { 0.5 * pitch };
        for n in 0..WALL_WIDE {
            let x = (f64::from(n) - f64::from(WALL_WIDE - 1) * 0.5) * pitch + shift;
            let y = f64::from(crate_half) + f64::from(course) * (pitch + 0.001);
            crates.push(world.add_body(&BodyDesc {
                friction: 0.6,
                ..BodyDesc::dynamic(crate_shape, DVec3::new(x, y, WALL_Z))
            })?);
        }
    }
    // The car, facing −z down the track.
    let (chassis, vehicle) = car(world, DVec3::new(0.0, 0.15, 0.0))?;
    Ok(Track {
        statics,
        chassis,
        vehicle,
        barrels,
        crates,
    })
}

/// Adds a car at `at`, facing −z, its chassis the model's shell: its body and its vehicle.
pub(super) fn car(world: &mut World, at: DVec3) -> Result<(BodyId, VehicleId)> {
    let (model, _) = car_model();
    let shell = model.mesh("car-shell").context("the model's shell")?;
    let points: Vec<Vec3> = shell
        .mesh
        .positions
        .iter()
        .map(|&p| Vec3::from(p))
        .collect();
    let body_shape = Shape::convex_hull(&points, 0.05, 100.0)?
        .with_center_of_mass_offset(Vec3::new(0.0, -CAR_WEIGHT_LOW, 0.0))?;
    let chassis = world.add_body(&BodyDesc {
        mass: Some(CAR_MASS),
        friction: 0.4,
        // Kept awake: a parked car sleeps, and a sleeping car takes no input.
        allow_sleep: false,
        ..BodyDesc::dynamic(&body_shape, at)
    })?;
    let vehicle = world.add_vehicle(chassis, &VEHICLE)?;
    Ok((chassis, vehicle))
}

/// The driver's controls as the world holds them: throttle (−1 to 1), steering (−1 left to
/// 1 right), handbrake (0 or 1), held until the next command.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Driver {
    pub car: Option<(BodyId, VehicleId)>,
    pub throttle: f32,
    pub steer: f32,
    pub handbrake: f32,
}

impl Driver {
    /// Hands the controls to the car for the coming step: a throttle astern while the car still
    /// rolls ahead brakes it instead.
    pub(super) fn tick(&self, world: &mut World) {
        let Some((chassis, vehicle)) = self.car else {
            return;
        };
        let mut t = Vec::new();
        let mut v = Vec::new();
        world.transforms(&[chassis], &mut t);
        world.velocities(&[chassis], &mut v);
        let ahead = v[0].linear.dot(t[0].rotation * Vec3::NEG_Z);
        let (throttle, brake) = if self.throttle < 0.0 && ahead > BRAKE_ABOVE {
            (0.0, -self.throttle)
        } else {
            (self.throttle, 0.0)
        };
        world.drive(vehicle, throttle, self.steer, brake, self.handbrake);
    }

    /// The car's wheels for the movers, after its body.
    pub(super) fn wheels(&self, world: &World, out: &mut Vec<Transform>) {
        if let Some((_, vehicle)) = self.car {
            let mut wheels = Vec::new();
            world.wheels(vehicle, &mut wheels);
            out.extend(wheels);
        }
    }
}
