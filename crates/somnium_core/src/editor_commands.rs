//! Phase 11.5E: Editor undo/redo command system.
//!
//! All mutating editor operations (inspector edits, create/delete, reparent)
//! are wrapped in an [`EditorCommand`] and routed through [`UndoStack`].
//! Ctrl+Z calls [`UndoStack::undo`]; Ctrl+Y calls [`UndoStack::redo`].
#![allow(missing_docs, clippy::wildcard_imports)]

use crate::{
    Children, LightComponent, MaterialComponent, MeshComponent, MeshKind, Name, Parent,
    TerrainComponent, Transform, VoxelTerrainComponent, WaterComponent, WorldTransform,
};
use somnium_ecs::{Entity, World};

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
            terrain: world.get::<TerrainComponent>(entity).copied(),
            voxel_terrain: world.get::<VoxelTerrainComponent>(entity).copied(),
            foliage: world.get::<crate::FoliageComponent>(entity).copied(),
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
            return match (self.foliage, self.children) {
                (Some(f), Some(children)) => {
                    world.spawn((transform, name, wt, terrain, f, children))
                }
                (Some(f), None) => world.spawn((transform, name, wt, terrain, f)),
                (None, Some(children)) => world.spawn((transform, name, wt, terrain, children)),
                (None, None) => world.spawn((transform, name, wt, terrain)),
            };
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
            (None, _, Some(light), _) => world.spawn((transform, name, wt, light)),
            _ => world.spawn((transform, name, wt)),
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
        let terrain = self.terrain.respawn(world);
        let mut water_snapshot = self.water;
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
        let entity = self.snapshot.respawn(world);
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
        if let Some(snap) = self.snapshot {
            let mut parent_snapshot = snap;
            if !self.child_snapshots.is_empty() {
                parent_snapshot.children = Some(Children::empty());
            }
            let entity = parent_snapshot.respawn(world);
            if let Some(parent) = snap.parent.map(|parent| parent.entity) {
                if let Some(children) = world.get_mut::<Children>(parent) {
                    children.push(entity);
                }
            }
            for mut child_snapshot in self.child_snapshots.iter().copied() {
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
            before = set
                .attachments
                .iter()
                .map(|a| a.execution_order)
                .collect();
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
/// Follows the live-scrub convention `SetInspectorValue` established: a
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

#[cfg(test)]
mod landscape_tests {
    use super::*;

    fn terrain_snapshot() -> EntitySnapshot {
        EntitySnapshot {
            transform: Some(Transform::from_translation(glam::Vec3::ZERO)),
            name: Some(Name::new("Terrain")),
            light: None,
            mesh: None,
            mat: None,
            wt: Some(WorldTransform::identity()),
            mesh_kind: None,
            is_particle_emitter: false,
            terrain: Some(TerrainComponent {
                terrain_id: 2,
                chunk_cells: 64,
                grid_x: 16,
                grid_z: 16,
                cell_size: 1.0,
                height_scale: 1.0,
            }),
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
            mesh_kind: None,
            is_particle_emitter: false,
            terrain: None,
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
        let child = world.get::<Children>(terrain).unwrap().as_slice()[0];
        assert_eq!(world.get::<Parent>(child).unwrap().entity, terrain);
        assert!(world.get::<WaterComponent>(child).is_some());

        assert!(stack.undo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), 0);
        assert!(stack.redo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), 2);
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
}
