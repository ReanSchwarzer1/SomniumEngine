// ScrollViewer: clips content to its bounds and applies a vertical scroll offset.
// Mouse wheel scrolls; content is clipped by the inherited draw clip stack.

use crate::{
    draw::DrawingContext,
    message::{UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct ScrollViewer {
    pub scroll_y: f32,
}

impl Control for ScrollViewer {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        for &ch in &widget.children {
            ctx.measure_child(ch, Vec2::new(available.x, f32::INFINITY));
        }
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            let rect = Rect::new(ox, oy - self.scroll_y, final_size.x, ds.y.max(final_size.y));
            ctx.arrange_child(ch, rect);
        }
        final_size
    }

    fn draw(&self, _widget: &Widget, _ctx: &mut DrawingContext) {}

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(WidgetMessage::MouseWheel { delta, .. }) = msg.data::<WidgetMessage>() {
            let delta = *delta;
            self.scroll_y = (self.scroll_y - delta).max(0.0);
            widget.invalidate_layout();
            msg.handled = true;
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
            Box::new(ScrollViewer { scroll_y: 0.0 }),
        )
    }
}
