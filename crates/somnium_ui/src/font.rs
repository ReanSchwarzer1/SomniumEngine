// Phase 12A-4 — font atlas.
// Uses fontdue (pure-Rust TrueType rasterizer) to rasterize glyphs on demand.
// Atlas is a 512×512 RGBA8 texture: RGB=255, A=glyph coverage, so vertex color tints the text.
// Glyph packing uses a simple shelf/row algorithm (cursor_x/cursor_y/row_height).
//
// FONT_ATLAS_TEXTURE_ID = 0 is the convention used by DrawingContext and UiPass.

use glam::Vec2;
use std::collections::HashMap;

/// wgpu texture_id reserved for the font atlas (DrawCommand.texture_id = Some(this)).
pub const FONT_ATLAS_TEXTURE_ID: u32 = 0;

/// Atlas texture dimensions — 1024² so a bundled Inter at several sizes fits
/// (Phase 26-A; was 512² when the editor only used Segoe at 12 px).
pub const ATLAS_WIDTH: u32 = 1024;
pub const ATLAS_HEIGHT: u32 = 1024;

/// Hash key for a cached glyph entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub codepoint: u32,
    /// f32 font size as raw bits — exact equality on constants, no NaN risk.
    pub px_bits: u32,
    pub font_id: u8,
}

impl GlyphKey {
    pub fn new(ch: char, px: f32, font_id: u8) -> Self {
        Self {
            codepoint: ch as u32,
            px_bits: px.to_bits(),
            font_id,
        }
    }
}

/// Packed glyph data returned from the atlas.
#[derive(Clone, Copy, Debug)]
pub struct GlyphInfo {
    /// Atlas UV coordinates (normalised 0..1).
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// Offset from cursor position to top-left of glyph bitmap (xmin, ymin as freetype metrics).
    pub xmin: f32,
    pub ymin: f32,
    /// Pixel dimensions of the rasterised glyph bitmap.
    pub px_w: f32,
    pub px_h: f32,
    /// Horizontal advance (cursor step to next character).
    pub advance: f32,
}

pub struct FontAtlas {
    /// RGBA8 pixel data — RGB=255 everywhere, A=glyph coverage.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Set when pixels are modified; UiPass should upload the texture and clear this flag.
    pub dirty: bool,

    fonts: Vec<fontdue::Font>,
    glyphs: HashMap<GlyphKey, GlyphInfo>,
    /// Device pixels per unit of UI **layout** space. Glyphs rasterize at
    /// `px * render_scale * SUPER_SAMPLE` and draw at `px`.
    ///
    /// This is deliberately **not** `Window::scale_factor()`. Somnium lays the
    /// widget tree out directly in physical pixels — `UiManager::reposition_panels`
    /// feeds `UserInterface::resize` the result of `window.inner_size()`, which
    /// winit reports in physical pixels — so one layout unit is one device pixel
    /// and the correct value is 1.0.
    ///
    /// Phase 27-B fixed a defect here. The pre-Styx atlas multiplied the raster
    /// size by the window scale factor *on top of* [`SUPER_SAMPLE`], so at 200 %
    /// a 13 px glyph rasterized at 39 px and was then minified into a 13 px
    /// quad — softer than the 1.5x supersample intends, not sharper, and 9x the
    /// atlas area per glyph. Feeding a real scale factor in here becomes correct
    /// only once layout itself moves to logical units; [`Self::set_render_scale`]
    /// is the hook for that day.
    pub render_scale: f32,

    // Shelf packing state
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    /// Set once the shelf allocator has refused a glyph. Text silently
    /// disappears past this point, so it must be observable.
    atlas_full: bool,
    /// Atlas pixels handed out to glyph bitmaps, for [`Self::utilization`].
    packed_px: u64,
}

/// Raster larger than the drawn size so 12–24 px Latin looks less crunchy.
///
/// Held at its pre-Styx value through Phase 27-B so the only intended changes
/// to text this phase are the integer-snapped glyph quad
/// (`DrawingContext::push_text_tracked`) and the coverage gamma
/// (`UiPass::DEFAULT_TEXT_GAMMA`). Whether 1.0 — pixel-exact rasterizer output,
/// now that quads land on the texel grid — reads better than 1.5 is a question
/// for the 27-B capture sheet, not for a guess in the source.
///
/// Full shaping and bidi (`cosmic-text`) remain the open 26-H item.
const SUPER_SAMPLE: f32 = 1.5;

impl FontAtlas {
    pub fn new() -> Self {
        let pixel_count = (ATLAS_WIDTH * ATLAS_HEIGHT) as usize * 4;
        Self {
            pixels: vec![0u8; pixel_count],
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
            dirty: false,
            fonts: Vec::new(),
            glyphs: HashMap::new(),
            render_scale: 1.0,
            cursor_x: 1, // 1px border to avoid UV bleeding
            cursor_y: 1,
            row_height: 0,
            atlas_full: false,
            packed_px: 0,
        }
    }

    /// Load a TrueType/OpenType font from bytes. Returns the font_id for use in draw calls.
    pub fn add_font(&mut self, bytes: &[u8]) -> Result<u8, &'static str> {
        let font = fontdue::Font::from_bytes(
            bytes,
            fontdue::FontSettings {
                collection_index: 0,
                scale: 160.0,
            },
        )?;
        let id = self.fonts.len() as u8;
        self.fonts.push(font);
        Ok(id)
    }

    pub fn has_fonts(&self) -> bool {
        !self.fonts.is_empty()
    }

    /// Measure text width/height using font metrics (no rasterization, fast).
    /// Returns (total_advance_width, line_height) in pixels.
    pub fn measure_text(&self, text: &str, px: f32, font_id: u8) -> Vec2 {
        self.measure_text_tracked(text, px, font_id, 0.0)
    }

    /// [`measure_text`](Self::measure_text) with letter-spacing. Must agree
    /// exactly with [`crate::draw::DrawingContext::push_text_tracked`], which
    /// also adds `tracking` after the final glyph.
    pub fn measure_text_tracked(&self, text: &str, px: f32, font_id: u8, tracking: f32) -> Vec2 {
        let Some(font) = self.fonts.get(font_id as usize) else {
            // Fallback: 8px wide, px tall per char
            return Vec2::new(
                text.chars().count() as f32 * (px * 0.55 + tracking).max(0.0),
                px,
            );
        };
        let line_h = font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent - m.descent)
            .unwrap_or(px);
        let mut max_w = 0.0f32;
        let mut lines = 0u32;
        for line in text.split('\n') {
            let w: f32 = line
                .chars()
                .map(|ch| font.metrics(ch, px).advance_width + tracking)
                .sum();
            max_w = max_w.max(w);
            lines += 1;
        }
        Vec2::new(max_w, line_h * lines.max(1) as f32)
    }

    /// Ascent above baseline in pixels (used by push_text to place glyphs).
    pub fn ascent(&self, px: f32, font_id: u8) -> f32 {
        self.fonts
            .get(font_id as usize)
            .and_then(|f| f.horizontal_line_metrics(px))
            .map(|m| m.ascent)
            .unwrap_or(px * 0.8)
    }

    /// Set device pixels per layout unit. See [`Self::render_scale`] — this is
    /// not the window scale factor while layout is in physical pixels.
    ///
    /// Changing it invalidates every cached glyph, because the cache is keyed on
    /// the raster size the scale produces.
    pub fn set_render_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.5, 4.0);
        if (scale - self.render_scale).abs() > 0.01 {
            self.render_scale = scale;
            self.glyphs.clear();
            self.pixels.fill(0);
            self.cursor_x = 1;
            self.cursor_y = 1;
            self.row_height = 0;
            self.atlas_full = false;
            self.packed_px = 0;
            self.dirty = true;
        }
    }

    /// Fraction of the atlas consumed by packed glyph bitmaps, 0..1.
    ///
    /// The shelf allocator never frees, so this only rises. It exists because
    /// exhaustion is otherwise invisible: [`Self::get_or_rasterize`] returns
    /// `None` and `push_text_tracked` degrades to a blank half-em advance, so
    /// a full atlas looks like missing text rather than like an error.
    pub fn utilization(&self) -> f32 {
        self.packed_px as f32 / (self.width as f32 * self.height as f32)
    }

    /// True once the atlas has refused at least one glyph.
    pub fn is_full(&self) -> bool {
        self.atlas_full
    }

    /// Number of distinct rasterized glyphs currently cached.
    pub fn cached_glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Get cached glyph or rasterize it into the atlas. Returns None if no font loaded or atlas full.
    pub fn get_or_rasterize(&mut self, ch: char, px: f32, font_id: u8) -> Option<GlyphInfo> {
        let raster_px = (px * self.render_scale * SUPER_SAMPLE).max(1.0);
        let key = GlyphKey::new(ch, raster_px, font_id);
        if let Some(&info) = self.glyphs.get(&key) {
            return Some(info);
        }
        let font = self.fonts.get(font_id as usize)?;
        let (metrics, bitmap) = font.rasterize(ch, raster_px);
        let inv = px / raster_px;

        // Whitespace / zero-size glyph — cache advance only
        if metrics.width == 0 || metrics.height == 0 {
            let info = GlyphInfo {
                uv_min: [0.0; 2],
                uv_max: [0.0; 2],
                xmin: metrics.xmin as f32 * inv,
                ymin: metrics.ymin as f32 * inv,
                px_w: 0.0,
                px_h: 0.0,
                advance: metrics.advance_width * inv,
            };
            self.glyphs.insert(key, info);
            return Some(info);
        }

        let gw = metrics.width as u32;
        let gh = metrics.height as u32;
        // Advance to next row if needed
        if self.cursor_x + gw + 1 > self.width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.cursor_y + gh + 1 > self.height {
            if !self.atlas_full {
                self.atlas_full = true;
                tracing::warn!(
                    cached_glyphs = self.glyphs.len(),
                    utilization = self.utilization(),
                    "font atlas exhausted; further glyphs will not render"
                );
            }
            return None;
        }

        // Blit glyph into RGBA8 atlas: R=G=B=255, A=coverage
        let w = self.width as usize;
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col];
                let idx = ((self.cursor_y as usize + row) * w + (self.cursor_x as usize + col)) * 4;
                self.pixels[idx] = 255;
                self.pixels[idx + 1] = 255;
                self.pixels[idx + 2] = 255;
                self.pixels[idx + 3] = alpha;
            }
        }

        let u0 = self.cursor_x as f32 / self.width as f32;
        let v0 = self.cursor_y as f32 / self.height as f32;
        let u1 = (self.cursor_x + gw) as f32 / self.width as f32;
        let v1 = (self.cursor_y + gh) as f32 / self.height as f32;

        let info = GlyphInfo {
            uv_min: [u0, v0],
            uv_max: [u1, v1],
            xmin: metrics.xmin as f32 * inv,
            ymin: metrics.ymin as f32 * inv,
            px_w: gw as f32 * inv,
            px_h: gh as f32 * inv,
            advance: metrics.advance_width * inv,
        };

        self.row_height = self.row_height.max(gh);
        self.cursor_x += gw + 1;
        self.packed_px += (gw as u64 + 1) * (gh as u64 + 1);
        self.dirty = true;
        self.glyphs.insert(key, info);
        Some(info)
    }
}

impl Default for FontAtlas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five bundled cuts, in the order `typography::FontRole` expects.
    const CUTS: [&[u8]; 5] = [
        include_bytes!("../assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
        include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
    ];

    fn loaded() -> FontAtlas {
        let mut a = FontAtlas::new();
        for cut in CUTS {
            a.add_font(cut).expect("bundled OFL cut must parse");
        }
        a
    }

    #[test]
    fn render_scale_is_one_because_layout_is_in_physical_pixels() {
        // Regression guard for the Phase 27-B defect: nothing may quietly wire
        // `Window::scale_factor()` in here while the widget tree is laid out in
        // physical pixels, or glyphs get rasterized large and minified back down.
        assert_eq!(FontAtlas::new().render_scale, 1.0);
    }

    #[test]
    fn raster_size_is_the_drawn_size_times_the_supersample() {
        let mut a = loaded();
        let info = a.get_or_rasterize('H', 13.0, 0).unwrap();
        // The glyph is cached under its raster size but reported at the drawn
        // size, so a 13 px 'H' must measure well under 13 px tall, never 1.5x it.
        assert!(info.px_h > 0.0 && info.px_h <= 13.0, "px_h = {}", info.px_h);
    }

    #[test]
    fn changing_render_scale_invalidates_every_cached_glyph() {
        let mut a = loaded();
        a.get_or_rasterize('A', 13.0, 0).unwrap();
        assert_eq!(a.cached_glyph_count(), 1);
        a.set_render_scale(2.0);
        assert_eq!(a.cached_glyph_count(), 0, "cache is keyed on raster size");
        assert!(a.dirty);
        assert_eq!(a.utilization(), 0.0);
    }

    #[test]
    fn an_unchanged_render_scale_keeps_the_cache() {
        let mut a = loaded();
        a.get_or_rasterize('A', 13.0, 0).unwrap();
        a.set_render_scale(1.0);
        assert_eq!(a.cached_glyph_count(), 1);
    }

    #[test]
    fn utilization_rises_as_glyphs_are_packed_and_starts_empty() {
        let mut a = loaded();
        assert_eq!(a.utilization(), 0.0);
        assert!(!a.is_full());
        for ch in "The quick brown fox".chars() {
            a.get_or_rasterize(ch, 13.0, 0);
        }
        assert!(a.utilization() > 0.0);
        assert!(!a.is_full());
    }

    /// Measures real atlas pressure for the editor's actual type inventory.
    ///
    /// This exists to settle a design question with data rather than a guess:
    /// Phase 27-B considered three-phase subpixel X positioning, which triples
    /// the number of cached bitmaps. That is only affordable if the single-phase
    /// inventory leaves room for it.
    #[test]
    fn measured_atlas_pressure_for_the_editor_type_inventory() {
        let mut a = loaded();
        // Every size in `typography::TextRole`, across all five cuts.
        let sizes = [11.0f32, 12.0, 13.0, 16.0, 22.0];
        let charset: String = (0x20u8..0x7F).map(|c| c as char).collect();

        for font_id in 0..CUTS.len() as u8 {
            for px in sizes {
                for ch in charset.chars() {
                    a.get_or_rasterize(ch, px, font_id);
                }
            }
        }

        let util = a.utilization();
        let glyphs = a.cached_glyph_count();
        println!(
            "atlas: {glyphs} glyphs, {:.1}% of {}x{} used, full={}",
            util * 100.0,
            a.width,
            a.height,
            a.is_full()
        );

        // The full Latin inventory must fit with room to spare — if this trips,
        // the shelf allocator or the atlas size needs revisiting before any new
        // face or size is added.
        assert!(
            !a.is_full(),
            "ASCII at five sizes across five cuts must fit"
        );
        assert!(util < 0.9, "utilization {util} leaves no headroom");

        // The 27-B decision, recorded as an assertion so it stays reproducible.
        //
        // Three-phase subpixel X positioning caches three bitmaps per glyph.
        // At the measured 47.7% single-phase footprint that is ~143% of a 10242
        // atlas -- it does not fit, and the failure mode is silent (a refused
        // glyph renders as a blank advance, not an error). So 27-B ships
        // integer-snapped glyph quads instead, which costs no extra atlas space
        // and removes the same bilinear smear for the common case.
        //
        // Subpixel phases stay blocked until the atlas grows to 2048^2 or the
        // shelf allocator learns to evict. Revisit there, not here.
        let phases_3x = util * 3.0;
        println!(
            "three-phase subpixel positioning would need {:.1}%",
            phases_3x * 100.0
        );
        assert!(
            phases_3x > 1.0,
            "three-phase subpixel positioning now fits ({:.1}%) -- revisit the              27-B decision to snap glyph quads instead",
            phases_3x * 100.0
        );
    }

    #[test]
    fn exhaustion_sets_the_full_flag_instead_of_failing_silently() {
        let mut a = loaded();
        // Rasterize progressively larger sizes until the shelf allocator refuses.
        let mut px = 64.0f32;
        while !a.is_full() && px < 4096.0 {
            for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
                if a.get_or_rasterize(ch, px, 0).is_none() {
                    break;
                }
            }
            px *= 1.5;
        }
        assert!(
            a.is_full(),
            "the allocator must eventually refuse and say so"
        );
    }

    #[test]
    fn measure_agrees_with_the_advance_the_draw_path_uses() {
        // `push_text_tracked` snaps the glyph quad but never the cursor, so
        // measured width must stay exactly advance + tracking per glyph.
        let a = loaded();
        let text = "Absorption mag.";
        let plain = a.measure_text(text, 13.0, 0).x;
        let tracked = a.measure_text_tracked(text, 13.0, 0, 0.5).x;
        let expected = plain + 0.5 * text.chars().count() as f32;
        assert!((tracked - expected).abs() < 0.001);
    }
}
