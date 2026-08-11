use glam::{Quat, Vec3};
use somnium_physics_sys::*;
use std::ffi::c_void;

use crate::{
    body::{BodyId, RigidBodyDescriptor},
    config::PhysicsConfig,
};

/// The main physics simulation world.
pub struct PhysicsWorld {
    system: *mut c_void,
}

impl PhysicsWorld {
    /// Create a new physics world.
    pub fn new(config: PhysicsConfig) -> Self {
        unsafe {
            jph_init();
            let system = jph_physics_system_create(
                config.max_bodies,
                config.max_body_pairs,
                config.max_contact_constraints,
            );

            jph_physics_system_set_gravity(
                system,
                config.gravity.x,
                config.gravity.y,
                config.gravity.z,
            );

            Self { system }
        }
    }

    /// Step the simulation forward by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        unsafe {
            jph_physics_system_update(self.system, dt, 1);
        }
    }

    /// Create and add a new rigid body to the simulation.
    pub fn create_body(&mut self, desc: RigidBodyDescriptor) -> BodyId {
        unsafe {
            let jolt_settings = desc.clone().into_jolt();

            // A shape that failed to build comes back null. Handing that to
            // Jolt trips an assert inside the body interface, so a bad
            // descriptor would take the whole simulation down instead of
            // failing locally.
            if jolt_settings.shape.is_null() {
                tracing::warn!("create_body: shape failed to build; no body created");
                return BodyId::INVALID;
            }

            let id = jph_body_interface_create_and_add_body(self.system, &jolt_settings, 1);

            // Release the shape reference since Jolt took ownership internally
            jph_shape_destroy(jolt_settings.shape);

            BodyId(id)
        }
    }

    /// Remove a body from the simulation (but don't destroy it).
    pub fn remove_body(&mut self, id: BodyId) {
        unsafe {
            jph_body_interface_remove_body(self.system, id.0);
        }
    }

    /// Destroy a body.
    pub fn destroy_body(&mut self, id: BodyId) {
        unsafe {
            jph_body_interface_destroy_body(self.system, id.0);
        }
    }

    /// Check if a body is active (not sleeping).
    pub fn is_active(&self, id: BodyId) -> bool {
        unsafe { jph_body_interface_is_active(self.system, id.0) != 0 }
    }

    /// Get the world-space position of a body.
    pub fn get_position(&self, id: BodyId) -> Vec3 {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        unsafe {
            jph_body_interface_get_position(self.system, id.0, &mut x, &mut y, &mut z);
        }
        Vec3::new(x, y, z)
    }

    /// Set the world-space position of a body.
    pub fn set_position(&mut self, id: BodyId, pos: Vec3, activate: bool) {
        unsafe {
            jph_body_interface_set_position(
                self.system,
                id.0,
                pos.x,
                pos.y,
                pos.z,
                if activate { 1 } else { 0 },
            );
        }
    }

    /// Get the world-space rotation of a body.
    pub fn get_rotation(&self, id: BodyId) -> Quat {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        let mut w = 0.0;
        unsafe {
            jph_body_interface_get_rotation(self.system, id.0, &mut x, &mut y, &mut z, &mut w);
        }
        Quat::from_xyzw(x, y, z, w)
    }

    /// Set the world-space rotation of a body.
    pub fn set_rotation(&mut self, id: BodyId, rotation: Quat, activate: bool) {
        unsafe {
            jph_body_interface_set_rotation(
                self.system,
                id.0,
                rotation.x,
                rotation.y,
                rotation.z,
                rotation.w,
                if activate { 1 } else { 0 },
            );
        }
    }

    /// Get the linear velocity of a body.
    pub fn get_linear_velocity(&self, id: BodyId) -> Vec3 {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        unsafe {
            jph_body_interface_get_linear_velocity(self.system, id.0, &mut x, &mut y, &mut z);
        }
        Vec3::new(x, y, z)
    }

    /// Set the linear velocity of a body.
    pub fn set_linear_velocity(&mut self, id: BodyId, vel: Vec3) {
        unsafe {
            jph_body_interface_set_linear_velocity(self.system, id.0, vel.x, vel.y, vel.z);
        }
    }

    /// Apply a force to the body.
    pub fn add_force(&mut self, id: BodyId, force: Vec3) {
        unsafe {
            jph_body_interface_add_force(self.system, id.0, force.x, force.y, force.z);
        }
    }

    /// Apply a force at a world-space point, producing both translation and
    /// torque. This is the primitive used by distributed buoyancy samples.
    pub fn add_force_at_position(&mut self, id: BodyId, force: Vec3, position: Vec3) {
        unsafe {
            jph_body_interface_add_force_at_position(
                self.system,
                id.0,
                force.x,
                force.y,
                force.z,
                position.x,
                position.y,
                position.z,
            );
        }
    }

    pub fn get_angular_velocity(&self, id: BodyId) -> Vec3 {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        unsafe {
            jph_body_interface_get_angular_velocity(self.system, id.0, &mut x, &mut y, &mut z);
        }
        Vec3::new(x, y, z)
    }

    pub fn set_angular_velocity(&mut self, id: BodyId, velocity: Vec3) {
        unsafe {
            jph_body_interface_set_angular_velocity(
                self.system,
                id.0,
                velocity.x,
                velocity.y,
                velocity.z,
            );
        }
    }

    /// Apply an impulse to the body.
    pub fn add_impulse(&mut self, id: BodyId, impulse: Vec3) {
        unsafe {
            jph_body_interface_add_impulse(self.system, id.0, impulse.x, impulse.y, impulse.z);
        }
    }

    /// Optimize the broad phase tree. Call this after adding a large batch of bodies.
    pub fn optimize_broad_phase(&mut self) {
        unsafe {
            jph_physics_system_optimize_broad_phase(self.system);
        }
    }

    /// Get the total number of bodies in the simulation.
    pub fn num_bodies(&self) -> u32 {
        unsafe { jph_physics_system_get_num_bodies(self.system) }
    }
}

impl Drop for PhysicsWorld {
    fn drop(&mut self) {
        unsafe {
            jph_physics_system_destroy(self.system);
            jph_shutdown();
        }
    }
}

unsafe impl Send for PhysicsWorld {}
unsafe impl Sync for PhysicsWorld {}
