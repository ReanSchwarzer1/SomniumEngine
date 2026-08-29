//! Phase 11.5E: Editor undo/redo command system.
//!
//! All mutating editor operations (inspector edits, create/delete, reparent)
//! are wrapped in an [`EditorCommand`] and routed through [`UndoStack`].
//! Ctrl+Z calls [`UndoStack::undo`]; Ctrl+Y calls [`UndoStack::redo`].
#![allow(missing_docs, clippy::wildcard_imports)]

use crate::{
    Children, LightComponent, MaterialComponent, MeshComponent, MeshKind, Name, Parent,
    TerrainComponent, Transform, UiCanvasComponent, VoxelTerrainComponent, WaterComponent,
    WorldPartitionComponent, WorldTransform,
};
use somnium_ecs::reflect::{
    ChangeScope, FieldFlags, FieldId, ReflectObject, ReflectValue, StableId,
};
use somnium_ecs::{Entity, PersistentId, World};
use somnium_ui::GestureId;

/// Assign one authored material asset to any number of entities as one
/// reversible operation. Distinct previous assignments are retained.
pub struct AssignMaterialCmd {
    entities: Vec<Entity>,
    asset: somnium_asset::database::AssetId,
    before: Vec<(Entity, MaterialComponent)>,
}

impl AssignMaterialCmd {
    #[must_use]
    pub fn new(
        world: &World,
        entities: Vec<Entity>,
        asset: somnium_asset::database::AssetId,
    ) -> Self {
        let before = entities
            .iter()
            .filter_map(|entity| {
                world
                    .get::<MaterialComponent>(*entity)
                    .copied()
                    .map(|m| (*entity, m))
            })
            .collect();
        Self {
            entities,
            asset,
            before,
        }
    }
}

impl EditorCommand for AssignMaterialCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        for entity in &self.entities {
            if let Some(material) = world.get_mut::<MaterialComponent>(*entity) {
                material.asset = self.asset;
                material.runtime_id = 0;
            }
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        for (entity, before) in &self.before {
            if let Some(material) = world.get_mut::<MaterialComponent>(*entity) {
                *material = *before;
            }
        }
    }

    fn description(&self) -> &str {
        "Assign Material"
    }

    fn is_no_op(&self) -> bool {
        self.before
            .iter()
            .all(|(_, material)| material.asset == self.asset)
    }
}

/// One reflected property edit fanned out across a multi-selection.
///
/// Godot's `multi_node_edit` and Unity both promise the same thing and it is
/// the promise that matters: setting roughness on twelve entities is *one*
/// history entry, so the undo that follows is one keystroke rather than
/// twelve. The per-entity commands are ordinary [`SetFieldCmd`]s, which is why
/// scoped undo, validation and coalescing all keep working unchanged.
pub struct SetFieldMultiCmd {
    commands: Vec<SetFieldCmd>,
    description: String,
    gesture: GestureId,
}

impl SetFieldMultiCmd {
    /// Build the fan-out. Entities that lack the component, or that would
    /// reject the value, are skipped rather than failing the whole edit — the
    /// intersection already guaranteed the row exists on every member, so a
    /// skip here means the world changed under the gesture.
    pub fn new(
        world: &World,
        entities: &[Entity],
        component: StableId,
        field: FieldId,
        value: ReflectValue,
        gesture: GestureId,
        mut baseline: impl FnMut(Entity) -> Option<FieldUndoSnapshot>,
    ) -> Result<Self, String> {
        let mut commands = Vec::new();
        for entity in entities {
            match SetFieldCmd::new(
                world,
                *entity,
                component,
                field,
                value.clone(),
                gesture,
                baseline(*entity),
            ) {
                Ok(command) => commands.push(command),
                Err(error) if commands.is_empty() && entities.len() == 1 => return Err(error),
                Err(_) => {}
            }
        }
        let first = commands
            .first()
            .ok_or("no selected entity accepted the edit")?;
        Ok(Self {
            description: first.description().to_owned(),
            gesture,
            commands,
        })
    }

    #[must_use]
    pub fn gesture(&self) -> GestureId {
        self.gesture
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl EditorCommand for SetFieldMultiCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        for command in &mut self.commands {
            command.execute(world, selected);
        }
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        for command in self.commands.iter_mut().rev() {
            command.undo(world, selected);
        }
    }

    fn description(&self) -> &str {
        &self.description
    }

    /// One entity still holding its baseline is enough to make the gesture
    /// real, so the whole fan-out only vanishes when nothing changed anywhere.
    fn is_no_op(&self) -> bool {
        self.commands.iter().all(EditorCommand::is_no_op)
    }
}

// ─── EditorCommand trait ──────────────────────────────────────────────────

/// A reversible editor operation.
///
/// Implement this trait for every mutation that should participate in
/// undo/redo. The [`UndoStack`] calls `execute` immediately on push and
/// `undo` on Ctrl+Z. `execute` is called again on Ctrl+Y (redo).
pub trait EditorCommand: Send + 'static {
    /// Apply the command to the world. Called on first push and on redo.
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>);
    /// Reverse the command. Called on undo.
    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>);
    /// Short human-readable description shown in the status bar.
    fn description(&self) -> &str;
    /// Commands whose final value equals their gesture baseline do not enter history.
    fn is_no_op(&self) -> bool {
        false
    }
}

// ─── UndoStack ────────────────────────────────────────────────────────────

/// Bounded command history for editor undo/redo.
///
/// - `push` executes a command immediately and appends it to the executed stack.
/// - `undo` pops the last executed command, calls its `undo()`, and moves it to
///   the redo stack.
/// - `redo` pops from the redo stack, calls `execute()` again, and moves it back
///   to the executed stack.
/// - Any new `push` clears the redo stack (new action invalidates redo history).
pub struct UndoStack {
    executed: Vec<Box<dyn EditorCommand>>,
    redo_stack: Vec<Box<dyn EditorCommand>>,
    max_size: usize,
}

impl UndoStack {
    pub fn new(max_size: usize) -> Self {
        Self {
            executed: Vec::new(),
            redo_stack: Vec::new(),
            max_size,
        }
    }

    /// Execute `cmd` and push it onto the undo stack.
    /// Clears the redo stack.
    pub fn push(
        &mut self,
        mut cmd: Box<dyn EditorCommand>,
        world: &mut World,
        selected: &mut Option<Entity>,
    ) {
        cmd.execute(world, selected);
        if cmd.is_no_op() {
            return;
        }
        self.redo_stack.clear();
        self.executed.push(cmd);
        if self.executed.len() > self.max_size {
            self.executed.remove(0);
        }
    }

    /// Undo the last executed command. Returns `true` if there was something to undo.
    pub fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) -> bool {
        let Some(mut cmd) = self.executed.pop() else {
            return false;
        };
        cmd.undo(world, selected);
        self.redo_stack.push(cmd);
        true
    }

    /// Redo the last undone command. Returns `true` if there was something to redo.
    pub fn redo(&mut self, world: &mut World, selected: &mut Option<Entity>) -> bool {
        let Some(mut cmd) = self.redo_stack.pop() else {
            return false;
        };
        cmd.execute(world, selected);
        self.executed.push(cmd);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.executed.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// The history as a list, oldest first, with the current position.
    ///
    /// CONTROL-J's history panel, following Flax's `History`. Entries are
    /// named after what changed — "Set wave_height", not "Change" — because a
    /// list of twenty rows all reading "Change" is not a history, it is a
    /// count.
    ///
    /// The position is the number of *executed* commands: index `0` is the
    /// state before anything happened, and index `n` is the state after the
    /// `n`th entry.
    #[must_use]
    pub fn history(&self) -> (Vec<&str>, usize) {
        let mut names: Vec<&str> = self
            .executed
            .iter()
            .map(|command| command.description())
            .collect();
        // The redo stack is stored newest-first, so it reads backwards.
        names.extend(
            self.redo_stack
                .iter()
                .rev()
                .map(|command| command.description()),
        );
        let position = self.executed.len();
        (names, position)
    }

    /// Move to a position in the history, undoing or redoing as needed.
    ///
    /// Returns how many steps were taken. A target beyond either end is
    /// clamped rather than refused: a click on the last row of a list that
    /// shrank under you should land on the end, not do nothing.
    pub fn jump_to(
        &mut self,
        target: usize,
        world: &mut World,
        selected: &mut Option<Entity>,
    ) -> usize {
        let total = self.executed.len() + self.redo_stack.len();
        let target = target.min(total);
        let mut steps = 0;
        while self.executed.len() > target {
            if !self.undo(world, selected) {
                break;
            }
            steps += 1;
        }
        while self.executed.len() < target {
            if !self.redo(world, selected) {
                break;
            }
            steps += 1;
        }
        steps
    }

    /// Push a command whose effect has already been applied to the world.
    /// Skips calling `execute()` — the command is only available to undo.
    pub fn push_silent(&mut self, cmd: Box<dyn EditorCommand>) {
        self.redo_stack.clear();
        self.executed.push(cmd);
        if self.executed.len() > self.max_size {
            self.executed.remove(0);
        }
    }
}

// ─── Generic reflected property edit ─────────────────────────────────────

/// State width selected by [`ChangeScope`] for one reversible property edit.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldUndoSnapshot {
    Field {
        component: StableId,
        field: FieldId,
        value: ReflectValue,
    },
    Component {
        component: StableId,
        values: ReflectObject,
    },
    Entity(Vec<(StableId, ReflectObject)>),
    Scene(Vec<(Entity, Vec<(StableId, ReflectObject)>)>),
}

impl FieldUndoSnapshot {
    pub fn capture(
        world: &World,
        entity: Entity,
        component: StableId,
        field: FieldId,
        scope: ChangeScope,
    ) -> Option<Self> {
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry.by_stable_id(component)?;
        match scope {
            ChangeScope::Field => Some(Self::Field {
                component,
                field,
                value: (schema.read_field)(world, entity, field)?,
            }),
            ChangeScope::Component => Some(Self::Component {
                component,
                values: (schema.snapshot)(world, entity)?,
            }),
            ChangeScope::Entity => Some(Self::Entity(
                registry
                    .schemas_on(world, entity)
                    .into_iter()
                    .filter_map(|schema| {
                        (schema.snapshot)(world, entity).map(|values| (schema.stable_id, values))
                    })
                    .collect(),
            )),
            ChangeScope::Scene => Some(Self::Scene(
                world
                    .entities()
                    .map(|item| {
                        let values = registry
                            .schemas_on(world, item)
                            .into_iter()
                            .filter_map(|schema| {
                                (schema.snapshot)(world, item)
                                    .map(|values| (schema.stable_id, values))
                            })
                            .collect();
                        (item, values)
                    })
                    .collect(),
            )),
        }
    }

    fn restore(&self, world: &mut World) {
        let registry = crate::reflect_registry::editor_registry();
        let mut restore_component = |entity, component, values: &ReflectObject| {
            if let Some(schema) = registry.by_stable_id(component) {
                let _ = (schema.apply)(world, entity, values);
            }
        };
        match self {
            Self::Field {
                component,
                field,
                value,
            } => {
                // Field snapshots are restored by SetFieldCmd, which knows the entity.
                let _ = (component, field, value);
            }
            Self::Component { .. } => {}
            Self::Entity(_) => {}
            Self::Scene(entities) => {
                for (entity, components) in entities {
                    for (component, values) in components {
                        restore_component(*entity, *component, values);
                    }
                }
            }
        }
    }

    fn field_value(
        &self,
        entity: Entity,
        component: StableId,
        field: FieldId,
    ) -> Option<ReflectValue> {
        match self {
            Self::Field {
                component: owner,
                field: owned,
                value,
            } if *owner == component && *owned == field => Some(value.clone()),
            Self::Component {
                component: owner,
                values,
            } if *owner == component => values.get(&field).cloned(),
            Self::Entity(components) => components
                .iter()
                .find(|(owner, _)| *owner == component)
                .and_then(|(_, values)| values.get(&field))
                .cloned(),
            Self::Scene(entities) => entities
                .iter()
                .find(|(owner, _)| *owner == entity)
                .and_then(|(_, components)| {
                    components.iter().find(|(owner, _)| *owner == component)
                })
                .and_then(|(_, values)| values.get(&field))
                .cloned(),
            _ => None,
        }
    }
}

/// Replace every field of one registered component in a single undo step.
///
/// Added by CONTROL-M, for presets. A preset touches six fields at once, and
/// six undo entries for one click is not a history anybody wants to walk back
/// through — Stride's `$"Update property {DisplayPath}"` rule (§6.2.3) says an
/// entry should be named after what the user did, and "Apply sky preset" is
/// what the user did.
///
/// Deliberately built on the same reflection path as [`SetFieldCmd`] rather
/// than storing the Rust value: the component is snapshotted and reapplied
/// through its schema, so a component that grows a field does not grow a
/// second place to remember it.
pub struct SetComponentCmd {
    entity: Entity,
    component: StableId,
    after: ReflectObject,
    before: ReflectObject,
    description: String,
}

impl SetComponentCmd {
    /// Snapshot `entity`'s `component` and stage `values` in its place.
    ///
    /// # Errors
    ///
    /// Names the component when it is unregistered or absent from the entity,
    /// and the field when a staged value fails its own validation — the
    /// preset, not the user, is at fault in that case and the message should
    /// say which field it got wrong.
    pub fn new(
        world: &World,
        entity: Entity,
        component: StableId,
        values: ReflectObject,
        description: impl Into<String>,
    ) -> Result<Self, String> {
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry
            .by_stable_id(component)
            .ok_or_else(|| format!("unknown component {component}"))?;
        let before =
            (schema.snapshot)(world, entity).ok_or_else(|| format!("entity has no {component}"))?;
        for (id, value) in &values {
            let field = schema
                .field(*id)
                .ok_or_else(|| format!("{component} has no field #{}", id.0))?;
            field.validate(value).map_err(|error| error.to_string())?;
        }
        Ok(Self {
            entity,
            component,
            after: values,
            before,
            description: description.into(),
        })
    }

    fn write(&self, world: &mut World, values: &ReflectObject) {
        let registry = crate::reflect_registry::editor_registry();
        if let Some(schema) = registry.by_stable_id(self.component) {
            let _ = (schema.apply)(world, self.entity, values);
        }
    }
}

impl EditorCommand for SetComponentCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.write(world, &self.after.clone());
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.write(world, &self.before.clone());
    }

    fn description(&self) -> &str {
        &self.description
    }

    /// A preset that changes nothing must not enter the history: clicking
    /// "Clear" twice should not cost two undo steps.
    fn is_no_op(&self) -> bool {
        self.after == self.before
    }
}

pub struct SetFieldCmd {
    entity: Entity,
    component: StableId,
    field: FieldId,
    value: ReflectValue,
    before_value: ReflectValue,
    before: FieldUndoSnapshot,
    gesture: GestureId,
    description: String,
}

impl SetFieldCmd {
    pub fn new(
        world: &World,
        entity: Entity,
        component: StableId,
        field: FieldId,
        value: ReflectValue,
        gesture: GestureId,
        before: Option<FieldUndoSnapshot>,
    ) -> Result<Self, String> {
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry
            .by_stable_id(component)
            .ok_or_else(|| format!("unknown component {component}"))?;
        let field_schema = schema
            .field(field)
            .ok_or_else(|| format!("unknown field #{}", field.0))?;
        if !field_schema.flags.contains(FieldFlags::EDIT) || field_schema.read_only {
            return Err(format!("{}.{} is read-only", component, field_schema.name));
        }
        field_schema
            .validate(&value)
            .map_err(|error| error.to_string())?;
        let before = before
            .or_else(|| {
                FieldUndoSnapshot::capture(world, entity, component, field, field_schema.scope)
            })
            .ok_or_else(|| "could not snapshot property edit".to_string())?;
        let before_value = before
            .field_value(entity, component, field)
            .ok_or_else(|| "snapshot omitted the edited field".to_string())?;
        Ok(Self {
            entity,
            component,
            field,
            value,
            before_value,
            before,
            gesture,
            description: format!(
                "Set {}",
                field_schema.display_name.unwrap_or(field_schema.name)
            ),
        })
    }

    pub fn apply_live(
        world: &mut World,
        entity: Entity,
        component: StableId,
        field: FieldId,
        mut value: ReflectValue,
    ) -> Result<(), String> {
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry
            .by_stable_id(component)
            .ok_or_else(|| format!("unknown component {component}"))?;
        let field_schema = schema
            .field(field)
            .ok_or_else(|| format!("unknown field #{}", field.0))?;
        if !field_schema.flags.contains(FieldFlags::EDIT) || field_schema.read_only {
            return Err("field is read-only".into());
        }
        if component.as_str() == "somnium.Water"
            && matches!(field_schema.name, "absorption" | "scattering")
        {
            if let (Some(ReflectValue::Vec3(current)), ReflectValue::Vec3(tint)) =
                ((schema.read_field)(world, entity, field), &value)
            {
                let magnitude = current[0].max(current[1]).max(current[2]).max(1.0e-6);
                value = ReflectValue::Vec3([
                    tint[0] * magnitude,
                    tint[1] * magnitude,
                    tint[2] * magnitude,
                ]);
            }
        }
        field_schema
            .validate(&value)
            .map_err(|error| error.to_string())?;
        let mut patch = ReflectObject::new();
        patch.insert(field, value);
        (schema.apply)(world, entity, &patch).map_err(|error| error.to_string())?;
        // Preserve component invariants formerly enforced by bespoke toggle
        // handlers. The generic path owns these now, so scripts, generated UI,
        // undo, and future surfaces all observe the same mutual exclusions.
        if component.as_str() == "somnium.PostProcess" {
            if let Some(pp) = world.get_mut::<crate::PostProcessComponent>(entity) {
                match field_schema.name {
                    // MORROWIND-AC: `taa_enabled` and `fsr_enabled` used to be
                    // re-applied here to restore their mutual exclusion after a
                    // generic patch. They are not fields any more — `aa` holds
                    // one value — so there is no pair left to reconcile.
                    "cas_enabled" => pp.set_cas_enabled(pp.cas_enabled),
                    "volumetrics_enabled" => pp.set_volumetrics_enabled(pp.volumetrics_enabled),
                    "light_shafts" => pp.set_light_shafts_enabled(pp.light_shafts),
                    "world_cache" => pp.set_world_cache_enabled(pp.world_cache),
                    "mesh_sdf" => pp.set_mesh_sdf_enabled(pp.mesh_sdf),
                    _ => {}
                }
            }
        }
        if component.as_str() == "somnium.Light" && field_schema.name == "color" {
            if let Some(light) = world.get_mut::<crate::LightComponent>(entity) {
                light.color_temperature_k = 0.0;
            }
        }
        Ok(())
    }

    pub fn scope(component: StableId, field: FieldId) -> Option<ChangeScope> {
        crate::reflect_registry::editor_registry()
            .by_stable_id(component)?
            .field(field)
            .map(|schema| schema.scope)
    }

    pub fn gesture(&self) -> GestureId {
        self.gesture
    }
}

impl EditorCommand for SetFieldCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let _ = Self::apply_live(
            world,
            self.entity,
            self.component,
            self.field,
            self.value.clone(),
        );
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let registry = crate::reflect_registry::editor_registry();
        match &self.before {
            FieldUndoSnapshot::Field {
                component,
                field,
                value,
            } => {
                let _ = Self::apply_live(world, self.entity, *component, *field, value.clone());
            }
            FieldUndoSnapshot::Component { component, values } => {
                if let Some(schema) = registry.by_stable_id(*component) {
                    let _ = (schema.apply)(world, self.entity, values);
                }
            }
            FieldUndoSnapshot::Entity(components) => {
                for (component, values) in components {
                    if let Some(schema) = registry.by_stable_id(*component) {
                        let _ = (schema.apply)(world, self.entity, values);
                    }
                }
            }
            FieldUndoSnapshot::Scene(_) => self.before.restore(world),
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
    fn is_no_op(&self) -> bool {
        self.before_value == self.value
    }
}

// ─── EntitySnapshot ───────────────────────────────────────────────────────

/// A copy of all editor-relevant components for one entity.
///
/// Used by [`DeleteEntityCmd`] to restore an entity on undo, and by
/// [`CreateEntityCmd`] to re-spawn a deleted creation on redo.
#[derive(Clone, Default)]
pub struct EntitySnapshot {
    pub transform: Option<Transform>,
    pub name: Option<Name>,
    pub light: Option<LightComponent>,
    pub mesh: Option<MeshComponent>,
    pub mat: Option<MaterialComponent>,
    pub wt: Option<WorldTransform>,
    pub mesh_kind: Option<MeshKind>,
    pub is_particle_emitter: bool,
    /// CONTROL-L/M/N. True when this entity carries the scene's environment —
    /// the day cycle, and (from CONTROL-M/N) the sky and weather beside it.
    /// A flag rather than three `Option`s because they are created, deleted
    /// and restored together as one authored object.
    pub environment: bool,
    /// CONTROL-O. A decal is a `Transform`, a `MaterialComponent` and this;
    /// the first two already round-trip, so only the third is new here.
    pub decal: Option<crate::decal::DecalComponent>,
    pub terrain: Option<TerrainComponent>,
    pub world_partition: Option<WorldPartitionComponent>,
    pub ui_canvas: Option<UiCanvasComponent>,
    pub voxel_terrain: Option<VoxelTerrainComponent>,
    pub foliage: Option<crate::FoliageComponent>,
    pub water: Option<WaterComponent>,
    pub parent: Option<Parent>,
    pub children: Option<Children>,
}

impl EntitySnapshot {
    /// Capture all editor components from a live entity.
    pub fn capture(world: &World, entity: Entity) -> Self {
        Self {
            transform: world.get::<Transform>(entity).copied(),
            name: world.get::<Name>(entity).copied(),
            light: world.get::<LightComponent>(entity).copied(),
            mesh: world.get::<MeshComponent>(entity).copied(),
            mat: world.get::<MaterialComponent>(entity).copied(),
            wt: world.get::<WorldTransform>(entity).copied(),
            mesh_kind: world.get::<MeshKind>(entity).copied(),
            is_particle_emitter: world.get::<crate::ParticleEmitter>(entity).is_some(),
            environment: world
                .get::<crate::time_of_day::TimeOfDayComponent>(entity)
                .is_some(),
            decal: world.get::<crate::decal::DecalComponent>(entity).copied(),
            terrain: world.get::<TerrainComponent>(entity).copied(),
            world_partition: world.get::<WorldPartitionComponent>(entity).cloned(),
            ui_canvas: world.get::<UiCanvasComponent>(entity).copied(),
            voxel_terrain: world.get::<VoxelTerrainComponent>(entity).copied(),
            foliage: world.get::<crate::FoliageComponent>(entity).cloned(),
            water: world.get::<WaterComponent>(entity).copied(),
            parent: world.get::<Parent>(entity).copied(),
            children: world.get::<Children>(entity).copied(),
        }
    }

    /// Spawn a new entity from this snapshot. Returns the new entity handle.
    pub fn respawn(self, world: &mut World) -> Entity {
        let transform = self
            .transform
            .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));
        let name = self.name.unwrap_or_else(|| Name::new("Entity"));
        let wt = self.wt.unwrap_or(WorldTransform::identity());

        if self.is_particle_emitter {
            return world.spawn((transform, name, wt, crate::ParticleEmitter::default()));
        }

        if let Some(decal) = self.decal {
            return match self.mat {
                Some(mat) => world.spawn((transform, name, wt, decal, mat)),
                None => world.spawn((transform, name, wt, decal)),
            };
        }

        if self.environment {
            return world.spawn((
                transform,
                name,
                wt,
                crate::time_of_day::TimeOfDayComponent::default(),
                crate::sky::SkyComponent::default(),
                crate::weather::WeatherComponent::default(),
            ));
        }

        if let Some(canvas) = self.ui_canvas {
            return world.spawn((transform, name, wt, canvas));
        }

        if let Some(water) = self.water {
            return match (self.mesh, self.mesh_kind, self.parent) {
                (Some(mesh), Some(kind), Some(parent)) => {
                    world.spawn((transform, name, wt, mesh, water, kind, parent))
                }
                (Some(mesh), Some(kind), None) => {
                    world.spawn((transform, name, wt, mesh, water, kind))
                }
                (Some(mesh), None, Some(parent)) => {
                    world.spawn((transform, name, wt, mesh, water, parent))
                }
                (Some(mesh), None, None) => world.spawn((transform, name, wt, mesh, water)),
                (None, _, Some(parent)) => world.spawn((transform, name, wt, water, parent)),
                (None, _, None) => world.spawn((transform, name, wt, water)),
            };
        }

        // Terrain entities only carry the component — the renderer-side
        // TerrainData survives deletion, so respawning reattaches to it.
        // Foliage rides along when present: the archetype ECS takes every
        // component at spawn time, so it cannot be attached afterwards.
        if let Some(terrain) = self.terrain {
            let entity = match (self.foliage, self.children) {
                (Some(f), Some(children)) => {
                    world.spawn((transform, name, wt, terrain, f, children))
                }
                (Some(f), None) => world.spawn((transform, name, wt, terrain, f)),
                (None, Some(children)) => world.spawn((transform, name, wt, terrain, children)),
                (None, None) => world.spawn((transform, name, wt, terrain)),
            };
            if let Some(partition) = self.world_partition {
                let _ = world.insert_component(entity, partition);
            }
            return entity;
        }

        // Voxel terrain: the game-layer driver is rebuilt from this component,
        // so respawning restores a working voxel world.
        if let Some(voxel) = self.voxel_terrain {
            return world.spawn((transform, name, wt, voxel));
        }

        // Archetype ECS requires all components at spawn time.
        // We branch on the presence of mesh+mat, light, and mesh_kind.
        match (self.mesh, self.mat, self.light, self.mesh_kind) {
            (Some(mesh), Some(mat), Some(light), Some(mk)) => {
                world.spawn((transform, name, wt, mesh, mat, light, mk))
            }
            (Some(mesh), Some(mat), Some(light), None) => {
                world.spawn((transform, name, wt, mesh, mat, light))
            }
            (Some(mesh), Some(mat), None, Some(mk)) => {
                world.spawn((transform, name, wt, mesh, mat, mk))
            }
            (Some(mesh), Some(mat), None, None) => world.spawn((transform, name, wt, mesh, mat)),
            // A mesh without a material is a real authored state — imported
            // geometry before a material is assigned, and every primitive the
            // Create menu makes. Before CONTROL-F these fell through to the
            // catch-all and silently lost their geometry on delete-then-undo,
            // and on paste.
            (Some(mesh), None, Some(light), Some(mk)) => {
                world.spawn((transform, name, wt, mesh, light, mk))
            }
            (Some(mesh), None, Some(light), None) => {
                world.spawn((transform, name, wt, mesh, light))
            }
            (Some(mesh), None, None, Some(mk)) => world.spawn((transform, name, wt, mesh, mk)),
            (Some(mesh), None, None, None) => world.spawn((transform, name, wt, mesh)),
            (None, _, Some(light), _) => world.spawn((transform, name, wt, light)),
            _ => world.spawn((transform, name, wt)),
        }
    }
}

/// Reversible hide/lock toggle.
///
/// Inserts [`crate::EditorFlags`] on demand, because the overwhelming majority
/// of entities never carry it and a default-valued component on every entity
/// would cost an archetype split for nothing.
pub struct SetEditorFlagsCmd {
    entity: Entity,
    before: crate::EditorFlags,
    after: crate::EditorFlags,
}

impl SetEditorFlagsCmd {
    /// Build the toggle from the current and desired flag state.
    pub fn new(entity: Entity, before: crate::EditorFlags, after: crate::EditorFlags) -> Self {
        Self {
            entity,
            before,
            after,
        }
    }

    fn write(&self, world: &mut World, value: crate::EditorFlags) {
        if let Some(flags) = world.get_mut::<crate::EditorFlags>(self.entity) {
            *flags = value;
        } else {
            let _ = world.insert_component(self.entity, value);
        }
    }
}

impl EditorCommand for SetEditorFlagsCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.write(world, self.after);
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.write(world, self.before);
    }

    fn description(&self) -> &str {
        if self.before.locked != self.after.locked {
            "Lock"
        } else {
            "Hide"
        }
    }

    fn is_no_op(&self) -> bool {
        self.before == self.after
    }
}

// ─── SetTransformCmd ──────────────────────────────────────────────────────

/// Reversible transform mutation from the inspector.
pub struct SetTransformCmd {
    entity_index: u32,
    old_transform: Transform,
    new_transform: Transform,
}

impl SetTransformCmd {
    pub fn new(entity_index: u32, old_transform: Transform, new_transform: Transform) -> Self {
        Self {
            entity_index,
            old_transform,
            new_transform,
        }
    }
}

impl EditorCommand for SetTransformCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(t) = world.get_mut::<Transform>(entity) {
                *t = self.new_transform;
            }
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(t) = world.get_mut::<Transform>(entity) {
                *t = self.old_transform;
            }
        }
    }

    fn description(&self) -> &str {
        "Set Transform"
    }
}

// ─── SetNameCmd ───────────────────────────────────────────────────────────

/// Reversible entity rename from the inspector.
pub struct SetNameCmd {
    entity_index: u32,
    old_name: Name,
    new_name: Name,
}

impl SetNameCmd {
    pub fn new(entity_index: u32, old_name: Name, new_name: Name) -> Self {
        Self {
            entity_index,
            old_name,
            new_name,
        }
    }
}

impl EditorCommand for SetNameCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(n) = world.get_mut::<Name>(entity) {
                *n = self.new_name;
            }
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(n) = world.get_mut::<Name>(entity) {
                *n = self.old_name;
            }
        }
    }

    fn description(&self) -> &str {
        "Rename Entity"
    }
}

// ─── SetLightCmd ──────────────────────────────────────────────────────────

/// Reversible light property mutation from the inspector.
pub struct SetLightCmd {
    entity_index: u32,
    old_light: LightComponent,
    new_light: LightComponent,
}

impl SetLightCmd {
    pub fn new(entity_index: u32, old_light: LightComponent, new_light: LightComponent) -> Self {
        Self {
            entity_index,
            old_light,
            new_light,
        }
    }
}

impl EditorCommand for SetLightCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(lc) = world.get_mut::<LightComponent>(entity) {
                *lc = self.new_light;
            }
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            if let Some(lc) = world.get_mut::<LightComponent>(entity) {
                *lc = self.old_light;
            }
        }
    }

    fn description(&self) -> &str {
        "Set Light"
    }
}

// ─── CreateEntityCmd ──────────────────────────────────────────────────────

/// Reversible entity creation.
///
/// On execute: spawns the entity from `snapshot` and records its index.
/// On undo: despawns the spawned entity.
/// On redo (execute again): respawns from snapshot again, updating the tracked index.
pub struct CreateEntityCmd {
    snapshot: EntitySnapshot,
    spawned_index: Option<u32>,
}

/// One undoable Terrain + child Water creation.
pub struct CreateLandscapeCmd {
    terrain: EntitySnapshot,
    water: EntitySnapshot,
    spawned: Option<(u32, u32)>,
}

impl CreateLandscapeCmd {
    pub fn new(mut terrain: EntitySnapshot, mut water: EntitySnapshot) -> Self {
        terrain.children = Some(Children::empty());
        water.parent = None;
        Self {
            terrain,
            water,
            spawned: None,
        }
    }
}

impl EditorCommand for CreateLandscapeCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        let terrain = self.terrain.clone().respawn(world);
        let mut water_snapshot = self.water.clone();
        water_snapshot.parent = Some(Parent { entity: terrain });
        let water = water_snapshot.respawn(world);
        if let Some(children) = world.get_mut::<Children>(terrain) {
            children.push(water);
        }
        self.spawned = Some((terrain.index(), water.index()));
        *selected = Some(terrain);
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some((terrain, water)) = self.spawned.take() {
            if let Some(entity) = world.find_entity_by_index(water) {
                world.despawn(entity);
            }
            if let Some(entity) = world.find_entity_by_index(terrain) {
                world.despawn(entity);
            }
        }
        *selected = None;
    }

    fn description(&self) -> &str {
        "Create Landscape"
    }
}

impl CreateEntityCmd {
    pub fn new(snapshot: EntitySnapshot) -> Self {
        Self {
            snapshot,
            spawned_index: None,
        }
    }
}

impl EditorCommand for CreateEntityCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        let entity = self.snapshot.clone().respawn(world);
        if let Some(parent) = self.snapshot.parent.map(|parent| parent.entity) {
            if let Some(children) = world.get_mut::<Children>(parent) {
                children.push(entity);
            }
        }
        self.spawned_index = Some(entity.index());
        *selected = Some(entity);
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(idx) = self.spawned_index {
            if let Some(entity) = world.find_entity_by_index(idx) {
                if let Some(parent) = world.get::<Parent>(entity).copied() {
                    if let Some(children) = world.get_mut::<Children>(parent.entity) {
                        children.remove(entity);
                    }
                }
                world.despawn(entity);
            }
            self.spawned_index = None;
        }
        *selected = None;
    }

    fn description(&self) -> &str {
        "Create Entity"
    }
}

// ─── DeleteEntityCmd ──────────────────────────────────────────────────────

/// Reversible entity deletion.
///
/// On execute: captures a component snapshot, then despawns.
/// On undo: respawns from snapshot (entity gets a new ID; `entity_index` is updated).
/// On redo (execute again): despawns the respawned entity by its updated index.
pub struct DeleteEntityCmd {
    entity_index: u32,
    snapshot: Option<EntitySnapshot>,
    child_snapshots: Vec<EntitySnapshot>,
}

impl DeleteEntityCmd {
    pub fn new(entity_index: u32) -> Self {
        Self {
            entity_index,
            snapshot: None,
            child_snapshots: Vec::new(),
        }
    }
}

impl EditorCommand for DeleteEntityCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            self.snapshot = Some(EntitySnapshot::capture(world, entity));
            self.child_snapshots = world
                .get::<Children>(entity)
                .map(|children| {
                    children
                        .as_slice()
                        .iter()
                        .copied()
                        .filter(|child| world.is_alive(*child))
                        .map(|child| EntitySnapshot::capture(world, child))
                        .collect()
                })
                .unwrap_or_default();
            let child_entities: Vec<Entity> = world
                .get::<Children>(entity)
                .map(|children| children.as_slice().to_vec())
                .unwrap_or_default();
            for child in child_entities {
                world.despawn(child);
            }
            if let Some(parent) = world.get::<Parent>(entity).copied() {
                if let Some(children) = world.get_mut::<Children>(parent.entity) {
                    children.remove(entity);
                }
            }
            world.despawn(entity);
            if *selected == Some(entity) {
                *selected = None;
            }
        }
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(snap) = self.snapshot.clone() {
            let restored_parent = snap.parent.map(|parent| parent.entity);
            let mut parent_snapshot = snap;
            if !self.child_snapshots.is_empty() {
                parent_snapshot.children = Some(Children::empty());
            }
            let entity = parent_snapshot.respawn(world);
            if let Some(parent) = restored_parent {
                if let Some(children) = world.get_mut::<Children>(parent) {
                    children.push(entity);
                }
            }
            for mut child_snapshot in self.child_snapshots.iter().cloned() {
                child_snapshot.parent = Some(Parent { entity });
                let child = child_snapshot.respawn(world);
                if let Some(children) = world.get_mut::<Children>(entity) {
                    children.push(child);
                }
            }
            // Update entity_index so the next execute (redo) targets the new entity.
            self.entity_index = entity.index();
            *selected = Some(entity);
        }
    }

    fn description(&self) -> &str {
        "Delete Entity"
    }
}

// ─── ReparentCmd ─────────────────────────────────────────────────────────

/// A set of commands presented to the undo stack as one authoring gesture.
pub struct CommandGroup {
    description: &'static str,
    commands: Vec<Box<dyn EditorCommand>>,
}

impl CommandGroup {
    pub fn new(description: &'static str, commands: Vec<Box<dyn EditorCommand>>) -> Self {
        Self {
            description,
            commands,
        }
    }
}

impl EditorCommand for CommandGroup {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        for command in &mut self.commands {
            command.execute(world, selected);
        }
    }
    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        for command in self.commands.iter_mut().rev() {
            command.undo(world, selected);
        }
    }
    fn description(&self) -> &str {
        self.description
    }
    fn is_no_op(&self) -> bool {
        self.commands.is_empty() || self.commands.iter().all(|c| c.is_no_op())
    }
}

/// Recoverable content import. Worker-created destination files are already
/// live when this command is recorded, so it is pushed silently; undo moves
/// them aside and redo restores the same collision-safe names.
pub struct FileImportCmd {
    files: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl FileImportCmd {
    pub fn new(destinations: Vec<std::path::PathBuf>) -> Self {
        let files = destinations
            .into_iter()
            .enumerate()
            .map(|(index, live)| {
                let recovery = live.with_extension(format!("somnium-undo-{index}"));
                (live, recovery)
            })
            .collect();
        Self { files }
    }
}

impl EditorCommand for FileImportCmd {
    fn execute(&mut self, _world: &mut World, _selected: &mut Option<Entity>) {
        for (live, recovery) in &self.files {
            if recovery.exists() && !live.exists() {
                let _ = std::fs::rename(recovery, live);
            }
        }
    }
    fn undo(&mut self, _world: &mut World, _selected: &mut Option<Entity>) {
        for (live, recovery) in &self.files {
            if live.exists() && !recovery.exists() {
                let _ = std::fs::rename(live, recovery);
            }
        }
    }
    fn description(&self) -> &str {
        "Import Files"
    }
}

/// Durable, validated multi-entity hierarchy move.
pub struct ReparentBatchCmd {
    children: Vec<PersistentId>,
    old_parents: Vec<Option<PersistentId>>,
    new_parent: Option<PersistentId>,
}

impl ReparentBatchCmd {
    pub fn new(
        world: &mut World,
        children: Vec<Entity>,
        new_parent: Option<Entity>,
    ) -> Result<Self, String> {
        if children.is_empty() {
            return Err("No entities were dragged".into());
        }
        if let Some(parent) = new_parent {
            if children.contains(&parent) {
                return Err("An entity cannot be parented to itself".into());
            }
            for child in &children {
                let mut cursor = Some(parent);
                while let Some(entity) = cursor {
                    if entity == *child {
                        return Err("An entity cannot be parented to its descendant".into());
                    }
                    cursor = world
                        .get::<Parent>(entity)
                        .and_then(|p| world.is_alive(p.entity).then_some(p.entity));
                }
            }
        }
        let new_parent_id = match new_parent {
            Some(e) => Some(world.ensure_persistent_id(e).map_err(|e| e.to_string())?),
            None => None,
        };
        let mut ids = Vec::with_capacity(children.len());
        let mut old = Vec::with_capacity(children.len());
        let mut changed = false;
        for child in children {
            ids.push(
                world
                    .ensure_persistent_id(child)
                    .map_err(|e| e.to_string())?,
            );
            let old_entity = world
                .get::<Parent>(child)
                .and_then(|p| world.is_alive(p.entity).then_some(p.entity));
            let old_id = match old_entity {
                Some(e) => Some(world.ensure_persistent_id(e).map_err(|e| e.to_string())?),
                None => None,
            };
            changed |= old_id != new_parent_id;
            old.push(old_id);
        }
        if !changed {
            return Err("The hierarchy would not change".into());
        }
        Ok(Self {
            children: ids,
            old_parents: old,
            new_parent: new_parent_id,
        })
    }

    fn apply(&self, world: &mut World, parents: impl Iterator<Item = Option<PersistentId>>) {
        let pairs: Vec<_> = self.children.iter().copied().zip(parents).collect();
        for (child_id, parent_id) in pairs {
            let Some(child) = world.entity_by_persistent_id(child_id) else {
                continue;
            };
            let parent = parent_id.and_then(|id| world.entity_by_persistent_id(id));
            do_reparent_entity(world, child, parent);
        }
    }
}

impl EditorCommand for ReparentBatchCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.apply(
            world,
            std::iter::repeat(self.new_parent).take(self.children.len()),
        );
    }
    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        self.apply(world, self.old_parents.iter().copied());
    }
    fn description(&self) -> &str {
        "Reparent Entities"
    }
}

/// Reversible hierarchy reparent.
pub struct ReparentCmd {
    child_index: u32,
    old_parent_index: Option<u32>,
    new_parent_index: Option<u32>,
}

impl ReparentCmd {
    pub fn new(
        child_index: u32,
        old_parent_index: Option<u32>,
        new_parent_index: Option<u32>,
    ) -> Self {
        Self {
            child_index,
            old_parent_index,
            new_parent_index,
        }
    }
}

impl EditorCommand for ReparentCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        do_reparent(world, self.child_index, self.new_parent_index);
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        do_reparent(world, self.child_index, self.old_parent_index);
    }

    fn description(&self) -> &str {
        "Reparent Entity"
    }
}

// ─── TerrainEditCmd (Phase 14D-4) ─────────────────────────────────────────

/// One pending terrain data restore produced by [`TerrainEditCmd`].
///
/// `EditorCommand` only receives the ECS `World`, but terrain data lives in
/// the renderer. Commands therefore push restore operations onto a queue
/// shared with the `Engine`, which drains it (with renderer access) right
/// after every undo/redo call.
pub enum TerrainRestoreOp {
    /// Restore a heightmap region: inclusive vertex rect + row-major heights.
    Heights {
        terrain_id: u32,
        region: (u32, u32, u32, u32),
        heights: Vec<f32>,
    },
    /// Restore a splatmap region: inclusive texel rect + row-major RGBA texels.
    Splat {
        terrain_id: u32,
        region: (u32, u32, u32, u32),
        texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
    },
}

/// Queue of terrain restores shared between commands and the `Engine`.
pub type TerrainRestoreQueue = std::sync::Arc<std::sync::Mutex<Vec<TerrainRestoreOp>>>;

/// Reversible terrain sculpt or paint stroke (Phase 14D-4 `TerrainEditCmd`).
///
/// Captured by the editor at stroke end (the stroke itself is applied live,
/// so the command is pushed with `push_silent`). Stores the before/after data
/// of the affected region only.
pub struct TerrainEditCmd {
    terrain_id: u32,
    region: (u32, u32, u32, u32),
    old_heights: Vec<f32>,
    new_heights: Vec<f32>,
    old_texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
    new_texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
    queue: TerrainRestoreQueue,
    is_paint: bool,
}

impl TerrainEditCmd {
    pub fn sculpt(
        terrain_id: u32,
        region: (u32, u32, u32, u32),
        old_heights: Vec<f32>,
        new_heights: Vec<f32>,
        queue: TerrainRestoreQueue,
    ) -> Self {
        Self {
            terrain_id,
            region,
            old_heights,
            new_heights,
            old_texels: Vec::new(),
            new_texels: Vec::new(),
            queue,
            is_paint: false,
        }
    }

    pub fn paint(
        terrain_id: u32,
        region: (u32, u32, u32, u32),
        old_texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
        new_texels: Vec<somnium_renderer::terrain::textures::SplatTexel>,
        queue: TerrainRestoreQueue,
    ) -> Self {
        Self {
            terrain_id,
            region,
            old_heights: Vec::new(),
            new_heights: Vec::new(),
            old_texels,
            new_texels,
            queue,
            is_paint: true,
        }
    }

    fn push_restore(&self, use_old: bool) {
        let op = if self.is_paint {
            TerrainRestoreOp::Splat {
                terrain_id: self.terrain_id,
                region: self.region,
                texels: if use_old {
                    self.old_texels.clone()
                } else {
                    self.new_texels.clone()
                },
            }
        } else {
            TerrainRestoreOp::Heights {
                terrain_id: self.terrain_id,
                region: self.region,
                heights: if use_old {
                    self.old_heights.clone()
                } else {
                    self.new_heights.clone()
                },
            }
        };
        if let Ok(mut q) = self.queue.lock() {
            q.push(op);
        }
    }
}

impl EditorCommand for TerrainEditCmd {
    fn execute(&mut self, _world: &mut World, _selected: &mut Option<Entity>) {
        self.push_restore(false);
    }

    fn undo(&mut self, _world: &mut World, _selected: &mut Option<Entity>) {
        self.push_restore(true);
    }

    fn description(&self) -> &str {
        if self.is_paint {
            "Paint Terrain"
        } else {
            "Sculpt Terrain"
        }
    }
}

// ─── Script attachments (Phase 16-D) ──────────────────────────────────────
//
// Attaching, removing, reordering and property edits are ordinary
// `EditorCommand`s, so Ctrl+Z covers scripting the way it covers every
// other authoring action. They all mutate the `ScriptSet` component; the
// runtime notices on its next reconcile and rebuilds the instance, which
// is why none of them has to know a VM exists.

use somnium_script::attachment::{ScriptAttachment, ScriptSet};
use somnium_script::ids::ScriptAssetId;
use somnium_script::value::ScriptValue;

/// Read the `ScriptSet` off an entity, run `edit` on it, and write it back.
///
/// `ScriptSet` is not `Copy` and the entity may not have one yet, so every
/// command below goes through this rather than repeating the dance.
fn edit_script_set(
    world: &mut World,
    entity_index: u32,
    edit: impl FnOnce(&mut ScriptSet),
) -> bool {
    let Some(entity) = world.find_entity_by_index(entity_index) else {
        return false;
    };
    let mut set = world.get::<ScriptSet>(entity).cloned().unwrap_or_default();
    edit(&mut set);
    world.insert_component(entity, set).is_ok()
}

/// Renumber `execution_order` to match list position.
///
/// The up/down arrows move a row in a list, and an author expects the list
/// order to *be* the run order. Authored `execution_order` values are
/// therefore rewritten by a reorder — documented in the help page, because
/// it is the one place this gesture overwrites something the author may
/// have typed.
fn renumber(set: &mut ScriptSet) {
    for (index, attachment) in set.attachments.iter_mut().enumerate() {
        attachment.execution_order = i32::try_from(index).unwrap_or(i32::MAX);
    }
}

/// Attach a script asset to an entity.
pub struct AttachScriptCmd {
    entity_index: u32,
    asset: ScriptAssetId,
    /// Minted once and reused on redo, so a redo restores the *same*
    /// attachment identity — which is what any migrated state is keyed by.
    attachment: ScriptAttachment,
}

impl AttachScriptCmd {
    pub fn new(entity_index: u32, asset: ScriptAssetId) -> Self {
        Self {
            entity_index,
            asset,
            attachment: ScriptAttachment::new(asset),
        }
    }

    /// The asset being attached.
    pub fn asset(&self) -> ScriptAssetId {
        self.asset
    }
}

impl EditorCommand for AttachScriptCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let attachment = self.attachment.clone();
        edit_script_set(world, self.entity_index, |set| {
            set.attach(attachment);
        });
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let instance = self.attachment.instance;
        edit_script_set(world, self.entity_index, |set| {
            set.detach(instance);
        });
    }

    fn description(&self) -> &str {
        "Attach Script"
    }
}

/// Remove one attachment, keeping enough to put it back.
pub struct DetachScriptCmd {
    entity_index: u32,
    index: usize,
    removed: Option<ScriptAttachment>,
}

impl DetachScriptCmd {
    pub fn new(entity_index: u32, index: usize) -> Self {
        Self {
            entity_index,
            index,
            removed: None,
        }
    }
}

impl EditorCommand for DetachScriptCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let index = self.index;
        let mut taken = None;
        edit_script_set(world, self.entity_index, |set| {
            if index < set.attachments.len() {
                taken = Some(set.attachments.remove(index));
            }
        });
        self.removed = taken;
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let Some(attachment) = self.removed.take() else {
            return;
        };
        let index = self.index;
        edit_script_set(world, self.entity_index, |set| {
            let at = index.min(set.attachments.len());
            set.attachments.insert(at, attachment);
        });
    }

    fn description(&self) -> &str {
        "Remove Script"
    }
}

/// Move one attachment earlier or later in execution order.
pub struct ReorderScriptCmd {
    entity_index: u32,
    from: usize,
    to: usize,
    /// The `execution_order` values before the renumber, so undo restores
    /// what the author had rather than a tidy 0,1,2.
    previous_orders: Vec<i32>,
}

impl ReorderScriptCmd {
    pub fn new(entity_index: u32, from: usize, to: usize) -> Self {
        Self {
            entity_index,
            from,
            to,
            previous_orders: Vec::new(),
        }
    }
}

impl EditorCommand for ReorderScriptCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let (from, to) = (self.from, self.to);
        let mut before = Vec::new();
        edit_script_set(world, self.entity_index, |set| {
            if from >= set.attachments.len() || to >= set.attachments.len() {
                return;
            }
            before = set.attachments.iter().map(|a| a.execution_order).collect();
            let moved = set.attachments.remove(from);
            set.attachments.insert(to, moved);
            renumber(set);
        });
        if !before.is_empty() {
            self.previous_orders = before;
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let (from, to) = (self.from, self.to);
        let orders = std::mem::take(&mut self.previous_orders);
        edit_script_set(world, self.entity_index, |set| {
            if from >= set.attachments.len() || to >= set.attachments.len() {
                return;
            }
            let moved = set.attachments.remove(to);
            set.attachments.insert(from, moved);
            for (attachment, order) in set.attachments.iter_mut().zip(&orders) {
                attachment.execution_order = *order;
            }
        });
        self.previous_orders = orders;
    }

    fn description(&self) -> &str {
        "Reorder Script"
    }
}

/// Switch one attachment on or off.
pub struct SetScriptEnabledCmd {
    entity_index: u32,
    index: usize,
    enabled: bool,
    was: bool,
}

impl SetScriptEnabledCmd {
    pub fn new(entity_index: u32, index: usize, enabled: bool) -> Self {
        Self {
            entity_index,
            index,
            enabled,
            was: !enabled,
        }
    }
}

impl EditorCommand for SetScriptEnabledCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let (index, enabled) = (self.index, self.enabled);
        let mut was = None;
        edit_script_set(world, self.entity_index, |set| {
            if let Some(attachment) = set.attachments.get_mut(index) {
                was = Some(attachment.enabled);
                attachment.enabled = enabled;
            }
        });
        if let Some(was) = was {
            self.was = was;
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let (index, was) = (self.index, self.was);
        edit_script_set(world, self.entity_index, |set| {
            if let Some(attachment) = set.attachments.get_mut(index) {
                attachment.enabled = was;
            }
        });
    }

    fn description(&self) -> &str {
        "Toggle Script"
    }
}

/// Edit one of a script's declared properties.
///
/// Follows the inspector live-scrub convention: a
/// drag is applied to the world and never recorded, and the gesture's
/// final value arrives once as a command. So this type only ever exists
/// for a committed edit.
pub struct SetScriptPropertyCmd {
    entity_index: u32,
    index: usize,
    field: String,
    value: ScriptValue,
    /// `None` when the property had no authored override and was showing
    /// the script's own default — undo has to remove the key, not write a
    /// copy of the default into the scene.
    previous: Option<ScriptValue>,
    captured: bool,
}

impl SetScriptPropertyCmd {
    pub fn new(entity_index: u32, index: usize, field: String, value: ScriptValue) -> Self {
        Self {
            entity_index,
            index,
            field,
            value,
            previous: None,
            captured: false,
        }
    }
}

impl EditorCommand for SetScriptPropertyCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let (index, value) = (self.index, self.value.clone());
        let field = self.field.clone();
        let capture = !self.captured;
        let mut previous = None;
        edit_script_set(world, self.entity_index, |set| {
            if let Some(attachment) = set.attachments.get_mut(index) {
                previous = attachment.properties.insert(field, value);
            }
        });
        if capture {
            self.previous = previous;
            self.captured = true;
        }
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        let index = self.index;
        let field = self.field.clone();
        let previous = self.previous.clone();
        edit_script_set(world, self.entity_index, |set| {
            if let Some(attachment) = set.attachments.get_mut(index) {
                match previous {
                    Some(value) => {
                        attachment.properties.insert(field, value);
                    }
                    None => {
                        attachment.properties.remove(&field);
                    }
                }
            }
        });
    }

    fn description(&self) -> &str {
        "Set Script Property"
    }
}

// ─── Shared reparent helper ───────────────────────────────────────────────

fn do_reparent(world: &mut World, child_idx: u32, new_parent_idx: Option<u32>) {
    let Some(child) = world.find_entity_by_index(child_idx) else {
        return;
    };

    // Detach child from its current parent's Children list.
    if let Some(old_p) = world.get::<Parent>(child).copied() {
        let old_parent = old_p.entity;
        if world.is_alive(old_parent) {
            if let Some(ch) = world.get_mut::<Children>(old_parent) {
                ch.remove(child);
            }
        }
    }

    if let Some(np_idx) = new_parent_idx {
        let Some(new_parent) = world.find_entity_by_index(np_idx) else {
            return;
        };
        if let Some(p) = world.get_mut::<Parent>(child) {
            p.entity = new_parent;
        }
        if let Some(ch) = world.get_mut::<Children>(new_parent) {
            ch.push(child);
        }
    } else {
        // Detach to root: mark parent as DANGLING (null sentinel).
        if let Some(p) = world.get_mut::<Parent>(child) {
            p.entity = Entity::DANGLING;
        }
    }
}

fn do_reparent_entity(world: &mut World, child: Entity, new_parent: Option<Entity>) {
    if let Some(old) = world
        .get::<Parent>(child)
        .and_then(|p| world.is_alive(p.entity).then_some(p.entity))
    {
        if let Some(children) = world.get_mut::<Children>(old) {
            children.remove(child);
        }
    }
    match new_parent {
        Some(parent) => {
            let _ = world.insert_component(child, Parent { entity: parent });
            if let Some(children) = world.get_mut::<Children>(parent) {
                if !children.as_slice().contains(&child) {
                    children.push(child);
                }
            } else {
                let mut children = Children::empty();
                children.push(child);
                let _ = world.insert_component(parent, children);
            }
        }
        None => {
            let _ = world.remove_component::<Parent>(child);
        }
    }
}

#[cfg(test)]
mod landscape_tests {
    use super::*;

    /// CONTROL-L/M/N: one Create row, one entity, three components.
    ///
    /// The archetype ECS takes every component at spawn time, so a missing arm
    /// in `respawn` does not error — it silently produces an Environment with
    /// no weather, and the first thing anybody notices is that the rain
    /// preset says "no Weather component in the scene".
    #[test]
    fn an_environment_respawns_with_all_three_of_its_components() {
        let mut world = World::new();
        let entity = EntitySnapshot {
            name: Some(Name::new("Environment")),
            environment: true,
            ..EntitySnapshot::default()
        }
        .respawn(&mut world);

        assert!(
            world
                .get::<crate::time_of_day::TimeOfDayComponent>(entity)
                .is_some()
        );
        assert!(world.get::<crate::sky::SkyComponent>(entity).is_some());
        assert!(
            world
                .get::<crate::weather::WeatherComponent>(entity)
                .is_some()
        );

        // And capture must see it, or delete-then-undo loses the environment.
        let captured = EntitySnapshot::capture(&world, entity);
        assert!(captured.environment);
    }

    /// CONTROL-M's preset path: six fields, one entry, and an exact undo.
    ///
    /// The alternative — six `SetFieldCmd`s — was measured against the plan's
    /// own rule that a history reading "Change" twenty times is not a history,
    /// and lost.
    #[test]
    fn a_whole_component_edit_is_one_named_undo_step() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(8);
        let entity = world.spawn((crate::sky::SkyComponent::default(),));

        let component = StableId::new("somnium.Sky");
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry.by_stable_id(component).expect("Sky is registered");

        let before = (schema.snapshot)(&world, entity).expect("Sky is on the entity");
        let staged = {
            let mut scratch = World::new();
            let mut storm = crate::sky::SkyComponent::default();
            assert!(storm.apply_preset("storm"));
            let temp = scratch.spawn((storm,));
            (schema.snapshot)(&scratch, temp).expect("staged Sky snapshots")
        };
        assert_ne!(before, staged);

        undo.push(
            Box::new(
                SetComponentCmd::new(
                    &world,
                    entity,
                    component,
                    staged.clone(),
                    "Sky preset: Storm",
                )
                .expect("staged values validate"),
            ),
            &mut world,
            &mut selected,
        );
        assert_eq!((schema.snapshot)(&world, entity), Some(staged));
        let (entries, position) = undo.history();
        assert_eq!(entries.last().copied(), Some("Sky preset: Storm"));
        assert_eq!(position, entries.len());

        undo.undo(&mut world, &mut selected);
        assert_eq!(
            (schema.snapshot)(&world, entity),
            Some(before),
            "undo restores every field, not only the ones the preset happened to change"
        );

        undo.redo(&mut world, &mut selected);
        assert_eq!(
            (schema.snapshot)(&world, entity).and_then(|values| values.get(&FieldId(1)).cloned()),
            Some(ReflectValue::F64(1.0)),
            "redo puts the storm back"
        );
    }

    /// Applying the same preset twice must not cost a second undo step.
    #[test]
    fn a_component_edit_that_changes_nothing_is_a_no_op() {
        let mut world = World::new();
        let entity = world.spawn((crate::sky::SkyComponent::default(),));
        let component = StableId::new("somnium.Sky");
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry.by_stable_id(component).expect("Sky is registered");
        let same = (schema.snapshot)(&world, entity).expect("Sky is on the entity");

        let command = SetComponentCmd::new(&world, entity, component, same, "Sky preset: Clear")
            .expect("identical values validate");
        assert!(command.is_no_op());
    }

    /// Every CONTROL-E route places a sentinel history entry beneath the drop
    /// and proves one undo removes only the drop. This is the model-node case:
    /// a glTF that spawns four entities is still one gesture, and the sentinel
    /// underneath it is untouched.
    #[test]
    fn imported_model_batch_is_one_undo_above_a_sentinel() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(8);

        // The sentinel: an unrelated authoring step the drop must not consume.
        undo.push(
            Box::new(CreateEntityCmd::new(EntitySnapshot {
                name: Some(Name::new("sentinel")),
                ..EntitySnapshot::default()
            })),
            &mut world,
            &mut selected,
        );
        let before = world.entities().count();

        let nodes: Vec<Box<dyn EditorCommand>> = (0..4)
            .map(|index| {
                Box::new(CreateEntityCmd::new(EntitySnapshot {
                    name: Some(Name::new(&format!("node{index}"))),
                    transform: Some(Transform {
                        translation: glam::Vec3::new(5.0, 1.0, 5.0),
                        ..Transform::default()
                    }),
                    ..EntitySnapshot::default()
                })) as Box<dyn EditorCommand>
            })
            .collect();
        undo.push(
            Box::new(CommandGroup::new("Import Model", nodes)),
            &mut world,
            &mut selected,
        );
        assert_eq!(world.entities().count(), before + 4);

        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(
            world.entities().count(),
            before,
            "one undo must remove every imported node"
        );
        assert!(
            world.entities().any(|e| world
                .get::<Name>(e)
                .is_some_and(|n| n.as_str() == "sentinel")),
            "the sentinel beneath the drop must survive"
        );
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), before + 4);
    }

    /// An empty group is a no-op, so a drop that resolved to nothing cannot
    /// leave a phantom row in the history panel CONTROL-J will show.
    #[test]
    fn an_empty_command_group_is_a_no_op() {
        assert!(CommandGroup::new("Attach Scripts", Vec::new()).is_no_op());
    }

    /// OS-file import completes asynchronously, so its undo entry is recorded
    /// after the copy already exists. Undo moves the file aside rather than
    /// deleting it, and redo restores the same collision-safe name.
    #[test]
    fn external_file_import_undo_is_recoverable_and_redoable() {
        let root = std::env::temp_dir().join(format!(
            "somnium_file_import_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let live = root.join("tree.png");
        std::fs::write(&live, b"png").unwrap();

        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(4);
        undo.push_silent(Box::new(FileImportCmd::new(vec![live.clone()])));
        assert!(live.exists(), "a silent push must not re-run the copy");

        assert!(undo.undo(&mut world, &mut selected));
        assert!(!live.exists(), "undo moves the import out of Content");

        assert!(undo.redo(&mut world, &mut selected));
        assert!(live.exists(), "redo restores the same collision-safe name");
        assert_eq!(std::fs::read(&live).unwrap(), b"png");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The history panel's model: a named list, and where in it we are.
    #[test]
    fn the_history_reads_as_a_list_with_a_position() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(8);
        let entity = world.spawn((
            Transform::default(),
            Name::new("box"),
            WorldTransform::identity(),
        ));

        for name in ["one", "two", "three"] {
            undo.push(
                Box::new(SetNameCmd::new(
                    entity.index(),
                    *world.get::<Name>(entity).unwrap(),
                    Name::new(name),
                )),
                &mut world,
                &mut selected,
            );
        }
        let (names, position) = undo.history();
        assert_eq!(names.len(), 3);
        assert_eq!(position, 3, "we are at the end");
        assert!(
            names.iter().all(|name| *name == "Rename Entity"),
            "entries are named after what changed: {names:?}"
        );

        // Undoing moves the marker without shortening the list — the point of
        // a history panel is that you can see where you could go back to.
        assert!(undo.undo(&mut world, &mut selected));
        let (names, position) = undo.history();
        assert_eq!(names.len(), 3, "{names:?}");
        assert_eq!(position, 2);
    }

    /// Clicking a row jumps there, in either direction, and the world follows.
    #[test]
    fn jumping_to_a_position_moves_the_world_both_ways() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(8);
        let entity = world.spawn((
            Transform::default(),
            Name::new("start"),
            WorldTransform::identity(),
        ));
        for name in ["one", "two", "three"] {
            undo.push(
                Box::new(SetNameCmd::new(
                    entity.index(),
                    *world.get::<Name>(entity).unwrap(),
                    Name::new(name),
                )),
                &mut world,
                &mut selected,
            );
        }
        assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "three");

        assert_eq!(undo.jump_to(1, &mut world, &mut selected), 2);
        assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "one");
        assert_eq!(undo.history().1, 1);

        assert_eq!(undo.jump_to(3, &mut world, &mut selected), 2);
        assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "three");

        // Position zero is the state before anything happened.
        undo.jump_to(0, &mut world, &mut selected);
        assert_eq!(world.get::<Name>(entity).unwrap().as_str(), "start");
    }

    /// A target beyond either end is clamped. A click on the last row of a
    /// list that shrank under you should land on the end, not do nothing.
    #[test]
    fn a_jump_past_either_end_is_clamped() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(4);
        let entity = world.spawn((
            Transform::default(),
            Name::new("start"),
            WorldTransform::identity(),
        ));
        undo.push(
            Box::new(SetNameCmd::new(
                entity.index(),
                Name::new("start"),
                Name::new("one"),
            )),
            &mut world,
            &mut selected,
        );
        assert_eq!(undo.jump_to(99, &mut world, &mut selected), 0);
        assert_eq!(undo.history().1, 1);
        assert_eq!(undo.jump_to(0, &mut world, &mut selected), 1);
        assert_eq!(undo.history().1, 0);
    }

    /// CONTROL-F's exit clause, literally: select twelve entities, set their
    /// roughness once, undo once.
    #[test]
    fn twelve_entities_edited_once_undo_once() {
        let mut world = World::new();
        let entities: Vec<_> = (0..12)
            .map(|_| {
                world.spawn((
                    Transform::default(),
                    Name::new("box"),
                    WorldTransform::identity(),
                ))
            })
            .collect();

        let component = StableId::new("somnium.Transform");
        let field = crate::reflect_registry::component_registry()
            .by_stable_id(component)
            .expect("Transform is registered")
            .fields
            .iter()
            .find(|field| field.name == "scale")
            .expect("Transform has a scale")
            .id;

        let mut selected = Some(entities[0]);
        let mut undo = UndoStack::new(8);
        // A sentinel beneath the edit, so "one undo" is checked against
        // something rather than against an empty stack.
        undo.push(
            Box::new(SetNameCmd::new(
                entities[0].index(),
                Name::new("box"),
                Name::new("sentinel"),
            )),
            &mut world,
            &mut selected,
        );

        let command = SetFieldMultiCmd::new(
            &world,
            &entities,
            component,
            field,
            ReflectValue::Vec3([2.0, 2.0, 2.0]),
            GestureId(11),
            |_| None,
        )
        .expect("the fan-out must build");
        assert_eq!(command.len(), 12);
        undo.push(Box::new(command), &mut world, &mut selected);
        for entity in &entities {
            assert_eq!(world.get::<Transform>(*entity).unwrap().scale.x, 2.0);
        }

        assert!(undo.undo(&mut world, &mut selected));
        for entity in &entities {
            assert_eq!(
                world.get::<Transform>(*entity).unwrap().scale.x,
                1.0,
                "one undo must restore every entity"
            );
        }
        assert_eq!(
            world.get::<Name>(entities[0]).unwrap().as_str(),
            "sentinel",
            "the entry beneath the multi-edit must survive"
        );
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(world.get::<Transform>(entities[11]).unwrap().scale.x, 2.0);
    }

    /// Every entity keeps its own baseline, so a fan-out over entities that
    /// started at different values restores all of them, not one shared value.
    #[test]
    fn the_fan_out_restores_each_entity_to_its_own_baseline() {
        let mut world = World::new();
        let entities: Vec<_> = (0..3)
            .map(|index| {
                world.spawn((
                    Transform::from_translation(glam::Vec3::splat(index as f32)),
                    Name::new("box"),
                    WorldTransform::identity(),
                ))
            })
            .collect();
        let component = StableId::new("somnium.Transform");
        let field = crate::reflect_registry::component_registry()
            .by_stable_id(component)
            .unwrap()
            .fields
            .iter()
            .find(|field| field.name == "translation")
            .unwrap()
            .id;

        let mut selected = None;
        let mut undo = UndoStack::new(4);
        let command = SetFieldMultiCmd::new(
            &world,
            &entities,
            component,
            field,
            ReflectValue::Vec3([9.0, 9.0, 9.0]),
            GestureId(3),
            |_| None,
        )
        .unwrap();
        undo.push(Box::new(command), &mut world, &mut selected);
        assert!(undo.undo(&mut world, &mut selected));
        for (index, entity) in entities.iter().enumerate() {
            assert_eq!(
                world.get::<Transform>(*entity).unwrap().translation.x,
                index as f32
            );
        }
    }

    /// A fan-out where nothing actually changed does not enter history.
    #[test]
    fn a_fan_out_that_changes_nothing_is_a_no_op() {
        let mut world = World::new();
        let entities: Vec<_> = (0..3)
            .map(|_| {
                world.spawn((
                    Transform::default(),
                    Name::new("box"),
                    WorldTransform::identity(),
                ))
            })
            .collect();
        let component = StableId::new("somnium.Transform");
        let field = crate::reflect_registry::component_registry()
            .by_stable_id(component)
            .unwrap()
            .fields
            .iter()
            .find(|field| field.name == "scale")
            .unwrap()
            .id;

        let mut selected = None;
        let mut undo = UndoStack::new(4);
        let command = SetFieldMultiCmd::new(
            &world,
            &entities,
            component,
            field,
            ReflectValue::Vec3([1.0, 1.0, 1.0]),
            GestureId(4),
            |_| None,
        )
        .unwrap();
        undo.push(Box::new(command), &mut world, &mut selected);
        assert!(!undo.can_undo(), "an unchanged edit must not enter history");
    }

    #[test]
    fn batch_reparent_is_one_undo_and_survives_entity_handles() {
        let mut world = World::new();
        let old = world.spawn((Children::empty(),));
        let next = world.spawn((Children::empty(),));
        let a = world.spawn((Parent { entity: old },));
        let b = world.spawn((Parent { entity: old },));
        world.get_mut::<Children>(old).unwrap().push(a);
        world.get_mut::<Children>(old).unwrap().push(b);
        let cmd = ReparentBatchCmd::new(&mut world, vec![a, b], Some(next)).unwrap();
        let mut undo = UndoStack::new(4);
        let mut selected = Some(a);
        undo.push(Box::new(cmd), &mut world, &mut selected);
        assert_eq!(world.get::<Parent>(a).unwrap().entity, next);
        assert_eq!(world.get::<Parent>(b).unwrap().entity, next);
        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(world.get::<Parent>(a).unwrap().entity, old);
        assert_eq!(world.get::<Parent>(b).unwrap().entity, old);
        assert!(!undo.undo(&mut world, &mut selected));
    }

    #[test]
    fn reparent_rejects_self_descendant_and_noop_before_mutation() {
        let mut world = World::new();
        let root = world.spawn((Children::empty(),));
        let child = world.spawn((Parent { entity: root }, Children::empty()));
        let grandchild = world.spawn((Parent { entity: child },));
        world.get_mut::<Children>(root).unwrap().push(child);
        world.get_mut::<Children>(child).unwrap().push(grandchild);
        assert!(ReparentBatchCmd::new(&mut world, vec![root], Some(root)).is_err());
        assert!(ReparentBatchCmd::new(&mut world, vec![root], Some(grandchild)).is_err());
        assert!(ReparentBatchCmd::new(&mut world, vec![child], Some(root)).is_err());
        assert_eq!(world.get::<Parent>(child).unwrap().entity, root);
    }

    #[test]
    fn vector_material_assignment_is_exactly_one_undo_step() {
        let mut world = World::new();
        let old_a = somnium_asset::database::AssetId::from_raw(11);
        let old_b = somnium_asset::database::AssetId::from_raw(22);
        let next = somnium_asset::database::AssetId::from_raw(33);
        let a = world.spawn((MaterialComponent {
            asset: old_a,
            runtime_id: 7,
        },));
        let b = world.spawn((MaterialComponent {
            asset: old_b,
            runtime_id: 8,
        },));
        let mut selected = Some(a);
        let mut undo = UndoStack::new(8);
        undo.push(
            Box::new(AssignMaterialCmd::new(&world, vec![a, b], next)),
            &mut world,
            &mut selected,
        );
        assert_eq!(world.get::<MaterialComponent>(a).unwrap().asset, next);
        assert_eq!(world.get::<MaterialComponent>(b).unwrap().asset, next);
        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(world.get::<MaterialComponent>(a).unwrap().asset, old_a);
        assert_eq!(world.get::<MaterialComponent>(b).unwrap().asset, old_b);
        assert!(!undo.undo(&mut world, &mut selected));
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(world.get::<MaterialComponent>(a).unwrap().asset, next);
        assert_eq!(world.get::<MaterialComponent>(b).unwrap().asset, next);
    }

    #[test]
    fn make_unique_undo_restores_assignment_without_deleting_the_copy() {
        let folder = std::env::temp_dir().join("somnium-make-unique-control-d");
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("Shared.sommat");
        std::fs::write(&source, "shared").unwrap();
        let copy = somnium_asset::material::unique_sibling(&source);
        std::fs::copy(&source, &copy).unwrap();

        let shared = somnium_asset::database::AssetId::from_relative_path("Shared.sommat");
        let unique =
            somnium_asset::database::AssetId::from_relative_path(copy.file_name().unwrap());
        let mut world = World::new();
        let entity = world.spawn((MaterialComponent {
            asset: shared,
            runtime_id: 4,
        },));
        let mut selected = Some(entity);
        let mut undo = UndoStack::new(4);
        undo.push(
            Box::new(AssignMaterialCmd::new(&world, vec![entity], unique)),
            &mut world,
            &mut selected,
        );
        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(
            world.get::<MaterialComponent>(entity).unwrap().asset,
            shared
        );
        assert!(copy.exists(), "undo must not delete authored content");
        let _ = std::fs::remove_file(copy);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(folder);
    }

    #[test]
    fn generated_material_field_uses_generic_live_gesture_undo() {
        let mut world = World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let registry = crate::reflect_registry::editor_registry();
        let schema = registry.by_name("somnium.asset.Material").unwrap();
        let field = schema.field_by_name("roughness").unwrap().id;
        let gesture = GestureId(77);
        let baseline =
            FieldUndoSnapshot::capture(&world, entity, schema.stable_id, field, ChangeScope::Field);
        SetFieldCmd::apply_live(
            &mut world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::F64(0.2),
        )
        .unwrap();
        let command = SetFieldCmd::new(
            &world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::F64(0.2),
            gesture,
            baseline,
        )
        .unwrap();
        let mut undo = UndoStack::new(4);
        let mut selected = Some(entity);
        undo.push(Box::new(command), &mut world, &mut selected);
        assert_eq!(
            world
                .get::<somnium_asset::material::MaterialAsset>(entity)
                .unwrap()
                .roughness,
            0.2
        );
        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(
            world
                .get::<somnium_asset::material::MaterialAsset>(entity)
                .unwrap()
                .roughness,
            0.5
        );
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(
            world
                .get::<somnium_asset::material::MaterialAsset>(entity)
                .unwrap()
                .roughness,
            0.2
        );
    }

    fn terrain_snapshot() -> EntitySnapshot {
        EntitySnapshot {
            transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
            name: Some(Name::new("Terrain")),
            light: None,
            mesh: None,
            mat: None,
            wt: Some(WorldTransform::identity()),
            environment: false,
            decal: None,
            mesh_kind: None,
            is_particle_emitter: false,
            terrain: Some(TerrainComponent {
                terrain_id: 2,
                chunk_cells: 64,
                grid_x: 16,
                grid_z: 16,
                cell_size: 1.0,
                height_scale: 1.0,
                ..TerrainComponent::default()
            }),
            world_partition: Some(WorldPartitionComponent {
                load_radius: 320.0,
                ..WorldPartitionComponent::default()
            }),
            ui_canvas: None,
            voxel_terrain: None,
            foliage: None,
            water: None,
            parent: None,
            children: Some(Children::empty()),
        }
    }

    fn water_snapshot() -> EntitySnapshot {
        EntitySnapshot {
            transform: Some(Transform::from_translation(glam::Vec3::new(
                512.0, 15.0, 512.0,
            ))),
            name: Some(Name::new("Water")),
            light: None,
            mesh: None,
            mat: None,
            wt: Some(WorldTransform::identity()),
            environment: false,
            decal: None,
            mesh_kind: None,
            is_particle_emitter: false,
            terrain: None,
            world_partition: None,
            ui_canvas: None,
            voxel_terrain: None,
            foliage: None,
            water: Some(WaterComponent::great_lakes(
                3,
                2,
                [0.0, 0.0, 1024.0, 1024.0],
            )),
            parent: None,
            children: None,
        }
    }

    #[test]
    fn landscape_create_undo_redo_preserves_two_entity_hierarchy() {
        let mut world = World::new();
        let mut selected = None;
        let mut stack = UndoStack::new(8);
        stack.push(
            Box::new(CreateLandscapeCmd::new(
                terrain_snapshot(),
                water_snapshot(),
            )),
            &mut world,
            &mut selected,
        );
        assert_eq!(world.entities().count(), 2);
        let terrain = selected.unwrap();
        assert_eq!(
            world
                .get::<WorldPartitionComponent>(terrain)
                .expect("landscape restores its streaming controls")
                .load_radius,
            320.0
        );
        let child = world.get::<Children>(terrain).unwrap().as_slice()[0];
        assert_eq!(world.get::<Parent>(child).unwrap().entity, terrain);
        assert!(world.get::<WaterComponent>(child).is_some());

        assert!(stack.undo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), 0);
        assert!(stack.redo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), 2);
        let terrain = selected.unwrap();
        assert!(world.get::<WorldPartitionComponent>(terrain).is_some());
    }

    #[test]
    fn ui_canvas_create_undo_redo_preserves_authored_settings() {
        let mut world = World::new();
        let mut selected = None;
        let mut stack = UndoStack::new(8);
        let canvas = UiCanvasComponent {
            width: 1280.0,
            height: 720.0,
            layer: 42,
            ..UiCanvasComponent::default()
        };
        stack.push(
            Box::new(CreateEntityCmd::new(EntitySnapshot {
                name: Some(Name::new("HUD")),
                ui_canvas: Some(canvas),
                ..EntitySnapshot::default()
            })),
            &mut world,
            &mut selected,
        );

        let created = selected.expect("create selects the canvas");
        assert_eq!(world.get::<UiCanvasComponent>(created), Some(&canvas));
        assert!(stack.undo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), 0);
        assert!(stack.redo(&mut world, &mut selected));
        let restored = selected.expect("redo reselects the canvas");
        assert_eq!(world.get::<UiCanvasComponent>(restored), Some(&canvas));
    }

    #[test]
    fn deleting_a_landscape_cascades_and_undo_restores_the_child() {
        let mut world = World::new();
        let mut selected = None;
        let mut create = CreateLandscapeCmd::new(terrain_snapshot(), water_snapshot());
        create.execute(&mut world, &mut selected);
        let terrain = selected.unwrap();
        let mut delete = DeleteEntityCmd::new(terrain.index());
        delete.execute(&mut world, &mut selected);
        assert_eq!(world.entities().count(), 0);
        delete.undo(&mut world, &mut selected);
        assert_eq!(world.entities().count(), 2);
        let terrain = selected.unwrap();
        let water = world.get::<Children>(terrain).unwrap().as_slice()[0];
        assert_eq!(world.get::<Parent>(water).unwrap().entity, terrain);
        assert!(world.get::<WaterComponent>(water).is_some());
    }

    #[test]
    fn reflected_field_edit_is_one_undo_step_and_redoes() {
        let mut world = World::new();
        let entity = world.spawn((Transform::default(),));
        let mut selected = Some(entity);
        let mut stack = UndoStack::new(8);
        let registry = crate::reflect_registry::component_registry();
        let schema = registry.by_name("somnium.Transform").unwrap();
        let field = schema.field_by_name("translation").unwrap().id;
        let command = SetFieldCmd::new(
            &world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::Vec3([2.0, 3.0, 4.0]),
            GestureId(7),
            None,
        )
        .unwrap();
        assert_eq!(command.gesture(), GestureId(7));
        stack.push(Box::new(command), &mut world, &mut selected);
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::vec3(2.0, 3.0, 4.0)
        );
        assert!(stack.undo(&mut world, &mut selected));
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::ZERO
        );
        assert!(stack.redo(&mut world, &mut selected));
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::vec3(2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn live_gesture_uses_the_mouse_down_baseline_and_coalesces() {
        let mut world = World::new();
        let entity = world.spawn((Transform::default(), TerrainComponent::default()));
        let mut selected = Some(entity);
        let mut stack = UndoStack::new(8);
        let registry = crate::reflect_registry::component_registry();
        let schema = registry.by_name("somnium.Terrain").unwrap();
        let field = schema.field_by_name("chunk_cells").unwrap().id;
        let baseline = FieldUndoSnapshot::capture(
            &world,
            entity,
            schema.stable_id,
            field,
            ChangeScope::Entity,
        )
        .unwrap();
        SetFieldCmd::apply_live(
            &mut world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::I64(96),
        )
        .unwrap();
        SetFieldCmd::apply_live(
            &mut world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::I64(128),
        )
        .unwrap();
        world.get_mut::<Transform>(entity).unwrap().translation.x = 99.0;
        let command = SetFieldCmd::new(
            &world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::I64(128),
            GestureId(9),
            Some(baseline),
        )
        .unwrap();
        stack.push(Box::new(command), &mut world, &mut selected);
        assert!(stack.undo(&mut world, &mut selected));
        assert_eq!(
            world.get::<TerrainComponent>(entity).unwrap().chunk_cells,
            TerrainComponent::default().chunk_cells
        );
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::ZERO,
            "entity-scoped undo restores dependent state"
        );
        assert!(
            !stack.undo(&mut world, &mut selected),
            "all live writes coalesce into one entry"
        );
    }

    #[test]
    fn no_op_reflected_edit_is_discarded() {
        let mut world = World::new();
        let entity = world.spawn((Transform::default(),));
        let mut selected = Some(entity);
        let mut stack = UndoStack::new(8);
        let registry = crate::reflect_registry::component_registry();
        let schema = registry.by_name("somnium.Transform").unwrap();
        let field = schema.field_by_name("translation").unwrap().id;
        let command = SetFieldCmd::new(
            &world,
            entity,
            schema.stable_id,
            field,
            ReflectValue::Vec3([0.0; 3]),
            GestureId(1),
            None,
        )
        .unwrap();
        stack.push(Box::new(command), &mut world, &mut selected);
        assert!(!stack.can_undo());
    }
}
