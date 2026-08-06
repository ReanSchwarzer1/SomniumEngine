#include "jolt_bridge.h"

#include <Jolt/Jolt.h>
#include <Jolt/RegisterTypes.h>
#include <Jolt/Core/Factory.h>
#include <Jolt/Core/TempAllocator.h>
#include <Jolt/Core/JobSystemThreadPool.h>
#include <Jolt/Physics/PhysicsSettings.h>
#include <Jolt/Physics/PhysicsSystem.h>
#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/HeightFieldShape.h>
#include <Jolt/Physics/Body/BodyCreationSettings.h>
#include <Jolt/Physics/Body/BodyActivationListener.h>

#include <thread>
#include <iostream>

using namespace JPH;

// Broadphase Layers
namespace Layers {
    static constexpr ObjectLayer NON_MOVING = 0;
    static constexpr ObjectLayer MOVING = 1;
    static constexpr ObjectLayer NUM_LAYERS = 2;
};

namespace BroadPhaseLayers {
    static constexpr BroadPhaseLayer NON_MOVING(0);
    static constexpr BroadPhaseLayer MOVING(1);
    static constexpr uint NUM_LAYERS(2);
};

// Boilerplate classes matching HelloWorld.cpp
class BPLayerInterfaceImpl final : public BroadPhaseLayerInterface {
public:
    BPLayerInterfaceImpl() {
        mObjectToBroadPhase[Layers::NON_MOVING] = BroadPhaseLayers::NON_MOVING;
        mObjectToBroadPhase[Layers::MOVING] = BroadPhaseLayers::MOVING;
    }
    virtual uint GetNumBroadPhaseLayers() const override { return BroadPhaseLayers::NUM_LAYERS; }
    virtual BroadPhaseLayer GetBroadPhaseLayer(ObjectLayer inLayer) const override { return mObjectToBroadPhase[inLayer]; }
#if defined(JPH_EXTERNAL_PROFILE) || defined(JPH_PROFILE_ENABLED)
    virtual const char *GetBroadPhaseLayerName(BroadPhaseLayer inLayer) const override {
        switch ((BroadPhaseLayer::Type)inLayer) {
            case (BroadPhaseLayer::Type)BroadPhaseLayers::NON_MOVING: return "NON_MOVING";
            case (BroadPhaseLayer::Type)BroadPhaseLayers::MOVING: return "MOVING";
            default: return "INVALID";
        }
    }
#endif
private:
    BroadPhaseLayer mObjectToBroadPhase[Layers::NUM_LAYERS];
};

class ObjectVsBroadPhaseLayerFilterImpl : public ObjectVsBroadPhaseLayerFilter {
public:
    virtual bool ShouldCollide(ObjectLayer inLayer1, BroadPhaseLayer inLayer2) const override {
        switch (inLayer1) {
            case Layers::NON_MOVING: return inLayer2 == BroadPhaseLayers::MOVING;
            case Layers::MOVING: return true;
            default: return false;
        }
    }
};

class ObjectLayerPairFilterImpl : public ObjectLayerPairFilter {
public:
    virtual bool ShouldCollide(ObjectLayer inObject1, ObjectLayer inObject2) const override {
        switch (inObject1) {
            case Layers::NON_MOVING: return inObject2 == Layers::MOVING;
            case Layers::MOVING: return true;
            default: return false;
        }
    }
};

// A struct to hold the system and its allocators
struct PhysicsContext {
    PhysicsSystem* system;
    TempAllocatorImpl* temp_allocator;
    JobSystemThreadPool* job_system;
    BPLayerInterfaceImpl bp_layer_interface;
    ObjectVsBroadPhaseLayerFilterImpl obj_vs_bp_filter;
    ObjectLayerPairFilterImpl obj_vs_obj_filter;
};

extern "C" {

void jph_init(void) {
    RegisterDefaultAllocator();
    Factory::sInstance = new Factory();
    RegisterTypes();
}

void jph_shutdown(void) {
    UnregisterTypes();
    delete Factory::sInstance;
    Factory::sInstance = nullptr;
}

void* jph_physics_system_create(uint32_t max_bodies, uint32_t max_body_pairs, uint32_t max_contact_constraints) {
    PhysicsContext* ctx = new PhysicsContext();
    
    // Allocate 10MB temp memory
    ctx->temp_allocator = new TempAllocatorImpl(10 * 1024 * 1024);
    
    // Allocate job system
    ctx->job_system = new JobSystemThreadPool(cMaxPhysicsJobs, cMaxPhysicsBarriers, std::thread::hardware_concurrency() - 1);
    
    ctx->system = new PhysicsSystem();
    ctx->system->Init(
        max_bodies, 0, max_body_pairs, max_contact_constraints, 
        ctx->bp_layer_interface, ctx->obj_vs_bp_filter, ctx->obj_vs_obj_filter
    );
    
    return ctx;
}

void jph_physics_system_destroy(void* system_ptr) {
    if (!system_ptr) return;
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    delete ctx->system;
    delete ctx->job_system;
    delete ctx->temp_allocator;
    delete ctx;
}

int jph_physics_system_update(void* system_ptr, float dt, int collision_steps) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    ctx->system->Update(dt, collision_steps, ctx->temp_allocator, ctx->job_system);
    return 0; // Return 0 for success
}

void jph_physics_system_set_gravity(void* system_ptr, float x, float y, float z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    ctx->system->SetGravity(Vec3(x, y, z));
}

void jph_physics_system_optimize_broad_phase(void* system_ptr) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    ctx->system->OptimizeBroadPhase();
}

uint32_t jph_physics_system_get_num_bodies(void* system_ptr) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    return ctx->system->GetNumBodies();
}

// Shapes
void* jph_box_shape_create(float hx, float hy, float hz) {
    BoxShapeSettings settings(Vec3(hx, hy, hz));
    ShapeSettings::ShapeResult result = settings.Create();
    if (result.HasError()) return nullptr;
    ShapeRefC shape = result.Get();
    shape->AddRef(); // Manually retain so we can pass the raw pointer across FFI
    return const_cast<Shape*>(shape.GetPtr());
}

void* jph_sphere_shape_create(float radius) {
    SphereShapeSettings settings(radius);
    ShapeSettings::ShapeResult result = settings.Create();
    if (result.HasError()) return nullptr;
    ShapeRefC shape = result.Get();
    shape->AddRef();
    return const_cast<Shape*>(shape.GetPtr());
}

void* jph_capsule_shape_create(float half_height, float radius) {
    CapsuleShapeSettings settings(half_height, radius);
    ShapeSettings::ShapeResult result = settings.Create();
    if (result.HasError()) return nullptr;
    ShapeRefC shape = result.Get();
    shape->AddRef();
    return const_cast<Shape*>(shape.GetPtr());
}

// Phase 17B: terrain collider.
//
// `samples` is a row-major sample_count x sample_count grid of heights in world
// units, X varying fastest. Jolt maps sample (x, z) to
// offset + scale * (x, height, z), so the caller passes the sample spacing as
// scale.x/scale.z and leaves scale.y at 1.
//
// Jolt requires sample_count to be a power of two of at least 2; the caller
// picks it with `terrain::collider::sample_count_for`. Anything else is
// rejected here rather than tripping an assert deep inside the shape build.
void* jph_heightfield_shape_create(const float* samples, uint32_t sample_count,
                                   float offset_x, float offset_y, float offset_z,
                                   float scale_x, float scale_y, float scale_z) {
    if (samples == nullptr) return nullptr;
    if (sample_count < 2) return nullptr;
    if ((sample_count & (sample_count - 1)) != 0) return nullptr;

    HeightFieldShapeSettings settings(
        samples,
        Vec3(offset_x, offset_y, offset_z),
        Vec3(scale_x, scale_y, scale_z),
        sample_count);
    ShapeSettings::ShapeResult result = settings.Create();
    if (result.HasError()) return nullptr;
    ShapeRefC shape = result.Get();
    shape->AddRef();
    return const_cast<Shape*>(shape.GetPtr());
}

void jph_shape_destroy(void* shape_ptr) {
    if (!shape_ptr) return;
    Shape* shape = (Shape*)shape_ptr;
    shape->Release();
}

// Body interface
uint32_t jph_body_interface_create_and_add_body(void* system_ptr, const JphBodyCreationSettings* settings, int activation) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyInterface& bi = ctx->system->GetBodyInterface();
    
    Shape* shape = (Shape*)settings->shape;
    Vec3 pos(settings->position[0], settings->position[1], settings->position[2]);
    Quat rot(settings->rotation[0], settings->rotation[1], settings->rotation[2], settings->rotation[3]);
    
    EMotionType motion_type;
    switch(settings->motion_type) {
        case 0: motion_type = EMotionType::Static; break;
        case 1: motion_type = EMotionType::Kinematic; break;
        case 2: motion_type = EMotionType::Dynamic; break;
        default: motion_type = EMotionType::Static; break;
    }
    
    BodyCreationSettings bcs(shape, pos, rot, motion_type, settings->object_layer);
    bcs.mLinearVelocity = Vec3(settings->linear_velocity[0], settings->linear_velocity[1], settings->linear_velocity[2]);
    bcs.mFriction = settings->friction;
    bcs.mRestitution = settings->restitution;
    bcs.mLinearDamping = settings->linear_damping;
    bcs.mAngularDamping = settings->angular_damping;
    bcs.mGravityFactor = settings->gravity_factor;
    bcs.mAllowSleeping = settings->allow_sleeping != 0;
    
    BodyID body_id = bi.CreateAndAddBody(bcs, activation ? EActivation::Activate : EActivation::DontActivate);
    return body_id.GetIndexAndSequenceNumber();
}

void jph_body_interface_remove_body(void* system_ptr, uint32_t body_id) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().RemoveBody(id);
}

void jph_body_interface_destroy_body(void* system_ptr, uint32_t body_id) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().DestroyBody(id);
}

int jph_body_interface_is_active(void* system_ptr, uint32_t body_id) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    return ctx->system->GetBodyInterface().IsActive(id) ? 1 : 0;
}

void jph_body_interface_get_position(void* system_ptr, uint32_t body_id, float* out_x, float* out_y, float* out_z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    Vec3 pos = ctx->system->GetBodyInterface().GetPosition(id);
    *out_x = pos.GetX();
    *out_y = pos.GetY();
    *out_z = pos.GetZ();
}

void jph_body_interface_set_position(void* system_ptr, uint32_t body_id, float x, float y, float z, int activation) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().SetPosition(id, Vec3(x, y, z), activation ? EActivation::Activate : EActivation::DontActivate);
}

void jph_body_interface_get_rotation(void* system_ptr, uint32_t body_id, float* out_x, float* out_y, float* out_z, float* out_w) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    Quat rot = ctx->system->GetBodyInterface().GetRotation(id);
    *out_x = rot.GetX();
    *out_y = rot.GetY();
    *out_z = rot.GetZ();
    *out_w = rot.GetW();
}

void jph_body_interface_get_linear_velocity(void* system_ptr, uint32_t body_id, float* out_x, float* out_y, float* out_z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    Vec3 vel = ctx->system->GetBodyInterface().GetLinearVelocity(id);
    *out_x = vel.GetX();
    *out_y = vel.GetY();
    *out_z = vel.GetZ();
}

void jph_body_interface_set_linear_velocity(void* system_ptr, uint32_t body_id, float x, float y, float z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().SetLinearVelocity(id, Vec3(x, y, z));
}

void jph_body_interface_add_force(void* system_ptr, uint32_t body_id, float x, float y, float z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().AddForce(id, Vec3(x, y, z));
}

void jph_body_interface_add_impulse(void* system_ptr, uint32_t body_id, float x, float y, float z) {
    PhysicsContext* ctx = (PhysicsContext*)system_ptr;
    BodyID id(body_id);
    ctx->system->GetBodyInterface().AddImpulse(id, Vec3(x, y, z));
}

} // extern "C"
