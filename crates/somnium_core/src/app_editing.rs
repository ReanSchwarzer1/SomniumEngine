//! Level editing on generated worlds: what a clicked object is and where it came from (the selection card), an edit
//! layer that outlives the generator scripts, object-first and triangle-accurate clicks, Drop to Ground, Align to
//! Surface, Find Floating Objects, hide and isolate, Find in Outliner and Save Scene As.
//!
//! The pure halves live in `scene_edits.rs` (the layer: apply and record) and `editor_ground.rs` (BVHs and the
//! ground rules); this file wires them to the live world, the renderer and the panels.

use super::*;
use crate::editor_ground::{self, Ground, Hit, MeshSurface, TriangleBvh};
use crate::scene_edits::{self, EditOp, Origin, SceneEdits, Scope};
use serde_json::Value;
use somnium_ecs::{Entity, PersistentId};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A click picks the part, not the object, when the object has more parts than this.
const CONTAINER_PARTS: usize = 400;
/// Find Floating Objects looks this far round the camera.
const FLOAT_RADIUS: f32 = 80.0;
/// Meshes with more triangles than this are clicked and stood on by their box.
const BVH_TRIANGLE_LIMIT: u32 = 3_000_000;

/// What the editor knows about the open scene's edits and the level tools' state.
#[derive(Default)]
pub(crate) struct EditingState {
    /// The opened scene file; Save writes the edit layer beside it.
    scene: Option<PathBuf>,
    /// The layer: as loaded, re-keyed where a generator renamed ids, with this session folded in at each save.
    layer: Option<SceneEdits>,
    /// The layer file exists but could not be read: Save refuses rather than overwrite it.
    layer_error: Option<String>,
    /// First-touch snapshots of every entity selected this session, and of everything under it.
    baselines: HashMap<PersistentId, Baseline>,
    /// Ids the opened scene and its streamed cells loaded with.
    base: HashSet<PersistentId>,
    /// Ids with saved edits, plus touched ones changed since their snapshot.
    edited: HashSet<PersistentId>,
    /// Ids the last floating scan reported.
    floating: HashSet<PersistentId>,
    /// Who made a streamed object: a cell's `sources` (unit root id -> label).
    sources: HashMap<PersistentId, String>,
    /// Each loaded file's own `made_by`, by project-relative path.
    made_by: HashMap<String, String>,
    /// glTF parts by source path (`None`: unreadable).
    parts: HashMap<String, Option<Arc<Vec<somnium_asset::GltfPart>>>>,
    /// Triangle BVHs by mesh range.
    bvhs: HashMap<(u32, u32, u32), Option<Arc<TriangleBvh>>>,
    /// Tools that write a name prefix, by prefix.
    prefix_tools: HashMap<String, Vec<String>>,
    /// Entities Isolate hid, with the flags they had.
    isolated: Vec<(Entity, Option<EditorFlags>)>,
    /// What the last scene load did with its layer, for one toast.
    summary: Option<String>,
    /// Terrain was sculpted or painted, which the layer does not hold.
    terrain_touched: bool,
    /// The card and the state it describes.
    card: Option<(CardKey, String)>,
    /// The undo generation the edited set was last checked at.
    checked_generation: u64,
    /// The selection last touched.
    touched_selection: Vec<Entity>,
    /// Frames counted for `SOMNIUM_EDITOR_SCRIPT`.
    script_frame: u64,
}

#[derive(Clone, PartialEq)]
struct CardKey {
    entity: Entity,
    generation: u64,
    count: usize,
    edited: bool,
    dirty: bool,
}

struct Baseline {
    json: Value,
    origin: Origin,
}

// ─── Small helpers ──────────────────────────────────────────────────────────

/// A path relative to the project, with forward slashes and `..` resolved, so one file always reads the same.
pub(crate) fn project_relative(root: Option<&Path>, path: &Path) -> String {
    // The project root may be held in its canonical form (`\\?\F:\...`) while a scene path was joined from the
    // plain one, or the other way round: try both before giving up on a relative path.
    let canonical = |p: &Path| std::fs::canonicalize(p).ok();
    let relative = root.and_then(|root| {
        path.strip_prefix(root).ok().map(Path::to_path_buf).or_else(|| {
            let (root, path) = (canonical(root)?, canonical(path)?);
            path.strip_prefix(&root).ok().map(Path::to_path_buf)
        })
    });
    let path = relative.as_deref().unwrap_or(path);
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if parts.last().is_some_and(|last| last != "..") {
                    parts.pop();
                } else {
                    parts.push("..".into());
                }
            }
            std::path::Component::CurDir | std::path::Component::RootDir => {}
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::Prefix(prefix) => parts.push(prefix.as_os_str().to_string_lossy().into_owned()),
        }
    }
    parts.join("/")
}

/// The entity at the top of `entity`'s hierarchy.
fn top_ancestor(world: &World, entity: Entity) -> Entity {
    let mut current = entity;
    for _ in 0..64 {
        match world.get::<Parent>(current).map(|parent| parent.entity) {
            Some(parent) if parent != Entity::DANGLING && world.is_alive(parent) => current = parent,
            _ => break,
        }
    }
    current
}

/// Parent -> children from every `Parent` (`Children` keeps only sixteen).
fn children_map(world: &World) -> HashMap<Entity, Vec<Entity>> {
    let mut map: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, parent) in world.iter_with::<Parent>() {
        if parent.entity != Entity::DANGLING && world.is_alive(parent.entity) {
            map.entry(parent.entity).or_default().push(entity);
        }
    }
    map
}

/// `root` and everything under it, parents first.
fn subtree(children: &HashMap<Entity, Vec<Entity>>, root: Entity) -> Vec<Entity> {
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        if let Some(kids) = children.get(&found[next]) {
            found.extend(kids.iter().copied());
        }
        next += 1;
    }
    found
}

fn model_of(world: &World, entity: Entity) -> Option<glam::Mat4> {
    world
        .get::<WorldTransform>(entity)
        .map(|world| world.0)
        .or_else(|| world.get::<Transform>(entity).map(Transform::to_matrix))
}

fn transformed_box(model: glam::Mat4, min: glam::Vec3, max: glam::Vec3) -> (glam::Vec3, glam::Vec3) {
    let mut lo = glam::Vec3::splat(f32::INFINITY);
    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
    for i in 0..8 {
        let corner = glam::Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let point = model.transform_point3(corner);
        lo = lo.min(point);
        hi = hi.max(point);
    }
    (lo, hi)
}

/// The world box round every mesh in `members`.
fn world_bounds(world: &World, renderer: &SomniumRenderer, members: &[Entity]) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut lo = glam::Vec3::splat(f32::INFINITY);
    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for &entity in members {
        let Some(mesh) = world.get::<MeshComponent>(entity) else {
            continue;
        };
        let (Some((min, max)), Some(model)) = (renderer.geometry.mesh_aabb(mesh.vertex_offset), model_of(world, entity))
        else {
            continue;
        };
        let (a, b) = transformed_box(model, glam::Vec3::from_array(min), glam::Vec3::from_array(max));
        lo = lo.min(a);
        hi = hi.max(b);
        any = true;
    }
    any.then_some((lo, hi))
}

fn name_of(world: &World, entity: Entity) -> String {
    world
        .get::<Name>(entity)
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| format!("Entity {}", entity.index()))
}

/// One mesh drawn somewhere, for the ground and click queries.
struct MeshItem {
    entity: Entity,
    mesh: MeshComponent,
    model: glam::Mat4,
    min: glam::Vec3,
    max: glam::Vec3,
}

/// Every visible mesh's world box, binned on an 8 m grid in XZ.
struct MeshIndex {
    items: Vec<MeshItem>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

const GRID: f32 = 8.0;

impl MeshIndex {
    fn build(world: &World, renderer: &SomniumRenderer) -> Self {
        let mut items = drawn_items(world, renderer);
        for (entity, mesh) in world.iter_with::<MeshComponent>() {
            if crate::is_hidden(world, entity) {
                continue;
            }
            let (Some((min, max)), Some(model)) = (renderer.geometry.mesh_aabb(mesh.vertex_offset), model_of(world, entity))
            else {
                continue;
            };
            let (lo, hi) = transformed_box(model, glam::Vec3::from_array(min), glam::Vec3::from_array(max));
            if !lo.is_finite() || !hi.is_finite() {
                continue;
            }
            items.push(MeshItem { entity, mesh: *mesh, model, min: lo, max: hi });
        }
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, item) in items.iter().enumerate() {
            let (x0, z0) = ((item.min.x / GRID).floor() as i32, (item.min.z / GRID).floor() as i32);
            let (x1, z1) = ((item.max.x / GRID).floor() as i32, (item.max.z / GRID).floor() as i32);
            // A mesh wider than the whole grid search (ground, a merged street) goes in every cell it spans; past 64
            // cells a side it is kept once, in its corner cell's overflow list.
            if (x1 - x0) > 64 || (z1 - z0) > 64 {
                grid.entry((i32::MIN, i32::MIN)).or_default().push(i);
                continue;
            }
            for x in x0..=x1 {
                for z in z0..=z1 {
                    grid.entry((x, z)).or_default().push(i);
                }
            }
        }
        Self { items, grid }
    }

    /// Indices of meshes whose box meets `min..max`.
    fn query(&self, min: glam::Vec3, max: glam::Vec3) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        let (x0, z0) = ((min.x / GRID).floor() as i32, (min.z / GRID).floor() as i32);
        let (x1, z1) = ((max.x / GRID).floor() as i32, (max.z / GRID).floor() as i32);
        let wide = self.grid.get(&(i32::MIN, i32::MIN)).into_iter().flatten();
        let cells = (x0..=x1).flat_map(|x| (z0..=z1).map(move |z| (x, z)));
        for i in cells.filter_map(|cell| self.grid.get(&cell)).flatten().chain(wide) {
            let item = &self.items[*i];
            if item.min.cmple(max).all() && item.max.cmpge(min).all() {
                found.push(*i);
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// What a game drew itself last frame for entities with no mesh component (TSF's rooms: a name, a transform and its own
/// room component), as mesh items. The draw's sort key names the entity, by the convention the selection outline uses;
/// a draw whose box is nowhere near its entity is someone else's key (characters key by vertex offset) and is left out.
fn drawn_items(world: &World, renderer: &SomniumRenderer) -> Vec<MeshItem> {
    let mut items = Vec::new();
    for draw in renderer.last_frame_draws() {
        let index = draw.sort_key.0 as u32;
        if index == 0 {
            continue;
        }
        let Some(entity) = world.find_entity_by_index(index) else {
            continue;
        };
        if world.get::<MeshComponent>(entity).is_some() || crate::is_hidden(world, entity) {
            continue;
        }
        let Some((min, max)) = renderer.geometry.mesh_aabb(draw.vertex_offset) else {
            continue;
        };
        let (lo, hi) = transformed_box(draw.transform, glam::Vec3::from_array(min), glam::Vec3::from_array(max));
        let at = model_of(world, entity).map_or(glam::Vec3::ZERO, |m| m.w_axis.truncate());
        let near = at.cmpge(lo - glam::Vec3::ONE).all() && at.cmple(hi + glam::Vec3::ONE).all();
        if !near || !lo.is_finite() || !hi.is_finite() {
            continue;
        }
        let mesh = MeshComponent {
            vertex_offset: draw.vertex_offset,
            index_offset: draw.index_offset,
            index_count: draw.index_count,
        };
        items.push(MeshItem { entity, mesh, model: draw.transform, min: lo, max: hi });
    }
    items
}

/// The nearest terrain hit along a unit ray.
fn terrain_hit(renderer: &SomniumRenderer, terrains: &[u32], origin: glam::Vec3, direction: glam::Vec3, max: f32) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for &id in terrains {
        let Some(terrain) = renderer.terrain(id) else {
            continue;
        };
        let Some(point) = terrain.raycast(origin, direction) else {
            continue;
        };
        let distance = (point - origin).dot(direction);
        if (0.0..=max).contains(&distance) && best.is_none_or(|b| distance < b.distance) {
            best = Some(Hit { distance, normal: glam::Vec3::Y });
        }
    }
    best
}

impl<G: GameApp> Engine<G> {
    // ─── The edit layer ─────────────────────────────────────────────────────

    /// A scene file is about to spawn: read its edit layer and apply it. Returns the edited document when there is
    /// one to apply.
    pub(super) fn edits_before_scene_spawn(&mut self, path: &str, document: &Value) -> Option<Value> {
        let scene = PathBuf::from(path);
        self.reset_editing(Some(scene.clone()));
        // A shipped game applies the layer and keeps nothing else: the bookkeeping below serves the editor.
        if !self.config.player_mode {
            let root = self.config.project_root.clone();
            self.editing.base = ids_in(document);
            if let Some(label) = made_by(document) {
                self.editing.made_by.insert(project_relative(root.as_deref(), &scene), label);
            }
        }
        match SceneEdits::read(&scene) {
            Ok(Some(mut layer)) => {
                let mut edited = document.clone();
                let applied = scene_edits::apply(&mut edited, &mut layer, Scope::Scene);
                self.note_applied(&applied);
                let file = scene_edits::layer_path(&scene);
                let mut summary = applied.summary();
                let waiting = layer.entities.iter().filter(|entry| !entry.from.is_empty()).count();
                if waiting > 0 {
                    if !summary.is_empty() {
                        summary.push_str("; ");
                    }
                    summary.push_str(&format!("{waiting} more apply as their cells stream in"));
                }
                if !summary.is_empty() {
                    self.editing.summary = Some(format!(
                        "{}: {summary}",
                        file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    ));
                }
                for (id, why) in &applied.orphaned {
                    warn!(id = %id, why = %why, "edit layer entry not applied");
                }
                self.editing.layer = Some(layer);
                Some(edited)
            }
            Ok(None) => {
                self.editing.layer = Some(SceneEdits::new(&scene));
                None
            }
            Err(error) => {
                warn!(%error, "edit layer unreadable");
                self.editing.summary = Some(format!("Your edits file could not be read, so it was not applied: {error}"));
                self.editing.layer_error = Some(error);
                None
            }
        }
    }

    /// After the scene spawned: say what its layer did.
    pub(super) fn edits_after_scene_spawn(&mut self) {
        if let Some(summary) = self.editing.summary.take() {
            self.toast(&summary);
        }
    }

    /// Forget the old scene's edit state (a new scene, a map, a different file).
    pub(super) fn reset_editing(&mut self, scene: Option<PathBuf>) {
        let parts = std::mem::take(&mut self.editing.parts);
        let prefix_tools = std::mem::take(&mut self.editing.prefix_tools);
        self.editing = EditingState { scene, parts, prefix_tools, ..EditingState::default() };
    }

    fn note_applied(&mut self, applied: &scene_edits::Applied) {
        for id in applied.changed.iter().chain(&applied.added) {
            if let Some(id) = PersistentId::parse_hex(id) {
                self.editing.edited.insert(id);
            }
        }
    }

    /// A streamed cell is about to spawn: its entries of the layer (and anything moved this session that has not
    /// been saved yet) go into its document.
    pub(super) fn edits_before_cell_spawn(&mut self, index: usize, document: &Value) -> Option<Value> {
        let root = self.config.project_root.clone();
        let file = project_relative(root.as_deref(), &self.cell_streamer.cells[index].file);
        if !self.config.player_mode {
            self.editing.base.extend(ids_in(document));
            if let Some(label) = made_by(document) {
                self.editing.made_by.insert(file.clone(), label);
            }
            if let Some(sources) = document.get("sources").and_then(Value::as_object) {
                for (id, label) in sources {
                    if let (Some(id), Some(label)) = (PersistentId::parse_hex(id), label.as_str()) {
                        self.editing.sources.insert(id, label.to_owned());
                    }
                }
            }
        }
        let layer = self.editing.layer.as_mut()?;
        if layer.entities.is_empty() {
            return None;
        }
        let mut edited = document.clone();
        let applied = scene_edits::apply(&mut edited, layer, Scope::Cell(&file));
        for (id, why) in &applied.orphaned {
            warn!(id = %id, why = %why, cell = %file, "edit layer entry not applied");
        }
        self.note_applied(&applied);
        Some(edited)
    }

    /// A streamed cell is about to unload: fold what was done to its entities into the layer first.
    pub(super) fn edits_before_cell_unload(&mut self, entities: &[Entity]) {
        let ids: HashSet<PersistentId> = entities
            .iter()
            .filter_map(|e| self.world.persistent_id(*e))
            .filter(|id| self.editing.baselines.contains_key(id))
            .collect();
        if !ids.is_empty() {
            self.fold_session(Some(&ids), true);
        }
    }

    /// Snapshot whatever has just been selected (and everything under it) before anything can change it.
    pub(super) fn touch_selection(&mut self) {
        if self.config.player_mode || self.play_session_active {
            return;
        }
        let generation = self.undo_stack.generation();
        if generation != self.editing.checked_generation {
            self.editing.checked_generation = generation;
            self.refresh_session_edits();
        }
        let selection = self.selection.as_slice();
        if selection == self.editing.touched_selection.as_slice() {
            return;
        }
        self.editing.touched_selection = selection.to_vec();
        let children = children_map(&self.world);
        let mut fresh: Vec<Entity> = Vec::new();
        for &entity in self.selection.as_slice() {
            for member in subtree(&children, entity) {
                let known = self
                    .world
                    .persistent_id(member)
                    .is_some_and(|id| self.editing.baselines.contains_key(&id));
                if !known && !fresh.contains(&member) {
                    fresh.push(member);
                }
            }
        }
        if fresh.is_empty() {
            return;
        }
        let snapshots = self.snapshots(&fresh);
        for entity in fresh {
            let Some(id) = self.world.persistent_id(entity) else {
                continue;
            };
            let Some(json) = snapshots.get(&id.to_string()).cloned() else {
                continue;
            };
            let origin = self.origin_of(entity);
            self.editing.baselines.insert(id, Baseline { json, origin });
        }
    }

    /// The scene documents of `entities`, by persistent id (minting ids where missing).
    fn snapshots(&mut self, entities: &[Entity]) -> HashMap<String, Value> {
        let alive: Vec<Entity> = entities.iter().copied().filter(|e| self.world.is_alive(*e)).collect();
        let Ok(document) = crate::scene_schema::entities_to_json(&mut self.world, &self.type_registry, &alive) else {
            return HashMap::new();
        };
        document["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entity| Some((entity.get("persistent_id")?.as_str()?.to_owned(), entity.clone())))
            .collect()
    }

    fn origin_of(&self, entity: Entity) -> Origin {
        let from = self
            .world
            .get::<crate::cell_streaming::Streamed>(entity)
            .and_then(|streamed| self.cell_streamer.cells.get(streamed.cell as usize))
            .map(|cell| project_relative(self.config.project_root.as_deref(), &cell.file))
            .unwrap_or_default();
        let parent = self
            .world
            .get::<Parent>(entity)
            .filter(|parent| self.world.is_alive(parent.entity))
            .and_then(|parent| self.world.get::<Name>(parent.entity))
            .map(|name| name.as_str().to_owned())
            .unwrap_or_default();
        Origin { from, parent }
    }

    /// Which touched entities differ from their snapshot now (for the marker), keeping the saved ones.
    fn refresh_session_edits(&mut self) {
        let touched: Vec<(PersistentId, Entity)> = self
            .editing
            .baselines
            .keys()
            .filter_map(|id| Some((*id, self.world.entity_by_persistent_id(*id)?)))
            .collect();
        if touched.is_empty() || touched.len() > 2_000 {
            return;
        }
        let entities: Vec<Entity> = touched.iter().map(|(_, e)| *e).collect();
        let now = self.with_isolation_lifted(|engine| engine.snapshots(&entities));
        for (id, _) in touched {
            let changed = now
                .get(&id.to_string())
                .zip(self.editing.baselines.get(&id))
                .is_some_and(|(now, baseline)| *now != baseline.json);
            let saved = self.editing.layer.as_ref().is_some_and(|layer| layer.entry(&id.to_string()).is_some());
            if changed || saved {
                self.editing.edited.insert(id);
            } else {
                self.editing.edited.remove(&id);
            }
        }
    }

    /// Fold this session's changes to the touched entities (all, or `only`) into the layer, and take fresh snapshots
    /// of the ones still alive. `unloading`: they are about to go with their cell, not deleted.
    fn fold_session(&mut self, only: Option<&HashSet<PersistentId>>, unloading: bool) {
        let Some(mut layer) = self.editing.layer.take() else {
            return;
        };
        let ids: Vec<PersistentId> = self
            .editing
            .baselines
            .keys()
            .filter(|id| only.is_none_or(|only| only.contains(id)))
            .copied()
            .collect();
        let alive: Vec<Entity> = ids.iter().filter_map(|id| self.world.entity_by_persistent_id(*id)).collect();
        let now = self.with_isolation_lifted(|engine| engine.snapshots(&alive));
        let gone: HashSet<String> = ids.iter().map(PersistentId::to_string).filter(|id| !now.contains_key(id)).collect();
        for id in ids {
            let hex = id.to_string();
            let Some(baseline) = self.editing.baselines.get_mut(&id) else {
                continue;
            };
            match now.get(&hex) {
                Some(current) => {
                    let added_before = layer.entry(&hex).is_some_and(|entry| entry.op == EditOp::Add);
                    if !self.editing.base.contains(&id) && !added_before {
                        scene_edits::record_add(&mut layer, current);
                    } else {
                        scene_edits::record_change(&mut layer, &baseline.json, current, &baseline.origin);
                    }
                    if unloading {
                        self.editing.baselines.remove(&id);
                    } else {
                        baseline.json = current.clone();
                    }
                }
                None => {
                    let baseline = self.editing.baselines.remove(&id).expect("looked up above");
                    let added = layer.entry(&hex).is_some_and(|entry| entry.op == EditOp::Add);
                    let parent_gone = scene_edits::parent_id_of(&baseline.json).is_some_and(|p| gone.contains(p));
                    if added || !parent_gone {
                        // An addition goes whole; a removal covers everything under it.
                        scene_edits::record_remove(&mut layer, &baseline.json, &baseline.origin);
                    }
                    self.editing.edited.remove(&id);
                }
            }
        }
        for entry in &layer.entities {
            if let Some(id) = PersistentId::parse_hex(&entry.id)
                && entry.op != EditOp::Remove
            {
                self.editing.edited.insert(id);
            }
        }
        self.editing.layer = Some(layer);
    }

    /// Ctrl+S: keep the edits beside the opened scene. A scene with no file of its own goes to Save As.
    pub(super) fn save_scene_edits(&mut self) {
        let Some(scene) = self.editing.scene.clone() else {
            self.save_scene_as();
            return;
        };
        if let Some(error) = self.editing.layer_error.clone() {
            self.toast(&format!(
                "Not saved: your edits file could not be read ({error}). Move it aside, open the scene again, then save."
            ));
            return;
        }
        self.flush_material_assets();
        self.fold_session(None, false);
        let Some(layer) = self.editing.layer.as_ref() else {
            return;
        };
        let count = layer.entities.len();
        match layer.write(&scene) {
            Ok(()) => {
                self.scene_dirty = false;
                self.autosave.saved();
                crate::autosave::clear(&self.config.content_root);
                self.remember_recent_scene(&scene);
                let file = scene_edits::layer_path(&scene);
                let file = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut message = if count == 0 {
                    "Saved: no edits to this scene".to_owned()
                } else {
                    format!("Saved {count} edited object{} to {file}", if count == 1 { "" } else { "s" })
                };
                if self.editing.terrain_touched {
                    message.push_str(" (terrain sculpting is not kept there: File > Save Scene As keeps it)");
                }
                self.toast(&message);
                if let Some(ui) = self.ui_manager.as_mut() {
                    ui.reset_inspector_baseline();
                }
            }
            Err(error) => self.toast(&format!("Save failed: {error}")),
        }
    }

    /// Write the whole scene, edits included, to a file the author picks; it becomes the opened scene.
    pub(super) fn save_scene_as(&mut self) {
        let start = self
            .editing
            .scene
            .as_ref()
            .and_then(|scene| scene.parent().map(Path::to_path_buf))
            .or_else(|| self.config.project_root.as_ref().map(|root| root.join("scenes")).filter(|dir| dir.is_dir()))
            .unwrap_or_else(|| self.config.content_root.clone());
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save Scene As")
            .add_filter("Scene", &["json", "somnium"])
            .set_directory(&start)
            .save_file()
        else {
            return;
        };
        self.flush_material_assets();
        // Cell entries live outside this file and keep applying to the cells.
        self.fold_session(None, false);
        let kept_for_cells: Vec<_> = self
            .editing
            .layer
            .as_ref()
            .map(|layer| layer.entities.iter().filter(|entry| !entry.from.is_empty()).cloned().collect())
            .unwrap_or_default();
        self.end_isolation();
        let text = path.to_string_lossy().into_owned();
        if let Err(error) = crate::scene_schema::save_scene_schema(&mut self.world, &self.type_registry, &text) {
            self.toast(&format!("Save failed: {error}"));
            return;
        }
        self.save_terrain_sidecars(&text);
        // The file now holds every edit: an old layer beside it would apply twice.
        let old = scene_edits::layer_path(&path);
        if old.exists() {
            let aside = old.with_extension("json.replaced");
            let _ = std::fs::remove_file(&aside);
            let _ = std::fs::rename(&old, &aside);
        }
        self.reset_editing(Some(path.clone()));
        self.editing.base = self.world.entities_with::<PersistentId>().filter_map(|e| self.world.persistent_id(e)).collect();
        let mut layer = SceneEdits::new(&path);
        layer.entities = kept_for_cells;
        if !layer.entities.is_empty()
            && let Err(error) = layer.write(&path)
        {
            self.toast(&format!("The scene saved, but its streamed-cell edits did not: {error}"));
        }
        self.editing.layer = Some(layer);
        self.scene_dirty = false;
        self.autosave.saved();
        crate::autosave::clear(&self.config.content_root);
        self.remember_recent_scene(&path);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_scene_name(name.clone());
            ui.reset_inspector_baseline();
        }
        self.toast(&format!("Saved the whole scene to {}", name.unwrap_or_default()));
    }

    /// Heightmap and splatmap sidecars for the scene's own terrains (streamed tiles keep theirs).
    pub(super) fn save_terrain_sidecars(&mut self, path: &str) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let terrain_ids: Vec<u32> = self
            .world
            .iter_with::<TerrainComponent>()
            .filter(|(e, _)| self.world.get::<crate::cell_streaming::Streamed>(*e).is_none())
            .map(|(_, terrain)| terrain.terrain_id)
            .collect();
        for id in terrain_ids {
            if let Some(terrain) = renderer.terrain(id) {
                let sidecar = format!("{path}.terrain{id}.bin");
                match terrain.save_binary(&sidecar) {
                    Ok(()) => info!("Terrain {} data saved to {}", id, sidecar),
                    Err(e) => warn!("Failed to save terrain {}: {}", id, e),
                }
            }
        }
    }

    /// Terrain was sculpted or painted: Save warns that the layer does not hold it.
    pub(super) fn note_terrain_edit(&mut self) {
        self.editing.terrain_touched = true;
    }

    // ─── Picking ────────────────────────────────────────────────────────────

    /// The triangle BVH of a mesh range, read back once and shared by every entity drawing it.
    fn bvh_for(&mut self, mesh: MeshComponent) -> Option<Arc<TriangleBvh>> {
        let key = (mesh.vertex_offset, mesh.index_offset, mesh.index_count);
        if let Some(found) = self.editing.bvhs.get(&key) {
            return found.clone();
        }
        let built = (mesh.index_count / 3 <= BVH_TRIANGLE_LIMIT)
            .then(|| {
                let (renderer, ctx) = (self.renderer.as_ref()?, self.render_ctx.as_ref()?);
                let (vertices, indices) =
                    renderer.read_mesh(ctx, mesh.vertex_offset, mesh.index_offset, mesh.index_count)?;
                let positions: Vec<glam::Vec3> = vertices.iter().map(|v| glam::Vec3::from_array(v.position)).collect();
                Some(Arc::new(TriangleBvh::build(&positions, &indices)))
            })
            .flatten();
        self.editing.bvhs.insert(key, built.clone());
        built
    }

    /// What a plain click on the viewport selects: the nearest surface under the cursor (triangles, not boxes), as
    /// the whole object it belongs to. Clicking the selected object again picks the part.
    pub(super) fn click_pick(&mut self) -> Option<Entity> {
        let (origin, direction) = self.cursor_ray()?;
        self.pick_along(origin, direction)
    }

    /// F4: the object at the centre of the view. In Play, stop first and select it in the restored editor world (by
    /// persistent id), framed, so what you saw while playing is what you fix.
    pub(super) fn select_looked_at(&mut self) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let origin = renderer.camera_pos;
        let centre = renderer.picking_view_proj().inverse().project_point3(glam::Vec3::new(0.0, 0.0, 0.5));
        let direction = (centre - origin).normalize_or_zero();
        let primary = self.selection.primary.take();
        let hit = (direction != glam::Vec3::ZERO).then(|| self.pick_along(origin, direction)).flatten();
        self.selection.primary = primary;
        let id = hit.and_then(|e| self.world.persistent_id(e));
        let name = hit.map(|e| name_of(&self.world, e));
        if self.play_session_active {
            self.handle_editor_event(EditorEvent::StopSimulation);
        }
        let Some(name) = name else {
            self.toast("Nothing to select at the centre of the view (the ground and grass are not objects)");
            return;
        };
        match id.and_then(|id| self.world.entity_by_persistent_id(id)) {
            Some(entity) => {
                self.selection.set_single(Some(entity));
                self.after_selection_change();
                self.focus_camera_on_selection();
                self.reveal_selection_in_outliner(true);
                self.toast(&format!("{name}: selected, the thing you were looking at. End drops it; Ctrl+S keeps it"));
            }
            None => self.toast(&format!("{name} only exists while the game runs (the game made it), so there is nothing to edit")),
        }
    }

    /// The object nearest along a world ray, triangle-accurate, as [`Self::click_pick`] means it.
    fn pick_along(&mut self, origin: glam::Vec3, direction: glam::Vec3) -> Option<Entity> {
        let renderer = self.renderer.as_ref()?;
        let pickable = |world: &World, entity: Entity| {
            world.get::<EditorFlags>(entity).is_none_or(|flags| !flags.locked && !flags.hidden)
        };
        let mut boxes: Vec<(f32, Entity)> = Vec::new();
        let proxies = self
            .world
            .entities_with::<crate::decal::DecalComponent>()
            .chain(self.world.entities_with::<LightComponent>())
            .chain(self.world.entities_with::<AudioEmitterComponent>())
            .chain(self.world.entities_with::<crate::ParticleEmitter>());
        for entity in self.world.entities_with::<MeshComponent>().chain(proxies) {
            if pickable(&self.world, entity)
                && let Some(d2) = entity_ray_hit_distance(&self.world, renderer, entity, origin, direction)
            {
                boxes.push((d2.sqrt(), entity));
            }
        }
        // What the game drew itself (TSF's rooms): picked by the same box-then-triangles test.
        let drawn = drawn_items(&self.world, renderer);
        let mut drawn_meshes: HashMap<Entity, (MeshComponent, glam::Mat4)> = HashMap::new();
        for item in &drawn {
            if !pickable(&self.world, item.entity) {
                continue;
            }
            let inverse = item.model.inverse();
            let (lo, hi) = renderer
                .geometry
                .mesh_aabb(item.mesh.vertex_offset)
                .map_or((glam::Vec3::ZERO, glam::Vec3::ZERO), |(a, b)| (glam::Vec3::from_array(a), glam::Vec3::from_array(b)));
            let local_origin = inverse.transform_point3(origin);
            let local_direction = inverse.transform_vector3(direction);
            if let Some(t) = ray_aabb_distance(local_origin, local_direction, lo, hi) {
                let hit = item.model.transform_point3(local_origin + local_direction * t);
                boxes.push((origin.distance(hit), item.entity));
                drawn_meshes.insert(item.entity, (item.mesh, item.model));
            }
        }
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
        boxes.dedup_by_key(|(_, entity)| *entity);
        let mut best: Option<(f32, Entity)> = None;
        for (box_distance, entity) in boxes {
            if best.is_some_and(|(d, _)| box_distance > d) {
                break;
            }
            let own = self.world.get::<MeshComponent>(entity).copied().zip(model_of(&self.world, entity));
            let exact = if let Some((mesh, model)) = own.or_else(|| drawn_meshes.get(&entity).copied()) {
                match self.bvh_for(mesh) {
                    Some(bvh) => {
                        let surface = MeshSurface {
                            bvh: &bvh,
                            model,
                            inverse: model.inverse(),
                            min: glam::Vec3::splat(f32::NEG_INFINITY),
                            max: glam::Vec3::splat(f32::INFINITY),
                        };
                        surface.raycast(origin, direction, 100_000.0).map(|hit| hit.distance)
                    }
                    None => Some(box_distance),
                }
            } else if self.world.get::<crate::decal::DecalComponent>(entity).is_some() {
                decal_slab_distance(&self.world, entity, origin, direction)
            } else {
                Some(box_distance)
            };
            if let Some(d) = exact
                && best.is_none_or(|(b, _)| d < b)
            {
                best = Some((d, entity));
            }
        }
        Some(self.object_for_click(best?.1))
    }

    /// The object a click on `part` means: its highest ancestor that is still one object (not a container of
    /// hundreds), or the part itself when that object is already selected.
    fn object_for_click(&self, part: Entity) -> Entity {
        let children = children_map(&self.world);
        let mut object = part;
        loop {
            let Some(parent) = self.world.get::<Parent>(object).map(|p| p.entity) else {
                break;
            };
            if parent == Entity::DANGLING || !self.world.is_alive(parent) || subtree(&children, parent).len() > CONTAINER_PARTS
            {
                break;
            }
            object = parent;
        }
        if object != part && self.selection.primary == Some(object) {
            part
        } else {
            object
        }
    }

    /// The selection's objects: each selected entity, minus any under another selected one.
    fn selected_objects(&self) -> Vec<Entity> {
        let selected: HashSet<Entity> = self.selection.as_slice().iter().copied().collect();
        self.selection
            .as_slice()
            .iter()
            .copied()
            .filter(|&entity| {
                let mut current = entity;
                for _ in 0..64 {
                    match self.world.get::<Parent>(current).map(|p| p.entity) {
                        Some(parent) if self.world.is_alive(parent) => {
                            if selected.contains(&parent) {
                                return false;
                            }
                            current = parent;
                        }
                        _ => break,
                    }
                }
                true
            })
            .collect()
    }

    // ─── Ground ─────────────────────────────────────────────────────────────

    /// Live terrains, their models set for ray casts.
    fn prepare_terrains(&mut self) -> Vec<u32> {
        let terrains: Vec<(u32, glam::Mat4)> = self
            .world
            .iter_with::<TerrainComponent>()
            .filter(|(_, terrain)| terrain.terrain_id != u32::MAX)
            .map(|(e, terrain)| {
                (terrain.terrain_id, self.world.get::<Transform>(e).map_or(glam::Mat4::IDENTITY, Transform::to_matrix))
            })
            .collect();
        let Some(renderer) = self.renderer.as_mut() else {
            return Vec::new();
        };
        terrains
            .into_iter()
            .filter_map(|(id, model)| {
                renderer.terrain_mut(id)?.model = model;
                Some(id)
            })
            .collect()
    }

    /// The object's lowest vertices in the world, from its meshes' BVHs; its box's footprint when it has none.
    fn object_base(&mut self, members: &[Entity], min: glam::Vec3, max: glam::Vec3) -> Vec<glam::Vec3> {
        let mut vertices = Vec::new();
        for &member in members {
            let (Some(mesh), Some(model)) = (self.world.get::<MeshComponent>(member).copied(), model_of(&self.world, member))
            else {
                continue;
            };
            if let Some(bvh) = self.bvh_for(mesh) {
                vertices.extend(bvh.corners().map(|corner| model.transform_point3(corner)));
            }
        }
        let base = editor_ground::base_points(&vertices, 9);
        if base.is_empty() { editor_ground::footprint(min, max) } else { base }
    }

    /// What an object with bounds `min..max` and lowest points `base` stands on, leaving `excluded` out.
    fn support_for(
        &mut self,
        base: &[glam::Vec3],
        min: glam::Vec3,
        max: glam::Vec3,
        excluded: &HashSet<Entity>,
        index: &MeshIndex,
        terrains: &[u32],
    ) -> editor_ground::Support {
        let reach = editor_ground::CONTACT + 0.05;
        let column_min = glam::Vec3::new(min.x - reach, min.y - editor_ground::SEARCH_DEPTH, min.z - reach);
        let column_max = max + glam::Vec3::splat(reach);
        let near: Vec<usize> = index.query(column_min, column_max);
        let mut picked: Vec<(Arc<TriangleBvh>, usize)> = Vec::new();
        for i in near {
            let item = &index.items[i];
            if excluded.contains(&item.entity) {
                continue;
            }
            if let Some(bvh) = self.bvh_for(item.mesh) {
                picked.push((bvh, i));
            }
        }
        let meshes = picked
            .iter()
            .map(|(bvh, i)| {
                let item = &index.items[*i];
                MeshSurface { bvh, model: item.model, inverse: item.model.inverse(), min: item.min, max: item.max }
            })
            .collect();
        let renderer = self.renderer.as_ref();
        let terrain = move |origin: glam::Vec3, direction: glam::Vec3, max: f32| {
            renderer.and_then(|renderer| terrain_hit(renderer, terrains, origin, direction, max))
        };
        let terrain: &dyn Fn(glam::Vec3, glam::Vec3, f32) -> Option<Hit> = &terrain;
        let ground = Ground { meshes, terrain: Some(terrain) };
        editor_ground::support_at(&ground, base, min, max)
    }

    /// Drop (and with `align`, first tilt) each selected object onto what is under it, as one undoable step.
    pub(super) fn drop_selection_to_ground(&mut self, align: bool) {
        let objects = self.selected_objects();
        if objects.is_empty() {
            self.toast("Select an object first");
            return;
        }
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        crate::propagate_transforms(&mut self.world);
        let index = MeshIndex::build(&self.world, renderer);
        let terrains = self.prepare_terrains();
        let children = children_map(&self.world);
        let mut commands: Vec<Box<dyn EditorCommand>> = Vec::new();
        let mut moved: Vec<String> = Vec::new();
        let mut left: Vec<String> = Vec::new();
        for object in objects {
            let Some(start) = self.world.get::<Transform>(object).copied() else {
                continue;
            };
            let members = subtree(&children, object);
            let excluded: HashSet<Entity> = members.iter().copied().collect();
            let name = name_of(&self.world, object);
            let Some(renderer) = self.renderer.as_ref() else {
                return;
            };
            let Some((mut min, mut max)) = world_bounds(&self.world, renderer, &members) else {
                left.push(format!("{name} (no mesh)"));
                continue;
            };
            let parent_world = crate::editor_gizmo::parent_world_matrix(&self.world, object);
            let Some(parent_inverse) = crate::editor_gizmo::invert_affine(parent_world) else {
                left.push(format!("{name} (its parent is squashed flat)"));
                continue;
            };
            let mut transform = start;
            if align {
                let base = self.object_base(&members, min, max);
                let support = self.support_for(&base, min, max, &excluded, &index, &terrains);
                if support.gap.is_some() {
                    let world_model = model_of(&self.world, object).unwrap_or(glam::Mat4::IDENTITY);
                    let (_, world_rotation, _) = world_model.to_scale_rotation_translation();
                    let up = world_rotation * glam::Vec3::Y;
                    let tilt = glam::Quat::from_rotation_arc(up, support.normal);
                    let (_, parent_rotation, _) = parent_world.to_scale_rotation_translation();
                    transform.rotation = (parent_rotation.inverse() * tilt * world_rotation).normalize();
                    // Measure the tilted object where it now sits.
                    if let Some(t) = self.world.get_mut::<Transform>(object) {
                        *t = transform;
                    }
                    crate::propagate_transforms(&mut self.world);
                    let renderer = self.renderer.as_ref().expect("checked above");
                    if let Some(bounds) = world_bounds(&self.world, renderer, &members) {
                        (min, max) = bounds;
                    }
                }
            }
            let base = self.object_base(&members, min, max);
            let support = self.support_for(&base, min, max, &excluded, &index, &terrains);
            match editor_ground::drop_offset(&support) {
                Some(offset) => {
                    let world_now = model_of(&self.world, object).unwrap_or(glam::Mat4::IDENTITY).w_axis.truncate();
                    transform.translation = parent_inverse.transform_point3(world_now - glam::Vec3::Y * offset);
                    moved.push(format!("{name} {:+.2} m", -offset));
                }
                None if support.gap.is_none() => left.push(format!("{name} (nothing under it)")),
                None if !align => left.push(format!("{name} (already resting)")),
                None => moved.push(format!("{name} (tilted)")),
            }
            // Put it back; the command applies the change, so undo has both ends.
            if let Some(t) = self.world.get_mut::<Transform>(object) {
                *t = start;
            }
            let unchanged = transform.translation == start.translation
                && transform.rotation == start.rotation
                && transform.scale == start.scale;
            if !unchanged {
                commands.push(Box::new(SetTransformCmd::new(object.index(), start, transform)));
            }
        }
        crate::propagate_transforms(&mut self.world);
        let label = if align { "Align to Surface" } else { "Drop to Ground" };
        if !commands.is_empty() {
            self.undo_stack.push(
                Box::new(CommandGroup::named(label.to_owned(), commands)),
                &mut self.world,
                &mut self.selection.primary,
            );
            self.selection.reconcile();
            self.scene_dirty = true;
            crate::propagate_transforms(&mut self.world);
        }
        let mut message = format!("{label}: ");
        if !moved.is_empty() {
            message.push_str(&summarise(&moved));
        }
        if !left.is_empty() {
            if !moved.is_empty() {
                message.push_str("; ");
            }
            message.push_str("left ");
            message.push_str(&summarise(&left));
        }
        self.toast(&message);
    }

    /// Select every object within `FLOAT_RADIUS` of the camera that hangs in the air, worst first in the log.
    pub(super) fn find_floating_objects(&mut self) {
        let started = std::time::Instant::now();
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let camera = renderer.camera_pos;
        crate::propagate_transforms(&mut self.world);
        let index = MeshIndex::build(&self.world, renderer);
        let terrains = self.prepare_terrains();
        let children = children_map(&self.world);
        let mut objects: Vec<Entity> = index
            .items
            .iter()
            .filter(|item| {
                let centre = (item.min + item.max) * 0.5;
                glam::Vec2::new(centre.x - camera.x, centre.z - camera.z).length() <= FLOAT_RADIUS
            })
            .map(|item| top_ancestor(&self.world, item.entity))
            .collect();
        objects.sort_unstable_by_key(|e| e.index());
        objects.dedup();
        let checked = objects.len();
        let mut found: Vec<(f32, Entity, String)> = Vec::new();
        for object in objects {
            if crate::is_hidden(&self.world, object) {
                continue;
            }
            let members = subtree(&children, object);
            if members.len() > CONTAINER_PARTS {
                continue;
            }
            let Some(renderer) = self.renderer.as_ref() else {
                return;
            };
            let Some((min, max)) = world_bounds(&self.world, renderer, &members) else {
                continue;
            };
            if (max.x - min.x).max(max.z - min.z) > editor_ground::PROP_SPAN {
                continue;
            }
            let excluded: HashSet<Entity> = members.iter().copied().collect();
            let base = self.object_base(&members, min, max);
            let support = self.support_for(&base, min, max, &excluded, &index, &terrains);
            if editor_ground::floats(&support, min, max) {
                found.push((support.gap.unwrap_or_default(), object, name_of(&self.world, object)));
            }
        }
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.editing.floating = found.iter().filter_map(|(_, e, _)| self.world.persistent_id(*e)).collect();
        let ms = started.elapsed().as_millis();
        if found.is_empty() {
            self.toast(&format!("No floating objects within {FLOAT_RADIUS:.0} m ({checked} objects checked, {ms} ms)"));
            return;
        }
        for (gap, object, name) in &found {
            info!(gap = *gap, entity = object.index(), "floating: {name}");
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.append_log(&format!("[floating] {} objects within {FLOAT_RADIUS:.0} m of the camera:", found.len()));
            for (gap, _, name) in found.iter().take(200) {
                ui.append_log(&format!("[floating]   {gap:.2} m  {name}"));
            }
        }
        let (worst_gap, _, worst) = &found[0];
        let message = format!(
            "{} floating object{} selected (worst {worst_gap:.2} m: {worst}). End drops them all; the Outliner's \
             \"floating\" search lists them.",
            found.len(),
            if found.len() == 1 { "" } else { "s" }
        );
        self.selection.set_many(found.iter().map(|(_, e, _)| *e));
        self.after_selection_change();
        self.toast(&message);
    }

    // ─── Hide, isolate, find ────────────────────────────────────────────────

    /// Hide every part of the selected objects, or show them all when they are all hidden. Undoable and saved.
    pub(super) fn toggle_hide_selection(&mut self) {
        let children = children_map(&self.world);
        let members: Vec<Entity> =
            self.selected_objects().into_iter().flat_map(|object| subtree(&children, object)).collect();
        if members.is_empty() {
            return;
        }
        let hide = !members.iter().all(|e| crate::is_hidden(&self.world, *e));
        let commands: Vec<Box<dyn EditorCommand>> = members
            .iter()
            .filter_map(|&entity| {
                let before = self.world.get::<EditorFlags>(entity).copied().unwrap_or_default();
                let after = EditorFlags { hidden: hide, ..before };
                (before != after).then(|| Box::new(SetEditorFlagsCmd::new(entity, before, after)) as Box<dyn EditorCommand>)
            })
            .collect();
        if commands.is_empty() {
            return;
        }
        let label = if hide { "Hide" } else { "Show" };
        self.undo_stack
            .push(Box::new(CommandGroup::named(label.to_owned(), commands)), &mut self.world, &mut self.selection.primary);
        self.selection.reconcile();
        self.scene_dirty = true;
        self.toast(&format!("{label} {} part{}", members.len(), if members.len() == 1 { "" } else { "s" }));
    }

    /// Hide every mesh and decal outside the selected objects until Show Everything. Pressed again, it ends.
    pub(super) fn isolate_selection(&mut self) {
        if !self.editing.isolated.is_empty() {
            self.end_isolation();
            return;
        }
        let children = children_map(&self.world);
        let mut keep: HashSet<Entity> = HashSet::new();
        for object in self.selected_objects() {
            keep.extend(subtree(&children, object));
        }
        if keep.is_empty() {
            return;
        }
        let others: Vec<Entity> = self
            .world
            .entities_with::<MeshComponent>()
            .chain(self.world.entities_with::<crate::decal::DecalComponent>())
            .filter(|e| !keep.contains(e) && !crate::is_hidden(&self.world, *e))
            .collect();
        for entity in others {
            let before = self.world.get::<EditorFlags>(entity).copied();
            self.editing.isolated.push((entity, before));
        }
        self.apply_isolation();
        self.toast("Isolated: Alt+H (or Shift+H again) shows everything. Isolation is never saved.");
    }

    fn apply_isolation(&mut self) {
        for (entity, before) in &self.editing.isolated {
            let flags = EditorFlags { hidden: true, ..before.unwrap_or_default() };
            if let Some(current) = self.world.get_mut::<EditorFlags>(*entity) {
                *current = flags;
            } else if self.world.is_alive(*entity) {
                let _ = self.world.insert_component(*entity, flags);
            }
        }
    }

    fn restore_isolated(&mut self) {
        for (entity, before) in &self.editing.isolated {
            if !self.world.is_alive(*entity) {
                continue;
            }
            match before {
                Some(flags) => {
                    if let Some(current) = self.world.get_mut::<EditorFlags>(*entity) {
                        *current = *flags;
                    }
                }
                None => {
                    let _ = self.world.remove_component::<EditorFlags>(*entity);
                }
            }
        }
    }

    /// Show what Isolate hid. Also runs before Play and before any whole-scene save.
    pub(super) fn end_isolation(&mut self) {
        if self.editing.isolated.is_empty() {
            return;
        }
        self.restore_isolated();
        self.editing.isolated.clear();
    }

    /// Run `f` with the isolation flags put back, so snapshots never record them.
    fn with_isolation_lifted<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if self.editing.isolated.is_empty() {
            return f(self);
        }
        self.restore_isolated();
        let result = f(self);
        self.apply_isolation();
        result
    }

    /// Scroll the Outliner to the primary selection and open the rows above it.
    pub(super) fn reveal_selection_in_outliner(&mut self, quiet: bool) {
        let Some(entity) = self.selection.primary else {
            return;
        };
        let mut ancestors = Vec::new();
        let mut current = entity;
        for _ in 0..64 {
            match self.world.get::<Parent>(current).map(|p| p.entity) {
                Some(parent) if self.world.is_alive(parent) => {
                    ancestors.push(parent.index());
                    current = parent;
                }
                _ => break,
            }
        }
        let shown = self.ui_manager.as_mut().is_some_and(|ui| ui.reveal_outliner_row(entity.index(), &ancestors));
        if !shown && !quiet {
            self.toast("The Outliner's search hides it: clear the search to see it there");
        }
    }

    /// Select the primary's parent.
    pub(super) fn select_parent(&mut self) {
        let parent = self
            .selection
            .primary
            .and_then(|e| self.world.get::<Parent>(e).map(|p| p.entity))
            .filter(|p| self.world.is_alive(*p));
        match parent {
            Some(parent) => {
                self.selection.set_single(Some(parent));
                self.after_selection_change();
                self.reveal_selection_in_outliner(true);
            }
            None => self.toast("It is already at the top"),
        }
    }

    // ─── The selection card ─────────────────────────────────────────────────

    /// What the primary selection is, where it came from and what can be done to it, several lines.
    pub(super) fn selection_card(&mut self) -> Option<String> {
        let entity = self.selection.primary.filter(|e| self.world.is_alive(*e))?;
        let id = self.world.persistent_id(entity);
        let key = CardKey {
            entity,
            generation: self.undo_stack.generation(),
            count: self.selection.len(),
            edited: id.is_some_and(|id| self.editing.edited.contains(&id)),
            dirty: self.scene_dirty,
        };
        if let Some((cached, card)) = &self.editing.card
            && *cached == key
        {
            return Some(card.clone());
        }
        let card = self.build_card(entity);
        info!("selection card:
{card}");
        self.editing.card = Some((key, card.clone()));
        Some(card)
    }

    /// `SOMNIUM_EDITOR_SCRIPT`: scripted editor input for proof captures in a background window, inert unless set.
    /// Steps are `<frame>:<action>`, separated by `;`: `pick=<x>,<y>` (a click at that fraction of the window),
    /// `select=<name part>` (the first entity whose name contains it), `cmd=<command id>`, `save`.
    pub(super) fn run_editor_script(&mut self) {
        static SCRIPT: std::sync::LazyLock<Vec<(u64, String)>> = std::sync::LazyLock::new(|| {
            std::env::var("SOMNIUM_EDITOR_SCRIPT")
                .unwrap_or_default()
                .split(';')
                .filter_map(|step| {
                    let (frame, action) = step.trim().split_once(':')?;
                    Some((frame.trim().parse().ok()?, action.trim().to_owned()))
                })
                .collect()
        });
        if SCRIPT.is_empty() {
            return;
        }
        self.editing.script_frame += 1;
        let frame = self.editing.script_frame;
        for (_, action) in SCRIPT.iter().filter(|(at, _)| *at == frame) {
            info!(frame, action = %action, "editor script");
            match action.split_once('=') {
                Some(("pick", at)) => {
                    let Some((x, y)) = at.split_once(',').and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?)))
                    else {
                        warn!(action = %action, "editor script: pick wants x,y fractions");
                        continue;
                    };
                    let (width, height) = self.viewport_size();
                    self.cursor_pos = (x * width, y * height);
                    let hit = self.click_pick();
                    info!(picked = %hit.map_or_else(|| "nothing".to_owned(), |e| name_of(&self.world, e)), "editor script");
                    self.selection.set_single(hit);
                    self.after_selection_change();
                    self.reveal_selection_in_outliner(true);
                }
                Some(("select", part)) => {
                    let found = self.world.iter_with::<Name>().find(|(_, name)| name.as_str().contains(part)).map(|(e, _)| e);
                    if found.is_none() {
                        warn!(part, "editor script: nothing by that name");
                    }
                    self.selection.set_single(found);
                    self.after_selection_change();
                    self.reveal_selection_in_outliner(true);
                }
                Some(("cmd", id)) => {
                    let ran = self.ui_manager.as_mut().is_some_and(|ui| ui.run_command_id(id));
                    if !ran {
                        warn!(id, "editor script: no such command, or it is disabled");
                    }
                }
                None if action == "save" => self.handle_editor_event(EditorEvent::SaveScene),
                _ => warn!(action = %action, "editor script: unknown step"),
            }
        }
    }

    fn build_card(&mut self, entity: Entity) -> String {
        let root = self.config.project_root.clone();
        let mut lines: Vec<String> = Vec::new();
        let object = top_ancestor(&self.world, entity);
        let name = name_of(&self.world, entity);
        let id = self.world.persistent_id(entity);
        let mut title = if object == entity {
            name.clone()
        } else {
            format!("{}  \u{203a}  {name}", name_of(&self.world, object))
        };
        if self.selection.len() > 1 {
            title.push_str(&format!("   (+{} more selected)", self.selection.len() - 1));
        }
        if id.is_some_and(|id| self.editing.edited.contains(&id)) {
            title.push_str("   [edited]");
        }
        if id.is_some_and(|id| self.editing.floating.contains(&id)) {
            title.push_str("   [floating]");
        }
        lines.push(title);

        // Mesh, its size, and what it is made of.
        if let Some(imported) = self.world.get::<crate::ImportedMesh>(entity).cloned() {
            let mut line = format!("mesh      {}  part {}", imported.source, imported.node);
            if let Some(size) = self.size_of(entity) {
                line.push_str(&format!("  ({size})"));
            }
            lines.push(line);
            if let Some(part) = self.gltf_part(&imported.source, imported.node as usize) {
                if !part.node.is_empty() || !part.mesh.is_empty() {
                    lines.push(format!("node      \"{}\"  mesh \"{}\"", part.node, part.mesh));
                }
                let material = self.material_label(entity).unwrap_or_else(|| format!("\"{}\" (from the model)", part.material));
                lines.push(format!("material  {material}"));
                if !part.images.is_empty() {
                    lines.push(format!("textures  {}", part.images.join(", ")));
                }
            } else if let Some(material) = self.material_label(entity) {
                lines.push(format!("material  {material}"));
            }
        } else if self.world.get::<MeshComponent>(entity).is_some() {
            let kind = self.world.get::<MeshKind>(entity).map_or("mesh".to_owned(), |kind| format!("{kind:?}"));
            let size = self.size_of(entity).map(|s| format!("  ({s})")).unwrap_or_default();
            lines.push(format!("mesh      built-in {kind}{size}"));
            if let Some(material) = self.material_label(entity) {
                lines.push(format!("material  {material}"));
            }
        } else {
            let children = children_map(&self.world);
            let members = subtree(&children, entity);
            let parts = members.len() - 1;
            // What the parts draw: the asset files an object is made of.
            let mut sources: Vec<String> = Vec::new();
            for member in &members {
                if let Some(imported) = self.world.get::<crate::ImportedMesh>(*member)
                    && !sources.contains(&imported.source)
                {
                    sources.push(imported.source.clone());
                }
            }
            match sources.as_slice() {
                [] => {}
                [one] => lines.push(format!("mesh      {one}")),
                [first, rest @ ..] => lines.push(format!("mesh      {first} and {} more file{}", rest.len(), if rest.len() == 1 { "" } else { "s" })),
            }
            let size = self
                .renderer
                .as_ref()
                .and_then(|r| world_bounds(&self.world, r, &members))
                .map(|(lo, hi)| {
                    let s = hi - lo;
                    format!(", {:.2} x {:.2} x {:.2} m", s.x, s.y, s.z)
                })
                .unwrap_or_default();
            lines.push(format!("object    {parts} part{}{size}", if parts == 1 { "" } else { "s" }));
        }

        // Where it came from.
        let streamed = self.world.get::<crate::cell_streaming::Streamed>(entity).copied();
        let file = match streamed.and_then(|s| self.cell_streamer.cells.get(s.cell as usize)) {
            Some(cell) => format!("cell {}", project_relative(root.as_deref(), &cell.file)),
            None => self
                .editing
                .scene
                .as_ref()
                .map(|scene| project_relative(root.as_deref(), scene))
                .unwrap_or_else(|| "(a scene with no file yet)".to_owned()),
        };
        let added = id.is_some_and(|id| {
            self.editing.layer.as_ref().and_then(|l| l.entry(&id.to_string())).is_some_and(|e| e.op == EditOp::Add)
                || (!self.editing.base.contains(&id) && streamed.is_none())
        });
        lines.push(if added { "file      added in the editor (kept in the edits file)".to_owned() } else { format!("file      {file}") });
        if let Some(made) = self.made_by_for(entity, &file) {
            lines.push(format!("made by   {made}"));
        }
        lines.push(format!(
            "id        entity {}  persistent {}",
            entity.index(),
            id.map_or_else(|| "(none yet)".to_owned(), |id| id.to_string())
        ));
        if let Some(line) = self.edit_line(entity) {
            lines.push(line);
        }
        lines.push("End drop to ground   Shift+End align   H hide   Shift+H isolate   Ctrl+Shift+F floating".to_owned());
        lines.join("\n")
    }

    fn size_of(&self, entity: Entity) -> Option<String> {
        let renderer = self.renderer.as_ref()?;
        let (lo, hi) = world_bounds(&self.world, renderer, &[entity])?;
        let s = hi - lo;
        Some(format!("{:.2} x {:.2} x {:.2} m", s.x, s.y, s.z))
    }

    fn material_label(&self, entity: Entity) -> Option<String> {
        let material = self.world.get::<MaterialComponent>(entity)?;
        if material.asset.raw() == 0 {
            return None;
        }
        let record = self.asset_gate.published().and_then(|db| db.get(material.asset));
        Some(record.map_or_else(|| format!("asset {:032x} (not in the content folder)", material.asset.raw()), |r| r.relative_path.clone()))
    }

    fn gltf_part(&mut self, source: &str, node: usize) -> Option<somnium_asset::GltfPart> {
        if !self.editing.parts.contains_key(source) {
            let parts = self
                .imported_source_path(source)
                .ok()
                .and_then(|path| somnium_asset::describe_gltf_parts(&path).ok())
                .map(Arc::new);
            self.editing.parts.insert(source.to_owned(), parts);
        }
        self.editing.parts.get(source)?.as_ref()?.get(node).cloned()
    }

    /// Who made it: a cell's per-object label, the file's own `made_by`, or the tools that write its name's prefix.
    fn made_by_for(&mut self, entity: Entity, file: &str) -> Option<String> {
        let object = top_ancestor(&self.world, entity);
        for candidate in [entity, object] {
            if let Some(label) = self.world.persistent_id(candidate).and_then(|id| self.editing.sources.get(&id)) {
                return Some(label.clone());
            }
        }
        let file = file.trim_start_matches("cell ");
        if let Some(label) = self.editing.made_by.get(file) {
            return Some(label.clone());
        }
        let name = name_of(&self.world, object);
        let prefix = name.split(" / ").next().filter(|p| p.len() >= 4 && *p != name)?.to_owned();
        if !self.editing.prefix_tools.contains_key(&prefix) {
            let tools = self.tools_writing(&prefix);
            self.editing.prefix_tools.insert(prefix.clone(), tools);
        }
        let tools = self.editing.prefix_tools.get(&prefix)?;
        match tools.as_slice() {
            [] => None,
            [one] => Some(format!("{one} (it names its objects \"{prefix} / ...\")")),
            [first, rest @ ..] => Some(format!(
                "{first} (it names its objects \"{prefix} / ...\"; {} more tool{} touch them)",
                rest.len(),
                if rest.len() == 1 { "" } else { "s" }
            )),
        }
    }

    /// The project's tools whose source mentions `prefix`, likeliest maker first: a tool that sets the prefix as a
    /// constant (`TAG = 'Furnish 20261005'`), then one whose file name shares a word with it, then the oldest, since
    /// the maker predates the fixers and audits that mention its objects later.
    fn tools_writing(&self, prefix: &str) -> Vec<String> {
        let Some(root) = self.config.project_root.as_ref() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(root.join("tools")) else {
            return Vec::new();
        };
        let mut found: Vec<(u8, std::time::SystemTime, String)> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "py"))
            .filter_map(|entry| {
                let text = std::fs::read_to_string(entry.path()).ok()?;
                let stem = entry.path().file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let score = maker_score(prefix, &stem, &text)?;
                let when = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                Some((score, when, format!("tools/{}", entry.file_name().to_string_lossy())))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found.into_iter().map(|(_, _, name)| name).collect()
    }

    /// What has been done to it: saved in the layer, or changed since it was first selected.
    fn edit_line(&mut self, entity: Entity) -> Option<String> {
        let id = self.world.persistent_id(entity)?;
        let hex = id.to_string();
        let saved = self.editing.layer.as_ref().and_then(|layer| layer.entry(&hex)).cloned();
        let now = self.world.get::<Transform>(entity).map(|t| t.translation);
        let unsaved = self.editing.edited.contains(&id) && self.scene_dirty;
        let file = self
            .editing
            .scene
            .as_ref()
            .map(|s| scene_edits::layer_path(s))
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        match (saved, unsaved) {
            (Some(entry), _) if entry.op == EditOp::Change => {
                let moved = entry.was.zip(now).map(|(was, now)| (glam::Vec3::from_array(was) - now).length());
                let what = moved.filter(|m| *m > 0.005).map_or_else(|| "changed".to_owned(), |m| format!("moved {m:.2} m"));
                Some(format!("edits     {what} since the generator made it; kept in {file}{}", if unsaved { " (more unsaved)" } else { "" }))
            }
            (Some(_), _) => Some(format!("edits     added in the editor; kept in {file}")),
            (None, true) => Some("edits     changed, not saved yet: Ctrl+S keeps it beside the scene".to_owned()),
            (None, false) => None,
        }
    }

    /// The Outliner row's marks: `edited`, `floating`, and the asset path `asset:` searches.
    pub(super) fn decorate_outliner_row(&self, entity: Entity, row: &mut somnium_ui::OutlinerRow) {
        if let Some(id) = self.world.persistent_id(entity) {
            if self.editing.edited.contains(&id) {
                row.tags.push("edited");
                row.name.push_str("  [edited]");
            }
            if self.editing.floating.contains(&id) {
                row.tags.push("floating");
            }
        }
        row.asset.clear();
        if let Some(imported) = self.world.get::<crate::ImportedMesh>(entity) {
            row.asset.push_str(&imported.source);
        }
    }

    /// Put the card on the clipboard.
    pub(super) fn copy_identity(&mut self) {
        match self.selection_card() {
            Some(card) => match copy_to_clipboard(&card) {
                Ok(()) => self.toast("Copied what it is"),
                Err(error) => self.toast(&format!("Could not copy: {error}")),
            },
            None => self.toast("Select something first"),
        }
    }
}

/// Every persistent id in a scene document.
fn ids_in(document: &Value) -> HashSet<PersistentId> {
    document["entities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entity| PersistentId::parse_hex(entity.get("persistent_id")?.as_str()?))
        .collect()
}

/// A document's own note of the tool that wrote it.
fn made_by(document: &Value) -> Option<String> {
    [&document["made_by"], &document["generated_by"], &document["rimeborn"]["made_by"]]
        .into_iter()
        .find_map(|value| value.as_str().map(str::to_owned))
}

/// How likely a tool (file `stem`, source `text`) is to be the one that makes objects named `prefix / ...`: 2 for
/// setting it as a constant (`TAG = 'Furnish 20261005'`), 1 for a file name sharing a word with it; `None` when it
/// never mentions it.
fn maker_score(prefix: &str, stem: &str, text: &str) -> Option<u8> {
    if !text.contains(prefix) {
        return None;
    }
    let sets_it = text.lines().any(|line| {
        let Some((name, value)) = line.trim_start().split_once('=') else {
            return false;
        };
        let name = name.trim();
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && value.trim_start().trim_start_matches('f').trim_start_matches(['\'', '"']).starts_with(prefix)
    });
    let stem = stem.to_ascii_lowercase();
    let named = prefix
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| word.len() >= 4)
        .any(|word| stem.contains(&word.to_ascii_lowercase()));
    Some(u8::from(sets_it) * 2 + u8::from(named))
}

/// A short list for a toast: the first few, then a count.
fn summarise(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(3).map(String::as_str).collect();
    let mut text = shown.join(", ");
    if items.len() > 3 {
        text.push_str(&format!(" and {} more", items.len() - 3));
    }
    text
}

/// A decal is clicked on its own surface, not anywhere in its projection box: the box is thinned to 4 cm along its
/// shortest axis.
fn decal_slab_distance(world: &World, entity: Entity, origin: glam::Vec3, direction: glam::Vec3) -> Option<f32> {
    let model = model_of(world, entity)?;
    let (scale, _, _) = model.to_scale_rotation_translation();
    let thin = if scale.x <= scale.y && scale.x <= scale.z {
        0
    } else if scale.y <= scale.z {
        1
    } else {
        2
    };
    let mut half = glam::Vec3::splat(0.5);
    half[thin] = (0.02 / scale[thin].abs().max(1e-4)).min(0.5);
    let inverse = invert_affine(model)?;
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);
    let t = ray_aabb_distance(local_origin, local_direction, -half, half)?;
    Some(origin.distance(model.transform_point3(local_origin + local_direction * t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_path_reads_the_same_however_it_was_joined() {
        let root = Path::new("F:/Somnium Games/rimeborn-fp");
        let joined = Path::new("F:/Somnium Games/rimeborn-fp/scenes/sunscar_cells/../terrain_tiles/t_0_0.scene.json");
        assert_eq!(project_relative(Some(root), joined), "scenes/terrain_tiles/t_0_0.scene.json");
        let cell = Path::new("F:/Somnium Games/rimeborn-fp/scenes/sunscar_cells/s_-64_9.scene.json");
        assert_eq!(project_relative(Some(root), cell), "scenes/sunscar_cells/s_-64_9.scene.json");
    }

    #[test]
    fn the_tool_that_names_a_prefix_is_its_maker_not_the_ones_that_mention_it() {
        let maker = "import json\nTAG = 'Furnish 20261005'\n";
        let fixer = "names = ('Furnish 20261005 / Design studio / chair',)\n";
        assert_eq!(maker_score("Furnish 20261005", "furnish_20261005", maker), Some(3));
        assert_eq!(maker_score("Furnish 20261005", "reach_fix_20261005", fixer), Some(1), "the date in its name");
        assert_eq!(maker_score("Furnish 20261005", "forest_o_20261029", fixer), Some(0));
        assert_eq!(maker_score("Furnish 20261005", "anything", "nothing here"), None);
    }

    #[test]
    fn a_toast_list_stays_short() {
        let items: Vec<String> = (0..5).map(|i| format!("crate {i}")).collect();
        assert_eq!(summarise(&items), "crate 0, crate 1, crate 2 and 2 more");
    }

    #[test]
    fn made_by_reads_each_place_tools_write_it() {
        let places = serde_json::json!({"entities": [], "rimeborn": {"made_by": "rimeborn-fp/tools/arch_montara.py"}});
        assert_eq!(made_by(&places).as_deref(), Some("rimeborn-fp/tools/arch_montara.py"));
        assert_eq!(made_by(&serde_json::json!({"made_by": "tools/compose.py"})).as_deref(), Some("tools/compose.py"));
        assert_eq!(made_by(&serde_json::json!({"entities": []})), None);
    }

    #[test]
    fn a_decal_is_clicked_on_its_face_not_in_front_of_it() {
        let mut world = World::new();
        // A 2 x 2 m decal projecting 1 m deep along z, at the origin.
        let decal = world.spawn((Transform {
            translation: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::new(2.0, 2.0, 1.0),
        },));
        let d = decal_slab_distance(&world, decal, glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::NEG_Z).unwrap();
        assert!((d - 4.98).abs() < 1e-3, "{d}: the slab's face, 2 cm in front of the plane, not the box's at 0.5 m");
    }
}
