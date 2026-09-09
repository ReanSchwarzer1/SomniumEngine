//! Durable object patches shared by prefab templates and player saves.
//! Objects are diffed by key, arrays are atomic values. No array index is an
//! entity identity. Missing parents are reported, never invented on rebase.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValuePatch {
    pub path: Vec<String>,
    /// None removes a key; Some(Null) explicitly writes JSON null.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub value: Option<Value>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

pub fn diff(before: &Value, after: &Value) -> Vec<ValuePatch> {
    fn walk(before: &Value, after: &Value, path: &mut Vec<String>, out: &mut Vec<ValuePatch>) {
        if before == after {
            return;
        }
        if let (Some(a), Some(b)) = (before.as_object(), after.as_object()) {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                path.push(key.clone());
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => walk(a, b, path, out),
                    (_, b) => out.push(ValuePatch {
                        path: path.clone(),
                        value: b.cloned(),
                    }),
                }
                path.pop();
            }
        } else {
            out.push(ValuePatch {
                path: path.clone(),
                value: Some(after.clone()),
            });
        }
    }
    let mut out = Vec::new();
    walk(before, after, &mut Vec::new(), &mut out);
    out
}

/// Apply where the new content still has a parent. Return orphaned patches.
pub fn apply(document: &mut Value, patches: &[ValuePatch]) -> Vec<ValuePatch> {
    let mut orphaned = Vec::new();
    for patch in patches {
        let Some((key, parents)) = patch.path.split_last() else {
            if let Some(value) = &patch.value {
                *document = value.clone();
            }
            continue;
        };
        fn descend<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
            match path.split_first() {
                Some((head, tail)) => descend(value.get_mut(head)?, tail),
                None => Some(value),
            }
        }
        let Some(target) = descend(document, parents) else {
            orphaned.push(patch.clone());
            continue;
        };
        if let Some(map) = target.as_object_mut() {
            if let Some(value) = &patch.value {
                map.insert(key.clone(), value.clone());
            } else {
                map.remove(key);
            }
        } else {
            orphaned.push(patch.clone());
        }
    }
    orphaned
}

pub fn entities(document: &Value) -> Result<BTreeMap<String, Value>, String> {
    let list = document
        .get("entities")
        .and_then(Value::as_array)
        .ok_or("missing entities")?;
    let mut out = BTreeMap::new();
    for entry in list {
        let id = entry
            .get("persistent_id")
            .and_then(Value::as_str)
            .ok_or("missing entity id")?;
        let parsed = somnium_ecs::PersistentId::parse_hex(id).ok_or("invalid entity id")?;
        if parsed.is_none() || out.insert(parsed.to_string(), entry.clone()).is_some() {
            return Err("zero or duplicate entity id".into());
        }
    }
    Ok(out)
}

pub fn document(entities: BTreeMap<String, Value>) -> Value {
    serde_json::json!({ "version": crate::scene_schema::SCENE_VERSION,
        "entities": entities.into_values().collect::<Vec<_>>() })
}

/// Remap tagged references recursively, retaining references outside the set.
pub fn remap(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for key in ["persistent_id", "$entity"] {
                if let Some(id) = fields.get(key).and_then(Value::as_str)
                    && let Some(new) = ids.get(id)
                {
                    fields.insert(key.into(), Value::String(new.clone()));
                }
            }
            for child in fields.values_mut() {
                remap(child, ids);
            }
        }
        Value::Array(values) => {
            for child in values {
                remap(child, ids);
            }
        }
        _ => {}
    }
}
