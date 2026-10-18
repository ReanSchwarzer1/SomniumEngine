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
    /// Places it is heard in (`aldermere`, `eirvane_hall/kitchen`): a place matches itself and everything under it.
    /// Empty: anywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<String>,
    /// Hours of the Lid's day it is heard, `[from, to)` of `0..24`, wrapping past midnight (`[20, 4]`). None: any
    /// hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<[f32; 2]>,
    /// How much of the sun the Lid covers, `[least, most]` of `0..1` (`[0.8, 1]` is Lidfall). None: any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eclipse: Option<[f32; 2]>,
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
    /// Places it is heard in (`aldermere`, `eirvane_hall/kitchen`): a place matches itself and everything under it.
    /// Empty: anywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<String>,
    /// Hours of the Lid's day it is heard, `[from, to)` of `0..24`, wrapping past midnight (`[20, 4]`). None: any
    /// hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<[f32; 2]>,
    /// How much of the sun the Lid covers, `[least, most]` of `0..1` (`[0.8, 1]` is Lidfall). None: any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eclipse: Option<[f32; 2]>,
}

/// Where and when a bark is asked for, and of whom. `hour` and `eclipse` are the scene's TimeOfDay `hour` and
/// `eclipse`; `place` is the game's name for where the listener is (`aldermere/market`).
#[derive(Clone, Copy, Debug)]
pub struct BarkContext<'a> {
    pub place: &'a str,
    /// `0..24`; `None` keeps barks with `hours` quiet.
    pub hour: Option<f32>,
    /// `0..1`; `0` is a clear sky.
    pub eclipse: f32,
    /// Who could say it: cast ids or roles (`villager`, `watch_guard`).
    pub near: &'a [&'a str],
    /// Barks and chatter (`set/id`) not to pick now: what was heard lately.
    pub avoid: &'a [String],
    /// Only barks and chatter of at least this priority: something more important is being said.
    pub min_priority: i32,
}

impl<'a> BarkContext<'a> {
    /// Just who is near: no place, no hour, a clear sky.
    #[must_use]
    pub fn near(near: &'a [&'a str]) -> Self {
        Self {
            place: "",
            hour: None,
            eclipse: 0.0,
            near,
            avoid: &[],
            min_priority: i32::MIN,
        }
    }
}

/// Whether a bark's or an exchange's place and time allow it here and now.
pub(crate) fn fits(
    places: &[String],
    hours: Option<[f32; 2]>,
    eclipse: Option<[f32; 2]>,
    context: &BarkContext,
) -> bool {
    let place = places.is_empty()
        || places.iter().any(|p| {
            context.place == p
                || context
                    .place
                    .strip_prefix(p.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        });
    let hour = hours.is_none_or(|[from, to]| {
        context.hour.is_some_and(|h| {
            if from <= to {
                (from..to).contains(&h)
            } else {
                h >= from || h < to
            }
        })
    });
    let lid = eclipse.is_none_or(|[least, most]| (least..=most).contains(&context.eclipse));
    place && hour && lid
}

fn window_problems(
    at: &str,
    places: &[String],
    hours: Option<[f32; 2]>,
    eclipse: Option<[f32; 2]>,
    problems: &mut Vec<Problem>,
) {
    if places
        .iter()
        .any(|p| p.is_empty() || p.split('/').any(str::is_empty))
    {
        problems.push(Problem::error(
            at,
            "places are names like aldermere or eirvane_hall/kitchen",
        ));
    }
    if hours.is_some_and(|h| h.iter().any(|x| !(0.0..=24.0).contains(x))) {
        problems.push(Problem::error(at, "hours are [from, to] within 0..24"));
    }
    if eclipse.is_some_and(|[least, most]| {
        !(0.0..=1.0).contains(&least) || !(0.0..=1.0).contains(&most) || least > most
    }) {
        problems.push(Problem::error(at, "eclipse is [least, most] within 0..1"));
    }
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
            window_problems(
                &bark.id,
                &bark.places,
                bark.hours,
                bark.eclipse,
                &mut problems,
            );
        }
        for chatter in &self.chatter {
            window_problems(
                &chatter.id,
                &chatter.places,
                chatter.hours,
                chatter.eclipse,
                &mut problems,
            );
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
