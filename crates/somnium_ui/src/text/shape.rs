//! Turning a string into positioned glyphs.
//!
//! MORROWIND-G item 1, adopted. That record chose `cosmic-text` and
//! deliberately did not land it, for a reason it stated plainly: shaping
//! changes every glyph position in the editor and there was no golden image to
//! A/B against. Two things have changed since, and both are why this is a
//! different library:
//!
//! - **The deciding row of that comparison is gone.** `cosmic-text` won over
//!   `parley` on editing — buffer, selection, cursor movement — because item 5
//!   was IME. Item 5 shipped in [`crate::text::ime`] without it, so the editing
//!   model is no longer something a shaper has to bring.
//! - **What is left to want is shaping alone**, and the surrounding parts are
//!   already built: [`crate::text::StyledRun`] is the run model,
//!   [`crate::text::fallback::FallbackChain`] resolves coverage, and
//!   [`crate::font::FontAtlas`] rasterises and packs. A library that also owns
//!   a font database, an atlas and a layout engine would arrive as a *second*
//!   text stack beside those, which is the thing GHOSTFENCE's `no-second-system`
//!   row exists to prevent.
//!
//! So: `rustybuzz` for shaping, `unicode-bidi` for the UAX #9 resolution
//! MORROWIND-G item 4 deferred, and the existing atlas for everything else.
//!
//! ```text
//!   "مرحبا Hello"
//!        │
//!        ├─ unicode-bidi ─────────  levels per byte, visual order per line
//!        │                          (item 4's deferred half)
//!        ├─ FallbackChain::split ─  longest spans one face can cover
//!        │                          (item 3, already built)
//!        └─ rustybuzz ────────────  positioned glyph ids per span
//!                                   (item 1, here)
//! ```

use crate::font::FontAtlas;
use crate::text::Direction;
use crate::text::fallback::FallbackChain;

/// One glyph, placed.
///
/// Positions are in **pixels at the requested size**, relative to the run
/// origin, and are deliberately not rounded: rounding each advance is what
/// destroys the kerning shaping exists to produce. The caller snaps the run
/// origin instead — Appendix A.5's resolution, and the reason this type carries
/// `f32` offsets rather than integers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// Index into the face, not a character. A ligature has one for two
    /// codepoints; an Arabic letter has a different one per joining form.
    pub glyph_id: u16,
    /// Which face drew it — a run can cross faces without the caller caring.
    pub font_id: u8,
    /// Pen position for this glyph, from the run origin.
    pub x: f32,
    pub y: f32,
    /// Byte offset in the source string this glyph came from.
    ///
    /// The cluster, not the character: it is what a caret is placed by, and
    /// what makes "one press of Left" move by a grapheme rather than into the
    /// middle of a ligature.
    pub cluster: usize,
}

/// A shaped line: its glyphs and how wide it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedLine {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: f32,
}

impl ShapedLine {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

/// Shape one line of text into positioned glyphs.
///
/// The whole pipeline: resolve bidi levels, split by what each face covers,
/// shape each span, and lay the spans out in **visual** order.
///
/// Returns `None` when nothing can be shaped — no faces loaded, or no face
/// covers anything in the string — so the caller can fall back to the
/// per-character path rather than draw an empty line.
#[must_use]
pub fn shape_line(
    text: &str,
    px: f32,
    atlas: &FontAtlas,
    chain: &FallbackChain<crate::text::fallback::CoverageSet>,
    base: Direction,
    primary: u8,
) -> Option<ShapedLine> {
    if text.is_empty() || atlas.font_count() == 0 {
        return None;
    }

    // ── Bidi ────────────────────────────────────────────────────────────────
    //
    // MORROWIND-G item 4 stopped at levels because "reordering belongs with the
    // thing that can reorder glyphs". This is that thing.
    let level = if base.is_rtl() {
        unicode_bidi::Level::rtl()
    } else {
        unicode_bidi::Level::ltr()
    };
    let bidi = unicode_bidi::BidiInfo::new(text, Some(level));
    let para = bidi.paragraphs.first()?;
    let line = para.range.clone();
    let (levels, ranges) = bidi.visual_runs(para, line);

    let mut out: Vec<PlacedGlyph> = Vec::new();
    let mut pen = 0.0f32;
    for run in ranges {
        let run_rtl = levels[run.start].is_rtl();
        // Coverage is asked per span, inside the bidi run: a run can still
        // cross faces, and a face change mid-run is not a direction change.
        // The caller's face first. A chain is a fallback chain: a label asking
        // for the bold cut is asking for bold, and regular covering Latin too
        // is not a reason to draw it in regular.
        for (span, font_id) in chain.split_preferring(&text[run.clone()], Some(primary)) {
            let Some(font_id) = font_id else {
                // No face covers this. MORROWIND-G's rule holds: keep the span
                // rather than substituting a space, because an invisible
                // missing glyph does not get reported.
                continue;
            };
            let absolute = run.start + span.start;
            let slice = &text[run.start + span.start..run.start + span.end];
            let Some(bytes) = atlas.font_bytes(font_id) else {
                continue;
            };
            let Some(face) = rustybuzz::Face::from_slice(bytes, 0) else {
                continue;
            };
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(slice);
            buffer.set_direction(if run_rtl {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            // Script and language from the text itself. Guessing beats a wrong
            // constant: a `Script::LATIN` hard-coded here disables every
            // script-specific feature, which is most of what shaping does.
            buffer.guess_segment_properties();

            let shaped = rustybuzz::shape(&face, &[], buffer);
            // Font units to pixels at the requested size.
            let scale = px / face.units_per_em() as f32;
            let infos = shaped.glyph_infos();
            let positions = shaped.glyph_positions();
            for (info, position) in infos.iter().zip(positions.iter()) {
                out.push(PlacedGlyph {
                    glyph_id: u16::try_from(info.glyph_id).unwrap_or(0),
                    font_id,
                    x: pen + position.x_offset as f32 * scale,
                    y: -position.y_offset as f32 * scale,
                    cluster: absolute + info.cluster as usize,
                });
                pen += position.x_advance as f32 * scale;
            }
        }
    }

    (!out.is_empty()).then_some(ShapedLine {
        glyphs: out,
        width: pen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::fallback::CoverageSet;

    /// The bundled Latin face, or `None` where it is not on disk.
    ///
    /// Shaping is a property of a *face*, so these tests need a real one. They
    /// skip rather than fail without it: a machine with no Segoe is not a
    /// broken shaper, and a test that cannot tell the difference is worse than
    /// one that says so.
    fn latin_atlas() -> Option<(FontAtlas, FallbackChain<CoverageSet>)> {
        let bytes = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf")
            .or_else(|_| std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"))
            .ok()?;
        let mut atlas = FontAtlas::new();
        let id = atlas.add_font(&bytes).ok()?;
        let mut chain = FallbackChain::new();
        chain.push(id, CoverageSet::new(vec![0..0x2FFF]));
        Some((atlas, chain))
    }

    #[test]
    fn shaping_places_one_glyph_per_latin_character() {
        let Some((atlas, chain)) = latin_atlas() else {
            return;
        };
        let line = shape_line("Hello", 16.0, &atlas, &chain, Direction::Ltr, 0)
            .expect("a Latin string shapes");
        assert_eq!(line.glyphs.len(), 5);
        assert!(line.width > 0.0);
        // Monotonic pen: LTR glyphs advance rightwards.
        for pair in line.glyphs.windows(2) {
            assert!(pair[1].x >= pair[0].x, "{:?}", line.glyphs);
        }
    }

    #[test]
    fn advances_are_not_whole_pixels() {
        // The point of shaping, and the thing the per-character path cannot do.
        // If every advance were an integer the kerning would be gone, which is
        // exactly the failure Appendix A.5 warns about when snapping shaped
        // output.
        let Some((atlas, chain)) = latin_atlas() else {
            return;
        };
        let line =
            shape_line("AVATAR Wave", 13.0, &atlas, &chain, Direction::Ltr, 0).expect("shapes");
        let fractional = line.glyphs.iter().any(|g| (g.x - g.x.round()).abs() > 1e-3);
        assert!(
            fractional,
            "every advance landed on a whole pixel: {:?}",
            line
        );
    }

    #[test]
    fn an_empty_string_shapes_to_nothing_rather_than_panicking() {
        let Some((atlas, chain)) = latin_atlas() else {
            return;
        };
        assert!(shape_line("", 16.0, &atlas, &chain, Direction::Ltr, 0).is_none());
    }

    #[test]
    fn text_no_face_covers_is_dropped_rather_than_drawn_wrong() {
        // An empty chain covers nothing. The rule from MORROWIND-G item 3 is
        // that a span with no face stays its own span — here it produces no
        // glyphs at all, and the caller sees `None` and takes the other path,
        // rather than every character rendering as glyph zero.
        let Some((atlas, _)) = latin_atlas() else {
            return;
        };
        let empty: FallbackChain<CoverageSet> = FallbackChain::new();
        assert!(shape_line("Hello", 16.0, &atlas, &empty, Direction::Ltr, 0).is_none());
    }

    #[test]
    fn an_arabic_line_is_laid_out_right_to_left() {
        // UAX #9's resolution, which MORROWIND-G item 4 deferred to whatever
        // could reorder glyphs. The first character of the string must end up
        // on the *right*, which is the one thing "always LTR" gets wrong on
        // every line of an Arabic UI.
        let Some((atlas, chain)) = latin_atlas() else {
            return;
        };
        let arabic = "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}";
        let Some(line) = shape_line(arabic, 16.0, &atlas, &chain, Direction::Rtl, 0) else {
            // Segoe UI has no Arabic; the coverage set claimed it does, so a
            // face without the glyphs simply shapes to notdefs or nothing.
            return;
        };
        let first = line.glyphs.iter().find(|g| g.cluster == 0);
        let last = line
            .glyphs
            .iter()
            .max_by_key(|g| g.cluster)
            .expect("at least one glyph");
        if let Some(first) = first {
            assert!(
                first.x >= last.x,
                "the first character of an RTL line belongs on the right: {line:?}"
            );
        }
    }

    #[test]
    fn a_mixed_line_puts_the_latin_run_in_reading_order() {
        // Bidi's actual job: the Latin inside an RTL paragraph still reads left
        // to right, even though the paragraph does not.
        let Some((atlas, chain)) = latin_atlas() else {
            return;
        };
        let mixed = "\u{0645}\u{0631} Hello";
        let Some(line) = shape_line(mixed, 16.0, &atlas, &chain, Direction::Rtl, 0) else {
            return;
        };
        let latin: Vec<_> = line
            .glyphs
            .iter()
            .filter(|g| {
                mixed
                    .as_bytes()
                    .get(g.cluster)
                    .is_some_and(u8::is_ascii_alphabetic)
            })
            .collect();
        for pair in latin.windows(2) {
            assert!(
                pair[1].x >= pair[0].x,
                "Latin inside an RTL line still reads left to right: {latin:?}"
            );
        }
    }
}
