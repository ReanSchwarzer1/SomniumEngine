//! A rigid body a script can read and write.
//!
//! # Why `applyForce` was not enough
//!
//! Phase 16 gave scripts one way to touch physics: queue a force. That is
//! right for a push, a explosion or thrust, and it is the wrong primitive
//! for a character. A walking character sets its velocity outright —
//! that is what makes it stop instantly when you release the key instead
//! of skating, and what makes running feel like running rather than like
//! accelerating. Expressing that through forces means fighting the
//! integrator with a PD controller, and it never feels right.
//!
//! So the body becomes a **component with script-visible fields**.
//! `velocity` is read and written through the ordinary mirror, which
//! means a character controller is a normal script with no special engine
//! support, and the Details panel shows it for free because it is in the
//! same registry as everything else.
//!
//! # The sync, and why it brackets the script phase
//!
//! ```text
//! read Jolt  →  component   (before scripts run)
//!        scripts read and write component.velocity
//! component  →  write Jolt   (after commands are applied)
//!        physics.step
//! ```
//!
//! Reading first is what lets a script see what physics did to it — the
//! velocity it actually has after a collision, not the one it asked for.
//! Writing after the command apply is what makes a script's write the
//! last word before integration.
//!
//! # Grounded is a heuristic, and says so
//!
//! Jolt's shape cast is not exposed through `somnium_physics`, so there
//! is no honest "is there floor under me" query available. `grounded` is
//! therefore derived from vertical speed: a body resting on something has
//! its gravity cancelled by the contact and sits at roughly zero. The
//! known false positive is the apex of a jump, where vertical speed also
//! passes through zero — which is why the shipped controller carries a
//! short cooldown after jumping rather than trusting this flag alone.
//!
//! Replacing it with a real cast is a `somnium_physics` job, and the
//! field's meaning does not change when that happens.

use somnium_ecs::component_schema;
use somnium_ecs::reflect::{ComponentSchema, TypeRegistry};
use somnium_ecs::{Component, Entity, World};
use somnium_physics::body::BodyId;
use somnium_physics::world::PhysicsWorld;

/// Vertical speed under which a body counts as standing on something.
///
/// Generous rather than tight: Jolt leaves a resting body with a small
/// non-zero velocity from contact resolution, and a threshold that only
/// accepted exact zero would report a standing character as airborne.
const GROUNDED_SPEED: f32 = 0.35;

/// A Jolt body, as a script sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBodyComponent {
    /// Jolt's body index.
    ///
    /// Process-local, so it is neither saved nor script-writable. A script
    /// that could set this could point one entity's controls at another
    /// entity's body.
    pub body: u32,
    /// Linear velocity in metres per second. **The script-facing control
    /// surface**: read what physics did, write what you want next.
    pub velocity: glam::Vec3,
    /// Whether the body appears to be resting on something. See the module
    /// docs — this is a vertical-speed heuristic, not a cast.
    pub grounded: bool,
    /// Whether the engine writes `velocity` back to Jolt.
    ///
    /// Off for a body that physics owns entirely — a dropped crate, the
    /// boat — so that reading its velocity from a script does not also
    /// mean overwriting it every step.
    pub script_driven: bool,
}

impl Component for RigidBodyComponent {}

impl Default for RigidBodyComponent {
    fn default() -> Self {
        Self {
            body: BodyId::INVALID.index(),
            velocity: glam::Vec3::ZERO,
            grounded: false,
            script_driven: true,
        }
    }
}

impl RigidBodyComponent {
    /// Wrap a Jolt body for script control.
    #[must_use]
    pub fn driven(body: BodyId) -> Self {
        Self {
            body: body.index(),
            ..Self::default()
        }
    }

    /// Wrap a Jolt body for observation only.
    #[must_use]
    pub fn observed(body: BodyId) -> Self {
        Self {
            body: body.index(),
            script_driven: false,
            ..Self::default()
        }
    }

    /// The Jolt handle, if this refers to a live body.
    #[must_use]
    pub fn body_id(&self) -> Option<BodyId> {
        (self.body != BodyId::INVALID.index()).then(|| BodyId::from_index(self.body))
    }
}

/// The schema. `velocity` is the only script-writable field on purpose —
/// everything else is either engine-owned or authored once.
pub(crate) fn rigid_body_schema() -> ComponentSchema {
    component_schema! {
        RigidBodyComponent as "somnium.RigidBody", display "Rigid Body", version 1,
        fields {
            // Process-local. Not saved: a body index from the last run
            // names a different body, or nothing, in this one.
            body { flags: FieldFlags::SCRIPT_READ },
            velocity,
            grounded { flags: FieldFlags::SERIALIZE.union(FieldFlags::SCRIPT_READ) },
            script_driven,
        }
    }
}

/// Copy Jolt's state into the components, before scripts run.
///
/// Also writes the body's position and rotation onto the entity's
/// `Transform`, so a script reading `ctx.self.transform` sees where
/// physics actually put it rather than where it was last frame.
pub fn read_physics_into_world(world: &mut World, physics: &PhysicsWorld) {
    let bodies: Vec<(Entity, RigidBodyComponent)> = world
        .entities()
        .filter_map(|entity| Some((entity, *world.get::<RigidBodyComponent>(entity)?)))
        .collect();

    for (entity, mut component) in bodies {
        let Some(body) = component.body_id() else {
            continue;
        };
        let velocity = physics.get_linear_velocity(body);
        component.velocity = velocity;
        component.grounded = velocity.y.abs() < GROUNDED_SPEED;
        if let Some(slot) = world.get_mut::<RigidBodyComponent>(entity) {
            *slot = component;
        }

        let position = physics.get_position(body);
        let rotation = physics.get_rotation(body);
        if let Some(transform) = world.get_mut::<crate::Transform>(entity) {
            transform.translation = position;
            // Only bodies physics owns get their rotation back. A
            // character turns by writing its own transform — handing it
            // Jolt's rotation would fight the script every step, since an
            // upright capsule's rotation is whatever the solver last left.
            if !component.script_driven {
                transform.rotation = rotation;
            }
        }
    }
}

/// Push script-written velocities back into Jolt, after the command
/// apply and before the step.
pub fn write_world_into_physics(world: &World, physics: &mut PhysicsWorld) {
    let driven: Vec<(BodyId, glam::Vec3)> = world
        .entities()
        .filter_map(|entity| {
            let component = world.get::<RigidBodyComponent>(entity)?;
            component
                .script_driven
                .then(|| Some((component.body_id()?, component.velocity)))
                .flatten()
        })
        .collect();

    for (body, velocity) in driven {
        // A NaN here would take the body to infinity three steps later and
        // surface somewhere else entirely. The command applier already
        // refuses non-finite writes; this is the second gate, for a value
        // that arrived some other way.
        if velocity.is_finite() {
            physics.set_linear_velocity(body, velocity);
        }
        // A rotation lock, which is what an upright character needs and
        // what Jolt's own `AllowedDOFs` would give if `somnium_physics`
        // exposed it. Without this a capsule tips over on the first slope
        // and then rolls, because nothing in the solver knows it is
        // supposed to be a person. Angular damping alone only slows that
        // down; zeroing it every step stops it.
        physics.set_angular_velocity(body, glam::Vec3::ZERO);
    }
}

/// Register the component. Called by [`component_registry`].
///
/// [`component_registry`]: crate::reflect_registry::component_registry
pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(rigid_body_schema());
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::reflect::{FieldFlags, StableId};

    #[test]
    fn an_invalid_body_has_no_handle() {
        assert!(RigidBodyComponent::default().body_id().is_none());
        assert_eq!(
            RigidBodyComponent::driven(BodyId::from_index(7)).body_id(),
            Some(BodyId::from_index(7))
        );
    }

    #[test]
    fn only_velocity_is_script_writable() {
        let schema = rigid_body_schema();
        let writable: Vec<&str> = schema
            .fields
            .iter()
            .filter(|f| f.flags.contains(FieldFlags::SCRIPT_WRITE))
            .map(|f| f.name)
            .collect();
        assert_eq!(
            writable,
            vec!["velocity", "script_driven"],
            "a script must not be able to point its controls at another entity's body"
        );
    }

    #[test]
    fn the_body_index_is_never_saved() {
        let schema = rigid_body_schema();
        let body = schema.field_by_name("body").unwrap();
        assert!(
            !body.flags.contains(FieldFlags::SERIALIZE),
            "a Jolt index from the last run names a different body in this one"
        );
    }

    #[test]
    fn the_schema_is_registered_under_its_stable_id() {
        let mut registry = TypeRegistry::new();
        register(&mut registry);
        assert!(
            registry
                .by_stable_id(StableId::new("somnium.RigidBody"))
                .is_some()
        );
    }

    #[test]
    fn an_observed_body_is_not_written_back() {
        assert!(!RigidBodyComponent::observed(BodyId::from_index(1)).script_driven);
        assert!(RigidBodyComponent::driven(BodyId::from_index(1)).script_driven);
    }
}
