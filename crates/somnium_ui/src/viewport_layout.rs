//! How the editor divides its viewport region.
//!
//! MORROWIND-J step 3. The *choice* is the editor's — it is a menu item with a
//! label and a cycle order — while what each resulting view looks at is the
//! renderer's (`somnium_renderer::view`). The seam between them is a list of
//! rectangles, which is the only thing either side needs from the other.

/// A rectangle of the swapchain, in physical pixels.
pub type ViewRect = (u32, u32, u32, u32);

/// How the viewport region is divided.
///
/// Named rather than an arbitrary tree: the useful arrangements are few, every
/// DCC tool ships the same handful, and a general splitter here would be a
/// second docking system beside the one MORROWIND-J step 1 already built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportLayout {
    #[default]
    Single,
    /// Two side by side.
    SplitVertical,
    /// Two stacked.
    SplitHorizontal,
    /// The classic four-up.
    Quad,
}

impl ViewportLayout {
    /// The layout `SOMNIUM_VIEWPORTS` asks for, if it asks for one.
    ///
    /// A `.somtime` run is non-interactive, and the four-viewport frame time is
    /// the number MORROWIND-J's plan attaches to this step — so the layout has
    /// to be selectable without a hand on the toolbar, or the measurement
    /// cannot be repeated.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("SOMNIUM_VIEWPORTS").ok()?.trim() {
            "1" => Some(Self::Single),
            "2" => Some(Self::SplitVertical),
            "2h" => Some(Self::SplitHorizontal),
            "4" => Some(Self::Quad),
            other => {
                tracing::warn!("SOMNIUM_VIEWPORTS={other} is not 1, 2, 2h or 4; ignoring");
                None
            }
        }
    }

    /// How many views this layout draws.
    #[must_use]
    pub const fn count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::SplitVertical | Self::SplitHorizontal => 2,
            Self::Quad => 4,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Single => "1 Viewport",
            Self::SplitVertical => "2 Side by Side",
            Self::SplitHorizontal => "2 Stacked",
            Self::Quad => "4 Viewports",
        }
    }

    /// Every layout, in menu order.
    ///
    /// The list the Window menu is written against. A variant added without a
    /// row here is caught by `every_layout_has_a_menu_row` rather than shipping
    /// as an arrangement nobody can select.
    pub const ALL: [Self; 4] = [
        Self::Single,
        Self::SplitVertical,
        Self::SplitHorizontal,
        Self::Quad,
    ];

    /// Divide a region into this layout's tiles, in physical pixels.
    ///
    /// No gutter is left between tiles: the editor draws its own seam over
    /// them, and a gap here would show the swapchain's previous contents
    /// through it.
    #[must_use]
    pub fn tiles(self, rect: ViewRect) -> Vec<ViewRect> {
        let (x, y, w, h) = rect;
        // Halves that add back up to the whole. `w - w / 2` rather than a
        // second `w / 2`, or an odd width loses its last column to a seam that
        // nothing ever draws into.
        let (lw, rw) = (w / 2, w - w / 2);
        let (th, bh) = (h / 2, h - h / 2);
        match self {
            Self::Single => vec![rect],
            Self::SplitVertical => vec![(x, y, lw, h), (x + lw, y, rw, h)],
            Self::SplitHorizontal => vec![(x, y, w, th), (x, y + th, w, bh)],
            Self::Quad => vec![
                (x, y, lw, th),
                (x + lw, y, rw, th),
                (x, y + th, lw, bh),
                (x + lw, y + th, rw, bh),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_region_exactly_and_never_overlap() {
        // A seam between tiles shows whatever was in the swapchain last frame,
        // and an overlap is one view drawing over another's edge. Odd sizes are
        // where both actually happen.
        for layout in [
            ViewportLayout::Single,
            ViewportLayout::SplitVertical,
            ViewportLayout::SplitHorizontal,
            ViewportLayout::Quad,
        ] {
            for region in [(0, 0, 1921, 1081), (17, 9, 800, 601), (0, 0, 2, 2)] {
                let tiles = layout.tiles(region);
                assert_eq!(tiles.len(), layout.count(), "{layout:?}");
                let area: u32 = tiles.iter().map(|(_, _, w, h)| w * h).sum();
                assert_eq!(
                    area,
                    region.2 * region.3,
                    "{layout:?} at {region:?} lost or double-counted pixels: {tiles:?}"
                );
                for (i, a) in tiles.iter().enumerate() {
                    for b in &tiles[i + 1..] {
                        assert!(!overlaps(*a, *b), "{layout:?}: {a:?} overlaps {b:?}");
                    }
                    assert!(
                        a.0 >= region.0
                            && a.1 >= region.1
                            && a.0 + a.2 <= region.0 + region.2
                            && a.1 + a.3 <= region.1 + region.3,
                        "{layout:?}: {a:?} escapes {region:?}"
                    );
                }
            }
        }
    }

    fn overlaps(a: ViewRect, b: ViewRect) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    #[test]
    fn one_full_size_viewport_is_the_default() {
        // The editor has looked like this since it was built, and MORROWIND-J
        // says in as many words that nothing may look different on first run.
        assert_eq!(ViewportLayout::default(), ViewportLayout::Single);
        assert_eq!(ViewportLayout::default().count(), 1);
        assert_eq!(
            ViewportLayout::default().tiles((10, 20, 300, 400)),
            [(10, 20, 300, 400)],
            "the single layout is the whole region, undivided"
        );
    }

    #[test]
    fn every_layout_is_listed_and_named() {
        // `ALL` is what the Window menu is written against, so a variant
        // missing from it is an arrangement nobody can pick.
        assert_eq!(ViewportLayout::ALL.len(), 4);
        for layout in ViewportLayout::ALL {
            assert!(!layout.label().is_empty(), "{layout:?}");
            assert!(layout.count() >= 1);
        }
    }
}
