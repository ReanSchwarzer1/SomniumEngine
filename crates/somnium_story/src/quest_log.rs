//! The quest runtime's state: where each quest is, which objectives are done, what is timed.
//!
//! This module only holds state and answers "what happens next": [`QuestLog::pending`] looks at the world and
//! returns at most one [`QuestStep`], and [`crate::story::Story`] applies it (running its effects, writing the
//! journal, raising events) and asks again until nothing is pending. One step at a time, in quest-id order, with
//! effects applied between steps, is what keeps a chain of quests that start each other deterministic.

use crate::expr::{Env, QUEST_STATES};
use crate::quest::{Expire, Quest};
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a quest is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStatus {
    #[default]
    Inactive,
    Active,
    Completed,
    Failed,
}

impl QuestStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            QuestStatus::Inactive => QUEST_STATES[0],
            QuestStatus::Active => QUEST_STATES[1],
            QuestStatus::Completed => QUEST_STATES[2],
            QuestStatus::Failed => QUEST_STATES[3],
        }
    }
}

/// Where an objective is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    Active,
    Done,
    Failed,
    /// Its stage was left before it finished.
    Dropped,
}

/// One objective's progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectiveState {
    pub status: ObjectiveStatus,
    /// Game time left on its timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_ms: Option<u64>,
}

/// One quest's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestState {
    pub status: QuestStatus,
    /// The current stage while active; the last one after it ends.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    /// Stages entered, in order (a branch shows which way it went).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub objectives: BTreeMap<String, ObjectiveState>,
    #[serde(default)]
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
    /// How many times it has been completed (repeatable quests).
    #[serde(default)]
    pub completions: u32,
}

impl QuestState {
    #[must_use]
    pub fn objective(&self, id: &str) -> Option<ObjectiveStatus> {
        self.objectives.get(id).map(|o| o.status)
    }
}

/// One thing that happens to a quest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestStep {
    Start { quest: String },
    Enter { quest: String, stage: String },
    ObjectiveDone { quest: String, objective: String },
    ObjectiveFailed { quest: String, objective: String },
    Complete { quest: String },
    Fail { quest: String },
}

/// Every quest's state, saved with the story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestLog {
    pub quests: BTreeMap<String, QuestState>,
    /// The quest the HUD follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked: Option<String>,
}

impl QuestLog {
    #[must_use]
    pub fn state(&self, quest: &str) -> Option<&QuestState> {
        self.quests.get(quest)
    }

    #[must_use]
    pub fn status(&self, quest: &str) -> QuestStatus {
        self.quests
            .get(quest)
            .map_or(QuestStatus::Inactive, |q| q.status)
    }

    /// The story function `quest('id')`, `stage('id')`, `done(...)` and `failed(...)` answers.
    #[must_use]
    pub fn answer(&self, func: crate::expr::Func, args: &[Value]) -> Value {
        use crate::expr::Func;
        let quest = args.first().map_or("", Value::as_text);
        let state = self.quests.get(quest);
        match func {
            Func::Quest => Value::Text(self.status(quest).as_str().into()),
            Func::Stage => Value::Text(
                state
                    .filter(|s| s.status == QuestStatus::Active)
                    .map(|s| s.stage.clone())
                    .unwrap_or_default(),
            ),
            Func::Done | Func::Failed => {
                let objective = args.get(1).map_or("", Value::as_text);
                let wanted = if func == Func::Done {
                    ObjectiveStatus::Done
                } else {
                    ObjectiveStatus::Failed
                };
                Value::Bool(state.and_then(|s| s.objective(objective)) == Some(wanted))
            }
            Func::Visited => Value::Int(0),
        }
    }

    /// The next thing that should happen, if anything. `quests` in id order.
    #[must_use]
    pub fn pending<'a>(
        &self,
        quests: impl Iterator<Item = &'a Quest>,
        env: &dyn Env,
    ) -> Option<QuestStep> {
        for quest in quests {
            let id = quest.id();
            let state = self.quests.get(id);
            let status = state.map_or(QuestStatus::Inactive, |s| s.status);
            if status == QuestStatus::Inactive {
                if quest.start_when.as_ref().is_some_and(|c| c.eval(env)) {
                    return Some(QuestStep::Start { quest: id.into() });
                }
                continue;
            }
            if status != QuestStatus::Active {
                continue;
            }
            let state = state.expect("an active quest has state");
            if !quest.fail_when.is_always() && quest.fail_when.eval(env) {
                return Some(QuestStep::Fail { quest: id.into() });
            }
            let Some(stage) = quest.stage(&state.stage) else {
                continue;
            };
            for objective in &stage.objectives {
                let oid = &objective.def.id;
                if state.objective(oid) != Some(ObjectiveStatus::Active) {
                    continue;
                }
                if !objective.fail_when.is_always() && objective.fail_when.eval(env) {
                    return Some(QuestStep::ObjectiveFailed {
                        quest: id.into(),
                        objective: oid.clone(),
                    });
                }
                let done = if objective.done_when.is_always() {
                    objective
                        .def
                        .count
                        .as_ref()
                        .is_some_and(|count| env.fact(&count.fact).as_int() >= count.target)
                } else {
                    objective.done_when.eval(env)
                };
                if done {
                    return Some(QuestStep::ObjectiveDone {
                        quest: id.into(),
                        objective: oid.clone(),
                    });
                }
            }
            let required_done = stage
                .objectives
                .iter()
                .filter(|o| !o.def.optional)
                .all(|o| state.objective(&o.def.id) == Some(ObjectiveStatus::Done));
            for (when, to) in &stage.next {
                let go = match when {
                    Some(condition) => condition.eval(env),
                    None => required_done,
                };
                if go {
                    return Some(QuestStep::Enter {
                        quest: id.into(),
                        stage: to.clone(),
                    });
                }
            }
            let required_failed = stage.objectives.iter().any(|o| {
                !o.def.optional && state.objective(&o.def.id) == Some(ObjectiveStatus::Failed)
            });
            if required_failed {
                return Some(QuestStep::Fail { quest: id.into() });
            }
        }
        None
    }

    /// Run timers down by `ms`; objectives whose time is up come back with what their expiry does.
    pub fn tick<'a>(&mut self, quests: impl Iterator<Item = &'a Quest>, ms: u64) -> Vec<QuestStep> {
        let mut expired = Vec::new();
        for quest in quests {
            let Some(state) = self.quests.get_mut(quest.id()) else {
                continue;
            };
            if state.status != QuestStatus::Active {
                continue;
            }
            for (oid, objective) in &mut state.objectives {
                if objective.status != ObjectiveStatus::Active {
                    continue;
                }
                let Some(remaining) = objective.remaining_ms.as_mut() else {
                    continue;
                };
                *remaining = remaining.saturating_sub(ms);
                if *remaining == 0 {
                    objective.remaining_ms = None;
                    let on_expire = quest
                        .objective(oid)
                        .map_or(Expire::Fail, |(_, o)| o.def.on_expire);
                    expired.push(match on_expire {
                        Expire::Fail => QuestStep::ObjectiveFailed {
                            quest: quest.id().into(),
                            objective: oid.clone(),
                        },
                        Expire::Complete => QuestStep::ObjectiveDone {
                            quest: quest.id().into(),
                            objective: oid.clone(),
                        },
                    });
                }
            }
        }
        expired
    }
}
