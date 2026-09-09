//! MORROWIND-AF: player deltas against authored content, versioned slots and
//! game-state transitions. A save never replaces the authored scene wholesale.
//!
//! References: O3DE Gems/SaveData/Code/Include/SaveData/SaveDataRequestBus.h
//! (slot ownership and completion contract), Gems/GameState (stack lifecycle).

use crate::scene_delta::{self, ValuePatch};
use crate::world_partition::CellCoord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Per-entity player changes. Fields unchanged by the player inherit new content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneDelta {
    /// Entities spawned by gameplay, keyed by durable id.
    pub spawned: BTreeMap<String, Value>,
    /// Authored entities removed by gameplay.
    pub removed: BTreeSet<String>,
    changes: BTreeMap<String, Vec<ValuePatch>>,
}

/// Result of rebasing saved changes onto the current authored scene.
#[derive(Clone, Debug)]
pub struct RebasedScene {
    /// Scene to load through the normal schema loader.
    pub scene: Value,
    /// Missing entity/component paths. Original save data is never discarded.
    pub unresolved: Vec<String>,
}

impl SceneDelta {
    fn validate(&self) -> Result<(), String> {
        let valid_id = |id: &str| {
            somnium_ecs::PersistentId::parse_hex(id)
                .is_some_and(|parsed| !parsed.is_none() && parsed.to_string() == id)
        };
        for (id, entry) in &self.spawned {
            if !valid_id(id)
                || entry.get("persistent_id").and_then(Value::as_str) != Some(id.as_str())
                || self.removed.contains(id)
                || self.changes.contains_key(id)
            {
                return Err("invalid or conflicting spawned identity".into());
            }
        }
        for id in &self.removed {
            if !valid_id(id) || self.changes.contains_key(id) {
                return Err("invalid or conflicting removed identity".into());
            }
        }
        for (id, patches) in &self.changes {
            if !valid_id(id) {
                return Err("invalid changed entity identity".into());
            }
            for patch in patches {
                if patch.path.is_empty()
                    || patch.path.len() > 64
                    || patch.path[0] == "persistent_id"
                {
                    return Err(
                        "save patch cannot replace an entity or change its durable identity".into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Record only changes made since the authored baseline was loaded.
    pub fn between(authored: &Value, played: &Value) -> Result<Self, String> {
        let a = scene_delta::entities(authored)?;
        let b = scene_delta::entities(played)?;
        let mut delta = Self::default();
        for (id, entry) in &b {
            if let Some(before) = a.get(id) {
                let patches = scene_delta::diff(before, entry);
                if !patches.is_empty() {
                    delta.changes.insert(id.clone(), patches);
                }
            } else {
                delta.spawned.insert(id.clone(), entry.clone());
            }
        }
        delta.removed = a
            .keys()
            .filter(|id| !b.contains_key(*id))
            .cloned()
            .collect();
        Ok(delta)
    }

    /// Apply player deltas after an author patch; preserve new defaults wherever
    /// gameplay did not write a value. Absent author content produces diagnostics.
    pub fn rebase(&self, authored: &Value) -> Result<RebasedScene, String> {
        self.validate()?;
        let mut entities = scene_delta::entities(authored)?;
        let mut unresolved = Vec::new();
        for id in &self.removed {
            entities.remove(id);
        }
        for (id, entry) in &self.spawned {
            if entities.contains_key(id) {
                return Err(format!("spawned entity collides with authored id {id}"));
            }
            if entry["persistent_id"].as_str() != Some(id) {
                return Err("spawned entity id mismatch".into());
            }
            entities.insert(id.clone(), entry.clone());
        }
        for (id, patches) in &self.changes {
            if let Some(entry) = entities.get_mut(id) {
                for orphan in scene_delta::apply(entry, patches) {
                    unresolved.push(format!("{id}: {}", orphan.path.join(".")));
                }
            } else {
                unresolved.push(format!("missing authored entity {id}"));
            }
        }
        Ok(RebasedScene {
            scene: scene_delta::document(entities),
            unresolved,
        })
    }
}

/// A streamed cell's independent delta. Saving it never deletes unloaded cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellSave {
    /// Cell ownership key.
    pub cell: CellCoord,
    /// Authored baseline content version for this cell.
    pub content_version: u64,
    /// Changes recorded while the cell was active.
    pub delta: SceneDelta,
}

/// User-visible slot metadata saved atomically with its body and screenshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveMetadata {
    /// User-facing slot title.
    pub title: String,
    /// Seconds since Unix epoch, supplied by the host for deterministic tests.
    pub saved_unix_secs: u64,
    /// Time spent playing, excluding paused frames.
    pub played_seconds: f64,
    /// Optional viewport PNG. Bounded to 4 MiB.
    pub screenshot_png: Vec<u8>,
}

/// Distinct save-game format, never passed to the scene loader directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveGame {
    /// Save format revision. Revision 1 is the initial migration source.
    pub version: u32,
    /// Author content revision; migration is explicit when this changes.
    pub content_version: u64,
    /// Metadata belonging to these exact saved values.
    pub metadata: SaveMetadata,
    /// Player changes in non-partitioned content.
    pub global: SceneDelta,
    /// Partial saves keyed by cell, retained across later partial writes.
    pub cells: Vec<CellSave>,
    /// Game-owned inventory, quest and script state, separate from scene fields.
    pub state: Value,
}

impl SaveGame {
    /// Start a save from a known content revision.
    #[must_use]
    pub fn new(content_version: u64) -> Self {
        Self {
            version: 2,
            content_version,
            metadata: SaveMetadata::default(),
            global: SceneDelta::default(),
            cells: Vec::new(),
            state: Value::Null,
        }
    }

    /// Update one loaded cell. Existing data for other cells is preserved.
    pub fn save_cell(
        &mut self,
        cell: CellCoord,
        authored: &Value,
        played: &Value,
    ) -> Result<(), String> {
        let saved = CellSave {
            cell,
            content_version: self.content_version,
            delta: SceneDelta::between(authored, played)?,
        };
        if let Some(old) = self.cells.iter_mut().find(|entry| entry.cell == cell) {
            *old = saved;
        } else {
            self.cells.push(saved);
            self.cells.sort_by_key(|entry| entry.cell);
        }
        Ok(())
    }

    /// Decode with a real format migration and reject corrupt/unsupported saves.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("save exceeds 64 MiB".into());
        }
        let mut value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        match value["version"].as_u64() {
            Some(1) => {
                value["version"] = Value::from(2);
                if value.get("cells").is_none() {
                    value["cells"] = Value::Array(Vec::new());
                }
            }
            Some(2) => {}
            _ => return Err("unsupported save format".into()),
        }
        let save: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        save.validate()?;
        Ok(save)
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != 2 {
            return Err("unsupported save format".into());
        }
        if !self.metadata.played_seconds.is_finite() || self.metadata.played_seconds < 0.0 {
            return Err("invalid play time".into());
        }
        let png = &self.metadata.screenshot_png;
        if png.len() > 4 * 1024 * 1024
            || (!png.is_empty() && !png.starts_with(b"\x89PNG\r\n\x1a\n"))
        {
            return Err("invalid or oversized screenshot PNG".into());
        }
        self.global.validate()?;
        let mut cells = BTreeSet::new();
        for cell in &self.cells {
            cell.delta.validate()?;
            if !cells.insert(cell.cell) {
                return Err("duplicate saved cell".into());
            }
        }
        Ok(())
    }

    /// Migrate game-owned state and deltas transactionally before changing the
    /// content stamp. Failure leaves the original save untouched.
    pub fn migrate_content(
        &mut self,
        target: u64,
        migrate: impl FnOnce(&mut SaveGame, u64, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        if target < self.content_version {
            return Err("content downgrade requires a newer game build".into());
        }
        if target == self.content_version {
            return Ok(());
        }
        let mut next = self.clone();
        migrate(&mut next, self.content_version, target)?;
        next.content_version = target;
        for cell in &mut next.cells {
            cell.content_version = target;
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

/// Slot storage rooted in a game-owned directory. Slot names cannot escape it.
#[derive(Clone, Debug)]
pub struct SaveSlots {
    directory: PathBuf,
}
impl SaveSlots {
    /// Choose the game's save directory (not its content directory).
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    fn path(&self, slot: &str) -> Result<PathBuf, String> {
        if slot.is_empty()
            || slot.len() > 64
            || !slot
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(
                "slot name must contain 1-64 letters, digits, hyphens or underscores".into(),
            );
        }
        Ok(self.directory.join(format!("{slot}.somsave")))
    }
    /// Publish metadata, screenshot and body together.
    pub fn write(&self, slot: &str, save: &SaveGame) -> Result<(), String> {
        save.validate()?;
        let bytes = serde_json::to_vec(save).map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("save exceeds 64 MiB".into());
        }
        atomic_write(&self.path(slot)?, &bytes)
    }
    /// Read a bounded slot without loading a scene or mutating game state.
    pub fn read(&self, slot: &str) -> Result<SaveGame, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(self.path(slot)?)
            .map_err(|e| e.to_string())?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        SaveGame::from_bytes(&bytes)
    }
    /// Sorted slot names and metadata for a load/save menu.
    pub fn list(&self) -> Result<Vec<(String, SaveMetadata)>, String> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        let mut slots: Vec<(String, SaveMetadata)> = Vec::new();
        for entry in std::fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("somsave") {
                continue;
            }
            if let Some(slot) = path.file_stem().and_then(|s| s.to_str()) {
                slots.push((slot.into(), self.read(slot)?.metadata));
            }
        }
        slots.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(slots)
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", somnium_ecs::PersistentId::mint()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Host modes. A pause or menu overlay suspends gameplay without deleting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameState {
    /// Main or overlay menu.
    Menu,
    /// Content is being loaded.
    Loading,
    /// Simulation and gameplay input are active.
    Playing,
    /// Gameplay is suspended.
    Paused,
}
/// Lifecycle notifications in the order a host must deliver them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateEvent {
    /// First entry into a state.
    Enter(GameState),
    /// An overlay covers this state.
    Suspend(GameState),
    /// An overlay was removed.
    Resume(GameState),
    /// State removed from the stack.
    Exit(GameState),
}
/// Explicit lifecycle stack; no hidden engine singleton or scene ownership.
#[derive(Clone, Debug)]
pub struct GameStateStack {
    states: Vec<GameState>,
}
impl Default for GameStateStack {
    fn default() -> Self {
        Self {
            states: vec![GameState::Menu],
        }
    }
}
impl GameStateStack {
    /// Current input/update owner.
    pub fn current(&self) -> GameState {
        *self.states.last().expect("state stack has a root")
    }
    /// Whether fixed gameplay updates should execute.
    pub fn simulating(&self) -> bool {
        self.current() == GameState::Playing
    }
    /// Push an overlay, returning suspend then enter notifications.
    pub fn push(&mut self, state: GameState) -> Vec<StateEvent> {
        let old = self.current();
        self.states.push(state);
        vec![StateEvent::Suspend(old), StateEvent::Enter(state)]
    }
    /// Pop an overlay; the root cannot be removed.
    pub fn pop(&mut self) -> Result<Vec<StateEvent>, String> {
        if self.states.len() == 1 {
            return Err("cannot pop the root game state".into());
        }
        let old = self.states.pop().unwrap();
        Ok(vec![
            StateEvent::Exit(old),
            StateEvent::Resume(self.current()),
        ])
    }
    /// Replace the current mode, e.g. Loading -> Playing.
    pub fn replace(&mut self, state: GameState) -> Vec<StateEvent> {
        let old = self.current();
        *self.states.last_mut().unwrap() = state;
        vec![StateEvent::Exit(old), StateEvent::Enter(state)]
    }
}

/// Designer slot profiles and Play-session operations.
pub mod editor;
