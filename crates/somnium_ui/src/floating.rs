//! A panel living in its own OS window.
//!
//! MORROWIND-J step 2. Step 1 built a dock tree that can express where a panel
//! sits; this is the case it cannot express, because the panel is not in the
//! window at all.
//!
//! # What a floating panel actually needs
//!
//! Not a widget. A second OS window is a second **surface**, and a surface is
//! the thing a [`crate::pass::UiPass`] renders into, so the host owns a second
//! window, a second surface and a second pass. What it does *not* own is a
//! second widget tree:
//!
//! ```text
//!   main window          floating window
//!   ───────────          ───────────────
//!   winit::Window        winit::Window        ┐ the host's (somnium_core)
//!   wgpu::Surface        wgpu::Surface        │
//!   UiPass               UiPass               ┘
//!         ╲                    ╱
//!          ╲                  ╱
//!        one UserInterface, one pool of handles
//!          root ── … ── dock          detached ── DETAILS
//! ```
//!
//! # Why the panel is moved rather than rebuilt
//!
//! The first cut of this built the panel a *second* time in a second
//! [`crate::ui::UserInterface`], from the same data. That works, and it works
//! only for a panel whose entire content is a store: the Output Log's lines,
//! the Outliner's projected rows. Details fails it. Its rows are generated from
//! reflected schemas and every one of them is wired to an editing path through
//! a map keyed on the row's handle, so rebuilding it in a second tree means
//! rebuilding that wiring too, and then keeping two copies of it honest.
//!
//! So the panel is not rebuilt. It is **detached**: unlinked from its parent in
//! the tree it already lives in, laid out against the floating window's size,
//! and drawn into the floating window's surface.
//! [`crate::ui::UserInterface::detach`] is the whole mechanism, and because the
//! handles never change, every binding, every message route and every open
//! gesture survives the move without knowing it happened.
//!
//! Two consequences worth stating, because they are what the design bought:
//!
//! * A floating panel is not a lesser copy. The floating Outliner has the
//!   filters, the context menu and the drag-and-drop the docked one has,
//!   because it *is* the docked one.
//! * The dock closes the gap by itself. A splitter with one child left gives
//!   that child the column, so nothing has to remember a size to collapse.

/// Which panel a floating window is showing.
///
/// An enum rather than a [`crate::message::NodeHandle`], because the window
/// outlives any particular layout: it is closed and reopened, and what it has
/// to remember across that is *which panel it is*.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FloatingKind {
    /// The Outliner's entity tree.
    Outliner,
    /// The Details panel, schema-generated rows and all.
    Details,
    /// The 3D viewport, its context bar and its overlays.
    Viewport,
    /// The Output Log.
    OutputLog,
}

impl FloatingKind {
    /// Every panel that can float, in menu order.
    pub const ALL: [Self; 4] = [
        Self::Outliner,
        Self::Details,
        Self::Viewport,
        Self::OutputLog,
    ];

    /// The panel `SOMNIUM_FLOAT` asks to open at startup, if it asks for one.
    ///
    /// A window only a menu can open is a window no automated run can look at,
    /// and this one has a GPU surface of its own.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("SOMNIUM_FLOAT").ok()?.trim() {
            "log" => Some(Self::OutputLog),
            "outliner" => Some(Self::Outliner),
            "details" => Some(Self::Details),
            "viewport" => Some(Self::Viewport),
            other => {
                tracing::warn!("SOMNIUM_FLOAT={other} is not a panel name; ignoring");
                None
            }
        }
    }

    /// The name as it reads in a menu.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OutputLog => "Output Log",
            Self::Outliner => "Outliner",
            Self::Details => "Details",
            Self::Viewport => "Viewport",
        }
    }

    /// The window title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::OutputLog => "Output Log - Somnium",
            Self::Outliner => "Outliner - Somnium",
            Self::Details => "Details - Somnium",
            Self::Viewport => "Viewport - Somnium",
        }
    }

    /// The size the window opens at, in logical pixels.
    #[must_use]
    pub const fn default_size(self) -> (u32, u32) {
        match self {
            // Wide and short: a log is read a line at a time and the lines are
            // long. Tall and narrow: an outliner is a list of short names, and
            // Details is a column of rows. The viewport is the one that wants
            // area, because it is the thing being looked at.
            Self::OutputLog => (900, 420),
            Self::Outliner => (360, 720),
            Self::Details => (400, 800),
            Self::Viewport => (1280, 760),
        }
    }

    /// Whether this window needs the renderer to draw a scene into it.
    ///
    /// True for exactly one panel, and the reason it is a method rather than an
    /// `== Viewport` at the call site: the host has to acquire, record and
    /// present a second scene surface, and that branch should be named for what
    /// it is instead of for which variant happens to want it.
    #[must_use]
    pub const fn hosts_scene(self) -> bool {
        matches!(self, Self::Viewport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_floatable_panel_has_a_name_and_a_size() {
        // `ALL` is what the Window menu is written against, so a variant
        // missing from it is a panel nobody can float.
        assert_eq!(FloatingKind::ALL.len(), 4);
        for kind in FloatingKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(kind.title().contains(kind.label()));
            let (w, h) = kind.default_size();
            assert!(w > 100 && h > 100, "{kind:?} opens at {w}x{h}");
        }
    }

    #[test]
    fn only_the_viewport_asks_the_renderer_for_anything() {
        let scene: Vec<_> = FloatingKind::ALL
            .into_iter()
            .filter(|kind| kind.hosts_scene())
            .collect();
        assert_eq!(scene, vec![FloatingKind::Viewport]);
    }
}
