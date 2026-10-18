//! The world state: one store of story facts that quests, dialogue, scripts and saves all read.
//!
//! Only facts that differ from their declared default are stored, in a `BTreeMap`, so two playthroughs that reached
//! the same facts by different routes hold identical maps and serialise to identical bytes. Every write that changes
//! something bumps [`WorldState::revision`] (a per-frame cache key, in the spirit of the ECS's change signatures)
//! and returns a [`FactChange`] for the caller to publish.

use crate::expr::{Effect, Env, Func};
use crate::value::{Schema, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One fact that changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactChange {
    pub fact: String,
    pub old: Value,
    pub new: Value,
}

/// Story facts, stored as differences from their defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldState {
    values: BTreeMap<String, Value>,
    revision: u64,
}

impl WorldState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A fact's value (its default when unset), or `None` when the schema does not declare it.
    #[must_use]
    pub fn get(&self, schema: &Schema, id: &str) -> Option<Value> {
        let def = schema.get(id)?;
        Some(
            self.values
                .get(id)
                .cloned()
                .unwrap_or_else(|| def.default_value()),
        )
    }

    /// A fact's value; an undeclared one reads as `false`. Compiled conditions only name declared facts, so this
    /// fallback is for hosts reading loosely.
    #[must_use]
    pub fn read(&self, schema: &Schema, id: &str) -> Value {
        self.get(schema, id).unwrap_or(Value::Bool(false))
    }

    /// Set a fact, checked against its declaration. `Ok(None)` when the value did not change.
    pub fn set(
        &mut self,
        schema: &Schema,
        id: &str,
        value: Value,
    ) -> Result<Option<FactChange>, String> {
        let def = schema
            .get(id)
            .ok_or_else(|| format!("{id} is not a declared fact"))?;
        let new = def.accept(value)?;
        let old = self
            .values
            .get(id)
            .cloned()
            .unwrap_or_else(|| def.default_value());
        if old == new {
            return Ok(None);
        }
        if new == def.default_value() {
            self.values.remove(id);
        } else {
            self.values.insert(id.to_owned(), new.clone());
        }
        self.revision += 1;
        Ok(Some(FactChange {
            fact: id.to_owned(),
            old,
            new,
        }))
    }

    /// Apply one fact effect, reading its values from `env`. Commands are the caller's; they return `Ok(None)`.
    pub fn apply(
        &mut self,
        schema: &Schema,
        effect: &Effect,
        env: &dyn Env,
    ) -> Result<Option<FactChange>, String> {
        match resolve(effect, env) {
            Some(write) => self.write(schema, &write),
            None => Ok(None),
        }
    }

    /// Apply one fact write whose values are already computed.
    pub fn write(&mut self, schema: &Schema, write: &Write) -> Result<Option<FactChange>, String> {
        let current =
            |state: &Self, fact: &str| state.get(schema, fact).unwrap_or(Value::Bool(false));
        match write {
            Write::Set(fact, value) => self.set(schema, fact, value.clone()),
            Write::Add(fact, delta) => {
                let value = current(self, fact).as_int().saturating_add(*delta);
                self.set(schema, fact, Value::Int(value))
            }
            Write::Insert(fact, member) | Write::Remove(fact, member) => {
                let Value::Set(mut set) = current(self, fact) else {
                    return Err(format!("{fact} is not a set"));
                };
                if matches!(write, Write::Insert(..)) {
                    set.insert(member.clone());
                } else {
                    set.remove(member);
                }
                self.set(schema, fact, Value::Set(set))
            }
            Write::Clear(fact) => {
                let def = schema
                    .get(fact)
                    .ok_or_else(|| format!("{fact} is not a declared fact"))?;
                self.set(schema, fact, def.default_value())
            }
        }
    }

    /// Increases by one on every change. Equal revisions mean equal state.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The facts that differ from their defaults, in id order.
    pub fn explicit(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.values.iter().map(|(id, value)| (id.as_str(), value))
    }

    /// The facts a save keeps: the explicit ones whose declaration is `saved`.
    #[must_use]
    pub fn snapshot(&self, schema: &Schema) -> BTreeMap<String, Value> {
        self.values
            .iter()
            .filter(|(id, _)| schema.get(id).is_some_and(|def| def.saved))
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect()
    }

    /// Rebuild from a save written against a possibly older schema.
    ///
    /// A fact the schema no longer declares, or whose value no longer fits (an enum lost a variant), is dropped and
    /// reported rather than failing the whole load: content changes between builds, and refusing a player's save
    /// over one renamed flag is the wrong trade. Integers are clamped into a narrowed range.
    #[must_use]
    pub fn restore(schema: &Schema, values: &BTreeMap<String, Value>) -> (Self, Vec<String>) {
        let mut state = Self::new();
        let mut dropped = Vec::new();
        for (id, value) in values {
            match state.set(schema, id, value.clone()) {
                Ok(_) => {}
                Err(error) => dropped.push(format!("dropped {id}: {error}")),
            }
        }
        state.revision = 0;
        (state, dropped)
    }
}

/// A fact write with its values computed: what an [`Effect`] becomes once its expressions are read, so the reading
/// (which borrows the story) and the writing (which changes it) happen one after the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Set(String, Value),
    Add(String, i64),
    Insert(String, String),
    Remove(String, String),
    Clear(String),
}

/// Read an effect's values. `None` for commands.
#[must_use]
pub fn resolve(effect: &Effect, env: &dyn Env) -> Option<Write> {
    Some(match effect {
        Effect::Set { fact, value } => Write::Set(fact.clone(), value.eval(env)),
        Effect::Add { fact, delta } => Write::Add(fact.clone(), delta.eval(env).as_int()),
        Effect::Insert { fact, member } => {
            Write::Insert(fact.clone(), member.eval(env).as_text().to_owned())
        }
        Effect::Remove { fact, member } => {
            Write::Remove(fact.clone(), member.eval(env).as_text().to_owned())
        }
        Effect::Clear { fact } => Write::Clear(fact.clone()),
        Effect::Command { .. } => return None,
    })
}

/// A read-only view of the world state for evaluating conditions outside a running story.
pub struct StateView<'a> {
    pub state: &'a WorldState,
    pub schema: &'a Schema,
}

impl Env for StateView<'_> {
    fn fact(&self, id: &str) -> Value {
        self.state.read(self.schema, id)
    }

    fn call(&self, func: Func, args: &[Value]) -> Value {
        // No quests or dialogue here: the defaults.
        struct Defaults;
        impl Env for Defaults {
            fn fact(&self, _: &str) -> Value {
                Value::Bool(false)
            }
        }
        Defaults.call(func, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{Condition, Context, Effects};
    use crate::value::{FactDef, FactType};

    fn schema() -> Schema {
        Schema::new([
            FactDef::new("met.mireille", FactType::Bool),
            FactDef::new("mireille.affection", FactType::Int).with_range(-10, 10),
            FactDef::new("caelan.weaves", FactType::Set),
            FactDef::new("in_combat", FactType::Bool).unsaved(),
            FactDef::new("lang.patter", FactType::Enum).with_values(&["none", "phrases", "fluent"]),
        ])
        .unwrap()
    }

    fn run(state: &mut WorldState, schema: &Schema, source: &str) -> Vec<FactChange> {
        let effects = Effects::compile(source, Context::new(schema, &[])).unwrap();
        let mut changes = Vec::new();
        for effect in effects.list() {
            let before = state.clone();
            let env = StateView {
                state: &before,
                schema,
            };
            if let Some(change) = state.apply(schema, effect, &env).unwrap() {
                changes.push(change);
            }
        }
        changes
    }

    #[test]
    fn effects_change_facts_and_report_each_change_once() {
        let schema = schema();
        let mut state = WorldState::new();
        let changes = run(
            &mut state,
            &schema,
            "met.mireille = true; mireille.affection += 15; caelan.weaves add 'frost_step'; met.mireille = true",
        );
        assert_eq!(
            changes.len(),
            3,
            "the second identical write is not a change"
        );
        assert_eq!(
            state.read(&schema, "mireille.affection"),
            Value::Int(10),
            "clamped"
        );
        assert_eq!(state.revision(), 3);
        let condition = Condition::compile(
            "met.mireille and 'frost_step' in caelan.weaves",
            Context::new(&schema, &[]),
        )
        .unwrap();
        assert!(condition.eval(&StateView {
            state: &state,
            schema: &schema
        }));
    }

    #[test]
    fn default_values_are_not_stored() {
        let schema = schema();
        let mut state = WorldState::new();
        run(
            &mut state,
            &schema,
            "mireille.affection += 2; mireille.affection -= 2",
        );
        assert_eq!(state.explicit().count(), 0);
        assert_eq!(state.revision(), 2);
        run(
            &mut state,
            &schema,
            "caelan.weaves add 'a'; caelan.weaves remove 'a'",
        );
        assert_eq!(
            state.explicit().count(),
            0,
            "an emptied set is the default again"
        );
    }

    #[test]
    fn saves_round_trip_and_survive_schema_changes() {
        let schema = schema();
        let mut state = WorldState::new();
        run(
            &mut state,
            &schema,
            "met.mireille = true; in_combat = true; lang.patter = 'fluent'; mireille.affection = 7",
        );
        let saved = state.snapshot(&schema);
        assert!(
            !saved.contains_key("in_combat"),
            "unsaved facts stay out of saves"
        );
        let json = serde_json::to_string(&saved).unwrap();
        assert_eq!(
            json,
            r#"{"lang.patter":"fluent","met.mireille":true,"mireille.affection":7}"#
        );

        // The next build renames a fact, drops a variant and narrows a range.
        let newer = Schema::new([
            FactDef::new("met.mireille_vael", FactType::Bool),
            FactDef::new("mireille.affection", FactType::Int).with_range(0, 5),
            FactDef::new("lang.patter", FactType::Enum).with_values(&["none", "phrases"]),
        ])
        .unwrap();
        let (restored, dropped) =
            WorldState::restore(&newer, &serde_json::from_str(&json).unwrap());
        assert_eq!(dropped.len(), 2, "{dropped:?}");
        assert_eq!(restored.read(&newer, "mireille.affection"), Value::Int(5));
        assert_eq!(restored.revision(), 0);
    }
}
