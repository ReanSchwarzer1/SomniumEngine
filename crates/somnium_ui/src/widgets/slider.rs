// Phase 20B: horizontal slider.
//
// Modelled on the same Control pattern as Button (ported from Fyrox's widget
// architecture, ATTRIBUTION §13.15), but needs the mouse capture added to
// `UserInterface` in this phase: a drag must keep receiving MouseMove after
// the cursor leaves the widget's bounds, and must end on MouseUp wherever that
// happens.
//
// Values are exposed on a normalized 0..1 track; callers map that to whatever
// range they need (the camera-speed control uses an exponential mapping so the
// low end stays fine-grained).

use crate::primitive::Primitive;
use crate::theme;
use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum SliderMessage {
    /// Emitted while dragging and on click-to-set. Value is in `0..=1`.
    Value(f32),
    /// Sent to the widget to update its position without emitting.
    SetValue(f32),
}

impl SliderMessage {
    pub fn set_value(dest: crate::message::NodeHandle, v: f32) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetValue(v))
    }
}

pub struct Slider {
    /// Normalized position in `0..=1`.
    pub value: f32,
    dragging: bool,
    /// Track colour.
    track: [u8; 4],
    /// Filled portion + handle colour.
    fill: [u8; 4],
}

/// Handle width in pixels.
const HANDLE_W: f32 = 8.0;
/// Track height in pixels (the widget itself is taller for an easy hit target).
const TRACK_H: f32 = 4.0;

impl Slider {
    /// Map a cursor x to a normalized value, accounting for the handle width so
    /// the extremes are actually reachable.
    fn value_at(bounds: Rect, x: f32) -> f32 {
        let usable = (bounds.w - HANDLE_W).max(1.0);
        ((x - bounds.x - HANDLE_W * 0.5) / usable).clamp(0.0, 1.0)
    }
}

impl Control for Slider {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        // Fill the width it is given; fixed comfortable height.
        Vec2::new(available.x.min(200.0), 18.0)
    }

    fn arrange_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        let mid_y = b.y + b.h * 0.5;
        let track_r = TRACK_H * 0.5;

        // Phase 27-D. The §2.4 audit called these out as two flat bars. The
        // track is now a recessed capsule, the filled range takes the accent
        // gradient, and the handle is a real lifted control.
        let track = Rect::new(b.x, mid_y - track_r, b.w, TRACK_H);
        ctx.push_primitive(
            Primitive::fill(track, self.track).with_radius(track_r),
            None,
        );
        ctx.push_primitive(
            Primitive::inset_shadow(track, [track_r; 4], t.inset.input.blur, t.inset.input.color.bytes()),
            None,
        );

        let usable = (b.w - HANDLE_W).max(1.0);
        let handle_x = b.x + self.value.clamp(0.0, 1.0) * usable;
        let filled_w = (handle_x - b.x).max(0.0);
        if filled_w > 0.0 {
            let g = t.gradient.rail_accent;
            ctx.push_primitive(
                Primitive::fill(Rect::new(b.x, mid_y - track_r, filled_w, TRACK_H), g.from.bytes())
                    .with_radius(track_r)
                    .with_gradient(g.to.bytes(), g.axis),
                None,
            );
        }

        let handle = Rect::new(handle_x, b.y + 2.0, HANDLE_W, b.h - 4.0);
        let handle_r = (HANDLE_W * 0.5).min(t.geometry.radius_popup);
        ctx.push_drop_shadow_rounded(handle, [handle_r; 4], t.elevation.raised);
        ctx.push_primitive(
            Primitive::fill(handle, self.fill).with_radius(handle_r),
            None,
        );
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> crate::node::CursorKind {
        crate::node::CursorKind::EwResize
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(SliderMessage::SetValue(v)) = msg.data::<SliderMessage>() {
            // External update (e.g. the scroll wheel changed the speed) — move
            // the handle without echoing a Value back, or the two would loop.
            self.value = v.clamp(0.0, 1.0);
            msg.handled = true;
            return;
        }

        let Some(wmsg) = msg.data::<WidgetMessage>() else {
            return;
        };
        let bounds = widget.screen_bounds();
        match wmsg {
            WidgetMessage::MouseDown { pos, .. } => {
                // Click anywhere on the track jumps the handle there, then the
                // same press continues as a drag.
                self.dragging = true;
                self.value = Self::value_at(bounds, pos.x);
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    SliderMessage::Value(self.value),
                ));
                msg.handled = true;
            }
            WidgetMessage::MouseMove { pos } => {
                if self.dragging {
                    let v = Self::value_at(bounds, pos.x);
                    if (v - self.value).abs() > f32::EPSILON {
                        self.value = v;
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            SliderMessage::Value(v),
                        ));
                    }
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseUp { .. } => {
                self.dragging = false;
                msg.handled = true;
            }
            _ => {}
        }
    }
}

pub struct SliderBuilder {
    widget: WidgetBuilder,
    value: f32,
    track: [u8; 4],
    fill: [u8; 4],
}

impl SliderBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            value: 0.0,
            track: crate::theme::BORDER_DARK,
            fill: crate::theme::ACCENT_BLUE,
        }
    }

    pub fn with_value(mut self, v: f32) -> Self {
        self.value = v.clamp(0.0, 1.0);
        self
    }

    pub fn with_colors(mut self, track: [u8; 4], fill: [u8; 4]) -> Self {
        self.track = track;
        self.fill = fill;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Slider {
                value: self.value,
                dragging: false,
                track: self.track,
                fill: self.fill,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::new(100.0, 0.0, 108.0, 18.0) // 100 px usable after the handle
    }

    #[test]
    fn clicking_the_left_edge_gives_zero_and_the_right_edge_gives_one() {
        let b = bounds();
        assert_eq!(Slider::value_at(b, b.x), 0.0);
        assert_eq!(Slider::value_at(b, b.x + b.w), 1.0);
    }

    #[test]
    fn the_midpoint_is_half() {
        let b = bounds();
        let mid = Slider::value_at(b, b.x + b.w * 0.5);
        assert!((mid - 0.5).abs() < 1e-3, "midpoint mapped to {mid}");
    }

    #[test]
    fn dragging_outside_the_track_clamps() {
        let b = bounds();
        assert_eq!(Slider::value_at(b, b.x - 500.0), 0.0);
        assert_eq!(Slider::value_at(b, b.x + 5000.0), 1.0);
    }
}
