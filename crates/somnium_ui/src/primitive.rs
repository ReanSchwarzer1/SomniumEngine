//! Phase 27-A (Styx) — the primitive quad.
//!
//! Before Styx, [`crate::draw::DrawingContext`] could emit exactly two shapes:
//! an axis-aligned solid quad and a textured quad. Corner radius, antialiasing,
//! gradients, real shadows and glow were all inexpressible, which is why
//! `theme::GeometryTokens::radius_*` was declared, threaded through
//! [`crate::style::Paint`], and never drawn by anything.
//!
//! A `Primitive` is one instance of the single UI pipeline. The vertex stage
//! generates a unit quad and expands it by [`Primitive::expand`]; the fragment
//! stage evaluates a signed distance field for the rounded box and derives
//! fill, gradient, border, shadow and glow analytically from that one distance.
//! Antialiasing comes from `fwidth` on the distance — there is no MSAA and no
//! supersampled target.
//!
//! # Colour contract
//!
//! Every colour field is **authored sRGB bytes with straight (unassociated)
//! alpha**, exactly as before Styx. `ui_pass.wgsl` decodes sRGB to linear once,
//! at the top of the fragment stage, before any gradient interpolation and
//! before the blend. Nothing else in the pipeline encodes or decodes. This is
//! the Phase 26-Zeta-B contract and Styx does not renegotiate it — see
//! `dev records/phase_27.md` §6.2.

use crate::types::Rect;

/// Sample the bound texture and multiply into the fill.
pub const FLAG_TEXTURED: u32 = 1 << 0;
/// The bound texture is a coverage mask (font/icon atlas), not colour. Applies
/// the text contrast correction described in [`crate::font`].
pub const FLAG_TEXT: u32 = 1 << 1;
/// This instance paints a shadow cast *by* `rect`, not `rect` itself.
pub const FLAG_SHADOW: u32 = 1 << 2;
/// Additive outer band, used only by the focus ring.
pub const FLAG_GLOW: u32 = 1 << 3;
/// Shadow is drawn inside the shape (input recession) rather than outside.
pub const FLAG_INSET: u32 = 1 << 4;
/// Interpolate `fill_a` → `fill_b` along `grad_axis`.
pub const FLAG_GRADIENT: u32 = 1 << 5;

/// Bit offset of the texture-layer selector inside [`Primitive::flags`].
///
/// Pre-Styx the bound texture lived on `DrawCommand`, so every alternation
/// between a panel fill, a label and an icon broke the batch — the real shell
/// measured 164 draw calls at 1920x1080 for 625 quads. Carrying the selector
/// per instance instead lets one bind group serve every atlas, so batches now
/// break only on a clip-rect change.
pub const TEX_SHIFT: u32 = 8;
pub const TEX_MASK: u32 = 0xFFu32 << TEX_SHIFT;

/// Extra quad margin for a plain fill, so the antialiasing ramp on the outer
/// half of the edge is not clipped by the quad itself.
pub const AA_PAD: f32 = 1.0;

/// One instance of the UI pipeline.
///
/// `#[repr(C)]` with every field aligned to 4 bytes and a total size that is a
/// multiple of 4, so there is no implicit padding and the `Pod` derive is
/// sound. Field order matches the `@location` bindings declared by
/// [`Primitive::VERTEX_ATTRS`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Primitive {
    /// `x, y, w, h` of the logical shape, before [`Self::expand`].
    pub rect: [f32; 4],
    /// `u0, v0, u1, v1` in the bound texture. Ignored without `FLAG_TEXTURED`.
    pub uv: [f32; 4],
    /// Corner radii, clockwise from top-left: `tl, tr, br, bl`.
    pub radii: [f32; 4],
    /// `offset_x, offset_y, blur, spread`. Ignored without `FLAG_SHADOW`.
    pub shadow: [f32; 4],
    /// Unit gradient direction in normalised rect space. Ignored without
    /// `FLAG_GRADIENT`.
    pub grad_axis: [f32; 2],
    /// Inset border stroke width in pixels. Zero disables the border band.
    pub border_width: f32,
    /// How far the drawn quad grows beyond `rect` on every side. Fills use
    /// [`AA_PAD`]; shadows and glows need blur plus spread plus offset.
    pub expand: f32,
    /// Authored sRGB, straight alpha.
    pub fill_a: [u8; 4],
    /// Gradient stop B. Equal to `fill_a` for a flat fill.
    pub fill_b: [u8; 4],
    pub border_color: [u8; 4],
    pub shadow_color: [u8; 4],
    pub flags: u32,
}

const _: () = assert!(std::mem::size_of::<Primitive>() == 100);

impl Primitive {
    /// Vertex-buffer attribute layout. Step mode is `Instance`; the unit quad
    /// itself is generated from `@builtin(vertex_index)` and needs no buffer.
    pub const VERTEX_ATTRS: [wgpu::VertexAttribute; 12] = wgpu::vertex_attr_array![
        0  => Float32x4, // rect          @  0
        1  => Float32x4, // uv            @ 16
        2  => Float32x4, // radii         @ 32
        3  => Float32x4, // shadow        @ 48
        4  => Float32x2, // grad_axis     @ 64
        5  => Float32,   // border_width  @ 72
        6  => Float32,   // expand        @ 76
        7  => Unorm8x4,  // fill_a        @ 80
        8  => Unorm8x4,  // fill_b        @ 84
        9  => Unorm8x4,  // border_color  @ 88
        10 => Unorm8x4,  // shadow_color  @ 92
        11 => Uint32,    // flags         @ 96
    ];

    /// Byte stride of one instance. Must equal `size_of::<Primitive>()`.
    pub const STRIDE: wgpu::BufferAddress = std::mem::size_of::<Primitive>() as wgpu::BufferAddress;

    /// A flat, square-cornered, opaque fill — the exact shape every pre-Styx
    /// `push_rect_filled` produced. Kept as the base for every other
    /// constructor so a regression here shows up in one place.
    pub fn fill(rect: Rect, color: [u8; 4]) -> Self {
        Self {
            rect: [rect.x, rect.y, rect.w, rect.h],
            uv: [0.0, 0.0, 1.0, 1.0],
            radii: [0.0; 4],
            shadow: [0.0; 4],
            grad_axis: [0.0, 0.0],
            border_width: 0.0,
            expand: AA_PAD,
            fill_a: color,
            fill_b: color,
            border_color: [0, 0, 0, 0],
            shadow_color: [0, 0, 0, 0],
            flags: 0,
        }
    }

    /// Uniform corner radius on all four corners.
    pub fn with_radius(mut self, r: f32) -> Self {
        let r = r.max(0.0);
        self.radii = [r; 4];
        self
    }

    /// Per-corner radii, clockwise from top-left.
    pub fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radii = radii.map(|r| r.max(0.0));
        self
    }

    /// Inset stroke. A zero or negative width clears the border.
    pub fn with_border(mut self, width: f32, color: [u8; 4]) -> Self {
        if width > 0.0 {
            self.border_width = width;
            self.border_color = color;
        } else {
            self.border_width = 0.0;
        }
        self
    }

    /// Two-stop linear gradient. `axis` is a direction in normalised rect
    /// space — `(0, 1)` is the top-to-bottom wash the chrome tokens use.
    /// Interpolation happens in linear space inside the shader, never here.
    pub fn with_gradient(mut self, to: [u8; 4], axis: [f32; 2]) -> Self {
        self.fill_b = to;
        self.grad_axis = axis;
        self.flags |= FLAG_GRADIENT;
        self
    }

    /// Which atlas this instance samples. Values match the historical
    /// `texture_id` constants: 0 is the font atlas, 1 the icon atlas.
    pub fn texture_layer(&self) -> u32 {
        (self.flags & TEX_MASK) >> TEX_SHIFT
    }

    /// Select the atlas sampled when `FLAG_TEXTURED` is set.
    pub fn with_texture_layer(mut self, layer: u32) -> Self {
        self.flags = (self.flags & !TEX_MASK) | ((layer << TEX_SHIFT) & TEX_MASK);
        self
    }

    /// Textured quad. `uv` is `[u0, v0, u1, v1]`.
    pub fn textured(rect: Rect, uv: [f32; 4], color: [u8; 4]) -> Self {
        let mut p = Self::fill(rect, color);
        p.uv = uv;
        p.flags |= FLAG_TEXTURED;
        // A textured quad samples an atlas cell whose edges are the quad edges;
        // growing it would sample outside the cell, so it gets no AA margin.
        p.expand = 0.0;
        p
    }

    /// Glyph quad from the font atlas: a coverage mask tinted by `color`.
    pub fn glyph(rect: Rect, uv: [f32; 4], color: [u8; 4]) -> Self {
        let mut p = Self::textured(rect, uv, color);
        p.flags |= FLAG_TEXT;
        p
    }

    /// Shadow cast by `rect`. Drawn as its own instance *behind* the caster,
    /// which is why `fill_a` is transparent — only the shadow band paints.
    pub fn shadow(
        rect: Rect,
        radii: [f32; 4],
        offset: [f32; 2],
        blur: f32,
        spread: f32,
        color: [u8; 4],
    ) -> Self {
        let blur = blur.max(0.0);
        let spread = spread.max(0.0);
        let mut p = Self::fill(rect, [0, 0, 0, 0]);
        p.radii = radii.map(|r| r.max(0.0));
        p.shadow = [offset[0], offset[1], blur, spread];
        p.shadow_color = color;
        p.flags |= FLAG_SHADOW;
        p.expand = blur + spread + offset[0].abs().max(offset[1].abs()) + AA_PAD;
        p
    }

    /// Outer additive band. Used by the focus ring and by nothing else —
    /// `dev records/phase_27.md` §5.4 caps glow at two roles.
    pub fn glow(rect: Rect, radii: [f32; 4], radius: f32, color: [u8; 4]) -> Self {
        let radius = radius.max(0.0);
        let mut p = Self::fill(rect, [0, 0, 0, 0]);
        p.radii = radii.map(|r| r.max(0.0));
        p.shadow = [0.0, 0.0, radius, 0.0];
        p.shadow_color = color;
        p.flags |= FLAG_SHADOW | FLAG_GLOW;
        p.expand = radius + AA_PAD;
        p
    }

    /// Recessed inner shadow, used to sink input fields below their panel.
    pub fn inset_shadow(rect: Rect, radii: [f32; 4], blur: f32, color: [u8; 4]) -> Self {
        let mut p = Self::fill(rect, [0, 0, 0, 0]);
        p.radii = radii.map(|r| r.max(0.0));
        p.shadow = [0.0, 1.0, blur.max(0.0), 0.0];
        p.shadow_color = color;
        p.flags |= FLAG_SHADOW | FLAG_INSET;
        // Inset shadows paint strictly inside the shape.
        p.expand = AA_PAD;
        p
    }

    /// Axis-aligned bounds actually covered by the drawn quad, including
    /// [`Self::expand`]. Used for clip-rect culling.
    pub fn drawn_bounds(&self) -> Rect {
        let e = self.expand;
        Rect::new(
            self.rect[0] - e,
            self.rect[1] - e,
            self.rect[2] + e * 2.0,
            self.rect[3] + e * 2.0,
        )
    }

    /// True when this instance is a plain flat opaque-shaped fill — the
    /// pre-Styx shape. The golden test in [`crate::draw`] uses this to assert
    /// that the compatibility shims did not gain behaviour.
    pub fn is_plain_fill(&self) -> bool {
        self.flags == 0
            && self.radii == [0.0; 4]
            && self.border_width == 0.0
            && self.fill_a == self.fill_b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_has_no_implicit_padding() {
        // `bytemuck::Pod` is only sound without padding bytes. Sum the fields.
        let field_bytes = 16 + 16 + 16 + 16 + 8 + 4 + 4 + 4 + 4 + 4 + 4 + 4;
        assert_eq!(std::mem::size_of::<Primitive>(), field_bytes);
        assert_eq!(std::mem::align_of::<Primitive>(), 4);
    }

    #[test]
    fn vertex_attrs_cover_every_field_in_declaration_order() {
        // Every field must have exactly one attribute at its real offset,
        // or the shader silently reads one field as another. `shadow_color`
        // at 92 was missing from the first draft of this array.
        let expected = [0, 16, 32, 48, 64, 72, 76, 80, 84, 88, 92, 96];
        let actual: Vec<u64> = Primitive::VERTEX_ATTRS.iter().map(|a| a.offset).collect();
        assert_eq!(actual, expected);

        // The last attribute plus its size must exactly fill the stride, which
        // is what proves no field was skipped at the end.
        assert_eq!(Primitive::STRIDE, 100);
        assert_eq!(actual.last().copied().unwrap() + 4, Primitive::STRIDE);
    }

    #[test]
    fn shader_locations_are_dense_and_ascending() {
        for (i, attr) in Primitive::VERTEX_ATTRS.iter().enumerate() {
            assert_eq!(attr.shader_location, i as u32);
        }
    }

    #[test]
    fn plain_fill_matches_the_pre_styx_shape() {
        let p = Primitive::fill(Rect::new(1.0, 2.0, 3.0, 4.0), [10, 20, 30, 255]);
        assert!(p.is_plain_fill());
        assert_eq!(p.expand, AA_PAD);
    }

    #[test]
    fn radius_and_border_are_clamped_non_negative() {
        let p = Primitive::fill(Rect::new(0.0, 0.0, 8.0, 8.0), [0; 4])
            .with_radius(-4.0)
            .with_border(-1.0, [255; 4]);
        assert_eq!(p.radii, [0.0; 4]);
        assert_eq!(p.border_width, 0.0);
    }

    #[test]
    fn shadow_quad_grows_to_cover_blur_spread_and_offset() {
        let p = Primitive::shadow(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            [4.0; 4],
            [0.0, 6.0],
            12.0,
            2.0,
            [0, 0, 0, 128],
        );
        assert_eq!(p.expand, 12.0 + 2.0 + 6.0 + AA_PAD);
        let b = p.drawn_bounds();
        assert!(b.w >= 10.0 + 2.0 * (12.0 + 2.0 + 6.0));
    }

    #[test]
    fn textured_quads_get_no_aa_margin() {
        // Growing a glyph quad would sample neighbouring atlas cells.
        let p = Primitive::glyph(
            Rect::new(0.0, 0.0, 4.0, 6.0),
            [0.0, 0.0, 0.1, 0.1],
            [255; 4],
        );
        assert_eq!(p.expand, 0.0);
        assert_ne!(p.flags & FLAG_TEXT, 0);
        assert_ne!(p.flags & FLAG_TEXTURED, 0);
    }
}
