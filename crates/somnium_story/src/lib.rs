//! Story systems for the Somnium Engine: world state, quests, dialogue, barks and the journal.
//!
//! Built for Rimeborn (an isometric RPG with branching story, six romances and many languages), written so that any
//! story-driven game can use it. Everything here is data and rules: no renderer, no widgets, no ECS. The editor's
//! graph catalogues and the dialogue presenter widget live in `somnium_ui`; the editor and MCP wiring in
//! `somnium_core`.
//!
//! The pieces, in the order a game meets them:
//!
//! - [`value`] and [`state`]: facts declared once in a [`Schema`], stored in a deterministic [`WorldState`].
//! - [`expr`]: the one-line condition and effect language every asset uses, checked against the schema at load.
//! - [`manifest`]: the game's story manifest (facts, cast, truth tiers, commands, source language).
//! - [`dialogue`] and [`conversation`]: dialogue graph assets and the runtime that plays them.
//! - [`quest`] and [`quest_log`]: quest assets (stages, objectives, branches, failure) and their runtime.
//! - [`barks`]: one-liners and ambient chatter picked by trigger, condition, cooldown and weight.
//! - [`journal`]: what the player has learned and how (truth tiers), fed by quests and effects.
//! - [`story`]: the runtime that owns all of it for one playthrough, and its save.
//! - [`database`]: loading a game's story content from disk, linting it as a whole and extracting its strings.
//! - [`presenter`]: the presentation model behind a light-novel dialogue box (typewriter, auto, skip, history).

pub mod barks;
pub mod conversation;
pub mod database;
pub mod dialogue;
pub mod expr;
pub mod journal;
pub mod manifest;
pub mod presenter;
pub mod problem;
pub mod quest;
pub mod quest_log;
pub mod rng;
pub mod state;
pub mod story;
pub mod text;
pub mod value;

pub use conversation::{Beat, ChoiceBeat, Conversation, LineBeat};
pub use database::{Issue, Severity, StoryDatabase, StorySources};
pub use dialogue::DialogueAsset;
pub use expr::{CommandDef, Condition, Context, Effects, Env, ExprError, ParamType};
pub use manifest::{Side, Speaker, StoryManifest};
pub use presenter::{Presenter, PresenterInput, PresenterOutput, PresenterSettings};
pub use problem::Problem;
pub use quest::QuestAsset;
pub use quest_log::{ObjectiveStatus, QuestStatus};
pub use state::{FactChange, WorldState};
pub use story::{Story, StoryEvent, StorySave};
pub use text::LocalText;
pub use value::{FactDef, FactType, Schema, Value};
