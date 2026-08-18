// TreeView — outliner + content path tree (Phase 26-B).

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    style::{Interaction, VisualState, tree_row},
    theme,
    types::Rect,
    typography::{TextRole, text_style},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Clone, Debug)]
pub struct TreeItem {
    pub id: u32,
    pub label: String,
    pub depth: u8,
    pub icon: IconId,
    pub has_children: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub enum TreeViewMessage {
    Select(u32),
    ToggleExpand(u32),
    SetItems(Vec<TreeItem>),
    SetSelected(Option<u32>),
}

pub struct TreeView {
    pub items: Vec<TreeItem>,
    pub selected: Option<u32>,
    pub font_id: u8,
    pub px: f32,
    /// Index of the row under the cursor, so hover is a row wash rather than a
    /// whole-widget one. `None` while the pointer is outside.
    hovered: Option<usize>,
}

impl Control for TreeView {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(
            available.x,
            (self.items.len() as f32 * theme::TREE_ROW_HEIGHT).max(theme::TREE_ROW_HEIGHT),
        )
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // Selected rows lift to the `body_strong` cut; the design's state
        // grammar reads weight before it reads the fill.
        let selected_style = text_style(TextRole::BodyStrong);
        let rest_style = text_style(TextRole::Body);
        for (i, item) in self.items.iter().enumerate() {
            let y = b.y + i as f32 * theme::TREE_ROW_HEIGHT;
            let row = Rect::new(b.x, y, b.w, theme::TREE_ROW_HEIGHT);
            let selected = self.selected == Some(item.id);
            let interaction = if selected {
                Interaction::Selected
            } else if self.hovered == Some(i) {
                Interaction::Hover
            } else {
                Interaction::Rest
            };
            let mut paint = tree_row(VisualState::with(interaction));

            // Phase 27-C: cross-fade the row hover. Keyed per row, so moving the
            // pointer down a list fades each row independently instead of
            // flashing the whole Outliner.
            let key = crate::motion::MotionKey::row(
                widget.handle.index(),
                i as u32,
                crate::motion::MotionProperty::HoverWash,
            );
            let target = if interaction == Interaction::Hover {
                1.0
            } else {
                0.0
            };
            ctx.motion.start(
                key,
                0.0,
                target,
                theme::active().motion.hover_ms as f32,
                crate::motion::Easing::Standard,
            );
            let wash = ctx.motion.value_or(key, target);
            // Selection is a different cue entirely and must never be faded
            // through, or a selected row would blink when the pointer crosses it.
            if !selected && wash > 0.0 && wash < 1.0 {
                let rest = tree_row(VisualState::with(Interaction::Rest));
                let hovered = tree_row(VisualState::with(Interaction::Hover));
                paint.background =
                    crate::motion::lerp_color(rest.background, hovered.background, wash);
                paint.foreground =
                    crate::motion::lerp_color(rest.foreground, hovered.foreground, wash);
            }

            // One call renders fill and rail together in the right order, and
            // the rail width comes from the live theme rather than a constant.
            if paint.background[3] != 0 || paint.rail.is_some() {
                ctx.push_paint(row, &paint);
            }
            let style = if selected { selected_style } else { rest_style };
            let indent = 8.0 + item.depth as f32 * 14.0;
            if item.has_children {
                let chev = Rect::new(b.x + indent, y + 6.0, 16.0, 16.0);
                let icon = if item.expanded {
                    IconId::ChevronDown
                } else {
                    IconId::Chevron
                };
                let (uv, tex) = icon.draw_quad(chev);
                ctx.push_textured_rect(
                    chev,
                    uv,
                    theme::active().semantic.text.secondary.bytes(),
                    tex,
                );
            }
            let ic = Rect::new(
                b.x + indent + 18.0,
                y + (theme::TREE_ROW_HEIGHT - theme::ICON_TREE) * 0.5,
                theme::ICON_TREE,
                theme::ICON_TREE,
            );
            let (uv, tex) = item.icon.draw_quad(ic);
            ctx.push_textured_rect(ic, uv, paint.foreground, tex);
            ctx.push_text(
                &item.label,
                Vec2::new(
                    b.x + indent + 18.0 + theme::ICON_TREE + 6.0,
                    y + (theme::TREE_ROW_HEIGHT - style.px) * 0.5 - 1.0,
                ),
                style.font_id(),
                style.px,
                paint.foreground,
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
        if let Some(TreeViewMessage::SetItems(items)) = msg.data::<TreeViewMessage>() {
            self.items = items.clone();
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        if let Some(TreeViewMessage::SetSelected(id)) = msg.data::<TreeViewMessage>() {
            self.selected = *id;
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseMove { pos }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let idx = ((pos.y - b.y) / theme::TREE_ROW_HEIGHT).floor();
            self.hovered = if idx >= 0.0 && (idx as usize) < self.items.len() {
                Some(idx as usize)
            } else {
                None
            };
        }
        if msg
            .data::<WidgetMessage>()
            .is_some_and(|m| matches!(m, WidgetMessage::MouseLeave))
        {
            self.hovered = None;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let idx = ((pos.y - b.y) / theme::TREE_ROW_HEIGHT).floor() as isize;
            if idx >= 0 && (idx as usize) < self.items.len() {
                let item = &self.items[idx as usize];
                let indent = 8.0 + item.depth as f32 * 12.0;
                if item.has_children && pos.x < b.x + indent + 14.0 {
                    let id = item.id;
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        TreeViewMessage::ToggleExpand(id),
                    ));
                } else {
                    let id = item.id;
                    self.selected = Some(id);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        TreeViewMessage::Select(id),
                    ));
                }
                msg.handled = true;
            }
        }
    }
}

pub struct TreeViewBuilder {
    widget: WidgetBuilder,
    font_id: u8,
    px: f32,
}

impl TreeViewBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            font_id: 0,
            px: 12.0,
        }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(TreeView {
                items: Vec::new(),
                selected: None,
                font_id: self.font_id,
                px: self.px,
                hovered: None,
            }),
        )
    }
}

impl TreeViewMessage {
    pub fn set_items(dest: crate::message::NodeHandle, items: Vec<TreeItem>) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetItems(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_has_no_selection() {
        let t = TreeView {
            items: Vec::new(),
            selected: None,
            font_id: 0,
            px: 12.0,
            hovered: None,
        };
        assert!(t.selected.is_none());
    }
}
