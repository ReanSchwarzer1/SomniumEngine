//! Phase 26-Zeta — the Details column grammar.
//!
//! Every inspector row in the editor goes through this widget so the Details
//! panel has one measured layout instead of ~120 hand-placed labels. The rules
//! are the approved redline (§06, "Details column grammar"):
//!
//! ```text
//! │← 14 ─→│←──── label 46 % (96…176) ────→│←──── value ────→│ 8 │
//!   gutter          ellipsised, tooltip           control     inset
//! ```
//!
//! * the label column is 46 % of panel width, clamped to 96–176 logical px;
//! * the value column takes the rest, minus the gutter and the right inset;
//! * labels never wrap — they ellipsise and gain a tooltip carrying the full
//!   text, because a wrapped label changes the row height and the whole point
//!   of the table is that rows do not move;
//! * under 240 px of panel width the row **stacks** label above value and
//!   grows from 24 to 40 px, so a narrow Details is cramped rather than
//!   truncated into uselessness;
//! * the 14 px left gutter carries the modified dot. It is the only modified
//!   indicator in the editor — no italics, no colour change on the label.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    typography::{TextRole, text_style},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

/// Left gutter reserved for the modified dot.
pub const GUTTER: f32 = 14.0;
/// Fraction of the row width given to the label column.
pub const LABEL_FRACTION: f32 = 0.46;
pub const LABEL_MIN: f32 = 96.0;
pub const LABEL_MAX: f32 = 176.0;
/// Below this row width the row stacks label over value.
pub const STACK_BELOW: f32 = 240.0;
/// Right inset before the panel edge / scrollbar.
pub const RIGHT_INSET: f32 = 8.0;

/// Geometry of one row at a given width. Pure so it can be unit-tested without
/// a widget tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowMetrics {
    pub stacked: bool,
    pub height: f32,
    pub label_x: f32,
    pub label_w: f32,
    pub value_x: f32,
    pub value_w: f32,
}

pub fn row_metrics(width: f32) -> RowMetrics {
    let d = &theme::NOCTURNE.density;
    if width < STACK_BELOW {
        let inner = (width - GUTTER - RIGHT_INSET).max(24.0);
        return RowMetrics {
            stacked: true,
            height: 40.0,
            label_x: GUTTER,
            label_w: inner,
            value_x: GUTTER,
            value_w: inner,
        };
    }
    let label_w = (width * LABEL_FRACTION).clamp(LABEL_MIN, LABEL_MAX);
    let value_x = GUTTER + label_w;
    RowMetrics {
        stacked: false,
        height: d.row_dense,
        label_x: GUTTER,
        label_w,
        value_x,
        value_w: (width - value_x - RIGHT_INSET).max(24.0),
    }
}

/// Truncate `text` to `max_w` with a trailing ellipsis, returning the string to
/// draw and whether it was shortened (the caller uses that to decide whether
/// the row needs a tooltip).
pub fn ellipsise(text: &str, max_w: f32, mut width_of: impl FnMut(&str) -> f32) -> (String, bool) {
    if max_w <= 0.0 || width_of(text) <= max_w {
        return (text.to_string(), false);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    while keep > 0 {
        keep -= 1;
        let candidate: String = chars[..keep].iter().collect::<String>() + "…";
        if width_of(&candidate) <= max_w {
            return (candidate, true);
        }
    }
    ("…".to_string(), true)
}

#[derive(Debug, Clone)]
pub enum PropertyRowMessage {
    /// Value differs from the row's baseline — show the gutter dot.
    SetModified(bool),
    /// Row is not editable (a preset-locked field, a derived value).
    SetReadOnly(bool),
    /// Sent `FromWidget` when the gutter dot is clicked. The editor answers it
    /// by writing the baseline back through the ordinary value path, so a
    /// revert is one `ValueChanged` and therefore one undo step.
    RevertRequested,
}

impl PropertyRowMessage {
    pub fn set_modified(dest: NodeHandle, modified: bool) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::ToWidget,
            Self::SetModified(modified),
        )
    }

    pub fn set_read_only(dest: NodeHandle, read_only: bool) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::ToWidget,
            Self::SetReadOnly(read_only),
        )
    }
}

/// Whether a click at `pos` inside `bounds` landed on the revert gutter.
pub fn hit_gutter(bounds: Rect, pos: Vec2) -> bool {
    pos.x >= bounds.x
        && pos.x < bounds.x + GUTTER
        && pos.y >= bounds.y
        && pos.y < bounds.y + bounds.h
}

pub struct PropertyRow {
    pub label: String,
    pub modified: bool,
    pub read_only: bool,
    /// Cursor is over the gutter — the dot grows a ring so it reads as a
    /// control rather than as decoration.
    hover_gutter: bool,
    /// Cached from the last arrange so `draw` paints the same columns the
    /// child was arranged into.
    metrics: std::cell::Cell<RowMetrics>,
}

impl PropertyRow {
    fn label_style(&self) -> crate::typography::TextStyle {
        let style = text_style(TextRole::Label);
        if self.read_only {
            style.with_color(theme::TEXT_DISABLED)
        } else {
            style
        }
    }
}

impl Control for PropertyRow {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let width = if available.x.is_finite() {
            available.x
        } else {
            // No constraint yet (a first pass inside an auto-sized parent) —
            // measure against the default Details width from the redline.
            340.0
        };
        let m = row_metrics(width);
        self.metrics.set(m);
        let child_constraint = Vec2::new(m.value_w, m.height);
        let mut child_h = 0.0f32;
        for &ch in &widget.children {
            ctx.measure_child(ch, child_constraint);
            child_h = child_h.max(ctx.desired_size(ch).y);
        }
        let height = if m.stacked {
            m.height
        } else {
            m.height.max(child_h)
        };
        Vec2::new(width, height)
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let origin = widget.actual_local_position;
        let m = row_metrics(final_size.x);
        self.metrics.set(m);
        // Value control: right column, vertically centred in the row when it
        // is shorter than the row (a 22 px control in a 24 px row).
        let (vx, vy, vh) = if m.stacked {
            (m.value_x, m.height * 0.5, m.height * 0.5)
        } else {
            (m.value_x, 0.0, final_size.y)
        };
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            let h = ds.y.min(vh).max(0.0);
            let y = origin.y + vy + ((vh - h) * 0.5).max(0.0);
            ctx.arrange_child(
                ch,
                Rect::new(origin.x + vx, y, m.value_w, h.max(ds.y.min(vh))),
            );
        }
        Vec2::new(final_size.x, final_size.y.max(m.height))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        if widget.background[3] != 0 {
            ctx.push_rect_filled(b, widget.background);
        }
        let m = self.metrics.get();
        let style = self.label_style();
        let font_id = style.font_id();

        // Modified dot — the single modified cue in the editor, and the click
        // target that reverts the row.
        if self.modified {
            let cx = b.x + GUTTER * 0.5;
            let cy = b.y + (if m.stacked { 12.0 } else { m.height * 0.5 });
            let t = theme::active();
            let accent = t.semantic.accent.default.bytes();
            if self.hover_gutter {
                let r = 5.0;
                // A ring, not a square: the pipeline can round it now.
                ctx.push_round_rect_border(
                    Rect::new(cx - r, cy - r, r * 2.0, r * 2.0),
                    r,
                    t.geometry.stroke_hairline,
                    t.semantic.accent.hover.bytes(),
                );
            }
            let r = 2.5;
            // The design calls this a dot, and until Styx it was a 5 px square.
            ctx.push_round_rect(Rect::new(cx - r, cy - r, r * 2.0, r * 2.0), r, accent);
        }

        let (label, _truncated) = ellipsise(&self.label, m.label_w, |s| {
            ctx.font_atlas.measure_text(s, style.px, font_id).x
        });
        let line_h = ctx
            .font_atlas
            .measure_text("Ag", style.px, font_id)
            .y
            .max(style.px);
        let label_y = if m.stacked {
            b.y + 4.0
        } else {
            b.y + ((m.height - line_h) * 0.5).max(0.0)
        };
        ctx.push_text(
            &label,
            Vec2::new(b.x + m.label_x, label_y),
            font_id,
            style.px,
            style.color,
        );
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> crate::node::CursorKind {
        if self.modified && !self.read_only && hit_gutter(widget.screen_bounds(), pos) {
            crate::node::CursorKind::Pointer
        } else {
            crate::node::CursorKind::Default
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(m) = msg.data::<PropertyRowMessage>() {
            match m {
                PropertyRowMessage::SetModified(v) => self.modified = *v,
                PropertyRowMessage::SetReadOnly(v) => self.read_only = *v,
                PropertyRowMessage::RevertRequested => {}
            }
            msg.handled = true;
        }
        if let Some(WidgetMessage::MouseMove { pos }) = msg.data::<WidgetMessage>() {
            self.hover_gutter = hit_gutter(widget.screen_bounds(), *pos);
        }
        if msg
            .data::<WidgetMessage>()
            .is_some_and(|m| matches!(m, WidgetMessage::MouseLeave))
        {
            self.hover_gutter = false;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            // Only a lit dot is clickable. An unmodified row has nothing to
            // revert to, and swallowing the click there would make the gutter
            // feel broken.
            if self.modified && !self.read_only && hit_gutter(widget.screen_bounds(), *pos) {
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    PropertyRowMessage::RevertRequested,
                ));
                msg.handled = true;
            }
        }
        // A row's label can be renamed (multi-select header, unit change).
        if let Some(TextMessage::SetText(s)) = msg.data::<TextMessage>() {
            self.label = s.clone();
            widget.invalidate_layout();
            msg.handled = true;
        }
    }
}

pub struct PropertyRowBuilder {
    widget: WidgetBuilder,
    label: String,
    modified: bool,
    read_only: bool,
}

impl PropertyRowBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            label: String::new(),
            modified: false,
            read_only: false,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn with_modified(mut self, modified: bool) -> Self {
        self.modified = modified;
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn build(self) -> UiNode {
        // The full label is always the tooltip: a row that fits today can be
        // truncated tomorrow by a splitter drag, and a tooltip that appears
        // only after truncation is a tooltip nobody discovers.
        let widget = self.widget.with_tooltip(self.label.clone());
        UiNode::new(
            widget.build(),
            Box::new(PropertyRow {
                label: self.label,
                modified: self.modified,
                read_only: self.read_only,
                hover_gutter: false,
                metrics: std::cell::Cell::new(row_metrics(340.0)),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_column_is_46_percent_clamped_to_the_redline_range() {
        // Default Details width from the redline.
        assert_eq!(row_metrics(340.0).label_w, 340.0 * LABEL_FRACTION);
        // Wide panel clamps at the maximum so the value column keeps growing.
        assert_eq!(row_metrics(900.0).label_w, LABEL_MAX);
        // The minimum is a floor the stacking rule reaches first — 46 % only
        // falls under 96 px below 209 px of width, and at 240 px the row has
        // already stacked. It stays as the documented floor so a future change
        // to STACK_BELOW cannot produce a 40 px label column by accident.
        for w in [STACK_BELOW, 300.0, 512.0, 4000.0] {
            let m = row_metrics(w);
            assert!(
                m.label_w >= LABEL_MIN && m.label_w <= LABEL_MAX,
                "width {w}"
            );
        }
    }

    #[test]
    fn rows_stack_and_grow_below_the_collapse_threshold() {
        let wide = row_metrics(STACK_BELOW);
        let narrow = row_metrics(STACK_BELOW - 1.0);
        assert!(!wide.stacked);
        assert_eq!(wide.height, theme::NOCTURNE.density.row_dense);
        assert!(narrow.stacked);
        assert_eq!(narrow.height, 40.0);
        // Stacked rows still reserve the gutter, so the modified dot does not
        // move when a splitter crosses the threshold.
        assert_eq!(narrow.label_x, GUTTER);
    }

    #[test]
    fn value_column_never_collapses_to_nothing() {
        for w in [0.0, 40.0, 120.0, 241.0, 4000.0] {
            assert!(row_metrics(w).value_w >= 24.0, "width {w}");
        }
    }

    #[test]
    fn ellipsis_shortens_only_when_the_label_overflows() {
        let width_of = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(
            ellipsise("Position X", 200.0, width_of),
            ("Position X".into(), false)
        );
        let (short, cut) = ellipsise("Great Lakes preset", 60.0, width_of);
        assert!(cut);
        assert!(short.ends_with('…'));
        assert!(width_of(&short) <= 60.0);
    }

    #[test]
    fn the_gutter_is_the_only_clickable_strip() {
        let b = Rect::new(100.0, 40.0, 340.0, 24.0);
        assert!(hit_gutter(b, Vec2::new(103.0, 50.0)));
        // One pixel past the gutter belongs to the label, which must stay
        // click-through so a drag-select in the panel is not interrupted.
        assert!(!hit_gutter(b, Vec2::new(100.0 + GUTTER, 50.0)));
        assert!(!hit_gutter(b, Vec2::new(103.0, 39.0)));
    }

    #[test]
    fn ellipsis_degenerates_to_a_single_glyph_rather_than_panicking() {
        let (s, cut) = ellipsise("Position X", 1.0, |t| t.chars().count() as f32 * 10.0);
        assert_eq!(s, "…");
        assert!(cut);
    }
}
