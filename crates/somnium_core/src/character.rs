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
//! inferred instead — but from the right quantity.
//!
//! The first version of this compared vertical *speed* against a small
//! threshold: `velocity.y.abs() < 0.35`. That is only true of a body
//! standing on **flat** ground. A body walking a slope is constrained to
//! the surface and therefore has a perfectly legitimate vertical speed of
//! `horizontal_speed * tan(slope)` — 0.39 m/s on a five-degree rise at
//! walking pace, past the threshold. Every character on every hill in the
//! engine read as airborne, which cost the shipped controller its
//! footsteps and its jump. The probe that found it is
//! `slopes_do_not_read_as_airborne` below.
//!
//! What actually separates standing from falling is not speed but
//! **acceleration**: a contact cancels gravity, so a supported body's
//! vertical speed barely changes across a step, whatever that speed is,
//! while a falling one loses `g * dt` every step without fail. Comparing
//! the change against half a step of gravity separates the two cleanly at
//! any slope, and — unlike the old test — it does *not* fire at the apex
//! of a jump, where speed passes through zero but gravity is still fully
//! in effect.
//!
//! Two details make it survive contact with real ground:
//!
//! * The comparison is against the velocity **handed to Jolt**, not the
//!   one read at the start of the step, so a script writing a jump is not
//!   mistaken for a step of free fall.
//! * Support is allowed to lapse for [`COYOTE_STEPS`] before `grounded`
//!   goes false. Walking a heightfield leaves the ground for a step at a
//!   time over every triangle edge, and a flag that flickered would be a
//!   flag no gameplay code could use.
//!
//! Replacing this with a real cast is still a `somnium_physics` job, and
//! the field's meaning does not change when that happens.

use somnium_ecs::component_schema;
use somnium_ecs::reflect::{ComponentSchema, TypeRegistry};
use somnium_ecs::{Component, Entity, World};
use somnium_physics::body::BodyId;
use somnium_physics::world::PhysicsWorld;

/// How much of a step's gravity may survive before a body counts as
/// falling, as a fraction of `g * dt`.
///
/// A supported body keeps none of it and a falling one keeps all of it, so
/// anything strictly between zero and one separates them. Half leaves the
/// widest margin on both sides, which is what absorbs the small velocity
/// noise Jolt's contact resolution leaves behind.
const FALLING_FRACTION: f32 = 0.5;

/// Fixed steps a body keeps `grounded` after support disappears.
///
/// Coyote time, and also the fix for a real artefact: a capsule walking a
/// heightfield is momentarily unsupported over every triangle edge it
/// crosses. Four steps is 67 ms at the engine's fixed rate — long enough
/// to bridge those gaps, far too short to bridge a jump.
const COYOTE_STEPS: u8 = 4;

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
    /// docs — this is inferred from cancelled gravity, not a cast.
    pub grounded: bool,
    /// Whether the engine writes `velocity` back to Jolt.
    ///
    /// Off for a body that physics owns entirely — a dropped crate, the
    /// boat — so that reading its velocity from a script does not also
    /// mean overwriting it every step.
    pub script_driven: bool,
    /// The vertical speed handed to Jolt at the end of the previous step.
    ///
    /// Private engine bookkeeping for the `grounded` test, and the reason
    /// that test survives a jump: the comparison is against what the step
    /// was *given*, so a script writing an upward velocity is not read as
    /// a step of free fall. Neither saved nor reflected — a value from the
    /// last run describes a step that did not happen in this one.
    settled_vertical_speed: f32,
    /// Consecutive fixed steps without support, saturating.
    air_steps: u8,
}

impl Component for RigidBodyComponent {}

impl Default for RigidBodyComponent {
    fn default() -> Self {
        Self {
            body: BodyId::INVALID.index(),
            velocity: glam::Vec3::ZERO,
            grounded: false,
            script_driven: true,
            settled_vertical_speed: 0.0,
            air_steps: 0,
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
///
/// `fixed_dt` is the step Jolt is about to be run at. It is an argument
/// rather than a constant because it, together with the world's gravity,
/// is the whole scale of the `grounded` test — see the module docs.
pub fn read_physics_into_world(world: &mut World, physics: &PhysicsWorld, fixed_dt: f32) {
    // One step of free fall, as a change in vertical speed: negative under
    // ordinary gravity, and clamped so a zero-gravity world reports every
    // body as supported rather than none of them.
    let falling_threshold = (physics.gravity().y * fixed_dt * FALLING_FRACTION).min(0.0);

    let bodies: Vec<(Entity, RigidBodyComponent)> = world
        .entities()
        .filter_map(|entity| Some((entity, *world.get::<RigidBodyComponent>(entity)?)))
        .collect();

    for (entity, mut component) in bodies {
        let Some(body) = component.body_id() else {
            continue;
        };
        let velocity = physics.get_linear_velocity(body);
        // How much of the step's gravity survived. A contact cancels it
        // whatever the body's speed along the surface, which is why this
        // works on a slope where comparing the speed itself does not.
        let kept = velocity.y - component.settled_vertical_speed;
        if kept >= falling_threshold {
            component.air_steps = 0;
        } else {
            component.air_steps = component.air_steps.saturating_add(1);
        }
        component.velocity = velocity;
        component.grounded = component.air_steps <= COYOTE_STEPS;
        // The default for a body nothing writes back: what Jolt has now is
        // what the coming step starts from. `write_world_into_physics`
        // overwrites this for the driven bodies it actually pushes.
        component.settled_vertical_speed = velocity.y;
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
///
/// Also records the vertical speed every body is left with, which is what
/// the next [`read_physics_into_world`] measures gravity against. Recording
/// it *here* rather than at read time is what stops a scripted jump from
/// looking like a step of free fall: the comparison is against the
/// velocity the step was given, not the one it started from.
pub fn write_world_into_physics(world: &mut World, physics: &mut PhysicsWorld) {
    let driven: Vec<(Entity, BodyId, glam::Vec3)> = world
        .entities()
        .filter_map(|entity| {
            let component = world.get::<RigidBodyComponent>(entity)?;
            component
                .script_driven
                .then(|| Some((entity, component.body_id()?, component.velocity)))
                .flatten()
        })
        .collect();

    for (entity, body, velocity) in driven {
        // A NaN here would take the body to infinity three steps later and
        // surface somewhere else entirely. The command applier already
        // refuses non-finite writes; this is the second gate, for a value
        // that arrived some other way.
        if velocity.is_finite() {
            physics.set_linear_velocity(body, velocity);
            // The step starts from what was just pushed, not from what was
            // read at the top of it. That distinction is the whole reason a
            // scripted jump does not read as a step of free fall.
            if let Some(slot) = world.get_mut::<RigidBodyComponent>(entity) {
                slot.settled_vertical_speed = velocity.y;
            }
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
