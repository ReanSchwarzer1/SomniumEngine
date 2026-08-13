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
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        if !self.is_open {
            return Vec2::ZERO;
        }
        // Menus and cards must not inherit the window size. Cap the constraint
        // so StackPanel/Border children size to their labels, not the screen.
        let content_avail = match self.placement {
            PopupPlacement::AnchorBelow => Vec2::new(available.x.min(280.0), 10_000.0),
            PopupPlacement::Center => Vec2::new(available.x.min(780.0), available.y.min(560.0)),
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
            let w = ds.x.max(1.0);
            let h = ds.y.max(1.0);
            let (x, y) = match self.placement {
                PopupPlacement::AnchorBelow if self.anchor.is_some() => {
                    let b = ctx.screen_bounds(self.anchor);
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
