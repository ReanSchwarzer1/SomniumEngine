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
    pub fn preset(self, window_w: f32, window_h: f32) -> WorkspaceLayout {
        // Redline §06 defaults: rail 168, Details 340, drawer 220.
        let details = 340.0;
        let base = WorkspaceLayout {
            tools: 168.0,
            details,
            outliner: 300.0,
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
            // Painting foliage is a drag-from-the-browser workflow, so the
            // drawer is open and taller than default.
            Workspace::Foliage => WorkspaceLayout {
                tools: 200.0,
                drawer_height: 280.0,
                bottom: BottomPanel::Content,
                ..base
            },
            // Lighting and materials are inspector-heavy and rail-light.
            Workspace::Lighting => WorkspaceLayout {
                tools: 120.0,
                details: 400.0,
                outliner: 260.0,
                bottom: BottomPanel::None,
                ..base
            },
            Workspace::Materials => WorkspaceLayout {
                tools: 120.0,
                details: 400.0,
                bottom: BottomPanel::Content,
                ..base
            },
            Workspace::Animation => WorkspaceLayout {
                tools: 120.0,
                details: 400.0,
                bottom: BottomPanel::None,
                ..base
            },
            // Debug reads the log, so the bottom row shows it and is tall.
            Workspace::Debug => WorkspaceLayout {
                tools: 120.0,
                drawer_height: (window_h * 0.35).clamp(140.0, 420.0),
                bottom: BottomPanel::Log,
                ..base
            },
            // Play gives the scene everything it can.
            Workspace::Play => WorkspaceLayout {
                tools: 120.0,
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
        self.tools = self.tools.clamp(120.0, 280.0);
        self.details = self.details.clamp(240.0, 520.0);
        self.outliner = self.outliner.clamp(120.0, (window_h * 0.6).max(160.0));
        self.drawer_height = self.drawer_height.clamp(140.0, (window_h * 0.6).max(160.0));

        // If the side panels would leave the viewport smaller than they are,
        // give the space back to the viewport rather than honouring the preset.
        let side = self.tools + self.details;
        let viewport = window_w - side;
        if viewport < side && window_w > 0.0 {
            let budget = (window_w * 0.45).max(240.0);
            let scale = budget / side.max(1.0);
            self.tools = (self.tools * scale).max(120.0);
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
                assert!(l.tools >= 120.0 && l.tools <= 280.0, "{ws:?} rail");
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
    fn debug_shows_the_log_and_terrain_shows_nothing() {
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
            BottomPanel::Content
        );
    }

    #[test]
    fn a_narrow_window_gives_the_viewport_its_space_back() {
        // 800 px wide would otherwise be 168 + 400 of chrome and 232 of scene.
        let l = Workspace::Lighting.preset(800.0, 600.0);
        assert!(l.viewport_width(800.0) > l.tools + l.details - 1.0);
    }
}
