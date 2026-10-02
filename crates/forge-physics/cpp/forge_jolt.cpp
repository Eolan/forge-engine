// Forge's C layer over Jolt Physics (D-009, issue #136); see forge_jolt.h. The layer set-up
// (two object layers, two broad-phase layers) follows Jolt's HelloWorld and JoltC.

#include "forge_jolt.h"

// Jolt.h comes before every other Jolt header.
#include <Jolt/Jolt.h>

#include <Jolt/Core/Factory.h>
#include <Jolt/Core/JobSystemThreadPool.h>
#include <Jolt/Core/TempAllocator.h>
#include <Jolt/Physics/Body/BodyCreationSettings.h>
#include <Jolt/Physics/Body/BodyLock.h>
#include <Jolt/Physics/Collision/CastResult.h>
#include <Jolt/Physics/Collision/RayCast.h>
#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/ConvexHullShape.h>
#include <Jolt/Physics/Collision/Shape/CylinderShape.h>
#include <Jolt/Physics/Collision/Shape/MeshShape.h>
#include <Jolt/Physics/Collision/Shape/OffsetCenterOfMassShape.h>
#include <Jolt/Physics/Collision/Shape/RotatedTranslatedShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>
#include <Jolt/Physics/PhysicsSettings.h>
#include <Jolt/Physics/PhysicsSystem.h>
#include <Jolt/Physics/StateRecorder.h>
#include <Jolt/RegisterTypes.h>

#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <vector>

namespace {

// Object layers: what never moves, and what does.
constexpr JPH::ObjectLayer kStill = 0;
constexpr JPH::ObjectLayer kMoving = 1;

namespace broad {
constexpr JPH::BroadPhaseLayer kStill(0);
constexpr JPH::BroadPhaseLayer kMoving(1);
constexpr JPH::uint kCount = 2;
} // namespace broad

// Still bodies meet only moving ones; moving ones meet everything.
class LayerPairs final : public JPH::ObjectLayerPairFilter {
public:
    bool ShouldCollide(JPH::ObjectLayer a, JPH::ObjectLayer b) const override {
        return a == kMoving || b == kMoving;
    }
};

class BroadLayers final : public JPH::BroadPhaseLayerInterface {
public:
    JPH::uint GetNumBroadPhaseLayers() const override { return broad::kCount; }
    JPH::BroadPhaseLayer GetBroadPhaseLayer(JPH::ObjectLayer layer) const override {
        return layer == kStill ? broad::kStill : broad::kMoving;
    }
#if defined(JPH_EXTERNAL_PROFILE) || defined(JPH_PROFILE_ENABLED)
    const char *GetBroadPhaseLayerName(JPH::BroadPhaseLayer layer) const override {
        return layer == broad::kStill ? "still" : "moving";
    }
#endif
};

class LayerVsBroad final : public JPH::ObjectVsBroadPhaseLayerFilter {
public:
    bool ShouldCollide(JPH::ObjectLayer layer, JPH::BroadPhaseLayer broad_layer) const override {
        return layer == kMoving || broad_layer == broad::kMoving;
    }
};

// A state recorder over a byte vector (Jolt's own goes through a string stream).
class Bytes final : public JPH::StateRecorder {
public:
    std::vector<uint8_t> data;
    size_t cursor = 0;
    bool failed = false;

    void WriteBytes(const void *in, size_t count) override {
        const uint8_t *bytes = static_cast<const uint8_t *>(in);
        data.insert(data.end(), bytes, bytes + count);
    }
    void ReadBytes(void *out, size_t count) override {
        if (cursor + count > data.size()) {
            failed = true;
            std::memset(out, 0, count);
            return;
        }
        std::memcpy(out, data.data() + cursor, count);
        cursor += count;
    }
    bool IsEOF() const override { return cursor >= data.size(); }
    bool IsFailed() const override { return failed; }
};

void trace(const char *format, ...) {
    va_list args;
    va_start(args, format);
    char line[1024];
    std::vsnprintf(line, sizeof line, format, args);
    va_end(args);
    std::fprintf(stderr, "jolt: %s\n", line);
}

#ifdef JPH_ENABLE_ASSERTS
bool assert_failed(const char *expression, const char *message, const char *file,
                   JPH::uint line) {
    std::fprintf(stderr, "jolt: %s:%u: %s %s\n", file, line, expression, message ? message : "");
    return true;
}
#endif

const JPH::Shape *shape_of(const FjShape *shape) {
    return reinterpret_cast<const JPH::Shape *>(shape);
}

// One reference to a created shape for the caller, or NULL with Jolt's reason on stderr.
FjShape *hand_out(const JPH::ShapeSettings::ShapeResult &result) {
    if (result.HasError()) {
        std::fprintf(stderr, "jolt: shape refused: %s\n", result.GetError().c_str());
        return nullptr;
    }
    const JPH::Shape *shape = result.Get().GetPtr();
    shape->AddRef();
    return reinterpret_cast<FjShape *>(const_cast<JPH::Shape *>(shape));
}

FjShape *convex(JPH::ConvexShapeSettings &settings, float density) {
    settings.SetEmbedded();
    if (density > 0.0f) {
        settings.SetDensity(density);
    }
    return hand_out(settings.Create());
}

JPH::BodyID id_of(uint32_t body) { return JPH::BodyID(body); }

JPH::Vec3 vec3(const float v[3]) { return JPH::Vec3(v[0], v[1], v[2]); }
JPH::RVec3 rvec3(const double v[3]) { return JPH::RVec3(v[0], v[1], v[2]); }
JPH::Quat quat(const float q[4]) { return JPH::Quat(q[0], q[1], q[2], q[3]); }

} // namespace

struct FjWorld {
    BroadLayers broad_layers;
    LayerVsBroad layer_vs_broad;
    LayerPairs layer_pairs;
    JPH::TempAllocatorImpl temp;
    JPH::JobSystemThreadPool jobs;
    JPH::PhysicsSystem system;
    Bytes saved;

    explicit FjWorld(const FjWorldDesc &desc)
        : temp(64 * 1024 * 1024),
          jobs(JPH::cMaxPhysicsJobs, JPH::cMaxPhysicsBarriers, static_cast<int>(desc.threads)) {
        system.Init(desc.max_bodies, 0, desc.max_body_pairs, desc.max_contact_constraints,
                    broad_layers, layer_vs_broad, layer_pairs);
        system.SetGravity(vec3(desc.gravity));
    }
};

extern "C" {

FjLayout fj_layout(void) {
    return FjLayout{static_cast<uint32_t>(sizeof(FjWorldDesc)),
                    static_cast<uint32_t>(sizeof(FjBodyDesc)),
                    static_cast<uint32_t>(sizeof(FjRayHit))};
}

void fj_init(void) {
    static std::once_flag once;
    std::call_once(once, [] {
        JPH::RegisterDefaultAllocator();
        JPH::Trace = trace;
#ifdef JPH_ENABLE_ASSERTS
        JPH::AssertFailed = assert_failed;
#endif
        JPH::Factory::sInstance = new JPH::Factory();
        JPH::RegisterTypes();
    });
}

FjShape *fj_shape_box(const float half_extent[3], float convex_radius, float density) {
    const float smallest = std::min(half_extent[0], std::min(half_extent[1], half_extent[2]));
    JPH::BoxShapeSettings settings(vec3(half_extent), std::min(convex_radius, smallest));
    return convex(settings, density);
}

FjShape *fj_shape_sphere(float radius, float density) {
    JPH::SphereShapeSettings settings(radius);
    return convex(settings, density);
}

FjShape *fj_shape_capsule(float half_height, float radius, float density) {
    JPH::CapsuleShapeSettings settings(half_height, radius);
    return convex(settings, density);
}

FjShape *fj_shape_cylinder(float half_height, float radius, float convex_radius, float density) {
    const float smallest = std::min(half_height, radius);
    JPH::CylinderShapeSettings settings(half_height, radius, std::min(convex_radius, smallest));
    return convex(settings, density);
}

FjShape *fj_shape_convex_hull(const float *points, uint32_t count, float max_convex_radius,
                              float density) {
    JPH::Array<JPH::Vec3> hull;
    hull.reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
        hull.push_back(JPH::Vec3(points[3 * i], points[3 * i + 1], points[3 * i + 2]));
    }
    JPH::ConvexHullShapeSettings settings(hull, max_convex_radius);
    return convex(settings, density);
}

FjShape *fj_shape_mesh(const float *vertices, uint32_t vertex_count, const uint32_t *indices,
                       uint32_t triangle_count) {
    JPH::VertexList points;
    points.reserve(vertex_count);
    for (uint32_t i = 0; i < vertex_count; ++i) {
        points.push_back(JPH::Float3(vertices[3 * i], vertices[3 * i + 1], vertices[3 * i + 2]));
    }
    JPH::IndexedTriangleList triangles;
    triangles.reserve(triangle_count);
    for (uint32_t t = 0; t < triangle_count; ++t) {
        triangles.push_back(
            JPH::IndexedTriangle(indices[3 * t], indices[3 * t + 1], indices[3 * t + 2], 0));
    }
    JPH::MeshShapeSettings settings(std::move(points), std::move(triangles));
    settings.SetEmbedded();
    return hand_out(settings.Create());
}

FjShape *fj_shape_offset(const FjShape *inner, const float position[3], const float rotation[4]) {
    JPH::RotatedTranslatedShapeSettings settings(vec3(position), quat(rotation), shape_of(inner));
    settings.SetEmbedded();
    return hand_out(settings.Create());
}

FjShape *fj_shape_offset_center_of_mass(const FjShape *inner, const float offset[3]) {
    JPH::OffsetCenterOfMassShapeSettings settings(vec3(offset), shape_of(inner));
    settings.SetEmbedded();
    return hand_out(settings.Create());
}

void fj_shape_release(const FjShape *shape) {
    if (shape != nullptr) {
        shape_of(shape)->Release();
    }
}

FjWorld *fj_world_new(const FjWorldDesc *desc) { return new FjWorld(*desc); }

void fj_world_free(FjWorld *world) { delete world; }

void fj_world_optimize_broad_phase(FjWorld *world) { world->system.OptimizeBroadPhase(); }

uint32_t fj_world_step(FjWorld *world, float dt, int32_t collision_steps) {
    return static_cast<uint32_t>(
        world->system.Update(dt, collision_steps, &world->temp, &world->jobs));
}

uint32_t fj_world_active_bodies(const FjWorld *world) {
    return world->system.GetNumActiveBodies(JPH::EBodyType::RigidBody);
}

uint32_t fj_body_add(FjWorld *world, const FjBodyDesc *desc) {
    const JPH::EMotionType motion = desc->motion == 0   ? JPH::EMotionType::Static
                                    : desc->motion == 1 ? JPH::EMotionType::Kinematic
                                                        : JPH::EMotionType::Dynamic;
    JPH::BodyCreationSettings settings(shape_of(desc->shape), rvec3(desc->position),
                                       quat(desc->rotation), motion,
                                       desc->motion == 0 ? kStill : kMoving);
    settings.mLinearVelocity = vec3(desc->linear_velocity);
    settings.mAngularVelocity = vec3(desc->angular_velocity);
    settings.mFriction = desc->friction;
    settings.mRestitution = desc->restitution;
    settings.mLinearDamping = desc->linear_damping;
    settings.mAngularDamping = desc->angular_damping;
    settings.mUserData = desc->user_data;
    settings.mAllowSleeping = desc->allow_sleep != 0;
    if (desc->ccd != 0) {
        settings.mMotionQuality = JPH::EMotionQuality::LinearCast;
    }
    if (desc->mass > 0.0f) {
        settings.mOverrideMassProperties = JPH::EOverrideMassProperties::CalculateInertia;
        settings.mMassPropertiesOverride.mMass = desc->mass;
    }
    const JPH::BodyID id = world->system.GetBodyInterface().CreateAndAddBody(
        settings, desc->activate != 0 ? JPH::EActivation::Activate
                                      : JPH::EActivation::DontActivate);
    return id.GetIndexAndSequenceNumber();
}

void fj_body_remove(FjWorld *world, uint32_t body) {
    JPH::BodyInterface &bodies = world->system.GetBodyInterface();
    bodies.RemoveBody(id_of(body));
    bodies.DestroyBody(id_of(body));
}

void fj_bodies_transforms(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                          double *positions, float *rotations) {
    const JPH::BodyInterface &all = world->system.GetBodyInterfaceNoLock();
    for (uint32_t i = 0; i < count; ++i) {
        JPH::RVec3 p;
        JPH::Quat q;
        all.GetPositionAndRotation(id_of(bodies[i]), p, q);
        positions[3 * i] = p.GetX();
        positions[3 * i + 1] = p.GetY();
        positions[3 * i + 2] = p.GetZ();
        rotations[4 * i] = q.GetX();
        rotations[4 * i + 1] = q.GetY();
        rotations[4 * i + 2] = q.GetZ();
        rotations[4 * i + 3] = q.GetW();
    }
}

void fj_bodies_velocities(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                          float *linear, float *angular) {
    const JPH::BodyInterface &all = world->system.GetBodyInterfaceNoLock();
    for (uint32_t i = 0; i < count; ++i) {
        JPH::Vec3 v;
        JPH::Vec3 w;
        all.GetLinearAndAngularVelocity(id_of(bodies[i]), v, w);
        v.StoreFloat3(reinterpret_cast<JPH::Float3 *>(linear + 3 * i));
        w.StoreFloat3(reinterpret_cast<JPH::Float3 *>(angular + 3 * i));
    }
}

void fj_bodies_active(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                      uint8_t *active) {
    const JPH::BodyInterface &all = world->system.GetBodyInterfaceNoLock();
    for (uint32_t i = 0; i < count; ++i) {
        active[i] = all.IsActive(id_of(bodies[i])) ? 1 : 0;
    }
}

void fj_body_add_impulse(FjWorld *world, uint32_t body, const float impulse[3]) {
    world->system.GetBodyInterface().AddImpulse(id_of(body), vec3(impulse));
}

void fj_body_add_impulse_at(FjWorld *world, uint32_t body, const float impulse[3],
                            const double point[3]) {
    world->system.GetBodyInterface().AddImpulse(id_of(body), vec3(impulse), rvec3(point));
}

void fj_body_add_force(FjWorld *world, uint32_t body, const float force[3]) {
    world->system.GetBodyInterface().AddForce(id_of(body), vec3(force));
}

void fj_bodies_push(FjWorld *world, const uint32_t *bodies, uint32_t count, const float *forces,
                    const double *points, const float *torques) {
    JPH::BodyInterface &all = world->system.GetBodyInterface();
    for (uint32_t i = 0; i < count; ++i) {
        all.AddForce(id_of(bodies[i]), vec3(forces + 3 * i), rvec3(points + 3 * i));
        all.AddTorque(id_of(bodies[i]), vec3(torques + 3 * i));
    }
}

void fj_bodies_centers_of_mass(const FjWorld *world, const uint32_t *bodies, uint32_t count,
                               double *centers) {
    const JPH::BodyInterface &all = world->system.GetBodyInterfaceNoLock();
    for (uint32_t i = 0; i < count; ++i) {
        const JPH::RVec3 c = all.GetCenterOfMassPosition(id_of(bodies[i]));
        centers[3 * i] = c.GetX();
        centers[3 * i + 1] = c.GetY();
        centers[3 * i + 2] = c.GetZ();
    }
}

void fj_body_set_velocity(FjWorld *world, uint32_t body, const float linear[3],
                          const float angular[3]) {
    world->system.GetBodyInterface().SetLinearAndAngularVelocity(id_of(body), vec3(linear),
                                                                 vec3(angular));
}

void fj_body_set_transform(FjWorld *world, uint32_t body, const double position[3],
                           const float rotation[4]) {
    world->system.GetBodyInterface().SetPositionAndRotation(
        id_of(body), rvec3(position), quat(rotation), JPH::EActivation::Activate);
}

int32_t fj_world_cast_ray(const FjWorld *world, const double origin[3], const float direction[3],
                          FjRayHit *hit) {
    const JPH::RRayCast ray{rvec3(origin), vec3(direction)};
    JPH::RayCastResult result;
    if (!world->system.GetNarrowPhaseQuery().CastRay(ray, result)) {
        return 0;
    }
    hit->body = result.mBodyID.GetIndexAndSequenceNumber();
    hit->fraction = result.mFraction;
    JPH::Vec3 normal = -vec3(direction).NormalizedOr(JPH::Vec3::sAxisY());
    JPH::BodyLockRead lock(world->system.GetBodyLockInterfaceNoLock(), result.mBodyID);
    if (lock.Succeeded()) {
        normal = lock.GetBody().GetWorldSpaceSurfaceNormal(result.mSubShapeID2,
                                                            ray.GetPointOnRay(result.mFraction));
    }
    normal.StoreFloat3(reinterpret_cast<JPH::Float3 *>(hit->normal));
    return 1;
}

const uint8_t *fj_world_save_state(FjWorld *world, size_t *size) {
    world->saved.data.clear();
    world->saved.cursor = 0;
    world->saved.failed = false;
    world->system.SaveState(world->saved);
    *size = world->saved.data.size();
    return world->saved.data.data();
}

int32_t fj_world_restore_state(FjWorld *world, const uint8_t *data, size_t size) {
    Bytes state;
    state.data.assign(data, data + size);
    return world->system.RestoreState(state) && !state.failed ? 1 : 0;
}

} // extern "C"
