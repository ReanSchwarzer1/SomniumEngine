//! Phase 16-A: schema-driven scene persistence.
//!
//! # `version` is a format tag, not a revision number
//!
//! A `.somnium` file's `version` field has always discriminated between
//! *different formats*, not successive revisions of one:
//!
//! | `version` | Format | Written by |
//! |---|---|---|
//! | 1 | Hand-written entity dump | [`crate::scene_serial`] |
//! | 2 | Map recipe — a factory `kind`, no entities | [`crate::map`] |
//! | 3 | Schema-driven entity dump | this module |
//!
//! Phase 16-A originally numbered this format 2, which collided with the
//! map recipe. It is 3. The three are mutually exclusive and a reader
//! rejects the two it does not handle rather than half-reading them.
//!
//! # What this format is for
//!
//! Version 1 is a hand-written walk that names `Transform`, `Light`,
//! `MeshKind`, `Terrain` and `Water` in Rust and spells out every field
//! of each. It cannot express a script attachment with author-declared
//! properties, and every new component makes it longer.
//!
//! Version 3 asks the [`TypeRegistry`] instead. It writes whatever is
//! registered and reads whatever it recognises, so a component becomes
//! saveable by being described once in
//! [`reflect_registry`](crate::reflect_registry) rather than by being
//! added here.
//!
//! # Identity
//!
//! Entities are keyed by [`PersistentId`], never by ECS index. An entity
//! reference inside a component — a `Parent`, a script's `target`
//! property — is written as the referent's persistent id and resolved on
//! load. That is why loading is two passes: every entity must exist
//! before any reference can be resolved.
//!
//! # Versioning
//!
//! Each component records the schema version it was written under, so a
//! future field rename can be migrated per component rather than by
//! bumping the whole file format.
//!
//! # Status
//!
//! This is what **Save Scene** writes. Loading it back through the editor
//! is not wired yet, and that is a pre-existing gap rather than a new
//! one: `EditorEvent::LoadScene` routes to [`crate::map::load_map`],
//! which only accepts version-2 map recipes, so the version-1 entity dump
//! was never readable by the editor either. Making an entity dump load
//! needs GPU-side reconstruction — meshes from `MeshKind`, terrain from
//! its sidecars, renderer uploads — and belongs with the editor work in
//! 16-D, not here.

use std::collections::BTreeMap;

use somnium_ecs::reflect::{
    AssetRef, FieldFlags, FieldType, ReflectObject, ReflectValue, TypeRegistry,
};
use somnium_ecs::{Entity, PersistentId, World};
use somnium_script::attachment::{PropertyBag, ScriptAttachment, ScriptSet};
use somnium_script::ids::{InstanceUuid, ScriptAssetId};

/// The format tag this module writes. See the module docs: 1 is the
/// hand-written dump, 2 is a map recipe, 3 is this.
pub const SCENE_VERSION: u64 = 3;

/// What went wrong reading a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// The file could not be read or written.
    Io(String),
    /// The text is not JSON, or not the shape a scene has.
    Malformed(String),
    /// The file is a version this build does not read.
    UnsupportedVersion(u64),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "scene i/o error: {message}"),
            Self::Malformed(message) => write!(f, "malformed scene: {message}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported scene version {v}"),
        }
    }
}

impl std::error::Error for SceneError {}

/// Anything the load recovered from but wants to report.
///
/// A scene that references a deleted script, or a component this build
/// does not know, still loads — it just says so. The alternative, failing
/// the whole file, loses the user's work over one stale reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneWarning {
    /// Persistent id of the entity the problem was on, in text form.
    pub entity: String,
    /// What happened.
    pub message: String,
}

/// Result of loading a scene.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Entities created, in file order.
    pub entities: Vec<Entity>,
    /// Recoverable problems.
    pub warnings: Vec<SceneWarning>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Values
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Widen a value for the file.
///
/// Entity and asset references become tagged objects rather than bare
/// numbers so that a reader can tell a reference from an integer without
/// consulting the schema — which matters for a human reading a diff.
fn value_to_json(world: &World, value: &ReflectValue) -> serde_json::Value {
    use serde_json::json;
    match value {
        // An unset reference and an absent value are both null in the
        // file; the schema's declared type is what tells them apart on
        // the way back in.
        ReflectValue::Nil | ReflectValue::Entity(None) | ReflectValue::Asset(None) => {
            serde_json::Value::Null
        }
        ReflectValue::Bool(v) => json!(v),
        ReflectValue::I64(v) => json!(v),
        ReflectValue::F64(v) => json!(v),
        ReflectValue::Str(v) => json!(v),
        ReflectValue::Vec2(v) => json!(v),
        ReflectValue::Vec3(v) => json!(v),
        ReflectValue::Vec4(v) | ReflectValue::Quat(v) => json!(v),
        ReflectValue::Entity(Some(entity)) => match world.persistent_id(*entity) {
            // An entity with no durable id cannot be referenced across a
            // save. Writing null rather than an index is the honest
            // answer: the reference is genuinely not persistable.
            Some(id) => json!({ "$entity": id.to_string() }),
            None => serde_json::Value::Null,
        },
        ReflectValue::Asset(Some(asset)) => json!({ "$asset": format!("{:032x}", asset.0) }),
        ReflectValue::Array(items) => {
            json!(
                items
                    .iter()
                    .map(|i| value_to_json(world, i))
                    .collect::<Vec<_>>()
            )
        }
        ReflectValue::Object(fields) => {
            let mut map = serde_json::Map::new();
            for (id, value) in fields {
                map.insert(id.0.to_string(), value_to_json(world, value));
            }
            json!({ "$fields": map })
        }
        ReflectValue::Map(entries) => {
            let mut map = serde_json::Map::new();
            for (key, value) in entries {
                map.insert(key.clone(), value_to_json(world, value));
            }
            json!({ "$map": map })
        }
    }
}

/// Narrow a value from the file, using the declared type to disambiguate.
///
/// The type is required, not optional: JSON cannot tell a `Vec4` from a
/// `Quat`, or a float that happens to be whole from an integer. Parsing
/// against the schema rather than guessing from the JSON is what makes
/// the round trip exact.
fn value_from_json(
    resolve: &dyn Fn(PersistentId) -> Option<Entity>,
    ty: &FieldType,
    json: &serde_json::Value,
) -> Option<ReflectValue> {
    fn floats<const N: usize>(json: &serde_json::Value) -> Option<[f32; N]> {
        let array = json.as_array()?;
        if array.len() != N {
            return None;
        }
        let mut out = [0.0_f32; N];
        for (slot, value) in out.iter_mut().zip(array) {
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = value.as_f64()? as f32;
            }
        }
        Some(out)
    }

    match ty {
        FieldType::Bool => json.as_bool().map(ReflectValue::Bool),
        FieldType::I64 | FieldType::Enum(_) => json.as_i64().map(ReflectValue::I64),
        FieldType::F64 => json.as_f64().map(ReflectValue::F64),
        FieldType::Str => json.as_str().map(|s| ReflectValue::Str(s.to_owned())),
        FieldType::Vec2 => floats::<2>(json).map(ReflectValue::Vec2),
        FieldType::Vec3 | FieldType::Color => floats::<3>(json).map(ReflectValue::Vec3),
        FieldType::Vec4 => floats::<4>(json).map(ReflectValue::Vec4),
        FieldType::Quat => floats::<4>(json).map(ReflectValue::Quat),
        FieldType::Entity => {
            if json.is_null() {
                return Some(ReflectValue::Entity(None));
            }
            let text = json.get("$entity")?.as_str()?;
            let id = PersistentId::parse_hex(text)?;
            // A dangling reference resolves to `None` rather than failing
            // the load: the target may legitimately have been deleted.
            Some(ReflectValue::Entity(resolve(id)))
        }
        FieldType::Asset => {
            if json.is_null() {
                return Some(ReflectValue::Asset(None));
            }
            let text = json.get("$asset")?.as_str()?;
            u128::from_str_radix(text, 16)
                .ok()
                .map(|raw| ReflectValue::Asset(Some(AssetRef(raw))))
        }
        FieldType::Array(inner) => {
            let items = json.as_array()?;
            items
                .iter()
                .map(|item| value_from_json(resolve, inner, item))
                .collect::<Option<Vec<_>>>()
                .map(ReflectValue::Array)
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Script attachments
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Script properties are the one place the file cannot consult a schema:
// the schema belongs to the script asset, which may not even compile at
// load time. So property values carry their own kind tag, and a value
// whose declared type has since changed is dropped by
// `ScriptSchema::resolve_properties` when the script is next described.

/// Write one property value with a kind tag.
fn property_to_json(world: &World, value: &ReflectValue) -> serde_json::Value {
    use serde_json::json;
    if let ReflectValue::Map(entries) = value {
        // Entries are tagged individually, because a map has no declared
        // type to parse its members against.
        let mut map = serde_json::Map::new();
        for (key, item) in entries {
            map.insert(key.clone(), property_to_json(world, item));
        }
        return json!({ "kind": "map", "value": { "$map": map } });
    }
    json!({ "kind": value.kind(), "value": value_to_json(world, value) })
}

/// Read one tagged property value back.
fn property_from_json(
    resolve: &dyn Fn(PersistentId) -> Option<Entity>,
    json: &serde_json::Value,
) -> Option<ReflectValue> {
    let kind = json.get("kind")?.as_str()?;
    let value = json.get("value")?;
    let ty = match kind {
        "bool" => FieldType::Bool,
        "integer" => FieldType::I64,
        "number" => FieldType::F64,
        "string" => FieldType::Str,
        "vec2" => FieldType::Vec2,
        "vec3" => FieldType::Vec3,
        "vec4" => FieldType::Vec4,
        "quat" => FieldType::Quat,
        "entity" => FieldType::Entity,
        "asset" => FieldType::Asset,
        "nil" => return Some(ReflectValue::Nil),
        // A script's own record, keyed by names the author chose. It has
        // no declared field type to parse against, so each entry carries
        // its own tag the same way the property itself does.
        "map" => {
            let object = value.get("$map")?.as_object()?;
            let mut entries = std::collections::BTreeMap::new();
            for (key, item) in object {
                entries.insert(key.clone(), property_from_json(resolve, item)?);
            }
            return Some(ReflectValue::Map(entries));
        }
        // Arrays and nested objects are not authored properties yet; a
        // file containing one is from a newer build, and dropping the
        // property is better than failing the scene.
        _ => return None,
    };
    value_from_json(resolve, &ty, value)
}

fn attachment_to_json(world: &World, attachment: &ScriptAttachment) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    for (name, value) in &attachment.properties {
        properties.insert(name.clone(), property_to_json(world, value));
    }
    serde_json::json!({
        "instance":        attachment.instance.to_string(),
        "asset":           attachment.asset.to_string(),
        "enabled":         attachment.enabled,
        "execution_order": attachment.execution_order,
        "schema_version":  attachment.schema_version,
        "api_version":     attachment.api_version,
        "properties":      properties,
    })
}

fn attachment_from_json(
    resolve: &dyn Fn(PersistentId) -> Option<Entity>,
    json: &serde_json::Value,
) -> Option<ScriptAttachment> {
    let mut properties = PropertyBag::new();
    if let Some(map) = json.get("properties").and_then(|p| p.as_object()) {
        for (name, value) in map {
            if let Some(parsed) = property_from_json(resolve, value) {
                properties.insert(name.clone(), parsed);
            }
        }
    }
    Some(ScriptAttachment {
        instance: InstanceUuid::parse_hex(json.get("instance")?.as_str()?)?,
        asset: ScriptAssetId::parse_hex(json.get("asset")?.as_str()?)?,
        enabled: json
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        execution_order: json
            .get("execution_order")
            .and_then(serde_json::Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(0),
        schema_version: json
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1),
        api_version: json
            .get("api_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1),
        properties,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Save
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Serialize the world to a version-2 scene document.
///
/// Every entity is given a persistent id if it does not have one, which
/// is why this takes `&mut World`: an entity that cannot be named cannot
/// be referenced, and silently dropping the reference later is worse than
/// minting the id now.
pub fn scene_to_json(world: &mut World, registry: &TypeRegistry) -> serde_json::Value {
    let all: Vec<Entity> = world.entities().collect();
    for entity in &all {
        let _ = world.ensure_persistent_id(*entity);
    }

    // Sort by persistent id so the file is stable: two saves of an
    // unchanged world must produce the same bytes, or every scene diff is
    // noise.
    let mut ordered: Vec<(PersistentId, Entity)> = all
        .iter()
        .filter_map(|&e| world.persistent_id(e).map(|id| (id, e)))
        .collect();
    ordered.sort_unstable_by_key(|(id, _)| *id);

    let entities: Vec<serde_json::Value> = ordered
        .iter()
        .map(|&(id, entity)| {
            let mut components = serde_json::Map::new();
            for schema in registry.schemas_on(world, entity) {
                let Some(record) = (schema.snapshot)(world, entity) else {
                    continue;
                };
                let mut fields = serde_json::Map::new();
                for field in &schema.fields {
                    if !field.flags.contains(FieldFlags::SERIALIZE) {
                        continue;
                    }
                    if let Some(value) = record.get(&field.id) {
                        fields.insert(field.name.to_owned(), value_to_json(world, value));
                    }
                }
                if fields.is_empty() {
                    continue;
                }
                components.insert(
                    schema.stable_id.as_str().to_owned(),
                    serde_json::json!({ "version": schema.version, "fields": fields }),
                );
            }

            let scripts: Vec<serde_json::Value> = world
                .get::<ScriptSet>(entity)
                .map(|set| {
                    set.attachments
                        .iter()
                        .map(|a| attachment_to_json(world, a))
                        .collect()
                })
                .unwrap_or_default();

            serde_json::json!({
                "persistent_id": id.to_string(),
                "components":    components,
                "scripts":       scripts,
            })
        })
        .collect();

    serde_json::json!({ "version": SCENE_VERSION, "entities": entities })
}

/// Write a version-2 scene to disk.
///
/// # Errors
///
/// [`SceneError::Io`] if the file cannot be written.
pub fn save_scene_schema(
    world: &mut World,
    registry: &TypeRegistry,
    path: &str,
) -> Result<(), SceneError> {
    let document = scene_to_json(world, registry);
    let text = serde_json::to_string_pretty(&document)
        .map_err(|e| SceneError::Malformed(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| SceneError::Io(e.to_string()))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Load
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Rebuild a world from a version-2 scene document.
///
/// Two passes: every entity is created with its persistent id first, so
/// that a component holding an entity reference can resolve it in the
/// second pass regardless of the order things appear in the file.
///
/// # Errors
///
/// [`SceneError::Malformed`] if the document is not a scene, or
/// [`SceneError::UnsupportedVersion`] if it is from a newer build.
// Two passes over the document, each with its own recovery behaviour.
// Splitting them into helpers would mean threading the resolver, the
// report and the registry through four signatures to save a line count.
#[allow(clippy::too_many_lines)]
pub fn scene_from_json(
    world: &mut World,
    registry: &TypeRegistry,
    document: &serde_json::Value,
) -> Result<LoadReport, SceneError> {
    let version = document
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SceneError::Malformed("no version".into()))?;
    if version != SCENE_VERSION {
        return Err(SceneError::UnsupportedVersion(version));
    }
    let entities_json = document
        .get("entities")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| SceneError::Malformed("no entities array".into()))?;

    let mut report = LoadReport::default();

    // Pass one: identity only.
    let mut by_id: BTreeMap<PersistentId, Entity> = BTreeMap::new();
    for entry in entities_json {
        let Some(id) = entry
            .get("persistent_id")
            .and_then(serde_json::Value::as_str)
            .and_then(PersistentId::parse_hex)
        else {
            return Err(SceneError::Malformed(
                "entity without a persistent id".into(),
            ));
        };
        let entity = world.spawn((id,));
        by_id.insert(id, entity);
        report.entities.push(entity);
    }

    let resolved = by_id.clone();
    let resolve = move |id: PersistentId| resolved.get(&id).copied();

    // Pass two: components and scripts.
    for (entry, &entity) in entities_json.iter().zip(&report.entities) {
        let entity_label = entry
            .get("persistent_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
            .to_owned();

        if let Some(components) = entry
            .get("components")
            .and_then(serde_json::Value::as_object)
        {
            for (name, body) in components {
                let Some(schema) = registry.by_name(name) else {
                    report.warnings.push(SceneWarning {
                        entity: entity_label.clone(),
                        message: format!("no component named `{name}` in this build; skipped"),
                    });
                    continue;
                };
                let stored_version = body
                    .get("version")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(schema.version);
                let Some(fields_json) = body.get("fields").and_then(serde_json::Value::as_object)
                else {
                    continue;
                };

                let mut record = ReflectObject::new();
                for (field_name, value_json) in fields_json {
                    let Some(field) = schema.field_by_name(field_name) else {
                        report.warnings.push(SceneWarning {
                            entity: entity_label.clone(),
                            message: format!("`{name}` has no field `{field_name}`; dropped"),
                        });
                        continue;
                    };
                    match value_from_json(&resolve, &field.ty, value_json) {
                        Some(value) => {
                            record.insert(field.id, value);
                        }
                        None => report.warnings.push(SceneWarning {
                            entity: entity_label.clone(),
                            message: format!(
                                "`{name}.{field_name}` is not a valid {}; left at default",
                                field.ty.name()
                            ),
                        }),
                    }
                }

                if stored_version != schema.version {
                    if let Some(migrate) = schema.migrate {
                        if let Err(err) = migrate(&mut record, stored_version) {
                            report.warnings.push(SceneWarning {
                                entity: entity_label.clone(),
                                message: format!("`{name}` migration failed: {err}"),
                            });
                            continue;
                        }
                    } else {
                        report.warnings.push(SceneWarning {
                            entity: entity_label.clone(),
                            message: format!(
                                "`{name}` was saved at version {stored_version}, this build is \
                                 version {} and has no migration; fields read as-is",
                                schema.version
                            ),
                        });
                    }
                }

                if let Err(err) = (schema.insert_default)(world, entity) {
                    report.warnings.push(SceneWarning {
                        entity: entity_label.clone(),
                        message: format!("could not attach `{name}`: {err}"),
                    });
                    continue;
                }
                if let Err(err) = (schema.apply)(world, entity, &record) {
                    report.warnings.push(SceneWarning {
                        entity: entity_label.clone(),
                        message: format!("could not write `{name}`: {err}"),
                    });
                }
            }
        }

        if let Some(scripts) = entry.get("scripts").and_then(serde_json::Value::as_array) {
            if scripts.is_empty() {
                continue;
            }
            let mut set = ScriptSet::new();
            for script_json in scripts {
                match attachment_from_json(&resolve, script_json) {
                    Some(attachment) => {
                        set.attach(attachment);
                    }
                    None => report.warnings.push(SceneWarning {
                        entity: entity_label.clone(),
                        message: "malformed script attachment; dropped".into(),
                    }),
                }
            }
            if let Err(err) = world.insert_component(entity, set) {
                report.warnings.push(SceneWarning {
                    entity: entity_label.clone(),
                    message: format!("could not attach scripts: {err}"),
                });
            }
        }
    }

    Ok(report)
}

/// Read a version-2 scene from disk.
///
/// # Errors
///
/// [`SceneError::Io`] if the file cannot be read, or the parse errors of
/// [`scene_from_json`].
pub fn load_scene_schema(
    world: &mut World,
    registry: &TypeRegistry,
    path: &str,
) -> Result<LoadReport, SceneError> {
    let text = std::fs::read_to_string(path).map_err(|e| SceneError::Io(e.to_string()))?;
    let document: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| SceneError::Malformed(e.to_string()))?;
    scene_from_json(world, registry, &document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_script::value::ScriptValue;

    use crate::reflect_registry::component_registry;
    use crate::{LightComponent, MeshComponent, Name, Parent, Transform};

    fn round_trip(world: &mut World, registry: &TypeRegistry) -> (World, LoadReport) {
        let document = scene_to_json(world, registry);
        let mut loaded = World::new();
        let report = scene_from_json(&mut loaded, registry, &document).unwrap();
        (loaded, report)
    }

    fn find(world: &World, name: &str) -> Entity {
        world
            .entities()
            .find(|&e| world.get::<Name>(e).is_some_and(|n| n.as_str() == name))
            .unwrap_or_else(|| panic!("no entity named {name}"))
    }

    #[test]
    fn components_round_trip_through_the_registry() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((
            Name::new("Sun"),
            Transform {
                translation: glam::Vec3::new(1.5, -2.0, 3.25),
                rotation: glam::Quat::from_rotation_x(0.75),
                scale: glam::Vec3::new(1.0, 2.0, 3.0),
            },
            LightComponent::directional(50_000.0),
        ));

        let (loaded, report) = round_trip(&mut world, &registry);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let sun = find(&loaded, "Sun");
        let transform = loaded.get::<Transform>(sun).unwrap();
        assert!((transform.translation - glam::Vec3::new(1.5, -2.0, 3.25)).length() < 1.0e-6);
        assert!((transform.scale - glam::Vec3::new(1.0, 2.0, 3.0)).length() < 1.0e-6);
        assert!(
            transform
                .rotation
                .angle_between(glam::Quat::from_rotation_x(0.75))
                < 1.0e-5
        );

        let light = loaded.get::<LightComponent>(sun).unwrap();
        assert!((light.intensity - 50_000.0).abs() < 1.0e-2);
        assert_eq!(light.light_type, crate::LightType::Directional);
    }

    #[test]
    fn entity_references_survive_by_persistent_id_not_by_index() {
        let registry = component_registry();
        let mut world = World::new();
        let parent = world.spawn((Name::new("Parent"), Transform::default()));
        world.spawn((
            Name::new("Child"),
            Transform::default(),
            Parent { entity: parent },
        ));

        let document = scene_to_json(&mut world, &registry);

        // Load into a world that is already populated, so the entities
        // land on different ECS indices than they had when saved. If the
        // reference were an index rather than a persistent id, this is
        // where it would point at the wrong object.
        let mut loaded = World::new();
        for i in 0..5 {
            loaded.spawn((Name::new(&format!("Decoy{i}")), Transform::default()));
        }
        let report = scene_from_json(&mut loaded, &registry, &document).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let loaded_parent = find(&loaded, "Parent");
        let loaded_child = find(&loaded, "Child");
        assert_ne!(
            loaded_parent.index(),
            parent.index(),
            "the test is only meaningful if the ECS index actually changed"
        );
        assert_eq!(
            loaded.get::<Parent>(loaded_child).unwrap().entity,
            loaded_parent
        );
    }

    #[test]
    fn a_reference_to_a_deleted_entity_loads_as_unset() {
        let registry = component_registry();
        let mut world = World::new();
        let parent = world.spawn((Name::new("Doomed"), Transform::default()));
        let child = world.spawn((
            Name::new("Orphan"),
            Transform::default(),
            Parent { entity: parent },
        ));
        let _ = world.ensure_persistent_id(parent);
        let _ = world.ensure_persistent_id(child);

        let mut document = scene_to_json(&mut world, &registry);
        // Drop the parent from the file, keeping the child's reference.
        let entities = document["entities"].as_array_mut().unwrap();
        let doomed = world.persistent_id(parent).unwrap().to_string();
        entities.retain(|e| e["persistent_id"].as_str() != Some(doomed.as_str()));

        let mut loaded = World::new();
        scene_from_json(&mut loaded, &registry, &document).unwrap();

        let orphan = find(&loaded, "Orphan");
        assert_eq!(
            loaded.get::<Parent>(orphan).unwrap().entity,
            Entity::DANGLING,
            "a dangling reference loads as unset instead of failing the scene"
        );
    }

    #[test]
    fn script_attachments_and_their_properties_survive() {
        let registry = component_registry();
        let mut world = World::new();
        let asset = ScriptAssetId::mint();

        let mut attachment = ScriptAttachment::new(asset);
        attachment.execution_order = -3;
        attachment.enabled = false;
        attachment.schema_version = 4;
        attachment
            .properties
            .insert("speed".into(), ScriptValue::F64(12.5));
        attachment
            .properties
            .insert("label".into(), ScriptValue::Str("wheel".into()));
        attachment
            .properties
            .insert("offset".into(), ScriptValue::Vec3([1.0, 0.0, -1.0]));
        attachment
            .properties
            .insert("armed".into(), ScriptValue::Bool(true));
        let instance = attachment.instance;

        let mut set = ScriptSet::new();
        set.attach(attachment);
        set.attach(ScriptAttachment::new(asset));

        world.spawn((Name::new("Scripted"), Transform::default(), set));

        let (loaded, report) = round_trip(&mut world, &registry);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let entity = find(&loaded, "Scripted");
        let set = loaded.get::<ScriptSet>(entity).unwrap();
        assert_eq!(set.len(), 2);

        let restored = set.get(instance).expect("attachment id must survive");
        assert_eq!(restored.asset, asset);
        assert_eq!(restored.execution_order, -3);
        assert!(!restored.enabled);
        assert_eq!(restored.schema_version, 4);
        assert_eq!(restored.properties["speed"], ScriptValue::F64(12.5));
        assert_eq!(
            restored.properties["label"],
            ScriptValue::Str("wheel".into())
        );
        assert_eq!(
            restored.properties["offset"],
            ScriptValue::Vec3([1.0, 0.0, -1.0])
        );
        assert_eq!(restored.properties["armed"], ScriptValue::Bool(true));
    }

    #[test]
    fn a_script_property_referencing_an_entity_is_remapped() {
        let registry = component_registry();
        let mut world = World::new();
        let target = world.spawn((Name::new("Target"), Transform::default()));

        let mut attachment = ScriptAttachment::new(ScriptAssetId::mint());
        attachment
            .properties
            .insert("target".into(), ScriptValue::Entity(Some(target)));
        let mut set = ScriptSet::new();
        let instance = set.attach(attachment);
        world.spawn((Name::new("Follower"), Transform::default(), set));

        let (loaded, _) = round_trip(&mut world, &registry);
        let follower = find(&loaded, "Follower");
        let loaded_target = find(&loaded, "Target");
        let value = &loaded
            .get::<ScriptSet>(follower)
            .unwrap()
            .get(instance)
            .unwrap()
            .properties["target"];
        assert_eq!(*value, ScriptValue::Entity(Some(loaded_target)));
    }

    #[test]
    fn an_attachment_to_a_missing_asset_still_loads_and_still_saves() {
        let registry = component_registry();
        let mut world = World::new();
        // An asset id that no file on disk corresponds to — the case
        // where someone deleted a script the scene still references.
        let ghost = ScriptAssetId::from_raw(0xdead_beef);
        let mut set = ScriptSet::new();
        let instance = set.attach(ScriptAttachment::new(ghost));
        world.spawn((Name::new("Haunted"), Transform::default(), set));

        let (mut loaded, report) = round_trip(&mut world, &registry);
        assert!(report.warnings.is_empty());

        let entity = find(&loaded, "Haunted");
        assert_eq!(
            loaded
                .get::<ScriptSet>(entity)
                .unwrap()
                .get(instance)
                .unwrap()
                .asset,
            ghost
        );

        // Saving again must not lose it.
        let again = scene_to_json(&mut loaded, &registry);
        let text = serde_json::to_string(&again).unwrap();
        assert!(text.contains(&ghost.to_string()));
    }

    #[test]
    fn saving_an_unchanged_world_twice_produces_identical_bytes() {
        let registry = component_registry();
        let mut world = World::new();
        for i in 0..12 {
            world.spawn((Name::new(&format!("E{i}")), Transform::default()));
        }
        let first = serde_json::to_string(&scene_to_json(&mut world, &registry)).unwrap();
        let second = serde_json::to_string(&scene_to_json(&mut world, &registry)).unwrap();
        assert_eq!(first, second, "scene output must be stable across saves");
    }

    #[test]
    fn engine_owned_fields_are_not_written_to_the_file() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((
            Name::new("Mesh"),
            MeshComponent {
                vertex_offset: 10,
                index_offset: 20,
                index_count: 30,
            },
        ));
        let text = serde_json::to_string(&scene_to_json(&mut world, &registry)).unwrap();
        assert!(
            !text.contains("somnium.Mesh"),
            "every field of Mesh is runtime-only, so the component writes nothing"
        );
    }

    #[test]
    fn material_asset_reference_round_trips_but_runtime_pool_id_does_not() {
        let registry = component_registry();
        let asset =
            somnium_asset::database::AssetId::from_relative_path("materials/Polished.sommat");
        let mut world = World::new();
        world.spawn((
            Name::new("Polished Cube"),
            Transform::default(),
            crate::MaterialComponent {
                asset,
                runtime_id: 913,
            },
        ));
        let document = scene_to_json(&mut world, &registry);
        let text = serde_json::to_string(&document).unwrap();
        assert!(text.contains(&asset.to_string()));
        assert!(!text.contains("runtime_id"));
        assert!(!text.contains("913"));

        let mut loaded = World::new();
        scene_from_json(&mut loaded, &registry, &document).unwrap();
        let entity = find(&loaded, "Polished Cube");
        let material = loaded.get::<crate::MaterialComponent>(entity).unwrap();
        assert_eq!(material.asset, asset);
        assert_eq!(material.runtime_id, 0);
    }

    #[test]
    fn an_unknown_component_warns_and_the_rest_of_the_entity_loads() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((Name::new("Future"), Transform::default()));
        let mut document = scene_to_json(&mut world, &registry);

        document["entities"][0]["components"]["mod.FromANewerBuild"] =
            serde_json::json!({ "version": 1, "fields": { "whatever": 3 } });

        let mut loaded = World::new();
        let report = scene_from_json(&mut loaded, &registry, &document).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].message.contains("mod.FromANewerBuild"));
        assert_eq!(
            loaded
                .get::<Name>(find(&loaded, "Future"))
                .unwrap()
                .as_str(),
            "Future"
        );
    }

    #[test]
    fn an_unknown_field_warns_and_the_rest_of_the_component_loads() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((Name::new("Partial"), Transform::default()));
        let mut document = scene_to_json(&mut world, &registry);

        document["entities"][0]["components"]["somnium.Transform"]["fields"]["shear"] =
            serde_json::json!([1.0, 2.0, 3.0]);

        let mut loaded = World::new();
        let report = scene_from_json(&mut loaded, &registry, &document).unwrap();
        assert!(report.warnings.iter().any(|w| w.message.contains("shear")));
        assert!(loaded.get::<Transform>(find(&loaded, "Partial")).is_some());
    }

    #[test]
    fn a_version_mismatch_without_a_migration_warns_rather_than_failing() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((Name::new("Old"), Transform::default()));
        let mut document = scene_to_json(&mut world, &registry);
        document["entities"][0]["components"]["somnium.Transform"]["version"] =
            serde_json::json!(0);

        let mut loaded = World::new();
        let report = scene_from_json(&mut loaded, &registry, &document).unwrap();
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.message.contains("version 0"))
        );
        assert!(loaded.get::<Transform>(find(&loaded, "Old")).is_some());
    }

    #[test]
    fn a_newer_file_format_is_refused_rather_than_half_read() {
        let registry = component_registry();
        let mut loaded = World::new();
        let document = serde_json::json!({ "version": 99, "entities": [] });
        assert_eq!(
            scene_from_json(&mut loaded, &registry, &document).unwrap_err(),
            SceneError::UnsupportedVersion(99)
        );
        assert_eq!(loaded.entity_count(), 0);
    }

    #[test]
    fn a_malformed_document_is_refused() {
        let registry = component_registry();
        let mut loaded = World::new();
        assert!(matches!(
            scene_from_json(&mut loaded, &registry, &serde_json::json!({})),
            Err(SceneError::Malformed(_))
        ));
        // Right format tag, wrong shape.
        assert!(matches!(
            scene_from_json(&mut loaded, &registry, &serde_json::json!({ "version": 3 })),
            Err(SceneError::Malformed(_))
        ));
    }

    #[test]
    fn a_file_on_disk_round_trips() {
        let registry = component_registry();
        let mut world = World::new();
        world.spawn((
            Name::new("OnDisk"),
            Transform::from_translation(glam::Vec3::Y),
        ));

        let path = std::env::temp_dir().join("somnium_scene_schema_round_trip.somnium");
        let path = path.to_str().unwrap();
        save_scene_schema(&mut world, &registry, path).unwrap();

        let mut loaded = World::new();
        let report = load_scene_schema(&mut loaded, &registry, path).unwrap();
        assert_eq!(report.entities.len(), 1);
        assert!(report.warnings.is_empty());
        assert!(
            (loaded
                .get::<Transform>(find(&loaded, "OnDisk"))
                .unwrap()
                .translation
                - glam::Vec3::Y)
                .length()
                < 1.0e-6
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn water_survives_the_schema_format_field_for_field() {
        // Parity with the version-1 hand-written walk. Water is the
        // component that walk spelled out at greatest length, so it is the
        // one where a registry omission would be least obvious.
        let registry = component_registry();
        let mut world = World::new();
        let terrain = world.spawn((
            Name::new("Terrain"),
            Transform::default(),
            crate::TerrainComponent::default(),
        ));
        let original = crate::WaterComponent::great_lakes(7, 4, [0.0, 0.0, 1024.0, 1024.0]);
        world.spawn((
            Name::new("Water"),
            Transform::from_translation(glam::Vec3::new(512.0, 15.0, 512.0)),
            original,
            Parent { entity: terrain },
        ));

        let (loaded, report) = round_trip(&mut world, &registry);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let water = *loaded
            .get::<crate::WaterComponent>(find(&loaded, "Water"))
            .expect("water component must survive");
        assert_eq!(
            water, original,
            "every water field must round-trip, not just the ones anyone remembered"
        );
        assert_eq!(
            loaded.get::<Parent>(find(&loaded, "Water")).unwrap().entity,
            find(&loaded, "Terrain"),
            "and the water's parent terrain link with it"
        );
    }

    #[test]
    fn foliage_now_survives_a_save_which_it_never_did_before() {
        let registry = component_registry();
        let mut world = World::new();
        let mut foliage = crate::FoliageComponent::default();
        foliage.enabled = true;
        foliage.density = 7.25;
        foliage.seed = 4242;
        foliage.max_instances = 31_000;
        world.spawn((Name::new("Ground"), Transform::default(), foliage));

        let (loaded, report) = round_trip(&mut world, &registry);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(
            *loaded
                .get::<crate::FoliageComponent>(find(&loaded, "Ground"))
                .unwrap(),
            foliage
        );
    }

    #[test]
    fn the_three_somnium_formats_are_mutually_exclusive() {
        // `version` discriminates formats, not revisions. Each reader must
        // refuse the other two outright rather than half-reading them —
        // this is the regression test for the collision where the
        // schema-driven dump was numbered 2, the same as a map recipe.
        let registry = component_registry();
        let mut loaded = World::new();

        let v1_dump = serde_json::json!({ "version": 1, "entities": [] });
        let v2_map = serde_json::json!({ "version": 2, "kind": "coastal" });
        assert_eq!(
            scene_from_json(&mut loaded, &registry, &v1_dump).unwrap_err(),
            SceneError::UnsupportedVersion(1)
        );
        assert_eq!(
            scene_from_json(&mut loaded, &registry, &v2_map).unwrap_err(),
            SceneError::UnsupportedVersion(2)
        );

        // And the map reader must refuse ours.
        let mut world = World::new();
        world.spawn((Name::new("Anything"), Transform::default()));
        let ours = serde_json::to_string(&scene_to_json(&mut world, &registry)).unwrap();
        assert!(
            crate::map::parse_map_kind_json(&ours).is_err(),
            "a schema scene must not parse as a map recipe"
        );
        assert!(crate::map::parse_map_kind_json(r#"{ "version": 2, "kind": "coastal" }"#).is_ok());
    }
}
