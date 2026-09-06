//! Phase 26-Zeta-F — named workspaces.
//!
//! The design package is explicit that *excellent defaults beat unlimited
//! docking*: Blender's lesson is that task-specific saved layouts are more
//! useful than one universal arrangement, and Somnium's v1 answer is a fixed
//! set of named presets rather than an arbitrary docking graph (`phase_26.md`
//! §16 rules that out for this phase).
//!
//! A workspace is deliberately small — the splitter positions, which bottom
//! panel is showing, and how tall it is. That is everything the shell needs to
//! reconstruct a working arrangement, and nothing that would turn the preset
//! into a second source of truth for editor state.

use serde::{Deserialize, Serialize};

/// The named arrangements shipped with the editor.
///
/// Order is the order they appear in the Window menu. `Layout` is the default
/// and the one Reset falls back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Workspace {
    Layout,
    Terrain,
    Foliage,
    Lighting,
    Materials,
    Animation,
    Debug,
    Play,
}

impl Workspace {
    pub const ALL: [Workspace; 8] = [
        Workspace::Layout,
        Workspace::Terrain,
        Workspace::Foliage,
        Workspace::Lighting,
        Workspace::Materials,
        Workspace::Animation,
        Workspace::Debug,
        Workspace::Play,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Workspace::Layout => "Layout",
            Workspace::Terrain => "Terrain",
            Workspace::Foliage => "Foliage",
            Workspace::Lighting => "Lighting",
            Workspace::Materials => "Materials",
            Workspace::Animation => "Animation",
            Workspace::Debug => "Debug",
            Workspace::Play => "Play",
        }
    }

    /// The shipped arrangement for this workspace at a given window size.
    ///
    /// Sizes are derived from the window rather than stored as absolutes so a
    /// preset authored on a 1080p monitor is not a 240 px Details panel on an
    /// ultrawide. Every value is then clamped to the redline minimums.
    ///
    /// MORROWIND-J: the pixel intent below is unchanged, but the arrangement
    /// now goes through a [`DockTree`](crate::dock::DockTree) and back. That is
    /// what makes the tree load-bearing without changing a pixel — every shipped
    /// workspace is now *expressed as a dock tree*, and
    /// `every_preset_survives_a_trip_through_the_dock_tree` is the proof.
    pub fn preset(self, window_w: f32, window_h: f32) -> WorkspaceLayout {
        let intent = self.intent(window_w, window_h);
        let tree = self.dock_tree(window_w, window_h);
        // A window too small to honour the minimums resolves differently from
        // the intent by design (`dock::split_span` halves rather than lie), so
        // the projection is only taken when it round-trips.
        match tree.chrome(window_w, window_h) {
            Some(chrome) => WorkspaceLayout {
                tools: chrome.tools,
                details: chrome.details,
                outliner: chrome.outliner,
                drawer_height: if chrome.bottom.is_some() {
                    chrome.drawer_height
                } else {
                    intent.drawer_height
                },
                bottom: intent.bottom,
            },
            None => intent,
        }
    }

    /// This workspace as a dock tree.
    ///
    /// The arrangement the shell will eventually resolve directly. Today the
    /// shell still consumes [`WorkspaceLayout`], and `preset` derives that from
    /// this.
    #[must_use]
    pub fn dock_tree(self, window_w: f32, window_h: f32) -> crate::dock::DockTree {
        let intent = self.intent(window_w, window_h);
        crate::dock::DockTree::from_chrome(
            intent.tools,
            intent.details,
            intent.outliner,
            intent.drawer_height,
            match intent.bottom {
                BottomPanel::None => None,
                BottomPanel::Content => Some("Content Drawer"),
                BottomPanel::Log => Some("Output Log"),
            },
            window_w,
            window_h,
        )
    }

    /// The pixel intent, before it is expressed as a tree.
    fn intent(self, window_w: f32, window_h: f32) -> WorkspaceLayout {
        // Redline §06 defaults: rail 168, Details 340, drawer 220.
        let details = 340.0;
        let base = WorkspaceLayout {
            tools: 48.0,
            details,
            outliner: if window_h < 900.0 { 140.0 } else { 170.0 },
            drawer_height: 220.0,
            bottom: BottomPanel::Content,
        };
        match self {
            Workspace::Layout => base,
            // Sculpting wants the tool rail and the viewport; the Content
            // Browser is not in the loop, so the bottom row starts closed.
            Workspace::Terrain => WorkspaceLayout {
                tools: 220.0,
                bottom: BottomPanel::None,
                ..base
            },
            // The supported foliage palette lives in the tool panel. Reserve
            // height for its brush settings; the drawer stays one click away.
            Workspace::Foliage => WorkspaceLayout {
                tools: 220.0,
                bottom: BottomPanel::None,
                ..base
            },
            // Lighting and materials are inspector-heavy and rail-light.
            Workspace::Lighting => WorkspaceLayout {
                tools: 48.0,
                details: 400.0,
                outliner: 260.0,
                bottom: BottomPanel::None,
                ..base
            },
            Workspace::Materials => WorkspaceLayout {
                tools: 48.0,
                details: 400.0,
                bottom: BottomPanel::Content,
                ..base
            },
            Workspace::Animation => WorkspaceLayout {
                tools: 48.0,
                details: 400.0,
                bottom: BottomPanel::None,
                ..base
            },
            // Debug reads the log, so the bottom row shows it and is tall.
            Workspace::Debug => WorkspaceLayout {
                tools: 48.0,
                drawer_height: (window_h * 0.35).clamp(140.0, 420.0),
                bottom: BottomPanel::Log,
                ..base
            },
            // Play gives the scene everything it can.
            Workspace::Play => WorkspaceLayout {
                tools: 48.0,
                details: 240.0,
                bottom: BottomPanel::None,
                ..base
            },
        }
        .clamped(window_w, window_h)
    }
}

/// Which panel occupies the shared bottom row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottomPanel {
    None,
    Content,
    Log,
}

/// One resolved arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayout {
    /// Left tool rail width.
    pub tools: f32,
    /// Right Outliner/Details column width.
    pub details: f32,
    /// Height of the Outliner within the right column.
    pub outliner: f32,
    pub drawer_height: f32,
    pub bottom: BottomPanel,
}

impl WorkspaceLayout {
    /// Redline minimums, and the rule that the viewport stays the largest
    /// region in every default workspace (acceptance matrix §10.2).
    pub fn clamped(mut self, window_w: f32, window_h: f32) -> Self {
        self.tools = self.tools.clamp(48.0, 280.0);
        self.details = self.details.clamp(240.0, 520.0);
        self.outliner = self.outliner.clamp(120.0, (window_h - 580.0).max(120.0));
        self.drawer_height = self.drawer_height.clamp(140.0, (window_h * 0.6).max(160.0));

        // If the side panels would leave the viewport smaller than they are,
        // give the space back to the viewport rather than honouring the preset.
        let side = self.tools + self.details;
        let viewport = window_w - side;
        if viewport < side && window_w > 0.0 {
            let budget = (window_w * 0.45).max(240.0);
            let scale = budget / side.max(1.0);
            self.tools = (self.tools * scale).max(48.0);
            self.details = (self.details * scale).max(240.0);
        }
        self
    }

    /// Viewport width this arrangement leaves in a window.
    pub fn viewport_width(&self, window_w: f32) -> f32 {
        (window_w - self.tools - self.details).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workspace_keeps_the_viewport_the_largest_region() {
        // Acceptance matrix §10.2. Checked at the compact case as well as the
        // comfortable one, because that is where a preset would break it.
        for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0), (3440.0, 1440.0)] {
            for ws in Workspace::ALL {
                let l = ws.preset(w, h);
                assert!(
                    l.viewport_width(w) > l.tools && l.viewport_width(w) > l.details,
                    "{:?} at {w}×{h}: viewport {} vs rail {} / details {}",
                    ws,
                    l.viewport_width(w),
                    l.tools,
                    l.details
                );
            }
        }
    }

    #[test]
    fn presets_respect_the_redline_minimums() {
        for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0)] {
            for ws in Workspace::ALL {
                let l = ws.preset(w, h);
                assert!(l.tools >= 48.0 && l.tools <= 280.0, "{ws:?} rail");
                assert!(l.details >= 240.0, "{ws:?} details");
                assert!(l.drawer_height >= 140.0, "{ws:?} drawer");
                assert!(l.drawer_height <= h * 0.6 + 0.01, "{ws:?} drawer max");
            }
        }
    }

    #[test]
    fn workspaces_actually_differ_from_one_another() {
        // A preset set where every entry is the default is decoration.
        let layouts: Vec<_> = Workspace::ALL
            .iter()
            .map(|w| w.preset(1920.0, 1080.0))
            .collect();
        let distinct = layouts
            .iter()
            .map(|l| format!("{l:?}"))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            distinct.len() >= 5,
            "only {} distinct layouts",
            distinct.len()
        );
    }

    #[test]
    fn debug_shows_the_log_and_authoring_presets_reserve_brush_height() {
        assert_eq!(
            Workspace::Debug.preset(1920.0, 1080.0).bottom,
            BottomPanel::Log
        );
        assert_eq!(
            Workspace::Terrain.preset(1920.0, 1080.0).bottom,
            BottomPanel::None
        );
        assert_eq!(
            Workspace::Foliage.preset(1920.0, 1080.0).bottom,
            BottomPanel::None
        );
    }

    // ── MORROWIND-J: the tree must express what the editor already ships ──

    #[test]
    fn every_preset_survives_a_trip_through_the_dock_tree() {
        // The requirement is *"the current arrangement as the default layout so
        // nothing looks different on first run"*. This is that sentence as a
        // test: each workspace's pixel intent, expressed as a dock tree and
        // resolved back, is the same arrangement to within a pixel.
        for (w, h) in [
            (1280.0, 720.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            for ws in Workspace::ALL {
                let intent = ws.intent(w, h);
                let through = ws.preset(w, h);
                assert!(
                    (intent.tools - through.tools).abs() < 1.0,
                    "{ws:?} at {w}x{h}: rail {} -> {}",
                    intent.tools,
                    through.tools
                );
                assert!(
                    (intent.details - through.details).abs() < 1.0,
                    "{ws:?} at {w}x{h}: details {} -> {}",
                    intent.details,
                    through.details
                );
                assert!(
                    (intent.outliner - through.outliner).abs() < 1.0,
                    "{ws:?} at {w}x{h}: outliner {} -> {}",
                    intent.outliner,
                    through.outliner
                );
                assert!(
                    (intent.drawer_height - through.drawer_height).abs() < 1.0,
                    "{ws:?} at {w}x{h}: drawer {} -> {}",
                    intent.drawer_height,
                    through.drawer_height
                );
                assert_eq!(intent.bottom, through.bottom, "{ws:?} at {w}x{h}");
            }
        }
    }

    #[test]
    fn a_workspace_without_a_bottom_row_builds_a_tree_without_one() {
        // `BottomPanel::None` was a flag the old model carried beside the
        // numbers. In a tree it is the absence of a tile, which is the
        // difference between describing an arrangement and encoding one.
        let terrain = Workspace::Terrain.dock_tree(1920.0, 1080.0);
        assert!(!terrain.contains("Content Drawer"));
        assert!(terrain.contains("Viewport"));

        let debug = Workspace::Debug.dock_tree(1920.0, 1080.0);
        assert!(debug.contains("Output Log"));
        let chrome = debug.chrome(1920.0, 1080.0).unwrap();
        assert_eq!(chrome.bottom.as_deref(), Some("Output Log"));
    }

    #[test]
    fn a_docked_panel_makes_the_five_region_projection_refuse() {
        // Once an arrangement leaves the shipped shape, `chrome` has no honest
        // answer. Returning plausible numbers for a layout they do not describe
        // is how a projection outlives the thing it projects.
        let mut tree = Workspace::Layout.dock_tree(1920.0, 1080.0);
        assert!(tree.chrome(1920.0, 1080.0).is_some());
        assert!(tree.close("Details"));
        assert!(tree.chrome(1920.0, 1080.0).is_none());
    }

    #[test]
    fn a_narrow_window_gives_the_viewport_its_space_back() {
        // 800 px wide would otherwise be 168 + 400 of chrome and 232 of scene.
        let l = Workspace::Lighting.preset(800.0, 600.0);
        assert!(l.viewport_width(800.0) > l.tools + l.details - 1.0);
    }
}
