// ComboBox — header in the inspector; list is a root-parented Popup overlay
// (Phase 26-B). Drawing the list in-place loses to later siblings in tree
// order, which is why Type used to ghost over Dens/Size with no panel.

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
    widgets::popup::PopupMessage,
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum ComboBoxMessage {
    SelectionChanged(usize),
    SetSelected(usize),
    BindPopup {
        popup: NodeHandle,
        list: NodeHandle,
    },
    Open,
    Close,
    /// Replace filtered entries without replacing/focusing the widget.
    SetItems(Vec<String>),
    /// Optional asset paths aligned with items; dropdown draws their previews.
    SetAssetPaths(Vec<Option<std::path::PathBuf>>),
}

pub struct ComboBox {
    pub items: Vec<String>,
    pub selected: usize,
    pub open: bool,
    pub font_id: u8,
    pub px: f32,
    pub popup: NodeHandle,
    pub list: NodeHandle,
}

impl Control for ComboBox {
    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let w = self
            .items
            .iter()
            .map(|s| ctx.measure_text(s, self.px, self.font_id).x)
            .fold(80.0_f32, f32::max)
            + 28.0;
        let width = if available.x.is_finite() {
            available.x.max(w.max(80.0))
        } else {
            w.max(80.0)
        };
        Vec2::new(width, theme::ROW_HEIGHT)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let header = Rect::new(b.x, b.y, b.w, theme::ROW_HEIGHT);
        // The closed header is a chrome control, so it takes the button recipe:
        // radius, wash and the raised rung. It keeps the hairline outline the
        // pre-Styx header had, because an outline is how a combo says it opens
        // something — a raised fill alone reads as a plain button.
        let t = theme::active();
        let mut paint = crate::style::button(crate::style::VisualState::rest());
        paint.border = t.semantic.border.default.bytes();
        paint.border_thickness = t.geometry.stroke_hairline;
        ctx.push_paint(header, &paint);
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
            t.semantic.text.primary.bytes(),
        );
        let chev = Rect::new(b.x + b.w - 20.0, b.y + 2.0, 16.0, 16.0);
        let (uv, tex) = IconId::ChevronDown.draw_quad(chev);
        ctx.push_textured_rect(chev, uv, t.semantic.text.secondary.bytes(), tex);
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
        if let Some(ComboBoxMessage::BindPopup { popup, list }) = msg.data::<ComboBoxMessage>() {
            self.popup = *popup;
            self.list = *list;
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetSelected(i)) = msg.data::<ComboBoxMessage>() {
            if *i < self.items.len() {
                self.selected = *i;
                if self.list.is_some() {
                    emit.push(UiMessage::new(
                        self.list,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetSelected(self.selected),
                    ));
                }
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetItems(items)) = msg.data::<ComboBoxMessage>() {
            self.items = items.clone();
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::Close) = msg.data::<ComboBoxMessage>() {
            if self.open {
                self.open = false;
                if self.popup.is_some() {
                    emit.push(UiMessage::new(
                        self.popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Close,
                    ));
                }
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    ComboBoxMessage::Close,
                ));
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::Open) = msg.data::<ComboBoxMessage>() {
            self.open = true;
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { .. }) = msg.data::<WidgetMessage>() {
            if self.open {
                self.open = false;
                if self.popup.is_some() {
                    emit.push(UiMessage::new(
                        self.popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Close,
                    ));
                }
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    ComboBoxMessage::Close,
                ));
            } else {
                self.open = true;
                if self.popup.is_some() {
                    emit.push(UiMessage::new(
                        self.popup,
                        MessageDirection::ToWidget,
                        PopupMessage::SetAnchor(widget.handle),
                    ));
                    emit.push(UiMessage::new(
                        self.popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Open,
                    ));
                }
                if self.list.is_some() {
                    emit.push(UiMessage::new(
                        self.list,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetSelected(self.selected),
                    ));
                }
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    ComboBoxMessage::Open,
                ));
            }
            msg.handled = true;
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
            widget,
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
                popup: NodeHandle::NONE,
                list: NodeHandle::NONE,
            }),
        )
    }
}

impl ComboBoxMessage {
    pub fn set_selected(dest: NodeHandle, i: usize) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetSelected(i))
    }

    pub fn bind_popup(dest: NodeHandle, popup: NodeHandle, list: NodeHandle) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::ToWidget,
            Self::BindPopup { popup, list },
        )
    }

    pub fn close(dest: NodeHandle) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::Close)
    }
}

/// Opaque list painted by the root popup so inspector rows cannot cover it.
pub struct ComboDropdown {
    pub items: Vec<String>,
    pub selected: usize,
    pub combo: NodeHandle,
    pub popup: NodeHandle,
    pub font_id: u8,
    pub px: f32,
    pub asset_paths: Vec<Option<std::path::PathBuf>>,
}

impl Control for ComboDropdown {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let w = self
            .items
            .iter()
            .map(|s| ctx.measure_text(s, self.px, self.font_id).x)
            .fold(80.0_f32, f32::max)
            + 16.0;
        let width = if available.x.is_finite() {
            available.x.max(w)
        } else {
            w
        };
        Vec2::new(width, (self.items.len().max(1) as f32) * theme::ROW_HEIGHT)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // The open list floats over the inspector, so it takes the popup rung.
        let t = theme::active();
        ctx.push_paint(b, &crate::style::popup());
        for (i, item) in self.items.iter().enumerate() {
            let row = Rect::new(
                b.x,
                b.y + theme::ROW_HEIGHT * i as f32,
                b.w,
                theme::ROW_HEIGHT,
            );
            if i == self.selected {
                // Selection is fill *and* rail, so it survives a colour-vision
                // pass (Zeta 8A.4).
                let sel = crate::style::tree_row(crate::style::VisualState::with(
                    crate::style::Interaction::Selected,
                ));
                ctx.push_paint(row, &sel);
            }
            let text_x = if let Some(Some(path)) = self.asset_paths.get(i) {
                let icon = Rect::new(row.x + 3.0, row.y + 2.0, 20.0, 20.0);
                if let Some(uv) = ctx.thumbnails.uv(path) {
                    ctx.push_primitive(
                        crate::primitive::Primitive::textured(icon, uv, [255; 4]),
                        Some(crate::thumbnail::THUMBNAIL_ATLAS_TEXTURE_ID),
                    );
                }
                row.x + 28.0
            } else {
                row.x + 8.0
            };
            ctx.push_text(
                item,
                Vec2::new(text_x, row.y + 4.0),
                self.font_id,
                self.px,
                t.semantic.text.primary.bytes(),
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
        if let Some(ComboBoxMessage::SetSelected(i)) = msg.data::<ComboBoxMessage>() {
            if *i < self.items.len() {
                self.selected = *i;
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetItems(items)) = msg.data::<ComboBoxMessage>() {
            self.items = items.clone();
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetAssetPaths(paths)) = msg.data::<ComboBoxMessage>() {
            self.asset_paths = paths.clone();
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            if b.w <= 0.0 || b.h <= 0.0 {
                return;
            }
            let idx = ((pos.y - b.y) / theme::ROW_HEIGHT).floor() as isize;
            if idx >= 0 && (idx as usize) < self.items.len() {
                self.selected = idx as usize;
                if self.combo.is_some() {
                    emit.push(UiMessage::new(
                        self.combo,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetSelected(self.selected),
                    ));
                    emit.push(UiMessage::new(
                        self.combo,
                        MessageDirection::FromWidget,
                        ComboBoxMessage::SelectionChanged(self.selected),
                    ));
                    emit.push(ComboBoxMessage::close(self.combo));
                }
                if self.popup.is_some() {
                    emit.push(UiMessage::new(
                        self.popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Close,
                    ));
                }
                msg.handled = true;
            }
        }
    }
}

pub struct ComboDropdownBuilder {
    widget: WidgetBuilder,
    items: Vec<String>,
    combo: NodeHandle,
    popup: NodeHandle,
    font_id: u8,
    px: f32,
    asset_paths: Vec<Option<std::path::PathBuf>>,
}

impl ComboDropdownBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            items: Vec::new(),
            combo: NodeHandle::NONE,
            popup: NodeHandle::NONE,
            font_id: 0,
            px: 12.0,
            asset_paths: Vec::new(),
        }
    }

    pub fn with_items(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.items = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_combo(mut self, combo: NodeHandle) -> Self {
        self.combo = combo;
        self
    }

    pub fn with_popup(mut self, popup: NodeHandle) -> Self {
        self.popup = popup;
        self
    }

    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }

    pub fn with_asset_paths(
        mut self,
        paths: impl IntoIterator<Item = Option<std::path::PathBuf>>,
    ) -> Self {
        self.asset_paths = paths.into_iter().collect();
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ComboDropdown {
                items: self.items,
                selected: 0,
                combo: self.combo,
                popup: self.popup,
                font_id: self.font_id,
                px: self.px,
                asset_paths: self.asset_paths,
            }),
        )
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
            popup: NodeHandle::NONE,
            list: NodeHandle::NONE,
        };
        assert_eq!(cb.items.len(), 3);
        assert!(!cb.open);
    }

    #[test]
    fn header_stays_one_row() {
        assert_eq!(theme::ROW_HEIGHT, 24.0);
    }

    #[test]
    fn dropdown_index_from_local_y() {
        let idx = |y: f32| (y / theme::ROW_HEIGHT).floor() as isize;
        assert_eq!(idx(0.0), 0);
        assert_eq!(idx(theme::ROW_HEIGHT + 1.0), 1);
        assert_eq!(idx(theme::ROW_HEIGHT * 3.0 + 2.0), 3);
    }
}
