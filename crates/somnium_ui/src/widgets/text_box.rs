// TextBox: single-line editable text widget.
// Focus/Unfocus via click; text input via WidgetMessage::Text; Backspace/Enter handling.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum TextBoxMessage {
    TextChanged(String),
    TextCommit(String),
}

impl TextBoxMessage {
    pub fn text_changed(dest: NodeHandle, text: String) -> UiMessage {
        UiMessage::new(dest, MessageDirection::FromWidget, Self::TextChanged(text))
    }
    pub fn text_commit(dest: NodeHandle, text: String) -> UiMessage {
        UiMessage::new(dest, MessageDirection::FromWidget, Self::TextCommit(text))
    }
}

pub struct TextBox {
    pub text: String,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
    pub focused: bool,
}

impl Control for TextBox {
    fn is_text_input(&self) -> bool {
        true
    }
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let sz = ctx.measure_text(&self.text, self.px, self.font_id);
        Vec2::new(available.x.max(sz.x.max(40.0)), sz.y.max(self.px + 6.0))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let bg = if self.focused {
            [40, 40, 60, 255]
        } else {
            [28, 28, 28, 255]
        };
        ctx.push_rect_filled(b, bg);
        let border_col = if self.focused {
            [100, 140, 200, 255]
        } else {
            [70, 70, 70, 255]
        };
        ctx.push_rect_border(b, 1.0, border_col);
        let text_origin = Vec2::new(b.x + 4.0, b.y + 3.0);
        ctx.push_text(&self.text, text_origin, self.font_id, self.px, self.color);
        if self.focused {
            let advance = ctx
                .font_atlas
                .measure_text(&self.text, self.px, self.font_id)
                .x;
            let cx = b.x + 4.0 + advance;
            ctx.push_rect_filled(Rect::new(cx, b.y + 3.0, 1.0, self.px), self.color);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(TextMessage::SetText(s)) = msg.data::<TextMessage>() {
            if !self.focused {
                self.text = s.clone();
                widget.invalidate_layout();
                msg.handled = true;
                return;
            }
        }
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg.clone() {
                WidgetMessage::Focus => {
                    self.focused = true;
                    msg.handled = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                    emit.push(TextBoxMessage::text_commit(
                        widget.handle,
                        self.text.clone(),
                    ));
                    msg.handled = true;
                }
                WidgetMessage::Text(s) => {
                    if self.focused {
                        self.text.push_str(&s);
                        widget.invalidate_layout();
                        emit.push(TextBoxMessage::text_changed(
                            widget.handle,
                            self.text.clone(),
                        ));
                        msg.handled = true;
                    }
                }
                WidgetMessage::KeyDown(key) => {
                    if self.focused {
                        match key {
                            KeyCode::Backspace => {
                                if !self.text.is_empty() {
                                    let mut chars = self.text.chars();
                                    chars.next_back();
                                    self.text = chars.as_str().to_owned();
                                    widget.invalidate_layout();
                                    emit.push(TextBoxMessage::text_changed(
                                        widget.handle,
                                        self.text.clone(),
                                    ));
                                }
                                msg.handled = true;
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                self.focused = false;
                                emit.push(TextBoxMessage::text_commit(
                                    widget.handle,
                                    self.text.clone(),
                                ));
                                msg.handled = true;
                            }
                            KeyCode::Escape => {
                                self.focused = false;
                                msg.handled = true;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub struct TextBoxBuilder {
    widget: WidgetBuilder,
    text: String,
    px: f32,
    color: [u8; 4],
    font_id: u8,
}

impl TextBoxBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            text: String::new(),
            px: 13.0,
            color: [220, 220, 220, 255],
            font_id: 0,
        }
    }

    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }
    pub fn with_font_size(mut self, px: f32) -> Self {
        self.px = px;
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn with_color(mut self, c: [u8; 4]) -> Self {
        self.color = c;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(TextBox {
                text: self.text,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
                focused: false,
            }),
        )
    }
}
