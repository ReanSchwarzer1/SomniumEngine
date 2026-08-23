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
}

pub struct ToastHost {
    items: Vec<(String, Instant)>,
    font_id: u8,
}

impl ToastHost {
    fn prune(&mut self) {
        let now = Instant::now();
        self.items
            .retain(|(_, t)| now.duration_since(*t) < Duration::from_secs(4));
    }
}

impl Control for ToastHost {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let now = Instant::now();
        let visible: Vec<&(String, Instant)> = self
            .items
            .iter()
            .filter(|(_, t)| now.duration_since(*t) < Duration::from_secs(4))
            .collect();
        for (i, (text, started)) in visible.iter().rev().enumerate() {
            let age = now.duration_since(*started).as_secs_f32();
            let alpha = if age > 3.0 {
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
                theme::TEXT_PRIMARY,
            );
        }
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(ToastMessage::Push(text)) = msg.data::<ToastMessage>() {
            self.items.push((text.clone(), Instant::now()));
            msg.handled = true;
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
