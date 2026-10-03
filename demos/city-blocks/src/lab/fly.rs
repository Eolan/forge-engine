//! `physics-lab --lab fly` (issue #141, Phase 3's step 5): an aeroplane modelled in Blender
//! (`assets/blender/plane.py`, read through glTF) on a runway on the lab's floor. Its wings, its
//! tailplane and its fin are flying surfaces (`forge_physics::aero`): the air past each lifts
//! and drags it by its incidence, the elevator, the ailerons and the rudder deflect them, and the
//! propeller pulls. The throttle and the stick are commands of the lab's world, so a flight
//! records, replays and goes through `--net`.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result};
use forge_geom::city::{Block, Imported, PropKind, PropSpec};
use forge_geom::model::{Model, load_glb};
use forge_physics::aero::{Air, Surface, push};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, World};
use glam::{DVec3, Mat4, Quat, Vec3};

/// The aeroplane: its mass, kg, the pull of its propeller at full throttle, N, where its
/// propeller turns in its frame, and the drag of its fuselage and gear (the drag coefficient
/// times the area, m²).
const MASS: f32 = 750.0;
const THRUST: f32 = 3000.0;
const PROPELLER: Vec3 = Vec3::new(0.0, 0.0, -2.12);
const FUSELAGE_DRAG: f32 = 0.45;
/// Its weight: ahead of the wing's lift (at z = −0.28) for a steady pitch, and of the main
/// wheels, which carry most of it.
const CENTER_OF_MASS: Vec3 = Vec3::new(0.0, -0.1, -0.4);
const MAIN_WHEELS_Z: f32 = 0.0;
/// Its scraping on the field when down off its wheels: the friction, and the height of its
/// origin under which it is down (1.37 m on its wheels, 0.65 m on a wing).
const SCRAPE: f32 = 0.6;
const SCRAPE_HEIGHT: f64 = 2.0;
/// How far the controls deflect their surfaces at full stick or pedal, as sines.
const ELEVATOR: f32 = 0.35;
const AILERON: f32 = 0.06;
const RUDDER: f32 = 0.2;
/// Where it starts: on the runway's threshold, facing down it along −z.
const START: DVec3 = DVec3::new(0.0, 1.45, 200.0);
/// The runway: its half sizes, metres.
pub(super) const RUNWAY_HALF: [f32; 3] = [12.0, 0.02, 260.0];
/// The field around it, in place of the lab's floor: its half side, metres.
pub(super) const FIELD_HALF: f32 = 2500.0;

/// The aeroplane's model, read once from `assets/models/plane.glb`, and its cache key.
pub(super) fn plane_model() -> &'static (Model, String) {
    static MODEL: OnceLock<(Model, String)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/plane.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the aeroplane's model {}: {e}", path.display()));
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, format!("assets/models/plane.glb#{digest:016x}"))
    })
}

/// The flight's props: the aeroplane, its propeller, the runway and the field.
pub(super) fn props() -> Vec<PropSpec> {
    let (model, key) = plane_model();
    let mesh = |name: &str| {
        model
            .mesh(name)
            .unwrap_or_else(|| panic!("the aeroplane's model has no {name}"))
    };
    vec![
        PropSpec {
            name: "lab-plane".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} plane"),
                mesh: Arc::new(mesh("plane").mesh.clone()),
            }),
        },
        PropSpec {
            name: "lab-propeller".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{key} propeller"),
                mesh: Arc::new(mesh("plane-prop").mesh.clone()),
            }),
        },
        PropSpec {
            name: "lab-runway".to_owned(),
            kind: PropKind::Block(Block {
                half: RUNWAY_HALF,
                radius: 0.01,
                segments: 32,
            }),
        },
        PropSpec {
            name: "lab-field".to_owned(),
            kind: PropKind::Block(Block {
                half: [FIELD_HALF, 0.5, FIELD_HALF],
                radius: 0.05,
                segments: 64,
            }),
        },
    ]
}

/// The flying surfaces in the aeroplane's frame (−z forward, +y up, +x right): the wing's two
/// halves rigged 2° up with 4° of dihedral (a high wing's effective dihedral: in a sideslip the
/// lower wing lifts more and levels the aeroplane), the tailplane rigged 1° down, the fin.
fn surfaces() -> [Surface; 4] {
    const DIHEDRAL: (f32, f32) = (0.07, 0.997_55);
    let wing = |x: f32| Surface {
        at: Vec3::new(x, 1.05, -0.28),
        normal: Vec3::new(-x.signum() * DIHEDRAL.0, DIHEDRAL.1, 0.0),
        forward: Vec3::NEG_Z,
        area: 6.6,
        aspect: 7.6,
        rigging: 0.035,
    };
    [
        wing(-2.6),
        wing(2.6),
        Surface {
            at: Vec3::new(0.0, 0.62, 4.54),
            normal: Vec3::Y,
            forward: Vec3::NEG_Z,
            area: 2.5,
            aspect: 4.6,
            rigging: -0.017,
        },
        Surface {
            at: Vec3::new(0.0, 1.4, 4.9),
            normal: Vec3::X,
            forward: Vec3::NEG_Z,
            area: 1.2,
            aspect: 1.4,
            rigging: 0.0,
        },
    ]
}

/// What the flight puts in the world: the runway (drawn) and the aeroplane.
pub(super) struct Field {
    pub statics: Vec<(usize, Mat4)>,
    pub plane: BodyId,
}

/// Builds the field into `world`: the runway drawn with `runway`, the aeroplane resting on its
/// gear at the threshold.
pub(super) fn build(world: &mut World, runway: usize) -> Result<Field> {
    let statics = vec![(
        runway,
        Mat4::from_translation(Vec3::new(0.0, RUNWAY_HALF[1], 0.0)),
    )];
    // Its collision: the hull of its three wheels, a tail skid and its fuselage's top. It rests
    // on its wheels, and rotates for the take-off about the main ones, just behind its weight;
    // the skid stops the tail from going through the ground. Its weight a little ahead of the
    // wing's lift and of the main wheels.
    let (model, _) = plane_model();
    model
        .mesh("plane-shell")
        .context("the aeroplane's model has its shell")?;
    let ground = -1.37;
    let points = [
        Vec3::new(0.0, ground, -1.65),
        Vec3::new(-1.0, ground, MAIN_WHEELS_Z),
        Vec3::new(1.0, ground, MAIN_WHEELS_Z),
        Vec3::new(0.0, -0.35, 3.6),
        Vec3::new(-0.55, 0.65, -1.9),
        Vec3::new(0.55, 0.65, -1.9),
        Vec3::new(-0.45, 0.85, 1.2),
        Vec3::new(0.45, 0.85, 1.2),
        Vec3::new(0.0, 0.7, 3.8),
    ];
    let shape = Shape::convex_hull(&points, 0.03, 100.0)?.with_center_of_mass_at(CENTER_OF_MASS)?;
    let plane = world.add_body(&BodyDesc {
        mass: Some(MASS),
        // Its wheels roll: little friction on the gear's spats.
        friction: 0.04,
        allow_sleep: false,
        ..BodyDesc::dynamic(&shape, START)
    })?;
    Ok(Field { statics, plane })
}

/// The pilot's controls as the world holds them: throttle (0 to 1), elevator (−1 pull, the
/// nose up, to 1 push), ailerons (−1 left to 1 right), rudder (−1 left to 1 right).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Pilot {
    pub plane: Option<BodyId>,
    pub throttle: f32,
    pub elevator: f32,
    pub aileron: f32,
    pub rudder: f32,
}

impl Pilot {
    /// The air's push and the propeller's pull for the coming step.
    pub(super) fn tick(&self, world: &mut World) {
        let Some(plane) = self.plane else {
            return;
        };
        let (mut t, mut v, mut c) = (Vec::new(), Vec::new(), Vec::new());
        world.transforms(&[plane], &mut t);
        world.velocities(&[plane], &mut v);
        world.centers_of_mass(&[plane], &mut c);
        // Rolling right lowers the right wing: its aileron up, the left one down.
        let deflections = [
            self.aileron * AILERON,
            -self.aileron * AILERON,
            self.elevator * ELEVATOR,
            -self.rudder * RUDDER,
        ];
        let (mut force, torque) = push(&surfaces(), &deflections, t[0], v[0], c[0], &Air::STILL);
        // The fuselage's and the gear's drag, and the propeller's pull along the nose.
        let speed = v[0].linear.length();
        force -= 0.5 * Air::STILL.density * FUSELAGE_DRAG * speed * v[0].linear;
        // Down and off its wheels (low, banked or turned over past 45°), its wings and fuselage
        // scrape on the field: the hull's low friction is its wheels', so the scraping is a
        // force of its own, easing off below 0.25 m/s so it never turns the slide back.
        let up = t[0].rotation * Vec3::Y;
        if up.y < 0.7 && t[0].position.y < SCRAPE_HEIGHT {
            let slide = Vec3::new(v[0].linear.x, 0.0, v[0].linear.z);
            force -= SCRAPE * MASS * 9.81 * slide / slide.length().max(0.25);
        }
        let pull = t[0].rotation * Vec3::NEG_Z * (self.throttle.clamp(0.0, 1.0) * THRUST);
        let propeller = t[0].position + (t[0].rotation * PROPELLER).as_dvec3();
        world.push(
            &[plane, plane],
            &[
                (force, t[0].position, torque),
                (pull, propeller, Vec3::ZERO),
            ],
        );
    }

    /// The propeller for the movers, after the bodies: at the nose, turned by the engine (for
    /// the eye only: the angle is not part of the state).
    pub(super) fn propeller(&self, world: &World, tick: u64, out: &mut Vec<Transform>) {
        let Some(plane) = self.plane else {
            return;
        };
        let mut t = Vec::new();
        world.transforms(&[plane], &mut t);
        let turns = (0.3 + 0.7 * self.throttle) * 0.9 * tick as f32;
        let spin = Quat::from_rotation_z(turns);
        out.push(Transform {
            position: t[0].position + (t[0].rotation * PROPELLER).as_dvec3(),
            rotation: t[0].rotation * spin,
        });
    }
}
