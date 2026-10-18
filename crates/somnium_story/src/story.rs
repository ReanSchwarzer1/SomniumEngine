//! The story runtime: one playthrough's facts, quests, dialogue memory, journal and barks, and its save.
//!
//! A game builds one [`Story`] over a shared [`StoryDatabase`], drives it with what happens (facts set by game
//! systems, effects from dialogue, arrival at markers, game time) and drains [`StoryEvent`]s each frame for its HUD,
//! journal, audio and its own commands. Everything is deterministic: the same content, save and inputs produce the
//! same events in the same order, which is what lets a quest chain be tested and a bug report be replayed.
//!
//! # Settling
//!
//! After every change the story **settles**: queued commands run (in the order effects wrote them), then the quest
//! log is asked for its next step (a quest starting, an objective completing, a stage changing), which is applied
//! with its effects, and so on until nothing is pending. A content loop that never settles (two stages that keep
//! leading to each other) is stopped after a fixed budget and reported, never hung on.

use crate::barks::{Bark, Chatter};
use crate::database::StoryDatabase;
use crate::expr::{Condition, Effect, Effects, Env, ExprError, Func};
use crate::journal::{EntryRef, Journal};
use crate::quest::{Quest, StageEnd};
use crate::quest_log::{
    ObjectiveState, ObjectiveStatus, QuestLog, QuestState, QuestStatus, QuestStep,
};
use crate::rng::Rng;
use crate::state::WorldState;
use crate::text::LocalText;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use somnium_i18n::Catalog;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

/// The save format this build writes.
pub const STORY_SAVE_VERSION: u32 = 1;

/// Steps one settle may take before it is called a loop.
const SETTLE_BUDGET: usize = 1024;

/// Something the game should hear about.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StoryEvent {
    FactChanged {
        fact: String,
        old: Value,
        new: Value,
    },
    /// A command the game declared in its manifest, from an effect.
    Command {
        name: String,
        args: Vec<Value>,
    },
    QuestStarted {
        quest: String,
    },
    QuestStage {
        quest: String,
        stage: String,
    },
    QuestCompleted {
        quest: String,
    },
    QuestFailed {
        quest: String,
    },
    QuestReset {
        quest: String,
    },
    ObjectiveDone {
        quest: String,
        objective: String,
    },
    ObjectiveFailed {
        quest: String,
        objective: String,
    },
    JournalAdded {
        sequence: u64,
    },
    JournalObscured {
        note: String,
        obscured: bool,
    },
    /// Content asked for something the runtime refused (an effect wrote a value an enum lacks, a quest loop).
    Problem {
        message: String,
    },
    /// The Voice announced something ([`Story::voice_view`] has it resolved): a notice, a chime, its log.
    Voice {
        sequence: u64,
        line: String,
    },
}

/// What a bark or chatter line has played recently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarkMemory {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last_ms: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub used: BTreeSet<String>,
}

/// A picked bark, resolved for the reader.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BarkPick {
    pub set: String,
    pub id: String,
    /// Who says it: the bark's speaker, or the first speaker the game offered when it names none.
    pub speaker: String,
    pub text: String,
    pub language: String,
    pub voice: String,
    pub tags: Vec<String>,
}

/// One line of picked chatter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatterLinePick {
    /// Which of the exchange's roles says it (an index into [`ChatterPick::roles`]).
    pub role: usize,
    pub text: String,
    pub language: String,
    pub voice: String,
}

/// A picked exchange; the game casts its roles onto the people standing there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatterPick {
    pub set: String,
    pub id: String,
    pub roles: Vec<String>,
    pub lines: Vec<ChatterLinePick>,
}

/// An objective's marker for the world canvas.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuestMarker {
    pub quest: String,
    pub objective: String,
    pub anchor: String,
    pub position: Option<[f32; 3]>,
    /// The region `position` is local to; empty for world metres. [`crate::quest::MarkerDef::world_position`].
    pub region: String,
    pub radius: f32,
    pub tracked: bool,
    pub priority: i32,
    pub text: String,
}

/// An objective as the HUD and journal show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectiveView {
    pub id: String,
    pub text: String,
    pub status: ObjectiveStatus,
    pub optional: bool,
    /// `(now, target)` for a counter.
    pub progress: Option<(i64, i64)>,
    pub remaining_ms: Option<u64>,
}

/// A quest journal line, resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LineView {
    pub sequence: u64,
    pub stage: String,
    pub text: String,
    pub tier: String,
    pub tier_rank: Option<usize>,
    pub at_ms: u64,
    pub read: bool,
}

/// A quest's page in the journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QuestPage {
    pub quest: String,
    pub title: String,
    pub summary: String,
    pub category: String,
    pub giver: String,
    pub status: QuestStatus,
    pub tracked: bool,
    pub objectives: Vec<ObjectiveView>,
    pub lines: Vec<LineView>,
}

/// A note, resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NoteView {
    pub sequence: u64,
    pub note: String,
    pub category: String,
    pub subject: String,
    pub title: String,
    pub text: String,
    pub tier: String,
    pub tier_rank: Option<usize>,
    pub obscured: bool,
    pub read: bool,
    pub at_ms: u64,
}

/// One of the Voice's announcements, resolved with the values it had when it was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VoiceView {
    pub sequence: u64,
    pub line: String,
    pub category: String,
    pub heading: String,
    pub text: String,
    pub voice: String,
    pub at_ms: u64,
    pub read: bool,
}

/// Someone on the journal's People page: a cast member the player has heard speak or has notes about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub portrait: String,
    pub expression: String,
    pub color: Option<[u8; 4]>,
    /// Lines of theirs the player has heard.
    pub lines_heard: u32,
    /// The notes about them (`subject`), as sequences into [`JournalView::notes`].
    pub notes: Vec<u64>,
}

/// The whole journal, resolved for drawing: active quests first (the tracked one at the top), then by priority and
/// recency; notes in arrival order; people in cast order; the Voice's log oldest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JournalView {
    pub quests: Vec<QuestPage>,
    pub notes: Vec<NoteView>,
    pub unread: usize,
    pub people: Vec<PersonView>,
    pub voice: Vec<VoiceView>,
}

/// What a save holds of the story. Goes in `SaveGame::state` under [`StorySave::KEY`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorySave {
    pub version: u32,
    /// The manifest id; a save from another game is refused.
    pub story: String,
    pub clock_ms: u64,
    pub rng: Rng,
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    #[serde(default)]
    pub quests: QuestLog,
    #[serde(default)]
    pub visits: BTreeMap<String, u32>,
    #[serde(default)]
    pub journal: Journal,
    #[serde(default)]
    pub barks: BarkMemory,
}

impl StorySave {
    /// The key in `SaveGame::state`.
    pub const KEY: &'static str = "story";

    /// Put this save into a game's save state (an object; created when null).
    pub fn write_into(&self, state: &mut serde_json::Value) -> Result<(), String> {
        if state.is_null() {
            *state = serde_json::Value::Object(serde_json::Map::new());
        }
        let object = state
            .as_object_mut()
            .ok_or("the game's save state is not an object")?;
        object.insert(
            Self::KEY.into(),
            serde_json::to_value(self).map_err(|e| e.to_string())?,
        );
        Ok(())
    }

    /// Read the story's part of a game's save state. `None` when the save has none (a save from before the story).
    pub fn read_from(state: &serde_json::Value) -> Result<Option<Self>, String> {
        match state.get(Self::KEY) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => serde_json::from_value(value.clone())
                .map(Some)
                .map_err(|e| format!("story save: {e}")),
        }
    }
}

/// One playthrough.
#[derive(Clone, Debug)]
pub struct Story {
    db: Arc<StoryDatabase>,
    state: WorldState,
    quests: QuestLog,
    visits: BTreeMap<String, u32>,
    journal: Journal,
    barks: BarkMemory,
    rng: Rng,
    clock_ms: u64,
    catalog: Catalog,
    events: Vec<StoryEvent>,
    queue: VecDeque<(String, Vec<Value>)>,
    revision: u64,
}

/// Evaluation inside the story, optionally scoped to a dialogue (for bare `visited('node')`).
pub(crate) struct Scope<'a> {
    pub story: &'a Story,
    pub dialogue: Option<&'a str>,
}

impl Env for Scope<'_> {
    fn fact(&self, id: &str) -> Value {
        self.story.state.read(&self.story.db.schema, id)
    }

    fn call(&self, func: Func, args: &[Value]) -> Value {
        if func == Func::Visited {
            let target = args.first().map_or("", Value::as_text);
            let key = match (target.contains('/'), self.dialogue) {
                (true, _) => target.to_owned(),
                (false, Some(dialogue)) => format!("{dialogue}/{target}"),
                (false, None) => return Value::Int(0),
            };
            return Value::Int(i64::from(self.story.visits.get(&key).copied().unwrap_or(0)));
        }
        self.story.quests.answer(func, args)
    }
}

/// The story's facts, except those an announcement captured: how a log entry reads as it did when it was made.
struct Captured<'a> {
    scope: Scope<'a>,
    values: &'a BTreeMap<String, Value>,
}

impl Env for Captured<'_> {
    fn fact(&self, id: &str) -> Value {
        self.values
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.scope.fact(id))
    }

    fn call(&self, func: Func, args: &[Value]) -> Value {
        self.scope.call(func, args)
    }
}

impl Story {
    /// A fresh playthrough. Quests whose start condition already holds start now.
    #[must_use]
    pub fn new(db: Arc<StoryDatabase>, seed: u64) -> Self {
        let mut story = Self {
            catalog: Catalog::new(db.manifest.source_locale.clone()),
            db,
            state: WorldState::new(),
            quests: QuestLog::default(),
            visits: BTreeMap::new(),
            journal: Journal::default(),
            barks: BarkMemory::default(),
            rng: Rng::new(seed),
            clock_ms: 0,
            events: Vec::new(),
            queue: VecDeque::new(),
            revision: 0,
        };
        story.settle();
        story
    }

    #[must_use]
    pub fn database(&self) -> &Arc<StoryDatabase> {
        &self.db
    }

    /// Increases on every change to facts, quests, the journal or dialogue memory: a cache key for UI.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Game time the story has been ticked through.
    #[must_use]
    pub fn clock_ms(&self) -> u64 {
        self.clock_ms
    }

    // --- facts ---------------------------------------------------------------------------------------------

    /// A fact's value, or `None` when undeclared.
    #[must_use]
    pub fn fact(&self, id: &str) -> Option<Value> {
        self.state.get(&self.db.schema, id)
    }

    #[must_use]
    pub fn world(&self) -> &WorldState {
        &self.state
    }

    /// Set a fact from game code (the player's skill rank rose, Lidfall began), then settle.
    pub fn set(&mut self, id: &str, value: impl Into<Value>) -> Result<(), String> {
        let change = self.state.set(&self.db.schema, id, value.into())?;
        if let Some(change) = change {
            self.revision += 1;
            self.events.push(StoryEvent::FactChanged {
                fact: change.fact,
                old: change.old,
                new: change.new,
            });
        }
        self.settle();
        Ok(())
    }

    /// Evaluate condition text against the story now (scripts, the MCP lint, a debug console).
    pub fn check(&self, condition: &str) -> Result<bool, ExprError> {
        let condition = Condition::compile(condition, self.db.context())?;
        Ok(self.eval(&condition, None))
    }

    /// Run effect text (scripts, debug), then settle.
    pub fn run(&mut self, effects: &str) -> Result<(), ExprError> {
        let effects = Effects::compile(effects, self.db.context())?;
        self.apply(&effects, None);
        Ok(())
    }

    /// Evaluate a compiled condition, optionally scoped to a dialogue.
    #[must_use]
    pub fn eval(&self, condition: &Condition, dialogue: Option<&str>) -> bool {
        condition.eval(&Scope {
            story: self,
            dialogue,
        })
    }

    /// Apply compiled effects, then settle.
    pub fn apply(&mut self, effects: &Effects, dialogue: Option<&str>) {
        self.write_effects(effects, dialogue);
        self.settle();
    }

    // --- dialogue memory -------------------------------------------------------------------------------------

    /// Times a dialogue node has been entered (options: chosen).
    #[must_use]
    pub fn visits(&self, dialogue: &str, node: &str) -> u32 {
        self.visits
            .get(&format!("{dialogue}/{node}"))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn visit(&mut self, dialogue: &str, node: &str) {
        *self.visits.entry(format!("{dialogue}/{node}")).or_insert(0) += 1;
        self.revision += 1;
    }

    pub(crate) fn roll(&mut self, sides: u32) -> i64 {
        self.rng.roll(sides)
    }

    // --- text ------------------------------------------------------------------------------------------------

    /// Load translations. The source text stays the fallback for anything the catalog lacks.
    pub fn set_catalog(&mut self, catalog: Catalog) {
        self.catalog = catalog;
        self.revision += 1;
    }

    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Resolve story text for the reader.
    #[must_use]
    pub fn text(&self, text: &LocalText, dialogue: Option<&str>) -> String {
        text.resolve(
            &self.catalog,
            &self.db.manifest.source_locale,
            &Scope {
                story: self,
                dialogue,
            },
        )
    }

    // --- events ----------------------------------------------------------------------------------------------

    /// Everything that happened since the last drain, in order.
    pub fn drain_events(&mut self) -> Vec<StoryEvent> {
        std::mem::take(&mut self.events)
    }

    #[must_use]
    pub fn pending_events(&self) -> &[StoryEvent] {
        &self.events
    }

    pub(crate) fn push_event(&mut self, event: StoryEvent) {
        self.events.push(event);
    }

    // --- time and place --------------------------------------------------------------------------------------

    /// Advance game time: objective timers run down. Paused frames should not tick.
    pub fn tick(&mut self, ms: u64) {
        if ms == 0 {
            return;
        }
        self.clock_ms = self.clock_ms.saturating_add(ms);
        let db = Arc::clone(&self.db);
        let expired = self.quests.tick(db.quests(), ms);
        for step in expired {
            self.step(step);
        }
        self.settle();
    }

    /// The player reached a named place: arrival objectives anchored there complete. Returns how many.
    pub fn reached(&mut self, anchor: &str) -> usize {
        self.arrive(|marker| !marker.anchor.is_empty() && marker.anchor == anchor)
    }

    /// The player is at `position` (world, game metres): arrival objectives whose marker is within its radius
    /// complete. `anchor` places named anchors and `region` a region's anchor (Rimeborn: `world::region_anchor`).
    /// Returns how many.
    pub fn arrived_at(
        &mut self,
        position: [f32; 3],
        anchor: impl Fn(&str) -> Option<[f32; 3]>,
        region: impl Fn(&str) -> Option<[f32; 3]>,
    ) -> usize {
        self.arrive(|marker| {
            marker.world_position(&anchor, &region).is_some_and(|t| {
                let d = [t[0] - position[0], t[1] - position[1], t[2] - position[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= marker.radius
            })
        })
    }

    fn arrive(&mut self, hit: impl Fn(&crate::quest::MarkerDef) -> bool) -> usize {
        let db = Arc::clone(&self.db);
        let mut steps = Vec::new();
        for quest in db.quests() {
            let Some(state) = self.quests.state(quest.id()) else {
                continue;
            };
            if state.status != QuestStatus::Active {
                continue;
            }
            let Some(stage) = quest.stage(&state.stage) else {
                continue;
            };
            for objective in &stage.objectives {
                if state.objective(&objective.def.id) == Some(ObjectiveStatus::Active)
                    && objective
                        .def
                        .marker
                        .as_ref()
                        .is_some_and(|m| m.arrive && hit(m))
                {
                    steps.push(QuestStep::ObjectiveDone {
                        quest: quest.id().into(),
                        objective: objective.def.id.clone(),
                    });
                }
            }
        }
        let count = steps.len();
        for step in steps {
            self.step(step);
        }
        self.settle();
        count
    }

    // --- quests ----------------------------------------------------------------------------------------------

    #[must_use]
    pub fn quests(&self) -> &QuestLog {
        &self.quests
    }

    #[must_use]
    pub fn quest_status(&self, quest: &str) -> QuestStatus {
        self.quests.status(quest)
    }

    /// Start a quest from game code, then settle. Refused for unknown quests.
    pub fn start_quest(&mut self, quest: &str) -> Result<(), String> {
        if self.db.quest(quest).is_none() {
            return Err(format!("no quest {quest}"));
        }
        self.queue
            .push_back(("start_quest".into(), vec![Value::Text(quest.into())]));
        self.settle();
        Ok(())
    }

    /// Choose the quest the HUD follows (`None`: the highest-priority active one is shown, none is tracked).
    pub fn track(&mut self, quest: Option<&str>) {
        let quest = quest.filter(|q| self.quests.status(q) == QuestStatus::Active);
        if self.quests.tracked.as_deref() != quest {
            self.quests.tracked = quest.map(str::to_owned);
            self.revision += 1;
        }
    }

    /// An active quest's objectives as the HUD shows them (hidden ones left out).
    #[must_use]
    pub fn objectives(&self, quest: &str) -> Vec<ObjectiveView> {
        let (Some(asset), Some(state)) = (self.db.quest(quest), self.quests.state(quest)) else {
            return Vec::new();
        };
        let mut views = Vec::new();
        for stage_id in dedup_path(&state.path) {
            let Some(stage) = asset.stage(stage_id) else {
                continue;
            };
            for objective in &stage.objectives {
                let Some(progress) = state.objectives.get(&objective.def.id) else {
                    continue;
                };
                if objective.def.hidden || progress.status == ObjectiveStatus::Dropped {
                    continue;
                }
                views.push(ObjectiveView {
                    id: objective.def.id.clone(),
                    text: self.text(&objective.def.text, None),
                    status: progress.status,
                    optional: objective.def.optional,
                    progress: objective.def.count.as_ref().map(|count| {
                        let now = self.state.read(&self.db.schema, &count.fact).as_int();
                        (now.min(count.target), count.target)
                    }),
                    remaining_ms: progress.remaining_ms,
                });
            }
        }
        views
    }

    /// Markers for every active objective that has one, the tracked quest's first.
    #[must_use]
    pub fn markers(&self) -> Vec<QuestMarker> {
        let mut markers = Vec::new();
        for quest in self.db.quests() {
            let Some(state) = self.quests.state(quest.id()) else {
                continue;
            };
            if state.status != QuestStatus::Active {
                continue;
            }
            let Some(stage) = quest.stage(&state.stage) else {
                continue;
            };
            for objective in &stage.objectives {
                let (Some(marker), Some(ObjectiveStatus::Active)) =
                    (&objective.def.marker, state.objective(&objective.def.id))
                else {
                    continue;
                };
                if objective.def.hidden {
                    continue;
                }
                markers.push(QuestMarker {
                    quest: quest.id().into(),
                    objective: objective.def.id.clone(),
                    anchor: marker.anchor.clone(),
                    position: marker.position,
                    region: marker.region.clone(),
                    radius: marker.radius,
                    tracked: self.quests.tracked.as_deref() == Some(quest.id()),
                    priority: quest.asset.priority,
                    text: self.text(&objective.def.text, None),
                });
            }
        }
        markers.sort_by(|a, b| {
            b.tracked
                .cmp(&a.tracked)
                .then(b.priority.cmp(&a.priority))
                .then_with(|| a.quest.cmp(&b.quest))
        });
        markers
    }

    // --- journal ---------------------------------------------------------------------------------------------

    #[must_use]
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Mark one entry read, or all of them.
    pub fn mark_read(&mut self, sequence: Option<u64>) {
        self.journal.mark_read(sequence);
        self.revision += 1;
    }

    /// The journal, resolved for drawing.
    #[must_use]
    pub fn journal_view(&self) -> JournalView {
        let manifest = &self.db.manifest;
        let mut pages: BTreeMap<&str, (QuestPage, u64)> = BTreeMap::new();
        let mut notes = Vec::new();
        for entry in self.journal.entries() {
            match &entry.entry {
                EntryRef::Quest { quest, stage, line } => {
                    let Some(asset) = self.db.quest(quest) else {
                        continue;
                    };
                    let Some(text) = asset
                        .stage(stage)
                        .and_then(|s| s.def.journal.iter().find(|l| l.id == *line))
                    else {
                        continue;
                    };
                    let page = pages
                        .entry(asset.id())
                        .or_insert_with(|| (self.page(asset), 0));
                    page.1 = page.1.max(entry.sequence);
                    page.0.lines.push(LineView {
                        sequence: entry.sequence,
                        stage: stage.clone(),
                        text: self.text(&text.text, None),
                        tier: text.tier.clone(),
                        tier_rank: manifest.tier_rank(&text.tier),
                        at_ms: entry.at_ms,
                        read: entry.read,
                    });
                }
                EntryRef::Voice { .. } => {}
                EntryRef::Note { note } => {
                    let Some(def) = self.db.note(note) else {
                        continue;
                    };
                    notes.push(NoteView {
                        sequence: entry.sequence,
                        note: note.clone(),
                        category: def.category.clone(),
                        subject: def.subject.clone(),
                        title: self.text(&def.title, None),
                        text: self.text(&def.text, None),
                        tier: def.tier.clone(),
                        tier_rank: manifest.tier_rank(&def.tier),
                        obscured: entry.obscured,
                        read: entry.read,
                        at_ms: entry.at_ms,
                    });
                }
            }
        }
        // A quest with no lines yet still has a page while it runs or once it has ended.
        for quest in self.db.quests() {
            if self.quests.status(quest.id()) != QuestStatus::Inactive {
                pages
                    .entry(quest.id())
                    .or_insert_with(|| (self.page(quest), 0));
            }
        }
        let mut quests: Vec<(QuestPage, u64, i32)> = pages
            .into_values()
            .map(|(page, last)| {
                let priority = self.db.quest(&page.quest).map_or(0, |q| q.asset.priority);
                (page, last, priority)
            })
            .collect();
        let rank = |status: QuestStatus| match status {
            QuestStatus::Active => 0,
            QuestStatus::Completed => 1,
            QuestStatus::Failed => 2,
            QuestStatus::Inactive => 3,
        };
        quests.sort_by(|(a, a_last, a_pri), (b, b_last, b_pri)| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then(b.tracked.cmp(&a.tracked))
                .then(b_pri.cmp(a_pri))
                .then(b_last.cmp(a_last))
                .then_with(|| a.quest.cmp(&b.quest))
        });
        let people = self.people(&notes);
        JournalView {
            quests: quests.into_iter().map(|(page, _, _)| page).collect(),
            notes,
            unread: self.journal.unread(),
            people,
            voice: self
                .journal
                .entries()
                .iter()
                .filter_map(|entry| self.voice_view(entry.sequence))
                .collect(),
        }
    }

    /// The cast the player has heard speak or has notes about, in cast order.
    fn people(&self, notes: &[NoteView]) -> Vec<PersonView> {
        let mut heard: BTreeMap<&str, u32> = BTreeMap::new();
        for dialogue in self.db.dialogues() {
            for node in &dialogue.asset.nodes {
                if let crate::dialogue::NodeDef::Line(line) = node {
                    if !line.speaker.is_empty() && self.visits(dialogue.id(), &line.id) > 0 {
                        *heard.entry(line.speaker.as_str()).or_default() += 1;
                    }
                }
            }
        }
        self.db
            .manifest
            .cast
            .iter()
            .filter(|speaker| speaker.journal)
            .filter_map(|speaker| {
                let about: Vec<u64> = notes
                    .iter()
                    .filter(|n| n.subject == speaker.id)
                    .map(|n| n.sequence)
                    .collect();
                let lines_heard = heard.get(speaker.id.as_str()).copied().unwrap_or(0);
                (lines_heard > 0 || !about.is_empty()).then(|| PersonView {
                    id: speaker.id.clone(),
                    name: self.text(&speaker.name, None),
                    portrait: speaker.portrait.clone(),
                    expression: if speaker.default_expression.is_empty() {
                        self.db
                            .manifest
                            .expressions
                            .first()
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        speaker.default_expression.clone()
                    },
                    color: crate::manifest::parse_color(&speaker.color),
                    lines_heard,
                    notes: about,
                })
            })
            .collect()
    }

    /// The Voice announces one of its lines: it goes in its log, saved, and a [`StoryEvent::Voice`] tells the game
    /// to show it. The facts its text names are kept as they are now. Returns the log entry's sequence.
    pub fn voice(&mut self, line: &str) -> Result<u64, String> {
        let def = self
            .db
            .voice_line(line)
            .ok_or_else(|| format!("voice('{line}'): no such voice line"))?;
        let mut values = BTreeMap::new();
        for text in [&def.heading, &def.text] {
            let names = text
                .placeholders()
                .into_iter()
                .chain([text.plural.clone(), text.gender.clone()]);
            for name in names {
                if self.db.schema.get(&name).is_some() {
                    let value = self.state.read(&self.db.schema, &name);
                    values.insert(name, value);
                }
            }
        }
        let sequence = self.journal.push(
            EntryRef::Voice {
                line: line.to_owned(),
                values,
            },
            self.clock_ms,
        );
        self.revision += 1;
        self.events.push(StoryEvent::Voice {
            sequence,
            line: line.to_owned(),
        });
        Ok(sequence)
    }

    /// One announcement of the Voice's log, resolved as it read when it was made.
    #[must_use]
    pub fn voice_view(&self, sequence: u64) -> Option<VoiceView> {
        let entry = self
            .journal
            .entries()
            .iter()
            .find(|e| e.sequence == sequence)?;
        let EntryRef::Voice { line, values } = &entry.entry else {
            return None;
        };
        let def = self.db.voice_line(line)?;
        let env = Captured {
            scope: Scope {
                story: self,
                dialogue: None,
            },
            values,
        };
        let source = &self.db.manifest.source_locale;
        Some(VoiceView {
            sequence,
            line: line.clone(),
            category: def.category.clone(),
            heading: def.heading.resolve(&self.catalog, source, &env),
            text: def.text.resolve(&self.catalog, source, &env),
            voice: def.voice.clone(),
            at_ms: entry.at_ms,
            read: entry.read,
        })
    }

    fn page(&self, quest: &Quest) -> QuestPage {
        let giver = self
            .db
            .manifest
            .speaker(&quest.asset.giver)
            .map(|s| self.text(&s.name, None))
            .unwrap_or_default();
        QuestPage {
            quest: quest.id().into(),
            title: self.text(&quest.asset.title, None),
            summary: self.text(&quest.asset.summary, None),
            category: quest.asset.category.clone(),
            giver,
            status: self.quests.status(quest.id()),
            tracked: self.quests.tracked.as_deref() == Some(quest.id()),
            objectives: self.objectives(quest.id()),
            lines: Vec::new(),
        }
    }

    // --- barks -----------------------------------------------------------------------------------------------

    /// Pick a bark for `trigger` among the speakers the game has at hand. Plays its effects.
    pub fn bark(&mut self, trigger: &str, speakers: &[&str]) -> Option<BarkPick> {
        let db = Arc::clone(&self.db);
        let mut candidates: Vec<(&str, &Bark, String)> = Vec::new();
        for set in db.bark_sets() {
            for bark in &set.barks {
                if bark.def.trigger != trigger {
                    continue;
                }
                let speaker = if bark.def.speaker.is_empty() {
                    match speakers.first() {
                        Some(first) => (*first).to_owned(),
                        None => continue,
                    }
                } else if speakers.contains(&bark.def.speaker.as_str()) {
                    bark.def.speaker.clone()
                } else {
                    continue;
                };
                let key = format!("{}/{}", set.id, bark.def.id);
                if self.ready(&key, bark.def.cooldown_ms, bark.def.once)
                    && self.eval(&bark.when, None)
                {
                    candidates.push((&set.id, bark, speaker));
                }
            }
        }
        let index = self.weighted(
            candidates
                .iter()
                .map(|(_, b, _)| (b.def.priority, b.def.weight)),
        )?;
        let (set, bark, speaker) = &candidates[index];
        let key = format!("{set}/{}", bark.def.id);
        self.played(&key, bark.def.once);
        let pick = BarkPick {
            set: (*set).to_owned(),
            id: bark.def.id.clone(),
            speaker: speaker.clone(),
            text: self.text(&bark.def.text, None),
            language: bark.def.language.clone(),
            voice: bark.def.voice.clone(),
            tags: bark.def.tags.clone(),
        };
        self.apply(&bark.effects, None);
        Some(pick)
    }

    /// Pick an exchange for `trigger` that `roles` people can perform. Plays its effects.
    pub fn chatter(&mut self, trigger: &str, roles: usize) -> Option<ChatterPick> {
        let db = Arc::clone(&self.db);
        let mut candidates: Vec<(&str, &Chatter)> = Vec::new();
        for set in db.bark_sets() {
            for chatter in &set.chatter {
                let key = format!("{}/{}", set.id, chatter.def.id);
                if chatter.def.trigger == trigger
                    && chatter.def.roles().len() <= roles
                    && self.ready(&key, chatter.def.cooldown_ms, chatter.def.once)
                    && self.eval(&chatter.when, None)
                {
                    candidates.push((&set.id, chatter));
                }
            }
        }
        let index = self.weighted(
            candidates
                .iter()
                .map(|(_, c)| (c.def.priority, c.def.weight)),
        )?;
        let (set, chatter) = candidates[index];
        self.played(&format!("{set}/{}", chatter.def.id), chatter.def.once);
        let role_names: Vec<String> = chatter.def.roles().into_iter().map(str::to_owned).collect();
        let lines = chatter
            .def
            .lines
            .iter()
            .map(|line| ChatterLinePick {
                role: role_names.iter().position(|r| *r == line.role).unwrap_or(0),
                text: self.text(&line.text, None),
                language: line.language.clone(),
                voice: line.voice.clone(),
            })
            .collect();
        let pick = ChatterPick {
            set: set.to_owned(),
            id: chatter.def.id.clone(),
            roles: role_names,
            lines,
        };
        self.apply(&chatter.effects, None);
        Some(pick)
    }

    fn ready(&self, key: &str, cooldown_ms: u64, once: bool) -> bool {
        !(once && self.barks.used.contains(key))
            && self
                .barks
                .last_ms
                .get(key)
                .is_none_or(|last| self.clock_ms.saturating_sub(*last) >= cooldown_ms)
    }

    fn played(&mut self, key: &str, once: bool) {
        self.barks.last_ms.insert(key.to_owned(), self.clock_ms);
        if once {
            self.barks.used.insert(key.to_owned());
        }
        self.revision += 1;
    }

    /// Among the highest priority, a weighted draw. `None` when there is nothing.
    fn weighted(&mut self, items: impl Iterator<Item = (i32, u32)>) -> Option<usize> {
        let items: Vec<(i32, u32)> = items.collect();
        let top = items.iter().map(|(p, _)| *p).max()?;
        let total: u64 = items
            .iter()
            .filter(|(p, _)| *p == top)
            .map(|(_, w)| u64::from(*w))
            .sum();
        let mut pick = self.rng.below(total);
        for (index, (priority, weight)) in items.iter().enumerate() {
            if *priority != top {
                continue;
            }
            if pick < u64::from(*weight) {
                return Some(index);
            }
            pick -= u64::from(*weight);
        }
        None
    }

    // --- saves -----------------------------------------------------------------------------------------------

    /// The story's part of a save. A conversation in progress is not part of it: save between conversations.
    #[must_use]
    pub fn save(&self) -> StorySave {
        StorySave {
            version: STORY_SAVE_VERSION,
            story: self.db.manifest.id.clone(),
            clock_ms: self.clock_ms,
            rng: self.rng,
            facts: self.state.snapshot(&self.db.schema),
            quests: self.quests.clone(),
            visits: self.visits.clone(),
            journal: self.journal.clone(),
            barks: self.barks.clone(),
        }
    }

    /// Restore a playthrough over (possibly newer) content. What the content no longer has is dropped and listed;
    /// a save from another game, or a newer format, is refused.
    pub fn load(db: Arc<StoryDatabase>, save: &StorySave) -> Result<(Self, Vec<String>), String> {
        if save.version > STORY_SAVE_VERSION {
            return Err(format!(
                "story save version {} is newer than this build reads ({STORY_SAVE_VERSION})",
                save.version
            ));
        }
        if save.story != db.manifest.id {
            return Err(format!(
                "this save belongs to {}, not {}",
                save.story, db.manifest.id
            ));
        }
        let (state, mut notes) = WorldState::restore(&db.schema, &save.facts);
        let mut quests = save.quests.clone();
        quests.quests.retain(|id, state| match db.quest(id) {
            None => {
                notes.push(format!("dropped quest {id}: no longer in the content"));
                false
            }
            Some(quest) => {
                if state.status == QuestStatus::Active && quest.stage(&state.stage).is_none() {
                    notes.push(format!(
                        "quest {id} is at stage {}, which the content no longer has; it will not advance",
                        state.stage
                    ));
                }
                state.objectives.retain(|objective, _| {
                    let keep = quest.objective(objective).is_some();
                    if !keep {
                        notes.push(format!("dropped objective {id}/{objective}"));
                    }
                    keep
                });
                true
            }
        });
        if quests
            .tracked
            .as_ref()
            .is_some_and(|q| quests.status(q) != QuestStatus::Active)
        {
            quests.tracked = None;
        }
        let mut journal = save.journal.clone();
        let dropped = journal.retain(|entry| match entry {
            EntryRef::Quest { quest, stage, line } => db
                .quest(quest)
                .and_then(|q| q.stage(stage))
                .is_some_and(|s| s.def.journal.iter().any(|l| l.id == *line)),
            EntryRef::Note { note } => db.note(note).is_some(),
            EntryRef::Voice { line, .. } => db.voice_line(line).is_some(),
        });
        if dropped > 0 {
            notes.push(format!(
                "dropped {dropped} journal entries the content no longer has"
            ));
        }
        let mut story = Self {
            catalog: Catalog::new(db.manifest.source_locale.clone()),
            db,
            state,
            quests,
            visits: save.visits.clone(),
            journal,
            barks: save.barks.clone(),
            rng: save.rng,
            clock_ms: save.clock_ms,
            events: Vec::new(),
            queue: VecDeque::new(),
            revision: 0,
        };
        story.settle();
        Ok((story, notes))
    }

    // --- the engine room -------------------------------------------------------------------------------------

    /// Write fact effects now and queue commands for the settle.
    fn write_effects(&mut self, effects: &Effects, dialogue: Option<&str>) {
        for effect in effects.list() {
            if let Effect::Command { name, args } = effect {
                let env = Scope {
                    story: self,
                    dialogue,
                };
                let values: Vec<Value> = args.iter().map(|arg| arg.eval(&env)).collect();
                self.queue.push_back((name.clone(), values));
                continue;
            }
            let write = crate::state::resolve(
                effect,
                &Scope {
                    story: self,
                    dialogue,
                },
            );
            let Some(write) = write else { continue };
            match self.state.write(&self.db.schema, &write) {
                Ok(Some(change)) => {
                    self.revision += 1;
                    self.events.push(StoryEvent::FactChanged {
                        fact: change.fact,
                        old: change.old,
                        new: change.new,
                    });
                }
                Ok(None) => {}
                Err(message) => self.events.push(StoryEvent::Problem { message }),
            }
        }
    }

    fn settle(&mut self) {
        for _ in 0..SETTLE_BUDGET {
            if let Some((name, args)) = self.queue.pop_front() {
                self.command(&name, &args);
                continue;
            }
            let db = Arc::clone(&self.db);
            let step = self.quests.pending(
                db.quests(),
                &Scope {
                    story: self,
                    dialogue: None,
                },
            );
            match step {
                Some(step) => self.step(step),
                None => return,
            }
        }
        self.queue.clear();
        self.events.push(StoryEvent::Problem {
            message: format!(
                "the story did not settle after {SETTLE_BUDGET} steps; two quest stages or effects keep triggering each other"
            ),
        });
    }

    fn command(&mut self, name: &str, args: &[Value]) {
        let text = |i: usize| args.get(i).map_or("", Value::as_text).to_owned();
        match name {
            "start_quest" => self.step(QuestStep::Start { quest: text(0) }),
            "set_stage" => {
                let quest = text(0);
                if self.quests.status(&quest) != QuestStatus::Active {
                    self.step(QuestStep::Start {
                        quest: quest.clone(),
                    });
                }
                self.step(QuestStep::Enter {
                    quest,
                    stage: text(1),
                });
            }
            "complete_objective" | "fail_objective" => {
                let (quest, objective) = (text(0), text(1));
                let active = self
                    .quests
                    .state(&quest)
                    .and_then(|s| s.objective(&objective))
                    == Some(ObjectiveStatus::Active);
                if active {
                    self.step(if name == "complete_objective" {
                        QuestStep::ObjectiveDone { quest, objective }
                    } else {
                        QuestStep::ObjectiveFailed { quest, objective }
                    });
                }
            }
            "complete_quest" => self.step(QuestStep::Complete { quest: text(0) }),
            "fail_quest" => self.step(QuestStep::Fail { quest: text(0) }),
            "reset_quest" => {
                let quest = text(0);
                if self.quests.quests.remove(&quest).is_some() {
                    if self.quests.tracked.as_deref() == Some(quest.as_str()) {
                        self.quests.tracked = None;
                    }
                    self.revision += 1;
                    self.events.push(StoryEvent::QuestReset { quest });
                }
            }
            "journal" => {
                let note = text(0);
                if self.db.note(&note).is_none() {
                    self.events.push(StoryEvent::Problem {
                        message: format!("journal('{note}'): no such note"),
                    });
                } else if let Some(sequence) =
                    self.journal.add(EntryRef::Note { note }, self.clock_ms)
                {
                    self.revision += 1;
                    self.events.push(StoryEvent::JournalAdded { sequence });
                }
            }
            "voice" => {
                if let Err(message) = self.voice(&text(0)) {
                    self.events.push(StoryEvent::Problem { message });
                }
            }
            "journal_obscure" | "journal_restore" => {
                let note = text(0);
                let obscured = name == "journal_obscure";
                if self.journal.set_obscured(&note, obscured) {
                    self.revision += 1;
                    self.events
                        .push(StoryEvent::JournalObscured { note, obscured });
                }
            }
            _ => self.events.push(StoryEvent::Command {
                name: name.to_owned(),
                args: args.to_vec(),
            }),
        }
    }

    fn step(&mut self, step: QuestStep) {
        let db = Arc::clone(&self.db);
        let quest_id = match &step {
            QuestStep::Start { quest }
            | QuestStep::Enter { quest, .. }
            | QuestStep::ObjectiveDone { quest, .. }
            | QuestStep::ObjectiveFailed { quest, .. }
            | QuestStep::Complete { quest }
            | QuestStep::Fail { quest } => quest.clone(),
        };
        let Some(quest) = db.quest(&quest_id) else {
            self.events.push(StoryEvent::Problem {
                message: format!("no quest {quest_id}"),
            });
            return;
        };
        let status = self.quests.status(&quest_id);
        match step {
            QuestStep::Start { .. } => {
                if status == QuestStatus::Active
                    || (status != QuestStatus::Inactive && !quest.asset.repeatable)
                {
                    return;
                }
                let completions = self.quests.state(&quest_id).map_or(0, |s| s.completions);
                self.quests.quests.insert(
                    quest_id.clone(),
                    QuestState {
                        status: QuestStatus::Active,
                        started_ms: self.clock_ms,
                        completions,
                        ..QuestState::default()
                    },
                );
                self.revision += 1;
                self.events.push(StoryEvent::QuestStarted {
                    quest: quest_id.clone(),
                });
                self.write_effects(&quest.on_start, None);
                if let Some(first) = quest.asset.first_stage() {
                    self.enter(quest, first);
                }
            }
            QuestStep::Enter { stage, .. } => {
                if status == QuestStatus::Active {
                    self.enter(quest, &stage);
                }
            }
            QuestStep::ObjectiveDone { ref objective, .. }
            | QuestStep::ObjectiveFailed { ref objective, .. } => {
                let done = matches!(step, QuestStep::ObjectiveDone { .. });
                let objective = objective.clone();
                let Some(state) = self.quests.quests.get_mut(&quest_id) else {
                    return;
                };
                let Some(progress) = state.objectives.get_mut(&objective) else {
                    return;
                };
                if status != QuestStatus::Active || progress.status != ObjectiveStatus::Active {
                    return;
                }
                progress.status = if done {
                    ObjectiveStatus::Done
                } else {
                    ObjectiveStatus::Failed
                };
                progress.remaining_ms = None;
                self.revision += 1;
                let Some((_, def)) = quest.objective(&objective) else {
                    return;
                };
                if done {
                    self.events.push(StoryEvent::ObjectiveDone {
                        quest: quest_id.clone(),
                        objective,
                    });
                    self.write_effects(&def.on_done, None);
                } else {
                    self.events.push(StoryEvent::ObjectiveFailed {
                        quest: quest_id.clone(),
                        objective,
                    });
                    self.write_effects(&def.on_fail, None);
                }
            }
            QuestStep::Complete { .. } | QuestStep::Fail { .. } => {
                if status == QuestStatus::Active {
                    self.finish(quest, matches!(step, QuestStep::Complete { .. }));
                }
            }
        }
    }

    fn enter(&mut self, quest: &Quest, stage_id: &str) {
        let Some(stage) = quest.stage(stage_id) else {
            self.events.push(StoryEvent::Problem {
                message: format!("{} has no stage {stage_id}", quest.id()),
            });
            return;
        };
        let Some(state) = self.quests.quests.get_mut(quest.id()) else {
            return;
        };
        for progress in state.objectives.values_mut() {
            if progress.status == ObjectiveStatus::Active {
                progress.status = ObjectiveStatus::Dropped;
                progress.remaining_ms = None;
            }
        }
        state.stage = stage_id.to_owned();
        state.path.push(stage_id.to_owned());
        for objective in &stage.objectives {
            state.objectives.insert(
                objective.def.id.clone(),
                ObjectiveState {
                    status: ObjectiveStatus::Active,
                    remaining_ms: objective.def.timer_ms,
                },
            );
        }
        self.revision += 1;
        self.events.push(StoryEvent::QuestStage {
            quest: quest.id().into(),
            stage: stage_id.to_owned(),
        });
        for line in &stage.def.journal {
            if let Some(sequence) = self.journal.add(
                EntryRef::Quest {
                    quest: quest.id().into(),
                    stage: stage_id.to_owned(),
                    line: line.id.clone(),
                },
                self.clock_ms,
            ) {
                self.events.push(StoryEvent::JournalAdded { sequence });
            }
        }
        self.write_effects(&stage.on_enter, None);
        match stage.def.end {
            Some(StageEnd::Complete) => self.finish(quest, true),
            Some(StageEnd::Fail) => self.finish(quest, false),
            None => {}
        }
    }

    fn finish(&mut self, quest: &Quest, completed: bool) {
        let Some(state) = self.quests.quests.get_mut(quest.id()) else {
            return;
        };
        if state.status != QuestStatus::Active {
            return;
        }
        state.status = if completed {
            QuestStatus::Completed
        } else {
            QuestStatus::Failed
        };
        state.ended_ms = Some(self.clock_ms);
        if completed {
            state.completions += 1;
        }
        for progress in state.objectives.values_mut() {
            if progress.status == ObjectiveStatus::Active {
                progress.status = ObjectiveStatus::Dropped;
                progress.remaining_ms = None;
            }
        }
        if self.quests.tracked.as_deref() == Some(quest.id()) {
            self.quests.tracked = None;
        }
        self.revision += 1;
        let id = quest.id().to_owned();
        if completed {
            self.events.push(StoryEvent::QuestCompleted { quest: id });
            self.write_effects(&quest.on_complete, None);
        } else {
            self.events.push(StoryEvent::QuestFailed { quest: id });
            self.write_effects(&quest.on_fail, None);
        }
    }
}

/// Stages in first-visit order.
fn dedup_path(path: &[String]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    path.iter()
        .filter(|s| seen.insert(s.as_str()))
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests;
