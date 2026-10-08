//! `physics-lab --lab creatures` (issues #143, #165 and #167, Phase 3's step 7): creatures as
//! powered ragdolls (D-012's physics layer) drawn as skinned meshes. Two bodies modelled in
//! Blender (`assets/blender/skinned_creatures.py`), each one continuous mesh on an armature of
//! eleven bones: an artist's mannequin on a stand and a dog. Each is a Jolt ragdoll of eleven
//! bodies, one a bone (its hull the vertices the bone carries most, its joint where the bone
//! starts), held by ball joints and hinges whose motors drive it to the pose of its clips (#167:
//! idle and walk in turns, each switch inertialized, the mannequins' heads turned to watch the
//! dogs, [`Motion`]), with a strength that lets a thrown ball shove them; a dog's torso is held
//! upright by a spring ([`balance`]). The bodies move the bones ([`skin`]), and the mesh bends at
//! the joints. The ↓ key lets them go limp, ↑ powers them again; a hard enough blow knocks a
//! mannequin off its stand.

use std::sync::OnceLock;

use anyhow::Result;
use forge_anim::{
    Chain, Clip, FootDown, Footfall, Inertializer, Pose, Rig, Skeleton, load_rigs, look_at,
    two_bone_toward,
};
use forge_core::dmath::{atan2, sin_cos};
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_geom::model::{Model, ModelMesh, load_glb};
use forge_geom::{TriMesh, VertexSkin};
use forge_physics::{
    BodyDesc, BodyId, JointId, JointLoad, Motors, RagdollId, RagdollJoint, RagdollPart, Shape,
    Transform, World,
};
use forge_sim::TICK;
use glam::{DVec3, Mat3, Mat4, Quat, Vec2, Vec3};

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

/// Where the creatures stand: the floor, the mannequins on their stands and the dogs before them
/// (#143), or a course of steps and ramps the dogs walk along alone (`--lab course`, #167's
/// feet on uneven ground).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ground {
    Flat,
    Course,
    /// The floor alone, the dogs walking their lanes as on the course (`--lab yard`, #185:
    /// over beds of sand and snow, which the physics does not see).
    Yard,
}

/// A stretch of the course: a lane across x within `COURSE_HALF_WIDTH` of `x`, along z from
/// `z.0` to `z.1`, its top rising from `h.0` to `h.1` (a step where they are equal, a ramp).
#[derive(Clone, Copy, Debug)]
pub(super) struct Stretch {
    x: f64,
    z: (f64, f64),
    h: (f64, f64),
}

const fn stretch(x: f64, z: (f64, f64), h: (f64, f64)) -> Stretch {
    Stretch { x, z, h }
}

/// The course's lanes, 0.8 m wide: on the left three steps of 5 cm up, a landing and down again;
/// on the right a ramp rising 10° (0.175 m over a metre), a landing and down again.
pub(super) const COURSE_HALF_WIDTH: f64 = 0.4;
pub(super) const STEP_RISE: f64 = 0.05;
pub(super) const STEP_RUN: f64 = 0.5;
pub(super) const RAMP_RISE: f64 = 0.175;
const COURSE: [Stretch; 9] = [
    stretch(-0.7, (-0.4, 0.1), (0.05, 0.05)),
    stretch(-0.7, (0.1, 0.6), (0.1, 0.1)),
    stretch(-0.7, (0.6, 1.1), (0.15, 0.15)),
    stretch(-0.7, (1.1, 2.1), (0.15, 0.15)),
    stretch(-0.7, (2.1, 2.6), (0.1, 0.1)),
    stretch(-0.7, (2.6, 3.1), (0.05, 0.05)),
    stretch(0.7, (-0.4, 0.6), (0.0, RAMP_RISE)),
    stretch(0.7, (0.6, 1.6), (RAMP_RISE, RAMP_RISE)),
    stretch(0.7, (1.6, 2.6), (RAMP_RISE, 0.0)),
];
/// The dogs at the course's foot, one a lane, facing up it (+z).
const DOGS_COURSE: [(f64, f64); 2] = [(-0.7, -1.6), (0.7, -1.6)];
/// How thick a ramp's slab is, metres.
pub(super) const RAMP_THICKNESS: f64 = 0.1;

/// The ground's height at (`x`, `z`) on `ground`: the floor's 0, or a stretch's top over it.
fn ground_at(ground: Ground, x: f64, z: f64) -> f64 {
    if ground != Ground::Course {
        return 0.0;
    }
    COURSE
        .iter()
        .filter(|s| (x - s.x).abs() <= COURSE_HALF_WIDTH && (s.z.0..=s.z.1).contains(&z))
        .map(|s| s.h.0 + (s.h.1 - s.h.0) * (z - s.z.0) / (s.z.1 - s.z.0))
        .fold(0.0, f64::max)
}

/// A stretch's slab: its turn (a ramp's about x, from the slope's rise over its run: square
/// roots alone), its middle and its half sizes. A step's stands on the floor; a ramp's top is
/// the slope, the rest of it under the floor or the landing.
fn slab(s: &Stretch) -> (Quat, DVec3, DVec3) {
    let run = s.z.1 - s.z.0;
    let rise = s.h.1 - s.h.0;
    if rise == 0.0 {
        let half = DVec3::new(COURSE_HALF_WIDTH, 0.5 * s.h.0, 0.5 * run);
        return (
            Quat::IDENTITY,
            DVec3::new(s.x, half.y, s.z.0 + half.z),
            half,
        );
    }
    let along = DVec3::new(0.0, rise, run).normalize();
    let turn = Quat::from_rotation_arc(Vec3::Z, along.as_vec3());
    let up = DVec3::new(0.0, along.z, -along.y);
    let top = DVec3::new(s.x, 0.5 * (s.h.0 + s.h.1), 0.5 * (s.z.0 + s.z.1));
    let half = DVec3::new(
        COURSE_HALF_WIDTH,
        0.5 * RAMP_THICKNESS,
        0.5 * (run * run + rise * rise).sqrt(),
    );
    (turn, top - up * half.y, half)
}
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
    /// A dog's paws, front left, front right, hind left, hind right; none for a mannequin.
    paws: Vec<Paw>,
    /// How far from the root's middle any vertex can be, whatever the pose: the skinned mesh's
    /// bounds (the chain of bones to the vertex's joints, plus the vertex's distance from them).
    pub reach: f32,
}

/// A dog's paw as its footfalls see it (#167's foot-down events): its lower leg's part, its pad
/// (the part's vertices within `PAD_HEIGHT` of its lowest, about the part's middle, in the bind
/// pose), the pad's middle there and its half sizes across (x) and along (z, the dog faces −z).
struct Paw {
    part: usize,
    pad: Vec<Vec3>,
    middle: Vec3,
    size: Vec2,
}

/// How high over a lower leg's lowest vertex its pad's reach, metres.
const PAD_HEIGHT: f32 = 0.015;

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
    // A dog's paws: each lower leg's vertices within `PAD_HEIGHT` of its lowest.
    let paws = match kind {
        Kind::Dog => [4, 6, 8, 10]
            .map(|part| {
                let lowest = points[part].iter().map(|p| p.y).fold(f32::MAX, f32::min);
                let pad: Vec<Vec3> = points[part]
                    .iter()
                    .copied()
                    .filter(|p| p.y <= lowest + PAD_HEIGHT)
                    .collect();
                let (low, high) = pad.iter().fold(
                    (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                    |(lo, hi), &p| (lo.min(p), hi.max(p)),
                );
                Paw {
                    part,
                    middle: 0.5 * (low + high),
                    size: 0.5 * Vec2::new(high.x - low.x, high.z - low.z),
                    pad,
                }
            })
            .into(),
        Kind::Mannequin => Vec::new(),
    };
    Body {
        mesh: model.mesh.clone(),
        skin,
        joints: skeleton.len(),
        joint,
        middle,
        points,
        pivot,
        paws,
        // A ragdoll's joints give a little under load.
        reach: reach * 1.1 + 0.05,
    }
}

/// The course's props (#167), after every other scene's in the lab's list: a step's slab (the
/// steps are stacks of them), a ramp's slab and the ramps' landing; then the dogs' paw print.
pub(super) fn course_props() -> Vec<PropSpec> {
    let ramp = slab(&COURSE[6]).2;
    let landing = slab(&COURSE[7]).2;
    [
        (
            "lab-step",
            [COURSE_HALF_WIDTH, 0.5 * STEP_RISE, 0.5 * STEP_RUN],
        ),
        ("lab-dog-ramp", ramp.to_array()),
        ("lab-landing", landing.to_array()),
    ]
    .into_iter()
    .map(|(name, half)| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half: half.map(|h| h as f32),
            radius: 0.008,
            segments: 2,
        }),
    })
    .chain(std::iter::once(PropSpec {
        // A paw print: a disc as wide as a pad (3 by 4 cm in half sizes), 2 mm thick.
        name: "lab-print".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile: vec![
                (0.0, 0.0),
                (PRINT_RADIUS, 0.0),
                (PRINT_RADIUS, PRINT_THICKNESS),
                (0.0, PRINT_THICKNESS),
            ],
            around: 20,
            along: 12,
            flutes: 0,
            flute_depth: 0.0,
            flute_span: (0.0, 0.0),
        }),
    }))
    .collect()
}

/// A drawn paw print's radius and thickness, metres, and how far over the ground it lies.
const PRINT_RADIUS: f32 = 0.035;
const PRINT_THICKNESS: f32 = 0.002;
const PRINT_LIFT: f32 = 0.0005;

/// Where the lab draws its `PRINTS` paw prints, slot by slot, into `out`: flat on the ground
/// at each footfall, turned to its heading; the slots not filled yet, and the footfalls on a
/// soft ground (`soft`'s materials, which keep their own prints, #185), parked out of sight at
/// `parked`.
pub(super) fn prints(feet: &Feet, soft: &[&str], parked: DVec3, out: &mut Vec<Transform>) {
    for k in 0..PRINTS {
        out.push(
            match feet
                .falls
                .get(k)
                .and_then(Option::as_ref)
                .filter(|f| !soft.contains(&f.material))
            {
                Some(f) => {
                    let across = f.normal.cross(f.heading);
                    Transform {
                        position: f.position + (f.normal * PRINT_LIFT).as_dvec3(),
                        rotation: Quat::from_mat3(&Mat3::from_cols(across, f.normal, f.heading)),
                    }
                }
                None => Transform {
                    position: parked,
                    rotation: Quat::IDENTITY,
                },
            },
        );
    }
}

/// Builds the course into `world` (a body a stretch) and returns its drawn slabs, from the props
/// at `props` ([`course_props`]' order), and its bodies with their props' names.
type Course = (Vec<(usize, Mat4)>, Vec<(BodyId, &'static str)>);
fn build_course(world: &mut World, props: usize) -> Result<Course> {
    let mut drawn = Vec::new();
    let mut grounds = Vec::new();
    for s in &COURSE {
        let (turn, middle, half) = slab(s);
        let shape = Shape::cuboid(half.as_vec3(), 0.005, 0.0)?;
        let body = world.add_body(&BodyDesc {
            rotation: turn,
            friction: 0.8,
            ..BodyDesc::fixed(&shape, middle)
        })?;
        let slabs = s.h.0 / STEP_RISE;
        grounds.push((
            body,
            if s.h.0 != s.h.1 {
                "lab-dog-ramp"
            } else if (slabs - slabs.round()).abs() < 1e-6 {
                "lab-step"
            } else {
                "lab-landing"
            },
        ));
        if s.h.0 != s.h.1 {
            drawn.push((
                props + 1,
                Mat4::from_rotation_translation(turn, middle.as_vec3()),
            ));
        } else if (slabs - slabs.round()).abs() < 1e-6 {
            // A step: stacks of slabs, a stack every run.
            let columns = ((s.z.1 - s.z.0) / STEP_RUN).round() as u32;
            for column in 0..columns {
                for level in 0..slabs.round() as u32 {
                    let at = DVec3::new(
                        s.x,
                        STEP_RISE * (f64::from(level) + 0.5),
                        s.z.0 + STEP_RUN * (f64::from(column) + 0.5),
                    );
                    drawn.push((props, Mat4::from_translation(at.as_vec3())));
                }
            }
        } else {
            drawn.push((props + 2, Mat4::from_translation(middle.as_vec3())));
        }
    }
    Ok((drawn, grounds))
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

/// One creature: its ragdoll, its root's body and where that stood as built, its parts' bodies
/// (its kind's order) and its weight (N), its kind, its phase, and its stand's joint (a
/// mannequin's).
#[derive(Clone, Copy, Debug)]
struct Creature {
    ragdoll: RagdollId,
    root: BodyId,
    parts: [BodyId; PARTS],
    weight: f32,
    stance: Transform,
    kind: Kind,
    phase: f64,
    stand: Option<JointId>,
}

/// The creatures as the world holds them.
#[derive(Clone, Debug)]
pub(super) struct Herd {
    creatures: Vec<Creature>,
    ground: Ground,
    /// The course's slabs and their materials' names (the props'); anything else fixed is the
    /// floor.
    grounds: Vec<(BodyId, &'static str)>,
}

/// The dogs' paws as they come down (#167's foot-down events, [`Herd::feel`]): whether each is
/// down (the herd's dogs in order, four paws each), how many footfalls there have been, and the
/// last `PRINTS` of them, the n-th in slot n % `PRINTS` (the lab draws a print in each; a slot no
/// footfall filled since the start or a restore is empty).
#[derive(Clone, Debug, Default)]
pub(super) struct Feet {
    down: Vec<FootDown>,
    count: u64,
    pub falls: Vec<Option<Footfall<&'static str>>>,
}

impl Feet {
    /// What a saved state keeps: the paws down, a bit each, and the footfalls' count.
    pub(super) fn state(&self) -> (u64, u64) {
        let down = self
            .down
            .iter()
            .enumerate()
            .filter(|(_, f)| f.is_down())
            .fold(0, |bits, (k, _)| bits | 1 << k);
        (down, self.count)
    }

    /// Back to a saved state: the slots a later footfall filled stay until the same ticks fill
    /// them again.
    pub(super) fn set_state(&mut self, (down, count): (u64, u64)) {
        // As many paws as the bits hold, before a first step has counted them.
        self.down.resize(64, FootDown::new(FOOT_DOWN));
        for (k, f) in self.down.iter_mut().enumerate() {
            f.set_down(down & 1 << k != 0);
        }
        self.count = count;
    }

    /// Back to the start: no paw down, no footfall.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// The last `n` footfalls (at most `PRINTS`), in their slots' order.
    pub(super) fn last_mut(
        &mut self,
        n: usize,
    ) -> impl Iterator<Item = &mut Footfall<&'static str>> {
        let n = n.min(PRINTS) as u64;
        let first = self.count.saturating_sub(n);
        let slots: Vec<usize> = (first..self.count)
            .map(|k| (k % PRINTS as u64) as usize)
            .collect();
        self.falls
            .iter_mut()
            .enumerate()
            .filter(move |(k, _)| slots.contains(k))
            .filter_map(|(_, f)| f.as_mut())
    }
}

/// A paw is down within `FOOT_DOWN` metres of the ground (and comes down again once it has been
/// twice as high); the ray finding the ground under it starts `FOOT_RAY.0` over its pad and
/// reaches `FOOT_RAY.1` down. The lab keeps the last `PRINTS` footfalls.
const FOOT_DOWN: f32 = 0.02;
const FOOT_RAY: (f64, f32) = (0.1, 0.5);
pub(super) const PRINTS: usize = 96;
/// The world's pull, m/s² (`WorldDesc`'s).
const GRAVITY: f32 = 9.81;

/// What the scene puts in the world: the poles drawn, the creatures' bodies (each creature's
/// parts in its kind's order, the creatures in the herd's), and the herd.
pub(super) struct Field {
    pub statics: Vec<(usize, Mat4)>,
    pub bodies: Vec<BodyId>,
    pub herd: Herd,
}

/// Turned to face +z: a half turn about y, exactly.
const FACING: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);

/// Builds the creatures into `world` on `ground`, the poles drawn with `pole` and the course with
/// the props from `course` ([`course_props`]).
pub(super) fn build(
    world: &mut World,
    pole: usize,
    course: usize,
    ground: Ground,
) -> Result<Field> {
    let mut statics = Vec::new();
    let mut all = Vec::new();
    let mut creatures = Vec::new();
    let mut grounds = Vec::new();
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
    let dog = |i: usize, &(x, z): &(f64, f64)| (Kind::Dog, DVec3::new(x, 0.0, z), 1.3 * i as f64);
    let placed: Vec<(Kind, DVec3, f64)> = match ground {
        Ground::Flat => MANNEQUINS_X
            .iter()
            .enumerate()
            .map(|(i, &x)| (Kind::Mannequin, DVec3::new(x, 0.0, 0.0), 0.7 * i as f64))
            .chain(DOGS.iter().enumerate().map(|(i, d)| dog(i, d)))
            .collect(),
        Ground::Course => {
            let (drawn, slabs) = build_course(world, course)?;
            statics.extend(drawn);
            grounds = slabs;
            DOGS_COURSE
                .iter()
                .enumerate()
                .map(|(i, d)| dog(i, d))
                .collect()
        }
        Ground::Yard => DOGS_COURSE
            .iter()
            .enumerate()
            .map(|(i, d)| dog(i, d))
            .collect(),
    };
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
        let weight = GRAVITY * shapes.iter().map(|(_, _, s)| s.mass()).sum::<f32>();
        creatures.push(Creature {
            root: bodies[0],
            parts: bodies.as_slice().try_into().expect("a part a body"),
            weight,
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
        herd: Herd {
            creatures,
            ground,
            grounds,
        },
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
/// How far a mannequin turns its head to watch a dog: the cosine of the largest turn (the neck's
/// cone is 0.6 rad), and how far over a dog's torso it looks, metres.
const HEAD_TURN: f32 = 0.85;
const DOG_HEAD: f64 = 0.25;

/// A kind's clips and its parts' joint frames as the motors take them (#167's step 3): what
/// turns a pose of its skeleton into its ragdoll's targets.
pub(super) struct Motion {
    rig: &'static Rig,
    /// Its idle and its walk, indices into the rig's clips.
    clips: [usize; 2],
    /// Per part: its joint's frame (Jolt's constraint space: the twist axis, the plane axis ×
    /// the twist axis and the plane axis as x, y and z) in its parent joint's frame at rest, and its
    /// joint's turn there at rest.
    frame: [Quat; PARTS],
    rest: [Quat; PARTS],
    hinge: [bool; PARTS],
    /// Its head's joint, and which way its face looks in that joint's frame (the creature faces
    /// −z).
    head: usize,
    face: Vec3,
    /// A dog's legs as two-bone chains, each to its paw's sole (in its lower leg's joint frame);
    /// none for a mannequin.
    legs: Vec<Chain>,
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
            // Jolt's constraint space: the twist axis, the plane axis × the twist axis, the plane
            // axis (`SwingTwistConstraint`'s and the hinge's). Laid out as twist, plane, normal
            // (#167's first take), a leg's swing fore and aft came out sideways: the dogs walked
            // crabwise.
            let basis = Quat::from_mat3(&Mat3::from_cols(twist, plane.cross(twist), plane));
            frame[k] = turn(body.joint[parent]).inverse() * basis;
            rest[k] = skeleton.rest().rotations[body.joint[k]];
        }
        let head = body.joint[parts.iter().position(|p| p.name == "head").expect("a head")];
        // A dog's legs, each to its paw's sole: the lowest vertex its lower leg carries.
        let legs = match kind {
            Kind::Dog => [(3, 4), (5, 6), (7, 8), (9, 10)]
                .map(|(upper, lower)| {
                    let sole = body.points[lower]
                        .iter()
                        .map(|&p| body.middle[lower] + p)
                        .min_by(|a, b| a.y.total_cmp(&b.y))
                        .expect("a lower leg's vertices");
                    Chain {
                        root: body.joint[upper],
                        middle: body.joint[lower],
                        tip: model[body.joint[lower]].inverse().transform_point3(sole),
                    }
                })
                .to_vec(),
            Kind::Mannequin => Vec::new(),
        };
        Self {
            rig,
            clips: [clip("idle"), clip("walk")],
            frame,
            rest,
            hinge: std::array::from_fn(|k| matches!(parts[k].joint, RagdollJoint::Hinge { .. })),
            head,
            face: turn(head).inverse() * Vec3::NEG_Z,
            legs,
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
                [2.0 * atan2(q.z, q.w), 0.0, 0.0, 0.0]
            } else {
                q.to_array()
            };
        }
        targets
    }

    /// Turns `pose`'s head to look at `target` (in the world) as far as `HEAD_TURN` allows, for
    /// creature `c` standing as built (#167's look-at).
    fn look(&self, c: &Creature, target: DVec3, pose: &mut Pose) {
        let body = c.kind.body();
        // The world's point in the creature's frame: its root stands at its middle, turned.
        let at =
            c.stance.rotation.inverse() * (target - c.stance.position).as_vec3() + body.middle[0];
        let mut model = vec![Mat4::IDENTITY; self.rig.skeleton.len()];
        look_at(
            &self.rig.skeleton,
            pose,
            self.head,
            self.face,
            at,
            HEAD_TURN,
            &mut model,
        );
    }

    /// Puts a dog's paws in `pose` on `ground` under them (#167's feet on uneven ground; on the
    /// `soft` ground over it where there is some, #194), its
    /// torso where its balance holds it: over (`x`, `z`), facing `facing` (the stance turned),
    /// at its stance's height over the ground's mean height under the paws. Each paw is as high
    /// over the ground as the clip has it over the floor, its leg bent to it by two-bone IK; the
    /// ground a paw is put on is the highest within `PAW_AHEAD` before it along the way it
    /// faces, so it is lifted onto a step before it meets its edge. On the floor the clip's pose
    /// is left as it is. Returns the mean height, which the torso is held over.
    fn plant(
        &self,
        c: &Creature,
        ground: Ground,
        soft: &dyn Fn(f64, f64) -> Option<f64>,
        (x, z): (f64, f64),
        facing: Quat,
        before: &Pose,
        pose: &mut Pose,
    ) -> f64 {
        if self.legs.is_empty() || ground == Ground::Flat {
            return 0.0;
        }
        let body = c.kind.body();
        let skeleton = &self.rig.skeleton;
        let mut model = vec![Mat4::IDENTITY; skeleton.len()];
        skeleton.model_space(pose, &mut model);
        let rotation = facing * c.stance.rotation;
        // Where it heads: from its torso to its head.
        let head = body.middle[1] - body.middle[0];
        let forward = Vec3::new(head.x, 0.0, head.z).normalize();
        let ahead = (rotation * forward).as_dvec3();
        let mut position = DVec3::new(x, c.stance.position.y, z);
        // The creature's frame to the world's, through the torso.
        let to_world =
            |position: DVec3, p: Vec3| position + (rotation * (p - body.middle[0])).as_dvec3();
        let paws: Vec<Vec3> = self
            .legs
            .iter()
            .map(|chain| model[chain.middle].transform_point3(chain.tip))
            .collect();
        let under = |at: DVec3| {
            let hard = ground_at(ground, at.x, at.z);
            soft(at.x, at.z).map_or(hard, |top| top.max(hard))
        };
        skeleton.model_space(before, &mut model);
        let paws_before: Vec<Vec3> = self
            .legs
            .iter()
            .map(|chain| model[chain.middle].transform_point3(chain.tip))
            .collect();
        let lift = paws
            .iter()
            .map(|&p| under(to_world(position, p)))
            .sum::<f64>()
            / paws.len() as f64;
        position.y += lift;
        for (k, (&chain, &paw)) in self.legs.iter().zip(&paws).enumerate() {
            // A front leg's lower joint bends forward of the line from its top to its paw
            // (its paw folds back), a hind leg's back (its hock).
            let pole = if k < 2 { forward } else { -forward };
            let at = to_world(position, paw);
            // Swinging: going forward in the clip, from where it was `PAW_BEFORE` earlier.
            let swinging = (paw - paws_before[k]).dot(forward) > 0.0;
            let reach = if swinging { PAW_AHEAD } else { 0.0 };
            let floor = (0..=PAW_SAMPLES)
                .map(|k| under(at + ahead * (reach * f64::from(k) / f64::from(PAW_SAMPLES))))
                .fold(f64::MIN, f64::max);
            // Before a rise, a little higher still: the paw comes onto it from above.
            let clear = if swinging && floor > under(at) + 0.01 {
                PAW_CLEARANCE
            } else {
                0.0
            };
            let target = DVec3::new(at.x, floor + clear + f64::from(paw.y), at.z);
            let target = rotation.inverse() * (target - position).as_vec3() + body.middle[0];
            two_bone_toward(skeleton, pose, chain, target, pole, &mut model);
        }
        lift
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
/// tipped it over. Off when it goes limp; a ball still shoves it. On the floor, while it walks,
/// where it faces turns at `WALK_TURN` radians a second, and it is held on its spot: it walks
/// 0.7 m/s on its legs since #167's frames were set right, 4 m in a walk.
const BALANCE_TURN: (f32, f32) = (3000.0, 300.0);
const BALANCE_HEIGHT: (f32, f32) = (4000.0, 400.0);
const WALK_TURN: f64 = 0.4;
/// The pull towards its lane's line on the course, or its spot on the floor (N a metre, N·s a
/// metre): a walk on the ragdoll's legs alone drifts sideways. And the push to its walk's pace
/// along the course (N a metre a second, up to `WALK_PACE` while it walks, 0 else), as games
/// carry a ragdoll along by its clip's root motion: on its legs alone it walks the floor at
/// about that pace but stalls at a 5 cm step or a 10° ramp, its paws on them without the grip
/// to climb.
const BALANCE_LANE: (f32, f32) = (1500.0, 300.0);
const BALANCE_PACE: f32 = 800.0;
const WALK_PACE: f64 = 0.7;
/// A swinging paw (one the clip moves forward, from where it had it `PAW_BEFORE` seconds
/// earlier) is put over the highest ground within `PAW_AHEAD` before it along the way it faces,
/// in so many samples, and `PAW_CLEARANCE` higher before a rise (#167): lifted onto a step
/// before it meets the edge. The clip starts a swing a few millimetres up and lifts a paw at
/// most 5 cm; told swinging by its height, the paw slid into the riser before it rose, and with
/// less clearance the motors' lag brought it to the edge under the step's top. A paw on the
/// ground is put on the ground under it.
const PAW_BEFORE: f64 = 0.05;
const PAW_AHEAD: f64 = 0.25;
const PAW_SAMPLES: u32 = 6;
/// How much higher than a rise ahead a paw is carried, metres.
const PAW_CLEARANCE: f64 = 0.05;

/// A turn of `yaw` radians about the vertical (`sin_cos`, the same on every machine).
fn facing(yaw: f64) -> Quat {
    let (s, k) = sin_cos(0.5 * yaw);
    Quat::from_xyzw(0.0, s as f32, 0.0, k as f32)
}

/// Pushes creature `c`'s root towards its stance (`BALANCE_TURN`, `BALANCE_HEIGHT`), turned by
/// `yaw` radians about the vertical and raised by `lift` metres (the ground under its paws).
fn balance(world: &mut World, c: &Creature, yaw: f64, lift: f64, guide: Guide) {
    let (mut at, mut moving) = (Vec::new(), Vec::new());
    world.transforms(&[c.root], &mut at);
    world.velocities(&[c.root], &mut moving);
    let (at, moving) = (at[0], moving[0]);
    // The turn back to upright, as an axis times its angle.
    let mut q = facing(yaw) * c.stance.rotation * at.rotation.inverse();
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
    let lift = BALANCE_HEIGHT.0 * (c.stance.position.y + lift - at.position.y) as f32
        - BALANCE_HEIGHT.1 * moving.linear.y;
    let hold =
        |to: f64, now: f64, speed: f32| BALANCE_LANE.0 * (to - now) as f32 - BALANCE_LANE.1 * speed;
    let (side, along) = match guide {
        Guide::Lane { x, pace } => (
            hold(x, at.position.x, moving.linear.x),
            BALANCE_PACE * (pace as f32 - moving.linear.z),
        ),
        Guide::Spot(spot) => (
            hold(spot.x, at.position.x, moving.linear.x),
            hold(spot.z, at.position.z, moving.linear.z),
        ),
    };
    world.push(
        &[c.root],
        &[(Vec3::new(side, lift, along), at.position, torque)],
    );
}

/// Where a dog's balance takes it across the ground: over a lane's line (x) at a pace up it
/// (+z, metres a second), or onto its spot.
#[derive(Clone, Copy, Debug)]
enum Guide {
    Lane { x: f64, pace: f64 },
    Spot(DVec3),
}

/// A part's joint axes from its middle and its pivot, in the creature's frame: along the part
/// from its joint, and across it about the creature's x (as [`build`] makes the joints).
fn axes(middle: Vec3, pivot: Vec3) -> (Vec3, Vec3) {
    let twist = (middle - pivot).normalize_or(Vec3::NEG_Y);
    let plane = (Vec3::X - twist * twist.x).normalize_or(Vec3::Z);
    (twist, plane)
}

/// How long a dog on the course takes to turn about, seconds, at the start of the idle between
/// two walks.
const TURN_ABOUT: f64 = 3.0;
/// Where a course dog's torso turns about (z, metres: before the course and past its end), and
/// how its pace eases into each (metres a second a metre).
const COURSE_ENDS: (f64, f64) = (-1.6, 2.8);
const PACE_GAIN: f64 = 2.0;

/// A course dog's heading `time` seconds into its schedule: its yaw (radians about the vertical,
/// half a turn more after each walk, eased over `TURN_ABOUT`, always the same way round) and
/// which way along z it walks (+1 up the course, −1 back down). A walk of `SEGMENT` takes it
/// over the course, the next one back.
fn course_heading(time: f64) -> (f64, f64) {
    let segment = (time / SEGMENT).floor().max(0.0);
    let into = time - segment * SEGMENT;
    let walks = (segment / 2.0).floor();
    let turned = if (segment as u64).is_multiple_of(2) && segment >= 2.0 {
        let s = (into / TURN_ABOUT).min(1.0);
        walks - 1.0 + s * s * (3.0 - 2.0 * s)
    } else {
        walks
    };
    let way = if (walks as u64).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    (std::f64::consts::PI * turned, way)
}

impl Herd {
    /// Before a step at `time` seconds: every creature's motors driven to its pose then (its
    /// clips, #167), or let go when `limp`. `soft`: the top of the soft ground at (x, z) where
    /// there is some (the yard's beds, #194), which a dog's paws stand on.
    pub(super) fn drive(
        &self,
        world: &mut World,
        time: f64,
        limp: bool,
        soft: &dyn Fn(f64, f64) -> Option<f64>,
    ) {
        // The mannequins watch the nearest dog walk by.
        let dogs: Vec<BodyId> = self
            .creatures
            .iter()
            .filter(|c| c.kind == Kind::Dog)
            .map(|c| c.root)
            .collect();
        let mut dogs_at = Vec::new();
        world.transforms(&dogs, &mut dogs_at);
        for c in &self.creatures {
            let motion = c.kind.motion();
            let mut pose = motion.rig.skeleton.rest().clone();
            motion.pose_at(time + c.phase, &mut pose);
            if c.stand.is_some() && !limp {
                let flat = |p: DVec3| DVec3::new(p.x, 0.0, p.z);
                let nearest = dogs_at.iter().min_by(|a, b| {
                    let (da, db) = (
                        flat(a.position).distance_squared(flat(c.stance.position)),
                        flat(b.position).distance_squared(flat(c.stance.position)),
                    );
                    da.total_cmp(&db)
                });
                if let Some(dog) = nearest {
                    motion.look(c, dog.position + DVec3::new(0.0, DOG_HEAD, 0.0), &mut pose);
                }
            }
            // A dog on the course: its paws on the ground under them, its torso held over it.
            // Where it faces: on the floor turning as it walks; on the course up it or back
            // down (`way`, +1 or −1 along z), turning about between its walks.
            let (yaw, way) = match self.ground {
                Ground::Flat => (WALK_TURN * motion.walked(time + c.phase), 1.0),
                Ground::Course | Ground::Yard => course_heading(time + c.phase),
            };
            let mut lift = 0.0;
            let mut torso = Vec::new();
            world.transforms(&[c.root], &mut torso);
            let at = torso[0].position;
            if c.stand.is_none() && !limp {
                let mut before = motion.rig.skeleton.rest().clone();
                motion.pose_at(time + c.phase - PAW_BEFORE, &mut before);
                lift = motion.plant(
                    c,
                    self.ground,
                    soft,
                    (at.x, at.z),
                    facing(yaw),
                    &before,
                    &mut pose,
                );
            }
            let targets = motion.targets(c.kind.body(), &pose);
            let motors = Motors {
                torque: if limp { 0.0 } else { c.kind.motors().torque },
                ..c.kind.motors()
            };
            world.drive_ragdoll(c.ragdoll, &targets, motors);
            if c.stand.is_none() && !limp {
                // On the course: over its lane, at its walk's pace while it walks. On the floor:
                // on its spot, which it walks about as it turns.
                let walking =
                    motion.walked(time + c.phase) > motion.walked(time + c.phase - f64::from(TICK));
                // Easing to a stop at the end it heads for, so its walks back and forth keep to
                // the course.
                let end = if way > 0.0 {
                    COURSE_ENDS.1
                } else {
                    COURSE_ENDS.0
                };
                let pace = ((end - at.z) * PACE_GAIN).clamp(-WALK_PACE, WALK_PACE);
                let guide = match self.ground {
                    Ground::Course | Ground::Yard => Guide::Lane {
                        x: c.stance.position.x,
                        pace: if walking { pace } else { 0.0 },
                    },
                    Ground::Flat => Guide::Spot(c.stance.position),
                };
                balance(world, c, yaw, lift, guide);
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

    /// After a step, at tick `tick`: the dogs' paws that came down in it, as footfalls into
    /// `feet` (#167's foot-down events). A paw is as high over the ground as its pad's lowest
    /// point over what a ray finds under the pad's middle, among the fixed bodies (the paw's own
    /// leg is in the way of any other). It presses with its dog's weight shared among the paws
    /// down after this step, over its pad (an ellipse of its half sizes). On `soft` ground (#194:
    /// its top at (x, z) where there is some, as [`Herd::drive`] takes it) a paw is as high over
    /// that, where it stands over what the ray finds. How many came down.
    pub(super) fn feel(
        &self,
        world: &World,
        tick: u64,
        feet: &mut Feet,
        soft: &dyn Fn(f64, f64) -> Option<f64>,
    ) -> usize {
        let dogs = self.creatures.iter().filter(|c| c.kind == Kind::Dog);
        feet.down
            .resize(4 * dogs.clone().count(), FootDown::new(FOOT_DOWN));
        let mut at = Vec::new();
        let mut came = 0;
        for (d, c) in dogs.enumerate() {
            let body = c.kind.body();
            let parts: Vec<BodyId> = body.paws.iter().map(|p| c.parts[p.part]).collect();
            world.transforms(&parts, &mut at);
            let mut falls = Vec::new();
            for (k, (paw, t)) in body.paws.iter().zip(&at).enumerate() {
                let rotation = t.rotation;
                let lowest = paw
                    .pad
                    .iter()
                    .map(|&p| t.position.y + f64::from((rotation * p).y))
                    .fold(f64::MAX, f64::min);
                let middle = t.position + (rotation * paw.middle).as_dvec3();
                let from = middle + DVec3::new(0.0, FOOT_RAY.0, 0.0);
                let Some(hit) = world.cast_ray_still(from, Vec3::new(0.0, -FOOT_RAY.1, 0.0)) else {
                    continue;
                };
                let hard = from.y - FOOT_RAY.1 as f64 * f64::from(hit.fraction);
                // On soft ground (#194): down over its top, up again over the hard ground under
                // it. A paw stands in its own print, below the top beside it, which its middle
                // crosses before it has risen twice the height over the print's floor: judged
                // up by the top, it stayed down across a bed and came down at its edges alone.
                let ground = soft(middle.x, middle.z).map_or(hard, |top| top.max(hard));
                let foot = &mut feet.down[4 * d + k];
                let over = if foot.is_down() { hard } else { ground };
                if foot.update((lowest - over) as f32) {
                    let material = self
                        .grounds
                        .iter()
                        .find(|(b, _)| *b == hit.body)
                        .map_or("lab-floor", |&(_, name)| name);
                    // Where the paw points, along the ground.
                    let forward = rotation * Vec3::NEG_Z;
                    let heading =
                        (forward - hit.normal * forward.dot(hit.normal)).normalize_or(Vec3::NEG_Z);
                    falls.push(Footfall {
                        tick,
                        position: DVec3::new(middle.x, ground, middle.z),
                        normal: hit.normal,
                        heading,
                        size: paw.size,
                        pressure: 0.0,
                        material,
                    });
                }
            }
            // The weight on the paws down now, over each one's pad.
            let down = feet.down[4 * d..4 * d + 4]
                .iter()
                .filter(|f| f.is_down())
                .count()
                .max(1);
            for mut fall in falls {
                let area = std::f32::consts::PI * fall.size.x * fall.size.y;
                fall.pressure = c.weight / down as f32 / area;
                feet.falls.resize(PRINTS, None);
                feet.falls[(feet.count % PRINTS as u64) as usize] = Some(fall);
                feet.count += 1;
                came += 1;
            }
        }
        came
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
    fn a_leg_swung_fore_and_aft_turns_its_joint_about_the_plane_axis() {
        // A dog's front leg swung forward 0.4 rad about the model's x, as the walk swings it: in
        // Jolt's constraint space (twist, plane × twist, plane) that is a turn about z. Laid out
        // as twist, plane, normal it came out about y, a swing sideways: the dogs walked
        // crabwise (#167).
        let (motion, body) = (Kind::Dog.motion(), Kind::Dog.body());
        let skeleton = &motion.rig.skeleton;
        let mut pose = skeleton.rest().clone();
        let mut model = vec![Mat4::IDENTITY; skeleton.len()];
        skeleton.model_space(&pose, &mut model);
        let k = 3; // upper-front-l, on the torso
        let parent = model[body.joint[0]].to_scale_rotation_translation().1;
        let j = body.joint[k];
        pose.rotations[j] =
            parent.inverse() * Quat::from_rotation_x(0.4) * parent * pose.rotations[j];
        let q = Quat::from_array(motion.targets(body, &pose)[k]);
        // Nothing about y (sideways); most of the half angle's sine (0.199) about z, the rest a
        // twist (x: the leg's axis leans a little from the vertical).
        assert!(
            q.y.abs() < 1e-3 && q.z.abs() > 0.18 && q.x.abs() < 0.07,
            "{q:?}"
        );
    }

    #[test]
    fn the_course_stands_where_the_paws_think_the_ground_is() {
        // The slabs as built, and rays down onto them: their tops are `ground_at`'s heights.
        let mut world = World::new(&forge_physics::WorldDesc::default());
        build(&mut world, 0, 0, Ground::Course).expect("the course");
        for s in &COURSE {
            for (dx, f) in [(0.3, 0.1), (-0.2, 0.5), (0.1, 0.9)] {
                let (x, z) = (s.x + dx, s.z.0 + f * (s.z.1 - s.z.0));
                let hit = world
                    .cast_ray(DVec3::new(x, 2.0, z), Vec3::new(0.0, -3.0, 0.0))
                    .expect("the course");
                let top = 2.0 - 3.0 * f64::from(hit.fraction);
                let ground = ground_at(Ground::Course, x, z);
                assert!(
                    (top - ground).abs() < 0.005,
                    "at {x} {z}: {top} against {ground}"
                );
            }
        }
        assert_eq!(ground_at(Ground::Course, 0.0, 0.0), 0.0);
        assert_eq!(ground_at(Ground::Flat, -0.7, 1.5), 0.0);
    }

    /// A footfall and its dog.
    type Fall = (usize, Footfall<&'static str>);

    /// The course's dogs over `seconds`: per dog, its torso's highest tilt (degrees), lowest
    /// height over the ground under it, the highest ground it stood over, its last z after
    /// walking up and the lowest z it came back to after that; the final transforms; and every
    /// footfall, with its dog.
    fn walk_the_course(seconds: u32) -> (Vec<[f64; 5]>, Vec<Transform>, Vec<Fall>) {
        let mut world = World::new(&forge_physics::WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(20.0, 0.5, 20.0), 0.05, 0.0).expect("a floor");
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .expect("the floor");
        let field = build(&mut world, 0, 0, Ground::Course).expect("the course");
        let roots: Vec<BodyId> = field.herd.creatures.iter().map(|c| c.root).collect();
        let mut seen = vec![[0.0, f64::MAX, 0.0, 0.0, f64::MAX]; roots.len()];
        let mut at = Vec::new();
        let (mut feet, mut falls) = (Feet::default(), Vec::new());
        for tick in 0..seconds * 60 {
            let time = f64::from(tick) * f64::from(TICK);
            field.herd.drive(&mut world, time, false, &|_, _| None);
            world.step(TICK, 1).expect("a step");
            let came = field
                .herd
                .feel(&world, u64::from(tick), &mut feet, &|_, _| None);
            for n in feet.count - came as u64..feet.count {
                let fall = feet.falls[(n % PRINTS as u64) as usize].expect("a footfall");
                // The steps' dog walks the lane at x < 0, the ramp's the other.
                falls.push((usize::from(fall.position.x > 0.0), fall));
            }
            world.transforms(&roots, &mut at);
            for (s, t) in seen.iter_mut().zip(&at) {
                let up = f64::from((t.rotation * Vec3::Y).y.clamp(-1.0, 1.0));
                let ground = ground_at(Ground::Course, t.position.x, t.position.z);
                s[0] = s[0].max(up.acos().to_degrees());
                s[1] = s[1].min(t.position.y - ground);
                s[2] = s[2].max(ground);
                // Its first walk up ends by 13 s for either phase, the walk back by 25 s.
                if time < 13.0 {
                    s[3] = t.position.z;
                } else {
                    s[4] = s[4].min(t.position.z);
                }
            }
        }
        (seen, at, falls)
    }

    #[test]
    fn the_dogs_walk_the_course_there_and_back_upright_and_replay() {
        let (seen, last, falls) = walk_the_course(26);
        for (dog, [tilt, low, high, up, back]) in seen.iter().enumerate() {
            assert!(*tilt < 8.0, "dog {dog} tilted {tilt}°");
            assert!(*low > 0.5, "dog {dog} sank to {low} m over the ground");
            // Over the landing (15 cm of steps, the ramp's 17.5), past it, and back before it.
            assert!(*high > 0.14, "dog {dog} stood over {high} m at most");
            assert!(*up > 1.5, "dog {dog} walked up to {up}");
            assert!(*back < -1.0, "dog {dog} came back to {back}");
        }
        // Their paws came down (#167's foot-down events): on the ground where the course has
        // it, on its slope and its material, a few times a second while they walk.
        for dog in 0..2 {
            let theirs: Vec<&Footfall<&str>> = falls
                .iter()
                .filter(|(d, _)| *d == dog)
                .map(|(_, f)| f)
                .collect();
            assert!(theirs.len() > 40, "dog {dog}: {} footfalls", theirs.len());
            for f in theirs {
                let (x, z) = (f.position.x, f.position.z);
                let under = COURSE
                    .iter()
                    .filter(|s| (x - s.x).abs() < COURSE_HALF_WIDTH && (s.z.0..s.z.1).contains(&z))
                    .max_by(|a, b| a.h.0.max(a.h.1).total_cmp(&b.h.0.max(b.h.1)));
                let (material, slope) = match under {
                    Some(s) if s.h.0 != s.h.1 => {
                        ("lab-dog-ramp", (s.h.1 - s.h.0) / (s.z.1 - s.z.0))
                    }
                    Some(s) if ((s.h.0 / STEP_RISE) - (s.h.0 / STEP_RISE).round()).abs() < 1e-6 => {
                        ("lab-step", 0.0)
                    }
                    Some(_) => ("lab-landing", 0.0),
                    None => ("lab-floor", 0.0),
                };
                let ground = ground_at(Ground::Course, x, z);
                // Off the edge of a step, a ray under the pad's middle may find the step's side.
                if (f.position.y - ground).abs() > 0.005 {
                    continue;
                }
                assert_eq!(f.material, material, "dog {dog} at {x} {z}");
                let tilt = Vec3::new(0.0, 1.0, -slope as f32).normalize();
                assert!(
                    f.normal.abs_diff_eq(tilt, 0.01),
                    "dog {dog} at {x} {z}: {}",
                    f.normal
                );
                assert!(f.heading.dot(f.normal).abs() < 1e-4, "{}", f.heading);
                // A dog of 72 kg on pads of about 3 by 4 cm: 40 to 190 kPa.
                assert!(
                    (30e3..250e3).contains(&f.pressure),
                    "dog {dog}: {} Pa",
                    f.pressure
                );
            }
        }
        let (_, again, falls_again) = walk_the_course(26);
        assert_eq!(last, again);
        assert_eq!(falls, falls_again);
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
