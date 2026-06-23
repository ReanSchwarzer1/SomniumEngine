#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::ffi::c_void;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct JphBodyCreationSettings {
    pub shape: *mut c_void,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub motion_type: u8, // 0=Static, 1=Kinematic, 2=Dynamic
    pub object_layer: u16,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_factor: f32,
    pub allow_sleeping: u8,
}

unsafe extern "C" {
    pub fn jph_init();
    pub fn jph_shutdown();

    pub fn jph_physics_system_create(max_bodies: u32, max_body_pairs: u32, max_contact_constraints: u32) -> *mut c_void;
    pub fn jph_physics_system_destroy(system: *mut c_void);
    pub fn jph_physics_system_update(system: *mut c_void, dt: f32, collision_steps: std::os::raw::c_int) -> std::os::raw::c_int;
    pub fn jph_physics_system_set_gravity(system: *mut c_void, x: f32, y: f32, z: f32);
    pub fn jph_physics_system_optimize_broad_phase(system: *mut c_void);
    pub fn jph_physics_system_get_num_bodies(system: *mut c_void) -> u32;

    pub fn jph_box_shape_create(hx: f32, hy: f32, hz: f32) -> *mut c_void;
    pub fn jph_sphere_shape_create(radius: f32) -> *mut c_void;
    pub fn jph_capsule_shape_create(half_height: f32, radius: f32) -> *mut c_void;
    pub fn jph_shape_destroy(shape: *mut c_void);

    pub fn jph_body_interface_create_and_add_body(system: *mut c_void, settings: *const JphBodyCreationSettings, activation: std::os::raw::c_int) -> u32;
    pub fn jph_body_interface_remove_body(system: *mut c_void, body_id: u32);
    pub fn jph_body_interface_destroy_body(system: *mut c_void, body_id: u32);
    pub fn jph_body_interface_is_active(system: *mut c_void, body_id: u32) -> std::os::raw::c_int;

    pub fn jph_body_interface_get_position(system: *mut c_void, body_id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32);
    pub fn jph_body_interface_set_position(system: *mut c_void, body_id: u32, x: f32, y: f32, z: f32, activation: std::os::raw::c_int);
    pub fn jph_body_interface_get_rotation(system: *mut c_void, body_id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32, out_w: *mut f32);
    pub fn jph_body_interface_get_linear_velocity(system: *mut c_void, body_id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32);
    pub fn jph_body_interface_set_linear_velocity(system: *mut c_void, body_id: u32, x: f32, y: f32, z: f32);
    pub fn jph_body_interface_add_force(system: *mut c_void, body_id: u32, x: f32, y: f32, z: f32);
    pub fn jph_body_interface_add_impulse(system: *mut c_void, body_id: u32, x: f32, y: f32, z: f32);
}
