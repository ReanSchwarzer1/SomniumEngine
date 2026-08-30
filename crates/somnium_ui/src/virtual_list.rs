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

/// The slice of a uniform grid that intersects a clip rectangle.
///
/// The content drawer's problem rather than the outliner's. A list windows on
/// one axis; a grid of tiles wraps, so the window is *rows of columns* and the
/// caller has to say how many columns the panel fits.
///
/// Kept beside [`RowWindow`] rather than folded into it: the two take different
/// inputs and a single function with a `columns: Option<usize>` would make
/// every list caller answer a question about grids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridWindow {
    /// First tile index to materialise.
    pub first: usize,
    /// How many tiles from `first`.
    pub count: usize,
    /// Tiles per row, as used to compute the window.
    pub columns: usize,
    /// Rows the whole grid needs, for the scrollable height.
    ///
    /// **This is what keeps the scrollbar honest.** A virtualised view that
    /// only ever builds thirty widgets must still report the height of all
    /// hundred thousand, or the scrollbar says the list is one screen long.
    pub total_rows: usize,
}

impl GridWindow {
    /// The tiles of `total` that intersect `clip`, given a tile size and the
    /// width available for a row.
    #[must_use]
    pub fn new(
        content_top: f32,
        tile: (f32, f32),
        available_w: f32,
        total: usize,
        clip: Rect,
    ) -> Self {
        let (tile_w, tile_h) = tile;
        let columns = if tile_w > 0.0 {
            ((available_w / tile_w).floor() as usize).max(1)
        } else {
            1
        };
        let total_rows = total.div_ceil(columns);
        if total == 0 || tile_h <= 0.0 || clip.h <= 0.0 || clip.w <= 0.0 {
            return Self {
                first: 0,
                count: 0,
                columns,
                total_rows,
            };
        }
        let rows = RowWindow::new(content_top, tile_h, total_rows, clip);
        // `min(total)` because the last row is usually short: a window of two
        // rows at four columns is eight tiles even when only five exist, and
        // indexing the other three is how a virtualised grid panics.
        let first = rows.first * columns;
        let last = ((rows.first + rows.count) * columns).min(total);
        Self {
            first,
            count: last.saturating_sub(first),
            columns,
            total_rows,
        }
    }

    /// The half-open range to iterate.
    #[must_use]
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.count
    }

    /// Where tile `index` sits, relative to the grid's own origin.
    #[must_use]
    pub fn tile_rect(&self, index: usize, tile: (f32, f32)) -> Rect {
        let (tile_w, tile_h) = tile;
        let column = index % self.columns.max(1);
        let row = index / self.columns.max(1);
        Rect::new(column as f32 * tile_w, row as f32 * tile_h, tile_w, tile_h)
    }

    /// The height the grid must report so the scrollbar is right.
    #[must_use]
    pub fn content_height(&self, tile_h: f32) -> f32 {
        self.total_rows as f32 * tile_h
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

    // ── The grid the content drawer needs ──────────────────────────────────

    const TILE: (f32, f32) = (96.0, 116.0);

    #[test]
    fn a_grid_window_is_bounded_by_the_viewport_and_not_the_asset_count() {
        // The drawer's half of MORROWIND-M's acceptance. Ten tiles across, six
        // rows visible, so the window is bounded no matter how many assets the
        // project has.
        let view = clip(0.0, 700.0);
        let mut counts = Vec::new();
        for total in [40usize, 40_000, 4_000_000] {
            let window = GridWindow::new(0.0, TILE, 960.0, total, view);
            assert_eq!(window.columns, 10);
            counts.push(window.count);
        }
        assert!(
            counts[1] == counts[2],
            "the window grew with the total: {counts:?}"
        );
        assert!(counts[1] <= 10 * (7 + 2 * OVERSCAN), "{}", counts[1]);
    }

    #[test]
    fn the_last_row_is_short_and_that_does_not_run_off_the_end() {
        // Five tiles in a four-column grid is two rows, the second holding one.
        // A window that returned eight indices here would panic the caller on
        // the three that do not exist.
        let window = GridWindow::new(0.0, TILE, 400.0, 5, clip(0.0, 700.0));
        assert_eq!(window.columns, 4);
        assert_eq!(window.first, 0);
        assert_eq!(window.count, 5, "{window:?}");
        assert_eq!(window.range().end, 5);
    }

    #[test]
    fn the_scrollable_height_counts_every_row_not_the_visible_ones() {
        // The bug a virtualised grid ships with if nobody thinks about it: the
        // scrollbar says the list is one screen long because only one screen of
        // widgets exists.
        let window = GridWindow::new(0.0, TILE, 400.0, 4_000, clip(0.0, 700.0));
        assert_eq!(window.total_rows, 1_000);
        assert!((window.content_height(TILE.1) - 116_000.0).abs() < 0.5);
        assert!(window.count < 100, "still only a screenful is built");
    }

    #[test]
    fn tiles_are_placed_in_reading_order() {
        let window = GridWindow::new(0.0, TILE, 400.0, 12, clip(0.0, 700.0));
        assert_eq!(window.tile_rect(0, TILE), Rect::new(0.0, 0.0, 96.0, 116.0));
        assert_eq!(
            window.tile_rect(3, TILE),
            Rect::new(288.0, 0.0, 96.0, 116.0)
        );
        // Index 4 wraps to the start of the second row.
        assert_eq!(
            window.tile_rect(4, TILE),
            Rect::new(0.0, 116.0, 96.0, 116.0)
        );
    }

    #[test]
    fn a_panel_narrower_than_one_tile_still_shows_one_column() {
        // Dragging the drawer narrow must not divide by zero columns.
        let window = GridWindow::new(0.0, TILE, 10.0, 20, clip(0.0, 700.0));
        assert_eq!(window.columns, 1);
        assert_eq!(window.total_rows, 20);
    }

    #[test]
    fn scrolling_a_grid_moves_the_window_without_changing_its_size() {
        let view = clip(0.0, 700.0);
        let top = GridWindow::new(0.0, TILE, 400.0, 4_000, view);
        let deep = GridWindow::new(-TILE.1 * 500.0, TILE, 400.0, 4_000, view);
        assert!(deep.first > top.first, "the window did not move");
        // One row of difference, and only one: at scroll zero there is no row
        // above the clip for the top overscan to reach.
        assert_eq!(deep.count - top.count, top.columns, "{top:?} vs {deep:?}");
        assert!(deep.range().end <= 4_000);
    }

    #[test]
    fn a_window_scrolled_past_the_end_is_still_a_range_you_can_slice() {
        // Walk into a folder of 40,000 assets, scroll to the bottom, then walk
        // into one with six. The scroll offset does not reset, so the window is
        // asked about content that is no longer there. `first` past the end
        // with a count of zero is still an out-of-bounds slice, and the drawer
        // slices its entry list with exactly this range.
        let assets: Vec<usize> = (0..6).collect();
        let window = GridWindow::new(-500_000.0, TILE, 400.0, assets.len(), clip(0.0, 700.0));
        assert!(window.is_empty());
        let range = window.range();
        assert!(
            range.end <= assets.len() && range.start <= range.end,
            "{range:?} would panic slicing {} entries",
            assets.len()
        );
        let _ = &assets[range];
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
