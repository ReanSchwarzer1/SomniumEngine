// NumericField: f32 display/edit widget for the inspector.
// Click to focus, type to edit, Enter/Unfocus to commit.
// Drag horizontally to scrub the value (right increases, left decreases).
// Only accepts digits, '.', and '-'.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, Modifiers, NodeHandle, UiMessage, WidgetMessage},
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
    /// Unit shown after the value, muted and right-aligned.
    ///
    /// Phase 27-G. A transform inspector reading `0.000` three times over says
    /// nothing about whether it is metres, degrees or a multiplier, and the
    /// answer differs per section. Empty means unitless.
    pub unit: &'static str,
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
    /// Displayed value before the current pointer gesture. Cancellation uses
    /// this even for the slider half, whose drag math has a different origin.
    gesture_origin: Option<f32>,
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

fn scrub_value(start: f32, dx: f32, step: f32, modifiers: Modifiers) -> f32 {
    let precision = if modifiers.shift || modifiers.alt {
        0.1
    } else {
        1.0
    };
    let mut value = start + dx * step * precision;
    if modifiers.ctrl && step > 0.0 {
        value = (value / step).round() * step;
    }
    value
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
    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn gesture_active(&self) -> bool {
        self.drag_origin.is_some() || self.slider_dragging
    }

    fn cancel_gesture(&mut self, widget: &mut Widget, emit: &mut Vec<UiMessage>) -> bool {
        if !self.gesture_active() {
            return false;
        }
        let original = self.gesture_origin.unwrap_or(self.value);
        self.drag_origin = None;
        self.gesture_origin = None;
        self.scrubbing = false;
        self.slider_dragging = false;
        if original != self.value {
            self.value = original;
            // Restoration is live rather than committed: cancelling must not
            // create an undo step for a gesture the author declined.
            emit.push(NumericFieldMessage::value_changing(widget.handle, original));
        }
        widget.invalidate_layout();
        true
    }

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

    fn numeric_value(&self) -> Option<f32> {
        Some(self.value)
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

        // Phase 27-G: this embedded scrub slider was missed by the 27-D widget
        // sweep and still drew flat bars. It now matches the standalone
        // `Slider` exactly — recessed capsule track, accent-gradient fill,
        // lifted handle — so a scrub control reads the same wherever it appears.
        let tk = theme::active();
        let mid_y = slider.y + slider.h * 0.5;
        let track_r = TRACK_H * 0.5;
        let track = Rect::new(slider.x, mid_y - track_r, slider.w, TRACK_H);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(track, tk.semantic.surface.input.bytes())
                .with_radius(track_r),
            None,
        );
        ctx.push_primitive(
            crate::primitive::Primitive::inset_shadow(
                track,
                [track_r; 4],
                tk.inset.input.blur,
                tk.inset.input.color.bytes(),
            ),
            None,
        );

        let usable = (slider.w - HANDLE_W).max(1.0);
        let handle_x = slider.x + t * usable;
        let filled_w = (handle_x - slider.x).max(0.0);
        if filled_w > 0.0 {
            let g = tk.gradient.rail_accent;
            ctx.push_primitive(
                crate::primitive::Primitive::fill(
                    Rect::new(slider.x, mid_y - track_r, filled_w, TRACK_H),
                    g.from.bytes(),
                )
                .with_radius(track_r)
                .with_gradient(g.to.bytes(), g.axis),
                None,
            );
        }
        let handle = Rect::new(handle_x, slider.y + 3.0, HANDLE_W, slider.h - 6.0);
        let handle_r = (HANDLE_W * 0.5).min(tk.geometry.radius_popup);
        ctx.push_drop_shadow_rounded(handle, [handle_r; 4], tk.elevation.raised);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(handle, tk.semantic.accent.default.bytes())
                .with_radius(handle_r),
            None,
        );

        let paint = crate::style::input(crate::style::VisualState::rest().focused(self.focused));
        ctx.push_paint(field, &paint);
        // Phase 27-G: the unit sits at the right edge, muted, and is dropped
        // while editing so it can never be mistaken for part of the text being
        // typed.
        if !self.unit.is_empty() && self.editing_text.is_none() {
            let uw = ctx
                .font_atlas
                .measure_text(self.unit, self.px - 1.0, self.font_id)
                .x;
            ctx.push_text(
                self.unit,
                Vec2::new(field.x + field.w - uw - 5.0, field.y + 3.5),
                self.font_id,
                self.px - 1.0,
                tk.semantic.text.muted.bytes(),
            );
        }

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
                    self.gesture_origin = Some(self.value);
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
                WidgetMessage::MouseMove { pos, mods } => {
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
                            let v = scrub_value(start_value, dx, self.drag_step, mods);
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
                    self.gesture_origin = None;
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
                WidgetMessage::KeyDown(key, _) => {
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
    unit: &'static str,
    value: f32,
    px: f32,
    color: [u8; 4],
    font_id: u8,
    drag_step: f32,
    slider_range: Option<(f32, f32)>,
}

impl NumericFieldBuilder {
    /// Unit shown after the value: `"m"`, `"°"`, `"×"`. Empty is unitless.
    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    pub fn new(widget: WidgetBuilder) -> Self {
        // Phase 26-Zeta-D: numeric values default to the mono_strong role.
        // fontdue applies no OpenType features, so the tabular figures the
        // token sheet asks for come from the face rather than from `tnum` —
        // JetBrains Mono's digits are all one advance wide, which is what
        // stops a row twitching under a scrub.
        let style = crate::typography::text_style(crate::typography::TextRole::MonoStrong);
        Self {
            widget,
            unit: "",
            value: 0.0,
            px: style.px,
            color: style.color,
            font_id: style.font_id(),
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
                unit: self.unit,
                editing_text: None,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
                focused: false,
                select_all: false,
                drag_step: self.drag_step,
                drag_origin: None,
                gesture_origin: None,
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

    #[test]
    fn shift_and_alt_are_precision_scrub_modifiers_and_ctrl_snaps() {
        let ordinary = scrub_value(10.0, 7.0, 0.5, Modifiers::default());
        let shift = scrub_value(
            10.0,
            7.0,
            0.5,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        );
        let alt = scrub_value(
            10.0,
            7.0,
            0.5,
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
        );
        let snapped = scrub_value(
            10.0,
            1.2,
            0.5,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        );
        assert_eq!(ordinary, 13.5);
        assert!((shift - 10.35).abs() < 1.0e-5);
        assert_eq!(shift, alt);
        assert_eq!(snapped, 10.5);
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::ui::UserInterface;

    /// Glyph quads drawn by a field with the given unit.
    ///
    /// Asserted through the draw list because `Control` exposes no downcast, so
    /// there is no way to read the field's state back directly.
    /// Load all five cuts, in the order the editor loads them.
    ///
    /// `typography::REGISTRY` is a process-global `OnceLock`: once any test has
    /// called the editor's `load_fonts`, every role resolves to the id that
    /// mapping assigns. A fixture that registers only two faces then asks for
    /// `MonoStrong` gets an id that does not exist, `get_or_rasterize` returns
    /// `None`, and the field renders **zero** glyphs — which passes in isolation
    /// and fails in the full suite, purely on test order.
    fn load_all_cuts(ui: &mut UserInterface) {
        for cut in [
            include_bytes!("../../assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../assets/fonts/Inter-Medium.ttf").as_slice(),
            include_bytes!("../../assets/fonts/Inter-SemiBold.ttf").as_slice(),
            include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
            include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf").as_slice(),
        ] {
            ui.add_font(cut).expect("bundled OFL cut parses");
        }
    }

    fn glyph_count(unit: &'static str) -> usize {
        let mut ui = UserInterface::new(400.0, 60.0);
        load_all_cuts(&mut ui);
        let root = ui.root();
        let field =
            NumericFieldBuilder::new(WidgetBuilder::new().with_width(200.0).with_height(22.0))
                .with_unit(unit)
                .build();
        ui.add_node(field, root);
        ui.perform_layout();
        ui.draw();
        ui.draw_ctx
            .instances
            .iter()
            .filter(|p| p.flags & crate::primitive::FLAG_TEXT != 0)
            .count()
    }

    #[test]
    fn a_unit_adds_exactly_its_own_glyphs() {
        // "0.000" either way; the unit is the only difference.
        let bare = glyph_count("");
        let metres = glyph_count("m");
        assert_eq!(metres, bare + 1, "one unit character, one extra glyph");
    }

    #[test]
    fn a_field_is_unitless_by_default() {
        let mut ui = UserInterface::new(400.0, 60.0);
        load_all_cuts(&mut ui);
        let root = ui.root();
        let field =
            NumericFieldBuilder::new(WidgetBuilder::new().with_width(200.0).with_height(22.0))
                .build();
        ui.add_node(field, root);
        ui.perform_layout();
        ui.draw();
        let drawn = ui
            .draw_ctx
            .instances
            .iter()
            .filter(|p| p.flags & crate::primitive::FLAG_TEXT != 0)
            .count();
        assert_eq!(drawn, glyph_count(""), "the default must add nothing");
        assert!(drawn > 0, "the value itself must still render");
    }
}
