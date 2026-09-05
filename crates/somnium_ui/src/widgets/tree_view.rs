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

/// The faintest an unset badge may be drawn.
///
/// Below this it stops being a quiet affordance and becomes an invisible one.
const GHOST_MIN_ALPHA: u8 = 40;

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
    ///
    /// Held by key and kept sorted — see [`crate::virtual_list::KeySelection`]
    /// for why an index-based selection is the thing that appears to jump when
    /// a list is filtered or scrolled.
    pub selection: crate::virtual_list::KeySelection,
    pub font_id: u8,
    pub px: f32,
    /// Index of the row under the cursor, so hover is a row wash rather than a
    /// whole-widget one. `None` while the pointer is outside.
    hovered: Option<usize>,
    focused: bool,
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
            b.y + index as f32 * theme::active().density.row_tree,
            b.w,
            theme::active().density.row_tree,
        )
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(
            available.x,
            (self.items.len() as f32 * theme::active().density.row_tree)
                .max(theme::active().density.row_tree),
        )
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // Selected rows lift to the `body_strong` cut; the design's state
        // grammar reads weight before it reads the fill.
        let selected_style = text_style(TextRole::BodyStrong);
        let rest_style = text_style(TextRole::Body);
        // MORROWIND-M. This loop used to run over every item, shaping a label
        // for each, whether or not the row was on screen — O(total rows) to
        // show the thirty that fit. Inside a scroll viewer the widget is as
        // tall as its content, so the clip is the only thing that knows what
        // is visible.
        let window = crate::virtual_list::RowWindow::new(
            b.y,
            theme::active().density.row_tree,
            self.items.len(),
            ctx.clip_rect(),
        );
        for i in window.range() {
            let item = &self.items[i];
            let y = b.y + i as f32 * theme::active().density.row_tree;
            let row = Rect::new(b.x, y, b.w, theme::active().density.row_tree);
            let primary = self.selected == Some(item.id);
            // Binary search, not a scan. `Vec::contains` per row is
            // O(rows x selected) per frame, which is invisible at ten rows and
            // quadratic at a hundred thousand.
            let selected = primary || self.selection.contains(item.id);
            let interaction = if selected {
                Interaction::Selected
            } else if self.hovered == Some(i) {
                Interaction::Hover
            } else {
                Interaction::Rest
            };
            let mut paint = tree_row(
                VisualState::with(interaction)
                    .inactive(!self.focused)
                    .focused(primary && self.focused),
            );

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
            let mut style =
                if primary { selected_style } else { rest_style }.with_color(paint.foreground);
            if item.hidden {
                style = style.with_color(theme::active().semantic.text.disabled.bytes());
            }
            // Badges live in a fixed right-hand gutter so the eye is always in
            // the same place, which is what makes click-and-drag down the
            // column a usable bulk toggle rather than a game of hit the target.
            let badge_x = b.x + b.w - BADGE_COLUMN;
            let mid = y + theme::active().density.row_tree * 0.5;
            let t = theme::active();
            if item.script_error {
                let dot = Rect::new(badge_x - 14.0, mid - 3.0, 6.0, 6.0);
                ctx.push_round_rect(dot, 3.0, t.semantic.status.error.bytes());
            }
            // **Both badges are drawn on every row**, which is the whole
            // affordance. Before this they appeared only when the flag was
            // *set*, so a visible unlocked row showed an empty gutter — the
            // click target was there and worked, and nothing on screen said so.
            // A control you have to already know about is one nobody finds.
            //
            // The states differ by weight rather than by position, after
            // Unreal: a set flag is at full secondary colour, an unset one is a
            // ghost that firms up when the pointer is on the row. That keeps a
            // long outliner scannable — the hidden rows are the ones you can
            // see from across the column — while leaving every row clickable.
            let hovered_row = self.hovered == Some(i);
            let badge_tint = |set: bool| {
                if set {
                    t.semantic.text.secondary.bytes()
                } else if hovered_row {
                    t.semantic.text.disabled.bytes()
                } else {
                    // Floored, not just divided. `disabled / 3` on a `u8` is
                    // zero for any theme whose disabled text is already
                    // translucent, and a fully transparent badge is the empty
                    // gutter this change exists to remove — reintroduced by a
                    // token change, in a way the primitive-count test cannot
                    // see.
                    let mut ghost = t.semantic.text.disabled.bytes();
                    ghost[3] = (ghost[3] / 3).max(GHOST_MIN_ALPHA);
                    ghost
                }
            };
            let badge = |ctx: &mut DrawingContext, x: f32, icon: IconId, set: bool| {
                let rect = Rect::new(x, mid - 6.0, 12.0, 12.0);
                let (uv, tex) = icon.draw_quad(rect);
                ctx.push_textured_rect(rect, uv, badge_tint(set), tex);
            };
            badge(ctx, badge_x, IconId::Locked, item.locked);
            badge(
                ctx,
                badge_x + 14.0,
                if item.hidden {
                    IconId::VisibilityOff
                } else {
                    IconId::Visibility
                },
                item.hidden,
            );
            let indent = 8.0 + item.depth as f32 * 14.0;
            // Hierarchy guides: one hairline per ancestor level, so a deep
            // tree reads as a tree rather than as a list of varying margins.
            for level in 0..item.depth {
                let x = b.x + 8.0 + level as f32 * 14.0 + 7.0;
                ctx.push_rect_filled(
                    Rect::new(x, y, 1.0, theme::active().density.row_tree),
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
                y + (theme::active().density.row_tree - theme::ICON_TREE) * 0.5,
                theme::ICON_TREE,
                theme::ICON_TREE,
            );
            let (uv, tex) = item.icon.draw_quad(ic);
            ctx.push_textured_rect(ic, uv, paint.foreground, tex);
            ctx.push_text(
                &item.label,
                Vec2::new(
                    b.x + indent + 18.0 + theme::ICON_TREE + 6.0,
                    y + (theme::active().density.row_tree - style.px) * 0.5 - 1.0,
                ),
                style.font_id(),
                style.px,
                style.color,
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
        match msg.data::<WidgetMessage>() {
            Some(WidgetMessage::Focus) => self.focused = true,
            Some(WidgetMessage::Unfocus) => self.focused = false,
            _ => {}
        }
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
            self.selection = crate::virtual_list::KeySelection::from_keys(ids.iter().copied());
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseMove { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let idx = ((pos.y - b.y) / theme::active().density.row_tree).floor();
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
            let idx = ((pos.y - b.y) / theme::active().density.row_tree).floor() as isize;
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
                selection: crate::virtual_list::KeySelection::default(),
                font_id: self.font_id,
                px: self.px,
                hovered: None,
                focused: false,
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

    /// A tree of `n` rows, laid out as a scroll viewer would lay it out: as
    /// tall as its content.
    fn tree_of(n: usize) -> (Widget, TreeView) {
        let mut widget = Widget::default();
        widget.actual_local_position = Vec2::new(0.0, 0.0);
        widget.actual_local_size = Vec2::new(300.0, n as f32 * theme::TREE_ROW_HEIGHT);
        let view = TreeView {
            items: (0..n)
                .map(|i| TreeItem {
                    id: i as u32,
                    label: format!("Entity {i}"),
                    depth: 0,
                    icon: crate::metaphor::icon_for_entity_name("Cube"),
                    has_children: false,
                    expanded: false,
                    hidden: false,
                    locked: false,
                    script_error: false,
                })
                .collect(),
            selected: None,
            selection: crate::virtual_list::KeySelection::default(),
            font_id: 0,
            px: 12.0,
            hovered: None,
            focused: false,
            badge_drag: None,
        };
        (widget, view)
    }

    /// Primitives emitted for `n` rows through a 660 px viewport.
    fn primitives_for(n: usize) -> usize {
        let (widget, view) = tree_of(n);
        let mut ctx = DrawingContext::new(300.0, 660.0);
        ctx.push_clip_rect(Rect::new(0.0, 0.0, 300.0, 660.0));
        view.draw(&widget, &mut ctx);
        ctx.instances.len()
    }

    /// MORROWIND-M. The acceptance criterion is 100,000 rows at 60 fps, and the
    /// only way there is for the per-frame work to stop depending on the total.
    ///
    /// This is that, measured through the real draw path rather than asserted
    /// about the windowing arithmetic: the same viewport emits the same number
    /// of primitives whether the tree holds a hundred rows or a hundred
    /// thousand. Before virtualisation the second number was a thousand times
    /// the first, and every one of those primitives carried a shaped label.
    #[test]
    fn drawing_a_hundred_thousand_rows_costs_what_thirty_rows_cost() {
        let small = primitives_for(100);
        let huge = primitives_for(100_000);
        assert!(small > 0, "the fixture drew nothing");
        assert_eq!(
            small, huge,
            "100 rows emitted {small} primitives and 100,000 emitted {huge}"
        );
    }

    #[test]
    fn scrolling_deep_into_a_long_tree_costs_the_same_as_not_scrolling() {
        // A thousand rows down, the work is the same size and it is *different*
        // work — the labels drawn are the ones under the clip.
        let (mut widget, view) = tree_of(100_000);
        widget.actual_local_position = Vec2::new(0.0, -1000.0 * theme::TREE_ROW_HEIGHT);
        let mut ctx = DrawingContext::new(300.0, 660.0);
        ctx.push_clip_rect(Rect::new(0.0, 0.0, 300.0, 660.0));
        view.draw(&widget, &mut ctx);
        let scrolled = ctx.instances.len();

        // Not exactly equal, and the difference is the point rather than slop:
        // at scroll zero there is no row above the clip, so the top overscan
        // row does not exist. One row of difference is the whole budget.
        let unscrolled = primitives_for(100_000);
        let one_row = primitives_for(1);
        assert!(
            scrolled >= unscrolled && scrolled <= unscrolled + one_row,
            "scrolled {scrolled}, unscrolled {unscrolled}, one row {one_row}"
        );
    }

    /// The affordance has to exist *before* you use it.
    ///
    /// Both badges are drawn on every row, so a visible unlocked row costs the
    /// same primitives as a hidden locked one. Before this the gutter was empty
    /// until a flag was set: the click target worked and nothing said so, which
    /// is the bug this asserts against.
    #[test]
    fn every_row_draws_its_badges_whatever_state_it_is_in() {
        let plain = primitives_for(1);

        let (widget, mut view) = tree_of(1);
        view.items[0].hidden = true;
        view.items[0].locked = true;
        let mut ctx = DrawingContext::new(300.0, 660.0);
        ctx.push_clip_rect(Rect::new(0.0, 0.0, 300.0, 660.0));
        view.draw(&widget, &mut ctx);

        assert_eq!(
            plain,
            ctx.instances.len(),
            "a visible unlocked row must draw the same badges as a hidden locked one"
        );
    }

    #[test]
    fn empty_tree_has_no_selection() {
        let t = TreeView {
            items: Vec::new(),
            selected: None,
            selection: crate::virtual_list::KeySelection::default(),
            font_id: 0,
            px: 12.0,
            hovered: None,
            focused: false,
            badge_drag: None,
        };
        assert!(t.selected.is_none());
    }
}
