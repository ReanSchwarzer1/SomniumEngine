// CheckBox — replaces [x]/[ ] buttons (Phase 26-B).

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum CheckBoxMessage {
    Check(bool),
    SetChecked(bool),
}

pub struct CheckBox {
    pub checked: bool,
    pub label: String,
    pub font_id: u8,
    pub px: f32,
}

impl Control for CheckBox {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        let text = ctx.measure_text(&self.label, self.px, self.font_id);
        Vec2::new(22.0 + 6.0 + text.x, text.y.max(20.0))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let box_r = Rect::new(
            b.x + 2.0,
            b.y + (b.h - theme::ICON_CHECK) * 0.5,
            theme::ICON_CHECK,
            theme::ICON_CHECK,
        );
        ctx.push_rect_filled(box_r, theme::BG_INPUT);
        ctx.push_rect_border(box_r, 1.0, theme::BORDER_MEDIUM);
        if self.checked {
            let (uv, tex) = IconId::Check.draw_quad(box_r);
            ctx.push_textured_rect(box_r, uv, theme::ACCENT, tex);
        }
        ctx.push_text(
            &self.label,
            Vec2::new(b.x + 24.0, b.y + (b.h - self.px) * 0.5),
            self.font_id,
            self.px,
            theme::TEXT_PRIMARY,
        );
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
        if let Some(CheckBoxMessage::SetChecked(v)) = msg.data::<CheckBoxMessage>() {
            self.checked = *v;
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { .. }) = msg.data::<WidgetMessage>() {
            self.checked = !self.checked;
            emit.push(UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                CheckBoxMessage::Check(self.checked),
            ));
            msg.handled = true;
        }
    }
}

pub struct CheckBoxBuilder {
    widget: WidgetBuilder,
    checked: bool,
    label: String,
    font_id: u8,
    px: f32,
}

impl CheckBoxBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            checked: false,
            label: String::new(),
            font_id: 0,
            px: 12.0,
        }
    }
    pub fn with_checked(mut self, v: bool) -> Self {
        self.checked = v;
        self
    }
    pub fn with_label(mut self, s: impl Into<String>) -> Self {
        self.label = s.into();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn with_font_size(mut self, px: f32) -> Self {
        self.px = px;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(CheckBox {
                checked: self.checked,
                label: self.label,
                font_id: self.font_id,
                px: self.px,
            }),
        )
    }
}

impl CheckBoxMessage {
    pub fn set_checked(dest: crate::message::NodeHandle, v: bool) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetChecked(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_starts_unchecked() {
        let cb = CheckBox {
            checked: false,
            label: "Hex".into(),
            font_id: 0,
            px: 12.0,
        };
        assert!(!cb.checked);
    }
}
