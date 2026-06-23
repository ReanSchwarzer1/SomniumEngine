//! Phase 11.5E: Editor undo/redo command system.
//!
//! All mutating editor operations (inspector edits, create/delete, reparent)
//! are wrapped in an [`EditorCommand`] and routed through [`UndoStack`].
//! Ctrl+Z calls [`UndoStack::undo`]; Ctrl+Y calls [`UndoStack::redo`].
#![allow(missing_docs, clippy::wildcard_imports)]

use somnium_ecs::{Entity, World};
use crate::{
    Children, LightComponent, MaterialComponent, MeshComponent, MeshKind,
    Name, Parent, TerrainComponent, Transform, WorldTransform,
};

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
        self.redo_stack.clear();
        self.executed.push(cmd);
        if self.executed.len() > self.max_size {
            self.executed.remove(0);
        }
    }

    /// Undo the last executed command. Returns `true` if there was something to undo.
    pub fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) -> bool {
        let Some(mut cmd) = self.executed.pop() else { return false };
        cmd.undo(world, selected);
        self.redo_stack.push(cmd);
        true
    }

    /// Redo the last undone command. Returns `true` if there was something to redo.
    pub fn redo(&mut self, world: &mut World, selected: &mut Option<Entity>) -> bool {
        let Some(mut cmd) = self.redo_stack.pop() else { return false };
        cmd.execute(world, selected);
        self.executed.push(cmd);
        true
    }

    pub fn can_undo(&self) -> bool { !self.executed.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo_stack.is_empty() }

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

// ─── EntitySnapshot ───────────────────────────────────────────────────────

/// A copy of all editor-relevant components for one entity.
///
/// Used by [`DeleteEntityCmd`] to restore an entity on undo, and by
/// [`CreateEntityCmd`] to re-spawn a deleted creation on redo.
#[derive(Clone, Copy)]
pub struct EntitySnapshot {
    pub transform: Option<Transform>,
    pub name: Option<Name>,
    pub light: Option<LightComponent>,
    pub mesh: Option<MeshComponent>,
    pub mat: Option<MaterialComponent>,
    pub wt: Option<WorldTransform>,
    pub mesh_kind: Option<MeshKind>,
    pub is_particle_emitter: bool,
    pub terrain: Option<TerrainComponent>,
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
            terrain: world.get::<TerrainComponent>(entity).copied(),
        }
    }

    /// Spawn a new entity from this snapshot. Returns the new entity handle.
    pub fn respawn(self, world: &mut World) -> Entity {
        let transform = self.transform.unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));
        let name = self.name.unwrap_or_else(|| Name::new("Entity"));
        let wt = self.wt.unwrap_or(WorldTransform::identity());

        if self.is_particle_emitter {
            return world.spawn((transform, name, wt, crate::ParticleEmitter::default()));
        }

        // Terrain entities only carry the component — the renderer-side
        // TerrainData survives deletion, so respawning reattaches to it.
        if let Some(terrain) = self.terrain {
            return world.spawn((transform, name, wt, terrain));
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
            (Some(mesh), Some(mat), None, None) => {
                world.spawn((transform, name, wt, mesh, mat))
            }
            (None, _, Some(light), _) => {
                world.spawn((transform, name, wt, light))
            }
            _ => {
                world.spawn((transform, name, wt))
            }
        }
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
        Self { entity_index, old_transform, new_transform }
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

    fn description(&self) -> &str { "Set Transform" }
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
        Self { entity_index, old_name, new_name }
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

    fn description(&self) -> &str { "Rename Entity" }
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
        Self { entity_index, old_light, new_light }
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

    fn description(&self) -> &str { "Set Light" }
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

impl CreateEntityCmd {
    pub fn new(snapshot: EntitySnapshot) -> Self {
        Self { snapshot, spawned_index: None }
    }
}

impl EditorCommand for CreateEntityCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        let entity = self.snapshot.respawn(world);
        self.spawned_index = Some(entity.index());
        *selected = Some(entity);
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(idx) = self.spawned_index {
            if let Some(entity) = world.find_entity_by_index(idx) {
                world.despawn(entity);
            }
            self.spawned_index = None;
        }
        *selected = None;
    }

    fn description(&self) -> &str { "Create Entity" }
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
}

impl DeleteEntityCmd {
    pub fn new(entity_index: u32) -> Self {
        Self { entity_index, snapshot: None }
    }
}

impl EditorCommand for DeleteEntityCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(entity) = world.find_entity_by_index(self.entity_index) {
            self.snapshot = Some(EntitySnapshot::capture(world, entity));
            world.despawn(entity);
            if *selected == Some(entity) {
                *selected = None;
            }
        }
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        if let Some(snap) = self.snapshot {
            let entity = snap.respawn(world);
            // Update entity_index so the next execute (redo) targets the new entity.
            self.entity_index = entity.index();
            *selected = Some(entity);
        }
    }

    fn description(&self) -> &str { "Delete Entity" }
}

// ─── ReparentCmd ─────────────────────────────────────────────────────────

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
        Self { child_index, old_parent_index, new_parent_index }
    }
}

impl EditorCommand for ReparentCmd {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        do_reparent(world, self.child_index, self.new_parent_index);
    }

    fn undo(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        do_reparent(world, self.child_index, self.old_parent_index);
    }

    fn description(&self) -> &str { "Reparent Entity" }
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
        texels: Vec<[u8; 4]>,
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
    old_texels: Vec<[u8; 4]>,
    new_texels: Vec<[u8; 4]>,
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
            terrain_id, region, old_heights, new_heights,
            old_texels: Vec::new(), new_texels: Vec::new(),
            queue, is_paint: false,
        }
    }

    pub fn paint(
        terrain_id: u32,
        region: (u32, u32, u32, u32),
        old_texels: Vec<[u8; 4]>,
        new_texels: Vec<[u8; 4]>,
        queue: TerrainRestoreQueue,
    ) -> Self {
        Self {
            terrain_id, region,
            old_heights: Vec::new(), new_heights: Vec::new(),
            old_texels, new_texels,
            queue, is_paint: true,
        }
    }

    fn push_restore(&self, use_old: bool) {
        let op = if self.is_paint {
            TerrainRestoreOp::Splat {
                terrain_id: self.terrain_id,
                region: self.region,
                texels: if use_old { self.old_texels.clone() } else { self.new_texels.clone() },
            }
        } else {
            TerrainRestoreOp::Heights {
                terrain_id: self.terrain_id,
                region: self.region,
                heights: if use_old { self.old_heights.clone() } else { self.new_heights.clone() },
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
        if self.is_paint { "Paint Terrain" } else { "Sculpt Terrain" }
    }
}

// ─── Shared reparent helper ───────────────────────────────────────────────

fn do_reparent(world: &mut World, child_idx: u32, new_parent_idx: Option<u32>) {
    let Some(child) = world.find_entity_by_index(child_idx) else { return };

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
        let Some(new_parent) = world.find_entity_by_index(np_idx) else { return };
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
