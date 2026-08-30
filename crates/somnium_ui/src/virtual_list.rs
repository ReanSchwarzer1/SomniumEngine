//! Windowing for long uniform-height lists (MORROWIND-M, step 1).
//!
//! One question: *given a clip rectangle, which rows can be seen?* Everything
//! that reads a long list — the outliner, the content drawer, the asset browser
//! — asks it, and until now none of them did.
//!
//! # The ceiling this exists to remove
//!
//! `TreeView::draw` iterated **every** item and emitted primitives for each,
//! label shaping included, whether or not the row was on screen. A widget in a
//! scroll viewer is as tall as its content, so a hundred thousand entities meant
//! a hundred thousand rows shaped and painted every frame to show the thirty
//! that fit. The cost was O(total rows) where the visible work is O(viewport ÷
//! row height) — about thirty — and no amount of GPU makes that difference up.
//!
//! MORROWIND-M's acceptance is *"100,000 rows at 60 fps"*. The property that
//! makes it reachable is that the work per frame stops depending on the total,
//! and [`RowWindow`] is where that property lives:
//! [`bounded_by_the_viewport_and_not_the_list`] is the test that says so.
//!
//! # Why a module and not four `if` statements
//!
//! Three panels need the same arithmetic and each would get it subtly wrong in
//! its own way — an off-by-one at the bottom edge that clips the last row, a
//! missing overscan that flickers on scroll, a negative index when the content
//! is scrolled above its clip. Those are the three bugs this module's tests
//! are about, and they are worth having once.
//!
//! [`bounded_by_the_viewport_and_not_the_list`]: tests::bounded_by_the_viewport_and_not_the_list

use crate::types::Rect;

/// Rows drawn beyond each edge of the clip.
///
/// Zero would be correct and would flicker: a scroll offset lands mid-row far
/// more often than not, and a row that is one pixel inside the clip has to be
/// painted or its top edge is a gap. One row of margin each way costs two rows
/// of work and removes the class entirely.
pub const OVERSCAN: usize = 1;

/// The slice of a uniform-height list that intersects a clip rectangle.
///
/// Half-open: `first..first + count`. Empty when the list is scrolled entirely
/// out of view, which is a real state — a collapsed panel, or a list below the
/// fold of a tall scroll viewer — and not an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowWindow {
    /// First row to materialise.
    pub first: usize,
    /// How many rows from `first`.
    pub count: usize,
}

impl RowWindow {
    /// The rows of `total` that intersect `clip`.
    ///
    /// `content_top` is where row zero starts in the same space as `clip`,
    /// which for a widget inside a scroll viewer is its screen bounds' `y` —
    /// already shifted by the scroll offset. That is deliberate: this module
    /// never learns what scrolling is, so it cannot disagree with the scroll
    /// viewer about it.
    #[must_use]
    pub fn new(content_top: f32, row_height: f32, total: usize, clip: Rect) -> Self {
        if total == 0 || row_height <= 0.0 || clip.h <= 0.0 || clip.w <= 0.0 {
            return Self::EMPTY;
        }
        // Rows entirely above the clip. `max(0.0)` because content scrolled
        // *below* its clip gives a negative index, and `as usize` on a negative
        // float is zero on some paths and a very large number on others — a
        // difference nobody should have to remember.
        let above = ((clip.y - content_top) / row_height).floor().max(0.0) as usize;
        let first = above.saturating_sub(OVERSCAN);

        let bottom = clip.y + clip.h;
        if bottom <= content_top {
            return Self::EMPTY;
        }
        // `ceil` so a row poking one pixel into the clip is included, and the
        // overscan row after it as well.
        let below = ((bottom - content_top) / row_height).ceil().max(0.0) as usize;
        let last = below.saturating_add(OVERSCAN).min(total);
        if first >= last {
            return Self::EMPTY;
        }
        Self {
            first,
            count: last - first,
        }
    }

    /// Nothing visible.
    pub const EMPTY: Self = Self { first: 0, count: 0 };

    /// The half-open range to iterate.
    #[must_use]
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.count
    }

    /// Whether a row index is inside the window.
    #[must_use]
    pub fn contains(&self, row: usize) -> bool {
        row >= self.first && row < self.first + self.count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// A selection held by key rather than by row index.
///
/// **This is what "stable selection across scroll" means.** An index is a
/// position in a list that filtering, sorting, expanding a parent and scrolling
/// all renumber; a key is the thing the user actually picked. Storing indices
/// is why a selection appears to jump when a list changes underneath it.
///
/// Sorted, so membership is a binary search. The tree view asked
/// `selected_set.contains(&id)` once per row against a `Vec`, which is
/// O(rows × selected) per frame — invisible at ten rows and quadratic at a
/// hundred thousand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeySelection {
    keys: Vec<u32>,
}

impl KeySelection {
    /// Build from any order, with duplicates removed.
    #[must_use]
    pub fn from_keys(keys: impl IntoIterator<Item = u32>) -> Self {
        let mut keys: Vec<u32> = keys.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        Self { keys }
    }

    /// Whether a key is selected. `O(log n)`.
    #[must_use]
    pub fn contains(&self, key: u32) -> bool {
        self.keys.binary_search(&key).is_ok()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// The selected keys, ascending.
    #[must_use]
    pub fn keys(&self) -> &[u32] {
        &self.keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 22.0;

    fn clip(y: f32, h: f32) -> Rect {
        Rect::new(0.0, y, 300.0, h)
    }

    // ── The property the acceptance criterion rests on ─────────────────────

    #[test]
    fn bounded_by_the_viewport_and_not_the_list() {
        // MORROWIND-M's acceptance is 100,000 rows at 60 fps. The only way that
        // is reachable is if the per-frame work stops depending on the total,
        // so this asserts the shape of the cost rather than a frame time: the
        // same viewport materialises the same handful of rows whether the list
        // holds a hundred rows or a hundred million.
        let view = clip(0.0, 660.0); // 30 rows of 22 px
        let mut counts = Vec::new();
        for total in [100usize, 100_000, 1_000_000, 100_000_000] {
            let window = RowWindow::new(0.0, ROW, total, view);
            counts.push(window.count);
            assert!(
                window.count <= 30 + 2 * OVERSCAN + 1,
                "{total} rows materialised {}",
                window.count
            );
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "the window changed with the total: {counts:?}"
        );
    }

    #[test]
    fn a_taller_viewport_is_the_only_thing_that_widens_the_window() {
        let short = RowWindow::new(0.0, ROW, 100_000, clip(0.0, 220.0));
        let tall = RowWindow::new(0.0, ROW, 100_000, clip(0.0, 880.0));
        assert!(tall.count > short.count);
        assert_eq!(short.first, tall.first);
    }

    // ── The three bugs three panels would each write for themselves ────────

    #[test]
    fn the_row_straddling_the_bottom_edge_is_included() {
        // A clip 30.5 rows tall must paint 31 rows, not 30, or the last one is
        // a gap where its top half should be.
        let view = clip(0.0, ROW * 30.5);
        let window = RowWindow::new(0.0, ROW, 1000, view);
        assert!(
            window.contains(30),
            "row 30 straddles the edge and must be drawn: {window:?}"
        );
    }

    #[test]
    fn content_scrolled_above_its_clip_never_produces_a_negative_index() {
        // A scroll viewer places its content at a negative offset. `as usize`
        // on a negative float is a trap, and this is the case that springs it.
        for top in [-1.0, -ROW * 0.5, -ROW * 4000.0] {
            let window = RowWindow::new(top, ROW, 100_000, clip(0.0, 660.0));
            assert!(window.first < 100_000, "top {top}: first {}", window.first);
            assert!(
                window.first + window.count <= 100_000,
                "top {top}: ran past the end"
            );
        }
    }

    #[test]
    fn a_window_never_runs_past_the_end_of_the_list() {
        // Scrolled to the very bottom, the overscan must clamp rather than
        // index a row that is not there.
        let total = 50;
        let window = RowWindow::new(-(ROW * (total as f32 - 5.0)), ROW, total, clip(0.0, 660.0));
        assert!(window.first + window.count <= total, "{window:?}");
        assert!(window.contains(total - 1), "the last row must be visible");
    }

    #[test]
    fn overscan_covers_the_row_the_scroll_offset_landed_inside() {
        // Scrolled half a row down: the row above the clip is partly visible
        // and must be painted.
        let window = RowWindow::new(-ROW * 10.5, ROW, 1000, clip(0.0, 660.0));
        assert!(window.contains(10), "the half-scrolled row: {window:?}");
    }

    // ── Degenerate inputs are states, not errors ───────────────────────────

    #[test]
    fn an_empty_or_invisible_list_produces_an_empty_window() {
        assert!(RowWindow::new(0.0, ROW, 0, clip(0.0, 660.0)).is_empty());
        assert!(RowWindow::new(0.0, 0.0, 100, clip(0.0, 660.0)).is_empty());
        assert!(RowWindow::new(0.0, ROW, 100, clip(0.0, 0.0)).is_empty());
        // Scrolled entirely past the bottom of its clip.
        assert!(RowWindow::new(700.0, ROW, 100, clip(0.0, 660.0)).is_empty());
        // The empty window must still be safe to iterate.
        assert_eq!(RowWindow::EMPTY.range().count(), 0);
    }

    // ── Selection ──────────────────────────────────────────────────────────

    #[test]
    fn a_selection_is_unmoved_by_scrolling() {
        // The whole point of holding keys: the window changes, the selection
        // does not. An index-based selection would have to be renumbered here,
        // and that renumbering is the bug this type exists to make impossible.
        let selection = KeySelection::from_keys([7, 4001, 99_998]);
        for top in [0.0, -ROW * 100.0, -ROW * 90_000.0] {
            let window = RowWindow::new(top, ROW, 100_000, clip(0.0, 660.0));
            let _ = window;
            assert!(selection.contains(4001));
            assert_eq!(selection.len(), 3);
        }
    }

    #[test]
    fn a_selection_sorts_and_deduplicates_what_it_is_given() {
        let selection = KeySelection::from_keys([9, 1, 9, 5, 1]);
        assert_eq!(selection.keys(), &[1, 5, 9]);
        assert!(selection.contains(5));
        assert!(!selection.contains(6));
        assert!(KeySelection::default().is_empty());
    }
}
