//! A light-novel dialogue box for games: portraits left and right with expression swaps, a name plate,
//! typewriter rich text, choices taken by mouse, number keys or arrows, a history log, and auto and skip.
//!
//! The widget draws `somnium_story`'s presentation model ([`Presenter`]) and turns mouse and keys into its inputs;
//! the game moves the conversation. Each frame the game sends [`DialoguePresenterMessage::Tick`] with real
//! milliseconds (reading is the player's, so it does not stop for game pause unless the game says so), feeds it
//! beats from its `Conversation` and reads back [`DialoguePresenterMessage::Output`]: `Next`, `Chose(i)`,
//! `PlayVoice(path)`, `StopVoice`.
//!
//! Portraits are the game's pictures, registered per portrait set and expression with
//! [`DialoguePresenterMessage::SetPortrait`] (texture slots from `UiCanvas::add_image`). Until a set has art, a
//! placeholder card in the speaker's colour stands in, named, so a scene can be played and judged before any
//! portrait is painted.
//!
//! A game's fluency transform (Rimeborn: unknown words drawn in their script) goes in `ShowLine`'s `display`, as
//! markup; `[face=name]` picks a face registered with [`DialoguePresenterMessage::SetFaces`].
//!
//! Keys: `1`-`9` choose, arrows move between choices, `Enter`/`Space` confirm or go on, `A` auto, `S` skip, `L` the
//! log, `Esc` closes the log and stops skipping. The mouse: click a choice, click the box to go on, the wheel up
//! opens the log.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, MouseButton, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    primitive::Primitive,
    text::rich::{self, RichStyle},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use somnium_story::conversation::{ChoiceBeat, LineBeat};
use somnium_story::manifest::Side;
use somnium_story::presenter::{Presenter, PresenterInput, PresenterOutput, PresenterSettings};
use std::collections::HashMap;

/// Messages to and from the presenter.
#[derive(Clone, Debug)]
pub enum DialoguePresenterMessage {
    /// Show a line. `display` is the game's transform of the text (markup), or `None` for the line's own.
    ShowLine {
        beat: LineBeat,
        display: Option<String>,
    },
    /// Show a menu under the last line.
    ShowChoices(Vec<ChoiceBeat>),
    /// A timed menu's clock: `(left, total)` milliseconds.
    SetTimeLeft(Option<(u64, u64)>),
    /// The conversation ended.
    Close,
    /// The voice clip a `PlayVoice` asked for finished.
    VoiceFinished,
    /// Real milliseconds since the last frame.
    Tick(u64),
    /// Input from the game's own bindings (a controller, input actions).
    Input(PresenterInput),
    /// A portrait picture: `slot` from `UiCanvas::add_image`, `size` in pixels. An empty `expression` is the set's
    /// fallback for expressions without their own picture.
    SetPortrait {
        portrait: String,
        expression: String,
        slot: u32,
        size: Vec2,
    },
    /// Faces for bold, italic and `[face=name]` text.
    SetFaces {
        bold: Option<u8>,
        italic: Option<u8>,
        faces: Vec<(String, u8)>,
    },
    SetSettings(PresenterSettings),
    /// Dim everything under the presenter (alpha 0-255) while it is up: an editor preview, a game's cutscene.
    SetBackdrop(u8),
    /// From the widget: what the player asked for.
    Output(PresenterOutput),
}

/// Where everything goes, in screen pixels, for a canvas of `bounds`.
#[derive(Clone, Debug, PartialEq)]
pub struct PresenterLayout {
    pub unit: f32,
    pub dialogue_box: Rect,
    pub text: Rect,
    /// `[left, right]`.
    pub portraits: [Rect; 2],
    pub choices: Vec<Rect>,
    /// Auto, Skip, Log.
    pub buttons: [Rect; 3],
    pub history: Rect,
}

impl PresenterLayout {
    #[must_use]
    pub fn new(bounds: Rect, choices: usize) -> Self {
        let unit = (bounds.h / 1080.0).max(0.25);
        let box_h = (bounds.h * 0.24).max(150.0 * unit);
        let dialogue_box = Rect::new(
            bounds.x + bounds.w * 0.08,
            bounds.y + bounds.h - box_h - bounds.h * 0.04,
            bounds.w * 0.84,
            box_h,
        );
        let text = Rect::new(
            dialogue_box.x + 44.0 * unit,
            dialogue_box.y + 40.0 * unit,
            dialogue_box.w - 88.0 * unit,
            dialogue_box.h - 64.0 * unit,
        );
        let portrait_h = bounds.h * 0.74;
        let portrait_w = portrait_h * 0.62;
        let portrait_bottom = dialogue_box.y + dialogue_box.h * 0.35;
        let portraits = [
            Rect::new(
                bounds.x + bounds.w * 0.05,
                portrait_bottom - portrait_h,
                portrait_w,
                portrait_h,
            ),
            Rect::new(
                bounds.x + bounds.w * 0.95 - portrait_w,
                portrait_bottom - portrait_h,
                portrait_w,
                portrait_h,
            ),
        ];
        let width = (bounds.w * 0.5).min(940.0 * unit);
        let row = 56.0 * unit;
        let gap = 10.0 * unit;
        let list_h = choices as f32 * (row + gap);
        let top = dialogue_box.y - 28.0 * unit - list_h;
        let choices = (0..choices)
            .map(|i| {
                Rect::new(
                    bounds.x + (bounds.w - width) * 0.5,
                    top + i as f32 * (row + gap),
                    width,
                    row,
                )
            })
            .collect();
        let button = Vec2::new(78.0 * unit, 30.0 * unit);
        let right = dialogue_box.x + dialogue_box.w - 16.0 * unit;
        let buttons = [2.0, 1.0, 0.0].map(|k: f32| {
            Rect::new(
                right - (k + 1.0) * (button.x + 8.0 * unit) + 8.0 * unit,
                dialogue_box.y + 10.0 * unit,
                button.x,
                button.y,
            )
        });
        let history = Rect::new(
            bounds.x + bounds.w * 0.12,
            bounds.y + bounds.h * 0.07,
            bounds.w * 0.76,
            bounds.h * 0.66,
        );
        Self {
            unit,
            dialogue_box,
            text,
            portraits,
            choices,
            buttons,
            history,
        }
    }
}

/// The widget.
pub struct DialoguePresenter {
    model: Presenter,
    font_id: u8,
    bold: Option<u8>,
    italic: Option<u8>,
    faces: Vec<(String, u8)>,
    portraits: HashMap<(String, String), (u32, Vec2)>,
    hover: Option<usize>,
    clock_ms: u64,
    history_scroll: f32,
    backdrop: u8,
}

impl DialoguePresenter {
    fn layout(&self, widget: &Widget) -> PresenterLayout {
        PresenterLayout::new(widget.screen_bounds(), self.model.choices().len())
    }

    fn style(&self, px: f32, color: [u8; 4]) -> RichStyle {
        RichStyle {
            font: self.font_id,
            px,
            color,
            bold: self.bold,
            italic: self.italic,
            faces: self.faces.clone(),
            leading: 1.45,
        }
    }

    fn emit_outputs(&mut self, widget: &mut Widget, emit: &mut Vec<UiMessage>) {
        for output in self.model.drain() {
            emit.push(UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                DialoguePresenterMessage::Output(output),
            ));
        }
        // Hidden, it lets the pointer through to whatever is under it.
        widget.hit_self = self.model.is_visible();
    }

    fn choice_at(&self, widget: &Widget, pos: Vec2) -> Option<usize> {
        self.layout(widget)
            .choices
            .iter()
            .position(|r| r.contains(pos))
    }

    fn input_for_key(key: KeyCode) -> Option<PresenterInput> {
        let digit = match key {
            KeyCode::Digit1 | KeyCode::Numpad1 => Some(0),
            KeyCode::Digit2 | KeyCode::Numpad2 => Some(1),
            KeyCode::Digit3 | KeyCode::Numpad3 => Some(2),
            KeyCode::Digit4 | KeyCode::Numpad4 => Some(3),
            KeyCode::Digit5 | KeyCode::Numpad5 => Some(4),
            KeyCode::Digit6 | KeyCode::Numpad6 => Some(5),
            KeyCode::Digit7 | KeyCode::Numpad7 => Some(6),
            KeyCode::Digit8 | KeyCode::Numpad8 => Some(7),
            KeyCode::Digit9 | KeyCode::Numpad9 => Some(8),
            _ => None,
        };
        if let Some(index) = digit {
            return Some(PresenterInput::Choose(index));
        }
        Some(match key {
            KeyCode::ArrowDown | KeyCode::ArrowRight => PresenterInput::FocusNext,
            KeyCode::ArrowUp | KeyCode::ArrowLeft => PresenterInput::FocusPrevious,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => PresenterInput::Confirm,
            KeyCode::KeyA => PresenterInput::ToggleAuto,
            KeyCode::KeyS => PresenterInput::ToggleSkip,
            KeyCode::KeyL | KeyCode::KeyH => PresenterInput::ToggleHistory,
            KeyCode::Escape => PresenterInput::Back,
            _ => return None,
        })
    }

    fn portrait_picture(&self, portrait: &str, expression: &str) -> Option<(u32, Vec2)> {
        self.portraits
            .get(&(portrait.to_owned(), expression.to_owned()))
            .or_else(|| self.portraits.get(&(portrait.to_owned(), String::new())))
            .copied()
    }

    fn speaker_colour(&self, speaker: &str) -> [u8; 4] {
        let line = self.model.line();
        line.filter(|l| l.speaker == speaker)
            .and_then(|l| l.color)
            .unwrap_or_else(|| theme::active().semantic.accent.default.bytes())
    }

    fn draw_portraits(&self, ctx: &mut DrawingContext, layout: &PresenterLayout) {
        let theme = theme::active();
        let mut order: Vec<usize> = (0..2).collect();
        // The listener first, so the speaker is drawn over any overlap.
        order.sort_by_key(|i| {
            self.model.portraits()[*i]
                .as_ref()
                .is_some_and(|p| p.active)
        });
        for index in order {
            let Some(portrait) = &self.model.portraits()[index] else {
                continue;
            };
            let area = layout.portraits[index];
            let dim = if portrait.active { 255 } else { 150 };
            let tint = [dim, dim, (f32::from(dim) * 1.06).min(255.0) as u8, 255];
            match self.portrait_picture(&portrait.portrait, &portrait.expression) {
                Some((slot, size)) => {
                    let aspect = if size.y > 0.0 { size.x / size.y } else { 0.62 };
                    let h = area.h * if portrait.active { 1.0 } else { 0.97 };
                    let w = h * aspect;
                    let x = if index == 0 {
                        area.x
                    } else {
                        area.x + area.w - w
                    };
                    let rect = Rect::new(x, area.y + area.h - h, w, h);
                    let mut picture = Primitive::textured(rect, [0.0, 0.0, 1.0, 1.0], tint);
                    picture.flags |= crate::primitive::FLAG_SRGB_TEXELS;
                    ctx.push_primitive(picture, Some(slot));
                }
                None => {
                    let colour = self.portrait_colour(&portrait.speaker);
                    let card = Rect::new(
                        area.x + area.w * 0.12,
                        area.y + area.h * 0.18,
                        area.w * 0.76,
                        area.h * 0.82,
                    );
                    let alpha = if portrait.active { 210 } else { 120 };
                    // A dark card tinted toward the speaker, nearly opaque like the art it stands in for.
                    let base = theme.semantic.surface.raised.bytes();
                    let tint =
                        |i: usize| ((u16::from(base[i]) * 3 + u16::from(colour[i])) / 4) as u8;
                    let fill = [
                        tint(0),
                        tint(1),
                        tint(2),
                        if portrait.active { 245 } else { 200 },
                    ];
                    ctx.push_primitive(
                        Primitive::fill(card, fill)
                            .with_radius(card.w * 0.18)
                            .with_border(2.0 * layout.unit, theme::with_alpha(colour, alpha)),
                        None,
                    );
                    let initial: String = portrait
                        .speaker
                        .chars()
                        .take(1)
                        .flat_map(char::to_uppercase)
                        .collect();
                    let px = card.w * 0.42;
                    let w = ctx.font_atlas.measure_text(&initial, px, self.font_id).x;
                    ctx.push_text(
                        &initial,
                        Vec2::new(card.x + (card.w - w) * 0.5, card.y + card.h * 0.22),
                        self.font_id,
                        px,
                        theme::with_alpha(colour, alpha),
                    );
                    let label = format!("{} · {}", portrait.portrait, portrait.expression);
                    let px = 15.0 * layout.unit;
                    let w = ctx.font_atlas.measure_text(&label, px, self.font_id).x;
                    ctx.push_text(
                        &label,
                        Vec2::new(card.x + (card.w - w) * 0.5, card.y + card.h * 0.62),
                        self.font_id,
                        px,
                        theme::with_alpha(theme.semantic.text.secondary.bytes(), alpha),
                    );
                }
            }
        }
    }

    fn portrait_colour(&self, speaker: &str) -> [u8; 4] {
        self.speaker_colour(speaker)
    }

    fn draw_box(&self, ctx: &mut DrawingContext, layout: &PresenterLayout) {
        let theme = theme::active();
        let unit = layout.unit;
        let panel = layout.dialogue_box;
        // A menu with no line before it (a scene that opens on a choice) shows no empty box.
        let Some(line) = self.model.line() else {
            return;
        };
        ctx.push_primitive(
            Primitive::fill(
                panel,
                theme::with_alpha(theme.semantic.surface.canvas.bytes(), 228),
            )
            .with_radius(18.0 * unit)
            .with_border(
                2.0 * unit,
                theme::with_alpha(theme.semantic.border.strong.bytes(), 200),
            ),
            None,
        );
        if !line.name.is_empty() {
            let px = 26.0 * unit;
            let w = ctx.font_atlas.measure_text(&line.name, px, self.font_id).x + 44.0 * unit;
            let x = if line.side == Side::Right && !line.portrait.is_empty() {
                panel.x + panel.w - w - 40.0 * unit
            } else {
                panel.x + 40.0 * unit
            };
            let plate = Rect::new(x, panel.y - 26.0 * unit, w, 48.0 * unit);
            let colour = line.color.unwrap_or(theme.semantic.accent.default.bytes());
            ctx.push_primitive(
                Primitive::fill(plate, theme.semantic.surface.raised.bytes())
                    .with_radius(12.0 * unit)
                    .with_border(2.0 * unit, colour),
                None,
            );
            ctx.push_text(
                &line.name,
                Vec2::new(plate.x + 22.0 * unit, plate.y + 10.0 * unit),
                self.font_id,
                px,
                colour,
            );
        }
        let text = self.model.display_text();
        let (plain, runs) =
            crate::text::markup::parse(text).unwrap_or_else(|_| (text.to_owned(), Vec::new()));
        let px = 29.0 * unit;
        let style = self.style(px, theme.semantic.text.primary.bytes());
        let font_atlas = &ctx.font_atlas;
        let laid = rich::layout(&plain, &runs, layout.text.w, &style, |s, px, font| {
            font_atlas.measure_text(s, px, font).x
        });
        ctx.push_clip_rect(layout.text);
        rich::draw(
            &laid,
            ctx,
            layout.text.pos(),
            self.model.revealed(),
            self.clock_ms as f32 / 1000.0,
            255,
        );
        ctx.pop_clip_rect();
        if self.model.line_complete() && self.model.choices().is_empty() && !line.menu_next {
            // The "go on" mark, bobbing.
            let bob = ((self.clock_ms as f32 / 1000.0) * 3.0).sin() * 3.0 * unit;
            let s = 14.0 * unit;
            let at = Vec2::new(
                panel.x + panel.w - 44.0 * unit,
                panel.y + panel.h - 34.0 * unit + bob,
            );
            ctx.push_primitive(
                Primitive::fill(
                    Rect::new(at.x, at.y, s, s * 0.6),
                    theme.semantic.accent.default.bytes(),
                )
                .with_radius(s * 0.3),
                None,
            );
        }
        let labels = [
            ("Auto", self.model.settings.auto),
            (
                "Skip",
                self.model.settings.skip != somnium_story::presenter::SkipMode::Off,
            ),
            ("Log", self.model.history_open()),
        ];
        for (rect, (label, on)) in layout.buttons.iter().zip(labels) {
            ctx.push_primitive(
                Primitive::fill(
                    *rect,
                    if on {
                        theme::with_alpha(theme.semantic.accent.default.bytes(), 200)
                    } else {
                        theme::with_alpha(theme.semantic.surface.raised.bytes(), 170)
                    },
                )
                .with_radius(8.0 * unit),
                None,
            );
            let px = 15.0 * unit;
            let w = ctx.font_atlas.measure_text(label, px, self.font_id).x;
            ctx.push_text(
                label,
                Vec2::new(rect.x + (rect.w - w) * 0.5, rect.y + 7.0 * unit),
                self.font_id,
                px,
                if on {
                    theme.semantic.text.inverse.bytes()
                } else {
                    theme.semantic.text.secondary.bytes()
                },
            );
        }
    }

    fn draw_choices(&self, ctx: &mut DrawingContext, layout: &PresenterLayout) {
        let theme = theme::active();
        let unit = layout.unit;
        let choices = self.model.choices();
        if choices.is_empty() {
            return;
        }
        if let (Some((left, total)), Some(first)) = (self.model.time_left(), layout.choices.first())
        {
            let fraction = if total == 0 {
                0.0
            } else {
                left as f32 / total as f32
            };
            let bar = Rect::new(first.x, first.y - 18.0 * unit, first.w, 6.0 * unit);
            ctx.push_primitive(
                Primitive::fill(
                    bar,
                    theme::with_alpha(theme.semantic.surface.raised.bytes(), 180),
                )
                .with_radius(3.0 * unit),
                None,
            );
            ctx.push_primitive(
                Primitive::fill(
                    Rect::new(bar.x, bar.y, bar.w * fraction, bar.h),
                    theme.semantic.status.warning.bytes(),
                )
                .with_radius(3.0 * unit),
                None,
            );
        }
        for (choice, rect) in choices.iter().zip(&layout.choices) {
            let focused = self.model.focus() == Some(choice.index);
            let hovered = self.hover == Some(choice.index);
            let fill = if !choice.available {
                theme::with_alpha(theme.semantic.surface.panel.bytes(), 170)
            } else if focused || hovered {
                theme::with_alpha(theme.semantic.accent.selected_bg.bytes(), 240)
            } else {
                theme::with_alpha(theme.semantic.surface.canvas.bytes(), 224)
            };
            ctx.push_primitive(
                Primitive::fill(*rect, fill)
                    .with_radius(12.0 * unit)
                    .with_border(
                        if focused { 2.5 * unit } else { 1.5 * unit },
                        if focused {
                            theme.semantic.accent.default.bytes()
                        } else {
                            theme::with_alpha(theme.semantic.border.default.bytes(), 200)
                        },
                    ),
                None,
            );
            let px = 24.0 * unit;
            let colour = if choice.available {
                theme.semantic.text.primary.bytes()
            } else {
                theme.semantic.text.muted.bytes()
            };
            let number = format!("{}", choice.index + 1);
            ctx.push_text(
                &number,
                Vec2::new(rect.x + 20.0 * unit, rect.y + 14.0 * unit),
                self.font_id,
                px,
                theme::with_alpha(
                    theme.semantic.accent.default.bytes(),
                    if choice.available { 255 } else { 120 },
                ),
            );
            let mut x = rect.x + 54.0 * unit;
            if let Some(check) = &choice.check {
                let label = match check.chance {
                    Some(chance) => format!("[{} · {chance}%]", check.label),
                    None => format!("[{}]", check.label),
                };
                let tag_colour = match check.passes {
                    Some(true) => theme.semantic.status.success.bytes(),
                    Some(false) => theme.semantic.text.muted.bytes(),
                    None => theme.semantic.status.warning.bytes(),
                };
                ctx.push_text(
                    &label,
                    Vec2::new(x, rect.y + 16.0 * unit),
                    self.font_id,
                    19.0 * unit,
                    tag_colour,
                );
                x += ctx
                    .font_atlas
                    .measure_text(&label, 19.0 * unit, self.font_id)
                    .x
                    + 12.0 * unit;
            }
            let (plain, runs) = crate::text::markup::parse(&choice.text)
                .unwrap_or_else(|_| (choice.text.clone(), Vec::new()));
            let style = self.style(px, colour);
            let font_atlas = &ctx.font_atlas;
            let laid = rich::layout(
                &plain,
                &runs,
                rect.x + rect.w - x - 18.0 * unit,
                &style,
                |s, px, font| font_atlas.measure_text(s, px, font).x,
            );
            ctx.push_clip_rect(*rect);
            rich::draw(
                &laid,
                ctx,
                Vec2::new(x, rect.y + 13.0 * unit),
                usize::MAX,
                self.clock_ms as f32 / 1000.0,
                255,
            );
            ctx.pop_clip_rect();
            if choice.taken && choice.available {
                ctx.push_primitive(
                    Primitive::fill(
                        Rect::new(
                            rect.x + rect.w - 18.0 * unit,
                            rect.y + rect.h * 0.5 - 4.0 * unit,
                            8.0 * unit,
                            8.0 * unit,
                        ),
                        theme::with_alpha(theme.semantic.text.muted.bytes(), 160),
                    )
                    .with_radius(4.0 * unit),
                    None,
                );
            }
        }
    }

    fn draw_history(&self, ctx: &mut DrawingContext, layout: &PresenterLayout) {
        if !self.model.history_open() {
            return;
        }
        let theme = theme::active();
        let unit = layout.unit;
        let panel = layout.history;
        ctx.push_primitive(
            Primitive::fill(
                panel,
                theme::with_alpha(theme.semantic.surface.canvas.bytes(), 245),
            )
            .with_radius(18.0 * unit)
            .with_border(2.0 * unit, theme.semantic.border.strong.bytes()),
            None,
        );
        ctx.push_text(
            "History",
            Vec2::new(panel.x + 32.0 * unit, panel.y + 22.0 * unit),
            self.font_id,
            26.0 * unit,
            theme.semantic.text.secondary.bytes(),
        );
        let body = Rect::new(
            panel.x + 32.0 * unit,
            panel.y + 70.0 * unit,
            panel.w - 64.0 * unit,
            panel.h - 96.0 * unit,
        );
        let px = 22.0 * unit;
        let font_atlas = &ctx.font_atlas;
        let mut entries: Vec<(f32, Vec<(String, [u8; 4])>)> = Vec::new();
        for entry in self.model.history() {
            let plain = crate::text::markup::parse(&entry.text)
                .map_or_else(|_| entry.text.clone(), |(p, _)| p);
            let (prefix, colour) = if entry.choice {
                ("› ".to_owned(), theme.semantic.accent.default.bytes())
            } else if entry.name.is_empty() {
                (String::new(), theme.semantic.text.secondary.bytes())
            } else {
                (
                    format!("{}: ", entry.name),
                    theme.semantic.text.primary.bytes(),
                )
            };
            let lines =
                crate::widgets::text::wrap_lines(&format!("{prefix}{plain}"), body.w, |s| {
                    font_atlas.measure_text(s, px, self.font_id).x
                });
            let height = lines.len() as f32 * px * 1.35 + 10.0 * unit;
            entries.push((height, lines.into_iter().map(|l| (l, colour)).collect()));
        }
        let total: f32 = entries.iter().map(|(h, _)| *h).sum();
        // Newest at the bottom; scrolling moves up through older lines.
        let scroll = self.history_scroll.clamp(0.0, (total - body.h).max(0.0));
        let mut y = body.y + body.h - total + scroll;
        ctx.push_clip_rect(body);
        for (height, lines) in entries {
            if y + height >= body.y && y <= body.y + body.h {
                for (row, (text, colour)) in lines.iter().enumerate() {
                    ctx.push_text(
                        text,
                        Vec2::new(body.x, y + row as f32 * px * 1.35),
                        self.font_id,
                        px,
                        *colour,
                    );
                }
            }
            y += height;
        }
        ctx.pop_clip_rect();
    }
}

impl Control for DialoguePresenter {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Dialog
    }

    fn a11y_name(&self) -> Option<String> {
        self.model.line().map(|line| {
            let text = crate::text::markup::parse(self.model.display_text())
                .map_or_else(|_| self.model.display_text().to_owned(), |(plain, _)| plain);
            if line.name.is_empty() {
                text
            } else {
                format!("{}: {text}", line.name)
            }
        })
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn is_keyboard_focusable(&self) -> bool {
        self.model.is_visible()
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        if self
            .choice_at(widget, pos)
            .is_some_and(|i| self.model.choices().get(i).is_some_and(|c| c.available))
        {
            CursorKind::Pointer
        } else {
            CursorKind::Default
        }
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if !self.model.is_visible() {
            return;
        }
        let layout = self.layout(widget);
        if self.backdrop > 0 {
            ctx.push_rect_filled(widget.screen_bounds(), [6, 6, 10, self.backdrop]);
        }
        self.draw_portraits(ctx, &layout);
        self.draw_box(ctx, &layout);
        self.draw_choices(ctx, &layout);
        self.draw_history(ctx, &layout);
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if msg.direction == MessageDirection::ToWidget
            && let Some(message) = msg.data::<DialoguePresenterMessage>().cloned()
        {
            match message {
                DialoguePresenterMessage::ShowLine { beat, display } => {
                    self.history_scroll = 0.0;
                    self.model.show_line(beat, display);
                }
                DialoguePresenterMessage::ShowChoices(choices) => {
                    self.hover = None;
                    self.model.show_choices(choices);
                }
                DialoguePresenterMessage::SetTimeLeft(time) => self.model.set_time_left(time),
                DialoguePresenterMessage::Close => self.model.close(),
                DialoguePresenterMessage::VoiceFinished => self.model.voice_finished(),
                DialoguePresenterMessage::Tick(ms) => {
                    self.clock_ms = self.clock_ms.wrapping_add(ms);
                    self.model.update(ms);
                }
                DialoguePresenterMessage::Input(input) => self.model.input(input),
                DialoguePresenterMessage::SetPortrait {
                    portrait,
                    expression,
                    slot,
                    size,
                } => {
                    self.portraits.insert((portrait, expression), (slot, size));
                }
                DialoguePresenterMessage::SetFaces {
                    bold,
                    italic,
                    faces,
                } => {
                    self.bold = bold;
                    self.italic = italic;
                    self.faces = faces;
                }
                DialoguePresenterMessage::SetSettings(settings) => self.model.settings = settings,
                DialoguePresenterMessage::SetBackdrop(alpha) => self.backdrop = alpha,
                DialoguePresenterMessage::Output(_) => return,
            }
            msg.handled = true;
            self.emit_outputs(widget, emit);
            return;
        }
        if !self.model.is_visible() {
            return;
        }
        let Some(message) = msg.data::<WidgetMessage>().cloned() else {
            return;
        };
        let layout = self.layout(widget);
        match message {
            WidgetMessage::MouseMove { pos, .. } => {
                self.hover = self.choice_at(widget, pos);
                self.model.input(PresenterInput::Hover(self.hover));
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(index) = layout.buttons.iter().position(|r| r.contains(pos)) {
                    self.model.input(match index {
                        0 => PresenterInput::ToggleAuto,
                        1 => PresenterInput::ToggleSkip,
                        _ => PresenterInput::ToggleHistory,
                    });
                } else if self.model.history_open() {
                    if !layout.history.contains(pos) {
                        self.model.input(PresenterInput::Back);
                    }
                } else if let Some(index) = self.choice_at(widget, pos) {
                    self.model.input(PresenterInput::Choose(index));
                } else if self.model.choices().is_empty() {
                    self.model.input(PresenterInput::Advance);
                }
            }
            WidgetMessage::MouseWheel { delta, .. } => {
                if self.model.history_open() {
                    self.history_scroll =
                        (self.history_scroll + delta.signum() * 60.0 * layout.unit).max(0.0);
                } else if delta > 0.0 {
                    self.model.input(PresenterInput::ToggleHistory);
                }
            }
            WidgetMessage::KeyDown(key, _) => match Self::input_for_key(key) {
                Some(input) => self.model.input(input),
                None => return,
            },
            _ => return,
        }
        msg.handled = true;
        self.emit_outputs(widget, emit);
    }
}

/// Builds a [`DialoguePresenter`], normally stretched over a whole screen canvas.
pub struct DialoguePresenterBuilder {
    widget: WidgetBuilder,
    font_id: u8,
    settings: PresenterSettings,
}

impl DialoguePresenterBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            font_id: 0,
            settings: PresenterSettings::default(),
        }
    }

    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn with_settings(mut self, settings: PresenterSettings) -> Self {
        self.settings = settings;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let mut widget = self.widget.with_background(theme::TRANSPARENT).build();
        widget.hit_self = false;
        UiNode::new(
            widget,
            Box::new(DialoguePresenter {
                model: Presenter::new(self.settings),
                font_id: self.font_id,
                bold: None,
                italic: None,
                faces: Vec::new(),
                portraits: HashMap::new(),
                hover: None,
                clock_ms: 0,
                history_scroll: 0.0,
                backdrop: 0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(speaker: &str, side: Side, text: &str) -> LineBeat {
        LineBeat {
            dialogue: "d".into(),
            node: "n1".into(),
            speaker: speaker.into(),
            name: speaker.to_uppercase(),
            portrait: format!("portraits/{speaker}"),
            expression: "neutral".into(),
            side,
            color: Some([200, 160, 255, 255]),
            text: text.into(),
            language: String::new(),
            voice: String::new(),
            tags: Vec::new(),
            seen: false,
            ambient: false,
            menu_next: false,
        }
    }

    fn choice(index: usize, available: bool) -> ChoiceBeat {
        ChoiceBeat {
            index,
            node: format!("o{index}"),
            text: format!("Choice {}", index + 1),
            available,
            check: None,
            tags: Vec::new(),
            taken: false,
        }
    }

    struct Rig {
        presenter: DialoguePresenter,
        widget: Widget,
    }

    impl Rig {
        fn new() -> Self {
            let mut widget = WidgetBuilder::new().build();
            widget.actual_local_size = Vec2::new(1920.0, 1080.0);
            // As the builder leaves it: hidden until a line arrives.
            widget.hit_self = false;
            let presenter = DialoguePresenter {
                model: Presenter::new(PresenterSettings {
                    chars_per_second: 10.0,
                    ..PresenterSettings::default()
                }),
                font_id: 0,
                bold: None,
                italic: None,
                faces: Vec::new(),
                portraits: HashMap::new(),
                hover: None,
                clock_ms: 0,
                history_scroll: 0.0,
                backdrop: 0,
            };
            Self { presenter, widget }
        }

        fn send(&mut self, message: impl std::any::Any + Send + 'static) -> Vec<PresenterOutput> {
            let mut msg = UiMessage::new(self.widget.handle, MessageDirection::ToWidget, message);
            let mut emit = Vec::new();
            self.presenter
                .handle_routed_message(&mut self.widget, &mut msg, &mut emit);
            emit.iter()
                .filter_map(|m| match m.data::<DialoguePresenterMessage>() {
                    Some(DialoguePresenterMessage::Output(out)) => Some(out.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn a_click_finishes_the_line_then_goes_on() {
        let mut rig = Rig::new();
        assert!(!rig.widget.hit_self, "hidden, it lets clicks through");
        rig.send(DialoguePresenterMessage::ShowLine {
            beat: beat("mira", Side::Right, "Hello there."),
            display: None,
        });
        assert!(rig.widget.hit_self);
        let click = WidgetMessage::MouseDown {
            pos: Vec2::new(960.0, 950.0),
            button: MouseButton::Left,
            mods: Default::default(),
        };
        assert!(
            rig.send(click.clone()).is_empty(),
            "the first click reveals the whole line"
        );
        assert!(rig.presenter.model.line_complete());
        assert_eq!(rig.send(click), [PresenterOutput::Next]);
    }

    #[test]
    fn choices_take_clicks_number_keys_and_arrows() {
        let mut rig = Rig::new();
        rig.send(DialoguePresenterMessage::ShowLine {
            beat: beat("mira", Side::Right, "Well?"),
            display: None,
        });
        rig.send(DialoguePresenterMessage::ShowChoices(vec![
            choice(0, true),
            choice(1, false),
            choice(2, true),
        ]));
        let layout = rig.presenter.layout(&rig.widget);
        assert_eq!(layout.choices.len(), 3);
        assert!(
            layout.choices[2].y + layout.choices[2].h < layout.dialogue_box.y,
            "choices sit above the box"
        );
        let key = |k| WidgetMessage::KeyDown(k, Default::default());
        assert!(rig.send(key(KeyCode::Digit2)).is_empty(), "a locked choice");
        rig.send(key(KeyCode::ArrowDown));
        assert_eq!(
            rig.presenter.model.focus(),
            Some(2),
            "arrows skip the locked one"
        );
        let centre = layout.choices[0].pos() + layout.choices[0].size() * 0.5;
        rig.send(WidgetMessage::MouseMove {
            pos: centre,
            mods: Default::default(),
        });
        assert_eq!(rig.presenter.hover, Some(0));
        assert_eq!(
            rig.send(WidgetMessage::MouseDown {
                pos: centre,
                button: MouseButton::Left,
                mods: Default::default()
            }),
            [PresenterOutput::Chose(0)]
        );
    }

    #[test]
    fn ticks_type_the_line_and_buttons_toggle_auto_and_the_log() {
        let mut rig = Rig::new();
        rig.send(DialoguePresenterMessage::ShowLine {
            beat: beat("mira", Side::Left, "abcdef"),
            display: Some("[b]abc[/b]def".into()),
        });
        rig.send(DialoguePresenterMessage::Tick(250));
        assert_eq!(rig.presenter.model.revealed(), 2);
        let layout = rig.presenter.layout(&rig.widget);
        let press = |rect: Rect| WidgetMessage::MouseDown {
            pos: rect.pos() + Vec2::splat(4.0),
            button: MouseButton::Left,
            mods: Default::default(),
        };
        rig.send(press(layout.buttons[0]));
        assert!(rig.presenter.model.settings.auto);
        rig.send(press(layout.buttons[2]));
        assert!(rig.presenter.model.history_open());
        rig.send(WidgetMessage::KeyDown(KeyCode::Escape, Default::default()));
        assert!(!rig.presenter.model.history_open());
        rig.send(DialoguePresenterMessage::Close);
        assert!(!rig.widget.hit_self);
    }

    #[test]
    fn the_layout_scales_with_the_canvas() {
        let small = PresenterLayout::new(Rect::new(0.0, 0.0, 1280.0, 720.0), 2);
        let large = PresenterLayout::new(Rect::new(0.0, 0.0, 2560.0, 1440.0), 2);
        assert!((large.dialogue_box.h / small.dialogue_box.h - 2.0).abs() < 0.01);
        assert!(small.portraits[0].x < small.portraits[1].x);
        assert!(small.buttons[0].x < small.buttons[1].x && small.buttons[1].x < small.buttons[2].x);
    }
}
