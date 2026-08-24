//! The second UI instance stream (Phase MORROWIND, MORROWIND-D, Seam 4b).
//!
//! # The rule this file exists to obey
//!
//! The 100-byte [`crate::primitive::Primitive`] is **frozen**. Phase 27 measured
//! 646 instances on the 1920x1080 shell against it, its twelve vertex attributes
//! are the Hades contract, and `assert!(size_of == 100)` is load-bearing.
//! MORROWIND-D adds a *second* stream with its own pipeline, drawn in the same
//! pass, ordered by the existing `draw_over` rule. It does not widen the first.
//!
//! Why not extend `Primitive` in place? Because a widened instance costs that
//! memory on **every flat fill**, and the shell is overwhelmingly flat fills.
//! Two streams cost one extra pipeline and one extra buffer, once.
//!
//! # Geometry is per-vertex; style is per-instance
//!
//! A rounded rect is one instance and no geometry — the quad pipeline evaluates
//! it analytically. A stroked bezier is a few hundred triangles, and there is no
//! analytic form. So the shaped stream splits:
//!
//! - [`ShapedInstance`] carries the 2x3 affine, the fills, the texture and mask
//!   slots and the flags. One per shape, uploaded as a storage buffer.
//! - [`ShapedVertex`] carries a local-space position, a UV and the index of the
//!   instance it belongs to. One per tessellated vertex, uploaded as a vertex
//!   buffer.
//!
//! The vertex stage looks its instance up and applies the transform, so a whole
//! run of shapes is one draw call regardless of how many shapes it contains.
//!
//! # Ordering
//!
//! **Do not bucket all quads then all shapes.** `DrawingContext` keeps one
//! ordered command list and each command records which stream it drew from; the
//! pass walks that list and switches pipeline when the stream changes. Bucketing
//! would reorder the shell and GHOSTFENCE's first row catches it immediately.

use crate::types::Rect;
use glam::Vec2;

/// Which of the two streams a [`crate::draw::DrawCommand`] draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stream {
    /// The frozen 100-byte `Primitive` pipeline. Every pre-MORROWIND widget.
    #[default]
    Quad,
    /// The shaped pipeline: transforms, paths, strokes, masks, textures.
    Shaped,
}

// ── flags ────────────────────────────────────────────────────────────────────

/// Fill is a linear gradient along `grad`. Without any gradient bit, `fill_a`
/// is used flat and `fill_b` is ignored.
pub const SHAPED_GRAD_LINEAR: u32 = 1 << 0;
/// Fill is a radial gradient from `grad.xy` outward to radius `grad.z`.
pub const SHAPED_GRAD_RADIAL: u32 = 1 << 1;
/// Fill is an angular (conic) gradient about `grad.xy`, starting at `grad.z`.
pub const SHAPED_GRAD_ANGULAR: u32 = 1 << 2;
/// The instance samples `texture` and multiplies it into the fill.
pub const SHAPED_TEXTURED: u32 = 1 << 3;
/// The sampled texture is a coverage mask in its alpha channel, not colour —
/// the same contract `FLAG_TEXT` has in the quad stream.
pub const SHAPED_COVERAGE: u32 = 1 << 4;

/// Texture slots the engine's own atlases occupy: font, icons, thumbnails.
///
/// These are the three fixed bindings MORROWIND-D replaces, kept at the same
/// indices so the existing bind group's *semantics* survive the change and no
/// call site has to be re-numbered.
pub const RESERVED_TEXTURE_SLOTS: u32 = 3;

/// Size of the bindless array, matching `ui_shaped.wgsl`.
///
/// Fixed rather than grown on demand: a binding array's length is part of the
/// bind-group layout, so growing it rebuilds the layout, the bind group and
/// both pipelines. 64 is well past what an editor plus a game HUD needs, and
/// the cost of an unused slot is one descriptor.
pub const MAX_TEXTURE_SLOTS: u32 = 64;

/// No texture. Chosen as `u32::MAX` rather than 0 because slot 0 is the font
/// atlas and a zero-initialised instance must not silently sample it.
pub const NO_TEXTURE: u32 = u32::MAX;
/// No mask beyond the inherited rectangular clip.
pub const NO_MASK: u32 = u32::MAX;

/// Per-shape style and transform.
///
/// `#[repr(C)]`, and **every member is 4-byte aligned on purpose**. This is a
/// storage-buffer array element, and WGSL rounds a struct's stride up to its
/// alignment: a single `vec4<f32>` member would align to 16 there and to 4
/// here, putting `grad` at a different offset in each language and making the
/// struct 80 bytes against 64. The WGSL mirror therefore spells both the affine
/// and the gradient as loose scalars, and
/// `tests/shaders_validate.rs::the_shaped_instance_struct_matches_the_rust_layout`
/// checks the two agree — it caught exactly that mismatch on the first run.
///
/// Getting this wrong decodes every instance after the first from the wrong
/// offset, which renders as "everything after the first shape is garbage" and
/// is the single most common way a struct mirror fails.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShapedInstance {
    /// Row-major 2x3 affine: `[a, b, c, d, tx, ty]`, applied as
    /// `(a*x + c*y + tx, b*x + d*y + ty)`. Identity is `[1, 0, 0, 1, 0, 0]`.
    pub xform: [f32; 6],
    /// Gradient parameters, meaning set by the `SHAPED_GRAD_*` bit:
    /// linear takes an axis in `xy`; radial takes a centre in `xy` and a radius
    /// in `z`; angular takes a centre in `xy` and a start angle in `z`.
    /// All in the shape's local space.
    pub grad: [f32; 4],
    /// Authored sRGB, straight alpha. The flat fill, and gradient stop A.
    pub fill_a: [u8; 4],
    /// Gradient stop B. Equal to `fill_a` for a flat fill.
    pub fill_b: [u8; 4],
    /// Bindless texture slot, or [`NO_TEXTURE`].
    pub texture: u32,
    /// Clip-mask slot, or [`NO_MASK`].
    pub mask: u32,
    /// `SHAPED_*` bits.
    pub flags: u32,
    /// Explicit, so the trailing word is visible on both sides rather than
    /// inferred by two compilers that might disagree about it.
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<ShapedInstance>() == 64);
const _: () = assert!(std::mem::size_of::<ShapedInstance>() % 16 == 0);

impl Default for ShapedInstance {
    fn default() -> Self {
        Self::identity([255, 255, 255, 255])
    }
}

impl ShapedInstance {
    /// An untransformed, untextured, unmasked flat fill.
    #[must_use]
    pub fn identity(fill: [u8; 4]) -> Self {
        Self {
            xform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            grad: [0.0; 4],
            fill_a: fill,
            fill_b: fill,
            texture: NO_TEXTURE,
            mask: NO_MASK,
            flags: 0,
            _pad: 0,
        }
    }

    /// Translate by `offset`.
    #[must_use]
    pub fn translated(mut self, offset: Vec2) -> Self {
        self.xform[4] += offset.x;
        self.xform[5] += offset.y;
        self
    }

    /// Rotate about `pivot` by `radians`, then keep the existing translation.
    ///
    /// Pivot-relative rather than about the origin, because every caller wants
    /// "spin this widget about its centre" and the origin form makes every one
    /// of them write the same three-line sandwich.
    #[must_use]
    pub fn rotated(mut self, radians: f32, pivot: Vec2) -> Self {
        let (sin, cos) = radians.sin_cos();
        let [a, b, c, d, tx, ty] = self.xform;
        // Compose: existing * translate(pivot) * rotate * translate(-pivot).
        let (ra, rb, rc, rd) = (cos, sin, -sin, cos);
        let na = a * ra + c * rb;
        let nb = b * ra + d * rb;
        let nc = a * rc + c * rd;
        let nd = b * rc + d * rd;
        // The pivot offset in the *pre-existing* frame.
        let ox = pivot.x - (ra * pivot.x + rc * pivot.y);
        let oy = pivot.y - (rb * pivot.x + rd * pivot.y);
        self.xform = [na, nb, nc, nd, tx + a * ox + c * oy, ty + b * ox + d * oy];
        self
    }

    /// Scale about the origin.
    #[must_use]
    pub fn scaled(mut self, scale: Vec2) -> Self {
        self.xform[0] *= scale.x;
        self.xform[1] *= scale.x;
        self.xform[2] *= scale.y;
        self.xform[3] *= scale.y;
        self
    }

    /// Apply this instance's transform to a local point.
    ///
    /// The CPU mirror of the vertex stage. MORROWIND-F hit-tests through it, so
    /// a pointer lands on the pixel it looks like it lands on rather than on the
    /// widget's untransformed bounds.
    #[must_use]
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let [a, b, c, d, tx, ty] = self.xform;
        Vec2::new(a * p.x + c * p.y + tx, b * p.x + d * p.y + ty)
    }

    /// Invert the transform, or `None` when it is singular.
    ///
    /// Hit-testing goes this way: a pointer in screen space becomes a point in
    /// the shape's local space, where the containment test is cheap. A zero
    /// scale is singular and must return `None` rather than a NaN that reports
    /// every hit test as a hit.
    #[must_use]
    pub fn invert(&self) -> Option<Self> {
        let [a, b, c, d, tx, ty] = self.xform;
        let det = a * d - b * c;
        if det.abs() < 1e-9 {
            return None;
        }
        let inv = 1.0 / det;
        let (ia, ib, ic, id) = (d * inv, -b * inv, -c * inv, a * inv);
        let mut out = *self;
        out.xform = [ia, ib, ic, id, -(ia * tx + ic * ty), -(ib * tx + id * ty)];
        Some(out)
    }

    /// A linear gradient along `axis`, in local space.
    #[must_use]
    pub fn with_linear_gradient(mut self, to: [u8; 4], axis: Vec2) -> Self {
        self.fill_b = to;
        self.grad = [axis.x, axis.y, 0.0, 0.0];
        self.flags |= SHAPED_GRAD_LINEAR;
        self
    }

    /// A radial gradient from `centre` out to `radius`, in local space.
    #[must_use]
    pub fn with_radial_gradient(mut self, to: [u8; 4], centre: Vec2, radius: f32) -> Self {
        self.fill_b = to;
        self.grad = [centre.x, centre.y, radius.max(1e-4), 0.0];
        self.flags |= SHAPED_GRAD_RADIAL;
        self
    }

    /// An angular (conic) gradient about `centre`, starting at `start` radians.
    #[must_use]
    pub fn with_angular_gradient(mut self, to: [u8; 4], centre: Vec2, start: f32) -> Self {
        self.fill_b = to;
        self.grad = [centre.x, centre.y, start, 0.0];
        self.flags |= SHAPED_GRAD_ANGULAR;
        self
    }

    /// Sample `slot`, multiplying it into the fill.
    #[must_use]
    pub fn with_texture(mut self, slot: u32) -> Self {
        self.texture = slot;
        self.flags |= SHAPED_TEXTURED;
        self
    }

    /// Sample `slot` as a coverage mask in alpha rather than as colour.
    #[must_use]
    pub fn with_coverage(mut self, slot: u32) -> Self {
        self.texture = slot;
        self.flags |= SHAPED_TEXTURED | SHAPED_COVERAGE;
        self
    }

    /// Multiply alpha by mask `slot`'s red channel.
    #[must_use]
    pub fn with_mask(mut self, slot: u32) -> Self {
        self.mask = slot;
        self
    }
}

/// One tessellated vertex.
///
/// 20 bytes, and `#[repr(C)]` over three 4-byte-aligned fields, so there is no
/// implicit padding and the `Pod` derive is sound.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShapedVertex {
    /// Position in the shape's local space, before the instance transform.
    pub pos: [f32; 2],
    /// Texture coordinate. Ignored without [`SHAPED_TEXTURED`].
    pub uv: [f32; 2],
    /// Index into the instance storage buffer.
    pub instance: u32,
}

const _: () = assert!(std::mem::size_of::<ShapedVertex>() == 20);

impl ShapedVertex {
    /// Vertex-buffer attribute layout. Step mode is `Vertex`, not `Instance` —
    /// the geometry *is* per-vertex here, which is the whole difference between
    /// this stream and the quad one.
    pub const VERTEX_ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x2, // pos      @  0
        1 => Float32x2, // uv       @  8
        2 => Uint32,    // instance @ 16
    ];

    /// Byte stride of one vertex.
    pub const STRIDE: wgpu::BufferAddress =
        std::mem::size_of::<ShapedVertex>() as wgpu::BufferAddress;
}

/// The per-frame shaped geometry: instances, vertices, and the flatten cache.
#[derive(Default)]
pub struct ShapedBuffers {
    /// One per shape, indexed by [`ShapedVertex::instance`].
    pub instances: Vec<ShapedInstance>,
    /// Tessellated triangles, three vertices each.
    pub vertices: Vec<ShapedVertex>,
}

impl ShapedBuffers {
    /// Drop the frame's geometry. The flatten cache is not held here and
    /// survives, which is the point of caching it.
    pub fn clear(&mut self) {
        self.instances.clear();
        self.vertices.clear();
    }

    /// Whether nothing was emitted this frame.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Push one shape: its style, and triangles in local space.
    ///
    /// `uv_from` maps local position to texture coordinate. `None` leaves UVs at
    /// zero, which is correct for an untextured shape and costs nothing.
    ///
    /// Returns the number of vertices appended, which is what the caller records
    /// in its [`crate::draw::DrawCommand`].
    pub fn push_shape(
        &mut self,
        instance: ShapedInstance,
        triangles: &[Vec2],
        uv_from: Option<Rect>,
    ) -> u32 {
        if triangles.len() < 3 {
            return 0;
        }
        let index = self.instances.len() as u32;
        self.instances.push(instance);
        self.vertices.reserve(triangles.len());
        for p in triangles {
            let uv = match uv_from {
                Some(r) if r.w.abs() > 1e-6 && r.h.abs() > 1e-6 => {
                    [(p.x - r.x) / r.w, (p.y - r.y) / r.h]
                }
                _ => [0.0, 0.0],
            };
            self.vertices.push(ShapedVertex {
                pos: [p.x, p.y],
                uv,
                instance: index,
            });
        }
        triangles.len() as u32
    }
}

impl ShapedBuffers {
    /// The topmost shape containing `point`, in screen space (MORROWIND-F).
    ///
    /// Walks in **reverse paint order**, so the answer is the shape a person
    /// can see rather than the first one drawn. Each candidate's transform is
    /// inverted to bring the point into the shape's local space, where the
    /// containment test is a point-in-triangle over geometry that is already
    /// there -- no second representation to keep in step with the drawn one.
    ///
    /// # What this does not test
    ///
    /// **The mask texture.** A masked shape's alpha lives on the GPU, and
    /// reading it back to answer a hit test would cost a stall per pointer
    /// move. So a circular avatar built from a rectangle plus an alpha mask
    /// hit-tests as its rectangle.
    ///
    /// That is a real limitation, stated rather than hidden. The fix where it
    /// matters is to build the shape as a *path* -- `Path::circle` and
    /// `push_path` -- which hit-tests exactly, because then the geometry is the
    /// shape. Masks are for the cases where that is impractical, and those are
    /// the cases where a rectangular hit region is usually fine.
    #[must_use]
    pub fn hit_test(&self, point: Vec2) -> Option<u32> {
        let mut vertex = self.vertices.len();
        while vertex >= 3 {
            vertex -= 3;
            let triangle = &self.vertices[vertex..vertex + 3];
            let index = triangle[0].instance;
            let Some(instance) = self.instances.get(index as usize) else {
                continue;
            };
            // A singular transform draws nothing, so it cannot be hit. Without
            // this the inverse is a NaN, every comparison is false, and the
            // widget reads as present but unclickable.
            let Some(inverse) = instance.invert() else {
                continue;
            };
            let local = inverse.apply(point);
            if point_in_triangle(
                local,
                Vec2::from(triangle[0].pos),
                Vec2::from(triangle[1].pos),
                Vec2::from(triangle[2].pos),
            ) {
                return Some(index);
            }
        }
        None
    }

    /// Every shape containing `point`, topmost first.
    ///
    /// For a caller that needs to walk past a shape which declines the hit: a
    /// transparent overlay, or a widget with hit testing switched off.
    #[must_use]
    pub fn hit_test_all(&self, point: Vec2) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut vertex = self.vertices.len();
        while vertex >= 3 {
            vertex -= 3;
            let triangle = &self.vertices[vertex..vertex + 3];
            let index = triangle[0].instance;
            let Some(instance) = self.instances.get(index as usize) else {
                continue;
            };
            let Some(inverse) = instance.invert() else {
                continue;
            };
            let local = inverse.apply(point);
            if point_in_triangle(
                local,
                Vec2::from(triangle[0].pos),
                Vec2::from(triangle[1].pos),
                Vec2::from(triangle[2].pos),
            ) && !out.contains(&index)
            {
                out.push(index);
            }
        }
        out
    }
}

/// Inclusive point-in-triangle, winding-agnostic.
///
/// Inclusive on the edges so a point exactly on the seam between the two
/// triangles of a quad hits rather than falling through. An exclusive test
/// there leaves a one-pixel dead line across the middle of every stroked
/// shape, which is maddening to diagnose.
fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let cross = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frozen_quad_instance_is_untouched() {
        // The whole premise of a second stream. If this ever fails, the
        // extension became a widening and Phase 27's 646-instance measurement
        // no longer describes the shell.
        assert_eq!(std::mem::size_of::<crate::primitive::Primitive>(), 100);
        assert_eq!(crate::primitive::Primitive::VERTEX_ATTRS.len(), 12);
    }

    #[test]
    fn identity_moves_nothing() {
        let inst = ShapedInstance::identity([1, 2, 3, 4]);
        let p = Vec2::new(7.0, -3.0);
        assert_eq!(inst.apply(p), p);
    }

    #[test]
    fn a_rotation_about_a_pivot_leaves_the_pivot_alone() {
        let pivot = Vec2::new(50.0, 20.0);
        let inst = ShapedInstance::identity([0; 4]).rotated(std::f32::consts::FRAC_PI_3, pivot);
        assert!(
            inst.apply(pivot).abs_diff_eq(pivot, 1e-3),
            "{:?}",
            inst.apply(pivot)
        );
    }

    #[test]
    fn a_quarter_turn_maps_x_to_y() {
        let inst =
            ShapedInstance::identity([0; 4]).rotated(std::f32::consts::FRAC_PI_2, Vec2::ZERO);
        assert!(inst.apply(Vec2::X).abs_diff_eq(Vec2::Y, 1e-5));
        assert!(inst.apply(Vec2::Y).abs_diff_eq(-Vec2::X, 1e-5));
    }

    #[test]
    fn transforms_compose_in_the_order_written() {
        let a = ShapedInstance::identity([0; 4])
            .translated(Vec2::new(10.0, 0.0))
            .scaled(Vec2::splat(2.0));
        // Scale applies in the local frame, so the translation is not scaled.
        assert_eq!(a.apply(Vec2::new(1.0, 0.0)), Vec2::new(12.0, 0.0));
    }

    /// Inversion round-trips, which is what MORROWIND-F's hit testing needs.
    #[test]
    fn inverting_a_transform_round_trips_a_point() {
        let inst = ShapedInstance::identity([0; 4])
            .translated(Vec2::new(30.0, -12.0))
            .rotated(0.7, Vec2::new(4.0, 4.0))
            .scaled(Vec2::new(2.0, 0.5));
        let inverse = inst.invert().expect("not singular");
        let p = Vec2::new(13.0, 5.0);
        assert!(inverse.apply(inst.apply(p)).abs_diff_eq(p, 1e-3));
    }

    /// A zero scale is singular and must say so.
    ///
    /// The alternative is a NaN transform, and a NaN comparison is false, which
    /// makes every hit test *miss* — a widget that is invisible and also
    /// unclickable, with nothing in the log.
    #[test]
    fn a_singular_transform_does_not_invert() {
        let flat = ShapedInstance::identity([0; 4]).scaled(Vec2::new(1.0, 0.0));
        assert!(flat.invert().is_none());
    }

    #[test]
    fn a_zeroed_instance_samples_no_texture() {
        // `Zeroable` means a zeroed instance is reachable. Slot 0 is the font
        // atlas, so a zero default would silently sample glyphs into every
        // untextured shape.
        let zeroed: ShapedInstance = bytemuck::Zeroable::zeroed();
        assert_eq!(zeroed.flags & SHAPED_TEXTURED, 0, "the flag gates the slot");
        assert_eq!(ShapedInstance::default().texture, NO_TEXTURE);
        assert_eq!(ShapedInstance::default().mask, NO_MASK);
    }

    #[test]
    fn pushing_a_shape_indexes_its_own_instance() {
        let mut buffers = ShapedBuffers::default();
        let tri = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        assert_eq!(buffers.push_shape(ShapedInstance::default(), &tri, None), 3);
        assert_eq!(buffers.push_shape(ShapedInstance::default(), &tri, None), 3);
        assert_eq!(buffers.instances.len(), 2);
        assert_eq!(buffers.vertices.len(), 6);
        assert_eq!(buffers.vertices[0].instance, 0);
        assert_eq!(buffers.vertices[3].instance, 1);
    }

    #[test]
    fn a_degenerate_shape_pushes_nothing() {
        let mut buffers = ShapedBuffers::default();
        assert_eq!(buffers.push_shape(ShapedInstance::default(), &[], None), 0);
        assert_eq!(
            buffers.push_shape(ShapedInstance::default(), &[Vec2::ZERO, Vec2::X], None),
            0,
            "two points is not a triangle, and a stray instance would still cost a slot"
        );
        assert!(buffers.instances.is_empty(), "no geometry, no instance");
    }

    #[test]
    fn uvs_normalise_against_the_given_rect() {
        let mut buffers = ShapedBuffers::default();
        let tri = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 20.0)];
        buffers.push_shape(
            ShapedInstance::default(),
            &tri,
            Some(Rect::new(0.0, 0.0, 10.0, 20.0)),
        );
        assert_eq!(buffers.vertices[0].uv, [0.0, 0.0]);
        assert_eq!(buffers.vertices[1].uv, [1.0, 0.0]);
        assert_eq!(buffers.vertices[2].uv, [1.0, 1.0]);
    }

    #[test]
    fn a_zero_sized_uv_rect_does_not_divide_by_zero() {
        let mut buffers = ShapedBuffers::default();
        let tri = [Vec2::ZERO, Vec2::X, Vec2::Y];
        buffers.push_shape(
            ShapedInstance::default(),
            &tri,
            Some(Rect::new(0.0, 0.0, 0.0, 0.0)),
        );
        assert!(buffers.vertices.iter().all(|v| v.uv == [0.0, 0.0]));
    }

    #[test]
    fn clearing_keeps_the_buffers_reusable() {
        let mut buffers = ShapedBuffers::default();
        let tri = [Vec2::ZERO, Vec2::X, Vec2::Y];
        buffers.push_shape(ShapedInstance::default(), &tri, None);
        buffers.clear();
        assert!(buffers.is_empty());
        assert_eq!(buffers.push_shape(ShapedInstance::default(), &tri, None), 3);
        assert_eq!(
            buffers.vertices[0].instance, 0,
            "indices restart with the frame"
        );
    }

    #[test]
    fn gradient_kinds_are_distinct_bits() {
        let base = ShapedInstance::identity([0; 4]);
        assert_ne!(
            base.with_linear_gradient([1; 4], Vec2::X).flags,
            base.with_radial_gradient([1; 4], Vec2::ZERO, 1.0).flags
        );
        assert_ne!(
            base.with_radial_gradient([1; 4], Vec2::ZERO, 1.0).flags,
            base.with_angular_gradient([1; 4], Vec2::ZERO, 0.0).flags
        );
    }

    #[test]
    fn a_radial_gradient_never_has_a_zero_radius() {
        // A zero radius is a division by zero in the fragment stage, and the
        // authored value for "a dot" is genuinely zero often enough to matter.
        let inst = ShapedInstance::identity([0; 4]).with_radial_gradient([1; 4], Vec2::ZERO, 0.0);
        assert!(inst.grad[2] > 0.0);
    }

    // -- MORROWIND-F: hit testing -------------------------------------------

    fn square(size: f32) -> [Vec2; 6] {
        [
            Vec2::ZERO,
            Vec2::new(size, 0.0),
            Vec2::new(size, size),
            Vec2::ZERO,
            Vec2::new(size, size),
            Vec2::new(0.0, size),
        ]
    }

    #[test]
    fn a_point_inside_an_untransformed_shape_hits() {
        let mut buffers = ShapedBuffers::default();
        buffers.push_shape(ShapedInstance::default(), &square(10.0), None);
        assert_eq!(buffers.hit_test(Vec2::new(5.0, 5.0)), Some(0));
        assert_eq!(buffers.hit_test(Vec2::new(15.0, 5.0)), None);
    }

    /// **The reason this exists.** A rotated shape is hit where it looks like
    /// it is, not where its untransformed bounds are.
    #[test]
    fn hit_testing_follows_the_transform() {
        let mut buffers = ShapedBuffers::default();
        let spun = ShapedInstance::identity([255; 4])
            .rotated(std::f32::consts::FRAC_PI_4, Vec2::splat(50.0))
            .translated(Vec2::new(100.0, 100.0));
        buffers.push_shape(spun, &square(100.0), None);

        // The shape's own centre, wherever the transform put it.
        let centre = spun.apply(Vec2::splat(50.0));
        assert_eq!(buffers.hit_test(centre), Some(0));

        // A corner of the *untransformed* bounds, which the rotation moved out
        // from under the pointer. A bounds-based hit test would say yes here.
        assert_eq!(
            buffers.hit_test(Vec2::new(100.0, 100.0)),
            None,
            "an untransformed-bounds hit test would wrongly hit here"
        );
    }

    /// The topmost shape wins, which is the one a person can see.
    #[test]
    fn hit_testing_walks_in_reverse_paint_order() {
        let mut buffers = ShapedBuffers::default();
        buffers.push_shape(ShapedInstance::identity([1; 4]), &square(50.0), None);
        buffers.push_shape(ShapedInstance::identity([2; 4]), &square(50.0), None);
        assert_eq!(buffers.hit_test(Vec2::splat(25.0)), Some(1));
        assert_eq!(buffers.hit_test_all(Vec2::splat(25.0)), vec![1, 0]);
    }

    /// A singular shape draws nothing and cannot be hit.
    #[test]
    fn a_singular_shape_is_not_hit() {
        let mut buffers = ShapedBuffers::default();
        buffers.push_shape(
            ShapedInstance::identity([1; 4]).scaled(Vec2::new(1.0, 0.0)),
            &square(50.0),
            None,
        );
        assert_eq!(buffers.hit_test(Vec2::splat(10.0)), None);
    }

    /// A point on the seam between a quad's two triangles still hits.
    #[test]
    fn the_diagonal_seam_of_a_quad_is_not_a_dead_line() {
        let mut buffers = ShapedBuffers::default();
        buffers.push_shape(ShapedInstance::default(), &square(10.0), None);
        for t in [0.25f32, 0.5, 0.75] {
            let on_seam = Vec2::splat(10.0 * t);
            assert_eq!(buffers.hit_test(on_seam), Some(0), "missed at t={t}");
        }
    }

    #[test]
    fn hit_testing_an_empty_frame_finds_nothing() {
        let buffers = ShapedBuffers::default();
        assert_eq!(buffers.hit_test(Vec2::ZERO), None);
        assert!(buffers.hit_test_all(Vec2::ZERO).is_empty());
    }
}
