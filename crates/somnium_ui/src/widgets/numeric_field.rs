// NumericField: f32 display/edit widget for the inspector.
// Click to focus, type to edit, Enter/Unfocus to commit.
// Drag horizontally to scrub the value (right increases, left decreases).
// Only accepts digits, '.', and '-'.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum NumericFieldMessage {
    /// Sent ToWidget to update the displayed value (skipped when editing).
    SetValue(f32),
    /// Emitted FromWidget when the user commits a new value.
    ValueChanged(f32),
    /// Emitted FromWidget on every step of a drag-scrub.
    ///
    /// Separate from `ValueChanged` so the receiver can apply it live without
    /// recording an undo entry — a 200-pixel drag would otherwise leave 200 of
    /// them. The gesture ends with a single `ValueChanged`.
    ValueChanging(f32),
}

impl NumericFieldMessage {
    pub fn set_value(dest: NodeHandle, value: f32) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetValue(value))
    }

    pub fn value_changed(dest: NodeHandle, value: f32) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::FromWidget,
            Self::ValueChanged(value),
        )
    }

    pub fn value_changing(dest: NodeHandle, value: f32) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::FromWidget,
            Self::ValueChanging(value),
        )
    }
}

pub struct NumericField {
    pub value: f32,
    editing_text: Option<String>,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
    pub focused: bool,
    /// True from the moment the field gains focus until the first edit.
    ///
    /// Focusing pre-fills the edit buffer with the current value so it can be
    /// amended, but the overwhelmingly common action is replacing it outright.
    /// Without this, typing appended: clicking a field reading `0.000` and
    /// typing `7` committed `0.0007`, which looks exactly like a field that
    /// does not respond at all. The first keystroke now clears the buffer, the
    /// same select-all-on-focus every other editor does.
    select_all: bool,
    /// Value change per pixel of horizontal drag. Fields differ by orders of
    /// magnitude — chromatic aberration lives around 0.004 while a position is
    /// in metres — so one global rate would make most of them unusable.
    pub drag_step: f32,
    /// Cursor x and field value at the moment the drag began. Steps are applied
    /// against this rather than accumulated, so the value cannot drift if
    /// something else writes to the field mid-drag.
    drag_origin: Option<(f32, f32)>,
    /// Set once the pointer has moved far enough for the gesture to count as a
    /// scrub rather than a click.
    scrubbing: bool,
    /// Optional slider range. `None` infers a range from `drag_step`.
    slider_range: Option<(f32, f32)>,
    slider_dragging: bool,
}

/// Pixels of travel before a press becomes a drag instead of a click. Without
/// a threshold, the hand tremor in an ordinary click would nudge the value.
const SCRUB_THRESHOLD: f32 = 3.0;
const FIELD_W: f32 = 72.0;
const SLIDER_GAP: f32 = 6.0;
const HANDLE_W: f32 = 8.0;
const TRACK_H: f32 = 4.0;

fn infer_slider_range(step: f32) -> (f32, f32) {
    if step <= 0.0005 {
        (0.0, 0.01)
    } else if step <= 0.005 {
        (0.0, 1.0)
    } else if step <= 0.02 {
        (0.0, 2.0)
    } else if step <= 0.05 {
        (-50.0, 50.0)
    } else if step <= 0.15 {
        (0.0, 200.0)
    } else if step <= 0.25 {
        (-180.0, 180.0)
    } else if step <= 1.0 {
        (0.0, 400.0)
    } else if step <= 5.0 {
        (0.0, 12_000.0)
    } else if step <= 50.0 {
        (0.0, 20_000.0)
    } else {
        (0.0, 80_000.0)
    }
}

impl NumericField {
    fn display_text(&self) -> String {
        self.editing_text
            .clone()
            .unwrap_or_else(|| format!("{:.3}", self.value))
    }

    fn effective_range(&self) -> (f32, f32) {
        let (mut lo, mut hi) = self
            .slider_range
            .unwrap_or_else(|| infer_slider_range(self.drag_step));
        if lo >= hi {
            hi = lo + 1.0;
        }
        lo = lo.min(self.value);
        hi = hi.max(self.value);
        (lo, hi)
    }

    fn split_rects(b: Rect) -> (Rect, Rect) {
        let field = FIELD_W.min(b.w * 0.45).max(56.0);
        let slider_w = (b.w - field - SLIDER_GAP).max(40.0);
        (
            Rect::new(b.x, b.y, slider_w, b.h),
            Rect::new(b.x + slider_w + SLIDER_GAP, b.y, field, b.h),
        )
    }

    fn value_from_slider(slider: Rect, x: f32, lo: f32, hi: f32) -> f32 {
        let usable = (slider.w - HANDLE_W).max(1.0);
        let t = ((x - slider.x - HANDLE_W * 0.5) / usable).clamp(0.0, 1.0);
        lo + t * (hi - lo)
    }
}

impl Control for NumericField {
    // Governs whether the UI swallows keyboard input. Tied to the live edit
    // state rather than the widget type, so keys reach the game again once a
    // scrub has ended the text-edit session.
    fn is_text_input(&self) -> bool {
        self.focused
    }
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let text = self.display_text();
        let sz = ctx.measure_text(&text, self.px, self.font_id);
        Vec2::new(
            available.x.min(220.0).max(140.0),
            sz.y.max(self.px + 6.0).max(theme::ROW_HEIGHT),
        )
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        let (slider, field) = Self::split_rects(widget.screen_bounds());
        if self.slider_dragging || slider.contains(pos) {
            CursorKind::EwResize
        } else if field.contains(pos) {
            CursorKind::Text
        } else {
            CursorKind::Default
        }
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let (slider, field) = Self::split_rects(b);
        let (lo, hi) = self.effective_range();
        let t = if hi > lo {
            ((self.value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let mid_y = slider.y + slider.h * 0.5;
        ctx.push_rect_filled(
            Rect::new(slider.x, mid_y - TRACK_H * 0.5, slider.w, TRACK_H),
            theme::BORDER_DARK,
        );
        let usable = (slider.w - HANDLE_W).max(1.0);
        let handle_x = slider.x + t * usable;
        ctx.push_rect_filled(
            Rect::new(
                slider.x,
                mid_y - TRACK_H * 0.5,
                handle_x - slider.x,
                TRACK_H,
            ),
            theme::ACCENT,
        );
        ctx.push_rect_filled(
            Rect::new(handle_x, slider.y + 3.0, HANDLE_W, slider.h - 6.0),
            theme::ACCENT,
        );

        let bg = theme::BG_INPUT;
        let bdr = if self.focused {
            theme::BORDER_FOCUS
        } else {
            theme::BORDER_DARK
        };
        ctx.push_rect_filled(field, bg);
        ctx.push_rect_border(field, 1.0, bdr);
        let text = self.display_text();
        let origin = Vec2::new(field.x + 4.0, field.y + 3.0);
        if self.focused && self.select_all && !text.is_empty() {
            let advance = ctx.font_atlas.measure_text(&text, self.px, self.font_id).x;
            ctx.push_rect_filled(
                Rect::new(field.x + 4.0, field.y + 3.0, advance, self.px),
                theme::ACCENT_DIM,
            );
        }
        ctx.push_text(&text, origin, self.font_id, self.px, self.color);
        if self.focused && !self.select_all {
            let advance = ctx.font_atlas.measure_text(&text, self.px, self.font_id).x;
            let cx = field.x + 4.0 + advance;
            ctx.push_rect_filled(Rect::new(cx, field.y + 3.0, 1.0, self.px), self.color);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(d) = msg.data::<NumericFieldMessage>() {
            if let NumericFieldMessage::SetValue(v) = d {
                let v = *v;
                if !self.focused {
                    self.value = v;
                    self.editing_text = None;
                    widget.invalidate_layout();
                    msg.handled = true;
                }
            }
            return;
        }

        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg.clone() {
                WidgetMessage::MouseDown { pos, .. } => {
                    let (slider, _field) = Self::split_rects(widget.screen_bounds());
                    if slider.contains(pos) {
                        self.slider_dragging = true;
                        self.focused = false;
                        self.select_all = false;
                        self.editing_text = None;
                        let (lo, hi) = self.effective_range();
                        let v = Self::value_from_slider(slider, pos.x, lo, hi);
                        if v != self.value {
                            self.value = v;
                            emit.push(NumericFieldMessage::value_changing(widget.handle, v));
                        }
                    } else {
                        self.drag_origin = Some((pos.x, self.value));
                        self.scrubbing = false;
                    }
                    msg.handled = true;
                }
                WidgetMessage::MouseMove { pos } => {
                    if self.slider_dragging {
                        let (slider, _) = Self::split_rects(widget.screen_bounds());
                        let (lo, hi) = self.effective_range();
                        let v = Self::value_from_slider(slider, pos.x, lo, hi);
                        if v != self.value {
                            self.value = v;
                            emit.push(NumericFieldMessage::value_changing(widget.handle, v));
                            widget.invalidate_layout();
                        }
                        msg.handled = true;
                    } else if let Some((start_x, start_value)) = self.drag_origin {
                        let dx = pos.x - start_x;
                        if !self.scrubbing && dx.abs() >= SCRUB_THRESHOLD {
                            self.scrubbing = true;
                            // A scrub is not a text edit. Drop the focus state
                            // the press handed us, or the field would show a
                            // caret and a selection while being dragged.
                            self.focused = false;
                            self.select_all = false;
                            self.editing_text = None;
                        }
                        if self.scrubbing {
                            let v = start_value + dx * self.drag_step;
                            if v != self.value {
                                self.value = v;
                                emit.push(NumericFieldMessage::value_changing(widget.handle, v));
                                widget.invalidate_layout();
                            }
                        }
                        msg.handled = true;
                    }
                }
                WidgetMessage::MouseUp { .. } => {
                    let was_slider = self.slider_dragging;
                    self.slider_dragging = false;
                    let was_scrubbing = self.scrubbing;
                    self.drag_origin = None;
                    self.scrubbing = false;
                    if was_scrubbing || was_slider {
                        emit.push(NumericFieldMessage::value_changed(
                            widget.handle,
                            self.value,
                        ));
                        widget.invalidate_layout();
                        msg.handled = true;
                    }
                }
                WidgetMessage::Focus => {
                    self.focused = true;
                    self.editing_text = Some(format!("{:.3}", self.value));
                    self.select_all = true;
                    widget.invalidate_layout();
                    msg.handled = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                    self.select_all = false;
                    if let Some(text) = self.editing_text.take() {
                        // An empty buffer means everything was deleted; that is
                        // an abandoned edit, not a request to set zero.
                        if let Ok(v) = text.trim().parse::<f32>() {
                            if v != self.value {
                                self.value = v;
                                emit.push(NumericFieldMessage::value_changed(widget.handle, v));
                            }
                        }
                    }
                    widget.invalidate_layout();
                    msg.handled = true;
                }
                WidgetMessage::Text(s) => {
                    if self.focused {
                        // Only clear on a character this field would actually
                        // accept, so a stray keypress does not wipe the value.
                        let accepted = s
                            .chars()
                            .any(|c| c.is_ascii_digit() || c == '.' || c == '-');
                        if self.select_all && accepted {
                            self.editing_text = Some(String::new());
                            self.select_all = false;
                        }
                        let t = self.editing_text.get_or_insert_with(String::new);
                        for ch in s.chars() {
                            if ch.is_ascii_digit() || ch == '.' || ch == '-' {
                                t.push(ch);
                            }
                        }
                        widget.invalidate_layout();
                        msg.handled = true;
                    }
                }
                WidgetMessage::KeyDown(key) => {
                    if self.focused {
                        match key {
                            KeyCode::Backspace => {
                                // Backspace on a whole-field selection clears
                                // it, matching every other text field.
                                if self.select_all {
                                    self.editing_text = Some(String::new());
                                    self.select_all = false;
                                } else if let Some(t) = &mut self.editing_text {
                                    if !t.is_empty() {
                                        let mut chars = t.chars();
                                        chars.next_back();
                                        *t = chars.as_str().to_owned();
                                    }
                                }
                                widget.invalidate_layout();
                                msg.handled = true;
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                self.focused = false;
                                self.select_all = false;
                                if let Some(text) = self.editing_text.take() {
                                    if let Ok(v) = text.trim().parse::<f32>() {
                                        if v != self.value {
                                            self.value = v;
                                            emit.push(NumericFieldMessage::value_changed(
                                                widget.handle,
                                                v,
                                            ));
                                        }
                                    }
                                }
                                widget.invalidate_layout();
                                msg.handled = true;
                            }
                            KeyCode::Escape => {
                                self.focused = false;
                                self.select_all = false;
                                self.editing_text = None;
                                widget.invalidate_layout();
                                msg.handled = true;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub struct NumericFieldBuilder {
    widget: WidgetBuilder,
    value: f32,
    px: f32,
    color: [u8; 4],
    font_id: u8,
    drag_step: f32,
    slider_range: Option<(f32, f32)>,
}

impl NumericFieldBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            value: 0.0,
            px: 12.0,
            color: theme::TEXT_PRIMARY,
            font_id: 0,
            drag_step: 0.05,
            slider_range: None,
        }
    }

    pub fn with_value(mut self, v: f32) -> Self {
        self.value = v;
        self
    }
    pub fn with_font_size(mut self, px: f32) -> Self {
        self.px = px;
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    /// Value change per pixel of horizontal drag-scrub.
    pub fn with_drag_step(mut self, step: f32) -> Self {
        self.drag_step = step;
        self
    }

    pub fn with_range(mut self, min: f32, max: f32) -> Self {
        self.slider_range = Some((min, max));
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(NumericField {
                value: self.value,
                editing_text: None,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
                focused: false,
                select_all: false,
                drag_step: self.drag_step,
                drag_origin: None,
                scrubbing: false,
                slider_range: self.slider_range,
                slider_dragging: false,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_maps_left_edge_to_min_and_right_edge_to_max() {
        let r = Rect::new(0.0, 0.0, 108.0, 18.0);
        assert!((NumericField::value_from_slider(r, 0.0, 0.0, 10.0) - 0.0).abs() < 1e-4);
        assert!((NumericField::value_from_slider(r, 108.0, 0.0, 10.0) - 10.0).abs() < 1e-4);
    }
}
