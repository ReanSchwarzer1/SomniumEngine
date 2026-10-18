//! Barks and ambient chatter (`*.sombarks`).
//!
//! A bark is one line a character throws out when something happens: spotted, hurt, a door opened, the Lid falling.
//! Chatter is a short exchange between people nearby, cast at run time onto whoever is standing there (roles `a`,
//! `b`, ...), which is how a crowd talks about the news in its own language. Both are picked by trigger, then
//! condition, then cooldown and `once`, then the highest priority, then a weighted draw from the story's saved
//! generator, so the same save and the same events bark the same way.

use crate::expr::{Condition, Context, Effects};
use crate::problem::{Problem, has_errors};
use crate::text::LocalText;
use crate::value::valid_id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The barks format this build reads.
pub const BARKS_VERSION: u32 = 1;

fn one() -> u32 {
    1
}

fn is_one(value: &u32) -> bool {
    *value == 1
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

/// One bark.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarkDef {
    pub id: String,
    /// The event that may play it (`spotted`, `hurt`, `lidfall`). Free; the game sends it.
    pub trigger: String,
    /// Who says it: a cast id or a role the game uses (`watch_guard`). Empty: whoever the game offers first.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    pub text: LocalText,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub voice: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: u32,
    /// Game-time milliseconds before this bark may play again.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub cooldown_ms: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effects: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// One line of chatter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatterLine {
    /// `a`, `b`, ...: who in the exchange says it.
    pub role: String,
    pub text: LocalText,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub voice: String,
}

/// One exchange.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatterDef {
    pub id: String,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub cooldown_ms: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effects: String,
    pub lines: Vec<ChatterLine>,
}

impl ChatterDef {
    /// The roles in first-speaking order.
    #[must_use]
    pub fn roles(&self) -> Vec<&str> {
        let mut roles: Vec<&str> = Vec::new();
        for line in &self.lines {
            if !roles.contains(&line.role.as_str()) {
                roles.push(&line.role);
            }
        }
        roles
    }
}

/// A barks file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarkSetAsset {
    pub version: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barks: Vec<BarkDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chatter: Vec<ChatterDef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl BarkSetAsset {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn structural_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if self.version != BARKS_VERSION {
            problems.push(Problem::error(
                "",
                format!("barks version {} is not {BARKS_VERSION}", self.version),
            ));
        }
        if !valid_id(&self.id) {
            problems.push(Problem::error(
                "",
                format!("barks id \"{}\" is not a valid id", self.id),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut check = |id: &str, trigger: &str, weight: u32, when: &str, effects: &str| {
            if !valid_id(id) || id.contains('.') || !ids.insert(id.to_owned()) {
                problems.push(Problem::error(
                    id,
                    "bark and chatter ids are unique letters, digits and _",
                ));
            }
            if trigger.trim().is_empty() {
                problems.push(Problem::error(id, "a trigger is needed"));
            }
            if weight == 0 {
                problems.push(Problem::error(
                    id,
                    "weight 0 never plays; remove it instead",
                ));
            }
            if let Err(e) = crate::expr::syntax(when, false) {
                problems.push(Problem::error(id, format!("when: {e}")));
            }
            if let Err(e) = crate::expr::syntax(effects, true) {
                problems.push(Problem::error(id, format!("effects: {e}")));
            }
        };
        for bark in &self.barks {
            check(
                &bark.id,
                &bark.trigger,
                bark.weight,
                &bark.when,
                &bark.effects,
            );
        }
        for chatter in &self.chatter {
            check(
                &chatter.id,
                &chatter.trigger,
                chatter.weight,
                &chatter.when,
                &chatter.effects,
            );
        }
        for bark in &self.barks {
            if bark.text.is_empty() {
                problems.push(Problem::error(&bark.id, "a bark needs text"));
            }
        }
        for chatter in &self.chatter {
            if chatter.lines.len() < 2 {
                problems.push(Problem::error(
                    &chatter.id,
                    "chatter is two lines or more; one is a bark",
                ));
            }
            if chatter
                .lines
                .iter()
                .any(|l| l.role.trim().is_empty() || l.text.is_empty())
            {
                problems.push(Problem::error(
                    &chatter.id,
                    "every chatter line needs a role and text",
                ));
            }
        }
        problems
    }
}

/// A bark with its expressions compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Bark {
    pub def: BarkDef,
    pub when: Condition,
    pub effects: Effects,
}

/// Chatter with its expressions compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Chatter {
    pub def: ChatterDef,
    pub when: Condition,
    pub effects: Effects,
}

/// A checked barks file.
#[derive(Clone, Debug, PartialEq)]
pub struct BarkSet {
    pub id: String,
    pub barks: Vec<Bark>,
    pub chatter: Vec<Chatter>,
}

impl BarkSet {
    pub fn compile(mut asset: BarkSetAsset, ctx: Context<'_>) -> Result<Self, Vec<Problem>> {
        let mut problems = asset.structural_problems();
        let set = asset.id.clone();
        let mut compile = |id: &str, when: &str, effects: &str| {
            let when = Condition::compile(when, ctx).unwrap_or_else(|e| {
                problems.push(Problem::error(id, format!("when: {e}")));
                Condition::always()
            });
            let effects = Effects::compile(effects, ctx).unwrap_or_else(|e| {
                problems.push(Problem::error(id, format!("effects: {e}")));
                Effects::default()
            });
            (when, effects)
        };
        let mut barks = Vec::new();
        for mut def in std::mem::take(&mut asset.barks) {
            def.text.assign_key(|| format!("bark.{set}.{}", def.id));
            let (when, effects) = compile(&def.id, &def.when, &def.effects);
            barks.push(Bark { def, when, effects });
        }
        let mut chatter = Vec::new();
        for mut def in std::mem::take(&mut asset.chatter) {
            for (index, line) in def.lines.iter_mut().enumerate() {
                line.text
                    .assign_key(|| format!("chatter.{set}.{}.{index}", def.id));
            }
            let (when, effects) = compile(&def.id, &def.when, &def.effects);
            chatter.push(Chatter { def, when, effects });
        }
        if has_errors(&problems) {
            return Err(problems);
        }
        Ok(Self {
            id: asset.id,
            barks,
            chatter,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn watch() -> BarkSetAsset {
        BarkSetAsset::from_json(
            r#"{"version": 1, "id": "watch",
              "barks": [
                {"id": "spotted_1", "trigger": "spotted", "speaker": "watch_guard", "text": "Oi! You!", "language": "patter", "cooldown_ms": 10000},
                {"id": "spotted_2", "trigger": "spotted", "speaker": "watch_guard", "text": "Stop there!", "weight": 3},
                {"id": "spotted_lidless", "trigger": "spotted", "text": "That's the frost-touched one!", "when": "lidless >= 3", "priority": 5, "once": true}
              ],
              "chatter": [
                {"id": "march_news", "trigger": "ambient", "lines": [
                  {"role": "a", "text": "Heard about the March?", "language": "tyffiran"},
                  {"role": "b", "text": "Frozen. All of it."},
                  {"role": "a", "text": "Gods keep them."}]}
              ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_barks_file_compiles_and_keys_its_lines() {
        let schema = crate::value::Schema::new([crate::value::FactDef::new(
            "lidless",
            crate::value::FactType::Int,
        )])
        .unwrap();
        let set = BarkSet::compile(watch(), Context::new(&schema, &[])).unwrap();
        assert_eq!(set.barks[0].def.text.key, "bark.watch.spotted_1");
        assert_eq!(
            set.chatter[0].def.lines[2].text.key,
            "chatter.watch.march_news.2"
        );
        assert_eq!(set.chatter[0].def.roles(), ["a", "b"]);
    }

    #[test]
    fn bark_mistakes_are_found() {
        let mut asset = watch();
        asset.barks[1].weight = 0;
        asset.barks[2].id = "spotted_1".into();
        asset.chatter[0].lines.truncate(1);
        assert_eq!(asset.structural_problems().len(), 3);
    }
}
