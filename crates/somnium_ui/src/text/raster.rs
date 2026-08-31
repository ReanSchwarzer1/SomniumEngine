//! Rasterising a glyph the **shaper** chose.
//!
//! MORROWIND-G item 1. The per-character path rasterises with `fontdue` and
//! always has; this exists because the shaped path cannot.
//!
//! # Why not just hand `fontdue` the shaped glyph id
//!
//! Because they are not the same numbers. Measured, on the editor's own
//! `Inter-Regular.ttf`:
//!
//! | character | `rustybuzz` | `fontdue` |
//! |---|---|---|
//! | `C` | 18 | 18 |
//! | `(` | 331 | 324 |
//! | `:` | 366 | 365 |
//! | `-` | 348 | 344 |
//!
//! Letters coincide and punctuation does not, and the divergence is not a
//! constant offset. Feeding one library's index to the other renders a
//! *different glyph* — which for punctuation showed up as an empty bitmap and a
//! missing character, and for a ligature would have shown up as plausible,
//! wrong text that nobody would think to check.
//!
//! So the shaped path rasterises from the same face the shaper read: outlines
//! from `ttf-parser` (which `rustybuzz` re-exports, so it is by construction
//! the same parse), filled by `tiny-skia`. Both are already in the tree —
//! `resvg` brought them for the icon atlas — so this is a new *use* of the
//! dependency graph rather than a new dependency in it.

use resvg::tiny_skia;
use rustybuzz::ttf_parser;

/// A rasterised glyph: an 8-bit coverage bitmap and where it sits.
pub struct RasterGlyph {
    /// Coverage, row-major, `width * height` bytes.
    pub coverage: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Left bearing, in pixels, from the pen position.
    pub xmin: f32,
    /// Distance from the baseline to the bitmap's **bottom**, y-up — the same
    /// convention `fontdue` reports, so both paths place glyphs identically.
    pub ymin: f32,
}

/// Rasterise one glyph of `face` at `px`.
///
/// Returns `None` for a glyph with no outline — a space, and legitimately so —
/// which the caller treats as "advance, draw nothing" exactly as it does for
/// the per-character path.
#[must_use]
pub fn rasterize(bytes: &[u8], glyph_id: u16, px: f32) -> Option<RasterGlyph> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let units_per_em = f32::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }
    let scale = px / units_per_em;

    let mut builder = OutlineBuilder::default();
    let bbox = face.outline_glyph(ttf_parser::GlyphId(glyph_id), &mut builder)?;
    let path = builder.finish()?;

    // The bounding box in pixels, expanded to whole pixels so the fill has
    // room. `floor`/`ceil` rather than `round`: a glyph clipped by half a pixel
    // is a glyph with a flat side.
    let x0 = (f32::from(bbox.x_min) * scale).floor();
    let y0 = (f32::from(bbox.y_min) * scale).floor();
    let x1 = (f32::from(bbox.x_max) * scale).ceil();
    let y1 = (f32::from(bbox.y_max) * scale).ceil();
    let width = (x1 - x0).max(1.0) as u32;
    let height = (y1 - y0).max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;

    // Font space is y-up with the origin on the baseline; a pixmap is y-down
    // with the origin at its top-left. One transform carries both changes.
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, -scale, -x0, y1);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);

    // Coverage is the alpha channel. The pixmap is premultiplied white, so
    // every channel already holds it; alpha is the one that means it.
    let coverage = pixmap.pixels().iter().map(|p| p.alpha()).collect();
    Some(RasterGlyph {
        coverage,
        width,
        height,
        xmin: x0,
        ymin: y0,
    })
}

/// Collects a `ttf-parser` outline into a `tiny-skia` path.
#[derive(Default)]
struct OutlineBuilder {
    builder: tiny_skia::PathBuilder,
}

impl OutlineBuilder {
    fn finish(self) -> Option<tiny_skia::Path> {
        self.builder.finish()
    }
}

impl ttf_parser::OutlineBuilder for OutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

    /// The glyph id `rustybuzz` gives a single character of `text`.
    fn shaped_id(text: &str) -> u16 {
        let face = rustybuzz::Face::from_slice(INTER, 0).expect("face");
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(&face, &[], buffer);
        u16::try_from(shaped.glyph_infos()[0].glyph_id).expect("fits")
    }

    #[test]
    fn the_punctuation_the_other_rasteriser_could_not_draw() {
        // The bug this module exists for, as a test. Each of these shapes to a
        // glyph id `fontdue` rasterises empty, because the two libraries do not
        // share an index space — and each one is a character the editor shows
        // in ordinary labels: "Coastal Surf (CC0)", "14:00", "a-b".
        for sample in ["(", ")", ":", "-", "C", "0"] {
            let id = shaped_id(sample);
            let glyph = rasterize(INTER, id, 13.0)
                .unwrap_or_else(|| panic!("{sample:?} (glyph {id}) has no outline"));
            assert!(
                glyph.width > 0 && glyph.height > 0,
                "{sample:?} rasterised to nothing"
            );
            assert!(
                glyph.coverage.iter().any(|&a| a > 0),
                "{sample:?} rasterised to a blank bitmap"
            );
            assert_eq!(
                glyph.coverage.len(),
                (glyph.width * glyph.height) as usize,
                "{sample:?} coverage does not match its dimensions"
            );
        }
    }

    #[test]
    fn a_space_has_no_outline_and_says_so() {
        // Not a failure: a space is a glyph with an advance and nothing to
        // draw, and the caller has to be able to tell that from an error.
        let id = shaped_id(" ");
        assert!(rasterize(INTER, id, 13.0).is_none());
    }

    #[test]
    fn a_bigger_size_gives_a_bigger_bitmap() {
        let id = shaped_id("W");
        let small = rasterize(INTER, id, 8.0).expect("outline");
        let large = rasterize(INTER, id, 32.0).expect("outline");
        assert!(
            large.width > small.width && large.height > small.height,
            "{}x{} against {}x{}",
            large.width,
            large.height,
            small.width,
            small.height
        );
    }

    #[test]
    fn a_glyph_id_the_face_does_not_have_is_refused() {
        assert!(rasterize(INTER, u16::MAX, 13.0).is_none());
    }
}
