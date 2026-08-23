// ScrollViewer: clips content and applies a vertical scroll offset.
// A persistent gutter on the right shows that the pane is scrollable.

use crate::{
    draw::DrawingContext,
    message::{UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use std::cell::Cell;

const BAR: f32 = 10.0;
const MIN_THUMB: f32 = 24.0;
/// Height of the fade that marks clipped content at a scroll edge.
const FADE_HEIGHT: f32 = 14.0;

pub struct ScrollViewer {
    pub scroll_y: f32,
    content_h: Cell<f32>,
    view_h: Cell<f32>,
    dragging: bool,
    drag_anchor_y: f32,
    drag_scroll0: f32,
}

impl ScrollViewer {
    fn max_scroll(&self) -> f32 {
        (self.content_h.get() - self.view_h.get()).max(0.0)
    }

    fn thumb_rect(&self, b: Rect) -> Rect {
        let track_h = b.h.max(0.0);
        let content_h = self.content_h.get().max(self.view_h.get()).max(1.0);
        let view_h = self.view_h.get().max(1.0);
        let min_thumb = MIN_THUMB.min(track_h);
        let thumb_h = (view_h / content_h * track_h).clamp(min_thumb, track_h);
        let travel = (track_h - thumb_h).max(0.0);
        let max = self.max_scroll();
        let t = if max > 0.0 { self.scroll_y / max } else { 0.0 };
        Rect::new(b.x + b.w - BAR, b.y + t * travel, BAR, thumb_h)
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    pub fn scroll_offset(&self) -> f32 {
        self.scroll_y
    }
}

impl Control for ScrollViewer {
    fn scroll_into_view(&mut self, widget: &mut Widget, target: Rect) -> bool {
        let viewport = widget.screen_bounds();
        let old = self.scroll_y;
        if target.y < viewport.y {
            self.scroll_y -= viewport.y - target.y;
        } else if target.y + target.h > viewport.y + viewport.h {
            self.scroll_y += target.y + target.h - (viewport.y + viewport.h);
        }
        self.clamp_scroll();
        if (old - self.scroll_y).abs() > 0.01 {
            widget.invalidate_layout();
            true
        } else {
            false
        }
    }

    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let inner = Vec2::new((available.x - BAR).max(1.0), f32::INFINITY);
        let mut content_h = 0.0f32;
        for &ch in &widget.children {
            ctx.measure_child(ch, inner);
            // Hidden children do not reserve height. A panel that stacks a
            // content state against an empty state would otherwise always be
            // as tall as both.
            if ctx.is_visible(ch) {
                content_h = content_h.max(ctx.desired_size(ch).y);
            }
        }
        self.content_h.set(content_h);
        self.view_h.set(available.y.max(1.0));
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let inner_w = (final_size.x - BAR).max(1.0);
        self.view_h.set(final_size.y.max(1.0));
        // Accumulate the tallest visible child rather than overwriting.
        //
        // This loop used to assign `content_h` on every iteration, so with more
        // than one child it kept the **last** one's height instead of the
        // largest. It was latent while every scroll viewer had exactly one
        // child; adding the Details empty state beside the property stack made
        // it real, and the Details panel silently stopped scrolling because a
        // short trailing child reported the whole content as viewport-sized.
        let mut content_h = final_size.y;
        for &ch in &widget.children {
            if !ctx.is_visible(ch) {
                continue;
            }
            let ds = ctx.desired_size(ch);
            content_h = content_h.max(ds.y.max(final_size.y));
        }
        for &ch in &widget.children {
            ctx.arrange_child(ch, Rect::new(ox, oy - self.scroll_y, inner_w, content_h));
        }
        self.content_h.set(content_h);
        final_size
    }

    fn draw(&self, _widget: &Widget, _ctx: &mut DrawingContext) {
        // Nothing paints beneath the content. The scrollbar and the edge
        // fades are overlays and are emitted from `draw_over`, after the
        // children have painted.
        //
        // They used to be emitted here, which put them *under* the very
        // content they annotate: correct geometry, correct colour, and
        // completely invisible. See `Control::draw_over`.
    }

    fn draw_over(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        let track = Rect::new(b.x + b.w - BAR, b.y, BAR, b.h);
        ctx.push_rect_filled(track, t.semantic.surface.input.bytes());
        ctx.push_rect_border(track, 1.0, t.semantic.border.subtle.bytes());

        let scrollable = self.content_h.get() > self.view_h.get() + 0.5;
        let thumb = self.thumb_rect(b);
        let color = if scrollable {
            if self.dragging {
                t.semantic.accent.default.bytes()
            } else {
                t.semantic.border.strong.bytes()
            }
        } else {
            t.semantic.border.default.bytes()
        };
        // A capsule thumb, so the gutter reads as a control rather than a slot.
        let thumb_r = (BAR * 0.5 - 1.0).max(0.0);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(thumb, color).with_radius(thumb_r),
            None,
        );

        // Phase 27-D, the §2.4 audit item: a clipped region must say that its
        // content continues. Drawn only while there is actually more to see, and
        // only at the edge there is more on, so a short list stays clean.
        if scrollable {
            let surface = t.semantic.surface.panel.bytes();
            let fade_h = FADE_HEIGHT.min(b.h * 0.25);
            let viewport_w = (b.w - BAR).max(0.0);
            if self.scroll_y > 0.5 {
                ctx.push_scroll_fade(Rect::new(b.x, b.y, viewport_w, fade_h), surface, true);
            }
            let max_offset = (self.content_h.get() - self.view_h.get()).max(0.0);
            if self.scroll_y < max_offset - 0.5 {
                ctx.push_scroll_fade(
                    Rect::new(b.x, b.y + b.h - fade_h, viewport_w, fade_h),
                    surface,
                    false,
                );
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        let b = widget.screen_bounds();
        self.view_h.set(b.h.max(1.0));
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseWheel { delta, .. } => {
                    self.scroll_y -= *delta;
                    self.clamp_scroll();
                    widget.invalidate_layout();
                    msg.handled = true;
                }
                WidgetMessage::MouseDown { pos, .. } => {
                    let track = Rect::new(b.x + b.w - BAR, b.y, BAR, b.h);
                    if track.contains(*pos) {
                        let thumb = self.thumb_rect(b);
                        if thumb.contains(*pos) {
                            self.dragging = true;
                            self.drag_anchor_y = pos.y;
                            self.drag_scroll0 = self.scroll_y;
                        } else {
                            let rel = ((pos.y - b.y) / b.h.max(1.0)).clamp(0.0, 1.0);
                            self.scroll_y = rel * self.max_scroll();
                            self.clamp_scroll();
                            widget.invalidate_layout();
                        }
                        msg.handled = true;
                    }
                }
                WidgetMessage::MouseMove { pos, .. } => {
                    if self.dragging {
                        let max = self.max_scroll();
                        let travel = (b.h - MIN_THUMB).max(1.0);
                        let dy = pos.y - self.drag_anchor_y;
                        self.scroll_y = self.drag_scroll0 + dy / travel * max;
                        self.clamp_scroll();
                        widget.invalidate_layout();
                        msg.handled = true;
                    }
                }
                WidgetMessage::MouseUp { .. } | WidgetMessage::MouseLeave => {
                    self.dragging = false;
                }
                _ => {}
            }
        }
    }
}

pub struct ScrollViewerBuilder {
    widget: WidgetBuilder,
}

impl ScrollViewerBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget }
    }

    pub fn build(self) -> crate::node::UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ScrollViewer {
                scroll_y: 0.0,
                content_h: Cell::new(0.0),
                view_h: Cell::new(0.0),
                dragging: false,
                drag_anchor_y: 0.0,
                drag_scroll0: 0.0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_viewer_has_no_scroll_range() {
        let v = ScrollViewer {
            scroll_y: 0.0,
            content_h: Cell::new(100.0),
            view_h: Cell::new(100.0),
            dragging: false,
            drag_anchor_y: 0.0,
            drag_scroll0: 0.0,
        };
        assert_eq!(v.max_scroll(), 0.0);
    }

    #[test]
    fn zero_height_track_does_not_panic() {
        let v = ScrollViewer {
            scroll_y: 0.0,
            content_h: Cell::new(200.0),
            view_h: Cell::new(0.0),
            dragging: false,
            drag_anchor_y: 0.0,
            drag_scroll0: 0.0,
        };
        let thumb = v.thumb_rect(Rect::new(0.0, 0.0, 40.0, 0.0));
        assert_eq!(thumb.h, 0.0);
    }
}
