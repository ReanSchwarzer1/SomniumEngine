//! MORROWIND-O: prefab templates, nested paths and schema-based instance edits.
//!
//! Architecture references: O3DE Prefab/Instance/Instance.h (alias paths and
//! ownership), Flax Level/Prefabs/Prefab.h (template defaults and instance diff).
//! Original implementation: expanded entities keep their last template baseline,
//! so every schema edit, including undo, becomes an override by comparison.

use crate::scene_delta::{self, ValuePatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use somnium_asset::database::AssetId;
use somnium_ecs::{
    Component, Entity, FieldId, PersistentId, ReflectValue, StableId, TypeRegistry, World,
};
use std::collections::{BTreeMap, BTreeSet};

/// A nested template mounted under a durable, template-local alias.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NestedPrefab {
    /// Stable alias; never a vector offset.
    pub alias: String,
    /// Referenced template asset.
    pub template: AssetId,
}

/// Versioned `.somprefab` source asset. Unknown scene fields are retained.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrefabTemplate {
    /// Prefab format version.
    pub version: u32,
    /// Schema scene fragment with durable local entity ids.
    pub scene: Value,
    /// Template-local root entity id.
    pub root: String,
    /// Referenced templates, expanded with cycle and depth validation.
    #[serde(default)]
    pub nested: Vec<NestedPrefab>,
}

/// Runtime link carried by every expanded instance member and saved in v4 scenes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrefabMember {
    /// Top-level template identity.
    pub template: AssetId,
    /// Source path used by editor propagation; runtime may resolve by AssetId.
    pub source: String,
    /// Durable id of the top-level instance root.
    pub root: String,
    /// Alias path followed by a local entity id.
    pub path: Vec<String>,
    /// Last resolved template values, with world references already remapped.
    pub baseline: Value,
    /// Overrides that no longer address content, preserved for future recovery.
    pub(crate) orphaned: Vec<ValuePatch>,
}
impl Component for PrefabMember {}

/// Template resolution is separate from instancing and may use cooked assets.
pub type PrefabLibrary = BTreeMap<AssetId, PrefabTemplate>;

/// One current field override in the editor's reflection vocabulary.
#[derive(Clone, Debug)]
pub struct FieldOverride {
    /// Entity being edited.
    pub entity: Entity,
    /// Registered component identity.
    pub component: StableId,
    /// Registered field identity.
    pub field: FieldId,
    /// Current value, read through the same schema as Details.
    pub value: ReflectValue,
    /// Undo/invalidation width declared by the field schema.
    pub scope: somnium_ecs::reflect::ChangeScope,
}

/// Rebuilding fields require their owning authoring workflow. The prefab
/// override route deliberately rejects them, as MORROWIND-O Appendix A.3.5
/// permits, rather than replaying only part of an entity or scene transaction.
pub(crate) fn check_field_scope(
    world: &World,
    entity: Entity,
    component: StableId,
    field: &somnium_ecs::reflect::FieldSchema,
) -> Result<(), String> {
    if world.get::<PrefabMember>(entity).is_some()
        && field.scope != somnium_ecs::reflect::ChangeScope::Field
    {
        return Err(format!(
            "Prefab override {}.{} requires {:?} rebuilding; break the prefab link before editing this field",
            component, field.name, field.scope
        ));
    }
    Ok(())
}

fn validate_patch_scopes(patches: &[ValuePatch], registry: &TypeRegistry) -> Result<(), String> {
    for patch in patches {
        if patch.path.first().map(String::as_str) != Some("components") {
            continue;
        }
        let Some(schema) = patch.path.get(1).and_then(|name| registry.by_name(name)) else {
            continue;
        };
        for field in &schema.fields {
            if field.scope != somnium_ecs::reflect::ChangeScope::Field
                && (patch.path.len() < 4 || patch.path[3] == field.name)
            {
                return Err(format!(
                    "Prefab patch {}.{} requires {:?} rebuilding; edit outside this prefab instance",
                    schema.stable_id, field.name, field.scope
                ));
            }
        }
    }
    Ok(())
}

fn clean(mut entry: Value) -> Value {
    if let Some(object) = entry.as_object_mut() {
        object.remove("prefab");
    }
    entry
}

impl PrefabTemplate {
    /// Capture a selection and all descendants as a template. Selection order
    /// chooses the root; durable ids, rather than ordering, name all members.
    pub fn capture(
        world: &mut World,
        registry: &TypeRegistry,
        selection: &[Entity],
    ) -> Result<Self, String> {
        if selection.is_empty() {
            return Err("select an entity to create a prefab".into());
        }
        let mut selected: std::collections::HashSet<Entity> = selection.iter().copied().collect();
        loop {
            let children: Vec<_> = world
                .entities()
                .filter(|e| {
                    world
                        .get::<crate::Parent>(*e)
                        .is_some_and(|p| selected.contains(&p.entity))
                })
                .collect();
            let count = selected.len();
            selected.extend(children);
            if selected.len() == count {
                break;
            }
        }
        let roots: Vec<_> = selected
            .iter()
            .copied()
            .filter(|e| {
                world.is_alive(*e)
                    && world
                        .get::<crate::Parent>(*e)
                        .is_none_or(|p| !selected.contains(&p.entity))
            })
            .collect();
        if roots.len() != 1 {
            return Err("Select one hierarchy root to create a prefab; group unrelated selections under an Empty entity first".into());
        }
        let root = roots[0];
        let scene = crate::scene_schema::entities_to_json(
            world,
            registry,
            &selected.into_iter().collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
        let mut scene = scene;
        let root_id = world
            .persistent_id(root)
            .ok_or("missing root id")?
            .to_string();
        for entry in scene["entities"].as_array_mut().ok_or("missing entities")? {
            *entry = clean(entry.take());
            if entry["persistent_id"].as_str() == Some(&root_id)
                && let Some(components) = entry["components"].as_object_mut()
            {
                components.remove("somnium.Parent");
            }
        }
        Ok(Self {
            version: 1,
            scene,
            root: world
                .persistent_id(root)
                .ok_or("missing root id")?
                .to_string(),
            nested: Vec::new(),
        })
    }

    /// Decode and structurally validate a template before world mutation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("prefab exceeds 64 MiB".into());
        }
        let template: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        template.validate()?;
        Ok(template)
    }

    /// Validate entity identity and nested aliases without resolving dependencies.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported prefab version".into());
        }
        let entities = scene_delta::entities(&self.scene)?;
        if !entities.contains_key(&self.root) {
            return Err("prefab root is absent".into());
        }
        let mut aliases = BTreeSet::new();
        for nested in &self.nested {
            if nested.alias.is_empty()
                || !nested
                    .alias
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !aliases.insert(&nested.alias)
            {
                return Err("empty, repeated or invalid nested alias".into());
            }
        }
        Ok(())
    }

    /// Atomically publish source bytes; an interrupted save keeps the old file.
    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        self.validate()?;
        crate::save_game::atomic_write(
            path,
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
}

/// Add a nested dependency only after resolving the candidate graph. Failure
/// leaves the library unchanged, including cycles, duplicate aliases and budgets.
pub fn add_nested_template(
    library: &mut PrefabLibrary,
    owner: AssetId,
    nested: AssetId,
    alias: &str,
) -> Result<(), String> {
    let mut candidate = library.clone();
    candidate
        .get_mut(&owner)
        .ok_or("missing owner template")?
        .nested
        .push(NestedPrefab {
            alias: alias.into(),
            template: nested,
        });
    expand(&candidate, owner)?;
    *library = candidate;
    Ok(())
}

fn expand(
    library: &PrefabLibrary,
    template: AssetId,
) -> Result<BTreeMap<Vec<String>, Value>, String> {
    fn visit(
        library: &PrefabLibrary,
        id: AssetId,
        prefix: &[String],
        stack: &mut Vec<AssetId>,
        out: &mut BTreeMap<Vec<String>, Value>,
    ) -> Result<(), String> {
        if stack.contains(&id) || stack.len() >= 32 {
            return Err("cyclic or too-deep nested prefab".into());
        }
        let template = library.get(&id).ok_or("unresolved nested prefab")?;
        template.validate()?;
        stack.push(id);
        // Rewrite local references to alias-qualified ids before final remapping.
        let entities = scene_delta::entities(&template.scene)?;
        let ids: BTreeMap<_, _> = entities
            .keys()
            .map(|id| (id.clone(), [prefix, &[id.clone()]].concat().join("/")))
            .collect();
        for (local, mut entry) in entities {
            let mut path = prefix.to_vec();
            path.push(local);
            scene_delta::remap(&mut entry, &ids);
            out.insert(path, clean(entry));
        }
        for nested in &template.nested {
            let mut path = prefix.to_vec();
            path.push(nested.alias.clone());
            visit(library, nested.template, &path, stack, out)?;
            let child_root = library[&nested.template].root.clone();
            let mut child_path = path;
            child_path.push(child_root);
            let owner = [prefix, &[template.root.clone()]].concat().join("/");
            if let Some(entry) = out.get_mut(&child_path) {
                entry["components"]["somnium.Parent"] =
                    serde_json::json!({"version":1,"fields":{"entity":{"$entity":owner}}});
            }
        }
        stack.pop();
        if out.len() > 100_000 {
            return Err("prefab exceeds entity budget".into());
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    visit(library, template, &[], &mut Vec::new(), &mut out)?;
    Ok(out)
}

fn resolved(
    library: &PrefabLibrary,
    template: AssetId,
    root: PersistentId,
) -> Result<BTreeMap<Vec<String>, Value>, String> {
    if root.is_none() {
        return Err("zero instance root".into());
    }
    let root_path = vec![
        library
            .get(&template)
            .ok_or("missing template")?
            .root
            .clone(),
    ];
    let mut entries = expand(library, template)?;
    let ids: BTreeMap<_, _> = entries
        .keys()
        .map(|path| {
            let id = if *path == root_path {
                root
            } else {
                PersistentId::from_raw(
                    AssetId::from_relative_path(&format!("{root}/{}", path.join("/"))).raw(),
                )
            };
            (path.join("/"), id.to_string())
        })
        .collect();
    for entry in entries.values_mut() {
        scene_delta::remap(entry, &ids);
    }
    Ok(entries)
}

/// Instantiate a template with fresh durable ids. Internal entity references
/// are remapped before the two-pass scene loader sees any values.
pub fn instantiate(
    world: &mut World,
    registry: &TypeRegistry,
    library: &PrefabLibrary,
    template: AssetId,
    source: &str,
) -> Result<Entity, String> {
    let root = PersistentId::mint();
    let entries = resolved(library, template, root)?;
    let scene = scene_delta::document(
        entries
            .values()
            .map(|entry| {
                (
                    entry["persistent_id"].as_str().unwrap().to_owned(),
                    entry.clone(),
                )
            })
            .collect(),
    );
    let report =
        crate::scene_schema::scene_from_json(world, registry, &scene).map_err(|e| e.to_string())?;
    for (path, baseline) in entries {
        let id = PersistentId::parse_hex(baseline["persistent_id"].as_str().unwrap()).unwrap();
        let entity = world
            .entity_by_persistent_id(id)
            .ok_or("instantiated member absent")?;
        world
            .insert_component(
                entity,
                PrefabMember {
                    template,
                    source: source.into(),
                    root: root.to_string(),
                    path,
                    baseline,
                    orphaned: Vec::new(),
                },
            )
            .map_err(|e| e.to_string())?;
    }
    debug_assert!(!report.entities.is_empty());
    world
        .entity_by_persistent_id(root)
        .ok_or("instance root absent".into())
}

/// Link an existing selection to its just-captured template without respawning.
pub fn link_selection(
    world: &mut World,
    template: &PrefabTemplate,
    asset: AssetId,
    source: &str,
) -> Result<(), String> {
    for (id, baseline) in scene_delta::entities(&template.scene)? {
        let entity = world
            .entity_by_persistent_id(PersistentId::parse_hex(&id).ok_or("invalid id")?)
            .ok_or("selection is no longer live")?;
        world
            .insert_component(
                entity,
                PrefabMember {
                    template: asset,
                    source: source.into(),
                    root: template.root.clone(),
                    path: vec![id],
                    baseline,
                    orphaned: Vec::new(),
                },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Report typed field overrides through the registered schema, including scope.
pub fn field_overrides(
    world: &mut World,
    registry: &TypeRegistry,
    entity: Entity,
) -> Result<Vec<FieldOverride>, String> {
    let Some(member) = world.get::<PrefabMember>(entity).cloned() else {
        return Ok(Vec::new());
    };
    let current = crate::scene_schema::entities_to_json(world, registry, &[entity])
        .map_err(|e| e.to_string())?;
    let entry = clean(current["entities"][0].clone());
    let mut out = Vec::new();
    for schema in registry.schemas_on(world, entity) {
        for field in &schema.fields {
            let path = [
                "components",
                schema.stable_id.as_str(),
                "fields",
                field.name,
            ];
            let current = path.iter().try_fold(&entry, |v, key| v.get(key));
            let baseline = path.iter().try_fold(&member.baseline, |v, key| v.get(key));
            if current != baseline
                && let Some(value) = (schema.read_field)(world, entity, field.id)
            {
                out.push(FieldOverride {
                    entity,
                    component: schema.stable_id,
                    field: field.id,
                    value,
                    scope: field.scope,
                });
            }
        }
    }
    Ok(out)
}

/// Refresh one instance after its template changed. Existing entity handles stay
/// stable. Orphaned overrides survive and are reported instead of being lost.
pub fn refresh(
    world: &mut World,
    registry: &TypeRegistry,
    library: &PrefabLibrary,
    root: Entity,
    keep_overrides: bool,
) -> Result<Vec<String>, String> {
    let link = world
        .get::<PrefabMember>(root)
        .cloned()
        .ok_or("selection is not a prefab instance")?;
    let root_id = PersistentId::parse_hex(&link.root).ok_or("invalid instance root")?;
    let mut entries = resolved(library, link.template, root_id)?;
    let members: Vec<_> = world
        .entities()
        .filter_map(|e| {
            world
                .get::<PrefabMember>(e)
                .filter(|m| m.root == link.root)
                .cloned()
                .map(|m| (e, m))
        })
        .collect();
    let mut by_path: BTreeMap<_, _> = members
        .into_iter()
        .map(|(e, m)| (m.path.clone(), (e, m)))
        .collect();
    let actual_ids: BTreeMap<_, _> = entries
        .iter()
        .filter_map(|(path, entry)| {
            let (entity, _) = by_path.get(path)?;
            Some((
                entry["persistent_id"].as_str()?.to_owned(),
                world.persistent_id(*entity)?.to_string(),
            ))
        })
        .collect();
    for entry in entries.values_mut() {
        scene_delta::remap(entry, &actual_ids);
    }
    let mut warnings = Vec::new();
    // Stage all changes before applying. New entities are loaded together, so
    // cross references among newly introduced template members resolve.
    let mut updates = Vec::new();
    let mut added = BTreeMap::new();
    for (path, mut baseline) in entries {
        if let Some((entity, member)) = by_path.remove(&path) {
            // Captured instances retain their original ids; refreshed/new members
            // use their actual member id rather than a fresh hash.
            baseline["persistent_id"] = Value::String(
                world
                    .persistent_id(entity)
                    .ok_or("missing member id")?
                    .to_string(),
            );
            let current = crate::scene_schema::entities_to_json(world, registry, &[entity])
                .map_err(|e| e.to_string())?;
            let mut patches = if keep_overrides {
                member.orphaned.clone()
            } else {
                Vec::new()
            };
            if keep_overrides {
                let current =
                    scene_delta::diff(&member.baseline, &clean(current["entities"][0].clone()));
                for patch in current {
                    patches.retain(|old| old.path != patch.path);
                    patches.push(patch);
                }
            }
            validate_patch_scopes(&patches, registry)?;
            validate_patch_scopes(&scene_delta::diff(&member.baseline, &baseline), registry)?;
            let mut updated = baseline.clone();
            let orphaned = scene_delta::apply(&mut updated, &patches);
            for patch in &orphaned {
                warnings.push(format!(
                    "{}: unresolved {}",
                    path.join("/"),
                    patch.path.join(".")
                ));
            }
            updates.push((
                entity,
                updated,
                PrefabMember {
                    baseline,
                    orphaned,
                    ..member
                },
            ));
        } else {
            let id = baseline["persistent_id"]
                .as_str()
                .ok_or("missing id")?
                .to_owned();
            let mut entry = baseline.clone();
            entry["prefab"] = serde_json::to_value(PrefabMember {
                template: link.template,
                source: link.source.clone(),
                root: link.root.clone(),
                path,
                baseline,
                orphaned: Vec::new(),
            })
            .map_err(|e| e.to_string())?;
            added.insert(id, entry);
        }
    }
    if !added.is_empty() {
        crate::scene_schema::scene_from_json(world, registry, &scene_delta::document(added))
            .map_err(|e| e.to_string())?;
    }
    for (entity, entry, member) in updates {
        crate::scene_schema::apply_entity_document(world, registry, entity, &entry)?;
        world
            .insert_component(entity, member)
            .map_err(|e| e.to_string())?;
    }
    for (_, (entity, member)) in by_path {
        let current = crate::scene_schema::entities_to_json(world, registry, &[entity])
            .map_err(|e| e.to_string())?;
        if keep_overrides && clean(current["entities"][0].clone()) != member.baseline {
            warnings.push(format!(
                "removed template member {} retained with overrides",
                member.path.join("/")
            ));
        } else {
            let _ = world.despawn(entity);
        }
    }
    crate::propagate_transforms(world);
    Ok(warnings)
}

/// Remove all links from this instance, preserving live data.
pub fn break_link(world: &mut World, entity: Entity) -> Result<(), String> {
    let root = world
        .get::<PrefabMember>(entity)
        .ok_or("selection is not a prefab")?
        .root
        .clone();
    let members: Vec<_> = world
        .entities()
        .filter(|e| {
            world
                .get::<PrefabMember>(*e)
                .is_some_and(|m| m.root == root)
        })
        .collect();
    for entity in members {
        let _ = world.remove_component::<PrefabMember>(entity);
    }
    Ok(())
}

/// Editor-only marker for a deliberately opened instance-editing scope.
#[derive(Clone, Copy, Debug)]
pub struct EditingInstance;
impl Component for EditingInstance {}

/// Execute a prefab authoring action from the one editor command registry.
/// All data operations are available separately to games and headless tools.
pub fn editor_action(
    world: &mut World,
    selection: &mut crate::selection::Selection,
    action: somnium_ui::editor_event::PrefabAction,
) -> Result<String, String> {
    use somnium_ui::editor_event::PrefabAction as A;
    let registry = crate::reflect_registry::component_registry();
    match action {
        A::Create => {
            let template = PrefabTemplate::capture(world, &registry, selection.as_slice())?;
            let Some(path) = rfd::FileDialog::new()
                .add_filter("Prefab", &["somprefab"])
                .set_directory("assets")
                .set_file_name("NewPrefab.somprefab")
                .save_file()
            else {
                return Ok("Prefab creation cancelled".into());
            };
            let source = project_source(&path)?;
            template.write(&path)?;
            link_selection(world, &template, source_id(&source), &source)?;
            Ok("Created and linked prefab".into())
        }
        A::Instantiate => {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("Prefab", &["somprefab"])
                .set_directory("assets")
                .pick_file()
            else {
                return Ok("Prefab open cancelled".into());
            };
            let source = project_source(&path)?;
            let (id, library) = read_library(&path)?;
            let root = instantiate(world, &registry, &library, id, &source)?;
            *selection = crate::selection::Selection::single(root);
            Ok("Instantiated prefab".into())
        }
        _ => {
            let entity = selection.primary.ok_or("select a prefab instance")?;
            let member = world
                .get::<PrefabMember>(entity)
                .cloned()
                .ok_or("selection is not a prefab instance")?;
            let root = world
                .entity_by_persistent_id(
                    PersistentId::parse_hex(&member.root).ok_or("invalid root")?,
                )
                .ok_or("instance root is missing")?;
            match action {
                A::Enter => {
                    world
                        .insert_component(root, EditingInstance)
                        .map_err(|e| e.to_string())?;
                    *selection = crate::selection::Selection::single(root);
                    Ok("Editing prefab instance; Details edits become overrides".into())
                }
                A::Exit => {
                    let _ = world.remove_component::<EditingInstance>(root);
                    *selection = crate::selection::Selection::single(root);
                    Ok("Exited prefab instance editing".into())
                }
                A::BreakLink => {
                    break_link(world, root)?;
                    Ok("Broke prefab link; entities preserved".into())
                }
                A::Revert => {
                    let (_, library) = read_library(std::path::Path::new(&member.source))?;
                    let warnings = refresh(world, &registry, &library, root, false)?;
                    Ok(format!(
                        "Reverted prefab instance ({} diagnostics)",
                        warnings.len()
                    ))
                }
                A::Propagate => {
                    let (_, mut library) = read_library(std::path::Path::new(&member.source))?;
                    propagate(world, &registry, root, &mut library)?;
                    library[&member.template].write(std::path::Path::new(&member.source))?;
                    let roots: Vec<_> = world
                        .entities()
                        .filter(|e| {
                            world.get::<PrefabMember>(*e).is_some_and(|m| {
                                m.template == member.template
                                    && world
                                        .persistent_id(*e)
                                        .is_some_and(|id| id.to_string() == m.root)
                            })
                        })
                        .collect();
                    for root in roots {
                        refresh(world, &registry, &library, root, true)?;
                    }
                    Ok("Propagated edits to template and refreshed its live instances".into())
                }
                A::Nest => {
                    let owner_path = std::path::Path::new(&member.source);
                    let owner_parent = owner_path
                        .parent()
                        .ok_or("prefab has no parent directory")?;
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter("Prefab", &["somprefab"])
                        .set_directory(owner_parent)
                        .pick_file()
                    else {
                        return Ok("Nesting cancelled".into());
                    };
                    if path
                        .parent()
                        .ok_or("nested prefab has no parent directory")?
                        .canonicalize()
                        .map_err(|e| e.to_string())?
                        != owner_parent.canonicalize().map_err(|e| e.to_string())?
                    {
                        return Err("Keep nested prefab sources beside their owner so the dependency can resolve after reload".into());
                    }
                    let (_, mut library) = read_library(owner_path)?;
                    let nested = source_id(&project_source(&path)?);
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .ok_or("nested prefab needs a name")?;
                    let base: String = stem
                        .chars()
                        .map(|c| {
                            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                                c
                            } else {
                                '_'
                            }
                        })
                        .collect();
                    let base = if base.is_empty() {
                        "nested".to_string()
                    } else {
                        base
                    };
                    let mut alias = base.clone();
                    let mut suffix = 2;
                    while library[&member.template]
                        .nested
                        .iter()
                        .any(|n| n.alias == alias)
                    {
                        alias = format!("{base}_{suffix}");
                        suffix += 1;
                    }
                    add_nested_template(&mut library, member.template, nested, &alias)?;
                    library[&member.template].write(owner_path)?;
                    let roots: Vec<_> = world
                        .entities()
                        .filter(|e| {
                            world.get::<PrefabMember>(*e).is_some_and(|m| {
                                m.template == member.template
                                    && world
                                        .persistent_id(*e)
                                        .is_some_and(|id| id.to_string() == m.root)
                            })
                        })
                        .collect();
                    for root in roots {
                        refresh(world, &registry, &library, root, true)?;
                    }
                    Ok(format!(
                        "Nested prefab as {alias}; refreshed live instances"
                    ))
                }
                _ => unreachable!(),
            }
        }
    }
}

fn source_id(source: &str) -> AssetId {
    AssetId::from_relative_path(source.strip_prefix("assets/").unwrap_or(source))
}

fn project_source(path: &std::path::Path) -> Result<String, String> {
    let project = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        project.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or("prefab has no parent directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let absolute = parent.join(absolute.file_name().ok_or("prefab needs a file name")?);
    let relative = absolute.strip_prefix(&project).map_err(|_| {
        "Store prefab assets inside the project so their identity remains portable".to_string()
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn read_library(path: &std::path::Path) -> Result<(AssetId, PrefabLibrary), String> {
    // Resolve sibling templates by stable source identity. Cooked runtime
    // callers provide a library directly and perform no directory I/O here.
    let mut library = PrefabLibrary::new();
    let parent = path.parent().ok_or("prefab has no parent directory")?;
    for entry in std::fs::read_dir(parent).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let candidate = entry.path();
        if candidate.extension().and_then(|s| s.to_str()) == Some("somprefab") {
            let bytes = std::fs::read(&candidate).map_err(|e| e.to_string())?;
            library.insert(
                source_id(&project_source(&candidate)?),
                PrefabTemplate::from_bytes(&bytes)?,
            );
        }
    }
    let id = source_id(&project_source(path)?);
    if !library.contains_key(&id) {
        return Err("prefab source is missing".into());
    }
    Ok((id, library))
}

/// Publish edits from one instance into its top-level template. Nested templates
/// are preserved; their overrides remain on the instance until edited directly.
pub fn propagate(
    world: &mut World,
    registry: &TypeRegistry,
    root: Entity,
    library: &mut PrefabLibrary,
) -> Result<(), String> {
    let link = world
        .get::<PrefabMember>(root)
        .cloned()
        .ok_or("not an instance")?;
    let members: Vec<_> = world
        .entities()
        .filter_map(|e| {
            world
                .get::<PrefabMember>(e)
                .filter(|m| m.root == link.root)
                .cloned()
                .map(|m| (e, m))
        })
        .collect();
    let reverse: BTreeMap<_, _> = members
        .iter()
        .filter_map(|(e, m)| Some((world.persistent_id(*e)?.to_string(), m.path.join("/"))))
        .collect();
    let template = library.get_mut(&link.template).ok_or("missing template")?;
    let mut entries = scene_delta::entities(&template.scene)?;
    for (entity, member) in members {
        if member.path.len() != 1 {
            continue;
        }
        let scene = crate::scene_schema::entities_to_json(world, registry, &[entity])
            .map_err(|e| e.to_string())?;
        let mut entry = clean(scene["entities"][0].clone());
        validate_patch_scopes(&scene_delta::diff(&member.baseline, &entry), registry)?;
        scene_delta::remap(&mut entry, &reverse);
        if member.path[0] == template.root
            && let Some(components) = entry["components"].as_object_mut()
        {
            components.remove("somnium.Parent");
        }
        entries.insert(member.path[0].clone(), entry);
    }
    template.scene = scene_delta::document(entries);
    template.validate()
}

/// One reversible prefab authoring transaction. The same persistent ids are
/// restored in place, so references from outside the edited instance survive.
pub struct PrefabEditCommand {
    before: Value,
    after: Value,
    sources: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
}
impl PrefabEditCommand {
    /// Capture both sides around a validated authoring operation.
    pub fn new(before: Value, after: Value) -> Self {
        Self {
            before,
            after,
            sources: Vec::new(),
        }
    }
    /// Include existing source files changed by Propagate/Nest. Undo/redo refuses
    /// to overwrite a file modified outside this transaction.
    pub fn with_sources(
        before: Value,
        after: Value,
        sources: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
    ) -> Self {
        Self {
            before,
            after,
            sources,
        }
    }
    fn restore(&self, world: &mut World, after: bool) -> Result<(), String> {
        let document = if after { &self.after } else { &self.before };
        scene_delta::entities(document)?;
        let mut original = Vec::with_capacity(self.sources.len());
        for (path, before_bytes, after_bytes) in &self.sources {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if bytes != *before_bytes && bytes != *after_bytes {
                return Err(format!(
                    "{} changed outside this prefab edit; undo refused to overwrite it",
                    path.display()
                ));
            }
            original.push(bytes);
        }
        for (index, (path, before_bytes, after_bytes)) in self.sources.iter().enumerate() {
            let bytes = if after { after_bytes } else { before_bytes };
            if let Err(error) = crate::save_game::atomic_write(path, bytes) {
                for (previous, (previous_path, _, _)) in self.sources.iter().enumerate().take(index)
                {
                    let _ = crate::save_game::atomic_write(previous_path, &original[previous]);
                }
                return Err(error);
            }
        }
        if let Err(error) = restore_document(world, document) {
            for ((path, _, _), bytes) in self.sources.iter().zip(original) {
                let _ = crate::save_game::atomic_write(path, &bytes);
            }
            return Err(error);
        }
        Ok(())
    }
}

pub(crate) fn restore_document(world: &mut World, document: &Value) -> Result<(), String> {
    let registry = crate::reflect_registry::component_registry();
    let desired = scene_delta::entities(document)?;
    let existing: BTreeMap<_, _> = world
        .entities()
        .filter(|e| world.get::<crate::AssetEditSession>(*e).is_none())
        .filter_map(|e| world.persistent_id(e).map(|id| (id.to_string(), e)))
        .collect();
    for (id, entity) in &existing {
        if !desired.contains_key(id) {
            world.despawn(*entity);
        }
    }
    let missing = desired
        .iter()
        .filter(|(id, _)| !existing.contains_key(*id))
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect();
    crate::scene_schema::scene_from_json(world, &registry, &scene_delta::document(missing))
        .map_err(|e| e.to_string())?;
    for (id, entry) in &desired {
        let entity = world
            .entity_by_persistent_id(PersistentId::parse_hex(id).ok_or("invalid id")?)
            .ok_or("missing restored entity")?;
        crate::scene_schema::apply_entity_document(world, &registry, entity, entry)?;
        if let Some(link) = entry.get("prefab").filter(|v| !v.is_null()) {
            let member: PrefabMember =
                serde_json::from_value(link.clone()).map_err(|e| e.to_string())?;
            world
                .insert_component(entity, member)
                .map_err(|e| e.to_string())?;
        } else {
            let _ = world.remove_component::<PrefabMember>(entity);
        }
    }
    crate::propagate_transforms(world);
    Ok(())
}
impl crate::EditorCommand for PrefabEditCommand {
    fn execute(&mut self, world: &mut World, _: &mut Option<Entity>) {
        if let Err(error) = self.restore(world, true) {
            tracing::error!(%error,"prefab redo failed");
        }
    }
    fn undo(&mut self, world: &mut World, _: &mut Option<Entity>) {
        if let Err(error) = self.restore(world, false) {
            tracing::error!(%error,"prefab undo failed");
        }
    }
    fn description(&self) -> &str {
        "Edit prefab instance"
    }
    fn is_no_op(&self) -> bool {
        self.before == self.after
            && self
                .sources
                .iter()
                .all(|(_, before, after)| before == after)
    }
}
