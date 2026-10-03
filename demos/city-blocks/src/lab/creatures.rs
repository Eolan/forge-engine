//! `physics-lab --lab creatures` (issue #143, Phase 3's step 7): creatures as powered ragdolls
//! (D-012's physics layer). Two puppets modelled in Blender (`assets/blender/creatures.py`),
//! each part a mesh and each joint an empty: an artist's mannequin on a stand and a dog. Each
//! is a Jolt ragdoll of eleven bodies held by ball joints and hinges whose motors drive it to a
//! pose that moves with the clock (the mannequins swing their arms and turn their heads, the
//! dogs wag their tails and nod), with a strength that lets a thrown ball shove them. The ↓ key
//! lets them go limp, ↑ powers them again; a hard enough blow knocks a mannequin off its stand.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result};
use forge_core::dmath::sin_cos;
use forge_geom::city::{Block, Imported, PropKind, PropSpec};
use forge_geom::model::{Model, ModelMaterial, load_glb};
use forge_physics::{
    BodyDesc, BodyId, JointId, JointLoad, Motors, RagdollId, RagdollJoint, RagdollPart, Shape,
    Transform, World,
};
use glam::{DVec3, Mat4, Quat, Vec3};

/// A part of a kind of creature: its name in the model, its parent's index (parents first),
/// how it turns on it.
struct Part {
    name: &'static str,
    parent: Option<usize>,
    joint: RagdollJoint,
}

const fn ball(cone: f32, twist: f32) -> RagdollJoint {
    RagdollJoint::SwingTwist {
        cone: (cone, cone),
        twist: (-twist, twist),
    }
}

const fn hinge(low: f32, high: f32) -> RagdollJoint {
    RagdollJoint::Hinge { range: (low, high) }
}

const fn part(name: &'static str, parent: Option<usize>, joint: RagdollJoint) -> Part {
    Part {
        name,
        parent,
        joint,
    }
}

/// The kinds: their parts' names after `mannequin-` or `dog-`, in order. An elbow bends the
/// forearm forward, a knee the shin back (the hinge turns about +x, the creature facing −z).
const MANNEQUIN: [Part; 11] = [
    part("pelvis", None, ball(0.0, 0.0)),
    part("chest", Some(0), ball(0.45, 0.5)),
    part("head", Some(1), ball(0.6, 0.9)),
    part("upper-arm-l", Some(1), ball(1.4, 0.8)),
    part("lower-arm-l", Some(3), hinge(-0.05, 2.4)),
    part("upper-arm-r", Some(1), ball(1.4, 0.8)),
    part("lower-arm-r", Some(5), hinge(-0.05, 2.4)),
    part("thigh-l", Some(0), ball(1.0, 0.4)),
    part("shin-l", Some(7), hinge(-2.3, 0.05)),
    part("thigh-r", Some(0), ball(1.0, 0.4)),
    part("shin-r", Some(9), hinge(-2.3, 0.05)),
];
const DOG: [Part; 11] = [
    part("torso", None, ball(0.0, 0.0)),
    part("head", Some(0), ball(0.6, 0.5)),
    part("tail", Some(0), ball(0.9, 0.3)),
    part("upper-front-l", Some(0), ball(0.6, 0.2)),
    part("lower-front-l", Some(3), hinge(-1.4, 0.3)),
    part("upper-front-r", Some(0), ball(0.6, 0.2)),
    part("lower-front-r", Some(5), hinge(-1.4, 0.3)),
    part("upper-hind-l", Some(0), ball(0.6, 0.2)),
    part("lower-hind-l", Some(7), hinge(-1.4, 0.3)),
    part("upper-hind-r", Some(0), ball(0.6, 0.2)),
    part("lower-hind-r", Some(9), hinge(-1.4, 0.3)),
];

/// The two kinds, by their names in the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Mannequin,
    Dog,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Mannequin => "mannequin",
            Self::Dog => "dog",
        }
    }

    fn parts(self) -> &'static [Part; 11] {
        match self {
            Self::Mannequin => &MANNEQUIN,
            Self::Dog => &DOG,
        }
    }

    /// Their wood or flesh, kg/m³, their motors, and their parts' friction.
    fn density(self) -> f32 {
        match self {
            Self::Mannequin => 600.0,
            Self::Dog => 1000.0,
        }
    }

    fn motors(self) -> Motors {
        match self {
            // Its limbs hold their swing to a few degrees; a ball shoves them.
            Self::Mannequin => Motors {
                stiffness: 800.0,
                damping: 30.0,
                torque: 150.0,
            },
            // Its legs carry a 45 kg torso, a few degrees of give.
            Self::Dog => Motors {
                stiffness: 4000.0,
                damping: 150.0,
                torque: 400.0,
            },
        }
    }
}

/// Parts in a kind: the props per kind come in this order.
pub(super) const PARTS: usize = 11;
/// Where they stand: the mannequins in a row, the dogs before them, all facing +z (the
/// camera), each with its own phase in its moves.
const MANNEQUINS_X: [f64; 3] = [-2.2, 0.0, 2.2];
const DOGS: [(f64, f64); 2] = [(-1.1, 1.6), (1.1, 1.6)];
/// A mannequin's stand: its pole's half sizes; it breaks off past this force (N) or torque
/// (N·m).
const POLE_HALF: [f32; 3] = [0.025, 0.44, 0.025];
const STAND_FORCE: f32 = 2000.0;
const STAND_TORQUE: f32 = 400.0;

/// The creatures' model, read once from `assets/models/creatures.glb`, and its cache key.
fn creatures_model() -> &'static (Model, String) {
    static MODEL: OnceLock<(Model, String)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/creatures.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the creatures' model {}: {e}", path.display()));
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, format!("assets/models/creatures.glb#{digest:016x}"))
    })
}

/// A part's mesh about its middle (the middle of its bounds), and that middle in the
/// creature's frame.
fn part_mesh(kind: Kind, k: usize) -> (Vec3, forge_geom::procedural::TriMesh) {
    let (model, _) = creatures_model();
    let name = format!("{}-{}", kind.prefix(), kind.parts()[k].name);
    let mut mesh = model
        .mesh(&name)
        .unwrap_or_else(|| panic!("the creatures' model has no {name}"))
        .mesh
        .clone();
    let (low, high) = mesh.positions.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), &p| (lo.min(Vec3::from(p)), hi.max(Vec3::from(p))),
    );
    let middle = 0.5 * (low + high);
    for p in &mut mesh.positions {
        *p = (Vec3::from(*p) - middle).to_array();
    }
    (middle, mesh)
}

/// The scene's props: each kind's parts (`lab-mannequin@PART`, `lab-dog@PART`, their
/// materials the kind's rows), then the stand's pole.
pub(super) fn props() -> Vec<PropSpec> {
    let (_, key) = creatures_model();
    let mut props = Vec::new();
    for kind in [Kind::Mannequin, Kind::Dog] {
        for (k, p) in kind.parts().iter().enumerate() {
            let (_, mesh) = part_mesh(kind, k);
            props.push(PropSpec {
                name: format!("lab-{}@{}", kind.prefix(), p.name),
                kind: PropKind::Imported(Imported {
                    key: format!("{key} {} {}", kind.prefix(), p.name),
                    mesh: Arc::new(mesh),
                }),
            });
        }
    }
    props.push(PropSpec {
        name: "lab-pole".to_owned(),
        kind: PropKind::Block(Block {
            half: POLE_HALF,
            radius: 0.01,
            segments: 2,
        }),
    });
    props
}

/// Each kind's materials, for its rows: those of the part that has them all (the mannequin's
/// chest: wood and its dark joints; the dog's head: fur and its dark nose and ears).
pub(super) fn materials() -> [(&'static str, Vec<ModelMaterial>); 2] {
    let (model, _) = creatures_model();
    let of = |name: &str| {
        model
            .mesh(name)
            .map(|m| m.materials.clone())
            .unwrap_or_default()
    };
    [
        ("lab-mannequin", of("mannequin-chest")),
        ("lab-dog", of("dog-head")),
    ]
}

/// One creature: its ragdoll and kind, its phase, and its stand's joint (a mannequin's).
#[derive(Clone, Copy, Debug)]
struct Creature {
    ragdoll: RagdollId,
    #[cfg(test)]
    root: BodyId,
    kind: Kind,
    phase: f64,
    stand: Option<JointId>,
}

/// The creatures as the world holds them.
#[derive(Clone, Debug)]
pub(super) struct Herd {
    creatures: Vec<Creature>,
}

/// What the scene puts in the world: the poles drawn, each part's bodies across the
/// creatures of its kind (in the props' order), and the herd.
pub(super) struct Field {
    pub statics: Vec<(usize, Mat4)>,
    pub parts: Vec<Vec<BodyId>>,
    pub herd: Herd,
}

/// Turned to face +z: a half turn about y, exactly.
const FACING: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);

/// Builds the creatures into `world`, the poles drawn with `pole`.
pub(super) fn build(world: &mut World, pole: usize) -> Result<Field> {
    let (model, _) = creatures_model();
    let mut statics = Vec::new();
    let mut parts: Vec<Vec<BodyId>> = vec![Vec::new(); 2 * PARTS];
    let mut creatures = Vec::new();
    let pole_shape = Shape::cuboid(Vec3::from_array(POLE_HALF), 0.01, 0.0)?;
    // The parts' shapes, a kind's at a time: hulls of their meshes' points (every few).
    let shapes = |kind: Kind| -> Result<Vec<(Vec3, Shape)>> {
        (0..PARTS)
            .map(|k| {
                let (middle, mesh) = part_mesh(kind, k);
                let step = (mesh.positions.len() / 200).max(1);
                let points: Vec<Vec3> = mesh
                    .positions
                    .iter()
                    .step_by(step)
                    .map(|&p| Vec3::from(p))
                    .collect();
                Ok((middle, Shape::convex_hull(&points, 0.01, kind.density())?))
            })
            .collect()
    };
    let placed: Vec<(Kind, DVec3, f64)> = MANNEQUINS_X
        .iter()
        .enumerate()
        .map(|(i, &x)| (Kind::Mannequin, DVec3::new(x, 0.0, 0.0), 0.7 * i as f64))
        .chain(
            DOGS.iter()
                .enumerate()
                .map(|(i, &(x, z))| (Kind::Dog, DVec3::new(x, 0.0, z), 1.3 * i as f64)),
        )
        .collect();
    let mannequin = shapes(Kind::Mannequin)?;
    let dog = shapes(Kind::Dog)?;
    for (kind, at, phase) in placed {
        let shapes = if kind == Kind::Mannequin {
            &mannequin
        } else {
            &dog
        };
        let to_world = |p: Vec3| at + (FACING * p).as_dvec3();
        let ragdoll_parts: Vec<RagdollPart> = kind
            .parts()
            .iter()
            .zip(shapes)
            .map(|(p, (middle, shape))| -> Result<RagdollPart> {
                let pivot = match p.parent {
                    Some(_) => model
                        .point(&format!("{}-joint-{}", kind.prefix(), p.name))
                        .with_context(|| format!("the joint of {}'s {}", kind.prefix(), p.name))?,
                    None => *middle,
                };
                // Along the part from its joint, and across it about the creature's x.
                let twist = (*middle - pivot).normalize_or(Vec3::NEG_Y);
                let plane = (Vec3::X - twist * twist.x).normalize_or(Vec3::Z);
                Ok(RagdollPart {
                    shape,
                    at: Transform {
                        position: to_world(*middle),
                        rotation: FACING,
                    },
                    parent: p.parent,
                    joint: p.joint,
                    pivot: to_world(pivot),
                    twist_axis: FACING * twist,
                    plane_axis: FACING * plane,
                    friction: 0.8,
                })
            })
            .collect::<Result<_>>()?;
        let (ragdoll, bodies) = world.add_ragdoll(&ragdoll_parts)?;
        let base = if kind == Kind::Mannequin { 0 } else { PARTS };
        for (k, &body) in bodies.iter().enumerate() {
            parts[base + k].push(body);
        }
        // A mannequin stands on a pole, its pelvis held to it.
        let stand = (kind == Kind::Mannequin).then(|| {
            let top = at + DVec3::new(0.0, 2.0 * f64::from(POLE_HALF[1]), 0.0);
            let middle = top - DVec3::new(0.0, f64::from(POLE_HALF[1]), 0.0);
            statics.push((pole, Mat4::from_translation(middle.as_vec3())));
            world.join_fixed(None, bodies[0], (0, 0))
        });
        if kind == Kind::Mannequin {
            let middle = at + DVec3::new(0.0, f64::from(POLE_HALF[1]), 0.0);
            world.add_body(&BodyDesc::fixed(&pole_shape, middle))?;
        }
        creatures.push(Creature {
            #[cfg(test)]
            root: bodies[0],
            ragdoll,
            kind,
            phase,
            stand,
        });
    }
    Ok(Field {
        statics,
        parts,
        herd: Herd { creatures },
    })
}

/// A turn of `angle` about one of a joint frame's axes (0 x, 1 y, 2 z), as the motors take it.
fn about(axis: usize, angle: f64) -> [f32; 4] {
    let (s, c) = sin_cos(0.5 * angle);
    let mut q = [0.0, 0.0, 0.0, c as f32];
    q[axis] = s as f32;
    q
}

impl Herd {
    /// Before a step at `time` seconds: every creature's motors driven to its pose then, or let
    /// go when `limp`.
    pub(super) fn drive(&self, world: &mut World, time: f64, limp: bool) {
        for c in &self.creatures {
            let t = time + c.phase;
            let wave = |rate: f64| sin_cos(rate * t).0;
            let mut targets = [[0.0_f32, 0.0, 0.0, 1.0]; PARTS];
            match c.kind {
                // Arms swinging about the shoulders' plane axis, elbows bending, the head
                // turning about its neck.
                Kind::Mannequin => {
                    targets[2] = about(0, 0.6 * wave(0.9));
                    targets[3] = about(1, 0.5 * wave(1.6));
                    targets[5] = about(1, -0.5 * wave(1.6));
                    targets[4] = [(0.5 + 0.3 * wave(1.6)) as f32, 0.0, 0.0, 0.0];
                    targets[6] = [(0.5 - 0.3 * wave(1.6)) as f32, 0.0, 0.0, 0.0];
                }
                // The tail wagging about its normal axis, the head nodding.
                Kind::Dog => {
                    targets[2] = about(2, 0.6 * wave(9.0));
                    targets[1] = about(1, 0.25 * wave(1.2));
                }
            }
            let motors = Motors {
                torque: if limp { 0.0 } else { c.kind.motors().torque },
                ..c.kind.motors()
            };
            world.drive_ragdoll(c.ragdoll, &targets, motors);
        }
    }

    /// After a step of `dt`: the stands that carried more than they hold let their mannequins
    /// go. How many.
    pub(super) fn knock(&self, world: &mut World, dt: f32) -> usize {
        let stands: Vec<JointId> = self.creatures.iter().filter_map(|c| c.stand).collect();
        let (mut holding, mut loads) = (Vec::new(), Vec::<JointLoad>::new());
        world.holding(&stands, &mut holding);
        world.joint_loads(&stands, &mut loads);
        let broken: Vec<JointId> = stands
            .iter()
            .zip(holding.iter().zip(&loads))
            .filter(|(_, (h, l))| {
                **h && (l.position > STAND_FORCE * dt || l.rotation > STAND_TORQUE * dt)
            })
            .map(|(&j, _)| j)
            .collect();
        world.set_holding(&broken, false);
        broken.len()
    }

    /// Each creature's kind and where its root (pelvis, torso) is.
    #[cfg(test)]
    pub(super) fn roots(&self, world: &World) -> Vec<(Kind, Transform)> {
        let roots: Vec<BodyId> = self.creatures.iter().map(|c| c.root).collect();
        let mut t = Vec::new();
        world.transforms(&roots, &mut t);
        self.creatures.iter().map(|c| c.kind).zip(t).collect()
    }

    /// The mannequins still on their stands.
    pub(super) fn standing(&self, world: &World) -> usize {
        let stands: Vec<JointId> = self.creatures.iter().filter_map(|c| c.stand).collect();
        let mut holding = Vec::new();
        world.holding(&stands, &mut holding);
        holding.iter().filter(|&&h| h).count()
    }
}
