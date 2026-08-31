//! The grid that draws a [`crate::data_table::DataTable`].
//!
//! MORROWIND-M item 2, second half. The model is what a table *is* — typed
//! columns, keyed rows, sorting, filtering, rectangular edits, CSV — and it is
//! tested without a pixel. This is the projection, and it owns three things the
//! model deliberately does not have:
//!
//! - **where the cells are**, which is arithmetic over a clip rectangle,
//! - **which cell is being typed into**, which is a text buffer and a caret,
//! - **the frozen header**, which is the one piece of chrome a grid cannot do
//!   without and the one thing a plain scroll viewer will not give it.
//!
//! One widget, not one per cell. A ten-thousand-row catalogue is ten thousand
//! rows of *data*, not ten thousand widgets, and the draw is windowed against
//! the clip exactly as [`crate::widgets::tree_view::TreeView`]'s is.

use crate::{
    data_table::{Cell, CellError, ColumnId, DataTable, RowId, SortOrder, View},
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    typography::{TextRole, text_style},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

/// Height of one body row, and of the header.
pub const ROW_H: f32 = 22.0;
/// Narrowest a column may be drawn before the grid simply overflows its panel.
/// Below this the header title is unreadable and the cell is unusable.
pub const MIN_COLUMN_W: f32 = 90.0;

#[derive(Debug, Clone)]
pub enum DataGridMessage {
    /// Replace the table. Selection and edit state are dropped with it,
    /// because a `RowId` from one table means nothing in another.
    SetTable(Box<DataTable>),
    /// A committed edit, with the table it produced.
    ///
    /// The table travels with the message because the host cannot reach into
    /// the widget for it — and because the moment a commit lands is the only
    /// moment the table is in a state worth saving. A host that read it
    /// whenever it liked would eventually read one mid-keystroke.
    Edited {
        rows: usize,
        columns: usize,
        table: Box<DataTable>,
    },
    /// Sort or filter changed, so a host showing a count can update it.
    ViewChanged,
    SetFilter(String),
    /// Show only rows with an empty cell — the localisation question.
    SetOnlyIncomplete(bool),
}

/// A rectangular selection, held by key at both corners.
///
/// Positions would be the obvious thing to store and the wrong one: sorting
/// between the anchor click and the shift-click renumbers every row, and a
/// range that meant rows 4–9 would silently come to mean six different rows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    anchor: (RowId, ColumnId),
    focus: (RowId, ColumnId),
}

pub struct DataGrid {
    pub table: DataTable,
    pub view: View,
    pub font_id: u8,
    /// The view's row order, recomputed only when the table or view changes.
    ///
    /// `visible_rows` sorts and filters; doing that inside `draw` would put an
    /// O(n log n) pass in every frame of an idle panel.
    rows: Vec<RowId>,
    selection: Option<Selection>,
    /// The edit buffer. `Some` means the focused cell is being typed into, and
    /// the underlying cell is unchanged until it commits.
    editing: Option<String>,
    /// Why the last commit was refused, shown against the cell that refused it.
    error: Option<String>,
    hovered: Option<usize>,
}

impl DataGrid {
    #[must_use]
    pub fn new(table: DataTable, font_id: u8) -> Self {
        let view = View::default();
        let rows = table.visible_rows(&view);
        Self {
            table,
            view,
            font_id,
            rows,
            selection: None,
            editing: None,
            error: None,
            hovered: None,
        }
    }

    /// Re-derive the visible order. Called after anything that could change it.
    pub fn refresh(&mut self) {
        self.rows = self.table.visible_rows(&self.view);
        // A selection whose row was filtered out is not a selection any more.
        // Left in place it would be an invisible target for the next keystroke.
        if let Some(selection) = &self.selection
            && !self.rows.contains(&selection.focus.0)
        {
            self.selection = None;
            self.editing = None;
        }
    }

    /// The rows the view is showing, in view order.
    #[must_use]
    pub fn visible_rows(&self) -> &[RowId] {
        &self.rows
    }

    /// The cell the keyboard is on, if any.
    #[must_use]
    pub fn focused_cell(&self) -> Option<(RowId, ColumnId)> {
        self.selection.as_ref().map(|s| s.focus.clone())
    }

    /// Whether a cell is being typed into right now.
    #[must_use]
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// The message the last refused edit produced.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn column_width(&self, total: f32) -> f32 {
        let columns = self.table.columns().len().max(1) as f32;
        (total / columns).max(MIN_COLUMN_W)
    }

    /// Rows and columns covered by the selection, in view order.
    ///
    /// The rectangle between two keys, resolved through the *current* order —
    /// which is why the corners are keys and this is computed fresh.
    fn selected_range(&self) -> Option<(Vec<RowId>, Vec<ColumnId>)> {
        let selection = self.selection.as_ref()?;
        let row_of = |id: RowId| self.rows.iter().position(|r| *r == id);
        let col_of = |id: &ColumnId| self.table.columns().iter().position(|c| &c.id == id);
        let (a, b) = (row_of(selection.anchor.0)?, row_of(selection.focus.0)?);
        let (c, d) = (col_of(&selection.anchor.1)?, col_of(&selection.focus.1)?);
        let rows = self.rows[a.min(b)..=a.max(b)].to_vec();
        let columns = self.table.columns()[c.min(d)..=c.max(d)]
            .iter()
            .map(|column| column.id.clone())
            .collect();
        Some((rows, columns))
    }

    /// Write the buffer into every selected cell, or refuse the lot.
    fn commit(&mut self, emit: &mut Vec<UiMessage>, handle: crate::message::NodeHandle) {
        let Some(text) = self.editing.take() else {
            return;
        };
        let Some((rows, columns)) = self.selected_range() else {
            return;
        };
        match self.table.set_range(&rows, &columns, &text) {
            Ok(_) => {
                self.error = None;
                let (r, c) = (rows.len(), columns.len());
                // The sort may have been *by* the column just written, so the
                // row can move out from under the cursor. Re-deriving keeps
                // the selection pointing at the same row wherever it went.
                self.refresh();
                emit.push(UiMessage::new(
                    handle,
                    MessageDirection::FromWidget,
                    DataGridMessage::Edited {
                        rows: r,
                        columns: c,
                        table: Box::new(self.table.clone()),
                    },
                ));
            }
            Err(error) => {
                // Refused edits keep the buffer. Throwing away what somebody
                // typed because it did not parse is how a grid loses work.
                self.error = Some(error_text(&error));
                self.editing = Some(text);
            }
        }
    }

    /// Move the focus by whole cells, ending any edit first.
    fn step(&mut self, drow: isize, dcolumn: isize) {
        let columns = self.table.columns();
        if self.rows.is_empty() || columns.is_empty() {
            return;
        }
        let (row, column) = match self.selection.as_ref() {
            Some(selection) => selection.focus.clone(),
            None => (self.rows[0], columns[0].id.clone()),
        };
        let r = self.rows.iter().position(|id| *id == row).unwrap_or(0);
        let c = columns.iter().position(|def| def.id == column).unwrap_or(0);
        let r = (r as isize + drow).clamp(0, self.rows.len() as isize - 1) as usize;
        let c = (c as isize + dcolumn).clamp(0, columns.len() as isize - 1) as usize;
        let cell = (self.rows[r], columns[c].id.clone());
        self.selection = Some(Selection {
            anchor: cell.clone(),
            focus: cell,
        });
        self.editing = None;
        self.error = None;
    }

    /// Cycle a column between ascending, descending and unsorted.
    ///
    /// Three states rather than two: insertion order is the order a catalogue
    /// was authored in, and there has to be a way back to it.
    fn cycle_sort(&mut self, column: &ColumnId) {
        self.view.sort = match self.view.sort.take() {
            Some((id, SortOrder::Ascending)) if &id == column => Some((id, SortOrder::Descending)),
            Some((id, SortOrder::Descending)) if &id == column => None,
            _ => Some((column.clone(), SortOrder::Ascending)),
        };
        self.refresh();
    }
}

fn error_text(error: &CellError) -> String {
    format!("{error}")
}

impl Control for DataGrid {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Table
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    /// With a cell selected, the keyboard belongs here.
    ///
    /// The same rule the curve editor follows, and for the same reason in
    /// reverse: a focused widget that reports true swallows every key before
    /// the game sees it. Without that, typing `w` into a cell would also switch
    /// the gizmo to Translate and `Delete` would remove the selected *entity* —
    /// while a grid nobody has clicked a cell in would go on eating the
    /// fly-cam's WASD, which presents as the camera simply not responding.
    fn is_text_input(&self) -> bool {
        self.selection.is_some()
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        // As tall as every row plus the header. Inside a scroll viewer this is
        // what makes the scrollbar honest about a table whose rows below the
        // fold are never drawn.
        Vec2::new(available.x, ROW_H + self.rows.len() as f32 * ROW_H)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let clip = ctx.clip_rect();
        let t = theme::active();
        let columns = self.table.columns();
        if columns.is_empty() {
            return;
        }
        let column_w = self.column_width(b.w);
        let body_top = b.y + ROW_H;

        // ── Body ────────────────────────────────────────────────────────────
        //
        // Windowed against the clip, so a ten-thousand-row catalogue costs what
        // the thirty visible rows cost. The header is drawn after the rows so
        // it paints over the one it overlaps when scrolled.
        let window = crate::virtual_list::RowWindow::new(body_top, ROW_H, self.rows.len(), clip);
        let selected = self.selected_range();
        let body_style = text_style(TextRole::Body);
        for i in window.range() {
            let row_id = self.rows[i];
            let y = body_top + i as f32 * ROW_H;
            let row_rect = Rect::new(b.x, y, b.w, ROW_H);
            // Zebra striping, not a border grid: at 22 px rows a full grid of
            // lines is louder than the data in it.
            if i % 2 == 1 {
                ctx.push_rect_filled(row_rect, t.semantic.surface.raised.bytes());
            }
            if self.hovered == Some(i) {
                ctx.push_rect_filled(row_rect, t.semantic.surface.hover.bytes());
            }
            for (c, column) in columns.iter().enumerate() {
                let x = b.x + c as f32 * column_w;
                let cell_rect = Rect::new(x, y, column_w, ROW_H);
                let in_range = selected.as_ref().is_some_and(|(rows, cols)| {
                    rows.contains(&row_id) && cols.contains(&column.id)
                });
                let focused = self
                    .selection
                    .as_ref()
                    .is_some_and(|s| s.focus.0 == row_id && s.focus.1 == column.id);
                if in_range {
                    ctx.push_rect_filled(cell_rect, t.semantic.accent.selected_bg.bytes());
                }
                if focused {
                    ctx.push_rect_border(cell_rect, 1.0, t.semantic.accent.default.bytes());
                }

                let cell = self.table.get(row_id, &column.id);
                // The edit buffer, not the cell, while typing: the cell is
                // unchanged until the edit commits, and showing the stored
                // value under a caret is how a grid appears to lose keystrokes.
                let (text, colour) = match (&self.editing, focused) {
                    (Some(buffer), true) => (format!("{buffer}|"), t.semantic.text.primary.bytes()),
                    _ if cell.is_empty() => {
                        // An empty cell is *shown* as empty rather than as an
                        // empty string, because the difference is the entire
                        // point of the localisation table.
                        ("—".to_string(), t.semantic.text.disabled.bytes())
                    }
                    _ => (cell.display(), t.semantic.text.primary.bytes()),
                };
                ctx.push_text(
                    &text,
                    Vec2::new(x + 6.0, y + (ROW_H - body_style.px) * 0.5 - 1.0),
                    self.font_id,
                    body_style.px,
                    colour,
                );
            }
        }

        // ── Frozen header ───────────────────────────────────────────────────
        //
        // Drawn at the top of the *clip*, not of the widget: inside a scroll
        // viewer the widget's own top scrolls away, and a header that goes with
        // it leaves a grid of unlabelled columns. At scroll zero the clip top
        // and the widget top are the same, so the header sits in the space the
        // measure reserved for it.
        let header_y = clip.y.max(b.y);
        let header = Rect::new(b.x, header_y, b.w, ROW_H);
        ctx.push_rect_filled(header, t.semantic.surface.header.bytes());
        ctx.push_rect_filled(
            Rect::new(b.x, header_y + ROW_H - 1.0, b.w, 1.0),
            t.semantic.border.subtle.bytes(),
        );
        let header_style = text_style(TextRole::SectionCaps);
        for (c, column) in columns.iter().enumerate() {
            let x = b.x + c as f32 * column_w;
            let sorted = self
                .view
                .sort
                .as_ref()
                .filter(|(id, _)| id == &column.id)
                .map(|(_, order)| *order);
            let title = match sorted {
                Some(SortOrder::Ascending) => format!("{} ^", column.title),
                Some(SortOrder::Descending) => format!("{} v", column.title),
                None => column.title.clone(),
            };
            ctx.push_text(
                &title,
                Vec2::new(x + 6.0, header_y + (ROW_H - header_style.px) * 0.5 - 1.0),
                self.font_id,
                header_style.px,
                t.semantic.text.secondary.bytes(),
            );
            if c > 0 {
                ctx.push_rect_filled(
                    Rect::new(x, header_y, 1.0, ROW_H),
                    t.semantic.border.subtle.bytes(),
                );
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(message) = msg.data::<DataGridMessage>() {
            match message {
                DataGridMessage::SetTable(table) => {
                    self.table = (**table).clone();
                    self.selection = None;
                    self.editing = None;
                    self.error = None;
                    self.refresh();
                    widget.invalidate_layout();
                }
                DataGridMessage::SetFilter(filter) => {
                    self.view.filter = filter.clone();
                    self.refresh();
                    widget.invalidate_layout();
                }
                DataGridMessage::SetOnlyIncomplete(only) => {
                    self.view.only_incomplete = *only;
                    self.refresh();
                    widget.invalidate_layout();
                }
                _ => {}
            }
            msg.handled = true;
            return;
        }

        let b = widget.screen_bounds();
        let columns: Vec<ColumnId> = self
            .table
            .columns()
            .iter()
            .map(|column| column.id.clone())
            .collect();
        let column_w = self.column_width(b.w);
        let column_at = |x: f32| -> Option<ColumnId> {
            let index = ((x - b.x) / column_w).floor();
            (index >= 0.0)
                .then(|| columns.get(index as usize).cloned())
                .flatten()
        };

        if let Some(WidgetMessage::MouseMove { pos, .. }) = msg.data::<WidgetMessage>() {
            let index = ((pos.y - (b.y + ROW_H)) / ROW_H).floor();
            self.hovered =
                (index >= 0.0 && (index as usize) < self.rows.len()).then(|| index as usize);
        }
        if msg
            .data::<WidgetMessage>()
            .is_some_and(|m| matches!(m, WidgetMessage::MouseLeave))
        {
            self.hovered = None;
        }

        if let Some(WidgetMessage::MouseDown { pos, mods, .. }) = msg.data::<WidgetMessage>() {
            let (pos, shift) = (*pos, mods.shift);
            // The header is wherever it was *drawn*, which when the grid is
            // scrolled is not where the widget's own top is. Reading the clip
            // here is what keeps the click and the paint agreeing.
            let header_y = widget.clip_bounds.y.max(b.y);
            if pos.y < header_y + ROW_H && pos.y >= header_y {
                if let Some(column) = column_at(pos.x) {
                    self.cycle_sort(&column);
                    widget.invalidate_layout();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        DataGridMessage::ViewChanged,
                    ));
                }
                msg.handled = true;
                return;
            }
            let index = ((pos.y - (b.y + ROW_H)) / ROW_H).floor();
            if index >= 0.0
                && (index as usize) < self.rows.len()
                && let Some(column) = column_at(pos.x)
            {
                let cell = (self.rows[index as usize], column);
                // Shift extends from the existing anchor; a plain click moves
                // both corners, which is what collapses a range to one cell.
                self.selection = Some(match (&self.selection, shift) {
                    (Some(existing), true) => Selection {
                        anchor: existing.anchor.clone(),
                        focus: cell,
                    },
                    _ => Selection {
                        anchor: cell.clone(),
                        focus: cell,
                    },
                });
                self.editing = None;
                self.error = None;
                msg.handled = true;
            }
        }

        if let Some(WidgetMessage::Text(text)) = msg.data::<WidgetMessage>() {
            if self.selection.is_some() {
                // Typing into a selected cell starts an edit that *replaces*
                // it, as every grid does: the old value is not a prefix of
                // what you meant to type.
                let text = text.clone();
                self.editing.get_or_insert_with(String::new).push_str(&text);
                msg.handled = true;
            }
        }

        if let Some(WidgetMessage::KeyDown(key, _)) = msg.data::<WidgetMessage>() {
            let key = *key;
            match key {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if self.editing.is_some() {
                        self.commit(emit, widget.handle);
                        widget.invalidate_layout();
                    } else {
                        // Enter on a cell that is not being edited opens it for
                        // editing with the value already there, so a
                        // correction is a correction rather than a retype.
                        if let Some((row, column)) = self.focused_cell() {
                            let cell = self.table.get(row, &column);
                            self.editing = Some(match cell {
                                Cell::Empty => String::new(),
                                other => other.display(),
                            });
                        }
                    }
                    msg.handled = true;
                }
                KeyCode::Escape => {
                    self.editing = None;
                    self.error = None;
                    msg.handled = true;
                }
                KeyCode::Backspace => {
                    if let Some(buffer) = self.editing.as_mut() {
                        buffer.pop();
                        msg.handled = true;
                    }
                }
                KeyCode::Delete => {
                    // Clearing is an edit like any other, and an empty string
                    // in a text column is `Cell::Empty` — which is what makes
                    // "show me what is missing" find it again.
                    if self.editing.is_none() {
                        self.editing = Some(String::new());
                        self.commit(emit, widget.handle);
                        widget.invalidate_layout();
                        msg.handled = true;
                    }
                }
                KeyCode::ArrowUp
                | KeyCode::ArrowDown
                | KeyCode::ArrowLeft
                | KeyCode::ArrowRight => {
                    if self.editing.is_none() {
                        let (dr, dc) = match key {
                            KeyCode::ArrowUp => (-1, 0),
                            KeyCode::ArrowDown => (1, 0),
                            KeyCode::ArrowLeft => (0, -1),
                            _ => (0, 1),
                        };
                        self.step(dr, dc);
                        msg.handled = true;
                    }
                }
                KeyCode::Tab => {
                    if self.editing.is_some() {
                        self.commit(emit, widget.handle);
                    }
                    self.step(0, 1);
                    widget.invalidate_layout();
                    msg.handled = true;
                }
                _ => {}
            }
        }
    }
}

pub struct DataGridBuilder {
    widget: WidgetBuilder,
    table: DataTable,
    font_id: u8,
}

impl DataGridBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            table: DataTable::default(),
            font_id: 0,
        }
    }

    #[must_use]
    pub fn with_table(mut self, table: DataTable) -> Self {
        self.table = table;
        self
    }

    #[must_use]
    pub fn with_font_id(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let grid = DataGrid::new(self.table, self.font_id);
        UiNode::new(self.widget.build(), Box::new(grid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_table::Column;
    use crate::message::Modifiers;

    /// A catalogue-shaped table: a key column and two locales, one of which is
    /// half-translated.
    fn catalogue(n: usize) -> DataTable {
        let mut table = DataTable::new(vec![
            Column::text("key", "Key"),
            Column::text("en", "English"),
            Column::text("fr", "Français"),
        ]);
        for i in 0..n {
            let mut cells = vec![
                ("key".to_string(), Cell::Text(format!("ui.button.{i}"))),
                ("en".to_string(), Cell::Text(format!("Button {i}"))),
            ];
            if i % 2 == 0 {
                cells.push(("fr".to_string(), Cell::Text(format!("Bouton {i}"))));
            }
            table.push_row(cells);
        }
        table
    }

    fn grid_of(table: DataTable) -> (Widget, DataGrid) {
        let mut widget = Widget::default();
        widget.actual_local_position = Vec2::ZERO;
        let grid = DataGrid::new(table, 0);
        widget.actual_local_size = Vec2::new(600.0, ROW_H + grid.rows.len() as f32 * ROW_H);
        widget.clip_bounds = Rect::new(0.0, 0.0, 600.0, 440.0);
        (widget, grid)
    }

    fn draw_into(widget: &Widget, grid: &DataGrid, clip: Rect) -> DrawingContext {
        let mut ctx = DrawingContext::new(600.0, 440.0);
        ctx.push_clip_rect(clip);
        grid.draw(widget, &mut ctx);
        ctx
    }

    fn click(
        widget: &mut Widget,
        grid: &mut DataGrid,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Vec<UiMessage> {
        let mut emit = Vec::new();
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos: Vec2::new(x, y),
                button: crate::message::MouseButton::Left,
                mods: Modifiers {
                    shift,
                    ..Default::default()
                },
            },
        );
        grid.handle_routed_message(widget, &mut msg, &mut emit);
        emit
    }

    fn type_text(widget: &mut Widget, grid: &mut DataGrid, text: &str) {
        let mut emit = Vec::new();
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            WidgetMessage::Text(text.to_string()),
        );
        grid.handle_routed_message(widget, &mut msg, &mut emit);
    }

    fn press(widget: &mut Widget, grid: &mut DataGrid, key: KeyCode) -> Vec<UiMessage> {
        let mut emit = Vec::new();
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            WidgetMessage::KeyDown(key, Modifiers::default()),
        );
        grid.handle_routed_message(widget, &mut msg, &mut emit);
        emit
    }

    /// Screen `y` of the first body row.
    const FIRST_ROW_Y: f32 = ROW_H + 4.0;

    #[test]
    fn ten_thousand_rows_cost_what_a_screenful_costs() {
        // MORROWIND-M's acceptance property, in the grid's own terms. The
        // localisation table of a shipped game is thousands of rows and the
        // panel is twenty of them tall.
        let clip = Rect::new(0.0, 0.0, 600.0, 440.0);
        let (small_w, small) = grid_of(catalogue(20));
        let (big_w, big) = grid_of(catalogue(10_000));
        let small = draw_into(&small_w, &small, clip).instances.len();
        let big = draw_into(&big_w, &big, clip).instances.len();
        assert!(
            big <= small + 40,
            "a screenful of a big table cost {big}, a small one {small}"
        );
    }

    #[test]
    fn the_header_stays_at_the_top_of_the_clip_when_the_grid_scrolls() {
        // A grid inside a scroll viewer is as tall as its content, so the
        // widget's own top scrolls away. A header that went with it would
        // leave a screen of unlabelled columns — which is the difference
        // between a table and a wall of strings.
        let clip = Rect::new(0.0, 0.0, 600.0, 440.0);
        let (mut widget, grid) = grid_of(catalogue(500));
        widget.actual_local_position = Vec2::new(0.0, -100.0 * ROW_H);
        let ctx = draw_into(&widget, &grid, clip);
        let header_band = ctx
            .instances
            .iter()
            .any(|p| (p.rect[1] - clip.y).abs() < 0.5 && p.rect[2] > 500.0);
        assert!(
            header_band,
            "no full-width band at the top of the clip: the header scrolled away"
        );
    }

    #[test]
    fn typing_changes_nothing_until_it_commits() {
        // The cell is the model's; the buffer is the widget's. A grid that
        // wrote through on every keystroke could not refuse a bad value, and
        // could not be escaped out of.
        let (mut widget, mut grid) = grid_of(catalogue(4));
        click(&mut widget, &mut grid, 250.0, FIRST_ROW_Y, false);
        let row = grid.focused_cell().expect("the click selects a cell").0;
        type_text(&mut widget, &mut grid, "Hello");
        assert!(grid.is_editing());
        assert_eq!(grid.table.get(row, "en"), Cell::Text("Button 0".into()));

        press(&mut widget, &mut grid, KeyCode::Escape);
        assert!(!grid.is_editing());
        assert_eq!(
            grid.table.get(row, "en"),
            Cell::Text("Button 0".into()),
            "escape must leave the cell alone"
        );

        type_text(&mut widget, &mut grid, "Hello");
        let emitted = press(&mut widget, &mut grid, KeyCode::Enter);
        assert_eq!(grid.table.get(row, "en"), Cell::Text("Hello".into()));
        assert_eq!(emitted.len(), 1, "a commit reports itself once");
    }

    #[test]
    fn a_refused_edit_keeps_what_was_typed() {
        // The typed-column promise, at the point it is felt. Throwing away the
        // text because it did not parse is how a grid loses work — and the
        // reason it was refused has to be readable somewhere.
        let mut table = DataTable::new(vec![Column::number("weight", "Weight")]);
        let row = table.push_row([("weight".to_string(), Cell::Number(1.0))]);
        let (mut widget, mut grid) = grid_of(table);
        click(&mut widget, &mut grid, 40.0, FIRST_ROW_Y, false);
        type_text(&mut widget, &mut grid, "heavy");
        press(&mut widget, &mut grid, KeyCode::Enter);

        assert_eq!(grid.table.get(row, "weight"), Cell::Number(1.0));
        assert!(grid.is_editing(), "the buffer survives a refusal");
        assert!(grid.error().is_some(), "and says why");
    }

    #[test]
    fn a_range_edit_that_fails_one_column_writes_none_of_them() {
        // `set_range` is all-or-nothing and this is the path that proves the
        // grid actually goes through it: half a paste is worse than none,
        // because the undo the user reaches for no longer matches.
        let mut table = DataTable::new(vec![
            Column::text("name", "Name"),
            Column::number("weight", "Weight"),
        ]);
        let row = table.push_row([
            ("name".to_string(), Cell::Text("Rock".into())),
            ("weight".to_string(), Cell::Number(2.0)),
        ]);
        let (mut widget, mut grid) = grid_of(table);
        click(&mut widget, &mut grid, 40.0, FIRST_ROW_Y, false);
        click(&mut widget, &mut grid, 400.0, FIRST_ROW_Y, true);
        type_text(&mut widget, &mut grid, "heavy");
        press(&mut widget, &mut grid, KeyCode::Enter);

        assert_eq!(grid.table.get(row, "name"), Cell::Text("Rock".into()));
        assert_eq!(grid.table.get(row, "weight"), Cell::Number(2.0));
        assert!(grid.error().is_some());
    }

    #[test]
    fn a_range_edit_that_parses_everywhere_writes_everywhere() {
        let (mut widget, mut grid) = grid_of(catalogue(4));
        click(&mut widget, &mut grid, 250.0, FIRST_ROW_Y, false);
        // Two rows down and one column across: a 2x2 rectangle.
        click(&mut widget, &mut grid, 450.0, FIRST_ROW_Y + ROW_H, true);
        type_text(&mut widget, &mut grid, "TODO");
        let emitted = press(&mut widget, &mut grid, KeyCode::Enter);

        let rows = grid.visible_rows().to_vec();
        for row in &rows[0..2] {
            assert_eq!(grid.table.get(*row, "en"), Cell::Text("TODO".into()));
            assert_eq!(grid.table.get(*row, "fr"), Cell::Text("TODO".into()));
        }
        assert_eq!(
            grid.table.get(rows[0], "key"),
            Cell::Text("ui.button.0".into()),
            "the key column was outside the rectangle"
        );
        assert!(matches!(
            emitted[0].data::<DataGridMessage>(),
            Some(DataGridMessage::Edited {
                rows: 2,
                columns: 2,
                ..
            })
        ));
    }

    #[test]
    fn a_header_click_cycles_ascending_descending_and_back_to_authored_order() {
        // Three states rather than two: insertion order is the order the
        // catalogue was authored in, and there has to be a way back to it.
        let (mut widget, mut grid) = grid_of(catalogue(6));
        let authored = grid.visible_rows().to_vec();

        click(&mut widget, &mut grid, 40.0, 4.0, false);
        assert!(matches!(grid.view.sort, Some((_, SortOrder::Ascending))));
        click(&mut widget, &mut grid, 40.0, 4.0, false);
        assert!(matches!(grid.view.sort, Some((_, SortOrder::Descending))));
        assert_eq!(
            grid.visible_rows().first(),
            authored.last(),
            "descending by key reverses a table keyed in order"
        );
        click(&mut widget, &mut grid, 40.0, 4.0, false);
        assert!(grid.view.sort.is_none());
        assert_eq!(grid.visible_rows(), authored, "back to authored order");
    }

    #[test]
    fn a_row_filtered_out_from_under_the_selection_stops_being_selected() {
        // Otherwise the next keystroke edits a row nobody can see, which is
        // the worst kind of edit: silent, and in the wrong place.
        let (mut widget, mut grid) = grid_of(catalogue(6));
        click(&mut widget, &mut grid, 40.0, FIRST_ROW_Y, false);
        assert!(grid.focused_cell().is_some());

        let mut emit = Vec::new();
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            DataGridMessage::SetFilter("button.5".into()),
        );
        grid.handle_routed_message(&mut widget, &mut msg, &mut emit);

        assert_eq!(grid.visible_rows().len(), 1);
        assert!(
            grid.focused_cell().is_none(),
            "the selected row is not on screen any more"
        );
    }

    #[test]
    fn only_incomplete_finds_the_untranslated_rows() {
        // The question a translator opens the table to ask. It is the model's
        // to answer; this is that the grid actually asks it.
        let (mut widget, mut grid) = grid_of(catalogue(6));
        let mut emit = Vec::new();
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            DataGridMessage::SetOnlyIncomplete(true),
        );
        grid.handle_routed_message(&mut widget, &mut msg, &mut emit);
        assert_eq!(grid.visible_rows().len(), 3, "the odd-numbered rows");
    }

    #[test]
    fn arrows_move_a_cell_at_a_time_and_stop_at_the_edges() {
        let (mut widget, mut grid) = grid_of(catalogue(3));
        click(&mut widget, &mut grid, 40.0, FIRST_ROW_Y, false);
        let start = grid.focused_cell().unwrap();

        press(&mut widget, &mut grid, KeyCode::ArrowUp);
        assert_eq!(grid.focused_cell().unwrap(), start, "clamped at the top");
        press(&mut widget, &mut grid, KeyCode::ArrowLeft);
        assert_eq!(grid.focused_cell().unwrap(), start, "and at the left");

        press(&mut widget, &mut grid, KeyCode::ArrowDown);
        press(&mut widget, &mut grid, KeyCode::ArrowRight);
        let moved = grid.focused_cell().unwrap();
        assert_ne!(moved.0, start.0);
        assert_eq!(moved.1, "en");
    }

    #[test]
    fn delete_empties_a_cell_rather_than_writing_an_empty_string() {
        // `Cell::Empty` and `Cell::Text("")` are different, and the difference
        // is what "show me what is missing" runs on.
        let (mut widget, mut grid) = grid_of(catalogue(2));
        click(&mut widget, &mut grid, 250.0, FIRST_ROW_Y, false);
        let row = grid.focused_cell().unwrap().0;
        press(&mut widget, &mut grid, KeyCode::Delete);
        assert!(
            grid.table.get(row, "en").is_empty(),
            "delete must leave the cell empty, not blank"
        );
    }
    #[test]
    fn the_keyboard_belongs_to_the_grid_only_once_a_cell_is_chosen() {
        // A focused widget that reports `is_text_input` swallows every key
        // before the game sees it. Both directions of getting this wrong are
        // real: always true means a grid nobody clicked into eats the fly-cam's
        // WASD, and always false means typing `w` into a cell also switches the
        // gizmo to Translate.
        let (mut widget, mut grid) = grid_of(catalogue(3));
        assert!(!grid.is_text_input(), "nothing is selected yet");
        click(&mut widget, &mut grid, 40.0, FIRST_ROW_Y, false);
        assert!(grid.is_text_input(), "a chosen cell owns the keyboard");
    }
}
