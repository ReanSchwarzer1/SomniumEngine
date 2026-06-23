// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/canvas.rs
// Canvas: absolute positioning — children given infinite space, placed at desired_local_position.

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct Canvas;

impl Control for Canvas {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        let inf = Vec2::new(f32::INFINITY, f32::INFINITY);
        for &ch in &widget.children {
            ctx.measure_child(ch, inf);
        }
        Vec2::ZERO
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        for &ch in &widget.children {
            let pos = ctx.desired_local_position(ch);
            let ds  = ctx.desired_size(ch);
            ctx.arrange_child(ch, Rect::new(ox + pos.x, oy + pos.y, ds.x, ds.y));
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        ctx.push_rect_filled(widget.screen_bounds(), widget.background);
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg:    &mut UiMessage,
        _emit:   &mut Vec<UiMessage>,
    ) {}
}

pub struct CanvasBuilder {
    widget: WidgetBuilder,
}

impl CanvasBuilder {
    pub fn new(widget: WidgetBuilder) -> Self { Self { widget } }

    pub fn build(self) -> UiNode {
        UiNode::new(self.widget.build(), Box::new(Canvas))
    }
}
