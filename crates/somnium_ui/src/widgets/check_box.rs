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
    /// Tri-state: the selection disagrees. Painted as a dash rather than a
    /// tick, and cleared the moment the box is clicked.
    pub mixed: bool,
    pub label: String,
    pub font_id: u8,
    pub px: f32,
}

impl Control for CheckBox {
    // MORROWIND-I. Three-valued on purpose: `mixed` is a state Somnium's
    // property inspector produces for a multi-selection, and reporting it as
    // unchecked would tell a reader the opposite of the truth.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::CheckBox
    }

    fn a11y_name(&self) -> Option<String> {
        (!self.label.trim().is_empty()).then(|| self.label.clone())
    }

    fn a11y_toggled(&self) -> Option<crate::a11y::Toggled> {
        Some(if self.mixed {
            crate::a11y::Toggled::Mixed
        } else if self.checked {
            crate::a11y::Toggled::True
        } else {
            crate::a11y::Toggled::False
        })
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

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
        // A checkbox is a tiny input: same recession and radius as the fields.
        let t = theme::active();
        let paint = crate::style::input(crate::style::VisualState::rest());
        ctx.push_paint(box_r, &paint);
        let _ = t;
        if self.mixed {
            // Tri-state. A dash across the box, not a tick and not empty:
            // either of those would claim a value the selection does not have.
            let bar = Rect::new(
                box_r.x + 3.0,
                box_r.y + box_r.h * 0.5 - 1.0,
                box_r.w - 6.0,
                2.0,
            );
            ctx.push_rect_filled(bar, theme::active().semantic.text.secondary.bytes());
        } else if self.checked {
            let (uv, tex) = IconId::Check.draw_quad(box_r);
            ctx.push_textured_rect(
                box_r,
                uv,
                theme::active().semantic.accent.default.bytes(),
                tex,
            );
        }
        ctx.push_text(
            &self.label,
            Vec2::new(b.x + 24.0, b.y + (b.h - self.px) * 0.5),
            self.font_id,
            self.px,
            theme::active().semantic.text.primary.bytes(),
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
            self.mixed = false;
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
    mixed: bool,
    checked: bool,
    label: String,
    font_id: u8,
    px: f32,
}

impl CheckBoxBuilder {
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
                mixed: self.mixed,
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
            mixed: false,
            checked: false,
            label: "Hex".into(),
            font_id: 0,
            px: 12.0,
        };
        assert!(!cb.checked);
    }
}
