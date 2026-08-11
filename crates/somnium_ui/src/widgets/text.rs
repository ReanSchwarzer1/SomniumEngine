// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/text.rs (simplified)
// Text widget: single-line text rendered via FontAtlas (Phase 12A-4).
// Multi-line / word-wrap deferred to a later pass.

use crate::{
    draw::DrawingContext,
    message::{TextMessage, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct Text {
    pub text: String,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
}

impl Control for Text {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        ctx.measure_text(&self.text, self.px, self.font_id)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let origin = widget.actual_local_position;
        ctx.push_text(&self.text, origin, self.font_id, self.px, self.color);
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(TextMessage::SetText(s)) = msg.data::<TextMessage>() {
            self.text = s.clone();
            widget.invalidate_layout();
            msg.handled = true;
        }
    }
}

pub struct TextBuilder {
    widget: WidgetBuilder,
    text: String,
    px: f32,
    color: [u8; 4],
    font_id: u8,
}

impl TextBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            text: String::new(),
            px: 14.0,
            color: [255, 255, 255, 255],
            font_id: 0,
        }
    }

    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }

    pub fn with_font_size(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn with_color(mut self, color: [u8; 4]) -> Self {
        self.color = color;
        self
    }

    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Text {
                text: self.text,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
            }),
        )
    }
}
