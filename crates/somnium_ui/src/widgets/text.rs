// Text widget: labels, plus optional wrapped long-form (Help).

use crate::{
    draw::DrawingContext,
    message::{TextMessage, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    theme, typography,
    typography::TextRole,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

pub struct Text {
    pub text: String,
    pub px: f32,
    pub color: [u8; 4],
    pub font_id: u8,
    pub wrap: bool,
    /// Extra advance per glyph. Non-zero only for the uppercase header role.
    pub tracking: f32,
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
    // MORROWIND-I. Static text is a label and *is* its own name. This is also
    // where most of a UI's accessible names come from: a button whose label is
    // a child text node has no name of its own, and the collapse rule in
    // `A11yTree::from_ui` is what lifts this one onto it.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Label
    }

    fn a11y_name(&self) -> Option<String> {
        (!self.text.trim().is_empty()).then(|| self.text.clone())
    }

    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let line_h = ctx.measure_text("Ag", self.px, self.font_id).y.max(self.px);
        let max_w = if self.wrap && available.x.is_finite() {
            available.x
        } else {
            f32::INFINITY
        };
        let tracking = self.tracking;
        let font_id = self.font_id;
        let px = self.px;
        let lines = wrap_lines(&self.text, max_w, |s| {
            ctx.measure_text_tracked(s, px, font_id, tracking).x
        });
        let w = lines
            .iter()
            .map(|s| ctx.measure_text_tracked(s, px, font_id, tracking).x)
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
            ctx.font_atlas
                .measure_text_tracked(s, self.px, self.font_id, self.tracking)
                .x
        });
        for (i, line) in lines.iter().enumerate() {
            ctx.push_text_tracked(
                line,
                Vec2::new(origin.x, origin.y + i as f32 * line_h),
                self.font_id,
                self.px,
                self.color,
                self.tracking,
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
    tracking: f32,
    role: Option<TextRole>,
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
            tracking: 0.0,
            role: None,
        }
    }

    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }

    /// Take size, face, colour, tracking and case from a semantic role.
    ///
    /// This is the preferred entry point: a call site that names `Section` or
    /// `MonoStrong` cannot drift from the token sheet the way a literal
    /// `with_font_size(11.0)` does. Call it **before** `with_color` /
    /// `with_font_size` if you need to override one of its parts, and note that
    /// it supersedes any `with_font_id` — the role owns the face.
    pub fn with_role(mut self, role: TextRole) -> Self {
        let style = typography::text_style(role);
        self.px = style.px;
        self.color = style.color;
        self.font_id = style.font_id();
        self.tracking = style.tracking;
        self.role = Some(role);
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

    pub fn with_tracking(mut self, tracking: f32) -> Self {
        self.tracking = tracking;
        self
    }

    pub fn build(self) -> UiNode {
        // The case transform belongs to the role, not to the caller: an
        // `update_*` path that later re-sends the label as SetText would
        // otherwise lose the transform and the header would silently change
        // case mid-session.
        let text = match self.role {
            Some(role) => typography::text_style(role).transform(&self.text),
            None => self.text,
        };
        UiNode::new(
            self.widget.build(),
            Box::new(Text {
                text,
                px: self.px,
                color: self.color,
                font_id: self.font_id,
                wrap: self.wrap,
                tracking: self.tracking,
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
