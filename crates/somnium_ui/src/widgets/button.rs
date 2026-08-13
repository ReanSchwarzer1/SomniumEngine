// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/button.rs
// Button: captures mouse on MouseDown; emits ButtonMessage::Click on MouseUp within bounds.
// Hover / press / selected fills so chrome controls read as buttons, not dead labels.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum ButtonMessage {
    Click,
    SetSelected(bool),
}

impl ButtonMessage {
    pub fn set_selected(dest: crate::message::NodeHandle, selected: bool) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::ToWidget,
            Self::SetSelected(selected),
        )
    }
}

pub struct Button {
    pub is_pressed: bool,
    pub hovered: bool,
    pub selected: bool,
}

impl Control for Button {
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
        let b = widget.screen_bounds();
        let fill = if self.is_pressed {
            theme::ACCENT_PRESSED
        } else if self.selected {
            theme::ACCENT_DIM
        } else if self.hovered {
            theme::BG_HOVER
        } else if widget.background[3] == 0 {
            theme::TRANSPARENT
        } else {
            widget.background
        };
        if fill[3] > 0 {
            ctx.push_rect_filled(b, fill);
        }
        if self.selected {
            ctx.push_rect_filled(Rect::new(b.x, b.y, 2.0, b.h), theme::ACCENT);
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
        if let Some(ButtonMessage::SetSelected(v)) = msg.data::<ButtonMessage>() {
            self.selected = *v;
            msg.handled = true;
            return;
        }
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseEnter => {
                    self.hovered = true;
                }
                WidgetMessage::MouseLeave => {
                    self.hovered = false;
                    self.is_pressed = false;
                }
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
                            ButtonMessage::Click,
                        ));
                    }
                    self.is_pressed = false;
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(key) => {
                    use crate::message::KeyCode;
                    if matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            ButtonMessage::Click,
                        ));
                        msg.handled = true;
                    }
                }
                _ => {}
            }
        }
    }
}

pub struct ButtonBuilder {
    widget: WidgetBuilder,
}

impl ButtonBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget }
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Button {
                is_pressed: false,
                hovered: false,
                selected: false,
            }),
        )
    }
}
