// SearchBox / Breadcrumb / PropertyRow / Tooltip helpers (Phase 26-B).

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::{Rect, Thickness},
    widget::{Widget, WidgetBuilder},
    widgets::{
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        text_box::TextBoxBuilder,
    },
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum SearchBoxMessage {
    Query(String),
}

pub struct SearchBox {
    pub query: String,
    pub font_id: u8,
    pub focused: bool,
}

impl Control for SearchBox {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(available.x.max(80.0), theme::ROW_HEIGHT)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        ctx.push_rect_filled(b, theme::BG_INPUT);
        ctx.push_rect_border(
            b,
            1.0,
            if self.focused {
                theme::BORDER_FOCUS
            } else {
                theme::BORDER_MEDIUM
            },
        );
        let ic = Rect::new(b.x + 4.0, b.y + 3.0, 16.0, 16.0);
        let (uv, tex) = IconId::Search.draw_quad(ic);
        ctx.push_textured_rect(ic, uv, theme::TEXT_SECONDARY, tex);
        let shown = if self.query.is_empty() {
            "Search"
        } else {
            self.query.as_str()
        };
        let color = if self.query.is_empty() {
            theme::TEXT_DISABLED
        } else {
            theme::TEXT_PRIMARY
        };
        ctx.push_text(
            shown,
            Vec2::new(b.x + 22.0, b.y + 4.0),
            self.font_id,
            12.0,
            color,
        );
    }

    fn is_text_input(&self) -> bool {
        self.focused
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
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::Focus => {
                    self.focused = true;
                    msg.handled = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                    msg.handled = true;
                }
                WidgetMessage::Text(s) => {
                    self.query.push_str(s);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(self.query.clone()),
                    ));
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(crate::message::KeyCode::Backspace) => {
                    self.query.pop();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(self.query.clone()),
                    ));
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(crate::message::KeyCode::Escape) => {
                    self.query.clear();
                    self.focused = false;
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(String::new()),
                    ));
                    msg.handled = true;
                }
                _ => {}
            }
        }
    }
}

pub struct SearchBoxBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl SearchBoxBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(SearchBox {
                query: String::new(),
                font_id: self.font_id,
                focused: false,
            }),
        )
    }
}

#[derive(Debug, Clone)]
pub enum BreadcrumbMessage {
    Navigate(usize),
    SetParts(Vec<String>),
}

pub struct Breadcrumb {
    pub parts: Vec<String>,
    pub font_id: u8,
}

impl Control for Breadcrumb {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let w: f32 = self
            .parts
            .iter()
            .map(|p| ctx.measure_text(p, 11.0, self.font_id).x + 16.0)
            .sum();
        Vec2::new(available.x.max(w), 20.0)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let mut x = b.x + 4.0;
        for (i, part) in self.parts.iter().enumerate() {
            ctx.push_text(
                part,
                Vec2::new(x, b.y + 3.0),
                self.font_id,
                11.0,
                theme::TEXT_LINK,
            );
            x += 8.0 + part.len() as f32 * 6.5;
            if i + 1 < self.parts.len() {
                ctx.push_text(
                    "/",
                    Vec2::new(x, b.y + 3.0),
                    self.font_id,
                    11.0,
                    theme::TEXT_SECONDARY,
                );
                x += 12.0;
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(BreadcrumbMessage::SetParts(parts)) = msg.data::<BreadcrumbMessage>() {
            self.parts = parts.clone();
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let mut x = b.x + 4.0;
            for (i, part) in self.parts.iter().enumerate() {
                let w = 8.0 + part.len() as f32 * 6.5;
                if pos.x >= x && pos.x < x + w {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        BreadcrumbMessage::Navigate(i),
                    ));
                    msg.handled = true;
                    return;
                }
                x += w + 12.0;
            }
        }
    }
}

pub struct BreadcrumbBuilder {
    widget: WidgetBuilder,
    parts: Vec<String>,
    font_id: u8,
}

impl BreadcrumbBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            parts: Vec::new(),
            font_id: 0,
        }
    }
    pub fn with_parts(mut self, p: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.parts = p.into_iter().map(Into::into).collect();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Breadcrumb {
                parts: self.parts,
                font_id: self.font_id,
            }),
        )
    }
}

/// Label + control row used by Details.
pub fn build_property_row(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    label: &str,
    font_id: u8,
    control: UiNode,
) -> (NodeHandle, NodeHandle) {
    let row = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_clip_to_bounds(false)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let row_h = ui.add_node(row, parent);
    let lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_width(110.0)
            .with_margin(Thickness {
                left: 8.0,
                top: 4.0,
                right: 4.0,
                bottom: 0.0,
            }),
    )
    .with_text(label)
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(lbl, row_h);
    let control_h = ui.add_node(control, row_h);
    (row_h, control_h)
}

pub fn build_search_field(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> NodeHandle {
    let node = SearchBoxBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT))
        .with_font_id(font_id)
        .build();
    ui.add_node(node, parent)
}

/// Unused TextBox helper kept so TextBox stays in the editor toolkit.
#[allow(dead_code)]
pub fn build_labelled_text_box(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> NodeHandle {
    let box_ = TextBoxBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT)).build();
    let _ = font_id;
    ui.add_node(box_, parent)
}

pub struct Tooltip {
    pub text: String,
    pub font_id: u8,
}

impl Control for Tooltip {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        ctx.measure_text(&self.text, 11.0, self.font_id) + Vec2::new(12.0, 8.0)
    }
    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        ctx.push_rect_filled(b, theme::BG_RAISED);
        ctx.push_rect_border(b, 1.0, theme::BORDER_MEDIUM);
        ctx.push_text(
            &self.text,
            Vec2::new(b.x + 6.0, b.y + 4.0),
            self.font_id,
            11.0,
            theme::TEXT_PRIMARY,
        );
    }
    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(TextMessage::SetText(s)) = msg.data::<TextMessage>() {
            self.text = s.clone();
            widget.invalidate_layout();
            msg.handled = true;
        }
    }
}

pub struct TooltipBuilder {
    widget: WidgetBuilder,
    text: String,
    font_id: u8,
}

impl TooltipBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            text: String::new(),
            font_id: 0,
        }
    }
    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.with_hit_test_visibility(false).build(),
            Box::new(Tooltip {
                text: self.text,
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_starts_empty() {
        let s = SearchBox {
            query: String::new(),
            font_id: 0,
            focused: false,
        };
        assert!(s.query.is_empty());
    }
}
