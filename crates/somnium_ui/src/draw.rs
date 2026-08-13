// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/draw.rs
// Stripped of Fyrox font/material/brush systems. Somnium uses:
// - solid color rects (background/border)
// - textured quads for glyphs (font atlas at texture_id 0, Phase 12A-4)
// DrawingContext produces CPU-side vertex/index lists consumed by UiPass.

use crate::{
    font::{FONT_ATLAS_TEXTURE_ID, FontAtlas},
    icons::IconAtlas,
    types::Rect,
};
use glam::Vec2;

/// Per-vertex data uploaded to UiPass vertex buffer.
/// Layout must match the `UiVertex` WGSL struct in ui.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4], // RGBA, premultiplied alpha
}

impl Vertex {
    pub fn colored(pos: Vec2, color: [u8; 4]) -> Self {
        Self {
            pos: pos.into(),
            uv: [0.0, 0.0],
            color,
        }
    }
    pub fn textured(pos: Vec2, uv: Vec2, color: [u8; 4]) -> Self {
        Self {
            pos: pos.into(),
            uv: uv.into(),
            color,
        }
    }
}

/// References one draw call in UiPass.
/// All draws are alpha-blended. Texture = None means use solid color from vertex.
#[derive(Clone, Debug)]
pub struct DrawCommand {
    pub clip_rect: Rect,
    /// wgpu texture-view index into UiPass's texture array (None = white 1×1 pixel).
    pub texture_id: Option<u32>,
    pub index_offset: u32,
    pub index_count: u32,
}

/// Accumulated draw list for one UI frame.
///
/// Widgets call the helper methods during their `draw()` call. The renderer
/// then uploads `vertices` and `indices` to GPU and issues one draw per command.
/// `font_atlas` persists across frames (not cleared by `clear()`).
pub struct DrawingContext {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub commands: Vec<DrawCommand>,
    clip_stack: Vec<Rect>,
    current_clip: Rect,
    pub font_atlas: FontAtlas,
    pub icon_atlas: IconAtlas,
}

impl DrawingContext {
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        let root_clip = Rect::new(0.0, 0.0, screen_w, screen_h);
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            commands: Vec::new(),
            clip_stack: Vec::new(),
            current_clip: root_clip,
            font_atlas: FontAtlas::new(),
            icon_atlas: IconAtlas::new(),
        }
    }

    /// Clear per-frame geometry. Does NOT clear font_atlas — glyphs persist across frames.
    pub fn clear(&mut self, screen_w: f32, screen_h: f32) {
        self.vertices.clear();
        self.indices.clear();
        self.commands.clear();
        self.clip_stack.clear();
        self.current_clip = Rect::new(0.0, 0.0, screen_w, screen_h);
    }

    pub fn push_clip_rect(&mut self, rect: Rect) {
        self.clip_stack.push(self.current_clip);
        self.current_clip = self.current_clip.intersect(&rect);
    }

    pub fn pop_clip_rect(&mut self) {
        self.current_clip = self.clip_stack.pop().unwrap_or(self.current_clip);
    }

    fn begin_command(&mut self, texture_id: Option<u32>) {
        let idx_offset = self.indices.len() as u32;
        // Merge with previous command if same clip + texture.
        if let Some(last) = self.commands.last() {
            if last.clip_rect == self.current_clip && last.texture_id == texture_id {
                return; // keep extending last command
            }
        }
        self.commands.push(DrawCommand {
            clip_rect: self.current_clip,
            texture_id,
            index_offset: idx_offset,
            index_count: 0,
        });
    }

    fn push_indices(&mut self, a: u32, b: u32, c: u32) {
        self.indices.push(a);
        self.indices.push(b);
        self.indices.push(c);
        if let Some(cmd) = self.commands.last_mut() {
            cmd.index_count += 3;
        }
    }

    /// Solid-color filled rectangle.
    pub fn push_rect_filled(&mut self, rect: Rect, color: [u8; 4]) {
        self.begin_command(None);
        let base = self.vertices.len() as u32;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        self.vertices.extend_from_slice(&[
            Vertex::colored(Vec2::new(x, y), color),
            Vertex::colored(Vec2::new(x + w, y), color),
            Vertex::colored(Vec2::new(x + w, y + h), color),
            Vertex::colored(Vec2::new(x, y + h), color),
        ]);
        self.push_indices(base, base + 1, base + 2);
        self.push_indices(base + 2, base + 3, base);
    }

    /// Solid-color border (outline only).
    pub fn push_rect_border(&mut self, rect: Rect, thickness: f32, color: [u8; 4]) {
        let t = thickness;
        self.push_rect_filled(Rect::new(rect.x, rect.y, rect.w, t), color); // top
        self.push_rect_filled(Rect::new(rect.x, rect.y + rect.h - t, rect.w, t), color); // bottom
        self.push_rect_filled(Rect::new(rect.x, rect.y + t, t, rect.h - 2.0 * t), color); // left
        self.push_rect_filled(
            Rect::new(rect.x + rect.w - t, rect.y + t, t, rect.h - 2.0 * t),
            color,
        ); // right
    }

    /// Render a run of text using the font atlas.
    ///
    /// `origin` is the top-left corner of the text block (above the ascenders).
    /// Glyphs are rasterized on first use and cached in the atlas permanently.
    /// Emits `DrawCommand` entries with `texture_id = Some(FONT_ATLAS_TEXTURE_ID)`.
    pub fn push_text(&mut self, text: &str, origin: Vec2, font_id: u8, px: f32, color: [u8; 4]) {
        // Ascent: distance from top-of-line to baseline (positive).
        let ascent = self.font_atlas.ascent(px, font_id);
        let mut baseline_y = origin.y + ascent;
        let mut cursor_x = origin.x;

        for ch in text.chars() {
            if ch == '\n' {
                cursor_x = origin.x;
                baseline_y += self.font_atlas.measure_text("Ag", px, font_id).y.max(px);
                continue;
            }
            let Some(info) = self.font_atlas.get_or_rasterize(ch, px, font_id) else {
                cursor_x += px * 0.5;
                continue;
            };
            // Zero-size glyphs (space, etc.) — advance cursor only
            if info.px_w == 0.0 {
                cursor_x += info.advance;
                continue;
            }
            // Glyph top-left in screen space:
            //   x = cursor_x + xmin  (horizontal bearing)
            //   y = baseline_y - (ymin + px_h)  (freetype y-up → screen y-down)
            let gx = cursor_x + info.xmin;
            let gy = baseline_y - (info.ymin + info.px_h);
            let rect = Rect::new(gx, gy, info.px_w, info.px_h);
            let uv = [
                Vec2::new(info.uv_min[0], info.uv_min[1]), // TL
                Vec2::new(info.uv_max[0], info.uv_min[1]), // TR
                Vec2::new(info.uv_max[0], info.uv_max[1]), // BR
                Vec2::new(info.uv_min[0], info.uv_max[1]), // BL
            ];
            self.push_textured_rect(rect, uv, color, FONT_ATLAS_TEXTURE_ID);
            cursor_x += info.advance;
        }
    }

    /// Textured quad (used for image widgets and glyph quads).
    pub fn push_textured_rect(
        &mut self,
        rect: Rect,
        uv: [Vec2; 4],
        color: [u8; 4],
        texture_id: u32,
    ) {
        self.begin_command(Some(texture_id));
        let base = self.vertices.len() as u32;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        self.vertices.extend_from_slice(&[
            Vertex::textured(Vec2::new(x, y), uv[0], color),
            Vertex::textured(Vec2::new(x + w, y), uv[1], color),
            Vertex::textured(Vec2::new(x + w, y + h), uv[2], color),
            Vertex::textured(Vec2::new(x, y + h), uv[3], color),
        ]);
        self.push_indices(base, base + 1, base + 2);
        self.push_indices(base + 2, base + 3, base);
    }

    /// 9-slice: corners stay unscaled, edges stretch on one axis, center tiles.
    /// `slice` is the inset from each edge of `src` (UV space 0..1 of the bound texture).
    pub fn push_nine_slice(&mut self, dest: Rect, texture_id: u32, slice: f32, color: [u8; 4]) {
        let s = slice.clamp(0.0, 0.49);
        let dw = dest.w.max(1.0);
        let dh = dest.h.max(1.0);
        let cx = (s * dw).min(dw * 0.45);
        let cy = (s * dh).min(dh * 0.45);
        let xs = [dest.x, dest.x + cx, dest.x + dw - cx, dest.x + dw];
        let ys = [dest.y, dest.y + cy, dest.y + dh - cy, dest.y + dh];
        let us = [0.0, s, 1.0 - s, 1.0];
        let vs = [0.0, s, 1.0 - s, 1.0];
        for row in 0..3 {
            for col in 0..3 {
                let r = Rect::new(
                    xs[col],
                    ys[row],
                    xs[col + 1] - xs[col],
                    ys[row + 1] - ys[row],
                );
                if r.w <= 0.0 || r.h <= 0.0 {
                    continue;
                }
                let uv = [
                    Vec2::new(us[col], vs[row]),
                    Vec2::new(us[col + 1], vs[row]),
                    Vec2::new(us[col + 1], vs[row + 1]),
                    Vec2::new(us[col], vs[row + 1]),
                ];
                self.push_textured_rect(r, uv, color, texture_id);
            }
        }
    }
}
