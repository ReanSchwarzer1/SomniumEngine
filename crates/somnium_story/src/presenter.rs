//! The presentation model behind a light-novel dialogue box: typewriter, inline cues, auto, skip, history and
//! choice focus, with no drawing in it.
//!
//! `somnium_ui`'s dialogue presenter widget draws this model and turns mouse and keys into [`PresenterInput`]; a
//! game can drive it from anything else (a controller, a test). The model never touches the [`crate::Story`]:
//! it says what the player asked for ([`PresenterOutput::Next`], [`PresenterOutput::Chose`]) and the game moves
//! the [`crate::Conversation`], so the same model can sit over a conversation, a cutscene's captions or a radio.
//!
//! # Inline cues
//!
//! A line may carry presentation cues the writer places by character, removed before display:
//!
//! - `[expr=angry]` swaps the speaker's portrait expression when the typewriter reaches it;
//! - `[pause=400]` holds the typewriter for that many milliseconds;
//! - `[speed=0.5]` scales the typewriter from there (1 is normal).
//!
//! Positions count visible characters, the same way the engine's rich-text markup does: tags (`[b]`, `[color=..]`)
//! take no room, `[[` is one `[`, a `[sprite=..]` is one.

use crate::conversation::{ChoiceBeat, LineBeat};
use crate::manifest::Side;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// When skip moves past lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipMode {
    #[default]
    Off,
    /// Only lines the player has read before.
    Seen,
    All,
}

/// Player-facing presentation options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresenterSettings {
    /// Typewriter speed. 0 shows each line whole at once (the accessibility "instant text").
    pub chars_per_second: f32,
    pub auto: bool,
    /// Auto mode waits this long after a line is fully shown, or after its voice ends...
    pub auto_delay_ms: u64,
    /// ...plus, for an unvoiced line, this much per visible character, so long lines stay up longer.
    pub auto_ms_per_char: u64,
    pub skip: SkipMode,
    /// How long a skipped line stays up.
    pub skip_line_ms: u64,
    pub history_limit: usize,
}

impl Default for PresenterSettings {
    fn default() -> Self {
        Self {
            chars_per_second: 45.0,
            auto: false,
            auto_delay_ms: 900,
            auto_ms_per_char: 35,
            skip: SkipMode::Off,
            skip_line_ms: 70,
            history_limit: 200,
        }
    }
}

/// A portrait on screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Portrait {
    pub speaker: String,
    pub portrait: String,
    pub expression: String,
    /// The one speaking; the other is drawn dimmed.
    pub active: bool,
}

/// One line in the history log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryEntry {
    pub speaker: String,
    pub name: String,
    /// Display text (markup kept, cues removed).
    pub text: String,
    /// A choice the player made, rather than a line.
    pub choice: bool,
}

/// What the player did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenterInput {
    /// Click, Space or Enter on the box: finish the line, or go on.
    Advance,
    /// A number key or a click on a choice (0-based).
    Choose(usize),
    FocusNext,
    FocusPrevious,
    /// Enter while choices are up: take the focused one.
    Confirm,
    /// The pointer is over a choice, or over none.
    Hover(Option<usize>),
    ToggleAuto,
    ToggleSkip,
    ToggleHistory,
    /// Escape: close the history, stop skipping.
    Back,
}

/// What the game should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresenterOutput {
    /// Move the conversation past the line.
    Next,
    /// Take this choice (0-based).
    Chose(usize),
    /// Play this voice clip; call [`Presenter::voice_finished`] when it ends.
    PlayVoice(String),
    StopVoice,
    /// `Back` with nothing to close or stop: the player wants out (a game's pause menu, an editor's preview).
    Back,
}

/// A cue inside a line.
#[derive(Clone, Debug, PartialEq)]
pub enum Cue {
    Expression(String),
    PauseMs(u64),
    Speed(f32),
}

/// A line split into display text and positioned cues.
#[derive(Clone, Debug, PartialEq)]
pub struct CuedText {
    /// The text without cues; markup intact.
    pub display: String,
    /// `(visible character index, cue)`, in order.
    pub cues: Vec<(usize, Cue)>,
    /// Visible characters in `display`.
    pub visible_len: usize,
}

/// Split cues out of `text`.
#[must_use]
pub fn parse_cues(text: &str) -> CuedText {
    let mut display = String::with_capacity(text.len());
    let mut cues = Vec::new();
    let mut visible = 0usize;
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        if rest.starts_with("[[") {
            display.push_str("[[");
            visible += 1;
            rest = &rest[2..];
            continue;
        }
        if ch == '['
            && let Some(close) = rest.find(']')
        {
            let tag = &rest[1..close];
            let (name, value) = tag.split_once('=').unwrap_or((tag, ""));
            let cue = match name.trim() {
                "expr" => Some(Cue::Expression(value.trim().to_owned())),
                "pause" => value.trim().parse().ok().map(Cue::PauseMs),
                "speed" => value
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .map(Cue::Speed),
                _ => None,
            };
            if let Some(cue) = cue {
                cues.push((visible, cue));
                rest = &rest[close + 1..];
                continue;
            }
            let bare = name.trim().trim_start_matches('/');
            if is_markup_tag(bare) {
                display.push_str(&rest[..=close]);
                if bare == "sprite" && !name.trim().starts_with('/') {
                    visible += 1;
                }
                rest = &rest[close + 1..];
                continue;
            }
        }
        display.push(ch);
        visible += 1;
        rest = &rest[ch.len_utf8()..];
    }
    CuedText {
        display,
        cues,
        visible_len: visible,
    }
}

/// The engine's rich-text vocabulary (`somnium_ui::text::markup`); an unknown tag shows as text there and here.
fn is_markup_tag(name: &str) -> bool {
    matches!(
        name,
        "b" | "i"
            | "u"
            | "s"
            | "color"
            | "size"
            | "font"
            | "face"
            | "link"
            | "wave"
            | "shake"
            | "sprite"
    )
}

#[derive(Clone, Debug)]
struct Shown {
    beat: LineBeat,
    text: CuedText,
    revealed: f32,
    next_cue: usize,
    pause_ms: u64,
    speed: f32,
    /// Time since the line was fully shown, or since its voice ended.
    held_ms: u64,
    voiced: bool,
    expression: String,
}

/// The model.
#[derive(Clone, Debug)]
pub struct Presenter {
    pub settings: PresenterSettings,
    line: Option<Shown>,
    choices: Vec<ChoiceBeat>,
    focus: Option<usize>,
    time_left: Option<(u64, u64)>,
    portraits: [Option<Portrait>; 2],
    history: VecDeque<HistoryEntry>,
    history_open: bool,
    voice_playing: bool,
    waiting_for_next: bool,
    outputs: Vec<PresenterOutput>,
    visible: bool,
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new(PresenterSettings::default())
    }
}

impl Presenter {
    #[must_use]
    pub fn new(settings: PresenterSettings) -> Self {
        Self {
            settings,
            line: None,
            choices: Vec::new(),
            focus: None,
            time_left: None,
            portraits: [None, None],
            history: VecDeque::new(),
            history_open: false,
            voice_playing: false,
            waiting_for_next: false,
            outputs: Vec::new(),
            visible: false,
        }
    }

    // --- what the game shows -----------------------------------------------------------------------------

    /// Show a line. `display` replaces the beat's text when the game transforms it first (Rimeborn's fluency:
    /// unknown words in their script); `None` uses the beat's own.
    pub fn show_line(&mut self, beat: LineBeat, display: Option<String>) {
        self.stop_voice();
        let text = parse_cues(display.as_deref().unwrap_or(&beat.text));
        let expression = beat.expression.clone();
        if !beat.portrait.is_empty() {
            let slot = match beat.side {
                Side::Left => 0,
                Side::Right => 1,
            };
            for (index, portrait) in self.portraits.iter_mut().enumerate() {
                if let Some(p) = portrait {
                    p.active = false;
                    if index != slot && p.speaker == beat.speaker {
                        *portrait = None;
                    }
                }
            }
            self.portraits[slot] = Some(Portrait {
                speaker: beat.speaker.clone(),
                portrait: beat.portrait.clone(),
                expression: expression.clone(),
                active: true,
            });
        } else {
            for portrait in self.portraits.iter_mut().flatten() {
                portrait.active = false;
            }
        }
        self.push_history(HistoryEntry {
            speaker: beat.speaker.clone(),
            name: beat.name.clone(),
            text: text.display.clone(),
            choice: false,
        });
        let voiced = !beat.voice.is_empty() && !self.skipping_line(&beat);
        if voiced {
            self.voice_playing = true;
            self.outputs
                .push(PresenterOutput::PlayVoice(beat.voice.clone()));
        }
        let instant = self.settings.chars_per_second <= 0.0 || self.skipping_line(&beat);
        let mut shown = Shown {
            revealed: 0.0,
            next_cue: 0,
            pause_ms: 0,
            speed: 1.0,
            held_ms: 0,
            voiced,
            expression,
            beat,
            text,
        };
        if instant {
            Self::reveal_all(&mut shown);
        } else {
            Self::apply_cues(&mut shown);
        }
        self.sync_expression(&shown);
        self.line = Some(shown);
        self.choices.clear();
        self.focus = None;
        self.time_left = None;
        self.waiting_for_next = false;
        self.visible = true;
    }

    /// Show a menu. The last line stays on screen above it.
    pub fn show_choices(&mut self, choices: Vec<ChoiceBeat>) {
        if let Some(line) = &mut self.line {
            Self::reveal_all(line);
        }
        self.focus = choices.iter().position(|c| c.available);
        self.choices = choices;
        self.waiting_for_next = false;
        self.visible = true;
    }

    /// A timed menu's clock: `(left, total)` milliseconds, from `Conversation::time_left_ms`.
    pub fn set_time_left(&mut self, time: Option<(u64, u64)>) {
        self.time_left = time;
    }

    /// The conversation ended: clear the box.
    pub fn close(&mut self) {
        self.stop_voice();
        self.line = None;
        self.choices.clear();
        self.focus = None;
        self.portraits = [None, None];
        self.history_open = false;
        self.visible = false;
        self.waiting_for_next = false;
    }

    /// The voice clip from [`PresenterOutput::PlayVoice`] ended.
    pub fn voice_finished(&mut self) {
        self.voice_playing = false;
        if let Some(line) = &mut self.line {
            line.held_ms = 0;
        }
    }

    // --- time and input ----------------------------------------------------------------------------------

    /// Advance the typewriter, cues, auto and skip by `ms` of real time (not game time: reading is the player's).
    pub fn update(&mut self, ms: u64) {
        let cps = self.settings.chars_per_second;
        let Some(line) = &mut self.line else { return };
        if !self.choices.is_empty() || self.waiting_for_next || self.history_open {
            return;
        }
        if (line.revealed as usize) < line.text.visible_len {
            let mut budget = ms as f32;
            while budget > 0.0 && (line.revealed as usize) < line.text.visible_len {
                if line.pause_ms > 0 {
                    let used = (line.pause_ms as f32).min(budget);
                    line.pause_ms -= used as u64;
                    budget -= used;
                    continue;
                }
                let per_char = 1000.0 / (cps * line.speed).max(0.001);
                let next_stop = line
                    .text
                    .cues
                    .get(line.next_cue)
                    .map_or(line.text.visible_len, |(at, _)| *at)
                    .max(line.revealed.floor() as usize);
                let chars_to_stop = next_stop as f32 - line.revealed;
                let time_to_stop = chars_to_stop * per_char;
                if chars_to_stop > 0.0 && budget < time_to_stop {
                    line.revealed += budget / per_char;
                    budget = 0.0;
                } else {
                    line.revealed = next_stop as f32;
                    budget -= time_to_stop.max(0.0);
                    if !Self::apply_cues(line) && next_stop >= line.text.visible_len {
                        break;
                    }
                }
            }
            let line = self.line.clone().expect("checked above");
            self.sync_expression(&line);
            return;
        }
        line.held_ms = line.held_ms.saturating_add(ms);
        let skipping = match self.settings.skip {
            SkipMode::Off => false,
            SkipMode::Seen => line.beat.seen,
            SkipMode::All => true,
        };
        let auto = self.settings.auto || line.beat.ambient;
        let wait = if skipping {
            Some(self.settings.skip_line_ms)
        } else if auto && !self.voice_playing {
            let reading = if line.voiced {
                0
            } else {
                self.settings.auto_ms_per_char * line.text.visible_len as u64
            };
            Some(self.settings.auto_delay_ms + reading)
        } else {
            None
        };
        if wait.is_some_and(|wait| line.held_ms >= wait) && !line.beat.menu_next {
            self.request_next();
        } else if skipping && line.beat.menu_next {
            // Skip stops at a menu: the player has to choose.
            self.settings.skip = SkipMode::Off;
        }
    }

    /// Handle one input.
    pub fn input(&mut self, input: PresenterInput) {
        match input {
            PresenterInput::Back => {
                if !self.history_open && self.settings.skip == SkipMode::Off {
                    self.outputs.push(PresenterOutput::Back);
                }
                self.history_open = false;
                self.settings.skip = SkipMode::Off;
            }
            PresenterInput::ToggleHistory => self.history_open = !self.history_open,
            PresenterInput::ToggleAuto => self.settings.auto = !self.settings.auto,
            PresenterInput::ToggleSkip => {
                self.settings.skip = if self.settings.skip == SkipMode::Off {
                    SkipMode::Seen
                } else {
                    SkipMode::Off
                };
            }
            _ if self.history_open => {
                if input == PresenterInput::Advance {
                    self.history_open = false;
                }
            }
            PresenterInput::Advance => {
                if !self.choices.is_empty() {
                    return;
                }
                let Some(line) = &mut self.line else { return };
                if (line.revealed as usize) < line.text.visible_len {
                    Self::reveal_all(line);
                    let line = line.clone();
                    self.sync_expression(&line);
                } else if !line.beat.menu_next {
                    self.request_next();
                }
            }
            PresenterInput::Choose(index) => {
                let Some(choice) = self.choices.get(index) else {
                    return;
                };
                if !choice.available {
                    return;
                }
                self.push_history(HistoryEntry {
                    speaker: String::new(),
                    name: String::new(),
                    text: choice.text.clone(),
                    choice: true,
                });
                self.outputs.push(PresenterOutput::Chose(index));
                self.stop_voice();
                self.choices.clear();
                self.focus = None;
                self.time_left = None;
                self.waiting_for_next = true;
            }
            PresenterInput::Confirm => {
                if let Some(focus) = self.focus.filter(|_| !self.choices.is_empty()) {
                    self.input(PresenterInput::Choose(focus));
                } else {
                    self.input(PresenterInput::Advance);
                }
            }
            PresenterInput::FocusNext | PresenterInput::FocusPrevious => {
                let available: Vec<usize> = self
                    .choices
                    .iter()
                    .filter(|c| c.available)
                    .map(|c| c.index)
                    .collect();
                if available.is_empty() {
                    return;
                }
                let at = self
                    .focus
                    .and_then(|f| available.iter().position(|i| *i == f));
                let next = match (at, input == PresenterInput::FocusNext) {
                    (None, _) => 0,
                    (Some(at), true) => (at + 1) % available.len(),
                    (Some(at), false) => (at + available.len() - 1) % available.len(),
                };
                self.focus = Some(available[next]);
            }
            PresenterInput::Hover(index) => {
                if let Some(index) = index
                    && self.choices.get(index).is_some_and(|c| c.available)
                {
                    self.focus = Some(index);
                }
            }
        }
    }

    /// What the game should do, since the last drain.
    pub fn drain(&mut self) -> Vec<PresenterOutput> {
        std::mem::take(&mut self.outputs)
    }

    // --- what the widget draws ---------------------------------------------------------------------------

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub fn line(&self) -> Option<&LineBeat> {
        self.line.as_ref().map(|l| &l.beat)
    }

    /// The display text (cues removed, markup kept).
    #[must_use]
    pub fn display_text(&self) -> &str {
        self.line.as_ref().map_or("", |l| l.text.display.as_str())
    }

    /// How many visible characters are shown so far.
    #[must_use]
    pub fn revealed(&self) -> usize {
        self.line
            .as_ref()
            .map_or(0, |l| (l.revealed as usize).min(l.text.visible_len))
    }

    /// Whether the line is fully shown (the "next" arrow can show).
    #[must_use]
    pub fn line_complete(&self) -> bool {
        self.line
            .as_ref()
            .is_some_and(|l| l.revealed as usize >= l.text.visible_len)
    }

    #[must_use]
    pub fn choices(&self) -> &[ChoiceBeat] {
        &self.choices
    }

    #[must_use]
    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    #[must_use]
    pub fn time_left(&self) -> Option<(u64, u64)> {
        self.time_left
    }

    /// `[left, right]`.
    #[must_use]
    pub fn portraits(&self) -> &[Option<Portrait>; 2] {
        &self.portraits
    }

    #[must_use]
    pub fn history(&self) -> &VecDeque<HistoryEntry> {
        &self.history
    }

    #[must_use]
    pub fn history_open(&self) -> bool {
        self.history_open
    }

    // --- inside ------------------------------------------------------------------------------------------

    fn skipping_line(&self, beat: &LineBeat) -> bool {
        match self.settings.skip {
            SkipMode::Off => false,
            SkipMode::Seen => beat.seen,
            SkipMode::All => true,
        }
    }

    fn request_next(&mut self) {
        if !self.waiting_for_next {
            self.waiting_for_next = true;
            self.stop_voice();
            self.outputs.push(PresenterOutput::Next);
        }
    }

    fn stop_voice(&mut self) {
        if self.voice_playing {
            self.voice_playing = false;
            self.outputs.push(PresenterOutput::StopVoice);
        }
    }

    fn push_history(&mut self, entry: HistoryEntry) {
        self.history.push_back(entry);
        while self.history.len() > self.settings.history_limit.max(1) {
            self.history.pop_front();
        }
    }

    /// Apply every cue at or before the reveal point. Returns whether any was applied.
    fn apply_cues(line: &mut Shown) -> bool {
        let mut applied = false;
        while let Some((at, cue)) = line.text.cues.get(line.next_cue) {
            if *at > line.revealed as usize {
                break;
            }
            match cue {
                Cue::Expression(expression) => line.expression.clone_from(expression),
                Cue::PauseMs(ms) => line.pause_ms = *ms,
                Cue::Speed(speed) => line.speed = *speed,
            }
            line.next_cue += 1;
            applied = true;
        }
        applied
    }

    fn reveal_all(line: &mut Shown) {
        line.revealed = line.text.visible_len as f32;
        Self::apply_cues(line);
        line.pause_ms = 0;
    }

    fn sync_expression(&mut self, line: &Shown) {
        for portrait in self.portraits.iter_mut().flatten() {
            if portrait.active && portrait.speaker == line.beat.speaker {
                portrait.expression.clone_from(&line.expression);
            }
        }
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
            color: None,
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

    #[test]
    fn cues_are_positioned_by_visible_characters() {
        let cued = parse_cues(
            "[b]Hi[/b], [expr=angry]you.[pause=300] [[ok] [sprite=coin]![speed=2]x [blink]",
        );
        assert_eq!(
            cued.display,
            "[b]Hi[/b], you. [[ok] [sprite=coin]!x [blink]"
        );
        assert_eq!(
            cued.cues,
            vec![
                (4, Cue::Expression("angry".into())),
                (8, Cue::PauseMs(300)),
                (16, Cue::Speed(2.0))
            ]
        );
        // "Hi, you. [ok] " is 14, the sprite 1, "!" 1, "x " 2, "[blink]" 7.
        assert_eq!(cued.visible_len, 25);
    }

    #[test]
    fn the_typewriter_reveals_and_honours_pauses_and_expressions() {
        let mut presenter = Presenter::new(PresenterSettings {
            chars_per_second: 10.0,
            ..PresenterSettings::default()
        });
        presenter.show_line(
            beat("mireille", Side::Right, "abc[expr=angry][pause=500]def"),
            None,
        );
        assert_eq!(presenter.revealed(), 0);
        presenter.update(250);
        assert_eq!(presenter.revealed(), 2);
        presenter.update(100);
        assert_eq!(presenter.revealed(), 3, "reached the cue");
        assert_eq!(
            presenter.portraits()[1].as_ref().unwrap().expression,
            "angry"
        );
        presenter.update(400);
        assert_eq!(presenter.revealed(), 3, "still paused");
        presenter.update(200);
        assert_eq!(presenter.revealed(), 4);
        presenter.input(PresenterInput::Advance);
        assert!(
            presenter.line_complete(),
            "the first advance finishes the line"
        );
        assert!(presenter.drain().is_empty());
        presenter.input(PresenterInput::Advance);
        assert_eq!(presenter.drain(), [PresenterOutput::Next]);
        presenter.input(PresenterInput::Advance);
        assert!(presenter.drain().is_empty(), "one Next per line");
    }

    #[test]
    fn auto_waits_for_the_voice_and_reading_time() {
        let mut presenter = Presenter::new(PresenterSettings {
            chars_per_second: 0.0,
            auto: true,
            auto_delay_ms: 500,
            auto_ms_per_char: 10,
            ..PresenterSettings::default()
        });
        let mut line = beat("mireille", Side::Right, "0123456789");
        line.voice = "vo/m1.ogg".into();
        presenter.show_line(line, None);
        assert_eq!(
            presenter.drain(),
            [PresenterOutput::PlayVoice("vo/m1.ogg".into())]
        );
        presenter.update(5_000);
        assert!(presenter.drain().is_empty(), "the voice is still playing");
        presenter.voice_finished();
        presenter.update(450);
        assert!(
            presenter.drain().is_empty(),
            "the delay runs from the end of the voice"
        );
        presenter.update(60);
        assert_eq!(presenter.drain(), [PresenterOutput::Next]);
        presenter.show_line(beat("mireille", Side::Right, "0123456789"), None);
        presenter.update(550);
        assert!(
            presenter.drain().is_empty(),
            "unvoiced: 600 ms of reading time for ten characters"
        );
        presenter.update(60);
        assert_eq!(presenter.drain(), [PresenterOutput::Next]);
    }

    #[test]
    fn skip_seen_passes_read_lines_and_stops_at_new_ones_and_menus() {
        let mut presenter = Presenter::new(PresenterSettings {
            skip: SkipMode::Seen,
            ..PresenterSettings::default()
        });
        let mut seen = beat("caelan", Side::Left, "Old line.");
        seen.seen = true;
        presenter.show_line(seen.clone(), None);
        assert!(presenter.line_complete());
        presenter.update(100);
        assert_eq!(presenter.drain(), [PresenterOutput::Next]);
        presenter.show_line(beat("caelan", Side::Left, "New line."), None);
        presenter.update(100);
        assert!(
            presenter.drain().is_empty(),
            "an unread line is not skipped"
        );
        seen.menu_next = true;
        presenter.show_line(seen, None);
        presenter.update(100);
        assert!(presenter.drain().is_empty());
        assert_eq!(
            presenter.settings.skip,
            SkipMode::Off,
            "skip stops at a menu"
        );
    }

    #[test]
    fn choices_take_number_keys_arrows_and_skip_locked_ones() {
        let mut presenter = Presenter::default();
        presenter.show_line(beat("mireille", Side::Right, "Well?"), None);
        presenter.show_choices(vec![choice(0, false), choice(1, true), choice(2, true)]);
        assert_eq!(
            presenter.focus(),
            Some(1),
            "focus starts on the first that can be taken"
        );
        presenter.input(PresenterInput::FocusNext);
        assert_eq!(presenter.focus(), Some(2));
        presenter.input(PresenterInput::FocusNext);
        assert_eq!(presenter.focus(), Some(1), "wraps, skipping the locked one");
        presenter.input(PresenterInput::Choose(0));
        assert!(presenter.drain().is_empty(), "a locked choice is refused");
        presenter.input(PresenterInput::Hover(Some(2)));
        presenter.input(PresenterInput::Confirm);
        assert_eq!(presenter.drain(), [PresenterOutput::Chose(2)]);
        assert!(presenter.choices().is_empty());
        let last = presenter.history().back().unwrap();
        assert!(last.choice && last.text == "Choice 3");
    }

    #[test]
    fn portraits_take_their_sides_and_the_listener_dims() {
        let mut presenter = Presenter::default();
        presenter.show_line(beat("caelan", Side::Left, "a"), None);
        presenter.show_line(beat("mireille", Side::Right, "b"), None);
        let [left, right] = presenter.portraits().clone();
        assert!(!left.unwrap().active && right.unwrap().active);
        let mut narrator = beat("", Side::Right, "The Lid falls.");
        narrator.portrait.clear();
        presenter.show_line(narrator, None);
        assert!(presenter.portraits().iter().flatten().all(|p| !p.active));
        presenter.input(PresenterInput::ToggleHistory);
        assert!(presenter.history_open());
        presenter.input(PresenterInput::Advance);
        assert!(
            !presenter.history_open(),
            "advance closes the history first"
        );
        assert_eq!(presenter.history().len(), 3);
        presenter.drain();
        presenter.input(PresenterInput::ToggleHistory);
        presenter.input(PresenterInput::Back);
        assert!(
            presenter.drain().is_empty(),
            "Back closes the history first"
        );
        presenter.input(PresenterInput::Back);
        assert_eq!(
            presenter.drain(),
            [PresenterOutput::Back],
            "then asks to leave"
        );
        presenter.close();
        assert!(!presenter.is_visible() && presenter.portraits().iter().all(Option::is_none));
    }

    #[test]
    fn a_game_transform_replaces_the_display_text() {
        let mut presenter = Presenter::new(PresenterSettings {
            chars_per_second: 0.0,
            ..PresenterSettings::default()
        });
        presenter.show_line(
            beat("watch_guard", Side::Right, "Halt there."),
            Some("[font=ladder]Hlt thr[/font] there.".into()),
        );
        assert_eq!(
            presenter.display_text(),
            "[font=ladder]Hlt thr[/font] there."
        );
        assert_eq!(presenter.revealed(), 14);
    }
}
