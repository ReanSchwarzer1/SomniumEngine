//! The interact prompt: the key and what it does ("E  Open"), floating over the thing the player would use
//! (`somnium_core::interact`). Restrained like the objective marks: a dark pill, a key cap, one line of text; a
//! refused one ("Locked") in a dimmer, warmer colour. Never takes the pointer.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
    widgets::objective_markers::{Placement, place},
};
use glam::{Mat4, Vec2, Vec3};

/// How far above its anchor the prompt floats, metres.
pub const LIFT: f32 = 0.55;

/// Messages to the prompt.
#[derive(Clone, Debug)]
pub enum InteractPromptMessage {
    /// Show it over `at` (world metres) through `view_proj` (world to clip).
    Show { view_proj: Mat4, at: Vec3, key: String, text: String, refused: bool },
    Hide,
}

/// The widget.
pub struct InteractPrompt {
    font_id: u8,
    shown: Option<(Mat4, Vec3, String, String, bool)>,
}

impl Control for InteractPrompt {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let Some((view_proj, at, key, text, refused)) = &self.shown else { return };
        let bounds = widget.screen_bounds();
        let Placement::Over(point) = place(*view_proj, *at + Vec3::Y * LIFT, bounds, 0.0) else { return };
        let unit = (bounds.h / 1080.0).max(0.25);
        let ink = if *refused { [214, 150, 122, 230] } else { [236, 226, 204, 240] };
        let (text_px, key_px, cap, pad) = (19.0 * unit, 16.0 * unit, 26.0 * unit, 9.0 * unit);
        let key_size = ctx.font_atlas.measure_text(key, key_px, self.font_id);
        let text_size = ctx.font_atlas.measure_text(text, text_px, self.font_id);
        let cap_w = cap.max(key_size.x + 10.0 * unit);
        let width = pad + cap_w + pad + text_size.x + pad * 1.4;
        let height = cap + 2.0 * pad * 0.6;
        let pill = Rect::new(point.x - width * 0.5, point.y - height * 0.5, width, height);
        ctx.push_round_rect(pill, height * 0.5, [12, 12, 16, 168]);
        let cap_rect = Rect::new(pill.x + pad, pill.y + (height - cap) * 0.5, cap_w, cap);
        ctx.push_round_rect(cap_rect, 4.0 * unit, theme::with_alpha(ink, 36));
        ctx.push_primitive(
            crate::primitive::Primitive::fill(cap_rect, [0, 0, 0, 0]).with_radius(4.0 * unit).with_border(1.5 * unit, ink),
            None,
        );
        let key_at = Vec2::new(cap_rect.x + (cap_w - key_size.x) * 0.5, cap_rect.y + (cap - key_size.y) * 0.5);
        ctx.push_text(key, key_at, self.font_id, key_px, ink);
        let text_at = Vec2::new(cap_rect.x + cap_w + pad, pill.y + (height - text_size.y) * 0.5);
        ctx.push_text(text, text_at, self.font_id, text_px, ink);
    }

    fn handle_routed_message(&mut self, _widget: &mut Widget, msg: &mut UiMessage, _emit: &mut Vec<UiMessage>) {
        if msg.direction != MessageDirection::ToWidget {
            return;
        }
        let Some(message) = msg.data::<InteractPromptMessage>().cloned() else { return };
        self.shown = match message {
            InteractPromptMessage::Show { view_proj, at, key, text, refused } => Some((view_proj, at, key, text, refused)),
            InteractPromptMessage::Hide => None,
        };
        msg.handled = true;
    }
}

/// Builds [`InteractPrompt`], normally stretched over the game's view.
pub struct InteractPromptBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl InteractPromptBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }

    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let mut widget = self.widget.with_background(theme::TRANSPARENT).build();
        widget.hit_self = false;
        UiNode::new(widget, Box::new(InteractPrompt { font_id: self.font_id, shown: None }))
    }
}
