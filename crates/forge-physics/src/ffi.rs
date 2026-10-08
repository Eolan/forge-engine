//! The C layer of `cpp/forge_jolt.h`, bound by hand: every struct mirrors its C twin field for
//! field, and the `layout` test checks the sizes against the compiler's.

#![allow(missing_docs)]

#[repr(C)]
pub struct FjWorld {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct FjShape {
    _opaque: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjWorldDesc {
    pub max_bodies: u32,
    pub max_body_pairs: u32,
    pub max_contact_constraints: u32,
    pub threads: u32,
    pub gravity: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjBodyDesc {
    pub shape: *const FjShape,
    pub position: [f64; 3],
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub mass: f32,
    pub user_data: u64,
    pub motion: u8,
    pub ccd: u8,
    pub allow_sleep: u8,
    pub activate: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FjRayHit {
    pub body: u32,
    pub fraction: f32,
    pub normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjCharacterDesc {
    pub position: [f64; 3],
    pub radius: f32,
    pub height: f32,
    pub max_slope: f32,
    pub mass: f32,
    pub max_strength: f32,
    pub step_up: f32,
    pub stick_down: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FjCharacterState {
    pub position: [f64; 3],
    pub velocity: [f32; 3],
    pub ground_normal: [f32; 3],
    pub ground_velocity: [f32; 3],
    pub ground_body: u32,
    pub ground_state: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjVehicleDesc {
    pub half_track: f32,
    pub half_wheelbase: f32,
    pub attach_y: f32,
    pub suspension_min: f32,
    pub suspension_max: f32,
    pub spring_frequency: f32,
    pub spring_damping: f32,
    pub wheel_radius: f32,
    pub wheel_width: f32,
    pub max_steer: f32,
    pub engine_torque: f32,
    pub max_rpm: f32,
    pub brake_torque: f32,
    pub handbrake_torque: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjRagdollPart {
    pub shape: *const FjShape,
    pub position: [f64; 3],
    pub rotation: [f32; 4],
    pub parent: i32,
    pub kind: u32,
    pub pivot: [f64; 3],
    pub twist_axis: [f32; 3],
    pub plane_axis: [f32; 3],
    pub normal_cone: f32,
    pub plane_cone: f32,
    pub twist_min: f32,
    pub twist_max: f32,
    pub friction: f32,
}

#[cfg(test)]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FjLayout {
    pub world_desc: u32,
    pub body_desc: u32,
    pub ray_hit: u32,
    pub character_desc: u32,
    pub character_state: u32,
    pub vehicle_desc: u32,
    pub ragdoll_part: u32,
    pub soft_body_desc: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FjSoftBodyDesc {
    pub points: *const f32,
    pub faces: *const u32,
    pub position: [f64; 3],
    pub vertex_count: u32,
    pub face_count: u32,
    pub inverse_mass: f32,
    pub compliance: f32,
    pub bend_compliance: f32,
    pub pressure: f32,
    pub friction: f32,
    pub restitution: f32,
    pub iterations: u32,
    pub pad: u32,
    pub user_data: u64,
}

unsafe extern "C" {
    #[cfg(test)]
    pub fn fj_layout() -> FjLayout;
    pub fn fj_init();

    pub fn fj_shape_box(half_extent: *const f32, convex_radius: f32, density: f32) -> *mut FjShape;
    pub fn fj_shape_sphere(radius: f32, density: f32) -> *mut FjShape;
    pub fn fj_shape_capsule(half_height: f32, radius: f32, density: f32) -> *mut FjShape;
    pub fn fj_shape_cylinder(
        half_height: f32,
        radius: f32,
        convex_radius: f32,
        density: f32,
    ) -> *mut FjShape;
    pub fn fj_shape_convex_hull(
        points: *const f32,
        count: u32,
        max_convex_radius: f32,
        density: f32,
    ) -> *mut FjShape;
    pub fn fj_shape_mesh(
        vertices: *const f32,
        vertex_count: u32,
        indices: *const u32,
        triangle_count: u32,
    ) -> *mut FjShape;
    pub fn fj_soft_body_add(world: *mut FjWorld, desc: *const FjSoftBodyDesc) -> u32;
    pub fn fj_soft_body_push(world: *mut FjWorld, body: u32, velocity: *const f32);
    pub fn fj_soft_body_vertices(
        world: *const FjWorld,
        body: u32,
        points: *mut f32,
        capacity: u32,
        origin: *mut f64,
    ) -> u32;
    pub fn fj_shape_height_field(
        samples: *const f32,
        count: u32,
        offset: *const f32,
        scale: *const f32,
    ) -> *mut FjShape;
    pub fn fj_shape_offset(
        inner: *const FjShape,
        position: *const f32,
        rotation: *const f32,
    ) -> *mut FjShape;
    pub fn fj_shape_offset_center_of_mass(
        inner: *const FjShape,
        offset: *const f32,
    ) -> *mut FjShape;
    pub fn fj_shape_center_of_mass(shape: *const FjShape, out: *mut f32);
    pub fn fj_shape_release(shape: *const FjShape);

    pub fn fj_world_new(desc: *const FjWorldDesc) -> *mut FjWorld;
    pub fn fj_world_free(world: *mut FjWorld);
    pub fn fj_world_optimize_broad_phase(world: *mut FjWorld);
    pub fn fj_world_step(world: *mut FjWorld, dt: f32, collision_steps: i32) -> u32;
    pub fn fj_world_active_bodies(world: *const FjWorld) -> u32;

    pub fn fj_body_add(world: *mut FjWorld, desc: *const FjBodyDesc) -> u32;
    pub fn fj_body_remove(world: *mut FjWorld, body: u32);

    pub fn fj_bodies_transforms(
        world: *const FjWorld,
        bodies: *const u32,
        count: u32,
        positions: *mut f64,
        rotations: *mut f32,
    );
    pub fn fj_bodies_velocities(
        world: *const FjWorld,
        bodies: *const u32,
        count: u32,
        linear: *mut f32,
        angular: *mut f32,
    );
    pub fn fj_bodies_active(world: *const FjWorld, bodies: *const u32, count: u32, active: *mut u8);

    pub fn fj_body_add_impulse(world: *mut FjWorld, body: u32, impulse: *const f32);
    pub fn fj_body_add_impulse_at(
        world: *mut FjWorld,
        body: u32,
        impulse: *const f32,
        point: *const f64,
    );
    pub fn fj_body_add_force(world: *mut FjWorld, body: u32, force: *const f32);
    pub fn fj_bodies_push(
        world: *mut FjWorld,
        bodies: *const u32,
        count: u32,
        forces: *const f32,
        points: *const f64,
        torques: *const f32,
    );
    pub fn fj_bodies_centers_of_mass(
        world: *const FjWorld,
        bodies: *const u32,
        count: u32,
        centers: *mut f64,
    );
    pub fn fj_body_set_velocity(
        world: *mut FjWorld,
        body: u32,
        linear: *const f32,
        angular: *const f32,
    );
    pub fn fj_body_set_transform(
        world: *mut FjWorld,
        body: u32,
        position: *const f64,
        rotation: *const f32,
    );

    pub fn fj_world_cast_ray(
        world: *const FjWorld,
        origin: *const f64,
        direction: *const f32,
        hit: *mut FjRayHit,
    ) -> i32;

    pub fn fj_character_add(world: *mut FjWorld, desc: *const FjCharacterDesc) -> u32;
    pub fn fj_character_move(world: *mut FjWorld, character: u32, dt: f32, velocity: *const f32);
    pub fn fj_character_state(world: *const FjWorld, character: u32, state: *mut FjCharacterState);

    pub fn fj_vehicle_add(world: *mut FjWorld, chassis: u32, desc: *const FjVehicleDesc) -> u32;
    pub fn fj_vehicle_drive(
        world: *mut FjWorld,
        vehicle: u32,
        forward: f32,
        right: f32,
        brake: f32,
        handbrake: f32,
    );
    pub fn fj_vehicle_wheels(
        world: *const FjWorld,
        vehicle: u32,
        positions: *mut f64,
        rotations: *mut f32,
    );
    pub fn fj_vehicle_engine(world: *const FjWorld, vehicle: u32, rpm: *mut f32, gear: *mut i32);

    pub fn fj_joint_fixed(
        world: *mut FjWorld,
        a: u32,
        b: u32,
        velocity_steps: u32,
        position_steps: u32,
    ) -> u32;
    pub fn fj_joint_distance(
        world: *mut FjWorld,
        a: u32,
        b: u32,
        point_a: *const f64,
        point_b: *const f64,
        min: f32,
        max: f32,
    ) -> u32;
    pub fn fj_joints_load(world: *const FjWorld, joints: *const u32, count: u32, loads: *mut f32);
    pub fn fj_joints_set(world: *mut FjWorld, joints: *const u32, count: u32, holding: *const u8);
    pub fn fj_ragdoll_add(
        world: *mut FjWorld,
        parts: *const FjRagdollPart,
        count: u32,
        bodies: *mut u32,
    ) -> u32;
    pub fn fj_ragdoll_drive(
        world: *mut FjWorld,
        ragdoll: u32,
        targets: *const f32,
        stiffness: f32,
        damping: f32,
        torque: f32,
    );
    pub fn fj_joints_holding(
        world: *const FjWorld,
        joints: *const u32,
        count: u32,
        holding: *mut u8,
    );

    pub fn fj_world_save_state(world: *mut FjWorld, size: *mut usize) -> *const u8;
    pub fn fj_world_restore_state(world: *mut FjWorld, data: *const u8, size: usize) -> i32;
}
