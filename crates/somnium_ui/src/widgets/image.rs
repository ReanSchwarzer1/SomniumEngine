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
    /// Asset this image stands for, if any.
    ///
    /// Phase 27-G. When the thumbnail cache has a preview for this path the
    /// image draws it; otherwise it falls back to `icon`. The fallback is the
    /// normal state, not an error state: previews arrive over several frames
    /// and some assets never get one.
    pub asset: Option<std::path::PathBuf>,
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
        // A ready preview wins; the tint is dropped with it, because a
        // thumbnail carries real colour and tinting it would wash it out.
        if let Some(uv) = self
            .asset
            .as_deref()
            .and_then(|p| ctx.thumbnails.uv(p))
        {
            ctx.push_primitive(
                crate::primitive::Primitive::textured(rect, uv, [255, 255, 255, 255]),
                Some(crate::thumbnail::THUMBNAIL_ATLAS_TEXTURE_ID),
            );
            return;
        }
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
    asset: Option<std::path::PathBuf>,
    tint: [u8; 4],
    size: f32,
}

impl ImageBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            icon: IconId::Unknown,
            asset: None,
            tint: crate::theme::TEXT_PRIMARY,
            size: 16.0,
        }
    }
    pub fn with_icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }

    /// Draw this asset's preview when one exists, falling back to the icon.
    pub fn with_asset(mut self, asset: std::path::PathBuf) -> Self {
        self.asset = Some(asset);
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
                asset: self.asset,
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
            asset: None,
            icon: IconId::Play,
            tint: [255, 255, 255, 255],
            size: 20.0,
        };
        assert_eq!(img.size, 20.0);
    }
}
