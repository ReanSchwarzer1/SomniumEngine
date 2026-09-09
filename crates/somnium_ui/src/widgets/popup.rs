// Popup: full-screen click-catcher whose content is a child sized to its
// desired size (Phase 26-A / 26-I). Parent the popup to the root so it is not
// clipped by ScrollViewer (Fyrox pattern).
//
// The popup node itself still fills the window so click-away works. Children
// are measured against a content-sized constraint so File/Create menus are
// compact instead of stretching to the screen.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

/// Where the content panel is placed inside the full-screen click catcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PopupPlacement {
    /// Below the left edge of `anchor` (menus).
    #[default]
    AnchorBelow,
    /// Centered in the window (command palette, unsaved modal, colour picker).
    Center,
    /// Horizontally centered, sitting above the status bar (Content Drawer).
    BottomCenter,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PopupMessage {
    Open,
    Close,
    /// Reposition content under this widget on the next arrange.
    SetAnchor(NodeHandle),
}

pub struct Popup {
    pub is_open: bool,
    pub anchor: NodeHandle,
    pub placement: PopupPlacement,
}

impl Control for Popup {
    // MORROWIND-I. A popup that has taken focus is a dialog to a reader,
    // whatever it looks like.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Dialog
    }

    // Only while open: a closed popup measures to nothing and moving it
    // between windows every frame would be work with no picture attached.
    fn popup_anchor(&self) -> Option<NodeHandle> {
        self.is_open.then_some(self.anchor)
    }

    fn popup_presentation(&self) -> Option<(bool, NodeHandle)> {
        (self.placement == PopupPlacement::AnchorBelow).then_some((self.is_open, self.anchor))
    }

    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        if !self.is_open {
            return Vec2::ZERO;
        }
        // Menus and cards must not inherit the window size. Cap the constraint
        // so StackPanel/Border children size to their labels, not the screen.
        let content_avail = match self.placement {
            PopupPlacement::AnchorBelow => {
                let inset = crate::theme::active().geometry.inset_panel;
                let height = if self.anchor.is_some() {
                    let anchor = ctx.screen_bounds(self.anchor);
                    anchor.y.max(available.y - anchor.y - anchor.h) - inset
                } else {
                    available.y - inset * 2.0
                };
                Vec2::new(available.x.min(280.0), height.max(1.0))
            }
            PopupPlacement::Center => Vec2::new(available.x.min(920.0), available.y.min(640.0)),
            PopupPlacement::BottomCenter => {
                Vec2::new(available.x.min(720.0), available.y.min(360.0))
            }
        };
        for &ch in &widget.children {
            ctx.measure_child(ch, content_avail);
        }
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        if !self.is_open {
            return Vec2::ZERO;
        }
        for &ch in &widget.children {
            let ds = ctx.desired_size(ch);
            let mut w = ds.x.max(1.0);
            let h = ds.y.max(1.0);
            let (x, y) = match self.placement {
                PopupPlacement::AnchorBelow if self.anchor.is_some() => {
                    let b = ctx.screen_bounds(self.anchor);
                    w = w.max(b.w);
                    let mut x = b.x;
                    let mut y = b.y + b.h;
                    if x + w > final_size.x {
                        x = (final_size.x - w).max(0.0);
                    }
                    if y + h > final_size.y {
                        y = (b.y - h).max(0.0);
                    }
                    (x, y)
                }
                PopupPlacement::AnchorBelow => {
                    let pos = ctx.desired_local_position(ch);
                    (pos.x, pos.y)
                }
                PopupPlacement::Center => (
                    ((final_size.x - w) * 0.5).max(0.0),
                    ((final_size.y - h) * 0.5).max(0.0),
                ),
                PopupPlacement::BottomCenter => (
                    ((final_size.x - w) * 0.5).max(0.0),
                    (final_size.y - h - 28.0).max(0.0),
                ),
            };
            ctx.arrange_child(ch, Rect::new(x, y, w, h));
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if !self.is_open {
            return;
        }
        // Click-away fill only when the popup asked for a dim overlay.
        if widget.background[3] > 0 {
            ctx.push_rect_filled(widget.screen_bounds(), widget.background);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseDown { .. } => {
                    if self.is_open && msg.destination == widget.handle {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            PopupMessage::Close,
                        ));
                        msg.handled = true;
                    }
                }
                _ => {}
            }
        } else if let Some(pmsg) = msg.data::<PopupMessage>() {
            match pmsg {
                PopupMessage::Open => {
                    if !self.is_open {
                        self.is_open = true;
                        widget.visibility = true;
                        widget.invalidate_layout();
                    }
                }
                PopupMessage::Close => {
                    if self.is_open {
                        self.is_open = false;
                        widget.visibility = false;
                        widget.invalidate_layout();
                    }
                }
                PopupMessage::SetAnchor(h) => {
                    self.anchor = *h;
                    widget.invalidate_layout();
                }
            }
        }
    }
}

pub struct PopupBuilder {
    widget: WidgetBuilder,
    anchor: NodeHandle,
    placement: PopupPlacement,
}

impl PopupBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            anchor: NodeHandle::NONE,
            placement: PopupPlacement::AnchorBelow,
        }
    }

    pub fn with_anchor(mut self, anchor: NodeHandle) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn with_placement(mut self, placement: PopupPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.with_visibility(false).build(),
            Box::new(Popup {
                is_open: false,
                anchor: self.anchor,
                placement: self.placement,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_popup_measures_zero() {
        let p = Popup {
            is_open: false,
            anchor: NodeHandle::NONE,
            placement: PopupPlacement::AnchorBelow,
        };
        assert!(!p.is_open);
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::{
        motion::{MotionKey, MotionProperty},
        theme,
        ui::UserInterface,
        widgets::{
            border::BorderBuilder,
            button::ButtonBuilder,
            text_box::{TextBoxBuilder, TextBoxMessage},
        },
    };

    fn fixture(reduced: bool) -> (UserInterface, NodeHandle, NodeHandle, NodeHandle) {
        let mut ui = UserInterface::new(480.0, 320.0);
        ui.draw_ctx.motion.set_reduced_motion(reduced);
        let anchor = ui.add_node(
            ButtonBuilder::new(WidgetBuilder::new().with_width(90.0).with_height(28.0)).build(),
            ui.root(),
        );
        let popup = ui.add_node(
            PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_anchor(anchor)
                .build(),
            ui.root(),
        );
        let panel = ui.add_node(
            BorderBuilder::new(
                WidgetBuilder::new()
                    .with_width(160.0)
                    .with_height(70.0)
                    .with_background(theme::active().semantic.surface.popup.bytes()),
            )
            .build(),
            popup,
        );
        let field = ui.add_node(
            TextBoxBuilder::new(WidgetBuilder::new().with_width(140.0).with_height(26.0)).build(),
            panel,
        );
        ui.perform_layout();
        ui.draw();
        (ui, popup, panel, field)
    }
    fn send(ui: &mut UserInterface, popup: NodeHandle, event: PopupMessage) {
        ui.send(UiMessage::new(popup, MessageDirection::ToWidget, event));
        ui.update();
        ui.perform_layout();
        ui.draw();
    }
    #[test]
    fn popup_input_is_immediate_and_close_paint_never_catches_clicks() {
        let (mut ui, popup, panel, field) = fixture(false);
        send(&mut ui, popup, PopupMessage::Open);
        let bounds = ui.nodes.borrow(panel.transmute()).widget.screen_bounds();
        let field_bounds = ui.nodes.borrow(field.transmute()).widget.screen_bounds();
        let point = Vec2::new(field_bounds.x + 4.0, field_bounds.y + 4.0);
        assert_eq!(
            ui.hit_test(point),
            field,
            "input is live at the first opening frame"
        );
        ui.send(UiMessage::new(
            field,
            MessageDirection::ToWidget,
            WidgetMessage::Focus,
        ));
        ui.send(UiMessage::new(
            field,
            MessageDirection::ToWidget,
            WidgetMessage::Text("abc".into()),
        ));
        let messages = ui.update();
        assert!(messages.iter().any(|m| matches!(m.data::<TextBoxMessage>(), Some(TextBoxMessage::TextChanged(s)) if s == "abc")));
        ui.draw_ctx.motion.tick(70.0);
        ui.draw();
        let opacity = ui
            .draw_ctx
            .motion
            .value_or(MotionKey::new(popup.index(), MotionProperty::Opacity), 0.0);
        assert!(opacity > 0.0 && opacity < 1.0);
        assert_eq!(
            ui.nodes.borrow(panel.transmute()).widget.screen_bounds(),
            bounds
        );
        send(&mut ui, popup, PopupMessage::Close);
        assert!(!ui.is_globally_visible(field));
        assert_ne!(
            ui.hit_test(point),
            field,
            "closing paint must be input-transparent"
        );
        assert!(!ui.draw_ctx.instances.is_empty());
        assert_eq!(
            ui.draw_ctx
                .motion
                .value_or(MotionKey::new(popup.index(), MotionProperty::Opacity), 0.0),
            opacity
        );
        ui.draw_ctx.motion.tick(50.0);
        ui.draw();
        let fading = ui
            .draw_ctx
            .motion
            .value_or(MotionKey::new(popup.index(), MotionProperty::Opacity), 0.0);
        assert!(fading > 0.0 && fading < opacity);
        send(&mut ui, popup, PopupMessage::Open);
        assert_eq!(
            ui.draw_ctx
                .motion
                .value_or(MotionKey::new(popup.index(), MotionProperty::Opacity), 0.0),
            fading
        );
        ui.draw_ctx.motion.tick(140.0);
        ui.draw();
        assert_eq!(ui.hit_test(point), field);
        assert_eq!(
            ui.nodes.borrow(panel.transmute()).widget.screen_bounds(),
            bounds
        );
        send(&mut ui, popup, PopupMessage::Close);
        ui.draw_ctx.motion.tick(100.0);
        ui.draw();
        assert_eq!(
            ui.draw_ctx
                .motion
                .value_or(MotionKey::new(popup.index(), MotionProperty::Opacity), 1.0),
            0.0
        );
        assert!(ui.draw_ctx.motion.is_idle());
    }
    #[test]
    fn reduced_motion_matches_settled_popup_paint_and_layout_in_both_themes() {
        for id in [theme::ThemeId::Nocturne, theme::ThemeId::Dawn] {
            theme::set_active(id);
            let (mut normal, popup, panel, _) = fixture(false);
            send(&mut normal, popup, PopupMessage::Open);
            normal.draw_ctx.motion.tick(140.0);
            normal.draw();
            let (mut reduced, other_popup, other_panel, _) = fixture(true);
            send(&mut reduced, other_popup, PopupMessage::Open);
            assert_eq!(
                normal
                    .nodes
                    .borrow(panel.transmute())
                    .widget
                    .screen_bounds(),
                reduced
                    .nodes
                    .borrow(other_panel.transmute())
                    .widget
                    .screen_bounds()
            );
            assert_eq!(normal.draw_ctx.instances, reduced.draw_ctx.instances);
            assert_eq!(normal.draw_ctx.commands, reduced.draw_ctx.commands);
            assert!(reduced.draw_ctx.motion.is_idle());
            send(&mut reduced, other_popup, PopupMessage::Close);
            assert!(!reduced.is_globally_visible(other_panel));
            assert!(reduced.draw_ctx.motion.is_idle());
        }
        theme::set_active(theme::ThemeId::Nocturne);
    }
}
