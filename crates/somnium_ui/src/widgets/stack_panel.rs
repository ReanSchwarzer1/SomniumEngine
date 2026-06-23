// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/stack_panel.rs
// StackPanel: orders children linearly along Vertical or Horizontal axis.

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}

pub struct StackPanel {
    pub orientation: Orientation,
}

impl Control for StackPanel {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let mut child_constraint = Vec2::new(f32::INFINITY, f32::INFINITY);
        match self.orientation {
            Orientation::Vertical   => child_constraint.x = available.x,
            Orientation::Horizontal => child_constraint.y = available.y,
        }

        let mut measured = Vec2::ZERO;
        for &ch in &widget.children {
            ctx.measure_child(ch, child_constraint);
            let ds = ctx.desired_size(ch);
            match self.orientation {
                Orientation::Vertical => {
                    if ds.x > measured.x { measured.x = ds.x; }
                    measured.y += ds.y;
                }
                Orientation::Horizontal => {
                    measured.x += ds.x;
                    if ds.y > measured.y { measured.y = ds.y; }
                }
            }
        }
        measured
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let mut width  = final_size.x;
        let mut height = 0.0f32;

        match self.orientation {
            Orientation::Vertical => {
                for &ch in &widget.children {
                    let ds = ctx.desired_size(ch);
                    let rect = Rect::new(ox, oy + height, width.max(ds.x), ds.y);
                    ctx.arrange_child(ch, rect);
                    width   = width.max(ds.x);
                    height += ds.y;
                }
                Vec2::new(width, height.max(final_size.y))
            }
            Orientation::Horizontal => {
                let mut x_offset = 0.0f32;
                let mut max_h    = final_size.y;
                for &ch in &widget.children {
                    let ds = ctx.desired_size(ch);
                    let rect = Rect::new(ox + x_offset, oy, ds.x, max_h.max(ds.y));
                    ctx.arrange_child(ch, rect);
                    x_offset += ds.x;
                    max_h     = max_h.max(ds.y);
                }
                Vec2::new(x_offset.max(final_size.x), max_h)
            }
        }
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

pub struct StackPanelBuilder {
    widget:      WidgetBuilder,
    orientation: Orientation,
}

impl StackPanelBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, orientation: Orientation::Vertical }
    }

    pub fn with_orientation(mut self, o: Orientation) -> Self {
        self.orientation = o;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(self.widget.build(), Box::new(StackPanel { orientation: self.orientation }))
    }
}
