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
#include <Jolt/Physics/Character/CharacterVirtual.h>
#include <Jolt/Physics/Vehicle/VehicleCollisionTester.h>
#include <Jolt/Physics/Vehicle/VehicleConstraint.h>
#include <Jolt/Physics/Vehicle/WheeledVehicleController.h>
#include <Jolt/Physics/Collision/CastResult.h>
#include <Jolt/Physics/Collision/RayCast.h>
#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/ConvexHullShape.h>
#include <Jolt/Physics/Collision/Shape/CylinderShape.h>
#include <Jolt/Physics/Collision/Shape/HeightFieldShape.h>
#include <Jolt/Physics/Collision/Shape/MeshShape.h>
#include <Jolt/Physics/Collision/Shape/OffsetCenterOfMassShape.h>
#include <Jolt/Physics/Collision/Shape/RotatedTranslatedShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>
#include <Jolt/Physics/Constraints/DistanceConstraint.h>
#include <Jolt/Physics/Constraints/FixedConstraint.h>
#include <Jolt/Physics/Constraints/HingeConstraint.h>
#include <Jolt/Physics/Constraints/SwingTwistConstraint.h>
#include <Jolt/Physics/Ragdoll/Ragdoll.h>
#include <Jolt/Physics/SoftBody/SoftBodyCreationSettings.h>
#include <Jolt/Physics/SoftBody/SoftBodyMotionProperties.h>
#include <Jolt/Physics/SoftBody/SoftBodySharedSettings.h>
#include <Jolt/Skeleton/Skeleton.h>
#include <Jolt/Physics/PhysicsSettings.h>
#include <Jolt/Physics/PhysicsSystem.h>
#include <Jolt/Physics/StateRecorder.h>
#include <Jolt/RegisterTypes.h>

#include <algorithm>
#include <cmath>
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
    // After the system, so they go first: each holds the system.
    JPH::CharacterVsCharacterCollisionSimple character_pairs;
    std::vector<JPH::Ref<JPH::CharacterVirtual>> characters;
    std::vector<JPH::CharacterVirtual::ExtendedUpdateSettings> character_steps;
    // The cars: their constraints (in the system, which saves them) and wheel testers.
    std::vector<JPH::Ref<JPH::VehicleConstraint>> vehicles;
    std::vector<JPH::Ref<JPH::VehicleCollisionTester>> vehicle_testers;
    // The joints, in the system too (which saves whether each holds, and its impulses).
    std::vector<JPH::Ref<JPH::TwoBodyConstraint>> joints;
    // The ragdolls and the settings they were made from (which map their joints to parts).
    std::vector<JPH::Ref<JPH::Ragdoll>> ragdolls;
    std::vector<JPH::Ref<JPH::RagdollSettings>> ragdoll_settings;

    explicit FjWorld(const FjWorldDesc &desc)
        : temp(64 * 1024 * 1024),
          jobs(JPH::cMaxPhysicsJobs, JPH::cMaxPhysicsBarriers, static_cast<int>(desc.threads)) {
        system.Init(desc.max_bodies, 0, desc.max_body_pairs, desc.max_contact_constraints,
                    broad_layers, layer_vs_broad, layer_pairs);
        system.SetGravity(vec3(desc.gravity));
    }

    // A ragdoll destroys its bodies as it goes, which must be out of the system by then.
    ~FjWorld() {
        for (const JPH::Ref<JPH::Ragdoll> &r : ragdolls) {
            r->RemoveFromPhysicsSystem();
        }
    }

    FjWorld(const FjWorld &) = delete;
    FjWorld &operator=(const FjWorld &) = delete;
};

namespace {

// Makes a joint between `a` and `b` (FJ_WORLD, an invalid id, for the world: Jolt's fixed
// body) and puts it in the system; its index.
uint32_t add_joint(FjWorld *world, const JPH::TwoBodyConstraintSettings &settings, uint32_t a,
                   uint32_t b) {
    JPH::BodyInterface &bodies = world->system.GetBodyInterface();
    JPH::Ref<JPH::TwoBodyConstraint> joint = bodies.CreateConstraint(&settings, id_of(a), id_of(b));
    world->system.AddConstraint(joint);
    world->joints.push_back(joint);
    return static_cast<uint32_t>(world->joints.size() - 1);
}

} // namespace

extern "C" {

FjLayout fj_layout(void) {
    return FjLayout{static_cast<uint32_t>(sizeof(FjWorldDesc)),
                    static_cast<uint32_t>(sizeof(FjBodyDesc)),
                    static_cast<uint32_t>(sizeof(FjRayHit)),
                    static_cast<uint32_t>(sizeof(FjCharacterDesc)),
                    static_cast<uint32_t>(sizeof(FjCharacterState)),
                    static_cast<uint32_t>(sizeof(FjVehicleDesc)),
                    static_cast<uint32_t>(sizeof(FjRagdollPart)),
                    static_cast<uint32_t>(sizeof(FjSoftBodyDesc))};
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

FjShape *fj_shape_height_field(const float *samples, uint32_t count, const float offset[3],
                               const float scale[3]) {
    JPH::HeightFieldShapeSettings settings(samples, vec3(offset), vec3(scale), count);
    settings.SetEmbedded();
    return hand_out(settings.Create());
}

FjShape *fj_shape_offset(const FjShape *inner, const float position[3], const float rotation[4]) {
    JPH::RotatedTranslatedShapeSettings settings(vec3(position), quat(rotation), shape_of(inner));
    settings.SetEmbedded();
    return hand_out(settings.Create());
}

void fj_shape_center_of_mass(const FjShape *shape, float out[3]) {
    shape_of(shape)->GetCenterOfMass().StoreFloat3(reinterpret_cast<JPH::Float3 *>(out));
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

uint32_t fj_soft_body_add(FjWorld *world, const FjSoftBodyDesc *desc) {
    JPH::Ref<JPH::SoftBodySharedSettings> shared = new JPH::SoftBodySharedSettings;
    for (uint32_t i = 0; i < desc->vertex_count; ++i) {
        const float *p = desc->points + 3 * i;
        shared->mVertices.push_back(JPH::SoftBodySharedSettings::Vertex(
            JPH::Float3(p[0], p[1], p[2]), JPH::Float3(0.0f, 0.0f, 0.0f), desc->inverse_mass));
    }
    for (uint32_t f = 0; f < desc->face_count; ++f) {
        const uint32_t *t = desc->faces + 3 * f;
        shared->AddFace(JPH::SoftBodySharedSettings::Face(t[0], t[1], t[2]));
    }
    const JPH::SoftBodySharedSettings::VertexAttributes attributes(
        desc->compliance, desc->compliance, desc->bend_compliance);
    shared->CreateConstraints(&attributes, 1, JPH::SoftBodySharedSettings::EBendType::Distance);
    shared->Optimize();
    JPH::SoftBodyCreationSettings settings(shared, rvec3(desc->position), JPH::Quat::sIdentity(),
                                           kMoving);
    settings.mPressure = desc->pressure;
    settings.mFriction = desc->friction;
    settings.mRestitution = desc->restitution;
    settings.mNumIterations = desc->iterations;
    settings.mGravityFactor = desc->gravity_factor;
    settings.mUserData = desc->user_data;
    const JPH::BodyID id = world->system.GetBodyInterface().CreateAndAddSoftBody(
        settings, JPH::EActivation::Activate);
    return id.IsInvalid() ? UINT32_MAX : id.GetIndexAndSequenceNumber();
}

void fj_soft_body_push(FjWorld *world, uint32_t body, const float velocity[3]) {
    const JPH::BodyID id = id_of(body);
    {
        JPH::BodyLockWrite lock(world->system.GetBodyLockInterface(), id);
        if (!lock.Succeeded() || !lock.GetBody().IsSoftBody()) {
            return;
        }
        auto *motion =
            static_cast<JPH::SoftBodyMotionProperties *>(lock.GetBody().GetMotionProperties());
        const JPH::Vec3 dv(velocity[0], velocity[1], velocity[2]);
        for (JPH::SoftBodyVertex &v : motion->GetVertices()) {
            if (v.mInvMass > 0.0f) {
                v.mVelocity += dv;
            }
        }
    }
    world->system.GetBodyInterface().ActivateBody(id);
}

void fj_soft_body_upright(FjWorld *world, uint32_t body, uint32_t top, float spring, float damping) {
    JPH::BodyLockWrite lock(world->system.GetBodyLockInterface(), id_of(body));
    if (!lock.Succeeded() || !lock.GetBody().IsSoftBody()) {
        return;
    }
    auto *motion = static_cast<JPH::SoftBodyMotionProperties *>(lock.GetBody().GetMotionProperties());
    JPH::Array<JPH::SoftBodyVertex> &vertices = motion->GetVertices();
    if (top >= vertices.size()) {
        return;
    }
    // The body's middle and its spin, as a rigid body's: L = Σ r × v, I = Σ (|r|² 1 − r rᵀ),
    // ω = I⁻¹ L (the points weigh alike).
    JPH::Vec3 middle = JPH::Vec3::sZero();
    JPH::Vec3 drift = JPH::Vec3::sZero();
    for (const JPH::SoftBodyVertex &v : vertices) {
        middle += v.mPosition;
        drift += v.mVelocity;
    }
    const float n = static_cast<float>(vertices.size());
    middle /= n;
    drift /= n;
    JPH::Vec3 momentum = JPH::Vec3::sZero();
    float xx = 0.0f, yy = 0.0f, zz = 0.0f, xy = 0.0f, xz = 0.0f, yz = 0.0f;
    for (const JPH::SoftBodyVertex &v : vertices) {
        const JPH::Vec3 r = v.mPosition - middle;
        momentum += r.Cross(v.mVelocity - drift);
        xx += r.GetX() * r.GetX();
        yy += r.GetY() * r.GetY();
        zz += r.GetZ() * r.GetZ();
        xy += r.GetX() * r.GetY();
        xz += r.GetX() * r.GetZ();
        yz += r.GetY() * r.GetZ();
    }
    const JPH::Mat44 inertia(JPH::Vec4(yy + zz, -xy, -xz, 0.0f), JPH::Vec4(-xy, xx + zz, -yz, 0.0f),
                             JPH::Vec4(-xz, -yz, xx + yy, 0.0f), JPH::Vec4(0.0f, 0.0f, 0.0f, 1.0f));
    const JPH::Vec3 spin = inertia.Inversed3x3().Multiply3x3(momentum);
    // Turned back: the top's way from the middle towards +y (about up × y, as far as the sine
    // of the tilt), and the spin damped.
    const JPH::Vec3 up = (vertices[top].mPosition - middle).NormalizedOr(JPH::Vec3::sAxisY());
    const JPH::Vec3 change = up.Cross(JPH::Vec3::sAxisY()) * spring - spin * damping;
    for (JPH::SoftBodyVertex &v : vertices) {
        if (v.mInvMass > 0.0f) {
            v.mVelocity += change.Cross(v.mPosition - middle);
        }
    }
}

uint32_t fj_soft_body_vertices(const FjWorld *world, uint32_t body, float *points, uint32_t capacity,
                               double origin[3]) {
    JPH::BodyLockRead lock(world->system.GetBodyLockInterface(), id_of(body));
    if (!lock.Succeeded() || !lock.GetBody().IsSoftBody()) {
        return 0;
    }
    const JPH::Body &b = lock.GetBody();
    const auto *motion = static_cast<const JPH::SoftBodyMotionProperties *>(b.GetMotionProperties());
    const JPH::RVec3 at = b.GetCenterOfMassPosition();
    origin[0] = at.GetX();
    origin[1] = at.GetY();
    origin[2] = at.GetZ();
    const JPH::Array<JPH::SoftBodyVertex> &vertices = motion->GetVertices();
    for (size_t i = 0; i < vertices.size() && i < capacity; ++i) {
        vertices[i].mPosition.StoreFloat3(reinterpret_cast<JPH::Float3 *>(points + 3 * i));
    }
    return static_cast<uint32_t>(vertices.size());
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

uint32_t fj_character_add(FjWorld *world, const FjCharacterDesc *desc) {
    const float half = std::max(0.5f * desc->height - desc->radius, 0.01f);
    JPH::Ref<JPH::CharacterVirtualSettings> settings = new JPH::CharacterVirtualSettings();
    // The capsule over its feet: the character's position is where it stands.
    settings->mShape = JPH::RotatedTranslatedShapeSettings(
                           JPH::Vec3(0.0f, half + desc->radius, 0.0f), JPH::Quat::sIdentity(),
                           new JPH::CapsuleShape(half, desc->radius))
                           .Create()
                           .Get();
    settings->mMaxSlopeAngle = desc->max_slope;
    settings->mMass = desc->mass;
    settings->mMaxStrength = desc->max_strength;
    // Only contacts under its lower half-sphere's centre hold it up.
    settings->mSupportingVolume = JPH::Plane(JPH::Vec3::sAxisY(), -desc->radius);
    const uint32_t index = static_cast<uint32_t>(world->characters.size());
    settings->mID = JPH::CharacterID(index + 1);
    JPH::Ref<JPH::CharacterVirtual> character = new JPH::CharacterVirtual(
        settings, rvec3(desc->position), JPH::Quat::sIdentity(), 0, &world->system);
    character->SetCharacterVsCharacterCollision(&world->character_pairs);
    world->character_pairs.Add(character);
    JPH::CharacterVirtual::ExtendedUpdateSettings steps;
    steps.mStickToFloorStepDown = JPH::Vec3(0.0f, -desc->stick_down, 0.0f);
    steps.mWalkStairsStepUp = JPH::Vec3(0.0f, desc->step_up, 0.0f);
    world->characters.push_back(character);
    world->character_steps.push_back(steps);
    return index;
}

void fj_character_move(FjWorld *world, uint32_t character, float dt, const float velocity[3]) {
    JPH::CharacterVirtual &c = *world->characters[character];
    c.SetLinearVelocity(vec3(velocity));
    c.ExtendedUpdate(dt, world->system.GetGravity(), world->character_steps[character],
                     world->system.GetDefaultBroadPhaseLayerFilter(kMoving),
                     world->system.GetDefaultLayerFilter(kMoving), JPH::BodyFilter(),
                     JPH::ShapeFilter(), world->temp);
}

void fj_character_state(const FjWorld *world, uint32_t character, FjCharacterState *state) {
    const JPH::CharacterVirtual &c = *world->characters[character];
    const JPH::RVec3 p = c.GetPosition();
    state->position[0] = p.GetX();
    state->position[1] = p.GetY();
    state->position[2] = p.GetZ();
    c.GetLinearVelocity().StoreFloat3(reinterpret_cast<JPH::Float3 *>(state->velocity));
    c.GetGroundNormal().StoreFloat3(reinterpret_cast<JPH::Float3 *>(state->ground_normal));
    c.GetGroundVelocity().StoreFloat3(reinterpret_cast<JPH::Float3 *>(state->ground_velocity));
    state->ground_body = c.GetGroundBodyID().GetIndexAndSequenceNumber();
    state->ground_state = static_cast<uint32_t>(c.GetGroundState());
}

uint32_t fj_vehicle_add(FjWorld *world, uint32_t chassis, const FjVehicleDesc *desc) {
    JPH::VehicleConstraintSettings settings;
    settings.mUp = JPH::Vec3::sAxisY();
    settings.mForward = -JPH::Vec3::sAxisZ();
    // Front left, front right, rear left, rear right; the front ones steer, the rear ones take
    // the handbrake.
    const float x[4] = {-desc->half_track, desc->half_track, -desc->half_track, desc->half_track};
    const float z[4] = {-desc->half_wheelbase, -desc->half_wheelbase, desc->half_wheelbase,
                        desc->half_wheelbase};
    for (int w = 0; w < 4; ++w) {
        JPH::WheelSettingsWV *wheel = new JPH::WheelSettingsWV();
        wheel->mPosition = JPH::Vec3(x[w], desc->attach_y, z[w]);
        wheel->mWheelForward = -JPH::Vec3::sAxisZ();
        wheel->mSuspensionMinLength = desc->suspension_min;
        wheel->mSuspensionMaxLength = desc->suspension_max;
        wheel->mSuspensionSpring.mFrequency = desc->spring_frequency;
        wheel->mSuspensionSpring.mDamping = desc->spring_damping;
        wheel->mRadius = desc->wheel_radius;
        wheel->mWidth = desc->wheel_width;
        wheel->mMaxSteerAngle = w < 2 ? desc->max_steer : 0.0f;
        wheel->mMaxBrakeTorque = desc->brake_torque;
        wheel->mMaxHandBrakeTorque = w < 2 ? 0.0f : desc->handbrake_torque;
        settings.mWheels.push_back(wheel);
    }
    JPH::Ref<JPH::WheeledVehicleControllerSettings> controller =
        new JPH::WheeledVehicleControllerSettings();
    controller->mEngine.mMaxTorque = desc->engine_torque;
    controller->mEngine.mMaxRPM = desc->max_rpm;
    // Front-wheel drive.
    controller->mDifferentials.resize(1);
    controller->mDifferentials[0].mLeftWheel = 0;
    controller->mDifferentials[0].mRightWheel = 1;
    settings.mController = controller;
    // An anti-roll bar on each axle keeps it from leaning over in the corners.
    settings.mAntiRollBars.resize(2);
    settings.mAntiRollBars[0].mLeftWheel = 0;
    settings.mAntiRollBars[0].mRightWheel = 1;
    settings.mAntiRollBars[1].mLeftWheel = 2;
    settings.mAntiRollBars[1].mRightWheel = 3;

    JPH::Ref<JPH::VehicleConstraint> vehicle;
    {
        // The chassis held while the constraint takes it.
        JPH::BodyLockWrite lock(world->system.GetBodyLockInterface(), id_of(chassis));
        if (!lock.Succeeded()) {
            return UINT32_MAX;
        }
        vehicle = new JPH::VehicleConstraint(lock.GetBody(), settings);
    }
    JPH::Ref<JPH::VehicleCollisionTester> tester =
        new JPH::VehicleCollisionTesterCastCylinder(kMoving);
    vehicle->SetVehicleCollisionTester(tester);
    world->system.AddConstraint(vehicle);
    world->system.AddStepListener(vehicle);
    world->vehicles.push_back(vehicle);
    world->vehicle_testers.push_back(tester);
    return static_cast<uint32_t>(world->vehicles.size() - 1);
}

void fj_vehicle_drive(FjWorld *world, uint32_t vehicle, float forward, float right, float brake,
                      float handbrake) {
    JPH::VehicleConstraint &v = *world->vehicles[vehicle];
    static_cast<JPH::WheeledVehicleController *>(v.GetController())
        ->SetDriverInput(forward, right, brake, handbrake);
    // A parked car sleeps; a driver's input wakes it.
    if (forward != 0.0f || right != 0.0f || brake != 0.0f || handbrake != 0.0f) {
        world->system.GetBodyInterface().ActivateBody(v.GetVehicleBody()->GetID());
    }
}

void fj_vehicle_wheels(const FjWorld *world, uint32_t vehicle, double *positions,
                       float *rotations) {
    const JPH::VehicleConstraint &v = *world->vehicles[vehicle];
    for (uint32_t w = 0; w < 4; ++w) {
        const JPH::RMat44 m = v.GetWheelWorldTransform(w, JPH::Vec3::sAxisX(), JPH::Vec3::sAxisY());
        const JPH::RVec3 p = m.GetTranslation();
        positions[3 * w] = p.GetX();
        positions[3 * w + 1] = p.GetY();
        positions[3 * w + 2] = p.GetZ();
        const JPH::Quat q = m.GetQuaternion();
        rotations[4 * w] = q.GetX();
        rotations[4 * w + 1] = q.GetY();
        rotations[4 * w + 2] = q.GetZ();
        rotations[4 * w + 3] = q.GetW();
    }
}

void fj_vehicle_engine(const FjWorld *world, uint32_t vehicle, float *rpm, int32_t *gear) {
    const auto *c = static_cast<const JPH::WheeledVehicleController *>(
        world->vehicles[vehicle]->GetController());
    *rpm = c->GetEngine().GetCurrentRPM();
    *gear = c->GetTransmission().GetCurrentGear();
}

uint32_t fj_joint_fixed(FjWorld *world, uint32_t a, uint32_t b, uint32_t velocity_steps,
                        uint32_t position_steps) {
    JPH::FixedConstraintSettings settings;
    settings.mAutoDetectPoint = true;
    settings.mNumVelocityStepsOverride = velocity_steps;
    settings.mNumPositionStepsOverride = position_steps;
    return add_joint(world, settings, a, b);
}

uint32_t fj_joint_distance(FjWorld *world, uint32_t a, uint32_t b, const double point_a[3],
                           const double point_b[3], float min, float max) {
    JPH::DistanceConstraintSettings settings;
    settings.mPoint1 = rvec3(point_a);
    settings.mPoint2 = rvec3(point_b);
    settings.mMinDistance = min;
    settings.mMaxDistance = max;
    return add_joint(world, settings, a, b);
}

void fj_joints_load(const FjWorld *world, const uint32_t *joints, uint32_t count, float *loads) {
    for (uint32_t i = 0; i < count; ++i) {
        const JPH::TwoBodyConstraint *c = world->joints[joints[i]];
        float position = 0.0f;
        float rotation = 0.0f;
        if (c->GetSubType() == JPH::EConstraintSubType::Fixed) {
            const auto *f = static_cast<const JPH::FixedConstraint *>(c);
            position = f->GetTotalLambdaPosition().Length();
            rotation = f->GetTotalLambdaRotation().Length();
        } else if (c->GetSubType() == JPH::EConstraintSubType::Distance) {
            position = std::abs(static_cast<const JPH::DistanceConstraint *>(c)->GetTotalLambdaPosition());
        }
        loads[2 * i] = position;
        loads[2 * i + 1] = rotation;
    }
}

void fj_joints_set(FjWorld *world, const uint32_t *joints, uint32_t count,
                   const uint8_t *holding) {
    JPH::BodyInterface &bodies = world->system.GetBodyInterface();
    for (uint32_t i = 0; i < count; ++i) {
        JPH::TwoBodyConstraint *c = world->joints[joints[i]];
        c->SetEnabled(holding[i] != 0);
        bodies.ActivateConstraint(c);
    }
}

void fj_joints_holding(const FjWorld *world, const uint32_t *joints, uint32_t count,
                       uint8_t *holding) {
    for (uint32_t i = 0; i < count; ++i) {
        holding[i] = world->joints[joints[i]]->GetEnabled() ? 1 : 0;
    }
}

uint32_t fj_ragdoll_add(FjWorld *world, const FjRagdollPart *parts, uint32_t count,
                        uint32_t *bodies) {
    JPH::Ref<JPH::Skeleton> skeleton = new JPH::Skeleton;
    JPH::Ref<JPH::RagdollSettings> settings = new JPH::RagdollSettings;
    settings->mSkeleton = skeleton;
    settings->mParts.resize(count);
    for (uint32_t i = 0; i < count; ++i) {
        const FjRagdollPart &p = parts[i];
        char name[16];
        std::snprintf(name, sizeof name, "part%u", i);
        skeleton->AddJoint(name, static_cast<int>(p.parent));
        JPH::RagdollSettings::Part &part = settings->mParts[i];
        part.SetShape(shape_of(p.shape));
        part.mPosition = rvec3(p.position);
        part.mRotation = quat(p.rotation);
        part.mMotionType = JPH::EMotionType::Dynamic;
        part.mObjectLayer = kMoving;
        part.mFriction = p.friction;
        part.mAllowSleeping = true;
        if (p.parent < 0) {
            continue;
        }
        const JPH::RVec3 pivot = rvec3(p.pivot);
        const JPH::Vec3 twist = vec3(p.twist_axis);
        const JPH::Vec3 plane = vec3(p.plane_axis);
        if (p.kind == 1) {
            auto *hinge = new JPH::HingeConstraintSettings;
            hinge->mPoint1 = hinge->mPoint2 = pivot;
            hinge->mHingeAxis1 = hinge->mHingeAxis2 = plane;
            hinge->mNormalAxis1 = hinge->mNormalAxis2 = twist;
            hinge->mLimitsMin = p.twist_min;
            hinge->mLimitsMax = p.twist_max;
            part.mToParent = hinge;
        } else {
            auto *joint = new JPH::SwingTwistConstraintSettings;
            joint->mPosition1 = joint->mPosition2 = pivot;
            joint->mTwistAxis1 = joint->mTwistAxis2 = twist;
            joint->mPlaneAxis1 = joint->mPlaneAxis2 = plane;
            joint->mNormalHalfConeAngle = p.normal_cone;
            joint->mPlaneHalfConeAngle = p.plane_cone;
            joint->mTwistMinAngle = p.twist_min;
            joint->mTwistMaxAngle = p.twist_max;
            part.mToParent = joint;
        }
    }
    settings->Stabilize();
    settings->DisableParentChildCollisions();
    settings->CalculateBodyIndexToConstraintIndex();
    settings->CalculateConstraintIndexToBodyIdxPair();
    const auto group = static_cast<JPH::CollisionGroup::GroupID>(world->ragdolls.size());
    JPH::Ref<JPH::Ragdoll> ragdoll = settings->CreateRagdoll(group, 0, &world->system);
    if (ragdoll == nullptr) {
        return UINT32_MAX;
    }
    ragdoll->AddToPhysicsSystem(JPH::EActivation::Activate);
    for (uint32_t i = 0; i < count; ++i) {
        bodies[i] = ragdoll->GetBodyID(static_cast<int>(i)).GetIndexAndSequenceNumber();
    }
    world->ragdolls.push_back(ragdoll);
    world->ragdoll_settings.push_back(settings);
    return static_cast<uint32_t>(world->ragdolls.size() - 1);
}

void fj_ragdoll_drive(FjWorld *world, uint32_t ragdoll, const float *targets, float stiffness,
                      float damping, float torque) {
    JPH::Ragdoll &r = *world->ragdolls[ragdoll];
    const JPH::RagdollSettings &s = *world->ragdoll_settings[ragdoll];
    const JPH::EMotorState state =
        torque > 0.0f ? JPH::EMotorState::Position : JPH::EMotorState::Off;
    bool woken = false;
    for (int c = 0; c < static_cast<int>(r.GetConstraintCount()); ++c) {
        const int part = s.GetBodyIndicesForConstraintIndex(c).second;
        const float *t = targets + 4 * part;
        JPH::TwoBodyConstraint *constraint = r.GetConstraint(c);
        if (constraint->GetSubType() == JPH::EConstraintSubType::Hinge) {
            auto *hinge = static_cast<JPH::HingeConstraint *>(constraint);
            JPH::MotorSettings &m = hinge->GetMotorSettings();
            m.mSpringSettings.mMode = JPH::ESpringMode::StiffnessAndDamping;
            m.mSpringSettings.mStiffness = stiffness;
            m.mSpringSettings.mDamping = damping;
            m.SetTorqueLimit(torque);
            woken |= hinge->GetMotorState() != state;
            hinge->SetMotorState(state);
            hinge->SetTargetAngle(t[0]);
        } else {
            auto *joint = static_cast<JPH::SwingTwistConstraint *>(constraint);
            for (JPH::MotorSettings *m :
                 {&joint->GetSwingMotorSettings(), &joint->GetTwistMotorSettings()}) {
                m->mSpringSettings.mMode = JPH::ESpringMode::StiffnessAndDamping;
                m->mSpringSettings.mStiffness = stiffness;
                m->mSpringSettings.mDamping = damping;
                m->SetTorqueLimit(torque);
            }
            woken |= joint->GetSwingMotorState() != state;
            joint->SetSwingMotorState(state);
            joint->SetTwistMotorState(state);
            joint->SetTargetOrientationCS(JPH::Quat(t[0], t[1], t[2], t[3]).Normalized());
        }
    }
    // A change of the motors' state wakes the ragdoll (a limp one falls), and a powered one is
    // kept awake, its targets moving it: asleep on their stands through an idle, the lab's
    // mannequins never took up their walk (#167). A limp one may sleep.
    if (woken || state == JPH::EMotorState::Position) {
        r.Activate();
    }
}

const uint8_t *fj_world_save_state(FjWorld *world, size_t *size) {
    world->saved.data.clear();
    world->saved.cursor = 0;
    world->saved.failed = false;
    world->system.SaveState(world->saved);
    // The characters after the bodies, in the order added (the system does not hold them).
    for (const JPH::Ref<JPH::CharacterVirtual> &c : world->characters) {
        c->SaveState(world->saved);
    }
    *size = world->saved.data.size();
    return world->saved.data.data();
}

int32_t fj_world_restore_state(FjWorld *world, const uint8_t *data, size_t size) {
    Bytes state;
    state.data.assign(data, data + size);
    if (!world->system.RestoreState(state)) {
        return 0;
    }
    for (const JPH::Ref<JPH::CharacterVirtual> &c : world->characters) {
        c->RestoreState(state);
    }
    return state.failed ? 0 : 1;
}

} // extern "C"
