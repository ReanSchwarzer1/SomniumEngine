//! The editor's and agents' side of `somnium_story`: where a project's story lives, checking one edited asset
//! against all of it, saving assets canonically, and the MCP `story` query.
//!
//! A project's story content is everything with a story extension under `<content>/story` (or the whole content
//! folder when it has no `story` folder): one `*.somstory` manifest, `*.somdialogue`, `*.somquest`, `*.sombarks` and
//! `*.somnotes` files. The graph editor checks the asset it has open by building the whole project with its edited
//! version swapped in, which is what finds a speaker missing from the cast or a quest that does not exist.

use serde_json::{Value, json};
use somnium_story::conversation::simulate;
use somnium_story::database::{EXTENSIONS, Issue, Severity, StorySources};
use somnium_story::{DialogueAsset, QuestAsset, Story, StoryDatabase, StoryEvent};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The graph catalogues whose documents are story assets.
#[must_use]
pub fn is_story_catalogue(catalogue: &str) -> bool {
    matches!(
        catalogue,
        somnium_ui::graph::dialogue::CATALOGUE | somnium_ui::graph::quest::CATALOGUE
    )
}

/// The graph catalogue a story file opens with.
#[must_use]
pub fn catalogue_for(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("somdialogue") => Some(somnium_ui::graph::dialogue::CATALOGUE),
        Some("somquest") => Some(somnium_ui::graph::quest::CATALOGUE),
        _ => None,
    }
}

/// The file extension a catalogue's assets use.
#[must_use]
pub fn extension_for(catalogue: &str) -> &'static str {
    if catalogue == somnium_ui::graph::quest::CATALOGUE {
        "somquest"
    } else {
        "somdialogue"
    }
}

/// Whether a path has one of the story extensions.
#[must_use]
pub fn is_story_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|(x, _)| *x == e))
}

/// Where a project's story content lives.
#[must_use]
pub fn story_root(content_root: &Path) -> PathBuf {
    let story = content_root.join("story");
    if story.is_dir() {
        story
    } else {
        content_root.to_path_buf()
    }
}

/// `dialogue <id>` or `quest <id>`: how issues name an asset.
///
/// # Errors
/// When `json` is not an asset of that catalogue.
pub fn asset_key(catalogue: &str, json: &str) -> Result<String, String> {
    Ok(if catalogue == somnium_ui::graph::quest::CATALOGUE {
        format!("quest {}", QuestAsset::from_json(json)?.id)
    } else {
        format!("dialogue {}", DialogueAsset::from_json(json)?.id)
    })
}

/// Pretty, stable JSON for an asset: what is written to disk.
///
/// # Errors
/// When `json` is not an asset of that catalogue.
pub fn canonical(catalogue: &str, json: &str) -> Result<String, String> {
    if catalogue == somnium_ui::graph::quest::CATALOGUE {
        QuestAsset::from_json(json)?.to_json()
    } else {
        DialogueAsset::from_json(json)?.to_json()
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The project's story sources with one asset replaced by `json` (its file, or its id, decides which).
///
/// # Errors
/// When `json` is not an asset of that catalogue.
pub fn sources_with(
    root: &Path,
    catalogue: &str,
    json: &str,
    source: Option<&Path>,
) -> Result<StorySources, String> {
    let mut sources = StorySources::read_dir(root);
    let file = source.map_or_else(
        || format!("(unsaved).{}", extension_for(catalogue)),
        |path| relative(root, path),
    );
    // The file on disk may be the broken version being fixed: the edit replaces it.
    sources.unreadable.retain(|issue| issue.file != file);
    if catalogue == somnium_ui::graph::quest::CATALOGUE {
        let asset = QuestAsset::from_json(json)?;
        sources
            .quests
            .retain(|(f, a)| *f != file && a.id != asset.id);
        sources.quests.push((file, asset));
    } else {
        let asset = DialogueAsset::from_json(json)?;
        sources
            .dialogues
            .retain(|(f, a)| *f != file && a.id != asset.id);
        sources.dialogues.push((file, asset));
    }
    Ok(sources)
}

/// What checking one asset found.
pub struct StoryCheck {
    /// `(where, message, warning)` for the asset itself.
    pub problems: Vec<(String, String, bool)>,
    /// Errors in the rest of the project's story (they do not stop this asset playing).
    pub elsewhere: Vec<Issue>,
    /// The project's content with the edit in, when the asset has no errors (what Play runs).
    pub database: Option<Arc<StoryDatabase>>,
    /// One line for the graph footer.
    pub status: String,
}

/// Check an edited asset against the project's story content.
///
/// # Errors
/// When `json` is not an asset of that catalogue; problems in it are the result, not an error.
pub fn check(
    root: &Path,
    catalogue: &str,
    json: &str,
    source: Option<&Path>,
) -> Result<StoryCheck, String> {
    let key = asset_key(catalogue, json)?;
    let sources = sources_with(root, catalogue, json, source)?;
    let (database, issues) = StoryDatabase::build_lenient(sources);
    let mut problems = Vec::new();
    let mut elsewhere = Vec::new();
    for issue in issues {
        if issue.asset == key {
            problems.push((issue.at, issue.message, issue.severity == Severity::Warning));
        } else if issue.asset == "manifest" || issue.file.is_empty() {
            problems.push((
                String::new(),
                format!("{} ({})", issue.message, root.display()),
                false,
            ));
        } else if issue.severity == Severity::Error {
            elsewhere.push(issue);
        }
    }
    let id = key.split_once(' ').map_or("", |(_, id)| id);
    let compiled = database.as_ref().is_some_and(|db| {
        if catalogue == somnium_ui::graph::quest::CATALOGUE {
            db.quest(id).is_some()
        } else {
            db.dialogue(id).is_some()
        }
    });
    let errors = problems.iter().filter(|p| !p.2).count();
    let warnings = problems.len() - errors;
    let status = match (errors, warnings, elsewhere.len()) {
        (0, 0, 0) => format!("{key}: no problems"),
        (0, 0, other) => format!("{key}: no problems · {other} error(s) in other story files"),
        _ => format!(
            "{key}: {errors} error(s), {warnings} warning(s){}",
            if elsewhere.is_empty() {
                String::new()
            } else {
                format!(" · {} error(s) in other story files", elsewhere.len())
            }
        ),
    };
    Ok(StoryCheck {
        problems,
        elsewhere,
        database: (compiled && errors == 0)
            .then(|| database.map(Arc::new))
            .flatten(),
        status,
    })
}

fn issue_json(issue: &Issue) -> Value {
    json!({"severity": issue.severity, "file": issue.file, "asset": issue.asset, "at": issue.at, "message": issue.message})
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .ok_or_else(|| format!("{key} required"))
}

/// The MCP `story` query: read, lint, check and play a project's story without changing anything.
///
/// `operation`: `overview` (default), `schema`, `lint`, `check` (`document_type` dialogue or quest, `content`, optional
/// `path`), `simulate` (`dialogue`, optional `entry`, `choices`, `setup` effects, `max_beats`), `strings`, `search`
/// (`text`).
///
/// # Errors
/// Unknown operations, missing arguments, a project without a usable manifest, a dialogue that does not start.
pub fn query(root: &Path, params: &Value) -> Result<Value, String> {
    let operation = params["operation"].as_str().unwrap_or("overview");
    if operation == "check" {
        let catalogue = match params["document_type"].as_str().unwrap_or("dialogue") {
            "dialogue" | "somnium.dialogue" => somnium_ui::graph::dialogue::CATALOGUE,
            "quest" | "somnium.quest" => somnium_ui::graph::quest::CATALOGUE,
            other => return Err(format!("check takes a dialogue or a quest, not {other}")),
        };
        let content = params
            .get("content")
            .ok_or("content required: the asset JSON")?;
        let json = if let Some(text) = content.as_str() {
            text.to_owned()
        } else {
            content.to_string()
        };
        let path = params["path"]
            .as_str()
            .map(|p| root.join(p.strip_prefix("story/").unwrap_or(p)));
        let result = check(root, catalogue, &json, path.as_deref())?;
        return Ok(json!({
            "ok": true,
            "status": result.status,
            "problems": result.problems.iter().map(|(at, message, warning)| json!({"at": at, "message": message, "warning": warning})).collect::<Vec<_>>(),
            "errors_elsewhere": result.elsewhere.iter().map(issue_json).collect::<Vec<_>>(),
        }));
    }
    let (database, issues) = StoryDatabase::build_lenient(StorySources::read_dir(root));
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    if operation == "lint" {
        let file = params["file"].as_str();
        let rows: Vec<Value> = issues
            .iter()
            .filter(|i| file.is_none_or(|f| i.file == f))
            .map(issue_json)
            .collect();
        return Ok(
            json!({"ok": true, "root": root, "errors": errors, "warnings": issues.len() - errors, "issues": rows}),
        );
    }
    let database = database.ok_or_else(|| {
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    match operation {
        "overview" => Ok(overview(root, &database, errors, issues.len() - errors)),
        "schema" => Ok(json!({
            "ok": true,
            "facts": database.schema.definitions().collect::<Vec<_>>(),
            "commands": database.manifest.commands,
            "builtin_commands": somnium_story::expr::BUILTIN_COMMANDS.iter().map(|(name, params)| json!({"name": name, "params": params})).collect::<Vec<_>>(),
        })),
        "strings" => Ok(json!({"ok": true, "table": database.strings()})),
        "search" => {
            let needle = text(params, "text")?;
            let mut hits = Vec::new();
            for dialogue in database.dialogues() {
                for node in dialogue.asset.search(needle) {
                    hits.push(json!({"dialogue": dialogue.id(), "node": node}));
                }
            }
            for quest in database.quests() {
                let text = serde_json::to_string(&quest.asset)
                    .unwrap_or_default()
                    .to_lowercase();
                if text.contains(&needle.to_lowercase()) {
                    hits.push(json!({"quest": quest.id()}));
                }
            }
            Ok(json!({"ok": true, "matches": hits}))
        }
        "simulate" => simulate_query(database, params),
        other => Err(format!(
            "story operation {other} is not one of overview, schema, lint, check, simulate, strings, search"
        )),
    }
}

/// The `simulate` operation: play a dialogue with scripted choices after setup effects.
fn simulate_query(database: StoryDatabase, params: &Value) -> Result<Value, String> {
    let dialogue = text(params, "dialogue")?;
    let entry = params["entry"].as_str().unwrap_or("");
    let choices: Vec<usize> = params["choices"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default();
    let mut story = Story::new(Arc::new(database), params["seed"].as_u64().unwrap_or(1));
    if let Some(setup) = params["setup"].as_str() {
        story.run(setup).map_err(|e| format!("setup: {e}"))?;
    }
    story.drain_events();
    let max_beats = params["max_beats"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(200)
        .min(2000);
    let transcript = simulate(&mut story, dialogue, entry, &choices, max_beats)?;
    let events: Vec<StoryEvent> = story.drain_events();
    Ok(json!({
        "ok": true,
        "transcript": transcript,
        "events": events,
        "facts": story
            .world()
            .explicit()
            .map(|(id, value)| (id.to_owned(), value.clone()))
            .collect::<std::collections::BTreeMap<_, _>>(),
    }))
}

/// The `overview` operation: the cast, the facts, every dialogue, quest and bark set.
fn overview(root: &Path, database: &StoryDatabase, errors: usize, warnings: usize) -> Value {
    json!({
        "ok": true,
        "root": root,
        "story": database.manifest.id,
        "source_locale": database.manifest.source_locale,
        "facts": database.schema.len(),
        "cast": database.manifest.cast.iter().map(|s| json!({"id": s.id, "name": s.name.source, "portrait": s.portrait, "expressions": s.expressions})).collect::<Vec<_>>(),
        "expressions": database.manifest.expressions,
        "tiers": database.manifest.tiers,
        "commands": database.manifest.commands,
        "dialogues": database.dialogues().map(|d| json!({
            "id": d.id(), "title": d.asset.title, "file": database.file_of(&format!("dialogue {}", d.id())),
            "nodes": d.nodes().len(), "entries": d.asset.entries.keys().collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "quests": database.quests().map(|q| json!({
            "id": q.id(), "title": q.asset.title.source, "file": database.file_of(&format!("quest {}", q.id())),
            "stages": q.asset.stages.iter().map(|s| &s.id).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "bark_sets": database.bark_sets().map(|b| json!({"id": b.id, "barks": b.barks.len(), "chatter": b.chatter.len()})).collect::<Vec<_>>(),
        "notes": database.notes().count(),
        "errors": errors,
        "warnings": warnings,
        "syntax": "conditions: met.x and rep.y >= 3 and lang.z >= 'fluent' and 'w' in set and quest('q') == 'active' and visited('node') > 0; effects: x = true; n += 1; set add 'w'; start_quest('q'); journal('note')",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{"version": 1, "id": "t", "facts": [{"id": "met.mira", "type": "bool"}],
        "cast": [{"id": "mira", "name": "Mira"}]}"#;
    const DIALOGUE: &str = r#"{"version": 1, "id": "intro", "nodes": [
        {"type": "line", "id": "n1", "speaker": "mira", "text": "Hello.", "effects": "met.mira = true"}]}"#;

    /// One fixed folder per test, overwritten each run: nothing is ever deleted (Rimeborn RULES 13).
    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("somnium-story-authoring-{name}"))
    }

    fn project(name: &str) -> PathBuf {
        let root = scratch(name);
        let story = root.join("story");
        std::fs::create_dir_all(&story).unwrap();
        std::fs::write(story.join("t.somstory"), MANIFEST).unwrap();
        std::fs::write(story.join("intro.somdialogue"), DIALOGUE).unwrap();
        root
    }

    #[test]
    fn an_edit_is_checked_against_the_whole_project() {
        let content = project("edit");
        let root = story_root(&content);
        assert!(root.ends_with("story"));
        let edited = DIALOGUE.replace("\"mira\", \"text\"", "\"nobody\", \"text\"");
        let result = check(
            &root,
            somnium_ui::graph::dialogue::CATALOGUE,
            &edited,
            Some(&root.join("intro.somdialogue")),
        )
        .unwrap();
        assert_eq!(result.problems.len(), 1, "{:?}", result.problems);
        assert!(result.problems[0].1.contains("nobody is not in the cast"));
        assert!(
            result.database.is_none(),
            "an asset with errors does not play"
        );
        let clean = check(
            &root,
            somnium_ui::graph::dialogue::CATALOGUE,
            DIALOGUE,
            None,
        )
        .unwrap();
        assert!(
            clean.problems.is_empty() && clean.database.is_some(),
            "{}",
            clean.status
        );
    }

    #[test]
    fn the_story_query_reads_lints_and_plays() {
        let content = project("query");
        let root = story_root(&content);
        let overview = query(&root, &json!({})).unwrap();
        assert_eq!(overview["dialogues"][0]["id"], "intro");
        assert_eq!(overview["dialogues"][0]["file"], "intro.somdialogue");
        let lint = query(&root, &json!({"operation": "lint"})).unwrap();
        assert_eq!(lint["errors"], 0);
        let run = query(
            &root,
            &json!({"operation": "simulate", "dialogue": "intro"}),
        )
        .unwrap();
        assert_eq!(run["transcript"][0]["text"], "Hello.");
        assert_eq!(run["facts"]["met.mira"], true);
        let checked = query(
            &root,
            &json!({"operation": "check", "document_type": "dialogue", "content": serde_json::from_str::<Value>(DIALOGUE).unwrap()}),
        )
        .unwrap();
        assert_eq!(checked["problems"].as_array().unwrap().len(), 0);
        let strings = query(&root, &json!({"operation": "strings"})).unwrap();
        assert_eq!(strings["table"]["strings"]["dlg.intro.n1"], "Hello.");
        assert!(query(&root, &json!({"operation": "dance"})).is_err());
    }

    /// A story saved to an engine save slot in the middle of a scene loads saying and showing exactly what it did:
    /// facts, quests and their markers, what was heard, the journal with its people and the Voice's log, the scene
    /// and the presenter's log.
    #[test]
    fn a_story_saved_to_a_slot_mid_scene_loads_saying_and_showing_the_same() {
        use crate::save_game::{SaveGame, SaveSlots};
        use somnium_story::{Beat, Conversation, ConversationSave, Presenter, StorySave};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/story");
        let (database, _) = StoryDatabase::load_dir(&root).unwrap_or_else(|i| panic!("{i:?}"));
        let database = Arc::new(database);
        let mut story = Story::new(Arc::clone(&database), 3);
        let mut presenter = Presenter::default();
        let mut talk = Conversation::start(&mut story, "lamplighter", "").unwrap();
        let show = |talk: &Conversation, presenter: &mut Presenter| match talk.beat() {
            Beat::Line(line) => presenter.show_line(line.clone(), None),
            Beat::Choices { choices, .. } => presenter.show_choices(choices.clone()),
            _ => {}
        };
        show(&talk, &mut presenter);
        while !matches!(talk.beat(), Beat::Choices { .. }) {
            talk.advance(&mut story).unwrap();
            show(&talk, &mut presenter);
        }
        let Beat::Choices { choices, .. } = talk.beat().clone() else {
            unreachable!()
        };
        let help = choices.iter().position(|c| c.node == "help").unwrap();
        talk.choose(&mut story, help).unwrap();
        show(&talk, &mut presenter);
        story.set("lamps.lit", 3).unwrap();
        assert!(
            story
                .drain_events()
                .iter()
                .any(|e| matches!(e, StoryEvent::Voice { .. })),
            "the lamps objective's on_done had the Voice speak"
        );

        let mut save = SaveGame::new(1);
        story.save().write_into(&mut save.state).unwrap();
        let mut scene = talk.save().unwrap();
        scene.history = presenter.history_for_save();
        scene.write_into(&mut save.state).unwrap();
        let slots = SaveSlots::new(scratch("slots"));
        slots.write("act_one", &save).unwrap();
        let back = slots.read("act_one").unwrap();

        let (mut loaded, notes) = Story::load(
            Arc::clone(&database),
            &StorySave::read_from(&back.state).unwrap().unwrap(),
        )
        .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let saved_scene = ConversationSave::read_from(&back.state).unwrap().unwrap();
        let again = Conversation::restore(&mut loaded, &saved_scene).unwrap();
        let mut shown = Presenter::default();
        shown.restore_history(saved_scene.history.clone());
        show(&again, &mut shown);

        assert_eq!(again.beat(), talk.beat(), "the same line on screen");
        assert_eq!(
            shown.history(),
            presenter.history(),
            "the same log behind it"
        );
        let view = loaded.journal_view();
        assert_eq!(view, story.journal_view());
        assert_eq!(view.voice.len(), 1);
        assert_eq!(view.voice[0].text, "All 3 lamps burn again.");
        assert!(view.people.iter().any(|p| p.id == "oren"));
        assert_eq!(loaded.markers(), story.markers());
        assert_eq!(loaded.save(), story.save());
        assert!(loaded.drain_events().is_empty(), "loading ran nothing");
    }

    /// The sample `hello_engine` ships must stay clean and playable down every road.
    #[test]
    fn the_shipped_sample_story_is_clean_and_plays() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/story");
        let lint = query(&root, &json!({"operation": "lint"})).unwrap();
        assert_eq!(lint["issues"].as_array().unwrap().len(), 0, "{lint:#}");
        let play = |choices: Value, setup: &str| {
            query(
                &root,
                &json!({"operation": "simulate", "dialogue": "lamplighter", "choices": choices, "setup": setup, "seed": 3}),
            )
            .unwrap()
        };
        let helped = play(json!([1, 0]), "");
        let last = helped["transcript"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["kind"], "end");
        assert_eq!(last["result"], "helping");
        assert!(
            helped["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["event"] == "quest_started")
        );
        assert_eq!(
            helped["facts"]["heard.lamp_story"], true,
            "the called tale ran"
        );
        let refused = play(json!([2, 1]), "night.dark = false");
        let kinds: Vec<&str> = refused["transcript"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"chose"), "{refused:#}");
        let fluent = play(json!([3, 0]), "lang.old_tongue = 'fluent'");
        assert!(
            fluent["transcript"]
                .to_string()
                .contains("Where did you learn"),
            "{fluent:#}"
        );
    }

    #[test]
    fn saved_assets_are_canonical() {
        let text = canonical(somnium_ui::graph::dialogue::CATALOGUE, DIALOGUE).unwrap();
        assert!(text.ends_with("}\n") && text.contains("\n  \"nodes\""));
        assert_eq!(
            canonical(somnium_ui::graph::dialogue::CATALOGUE, &text).unwrap(),
            text
        );
        assert_eq!(
            catalogue_for(Path::new("a/b.somquest")),
            Some(somnium_ui::graph::quest::CATALOGUE)
        );
        assert!(is_story_file(Path::new("x.sombarks")) && !is_story_file(Path::new("x.somgraph")));
    }
}
