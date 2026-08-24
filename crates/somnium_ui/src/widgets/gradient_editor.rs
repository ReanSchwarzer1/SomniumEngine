//! Phase CONTROL-K: the colour-ramp editor.
//!
//! Deliberately *not* a second colour picker. A stop's colour is edited by the
//! `ColorPicker` the editor already has: activating a stop emits
//! [`GradientEditorMessage::StopActivated`], the shell opens the existing
//! popup anchored to this widget, and the colour comes back through the same
//! `ColorPickerMessage` path every other swatch uses. Phase CONTROL §3 refuses
//! a second reflection system for the same reason it refuses a second colour
//! surface: two descriptions of one thing drift.
//!
//! # Reference architecture
//!
//! - `example_repo/godot/godot-master/editor/gui/editor_spin_slider.cpp` and
//!   Godot's `GradientEditor` — the stop-handle strip below the ramp, and the
//!   rule that a click on the ramp itself adds a stop with the colour that was
//!   already there, so adding a stop never changes the gradient.
//! - Unity's `GradientEditor` — alpha shown against a checkerboard, because a
//!   fading ramp over a flat background is indistinguishable from a darkening
//!   one.

use glam::Vec2;
use somnium_ecs::curve::{Gradient, GradientStop};

use crate::primitive::Primitive;
use crate::theme;
use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, MouseButton, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};

/// Height of the ramp strip.
const RAMP_H: f32 = 22.0;
/// Height of the handle strip below it.
const HANDLE_H: f32 = 12.0;
/// Half-width of a stop handle.
const HANDLE_HALF: f32 = 5.0;
/// Checkerboard cell size, in pixels.
const CHECKER: f32 = 6.0;
/// Horizontal step, in pixels, between ramp samples.
const SAMPLE_STEP_PX: f32 = 3.0;
/// Milliseconds within which a second press counts as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;

#[derive(Debug, Clone, PartialEq)]
pub enum GradientEditorMessage {
    /// The user changed the gradient. `live` follows the drag-scrub
    /// convention.
    Value { gradient: Gradient, live: bool },
    /// Replace the displayed gradient without emitting anything back.
    SetValue(Gradient),
    /// A stop was double-clicked and wants the shared colour picker. Carries
    /// the stop index and its current linear RGBA.
    StopActivated { index: usize, color: [f32; 4] },
}

impl GradientEditorMessage {
    /// Push a new gradient into the widget.
    #[must_use]
    pub fn set_value(dest: NodeHandle, gradient: Gradient) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetValue(gradient))
    }
}

pub struct GradientEditor {
    gradient: Gradient,
    selected: Option<usize>,
    dragging: Option<usize>,
    last_press: Option<(std::time::Instant, Vec2)>,
    font_id: u8,
}

impl GradientEditor {
    fn ramp_rect(bounds: Rect) -> Rect {
        Rect::new(bounds.x, bounds.y, bounds.w.max(1.0), RAMP_H)
    }

    fn handle_strip(bounds: Rect) -> Rect {
        Rect::new(bounds.x, bounds.y + RAMP_H, bounds.w.max(1.0), HANDLE_H)
    }

    fn stop_x(ramp: Rect, t: f32) -> f32 {
        ramp.x + t.clamp(0.0, 1.0) * ramp.w
    }

    fn t_at(ramp: Rect, x: f32) -> f32 {
        ((x - ramp.x) / ramp.w.max(1.0)).clamp(0.0, 1.0)
    }

    /// Index of the stop handle nearest `x`, if one is within grabbing range.
    fn stop_at(&self, ramp: Rect, x: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, stop) in self.gradient.stops().iter().enumerate() {
            let d = (Self::stop_x(ramp, stop.t) - x).abs();
            if d <= HANDLE_HALF + 2.0 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// The currently selected stop's index, if it is still in range.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected.filter(|i| *i < self.gradient.len())
    }

    fn emit(&self, widget: &Widget, emit: &mut Vec<UiMessage>, live: bool) {
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            GradientEditorMessage::Value {
                gradient: self.gradient.clone(),
                live,
            },
        ));
    }
}

/// Convert a linear RGBA sample to the sRGB bytes the paint layer expects.
///
/// The gradient stores linear values because everything downstream of the
/// editor is linear; the swatch has to encode on the way to the screen or a
/// mid-grey stop paints as a much darker one.
fn to_srgb8(linear: [f32; 4]) -> [u8; 4] {
    fn channel(c: f32) -> u8 {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (s * 255.0 + 0.5) as u8
        }
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    [
        channel(linear[0]),
        channel(linear[1]),
        channel(linear[2]),
        (linear[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
    ]
}

impl Control for GradientEditor {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(available.x.max(120.0), RAMP_H + HANDLE_H)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        let ramp = Self::ramp_rect(b);

        // Checkerboard first, so alpha reads as transparency rather than as a
        // darker colour.
        let light = [0x50, 0x50, 0x50, 0xFF];
        let dark = [0x38, 0x38, 0x38, 0xFF];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cols = (ramp.w / CHECKER).ceil() as usize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = (ramp.h / CHECKER).ceil() as usize;
        for row in 0..rows {
            for col in 0..cols {
                #[allow(clippy::cast_precision_loss)]
                let x = ramp.x + col as f32 * CHECKER;
                #[allow(clippy::cast_precision_loss)]
                let y = ramp.y + row as f32 * CHECKER;
                let w = CHECKER.min(ramp.x + ramp.w - x);
                let h = CHECKER.min(ramp.y + ramp.h - y);
                let color = if (row + col) % 2 == 0 { light } else { dark };
                ctx.push_rect_filled(Rect::new(x, y, w, h), color);
            }
        }

        // The ramp, as columns. Three pixels each: a ramp is smooth by
        // construction, so this is indistinguishable from per-pixel and costs
        // a third of the quads.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = ((ramp.w / SAMPLE_STEP_PX).ceil() as usize).max(1);
        for i in 0..steps {
            #[allow(clippy::cast_precision_loss)]
            let u = (i as f32 + 0.5) / steps as f32;
            #[allow(clippy::cast_precision_loss)]
            let x = ramp.x + i as f32 * SAMPLE_STEP_PX;
            let w = SAMPLE_STEP_PX.min(ramp.x + ramp.w - x);
            if w <= 0.0 {
                break;
            }
            ctx.push_rect_filled(
                Rect::new(x, ramp.y, w, ramp.h),
                to_srgb8(self.gradient.evaluate(u)),
            );
        }
        ctx.push_rect_border(ramp, 1.0, t.semantic.border.default.bytes());

        if self.gradient.is_empty() {
            ctx.push_text(
                "Double-click to add a stop",
                Vec2::new(ramp.x + 6.0, ramp.y + ramp.h * 0.5 - 5.0),
                self.font_id,
                11.0,
                t.semantic.text.muted.bytes(),
            );
        }

        // Handles.
        let strip = Self::handle_strip(b);
        for (i, stop) in self.gradient.stops().iter().enumerate() {
            let x = Self::stop_x(ramp, stop.t);
            let selected = self.selected == Some(i);
            let rect = Rect::new(
                x - HANDLE_HALF,
                strip.y + 1.0,
                HANDLE_HALF * 2.0,
                strip.h - 2.0,
            );
            ctx.push_primitive(
                Primitive::fill(rect, to_srgb8([stop.color[0], stop.color[1], stop.color[2], 1.0]))
                    .with_radius(2.0)
                    .with_border(
                        if selected { 2.0 } else { 1.0 },
                        if selected {
                            t.semantic.text.primary.bytes()
                        } else {
                            t.semantic.border.strong.bytes()
                        },
                    ),
                None,
            );
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        CursorKind::EwResize
    }

    /// Claim the keyboard only while a stop is selected — see
    /// `CurveEditor::is_text_input` for why an unconditional claim is a bug.
    fn is_text_input(&self) -> bool {
        self.selected.is_some()
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(GradientEditorMessage::SetValue(gradient)) = msg.data::<GradientEditorMessage>()
        {
            if self.dragging.is_none() {
                self.gradient = gradient.clone();
                if self.selected.is_some_and(|i| i >= self.gradient.len()) {
                    self.selected = None;
                }
            }
            msg.handled = true;
            return;
        }

        // Cloned rather than borrowed: every arm below sets `msg.handled`,
        // and a borrow of `msg.data` outlives that assignment.
        let Some(wmsg) = msg.data::<WidgetMessage>().cloned() else {
            return;
        };
        let bounds = widget.screen_bounds();
        let ramp = Self::ramp_rect(bounds);

        match &wmsg {
            WidgetMessage::MouseDown { pos, button, .. } => {
                msg.handled = true;
                let now = std::time::Instant::now();
                let double = self.last_press.is_some_and(|(at, p)| {
                    now.duration_since(at).as_millis() <= DOUBLE_CLICK_MS && p.distance(*pos) < 6.0
                });
                self.last_press = Some((now, *pos));

                match self.stop_at(ramp, pos.x) {
                    Some(index) if *button == MouseButton::Left => {
                        self.selected = Some(index);
                        if double {
                            let color = self.gradient.stops()[index].color;
                            emit.push(UiMessage::new(
                                widget.handle,
                                MessageDirection::FromWidget,
                                GradientEditorMessage::StopActivated { index, color },
                            ));
                        } else {
                            self.dragging = Some(index);
                        }
                    }
                    Some(index) if *button == MouseButton::Right => {
                        self.gradient.remove(index);
                        self.selected = None;
                        self.emit(widget, emit, false);
                    }
                    _ if double && *button == MouseButton::Left => {
                        // Godot's rule: the new stop takes the colour the ramp
                        // already had there, so adding one changes nothing
                        // until it is moved or recoloured.
                        let t = Self::t_at(ramp, pos.x);
                        let color = self.gradient.evaluate(t);
                        let index = self.gradient.insert(GradientStop::new(t, color));
                        self.selected = Some(index);
                        self.dragging = Some(index);
                        self.emit(widget, emit, false);
                    }
                    _ => {}
                }
            }
            WidgetMessage::MouseMove { pos, mods } => {
                if let Some(index) = self.dragging {
                    let t = Self::t_at(ramp, pos.x);
                    // Ctrl snaps to twentieths, which is what a "stop at 25%"
                    // needs and what dragging by eye never quite reaches.
                    let t = if mods.ctrl { (t * 20.0).round() / 20.0 } else { t };
                    let moved = self.gradient.move_stop(index, t);
                    self.dragging = Some(moved);
                    self.selected = Some(moved);
                    self.emit(widget, emit, true);
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseUp { .. } => {
                if self.dragging.take().is_some() {
                    self.emit(widget, emit, false);
                }
                msg.handled = true;
            }
            WidgetMessage::KeyDown(key, _) => {
                if matches!(key, KeyCode::Delete | KeyCode::Backspace) {
                    if let Some(index) = self.selected.take() {
                        self.gradient.remove(index);
                        self.emit(widget, emit, false);
                    }
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
}

pub struct GradientEditorBuilder {
    widget: WidgetBuilder,
    gradient: Gradient,
    font_id: u8,
}

impl GradientEditorBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            gradient: Gradient::empty(),
            font_id: 0,
        }
    }

    #[must_use]
    pub fn with_gradient(mut self, gradient: Gradient) -> Self {
        self.gradient = gradient;
        self
    }

    #[must_use]
    pub fn with_font_id(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(GradientEditor {
                gradient: self.gradient,
                selected: None,
                dragging: None,
                last_press: None,
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(gradient: Gradient) -> GradientEditor {
        GradientEditor {
            gradient,
            selected: None,
            dragging: None,
            last_press: None,
            font_id: 0,
        }
    }

    fn ramp() -> Rect {
        Rect::new(0.0, 0.0, 100.0, RAMP_H)
    }

    #[test]
    fn stop_positions_and_hit_tests_agree() {
        let e = editor(Gradient::ramp([0.0; 4], [1.0; 4]));
        assert_eq!(e.stop_at(ramp(), GradientEditor::stop_x(ramp(), 0.0)), Some(0));
        assert_eq!(e.stop_at(ramp(), GradientEditor::stop_x(ramp(), 1.0)), Some(1));
        assert_eq!(e.stop_at(ramp(), 50.0), None);
    }

    #[test]
    fn t_at_clamps_to_the_ramp() {
        assert_eq!(GradientEditor::t_at(ramp(), -40.0), 0.0);
        assert_eq!(GradientEditor::t_at(ramp(), 400.0), 1.0);
        assert!((GradientEditor::t_at(ramp(), 50.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn adding_a_stop_at_the_sampled_colour_does_not_change_the_ramp() {
        // Godot's rule, and the reason it matters: a click that visibly
        // recolours the gradient is a click nobody makes twice.
        let mut gradient = Gradient::ramp([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);
        let before: Vec<[f32; 4]> = (0..11_u8)
            .map(|i| gradient.evaluate(f32::from(i) / 10.0))
            .collect();
        let color = gradient.evaluate(0.37);
        gradient.insert(GradientStop::new(0.37, color));
        for (i, expected) in before.iter().enumerate() {
            let got = gradient.evaluate(f32::from(u8::try_from(i).unwrap()) / 10.0);
            for c in 0..4 {
                assert!((got[c] - expected[c]).abs() < 1e-5, "channel {c} at {i}");
            }
        }
    }

    #[test]
    fn linear_values_are_encoded_for_the_paint_layer() {
        // Mid-grey in linear is not mid-grey on screen; the swatch has to say
        // so or every gradient in the editor reads too dark.
        let bytes = to_srgb8([0.5, 0.5, 0.5, 1.0]);
        assert!(bytes[0] > 180, "linear 0.5 encoded to {}", bytes[0]);
        assert_eq!(to_srgb8([0.0, 0.0, 0.0, 0.0]), [0, 0, 0, 0]);
        assert_eq!(to_srgb8([1.0, 1.0, 1.0, 1.0]), [255, 255, 255, 255]);
    }
}
