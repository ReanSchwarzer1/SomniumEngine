//! Persist splitter widths across editor sessions (Phase 26-I).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ChromeLayout {
    pub tools: f32,
    pub viewport: f32,
    pub outliner: f32,
}

impl Default for ChromeLayout {
    fn default() -> Self {
        Self {
            tools: 40.0,
            viewport: 720.0,
            outliner: 240.0,
        }
    }
}

fn layout_path() -> PathBuf {
    let mut dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    dir.push("SomniumEngine");
    dir.push("editor_layout.json");
    dir
}

pub fn load() -> ChromeLayout {
    let path = layout_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(layout: ChromeLayout) {
    let path = layout_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&layout) {
        let _ = std::fs::write(path, json);
    }
}
