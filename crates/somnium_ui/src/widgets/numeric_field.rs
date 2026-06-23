// NumericField: f32 display/edit widget for the inspector.
// Click to focus, type to edit, Enter/Unfocus to commit.
// Only accepts digits, '.', and '-'.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum NumericFieldMessage {
    /// Sent ToWidget to update the displayed value (skipped when editing).
    SetValue(f32),
    /// Emitted FromWidget when the user commits a new value.
    ValueChanged(f32),
}

impl NumericFieldMessage {
    pub fn set_value(dest: NodeHandle, value: f32) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetValue(value))
    }

    pub fn value_changed(dest: NodeHandle, value: f32) -> UiMessage {
        UiMessage::new(dest, MessageDirection::FromWidget, Self::ValueChanged(value))
    }
}

pub struct NumericField {
    pub value:       f32,
    editing_text:    Option<String>,
    pub px:          f32,
    pub color:       [u8; 4],
    pub font_id:     u8,
    pub focused:     bool,
}

impl NumericField {
    fn display_text(&self) -> String {
        self.editing_text.clone().unwrap_or_else(|| format!("{:.3}", self.value))
    }
}

impl Control for NumericField {
    fn is_text_input(&self) -> bool { true }
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let text = self.display_text();
        let sz   = ctx.measure_text(&text, self.px, self.font_id);
        Vec2::new(available.x.min(sz.x.max(60.0)), sz.y.max(self.px + 6.0))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b   = widget.screen_bounds();
        let bg  = if self.focused { [40, 40, 60, 255] } else { [28, 28, 28, 255] };
        let bdr = if self.focused { [100, 140, 200, 255] } else { [60, 60, 60, 255] };
        ctx.push_rect_filled(b, bg);
        ctx.push_rect_border(b, 1.0, bdr);
        let text   = self.display_text();
        let origin = Vec2::new(b.x + 4.0, b.y + 3.0);
        ctx.push_text(&text, origin, self.font_id, self.px, self.color);
        if self.focused {
            let advance = ctx.font_atlas.measure_text(&text, self.px, self.font_id).x;
            let cx = b.x + 4.0 + advance;
            ctx.push_rect_filled(Rect::new(cx, b.y + 3.0, 1.0, self.px), self.color);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg:    &mut UiMessage,
        emit:   &mut Vec<UiMessage>,
    ) {
        if let Some(d) = msg.data::<NumericFieldMessage>() {
            if let NumericFieldMessage::SetValue(v) = d {
                let v = *v;
                if !self.focused {
                    self.value = v;
                    self.editing_text = None;
                    widget.invalidate_layout();
                    msg.handled = true;
                }
            }
            return;
        }

        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg.clone() {
                WidgetMessage::Focus => {
                    self.focused      = true;
                    self.editing_text = Some(format!("{:.3}", self.value));
                    msg.handled       = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                    if let Some(text) = self.editing_text.take() {
                        if let Ok(v) = text.trim().parse::<f32>() {
                            self.value = v;
                            emit.push(NumericFieldMessage::value_changed(widget.handle, v));
                        }
                    }
                    widget.invalidate_layout();
                    msg.handled = true;
                }
                WidgetMessage::Text(s) => {
                    if self.focused {
                        let t = self.editing_text.get_or_insert_with(String::new);
                        for ch in s.chars() {
                            if ch.is_ascii_digit() || ch == '.' || ch == '-' {
                                t.push(ch);
                            }
                        }
                        msg.handled = true;
                    }
                }
                WidgetMessage::KeyDown(key) => {
                    if self.focused {
                        match key {
                            KeyCode::Backspace => {
                                if let Some(t) = &mut self.editing_text {
                                    if !t.is_empty() {
                                        let mut chars = t.chars();
                                        chars.next_back();
                                        *t = chars.as_str().to_owned();
                                    }
                                }
                                msg.handled = true;
                            }
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                self.focused = false;
                                if let Some(text) = self.editing_text.take() {
                                    if let Ok(v) = text.trim().parse::<f32>() {
                                        self.value = v;
                                        emit.push(NumericFieldMessage::value_changed(widget.handle, v));
                                    }
                                }
                                widget.invalidate_layout();
                                msg.handled = true;
                            }
                            KeyCode::Escape => {
                                self.focused      = false;
                                self.editing_text = None;
                                widget.invalidate_layout();
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

pub struct NumericFieldBuilder {
    widget:  WidgetBuilder,
    value:   f32,
    px:      f32,
    color:   [u8; 4],
    font_id: u8,
}

impl NumericFieldBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            value:   0.0,
            px:      12.0,
            color:   [200, 200, 200, 255],
            font_id: 0,
        }
    }

    pub fn with_value(mut self, v: f32) -> Self { self.value = v; self }
    pub fn with_font_size(mut self, px: f32) -> Self { self.px = px; self }
    pub fn with_font_id(mut self, id: u8) -> Self { self.font_id = id; self }

    pub fn build(self) -> UiNode {
        UiNode::new(self.widget.build(), Box::new(NumericField {
            value:        self.value,
            editing_text: None,
            px:           self.px,
            color:        self.color,
            font_id:      self.font_id,
            focused:      false,
        }))
    }
}
