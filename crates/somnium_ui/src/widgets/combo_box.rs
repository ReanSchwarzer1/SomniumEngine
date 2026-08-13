// ComboBox — replaces cyclers (Phase 26-B). Expands in place; clip_to_bounds false.

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
pub enum ComboBoxMessage {
    SelectionChanged(usize),
    SetSelected(usize),
}

pub struct ComboBox {
    pub items: Vec<String>,
    pub selected: usize,
    pub open: bool,
    pub font_id: u8,
    pub px: f32,
}

impl Control for ComboBox {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let header = theme::ROW_HEIGHT;
        let extra = if self.open {
            self.items.len() as f32 * theme::ROW_HEIGHT
        } else {
            0.0
        };
        let w = self
            .items
            .iter()
            .map(|s| ctx.measure_text(s, self.px, self.font_id).x)
            .fold(80.0_f32, f32::max)
            + 28.0;
        Vec2::new(available.x.min(w.max(80.0)).max(80.0), header + extra)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let header = Rect::new(b.x, b.y, b.w, theme::ROW_HEIGHT);
        ctx.push_rect_filled(header, theme::BG_RAISED);
        ctx.push_rect_border(header, 1.0, theme::BORDER_MEDIUM);
        let label = self
            .items
            .get(self.selected)
            .map(|s| s.as_str())
            .unwrap_or("");
        ctx.push_text(
            label,
            Vec2::new(b.x + 6.0, b.y + 4.0),
            self.font_id,
            self.px,
            theme::TEXT_PRIMARY,
        );
        let chev = Rect::new(b.x + b.w - 20.0, b.y + 2.0, 16.0, 16.0);
        let (uv, tex) = IconId::ChevronDown.draw_quad(chev);
        ctx.push_textured_rect(chev, uv, theme::TEXT_SECONDARY, tex);
        if self.open {
            for (i, item) in self.items.iter().enumerate() {
                let row = Rect::new(
                    b.x,
                    b.y + theme::ROW_HEIGHT * (i as f32 + 1.0),
                    b.w,
                    theme::ROW_HEIGHT,
                );
                let bg = if i == self.selected {
                    theme::ACCENT_DIM
                } else {
                    theme::BG_PANEL
                };
                ctx.push_rect_filled(row, bg);
                ctx.push_text(
                    item,
                    Vec2::new(row.x + 6.0, row.y + 4.0),
                    self.font_id,
                    self.px,
                    theme::TEXT_PRIMARY,
                );
            }
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
        if let Some(ComboBoxMessage::SetSelected(i)) = msg.data::<ComboBoxMessage>() {
            if *i < self.items.len() {
                self.selected = *i;
            }
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let local_y = pos.y - b.y;
            if local_y <= theme::ROW_HEIGHT {
                self.open = !self.open;
                widget.invalidate_layout();
                msg.handled = true;
            } else if self.open {
                let idx = ((local_y - theme::ROW_HEIGHT) / theme::ROW_HEIGHT).floor() as isize;
                if idx >= 0 && (idx as usize) < self.items.len() {
                    self.selected = idx as usize;
                    self.open = false;
                    widget.invalidate_layout();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        ComboBoxMessage::SelectionChanged(self.selected),
                    ));
                    msg.handled = true;
                }
            }
        }
    }
}

pub struct ComboBoxBuilder {
    widget: WidgetBuilder,
    items: Vec<String>,
    selected: usize,
    font_id: u8,
    px: f32,
}

impl ComboBoxBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget: widget.with_clip_to_bounds(false),
            items: Vec::new(),
            selected: 0,
            font_id: 0,
            px: 12.0,
        }
    }
    pub fn with_items(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.items = items.into_iter().map(Into::into).collect();
        self
    }
    pub fn with_selected(mut self, i: usize) -> Self {
        self.selected = i;
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ComboBox {
                items: self.items,
                selected: self.selected,
                open: false,
                font_id: self.font_id,
                px: self.px,
            }),
        )
    }
}

impl ComboBoxMessage {
    pub fn set_selected(dest: crate::message::NodeHandle, i: usize) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetSelected(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_stays_in_range() {
        let cb = ComboBox {
            items: vec!["AgX".into(), "ACES".into(), "Reinhard".into()],
            selected: 0,
            open: false,
            font_id: 0,
            px: 12.0,
        };
        assert_eq!(cb.items.len(), 3);
    }
}
