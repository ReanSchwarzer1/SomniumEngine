// Text widget: labels, plus optional wrapped long-form (Help).

use crate::{
    draw::DrawingContext,
    message::{TextMessage, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct Text {
    pub text: String,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
    pub wrap: bool,
}

pub fn wrap_lines(text: &str, max_w: f32, mut width_of: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        if !max_w.is_finite() || max_w <= 8.0 {
            lines.push(para.to_string());
            continue;
        }
        let mut current = String::new();
        for word in para.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if width_of(&candidate) <= max_w || current.is_empty() {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl Control for Text {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let line_h = ctx.measure_text("Ag", self.px, self.font_id).y.max(self.px);
        let max_w = if self.wrap && available.x.is_finite() {
            available.x
        } else {
            f32::INFINITY
        };
        let lines = wrap_lines(&self.text, max_w, |s| {
            ctx.measure_text(s, self.px, self.font_id).x
        });
        let w = lines
            .iter()
            .map(|s| ctx.measure_text(s, self.px, self.font_id).x)
            .fold(0.0f32, f32::max);
        Vec2::new(w.min(available.x.max(w)), line_h * lines.len() as f32)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let origin = widget.actual_local_position;
        let line_h = ctx
            .font_atlas
            .measure_text("Ag", self.px, self.font_id)
            .y
            .max(self.px);
        let max_w = if self.wrap {
            widget.actual_local_size.x
        } else {
            f32::INFINITY
        };
        let lines = wrap_lines(&self.text, max_w, |s| {
            ctx.font_atlas.measure_text(s, self.px, self.font_id).x
        });
        for (i, line) in lines.iter().enumerate() {
            ctx.push_text(
                line,
                Vec2::new(origin.x, origin.y + i as f32 * line_h),
                self.font_id,
                self.px,
                self.color,
            );
        }
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
    wrap: bool,
}

impl TextBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            text: String::new(),
            px: theme::NOCTURNE.typography.body,
            color: theme::TEXT_PRIMARY,
            font_id: 0,
            wrap: false,
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

    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
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
                wrap: self.wrap,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_on_newlines_and_width() {
        let lines = wrap_lines("one two three", 20.0, |s| s.len() as f32 * 4.0);
        assert!(lines.len() >= 2);
        let nl = wrap_lines("a\nb", 1000.0, |s| s.len() as f32);
        assert_eq!(nl, vec!["a".to_string(), "b".to_string()]);
    }
}
