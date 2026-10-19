//! A game's save slots: the story, and a blob of the game's own state, in the engine's [`SaveGame`] format
//! (Rimeborn's Act VI slice, 19 Oct 2026; contract `docs/editor/saves.md`).
//!
//! The editor's play slots ([`super::editor::DesignerSaves`]) save the whole scene as a delta against the authored one,
//! which suits a level. A streamed open world is not saved that way: its cells are content, rebuilt as the player
//! comes near, and what the player changed lives in the story's facts (a door's `requires`, a taken thing's
//! `gone_when`) and in the game's own systems (the purse, the errands, the player's place, health and mount, the
//! hour). So a game save holds two things:
//! - the story, from `somnium_story`'s own save ([`StorySave`], facts, quests, the tracked one, the journal), written
//!   and restored here;
//! - the game's blob: whatever JSON its systems put in, handed back as it was.
//!
//! Slots are files in the user's data folder ([`user_save_dir`]), written atomically with their metadata. A save
//! from a newer content version is refused; an older one loads, the story dropping what the content no longer has.

use super::{SaveGame, SaveMetadata, SaveSlots};
use serde_json::Value;
use somnium_story::{Story, StoryDatabase, StorySave};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The key the game's blob is kept under in [`SaveGame::state`], beside the story's.
pub const GAME_KEY: &str = "game";

/// Where a game's saves go on this machine: the user's data folder (`%APPDATA%` on Windows, Application Support on
/// macOS, `$XDG_DATA_HOME` or `~/.local/share` elsewhere), never the project.
#[must_use]
pub fn user_save_dir(game: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| cfg!(target_os = "macos").then(|| home().map(|h| h.join("Library/Application Support"))).flatten())
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| home().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(game).join("saves")
}

/// A slot read back.
#[non_exhaustive]
pub struct Loaded {
    /// Title, when, and how long played.
    pub metadata: SaveMetadata,
    /// The content version it was saved with (never newer than the game's).
    pub content_version: u64,
    /// The story restored over the current content, with what the content no longer had; `None` when the save holds
    /// no story.
    pub story: Option<(Story, Vec<String>)>,
    /// The game's own state, as it saved it (`null` if it saved none).
    pub game: Value,
}

/// A game's save slots.
#[derive(Clone, Debug)]
pub struct GameSaves {
    slots: SaveSlots,
    directory: PathBuf,
    content_version: u64,
}

impl GameSaves {
    /// Slots in `directory`, for content at `content_version` (raise it when saved state changes meaning).
    pub fn new(directory: impl Into<PathBuf>, content_version: u64) -> Self {
        let directory = directory.into();
        Self { slots: SaveSlots::new(&directory), directory, content_version }
    }

    /// Slots in the user's data folder for `game` ([`user_save_dir`]).
    #[must_use]
    pub fn for_game(game: &str, content_version: u64) -> Self {
        Self::new(user_save_dir(game), content_version)
    }

    /// The folder the slots are in.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Save `story` (between conversations: a conversation in progress is not part of it) and `game` into `slot`
    /// (1-64 letters, digits, `-` and `_`), replacing it atomically. `metadata.saved_unix_secs` 0 is stamped now.
    pub fn save(&self, slot: &str, mut metadata: SaveMetadata, story: Option<&Story>, game: Value) -> Result<(), String> {
        if metadata.saved_unix_secs == 0 {
            metadata.saved_unix_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
        }
        let mut save = SaveGame::new(self.content_version);
        save.metadata = metadata;
        let mut state = serde_json::json!({ GAME_KEY: game });
        if let Some(story) = story {
            story.save().write_into(&mut state)?;
        }
        save.state = state;
        self.slots.write(slot, &save)
    }

    /// Read `slot` and restore its story over `db` (the game's current story content). Nothing in the running game is
    /// touched: the game puts the returned story and blob in place. Refused: a missing or corrupt slot, a save from a
    /// newer content version, a story from another game, or a save with a story when `db` is `None`.
    pub fn load(&self, slot: &str, db: Option<Arc<StoryDatabase>>) -> Result<Loaded, String> {
        let save = self.slots.read(slot)?;
        if save.content_version > self.content_version {
            return Err(format!(
                "'{slot}' was saved by newer content ({} > {}); it needs a newer build",
                save.content_version, self.content_version
            ));
        }
        let story = match (StorySave::read_from(&save.state)?, db) {
            (Some(saved), Some(db)) => Some(Story::load(db, &saved)?),
            (Some(_), None) => return Err(format!("'{slot}' holds a story, but no story content was given to load it into")),
            (None, _) => None,
        };
        let game = save.state.get(GAME_KEY).cloned().unwrap_or(Value::Null);
        Ok(Loaded { metadata: save.metadata, content_version: save.content_version, story, game })
    }

    /// Every readable slot and its metadata, newest first, for a load menu. A corrupt file is left out (and logged),
    /// never hiding the others.
    pub fn list(&self) -> Vec<(String, SaveMetadata)> {
        let Ok(files) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut slots: Vec<(String, SaveMetadata)> = files
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("somsave"))
            .filter_map(|path| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
            .filter_map(|slot| match self.slots.read(&slot) {
                Ok(save) => Some((slot, save.metadata)),
                Err(error) => {
                    tracing::warn!("save slot '{slot}' is unreadable: {error}");
                    None
                }
            })
            .collect();
        slots.sort_by(|a, b| b.1.saved_unix_secs.cmp(&a.1.saved_unix_secs).then_with(|| a.0.cmp(&b.0)));
        slots
    }

    /// The newest readable slot: what Continue loads.
    #[must_use]
    pub fn latest(&self) -> Option<String> {
        self.list().into_iter().next().map(|(slot, _)| slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_story::StorySources;

    fn story_db(id: &str) -> Arc<StoryDatabase> {
        let mut sources = StorySources::default();
        sources.add_file(
            "slice.somstory",
            &format!(
                r#"{{"version": 1, "id": "{id}", "facts": [
                    {{"id": "gold", "type": "int"}}, {{"id": "taken.purse_01", "type": "bool"}}]}}"#
            ),
        );
        let (db, _) = StoryDatabase::build(sources).unwrap_or_else(|issues| panic!("{issues:?}"));
        Arc::new(db)
    }

    fn folder(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("somnium_game_saves_{name}_{}_{nanos}", std::process::id()))
    }

    fn titled(title: &str, at: u64) -> SaveMetadata {
        SaveMetadata { title: title.into(), saved_unix_secs: at, played_seconds: 12.5, ..Default::default() }
    }

    #[test]
    fn a_slot_brings_back_the_story_and_the_games_own_state() {
        let directory = folder("round_trip");
        let saves = GameSaves::new(&directory, 3);
        let db = story_db("slice");
        let mut story = Story::new(db.clone(), 7);
        story.set("gold", 12).unwrap();
        story.set("taken.purse_01", true).unwrap();
        let game = serde_json::json!({
            "player": {"position": [12.0, 3.5, -40.0], "yaw": 1.2, "health": 0.8, "mount": "bay_mare"},
            "time": {"lid_hour": 17.25},
            "world": {"purse": 57, "errands": ["firewood"], "hue": 0, "rep": {"aldermere": 3}}
        });
        saves.save("quick", titled("Aldermere, before the Ice Test", 100), Some(&story), game.clone()).unwrap();
        // The game plays on and changes things; loading puts the saved ones back.
        story.set("gold", 0).unwrap();
        let loaded = saves.load("quick", Some(db)).unwrap();
        let (restored, notes) = loaded.story.unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(restored.fact("gold"), Some(12.into()));
        assert_eq!(restored.fact("taken.purse_01"), Some(true.into()));
        assert_eq!(loaded.game, game);
        assert_eq!((loaded.metadata.title.as_str(), loaded.metadata.played_seconds), ("Aldermere, before the Ice Test", 12.5));
        assert_eq!(loaded.content_version, 3);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn continue_is_the_newest_and_a_broken_file_hides_nothing() {
        let directory = folder("list");
        let saves = GameSaves::new(&directory, 1);
        assert_eq!(saves.latest(), None, "no folder yet");
        saves.save("knot", titled("A Knot", 300), None, Value::Null).unwrap();
        saves.save("quick", titled("Quick", 200), None, Value::Null).unwrap();
        saves.save("autosave", titled("Quest done", 250), None, Value::Null).unwrap();
        std::fs::write(directory.join("broken.somsave"), b"{ not a save").unwrap();
        let listed: Vec<String> = saves.list().into_iter().map(|(slot, _)| slot).collect();
        assert_eq!(listed, ["knot", "autosave", "quick"]);
        assert_eq!(saves.latest().as_deref(), Some("knot"));
        // A slot saved again replaces itself and moves to the front.
        saves.save("quick", titled("Quick again", 400), None, Value::Null).unwrap();
        assert_eq!(saves.latest().as_deref(), Some("quick"));
        assert_eq!(saves.list().len(), 3);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn what_cannot_be_loaded_is_refused_and_says_why() {
        let directory = folder("refused");
        let newer = GameSaves::new(&directory, 5);
        newer.save("future", titled("From a newer build", 1), None, Value::Null).unwrap();
        let older = GameSaves::new(&directory, 4);
        assert!(older.load("future", None).err().unwrap().contains("newer content"));
        assert!(newer.load("future", None).is_ok(), "the same version loads");
        assert!(older.load("missing", None).is_err());
        assert!(older.save("../escape", titled("x", 1), None, Value::Null).is_err(), "slot names stay in the folder");
        // A story from another game, and a story with nothing to load it into.
        let mut story = Story::new(story_db("another_game"), 1);
        story.set("gold", 3).unwrap();
        newer.save("theirs", titled("Theirs", 1), Some(&story), Value::Null).unwrap();
        assert!(newer.load("theirs", Some(story_db("slice"))).is_err());
        assert!(newer.load("theirs", None).err().unwrap().contains("holds a story"));
        // A save with no story loads with none, whatever content is given.
        let loaded = newer.load("future", Some(story_db("slice"))).unwrap();
        assert!(loaded.story.is_none() && loaded.game.is_null());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saves_go_to_the_users_data_folder_not_the_project() {
        let dir = user_save_dir("Rimeborn");
        assert!(dir.ends_with(Path::new("Rimeborn").join("saves")), "{}", dir.display());
        assert!(!dir.starts_with(env!("CARGO_MANIFEST_DIR")), "{}", dir.display());
    }
}
