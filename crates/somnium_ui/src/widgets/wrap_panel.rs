// WrapPanel: left-to-right tiles that wrap to the next row (Content Drawer).

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct WrapPanel {
    pub h_gap: f32,
    pub v_gap: f32,
}

impl Control for WrapPanel {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let max_w = if available.x.is_finite() {
            available.x.max(1.0)
        } else {
            10_000.0
        };
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut row_h = 0.0f32;
        let mut used_w = 0.0f32;
        for &ch in &widget.children {
            ctx.measure_child(ch, Vec2::new(max_w, f32::INFINITY));
            let ds = ctx.desired_size(ch);
            if x > 0.0 && x + ds.x > max_w {
                y += row_h + self.v_gap;
                x = 0.0;
                row_h = 0.0;
            }
            x += ds.x + self.h_gap;
            row_h = row_h.max(ds.y);
            used_w = used_w.max(x - self.h_gap);
        }
        Vec2::new(used_w.max(1.0), (y + row_h).max(1.0))
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let max_w = final_size.x.max(1.0);
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut row_h = 0.0f32;
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            if x > 0.0 && x + ds.x > max_w {
                y += row_h + self.v_gap;
                x = 0.0;
                row_h = 0.0;
            }
            ctx.arrange_child(ch, Rect::new(ox + x, oy + y, ds.x, ds.y));
            x += ds.x + self.h_gap;
            row_h = row_h.max(ds.y);
        }
        Vec2::new(final_size.x, (y + row_h).max(final_size.y))
    }

    fn draw(&self, _widget: &Widget, _ctx: &mut DrawingContext) {}

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
    }
}

pub struct WrapPanelBuilder {
    widget: WidgetBuilder,
    h_gap: f32,
    v_gap: f32,
}

impl WrapPanelBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            h_gap: 8.0,
            v_gap: 8.0,
        }
    }

    pub fn with_gap(mut self, h: f32, v: f32) -> Self {
        self.h_gap = h;
        self.v_gap = v;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(WrapPanel {
                h_gap: self.h_gap,
                v_gap: self.v_gap,
            }),
        )
    }
}
