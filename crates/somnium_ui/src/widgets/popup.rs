use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum PopupMessage {
    Open,
    Close,
}

pub struct Popup {
    pub is_open: bool,
}

impl Control for Popup {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        if !self.is_open {
            return Vec2::ZERO;
        }
        for &ch in &widget.children {
            ctx.measure_child(ch, available);
        }
        // Popup takes up the entire available space to catch outside clicks
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        if !self.is_open {
            return Vec2::ZERO;
        }
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let rect = Rect::new(ox, oy, final_size.x, final_size.y);
        
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            // The content should position itself via desired_local_position and margins
            let pos = ctx.desired_local_position(ch);
            ctx.arrange_child(ch, Rect::new(ox + pos.x, oy + pos.y, ds.x, ds.y));
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if self.is_open {
            ctx.push_rect_filled(widget.screen_bounds(), widget.background);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg:    &mut UiMessage,
        emit:   &mut Vec<UiMessage>,
    ) {
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseDown { pos, .. } => {
                    if self.is_open {
                        // If clicked directly on the transparent backdrop, close
                        if msg.destination == widget.handle {
                            emit.push(UiMessage::new(
                                widget.handle,
                                MessageDirection::FromWidget,
                                PopupMessage::Close,
                            ));
                            msg.handled = true;
                        }
                    }
                }
                _ => {}
            }
        } else if let Some(pmsg) = msg.data::<PopupMessage>() {
            match pmsg {
                PopupMessage::Open => {
                    if !self.is_open {
                        self.is_open = true;
                        widget.visibility = true;
                        widget.invalidate_layout();
                    }
                }
                PopupMessage::Close => {
                    if self.is_open {
                        self.is_open = false;
                        widget.visibility = false;
                        widget.invalidate_layout();
                    }
                }
            }
        }
    }
}

pub struct PopupBuilder {
    widget: WidgetBuilder,
}

impl PopupBuilder {
    pub fn new(widget: WidgetBuilder) -> Self { Self { widget } }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.with_visibility(false).build(),
            Box::new(Popup { is_open: false })
        )
    }
}
