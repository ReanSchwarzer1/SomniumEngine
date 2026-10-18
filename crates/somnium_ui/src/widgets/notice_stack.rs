//! A game's notices: short messages that type themselves on, hold, and fade, a few at a time, the rest queued.
//! Rimeborn's Voice ("Confirmed. Weave acquired: Frost Step.") is one; a quest update or a skill rank is another.
//!
//! The game drives it in game time: [`NoticeMessage::Tick`] each frame it is not paused, [`NoticeMessage::Push`]
//! when something happens. It answers with [`NoticeMessage::Output`]: `Shown` when a notice comes up (play the
//! chime, add it to the journal's log) and `Done` when it has gone. Notices never take the pointer or focus.
//!
//! Text is markup (`[b]`, `[color=]`, `[face=]`), so a game's own styling, a fluency transform, comes through.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    primitive::Primitive,
    text::rich::{self, RichStyle},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use std::collections::{HashMap, VecDeque};

/// One notice.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    /// Picks the accent colour ([`NoticeMessage::SetCategory`]); a game's own kinds: `voice`, `quest`, `skill`.
    pub category: String,
    /// A small line above the text ("WEAVE ACQUIRED"); may be empty.
    pub heading: String,
    /// The message, as markup.
    pub text: String,
    /// How long it stays once typed; `None` for a time by its length.
    pub hold_ms: Option<u64>,
}

/// What the stack tells the game.
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeEvent {
    /// The `id`th notice pushed (counting from 0) came up.
    Shown(u64, Notice),
    /// It has gone.
    Done(u64),
}

/// Messages to and from the stack.
#[derive(Clone, Debug)]
pub enum NoticeMessage {
    /// Queue a notice; it shows as soon as there is room.
    Push(Notice),
    /// Game milliseconds since the last frame.
    Tick(u64),
    /// Send the oldest notice on screen away now.
    Dismiss,
    /// Drop everything, shown and queued, without `Done`s (a load, a scene cut).
    Clear,
    /// The accent colour for a category.
    SetCategory(String, [u8; 4]),
    /// How many show at once (at least one) and how fast they type (0: at once).
    SetPace {
        visible: usize,
        chars_per_second: f32,
    },
    /// From the widget.
    Output(NoticeEvent),
}

const ENTER_MS: u64 = 180;
const EXIT_MS: u64 = 400;

struct Showing {
    id: u64,
    notice: Notice,
    age_ms: u64,
    chars: usize,
}

impl Showing {
    fn reveal_ms(&self, chars_per_second: f32) -> u64 {
        if chars_per_second <= 0.0 {
            0
        } else {
            (self.chars as f32 * 1000.0 / chars_per_second) as u64
        }
    }

    fn exit_at(&self, chars_per_second: f32) -> u64 {
        let hold = self.notice.hold_ms.unwrap_or(2400 + 35 * self.chars as u64);
        self.reveal_ms(chars_per_second) + hold
    }
}

/// The widget.
pub struct NoticeStack {
    font_id: u8,
    visible: usize,
    chars_per_second: f32,
    categories: HashMap<String, [u8; 4]>,
    showing: Vec<Showing>,
    waiting: VecDeque<(u64, Notice)>,
    next_id: u64,
    clock_ms: u64,
}

impl NoticeStack {
    fn plain(text: &str) -> String {
        crate::text::markup::parse(text).map_or_else(|_| text.to_owned(), |(plain, _)| plain)
    }

    /// Move time on, retire what has gone and bring up what fits; the events in order.
    fn step(&mut self, ms: u64) -> Vec<NoticeEvent> {
        let mut events = Vec::new();
        let cps = self.chars_per_second;
        for showing in &mut self.showing {
            showing.age_ms = showing.age_ms.saturating_add(ms);
        }
        self.showing.retain(|s| {
            let gone = s.age_ms >= s.exit_at(cps) + EXIT_MS;
            if gone {
                events.push(NoticeEvent::Done(s.id));
            }
            !gone
        });
        while self.showing.len() < self.visible.max(1) {
            let Some((id, notice)) = self.waiting.pop_front() else {
                break;
            };
            let chars = Self::plain(&notice.text).chars().count();
            events.push(NoticeEvent::Shown(id, notice.clone()));
            self.showing.push(Showing {
                id,
                notice,
                age_ms: 0,
                chars,
            });
        }
        events
    }

    /// `0..=1` opacity of a notice: in over [`ENTER_MS`], out over [`EXIT_MS`].
    fn opacity(&self, s: &Showing) -> f32 {
        let exit = s.exit_at(self.chars_per_second);
        let enter = (s.age_ms as f32 / ENTER_MS as f32).min(1.0);
        let leave = 1.0 - (s.age_ms.saturating_sub(exit) as f32 / EXIT_MS as f32).min(1.0);
        enter.min(leave)
    }

    fn revealed(&self, s: &Showing) -> usize {
        let reveal = s.reveal_ms(self.chars_per_second);
        if reveal == 0 || s.age_ms >= reveal {
            usize::MAX
        } else {
            (s.age_ms as f32 / 1000.0 * self.chars_per_second) as usize
        }
    }
}

impl Control for NoticeStack {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Alert
    }

    fn a11y_name(&self) -> Option<String> {
        self.showing.last().map(|s| {
            let text = Self::plain(&s.notice.text);
            if s.notice.heading.is_empty() {
                text
            } else {
                format!("{}: {text}", s.notice.heading)
            }
        })
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let bounds = widget.screen_bounds();
        let unit = (bounds.h / 1080.0).max(0.25);
        let theme = theme::active();
        let width = (760.0 * unit).min(bounds.w - 32.0 * unit);
        let x = bounds.x + (bounds.w - width) * 0.5;
        let mut y = bounds.y + 56.0 * unit;
        let pad = 22.0 * unit;
        for s in &self.showing {
            let alpha = (self.opacity(s) * 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let accent = self
                .categories
                .get(&s.notice.category)
                .copied()
                .unwrap_or(theme.semantic.accent.default.bytes());
            let (plain, runs) = crate::text::markup::parse(&s.notice.text)
                .unwrap_or_else(|_| (s.notice.text.clone(), Vec::new()));
            let style = RichStyle {
                font: self.font_id,
                px: 25.0 * unit,
                color: theme.semantic.text.primary.bytes(),
                bold: None,
                italic: None,
                faces: Vec::new(),
                leading: 1.4,
            };
            let font_atlas = &ctx.font_atlas;
            let laid = rich::layout(&plain, &runs, width - pad * 2.0, &style, |t, px, font| {
                font_atlas.measure_text(t, px, font).x
            });
            let heading_h = if s.notice.heading.is_empty() {
                0.0
            } else {
                26.0 * unit
            };
            let h = pad * 2.0 + heading_h + laid.height();
            // Slides down a little as it arrives.
            let drop = (1.0 - (s.age_ms as f32 / ENTER_MS as f32).min(1.0)) * -10.0 * unit;
            let card = Rect::new(x, y + drop, width, h);
            ctx.push_primitive(
                Primitive::fill(
                    card,
                    theme::with_alpha(theme.semantic.surface.canvas.bytes(), alpha.min(226)),
                )
                .with_radius(10.0 * unit)
                .with_border(1.5 * unit, theme::with_alpha(accent, alpha / 2)),
                None,
            );
            ctx.push_rect_filled(
                Rect::new(
                    card.x,
                    card.y + 10.0 * unit,
                    4.0 * unit,
                    card.h - 20.0 * unit,
                ),
                theme::with_alpha(accent, alpha),
            );
            if heading_h > 0.0 {
                ctx.push_text(
                    &s.notice.heading,
                    Vec2::new(card.x + pad, card.y + pad - 2.0 * unit),
                    self.font_id,
                    16.0 * unit,
                    theme::with_alpha(accent, alpha),
                );
            }
            ctx.push_clip_rect(card);
            rich::draw(
                &laid,
                ctx,
                Vec2::new(card.x + pad, card.y + pad + heading_h),
                self.revealed(s),
                self.clock_ms as f32 / 1000.0,
                alpha,
            );
            ctx.pop_clip_rect();
            y += h + 12.0 * unit;
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if msg.direction != MessageDirection::ToWidget {
            return;
        }
        let Some(message) = msg.data::<NoticeMessage>().cloned() else {
            return;
        };
        let events = match message {
            NoticeMessage::Push(notice) => {
                self.waiting.push_back((self.next_id, notice));
                self.next_id += 1;
                self.step(0)
            }
            NoticeMessage::Tick(ms) => {
                self.clock_ms = self.clock_ms.wrapping_add(ms);
                self.step(ms)
            }
            NoticeMessage::Dismiss => {
                let cps = self.chars_per_second;
                if let Some(s) = self.showing.iter_mut().find(|s| s.age_ms < s.exit_at(cps)) {
                    s.age_ms = s.exit_at(cps);
                }
                Vec::new()
            }
            NoticeMessage::Clear => {
                self.showing.clear();
                self.waiting.clear();
                Vec::new()
            }
            NoticeMessage::SetCategory(name, colour) => {
                self.categories.insert(name, colour);
                Vec::new()
            }
            NoticeMessage::SetPace {
                visible,
                chars_per_second,
            } => {
                self.visible = visible.max(1);
                self.chars_per_second = chars_per_second.max(0.0);
                self.step(0)
            }
            NoticeMessage::Output(_) => return,
        };
        msg.handled = true;
        emit.extend(events.into_iter().map(|event| {
            UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                NoticeMessage::Output(event),
            )
        }));
    }
}

/// Builds a [`NoticeStack`], normally stretched over a whole screen canvas.
pub struct NoticeStackBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl NoticeStackBuilder {
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
        UiNode::new(
            widget,
            Box::new(NoticeStack {
                font_id: self.font_id,
                visible: 3,
                chars_per_second: 60.0,
                categories: HashMap::new(),
                showing: Vec::new(),
                waiting: VecDeque::new(),
                next_id: 0,
                clock_ms: 0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack() -> (NoticeStack, Widget) {
        let mut widget = WidgetBuilder::new().build();
        widget.actual_local_size = Vec2::new(1920.0, 1080.0);
        let stack = NoticeStack {
            font_id: 0,
            visible: 2,
            chars_per_second: 10.0,
            categories: HashMap::new(),
            showing: Vec::new(),
            waiting: VecDeque::new(),
            next_id: 0,
            clock_ms: 0,
        };
        (stack, widget)
    }

    fn send(
        stack: &mut NoticeStack,
        widget: &mut Widget,
        message: NoticeMessage,
    ) -> Vec<NoticeEvent> {
        let mut msg = UiMessage::new(widget.handle, MessageDirection::ToWidget, message);
        let mut emit = Vec::new();
        stack.handle_routed_message(widget, &mut msg, &mut emit);
        emit.iter()
            .filter_map(|m| match m.data::<NoticeMessage>() {
                Some(NoticeMessage::Output(event)) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    fn notice(text: &str, hold_ms: u64) -> NoticeMessage {
        NoticeMessage::Push(Notice {
            category: "voice".into(),
            heading: String::new(),
            text: text.into(),
            hold_ms: Some(hold_ms),
        })
    }

    #[test]
    fn notices_queue_type_on_hold_and_go_in_order() {
        let (mut s, mut w) = stack();
        let shown = |events: &[NoticeEvent]| -> Vec<u64> {
            events
                .iter()
                .filter_map(|e| match e {
                    NoticeEvent::Shown(id, _) => Some(*id),
                    NoticeEvent::Done(_) => None,
                })
                .collect()
        };
        assert_eq!(
            shown(&send(&mut s, &mut w, notice("[b]Confirmed.[/b]", 1000))),
            [0]
        );
        assert_eq!(shown(&send(&mut s, &mut w, notice("Second.", 5000))), [1]);
        assert!(
            send(&mut s, &mut w, notice("Third.", 1000)).is_empty(),
            "two at a time; the third waits"
        );
        // "Confirmed." is 10 characters: a second to type at 10 a second.
        send(&mut s, &mut w, NoticeMessage::Tick(500));
        assert_eq!(s.revealed(&s.showing[0]), 5);
        send(&mut s, &mut w, NoticeMessage::Tick(1500));
        assert_eq!(s.revealed(&s.showing[0]), usize::MAX);
        assert!(s.opacity(&s.showing[0]) == 1.0);
        send(&mut s, &mut w, NoticeMessage::Tick(200));
        assert!(s.opacity(&s.showing[0]) < 1.0, "fading after its hold");
        let events = send(&mut s, &mut w, NoticeMessage::Tick(300));
        assert_eq!(events[0], NoticeEvent::Done(0));
        assert_eq!(shown(&events), [2], "the queued one takes its place");
        send(&mut s, &mut w, NoticeMessage::Dismiss);
        let events = send(&mut s, &mut w, NoticeMessage::Tick(EXIT_MS));
        assert_eq!(
            events,
            [NoticeEvent::Done(1)],
            "dismissed, it fades out at once"
        );
        assert_eq!(s.a11y_name().as_deref(), Some("Third."));
        send(&mut s, &mut w, NoticeMessage::Clear);
        assert!(s.showing.is_empty() && s.waiting.is_empty());
    }
}
