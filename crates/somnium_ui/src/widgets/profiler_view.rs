//! The profiler overlay, drawn in the editor's current recipe (Phase ALYX, 18 Oct 2026).
//!
//! One widget paints the whole card, the same way the command palette does, because the rows change every frame:
//! a header (glyph, title, the frame's GPU time in large figures), a budget bar against 60 fps, then each section
//! under its glyph, each timed row with a share-of-frame bar behind its value. Rows that cost nothing are folded into
//! one line, and the list scrolls with the wheel rather than running off the viewport.

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::Rect,
    typography::{FontRole, font_id},
    widget::{Widget, WidgetBuilder},
    ProfilerRow,
};
use glam::Vec2;

/// Replace what the overlay shows: its rows and the editor's frame rate.
#[derive(Clone, Debug)]
pub enum ProfilerViewMessage {
    SetRows { rows: Vec<ProfilerRow>, fps: f32 },
}

const WIDTH: f32 = 320.0;
const HEADER: f32 = 58.0;
const ROW: f32 = 18.0;
const SECTION: f32 = 26.0;
/// The frame budget the bar measures against: 60 frames a second.
const BUDGET_MS: f32 = 1000.0 / 60.0;
/// A timed row under this many milliseconds is folded into the section's idle line.
const IDLE_MS: f32 = 0.0005;

pub struct ProfilerView {
    rows: Vec<ProfilerRow>,
    fps: f32,
    scroll: f32,
    font_id: u8,
}

/// What a row turns into on screen once idle rows are folded.
#[derive(Debug, PartialEq)]
enum Line<'a> {
    Section(&'a ProfilerRow),
    Row(&'a ProfilerRow),
    Idle(usize),
}

impl ProfilerView {
    /// The frame's GPU time: the first timed row called "Frame".
    fn frame_ms(&self) -> Option<f32> {
        self.rows.iter().find(|row| row.label == "Frame").and_then(|row| row.ms)
    }

    fn lines(&self) -> Vec<Line<'_>> {
        let mut lines = Vec::with_capacity(self.rows.len());
        let mut idle = 0;
        for row in &self.rows {
            if row.section.is_some() {
                if idle > 0 {
                    lines.push(Line::Idle(idle));
                    idle = 0;
                }
                lines.push(Line::Section(row));
            } else if row.ms.is_some_and(|ms| ms < IDLE_MS) {
                idle += 1;
            } else {
                lines.push(Line::Row(row));
            }
        }
        if idle > 0 {
            lines.push(Line::Idle(idle));
        }
        lines
    }

    fn content_height(&self) -> f32 {
        HEADER
            + self
                .lines()
                .iter()
                .map(|line| if matches!(line, Line::Section(_)) { SECTION } else { ROW })
                .sum::<f32>()
            + 10.0
    }
}

impl Control for ProfilerView {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(WIDTH, self.content_height().min(available.y.max(HEADER + ROW)))
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        CursorKind::Default
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        let text = t.semantic.text;
        let status = t.semantic.status;
        ctx.push_drop_shadow_rounded(b, [t.geometry.radius_popup; 4], t.elevation.popup);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(b, t.semantic.surface.popup.bytes())
                .with_radius(t.geometry.radius_popup)
                .with_border(t.geometry.stroke_hairline, t.semantic.border.subtle.bytes()),
            None,
        );
        let ui_font = self.font_id;
        let strong = font_id(FontRole::UiSemiBold);
        let mono = font_id(FontRole::Mono);

        // Header: glyph and title, the frame time large and flush right, the budget bar under them.
        let glyph = Rect::new(b.x + 12.0, b.y + 12.0, 18.0, 18.0);
        let (uv, texture) = IconId::Profiler.draw_quad(glyph);
        ctx.push_textured_rect(glyph, uv, t.semantic.accent.default.bytes(), texture);
        ctx.push_text("Profiler", Vec2::new(b.x + 38.0, b.y + 12.0), strong, 14.0, text.primary.bytes());
        let frame = self.frame_ms();
        let figure = frame.map_or_else(|| "collecting".to_string(), |ms| format!("{ms:.2} ms"));
        let figure_w = ctx.font_atlas.measure_text(&figure, 16.0, strong).x;
        ctx.push_text(&figure, Vec2::new(b.x + b.w - 12.0 - figure_w, b.y + 10.0), strong, 16.0, text.emphasis.bytes());
        if self.fps > 0.0 {
            let rate = format!("{:.0} fps", self.fps);
            let rate_w = ctx.font_atlas.measure_text(&rate, 11.0, ui_font).x;
            ctx.push_text(&rate, Vec2::new(b.x + b.w - 12.0 - figure_w - 10.0 - rate_w, b.y + 14.0), ui_font, 11.0, text.muted.bytes());
        }
        let track = Rect::new(b.x + 12.0, b.y + 38.0, b.w - 24.0, 4.0);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(track, t.semantic.surface.raised.bytes()).with_radius(2.0),
            None,
        );
        if let Some(ms) = frame {
            let share = (ms / BUDGET_MS).clamp(0.0, 1.0);
            let colour = if ms <= BUDGET_MS * 0.75 {
                status.success
            } else if ms <= BUDGET_MS {
                status.warning
            } else {
                status.error
            };
            ctx.push_primitive(
                crate::primitive::Primitive::fill(Rect::new(track.x, track.y, track.w * share, track.h), colour.bytes())
                    .with_radius(2.0),
                None,
            );
        }
        ctx.push_text("of a 60 fps frame", Vec2::new(b.x + 12.0, b.y + 44.0), ui_font, 10.0, text.muted.bytes());

        // The rows, clipped below the header and scrolled.
        let body = Rect::new(b.x, b.y + HEADER, b.w, (b.h - HEADER - 4.0).max(0.0));
        ctx.push_clip_rect(body);
        let mut y = body.y - self.scroll;
        let reference = frame.unwrap_or(BUDGET_MS).max(0.001);
        for line in self.lines() {
            match line {
                Line::Section(row) => {
                    let icon = row.section.unwrap_or(IconId::Chart);
                    let at = Rect::new(b.x + 12.0, y + 8.0, 14.0, 14.0);
                    let (uv, texture) = icon.draw_quad(at);
                    ctx.push_textured_rect(at, uv, text.muted.bytes(), texture);
                    ctx.push_text(&row.label.to_uppercase(), Vec2::new(b.x + 32.0, y + 9.0), strong, 10.0, text.secondary.bytes());
                    if !row.value.is_empty() {
                        let w = ctx.font_atlas.measure_text(&row.value, 11.0, mono).x;
                        ctx.push_text(&row.value, Vec2::new(b.x + b.w - 12.0 - w, y + 8.0), mono, 11.0, text.secondary.bytes());
                    }
                    y += SECTION;
                }
                Line::Row(row) => {
                    let indent = 12.0 + 12.0 * f32::from(row.depth.saturating_sub(1));
                    if let Some(ms) = row.ms {
                        // The share of the frame, behind the value: wide is where the time goes.
                        let share = (ms / reference).clamp(0.0, 1.0);
                        // Between the labels and the figures, never under the figures.
                        let bar = Rect::new(b.x + 140.0, y + 3.0, (b.w - 140.0 - 84.0) * share, ROW - 6.0);
                        let mut fill = t.semantic.accent.default.bytes();
                        fill[3] = 70;
                        ctx.push_primitive(crate::primitive::Primitive::fill(bar, fill).with_radius(2.0), None);
                    }
                    let (label, _) = ctx.font_atlas.ellipsise(&row.label, 140.0 - indent - 4.0, 11.0, ui_font);
                    ctx.push_text(&label, Vec2::new(b.x + indent, y + 2.0), ui_font, 11.0, text.secondary.bytes());
                    let w = ctx.font_atlas.measure_text(&row.value, 11.0, mono).x;
                    ctx.push_text(&row.value, Vec2::new(b.x + b.w - 12.0 - w, y + 2.0), mono, 11.0, text.primary.bytes());
                    y += ROW;
                }
                Line::Idle(count) => {
                    let note = format!("{count} idle {}", if count == 1 { "pass" } else { "passes" });
                    ctx.push_text(&note, Vec2::new(b.x + 24.0, y + 2.0), ui_font, 10.0, text.muted.bytes());
                    y += ROW;
                }
            }
        }
        ctx.pop_clip_rect();
    }

    fn handle_routed_message(&mut self, widget: &mut Widget, msg: &mut UiMessage, _emit: &mut Vec<UiMessage>) {
        if let Some(ProfilerViewMessage::SetRows { rows, fps }) = msg.data::<ProfilerViewMessage>() {
            let grew = self.rows.len() != rows.len();
            self.rows = rows.clone();
            self.fps = *fps;
            if grew {
                widget.invalidate_layout();
            }
            return;
        }
        if let Some(WidgetMessage::MouseWheel { delta, .. }) = msg.data::<WidgetMessage>() {
            let visible = widget.actual_size().y - HEADER;
            let most = (self.content_height() - HEADER - visible).max(0.0);
            self.scroll = (self.scroll - *delta).clamp(0.0, most);
            msg.handled = true;
        }
    }
}

pub struct ProfilerViewBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl ProfilerViewBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }

    pub fn with_font_id(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ProfilerView { rows: Vec::new(), fps: 0.0, scroll: 0.0, font_id: self.font_id }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(label: &str, ms: f32, depth: u8) -> ProfilerRow {
        ProfilerRow::timing(label, ms, depth)
    }

    #[test]
    fn idle_passes_fold_into_one_line_per_section() {
        let view = ProfilerView {
            rows: vec![
                ProfilerRow::section("GPU", IconId::Chart, ""),
                timed("Frame", 6.0, 1),
                timed("Clouds", 0.0, 1),
                timed("DoF", 0.0, 1),
                timed("Shading", 4.0, 1),
                ProfilerRow::section("Scripts", IconId::Script, "0.1 ms"),
                ProfilerRow::stat("calls", "12", 1),
            ],
            fps: 60.0,
            scroll: 0.0,
            font_id: 0,
        };
        let lines = view.lines();
        assert_eq!(lines.iter().filter(|l| matches!(l, Line::Idle(2))).count(), 1);
        assert_eq!(lines.len(), 6);
        assert_eq!(view.frame_ms(), Some(6.0));
    }
}
