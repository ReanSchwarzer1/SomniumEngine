// TabControl — header strip + one visible page (Phase 26-B).

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum TabControlMessage {
    Select(usize),
}

pub struct TabControl {
    pub titles: Vec<String>,
    pub selected: usize,
    pub font_id: u8,
}

impl Control for TabControl {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let header = 22.0;
        for (i, &ch) in widget.children.iter().enumerate() {
            if i == self.selected {
                ctx.measure_child(ch, Vec2::new(available.x, (available.y - header).max(0.0)));
            } else {
                ctx.measure_child(ch, Vec2::ZERO);
            }
        }
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let header = 22.0;
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        for (i, &ch) in widget.children.iter().enumerate() {
            if i == self.selected {
                ctx.arrange_child(
                    ch,
                    Rect::new(
                        ox,
                        oy + header,
                        final_size.x,
                        (final_size.y - header).max(0.0),
                    ),
                );
            } else {
                ctx.arrange_child(ch, Rect::new(ox, oy + header, 0.0, 0.0));
            }
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        ctx.push_rect_filled(Rect::new(b.x, b.y, b.w, 22.0), theme::BG_HEADER);
        let n = self.titles.len().max(1) as f32;
        let tw = b.w / n;
        for (i, title) in self.titles.iter().enumerate() {
            let r = Rect::new(b.x + i as f32 * tw, b.y, tw, 22.0);
            if i == self.selected {
                ctx.push_rect_filled(r, theme::BG_PANEL);
                ctx.push_rect_filled(Rect::new(r.x, r.y + 20.0, r.w, 2.0), theme::ACCENT);
            }
            ctx.push_text(
                title,
                Vec2::new(r.x + 8.0, r.y + 4.0),
                self.font_id,
                11.0,
                theme::TEXT_PRIMARY,
            );
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
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            if pos.y <= b.y + 22.0 && !self.titles.is_empty() {
                let tw = b.w / self.titles.len() as f32;
                let i = ((pos.x - b.x) / tw).floor() as isize;
                if i >= 0 && (i as usize) < self.titles.len() {
                    self.selected = i as usize;
                    widget.invalidate_layout();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        TabControlMessage::Select(self.selected),
                    ));
                    msg.handled = true;
                }
            }
        }
    }
}

pub struct TabControlBuilder {
    widget: WidgetBuilder,
    titles: Vec<String>,
    font_id: u8,
}

impl TabControlBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            titles: Vec::new(),
            font_id: 0,
        }
    }
    pub fn with_titles(mut self, t: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.titles = t.into_iter().map(Into::into).collect();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(TabControl {
                titles: self.titles,
                selected: 0,
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_first_tab() {
        let t = TabControl {
            titles: vec!["Log".into(), "Content".into()],
            selected: 0,
            font_id: 0,
        };
        assert_eq!(t.selected, 0);
    }
}
