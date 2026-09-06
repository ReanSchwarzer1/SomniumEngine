//! Persist splitter widths across editor sessions (Phase 26-I), and the dock
//! arrangement (MORROWIND-J).

use crate::dock::DockTree;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Redline §06 default and range for the right-hand Outliner/Details column.
pub const DETAILS_DEFAULT: f32 = 340.0;
pub const DETAILS_MIN: f32 = 240.0;
pub const DETAILS_MAX: f32 = 520.0;
/// Splitter thickness, twice — the tool splitter and the content splitter.
const SPLITTERS: f32 = 12.0;

/// What the shell looked like when the editor last closed: the widths a
/// splitter drag records, and which panels were in windows of their own.
///
/// Deliberately not `Copy`. It stopped being a handful of floats when it
/// gained the floating set, and a hidden clone of a growing struct on every
/// splitter drag is not something a derive should be deciding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Panels that were in their own window when the editor last closed.
    ///
    /// MORROWIND-J step 2. Stored by the panel's own slug rather than by
    /// index, so a build that adds a floatable panel reads an old file without
    /// mistaking one panel for another; an unrecognised name is dropped on
    /// load, which is what makes the file safe to hand backwards as well.
    ///
    /// `serde(default)` because every layout file written before this existed
    /// has no such key, and an editor that would not open over a missing
    /// key would be a worse editor than one that opens with nothing floating.
    #[serde(default)]
    pub floating: Vec<String>,
}

impl Default for ChromeLayout {
    fn default() -> Self {
        Self {
            tools: 48.0,
            viewport: 0.0,
            details: DETAILS_DEFAULT,
            outliner: 170.0,
            floating: Vec::new(),
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
        // A name this build does not know is a panel it cannot open, and
        // carrying it would leave the file claiming a window that never
        // appears. Duplicates go the same way: the manager keys on the panel.
        let mut seen = std::collections::BTreeSet::new();
        self.floating.retain(|name| {
            crate::floating::FloatingKind::from_slug(name).is_some() && seen.insert(name.clone())
        });
        self.tools = self.tools.clamp(48.0, 280.0);
        // Reserve room for sticky Details identity/filter rows and authored fields
        // above the default drawer at the 720 px target size.
        self.outliner = self.outliner.clamp(120.0, (window_h - 580.0).max(120.0));

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

/// Where the dock arrangement lives.
///
/// A **second file**, beside `editor_layout.json` rather than inside it. The
/// two answer different questions and fail differently: a corrupt splitter
/// width costs a column, and a corrupt dock tree costs the whole shell. Keeping
/// them apart means a tree that will not load falls back to the shipped
/// arrangement without also discarding a splitter drag, and a build that
/// predates docking still reads its own file untouched.
fn dock_path() -> PathBuf {
    let mut path = layout_path();
    path.set_file_name("editor_dock.json");
    path
}

/// Load the stored dock arrangement, repaired.
///
/// Never fails. A missing, unparsable or nonsensical file becomes
/// [`DockTree::default_layout`], because an editor that will not open because
/// its layout file is bad is worse than an editor that opens with the layout it
/// shipped with. [`DockTree::repair`] is the whole reason this can promise
/// that: a file that parses but describes an impossible tree — an empty tile, a
/// panel in two places, a ratio of 40 — is repaired rather than trusted or
/// rejected.
#[must_use]
pub fn load_dock() -> DockTree {
    let mut tree: DockTree = std::fs::read_to_string(dock_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    tree.repair();
    tree
}

/// Store the dock arrangement.
pub fn save_dock(tree: &DockTree) {
    let path = dock_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(tree) {
        let _ = std::fs::write(path, json);
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

pub fn save(layout: &ChromeLayout) {
    let path = layout_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(layout) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MORROWIND-J: the dock arrangement ─────────────────────────────────

    #[test]
    fn a_dock_file_that_will_not_parse_opens_the_shipped_layout() {
        // The failure this exists for: a truncated write, a hand edit, or a
        // file from a build that spelled the tree differently. None of them may
        // stop the editor opening.
        for text in ["", "{", "null", r#"{"root":{"Tabs":{"panels":[]}}}"#] {
            let parsed: Option<DockTree> = serde_json::from_str(text).ok();
            let mut tree = parsed.unwrap_or_default();
            tree.repair();
            assert!(
                tree.contains("Viewport"),
                "{text:?} left no viewport to draw into"
            );
        }
    }

    #[test]
    fn the_two_layout_files_are_separate_on_purpose() {
        // Stated as a test because the temptation to merge them is obvious and
        // the reason not to is not: they fail differently, and a bad tree must
        // not cost a good splitter drag.
        assert_ne!(layout_path(), dock_path());
        assert_eq!(layout_path().parent(), dock_path().parent());
    }

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

    // ── MORROWIND-J: which panels were in windows of their own ────────────

    #[test]
    fn a_layout_file_from_before_floating_windows_still_loads() {
        // Every file on disk today has no such key. An editor that refused to
        // open over a missing one would be a worse editor than one that opens
        // with nothing floating, which is also what it looked like when the
        // file was written.
        let legacy = r#"{"tools":168.0,"viewport":0.0,"details":340.0,"outliner":300.0}"#;
        let parsed: ChromeLayout = serde_json::from_str(legacy).expect("parses");
        assert!(parsed.floating.is_empty());
    }

    #[test]
    fn a_panel_this_build_does_not_have_is_dropped_rather_than_carried() {
        // A file written by a later build names a panel this one cannot open.
        // Carrying it would leave the layout claiming a window that never
        // appears, and there would be nothing to close.
        let ahead = ChromeLayout {
            floating: vec![
                "outliner".into(),
                "timeline".into(),
                "outliner".into(),
                "".into(),
            ],
            ..ChromeLayout::default()
        };
        let r = ahead.resolved(1920.0, 1080.0);
        assert_eq!(
            r.floating,
            vec!["outliner".to_string()],
            "unknown and duplicate dropped"
        );
    }

    #[test]
    fn every_floatable_panel_survives_the_file() {
        for kind in crate::floating::FloatingKind::ALL {
            let stored = ChromeLayout {
                floating: vec![kind.slug().to_owned()],
                ..ChromeLayout::default()
            };
            let text = serde_json::to_string(&stored).expect("serialises");
            let back: ChromeLayout = serde_json::from_str(&text).expect("parses");
            let back = back.resolved(1920.0, 1080.0);
            assert_eq!(
                back.floating,
                vec![kind.slug().to_string()],
                "{kind:?} did not survive the round trip"
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
            ..ChromeLayout::default()
        };
        assert_eq!(stored.clone().resolved(1920.0, 1080.0), stored);
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
            ..ChromeLayout::default()
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
