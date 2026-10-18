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
    /// Opening reveals the current row without resetting scroll on model refresh.
    RevealSelected,
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
    /// Asset files aligned with `items`, when the combo picks an asset; what `swatch` shows.
    pub asset_paths: Vec<Option<std::path::PathBuf>>,
    /// An [`AssetSwatch`] beside the combo that shows the chosen asset's picture, or `NONE`.
    pub swatch: NodeHandle,
    /// See [`super::MIXED_PLACEHOLDER`]. Cleared on the first choice.
    pub mixed: bool,
    pub open: bool,
    pub font_id: u8,
    pub px: f32,
    pub popup: NodeHandle,
    pub list: NodeHandle,
}

impl ComboBox {
    /// Show the chosen asset (none while the row is mixed) on the swatch, if there is one.
    fn show_on_swatch(&self, emit: &mut Vec<UiMessage>) {
        if self.swatch.is_some() {
            let path = if self.mixed { None } else { self.asset_paths.get(self.selected).cloned().flatten() };
            emit.push(UiMessage::new(self.swatch, MessageDirection::ToWidget, AssetSwatchMessage::SetAsset(path)));
        }
    }
}

impl Control for ComboBox {
    fn owned_popup(&self) -> Option<NodeHandle> {
        self.popup.is_some().then_some(self.popup)
    }

    // MORROWIND-I.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::ComboBox
    }

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
            available.x
        } else {
            w.max(80.0)
        };
        Vec2::new(width, theme::active().density.row_dense)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let header = Rect::new(b.x, b.y, b.w, theme::active().density.row_dense);
        // The closed header is a chrome control, so it takes the button recipe:
        // radius, wash and the raised rung. It keeps the hairline outline the
        // pre-Styx header had, because an outline is how a combo says it opens
        // something — a raised fill alone reads as a plain button.
        let t = theme::active();
        let mut paint = crate::style::button(crate::style::VisualState::rest());
        paint.border = t.semantic.border.default.bytes();
        paint.border_thickness = t.geometry.stroke_hairline;
        ctx.push_paint(header, &paint);
        let label = if self.mixed {
            super::MIXED_PLACEHOLDER
        } else {
            self.items
                .get(self.selected)
                .map(|s| s.as_str())
                .unwrap_or("")
        };
        let (label, _) = ctx.font_atlas.ellipsise(label, (b.w - 32.0).max(0.0), self.px, self.font_id);
        ctx.push_text(
            &label,
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
        if let Some(crate::message::MixedValue(mixed)) = msg.data::<crate::message::MixedValue>() {
            if !self.open {
                self.mixed = *mixed;
                self.show_on_swatch(emit);
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetAssetPaths(paths)) = msg.data::<ComboBoxMessage>() {
            // From the list beside it (see `ComboDropdown`); not passed back, which would loop.
            self.asset_paths = paths.clone();
            self.show_on_swatch(emit);
            msg.handled = true;
            return;
        }

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
                self.show_on_swatch(emit);
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetItems(items)) = msg.data::<ComboBoxMessage>() {
            self.items = items.clone();
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
            widget.invalidate_layout();
            if self.list.is_some() {
                emit.push(UiMessage::new(
                    self.list,
                    MessageDirection::ToWidget,
                    ComboBoxMessage::SetItems(items.clone()),
                ));
            }
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
            self.mixed = false;
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
                if self.list.is_some() {
                    emit.push(UiMessage::new(
                        self.list,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::RevealSelected,
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
    mixed: bool,
    asset_paths: Vec<Option<std::path::PathBuf>>,
    swatch: NodeHandle,
    items: Vec<String>,
    selected: usize,
    font_id: u8,
    px: f32,
}

impl ComboBoxBuilder {
    /// Display [`super::MIXED_PLACEHOLDER`] until the control is touched.
    /// Multi-selection is the only caller; a single selection never sets it.
    pub fn with_mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            mixed: false,
            asset_paths: Vec::new(),
            swatch: NodeHandle::NONE,
            widget,
            items: Vec::new(),
            selected: 0,
            font_id: 0,
            px: 12.0,
        }
    }
    /// The asset each item stands for, for the swatch.
    pub fn with_asset_paths(mut self, paths: impl IntoIterator<Item = Option<std::path::PathBuf>>) -> Self {
        self.asset_paths = paths.into_iter().collect();
        self
    }
    /// An [`AssetSwatch`] to keep showing the chosen asset's picture.
    pub fn with_swatch(mut self, swatch: NodeHandle) -> Self {
        self.swatch = swatch;
        self
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
                mixed: self.mixed,
                asset_paths: self.asset_paths,
                swatch: self.swatch,
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

impl ComboDropdown {
    fn reveal_selected(&self, widget: &Widget, emit: &mut Vec<UiMessage>) {
        emit.push(UiMessage::new(
            widget.parent,
            MessageDirection::ToWidget,
            super::scroll_viewer::ScrollViewerMessage::ScrollTo(
                self.selected.saturating_sub(2) as f32 * theme::active().density.row_dense,
            ),
        ));
    }
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
            available.x
        } else {
            w
        };
        Vec2::new(
            width,
            (self.items.len().max(1) as f32) * theme::active().density.row_dense,
        )
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // The shared picker frame owns the surface. Only visible rows need text
        // and thumbnail work, even when an asset database has thousands of entries.
        let t = theme::active();
        let clip = ctx.clip_rect();
        let row_h = t.density.row_dense;
        let first = (((clip.y - b.y) / row_h).floor().max(0.0) as usize).min(self.items.len());
        let end =
            (((clip.y + clip.h - b.y) / row_h).ceil().max(0.0) as usize).min(self.items.len());
        for i in first..end {
            let item = &self.items[i];
            let row = Rect::new(
                b.x,
                b.y + theme::active().density.row_dense * i as f32,
                b.w,
                theme::active().density.row_dense,
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
                    let mut picture = crate::primitive::Primitive::textured(icon, uv, [255; 4]);
                    picture.flags |= crate::primitive::FLAG_SRGB_TEXELS;
                    ctx.push_primitive(picture, Some(crate::thumbnail::THUMBNAIL_ATLAS_TEXTURE_ID));
                }
                row.x + 28.0
            } else {
                row.x + 8.0
            };
            let (label, _) = ctx.font_atlas.ellipsise(
                item,
                (row.x + row.w - text_x - 8.0).max(0.0),
                self.px,
                self.font_id,
            );
            ctx.push_text(
                &label,
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
        if let Some(ComboBoxMessage::RevealSelected) = msg.data::<ComboBoxMessage>() {
            self.reveal_selected(widget, emit);
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetSelected(i)) = msg.data::<ComboBoxMessage>() {
            if *i < self.items.len() && self.selected != *i {
                self.selected = *i;
                self.reveal_selected(widget, emit);
            }
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetItems(items)) = msg.data::<ComboBoxMessage>() {
            self.items = items.clone();
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
            widget.invalidate_layout();
            emit.push(UiMessage::new(
                widget.parent,
                MessageDirection::ToWidget,
                super::scroll_viewer::ScrollViewerMessage::ScrollTo(0.0),
            ));
            msg.handled = true;
            return;
        }
        if let Some(ComboBoxMessage::SetAssetPaths(paths)) = msg.data::<ComboBoxMessage>() {
            self.asset_paths = paths.clone();
            // The combo's swatch follows the same list (a choice added after the picker was built).
            if self.combo.is_some() {
                emit.push(UiMessage::new(
                    self.combo,
                    MessageDirection::ToWidget,
                    ComboBoxMessage::SetAssetPaths(paths.clone()),
                ));
            }
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            if b.w <= 0.0 || b.h <= 0.0 {
                return;
            }
            let idx = ((pos.y - b.y) / theme::active().density.row_dense).floor() as isize;
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

/// The picture beside an asset picker: the chosen asset's thumbnail (asked for when first drawn), or `icon` while
/// there is none. A texture slot's swatch shows the texture it holds.
pub struct AssetSwatch {
    pub asset: Option<std::path::PathBuf>,
    pub icon: IconId,
    pub size: f32,
    /// Drawn in a sunken well (a slot), or bare (a chip's picture).
    pub framed: bool,
}

/// Change the asset an [`AssetSwatch`] shows; its combo sends this whenever the choice changes.
#[derive(Clone, Debug, PartialEq)]
pub enum AssetSwatchMessage {
    SetAsset(Option<std::path::PathBuf>),
}

impl Control for AssetSwatch {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Image
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::splat(self.size)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let side = self.size.min(b.w).min(b.h);
        let rect = Rect::new(b.x + (b.w - side) * 0.5, b.y + (b.h - side) * 0.5, side, side);
        let t = theme::active();
        let inner = if self.framed {
            // A sunken well, flat: the picture is the subject, not the frame.
            let mut frame = crate::style::button(crate::style::VisualState::rest());
            frame.background = t.semantic.surface.canvas.bytes();
            frame.gradient = None;
            frame.elevation = None;
            frame.border = t.semantic.border.default.bytes();
            frame.border_thickness = t.geometry.stroke_hairline;
            ctx.push_paint(rect, &frame);
            Rect::new(rect.x + 2.0, rect.y + 2.0, (rect.w - 4.0).max(0.0), (rect.h - 4.0).max(0.0))
        } else {
            rect
        };
        if let Some(path) = self.asset.as_deref() {
            match ctx.thumbnails.uv(path) {
                Some(uv) => {
                    // The thumbnail's texels are sRGB colour: decoded, so the slot shows the texture's own colours.
                    let mut quad = crate::primitive::Primitive::textured(inner, uv, [255; 4]);
                    quad.flags |= crate::primitive::FLAG_SRGB_TEXELS;
                    ctx.push_primitive(quad, Some(crate::thumbnail::THUMBNAIL_ATLAS_TEXTURE_ID));
                    return;
                }
                // Asked for once; a texture that has no preview stays on the icon.
                None => ctx.thumbnails.request(path, true),
            }
        }
        let glyph = Rect::new(inner.x + inner.w * 0.25, inner.y + inner.h * 0.25, inner.w * 0.5, inner.h * 0.5);
        let (uv, tex) = self.icon.draw_quad(glyph);
        ctx.push_textured_rect(glyph, uv, t.semantic.text.disabled.bytes(), tex);
    }

    fn handle_routed_message(&mut self, _widget: &mut Widget, msg: &mut UiMessage, _emit: &mut Vec<UiMessage>) {
        if let Some(AssetSwatchMessage::SetAsset(path)) = msg.data::<AssetSwatchMessage>() {
            self.asset = path.clone();
            msg.handled = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A texture slot's swatch follows its picker: a model refresh, a mixed selection, and a choice the list learned
    /// after it was built.
    #[test]
    fn the_swatch_follows_the_pickers_choice() {
        let mut ui = crate::ui::UserInterface::new(400.0, 300.0);
        let root = ui.root();
        let swatch = ui.add_node(
            UiNode::new(WidgetBuilder::new().build(), Box::new(AssetSwatch { asset: None, icon: IconId::Texture, size: 40.0, framed: true })),
            root,
        );
        let paths = vec![None, Some(std::path::PathBuf::from("a.png")), Some(std::path::PathBuf::from("b.png"))];
        let combo = ui.add_node(
            ComboBoxBuilder::new(WidgetBuilder::new())
                .with_items(["None", "a", "b"])
                .with_asset_paths(paths.clone())
                .with_swatch(swatch)
                .build(),
            root,
        );
        let list = ui.add_node(
            ComboDropdownBuilder::new(WidgetBuilder::new()).with_items(["None", "a", "b"]).with_asset_paths(paths).with_combo(combo).build(),
            root,
        );
        ui.send(ComboBoxMessage::bind_popup(combo, NodeHandle::NONE, list));
        // What the swatch shows, read back as the thumbnail it asks the cache for when drawn.
        let shown = |ui: &mut crate::ui::UserInterface| {
            ui.update();
            ui.update();
            ui.draw_ctx.thumbnails.clear();
            let node = ui.nodes.borrow(swatch.transmute());
            node.control.draw(&node.widget, &mut ui.draw_ctx);
            ["a.png", "b.png"]
                .iter()
                .position(|p| ui.draw_ctx.thumbnails.state(std::path::Path::new(p)).is_some())
        };
        ui.send(ComboBoxMessage::set_selected(combo, 2));
        assert_eq!(shown(&mut ui), Some(1), "a refresh to b shows b");
        ui.send(UiMessage::new(combo, MessageDirection::ToWidget, crate::message::MixedValue(true)));
        assert_eq!(shown(&mut ui), None, "a mixed row shows no texture");
        ui.send(UiMessage::new(combo, MessageDirection::ToWidget, crate::message::MixedValue(false)));
        ui.send(ComboBoxMessage::set_selected(combo, 1));
        assert_eq!(shown(&mut ui), Some(0));
        // The list learns a fourth choice after it was built; the swatch can show it.
        let items: Vec<String> = ["None", "x", "y", "b"].map(String::from).to_vec();
        ui.send(UiMessage::new(combo, MessageDirection::ToWidget, ComboBoxMessage::SetItems(items)));
        ui.send(UiMessage::new(
            list,
            MessageDirection::ToWidget,
            ComboBoxMessage::SetAssetPaths(vec![None, None, None, Some(std::path::PathBuf::from("b.png"))]),
        ));
        ui.update();
        ui.send(ComboBoxMessage::set_selected(combo, 3));
        assert_eq!(shown(&mut ui), Some(1));
    }

    #[test]
    fn selection_stays_in_range() {
        let cb = ComboBox {
            asset_paths: Vec::new(),
            swatch: NodeHandle::NONE,
            mixed: false,
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
