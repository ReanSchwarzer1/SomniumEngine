//! Persist splitter widths across editor sessions (Phase 26-I).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Redline §06 default and range for the right-hand Outliner/Details column.
pub const DETAILS_DEFAULT: f32 = 340.0;
pub const DETAILS_MIN: f32 = 240.0;
pub const DETAILS_MAX: f32 = 520.0;
/// Splitter thickness, twice — the tool splitter and the content splitter.
const SPLITTERS: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChromeLayout {
    pub tools: f32,
    /// Viewport width in pixels. **Derived**, not authoritative: it is what the
    /// splitter widget wants, but it is meaningless on a window of a different
    /// size, so it is recomputed from `details` on every load.
    pub viewport: f32,
    /// Right column width. This is the value that actually transfers across
    /// window sizes and monitors, and the one a splitter drag records.
    ///
    /// `serde(default)` so a layout file written before this field existed
    /// still loads; `resolved` then rebuilds it from the legacy `viewport`.
    #[serde(default)]
    pub details: f32,
    pub outliner: f32,
}

impl Default for ChromeLayout {
    fn default() -> Self {
        Self {
            tools: 168.0,
            viewport: 0.0,
            details: DETAILS_DEFAULT,
            outliner: 300.0,
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
        self.tools = self.tools.clamp(120.0, 280.0);
        self.outliner = self.outliner.clamp(120.0, (window_h * 0.6).max(160.0));

        let available = (window_w - self.tools - SPLITTERS).max(DETAILS_MIN);
        // Prefer the stored column width; fall back to deriving it from a
        // legacy file's absolute viewport.
        let stored = if self.details > 0.0 {
            self.details
        } else {
            available - self.viewport
        };
        // A value outside the range is not clamped to the nearest edge — it is
        // a layout from a different window, and pinning it to 240 or 520 would
        // hand the user a panel they never chose. The shipped default is the
        // better answer, capped so it cannot eat the viewport on a small screen.
        let details = if (DETAILS_MIN..=DETAILS_MAX).contains(&stored) {
            stored
        } else {
            DETAILS_DEFAULT.min(available * 0.4).max(DETAILS_MIN)
        };
        self.details = details.min((available - 200.0).max(DETAILS_MIN));
        self.viewport = (available - self.details).max(200.0);
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
            viewport: 1920.0 - 200.0 - SPLITTERS - 380.0,
            details: 380.0,
            outliner: 300.0,
        };
        assert_eq!(stored.resolved(1920.0, 1080.0), stored);
    }

    #[test]
    fn a_column_width_transfers_between_window_sizes() {
        // The whole point of storing the column rather than the viewport: drag
        // Details to 380 on a 2560 monitor, reopen on a 1920 one, still 380.
        let wide = ChromeLayout {
            details: 380.0,
            ..ChromeLayout::default()
        }
        .resolved(2560.0, 1440.0);
        let narrow = wide.resolved(1920.0, 1080.0);
        assert_eq!(narrow.details, 380.0);
        assert!(narrow.viewport > 0.0);
    }

    #[test]
    fn a_legacy_file_with_a_nonsense_viewport_falls_back_to_the_default() {
        // The real case that produced this: a file written on a wide window
        // stored viewport 2040, which on a 1280 window derives a negative
        // column. Pinning that to the 240 minimum gave a cramped panel nobody
        // asked for.
        let legacy = ChromeLayout {
            tools: 120.0,
            viewport: 2040.0,
            details: 0.0,
            outliner: 300.0,
        };
        let r = legacy.resolved(1280.0, 720.0);
        assert!(
            r.details > DETAILS_MIN,
            "fell back to the clamp, not the default"
        );
        assert!(
            r.viewport > r.details,
            "viewport is still the larger region"
        );
    }
}
