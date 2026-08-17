use somnium_physics_sys::JphBodyCreationSettings;

use crate::shape::ColliderShape;
use glam::{Quat, Vec3};

/// Opaque body handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyId(pub(crate) u32);

impl BodyId {
    /// Returned when a body could not be created — most often because its
    /// shape failed to build. Matches Jolt's own invalid body id.
    pub const INVALID: Self = Self(u32::MAX);

    /// Whether this id refers to a real body.
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }

    /// Raw Jolt body index.
    pub fn index(self) -> u32 {
        self.0
    }

    /// Rebuild a handle from a raw index.
    ///
    /// For components that store the index rather than the handle — a
    /// body id is process-local, so anything that survives a frame stores
    /// the number and reconstitutes it here. Not validated, for the same
    /// reason Jolt does not: an index that names nothing is answered by
    /// the body interface, not by this constructor.
    pub fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Motion type for rigid bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionType {
    Static = 0,
    Kinematic = 1,
    Dynamic = 2,
}

/// Settings to create a new rigid body.
#[derive(Debug, Clone)]
pub struct RigidBodyDescriptor {
    pub shape: ColliderShape,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear_velocity: Vec3,
    pub motion_type: MotionType,
    pub object_layer: u16,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_factor: f32,
    pub allow_sleeping: bool,
}

impl Default for RigidBodyDescriptor {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere { radius: 1.0 },
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            motion_type: MotionType::Static,
            object_layer: 0,
            friction: 0.2,
            restitution: 0.0,
            linear_damping: 0.05,
            angular_damping: 0.05,
            gravity_factor: 1.0,
            allow_sleeping: true,
        }
    }
}

impl RigidBodyDescriptor {
    pub(crate) fn into_jolt(self) -> JphBodyCreationSettings {
        JphBodyCreationSettings {
            shape: self.shape.into_jolt(),
            position: self.position.into(),
            rotation: self.rotation.into(),
            linear_velocity: self.linear_velocity.into(),
            motion_type: self.motion_type as u8,
            object_layer: self.object_layer,
            friction: self.friction,
            restitution: self.restitution,
            linear_damping: self.linear_damping,
            angular_damping: self.angular_damping,
            gravity_factor: self.gravity_factor,
            allow_sleeping: if self.allow_sleeping { 1 } else { 0 },
        }
    }
}
