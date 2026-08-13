// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/grid.rs
// Grid: rows/columns with SizeMode::Strict/Auto/Stretch, 4-group measurement algorithm.
// RefCell used for mutable measurement state (cells/groups) shared across &self methods.

use crate::{
    draw::DrawingContext,
    message::{NodeHandle, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use std::cell::RefCell;

// ---------------------------------------------------------------------------
// Dimension types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SizeMode {
    #[default]
    Strict,
    Auto,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GridDimension {
    pub size_mode: SizeMode,
    pub desired_size: f32,
    pub actual_size: f32,
    pub location: f32,
    pub(crate) unmeasured_node_count: usize,
}

impl GridDimension {
    pub fn strict(desired_size: f32) -> Self {
        Self {
            size_mode: SizeMode::Strict,
            desired_size,
            actual_size: desired_size,
            ..Default::default()
        }
    }
    pub fn auto() -> Self {
        Self {
            size_mode: SizeMode::Auto,
            ..Default::default()
        }
    }
    pub fn stretch() -> Self {
        Self {
            size_mode: SizeMode::Stretch,
            ..Default::default()
        }
    }

    fn update_size(&mut self, node_size: f32, available_size: f32) {
        match self.size_mode {
            SizeMode::Strict => {}
            SizeMode::Auto => {
                self.desired_size = self.desired_size.max(node_size);
                self.actual_size = self.desired_size;
            }
            SizeMode::Stretch => {
                self.actual_size = if available_size.is_finite() {
                    self.desired_size + available_size
                } else {
                    node_size
                };
            }
        }
    }
}

pub type Row = GridDimension;
pub type Column = GridDimension;

// ---------------------------------------------------------------------------
// Internal cell/group helpers
// ---------------------------------------------------------------------------

pub struct Cell {
    pub nodes: Vec<NodeHandle>,
    pub row_index: usize,
    pub column_index: usize,
}

/// Maps (row_mode, col_mode) → group index (0–3).
/// See Fyrox grid.rs §"Group 0 represents …" for rationale.
fn group_index(row: SizeMode, col: SizeMode) -> usize {
    match (row, col) {
        (SizeMode::Strict, SizeMode::Strict)
        | (SizeMode::Strict, SizeMode::Auto)
        | (SizeMode::Auto, SizeMode::Strict)
        | (SizeMode::Auto, SizeMode::Auto) => 0,
        (SizeMode::Stretch, SizeMode::Auto) => 1,
        (SizeMode::Strict, SizeMode::Stretch) | (SizeMode::Auto, SizeMode::Stretch) => 2,
        (SizeMode::Stretch, SizeMode::Strict) | (SizeMode::Stretch, SizeMode::Stretch) => 3,
    }
}

fn choose_constraint(dim: &GridDimension, available: f32) -> f32 {
    match dim.size_mode {
        SizeMode::Strict => dim.desired_size,
        SizeMode::Stretch => dim.desired_size + available,
        SizeMode::Auto => f32::INFINITY,
    }
}

fn count_stretch(dims: &[GridDimension]) -> usize {
    dims.iter()
        .filter(|d| d.size_mode == SizeMode::Stretch)
        .count()
}

fn total_non_stretch_desired(dims: &[GridDimension]) -> Option<f32> {
    if dims.iter().all(|d| d.size_mode != SizeMode::Stretch) {
        return Some(0.0);
    }
    if dims.iter().all(|d| d.unmeasured_node_count == 0) {
        Some(dims.iter().map(|d| d.desired_size).sum())
    } else {
        None
    }
}

fn avg_stretch(dims: &RefCell<Vec<GridDimension>>, available: f32) -> Option<f32> {
    if available.is_infinite() {
        return Some(available);
    }
    let dims = dims.borrow();
    let n = count_stretch(&dims);
    if n > 0 {
        let rest = available - total_non_stretch_desired(&dims)?;
        Some(rest / n as f32)
    } else {
        Some(0.0)
    }
}

fn arrange_dims(dims: &mut [GridDimension], final_size: f32) {
    let preset: f32 = dims.iter().map(|d| d.desired_size).sum();
    let stretch_n = count_stretch(dims);
    let avg = if stretch_n > 0 {
        (final_size - preset) / stretch_n as f32
    } else {
        0.0
    };

    let mut loc = 0.0;
    for d in dims.iter_mut() {
        d.location = loc;
        d.actual_size = match d.size_mode {
            SizeMode::Strict | SizeMode::Auto => d.desired_size,
            SizeMode::Stretch => d.desired_size + avg,
        };
        loc += d.actual_size;
    }
}

// ---------------------------------------------------------------------------
// Grid control
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum GridMessage {
    /// Set a strict row's height in pixels (0 hides the row).
    SetRowSize(usize, f32),
}

pub struct Grid {
    pub rows: RefCell<Vec<Row>>,
    pub columns: RefCell<Vec<Column>>,
    pub draw_border: bool,
    pub border_thickness: f32,
    cells: RefCell<Vec<Cell>>,
    groups: RefCell<[Vec<usize>; 4]>,
}

impl Grid {
    fn calc_needed_measurements(&self, widget: &Widget, ctx: &mut LayoutCtx) {
        // Reset dimensions
        {
            let mut rows = self.rows.borrow_mut();
            let mut cols = self.columns.borrow_mut();
            for d in rows.iter_mut().chain(cols.iter_mut()) {
                d.unmeasured_node_count = 0;
                match d.size_mode {
                    SizeMode::Auto => d.desired_size = 0.0,
                    SizeMode::Strict => d.actual_size = d.desired_size,
                    SizeMode::Stretch => {}
                }
            }
        }
        // Collect child row/col (no RefCell borrows held during ctx access)
        let positions: Vec<(usize, usize)> = widget
            .children
            .iter()
            .map(|&ch| (ctx.row(ch), ctx.column(ch)))
            .collect();
        {
            let mut rows = self.rows.borrow_mut();
            let mut cols = self.columns.borrow_mut();
            for (ri, ci) in positions {
                if let Some(c) = cols.get_mut(ci) {
                    if c.size_mode == SizeMode::Auto {
                        c.unmeasured_node_count += 1;
                    }
                }
                if let Some(r) = rows.get_mut(ri) {
                    if r.size_mode == SizeMode::Auto {
                        r.unmeasured_node_count += 1;
                    }
                }
            }
        }
    }

    fn initialize_measure(&self, widget: &Widget, ctx: &mut LayoutCtx) {
        self.calc_needed_measurements(widget, ctx);

        // Collect child positions before any RefCell borrows
        let positions: Vec<(NodeHandle, usize, usize)> = widget
            .children
            .iter()
            .map(|&ch| (ch, ctx.row(ch), ctx.column(ch)))
            .collect();

        let mut groups = self.groups.borrow_mut();
        for g in groups.iter_mut() {
            g.clear();
        }
        let mut cells = self.cells.borrow_mut();
        cells.clear();

        let rows = self.rows.borrow();
        let columns = self.columns.borrow();
        for (ci, col) in columns.iter().enumerate() {
            for (ri, row) in rows.iter().enumerate() {
                let g = group_index(row.size_mode, col.size_mode);
                groups[g].push(cells.len());
                let nodes: Vec<NodeHandle> = positions
                    .iter()
                    .filter(|(_, r, c)| *r == ri && *c == ci)
                    .map(|(h, _, _)| *h)
                    .collect();
                cells.push(Cell {
                    nodes,
                    row_index: ri,
                    column_index: ci,
                });
            }
        }
    }

    fn measure_cell_node(
        &self,
        child: NodeHandle,
        ctx: &mut LayoutCtx,
        avail: Vec2,
        mw: bool,
        mh: bool,
    ) {
        let ri = ctx.row(child);
        let ci = ctx.column(child);

        let constraint = {
            let rows = self.rows.borrow();
            let cols = self.columns.borrow();
            let Some(row) = rows.get(ri) else {
                return;
            };
            let Some(col) = cols.get(ci) else {
                return;
            };
            Vec2::new(
                choose_constraint(col, avail.x),
                choose_constraint(row, avail.y),
            )
        };

        ctx.measure_child(child, constraint);
        let ds = ctx.desired_size(child);

        let mut rows = self.rows.borrow_mut();
        let mut cols = self.columns.borrow_mut();
        let Some(row) = rows.get_mut(ri) else {
            return;
        };
        let Some(col) = cols.get_mut(ci) else {
            return;
        };

        if mw {
            col.update_size(ds.x, avail.x);
            if col.size_mode == SizeMode::Auto {
                col.unmeasured_node_count = col.unmeasured_node_count.saturating_sub(1);
            }
        }
        if mh {
            row.update_size(ds.y, avail.y);
            if row.size_mode == SizeMode::Auto {
                row.unmeasured_node_count = row.unmeasured_node_count.saturating_sub(1);
            }
        }
    }

    fn measure_group(&self, group: &[usize], ctx: &mut LayoutCtx, avail: Vec2) {
        let nodes: Vec<NodeHandle> = {
            let cells = self.cells.borrow();
            group.iter().flat_map(|&i| cells[i].nodes.clone()).collect()
        };
        for n in nodes {
            self.measure_cell_node(n, ctx, avail, true, true);
        }
    }

    fn measure_group_width(&self, group: &[usize], ctx: &mut LayoutCtx, avail: Vec2) {
        let nodes: Vec<NodeHandle> = {
            let cells = self.cells.borrow();
            group.iter().flat_map(|&i| cells[i].nodes.clone()).collect()
        };
        for n in nodes {
            self.measure_cell_node(n, ctx, avail, true, false);
        }
    }

    fn measure_group_height(&self, group: &[usize], ctx: &mut LayoutCtx, avail: Vec2) {
        let nodes: Vec<NodeHandle> = {
            let cells = self.cells.borrow();
            group.iter().flat_map(|&i| cells[i].nodes.clone()).collect()
        };
        for n in nodes {
            self.measure_cell_node(n, ctx, avail, false, true);
        }
    }
}

impl Control for Grid {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        if self.columns.borrow().is_empty() || self.rows.borrow().is_empty() {
            // No rows/columns defined — act like a Canvas
            let inf = Vec2::new(f32::INFINITY, f32::INFINITY);
            let mut desired = Vec2::ZERO;
            for &ch in &widget.children {
                ctx.measure_child(ch, inf);
                let ds = ctx.desired_size(ch);
                if ds.x > desired.x {
                    desired.x = ds.x;
                }
                if ds.y > desired.y {
                    desired.y = ds.y;
                }
            }
            return desired;
        }

        self.initialize_measure(widget, ctx);

        // Clone group indices so we can release the groups borrow before calling measure helpers
        let (g0, g1, g2, g3) = {
            let groups = self.groups.borrow();
            (
                groups[0].clone(),
                groups[1].clone(),
                groups[2].clone(),
                groups[3].clone(),
            )
        };

        // 4-group algorithm from Fyrox grid.rs
        self.measure_group(&g0, ctx, available);

        if let Some(sy) = avg_stretch(&self.rows, available.y) {
            self.measure_group(&g1, ctx, Vec2::new(available.x, sy));
            let sx = avg_stretch(&self.columns, available.x).unwrap();
            self.measure_group(&g2, ctx, Vec2::new(sx, available.y));
            self.measure_group(&g3, ctx, Vec2::new(sx, sy));
        } else if let Some(sx) = avg_stretch(&self.columns, available.x) {
            self.measure_group(&g2, ctx, Vec2::new(sx, available.y));
            let sy = avg_stretch(&self.rows, available.y).unwrap();
            self.measure_group(&g3, ctx, Vec2::new(sx, sy));
        } else {
            self.measure_group_width(&g1, ctx, Vec2::new(f32::INFINITY, f32::INFINITY));
            let sx = avg_stretch(&self.columns, available.x).unwrap();
            self.measure_group(&g2, ctx, Vec2::new(sx, available.y));
            let sy = avg_stretch(&self.rows, available.y).unwrap();
            self.measure_group_height(&g1, ctx, Vec2::new(available.x, sy));
            self.measure_group(&g3, ctx, Vec2::new(sx, sy));
        }

        Vec2::new(
            self.columns.borrow().iter().map(|c| c.actual_size).sum(),
            self.rows.borrow().iter().map(|r| r.actual_size).sum(),
        )
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        if self.columns.borrow().is_empty() || self.rows.borrow().is_empty() {
            let rect = Rect::new(0.0, 0.0, final_size.x, final_size.y);
            for &ch in &widget.children {
                ctx.arrange_child(ch, rect);
            }
            return final_size;
        }

        {
            let mut cols = self.columns.borrow_mut();
            let mut rows = self.rows.borrow_mut();
            arrange_dims(&mut cols, final_size.x);
            arrange_dims(&mut rows, final_size.y);
        }

        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        for &ch in &widget.children {
            let ri = ctx.row(ch);
            let ci = ctx.column(ch);
            let cols = self.columns.borrow();
            let rows = self.rows.borrow();
            if let (Some(col), Some(row)) = (cols.get(ci), rows.get(ri)) {
                ctx.arrange_child(
                    ch,
                    Rect::new(
                        ox + col.location,
                        oy + row.location,
                        col.actual_size,
                        row.actual_size,
                    ),
                );
            }
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        ctx.push_rect_filled(widget.screen_bounds(), widget.background);
        if self.draw_border {
            let b = widget.screen_bounds();
            let fg = widget.foreground;
            let t = self.border_thickness;
            // Outer border
            ctx.push_rect_border(b, t, fg);
            // Column dividers
            let mut x = b.x;
            for col in self.columns.borrow().iter() {
                x += col.actual_size;
                ctx.push_rect_filled(Rect::new(x - t * 0.5, b.y, t, b.h), fg);
            }
            // Row dividers
            let mut y = b.y;
            for row in self.rows.borrow().iter() {
                y += row.actual_size;
                ctx.push_rect_filled(Rect::new(b.x, y - t * 0.5, b.w, t), fg);
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(GridMessage::SetRowSize(i, h)) = msg.data::<GridMessage>() {
            if let Some(row) = self.rows.borrow_mut().get_mut(*i) {
                row.desired_size = *h;
                row.actual_size = *h;
            }
            widget.invalidate_layout();
            msg.handled = true;
        }
    }
}

// ---------------------------------------------------------------------------
// GridBuilder
// ---------------------------------------------------------------------------

pub struct GridBuilder {
    widget: WidgetBuilder,
    rows: Vec<Row>,
    columns: Vec<Column>,
    draw_border: bool,
    border_thickness: f32,
}

impl GridBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            rows: Vec::new(),
            columns: Vec::new(),
            draw_border: false,
            border_thickness: 1.0,
        }
    }

    pub fn add_row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }
    pub fn add_column(mut self, col: Column) -> Self {
        self.columns.push(col);
        self
    }
    pub fn with_draw_border(mut self, v: bool) -> Self {
        self.draw_border = v;
        self
    }
    pub fn with_border_thickness(mut self, t: f32) -> Self {
        self.border_thickness = t;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Grid {
                rows: RefCell::new(self.rows),
                columns: RefCell::new(self.columns),
                draw_border: self.draw_border,
                border_thickness: self.border_thickness,
                cells: RefCell::new(Vec::new()),
                groups: RefCell::new([Vec::new(), Vec::new(), Vec::new(), Vec::new()]),
            }),
        )
    }
}
