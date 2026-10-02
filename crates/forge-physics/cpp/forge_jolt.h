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

// The sizes of the structs above, for the layout test.
typedef struct FjLayout {
    uint32_t world_desc;
    uint32_t body_desc;
    uint32_t ray_hit;
    uint32_t character_desc;
    uint32_t character_state;
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
// `inner` moved by `position` and turned by `rotation` (x, y, z, w) in its body's frame.
FjShape *fj_shape_offset(const FjShape *inner, const float position[3], const float rotation[4]);
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

// The nearest hit along `direction` (its length is the ray's) from `origin`; 0 when none.
int32_t fj_world_cast_ray(const FjWorld *world, const double origin[3], const float direction[3],
                          FjRayHit *hit);

// Adds a character; its index. The world's characters are numbered from 1 in the order added
// (Jolt's own numbering runs across every world of the process), and saved and restored with it.
uint32_t fj_character_add(FjWorld *world, const FjCharacterDesc *desc);
// Moves it through a step of `dt` at `velocity` (the world's gravity presses it down).
void fj_character_move(FjWorld *world, uint32_t character, float dt, const float velocity[3]);
void fj_character_state(const FjWorld *world, uint32_t character, FjCharacterState *state);

// The whole simulation state (bodies, contacts, constraints) into a buffer the world owns,
// valid until the next call; `size` receives its length.
const uint8_t *fj_world_save_state(FjWorld *world, size_t *size);
// Back to a saved state of the same world (the same bodies); 0 when it does not fit.
int32_t fj_world_restore_state(FjWorld *world, const uint8_t *data, size_t size);

#ifdef __cplusplus
}
#endif

#endif
