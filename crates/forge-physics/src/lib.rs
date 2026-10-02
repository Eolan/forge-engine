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

mod ffi;

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
