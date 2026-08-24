//! Text: runs, markup, fallback, IME and localisation (MORROWIND-G).
//!
//! §8 calls this *"the largest single sub-phase in Track 1, and the one most
//! likely to be under-estimated"*. This module is the part of it that can be
//! built and proven without a GPU and without committing to a shaper; the
//! shaper decision is recorded below and its integration is deliberately
//! separate.
//!
//! # The run model, which is the thing that was missing
//!
//! `font.rs` rasterises glyphs keyed by `(char, px, font_id)` and `draw.rs`
//! walks a `&str` a `char` at a time, advancing by each glyph's advance. That
//! works for Latin at one size in one colour and cannot express anything else:
//! there is **no concept of a run**, so there is nowhere to put a colour change
//! mid-sentence, a fallback face for one codepoint, a shaped cluster whose
//! glyph count differs from its character count, or a bidi level.
//!
//! [`StyledRun`] is that missing concept. Everything else in this module either
//! produces runs ([`markup`]) or annotates them ([`fallback`]), and a shaper
//! consumes them. Introducing it first is what makes the shaper a *substitution*
//! rather than a rewrite.
//!
//! # The shaper decision
//!
//! §8 item 1 requires this sub-phase to decide between two candidates and say
//! why, because Phase 27 deferred `cosmic-text` once and the ecosystem moved
//! while it was deferred.
//!
//! **Decision: `cosmic-text`, behind `SOMNIUM_UI_SHAPER=1`, default off.**
//!
//! | | `cosmic-text` | `parley` |
//! |---|---|---|
//! | Shaping | `harfrust` — a Rust HarfBuzz | `swash` |
//! | Rasterising | `swash` | `swash` |
//! | Scope | Buffer, layout, editing, and an editor with selection and cursor movement | Layout and rich-text ranges; editing is the caller's |
//! | Fit to what exists here | Higher: `text_box.rs` needs selection and cursor movement and currently hand-rolls both | Lower: would leave that hand-rolled code in place |
//! | Risk | A larger surface to adopt, and its own font database | Smaller, but the editing gap stays open |
//!
//! `cosmic-text` wins on the second-to-last row. MORROWIND-G's item 5 is IME,
//! and IME is *editing*: composition, candidate windows, and a caret that moves
//! by cluster rather than by byte. A shaper that hands back positioned glyphs
//! and leaves editing to the caller means writing cluster-aware caret movement
//! by hand, which is the part of text handling that is hardest to get right and
//! easiest to get subtly wrong.
//!
//! **It is not adopted in this sub-phase, and that is Appendix A.5's
//! instruction, not a shortcut:**
//!
//! > *"Phase 27 froze block-origin text snapping to get crisp glyphs at 1x DPI.
//! > A shaper returns sub-pixel advances; naive snapping of shaped output
//! > destroys kerning, and naive non-snapping blurs the editor's own chrome.
//! > The resolution is to snap the *run origin* and keep advances sub-pixel
//! > within the run — **but that is a claim, not a result.** Land the shaper
//! > behind `SOMNIUM_UI_SHAPER=1`, A/B it, and only then flip the default.
//! > GHOSTFENCE's golden-image row is what makes the A/B decidable rather than
//! > a matter of opinion."*
//!
//! GHOSTFENCE's golden-image row has **no reference images yet**. Landing a
//! shaper now would mean flipping the most visually sensitive switch in the
//! editor with no way to tell whether the chrome got worse. So this sub-phase
//! builds the run model the shaper needs, records the choice, and stops there.
//! [`ShaperPolicy`] is the switch, and it reads the environment variable A.5
//! names.
//!
//! # What is deferred, and by whose decision
//!
//! - **Vertical writing modes.** §8 item 4 defers them explicitly ("bidi is in;
//!   vertical writing modes are explicitly deferred, §14.5"). Not attempted.
//! - **Bidi reordering.** [`Direction`] and paragraph-level direction are here
//!   because a run needs to carry a level for a shaper to use. The UAX #9
//!   resolution algorithm belongs with the shaper that consumes it — writing it
//!   against a pipeline that cannot reorder glyphs would produce levels nothing
//!   reads.

pub mod fallback;
pub mod ime;
pub mod localize;
pub mod markup;

pub use fallback::{FallbackChain, FaceCoverage};
pub use ime::{Composition, ImeEvent};
pub use localize::{LocalizedText, TextKey};
pub use markup::{MarkupError, parse};

use crate::typography::FontRole;

/// Writing direction for a paragraph or a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    /// Left to right. Latin, CJK, Devanagari.
    #[default]
    Ltr,
    /// Right to left. Arabic, Hebrew.
    Rtl,
}

impl Direction {
    /// Guess a paragraph's base direction from its first strong character.
    ///
    /// UAX #9's P2/P3 rule, reduced to the ranges that matter: scan for the
    /// first character with a strong direction and take it; default to LTR if
    /// there is none.
    ///
    /// This is a *heuristic for the base direction only*, not the bidi
    /// algorithm. It is here because a caller that knows the direction should
    /// say so and a caller that does not needs a defensible default — and
    /// "always LTR" mangles the first line of every Arabic UI.
    #[must_use]
    pub fn of_paragraph(text: &str) -> Self {
        for ch in text.chars() {
            match ch as u32 {
                // Hebrew, Arabic, Syriac, Thaana, N'Ko, Samaritan.
                0x0590..=0x08FF => return Self::Rtl,
                // Arabic Presentation Forms A and B.
                0xFB1D..=0xFDFF | 0xFE70..=0xFEFF => return Self::Rtl,
                // Latin, Greek, Cyrillic, Armenian; CJK; Hangul; Devanagari.
                0x0041..=0x05BF
                | 0x0900..=0x1FFF
                | 0x2C00..=0xD7FF
                | 0xF900..=0xFB17
                | 0xFF00..=0xFFEF => return Self::Ltr,
                _ => {}
            }
        }
        Self::Ltr
    }

    /// Whether this is right-to-left.
    #[must_use]
    pub fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// A decoration applied to a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decoration {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

/// A per-run animation, for the cases §8 names by name.
///
/// > *"colour, size, weight, style, inline sprite, link, and **wave/shake for
/// > damage numbers**."*
///
/// Carried on the run rather than applied by the caller because the effect is
/// per-glyph within a run — a wave that moves the whole run together is a
/// translation, not a wave — and only the text layer knows where the glyphs are.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Motion {
    #[default]
    None,
    /// Vertical sine, `amplitude` pixels, `frequency` cycles per second, with
    /// each glyph phase-shifted along the run.
    Wave { amplitude: f32, frequency: f32 },
    /// Random per-glyph jitter within `amplitude` pixels.
    Shake { amplitude: f32 },
}

/// One span of text with uniform style.
///
/// The unit a shaper shapes, a fallback chain annotates and the draw layer
/// emits. **`text` is a byte range into the source string, not a copy** — a
/// paragraph with a dozen colour changes should not allocate a dozen strings,
/// and a range keeps the run tied to the text it came from so a caret offset
/// means the same thing on both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledRun {
    /// Byte range into the source string. Always on a `char` boundary.
    pub range: std::ops::Range<usize>,
    /// Authored sRGB, straight alpha. `None` inherits the caller's colour,
    /// which is what an unmarked run gets — so markup that sets no colour does
    /// not override the theme.
    pub color: Option<[u8; 4]>,
    /// Size in logical pixels. `None` inherits.
    pub size: Option<f32>,
    /// Which face to use. `None` inherits.
    pub role: Option<FontRole>,
    pub decoration: Decoration,
    pub motion: Motion,
    /// A link target. Present makes the run interactive.
    pub link: Option<String>,
    /// An inline sprite drawn in place of the run's text.
    ///
    /// The run still carries a `range`, and it is the placeholder character's —
    /// which is what lets a caret step over a sprite as one unit and a
    /// selection include it.
    pub sprite: Option<String>,
    /// Resolved writing direction.
    pub direction: Direction,
}

impl Default for StyledRun {
    fn default() -> Self {
        Self {
            range: 0..0,
            color: None,
            size: None,
            role: None,
            decoration: Decoration::default(),
            motion: Motion::None,
            link: None,
            sprite: None,
            direction: Direction::Ltr,
        }
    }
}

impl StyledRun {
    /// A plain run over `range`.
    #[must_use]
    pub fn plain(range: std::ops::Range<usize>) -> Self {
        Self {
            range,
            ..Default::default()
        }
    }

    /// Whether the run covers no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.range.is_empty() && self.sprite.is_none()
    }

    /// The text this run covers.
    ///
    /// # Panics
    ///
    /// Panics if the range is not on a `char` boundary of `source`. The parser
    /// only ever produces boundaries, so a panic here means a caller built a
    /// run by hand and got it wrong — which is worth a panic rather than a
    /// silently truncated string.
    #[must_use]
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.clone()]
    }
}

/// Whether the shaper is enabled.
///
/// Reads `SOMNIUM_UI_SHAPER`, per Appendix A.5. **Default off**: turning it on
/// changes every glyph position in the editor, and GHOSTFENCE has no golden
/// reference to A/B against yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShaperPolicy {
    /// `fontdue` per-character advances, block-origin snapped. Phase 27's
    /// behaviour, byte for byte.
    #[default]
    PerCharacter,
    /// Shaped runs: sub-pixel advances within a run, the run origin snapped.
    ///
    /// **Not implemented.** Selecting it is how the A/B gets set up once there
    /// is a golden image to compare against; until then it behaves as
    /// [`Self::PerCharacter`] and says so.
    Shaped,
}

impl ShaperPolicy {
    /// Read the policy from the environment.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("SOMNIUM_UI_SHAPER").as_deref() {
            Ok("1") => Self::Shaped,
            _ => Self::PerCharacter,
        }
    }

    /// Whether shaping is actually available.
    ///
    /// Always `false` today. The method exists so the call site that will
    /// branch on it is written once, now, rather than retrofitted — and so
    /// `Shaped` cannot be mistaken for "working" by reading the enum.
    #[must_use]
    pub fn is_available(self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_latin_paragraph_is_left_to_right() {
        assert_eq!(Direction::of_paragraph("Hello"), Direction::Ltr);
        assert_eq!(Direction::of_paragraph("  \u{2014} Hello"), Direction::Ltr);
    }

    /// An Arabic paragraph is right-to-left, and leading punctuation does not
    /// change that.
    ///
    /// "Always LTR" mangles the first line of every Arabic UI, which is why the
    /// heuristic exists at all rather than defaulting silently.
    #[test]
    fn an_arabic_paragraph_is_right_to_left() {
        assert_eq!(Direction::of_paragraph("\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"), Direction::Rtl);
        assert_eq!(Direction::of_paragraph("  (\u{05E9}\u{05DC}\u{05D5}\u{05DD}"), Direction::Rtl);
    }

    /// The *first strong* character decides, not the majority.
    ///
    /// UAX #9's P2 rule. A sentence that starts in English and quotes Arabic is
    /// an LTR paragraph containing an RTL run, not an RTL paragraph.
    #[test]
    fn the_first_strong_character_decides() {
        assert_eq!(
            Direction::of_paragraph("Name: \u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
            Direction::Ltr
        );
    }

    #[test]
    fn digits_and_punctuation_are_not_strong() {
        assert_eq!(Direction::of_paragraph("123 !?"), Direction::Ltr);
        assert_eq!(
            Direction::of_paragraph("123 \u{05D0}"),
            Direction::Rtl,
            "the digits are neutral; the Hebrew letter decides"
        );
    }

    #[test]
    fn cjk_is_left_to_right() {
        assert_eq!(Direction::of_paragraph("\u{4F60}\u{597D}"), Direction::Ltr);
        assert_eq!(Direction::of_paragraph("\u{3053}\u{3093}"), Direction::Ltr);
    }

    /// A run is a range, not a copy.
    #[test]
    fn a_run_slices_its_source_rather_than_owning_it() {
        let source = "red and blue";
        let run = StyledRun::plain(8..12);
        assert_eq!(run.slice(source), "blue");
        assert_eq!(std::mem::size_of_val(&run.range), 16);
    }

    /// An unmarked run inherits rather than overriding.
    ///
    /// A markup parser that filled in defaults would make every unstyled span
    /// override the theme, which is how a "plain" label ends up hard-coded
    /// white in a light theme.
    #[test]
    fn an_unstyled_run_inherits_everything() {
        let run = StyledRun::plain(0..4);
        assert_eq!(run.color, None);
        assert_eq!(run.size, None);
        assert_eq!(run.role, None);
        assert_eq!(run.decoration, Decoration::default());
        assert_eq!(run.motion, Motion::None);
    }

    /// The shaper is off, and says so, rather than being selectable-but-broken.
    #[test]
    fn the_shaper_is_off_by_default_and_admits_it() {
        assert_eq!(ShaperPolicy::default(), ShaperPolicy::PerCharacter);
        assert!(!ShaperPolicy::Shaped.is_available());
        assert!(!ShaperPolicy::PerCharacter.is_available());
    }
}
