//! Phase 16-A: the two sides of the scripting boundary.
//!
//! [`EngineWorldView`] is the read side — copy-out access to the world,
//! answering a stale handle with `None` rather than a panic.
//!
//! [`apply_commands`] is the write side. It is the **only** thing in the
//! engine that turns a script's intent into a change, and it is a plain
//! function over `&mut World` so that every rule it enforces can be
//! tested without a window, a GPU or a physics world.
//!
//! # What "validated" means here
//!
//! Every command is re-checked at apply time, not at emit time, because
//! the world has moved since the script ran:
//!
//! * the target entity is re-checked for liveness — a script legitimately
//!   holds handles across frames, and anything may have destroyed them;
//! * the component name is resolved through the registry, so an unknown
//!   one is a diagnostic rather than a silent no-op;
//! * every field is checked against its declared type and range, and
//!   against its [`FieldFlags`] — a script cannot write a field the
//!   schema marks as engine-owned;
//! * non-finite floats are rejected before they can reach physics or a
//!   scene file, where they would surface much later and somewhere else.
//!
//! A rejected command is recorded in [`ApplyOutcome::rejected`] and the
//! rest of the batch still applies. One bad write does not cost a script
//! its frame, and it never costs another script anything.
//!
//! # Why despawn is last
//!
//! Destruction is collected during the pass and executed after it. That
//! is what lets an entity despawn itself from inside its own callback,
//! and what stops a despawn earlier in the batch from turning every later
//! command that mentions the same entity into a spurious rejection.

use somnium_ecs::reflect::{FieldFlags, TypeRegistry};
use somnium_ecs::{Entity, FieldId, PersistentId, ReflectObject, StableId, World};
use somnium_script::command::{ForceMode, LogLevel, QueuedCommand, ScriptCommand, SpawnToken};
use somnium_script::order::OrderKey;
use somnium_script::snapshot::WorldView;
use somnium_script::value::ScriptValue;
use somnium_script::ScriptAssetId;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Read side
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Copy-out access to the world for a running script.
///
/// Holds borrows for the duration of one callback and hands out nothing
/// that outlives it.
pub struct EngineWorldView<'a> {
    /// The world being read.
    pub world: &'a World,
    /// Schemas used to resolve component names and field names.
    pub registry: &'a TypeRegistry,
}

impl<'a> EngineWorldView<'a> {
    /// Wrap a world and registry.
    #[must_use]
    pub fn new(world: &'a World, registry: &'a TypeRegistry) -> Self {
        Self { world, registry }
    }
}

impl WorldView for EngineWorldView<'_> {
    fn is_alive(&self, entity: Entity) -> bool {
        self.world.is_alive(entity)
    }

    fn read_component(&self, entity: Entity, component: StableId) -> Option<ReflectObject> {
        let schema = self.registry.by_stable_id(component)?;
        (schema.snapshot)(self.world, entity)
    }

    fn read_field(
        &self,
        entity: Entity,
        component: StableId,
        field: &str,
    ) -> Option<ScriptValue> {
        let schema = self.registry.by_stable_id(component)?;
        let field = schema.field_by_name(field)?;
        if !field.flags.contains(FieldFlags::SCRIPT_READ) {
            return None;
        }
        (schema.read_field)(self.world, entity, field.id)
    }

    fn persistent_id(&self, entity: Entity) -> Option<PersistentId> {
        self.world.persistent_id(entity)
    }

    fn entity_by_persistent_id(&self, id: PersistentId) -> Option<Entity> {
        self.world.entity_by_persistent_id(id)
    }

    fn components_on(&self, entity: Entity) -> Vec<StableId> {
        self.registry
            .schemas_on(self.world, entity)
            .iter()
            .map(|schema| schema.stable_id)
            .collect()
    }

    fn component_by_name(&self, name: &str) -> Option<StableId> {
        self.registry.by_name(name).map(|schema| schema.stable_id)
    }

    fn field_by_name(&self, component: StableId, field: &str) -> Option<FieldId> {
        self.registry
            .by_stable_id(component)?
            .field_by_name(field)
            .map(|f| f.id)
    }

    fn is_field_writable(&self, component: StableId, field: &str) -> bool {
        self.registry
            .by_stable_id(component)
            .and_then(|schema| schema.field_by_name(field))
            .is_some_and(|f| f.flags.contains(FieldFlags::SCRIPT_WRITE))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Write side
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Why a command did not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// The target entity is no longer alive.
    StaleEntity,
    /// No schema is registered under that component name.
    UnknownComponent,
    /// The entity does not have the component being written.
    MissingComponent,
    /// A field id is not in the component's schema.
    UnknownField,
    /// A field is engine-owned and not script-writable.
    ReadOnlyField,
    /// A value failed its type, range or finiteness check.
    InvalidValue,
    /// The attachment exceeded its per-callback command allowance.
    BudgetExceeded,
}

/// A command that was validated and refused, with enough context for a
/// diagnostic that names the script rather than the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRejection {
    /// Which attachment emitted it.
    pub order: OrderKey,
    /// Why it was refused.
    pub reason: RejectReason,
    /// Human-readable detail.
    pub detail: String,
}

/// An event a script asked to send, ready for the dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEvent {
    /// Who sent it.
    pub source: Entity,
    /// Event name.
    pub name: String,
    /// Payload.
    pub payload: ReflectObject,
}

/// Everything one apply pass produced.
///
/// Side effects that need subsystems the applier deliberately does not
/// depend on — physics and audio — come back as data for the caller to
/// route. That is what keeps this function testable headlessly.
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    /// Entities created, tagged with the token the script is holding and
    /// the attachment that asked. Fed into the next snapshot.
    pub spawned: Vec<(OrderKey, SpawnToken, Entity)>,
    /// Entities destroyed.
    pub despawned: Vec<Entity>,
    /// Forces for the caller to hand to physics.
    pub forces: Vec<(Entity, [f32; 3], ForceMode)>,
    /// Sounds for the caller to hand to audio.
    pub audio: Vec<(ScriptAssetId, f32)>,
    /// Events for the caller to dispatch.
    pub events: Vec<PendingEvent>,
    /// Log lines, attributed to their attachment.
    pub logs: Vec<(OrderKey, LogLevel, String)>,
    /// Commands that were refused.
    pub rejected: Vec<CommandRejection>,
    /// Commands that were applied, for the profiler.
    pub applied: usize,
}

/// Apply one phase's worth of commands.
///
/// `commands` must already be in apply order — that is
/// [`CommandBuffer::drain_sorted`](somnium_script::command::CommandBuffer::drain_sorted)'s
/// job, and doing it there rather than here keeps the ordering rule in
/// one place.
// One arm per command kind. The dispatch is the function; breaking it up
// would hide the fact that this is an exhaustive match over the whole
// script-to-engine vocabulary, which is exactly what a reader needs to
// see in one place.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn apply_commands(
    world: &mut World,
    registry: &TypeRegistry,
    commands: Vec<QueuedCommand>,
) -> ApplyOutcome {
    let mut outcome = ApplyOutcome::default();
    let mut pending_despawn: Vec<Entity> = Vec::new();

    for queued in commands {
        let order = queued.order;
        match queued.command {
            ScriptCommand::SetFields {
                entity,
                component,
                fields,
            } => {
                apply_set_fields(world, registry, order, entity, component, &fields, &mut outcome);
            }

            ScriptCommand::AddComponent {
                entity,
                component,
                fields,
            } => {
                apply_add_component(
                    world,
                    registry,
                    order,
                    entity,
                    component,
                    &fields,
                    &mut outcome,
                );
            }

            ScriptCommand::RemoveComponent { entity, component } => {
                let Some(schema) = registry.by_stable_id(component) else {
                    outcome.reject(order, RejectReason::UnknownComponent, component.as_str());
                    continue;
                };
                match (schema.remove)(world, entity) {
                    Ok(_) => outcome.applied += 1,
                    Err(err) => {
                        outcome.reject(order, RejectReason::StaleEntity, &err.to_string());
                    }
                }
            }

            ScriptCommand::Spawn { token, components } => {
                // Spawn empty, then attach each component through its own
                // schema. Going through `insert_default` + `apply` rather
                // than a bundle is what lets a script name components it
                // has no Rust type for.
                let entity = world.spawn((PersistentId::mint(),));
                let mut ok = true;
                for (component, fields) in &components {
                    let Some(schema) = registry.by_stable_id(*component) else {
                        outcome.reject(order, RejectReason::UnknownComponent, component.as_str());
                        ok = false;
                        continue;
                    };
                    if (schema.insert_default)(world, entity).is_err() {
                        outcome.reject(order, RejectReason::StaleEntity, component.as_str());
                        ok = false;
                        continue;
                    }
                    apply_set_fields(
                        world, registry, order, entity, *component, fields, &mut outcome,
                    );
                }
                if ok {
                    outcome.applied += 1;
                }
                outcome.spawned.push((order, token, entity));
            }

            ScriptCommand::Despawn { entity } => {
                // Deferred to the end of the pass: see the module docs.
                if world.is_alive(entity) {
                    if !pending_despawn.contains(&entity) {
                        pending_despawn.push(entity);
                    }
                    outcome.applied += 1;
                } else {
                    outcome.reject(order, RejectReason::StaleEntity, "despawn");
                }
            }

            ScriptCommand::ApplyForce {
                entity,
                force,
                mode,
            } => {
                if !world.is_alive(entity) {
                    outcome.reject(order, RejectReason::StaleEntity, "apply_force");
                } else if force.iter().any(|f| !f.is_finite()) {
                    // A NaN force is the classic way to make a physics
                    // body vanish to infinity three seconds later.
                    outcome.reject(order, RejectReason::InvalidValue, "force is not finite");
                } else {
                    outcome.forces.push((entity, force, mode));
                    outcome.applied += 1;
                }
            }

            ScriptCommand::PlayAudio { asset, volume } => {
                if !volume.is_finite() || volume < 0.0 {
                    outcome.reject(order, RejectReason::InvalidValue, "volume");
                } else {
                    outcome.audio.push((asset, volume));
                    outcome.applied += 1;
                }
            }

            ScriptCommand::EmitEvent { name, payload } => {
                let source = world
                    .entity_by_persistent_id(order.entity)
                    .unwrap_or(Entity::DANGLING);
                outcome.events.push(PendingEvent {
                    source,
                    name,
                    payload,
                });
                outcome.applied += 1;
            }

            ScriptCommand::Log { level, message } => {
                outcome.logs.push((order, level, message));
                outcome.applied += 1;
            }
        }
    }

    // The safe point.
    for entity in pending_despawn {
        if world.despawn(entity) {
            outcome.despawned.push(entity);
        }
    }

    outcome
}

/// Validate and write a record into a component the entity already has.
fn apply_set_fields(
    world: &mut World,
    registry: &TypeRegistry,
    order: OrderKey,
    entity: Entity,
    component: StableId,
    fields: &ReflectObject,
    outcome: &mut ApplyOutcome,
) {
    if !world.is_alive(entity) {
        outcome.reject(order, RejectReason::StaleEntity, component.as_str());
        return;
    }
    let Some(schema) = registry.by_stable_id(component) else {
        outcome.reject(order, RejectReason::UnknownComponent, component.as_str());
        return;
    };

    // Check the whole record before writing any of it, so a batch that is
    // wrong halfway through leaves the component as it was.
    for (id, value) in fields {
        let Some(field) = schema.field(*id) else {
            outcome.reject(order, RejectReason::UnknownField, &format!("#{}", id.0));
            return;
        };
        if !field.flags.contains(FieldFlags::SCRIPT_WRITE) {
            outcome.reject(order, RejectReason::ReadOnlyField, field.name);
            return;
        }
        if let Err(err) = field.validate(value) {
            outcome.reject(order, RejectReason::InvalidValue, &err.to_string());
            return;
        }
    }

    match (schema.apply)(world, entity, fields) {
        Ok(()) => outcome.applied += 1,
        Err(err) => outcome.reject(order, RejectReason::MissingComponent, &err.to_string()),
    }
}

/// Attach a component at its defaults, then write the given fields.
fn apply_add_component(
    world: &mut World,
    registry: &TypeRegistry,
    order: OrderKey,
    entity: Entity,
    component: StableId,
    fields: &ReflectObject,
    outcome: &mut ApplyOutcome,
) {
    if !world.is_alive(entity) {
        outcome.reject(order, RejectReason::StaleEntity, component.as_str());
        return;
    }
    let Some(schema) = registry.by_stable_id(component) else {
        outcome.reject(order, RejectReason::UnknownComponent, component.as_str());
        return;
    };
    if let Err(err) = (schema.insert_default)(world, entity) {
        outcome.reject(order, RejectReason::StaleEntity, &err.to_string());
        return;
    }
    outcome.applied += 1;
    if !fields.is_empty() {
        apply_set_fields(world, registry, order, entity, component, fields, outcome);
    }
}

impl ApplyOutcome {
    /// Record a refusal.
    fn reject(&mut self, order: OrderKey, reason: RejectReason, detail: &str) {
        self.rejected.push(CommandRejection {
            order,
            reason,
            detail: detail.to_owned(),
        });
    }

    /// Whether anything was refused.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.rejected.is_empty()
    }

    /// Resolve a spawn token to the entity it produced.
    #[must_use]
    pub fn resolve(&self, order: OrderKey, token: SpawnToken) -> Option<Entity> {
        self.spawned
            .iter()
            .find(|(o, t, _)| *o == order && *t == token)
            .map(|(_, _, entity)| *entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::reflect::{FieldId, ReflectValue};
    use somnium_script::command::CommandBuffer;
    use somnium_script::ids::InstanceUuid;

    use crate::reflect_registry::component_registry;
    use crate::{LightComponent, MeshComponent, Name, Transform};

    const TRANSFORM: StableId = StableId::new("somnium.Transform");
    const NAME: StableId = StableId::new("somnium.Name");
    const MESH: StableId = StableId::new("somnium.Mesh");
    const LIGHT: StableId = StableId::new("somnium.Light");

    fn order_for(world: &mut World, entity: Entity) -> OrderKey {
        let id = world.ensure_persistent_id(entity).unwrap();
        OrderKey::new(0, id, InstanceUuid::mint())
    }

    fn translation(value: [f32; 3]) -> ReflectObject {
        let mut record = ReflectObject::new();
        record.insert(FieldId(0), ReflectValue::Vec3(value));
        record
    }

    fn run(
        world: &mut World,
        registry: &TypeRegistry,
        order: OrderKey,
        commands: Vec<ScriptCommand>,
    ) -> ApplyOutcome {
        let mut buffer = CommandBuffer::new();
        buffer.begin(order);
        for command in commands {
            buffer.push(command);
        }
        buffer.end();
        apply_commands(world, registry, buffer.drain_sorted())
    }

    // ── Read side ──────────────────────────────────────────────────

    #[test]
    fn reads_copy_out_and_stale_handles_answer_none() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(), Name::new("Thing")));

        {
            let view = EngineWorldView::new(&world, &registry);
            assert!(view.is_alive(e));
            assert_eq!(
                view.read_field(e, NAME, "value"),
                Some(ScriptValue::Str("Thing".into()))
            );
            assert_eq!(view.components_on(e), vec![NAME, TRANSFORM]);
            assert!(view.read_component(e, LIGHT).is_none(), "not on this entity");
        }

        world.despawn(e);
        let view = EngineWorldView::new(&world, &registry);
        assert!(!view.is_alive(e));
        assert_eq!(view.read_component(e, TRANSFORM), None);
        assert_eq!(view.read_field(e, NAME, "value"), None);
    }

    #[test]
    fn an_unknown_component_or_field_reads_as_none_not_a_panic() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let view = EngineWorldView::new(&world, &registry);

        assert_eq!(view.read_component(e, StableId::new("mod.NotReal")), None);
        assert_eq!(view.read_field(e, TRANSFORM, "not_a_field"), None);
    }

    // ── Write side ─────────────────────────────────────────────────

    #[test]
    fn set_fields_writes_through_the_schema() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::SetFields {
                entity: e,
                component: TRANSFORM,
                fields: translation([4.0, 5.0, 6.0]),
            }],
        );

        assert!(outcome.is_clean(), "{:?}", outcome.rejected);
        assert_eq!(outcome.applied, 1);
        let after = world.get::<Transform>(e).unwrap();
        assert!((after.translation - glam::Vec3::new(4.0, 5.0, 6.0)).length() < 1.0e-6);
    }

    #[test]
    fn a_write_to_a_dead_entity_is_rejected_not_fatal() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);
        world.despawn(e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::SetFields {
                entity: e,
                component: TRANSFORM,
                fields: translation([1.0, 1.0, 1.0]),
            }],
        );
        assert_eq!(outcome.applied, 0);
        assert_eq!(outcome.rejected.len(), 1);
        assert_eq!(outcome.rejected[0].reason, RejectReason::StaleEntity);
    }

    #[test]
    fn one_rejected_command_does_not_stop_the_rest_of_the_batch() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(), Name::new("before")));
        let order = order_for(&mut world, e);

        let mut bad_name = ReflectObject::new();
        bad_name.insert(FieldId(0), ReflectValue::I64(7)); // Name wants a string

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![
                ScriptCommand::SetFields {
                    entity: e,
                    component: NAME,
                    fields: bad_name,
                },
                ScriptCommand::SetFields {
                    entity: e,
                    component: TRANSFORM,
                    fields: translation([9.0, 0.0, 0.0]),
                },
            ],
        );

        assert_eq!(outcome.rejected.len(), 1);
        assert_eq!(outcome.applied, 1);
        assert_eq!(world.get::<Name>(e).unwrap().as_str(), "before");
        assert!((world.get::<Transform>(e).unwrap().translation.x - 9.0).abs() < 1.0e-6);
    }

    #[test]
    fn a_non_finite_value_never_reaches_the_world() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::SetFields {
                entity: e,
                component: TRANSFORM,
                fields: translation([f32::NAN, 0.0, 0.0]),
            }],
        );
        assert_eq!(outcome.rejected[0].reason, RejectReason::InvalidValue);
        assert!(world.get::<Transform>(e).unwrap().translation.x.abs() < f32::EPSILON);
    }

    #[test]
    fn a_non_finite_force_never_reaches_physics() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::ApplyForce {
                entity: e,
                force: [0.0, f32::INFINITY, 0.0],
                mode: ForceMode::Impulse,
            }],
        );
        assert!(outcome.forces.is_empty());
        assert_eq!(outcome.rejected[0].reason, RejectReason::InvalidValue);
    }

    #[test]
    fn engine_owned_fields_cannot_be_written_by_a_script() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((MeshComponent::default(),));
        let order = order_for(&mut world, e);

        let mut fields = ReflectObject::new();
        fields.insert(FieldId(2), ReflectValue::I64(999)); // index_count

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::SetFields {
                entity: e,
                component: MESH,
                fields,
            }],
        );
        assert_eq!(outcome.rejected[0].reason, RejectReason::ReadOnlyField);
        assert_eq!(world.get::<MeshComponent>(e).unwrap().index_count, 0);
    }

    #[test]
    fn an_out_of_range_value_is_refused_by_the_declared_bound() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((LightComponent::directional(1000.0),));
        let order = order_for(&mut world, e);

        let schema = registry.by_stable_id(LIGHT).unwrap();
        let intensity = schema.field_by_name("intensity").unwrap();
        let mut fields = ReflectObject::new();
        fields.insert(intensity.id, ReflectValue::F64(-5.0));

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::SetFields {
                entity: e,
                component: LIGHT,
                fields,
            }],
        );
        assert_eq!(outcome.rejected[0].reason, RejectReason::InvalidValue);
        assert!((world.get::<LightComponent>(e).unwrap().intensity - 1000.0).abs() < 1.0e-3);
    }

    #[test]
    fn add_and_remove_component_go_through_archetype_migration() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::AddComponent {
                entity: e,
                component: NAME,
                fields: {
                    let mut record = ReflectObject::new();
                    record.insert(FieldId(0), ReflectValue::Str("Added".into()));
                    record
                },
            }],
        );
        assert!(outcome.is_clean(), "{:?}", outcome.rejected);
        assert_eq!(world.get::<Name>(e).unwrap().as_str(), "Added");

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::RemoveComponent {
                entity: e,
                component: NAME,
            }],
        );
        assert!(outcome.is_clean());
        assert!(world.get::<Name>(e).is_none());
        assert!(world.get::<Transform>(e).is_some());
    }

    #[test]
    fn an_unknown_component_name_is_a_diagnostic_not_a_silent_no_op() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![ScriptCommand::AddComponent {
                entity: e,
                component: StableId::new("mod.Health"),
                fields: ReflectObject::new(),
            }],
        );
        assert_eq!(outcome.rejected[0].reason, RejectReason::UnknownComponent);
        assert!(outcome.rejected[0].detail.contains("mod.Health"));
    }

    #[test]
    fn spawn_produces_a_resolvable_entity_with_its_components() {
        let registry = component_registry();
        let mut world = World::new();
        let anchor = world.spawn((Transform::default(),));
        let order = order_for(&mut world, anchor);

        let mut buffer = CommandBuffer::new();
        let token = buffer.new_spawn_token();
        buffer.begin(order);
        buffer.push(ScriptCommand::Spawn {
            token,
            components: vec![
                (TRANSFORM, translation([7.0, 0.0, 0.0])),
                (
                    NAME,
                    {
                        let mut record = ReflectObject::new();
                        record.insert(FieldId(0), ReflectValue::Str("Spawned".into()));
                        record
                    },
                ),
            ],
        });
        buffer.end();

        let outcome = apply_commands(&mut world, &registry, buffer.drain_sorted());
        assert!(outcome.is_clean(), "{:?}", outcome.rejected);

        let spawned = outcome.resolve(order, token).expect("token should resolve");
        assert!(world.is_alive(spawned));
        assert_eq!(world.get::<Name>(spawned).unwrap().as_str(), "Spawned");
        assert!((world.get::<Transform>(spawned).unwrap().translation.x - 7.0).abs() < 1.0e-6);
        assert!(
            world.persistent_id(spawned).is_some(),
            "a spawned entity is nameable across a save"
        );
    }

    #[test]
    fn despawn_happens_after_every_other_command_in_the_batch() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        // Despawn first, then write. The write must still land, because
        // destruction is deferred to the end of the pass.
        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![
                ScriptCommand::Despawn { entity: e },
                ScriptCommand::SetFields {
                    entity: e,
                    component: TRANSFORM,
                    fields: translation([3.0, 0.0, 0.0]),
                },
            ],
        );

        assert!(outcome.is_clean(), "{:?}", outcome.rejected);
        assert_eq!(outcome.despawned, vec![e]);
        assert!(!world.is_alive(e), "the entity is gone by the end of the pass");
    }

    #[test]
    fn despawning_the_same_entity_twice_destroys_it_once() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![
                ScriptCommand::Despawn { entity: e },
                ScriptCommand::Despawn { entity: e },
            ],
        );
        assert_eq!(outcome.despawned, vec![e]);
    }

    #[test]
    fn logs_and_events_are_attributed_to_their_attachment() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let order = order_for(&mut world, e);

        let outcome = run(
            &mut world,
            &registry,
            order,
            vec![
                ScriptCommand::Log {
                    level: LogLevel::Warn,
                    message: "careful".into(),
                },
                ScriptCommand::EmitEvent {
                    name: "door.opened".into(),
                    payload: ReflectObject::new(),
                },
            ],
        );

        assert_eq!(outcome.logs.len(), 1);
        assert_eq!(outcome.logs[0].0, order);
        assert_eq!(outcome.logs[0].1, LogLevel::Warn);
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].name, "door.opened");
        assert_eq!(
            outcome.events[0].source, e,
            "the event is traceable back to the emitting entity"
        );
    }

    #[test]
    fn two_attachments_writing_the_same_field_resolve_by_order_not_by_luck() {
        let registry = component_registry();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));
        let id = world.ensure_persistent_id(e).unwrap();

        let first = OrderKey::new(0, id, InstanceUuid::from_raw(1));
        let second = OrderKey::new(5, id, InstanceUuid::from_raw(2));

        let mut buffer = CommandBuffer::new();
        // Emit the *later* attachment first, to prove emission order is
        // not what decides the winner.
        buffer.begin(second);
        buffer.push(ScriptCommand::SetFields {
            entity: e,
            component: TRANSFORM,
            fields: translation([2.0, 0.0, 0.0]),
        });
        buffer.end();
        buffer.begin(first);
        buffer.push(ScriptCommand::SetFields {
            entity: e,
            component: TRANSFORM,
            fields: translation([1.0, 0.0, 0.0]),
        });
        buffer.end();

        let _ = apply_commands(&mut world, &registry, buffer.drain_sorted());
        assert!(
            (world.get::<Transform>(e).unwrap().translation.x - 2.0).abs() < 1.0e-6,
            "the higher execution_order applies last and therefore wins"
        );
    }
}
