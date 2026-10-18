//! Quest assets (`*.somquest`): stages, objectives, branches, failure and journal lines.
//!
//! A quest is a small graph of stages. Entering a stage writes its journal lines (each with a truth tier), runs its
//! effects and activates its objectives; the stage's `next` links, tried in order, say where the quest goes once
//! they hold. A link with no condition means "when every required objective here is done". A required objective
//! that fails, with no link to catch it, fails the quest; a link like `{"when": "failed('varga_job', 'unseen')",
//! "to": "loud_way"}` turns the failure into a branch instead.
//!
//! Objectives complete by a condition (`done_when`), by a counter reaching its target (`count`), by the game
//! reporting arrival at their marker, by a story command (`complete_objective`), or when their timer runs out
//! (`on_expire: complete`). Their markers are what the HUD puts on the world canvas.

use crate::expr::{Condition, Context, Effects, ExprError};
use crate::problem::{Problem, has_errors};
use crate::text::LocalText;
use crate::value::{FactType, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The quest format this build reads.
pub const QUEST_VERSION: u32 = 1;

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

/// A line the journal gains when a stage is entered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalLine {
    pub id: String,
    pub text: LocalText,
    /// How the protagonist knows this (one of the manifest's tiers). Empty: untiered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tier: String,
}

/// A counter objective: done when `fact >= target`; the journal shows `fact / target`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountDef {
    pub fact: String,
    pub target: i64,
}

/// Where an objective points on the world canvas.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerDef {
    /// A named place the game resolves (an entity name, a spawn point). Wins over `position` when both are given.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anchor: String,
    /// A world position in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    /// Arrival radius in metres, and the marker's area when drawn as a zone.
    #[serde(default)]
    pub radius: f32,
    /// Reaching the marker completes the objective.
    #[serde(default, skip_serializing_if = "is_false")]
    pub arrive: bool,
}

/// What a running-out timer does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expire {
    #[default]
    Fail,
    Complete,
}

/// One objective.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveDef {
    /// Unique within the quest, across stages: `done('quest', 'objective')` names it.
    pub id: String,
    pub text: LocalText,
    /// Optional objectives never hold a stage back and never fail the quest.
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    /// Tracked and completable but not listed (a secret way through).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub done_when: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fail_when: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<CountDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<MarkerDef>,
    /// Game-time milliseconds from activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_ms: Option<u64>,
    #[serde(default)]
    pub on_expire: Expire,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_done: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_fail: String,
}

/// Where a stage goes, and when.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageLink {
    /// Empty: when every required objective of the stage is done.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    pub to: String,
}

/// How a stage ends the quest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageEnd {
    Complete,
    Fail,
}

/// One stage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageDef {
    pub id: String,
    /// A heading for the stage in the journal. Optional.
    #[serde(default, skip_serializing_if = "LocalText::is_empty")]
    pub title: LocalText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub journal: Vec<JournalLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objectives: Vec<ObjectiveDef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_enter: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<StageLink>,
    /// Entering this stage completes or fails the quest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<StageEnd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A quest asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAsset {
    pub version: u32,
    pub id: String,
    pub title: LocalText,
    #[serde(default, skip_serializing_if = "LocalText::is_empty")]
    pub summary: LocalText,
    /// Free: `main`, `companion`, `faction`, `contract`, `event`. The journal groups by it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// A cast id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub giver: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    /// Higher sorts first in the journal and wins marker overlaps.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority: i32,
    /// Starts by itself when this holds. Empty: only `start_quest`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub start_when: String,
    /// May be started again after it ends.
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeatable: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fail_when: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_start: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_complete: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_fail: String,
    /// The first stage. Empty: the first listed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first: String,
    pub stages: Vec<StageDef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl QuestAsset {
    #[must_use]
    pub fn new(id: impl Into<String>, title: &str) -> Self {
        Self {
            version: QUEST_VERSION,
            id: id.into(),
            title: title.into(),
            summary: LocalText::default(),
            category: String::new(),
            giver: String::new(),
            region: String::new(),
            priority: 0,
            start_when: String::new(),
            repeatable: false,
            fail_when: String::new(),
            on_start: String::new(),
            on_complete: String::new(),
            on_fail: String::new(),
            first: String::new(),
            stages: Vec::new(),
            note: String::new(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> Result<String, String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        Ok(text)
    }

    #[must_use]
    pub fn stage(&self, id: &str) -> Option<&StageDef> {
        self.stages.iter().find(|s| s.id == id)
    }

    /// The first stage's id.
    #[must_use]
    pub fn first_stage(&self) -> Option<&str> {
        if self.first.is_empty() {
            self.stages.first().map(|s| s.id.as_str())
        } else {
            self.stage(&self.first).map(|s| s.id.as_str())
        }
    }

    /// Every objective with the stage it belongs to.
    pub fn objectives(&self) -> impl Iterator<Item = (&StageDef, &ObjectiveDef)> {
        self.stages
            .iter()
            .flat_map(|stage| stage.objectives.iter().map(move |o| (stage, o)))
    }

    /// Problems that need no manifest.
    #[must_use]
    pub fn structural_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut error = |at: String, message: String| problems.push(Problem::error(at, message));
        if self.version != QUEST_VERSION {
            error(
                String::new(),
                format!("quest version {} is not {QUEST_VERSION}", self.version),
            );
        }
        if !valid_id(&self.id) {
            error(
                String::new(),
                format!("quest id \"{}\" is not a valid id", self.id),
            );
        }
        if self.title.is_empty() {
            error(String::new(), "a quest needs a title".into());
        }
        if self.stages.is_empty() {
            error(String::new(), "a quest needs at least one stage".into());
        }
        let stage_ids: BTreeSet<&str> = self.stages.iter().map(|s| s.id.as_str()).collect();
        if stage_ids.len() != self.stages.len() {
            error(String::new(), "two stages share an id".into());
        }
        if !self.first.is_empty() && !stage_ids.contains(self.first.as_str()) {
            error(
                String::new(),
                format!("first stage {} does not exist", self.first),
            );
        }
        let mut objective_ids = BTreeSet::new();
        for (field, source, effects) in [
            ("start_when", &self.start_when, false),
            ("fail_when", &self.fail_when, false),
            ("on_start", &self.on_start, true),
            ("on_complete", &self.on_complete, true),
            ("on_fail", &self.on_fail, true),
        ] {
            if let Err(e) = crate::expr::syntax(source, effects) {
                error(String::new(), format!("{field}: {e}"));
            }
        }
        for stage in &self.stages {
            let at = format!("stage {}", stage.id);
            if !valid_id(&stage.id) || stage.id.contains('.') {
                error(at.clone(), "stage ids are letters, digits and _".into());
            }
            if let Err(e) = crate::expr::syntax(&stage.on_enter, true) {
                error(at.clone(), format!("on_enter: {e}"));
            }
            let mut lines = BTreeSet::new();
            for line in &stage.journal {
                if !valid_id(&line.id) || line.id.contains('.') || !lines.insert(&line.id) {
                    error(
                        at.clone(),
                        format!("journal line id \"{}\" is invalid or repeated", line.id),
                    );
                }
                if line.text.is_empty() {
                    error(at.clone(), format!("journal line {} has no text", line.id));
                }
            }
            for link in &stage.next {
                if !stage_ids.contains(link.to.as_str()) {
                    error(
                        at.clone(),
                        format!("next leads to {}, which is not a stage", link.to),
                    );
                }
                if link.to == stage.id {
                    error(at.clone(), "a stage cannot lead to itself".into());
                }
                if let Err(e) = crate::expr::syntax(&link.when, false) {
                    error(at.clone(), format!("next to {}: {e}", link.to));
                }
            }
            if stage.end.is_some() && (!stage.next.is_empty() || !stage.objectives.is_empty()) {
                error(
                    at.clone(),
                    "an ending stage has no objectives or next links".into(),
                );
            }
            if stage.end.is_none() && stage.next.is_empty() {
                error(at.clone(), "a stage needs next links or an end".into());
            }
            for objective in &stage.objectives {
                let at = format!("{at} / objective {}", objective.id);
                if !valid_id(&objective.id) || objective.id.contains('.') {
                    error(at.clone(), "objective ids are letters, digits and _".into());
                }
                if !objective_ids.insert(objective.id.as_str()) {
                    error(at.clone(), "objective ids are unique within a quest".into());
                }
                if objective.text.is_empty() && !objective.hidden {
                    error(at.clone(), "a listed objective needs text".into());
                }
                if objective.timer_ms == Some(0) {
                    error(at.clone(), "timer_ms must be above zero".into());
                }
                if let Some(marker) = &objective.marker {
                    if marker.anchor.is_empty() && marker.position.is_none() {
                        error(at.clone(), "a marker needs an anchor or a position".into());
                    }
                    if !(marker.radius.is_finite() && marker.radius >= 0.0)
                        || marker
                            .position
                            .is_some_and(|p| p.iter().any(|v| !v.is_finite()))
                    {
                        error(
                            at.clone(),
                            "marker numbers must be finite, radius not negative".into(),
                        );
                    }
                    if marker.arrive && marker.radius <= 0.0 {
                        error(at.clone(), "an arrival marker needs a radius".into());
                    }
                }
                for (field, source, effects) in [
                    ("done_when", &objective.done_when, false),
                    ("fail_when", &objective.fail_when, false),
                    ("on_done", &objective.on_done, true),
                    ("on_fail", &objective.on_fail, true),
                ] {
                    if let Err(e) = crate::expr::syntax(source, effects) {
                        error(at.clone(), format!("{field}: {e}"));
                    }
                }
            }
        }
        problems
    }

    /// Stages no path from the first stage reaches.
    #[must_use]
    pub fn unreachable(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<&str> = self.first_stage().into_iter().collect();
        while let Some(id) = stack.pop() {
            if seen.insert(id)
                && let Some(stage) = self.stage(id)
            {
                stack.extend(stage.next.iter().map(|l| l.to.as_str()));
            }
        }
        self.stages
            .iter()
            .filter(|s| !seen.contains(s.id.as_str()))
            .map(|s| s.id.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Compiled form

/// An objective with its expressions compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub def: ObjectiveDef,
    pub done_when: Condition,
    pub fail_when: Condition,
    pub on_done: Effects,
    pub on_fail: Effects,
}

/// A stage with its expressions compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub def: StageDef,
    pub on_enter: Effects,
    /// `(condition, target)`; `None` is "every required objective done".
    pub next: Vec<(Option<Condition>, String)>,
    pub objectives: Vec<Objective>,
}

/// A checked quest.
#[derive(Clone, Debug, PartialEq)]
pub struct Quest {
    pub asset: QuestAsset,
    pub warnings: Vec<Problem>,
    pub start_when: Option<Condition>,
    pub fail_when: Condition,
    pub on_start: Effects,
    pub on_complete: Effects,
    pub on_fail: Effects,
    stages: Vec<Stage>,
}

impl Quest {
    pub fn compile(mut asset: QuestAsset, ctx: Context<'_>) -> Result<Self, Vec<Problem>> {
        let mut problems = asset.structural_problems();
        assign_keys(&mut asset);
        let mut report = |at: &str, field: &str, error: ExprError| {
            problems.push(Problem::error(at, format!("{field}: {error}")));
        };
        let mut condition = |source: &str, at: &str, field: &str| {
            Condition::compile(source, ctx).unwrap_or_else(|e| {
                report(at, field, e);
                Condition::always()
            })
        };
        let start_when = (!asset.start_when.trim().is_empty())
            .then(|| condition(&asset.start_when, "", "start_when"));
        let fail_when = condition(&asset.fail_when, "", "fail_when");
        let mut stages = Vec::new();
        let mut counts = Vec::new();
        let mut stage_conditions = Vec::new();
        for stage in &asset.stages {
            let at = format!("stage {}", stage.id);
            let mut next = Vec::new();
            for link in &stage.next {
                next.push((
                    (!link.when.trim().is_empty())
                        .then(|| condition(&link.when, &at, &format!("next to {}", link.to))),
                    link.to.clone(),
                ));
            }
            let mut objectives = Vec::new();
            for objective in &stage.objectives {
                let at = format!("{at} / objective {}", objective.id);
                if let Some(count) = &objective.count {
                    counts.push((at.clone(), count.clone()));
                }
                objectives.push((
                    objective.clone(),
                    condition(&objective.done_when, &at, "done_when"),
                    condition(&objective.fail_when, &at, "fail_when"),
                    at,
                ));
            }
            stage_conditions.push((stage.clone(), at, next, objectives));
        }
        let effects = |source: &str, at: &str, field: &str, problems: &mut Vec<Problem>| {
            Effects::compile(source, ctx).unwrap_or_else(|e| {
                problems.push(Problem::error(at, format!("{field}: {e}")));
                Effects::default()
            })
        };
        let on_start = effects(&asset.on_start, "", "on_start", &mut problems);
        let on_complete = effects(&asset.on_complete, "", "on_complete", &mut problems);
        let on_fail = effects(&asset.on_fail, "", "on_fail", &mut problems);
        for (def, at, next, objectives) in stage_conditions {
            let on_enter = effects(&def.on_enter, &at, "on_enter", &mut problems);
            let objectives = objectives
                .into_iter()
                .map(|(def, done_when, fail_when, at)| Objective {
                    on_done: effects(&def.on_done, &at, "on_done", &mut problems),
                    on_fail: effects(&def.on_fail, &at, "on_fail", &mut problems),
                    def,
                    done_when,
                    fail_when,
                })
                .collect();
            stages.push(Stage {
                def,
                on_enter,
                next,
                objectives,
            });
        }
        for (at, count) in counts {
            match ctx.schema.get(&count.fact) {
                Some(def) if def.ty == FactType::Int => {}
                Some(def) => problems.push(Problem::error(
                    at,
                    format!("count.fact {} is a {:?} fact, not a number", def.id, def.ty),
                )),
                None => problems.push(Problem::error(
                    at,
                    format!("count.fact {} is not a declared fact", count.fact),
                )),
            }
        }
        if !has_errors(&problems) {
            for stage in asset.unreachable() {
                problems.push(Problem::warning(
                    format!("stage {stage}"),
                    "no path from the first stage reaches this stage",
                ));
            }
        }
        problems.sort();
        problems.dedup();
        if has_errors(&problems) {
            return Err(problems);
        }
        Ok(Self {
            asset,
            warnings: problems,
            start_when,
            fail_when,
            on_start,
            on_complete,
            on_fail,
            stages,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.asset.id
    }

    #[must_use]
    pub fn stage(&self, id: &str) -> Option<&Stage> {
        self.stages.iter().find(|s| s.def.id == id)
    }

    #[must_use]
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// An objective and the stage it is in.
    #[must_use]
    pub fn objective(&self, id: &str) -> Option<(&Stage, &Objective)> {
        self.stages.iter().find_map(|stage| {
            stage
                .objectives
                .iter()
                .find(|o| o.def.id == id)
                .map(|o| (stage, o))
        })
    }
}

/// Give every text in a quest its key: `quest.<id>.title`, `quest.<id>.<stage>.<line>`, `quest.<id>.obj.<objective>`.
pub fn assign_keys(asset: &mut QuestAsset) {
    let id = asset.id.clone();
    asset.title.assign_key(|| format!("quest.{id}.title"));
    asset.summary.assign_key(|| format!("quest.{id}.summary"));
    for stage in &mut asset.stages {
        let stage_id = stage.id.clone();
        stage
            .title
            .assign_key(|| format!("quest.{id}.{stage_id}.title"));
        for line in &mut stage.journal {
            let line_id = line.id.clone();
            line.text
                .assign_key(|| format!("quest.{id}.{stage_id}.{line_id}"));
        }
        for objective in &mut stage.objectives {
            let objective_id = objective.id.clone();
            objective
                .text
                .assign_key(|| format!("quest.{id}.obj.{objective_id}"));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::value::{FactDef, Schema};

    pub(crate) fn varga() -> QuestAsset {
        QuestAsset::from_json(
            r#"{
            "version": 1, "id": "varga_job", "title": "The Varga Strongroom", "category": "contract",
            "giver": "foreman", "summary": "Get into the Varga warehouse's strongroom.",
            "stages": [
              {"id": "offer", "journal": [{"id": "j1", "text": "A Varga foreman wants a lock opened.", "tier": "witnessed"}],
               "objectives": [{"id": "hear", "text": "Hear the foreman out", "done_when": "met.foreman"}],
               "next": [{"to": "inside"}]},
              {"id": "inside",
               "objectives": [
                 {"id": "guards", "text": "Deal with the guards", "count": {"fact": "varga.guards_down", "target": 3},
                  "marker": {"anchor": "varga_quay", "radius": 4.0}},
                 {"id": "unseen", "text": "Stay unseen", "optional": true, "fail_when": "varga.alarm"},
                 {"id": "door", "text": "Reach the strongroom", "marker": {"anchor": "strongroom_door", "radius": 2.0, "arrive": true},
                  "timer_ms": 60000}
               ],
               "next": [{"when": "varga.alarm and done('varga_job', 'door')", "to": "loud"}, {"to": "quiet"}]},
              {"id": "quiet", "journal": [{"id": "j1", "text": "Nobody saw a thing.", "tier": "measured"}], "end": "complete"},
              {"id": "loud", "journal": [{"id": "j1", "text": "The whole quay heard.", "tier": "retold"}], "end": "complete"}
            ],
            "on_complete": "rep.varga += 5"
        }"#,
        )
        .unwrap()
    }

    pub(crate) fn schema() -> Schema {
        Schema::new([
            FactDef::new("met.foreman", FactType::Bool),
            FactDef::new("varga.guards_down", FactType::Int),
            FactDef::new("varga.alarm", FactType::Bool),
            FactDef::new("rep.*", FactType::Int),
        ])
        .unwrap()
    }

    #[test]
    fn the_sample_compiles_and_keys_its_text() {
        let schema = schema();
        let quest = Quest::compile(varga(), Context::new(&schema, &[])).unwrap();
        assert_eq!(quest.asset.title.key, "quest.varga_job.title");
        let (stage, objective) = quest.objective("guards").unwrap();
        assert_eq!(stage.def.id, "inside");
        assert_eq!(objective.def.text.key, "quest.varga_job.obj.guards");
        assert_eq!(
            quest.stage("quiet").unwrap().def.journal[0].text.key,
            "quest.varga_job.quiet.j1"
        );
        assert!(quest.warnings.is_empty(), "{:?}", quest.warnings);
        let json = varga().to_json().unwrap();
        assert_eq!(QuestAsset::from_json(&json).unwrap(), varga());
    }

    #[test]
    fn quest_mistakes_are_located() {
        let mut asset = varga();
        asset.stages[1].next.push(StageLink {
            when: String::new(),
            to: "nowhere".into(),
        });
        asset.stages[1].objectives[0].count = Some(CountDef {
            fact: "varga.alarm".into(),
            target: 3,
        });
        asset.stages[2].objectives.push(ObjectiveDef {
            id: "hear".into(),
            text: "dup".into(),
            ..ObjectiveDef::default()
        });
        asset.stages[1].objectives[2]
            .marker
            .as_mut()
            .unwrap()
            .radius = 0.0;
        let schema = schema();
        let problems = Quest::compile(asset, Context::new(&schema, &[])).unwrap_err();
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert!(text.iter().any(|t| t.contains("nowhere")), "{text:#?}");
        assert!(text.iter().any(|t| t.contains("not a number")));
        assert!(text.iter().any(|t| t.contains("unique within a quest")));
        assert!(text.iter().any(|t| t.contains("ending stage")));
        assert!(
            text.iter()
                .any(|t| t.contains("arrival marker needs a radius"))
        );
    }

    #[test]
    fn unreachable_stages_are_a_warning() {
        let mut asset = varga();
        asset.stages.push(StageDef {
            id: "orphan".into(),
            end: Some(StageEnd::Fail),
            ..StageDef::default()
        });
        let schema = schema();
        let quest = Quest::compile(asset, Context::new(&schema, &[])).unwrap();
        assert_eq!(quest.warnings.len(), 1);
        assert!(quest.warnings[0].warning);
    }
}
