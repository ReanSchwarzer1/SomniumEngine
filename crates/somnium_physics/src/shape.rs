use glam::Vec3;
use std::ffi::c_void;

/// Collision shape for a rigid body.
#[derive(Debug, Clone)]
pub enum ColliderShape {
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
    /// Phase 17B: static terrain surface.
    ///
    /// `samples` is a row-major `sample_count * sample_count` grid of heights in
    /// world units with X varying fastest; `scale` is the world spacing between
    /// samples on X/Z, with Y left at 1 because the samples are already scaled.
    HeightField {
        samples: Vec<f32>,
        sample_count: u32,
        scale: Vec3,
    },
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
                ColliderShape::HeightField { samples, sample_count, scale } => {
                    // Guard here as well as in the bridge: a mismatched length
                    // would have Jolt read past the end of the slice.
                    let expected = (*sample_count as usize).saturating_mul(*sample_count as usize);
                    if samples.len() < expected {
                        return std::ptr::null_mut();
                    }
                    somnium_physics_sys::jph_heightfield_shape_create(
                        samples.as_ptr(),
                        *sample_count,
                        0.0,
                        0.0,
                        0.0,
                        scale.x,
                        scale.y,
                        scale.z,
                    )
                }
            }
        }
    }
}
