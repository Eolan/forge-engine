//! `physics-lab --lab creatures` (issues #143 and #165, Phase 3's step 7): creatures as powered
//! ragdolls (D-012's physics layer) drawn as skinned meshes. Two bodies modelled in Blender
//! (`assets/blender/skinned_creatures.py`), each one continuous mesh on an armature of eleven
//! bones: an artist's mannequin on a stand and a dog. Each is a Jolt ragdoll of eleven bodies,
//! one a bone (its hull the vertices the bone carries most, its joint where the bone starts),
//! held by ball joints and hinges whose motors drive it to a pose that moves with the clock (the
//! mannequins swing their arms and turn their heads, the dogs wag their tails and nod), with a
//! strength that lets a thrown ball shove them. The bodies move the bones ([`skin`]), and the
//! mesh bends at the joints. The ↓ key lets them go limp, ↑ powers them again; a hard enough
//! blow knocks a mannequin off its stand.

use std::sync::OnceLock;

use anyhow::Result;
use forge_anim::{Rig, Skeleton, load_rigs};
use forge_core::dmath::sin_cos;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_geom::model::{Model, ModelMesh, load_glb};
use forge_geom::{TriMesh, VertexSkin};
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

    /// Its body.
    pub(super) fn body(self) -> &'static Body {
        let [mannequin, dog] = bodies();
        match self {
            Self::Mannequin => mannequin,
            Self::Dog => dog,
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

/// A kind's body as the skinned model gives it (#165): its mesh in the bind pose, each vertex's
/// joints, and what the ragdoll and the skinning take from them.
pub(super) struct Body {
    /// The bind pose, in the creature's frame (glTF's: +y up, facing −z).
    pub mesh: TriMesh,
    /// Per vertex, its joints in the skin's order.
    pub skin: Vec<VertexSkin>,
    /// Joints in the skin.
    pub joints: usize,
    /// Per part (the kind's order), its joint in the skin's order.
    pub joint: [usize; PARTS],
    /// Per part, the middle of the bounds of the vertices it carries most: where its body is.
    pub middle: [Vec3; PARTS],
    /// Per part, those vertices about its middle: its hull's points.
    points: Vec<Vec<Vec3>>,
    /// Per part, where its bone starts: its joint with its parent.
    pivot: [Vec3; PARTS],
    /// How far from the root's middle any vertex can be, whatever the pose: the skinned mesh's
    /// bounds (the chain of bones to the vertex's joints, plus the vertex's distance from them).
    pub reach: f32,
}

/// The creatures' model and their rigs, read once from `assets/models/skinned-creatures.glb`
/// (`assets/blender/skinned_creatures.py`): the meshes, their textures (#166) and the clips.
pub(super) fn model() -> &'static (Model, Vec<Rig>) {
    static MODEL: OnceLock<(Model, Vec<Rig>)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/skinned-creatures.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the creatures' model {}: {e}", path.display()));
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let rigs = load_rigs(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, rigs)
    })
}

/// The two kinds' bodies, from [`model`].
pub(super) fn bodies() -> &'static [Body; 2] {
    static BODIES: OnceLock<[Body; 2]> = OnceLock::new();
    BODIES.get_or_init(|| {
        let (model, rigs) = model();
        [Kind::Mannequin, Kind::Dog].map(|kind| {
            let prefix = kind.prefix();
            let mesh = model
                .mesh(&format!("{prefix}-body"))
                .unwrap_or_else(|| panic!("the creatures' model has no {prefix}-body"));
            let rig = rigs
                .iter()
                .find(|r| r.name == prefix)
                .unwrap_or_else(|| panic!("the creatures' model has no {prefix} rig"));
            body(kind, mesh, &rig.skeleton)
        })
    })
}

/// `kind`'s body from its mesh and skeleton.
fn body(kind: Kind, model: &ModelMesh, skeleton: &Skeleton) -> Body {
    let parts = kind.parts();
    let skin = model.skin.clone().expect("a skinned body");
    let joint: [usize; PARTS] = std::array::from_fn(|k| {
        let p = &parts[k];
        skeleton
            .joint(p.name)
            .unwrap_or_else(|| panic!("{}'s skeleton has no {}", kind.prefix(), p.name))
    });
    let part_of_joint = |j: usize| joint.iter().position(|&k| k == j);
    // Each vertex goes to the part of its heaviest joint.
    let mut points: Vec<Vec<Vec3>> = vec![Vec::new(); PARTS];
    for (p, s) in model.mesh.positions.iter().zip(&skin) {
        let heaviest = (0..4)
            .max_by(|&a, &b| s.weights[a].total_cmp(&s.weights[b]))
            .expect("four weights");
        if let Some(k) = part_of_joint(usize::from(s.joints[heaviest])) {
            points[k].push(Vec3::from(*p));
        }
    }
    let middle: [Vec3; PARTS] = std::array::from_fn(|k| {
        let (low, high) = points[k].iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        0.5 * (low + high)
    });
    for (k, points) in points.iter_mut().enumerate() {
        for p in points {
            *p -= middle[k];
        }
    }
    let mut model_space = vec![Mat4::IDENTITY; skeleton.len()];
    skeleton.model_space(skeleton.rest(), &mut model_space);
    let pivot: [Vec3; PARTS] = std::array::from_fn(|k| match parts[k].parent {
        Some(_) => model_space[joint[k]].w_axis.truncate(),
        None => middle[k],
    });
    // How far each part's pivot can be from the root's middle: its bones laid end to end.
    let mut chain = [0.0_f32; PARTS];
    for (k, p) in parts.iter().enumerate() {
        if let Some(parent) = p.parent {
            chain[k] = chain[parent] + pivot[k].distance(pivot[parent]);
        }
    }
    let reach = model
        .mesh
        .positions
        .iter()
        .zip(&skin)
        .flat_map(|(p, s)| {
            (0..4).filter(|&i| s.weights[i] > 0.0).filter_map(move |i| {
                part_of_joint(usize::from(s.joints[i]))
                    .map(|k| chain[k] + Vec3::from(*p).distance(pivot[k]))
            })
        })
        .fold(0.0_f32, f32::max);
    Body {
        mesh: model.mesh.clone(),
        skin,
        joints: skeleton.len(),
        joint,
        middle,
        points,
        pivot,
        // A ragdoll's joints give a little under load.
        reach: reach * 1.1 + 0.05,
    }
}

/// The scene's props: the stand's pole (the creatures are skinned meshes, not props).
pub(super) fn props() -> Vec<PropSpec> {
    vec![PropSpec {
        name: "lab-pole".to_owned(),
        kind: PropKind::Block(Block {
            half: POLE_HALF,
            radius: 0.01,
            segments: 2,
        }),
    }]
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

/// What the scene puts in the world: the poles drawn, the creatures' bodies (each creature's
/// parts in its kind's order, the creatures in the herd's), and the herd.
pub(super) struct Field {
    pub statics: Vec<(usize, Mat4)>,
    pub bodies: Vec<BodyId>,
    pub herd: Herd,
}

/// Turned to face +z: a half turn about y, exactly.
const FACING: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);

/// Builds the creatures into `world`, the poles drawn with `pole`.
pub(super) fn build(world: &mut World, pole: usize) -> Result<Field> {
    let mut statics = Vec::new();
    let mut all = Vec::new();
    let mut creatures = Vec::new();
    let pole_shape = Shape::cuboid(Vec3::from_array(POLE_HALF), 0.01, 0.0)?;
    // The parts' shapes, a kind's at a time: hulls of the vertices each carries most (every
    // few).
    let shapes = |kind: Kind| -> Result<Vec<(Vec3, Vec3, Shape)>> {
        let body = kind.body();
        (0..PARTS)
            .map(|k| {
                let step = (body.points[k].len() / 200).max(1);
                let points: Vec<Vec3> = body.points[k].iter().step_by(step).copied().collect();
                let shape = Shape::convex_hull(&points, 0.01, kind.density())?;
                Ok((body.middle[k], body.pivot[k], shape))
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
            .map(|(p, (middle, pivot, shape))| -> Result<RagdollPart> {
                let pivot = *pivot;
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
        all.extend_from_slice(&bodies);
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
        bodies: all,
        herd: Herd { creatures },
    })
}

/// How a creature of `kind` is drawn from its parts' transforms (`parts`, its kind's order,
/// relative to the scene's origin): returns its mover's position, its root's (the mover is not
/// turned), and writes per joint, in the skin's order, the matrix that takes a bind-pose vertex
/// to where the joint's part now carries it, in the mover's frame. A part's body was made at
/// its middle turned by [`FACING`], the creature's bind pose placed by the same turn: a vertex
/// `v` of part `k` is then at `position_k + rotation_k (v − middle_k)`.
pub(super) fn skin(kind: Kind, parts: &[(Vec3, Quat)], out: &mut Vec<Mat4>) -> Vec3 {
    let body = kind.body();
    let root = parts[0].0;
    out.clear();
    out.resize(body.joints, Mat4::IDENTITY);
    for (k, &(position, rotation)) in parts.iter().enumerate() {
        out[body.joint[k]] =
            Mat4::from_rotation_translation(rotation, position - root - rotation * body.middle[k]);
    }
    root
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

    /// Each creature's kind, in the herd's order (its bodies' in [`Field::bodies`]).
    pub(super) fn kinds(&self) -> Vec<Kind> {
        self.creatures.iter().map(|c| c.kind).collect()
    }

    /// The mannequins still on their stands.
    pub(super) fn standing(&self, world: &World) -> usize {
        let stands: Vec<JointId> = self.creatures.iter().filter_map(|c| c.stand).collect();
        let mut holding = Vec::new();
        world.holding(&stands, &mut holding);
        holding.iter().filter(|&&h| h).count()
    }
}
