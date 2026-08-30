//! The named input actions a script sees, and the world it is allowed to
//! change while Play is running.

use std::collections::BTreeMap;

use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Entity, PersistentId, ReflectObject, StableId, World};
use somnium_input::{ActionValue, InputSystem};
use somnium_script::snapshot::{InputActionSnapshot, InputSnapshot};

/// Named action values sampled for deterministic script fixed steps.
#[derive(Debug, Default, Clone)]
pub struct ScriptInputTracker {
    snapshot: InputSnapshot,
}

impl ScriptInputTracker {
    /// A tracker with nothing held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sample every enabled action from the engine input system.
    pub fn capture(&mut self, input: &InputSystem) {
        let mut actions = BTreeMap::new();
        for action in input
            .maps()
            .iter()
            .filter(|map| map.enabled)
            .flat_map(|map| &map.actions)
        {
            let value = input.actions().value(&action.name).unwrap_or_else(|| {
                ActionValue::zero(action.kind)
            });
            let axes = value.as_vec2();
            // A render frame may complete without a fixed step. Preserve an
            // activation edge until `end_step` consumes it, or a quick tap at
            // a high render rate can disappear between script samples.
            let pending_press = self
                .snapshot
                .actions
                .get(&action.name)
                .is_some_and(|state| state.pressed);
            actions.insert(
                action.name.clone(),
                InputActionSnapshot {
                    value: axes.to_array(),
                    active: input.is_active(&action.name),
                    pressed: pending_press || input.just_activated(&action.name),
                },
            );
        }
        self.snapshot.actions = actions;
    }

    /// What a phase sees.
    #[must_use]
    pub fn snapshot(&self) -> InputSnapshot {
        self.snapshot.clone()
    }

    /// Clear the edge-triggered half. Called once per fixed step, after
    /// the phase has run.
    pub fn end_step(&mut self) {
        for action in self.snapshot.actions.values_mut() {
            action.pressed = false;
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Play / stop world separation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Every registered component of every entity, as it was when Play was
/// pressed.
///
/// # Why this and not a scene file
///
/// Round-tripping through `.somnium` would be the obvious answer and is
/// the wrong one: loading an entity dump needs GPU-side reconstruction
/// (meshes from `MeshKind`, terrain sidecars, renderer uploads), and none
/// of that is what Stop is for. Stop has to undo what *scripts* did, and
/// scripts can only touch what the [`TypeRegistry`] describes — so
/// capturing exactly that is both sufficient and free of the renderer.
///
/// Entities are keyed by [`PersistentId`], not by handle, because an
/// entity destroyed and restored gets a new index and generation.
#[derive(Debug, Default, Clone)]
pub struct WorldCheckpoint {
    entities: BTreeMap<PersistentId, Vec<(StableId, ReflectObject)>>,
}

impl WorldCheckpoint {
    /// Capture the world.
    ///
    /// Mints a [`PersistentId`] for anything that lacks one, since an
    /// entity with no durable name cannot be restored onto itself.
    #[must_use]
    pub fn capture(world: &mut World, registry: &TypeRegistry) -> Self {
        let all: Vec<Entity> = world.entities().collect();
        for entity in &all {
            let _ = world.ensure_persistent_id(*entity);
        }

        let mut entities = BTreeMap::new();
        for entity in world.entities().collect::<Vec<_>>() {
            let Some(id) = world.persistent_id(entity) else {
                continue;
            };
            let components = registry
                .schemas_on(world, entity)
                .iter()
                .filter_map(|schema| {
                    (schema.snapshot)(world, entity).map(|record| (schema.stable_id, record))
                })
                .collect();
            entities.insert(id, components);
        }
        Self { entities }
    }

    /// Put the world back.
    ///
    /// Three cases, and all three are real: an entity that survived has
    /// its fields written back; an entity a script destroyed is respawned
    /// from its record; an entity a script created is destroyed.
    pub fn restore(&self, world: &mut World, registry: &TypeRegistry) {
        // Anything with no captured record was created during play.
        let intruders: Vec<Entity> = world
            .entities()
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|entity| {
                world
                    .persistent_id(*entity)
                    .is_none_or(|id| !self.entities.contains_key(&id))
            })
            .collect();
        for entity in intruders {
            world.despawn(entity);
        }

        for (id, components) in &self.entities {
            let entity = if let Some(entity) = world.entity_by_persistent_id(*id) {
                entity
            } else {
                // Destroyed during play. Rebuild it at its defaults; the
                // captured values are written over the top below.
                let entity = world.spawn((*id,));
                for (stable, _) in components {
                    if let Some(schema) = registry.by_stable_id(*stable) {
                        let _ = (schema.insert_default)(world, entity);
                    }
                }
                entity
            };
            for (stable, record) in components {
                let Some(schema) = registry.by_stable_id(*stable) else {
                    continue;
                };
                if (schema.apply)(world, entity, record).is_err() {
                    // The component was removed during play; put it back
                    // at its defaults and write the captured values over.
                    if (schema.insert_default)(world, entity).is_ok() {
                        let _ = (schema.apply)(world, entity, record);
                    }
                }
            }
            // A component a script *added* during play is not in the
            // record, and must go.
            let extra: Vec<StableId> = registry
                .schemas_on(world, entity)
                .iter()
                .map(|schema| schema.stable_id)
                .filter(|stable| !components.iter().any(|(captured, _)| captured == stable))
                .collect();
            for stable in extra {
                if let Some(schema) = registry.by_stable_id(stable) {
                    let _ = (schema.remove)(world, entity);
                }
            }
        }
    }

    /// How many entities were captured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether nothing was captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect_registry::component_registry;
    use crate::{Name, Transform};

    #[test]
    fn scripts_receive_named_actions_instead_of_hardware_keys() {
        let mut input = InputSystem::with_default_maps();
        assert!(input.devices_mut().set_key("w", true));
        input.update(1.0 / 60.0);

        let mut tracker = ScriptInputTracker::new();
        tracker.capture(&input);
        let snapshot = tracker.snapshot();
        assert!(snapshot.action_down("Move"));
        assert!(snapshot.action_pressed("Move"));
        assert!(snapshot.vector2("Move")[1] < 0.0);

        input.update(1.0 / 240.0);
        tracker.capture(&input);
        assert!(
            tracker.snapshot().action_pressed("Move"),
            "the edge survives a render frame with no fixed step"
        );
        tracker.end_step();
        assert!(tracker.snapshot().action_down("Move"));
        assert!(!tracker.snapshot().action_pressed("Move"));
    }

    #[test]
    fn stop_puts_back_a_field_that_play_changed() {
        let registry = component_registry();
        let mut world = World::new();
        let entity = world.spawn((Transform::default(), Name::new("Original")));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);

        world.get_mut::<Transform>(entity).unwrap().translation = glam::Vec3::new(9.0, 9.0, 9.0);
        checkpoint.restore(&mut world, &registry);

        assert!(
            world.get::<Transform>(entity).unwrap().translation.length() < 1.0e-6,
            "Stop must restore the authored world exactly"
        );
    }

    #[test]
    fn stop_destroys_what_play_created_and_restores_what_it_destroyed() {
        let registry = component_registry();
        let mut world = World::new();
        let keep = world.spawn((Transform::default(), Name::new("Keep")));
        let doomed = world.spawn((
            Transform::from_translation(glam::Vec3::X),
            Name::new("Doomed"),
        ));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);
        assert_eq!(checkpoint.len(), 2);

        world.despawn(doomed);
        world.spawn((Transform::default(), Name::new("Spawned by a script")));
        assert_eq!(world.entities().count(), 2);

        checkpoint.restore(&mut world, &registry);
        assert_eq!(world.entities().count(), 2, "the intruder is gone");
        assert!(world.is_alive(keep));

        let names: Vec<String> = world
            .entities()
            .filter_map(|entity| world.get::<Name>(entity).map(|n| n.as_str().to_string()))
            .collect();
        assert!(names.contains(&"Keep".to_string()));
        assert!(
            names.contains(&"Doomed".to_string()),
            "an entity a script destroyed comes back: {names:?}"
        );
    }

    #[test]
    fn stop_removes_a_component_that_play_added() {
        let registry = component_registry();
        let mut world = World::new();
        let entity = world.spawn((Transform::default(),));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);

        world
            .insert_component(entity, Name::new("Added by a script"))
            .unwrap();
        assert!(world.get::<Name>(entity).is_some());

        checkpoint.restore(&mut world, &registry);
        assert!(
            world.get::<Name>(entity).is_none(),
            "a component a script attached during play must not survive Stop"
        );
    }
}
