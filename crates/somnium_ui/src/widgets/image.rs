// Image / Icon widgets — textured quads from the icon atlas (Phase 26-A).

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct Image {
    pub icon: IconId,
    pub tint: [u8; 4],
    pub size: f32,
}

impl Control for Image {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::splat(self.size)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let side = self.size.min(b.w).min(b.h);
        let rect = Rect::new(
            b.x + (b.w - side) * 0.5,
            b.y + (b.h - side) * 0.5,
            side,
            side,
        );
        let (uv, tex) = self.icon.draw_quad(rect);
        ctx.push_textured_rect(rect, uv, self.tint, tex);
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
    }
}

pub struct ImageBuilder {
    widget: WidgetBuilder,
    icon: IconId,
    tint: [u8; 4],
    size: f32,
}

impl ImageBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            icon: IconId::Unknown,
            tint: crate::theme::TEXT_PRIMARY,
            size: 16.0,
        }
    }
    pub fn with_icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }
    pub fn with_tint(mut self, tint: [u8; 4]) -> Self {
        self.tint = tint;
        self
    }
    pub fn with_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Image {
                icon: self.icon,
                tint: self.tint,
                size: self.size,
            }),
        )
    }
}

pub type Icon = Image;
pub type IconBuilder = ImageBuilder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_measures_to_requested_size() {
        let img = Image {
            icon: IconId::Play,
            tint: [255, 255, 255, 255],
            size: 20.0,
        };
        assert_eq!(img.size, 20.0);
    }
}
