//! `forge-physics` — rigid bodies through Jolt Physics 5.6 (D-009, issue #136).
//!
//! Jolt is built from `third_party/jolt` with `CROSS_PLATFORM_DETERMINISTIC` and
//! `JPH_DOUBLE_PRECISION`: the same calls in the same order give the same bits on Windows and
//! Linux and at any thread count, positions are `f64` and everything else `f32`. It is reached
//! through a narrow C layer of Forge's own (`cpp/forge_jolt.h`, after JoltC), which reads and
//! writes many bodies per call; this crate wraps that layer and holds all of the physics'
//! `unsafe`.
//!
//! A [`World`] owns its bodies and its worker threads; [`Shape`]s are shared between the bodies
//! made of them. [`World::save_state`] and [`World::restore_state`] take the whole simulation
//! back to an earlier step, and [`state_hash`] digests the bodies' transforms for the
//! determinism checks (`docs/research/physics-fluids.md` §6).

#![allow(unsafe_code)]

pub mod aero;
pub mod buoyancy;
mod ffi;
pub mod shallow;

use std::ptr::NonNull;

use glam::{DVec3, Quat, Vec3};

/// Why the physics refused something.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PhysicsError {
    /// Jolt refused the shape (degenerate, too few points); its reason went to stderr.
    #[error("Jolt refused the shape")]
    ShapeRefused,
    /// The world holds as many bodies as it was made for.
    #[error("the world is full")]
    WorldFull,
    /// The step ran out of room for body pairs, contacts or constraints (Jolt's flags).
    #[error("the step ran out of room (Jolt's flags {0:#x})")]
    Step(u32),
    /// The saved state does not fit this world.
    #[error("the saved state does not fit this world")]
    State,
}

/// A collision shape, shared by every body made of it.
#[derive(Debug)]
pub struct Shape {
    raw: NonNull<ffi::FjShape>,
}

// SAFETY: Jolt's shapes are immutable once created and counted by an atomic reference count, so
// they may be shared and dropped from any thread.
unsafe impl Send for Shape {}
// SAFETY: as above: nothing reachable through `&Shape` mutates.
unsafe impl Sync for Shape {}

impl Shape {
    fn wrap(raw: *mut ffi::FjShape) -> Result<Self, PhysicsError> {
        NonNull::new(raw)
            .map(|raw| Self { raw })
            .ok_or(PhysicsError::ShapeRefused)
    }

    /// A box of the given half sizes, metres, its edges rounded by `convex_radius` (at most the
    /// smallest half size), of `density` kg/m³.
    pub fn cuboid(
        half_extent: Vec3,
        convex_radius: f32,
        density: f32,
    ) -> Result<Self, PhysicsError> {
        init();
        let h = half_extent.to_array();
        // SAFETY: `h` is three floats, read during the call.
        Self::wrap(unsafe { ffi::fj_shape_box(h.as_ptr(), convex_radius, density) })
    }

    /// A ball of `radius` metres, of `density` kg/m³.
    pub fn sphere(radius: f32, density: f32) -> Result<Self, PhysicsError> {
        init();
        // SAFETY: plain values.
        Self::wrap(unsafe { ffi::fj_shape_sphere(radius, density) })
    }

    /// A capsule along +y: a cylinder `2 half_height` long capped by half balls of `radius`.
    pub fn capsule(half_height: f32, radius: f32, density: f32) -> Result<Self, PhysicsError> {
        init();
        // SAFETY: plain values.
        Self::wrap(unsafe { ffi::fj_shape_capsule(half_height, radius, density) })
    }

    /// A cylinder along +y, `2 half_height` long, centred on the origin, its rims rounded by
    /// `convex_radius`.
    pub fn cylinder(
        half_height: f32,
        radius: f32,
        convex_radius: f32,
        density: f32,
    ) -> Result<Self, PhysicsError> {
        init();
        // SAFETY: plain values.
        Self::wrap(unsafe { ffi::fj_shape_cylinder(half_height, radius, convex_radius, density) })
    }

    /// The convex hull of `points`, rounded by at most `max_convex_radius`.
    pub fn convex_hull(
        points: &[Vec3],
        max_convex_radius: f32,
        density: f32,
    ) -> Result<Self, PhysicsError> {
        init();
        let flat: Vec<f32> = points.iter().flat_map(|p| p.to_array()).collect();
        // SAFETY: `flat` holds three floats per point, read during the call.
        Self::wrap(unsafe {
            ffi::fj_shape_convex_hull(
                flat.as_ptr(),
                points.len() as u32,
                max_convex_radius,
                density,
            )
        })
    }

    /// A static triangle mesh (for still bodies only).
    pub fn mesh(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Result<Self, PhysicsError> {
        init();
        let flat: Vec<f32> = vertices.iter().flat_map(|p| p.to_array()).collect();
        // SAFETY: `flat` holds three floats per vertex and `triangles` three indices per
        // triangle (`[u32; 3]` is laid out as three `u32`), both read during the call.
        Self::wrap(unsafe {
            ffi::fj_shape_mesh(
                flat.as_ptr(),
                vertices.len() as u32,
                triangles.as_ptr().cast(),
                triangles.len() as u32,
            )
        })
    }

    /// A static height field (for still bodies only) of `count` × `count` `samples`, row-major
    /// along +z: the grid's point (x, z) stands at `offset + scale * (x, sample, z)` in its
    /// body's frame. Jolt asks for at least four samples a side and works best with a power of
    /// two.
    pub fn height_field(
        samples: &[f32],
        count: u32,
        offset: Vec3,
        scale: Vec3,
    ) -> Result<Self, PhysicsError> {
        init();
        assert_eq!(
            samples.len(),
            (count as usize) * (count as usize),
            "a height field's samples: count × count"
        );
        let (o, s) = (offset.to_array(), scale.to_array());
        // SAFETY: `samples` holds `count²` floats, `o` and `s` three each, all read during the
        // call (Jolt copies the samples into its own compressed blocks).
        Self::wrap(unsafe {
            ffi::fj_shape_height_field(samples.as_ptr(), count, o.as_ptr(), s.as_ptr())
        })
    }

    /// This shape moved by `position` and turned by `rotation` in its body's frame: a mesh whose
    /// origin is not its centre (a barrel's at its bottom) gets a body whose origin is the
    /// mesh's.
    pub fn offset(&self, position: Vec3, rotation: Quat) -> Result<Self, PhysicsError> {
        let (p, r) = (position.to_array(), rotation.to_array());
        // SAFETY: `self.raw` is a live shape, and `p` and `r` three and four floats, all read
        // during the call; the new shape takes its own reference to this one.
        Self::wrap(unsafe { ffi::fj_shape_offset(self.raw.as_ptr(), p.as_ptr(), r.as_ptr()) })
    }
}

impl Shape {
    /// Its centre of mass in its body's frame (a hull's, of its volume at even density).
    pub fn center_of_mass(&self) -> Vec3 {
        let mut c = [0.0_f32; 3];
        // SAFETY: `self.raw` is a live shape; three floats written during the call.
        unsafe { ffi::fj_shape_center_of_mass(self.raw.as_ptr(), c.as_mut_ptr()) };
        Vec3::from_array(c)
    }

    /// This shape with its centre of mass at `at` in its body's frame (an aeroplane's, ahead
    /// of its wing's lift and over its wheels).
    pub fn with_center_of_mass_at(&self, at: Vec3) -> Result<Self, PhysicsError> {
        self.with_center_of_mass_offset(at - self.center_of_mass())
    }

    /// This shape with its centre of mass moved by `offset` in its body's frame: a boat's
    /// weight sits low in its hull, which keeps it upright.
    pub fn with_center_of_mass_offset(&self, offset: Vec3) -> Result<Self, PhysicsError> {
        let o = offset.to_array();
        // SAFETY: `self.raw` is a live shape and `o` three floats, read during the call; the
        // new shape takes its own reference to this one.
        Self::wrap(unsafe { ffi::fj_shape_offset_center_of_mass(self.raw.as_ptr(), o.as_ptr()) })
    }
}

impl Drop for Shape {
    fn drop(&mut self) {
        // SAFETY: `self.raw` holds one reference, dropped here once.
        unsafe { ffi::fj_shape_release(self.raw.as_ptr()) };
    }
}

/// How a body moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Never: the ground, walls.
    Static,
    /// Moved by its velocity alone, pushing what it meets: platforms, doors.
    Kinematic,
    /// By the forces on it.
    Dynamic,
}

/// A body to add: its shape, where it starts and what it is like.
#[derive(Clone, Copy, Debug)]
pub struct BodyDesc<'a> {
    /// What it collides as.
    pub shape: &'a Shape,
    /// Its origin, metres.
    pub position: DVec3,
    /// Its rotation.
    pub rotation: Quat,
    /// How it moves.
    pub motion: Motion,
    /// Its starting velocity, m/s.
    pub linear_velocity: Vec3,
    /// Its starting spin, rad/s.
    pub angular_velocity: Vec3,
    /// Coulomb friction (combined with the other body's by their geometric mean).
    pub friction: f32,
    /// Bounciness, 0 to 1 (the larger of the two bodies').
    pub restitution: f32,
    /// Velocity lost per second, a share.
    pub linear_damping: f32,
    /// Spin lost per second, a share.
    pub angular_damping: f32,
    /// Kilograms, or `None` for its shape's volume times its density.
    pub mass: Option<f32>,
    /// A number of the caller's, carried with the body.
    pub user_data: u64,
    /// Swept collision, so a fast body cannot pass through a thin one.
    pub ccd: bool,
    /// Whether it may sleep when it comes to rest.
    pub allow_sleep: bool,
    /// Whether it starts asleep (until something wakes it: a contact, an impulse, a move).
    pub asleep: bool,
}

impl<'a> BodyDesc<'a> {
    /// A dynamic body of `shape` at `position`, at rest, with Jolt's defaults.
    pub fn dynamic(shape: &'a Shape, position: DVec3) -> Self {
        Self {
            shape,
            position,
            rotation: Quat::IDENTITY,
            motion: Motion::Dynamic,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            friction: 0.2,
            restitution: 0.0,
            linear_damping: 0.05,
            angular_damping: 0.05,
            mass: None,
            user_data: 0,
            ccd: false,
            allow_sleep: true,
            asleep: false,
        }
    }

    /// A body that never moves.
    pub fn fixed(shape: &'a Shape, position: DVec3) -> Self {
        Self {
            motion: Motion::Static,
            ..Self::dynamic(shape, position)
        }
    }
}

/// A body of a [`World`]: Jolt's index and sequence number.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyId(u32);

impl BodyId {
    /// Its raw value, for logs and replication.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Where a body is: its origin, metres, and its rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Its origin, metres.
    pub position: DVec3,
    /// Its rotation.
    pub rotation: Quat,
}

/// A body's motion: its velocity, m/s, and its spin, rad/s.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Velocity {
    /// m/s.
    pub linear: Vec3,
    /// rad/s.
    pub angular: Vec3,
}

/// A walking character to add (Jolt's `CharacterVirtual`, D-009): a capsule standing on its
/// feet.
#[derive(Clone, Copy, Debug)]
pub struct CharacterDesc {
    /// Its feet, metres.
    pub position: DVec3,
    /// The capsule's radius, metres.
    pub radius: f32,
    /// Feet to the top of its head, metres.
    pub height: f32,
    /// The steepest ground it walks up, radians.
    pub max_slope: f32,
    /// kg: how hard it presses what it stands on.
    pub mass: f32,
    /// N: how hard it pushes what it walks into.
    pub max_strength: f32,
    /// How high a step it walks up, metres.
    pub step_up: f32,
    /// How far down it keeps to the ground walking down a slope or a step, metres.
    pub stick_down: f32,
}

impl Default for CharacterDesc {
    /// A person: 1.8 m tall, 0.3 m round, 70 kg, up slopes of 45° and steps of 40 cm.
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            radius: 0.3,
            height: 1.8,
            // A quarter turn's half, as a constant: no trigonometry here.
            max_slope: std::f32::consts::FRAC_PI_4,
            mass: 70.0,
            max_strength: 400.0,
            step_up: 0.4,
            stick_down: 0.5,
        }
    }
}

/// A character of a [`World`]: its index, in the order added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharacterId(u32);

/// What a character stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    /// Ground it can walk on.
    Firm,
    /// Ground too steep to climb: it slides.
    Steep,
    /// Something it touches that does not hold it up.
    NotSupported,
    /// Nothing.
    InAir,
}

/// Where a character is and what it stands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterState {
    /// Its feet, metres.
    pub position: DVec3,
    /// m/s.
    pub velocity: Vec3,
    /// What it stands on.
    pub ground: Ground,
    /// The ground's normal there.
    pub ground_normal: Vec3,
    /// How fast the ground moves (a platform, a deck), m/s.
    pub ground_velocity: Vec3,
    /// The body it stands on, if any.
    pub ground_body: Option<BodyId>,
}

/// A four-wheeled car on a chassis body (Jolt's `VehicleConstraint`, D-009): front-wheel drive,
/// the front wheels steering, the rear ones braking with the handbrake, an anti-roll bar on
/// each axle. The chassis' frame: −z forward, +y up.
#[derive(Clone, Copy, Debug)]
pub struct VehicleDesc {
    /// Half the distance between the left and right wheels, metres.
    pub half_track: f32,
    /// Half the distance between the axles, metres.
    pub half_wheelbase: f32,
    /// Where the suspension hangs from, metres up in the chassis' frame.
    pub attach_y: f32,
    /// How far the suspension reaches down, at the least and the most, metres.
    pub suspension: (f32, f32),
    /// The springs' frequency (Hz) and damping (1 critical).
    pub spring: (f32, f32),
    /// The wheels' radius and width, metres.
    pub wheel: (f32, f32),
    /// The front wheels' steering at the most, radians.
    pub max_steer: f32,
    /// The engine's torque, N·m, and its top revs, rpm.
    pub engine: (f32, f32),
    /// The brakes' torque on each wheel and the handbrake's on the rear ones, N·m.
    pub brakes: (f32, f32),
}

/// A car of a [`World`]: its index, in the order added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VehicleId(u32);

/// A joint of a [`World`] (#142): its index, in the order added.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JointId(u32);

/// How a part of a ragdoll turns on its parent (#143).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RagdollJoint {
    /// A ball joint (a shoulder, a hip, a neck): the part swings within a cone of these half
    /// angles round its twist axis and twists about it within a range, radians.
    SwingTwist {
        /// The cone's half angles: across the normal axis and across the plane axis.
        cone: (f32, f32),
        /// The twist's least and most.
        twist: (f32, f32),
    },
    /// A hinge (a knee, an elbow) about the part's plane axis, within a range, radians.
    Hinge {
        /// Its least and most angle from the pose as built.
        range: (f32, f32),
    },
}

/// A part of a ragdoll as built: a body and the joint that holds it to its parent.
#[derive(Clone, Copy, Debug)]
pub struct RagdollPart<'a> {
    /// Its shape (with its density).
    pub shape: &'a Shape,
    /// Where it is.
    pub at: Transform,
    /// Its parent's index among the parts (parents come first), `None` for the root.
    pub parent: Option<usize>,
    /// How it turns on its parent (ignored for the root).
    pub joint: RagdollJoint,
    /// The joint's point.
    pub pivot: DVec3,
    /// Along the part from the pivot (unit), and across it at a right angle (unit): a hinge
    /// turns about the latter.
    pub twist_axis: Vec3,
    /// See `twist_axis`.
    pub plane_axis: Vec3,
    /// Its friction.
    pub friction: f32,
}

/// A ragdoll of a [`World`]: its index, in the order added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RagdollId(u32);

/// The motors that drive a ragdoll's joints to a pose: a spring of `stiffness` and `damping`
/// whatever its parts weigh (a spring given by its frequency would scale with each joint's own
/// light part and let a torso sag), at most `torque` N·m (0: limp).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motors {
    /// N·m a radian.
    pub stiffness: f32,
    /// N·m·s a radian.
    pub damping: f32,
    /// N·m at the most; 0 lets the joints go.
    pub torque: f32,
}

/// What a joint carried in the last step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JointLoad {
    /// The impulse of the part holding its points together, N·s.
    pub position: f32,
    /// The impulse of the part holding its bodies' turn, N·m·s (0 for a distance joint).
    pub rotation: f32,
}

/// The nearest hit of a ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The body hit.
    pub body: BodyId,
    /// How far along the ray, a share of its length.
    pub fraction: f32,
    /// The surface's normal there.
    pub normal: Vec3,
}

/// A world's capacities and threads.
#[derive(Clone, Copy, Debug)]
pub struct WorldDesc {
    /// The most bodies it holds.
    pub max_bodies: u32,
    /// The most pairs of bodies whose bounds overlap in a step.
    pub max_body_pairs: u32,
    /// The most contacts between bodies in a step.
    pub max_contact_constraints: u32,
    /// Worker threads besides the caller's, which also works during a step.
    pub threads: u32,
    /// m/s².
    pub gravity: Vec3,
}

impl Default for WorldDesc {
    fn default() -> Self {
        Self {
            max_bodies: 65_536,
            max_body_pairs: 65_536,
            max_contact_constraints: 32_768,
            threads: 3,
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

/// A physics world: its bodies, its broad phase and its worker threads.
pub struct World {
    raw: NonNull<ffi::FjWorld>,
    threads: u32,
}

// SAFETY: the world is used from one thread at a time (`&mut self` for every change); Jolt's
// own threads live inside it and are joined when it is dropped.
unsafe impl Send for World {}

/// Registers Jolt's types, once per process.
fn init() {
    // SAFETY: no arguments; the C layer runs it once (`std::call_once`).
    unsafe { ffi::fj_init() };
}

impl World {
    /// An empty world.
    pub fn new(desc: &WorldDesc) -> Self {
        init();
        let raw = ffi::FjWorldDesc {
            max_bodies: desc.max_bodies,
            max_body_pairs: desc.max_body_pairs,
            max_contact_constraints: desc.max_contact_constraints,
            threads: desc.threads,
            gravity: desc.gravity.to_array(),
        };
        // SAFETY: `raw` is read during the call; the world it returns is ours to free.
        let world = unsafe { ffi::fj_world_new(&raw) };
        Self {
            raw: NonNull::new(world).expect("Jolt's world"),
            threads: desc.threads,
        }
    }

    /// Worker threads besides the caller's.
    pub fn threads(&self) -> u32 {
        self.threads
    }

    /// Adds a body; it starts awake unless it is static or [`BodyDesc::asleep`].
    pub fn add_body(&mut self, desc: &BodyDesc) -> Result<BodyId, PhysicsError> {
        let raw = ffi::FjBodyDesc {
            shape: desc.shape.raw.as_ptr(),
            position: desc.position.to_array(),
            rotation: desc.rotation.to_array(),
            linear_velocity: desc.linear_velocity.to_array(),
            angular_velocity: desc.angular_velocity.to_array(),
            friction: desc.friction,
            restitution: desc.restitution,
            linear_damping: desc.linear_damping,
            angular_damping: desc.angular_damping,
            mass: desc.mass.unwrap_or(0.0),
            user_data: desc.user_data,
            motion: match desc.motion {
                Motion::Static => 0,
                Motion::Kinematic => 1,
                Motion::Dynamic => 2,
            },
            ccd: u8::from(desc.ccd),
            allow_sleep: u8::from(desc.allow_sleep),
            activate: u8::from(desc.motion != Motion::Static && !desc.asleep),
        };
        // SAFETY: the world is live and `raw` (and the shape it points at, which the body
        // takes a reference to) is read during the call.
        let id = unsafe { ffi::fj_body_add(self.raw.as_ptr(), &raw) };
        if id == u32::MAX {
            Err(PhysicsError::WorldFull)
        } else {
            Ok(BodyId(id))
        }
    }

    /// Removes a body for good.
    pub fn remove_body(&mut self, body: BodyId) {
        // SAFETY: the world is live; Jolt checks the id.
        unsafe { ffi::fj_body_remove(self.raw.as_ptr(), body.0) };
    }

    /// Rebuilds the broad phase's tree, after adding many bodies at once.
    pub fn optimize_broad_phase(&mut self) {
        // SAFETY: the world is live.
        unsafe { ffi::fj_world_optimize_broad_phase(self.raw.as_ptr()) };
    }

    /// Advances the world by `dt` seconds in `collision_steps` steps (one per 1/60 s or less).
    pub fn step(&mut self, dt: f32, collision_steps: u32) -> Result<(), PhysicsError> {
        // SAFETY: the world is live; the step runs on its own threads and returns when done.
        let flags =
            unsafe { ffi::fj_world_step(self.raw.as_ptr(), dt, collision_steps.max(1) as i32) };
        if flags == 0 {
            Ok(())
        } else {
            Err(PhysicsError::Step(flags))
        }
    }

    /// The bodies awake now.
    pub fn active_bodies(&self) -> u32 {
        // SAFETY: the world is live.
        unsafe { ffi::fj_world_active_bodies(self.raw.as_ptr()) }
    }

    /// The transforms of `bodies`, in their order, into `out` (cleared first).
    pub fn transforms(&self, bodies: &[BodyId], out: &mut Vec<Transform>) {
        let n = bodies.len();
        let mut positions = vec![[0.0_f64; 3]; n];
        let mut rotations = vec![[0.0_f32; 4]; n];
        // SAFETY: `BodyId` is a transparent `u32`; the buffers hold three doubles and four
        // floats per body, written during the call.
        unsafe {
            ffi::fj_bodies_transforms(
                self.raw.as_ptr(),
                bodies.as_ptr().cast(),
                n as u32,
                positions.as_mut_ptr().cast(),
                rotations.as_mut_ptr().cast(),
            );
        }
        out.clear();
        out.extend(positions.iter().zip(&rotations).map(|(&p, &r)| Transform {
            position: DVec3::from_array(p),
            rotation: Quat::from_array(r),
        }));
    }

    /// The velocities of `bodies`, in their order, into `out` (cleared first).
    pub fn velocities(&self, bodies: &[BodyId], out: &mut Vec<Velocity>) {
        let n = bodies.len();
        let mut linear = vec![[0.0_f32; 3]; n];
        let mut angular = vec![[0.0_f32; 3]; n];
        // SAFETY: as in `transforms`: three floats per body in each buffer.
        unsafe {
            ffi::fj_bodies_velocities(
                self.raw.as_ptr(),
                bodies.as_ptr().cast(),
                n as u32,
                linear.as_mut_ptr().cast(),
                angular.as_mut_ptr().cast(),
            );
        }
        out.clear();
        out.extend(linear.iter().zip(&angular).map(|(&v, &w)| Velocity {
            linear: Vec3::from_array(v),
            angular: Vec3::from_array(w),
        }));
    }

    /// Whether each of `bodies` is awake, in their order, into `out` (cleared first).
    pub fn awake(&self, bodies: &[BodyId], out: &mut Vec<bool>) {
        let mut active = vec![0_u8; bodies.len()];
        // SAFETY: a byte per body, written during the call.
        unsafe {
            ffi::fj_bodies_active(
                self.raw.as_ptr(),
                bodies.as_ptr().cast(),
                bodies.len() as u32,
                active.as_mut_ptr(),
            );
        }
        out.clear();
        out.extend(active.iter().map(|&a| a != 0));
    }

    /// A digest of `bodies`' transforms and velocities to the bit: two worlds whose digests
    /// agree hold those bodies in the same place and the same motion (`forge-sim`'s checks).
    pub fn digest(&self, bodies: &[BodyId]) -> u64 {
        let mut transforms = Vec::new();
        let mut velocities = Vec::new();
        self.transforms(bodies, &mut transforms);
        self.velocities(bodies, &mut velocities);
        let mut bytes = Vec::with_capacity(bodies.len() * 64);
        for t in &transforms {
            for p in t.position.to_array() {
                bytes.extend_from_slice(&p.to_bits().to_le_bytes());
            }
            for r in t.rotation.to_array() {
                bytes.extend_from_slice(&r.to_bits().to_le_bytes());
            }
        }
        for v in &velocities {
            for x in v.linear.to_array().into_iter().chain(v.angular.to_array()) {
                bytes.extend_from_slice(&x.to_bits().to_le_bytes());
            }
        }
        xxhash_rust::xxh3::xxh3_64(&bytes)
    }

    /// Pushes a body through its centre of mass, N·s, waking it.
    pub fn add_impulse(&mut self, body: BodyId, impulse: Vec3) {
        let i = impulse.to_array();
        // SAFETY: the world is live; `i` is three floats read during the call.
        unsafe { ffi::fj_body_add_impulse(self.raw.as_ptr(), body.0, i.as_ptr()) };
    }

    /// Pushes a body at a point of the world, N·s, waking it.
    pub fn add_impulse_at(&mut self, body: BodyId, impulse: Vec3, point: DVec3) {
        let (i, p) = (impulse.to_array(), point.to_array());
        // SAFETY: the world is live; `i` and `p` are read during the call.
        unsafe { ffi::fj_body_add_impulse_at(self.raw.as_ptr(), body.0, i.as_ptr(), p.as_ptr()) };
    }

    /// A force through a body's centre of mass for the next step, N.
    pub fn add_force(&mut self, body: BodyId, force: Vec3) {
        let f = force.to_array();
        // SAFETY: the world is live; `f` is read during the call.
        unsafe { ffi::fj_body_add_force(self.raw.as_ptr(), body.0, f.as_ptr()) };
    }

    /// For the next step, pushes each of `bodies` by its force through its point of the world
    /// and its torque, waking it (the water's [`buoyancy::Push`]es, one call for them all).
    pub fn push(&mut self, bodies: &[BodyId], pushes: &[(Vec3, DVec3, Vec3)]) {
        assert_eq!(bodies.len(), pushes.len(), "a push per body");
        let forces: Vec<[f32; 3]> = pushes.iter().map(|p| p.0.to_array()).collect();
        let points: Vec<[f64; 3]> = pushes.iter().map(|p| p.1.to_array()).collect();
        let torques: Vec<[f32; 3]> = pushes.iter().map(|p| p.2.to_array()).collect();
        // SAFETY: the world is live; `BodyId` is a transparent `u32`, and each buffer holds
        // three numbers per body, read during the call.
        unsafe {
            ffi::fj_bodies_push(
                self.raw.as_ptr(),
                bodies.as_ptr().cast(),
                bodies.len() as u32,
                forces.as_ptr().cast(),
                points.as_ptr().cast(),
                torques.as_ptr().cast(),
            );
        }
    }

    /// The centres of mass of `bodies`, in their order, into `out` (cleared first).
    pub fn centers_of_mass(&self, bodies: &[BodyId], out: &mut Vec<DVec3>) {
        let mut centers = vec![[0.0_f64; 3]; bodies.len()];
        // SAFETY: three doubles per body, written during the call.
        unsafe {
            ffi::fj_bodies_centers_of_mass(
                self.raw.as_ptr(),
                bodies.as_ptr().cast(),
                bodies.len() as u32,
                centers.as_mut_ptr().cast(),
            );
        }
        out.clear();
        out.extend(centers.iter().map(|&c| DVec3::from_array(c)));
    }

    /// Sets a body's velocity and spin.
    pub fn set_velocity(&mut self, body: BodyId, velocity: Velocity) {
        let (v, w) = (velocity.linear.to_array(), velocity.angular.to_array());
        // SAFETY: the world is live; `v` and `w` are read during the call.
        unsafe { ffi::fj_body_set_velocity(self.raw.as_ptr(), body.0, v.as_ptr(), w.as_ptr()) };
    }

    /// Moves a body to a transform at once, waking it.
    pub fn set_transform(&mut self, body: BodyId, transform: Transform) {
        let (p, r) = (transform.position.to_array(), transform.rotation.to_array());
        // SAFETY: the world is live; `p` and `r` are read during the call.
        unsafe { ffi::fj_body_set_transform(self.raw.as_ptr(), body.0, p.as_ptr(), r.as_ptr()) };
    }

    /// The nearest body along `direction` from `origin`, within the direction's length.
    pub fn cast_ray(&self, origin: DVec3, direction: Vec3) -> Option<RayHit> {
        let (o, d) = (origin.to_array(), direction.to_array());
        let mut hit = ffi::FjRayHit::default();
        // SAFETY: the world is live; `o` and `d` are read and `hit` written during the call.
        let found =
            unsafe { ffi::fj_world_cast_ray(self.raw.as_ptr(), o.as_ptr(), d.as_ptr(), &mut hit) };
        (found != 0).then(|| RayHit {
            body: BodyId(hit.body),
            fraction: hit.fraction,
            normal: Vec3::from_array(hit.normal),
        })
    }

    /// Adds a walking character; it is saved and restored with the world.
    pub fn add_character(&mut self, desc: &CharacterDesc) -> CharacterId {
        let raw = ffi::FjCharacterDesc {
            position: desc.position.to_array(),
            radius: desc.radius,
            height: desc.height,
            max_slope: desc.max_slope,
            mass: desc.mass,
            max_strength: desc.max_strength,
            step_up: desc.step_up,
            stick_down: desc.stick_down,
        };
        // SAFETY: the world is live and `raw` is read during the call; the character lives
        // as long as the world.
        CharacterId(unsafe { ffi::fj_character_add(self.raw.as_ptr(), &raw) })
    }

    /// Moves a character through a step of `dt` seconds at `velocity`: it slides along what it
    /// meets, walks up steps, keeps to the ground going down, and pushes what it walks into.
    /// The world's gravity presses it on what it stands on; its fall is the caller's, in the
    /// velocity.
    pub fn move_character(&mut self, character: CharacterId, dt: f32, velocity: Vec3) {
        let v = velocity.to_array();
        // SAFETY: the world is live, the index one it gave, `v` read during the call.
        unsafe { ffi::fj_character_move(self.raw.as_ptr(), character.0, dt, v.as_ptr()) };
    }

    /// Where a character is and what it stands on.
    pub fn character(&self, character: CharacterId) -> CharacterState {
        let mut s = ffi::FjCharacterState::default();
        // SAFETY: the world is live, the index one it gave, `s` written during the call.
        unsafe { ffi::fj_character_state(self.raw.as_ptr(), character.0, &mut s) };
        CharacterState {
            position: DVec3::from_array(s.position),
            velocity: Vec3::from_array(s.velocity),
            ground: match s.ground_state {
                0 => Ground::Firm,
                1 => Ground::Steep,
                2 => Ground::NotSupported,
                _ => Ground::InAir,
            },
            ground_normal: Vec3::from_array(s.ground_normal),
            ground_velocity: Vec3::from_array(s.ground_velocity),
            ground_body: (s.ground_body != u32::MAX).then_some(BodyId(s.ground_body)),
        }
    }

    /// Makes `chassis` (a dynamic body) a car; it is saved and restored with the world.
    pub fn add_vehicle(
        &mut self,
        chassis: BodyId,
        desc: &VehicleDesc,
    ) -> Result<VehicleId, PhysicsError> {
        let raw = ffi::FjVehicleDesc {
            half_track: desc.half_track,
            half_wheelbase: desc.half_wheelbase,
            attach_y: desc.attach_y,
            suspension_min: desc.suspension.0,
            suspension_max: desc.suspension.1,
            spring_frequency: desc.spring.0,
            spring_damping: desc.spring.1,
            wheel_radius: desc.wheel.0,
            wheel_width: desc.wheel.1,
            max_steer: desc.max_steer,
            engine_torque: desc.engine.0,
            max_rpm: desc.engine.1,
            brake_torque: desc.brakes.0,
            handbrake_torque: desc.brakes.1,
        };
        // SAFETY: the world is live and `raw` read during the call; the constraint lives as
        // long as the world.
        let id = unsafe { ffi::fj_vehicle_add(self.raw.as_ptr(), chassis.0, &raw) };
        if id == u32::MAX {
            Err(PhysicsError::WorldFull)
        } else {
            Ok(VehicleId(id))
        }
    }

    /// The driver: throttle (−1 astern to 1 ahead), steering (−1 left to 1 right), brake and
    /// handbrake (0 to 1), held until the next call; any of them wakes a parked car.
    pub fn drive(
        &mut self,
        vehicle: VehicleId,
        throttle: f32,
        steer: f32,
        brake: f32,
        handbrake: f32,
    ) {
        // SAFETY: the world is live and the index one it gave.
        unsafe {
            ffi::fj_vehicle_drive(
                self.raw.as_ptr(),
                vehicle.0,
                throttle,
                steer,
                brake,
                handbrake,
            );
        }
    }

    /// A car's four wheels in the world (front left, front right, rear left, rear right), their
    /// axles along their x, into `out` (cleared first).
    pub fn wheels(&self, vehicle: VehicleId, out: &mut Vec<Transform>) {
        let mut positions = [[0.0_f64; 3]; 4];
        let mut rotations = [[0.0_f32; 4]; 4];
        // SAFETY: the world is live, the index one it gave; four transforms written.
        unsafe {
            ffi::fj_vehicle_wheels(
                self.raw.as_ptr(),
                vehicle.0,
                positions.as_mut_ptr().cast(),
                rotations.as_mut_ptr().cast(),
            );
        }
        out.clear();
        out.extend(positions.iter().zip(&rotations).map(|(&p, &r)| Transform {
            position: DVec3::from_array(p),
            rotation: Quat::from_array(r),
        }));
    }

    /// A car's engine: its revs (rpm) and the gear engaged (0 neutral, −1 reverse).
    pub fn engine(&self, vehicle: VehicleId) -> (f32, i32) {
        let (mut rpm, mut gear) = (0.0, 0);
        // SAFETY: the world is live, the index one it gave; two numbers written.
        unsafe { ffi::fj_vehicle_engine(self.raw.as_ptr(), vehicle.0, &mut rpm, &mut gear) };
        (rpm, gear)
    }

    /// Holds `b` to `a` (or to the world, `None`) as they are now, like mortar or a weld (Jolt's
    /// fixed constraint about the point between them). It is saved and restored with the world,
    /// broken or not. `steps`: the solver's velocity and position iterations at the least over
    /// the bodies it holds, `(0, 0)` for the world's (10 and 2); a wall of many courses needs
    /// more to stand rigid.
    pub fn join_fixed(&mut self, a: Option<BodyId>, b: BodyId, steps: (u32, u32)) -> JointId {
        // SAFETY: the world is live and the bodies its own (or the world's marker).
        JointId(unsafe {
            ffi::fj_joint_fixed(
                self.raw.as_ptr(),
                a.map_or(u32::MAX, |a| a.0),
                b.0,
                steps.0,
                steps.1,
            )
        })
    }

    /// Keeps `point_b` of `b` between `range.0` and `range.1` metres from `point_a` of `a` (or of
    /// the world, `None`), both points given in the world as they are now: a chain or a rod.
    pub fn join_distance(
        &mut self,
        a: Option<BodyId>,
        b: BodyId,
        point_a: DVec3,
        point_b: DVec3,
        range: (f32, f32),
    ) -> JointId {
        let (pa, pb) = (point_a.to_array(), point_b.to_array());
        // SAFETY: the world is live, the bodies its own; the points read during the call.
        JointId(unsafe {
            ffi::fj_joint_distance(
                self.raw.as_ptr(),
                a.map_or(u32::MAX, |a| a.0),
                b.0,
                pa.as_ptr(),
                pb.as_ptr(),
                range.0,
                range.1,
            )
        })
    }

    /// What `joints` carried in the last step, into `out` (cleared first). A load over the
    /// step's length is the force: what decides whether mortar breaks.
    pub fn joint_loads(&self, joints: &[JointId], out: &mut Vec<JointLoad>) {
        let mut loads = vec![[0.0_f32; 2]; joints.len()];
        // SAFETY: the world is live, the ids its own (`JointId` is a `u32`); two floats a joint.
        unsafe {
            ffi::fj_joints_load(
                self.raw.as_ptr(),
                joints.as_ptr().cast(),
                joints.len() as u32,
                loads.as_mut_ptr().cast(),
            );
        }
        out.clear();
        out.extend(
            loads
                .iter()
                .map(|&[position, rotation]| JointLoad { position, rotation }),
        );
    }

    /// Breaks `joints` (`holding` false) or mends them (true), waking their bodies.
    pub fn set_holding(&mut self, joints: &[JointId], holding: bool) {
        let flags = vec![u8::from(holding); joints.len()];
        // SAFETY: the world is live, the ids its own; a flag a joint.
        unsafe {
            ffi::fj_joints_set(
                self.raw.as_ptr(),
                joints.as_ptr().cast(),
                joints.len() as u32,
                flags.as_ptr(),
            );
        }
    }

    /// Adds a ragdoll (#143): Jolt's ragdoll of `parts`, each a body held to its parent by its
    /// joint, a part not colliding with its parent. Its bodies, in the parts' order, come back
    /// with it. Saved and restored with the world.
    pub fn add_ragdoll(
        &mut self,
        parts: &[RagdollPart],
    ) -> Result<(RagdollId, Vec<BodyId>), PhysicsError> {
        let raw: Vec<ffi::FjRagdollPart> = parts
            .iter()
            .map(|p| {
                let (kind, normal_cone, plane_cone, twist) = match p.joint {
                    RagdollJoint::SwingTwist { cone, twist } => (0, cone.0, cone.1, twist),
                    RagdollJoint::Hinge { range } => (1, 0.0, 0.0, range),
                };
                ffi::FjRagdollPart {
                    shape: p.shape.raw.as_ptr(),
                    position: p.at.position.to_array(),
                    rotation: p.at.rotation.to_array(),
                    parent: p.parent.map_or(-1, |k| k as i32),
                    kind,
                    pivot: p.pivot.to_array(),
                    twist_axis: p.twist_axis.to_array(),
                    plane_axis: p.plane_axis.to_array(),
                    normal_cone,
                    plane_cone,
                    twist_min: twist.0,
                    twist_max: twist.1,
                    friction: p.friction,
                }
            })
            .collect();
        let mut bodies = vec![0_u32; parts.len()];
        // SAFETY: the world is live, the parts' shapes outlive the call (Jolt keeps its own
        // references), and one body id is written a part.
        let id = unsafe {
            ffi::fj_ragdoll_add(
                self.raw.as_ptr(),
                raw.as_ptr(),
                raw.len() as u32,
                bodies.as_mut_ptr(),
            )
        };
        if id == u32::MAX {
            return Err(PhysicsError::WorldFull);
        }
        Ok((RagdollId(id), bodies.into_iter().map(BodyId).collect()))
    }

    /// Drives a ragdoll's joints towards `targets`, one a part (the root's ignored): for a
    /// ball joint the part's turn in its joint's frame (x along the twist axis, y along the
    /// plane axis; identity is the pose as built), for a hinge its angle in `x`. A powered ragdoll
    /// is kept awake, so its targets move it however still it was; a limp one may sleep.
    pub fn drive_ragdoll(&mut self, ragdoll: RagdollId, targets: &[[f32; 4]], motors: Motors) {
        // SAFETY: the world is live, the index one it gave, four floats a part read.
        unsafe {
            ffi::fj_ragdoll_drive(
                self.raw.as_ptr(),
                ragdoll.0,
                targets.as_ptr().cast(),
                motors.stiffness,
                motors.damping,
                motors.torque,
            );
        }
    }

    /// Whether `joints` hold, into `out` (cleared first).
    pub fn holding(&self, joints: &[JointId], out: &mut Vec<bool>) {
        let mut flags = vec![0_u8; joints.len()];
        // SAFETY: the world is live, the ids its own; a flag a joint.
        unsafe {
            ffi::fj_joints_holding(
                self.raw.as_ptr(),
                joints.as_ptr().cast(),
                joints.len() as u32,
                flags.as_mut_ptr(),
            );
        }
        out.clear();
        out.extend(flags.iter().map(|&f| f != 0));
    }

    /// The whole simulation's state (bodies, contacts, constraints): what
    /// [`World::restore_state`] takes it back to.
    pub fn save_state(&mut self) -> Vec<u8> {
        let mut size = 0;
        // SAFETY: the world is live; the buffer it returns stays valid until its next save,
        // and is copied out before this borrow ends.
        unsafe {
            let data = ffi::fj_world_save_state(self.raw.as_ptr(), &mut size);
            std::slice::from_raw_parts(data, size).to_vec()
        }
    }

    /// Takes the world back to a state [`World::save_state`] saved, with the same bodies.
    pub fn restore_state(&mut self, state: &[u8]) -> Result<(), PhysicsError> {
        // SAFETY: the world is live; `state` is read during the call.
        let ok =
            unsafe { ffi::fj_world_restore_state(self.raw.as_ptr(), state.as_ptr(), state.len()) };
        if ok != 0 {
            Ok(())
        } else {
            Err(PhysicsError::State)
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // SAFETY: the world was made by `fj_world_new` and is freed here once; its bodies keep
        // their shapes' references, which it drops.
        unsafe { ffi::fj_world_free(self.raw.as_ptr()) };
    }
}

/// A digest of transforms to the bit: the determinism checks' currency (the same calls in the
/// same order must give the same digest on every platform and thread count).
pub fn state_hash(transforms: &[Transform]) -> u64 {
    let mut bytes = Vec::with_capacity(transforms.len() * 40);
    for t in transforms {
        for p in t.position.to_array() {
            bytes.extend_from_slice(&p.to_bits().to_le_bytes());
        }
        for r in t.rotation.to_array() {
            bytes.extend_from_slice(&r.to_bits().to_le_bytes());
        }
    }
    xxhash_rust::xxh3::xxh3_64(&bytes)
}

#[cfg(test)]
mod tests;
