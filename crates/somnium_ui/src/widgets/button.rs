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
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq)]
pub enum ButtonMessage {
    Click,
    DoubleClick,
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
    last_up: Option<Instant>,
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
                        let now = Instant::now();
                        let double = self
                            .last_up
                            .is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK);
                        self.last_up = Some(now);
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            if double {
                                ButtonMessage::DoubleClick
                            } else {
                                ButtonMessage::Click
                            },
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
                last_up: None,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MouseButton, NodeHandle};

    fn click(button: &mut Button, widget: &mut Widget, emit: &mut Vec<UiMessage>) {
        let pos = Vec2::new(4.0, 4.0);
        let mut down = UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
            },
        );
        button.handle_routed_message(widget, &mut down, emit);
        let mut up = UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            WidgetMessage::MouseUp {
                pos,
                button: MouseButton::Left,
            },
        );
        button.handle_routed_message(widget, &mut up, emit);
    }

    #[test]
    fn second_click_within_400ms_emits_double_click() {
        let mut button = Button {
            is_pressed: false,
            hovered: false,
            selected: false,
            last_up: None,
        };
        let mut widget = Widget::default();
        widget.handle = NodeHandle::NONE;
        widget.actual_local_position = Vec2::ZERO;
        widget.actual_local_size = Vec2::new(32.0, 32.0);
        let mut emit = Vec::new();
        click(&mut button, &mut widget, &mut emit);
        click(&mut button, &mut widget, &mut emit);
        assert!(matches!(
            emit[0].data::<ButtonMessage>(),
            Some(ButtonMessage::Click)
        ));
        assert!(matches!(
            emit[1].data::<ButtonMessage>(),
            Some(ButtonMessage::DoubleClick)
        ));
    }
}
