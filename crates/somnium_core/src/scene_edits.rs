//! Edit layers: the changes someone makes in the editor to a scene a tool generated, kept beside the scene and
//! applied every time it loads.
//!
//! `<scene>.edits.json` lists moves (any changed field), deletions and additions, keyed by persistent id. The engine
//! applies the layer to the scene's document before anything spawns, in the editor and in the game alike, and a
//! streamed cell gets the entries for its entities as it installs. A generator can therefore rewrite the scene as
//! often as it likes: it never reads or writes the layer, and the edits land again on the next load.
//!
//! Each entry carries a fingerprint (name, mesh, parent's name and the position the generator gave it). An id a
//! generator re-used for a different object is not edited by mistake, and an object whose id changed is found again
//! by its fingerprint. Whatever cannot be placed is reported and kept, never dropped.

use crate::scene_delta::{self, ValuePatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The layer file's `format` value.
pub const FORMAT: &str = "somnium-scene-edits";
/// An entity whose generated position is further than this from the one recorded is a different object.
pub const SAME_OBJECT_RADIUS: f32 = 5.0;
/// A lost id is found again only by a fingerprint within this distance of the recorded position.
pub const REMATCH_RADIUS: f32 = 2.0;

/// `<scene>.edits.json`, beside the scene.
#[must_use]
pub fn layer_path(scene: &Path) -> PathBuf {
    let mut name = scene.as_os_str().to_owned();
    name.push(".edits.json");
    PathBuf::from(name)
}

/// One scene's edits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneEdits {
    /// Always [`FORMAT`].
    pub format: String,
    /// 1.
    pub version: u32,
    /// The scene's file name, for a person reading the layer.
    pub scene: String,
    /// What the file is, for a person who opens it.
    #[serde(default)]
    pub note: String,
    /// One entry per edited entity.
    #[serde(default)]
    pub entities: Vec<EditEntry>,
}

impl SceneEdits {
    /// An empty layer for `scene`.
    #[must_use]
    pub fn new(scene: &Path) -> Self {
        let name = scene.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self {
            format: FORMAT.into(),
            version: 1,
            note: format!(
                "Editor changes to {name}, applied after it loads, in the editor and in the game. Generator scripts \
                 never write this file; delete an entry to put that object back the way the generator made it."
            ),
            scene: name,
            entities: Vec::new(),
        }
    }

    /// The layer beside `scene`, if there is one.
    ///
    /// # Errors
    /// A layer that exists but cannot be read or parsed. It is never treated as empty, or the next save would
    /// overwrite the work in it.
    pub fn read(scene: &Path) -> Result<Option<Self>, String> {
        let path = layer_path(scene);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let layer: Self = serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        if layer.format != FORMAT {
            return Err(format!("{}: not a {FORMAT} file", path.display()));
        }
        Ok(Some(layer))
    }

    /// Write the layer beside `scene`, or move an empty one aside so a scene without edits has no layer.
    ///
    /// # Errors
    /// The file could not be written.
    pub fn write(&self, scene: &Path) -> Result<(), String> {
        let path = layer_path(scene);
        if self.entities.is_empty() {
            if path.exists() {
                // Kept, renamed: the house rule is that nothing is hard-deleted.
                let aside = path.with_extension("json.cleared");
                let _ = std::fs::remove_file(&aside);
                std::fs::rename(&path, &aside).map_err(|error| format!("{}: {error}", path.display()))?;
            }
            return Ok(());
        }
        let mut sorted = self.clone();
        sorted.entities.sort_by(|a, b| a.id.cmp(&b.id));
        let text = serde_json::to_string_pretty(&sorted).map_err(|error| error.to_string())?;
        let temporary = path.with_extension("json.writing");
        std::fs::write(&temporary, text + "\n").map_err(|error| format!("{}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &path).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The entry for `id`.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&EditEntry> {
        self.entities.iter().find(|entry| entry.id == id)
    }
}

/// What an entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditOp {
    /// Fields changed (a move is a change to `somnium.Transform`).
    Change,
    /// The entity and everything under it removed.
    Remove,
    /// A new entity.
    Add,
}

/// One edited entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditEntry {
    /// Persistent id, 32 hex digits.
    pub id: String,
    /// Change, remove or add.
    pub op: EditOp,
    /// The entity's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// `source#node` for an imported mesh, the primitive's kind otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mesh: String,
    /// The parent's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// The local position the generator gave it, before any edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was: Option<[f32; 3]>,
    /// The streamed cell it loaded from, relative to the project; empty for the scene itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// The changes, applied in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<ValuePatch>,
    /// The whole entity, for an addition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<Value>,
}

// ─── Reading an entity document ─────────────────────────────────────────────

fn field<'a>(entity: &'a Value, component: &str, name: &str) -> Option<&'a Value> {
    entity.get("components")?.get(component)?.get("fields")?.get(name)
}

/// The entity's `somnium.Name`.
#[must_use]
pub fn name_of(entity: &Value) -> &str {
    field(entity, "somnium.Name", "value").and_then(Value::as_str).unwrap_or("")
}

/// `source#node` for an imported mesh, the primitive's kind for a built-in one, empty otherwise.
#[must_use]
pub fn mesh_of(entity: &Value) -> String {
    if let Some(source) = field(entity, "somnium.ImportedMesh", "source").and_then(Value::as_str) {
        let node = field(entity, "somnium.ImportedMesh", "node").and_then(Value::as_u64).unwrap_or(0);
        return format!("{source}#{node}");
    }
    entity
        .get("components")
        .and_then(|c| c.get("somnium.MeshKind"))
        .and_then(|kind| kind.get("fields"))
        .map(|fields| fields.to_string())
        .unwrap_or_default()
}

/// The entity's local translation.
#[must_use]
pub fn translation_of(entity: &Value) -> Option<[f32; 3]> {
    let list = field(entity, "somnium.Transform", "translation")?.as_array()?;
    let at = |i: usize| list.get(i).and_then(Value::as_f64).map(|v| v as f32);
    Some([at(0)?, at(1)?, at(2)?])
}

/// The persistent id of the entity's parent.
#[must_use]
pub fn parent_id_of(entity: &Value) -> Option<&str> {
    field(entity, "somnium.Parent", "entity")?.get("$entity")?.as_str()
}

fn id_of(entity: &Value) -> Option<&str> {
    entity.get("persistent_id").and_then(Value::as_str)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

// ─── Applying ───────────────────────────────────────────────────────────────

/// Which entries a document gets.
#[derive(Clone, Copy, Debug)]
pub enum Scope<'a> {
    /// The opened scene: its own entries and every addition.
    Scene,
    /// A streamed cell (its path relative to the project): the entries recorded against it, and any entry whose
    /// entity a re-composed world moved into it. Never additions.
    Cell(&'a str),
}

/// What [`apply`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Ids now changed in the document.
    pub changed: Vec<String>,
    /// Ids removed, each with everything under it.
    pub removed: Vec<String>,
    /// Ids added.
    pub added: Vec<String>,
    /// `(recorded id, id found by fingerprint)`; the layer has been re-keyed.
    pub rematched: Vec<(String, String)>,
    /// Edited objects the generator has also moved since; the edit still won.
    pub conflicts: Vec<String>,
    /// `(id, why)`: entries that could not be placed. They stay in the layer.
    pub orphaned: Vec<(String, String)>,
}

impl Applied {
    /// One line for a person: what happened, and what needs them.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let done = self.changed.len() + self.removed.len() + self.added.len();
        if done > 0 {
            parts.push(format!("{done} edit{} applied", if done == 1 { "" } else { "s" }));
        }
        if !self.rematched.is_empty() {
            parts.push(format!("{} found again after the generator renamed them", self.rematched.len()));
        }
        if !self.conflicts.is_empty() {
            parts.push(format!("{} also moved by the generator (yours kept)", self.conflicts.len()));
        }
        if !self.orphaned.is_empty() {
            parts.push(format!("{} could not be matched (kept in the edits file)", self.orphaned.len()));
        }
        parts.join(", ")
    }

}

struct Index {
    by_id: HashMap<String, usize>,
    names: Vec<String>,
    children: HashMap<String, Vec<usize>>,
}

impl Index {
    fn of(entities: &[Value]) -> Self {
        let by_id = entities
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((id_of(e)?.to_owned(), i)))
            .collect();
        let names = entities.iter().map(|e| name_of(e).to_owned()).collect();
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in entities.iter().enumerate() {
            if let Some(parent) = parent_id_of(e) {
                children.entry(parent.to_owned()).or_default().push(i);
            }
        }
        Self { by_id, names, children }
    }

    fn parent_name(&self, entity: &Value) -> String {
        parent_id_of(entity)
            .and_then(|id| self.by_id.get(id))
            .map(|&i| self.names[i].clone())
            .unwrap_or_default()
    }

    /// Every index at or under `root`.
    fn subtree(&self, entities: &[Value], root: usize) -> Vec<usize> {
        let mut found = vec![root];
        let mut next = 0;
        while next < found.len() {
            if let Some(kids) = id_of(&entities[found[next]]).and_then(|id| self.children.get(id)) {
                found.extend(kids.iter().copied().filter(|kid| *kid != root));
            }
            next += 1;
        }
        found
    }
}

/// Why `entity` is not the object `entry` was recorded against, or `None` when it is.
fn mismatch(entry: &EditEntry, entity: &Value, parent: &str) -> Option<String> {
    if !entry.name.is_empty() && name_of(entity) != entry.name {
        return Some(format!("its id now names \"{}\", not \"{}\"", name_of(entity), entry.name));
    }
    let mesh = mesh_of(entity);
    if !entry.mesh.is_empty() && !mesh.is_empty() && mesh != entry.mesh {
        return Some(format!("its id now names a different mesh ({mesh})"));
    }
    if !entry.parent.is_empty() && !parent.is_empty() && parent != entry.parent {
        return Some(format!("its id now sits under \"{parent}\", not \"{}\"", entry.parent));
    }
    None
}

/// The one entity matching `entry`'s fingerprint, nearest its recorded position. Ambiguity is a miss: editing the
/// wrong one of two identical chairs is worse than asking.
fn rematch(
    entry: &EditEntry,
    entities: &[Value],
    index: &Index,
    claimed: &HashSet<String>,
    radius: f32,
) -> Option<usize> {
    if entry.name.is_empty() || entry.op == EditOp::Add {
        return None;
    }
    let mut best: Vec<(f32, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, e)| name_of(e) == entry.name)
        .filter(|(_, e)| id_of(e).is_some_and(|id| !claimed.contains(id)))
        .filter(|(_, e)| entry.mesh.is_empty() || mesh_of(e) == entry.mesh)
        .filter(|(_, e)| entry.parent.is_empty() || index.parent_name(e) == entry.parent)
        .filter_map(|(i, e)| match (entry.was, translation_of(e)) {
            (Some(was), Some(now)) => (distance(was, now) <= radius).then_some((distance(was, now), i)),
            (None, _) => Some((0.0, i)),
            (Some(_), None) => None,
        })
        .collect();
    best.sort_by(|a, b| a.0.total_cmp(&b.0));
    match best.as_slice() {
        [only] => Some(only.1),
        [first, second, ..] if second.0 - first.0 > 0.01 => Some(first.1),
        _ => None,
    }
}

/// The entity `entry` means: its own id while that still names the same object, else the nearest entity with the
/// same fingerprint. Generators that number their objects can hand one id to the next object of the same name, so
/// an id whose entity has wandered is only trusted when no same-named entity sits nearer the recorded position.
fn resolve(
    entry: &EditEntry,
    present: Option<usize>,
    entities: &[Value],
    index: &Index,
    claimed: &HashSet<String>,
) -> Result<usize, String> {
    let Some(i) = present else {
        return rematch(entry, entities, index, claimed, REMATCH_RADIUS)
            .ok_or_else(|| "it is no longer in the scene".to_owned());
    };
    if let Some(why) = mismatch(entry, &entities[i], &index.parent_name(&entities[i])) {
        return rematch(entry, entities, index, claimed, REMATCH_RADIUS).ok_or(why);
    }
    let (Some(was), Some(now)) = (entry.was, translation_of(&entities[i])) else {
        return Ok(i);
    };
    let wandered = distance(was, now);
    if wandered <= 0.05 {
        return Ok(i);
    }
    // Something of the same kind nearer where it was is the object; this id was handed on.
    if let Some(nearer) = rematch(entry, entities, index, claimed, wandered - 0.01)
        && nearer != i
    {
        return Ok(nearer);
    }
    if wandered <= SAME_OBJECT_RADIUS {
        Ok(i)
    } else {
        Err(format!("the generator put it {wandered:.1} m from where it was"))
    }
}

/// Apply `layer` to a scene document (`{"version", "entities": [...]}`), in place, before it spawns.
///
/// Entries found by fingerprint are re-keyed in `layer`, and a cell entry's `from` follows its entity to the cell
/// that holds it now; the caller writes the layer back when it next saves.
pub fn apply(document: &mut Value, layer: &mut SceneEdits, scope: Scope<'_>) -> Applied {
    let mut applied = Applied::default();
    let Some(entities) = document.get_mut("entities").and_then(Value::as_array_mut) else {
        return applied;
    };
    let index = Index::of(entities);
    let mut claimed: HashSet<String> = HashSet::new();
    let mut removals: Vec<usize> = Vec::new();
    let mut additions: Vec<Value> = Vec::new();

    for entry in &mut layer.entities {
        let present = index.by_id.get(&entry.id).copied();
        let ours = match scope {
            Scope::Scene => entry.from.is_empty(),
            Scope::Cell(file) => entry.op != EditOp::Add && (entry.from == file || present.is_some()),
        };
        if !ours {
            continue;
        }
        if entry.op == EditOp::Add {
            if present.is_some() {
                // The generator (or a bake) now makes it: don't spawn a second copy under the same id.
                claimed.insert(entry.id.clone());
                continue;
            }
            if let Some(entity) = &entry.entity {
                additions.push(entity.clone());
                applied.added.push(entry.id.clone());
            }
            continue;
        }

        let target = match resolve(entry, present, entities, &index, &claimed) {
            Ok(i) => i,
            Err(why) => {
                applied.orphaned.push((entry.id.clone(), why));
                continue;
            }
        };
        if id_of(&entities[target]) != Some(entry.id.as_str()) {
            let new_id = id_of(&entities[target]).unwrap_or_default().to_owned();
            applied.rematched.push((entry.id.clone(), new_id.clone()));
            entry.id = new_id;
        }
        claimed.insert(entry.id.clone());
        if let Scope::Cell(file) = scope
            && entry.from != file
        {
            entry.from = file.to_owned();
        }
        if let (Some(was), Some(now)) = (entry.was, translation_of(&entities[target]))
            && distance(was, now) > 0.01
        {
            applied.conflicts.push(entry.id.clone());
        }
        match entry.op {
            EditOp::Change => {
                let orphans = scene_delta::apply(&mut entities[target], &entry.patches);
                if !orphans.is_empty() {
                    let paths: Vec<String> = orphans.iter().map(|p| p.path.join(".")).collect();
                    applied.orphaned.push((entry.id.clone(), format!("no field at {}", paths.join(", "))));
                }
                applied.changed.push(entry.id.clone());
            }
            EditOp::Remove => {
                removals.extend(index.subtree(entities, target));
                applied.removed.push(entry.id.clone());
            }
            EditOp::Add => unreachable!("additions are handled above"),
        }
    }

    removals.sort_unstable();
    removals.dedup();
    for i in removals.into_iter().rev() {
        entities.remove(i);
    }
    entities.extend(additions);
    applied
}

// ─── Recording ──────────────────────────────────────────────────────────────

/// Where an edited entity came from, for its entry's fingerprint.
#[derive(Clone, Debug, Default)]
pub struct Origin {
    /// The streamed cell's path relative to the project; empty for the scene itself.
    pub from: String,
    /// The parent's name.
    pub parent: String,
}

/// Later patches on the same path, or on one above it, make earlier ones irrelevant.
fn compact(patches: Vec<ValuePatch>) -> Vec<ValuePatch> {
    let mut out: Vec<ValuePatch> = Vec::new();
    for patch in patches {
        out.retain(|earlier| !earlier.path.starts_with(&patch.path));
        out.push(patch);
    }
    out
}

/// Record that the entity `before` (as it was when first touched) is now `after`.
pub fn record_change(layer: &mut SceneEdits, before: &Value, after: &Value, origin: &Origin) {
    let Some(id) = id_of(after).map(str::to_owned) else {
        return;
    };
    let patches: Vec<ValuePatch> = scene_delta::diff(before, after)
        .into_iter()
        .filter(|patch| patch.path.first().is_some_and(|head| head != "persistent_id"))
        .collect();
    if let Some(entry) = layer.entities.iter_mut().find(|entry| entry.id == id) {
        match entry.op {
            EditOp::Add => entry.entity = Some(after.clone()),
            EditOp::Remove => {
                // Removed in an earlier session and back now (undo, or re-created): what remains is a change.
                entry.op = EditOp::Change;
                entry.patches = patches;
            }
            EditOp::Change => {
                let mut all = std::mem::take(&mut entry.patches);
                all.extend(patches);
                entry.patches = compact(all);
            }
        }
        return;
    }
    if patches.is_empty() {
        return;
    }
    layer.entities.push(EditEntry {
        id,
        op: EditOp::Change,
        name: name_of(before).to_owned(),
        mesh: mesh_of(before),
        parent: origin.parent.clone(),
        was: translation_of(before),
        from: origin.from.clone(),
        patches,
        entity: None,
    });
}

/// Record that the entity `before` was deleted.
pub fn record_remove(layer: &mut SceneEdits, before: &Value, origin: &Origin) {
    let Some(id) = id_of(before).map(str::to_owned) else {
        return;
    };
    if let Some(at) = layer.entities.iter().position(|entry| entry.id == id) {
        if layer.entities[at].op == EditOp::Add {
            // Added and then deleted: nothing to keep.
            layer.entities.remove(at);
            return;
        }
        let entry = &mut layer.entities[at];
        entry.op = EditOp::Remove;
        entry.patches.clear();
        return;
    }
    layer.entities.push(EditEntry {
        id,
        op: EditOp::Remove,
        name: name_of(before).to_owned(),
        mesh: mesh_of(before),
        parent: origin.parent.clone(),
        was: translation_of(before),
        from: origin.from.clone(),
        patches: Vec::new(),
        entity: None,
    });
}

/// Record a new entity.
pub fn record_add(layer: &mut SceneEdits, entity: &Value) {
    let Some(id) = id_of(entity).map(str::to_owned) else {
        return;
    };
    if let Some(entry) = layer.entities.iter_mut().find(|entry| entry.id == id) {
        entry.entity = Some(entity.clone());
        return;
    }
    layer.entities.push(EditEntry {
        id,
        op: EditOp::Add,
        name: name_of(entity).to_owned(),
        mesh: mesh_of(entity),
        parent: String::new(),
        was: None,
        from: String::new(),
        patches: Vec::new(),
        entity: Some(entity.clone()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity(id: &str, name: &str, at: [f32; 3]) -> Value {
        json!({
            "persistent_id": id,
            "components": {
                "somnium.Name": {"version": 1, "fields": {"value": name}},
                "somnium.Transform": {"version": 1, "fields": {
                    "translation": at, "rotation": [0.0, 0.0, 0.0, 1.0], "scale": [1.0, 1.0, 1.0]}},
                "somnium.ImportedMesh": {"version": 1, "fields": {"source": "models/bench.glb", "node": 2}}
            },
            "scripts": [], "prefab": null
        })
    }

    fn child(id: &str, name: &str, parent: &str) -> Value {
        json!({
            "persistent_id": id,
            "components": {
                "somnium.Name": {"version": 1, "fields": {"value": name}},
                "somnium.Parent": {"version": 1, "fields": {"entity": {"$entity": parent}}}
            },
            "scripts": [], "prefab": null
        })
    }

    const A: &str = "000000000000000000000000000000a1";
    const B: &str = "000000000000000000000000000000b2";
    const C: &str = "000000000000000000000000000000c3";
    const D: &str = "000000000000000000000000000000d4";

    fn scene(entities: Vec<Value>) -> Value {
        json!({"version": 4, "entities": entities})
    }

    fn moved(before: &Value, to: [f32; 3]) -> Value {
        let mut after = before.clone();
        after["components"]["somnium.Transform"]["fields"]["translation"] = json!(to);
        after
    }

    fn layer() -> SceneEdits {
        SceneEdits::new(Path::new("scenes/town.scene.json"))
    }

    #[test]
    fn a_move_survives_a_regenerated_scene() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        assert_eq!(edits.entities.len(), 1);

        // The generator runs again: same object, same id, fresh document.
        let mut regenerated = scene(vec![entity(A, "Bench", [1.0, 0.4, 2.0]), entity(B, "Lamp", [5.0, 0.0, 5.0])]);
        let applied = apply(&mut regenerated, &mut edits, Scope::Scene);
        assert_eq!(applied.changed, vec![A.to_owned()]);
        assert_eq!(translation_of(&regenerated["entities"][0]), Some([1.0, 0.0, 2.0]));
        assert_eq!(translation_of(&regenerated["entities"][1]), Some([5.0, 0.0, 5.0]), "the lamp is untouched");
    }

    #[test]
    fn applying_twice_changes_nothing_more() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        let mut once = scene(vec![bench.clone()]);
        apply(&mut once, &mut edits, Scope::Scene);
        let mut twice = once.clone();
        apply(&mut twice, &mut edits, Scope::Scene);
        assert_eq!(once, twice);
    }

    #[test]
    fn an_id_reused_for_a_different_object_is_not_edited() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        // A layout change: the counter-based id now names a crate across the street.
        let mut regenerated = scene(vec![entity(A, "Crate", [30.0, 0.0, 2.0])]);
        let applied = apply(&mut regenerated, &mut edits, Scope::Scene);
        assert!(applied.changed.is_empty());
        assert_eq!(applied.orphaned.len(), 1);
        assert_eq!(translation_of(&regenerated["entities"][0]), Some([30.0, 0.0, 2.0]));
        assert_eq!(edits.entities.len(), 1, "an orphan is kept, never dropped");
    }

    #[test]
    fn a_renamed_id_is_found_again_by_its_fingerprint() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        // The generator minted a new id for the same bench, a few centimetres from where it was.
        let mut regenerated = scene(vec![entity(C, "Bench", [1.05, 0.4, 2.0]), entity(D, "Bench", [9.0, 0.4, 2.0])]);
        let applied = apply(&mut regenerated, &mut edits, Scope::Scene);
        assert_eq!(applied.rematched, vec![(A.to_owned(), C.to_owned())]);
        assert_eq!(edits.entities[0].id, C, "the layer is re-keyed");
        assert_eq!(applied.conflicts, vec![C.to_owned()], "the generator moved it too");
        assert_eq!(translation_of(&regenerated["entities"][0]), Some([1.0, 0.0, 2.0]));
        assert_eq!(translation_of(&regenerated["entities"][1]), Some([9.0, 0.4, 2.0]));
    }

    #[test]
    fn an_id_handed_on_to_the_next_object_of_the_same_name_follows_the_object() {
        // A generator that numbers its objects inserted one before ours: our id now names the next weed along.
        let weed = entity(A, "Town redesign / wall-foot weeds", [1.0, 0.2, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &weed, &moved(&weed, [1.0, 0.0, 2.0]), &Origin::default());
        let mut regenerated = scene(vec![
            entity(B, "Town redesign / wall-foot weeds", [1.0, 0.2, 2.0]),
            entity(A, "Town redesign / wall-foot weeds", [2.5, 0.2, 2.0]),
        ]);
        let applied = apply(&mut regenerated, &mut edits, Scope::Scene);
        assert_eq!(applied.rematched, vec![(A.to_owned(), B.to_owned())]);
        assert_eq!(translation_of(&regenerated["entities"][0]), Some([1.0, 0.0, 2.0]));
        assert_eq!(translation_of(&regenerated["entities"][1]), Some([2.5, 0.2, 2.0]), "the other weed is untouched");
    }

    #[test]
    fn two_identical_candidates_are_left_alone() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        let mut regenerated = scene(vec![entity(C, "Bench", [1.5, 0.4, 2.0]), entity(D, "Bench", [0.5, 0.4, 2.0])]);
        let applied = apply(&mut regenerated, &mut edits, Scope::Scene);
        assert!(applied.rematched.is_empty());
        assert_eq!(applied.orphaned.len(), 1);
    }

    #[test]
    fn a_removal_takes_the_whole_subtree() {
        let root = entity(A, "House", [0.0, 0.0, 0.0]);
        let mut edits = layer();
        record_remove(&mut edits, &root, &Origin::default());
        let mut document = scene(vec![
            root,
            child(B, "Wall", A),
            child(C, "Window", B),
            entity(D, "Tree", [4.0, 0.0, 0.0]),
        ]);
        let applied = apply(&mut document, &mut edits, Scope::Scene);
        assert_eq!(applied.removed, vec![A.to_owned()]);
        let left: Vec<&str> = document["entities"].as_array().unwrap().iter().map(name_of).collect();
        assert_eq!(left, vec!["Tree"]);
    }

    #[test]
    fn an_addition_spawns_once_and_never_twice() {
        let mut edits = layer();
        record_add(&mut edits, &entity(D, "Rock", [3.0, 0.0, 3.0]));
        let mut document = scene(vec![entity(A, "Bench", [1.0, 0.0, 2.0])]);
        apply(&mut document, &mut edits, Scope::Scene);
        assert_eq!(document["entities"].as_array().unwrap().len(), 2);
        // A second application, or a generator that now makes the rock itself, must not duplicate the id.
        apply(&mut document, &mut edits, Scope::Scene);
        assert_eq!(document["entities"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn added_then_deleted_leaves_nothing() {
        let mut edits = layer();
        let rock = entity(D, "Rock", [3.0, 0.0, 3.0]);
        record_add(&mut edits, &rock);
        record_remove(&mut edits, &rock, &Origin::default());
        assert!(edits.entities.is_empty());
    }

    #[test]
    fn a_cell_gets_only_its_own_entries_and_never_additions() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        let in_cell = Origin { from: "scenes/cells/s_0_0.scene.json".into(), parent: String::new() };
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &in_cell);
        record_add(&mut edits, &entity(D, "Rock", [3.0, 0.0, 3.0]));

        let mut other_cell = scene(vec![entity(B, "Lamp", [5.0, 0.0, 5.0])]);
        let applied = apply(&mut other_cell, &mut edits, Scope::Cell("scenes/cells/s_1_0.scene.json"));
        assert_eq!(applied, Applied::default(), "nothing of the bench's, and no rock");

        let mut its_cell = scene(vec![bench]);
        let applied = apply(&mut its_cell, &mut edits, Scope::Cell("scenes/cells/s_0_0.scene.json"));
        assert_eq!(applied.changed, vec![A.to_owned()]);
        assert_eq!(its_cell["entities"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_entry_follows_its_entity_into_another_cell() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        let old = Origin { from: "cells/a.json".into(), parent: String::new() };
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &old);
        let mut new_home = scene(vec![bench]);
        let applied = apply(&mut new_home, &mut edits, Scope::Cell("cells/b.json"));
        assert_eq!(applied.changed, vec![A.to_owned()]);
        assert_eq!(edits.entities[0].from, "cells/b.json");
    }

    #[test]
    fn repeated_moves_keep_one_patch_per_field() {
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = layer();
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        let first = moved(&bench, [1.0, 0.0, 2.0]);
        record_change(&mut edits, &first, &moved(&first, [2.0, 0.0, 2.0]), &Origin::default());
        assert_eq!(edits.entities[0].patches.len(), 1);
        assert_eq!(edits.entities[0].was, Some([1.0, 0.4, 2.0]), "the generated position is kept from the first");
    }

    #[test]
    fn the_layer_round_trips_through_its_file() {
        let directory = std::env::temp_dir().join(format!("somnium-edits-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let scene_path = directory.join("town.scene.json");
        let bench = entity(A, "Bench", [1.0, 0.4, 2.0]);
        let mut edits = SceneEdits::new(&scene_path);
        record_change(&mut edits, &bench, &moved(&bench, [1.0, 0.0, 2.0]), &Origin::default());
        edits.write(&scene_path).unwrap();
        assert_eq!(SceneEdits::read(&scene_path).unwrap(), Some(edits));

        // Emptied: the file is set aside, not deleted.
        SceneEdits::new(&scene_path).write(&scene_path).unwrap();
        assert_eq!(SceneEdits::read(&scene_path).unwrap(), None);
        assert!(layer_path(&scene_path).with_extension("json.cleared").exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unreadable_layer_is_an_error_not_an_empty_one() {
        let directory = std::env::temp_dir().join(format!("somnium-edits-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let scene_path = directory.join("town.scene.json");
        std::fs::write(layer_path(&scene_path), "{ not json").unwrap();
        assert!(SceneEdits::read(&scene_path).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
