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
    declarations()
        .read()
        .expect("game registration lock poisoned")
        .clone()
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
