// Context menu — popup list of labelled actions (Phase 26-B).

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub enum ContextMenuMessage {
    Activate(u32),
    SetItems(Vec<MenuItem>),
}

pub struct ContextMenu {
    pub items: Vec<MenuItem>,
    pub font_id: u8,
}

impl Control for ContextMenu {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        let w = self
            .items
            .iter()
            .map(|i| ctx.measure_text(&i.label, 12.0, self.font_id).x)
            .fold(120.0_f32, f32::max)
            + 16.0;
        Vec2::new(w, self.items.len() as f32 * theme::ROW_HEIGHT + 4.0)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // A context menu floats, so it takes the popup rung and its radius.
        let t = theme::active();
        ctx.push_paint(b, &crate::style::popup());
        let _ = t;
        for (i, item) in self.items.iter().enumerate() {
            let y = b.y + 2.0 + i as f32 * theme::ROW_HEIGHT;
            let color = if item.enabled {
                theme::active().semantic.text.primary.bytes()
            } else {
                theme::active().semantic.text.disabled.bytes()
            };
            ctx.push_text(
                &item.label,
                Vec2::new(b.x + 8.0, y + 4.0),
                self.font_id,
                12.0,
                color,
            );
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(ContextMenuMessage::SetItems(items)) = msg.data::<ContextMenuMessage>() {
            self.items = items.clone();
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let idx = ((pos.y - b.y - 2.0) / theme::ROW_HEIGHT).floor() as isize;
            if idx >= 0 && (idx as usize) < self.items.len() {
                let item = &self.items[idx as usize];
                if item.enabled {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        ContextMenuMessage::Activate(item.id),
                    ));
                }
                msg.handled = true;
            }
        }
    }
}

pub struct ContextMenuBuilder {
    widget: WidgetBuilder,
    items: Vec<MenuItem>,
    font_id: u8,
}

impl ContextMenuBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            items: Vec::new(),
            font_id: 0,
        }
    }
    pub fn with_items(mut self, items: Vec<MenuItem>) -> Self {
        self.items = items;
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ContextMenu {
                items: self.items,
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_item_can_be_disabled() {
        let i = MenuItem {
            id: 1,
            label: "Open…".into(),
            enabled: false,
        };
        assert!(!i.enabled);
    }
}
