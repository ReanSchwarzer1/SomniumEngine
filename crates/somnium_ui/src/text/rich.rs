//! Rich text laid out in lines and drawn, with a reveal limit for typewriters.
//!
//! [`super::markup::parse`] turns BBCode into plain text and styled runs; nothing drew those runs until a dialogue
//! box needed to. This lays them out (word wrap across style changes, explicit line breaks) and draws them: colour,
//! size, a game's named faces, underline and strikethrough, wave and shake, inline sprites as tokens, and bold by
//! a registered face or, without one, by drawing twice. Italic needs a registered face; without one it stays upright.
//!
//! `revealed` counts visible characters (a sprite is one), so a typewriter shows a line letter by letter without
//! re-parsing it, and the reveal never splits a styled run's look from its text.

use super::{Decoration, Motion, StyledRun};
use crate::draw::DrawingContext;
use crate::types::Rect;
use glam::Vec2;

/// Faces and defaults a layout draws with.
#[derive(Clone, Debug, Default)]
pub struct RichStyle {
    /// The base face.
    pub font: u8,
    pub px: f32,
    pub color: [u8; 4],
    /// A bold face, when the canvas has one.
    pub bold: Option<u8>,
    /// An italic face, when the canvas has one.
    pub italic: Option<u8>,
    /// `[face=name]` faces: `(name, font id)`.
    pub faces: Vec<(String, u8)>,
    /// Line height as a multiple of the largest size in the text.
    pub leading: f32,
}

/// One drawn span: uniform style, one line.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Index of its first visible character in the whole text.
    pub start: usize,
    pub text: String,
    pub x: f32,
    pub line: usize,
    pub width: f32,
    pub font: u8,
    pub px: f32,
    pub color: [u8; 4],
    pub decoration: Decoration,
    pub motion: Motion,
    pub sprite: Option<String>,
    /// Drawn twice, a hair apart: bold with no bold face.
    pub faux_bold: bool,
}

/// A laid-out text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichLayout {
    pub pieces: Vec<Piece>,
    pub lines: usize,
    pub line_height: f32,
    /// The widest line.
    pub width: f32,
    /// Visible characters in total.
    pub chars: usize,
}

impl RichLayout {
    #[must_use]
    pub fn height(&self) -> f32 {
        self.lines as f32 * self.line_height
    }
}

struct Seg {
    start: usize,
    text: String,
    run: usize,
}

/// Lay `plain` (styled by `runs`, as [`super::markup::parse`] returns them) out within `max_width`.
pub fn layout(
    plain: &str,
    runs: &[StyledRun],
    max_width: f32,
    style: &RichStyle,
    mut measure: impl FnMut(&str, f32, u8) -> f32,
) -> RichLayout {
    let run_at = |byte: usize| runs.iter().position(|r| r.range.contains(&byte));
    let look =
        |run: Option<usize>| -> (u8, f32, [u8; 4], Decoration, Motion, Option<String>, bool) {
            let Some(run) = run.and_then(|i| runs.get(i)) else {
                return (
                    style.font,
                    style.px,
                    style.color,
                    Decoration::default(),
                    Motion::None,
                    None,
                    false,
                );
            };
            let mut font = run
                .face
                .as_ref()
                .and_then(|face| {
                    style
                        .faces
                        .iter()
                        .find(|(name, _)| name == face)
                        .map(|(_, id)| *id)
                })
                .unwrap_or(style.font);
            let mut faux_bold = false;
            if run.face.is_none() {
                if run.decoration.bold {
                    match style.bold {
                        Some(bold) => font = bold,
                        None => faux_bold = true,
                    }
                } else if run.decoration.italic
                    && let Some(italic) = style.italic
                {
                    font = italic;
                }
            }
            (
                font,
                run.size.unwrap_or(style.px),
                run.color.unwrap_or(style.color),
                run.decoration,
                run.motion,
                run.sprite.clone(),
                faux_bold,
            )
        };
    // Words: runs of non-space with their trailing spaces, split where the style changes; `\n` is its own token.
    let mut words: Vec<Vec<Seg>> = Vec::new();
    let mut word: Vec<Seg> = Vec::new();
    let mut in_space = false;
    let mut index = 0usize;
    for (byte, ch) in plain.char_indices() {
        let run = run_at(byte).unwrap_or(usize::MAX);
        if ch == '\n' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            words.push(Vec::new());
            in_space = false;
            // A break is a character too, as the presenter's cue positions count it.
            index += 1;
            continue;
        }
        let space = ch == ' ';
        if !space && in_space && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        in_space = space;
        match word.last_mut() {
            Some(seg) if seg.run == run => seg.text.push(ch),
            _ => word.push(Seg {
                start: index,
                text: ch.to_string(),
                run,
            }),
        }
        index += 1;
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut out = RichLayout {
        chars: index,
        ..RichLayout::default()
    };
    let mut largest = style.px;
    let mut x = 0.0f32;
    let mut line = 0usize;
    for word in words {
        if word.is_empty() {
            line += 1;
            x = 0.0;
            continue;
        }
        let widths: Vec<f32> = word
            .iter()
            .map(|seg| {
                let (font, px, _, _, _, sprite, _) =
                    look((seg.run != usize::MAX).then_some(seg.run));
                if sprite.is_some() {
                    px
                } else {
                    measure(&seg.text, px, font)
                }
            })
            .collect();
        let trailing = word
            .last()
            .map(|seg| {
                let spaces = seg.text.len() - seg.text.trim_end_matches(' ').len();
                let (font, px, ..) = look((seg.run != usize::MAX).then_some(seg.run));
                if spaces == 0 {
                    0.0
                } else {
                    measure(&" ".repeat(spaces), px, font)
                }
            })
            .unwrap_or(0.0);
        let width: f32 = widths.iter().sum();
        if x > 0.0 && x + width - trailing > max_width {
            line += 1;
            x = 0.0;
        }
        for (seg, w) in word.into_iter().zip(widths) {
            let (font, px, color, decoration, motion, sprite, faux_bold) =
                look((seg.run != usize::MAX).then_some(seg.run));
            largest = largest.max(px);
            match out.pieces.last_mut() {
                Some(last)
                    if last.line == line
                        && last.font == font
                        && last.px == px
                        && last.color == color
                        && last.decoration == decoration
                        && last.motion == motion
                        && last.sprite.is_none()
                        && sprite.is_none()
                        && last.faux_bold == faux_bold
                        && last.start + last.text.chars().count() == seg.start =>
                {
                    last.text.push_str(&seg.text);
                    last.width += w;
                }
                _ => out.pieces.push(Piece {
                    start: seg.start,
                    text: seg.text,
                    x,
                    line,
                    width: w,
                    font,
                    px,
                    color,
                    decoration,
                    motion,
                    sprite,
                    faux_bold,
                }),
            }
            x += w;
            out.width = out.width.max(x);
        }
    }
    out.lines = if plain.is_empty() { 0 } else { line + 1 };
    out.line_height = largest
        * if style.leading > 0.0 {
            style.leading
        } else {
            1.4
        };
    out
}

/// Draw the first `revealed` visible characters of `layout` at `origin`. `time_s` animates wave and shake;
/// `alpha` fades the whole text.
pub fn draw(
    layout: &RichLayout,
    ctx: &mut DrawingContext,
    origin: Vec2,
    revealed: usize,
    time_s: f32,
    alpha: u8,
) {
    for piece in &layout.pieces {
        let shown = revealed
            .saturating_sub(piece.start)
            .min(piece.text.chars().count());
        if shown == 0 {
            continue;
        }
        let mut color = piece.color;
        color[3] = ((u16::from(color[3]) * u16::from(alpha)) / 255) as u8;
        let top = origin.y
            + piece.line as f32 * layout.line_height
            + (layout.line_height - piece.px) * 0.5;
        let at = Vec2::new(origin.x + piece.x, top);
        if let Some(_sprite) = &piece.sprite {
            // A token where the sprite goes: games with sprite sheets draw their own over it.
            ctx.push_primitive(
                crate::primitive::Primitive::fill(
                    Rect::new(at.x + 1.0, at.y + 1.0, piece.px - 2.0, piece.px - 2.0),
                    color,
                )
                .with_radius(piece.px * 0.25),
                None,
            );
            continue;
        }
        let text: String = piece.text.chars().take(shown).collect();
        match piece.motion {
            Motion::None => {
                ctx.push_text(&text, at, piece.font, piece.px, color);
                if piece.faux_bold {
                    ctx.push_text(&text, at + Vec2::new(0.6, 0.0), piece.font, piece.px, color);
                }
            }
            Motion::Wave { .. } | Motion::Shake { .. } => {
                let mut x = at.x;
                for (i, ch) in text.chars().enumerate() {
                    let glyph = ch.to_string();
                    let k = (piece.start + i) as f32;
                    let offset = match piece.motion {
                        Motion::Wave {
                            amplitude,
                            frequency,
                        } => Vec2::new(
                            0.0,
                            amplitude
                                * (std::f32::consts::TAU * frequency * time_s + k * 0.55).sin(),
                        ),
                        Motion::Shake { amplitude } => {
                            let tick = (time_s * 30.0).floor();
                            let noise = |seed: f32| {
                                ((seed * 12.9898 + tick * 78.233).sin() * 43_758.547).fract() * 2.0
                                    - 1.0
                            };
                            Vec2::new(noise(k) * amplitude, noise(k + 0.5) * amplitude)
                        }
                        Motion::None => Vec2::ZERO,
                    };
                    ctx.push_text(
                        &glyph,
                        Vec2::new(x, at.y) + offset,
                        piece.font,
                        piece.px,
                        color,
                    );
                    if piece.faux_bold {
                        ctx.push_text(
                            &glyph,
                            Vec2::new(x + 0.6, at.y) + offset,
                            piece.font,
                            piece.px,
                            color,
                        );
                    }
                    x += ctx.font_atlas.measure_text(&glyph, piece.px, piece.font).x;
                }
            }
        }
        let partial = shown < piece.text.chars().count();
        let width = if partial {
            ctx.font_atlas.measure_text(&text, piece.px, piece.font).x
        } else {
            piece.width
        };
        if piece.decoration.underline {
            ctx.push_rect_filled(
                Rect::new(
                    at.x,
                    top + piece.px * 1.08,
                    width,
                    (piece.px / 14.0).max(1.0),
                ),
                color,
            );
        }
        if piece.decoration.strikethrough {
            ctx.push_rect_filled(
                Rect::new(
                    at.x,
                    top + piece.px * 0.58,
                    width,
                    (piece.px / 14.0).max(1.0),
                ),
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(text: &str, px: f32, _: u8) -> f32 {
        text.chars().count() as f32 * px * 0.5
    }

    fn laid(markup: &str, width: f32) -> RichLayout {
        let (plain, runs) = super::super::markup::parse(markup).unwrap();
        let style = RichStyle {
            font: 0,
            px: 10.0,
            color: [255; 4],
            bold: None,
            italic: Some(7),
            faces: vec![("ladder".into(), 3)],
            leading: 1.5,
        };
        layout(&plain, &runs, width, &style, measure)
    }

    #[test]
    fn words_wrap_across_style_changes() {
        // 5 px a character.
        let layout = laid("one [color=#ff0000]two[/color] three four", 50.0);
        let lines: Vec<(usize, &str)> = layout
            .pieces
            .iter()
            .map(|p| (p.line, p.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [(0, "one "), (0, "two"), (0, " "), (1, "three four")]
        );
        assert_eq!(layout.lines, 2);
        assert_eq!(layout.line_height, 15.0);
        assert_eq!(layout.chars, 18);
        assert_eq!(layout.pieces[1].color, [255, 0, 0, 255]);
    }

    #[test]
    fn faces_bold_italic_and_breaks() {
        let layout = laid(
            "[face=ladder]Hlt[/face] [b]now[/b]\n[i]soft[/i] [sprite=coin]",
            400.0,
        );
        let find = |text: &str| {
            layout
                .pieces
                .iter()
                .find(|p| p.text.trim() == text)
                .unwrap()
        };
        assert_eq!(find("Hlt").font, 3, "a registered face");
        assert!(find("now").faux_bold, "bold with no bold face draws twice");
        assert_eq!(find("soft").font, 7);
        assert_eq!(find("soft").line, 1, "an explicit break");
        let sprite = layout.pieces.iter().find(|p| p.sprite.is_some()).unwrap();
        assert_eq!((sprite.width, sprite.start), (10.0, 13));
        assert_eq!(layout.chars, 14, "a sprite is one character");
    }

    #[test]
    fn an_empty_text_has_no_lines() {
        let layout = laid("", 100.0);
        assert_eq!((layout.lines, layout.chars), (0, 0));
    }
}
