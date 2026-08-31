//! A panel living in its own OS window.
//!
//! MORROWIND-J step 2. Step 1 built a dock tree that can express where a panel
//! sits; this is the case it cannot express, because the panel is not in the
//! window at all.
//!
//! # What a floating panel actually needs
//!
//! Not a widget. A second OS window is a second **surface**, so the host owns a
//! second window and a second surface. It owns nothing else:
//!
//! ```text
//!   main window          floating window
//!   ───────────          ───────────────
//!   winit::Window        winit::Window        ┐ the host's (somnium_core)
//!   wgpu::Surface        wgpu::Surface        ┘
//!         ╲                    ╱
//!          ╲                  ╱
//!     one UserInterface, one DrawingContext, one UiPass
//!          root ── … ── dock          detached ── DETAILS
//! ```
//!
//! One [`crate::pass::UiPass`], and that is not thrift. A pass owns the GPU
//! copy of the font atlas, the icon atlas, the thumbnail atlas and every
//! registered texture, and each upload is guarded by a dirty flag that the
//! first pass to prepare clears. A second pass therefore draws against blank
//! atlases: panels, sliders and check boxes appear, and not one glyph or icon
//! does. It looked exactly like a font that had failed to load.
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

    /// The panels `SOMNIUM_FLOAT` asks to open at startup.
    ///
    /// A window only a menu can open is a window no automated run can look at,
    /// and these have GPU surfaces of their own. Comma-separated, because the
    /// interesting case is more than one: every window draws through the same
    /// pass, and one window can only ever prove that it does not conflict with
    /// the editor.
    #[must_use]
    pub fn from_env() -> Vec<Self> {
        let Ok(raw) = std::env::var("SOMNIUM_FLOAT") else {
            return Vec::new();
        };
        raw.split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .filter_map(|name| {
                let found = Self::from_slug(name);
                if found.is_none() {
                    tracing::warn!("SOMNIUM_FLOAT={name} is not a panel name; ignoring");
                }
                found
            })
            .collect()
    }

    /// The panel a slug names, if this build has one.
    ///
    /// The inverse of [`Self::slug`], and the reason a layout file stores names
    /// rather than indices: a build that adds a floatable panel can still read
    /// a file written by one that did not, and an unknown name is a panel that
    /// does not exist rather than the wrong panel opening.
    #[must_use]
    pub fn from_slug(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.slug() == name)
    }

    /// The panel's name in an environment variable and in a file name.
    ///
    /// One table rather than two: the name `SOMNIUM_FLOAT` accepts is the name
    /// `SOMNIUM_FLOAT_PNG` writes, so a run that asked for four windows comes
    /// back with four files that say which is which.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::OutputLog => "log",
            Self::Outliner => "outliner",
            Self::Details => "details",
            Self::Viewport => "viewport",
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
            // Wide and short: a log is read a line at a time, the lines are
            // long, and its toolbar is nine controls that a narrower window
            // would truncate. Tall and narrow: an outliner is a list of short
            // names, and Details is a column of rows. The viewport is the one
            // that wants area, because it is the thing being looked at.
            Self::OutputLog => (1120, 460),
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
    fn a_slug_round_trips_and_an_unknown_one_is_nobody() {
        for kind in FloatingKind::ALL {
            assert_eq!(FloatingKind::from_slug(kind.slug()), Some(kind));
        }
        // What a layout file from a later build looks like to this one.
        assert_eq!(FloatingKind::from_slug("timeline"), None);
        assert_eq!(FloatingKind::from_slug(""), None);
        assert_eq!(FloatingKind::from_slug("Outliner"), None, "slugs are exact");
    }

    #[test]
    fn every_panel_answers_to_its_own_slug() {
        // `from_env` is written against `slug`, so a variant whose slug
        // collided with another would silently open the wrong window.
        for kind in FloatingKind::ALL {
            assert!(!kind.slug().is_empty());
            let same: Vec<_> = FloatingKind::ALL
                .into_iter()
                .filter(|other| other.slug() == kind.slug())
                .collect();
            assert_eq!(same, vec![kind], "{:?} shares a slug", kind);
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
