// Forge's C layer over Jolt Physics (D-009, issue #136): narrow, batched, and bound by hand in
// `src/ffi.rs`. Written after JoltC (SecondHalfGames, MIT/Apache-2.0), whose opaque handles and
// layer set-up it follows; unlike JoltC it exposes only what Forge calls, and reads and writes
// many bodies per call.
//
// Every struct here has a twin in `src/ffi.rs`; `fj_layout` reports their sizes so a test can
// check the two agree.

#ifndef FORGE_JOLT_H
#define FORGE_JOLT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FjWorld FjWorld;
typedef struct FjShape FjShape;

// A world's capacities and threads.
typedef struct FjWorldDesc {
    uint32_t max_bodies;
    uint32_t max_body_pairs;
    uint32_t max_contact_constraints;
    // Worker threads besides the caller's (0: the caller alone).
    uint32_t threads;
    float gravity[3];
} FjWorldDesc;

// What a body is made of and where it starts.
typedef struct FjBodyDesc {
    const FjShape *shape;
    double position[3];
    // x, y, z, w.
    float rotation[4];
    float linear_velocity[3];
    float angular_velocity[3];
    float friction;
    float restitution;
    float linear_damping;
    float angular_damping;
    // Kilograms; 0 takes the mass from the shape's density.
    float mass;
    uint64_t user_data;
    // 0 static, 1 kinematic, 2 dynamic.
    uint8_t motion;
    // 1: swept motion (continuous collision) for fast bodies.
    uint8_t ccd;
    uint8_t allow_sleep;
    uint8_t activate;
} FjBodyDesc;

// The nearest hit of a ray.
typedef struct FjRayHit {
    uint32_t body;
    // Along the ray's direction, 0 to 1.
    float fraction;
    float normal[3];
} FjRayHit;

// A walking character (Jolt's CharacterVirtual): a capsule standing on its feet, swept through
// the world by its velocity, stepping up stairs and down slopes, pushing what it walks into.
typedef struct FjCharacterDesc {
    // Its feet, metres.
    double position[3];
    float radius;
    // Feet to the top of its head, metres.
    float height;
    // The steepest ground it walks up, radians.
    float max_slope;
    // kg: how hard it presses what it stands on.
    float mass;
    // N: how hard it pushes what it walks into.
    float max_strength;
    // How high a step it walks up, and how far down it keeps to the ground, metres.
    float step_up;
    float stick_down;
} FjCharacterDesc;

// Where a character is and what it stands on.
typedef struct FjCharacterState {
    double position[3];
    float velocity[3];
    float ground_normal[3];
    float ground_velocity[3];
    uint32_t ground_body;
    // 0 on the ground, 1 on ground too steep, 2 touching but not held, 3 in the air.
    uint32_t ground_state;
} FjCharacterState;

// A four-wheeled car on a chassis body (Jolt's VehicleConstraint with its wheeled controller):
// the wheels hang from the chassis on springs, their contact found by casting a cylinder down.
// The chassis' frame: −z forward, +y up; wheels front left, front right, rear left, rear right.
typedef struct FjVehicleDesc {
    // Half the distance between the left and right wheels, and between the axles, metres.
    float half_track;
    float half_wheelbase;
    // Where the suspension hangs from in the chassis' frame, metres up, and how far it reaches
    // down at the most and the least.
    float attach_y;
    float suspension_min;
    float suspension_max;
    // The springs: their frequency (Hz) and damping (1 critical).
    float spring_frequency;
    float spring_damping;
    float wheel_radius;
    float wheel_width;
    // The front wheels' steering at the most, radians.
    float max_steer;
    // The engine's torque (N·m) and top revs (rpm), the brakes' and the handbrake's torques.
    float engine_torque;
    float max_rpm;
    float brake_torque;
    float handbrake_torque;
} FjVehicleDesc;

// A soft body to add: a closed surface of `vertex_count` points (three floats each, about its
// middle) and `face_count` triangles (three indices each), every vertex of `inverse_mass`; its
// edges' and shear edges' `compliance` and its bends' (FLT_MAX for none) made from the faces, the
// `pressure` inside it (n R T), `iterations` of the solver a step, at `position`.
typedef struct FjSoftBodyDesc {
    const float *points;
    const uint32_t *faces;
    double position[3];
    uint32_t vertex_count;
    uint32_t face_count;
    float inverse_mass;
    float compliance;
    float bend_compliance;
    float pressure;
    float friction;
    float restitution;
    uint32_t iterations;
    float gravity_factor; // what share of the world's gravity pulls it
    uint64_t user_data;
} FjSoftBodyDesc;

// The sizes of the structs above, for the layout test.
typedef struct FjLayout {
    uint32_t world_desc;
    uint32_t body_desc;
    uint32_t ray_hit;
    uint32_t character_desc;
    uint32_t character_state;
    uint32_t vehicle_desc;
    uint32_t ragdoll_part;
    uint32_t soft_body_desc;
} FjLayout;

FjLayout fj_layout(void);

// Registers Jolt's types once per process; every other call needs it done.
void fj_init(void);

// Shapes, reference counted: each constructor returns one reference, `fj_shape_release` drops
// it, and a body keeps its own. NULL when Jolt refuses the shape.
FjShape *fj_shape_box(const float half_extent[3], float convex_radius, float density);
FjShape *fj_shape_sphere(float radius, float density);
FjShape *fj_shape_capsule(float half_height, float radius, float density);
FjShape *fj_shape_cylinder(float half_height, float radius, float convex_radius, float density);
FjShape *fj_shape_convex_hull(const float *points, uint32_t count, float max_convex_radius,
                              float density);
// A static triangle mesh: `count` triangles of three indices each.
FjShape *fj_shape_mesh(const float *vertices, uint32_t vertex_count, const uint32_t *indices,
                       uint32_t triangle_count);
// A static height field of `count` × `count` samples, row-major along +z: the point (x, z) of
// the grid stands at `offset` + `scale` × (x, samples[z * count + x], z).
FjShape *fj_shape_height_field(const float *samples, uint32_t count, const float offset[3],
                               const float scale[3]);
// `inner` moved by `position` and turned by `rotation` (x, y, z, w) in its body's frame.
FjShape *fj_shape_offset(const FjShape *inner, const float position[3], const float rotation[4]);
// A shape's centre of mass in its frame (three floats).
void fj_shape_center_of_mass(const FjShape *shape, float out[3]);
// Its mass, kg, from its volume and density.
float fj_shape_mass(const FjShape *shape);
// `inner` with its centre of mass moved by `offset` (a boat's weight low in its hull).
FjShape *fj_shape_offset_center_of_mass(const FjShape *inner, const float offset[3]);
void fj_shape_release(const FjShape *shape);

FjWorld *fj_world_new(const FjWorldDesc *desc);
void fj_world_free(FjWorld *world);
// Rebuilds the broad phase's tree: after adding many bodies at once.
void fj_world_optimize_broad_phase(FjWorld *world);
// Advances by `dt` seconds in `collision_steps` steps; Jolt's error flags (0: none).
uint32_t fj_world_step(FjWorld *world, float dt, int32_t collision_steps);
uint32_t fj_world_active_bodies(const FjWorld *world);

// The body's id; 0xFFFFFFFF when the world is full.
uint32_t fj_body_add(FjWorld *world, const FjBodyDesc *desc);
// A soft body (FjSoftBodyDesc): its index and sequence number as a body's, UINT32_MAX when the
// world is full.
uint32_t fj_soft_body_add(FjWorld *world, const FjSoftBodyDesc *desc);
// Adds `velocity` (three floats) to every free vertex of a soft body and wakes it.
void fj_soft_body_push(FjWorld *world, uint32_t body, const float velocity[3]);
// Keeps a soft body upright as a whole: its spin (as a rigid body's) damped by `damping`, and an
// angular velocity of `spring` times the sine of its tilt (its point `top`'s way from its middle
// against +y) turning it back, both given to its points' velocities.
void fj_soft_body_upright(FjWorld *world, uint32_t body, uint32_t top, float spring, float damping);
// A soft body's vertices after the last step: three floats each into `points` (at most `capacity`),
// about `origin` (three doubles), its middle in the world. Returns their number (0 for a body that
// is no soft body).
uint32_t fj_soft_body_vertices(const FjWorld *world, uint32_t body, float *points, uint32_t capacity,
                               double origin[3]);
void fj_body_remove(FjWorld *world, uint32_t body);

// Positions (three doubles each) and rotations (four floats each) of `count` bodies.
void fj_bodies_transforms(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                          double *positions, float *rotations);
// Linear and angular velocities (three floats each) of `count` bodies.
void fj_bodies_velocities(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                          float *linear, float *angular);
// Whether each of `count` bodies is awake (1) or asleep (0).
void fj_bodies_active(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                      uint8_t *active);

void fj_body_add_impulse(FjWorld *world, uint32_t body, const float impulse[3]);
void fj_body_add_impulse_at(FjWorld *world, uint32_t body, const float impulse[3],
                            const double point[3]);
void fj_body_add_force(FjWorld *world, uint32_t body, const float force[3]);
// For the next step, per body: a force (three floats) through a point of the world (three
// doubles) and a torque (three floats), waking the body.
void fj_bodies_push(FjWorld *world, const uint32_t *bodies, uint32_t count, const float *forces,
                    const double *points, const float *torques);
// The centres of mass (three doubles each) of `count` bodies.
void fj_bodies_centers_of_mass(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                               double *centers);
void fj_body_set_velocity(FjWorld *world, uint32_t body, const float linear[3],
                          const float angular[3]);
void fj_body_set_transform(FjWorld *world, uint32_t body, const double position[3],
                           const float rotation[4]);

// The nearest hit along `direction` (its length is the ray's) from `origin`, among the still
// bodies alone when `still_only`; 0 when none.
int32_t fj_world_cast_ray(const FjWorld *world, const double origin[3], const float direction[3],
                          int32_t still_only, FjRayHit *hit);

// Adds a character; its index. The world's characters are numbered from 1 in the order added
// (Jolt's own numbering runs across every world of the process), and saved and restored with it.
uint32_t fj_character_add(FjWorld *world, const FjCharacterDesc *desc);
// Moves it through a step of `dt` at `velocity` (the world's gravity presses it down).
void fj_character_move(FjWorld *world, uint32_t character, float dt, const float velocity[3]);
void fj_character_state(const FjWorld *world, uint32_t character, FjCharacterState *state);

// Makes `chassis` a car; its index. Saved and restored with the world (a constraint of it).
uint32_t fj_vehicle_add(FjWorld *world, uint32_t chassis, const FjVehicleDesc *desc);
// The driver: throttle (−1 astern to 1), steering (−1 left to 1 right), brake and handbrake
// (0 to 1), held until the next call.
void fj_vehicle_drive(FjWorld *world, uint32_t vehicle, float forward, float right, float brake,
                      float handbrake);
// The four wheels' transforms in the world (three doubles and four floats each), their axles
// along their x.
void fj_vehicle_wheels(const FjWorld *world, uint32_t vehicle, double *positions,
                       float *rotations);
// The engine's revs (rpm) and the gear engaged (0 neutral, −1 reverse).
void fj_vehicle_engine(const FjWorld *world, uint32_t vehicle, float *rpm, int32_t *gear);

// Joints (#142): what holds two bodies together, FJ_WORLD in place of a body for the world.
// Each is saved and restored with the world: whether it holds, and its impulses.
#define FJ_WORLD 0xffffffffu
// Holds `a` and `b` as they are now (Jolt's FixedConstraint about the point between them); its
// index. The solver takes at least `velocity_steps` and `position_steps` (0: the world's 10
// and 2) over the bodies it holds: a tall stack of joints needs more to stay rigid.
uint32_t fj_joint_fixed(FjWorld *world, uint32_t a, uint32_t b, uint32_t velocity_steps,
                        uint32_t position_steps);
// Keeps `point_b` of `b` between `min` and `max` from `point_a` of `a`, both given in the world
// as they are now (Jolt's DistanceConstraint): a chain, a rope or a rod. Its index.
uint32_t fj_joint_distance(FjWorld *world, uint32_t a, uint32_t b, const double point_a[3],
                           const double point_b[3], float min, float max);
// What `count` joints carried in the last step, two floats each: the impulse of the part that
// holds their points together (N·s) and of the part that holds their turn (N·m·s, 0 for a
// distance).
void fj_joints_load(const FjWorld *world, const uint32_t *joints, uint32_t count, float *loads);
// Breaks (0) or mends (1) joints, waking their bodies; a broken joint holds nothing.
void fj_joints_set(FjWorld *world, const uint32_t *joints, uint32_t count, const uint8_t *holding);
// Whether joints hold (1) or are broken (0).
void fj_joints_holding(const FjWorld *world, const uint32_t *joints, uint32_t count,
                       uint8_t *holding);

// A part of a ragdoll (#143): a body, and the joint that holds it to its parent part. Everything
// is given in the world as built; the parts come parents first.
typedef struct FjRagdollPart {
    const FjShape *shape;
    double position[3];
    float rotation[4];
    // The parent's index among the parts, −1 for the root (which has no joint).
    int32_t parent;
    // 0: a swing-twist joint (a shoulder, a hip, a neck); 1: a hinge (a knee, an elbow) about
    // `plane_axis`.
    uint32_t kind;
    double pivot[3];
    // Along the part from the pivot (unit), and across it (unit, at a right angle to it).
    float twist_axis[3];
    float plane_axis[3];
    // Swing-twist: the swing cone's half angles and the twist's range; a hinge: its range in
    // `twist_min` and `twist_max`. Radians.
    float normal_cone;
    float plane_cone;
    float twist_min;
    float twist_max;
    float friction;
} FjRagdollPart;

// Adds a ragdoll of `count` parts (Jolt's Ragdoll: its parts' bodies, and their joints; a part
// does not collide with its parent); writes its parts' bodies into `bodies`, returns its index.
// Saved and restored with the world.
uint32_t fj_ragdoll_add(FjWorld *world, const FjRagdollPart *parts, uint32_t count,
                        uint32_t *bodies);
// Drives a ragdoll's joints towards `targets`, four floats a part (the root's ignored): for a
// swing-twist joint the part's turn in its joint's frame (a quaternion, x y z w; identity is
// the pose as built), for a hinge its angle in the first. Motors of a spring of `stiffness`
// (N·m a radian) and `damping` (N·m·s a radian), whatever the parts weigh, at most `torque`
// N·m; a torque of 0 lets the ragdoll go limp.
void fj_ragdoll_drive(FjWorld *world, uint32_t ragdoll, const float *targets, float stiffness,
                      float damping, float torque);

// The whole simulation state (bodies, contacts, constraints) into a buffer the world owns,
// valid until the next call; `size` receives its length.
const uint8_t *fj_world_save_state(FjWorld *world, size_t *size);
// Back to a saved state of the same world (the same bodies); 0 when it does not fit.
int32_t fj_world_restore_state(FjWorld *world, const uint8_t *data, size_t size);

#ifdef __cplusplus
}
#endif

#endif
