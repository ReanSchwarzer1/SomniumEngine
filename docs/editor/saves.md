# Saving and loading a game

A game's saves come from `somnium_core::save_game::game::GameSaves`. It writes the engine's `SaveGame` format (a
`.somsave` file): versioned, written atomically, and kept with its title, time and play time.

A save holds two things:
- **the story**: `somnium_story`'s own save, so every fact, every quest and its stage, the tracked quest, the
  journal, visits and bark memory;
- **the game's blob**: any JSON the game's systems put in, handed back exactly as it was.

The editor's **Save Game Slot** (play slots in the Details panel) is a different thing: it saves the whole scene as a
delta against the authored scene, which suits a level. A streamed open world isn't saved that way. Its cells are
content, rebuilt as the player comes near, and what the player changed lives in the story and in the game's own
systems.

## Where the state goes

Put durable world state in **story facts** wherever it fits. The story save carries facts for free, and conditions
read them. In Rimeborn these are all facts:
- the purse (`money.purse`);
- reputation (`rep.*`), trust (`trust.*`) and the Hue and Cry (`hue.*`);
- crimes (`crime.count`);
- errands: slot quests with their alias facts;
- taken things (`taken.<id>`, read by an Interactable's **Gone When**);
- unlocked doors (read by **Requires**).

A fact declared `saved: false` (for example `crime.wanted`) is left out on purpose.

What is not a fact goes in the game's blob: where the player stands and faces, their health, whether they ride, and
the hour.

## The calls

```rust
use somnium_core::save_game::{SaveMetadata, game::GameSaves};

// Once: the user's data folder (%APPDATA%\Rimeborn\saves on Windows), at the game's content version.
let saves = GameSaves::for_game("Rimeborn", 1);

// Save: between conversations (a conversation in progress is not part of the story's save).
let metadata = SaveMetadata { title: "Aldermere, before the Ice Test".into(), played_seconds, ..Default::default() };
saves.save("quick", metadata, Some(&story), serde_json::json!({ "player": ..., "lid_hour": 17.25 }))?;

// Load: nothing running is touched; the game puts the story and the blob in place.
let loaded = saves.load("quick", Some(story_db.clone()))?;
if let Some((story, dropped)) = loaded.story { /* replace the game's Story; log `dropped` */ }
let blob = loaded.game;

// A menu: every readable slot, newest first; Continue is the newest.
for (slot, meta) in saves.list() { /* meta.title, meta.saved_unix_secs, meta.played_seconds */ }
let continue_slot = saves.latest();
```

- **Slot names** are 1 to 64 letters, digits, `-` and `_`. Saving a slot again replaces it atomically, so a crash
  mid-save leaves the old file.
- **Versions.** A save from a newer content version is refused with the reason. An older one loads: the story
  drops what the content no longer has and lists it, and the game migrates its own blob if it changed.
- **Errors.** A save from another game's story is refused. So is a save that holds a story when no story content
  was given to load it into.
- **The list.** A corrupt file is left out of `list` (and logged), never hiding the other slots.
- **Thumbnails.** Put a PNG of up to 4 MiB in `SaveMetadata::screenshot_png` for a load menu.

## After a load, mid-play

- **Streamed cells** that are already loaded keep what happened in them since the save. To bring them back as the
  save has them (a taken purse returns), turn the scene's **Cell Streaming** off for a frame, then on: off unloads
  every cell, and they stream back in around the player's restored place.
- **Interactables** that arrive while their **Gone When** fact holds are removed as they appear.

## Rimeborn's slots

| Slot | Written | Read |
|---|---|---|
| `knot` | Autosave at every `tie_knot(...)` story command (GAMEPLAY 4.2: the Return goes back to the last Knot) | On a Return, and by Continue when it is the newest |
| `autosave` | Autosave when a quest completes | Load menu, Continue |
| `quick` | F5 | F9, the load menu, Continue |

A Return isn't a plain load. The game loads `knot`, then lays the Return's cost on top: a memory of Earth frosts
over, the Lidless track rises. The cost must survive later Returns, so the game writes it into the loaded story
before play resumes, and saves `knot` again.
