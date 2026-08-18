//! Command palette (Phase 26-I). Ctrl+P / Ctrl+Shift+P.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum CommandPaletteMessage {
    Run(usize),
    Query(String),
    SetQuery(String),
}

pub struct PaletteItem {
    pub label: String,
    pub hint: String,
}

pub struct CommandPalette {
    pub query: String,
    pub selected: usize,
    pub font_id: u8,
    pub items: Vec<PaletteItem>,
}

impl CommandPalette {
    fn filtered(&self) -> Vec<(usize, &PaletteItem)> {
        let q = self.query.to_ascii_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| q.is_empty() || it.label.to_ascii_lowercase().contains(&q))
            .collect()
    }
}

impl Control for CommandPalette {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::new(420.0, 280.0)
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        let b = widget.screen_bounds();
        if pos.y < b.y + 28.0 {
            CursorKind::Text
        } else {
            CursorKind::Pointer
        }
    }

    fn is_text_input(&self) -> bool {
        true
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // The palette is the highest ordinary surface in the shell: modal rung,
        // modal radius, and the focus border it already had.
        let t = theme::active();
        ctx.push_drop_shadow_rounded(b, [t.geometry.radius_modal; 4], t.elevation.modal);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(b, t.semantic.surface.popup.bytes())
                .with_radius(t.geometry.radius_modal)
                .with_border(t.geometry.stroke_focus, t.semantic.border.focus.bytes()),
            None,
        );
        let search = Rect::new(b.x + 8.0, b.y + 8.0, b.w - 16.0, 22.0);
        ctx.push_paint(search, &crate::style::input(crate::style::VisualState::rest()));
        let shown = if self.query.is_empty() {
            "Search commands…"
        } else {
            self.query.as_str()
        };
        let color = if self.query.is_empty() {
            t.semantic.text.disabled.bytes()
        } else {
            t.semantic.text.primary.bytes()
        };
        ctx.push_text(
            shown,
            Vec2::new(search.x + 6.0, search.y + 4.0),
            self.font_id,
            12.0,
            color,
        );

        let filtered = self.filtered();
        let sel = self.selected.min(filtered.len().saturating_sub(1));
        for (row, (_orig, item)) in filtered.iter().take(10).enumerate() {
            let y = b.y + 36.0 + row as f32 * 22.0;
            let row_r = Rect::new(b.x + 8.0, y, b.w - 16.0, 22.0);
            if row == sel {
                let selected = crate::style::tree_row(crate::style::VisualState::with(
                    crate::style::Interaction::Selected,
                ));
                ctx.push_paint(row_r, &selected);
            }
            ctx.push_text(
                &item.label,
                Vec2::new(row_r.x + 8.0, y + 4.0),
                self.font_id,
                12.0,
                t.semantic.text.primary.bytes(),
            );
            if !item.hint.is_empty() {
                ctx.push_text(
                    &item.hint,
                    Vec2::new(row_r.x + row_r.w - 80.0, y + 4.0),
                    self.font_id,
                    11.0,
                    t.semantic.text.secondary.bytes(),
                );
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(CommandPaletteMessage::SetQuery(q)) = msg.data::<CommandPaletteMessage>() {
            self.query = q.clone();
            self.selected = 0;
            msg.handled = true;
            return;
        }
        let Some(wmsg) = msg.data::<WidgetMessage>() else {
            return;
        };
        match wmsg.clone() {
            WidgetMessage::Text(s) => {
                self.query.push_str(&s);
                self.selected = 0;
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    CommandPaletteMessage::Query(self.query.clone()),
                ));
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Backspace) => {
                self.query.pop();
                self.selected = 0;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowDown) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowUp) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + n - 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Enter | KeyCode::NumpadEnter) => {
                let filtered = self.filtered();
                if let Some((orig, _)) = filtered.get(self.selected) {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        CommandPaletteMessage::Run(*orig),
                    ));
                }
                msg.handled = true;
            }
            WidgetMessage::MouseDown { pos, .. } => {
                let b = widget.screen_bounds();
                if pos.y > b.y + 36.0 {
                    let row = ((pos.y - b.y - 36.0) / 22.0).floor() as usize;
                    let filtered = self.filtered();
                    if let Some((orig, _)) = filtered.get(row) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            CommandPaletteMessage::Run(*orig),
                        ));
                    }
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
}

pub struct CommandPaletteBuilder {
    widget: WidgetBuilder,
    font_id: u8,
    items: Vec<PaletteItem>,
}

impl CommandPaletteBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            font_id: 0,
            items: Vec::new(),
        }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn with_items(mut self, items: Vec<PaletteItem>) -> Self {
        self.items = items;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(CommandPalette {
                query: String::new(),
                selected: 0,
                font_id: self.font_id,
                items: self.items,
            }),
        )
    }
}
