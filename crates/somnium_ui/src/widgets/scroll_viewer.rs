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
        let track_h = b.h;
        let content_h = self.content_h.get().max(self.view_h.get());
        let view_h = self.view_h.get().max(1.0);
        let thumb_h = (view_h / content_h * track_h).clamp(MIN_THUMB, track_h);
        let travel = (track_h - thumb_h).max(0.0);
        let max = self.max_scroll();
        let t = if max > 0.0 { self.scroll_y / max } else { 0.0 };
        Rect::new(b.x + b.w - BAR, b.y + t * travel, BAR, thumb_h)
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }
}

impl Control for ScrollViewer {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let inner = Vec2::new((available.x - BAR).max(1.0), f32::INFINITY);
        let mut content_h = 0.0f32;
        for &ch in &widget.children {
            ctx.measure_child(ch, inner);
            content_h = content_h.max(ctx.desired_size(ch).y);
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
        let mut content_h = final_size.y;
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            content_h = ds.y.max(final_size.y);
            ctx.arrange_child(ch, Rect::new(ox, oy - self.scroll_y, inner_w, content_h));
        }
        self.content_h.set(content_h);
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let track = Rect::new(b.x + b.w - BAR, b.y, BAR, b.h);
        ctx.push_rect_filled(track, theme::BG_INPUT);
        ctx.push_rect_border(track, 1.0, theme::BORDER_DARK);
        let thumb = self.thumb_rect(b);
        let color = if self.content_h.get() > self.view_h.get() + 0.5 {
            if self.dragging {
                theme::ACCENT
            } else {
                theme::BORDER_LIGHT
            }
        } else {
            theme::BORDER_MEDIUM
        };
        ctx.push_rect_filled(thumb, color);
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
                WidgetMessage::MouseMove { pos } => {
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
}
