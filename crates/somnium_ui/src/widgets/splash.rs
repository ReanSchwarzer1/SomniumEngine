//! The editor's launch splash (Phase ALYX-J).
//!
//! The engine mark drawn on stroke by stroke, the wordmark tracking in, and what the editor is doing; then, once
//! loading is done, the two halves of the mark turn apart as the cover fades and the editor shows through. The mark
//! comes from the same SVG as the title bar and the taskbar icon (`assets/brand/somnium-s-eclipse.svg`), so the
//! three can never drift.
//!
//! Timing curves are After Effects-style temporal eases, translated from EffectCraft's `keyframe` crate
//! (`ease_progress`; MIT or Apache-2.0, `Downloads/Craft_Source/effectcraft-main/crates/keyframe/src/lib.rs`).
use std::time::Instant;

use glam::Vec2;

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    path::{Cap, Join, Path, Stroke},
    shaped::ShapedInstance,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};

/// The mark's view box, and its stroke width in that box.
const MARK_BOX: f32 = 64.0;
const MARK_STROKE: f32 = 9.2;
/// On screen.
const MARK_PX: f32 = 104.0;
/// The intro's length, and how long the cover takes to leave.
const INTRO_S: f32 = 1.1;
const EXIT_S: f32 = 0.6;

fn bez(a: f64, b: f64, c: f64, d: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c + u * u * u * d
}

fn solve_u(x0: f64, x1: f64, x2: f64, x3: f64, x: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..48 {
        let mid = (lo + hi) * 0.5;
        if bez(x0, x1, x2, x3, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// EffectCraft's `ease_progress`: progress through a temporal Bezier segment with normalised out/in speeds and
/// influences. Speeds of 0 with influences of a third are After Effects' Easy Ease.
#[must_use]
pub fn ease_progress(out_speed: f64, out_influence: f64, in_speed: f64, in_influence: f64, x: f64) -> f64 {
    let (i0, i1) = (out_influence.clamp(0.0, 1.0), in_influence.clamp(0.0, 1.0));
    let u = solve_u(0.0, i0, 1.0 - i1, 1.0, x.clamp(0.0, 1.0));
    bez(0.0, out_speed * i0, 1.0 - in_speed * i1, 1.0, u)
}

/// A long, soft landing (out fast, in very slowly), for things arriving.
fn arrive(x: f32) -> f32 {
    ease_progress(1.6, 0.25, 0.0, 0.85, f64::from(x)) as f32
}

/// Easy Ease, for things leaving.
fn easy(x: f32) -> f32 {
    ease_progress(0.0, 0.33, 0.0, 0.33, f64::from(x)) as f32
}

/// The mark's strokes as polylines in its 64-unit box, flattened from the SVG with every transform applied.
fn mark_strokes() -> Vec<Vec<Vec2>> {
    let Some(source) = crate::icon_svg::source_for(crate::icons::IconId::EngineMark) else {
        return Vec::new();
    };
    let Ok(tree) = resvg::usvg::Tree::from_str(source, &resvg::usvg::Options::default()) else {
        return Vec::new();
    };
    let mut strokes = Vec::new();
    collect(tree.root(), &mut strokes);
    strokes
}

fn collect(group: &resvg::usvg::Group, out: &mut Vec<Vec<Vec2>>) {
    use resvg::tiny_skia::PathSegment;
    for node in group.children() {
        match node {
            resvg::usvg::Node::Group(inner) => collect(inner, out),
            resvg::usvg::Node::Path(path) => {
                let t = path.abs_transform();
                let map = |p: resvg::tiny_skia::Point| {
                    Vec2::new(t.sx * p.x + t.kx * p.y + t.tx, t.ky * p.x + t.sy * p.y + t.ty)
                };
                let mut line: Vec<Vec2> = Vec::new();
                for segment in path.data().segments() {
                    match segment {
                        PathSegment::MoveTo(p) => {
                            if line.len() > 1 {
                                out.push(std::mem::take(&mut line));
                            }
                            line = vec![map(p)];
                        }
                        PathSegment::LineTo(p) => line.push(map(p)),
                        PathSegment::QuadTo(c, p) => {
                            let a = *line.last().unwrap_or(&map(c));
                            let (c, p) = (map(c), map(p));
                            for i in 1..=12 {
                                let u = i as f32 / 12.0;
                                line.push(a * (1.0 - u) * (1.0 - u) + c * 2.0 * u * (1.0 - u) + p * u * u);
                            }
                        }
                        PathSegment::CubicTo(c1, c2, p) => {
                            let a = *line.last().unwrap_or(&map(c1));
                            let (c1, c2, p) = (map(c1), map(c2), map(p));
                            for i in 1..=16 {
                                let u = i as f32 / 16.0;
                                let v = 1.0 - u;
                                line.push(a * v * v * v + c1 * 3.0 * v * v * u + c2 * 3.0 * v * u * u + p * u * u * u);
                            }
                        }
                        PathSegment::Close => {
                            if let Some(&first) = line.first() {
                                line.push(first);
                            }
                        }
                    }
                }
                // A cubic that starts where the move put the pen repeats its first point; a zero-length segment
                // has no direction, and its round cap tessellates into a hole.
                line.dedup_by(|b, a| a.distance(*b) < 1e-3);
                if line.len() > 1 {
                    out.push(line);
                }
            }
            _ => {}
        }
    }
}

/// The first `fraction` of a polyline, by length.
fn partial(line: &[Vec2], fraction: f32) -> Vec<Vec2> {
    let total: f32 = line.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * fraction.clamp(0.0, 1.0);
    let mut out = vec![line[0]];
    for w in line.windows(2) {
        let length = w[0].distance(w[1]);
        if length >= left {
            if length > 0.0 {
                out.push(w[0].lerp(w[1], left / length));
            }
            break;
        }
        out.push(w[1]);
        left -= length;
    }
    // Rounding can leave the last step at nothing; a zero-length tail has no direction and its cap draws a hole.
    out.dedup_by(|b, a| a.distance(*b) < 1e-3);
    out
}

#[derive(Clone, Debug)]
pub enum SplashMessage {
    /// What the editor is doing now, in a few words.
    Status(String),
    /// Loading is done: leave once the intro has played.
    Finish,
    /// Draw everything at rest: no draw-on, no turn, only the fade.
    ReducedMotion(bool),
}

pub struct Splash {
    born: Instant,
    finish: Option<Instant>,
    status: String,
    strokes: Vec<Vec<Vec2>>,
    font_id: u8,
    reduced: bool,
}

impl Splash {
    fn intro(&self, now: Instant) -> f32 {
        if self.reduced {
            1.0
        } else {
            (now - self.born).as_secs_f32() / INTRO_S
        }
    }

    /// 0 while showing, rising to 1 as the cover leaves.
    #[must_use]
    pub fn exit(&self, now: Instant) -> f32 {
        exit_progress(self.born, self.finish, now)
    }

    /// Whether it has finished leaving, so its owner can hide it.
    #[must_use]
    pub fn gone(&self, now: Instant) -> bool {
        self.exit(now) >= 1.0
    }
}

/// How far a splash born at `born` and told to finish at `finish` has left by `now`: 0 while showing, 1 once gone.
/// The exit starts when asked, but never before the intro has played. Its owner uses the same clock to drop it.
#[must_use]
pub fn exit_progress(born: Instant, finish: Option<Instant>, now: Instant) -> f32 {
    finish
        .map(|finish| finish.max(born + std::time::Duration::from_secs_f32(INTRO_S)))
        .map_or(0.0, |start| (now.saturating_duration_since(start).as_secs_f32() / EXIT_S).clamp(0.0, 1.0))
}

impl Control for Splash {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let now = Instant::now();
        let t = crate::theme::active();
        let b = widget.screen_bounds();
        let exit = easy(self.exit(now));
        let fade = 1.0 - exit;
        let alpha = |colour: [u8; 4], a: f32| [colour[0], colour[1], colour[2], (f32::from(colour[3]) * a) as u8];
        ctx.push_rect_filled(b, alpha(t.semantic.surface.canvas.bytes(), fade));

        let centre = Vec2::new(b.x + b.w * 0.5, b.y + b.h * 0.5 - 36.0);
        let scale = MARK_PX / MARK_BOX * (1.0 + 0.12 * exit);
        let intro = self.intro(now);
        let accent = t.semantic.accent.default.bytes();
        for (index, stroke) in self.strokes.iter().enumerate() {
            // The second half starts a beat after the first; on the way out they turn apart.
            let drawn = arrive(((intro - 0.12 * index as f32) / 0.8).clamp(0.0, 1.0));
            if drawn <= 0.001 || stroke.len() < 2 {
                continue;
            }
            let turn = if self.reduced { 0.0 } else { exit * 0.9 * if index == 0 { 1.0 } else { -1.0 } };
            let (sin, cos) = turn.sin_cos();
            let place = |p: Vec2| {
                let local = (p - Vec2::splat(MARK_BOX * 0.5)) * scale;
                centre + Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
            };
            let points: Vec<Vec2> = partial(stroke, drawn).into_iter().map(place).collect();
            let path = Path::polyline(&points);
            // Faint widening strokes under the real one fall off like a glow; a single wide one read as a dark ring.
            // No wider than 1.6 strokes: the S's counters are 1.9 strokes across, and a glow that nearly fills them
            // leaves a pin-point of background in each (it read as two dark dots).
            for (width, strength) in [(1.6, 0.04), (1.4, 0.06), (1.2, 0.09), (1.0, 1.0)] {
                let width = MARK_STROKE * scale * width;
                ctx.push_stroke(
                    &path,
                    &Stroke::new(width).with_cap(Cap::Round).with_join(Join::Round),
                    ShapedInstance::identity(alpha(accent, strength * fade)),
                );
            }
        }

        // The wordmark tracks in from wide to its resting spacing.
        let word = ((intro - 0.35) / 0.65).clamp(0.0, 1.0);
        if word > 0.0 {
            let settle = arrive(word);
            let px = 26.0;
            // Spacing in ems, settling from wide to its rest; the draw adds it in pixels after every letter,
            // the last included, so the word's own width is the tracked measure less one step.
            let tracking = (0.56 - 0.32 * settle) * px;
            let font = crate::typography::font_id(crate::typography::FontRole::UiSemiBold);
            let width = ctx.font_atlas.measure_text_tracked("SOMNIUM", px, font, tracking).x - tracking;
            ctx.push_text_tracked(
                "SOMNIUM",
                Vec2::new(centre.x - width * 0.5, centre.y + MARK_PX * 0.5 + 26.0),
                font,
                px,
                alpha(t.semantic.text.emphasis.bytes(), settle * fade),
                tracking,
            );
        }

        // What is happening, and a hairline that says the editor is alive while it happens.
        let line_y = centre.y + MARK_PX * 0.5 + 84.0;
        let status = if self.finish.is_some() { "Ready" } else { self.status.as_str() };
        let status_w = ctx.font_atlas.measure_text(status, 12.0, self.font_id).x;
        ctx.push_text(
            status,
            Vec2::new(centre.x - status_w * 0.5, line_y),
            self.font_id,
            12.0,
            alpha(t.semantic.text.muted.bytes(), fade),
        );
        let track = Rect::new(centre.x - 110.0, line_y + 24.0, 220.0, 2.0);
        ctx.push_rect_filled(track, alpha(t.semantic.border.subtle.bytes(), fade));
        if self.finish.is_some() {
            ctx.push_rect_filled(track, alpha(accent, fade));
        } else {
            let cycle = ((now - self.born).as_secs_f32() / 1.4).fract();
            let head = easy(cycle);
            let x0 = track.x + track.w * (head - 0.28).max(0.0);
            let x1 = track.x + track.w * head.min(1.0);
            ctx.push_rect_filled(Rect::new(x0, track.y, (x1 - x0).max(0.0), track.h), alpha(accent, fade));
        }
    }

    fn handle_routed_message(&mut self, _widget: &mut Widget, msg: &mut UiMessage, _emit: &mut Vec<UiMessage>) {
        match msg.data::<SplashMessage>() {
            Some(SplashMessage::Status(text)) => self.status.clone_from(text),
            Some(SplashMessage::Finish) => {
                self.finish.get_or_insert_with(Instant::now);
            }
            Some(SplashMessage::ReducedMotion(on)) => self.reduced = *on,
            None => return,
        }
        msg.handled = true;
    }
}

pub struct SplashBuilder {
    widget: WidgetBuilder,
    status: String,
    font_id: u8,
}

impl SplashBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, status: "Starting".into(), font_id: 0 }
    }

    #[must_use]
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    #[must_use]
    pub fn with_font_id(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Splash {
                born: Instant::now(),
                finish: None,
                status: self.status,
                strokes: mark_strokes(),
                font_id: self.font_id,
                reduced: false,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ease_starts_and_lands_where_it_should() {
        for f in [arrive, easy] {
            assert!(f(0.0).abs() < 1e-3);
            assert!((f(1.0) - 1.0).abs() < 1e-3);
            assert!(f(0.25) < f(0.5) && f(0.5) < f(0.75), "monotone");
        }
        assert!(arrive(0.3) > 0.5, "arriving is fast early and slow late");
    }

    #[test]
    fn the_mark_is_two_strokes_from_its_svg() {
        let strokes = mark_strokes();
        assert_eq!(strokes.len(), 2);
        for stroke in &strokes {
            assert!(stroke.iter().all(|p| p.x > -1.0 && p.x < MARK_BOX + 1.0 && p.y > -1.0 && p.y < MARK_BOX + 1.0));
        }
    }

    #[test]
    fn the_exit_waits_for_the_intro_and_then_finishes() {
        let now = Instant::now();
        let mut splash = Splash {
            born: now,
            finish: Some(now),
            status: String::new(),
            strokes: Vec::new(),
            font_id: 0,
            reduced: false,
        };
        assert_eq!(splash.exit(now), 0.0, "finishing early still lets the intro play");
        let later = now + std::time::Duration::from_secs_f32(INTRO_S + EXIT_S + 0.01);
        assert!(splash.gone(later));
        splash.finish = None;
        assert!(!splash.gone(later), "never leaves on its own");
    }

    #[test]
    fn a_partial_stroke_is_its_share_of_the_length() {
        let line = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        let half = partial(&line, 0.5);
        assert_eq!(half.last().copied(), Some(Vec2::new(10.0, 0.0)));
        let quarter = partial(&line, 0.25);
        assert!((quarter.last().unwrap().x - 5.0).abs() < 1e-4);
        for stroke in mark_strokes() {
            let whole = partial(&stroke, 1.0);
            assert!(
                whole.windows(2).all(|w| w[0].distance(w[1]) > 1e-4),
                "no zero-length step, or its cap draws a hole"
            );
        }
    }
}
