//! Transient status toasts (Phase 26-I). Hit-test is off — they never steal clicks.

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum ToastMessage {
    Push(String),
    /// CONTROL-I: an error toast, which does **not** expire.
    ///
    /// A four-second failure notice is a failure notice you miss while looking
    /// at the thing that failed. Errors stay until dismissed; everything else
    /// still fades.
    PushError(String),
    /// Dismiss the oldest sticky toast — what a click on one does.
    DismissOldest,
}

/// One toast. `sticky` is the whole of the CONTROL-I change: a sticky toast is
/// exempt from the four-second prune and paints at full opacity forever.
struct Toast {
    text: String,
    raised: Instant,
    sticky: bool,
}

pub struct ToastHost {
    items: Vec<Toast>,
    font_id: u8,
}

impl ToastHost {
    fn prune(&mut self) {
        let now = Instant::now();
        self.items.retain(|toast| {
            toast.sticky || now.duration_since(toast.raised) < Duration::from_secs(4)
        });
    }

    /// Whether anything is still on screen. Used by the tests, and by the
    /// shell to decide whether the host needs painting at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many sticky toasts are waiting to be dismissed.
    #[must_use]
    pub fn sticky_count(&self) -> usize {
        self.items.iter().filter(|toast| toast.sticky).count()
    }
}

impl Control for ToastHost {
    // MORROWIND-I. A toast is the reason `Politeness` exists: it appears
    // without taking focus, so a reader that only speaks on focus change never
    // mentions it, and the user never learns that the save failed.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Alert
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let now = Instant::now();
        let visible: Vec<&Toast> = self
            .items
            .iter()
            .filter(|toast| {
                toast.sticky || now.duration_since(toast.raised) < Duration::from_secs(4)
            })
            .collect();
        for (i, toast) in visible.iter().rev().enumerate() {
            let text = &toast.text;
            let age = now.duration_since(toast.raised).as_secs_f32();
            let alpha = if toast.sticky {
                255
            } else if age > 3.0 {
                ((4.0 - age) * 255.0).clamp(0.0, 255.0) as u8
            } else {
                230
            };
            let w = (text.len() as f32 * 7.0 + 24.0).clamp(120.0, 360.0);
            let h = 28.0;
            let x = b.x + b.w - w - 16.0;
            let y = b.y + b.h - 48.0 - i as f32 * 34.0;
            // Phase 27-D: a toast is the top rung of the elevation ladder, so
            // it reads as above the modal rather than pasted onto the status bar.
            let t = theme::active();
            let rect = Rect::new(x, y, w, h);
            let radii = [t.geometry.radius_popup; 4];
            let mut lifted = t.elevation.toast;
            // Fade the shadow out with the toast itself.
            lifted.alpha *= alpha as f32 / 255.0;
            ctx.push_drop_shadow_rounded(rect, radii, lifted);
            ctx.push_primitive(
                crate::primitive::Primitive::fill(
                    rect,
                    theme::with_alpha(t.semantic.surface.popup.bytes(), alpha),
                )
                .with_radii(radii)
                .with_border(
                    t.geometry.stroke_hairline,
                    t.semantic.border.default.bytes(),
                ),
                None,
            );
            ctx.push_text(
                text,
                Vec2::new(x + 10.0, y + 7.0),
                self.font_id,
                12.0,
                if toast.sticky {
                    t.semantic.status.error.bytes()
                } else {
                    theme::TEXT_PRIMARY
                },
            );
        }
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        match msg.data::<ToastMessage>() {
            Some(ToastMessage::Push(text)) => {
                self.items.push(Toast {
                    text: text.clone(),
                    raised: Instant::now(),
                    sticky: false,
                });
                msg.handled = true;
            }
            Some(ToastMessage::PushError(text)) => {
                self.items.push(Toast {
                    text: text.clone(),
                    raised: Instant::now(),
                    sticky: true,
                });
                msg.handled = true;
            }
            Some(ToastMessage::DismissOldest) => {
                if let Some(index) = self.items.iter().position(|toast| toast.sticky) {
                    self.items.remove(index);
                }
                msg.handled = true;
            }
            None => {}
        }
        self.prune();
    }
}

pub struct ToastHostBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl ToastHostBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget
                .with_hit_test_visibility(false)
                .with_background(theme::TRANSPARENT)
                .build(),
            Box::new(ToastHost {
                items: Vec::new(),
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> ToastHost {
        ToastHost {
            items: Vec::new(),
            font_id: 0,
        }
    }

    fn push(host: &mut ToastHost, message: ToastMessage) {
        let mut widget = WidgetBuilder::new().build();
        let mut msg = UiMessage::new(
            widget.handle,
            crate::message::MessageDirection::ToWidget,
            message,
        );
        host.handle_routed_message(&mut widget, &mut msg, &mut Vec::new());
    }

    /// The CONTROL-I change: a four-second failure notice is one you miss
    /// while looking at the thing that failed.
    #[test]
    fn an_error_toast_outlives_the_prune_and_an_ordinary_one_does_not() {
        let mut host = host();
        push(&mut host, ToastMessage::Push("Scene saved".into()));
        push(&mut host, ToastMessage::PushError("Import failed".into()));
        assert_eq!(host.items.len(), 2);
        assert_eq!(host.sticky_count(), 1);

        // Age both past the four-second window.
        let old = Instant::now() - Duration::from_secs(10);
        for toast in &mut host.items {
            toast.raised = old;
        }
        host.prune();
        assert_eq!(host.items.len(), 1, "only the error survives");
        assert!(host.items[0].sticky);
    }

    #[test]
    fn dismiss_removes_the_oldest_error_and_leaves_the_rest() {
        let mut host = host();
        push(&mut host, ToastMessage::PushError("first".into()));
        push(&mut host, ToastMessage::PushError("second".into()));
        push(&mut host, ToastMessage::DismissOldest);
        assert_eq!(host.sticky_count(), 1);
        assert_eq!(host.items[0].text, "second");

        push(&mut host, ToastMessage::DismissOldest);
        assert!(host.is_empty());
        // Dismissing an empty host is a no-op rather than a panic.
        push(&mut host, ToastMessage::DismissOldest);
        assert!(host.is_empty());
    }
}
