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
    /// Physical pixels per logical pixel (HiDPI). Glyphs rasterize at
    /// `px * dpi_scale * SUPER_SAMPLE` and draw at logical `px`.
    pub dpi_scale: f32,

    // Shelf packing state
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

/// Raster larger than the layout size so 12–24 px Latin looks less crunchy.
/// Full SDF / cosmic-text shaping is the 26-H slip (see phase_26.md).
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
            dpi_scale: 1.0,
            cursor_x: 1, // 1px border to avoid UV bleeding
            cursor_y: 1,
            row_height: 0,
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

    pub fn set_dpi_scale(&mut self, scale: f32) {
        let scale = scale.clamp(1.0, 4.0);
        if (scale - self.dpi_scale).abs() > 0.01 {
            self.dpi_scale = scale;
        }
    }

    /// Get cached glyph or rasterize it into the atlas. Returns None if no font loaded or atlas full.
    pub fn get_or_rasterize(&mut self, ch: char, px: f32, font_id: u8) -> Option<GlyphInfo> {
        let raster_px = (px * self.dpi_scale * SUPER_SAMPLE).max(1.0);
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
            return None; // Atlas full — caller should handle gracefully
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
