//! Host side of the graph editor's story catalogues: Open, Save, Check and Play for dialogue and quest assets.
//!
//! The graph control converts between the graph and the asset itself (it keeps the text keys a graph cannot hold),
//! so everything arriving here is asset JSON. Checking builds the project's whole story with the edit in; Play hands
//! that build to the control, which runs the conversation on its own panel.
use super::*;
use crate::story_authoring as story;

impl<G: GameApp> Engine<G> {
    /// The project's story folder.
    pub(super) fn story_root(&self) -> std::path::PathBuf {
        story::story_root(&self.config.content_root)
    }

    /// Open a `*.somdialogue` or `*.somquest` on the graph control.
    pub(super) fn open_story_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let catalogue = story::catalogue_for(path).ok_or("not a dialogue or quest file")?;
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let ui = self.ui_manager.as_mut().ok_or("Editor unavailable")?;
        ui.open_story_document(catalogue, &json, Some(&path.to_string_lossy()));
        Ok(())
    }

    /// Save (`apply` false), Check (`apply` and `preview`) or Play (`apply` alone) a story asset.
    pub(super) fn apply_story_document(
        &mut self,
        catalogue: &str,
        json: &str,
        apply: bool,
        preview: bool,
        source: Option<&str>,
    ) -> Result<String, String> {
        let root = self.story_root();
        if !apply {
            let text = story::canonical(catalogue, json)?;
            let extension = story::extension_for(catalogue);
            let path = source.map(std::path::PathBuf::from).or_else(|| {
                let id = story::asset_key(catalogue, json)
                    .ok()
                    .and_then(|key| key.split_once(' ').map(|(_, id)| id.to_owned()))
                    .unwrap_or_else(|| "new".into());
                rfd::FileDialog::new()
                    .add_filter(
                        if extension == "somquest" {
                            "Quest"
                        } else {
                            "Dialogue"
                        },
                        &[extension],
                    )
                    .set_directory(&root)
                    .set_file_name(format!("{id}.{extension}"))
                    .save_file()
            });
            let Some(path) = path else {
                return Ok("Save cancelled".into());
            };
            if path.extension().and_then(|e| e.to_str()) != Some(extension) {
                return Err(format!(
                    "a {} saves as .{extension}",
                    if extension == "somquest" {
                        "quest"
                    } else {
                        "dialogue"
                    }
                ));
            }
            crate::save_game::atomic_write(&path, text.as_bytes())?;
            if let Some(ui) = &mut self.ui_manager {
                ui.set_authoring_graph_source(&path.to_string_lossy());
            }
            return Ok(format!("Saved {}", path.display()));
        }
        let result = story::check(&root, catalogue, json, source.map(std::path::Path::new))?;
        let errors = result.problems.iter().any(|p| !p.2);
        if let Some(ui) = &mut self.ui_manager {
            ui.set_story_problems(result.problems.clone());
        }
        if preview {
            return if errors {
                Err(result.status)
            } else {
                Ok(result.status)
            };
        }
        if catalogue != somnium_ui::graph::dialogue::CATALOGUE {
            return Err("Play runs dialogues; Check validates a quest".into());
        }
        match result.database {
            Some(database) if !errors => {
                if let Some(ui) = &mut self.ui_manager {
                    ui.start_story_preview(database);
                }
                Ok("Playing · Space: next · 1-9: choose · Esc: stop".into())
            }
            _ => Err(result.status),
        }
    }
}
