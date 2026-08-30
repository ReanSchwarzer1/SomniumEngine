//! What a script is allowed to see.
//!
//! # Why reads are a trait and not a copy of the world
//!
//! The obvious reading of "give the script an immutable snapshot" is to
//! materialise every component it might touch before each phase. That is
//! correct and unaffordable: the budget for ten thousand component reads
//! is 1.5 ms, and copying the world to find out which ten thousand
//! mattered would spend it before a single script ran.
//!
//! So the split is:
//!
//! * **[`ScriptSnapshot`]** carries the small, always-needed things —
//!   time, input, the script's own identity and components, its pending
//!   events, and the results of last phase's spawns.
//! * **[`WorldView`]** answers everything else on demand, by *copying
//!   out* rather than by lending. A script can read any entity; it cannot
//!   hold a reference to one.
//!
//! Both halves are read-only. Every mutation goes through
//! [`CommandBuffer`](crate::command::CommandBuffer), so nothing a script
//! does can invalidate an iteration in progress.

use std::collections::BTreeMap;

use somnium_ecs::{Entity, FieldId, PersistentId, ReflectObject, StableId};

use crate::command::SpawnToken;
use crate::value::ScriptValue;

/// Clock values for one script phase.
///
/// Fixed-step callbacks see `fixed_delta` and `simulation_time` and
/// nothing else — no wall clock, no frame count — because those are the
/// only two values that are the same on a replay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSnapshot {
    /// Duration of one fixed step, in seconds.
    pub fixed_delta: f32,
    /// Duration of the current variable frame, in seconds. Meaningless in
    /// a fixed-step callback and not supplied to one.
    pub delta: f32,
    /// Simulation time, advanced only by completed fixed steps.
    pub simulation_time: f64,
    /// Number of fixed steps since the simulation started.
    pub step: u64,
}

impl Default for TimeSnapshot {
    /// A stopped 60 Hz clock. `fixed_delta` is never zero even at rest,
    /// because a script that divides by it would otherwise produce
    /// infinities the moment a caller forgot to fill this in.
    fn default() -> Self {
        Self {
            fixed_delta: 1.0 / 60.0,
            delta: 1.0 / 60.0,
            simulation_time: 0.0,
            step: 0,
        }
    }
}

/// One named input action as a fixed step sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputActionSnapshot {
    /// Value represented uniformly as two axes. Digital and 1D actions use X.
    pub value: [f32; 2],
    /// Whether the value is above the action system's activation threshold.
    pub active: bool,
    /// Whether the action became active this frame.
    pub pressed: bool,
}

/// Input state as scripts see it.
///
/// Actions are named gameplay verbs (for example `Move`, `Look`, and `Jump`),
/// not hardware controls. That keeps scripts valid when a player rebinds a
/// keyboard key or switches to a gamepad.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputSnapshot {
    /// Deterministically ordered action values, keyed by authored name.
    pub actions: BTreeMap<String, InputActionSnapshot>,
}

impl InputSnapshot {
    /// Whether a named action is active.
    #[must_use]
    pub fn action_down(&self, action: &str) -> bool {
        self.actions.get(action).is_some_and(|state| state.active)
    }

    /// Whether a named action became active this frame.
    #[must_use]
    pub fn action_pressed(&self, action: &str) -> bool {
        self.actions.get(action).is_some_and(|state| state.pressed)
    }

    /// Read a named action as one axis.
    #[must_use]
    pub fn axis(&self, action: &str) -> f32 {
        self.actions.get(action).map_or(0.0, |state| state.value[0])
    }

    /// Read a named action as a 2D vector.
    #[must_use]
    pub fn vector2(&self, action: &str) -> [f32; 2] {
        self.actions
            .get(action)
            .map_or([0.0; 2], |state| state.value)
    }
}

/// A game event delivered to a script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptEvent {
    /// Event name.
    pub name: String,
    /// Monotonically increasing across the whole simulation, so a replay
    /// can assert that the same events arrived in the same order.
    pub sequence: u64,
    /// Who emitted it, if anyone.
    pub source: Option<Entity>,
    /// Payload.
    pub payload: ReflectObject,
}

/// Everything handed to one script attachment for one phase.
#[derive(Debug, Clone)]
pub struct ScriptSnapshot {
    /// Clock.
    pub time: TimeSnapshot,
    /// Input.
    pub input: InputSnapshot,
    /// The entity this attachment is on.
    pub self_entity: Entity,
    /// That entity's durable id.
    pub self_persistent: PersistentId,
    /// The entity's own components, pre-read because a script almost
    /// always wants at least its own transform.
    pub self_components: BTreeMap<StableId, ReflectObject>,
    /// Entities created by this attachment's spawns in the previous
    /// phase, so a script can find what it asked for.
    pub spawn_results: Vec<(SpawnToken, Entity)>,
    /// Events addressed to this attachment, in sequence order.
    pub events: Vec<ScriptEvent>,
    /// Seed for this attachment's engine-owned random stream. Derived
    /// from the world seed and the attachment's durable identity, so it
    /// is the same on every replay and different for every attachment.
    pub rng_seed: u64,
}

impl ScriptSnapshot {
    /// Read one of the entity's own component records.
    #[must_use]
    pub fn own_component(&self, component: StableId) -> Option<&ReflectObject> {
        self.self_components.get(&component)
    }
}

/// Read-only access to the rest of the world.
///
/// Every method **copies out**. Nothing here returns a borrow, which is
/// what allows a backend to hold a `&dyn WorldView` across an entire
/// callback without freezing the engine's own access patterns into the
/// scripting API.
///
/// A stale entity handle is answered with `None`, never a panic: scripts
/// hold handles across frames as a matter of course, and the entity they
/// point at may have been destroyed by anything.
pub trait WorldView {
    /// Whether a handle still refers to a live entity.
    fn is_alive(&self, entity: Entity) -> bool;

    /// Copy out one component's fields.
    fn read_component(&self, entity: Entity, component: StableId) -> Option<ReflectObject>;

    /// Copy out one field, by its declared name.
    fn read_field(&self, entity: Entity, component: StableId, field: &str) -> Option<ScriptValue>;

    /// The durable id of an entity, if it has one.
    fn persistent_id(&self, entity: Entity) -> Option<PersistentId>;

    /// Resolve a durable id back to a live handle.
    fn entity_by_persistent_id(&self, id: PersistentId) -> Option<Entity>;

    /// Which components an entity has, in stable-id order.
    fn components_on(&self, entity: Entity) -> Vec<StableId>;

    // ── Name resolution ─────────────────────────────────────────────
    //
    // A script names components and fields with strings; the engine keys
    // them by [`StableId`] and [`FieldId`]. Resolution has to happen
    // somewhere, and it happens here rather than in the backend because
    // the registry that knows the answers lives with the world, not with
    // the language. A backend that had to carry its own copy of the
    // component list would be a second source of truth by another name.

    /// Resolve a component name written by a script.
    ///
    /// Returns `None` for a name no schema is registered under — which
    /// also means a script cannot cause unbounded interning by looping
    /// over misspelled names.
    fn component_by_name(&self, name: &str) -> Option<StableId>;

    /// Resolve a field name on a known component.
    fn field_by_name(&self, component: StableId, field: &str) -> Option<FieldId>;

    /// Whether a field may be written by a script.
    ///
    /// Separate from resolution because "no such field" and "that field is
    /// engine-owned" are different mistakes and deserve different
    /// messages.
    fn is_field_writable(&self, component: StableId, field: &str) -> bool;

    /// Copy out one field by id, skipping name resolution.
    ///
    /// The mirrored-property path resolves names once when an attachment
    /// is first called and then reads by id every frame, so this is the
    /// form that runs in the hot loop.
    fn read_field_id(
        &self,
        entity: Entity,
        component: StableId,
        field: FieldId,
    ) -> Option<ScriptValue>;

    /// Every script-visible field of a component: name, id, and whether a
    /// script may write it.
    ///
    /// Called once per attachment when its mirror is built, never per
    /// frame, so returning owned strings is fine here.
    fn script_fields(&self, component: StableId) -> Vec<(String, FieldId, bool)>;

    /// A field's declared type.
    ///
    /// The shape converter cannot tell a quaternion from four numbers —
    /// nothing in the value says which it is — so the boundary asks the
    /// schema and re-tags through
    /// [`FieldType::coerce`](somnium_ecs::reflect::FieldType::coerce). This
    /// is the method that makes writing a rotation from a script possible
    /// without loosening the type check for scene files.
    fn field_type(
        &self,
        component: StableId,
        field: FieldId,
    ) -> Option<somnium_ecs::reflect::FieldType>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_lookups_are_named_and_typed() {
        let input = InputSnapshot {
            actions: [(
                "Move".to_string(),
                InputActionSnapshot {
                    value: [1.0, -0.5],
                    active: true,
                    pressed: true,
                },
            )]
            .into_iter()
            .collect(),
        };
        assert!(input.action_down("Move"));
        assert!(input.action_pressed("Move"));
        assert_eq!(input.axis("Move"), 1.0);
        assert_eq!(input.vector2("Move"), [1.0, -0.5]);
    }

    #[test]
    fn an_empty_input_snapshot_reports_nothing_held() {
        let input = InputSnapshot::default();
        assert!(!input.action_down("Nothing"));
        assert!(!input.action_pressed("Nothing"));
        assert_eq!(input.vector2("Nothing"), [0.0; 2]);
    }
}
