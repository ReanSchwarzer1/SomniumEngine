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

/// Width reserved on the right of every row for the lock and hide badges.
/// A constant rather than a layout pass because the Outliner's rows are a
/// fixed-height list and the column has to line up perfectly for the
/// drag-down bulk toggle to work.
pub const BADGE_COLUMN: f32 = 34.0;

#[derive(Clone, Debug)]
pub struct TreeItem {
    pub id: u32,
    pub label: String,
    pub depth: u8,
    pub icon: IconId,
    pub has_children: bool,
    pub expanded: bool,
    /// Hidden in the viewport. Drawn dimmed, with the eye badge struck out.
    pub hidden: bool,
    /// Locked against viewport picking and transforms.
    pub locked: bool,
    /// This entity's scripts failed to compile.
    pub script_error: bool,
}

#[derive(Debug, Clone)]
pub enum TreeViewMessage {
    Select(u32),
    /// A click in the badge gutter. `lock` distinguishes the two columns.
    ToggleBadge {
        id: u32,
        lock: bool,
    },
    ToggleExpand(u32),
    SetItems(Vec<TreeItem>),
    SetSelected(Option<u32>),
    /// CONTROL-F: everything selected, primary included. The widget paints the
    /// set and reports clicks; it does not decide what a selection is.
    SetSelectedSet(Vec<u32>),
}

pub struct TreeView {
    pub items: Vec<TreeItem>,
    /// The primary. It alone carries the strong text cut and the focus ring,
    /// because it is the gizmo pivot and the Details subject.
    pub selected: Option<u32>,
    /// Every selected row. Contains the primary once the host has published a
    /// set; empty means "primary only", which is what a fresh tree shows.
    pub selected_set: Vec<u32>,
    pub font_id: u8,
    pub px: f32,
    /// Index of the row under the cursor, so hover is a row wash rather than a
    /// whole-widget one. `None` while the pointer is outside.
    hovered: Option<usize>,
    /// A badge-column drag is in flight; the flag says which column.
    badge_drag: Option<bool>,
}

impl Control for TreeView {
    // MORROWIND-I. A tree of selectable rows reads as a list: Somnium's
    // outliner is the case, and `Role::List` is what a reader can navigate
    // with its list commands.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::List
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn focus_bounds(&self, widget: &Widget) -> Rect {
        let b = widget.screen_bounds();
        let index = self
            .selected
            .and_then(|id| self.items.iter().position(|item| item.id == id))
            .unwrap_or(0);
        Rect::new(
            b.x,
            b.y + index as f32 * theme::TREE_ROW_HEIGHT,
            b.w,
            theme::TREE_ROW_HEIGHT,
        )
    }

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
            let primary = self.selected == Some(item.id);
            let selected = primary || self.selected_set.contains(&item.id);
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
            let mut style = if primary { selected_style } else { rest_style };
            if item.hidden {
                style = style.with_color(theme::TEXT_DISABLED);
            }
            // Badges live in a fixed right-hand gutter so the eye is always in
            // the same place, which is what makes click-and-drag down the
            // column a usable bulk toggle rather than a game of hit the target.
            let badge_x = b.x + b.w - BADGE_COLUMN;
            let mid = y + theme::TREE_ROW_HEIGHT * 0.5;
            let t = theme::active();
            if item.script_error {
                let dot = Rect::new(badge_x - 14.0, mid - 3.0, 6.0, 6.0);
                ctx.push_round_rect(dot, 3.0, t.semantic.status.error.bytes());
            }
            if item.locked {
                let bar = Rect::new(badge_x + 2.0, mid - 5.0, 8.0, 10.0);
                ctx.push_round_rect_border(bar, 2.0, 1.0, t.semantic.text.secondary.bytes());
            }
            if item.hidden {
                let bar = Rect::new(badge_x + 14.0, mid - 1.0, 12.0, 2.0);
                ctx.push_rect_filled(bar, t.semantic.text.secondary.bytes());
            }
            let indent = 8.0 + item.depth as f32 * 14.0;
            // Hierarchy guides: one hairline per ancestor level, so a deep
            // tree reads as a tree rather than as a list of varying margins.
            for level in 0..item.depth {
                let x = b.x + 8.0 + level as f32 * 14.0 + 7.0;
                ctx.push_rect_filled(
                    Rect::new(x, y, 1.0, theme::TREE_ROW_HEIGHT),
                    theme::active().semantic.border.subtle.bytes(),
                );
            }
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
        if let Some(TreeViewMessage::SetSelectedSet(ids)) = msg.data::<TreeViewMessage>() {
            self.selected_set = ids.clone();
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseMove { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let idx = ((pos.y - b.y) / theme::TREE_ROW_HEIGHT).floor();
            let row = (idx >= 0.0 && (idx as usize) < self.items.len()).then(|| idx as usize);
            // Godot 4.8's bulk toggle: press in the badge column and drag down
            // to set a run of rows. Each row is reported once, when the drag
            // first reaches it.
            if let Some(lock) = self.badge_drag
                && let Some(row) = row
                && self.hovered != Some(row)
            {
                let id = self.items[row].id;
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    TreeViewMessage::ToggleBadge { id, lock },
                ));
            }
            self.hovered = row;
        }
        if msg
            .data::<WidgetMessage>()
            .is_some_and(|m| matches!(m, WidgetMessage::MouseUp { .. }))
        {
            self.badge_drag = None;
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
                let badge_x = b.x + b.w - BADGE_COLUMN;
                if pos.x >= badge_x {
                    // The badge gutter. `lock` is the left of the two columns.
                    let id = item.id;
                    let lock = pos.x < badge_x + 12.0;
                    self.badge_drag = Some(lock);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        TreeViewMessage::ToggleBadge { id, lock },
                    ));
                    msg.handled = true;
                    return;
                }
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
        if let Some(WidgetMessage::KeyDown(key, _)) = msg.data::<WidgetMessage>() {
            use crate::message::KeyCode;
            if self.items.is_empty() {
                return;
            }
            let current = self
                .selected
                .and_then(|id| self.items.iter().position(|item| item.id == id));
            let select = |index: usize, this: &mut Self, emit: &mut Vec<UiMessage>| {
                let id = this.items[index].id;
                this.selected = Some(id);
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    TreeViewMessage::Select(id),
                ));
            };
            match key {
                KeyCode::ArrowDown => {
                    select(
                        current.map_or(0, |i| (i + 1).min(self.items.len() - 1)),
                        self,
                        emit,
                    );
                    msg.handled = true;
                }
                KeyCode::ArrowUp => {
                    select(current.map_or(0, |i| i.saturating_sub(1)), self, emit);
                    msg.handled = true;
                }
                KeyCode::Home => {
                    select(0, self, emit);
                    msg.handled = true;
                }
                KeyCode::End => {
                    select(self.items.len() - 1, self, emit);
                    msg.handled = true;
                }
                KeyCode::ArrowRight => {
                    if let Some(index) = current {
                        let item = &self.items[index];
                        if item.has_children && !item.expanded {
                            emit.push(UiMessage::new(
                                widget.handle,
                                MessageDirection::FromWidget,
                                TreeViewMessage::ToggleExpand(item.id),
                            ));
                        } else if item.has_children && index + 1 < self.items.len() {
                            select(index + 1, self, emit);
                        }
                        msg.handled = true;
                    }
                }
                KeyCode::ArrowLeft => {
                    if let Some(index) = current {
                        let item = &self.items[index];
                        if item.has_children && item.expanded {
                            emit.push(UiMessage::new(
                                widget.handle,
                                MessageDirection::FromWidget,
                                TreeViewMessage::ToggleExpand(item.id),
                            ));
                        } else if item.depth > 0 {
                            if let Some(parent) = (0..index)
                                .rev()
                                .find(|candidate| self.items[*candidate].depth < item.depth)
                            {
                                select(parent, self, emit);
                            }
                        }
                        msg.handled = true;
                    }
                }
                _ => {}
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
                selected_set: Vec::new(),
                font_id: self.font_id,
                px: self.px,
                hovered: None,
                badge_drag: None,
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
            selected_set: Vec::new(),
            font_id: 0,
            px: 12.0,
            hovered: None,
            badge_drag: None,
        };
        assert!(t.selected.is_none());
    }
}
