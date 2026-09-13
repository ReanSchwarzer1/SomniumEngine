//! Game-owned authoring declarations shared by every engine adapter.

use serde_json::Value;
use somnium_ecs::{Entity, World, reflect::TypeRegistry};
use std::sync::{OnceLock, RwLock};

/// A game document with the same validator for file edits and automation.
#[derive(Clone)]
pub struct GameDocument {
    pub id: &'static str,
    pub label: &'static str,
    pub extension: &'static str,
    pub schema: Value,
    pub validate: fn(&Value) -> Result<(), String>,
}

/// A reusable, undoable scene construction operation. Runs on a staging world.
#[derive(Clone)]
pub struct GamePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub schema: Value,
    pub build: fn(&mut World, &Value) -> Result<Vec<Entity>, String>,
}

/// Per-process game declarations, installed before the engine creates schemas.
/// A running engine never replaces this catalogue with another game's types.
#[derive(Clone, Default)]
pub struct GameRegistration {
    pub label: String,
    pub components: Vec<fn(&mut TypeRegistry)>,
    pub documents: Vec<GameDocument>,
    pub presets: Vec<GamePreset>,
    /// Restore game-owned derived state after an authored world change.
    pub rebuild: Vec<fn(&mut World)>,
}

fn declarations() -> &'static RwLock<GameRegistration> {
    static GAME: OnceLock<RwLock<GameRegistration>> = OnceLock::new();
    GAME.get_or_init(|| RwLock::new(GameRegistration::default()))
}

/// Register once at engine startup. Fail explicitly on ambiguous IDs.
pub fn install(game: GameRegistration) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    for id in game
        .documents
        .iter()
        .map(|d| d.id)
        .chain(game.presets.iter().map(|p| p.id))
    {
        if id.is_empty() || !ids.insert(id) {
            return Err(format!("duplicate or empty game authoring id: {id}"));
        }
    }
    let mut current = declarations()
        .write()
        .map_err(|_| "game registration lock poisoned")?;
    if !current.components.is_empty()
        || !current.presets.is_empty()
        || !current.documents.is_empty()
    {
        return Err("game authoring is already registered in this process".into());
    }
    *current = game;
    Ok(())
}

/// Snapshot metadata without retaining a lock across a game callback.
pub fn game_registration() -> GameRegistration {
    let mut game = declarations()
        .read()
        .expect("game registration lock poisoned")
        .clone();
    game.documents.extend(engine_documents());
    game
}

/// Built-in assets share validators with the native graph, timeline and UI owners.
pub fn engine_documents() -> Vec<GameDocument> {
    fn ui(v: &Value) -> Result<(), String> {
        somnium_ui::somui::UiDocument::from_json(&v.to_string())
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }
    fn scatter(v: &Value) -> Result<(), String> {
        somnium_ui::graph::serial::from_json(
            &v.to_string(),
            &somnium_ui::graph::scatter::catalogue(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    fn behavior(v: &Value) -> Result<(), String> {
        somnium_ui::graph::serial::from_json(
            &v.to_string(),
            &somnium_ui::graph::behavior::catalogue(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    fn material(v: &Value) -> Result<(), String> {
        somnium_ui::graph::serial::from_json(
            &v.to_string(),
            &somnium_ui::graph::catalogues::material(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    fn timeline(v: &Value) -> Result<(), String> {
        somnium_ui::timeline::serial::from_json(
            &v.to_string(),
            &somnium_ui::timeline::catalogues::animation(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    fn luau(v: &Value) -> Result<(), String> {
        let text = v["text"].as_str().ok_or("Luau document needs text")?;
        somnium_script_luau::validate_source(text)
    }

    let definitions: [(&str, &str, &str, fn(&Value) -> Result<(), String>); 6] = [
        ("somnium.luau_source", "Luau script", "luau", luau),
        ("somnium.ui_document", "UI layout", "somui", ui),
        (
            "somnium.scatter_graph",
            "Scatter graph",
            "somgraph",
            scatter,
        ),
        (
            "somnium.behavior_graph",
            "Behavior graph",
            "somgraph",
            behavior,
        ),
        (
            "somnium.material_graph",
            "Material graph",
            "somgraph",
            material,
        ),
        (
            "somnium.timeline",
            "Animation timeline",
            "somtimeline",
            timeline,
        ),
    ];
    definitions.into_iter().map(|(id,label,extension,validate)|GameDocument{id,label,extension,schema:serde_json::json!({"type":"object","description":"Validated by the native asset document schema"}),validate}).collect()
}

/// Called by the one component-registry factory used throughout the engine.
pub(crate) fn extend_components(registry: &mut TypeRegistry) {
    for register in game_registration().components {
        register(registry);
    }
}

pub(crate) fn rebuild(world: &mut World) {
    for callback in game_registration().rebuild {
        callback(world);
    }
}
