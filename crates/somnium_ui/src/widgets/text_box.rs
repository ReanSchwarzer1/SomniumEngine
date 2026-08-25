// TextBox: single-line editable text widget.
// Focus/Unfocus via click; text input via WidgetMessage::Text; Backspace/Enter handling.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
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
    /// See [`super::MIXED_PLACEHOLDER`]. Cleared by the first keystroke.
    pub mixed: bool,
    pub text: String,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
    pub focused: bool,
}

impl Control for TextBox {
    // MORROWIND-I. A text box's *value* is its contents and its *name* is the
    // label beside it, which this control does not own — hence no `a11y_name`.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::TextInput
    }

    fn a11y_value(&self) -> Option<String> {
        Some(self.text.clone())
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn is_text_input(&self) -> bool {
        true
    }
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let sz = ctx.measure_text(&self.text, self.px, self.font_id);
        Vec2::new(available.x.max(sz.x.max(40.0)), sz.y.max(self.px + 6.0))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // Phase 27-D: the input recipe carries the radius, the recession and
        // the focus ring plus its glow.
        let paint = crate::style::input(crate::style::VisualState::rest().focused(self.focused));
        ctx.push_paint(b, &paint);
        let text_origin = Vec2::new(b.x + 4.0, b.y + 3.0);
        let shown = if self.mixed {
            super::MIXED_PLACEHOLDER
        } else {
            self.text.as_str()
        };
        ctx.push_text(shown, text_origin, self.font_id, self.px, self.color);
        if self.focused {
            let advance = ctx
                .font_atlas
                .measure_text(&self.text, self.px, self.font_id)
                .x;
            let cx = b.x + 4.0 + advance;
            ctx.push_rect_filled(Rect::new(cx, b.y + 3.0, 1.0, self.px), self.color);
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> crate::node::CursorKind {
        crate::node::CursorKind::Text
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
                    self.mixed = false;
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
                WidgetMessage::KeyDown(key, _) => {
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
    mixed: bool,
    text: String,
    px: f32,
    color: [u8; 4],
    font_id: u8,
}

impl TextBoxBuilder {
    /// Display [`super::MIXED_PLACEHOLDER`] until the control is touched.
    /// Multi-selection is the only caller; a single selection never sets it.
    pub fn with_mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            mixed: false,
            widget,
            text: String::new(),
            px: theme::NOCTURNE.typography.body,
            color: theme::TEXT_PRIMARY,
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
                mixed: self.mixed,
                text: self.text,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
                focused: false,
            }),
        )
    }
}
