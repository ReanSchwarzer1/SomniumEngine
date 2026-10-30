//! Story facts: the values the world state holds and the schema that declares them.
//!
//! A game declares every fact it keeps (`mireille.affection`, `caelan.weaves`, `march.frozen`) once, with a type, a
//! default and a line of documentation. Everything that reads or writes a fact (a quest stage, a dialogue choice, a
//! script, a save) is checked against that declaration when the content loads, so a typo is a load-time finding with
//! a place attached, not a flag that is silently never set.
//!
//! Families (`rep.*`) declare a whole namespace of facts of one type, for things a game counts per name: reputation
//! per faction, kills per creature, visits per town.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One fact's value.
///
/// Untagged, so a save or a document reads as plain JSON: `true`, `3`, `"conversant"`, `["frost_step"]`. An enum
/// fact holds its variant as [`Value::Text`]; the schema says which strings are allowed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
    Set(BTreeSet<String>),
}

impl Value {
    #[must_use]
    pub fn as_bool(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    #[must_use]
    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(value) => *value,
            Value::Bool(value) => i64::from(*value),
            _ => 0,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> &str {
        match self {
            Value::Text(text) => text,
            _ => "",
        }
    }

    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Text(_) => "text",
            Value::Set(_) => "set",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(value) => write!(f, "{value}"),
            Value::Int(value) => write!(f, "{value}"),
            Value::Text(text) => f.write_str(text),
            Value::Set(set) => {
                let items: Vec<&str> = set.iter().map(String::as_str).collect();
                write!(f, "{{{}}}", items.join(", "))
            }
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Text(value.to_owned())
    }
}

/// What kind of value a fact holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactType {
    Bool,
    /// A whole number, optionally clamped to `min..=max`.
    Int,
    /// Free text: a name the player chose, the last town visited.
    Text,
    /// One of `values`, ordered: `lang.patter >= "conversant"` compares by position.
    Enum,
    /// A set of names (`caelan.weaves`); `values`, when given, is the closed list of allowed members.
    Set,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// One declared fact, or a family of them when `id` ends in `.*`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactDef {
    pub id: String,
    #[serde(rename = "type")]
    pub ty: FactType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// Enum variants in order, or a set's allowed members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// The value an unset fact reads as. Omitted: false, 0 (or `min` when 0 is out of range), "", the first enum
    /// variant, the empty set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// Off for facts the game mirrors every session (is the player in combat, is it Lidfall): they are read by
    /// conditions but never written into a save.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub saved: bool,
}

impl FactDef {
    #[must_use]
    pub fn new(id: impl Into<String>, ty: FactType) -> Self {
        Self {
            id: id.into(),
            ty,
            min: None,
            max: None,
            values: Vec::new(),
            default: None,
            doc: String::new(),
            saved: true,
        }
    }

    #[must_use]
    pub fn with_values(mut self, values: &[&str]) -> Self {
        self.values = values.iter().map(|v| (*v).to_owned()).collect();
        self
    }

    #[must_use]
    pub fn with_range(mut self, min: i64, max: i64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    #[must_use]
    pub fn with_default(mut self, value: impl Into<Value>) -> Self {
        self.default = Some(value.into());
        self
    }

    #[must_use]
    pub fn unsaved(mut self) -> Self {
        self.saved = false;
        self
    }

    /// Whether this declares a family (`rep.*`).
    #[must_use]
    pub fn is_family(&self) -> bool {
        self.id.ends_with(".*")
    }

    /// The value an unset fact reads as.
    #[must_use]
    pub fn default_value(&self) -> Value {
        if let Some(value) = &self.default {
            return value.clone();
        }
        match self.ty {
            FactType::Bool => Value::Bool(false),
            FactType::Int => Value::Int(self.clamp(0)),
            FactType::Text => Value::Text(String::new()),
            FactType::Enum => Value::Text(self.values.first().cloned().unwrap_or_default()),
            FactType::Set => Value::Set(BTreeSet::new()),
        }
    }

    fn clamp(&self, value: i64) -> i64 {
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }

    /// Check `value` against this declaration and return the value to store.
    ///
    /// Integers are clamped into range rather than refused: `affection += 5` at the cap is the author's intent
    /// ("as high as it goes"), and refusing it would drop the rest of the effect batch with it.
    pub fn accept(&self, value: Value) -> Result<Value, String> {
        match (self.ty, value) {
            (FactType::Bool, value @ Value::Bool(_)) => Ok(value),
            (FactType::Int, Value::Int(number)) => Ok(Value::Int(self.clamp(number))),
            (FactType::Text, value @ Value::Text(_)) => Ok(value),
            (FactType::Enum, Value::Text(text)) => {
                if self.values.contains(&text) {
                    Ok(Value::Text(text))
                } else {
                    Err(format!(
                        "\"{text}\" is not one of {} ({})",
                        self.id,
                        self.values.join(", ")
                    ))
                }
            }
            (FactType::Set, Value::Set(members)) => {
                if let Some(bad) = members.iter().find(|m| !self.allows_member(m)) {
                    Err(format!("\"{bad}\" is not an allowed member of {}", self.id))
                } else {
                    Ok(Value::Set(members))
                }
            }
            (ty, value) => Err(format!(
                "{} is a {} fact; got a {} ({value})",
                self.id,
                type_label(ty),
                value.type_name()
            )),
        }
    }

    /// Whether a set fact may contain `member`.
    #[must_use]
    pub fn allows_member(&self, member: &str) -> bool {
        self.values.is_empty() || self.values.iter().any(|v| v == member)
    }

    fn validate(&self) -> Result<(), String> {
        let id = self.id.strip_suffix(".*").unwrap_or(&self.id);
        if !valid_id(id) {
            return Err(format!(
                "fact id \"{}\" must be letters, digits and _ in dot-separated parts, starting with a letter",
                self.id
            ));
        }
        match self.ty {
            FactType::Int => {
                if let (Some(min), Some(max)) = (self.min, self.max)
                    && min > max
                {
                    return Err(format!("{}: min {min} is above max {max}", self.id));
                }
                if !self.values.is_empty() {
                    return Err(format!("{}: an int fact has no values list", self.id));
                }
            }
            FactType::Enum => {
                if self.values.is_empty() {
                    return Err(format!("{}: an enum fact needs its values", self.id));
                }
            }
            FactType::Bool | FactType::Text => {
                if !self.values.is_empty() {
                    return Err(format!("{}: only enum and set facts list values", self.id));
                }
            }
            FactType::Set => {}
        }
        if self.ty != FactType::Int && (self.min.is_some() || self.max.is_some()) {
            return Err(format!("{}: only int facts have min and max", self.id));
        }
        let mut seen = BTreeSet::new();
        for value in &self.values {
            if value.trim().is_empty() || !seen.insert(value) {
                return Err(format!("{}: empty or repeated value \"{value}\"", self.id));
            }
        }
        if let Some(default) = &self.default {
            let accepted = self.accept(default.clone())?;
            if accepted != *default {
                return Err(format!("{}: default {default} is out of range", self.id));
            }
        }
        Ok(())
    }
}

pub(crate) fn type_label(ty: FactType) -> &'static str {
    match ty {
        FactType::Bool => "bool",
        FactType::Int => "int",
        FactType::Text => "text",
        FactType::Enum => "enum",
        FactType::Set => "set",
    }
}

/// A fact or story id: letters, digits and `_` in dot-separated parts, the first starting with a letter or `_`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

/// Every fact a game declares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    facts: BTreeMap<String, FactDef>,
    /// `(prefix with the trailing dot, definition)`, longest prefix first so `rep.church.*` wins over `rep.*`.
    families: Vec<(String, FactDef)>,
}

impl Schema {
    /// Build from declarations, refusing bad and duplicate ones.
    pub fn new(defs: impl IntoIterator<Item = FactDef>) -> Result<Self, Vec<String>> {
        let mut schema = Self::default();
        let mut errors = Vec::new();
        for def in defs {
            if let Err(error) = def.validate() {
                errors.push(error);
                continue;
            }
            if def.is_family() {
                let prefix = def.id.trim_end_matches('*').to_owned();
                if schema.families.iter().any(|(p, _)| *p == prefix) {
                    errors.push(format!("fact family {} is declared twice", def.id));
                } else {
                    schema.families.push((prefix, def));
                }
            } else if schema.facts.contains_key(&def.id) {
                errors.push(format!("fact {} is declared twice", def.id));
            } else {
                schema.facts.insert(def.id.clone(), def);
            }
        }
        schema
            .families
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        if errors.is_empty() {
            Ok(schema)
        } else {
            Err(errors)
        }
    }

    /// The declaration that governs `id`: the exact fact, else the longest family whose prefix it extends.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&FactDef> {
        if let Some(def) = self.facts.get(id) {
            return Some(def);
        }
        self.families
            .iter()
            .find(|(prefix, _)| {
                id.len() > prefix.len() && id.starts_with(prefix.as_str()) && valid_id(id)
            })
            .map(|(_, def)| def)
    }

    /// Every exact fact and family, in stable order (facts by id, then families).
    pub fn definitions(&self) -> impl Iterator<Item = &FactDef> {
        self.facts
            .values()
            .chain(self.families.iter().map(|(_, def)| def))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.facts.len() + self.families.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        Schema::new([
            FactDef::new("met.mireille", FactType::Bool),
            FactDef::new("mireille.affection", FactType::Int).with_range(-10, 10),
            FactDef::new("lang.patter", FactType::Enum).with_values(&[
                "none",
                "phrases",
                "conversant",
                "fluent",
                "native",
            ]),
            FactDef::new("caelan.weaves", FactType::Set),
            FactDef::new("rep.*", FactType::Int).with_range(-100, 100),
            FactDef::new("rep.church.*", FactType::Int).with_range(-5, 5),
        ])
        .unwrap()
    }

    #[test]
    fn values_read_as_plain_json() {
        let value: Value = serde_json::from_str(r#"["b","a"]"#).unwrap();
        assert_eq!(value, Value::Set(["a".into(), "b".into()].into()));
        assert_eq!(serde_json::to_string(&Value::Int(3)).unwrap(), "3");
        assert_eq!(
            serde_json::from_str::<Value>("\"conversant\"").unwrap(),
            Value::Text("conversant".into())
        );
    }

    #[test]
    fn families_resolve_by_longest_prefix() {
        let schema = schema();
        assert_eq!(schema.get("rep.lance").unwrap().id, "rep.*");
        assert_eq!(schema.get("rep.church.tyffir").unwrap().id, "rep.church.*");
        assert!(schema.get("rep.").is_none());
        assert!(schema.get("rep").is_none());
        assert!(schema.get("unknown").is_none());
    }

    #[test]
    fn ints_clamp_and_enums_refuse_strangers() {
        let schema = schema();
        let affection = schema.get("mireille.affection").unwrap();
        assert_eq!(affection.accept(Value::Int(40)).unwrap(), Value::Int(10));
        let patter = schema.get("lang.patter").unwrap();
        assert!(patter.accept("fluent".into()).is_ok());
        assert!(patter.accept("klingon".into()).is_err());
        assert!(patter.accept(Value::Int(1)).is_err());
        assert_eq!(patter.default_value(), Value::Text("none".into()));
    }

    #[test]
    fn bad_declarations_are_refused_with_reasons() {
        let errors = Schema::new([
            FactDef::new("a", FactType::Enum),
            FactDef::new("b", FactType::Int).with_range(5, 1),
            FactDef::new("1bad", FactType::Bool),
            FactDef::new("c", FactType::Bool),
            FactDef::new("c", FactType::Bool),
            FactDef::new("d", FactType::Int)
                .with_range(0, 3)
                .with_default(Value::Int(9)),
        ])
        .unwrap_err();
        assert_eq!(errors.len(), 5, "{errors:?}");
    }

    #[test]
    fn declarations_round_trip_and_reject_unknown_fields() {
        let def: FactDef =
            serde_json::from_str(r#"{"id":"x","type":"int","min":0,"max":3,"doc":"d"}"#).unwrap();
        assert_eq!(def.default_value(), Value::Int(0));
        assert!(serde_json::from_str::<FactDef>(r#"{"id":"x","type":"int","mn":0}"#).is_err());
        let json = serde_json::to_string(&def).unwrap();
        assert_eq!(serde_json::from_str::<FactDef>(&json).unwrap(), def);
    }
}
