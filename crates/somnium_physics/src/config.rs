use glam::Vec3;

/// Configuration for the physics simulation.
#[derive(Debug, Clone)]
pub struct PhysicsConfig {
    /// Gravity vector. Default is (0, -9.81, 0).
    pub gravity: Vec3,
    /// Maximum number of bodies in the simulation.
    pub max_bodies: u32,
    /// Maximum number of body pairs in the broadphase.
    pub max_body_pairs: u32,
    /// Maximum number of contact constraints.
    pub max_contact_constraints: u32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            max_bodies: 1024,
            max_body_pairs: 1024,
            max_contact_constraints: 1024,
        }
    }
}
