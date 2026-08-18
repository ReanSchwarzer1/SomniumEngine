use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuMessage {
    Click,
}

pub struct Menu {
    pub is_pressed: bool,
}

impl Control for Menu {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let mut desired = Vec2::ZERO;
        for &ch in &widget.children {
            ctx.measure_child(ch, available);
            let ds = ctx.desired_size(ch);
            if ds.x > desired.x {
                desired.x = ds.x;
            }
            if ds.y > desired.y {
                desired.y = ds.y;
            }
        }
        desired
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let rect = Rect::new(ox, oy, final_size.x, final_size.y);
        for &ch in &widget.children {
            ctx.arrange_child(ch, rect);
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let mut bg = widget.background;
        if self.is_pressed {
            bg = [
                (bg[0] as f32 * 0.8) as u8,
                (bg[1] as f32 * 0.8) as u8,
                (bg[2] as f32 * 0.8) as u8,
                bg[3],
            ];
        }
        let b = widget.screen_bounds();
        match crate::theme::wash_for_surface(bg) {
            Some(g) => ctx.push_primitive(
                crate::primitive::Primitive::fill(b, g.from.bytes())
                    .with_gradient(g.to.bytes(), g.axis),
                None,
            ),
            None => ctx.push_rect_filled(b, bg),
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> crate::node::CursorKind {
        crate::node::CursorKind::Pointer
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseDown { .. } => {
                    self.is_pressed = true;
                    msg.handled = true;
                }
                WidgetMessage::MouseUp { pos, .. } => {
                    let pos = *pos;
                    if self.is_pressed && widget.screen_bounds().contains(pos) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            MenuMessage::Click,
                        ));
                    }
                    self.is_pressed = false;
                    msg.handled = true;
                }
                _ => {}
            }
        }
    }
}

pub struct MenuBuilder {
    widget: WidgetBuilder,
}

impl MenuBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget }
    }

    pub fn build(self) -> UiNode {
        UiNode::new(self.widget.build(), Box::new(Menu { is_pressed: false }))
    }
}
