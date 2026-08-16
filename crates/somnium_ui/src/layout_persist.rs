//! Persist splitter widths across editor sessions (Phase 26-I).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChromeLayout {
    pub tools: f32,
    pub viewport: f32,
    pub outliner: f32,
}

impl Default for ChromeLayout {
    fn default() -> Self {
        Self {
            tools: 128.0,
            viewport: 720.0,
            outliner: 240.0,
        }
    }
}

impl ChromeLayout {
    /// Resolve stored absolute pixel widths against the actual window.
    ///
    /// Phase 26-Zeta-F. These values are absolutes, so a layout written on one
    /// monitor — or the 720 px default — leaves the Details column at whatever
    /// is left over, which on a 1920 px window was over a thousand pixels of
    /// inspector and a postage-stamp viewport. Clamping the *derived* right
    /// column into the redline's 240–520 range fixes that without discarding a
    /// deliberate splitter drag, because a drag inside the range round-trips
    /// unchanged.
    pub fn resolved(mut self, window_w: f32, window_h: f32) -> Self {
        const SPLITTER: f32 = 6.0;
        self.tools = self.tools.clamp(120.0, 280.0);
        self.outliner = self.outliner.clamp(120.0, (window_h * 0.6).max(160.0));

        let available = (window_w - self.tools - SPLITTER * 2.0).max(240.0);
        let details = (available - self.viewport).clamp(240.0, 520.0);
        self.viewport = (available - details).max(200.0);
        self
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_details_column_lands_in_the_redline_range_at_every_size() {
        for (w, h) in [
            (1280.0, 720.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            let l = ChromeLayout::default().resolved(w, h);
            let details = w - l.tools - l.viewport - 12.0;
            assert!(
                (239.0..=521.0).contains(&details),
                "{w}x{h}: details column {details}"
            );
            assert!(
                l.viewport > l.tools + details,
                "{w}x{h}: viewport {} is not the largest region",
                l.viewport
            );
        }
    }

    #[test]
    fn a_deliberate_drag_inside_the_range_round_trips() {
        // Resolving must not fight the user: a stored layout whose Details
        // column is already legal comes back unchanged.
        let stored = ChromeLayout {
            tools: 200.0,
            viewport: 1920.0 - 200.0 - 12.0 - 380.0,
            outliner: 300.0,
        };
        assert_eq!(stored.resolved(1920.0, 1080.0), stored);
    }
}
