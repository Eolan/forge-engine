//! `physics-lab --lab creatures` (issues #143, #165 and #167, Phase 3's step 7): creatures as
//! powered ragdolls (D-012's physics layer) drawn as skinned meshes. Two bodies modelled in
//! Blender (`assets/blender/skinned_creatures.py`), each one continuous mesh on an armature of
//! eleven bones: an artist's mannequin on a stand and a dog. Each is a Jolt ragdoll of eleven
//! bodies, one a bone (its hull the vertices the bone carries most, its joint where the bone
//! starts), held by ball joints and hinges whose motors drive it to the pose of its clips (#167:
//! idle and walk in turns, each switch inertialized, [`Motion`]), with a strength that lets a
//! thrown ball shove them; a dog's torso is held upright by a spring ([`balance`]). The bodies
//! move the bones ([`skin`]), and the mesh bends at the joints. The ↓ key lets them go limp, ↑
//! powers them again; a hard enough blow knocks a mannequin off its stand.

use std::sync::OnceLock;

use anyhow::Result;
use forge_anim::{Clip, Inertializer, Pose, Rig, Skeleton, load_rigs};
use forge_core::dmath::{atan2, sin_cos};
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_geom::model::{Model, ModelMesh, load_glb};
use forge_geom::{TriMesh, VertexSkin};
use forge_physics::{
    BodyDesc, BodyId, JointId, JointLoad, Motors, RagdollId, RagdollJoint, RagdollPart, Shape,
    Transform, World,
};
use forge_sim::TICK;
use glam::{DVec3, Mat3, Mat4, Quat, Vec3};

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
/// forearm forward, a knee the shin back (the hinge turns about +x, the creature facing −z). A
/// dog's front legs fold back at their lower joint and its hind legs forward, as hocks do (its
/// walk clip bends them so, #167).
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
    part("lower-hind-l", Some(7), hinge(-0.3, 1.4)),
    part("upper-hind-r", Some(0), ball(0.6, 0.2)),
    part("lower-hind-r", Some(9), hinge(-0.3, 1.4)),
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

    /// Its clips and how a pose drives its motors (#167).
    pub(super) fn motion(self) -> &'static Motion {
        let [mannequin, dog] = motions();
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
/// (N·m). Walking on it loads it with up to 2.9 kN and 400 N·m, its legs brushing the pole; a
/// thrown ball with ten times that (#167: at 2 kN and 400 N·m, walking threw them all off).
const POLE_HALF: [f32; 3] = [0.025, 0.44, 0.025];
const STAND_FORCE: f32 = 5000.0;
const STAND_TORQUE: f32 = 1000.0;

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

/// One creature: its ragdoll, its root's body and where that stood as built, its kind, its
/// phase, and its stand's joint (a mannequin's).
#[derive(Clone, Copy, Debug)]
struct Creature {
    ragdoll: RagdollId,
    root: BodyId,
    stance: Transform,
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
                let (twist, plane) = axes(*middle, pivot);
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
            root: bodies[0],
            stance: ragdoll_parts[0].at,
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

/// How long a creature plays a clip before the other (idle, walk, idle…), seconds, and how long
/// a switch takes to die away (#167).
const SEGMENT: f64 = 6.0;
const SWITCH: f32 = 0.4;

/// A kind's clips and its parts' joint frames as the motors take them (#167's step 3): what
/// turns a pose of its skeleton into its ragdoll's targets.
pub(super) struct Motion {
    rig: &'static Rig,
    /// Its idle and its walk, indices into the rig's clips.
    clips: [usize; 2],
    /// Per part: its joint's frame (the twist axis, the plane axis and their normal as x, y and
    /// z) in its parent joint's frame at rest, and its joint's turn there at rest.
    frame: [Quat; PARTS],
    rest: [Quat; PARTS],
    hinge: [bool; PARTS],
}

impl Motion {
    fn new(kind: Kind, rig: &'static Rig) -> Self {
        let body = kind.body();
        let parts = kind.parts();
        let skeleton = &rig.skeleton;
        let clip = |name: &str| {
            let name = format!("{}-{name}", kind.prefix());
            rig.clips
                .iter()
                .position(|c| c.name == name)
                .unwrap_or_else(|| panic!("the creatures' model has no clip {name}"))
        };
        let mut model = vec![Mat4::IDENTITY; skeleton.len()];
        skeleton.model_space(skeleton.rest(), &mut model);
        let turn = |j: usize| model[j].to_scale_rotation_translation().1;
        let mut frame = [Quat::IDENTITY; PARTS];
        let mut rest = [Quat::IDENTITY; PARTS];
        for k in 0..PARTS {
            let Some(parent) = parts[k].parent else {
                continue;
            };
            // The axes the ragdoll's joint was built with (`build`), in the model's frame.
            let (twist, plane) = axes(body.middle[k], body.pivot[k]);
            let basis = Quat::from_mat3(&Mat3::from_cols(twist, plane, twist.cross(plane)));
            frame[k] = turn(body.joint[parent]).inverse() * basis;
            rest[k] = skeleton.rest().rotations[body.joint[k]];
        }
        Self {
            rig,
            clips: [clip("idle"), clip("walk")],
            frame,
            rest,
            hinge: std::array::from_fn(|k| matches!(parts[k].joint, RagdollJoint::Hinge { .. })),
        }
    }

    /// The motors' targets for `pose`, a part each (the root's unused): a ball joint's turn in
    /// its frame, a hinge's angle in x. The rest pose gives the pose the ragdoll was built in.
    fn targets(&self, body: &Body, pose: &Pose) -> [[f32; 4]; PARTS] {
        let mut targets = [[0.0, 0.0, 0.0, 1.0]; PARTS];
        for (k, target) in targets.iter_mut().enumerate().skip(1) {
            // The joint's turn from rest, in its parent's frame, then in the joint's.
            let change = pose.rotations[body.joint[k]] * self.rest[k].inverse();
            let mut q = (self.frame[k].inverse() * change * self.frame[k]).normalize();
            if q.w < 0.0 {
                q = -q;
            }
            *target = if self.hinge[k] {
                [2.0 * atan2(q.y, q.w), 0.0, 0.0, 0.0]
            } else {
                q.to_array()
            };
        }
        targets
    }

    /// Seconds spent walking `time` seconds into a creature's schedule (its odd segments).
    fn walked(&self, time: f64) -> f64 {
        let segment = (time / SEGMENT).floor().max(0.0);
        let walks = (segment / 2.0).floor() * SEGMENT;
        if segment as u64 % 2 == 1 {
            walks + (time - segment * SEGMENT)
        } else {
            walks
        }
    }

    /// The pose `time` seconds into a creature's schedule: idle and walk in turns of `SEGMENT`,
    /// each switch dying away over `SWITCH` (inertialized, evaluated afresh from the clock, so a
    /// replay or a rollback gets the same).
    fn pose_at(&self, time: f64, out: &mut Pose) {
        let skeleton = &self.rig.skeleton;
        let segment = (time / SEGMENT).floor().max(0.0);
        let into = (time - segment * SEGMENT) as f32;
        let clip = |segment: f64| &self.rig.clips[self.clips[segment as usize % 2]];
        let sample = |c: &Clip, t: f32, out: &mut Pose| c.sample(skeleton, c.wrap(t), out);
        sample(clip(segment), into, out);
        if segment >= 1.0 && into < SWITCH {
            let (before, now) = (clip(segment - 1.0), clip(segment));
            let ran = SEGMENT as f32;
            let [mut from, mut from_before, mut to, mut to_before] =
                std::array::from_fn(|_| out.clone());
            sample(before, ran, &mut from);
            sample(before, ran - TICK, &mut from_before);
            sample(now, 0.0, &mut to);
            sample(now, -TICK, &mut to_before);
            let mut inertia = Inertializer::default();
            inertia.start((&from, &from_before), (&to, &to_before), TICK, SWITCH);
            inertia.apply_at(out, into);
        }
    }
}

/// The two kinds' motions, from [`model`].
fn motions() -> &'static [Motion; 2] {
    static MOTIONS: OnceLock<[Motion; 2]> = OnceLock::new();
    MOTIONS.get_or_init(|| {
        let (_, rigs) = model();
        [Kind::Mannequin, Kind::Dog].map(|kind| {
            let rig = rigs
                .iter()
                .find(|r| r.name == kind.prefix())
                .unwrap_or_else(|| panic!("the creatures' model has no {} rig", kind.prefix()));
            Motion::new(kind, rig)
        })
    })
}

/// A dog's balance (#167): its torso pulled towards where it stood as built, upright and at its
/// height, by springs with damping (N·m a radian and N·m·s a radian; N a metre and N·s a metre),
/// as games hold up their powered ragdolls. On its motors alone a walk, two feet down at a time,
/// tipped it over. Off when it goes limp; a ball still shoves it. While it walks, where it faces
/// turns at `WALK_TURN` radians a second, so it walks a circle about its spot and stays in view.
const BALANCE_TURN: (f32, f32) = (3000.0, 300.0);
const BALANCE_HEIGHT: (f32, f32) = (4000.0, 400.0);
const WALK_TURN: f64 = 0.4;

/// Pushes creature `c`'s root towards its stance (`BALANCE_TURN`, `BALANCE_HEIGHT`), turned by
/// `yaw` radians about the vertical.
fn balance(world: &mut World, c: &Creature, yaw: f64) {
    let (mut at, mut moving) = (Vec::new(), Vec::new());
    world.transforms(&[c.root], &mut at);
    world.velocities(&[c.root], &mut moving);
    let (at, moving) = (at[0], moving[0]);
    // The turn back to upright, as an axis times its angle.
    let (s, k) = sin_cos(0.5 * yaw);
    let facing = Quat::from_xyzw(0.0, s as f32, 0.0, k as f32);
    let mut q = facing * c.stance.rotation * at.rotation.inverse();
    if q.w < 0.0 {
        q = -q;
    }
    let v = Vec3::new(q.x, q.y, q.z);
    let sine = v.length();
    let back = if sine > 1e-6 {
        v * (2.0 * atan2(sine, q.w) / sine)
    } else {
        Vec3::ZERO
    };
    let torque = BALANCE_TURN.0 * back - BALANCE_TURN.1 * moving.angular;
    let lift = BALANCE_HEIGHT.0 * (c.stance.position.y - at.position.y) as f32
        - BALANCE_HEIGHT.1 * moving.linear.y;
    world.push(
        &[c.root],
        &[(Vec3::new(0.0, lift, 0.0), at.position, torque)],
    );
}

/// A part's joint axes from its middle and its pivot, in the creature's frame: along the part
/// from its joint, and across it about the creature's x (as [`build`] makes the joints).
fn axes(middle: Vec3, pivot: Vec3) -> (Vec3, Vec3) {
    let twist = (middle - pivot).normalize_or(Vec3::NEG_Y);
    let plane = (Vec3::X - twist * twist.x).normalize_or(Vec3::Z);
    (twist, plane)
}

impl Herd {
    /// Before a step at `time` seconds: every creature's motors driven to its pose then (its
    /// clips, #167), or let go when `limp`.
    pub(super) fn drive(&self, world: &mut World, time: f64, limp: bool) {
        for c in &self.creatures {
            let motion = c.kind.motion();
            let mut pose = motion.rig.skeleton.rest().clone();
            motion.pose_at(time + c.phase, &mut pose);
            let targets = motion.targets(c.kind.body(), &pose);
            let motors = Motors {
                torque: if limp { 0.0 } else { c.kind.motors().torque },
                ..c.kind.motors()
            };
            world.drive_ragdoll(c.ragdoll, &targets, motors);
            if c.stand.is_none() && !limp {
                balance(world, c, WALK_TURN * motion.walked(time + c.phase));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rest_pose_drives_the_ragdolls_to_the_pose_they_were_built_in() {
        for kind in [Kind::Mannequin, Kind::Dog] {
            let (motion, body) = (kind.motion(), kind.body());
            // Each part's joint hangs from its parent's in the skeleton too.
            for (k, part) in kind.parts().iter().enumerate() {
                if let Some(parent) = part.parent {
                    assert_eq!(
                        motion.rig.skeleton.parent(body.joint[k]),
                        Some(body.joint[parent]),
                        "{:?}'s {}",
                        kind,
                        part.name
                    );
                }
            }
            let targets = motion.targets(body, motion.rig.skeleton.rest());
            for (k, t) in targets.iter().enumerate().skip(1) {
                let identity = if motion.hinge[k] {
                    t[0].abs() < 1e-5
                } else {
                    Quat::from_array(*t).abs_diff_eq(Quat::IDENTITY, 1e-5)
                };
                assert!(identity, "{kind:?}'s part {k}: {t:?}");
            }
        }
    }

    #[test]
    fn the_walks_bend_the_hinges_within_their_limits() {
        for kind in [Kind::Mannequin, Kind::Dog] {
            let (motion, body) = (kind.motion(), kind.body());
            let walk = &motion.rig.clips[motion.clips[1]];
            let mut pose = motion.rig.skeleton.rest().clone();
            let (mut lowest, mut highest) = ([f32::MAX; PARTS], [f32::MIN; PARTS]);
            for step in 0..60 {
                walk.sample(
                    &motion.rig.skeleton,
                    walk.duration * step as f32 / 60.0,
                    &mut pose,
                );
                let targets = motion.targets(body, &pose);
                for k in 0..PARTS {
                    lowest[k] = lowest[k].min(targets[k][0]);
                    highest[k] = highest[k].max(targets[k][0]);
                }
            }
            for (k, part) in kind.parts().iter().enumerate() {
                if let RagdollJoint::Hinge { range } = part.joint {
                    // Bent the way the hinge bends, and bent at some point of the walk.
                    assert!(
                        lowest[k] >= range.0 - 0.1 && highest[k] <= range.1 + 0.1,
                        "{kind:?}'s {}: {}..{} against {range:?}",
                        part.name,
                        lowest[k],
                        highest[k]
                    );
                    assert!(
                        highest[k] - lowest[k] > 0.1,
                        "{kind:?}'s {} still",
                        part.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_switch_between_clips_carries_the_pose_on_without_a_jump() {
        let motion = Kind::Mannequin.motion();
        let mut pose = motion.rig.skeleton.rest().clone();
        let mut previous = pose.clone();
        let mut biggest = 0.0_f32;
        // Across the first switch, idle to walk, a tick at a time.
        let ticks = (2.0 / f64::from(TICK)) as usize;
        let start = SEGMENT - 1.0;
        for t in 0..ticks {
            previous.clone_from(&pose);
            motion.pose_at(start + t as f64 * f64::from(TICK), &mut pose);
            if t > 0 {
                let change = pose
                    .rotations
                    .iter()
                    .zip(&previous.rotations)
                    .map(|(a, b)| a.angle_between(*b))
                    .fold(0.0, f32::max);
                biggest = biggest.max(change);
            }
        }
        // The walk alone turns a joint by under 0.1 rad a tick; the switch adds no jump.
        assert!(biggest < 0.12, "a joint turned {biggest} rad in a tick");
    }
}
