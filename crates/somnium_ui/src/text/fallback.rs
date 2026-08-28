//! Font fallback chains (MORROWIND-G item 3).
//!
//! > *"Font fallback chains, so a CJK glyph in an English UI renders rather
//! > than showing tofu."*
//!
//! # What tofu costs, and where it comes from
//!
//! `FontAtlas::get_or_rasterize` takes a `font_id` and asks that one face for a
//! glyph. When the face has no glyph for the codepoint, `fontdue` returns the
//! `.notdef` box — the tofu — and the UI renders a row of rectangles where a
//! player's name was. The five bundled cuts are Latin; **every non-Latin
//! character in the engine is tofu today**, which includes every save-game name
//! typed on a Japanese keyboard.
//!
//! The fix is a chain: ask each face in turn and take the first that covers the
//! codepoint. That is the easy half. The hard half is doing it *per run*
//! without splitting every string into one run per character, which is what
//! [`FallbackChain::split`] is for.
//!
//! # Coverage is asked of the face, not guessed from the codepoint
//!
//! A tempting shortcut is to route by Unicode block — CJK to the CJK face,
//! Latin to the UI face. It is wrong in both directions: a good CJK face
//! contains Latin (and its Latin is designed to sit with its Han), and a Latin
//! face routinely contains Greek, Cyrillic and arrows that a block table would
//! send elsewhere. So [`FaceCoverage`] is a *question asked of a face*, and the
//! block ranges here exist only to order the chain sensibly, never to decide.

use std::ops::Range;

/// Whether a face can render a codepoint.
///
/// A trait rather than a concrete type so the chain can be tested without a
/// font file, and so a future `cosmic-text` font database can implement it
/// without this module learning what a database is.
pub trait FaceCoverage {
    /// Whether this face has a glyph for `ch`.
    fn covers(&self, ch: char) -> bool;
}

/// A face's id and the codepoints it covers.
///
/// The simple implementation, used by the atlas and by tests.
#[derive(Clone, Debug, Default)]
pub struct CoverageSet {
    /// Sorted, non-overlapping ranges of covered codepoints.
    ranges: Vec<Range<u32>>,
}

impl CoverageSet {
    /// A set covering exactly `ranges`.
    #[must_use]
    pub fn new(mut ranges: Vec<Range<u32>>) -> Self {
        ranges.sort_by_key(|r| r.start);
        Self { ranges }
    }

    /// Basic Latin plus Latin-1, which is what the bundled cuts actually cover.
    #[must_use]
    pub fn latin() -> Self {
        Self::new(vec![0x0020..0x0250, 0x2000..0x2100])
    }

    /// Whether anything is covered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

impl FaceCoverage for CoverageSet {
    fn covers(&self, ch: char) -> bool {
        let code = ch as u32;
        self.ranges
            .binary_search_by(|r| {
                if code < r.start {
                    std::cmp::Ordering::Greater
                } else if code >= r.end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// An ordered list of faces to try.
pub struct FallbackChain<F> {
    faces: Vec<(u8, F)>,
}

impl<F> Default for FallbackChain<F> {
    fn default() -> Self {
        Self { faces: Vec::new() }
    }
}

impl<F: FaceCoverage> FallbackChain<F> {
    /// An empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a face. Order is priority order.
    pub fn push(&mut self, font_id: u8, coverage: F) {
        self.faces.push((font_id, coverage));
    }

    /// How many faces are in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// Whether the chain is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// The first face covering `ch`, or `None` when none does.
    ///
    /// `None` is genuinely tofu, and the caller should render it as such rather
    /// than substituting a space: a missing glyph the player can see gets
    /// reported, and one that silently vanishes does not.
    #[must_use]
    pub fn face_for(&self, ch: char) -> Option<u8> {
        self.faces
            .iter()
            .find(|(_, coverage)| coverage.covers(ch))
            .map(|(id, _)| *id)
    }

    /// Split `text` into the longest possible spans that share one face.
    ///
    /// **The whole point of the chain.** A per-character lookup would produce
    /// one run per character, and a run is the unit a shaper shapes — so
    /// per-character runs mean per-character shaping, which is exactly the
    /// kerning-free output shaping exists to avoid.
    ///
    /// Ranges are byte ranges into `text` and always land on char boundaries.
    /// A span whose face is `None` is tofu and is kept as its own span, so a
    /// caller can render it distinctly or log it.
    #[must_use]
    pub fn split(&self, text: &str) -> Vec<(Range<usize>, Option<u8>)> {
        let mut spans: Vec<(Range<usize>, Option<u8>)> = Vec::new();
        for (offset, ch) in text.char_indices() {
            let face = self.face_for(ch);
            let end = offset + ch.len_utf8();
            match spans.last_mut() {
                Some((range, current)) if *current == face => range.end = end,
                _ => spans.push((offset..end, face)),
            }
        }
        spans
    }
}

/// Codepoint ranges worth having a face for, in the order a chain should try.
///
/// **Ordering only.** Coverage is asked of the face; these exist so a chain
/// assembled from a font directory puts the scripts a UI is most likely to need
/// before the ones it is least likely to, rather than in directory order.
pub const SCRIPT_PRIORITY: &[(&str, Range<u32>)] = &[
    ("Latin", 0x0020..0x0250),
    ("Greek", 0x0370..0x0400),
    ("Cyrillic", 0x0400..0x0500),
    ("Hebrew", 0x0590..0x0600),
    ("Arabic", 0x0600..0x0700),
    ("Devanagari", 0x0900..0x0980),
    ("Thai", 0x0E00..0x0E80),
    ("Hangul Jamo", 0x1100..0x1200),
    ("CJK Symbols", 0x3000..0x3040),
    ("Hiragana", 0x3040..0x30A0),
    ("Katakana", 0x30A0..0x3100),
    ("CJK Unified", 0x4E00..0xA000),
    ("Hangul Syllables", 0xAC00..0xD7B0),
    ("Emoji", 0x1F300..0x1FAFF),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn latin() -> CoverageSet {
        CoverageSet::latin()
    }

    fn cjk() -> CoverageSet {
        CoverageSet::new(vec![0x3000..0x3100, 0x4E00..0xA000])
    }

    fn everything() -> CoverageSet {
        CoverageSet::new(vec![0..0x110000])
    }

    #[test]
    fn coverage_answers_for_its_own_ranges() {
        let set = latin();
        assert!(set.covers('A'));
        assert!(set.covers('\u{00E9}'));
        assert!(!set.covers('\u{4F60}'));
        assert!(!set.covers('\u{1F600}'));
    }

    #[test]
    fn an_empty_chain_covers_nothing() {
        let chain: FallbackChain<CoverageSet> = FallbackChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.face_for('A'), None);
        assert_eq!(chain.split("hi"), vec![(0..2, None)]);
    }

    #[test]
    fn the_first_covering_face_wins() {
        let mut chain = FallbackChain::new();
        chain.push(0, latin());
        chain.push(1, cjk());
        chain.push(2, everything());
        assert_eq!(chain.face_for('A'), Some(0));
        assert_eq!(chain.face_for('\u{4F60}'), Some(1));
        assert_eq!(
            chain.face_for('\u{1F600}'),
            Some(2),
            "the catch-all catches"
        );
    }

    /// **The reason the chain exists.** A CJK glyph in an English UI finds a
    /// face instead of rendering a box.
    #[test]
    fn a_cjk_name_in_a_latin_ui_is_not_tofu() {
        let mut chain = FallbackChain::new();
        chain.push(0, latin());
        chain.push(1, cjk());
        for ch in "\u{5C71}\u{7530}".chars() {
            assert_eq!(chain.face_for(ch), Some(1), "{ch} fell through to tofu");
        }
    }

    /// **The reason `split` exists.** Spans are as long as possible, because a
    /// run is the unit a shaper shapes and per-character runs mean
    /// per-character shaping — the kerning-free output shaping exists to avoid.
    #[test]
    fn split_produces_the_longest_spans_it_can() {
        let mut chain = FallbackChain::new();
        chain.push(0, latin());
        chain.push(1, cjk());

        let text = "Hello \u{4F60}\u{597D} world";
        let spans = chain.split(text);
        assert_eq!(spans.len(), 3, "{spans:?}");
        assert_eq!(spans[0].1, Some(0));
        assert_eq!(&text[spans[0].0.clone()], "Hello ");
        assert_eq!(spans[1].1, Some(1));
        assert_eq!(&text[spans[1].0.clone()], "\u{4F60}\u{597D}");
        assert_eq!(spans[2].1, Some(0));
        assert_eq!(&text[spans[2].0.clone()], " world");
    }

    /// A span with no face is kept as its own span rather than merged away.
    ///
    /// Substituting a space would make a missing glyph invisible, and an
    /// invisible bug does not get reported.
    #[test]
    fn tofu_is_its_own_span_rather_than_hidden() {
        let mut chain = FallbackChain::new();
        chain.push(0, latin());
        let text = "a\u{4F60}b";
        let spans = chain.split(text);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].1, None);
        assert_eq!(&text[spans[1].0.clone()], "\u{4F60}");
    }

    /// Ranges land on char boundaries, so slicing does not panic.
    #[test]
    fn split_ranges_are_char_boundaries() {
        let mut chain = FallbackChain::new();
        chain.push(0, everything());
        let text = "a\u{00E9}\u{4F60}\u{1F600}z";
        for (range, _) in chain.split(text) {
            let _ = &text[range]; // would panic off a boundary
        }
    }

    #[test]
    fn splitting_empty_text_produces_no_spans() {
        let mut chain = FallbackChain::new();
        chain.push(0, latin());
        assert!(chain.split("").is_empty());
    }

    /// Coverage is asked of the face, never inferred from the block.
    ///
    /// A good CJK face contains Latin, and its Latin is designed to sit with
    /// its Han. Routing by block would send that Latin to the wrong face and
    /// produce a visible mismatch in the middle of a Japanese sentence.
    #[test]
    fn a_cjk_face_that_covers_latin_is_allowed_to_serve_it() {
        let cjk_with_latin = CoverageSet::new(vec![0x0020..0x0250, 0x4E00..0xA000]);
        let mut chain = FallbackChain::new();
        chain.push(7, cjk_with_latin);
        assert_eq!(chain.face_for('A'), Some(7));
        assert_eq!(chain.face_for('\u{4F60}'), Some(7));
        assert_eq!(
            chain.split("A\u{4F60}").len(),
            1,
            "one face covers both, so it is one span and one shaping run"
        );
    }

    #[test]
    fn the_priority_table_is_ordered_and_disjoint() {
        for pair in SCRIPT_PRIORITY.windows(2) {
            assert!(
                pair[0].1.end <= pair[1].1.start,
                "{} overlaps or follows {}",
                pair[0].0,
                pair[1].0
            );
        }
    }
}
