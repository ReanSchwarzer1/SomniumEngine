#ifndef JOLT_BRIDGE_H
#define JOLT_BRIDGE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Initialization
void jph_init(void);
void jph_shutdown(void);

// PhysicsSystem
void* jph_physics_system_create(uint32_t max_bodies, uint32_t max_body_pairs, uint32_t max_contact_constraints);
void  jph_physics_system_destroy(void* system);
int   jph_physics_system_update(void* system, float dt, int collision_steps);
void  jph_physics_system_set_gravity(void* system, float x, float y, float z);
void  jph_physics_system_optimize_broad_phase(void* system);

// Shapes
void* jph_box_shape_create(float hx, float hy, float hz);
void* jph_sphere_shape_create(float radius);
void* jph_capsule_shape_create(float half_height, float radius);
void  jph_shape_destroy(void* shape);

// BodyCreationSettings
typedef struct JphBodyCreationSettings {
    void* shape;
    float position[3];
    float rotation[4];
    float linear_velocity[3];
    uint8_t motion_type; // 0=Static, 1=Kinematic, 2=Dynamic
    uint16_t object_layer;
    float friction;
    float restitution;
    float linear_damping;
    float angular_damping;
    float gravity_factor;
    uint8_t allow_sleeping;
} JphBodyCreationSettings;

// Body Interface
uint32_t jph_body_interface_create_and_add_body(void* system, const JphBodyCreationSettings* settings, int activation);
void     jph_body_interface_remove_body(void* system, uint32_t body_id);
void     jph_body_interface_destroy_body(void* system, uint32_t body_id);
int      jph_body_interface_is_active(void* system, uint32_t body_id);

// Body properties
void jph_body_interface_get_position(void* system, uint32_t body_id, float* out_x, float* out_y, float* out_z);
void jph_body_interface_set_position(void* system, uint32_t body_id, float x, float y, float z, int activation);
void jph_body_interface_get_rotation(void* system, uint32_t body_id, float* out_x, float* out_y, float* out_z, float* out_w);
void jph_body_interface_get_linear_velocity(void* system, uint32_t body_id, float* out_x, float* out_y, float* out_z);
void jph_body_interface_set_linear_velocity(void* system, uint32_t body_id, float x, float y, float z);
void jph_body_interface_add_force(void* system, uint32_t body_id, float x, float y, float z);
void jph_body_interface_add_impulse(void* system, uint32_t body_id, float x, float y, float z);

#ifdef __cplusplus
}
#endif

#endif // JOLT_BRIDGE_H
