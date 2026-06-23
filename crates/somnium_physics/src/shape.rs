use glam::Vec3;
use std::ffi::c_void;

/// Collision shape for a rigid body.
#[derive(Debug, Clone)]
pub enum ColliderShape {
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
}

impl ColliderShape {
    /// Convert to a Jolt opaque pointer.
    pub(crate) fn into_jolt(&self) -> *mut c_void {
        unsafe {
            match self {
                ColliderShape::Box { half_extents } => {
                    somnium_physics_sys::jph_box_shape_create(
                        half_extents.x,
                        half_extents.y,
                        half_extents.z,
                    )
                }
                ColliderShape::Sphere { radius } => {
                    somnium_physics_sys::jph_sphere_shape_create(*radius)
                }
                ColliderShape::Capsule { half_height, radius } => {
                    somnium_physics_sys::jph_capsule_shape_create(*half_height, *radius)
                }
            }
        }
    }
}
