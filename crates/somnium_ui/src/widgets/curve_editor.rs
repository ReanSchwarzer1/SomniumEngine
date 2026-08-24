//! Phase CONTROL-K: the curve editor, and the compact form of it that sits in
//! a `PropertyRow`.
//!
//! # Reference architecture
//!
//! - `example_repo/fyrox/Fyrox-master/fyrox-ui/src/curve/mod.rs` — the
//!   view-transform-per-widget model (zoom and pan are widget state, not
//!   authored data), key hit-testing by screen radius rather than by domain
//!   distance, and the right-click interpolation cycle.
//! - `example_repo/flax/FlaxEngine-master/Source/Editor/GUI/CurveEditor.cs` —
//!   the "keys are always sorted, selection is re-found after every mutation"
//!   rule, which is the difference between dragging a key past its neighbour
//!   and losing the drag halfway through.
//!
//! # The two rules this widget is judged by
//!
//! **Live.** Every drag emits [`CurveEditorMessage::Value`] with `live: true`
//! on every move and once more with `live: false` on release. That is the same
//! drag-scrub convention `NumericField` uses, so a curve coalesces into one
//! undo entry through the existing gesture machinery and drives its consumer
//! on the frame it is edited. Phase CONTROL §5.3 names a "Refresh Settings"
//! button as the anti-pattern; there is no refresh step anywhere in this file.
//!
//! **The widget never owns the curve.** It draws the curve it was last told
//! about and reports what the user did to it. `SetValue` overwrites its copy
//! without echoing, exactly as `Slider::SetValue` does, or the two would loop.

use glam::Vec2;
use somnium_ecs::curve::{Curve, CurveKey, Interpolation};

use crate::primitive::Primitive;
use crate::theme;
use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, MouseButton, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};

/// Screen radius, in logical pixels, within which a click grabs a key.
const KEY_GRAB_PX: f32 = 7.0;
/// Half-width of a drawn key marker.
const KEY_HALF: f32 = 3.5;
/// Horizontal step, in pixels, between polyline samples.
const SAMPLE_STEP_PX: f32 = 2.0;
/// Inset from the widget's bounds to the plotting area.
const PAD: f32 = 5.0;
/// Milliseconds within which a second press counts as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;
/// Grid divisions on each axis.
const GRID_DIVS: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum CurveEditorMessage {
    /// The user changed the curve. `live` follows the drag-scrub convention:
    /// true while a gesture is in progress, false exactly once at its end.
    Value { curve: Curve, live: bool },
    /// Replace the displayed curve without emitting anything back.
    SetValue(Curve),
}

impl CurveEditorMessage {
    /// Push a new curve into the widget.
    #[must_use]
    pub fn set_value(dest: NodeHandle, curve: Curve) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetValue(curve))
    }
}

/// What a press started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    None,
    /// Dragging the key at this index.
    MoveKey(usize),
    /// Panning the view.
    Pan,
}

pub struct CurveEditor {
    curve: Curve,
    /// Visible time window.
    view_t: (f32, f32),
    /// Visible value window.
    view_v: (f32, f32),
    /// The domain the curve is authored over — what a reset zoom returns to,
    /// and the range a double-click's new key is placed within.
    domain_t: (f32, f32),
    domain_v: (f32, f32),
    selected: Option<usize>,
    gesture: Gesture,
    /// Pointer position when the current gesture began, for panning.
    anchor: Vec2,
    anchor_view_t: (f32, f32),
    anchor_view_v: (f32, f32),
    last_press: Option<(std::time::Instant, Vec2)>,
    font_id: u8,
    height: f32,
    /// Snap increment used while `Ctrl` is held.
    snap_t: f32,
    snap_v: f32,
}

impl CurveEditor {
    /// The plotting area inside the widget's border.
    fn plot(bounds: Rect) -> Rect {
        Rect::new(
            bounds.x + PAD,
            bounds.y + PAD,
            (bounds.w - PAD * 2.0).max(1.0),
            (bounds.h - PAD * 2.0).max(1.0),
        )
    }

    fn to_screen(&self, plot: Rect, t: f32, v: f32) -> Vec2 {
        let (t0, t1) = self.view_t;
        let (v0, v1) = self.view_v;
        let u = if (t1 - t0).abs() > f32::EPSILON {
            (t - t0) / (t1 - t0)
        } else {
            0.0
        };
        let w = if (v1 - v0).abs() > f32::EPSILON {
            (v - v0) / (v1 - v0)
        } else {
            0.0
        };
        Vec2::new(plot.x + u * plot.w, plot.y + (1.0 - w) * plot.h)
    }

    fn to_domain(&self, plot: Rect, pos: Vec2) -> (f32, f32) {
        let (t0, t1) = self.view_t;
        let (v0, v1) = self.view_v;
        let u = ((pos.x - plot.x) / plot.w).clamp(-1.0, 2.0);
        let w = 1.0 - ((pos.y - plot.y) / plot.h).clamp(-1.0, 2.0);
        (t0 + u * (t1 - t0), v0 + w * (v1 - v0))
    }

    /// Index of the key nearest `pos`, if one is within [`KEY_GRAB_PX`].
    fn key_at(&self, plot: Rect, pos: Vec2) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, key) in self.curve.keys().iter().enumerate() {
            let p = self.to_screen(plot, key.t, key.v);
            let d = p.distance(pos);
            if d <= KEY_GRAB_PX && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Snap a domain point when `Ctrl` is held.
    fn snapped(&self, t: f32, v: f32, snap: bool) -> (f32, f32) {
        if !snap {
            return (t, v);
        }
        let q = |x: f32, step: f32| {
            if step > 0.0 {
                (x / step).round() * step
            } else {
                x
            }
        };
        (q(t, self.snap_t), q(v, self.snap_v))
    }

    fn emit(&self, widget: &Widget, emit: &mut Vec<UiMessage>, live: bool) {
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            CurveEditorMessage::Value {
                curve: self.curve.clone(),
                live,
            },
        ));
    }

    /// A preset, rescaled from its unit shape into this field's declared
    /// domain and range. A preset that ignored the range would put a light
    /// intensity curve between 0 and 1 lux.
    fn preset(&self, index: usize) -> Curve {
        let (_, build) = Curve::PRESETS[index.min(Curve::PRESETS.len() - 1)];
        let (t0, t1) = self.domain_t;
        let (v0, v1) = self.domain_v;
        Curve::from_keys(
            build()
                .keys()
                .iter()
                .map(|key| CurveKey {
                    t: t0 + key.t * (t1 - t0),
                    v: v0 + key.v * (v1 - v0),
                    // Tangents are value-per-time, so rescaling both axes
                    // rescales the slope by their ratio.
                    in_tangent: key.in_tangent * (v1 - v0) / (t1 - t0).max(f32::EPSILON),
                    out_tangent: key.out_tangent * (v1 - v0) / (t1 - t0).max(f32::EPSILON),
                    interpolation: key.interpolation,
                })
                .collect(),
        )
    }

    /// Reset the view to the authored domain, padded so end keys are not on
    /// the border.
    fn frame_domain(&mut self) {
        let (t0, t1) = self.domain_t;
        let (v0, v1) = self.domain_v;
        let tp = ((t1 - t0) * 0.04).max(f32::EPSILON);
        let vp = ((v1 - v0) * 0.08).max(f32::EPSILON);
        self.view_t = (t0 - tp, t1 + tp);
        self.view_v = (v0 - vp, v1 + vp);
    }
}

impl Control for CurveEditor {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(available.x.max(120.0), self.height)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        let plot = Self::plot(b);

        ctx.push_primitive(
            Primitive::fill(b, t.semantic.surface.input.bytes())
                .with_radius(t.geometry.radius_input)
                .with_border(1.0, t.semantic.border.default.bytes()),
            None,
        );

        // Grid. Thin filled rects rather than lines because the primitive
        // pipeline has no line topology — Phase 27 §6 froze the quad layout,
        // and a curve editor is not a reason to fork it.
        let grid = t.semantic.border.subtle.bytes();
        for i in 1..GRID_DIVS {
            #[allow(clippy::cast_precision_loss)]
            let f = i as f32 / GRID_DIVS as f32;
            ctx.push_rect_filled(Rect::new(plot.x + plot.w * f, plot.y, 1.0, plot.h), grid);
            ctx.push_rect_filled(Rect::new(plot.x, plot.y + plot.h * f, plot.w, 1.0), grid);
        }

        if self.curve.is_empty() {
            ctx.push_text(
                "Double-click to add a key",
                Vec2::new(plot.x + 6.0, plot.y + plot.h * 0.5 - 5.0),
                self.font_id,
                11.0,
                t.semantic.text.muted.bytes(),
            );
            return;
        }

        // The curve itself, as a run of vertical bars joining consecutive
        // samples. Two pixels per sample is below the eye's threshold for a
        // smooth line at this size and costs ~150 quads for a full-width row.
        let line = t.semantic.accent.default.bytes();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = ((plot.w / SAMPLE_STEP_PX).ceil() as usize).max(2);
        let mut prev: Option<Vec2> = None;
        for i in 0..=steps {
            #[allow(clippy::cast_precision_loss)]
            let u = i as f32 / steps as f32;
            let time = self.view_t.0 + (self.view_t.1 - self.view_t.0) * u;
            let p = self.to_screen(plot, time, self.curve.evaluate(time));
            if let Some(q) = prev {
                let y0 = q.y.min(p.y).clamp(plot.y, plot.y + plot.h);
                let y1 = q.y.max(p.y).clamp(plot.y, plot.y + plot.h);
                ctx.push_rect_filled(
                    Rect::new(q.x, y0, (p.x - q.x).max(1.0), (y1 - y0).max(1.5)),
                    line,
                );
            }
            prev = Some(p);
        }

        // Keys on top, so a key never disappears under its own curve.
        for (i, key) in self.curve.keys().iter().enumerate() {
            let p = self.to_screen(plot, key.t, key.v);
            if p.x < plot.x - KEY_HALF || p.x > plot.x + plot.w + KEY_HALF {
                continue;
            }
            let selected = self.selected == Some(i);
            let half = if selected { KEY_HALF + 1.0 } else { KEY_HALF };
            let fill = if selected {
                t.semantic.text.primary.bytes()
            } else {
                t.semantic.accent.default.bytes()
            };
            let rect = Rect::new(p.x - half, p.y - half, half * 2.0, half * 2.0);
            // A stepped key reads as a square, a smooth one as a dot, a linear
            // one in between: the interpolation mode has to be visible without
            // opening anything, or right-click-to-cycle is a guessing game.
            let radius = match key.interpolation {
                Interpolation::Step => 0.0,
                Interpolation::Linear => half * 0.4,
                Interpolation::Smooth => half,
            };
            ctx.push_primitive(
                Primitive::fill(rect, fill)
                    .with_radius(radius)
                    .with_border(1.0, t.semantic.surface.input.bytes()),
                None,
            );
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        CursorKind::Default
    }

    /// Claim the keyboard **only while a key is selected**.
    ///
    /// Focused widgets that report true here swallow every key before the game
    /// sees it, which is right for a `TextBox` and wrong for a curve nobody is
    /// editing: a curve row that had been clicked once went on eating WASD, and
    /// the symptom was the fly-cam not responding. With a selection the
    /// keyboard genuinely belongs here — `Delete` removes, the arrows nudge,
    /// the digits apply a preset — and without one there is nothing to take.
    fn is_text_input(&self) -> bool {
        self.selected.is_some()
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(CurveEditorMessage::SetValue(curve)) = msg.data::<CurveEditorMessage>() {
            if self.gesture == Gesture::None {
                self.curve = curve.clone();
                if self.selected.is_some_and(|i| i >= self.curve.len()) {
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
        let plot = Self::plot(bounds);

        match &wmsg {
            WidgetMessage::MouseDown { pos, button, mods } => {
                msg.handled = true;
                match button {
                    MouseButton::Middle => {
                        self.gesture = Gesture::Pan;
                        self.anchor = *pos;
                        self.anchor_view_t = self.view_t;
                        self.anchor_view_v = self.view_v;
                    }
                    MouseButton::Right => {
                        // Cycle the interpolation of the key under the cursor.
                        // Nothing under the cursor frames the domain instead,
                        // which is the cheapest way out of a lost view.
                        if let Some(index) = self.key_at(plot, *pos) {
                            if let Some(key) = self.curve.key_mut(index) {
                                key.interpolation = key.interpolation.cycled();
                            }
                            self.selected = Some(index);
                            self.emit(widget, emit, false);
                        } else {
                            self.frame_domain();
                        }
                    }
                    _ => {
                        let now = std::time::Instant::now();
                        let double = self.last_press.is_some_and(|(at, p)| {
                            now.duration_since(at).as_millis() <= DOUBLE_CLICK_MS
                                && p.distance(*pos) < 6.0
                        });
                        self.last_press = Some((now, *pos));

                        if let Some(index) = self.key_at(plot, *pos) {
                            if double {
                                // Double-click on a key removes it. The last
                                // key is removable too: an empty curve is a
                                // legitimate authored state.
                                self.curve.remove(index);
                                self.selected = None;
                                self.emit(widget, emit, false);
                            } else {
                                self.selected = Some(index);
                                self.gesture = Gesture::MoveKey(index);
                            }
                        } else if double {
                            let (t, v) = self.to_domain(plot, *pos);
                            let (t, v) = self.snapped(t, v, mods.ctrl);
                            let index = self.curve.insert(CurveKey {
                                interpolation: Interpolation::Smooth,
                                ..CurveKey::new(t, v)
                            });
                            self.selected = Some(index);
                            self.gesture = Gesture::MoveKey(index);
                            self.emit(widget, emit, false);
                        } else {
                            self.selected = None;
                        }
                    }
                }
            }
            WidgetMessage::MouseMove { pos, mods } => match self.gesture {
                Gesture::MoveKey(index) => {
                    let (t, v) = self.to_domain(plot, *pos);
                    let (t, v) = self.snapped(t, v, mods.ctrl);
                    // Shift is the precision modifier everywhere else in this
                    // editor; here it locks the key's time so a value can be
                    // nudged without the key sliding along the track.
                    let t = if mods.shift {
                        self.curve.keys().get(index).map_or(t, |k| k.t)
                    } else {
                        t
                    };
                    let moved = self.curve.move_key(index, t, v);
                    self.gesture = Gesture::MoveKey(moved);
                    self.selected = Some(moved);
                    self.emit(widget, emit, true);
                    msg.handled = true;
                }
                Gesture::Pan => {
                    let d = *pos - self.anchor;
                    let (t0, t1) = self.anchor_view_t;
                    let (v0, v1) = self.anchor_view_v;
                    let dt = -d.x / plot.w * (t1 - t0);
                    let dv = d.y / plot.h * (v1 - v0);
                    self.view_t = (t0 + dt, t1 + dt);
                    self.view_v = (v0 + dv, v1 + dv);
                    msg.handled = true;
                }
                Gesture::None => {}
            },
            WidgetMessage::MouseUp { .. } => {
                if matches!(self.gesture, Gesture::MoveKey(_)) {
                    self.emit(widget, emit, false);
                }
                self.gesture = Gesture::None;
                msg.handled = true;
            }
            WidgetMessage::MouseWheel { pos, delta, mods } => {
                // Zoom about the cursor, so the point under the pointer stays
                // put. Ctrl zooms the value axis instead of time — one wheel,
                // two axes, the convention Fyrox and Flax both use.
                let scale = if *delta > 0.0 { 0.85 } else { 1.0 / 0.85 };
                if mods.ctrl {
                    let (v0, v1) = self.view_v;
                    let anchor = v1 - (pos.y - plot.y) / plot.h * (v1 - v0);
                    self.view_v = (
                        anchor + (v0 - anchor) * scale,
                        anchor + (v1 - anchor) * scale,
                    );
                } else {
                    let (t0, t1) = self.view_t;
                    let anchor = t0 + (pos.x - plot.x) / plot.w * (t1 - t0);
                    self.view_t = (
                        anchor + (t0 - anchor) * scale,
                        anchor + (t1 - anchor) * scale,
                    );
                }
                msg.handled = true;
            }
            WidgetMessage::KeyDown(key, mods) => match key {
                KeyCode::Delete | KeyCode::Backspace => {
                    if let Some(index) = self.selected.take() {
                        self.curve.remove(index);
                        self.emit(widget, emit, false);
                    }
                    msg.handled = true;
                }
                KeyCode::KeyF => {
                    self.frame_domain();
                    msg.handled = true;
                }
                // Presets, on the digits, scaled into the field's declared
                // value range. Keyed rather than menued because a curve row is
                // 96 px tall and a preset dropdown inside it would cost more
                // of that than the curve.
                KeyCode::Digit1
                | KeyCode::Digit2
                | KeyCode::Digit3
                | KeyCode::Digit4
                | KeyCode::Digit5 => {
                    let index = match key {
                        KeyCode::Digit1 => 0,
                        KeyCode::Digit2 => 1,
                        KeyCode::Digit3 => 2,
                        KeyCode::Digit4 => 3,
                        _ => 4,
                    };
                    self.curve = self.preset(index);
                    self.selected = None;
                    self.emit(widget, emit, false);
                    msg.handled = true;
                }
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    if let Some(index) = self.selected {
                        let span = (self.view_v.1 - self.view_v.0).abs();
                        let nudge = if mods.shift {
                            span * 0.002
                        } else {
                            span * 0.02
                        };
                        let sign = if *key == KeyCode::ArrowUp { 1.0 } else { -1.0 };
                        if let Some(k) = self.curve.keys().get(index).copied() {
                            self.curve.move_key(index, k.t, k.v + nudge * sign);
                            self.emit(widget, emit, false);
                        }
                    }
                    msg.handled = true;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

pub struct CurveEditorBuilder {
    widget: WidgetBuilder,
    curve: Curve,
    domain_t: (f32, f32),
    domain_v: (f32, f32),
    font_id: u8,
    height: f32,
}

impl CurveEditorBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            curve: Curve::empty(),
            domain_t: (0.0, 1.0),
            domain_v: (0.0, 1.0),
            font_id: 0,
            height: 96.0,
        }
    }

    #[must_use]
    pub fn with_curve(mut self, curve: Curve) -> Self {
        self.curve = curve;
        self
    }

    /// The authored time domain — usually a field's `soft_min`/`soft_max`.
    #[must_use]
    pub fn with_domain(mut self, t0: f32, t1: f32) -> Self {
        if t1 > t0 {
            self.domain_t = (t0, t1);
        }
        self
    }

    /// The authored value range — usually a field's `min`/`max`.
    #[must_use]
    pub fn with_range(mut self, v0: f32, v1: f32) -> Self {
        if v1 > v0 {
            self.domain_v = (v0, v1);
        }
        self
    }

    #[must_use]
    pub fn with_font_id(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn with_height(mut self, height: f32) -> Self {
        self.height = height.max(32.0);
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let mut editor = CurveEditor {
            curve: self.curve,
            view_t: self.domain_t,
            view_v: self.domain_v,
            domain_t: self.domain_t,
            domain_v: self.domain_v,
            selected: None,
            gesture: Gesture::None,
            anchor: Vec2::ZERO,
            anchor_view_t: self.domain_t,
            anchor_view_v: self.domain_v,
            last_press: None,
            font_id: self.font_id,
            height: self.height,
            snap_t: (self.domain_t.1 - self.domain_t.0) / 24.0,
            snap_v: (self.domain_v.1 - self.domain_v.0) / 20.0,
        };
        editor.frame_domain();
        UiNode::new(self.widget.build(), Box::new(editor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> CurveEditor {
        let node = CurveEditorBuilder::new(WidgetBuilder::new())
            .with_curve(Curve::ramp(0.0, 1.0))
            .build();
        // The builder returns a boxed control; rebuild the same state directly
        // so the tests can reach the mapping helpers.
        drop(node);
        let mut e = CurveEditor {
            curve: Curve::ramp(0.0, 1.0),
            view_t: (0.0, 1.0),
            view_v: (0.0, 1.0),
            domain_t: (0.0, 1.0),
            domain_v: (0.0, 1.0),
            selected: None,
            gesture: Gesture::None,
            anchor: Vec2::ZERO,
            anchor_view_t: (0.0, 1.0),
            anchor_view_v: (0.0, 1.0),
            last_press: None,
            font_id: 0,
            height: 96.0,
            snap_t: 0.25,
            snap_v: 0.25,
        };
        e.view_t = (0.0, 1.0);
        e.view_v = (0.0, 1.0);
        e
    }

    fn plot() -> Rect {
        Rect::new(0.0, 0.0, 100.0, 100.0)
    }

    #[test]
    fn screen_and_domain_are_inverses() {
        let e = editor();
        let p = e.to_screen(plot(), 0.25, 0.75);
        let (t, v) = e.to_domain(plot(), p);
        assert!((t - 0.25).abs() < 1e-4, "t {t}");
        assert!((v - 0.75).abs() < 1e-4, "v {v}");
    }

    #[test]
    fn value_increases_upward_on_screen() {
        // The one axis that is easy to get backwards, and the mistake is not
        // visible in a symmetric test curve.
        let e = editor();
        let low = e.to_screen(plot(), 0.5, 0.0);
        let high = e.to_screen(plot(), 0.5, 1.0);
        assert!(high.y < low.y);
    }

    #[test]
    fn a_click_grabs_the_nearest_key_within_the_radius() {
        let e = editor();
        let p = e.to_screen(plot(), 0.0, 0.0);
        assert_eq!(e.key_at(plot(), p), Some(0));
        assert_eq!(
            e.key_at(plot(), p + Vec2::new(KEY_GRAB_PX + 2.0, 0.0)),
            None
        );
    }

    #[test]
    fn snapping_quantises_only_when_asked() {
        let e = editor();
        assert_eq!(e.snapped(0.31, 0.31, false), (0.31, 0.31));
        let (t, v) = e.snapped(0.31, 0.31, true);
        assert!((t - 0.25).abs() < 1e-5, "t {t}");
        assert!((v - 0.25).abs() < 1e-5, "v {v}");
    }

    #[test]
    fn framing_pads_the_domain_so_end_keys_are_not_on_the_border() {
        let mut e = editor();
        e.frame_domain();
        assert!(e.view_t.0 < 0.0 && e.view_t.1 > 1.0);
        assert!(e.view_v.0 < 0.0 && e.view_v.1 > 1.0);
    }

    #[test]
    fn a_preset_lands_inside_the_declared_range() {
        // A field declaring 0..100 000 lux must not receive a curve authored
        // between zero and one.
        let mut e = editor();
        e.domain_t = (0.0, 24.0);
        e.domain_v = (0.0, 100_000.0);
        let curve = e.preset(0);
        assert!((curve.evaluate(0.0) - 0.0).abs() < 1e-3);
        assert!((curve.evaluate(24.0) - 100_000.0).abs() < 1.0);
    }

    #[test]
    fn dragging_a_key_past_its_neighbour_keeps_the_drag() {
        // Flax's rule. Without re-finding the index after the sort, the drag
        // silently transfers to whichever key ended up at the old position.
        let mut e = editor();
        e.curve = Curve::from_keys(vec![
            CurveKey::new(0.0, 0.0),
            CurveKey::new(0.5, 0.5),
            CurveKey::new(1.0, 1.0),
        ]);
        let moved = e.curve.move_key(1, 1.5, 0.5);
        assert_eq!(moved, 2, "the dragged key is now last");
        assert_eq!(e.curve.keys()[2].v, 0.5);
    }
}
