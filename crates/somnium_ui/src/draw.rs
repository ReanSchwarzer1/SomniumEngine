// Originally a port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/draw.rs.
//
// Phase 27-A (Styx) replaced the vertex/index draw list with an instance list.
// Every shape the UI can draw is now one `Primitive` (see `primitive.rs`), and
// `UiPass` issues one instanced draw per `DrawCommand`. The six historical
// `push_*` entry points survive unchanged as thin constructors over that
// instance, which is what lets the ~86 widget call sites migrate to radius,
// gradients and real shadows one recipe at a time instead of all at once
// (`dev records/phase_27.md` §6.4).

use crate::{
    font::{FONT_ATLAS_TEXTURE_ID, FontAtlas},
    icons::IconAtlas,
    primitive::Primitive,
    types::Rect,
};
use glam::Vec2;

/// One instanced draw: a contiguous run of primitives sharing a clip rect.
///
/// The bound texture is no longer part of the key. Every atlas is bound at once
/// and selected per instance through [`Primitive::texture_layer`], so a run of
/// panel fills, labels and icons inside one clip region is a single draw.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawCommand {
    pub clip_rect: Rect,
    /// Which stream the run comes from (MORROWIND-D).
    ///
    /// **This is what lets `draw_over` survive two pipelines.** The command
    /// list stays in paint order and the pass switches pipeline when the stream
    /// changes; it does not bucket all quads and then all shapes, which would
    /// reorder the shell.
    pub stream: crate::shaped::Stream,
    /// For [`Stream::Quad`](crate::shaped::Stream::Quad), the first instance.
    /// For [`Stream::Shaped`](crate::shaped::Stream::Shaped), the first
    /// *vertex* — shaped geometry is per-vertex, because a stroked bezier has
    /// no analytic form to expand from one instance.
    pub instance_offset: u32,
    /// Instances for `Quad`, vertices for `Shaped`. See `instance_offset`.
    pub instance_count: u32,
}

/// Accumulated draw list for one UI frame.
///
/// Widgets call the helper methods during their `draw()` call. `UiPass` then
/// uploads `instances` and issues one instanced draw per command. `font_atlas`
/// and `icon_atlas` persist across frames (not cleared by `clear()`).
pub struct DrawingContext {
    /// The frame's primitive instances, in paint order.
    pub instances: Vec<Primitive>,
    pub commands: Vec<DrawCommand>,
    clip_stack: Vec<Rect>,
    current_clip: Rect,
    pub font_atlas: FontAtlas,
    pub icon_atlas: IconAtlas,
    /// Phase 27-C. Lives here rather than on `UserInterface` because a widget
    /// receives `&mut DrawingContext` in `draw()` and nothing else — which is
    /// also the moment it knows its own interaction state, so it can both read
    /// a wash value and retarget the track in one place.
    pub motion: crate::motion::Animator,
    /// Asset previews for the Content Drawer. Lives beside the atlases because
    /// it *is* one, and because widgets reach it the same way.
    pub thumbnails: crate::thumbnail::ThumbnailCache,
    /// MORROWIND-D. The second stream: transforms, paths, strokes, masks.
    pub shaped: crate::shaped::ShapedBuffers,
    /// Transform stack applied to shaped pushes.
    ///
    /// Quad primitives are **not** transformed: the frozen 100-byte instance
    /// has no transform field, which is §4.5's finding restated. A widget that
    /// wants to rotate emits shaped geometry, and that is the whole reason this
    /// stream exists.
    transform_stack: Vec<crate::shaped::ShapedInstance>,
    /// Flattened contours, keyed by path and tolerance.
    ///
    /// A node graph's wires do not change shape while the user pans, and
    /// re-flattening them every frame is the obvious performance mistake. The
    /// cache survives `clear()`; the geometry it produced does not.
    flatten_cache: std::collections::HashMap<FlattenKey, std::rc::Rc<Vec<crate::path::Contour>>>,
    /// How many textures a game has registered, beyond the three atlases.
    registered_textures: u32,
    /// Device-pixel flattening tolerance. Recomputed on a DPI change.
    tolerance: f32,
}

/// Cache key for a flattened path.
///
/// The tolerance is quantised to a hundredth of a pixel and stored as an
/// integer, because `f32` is not `Hash` and because a tolerance that differs in
/// the seventh decimal is the same flattening. Rounding it into the key is what
/// stops a DPI scale that wobbles by a rounding error from missing every frame.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FlattenKey {
    path: crate::path::Path,
    tolerance_centi: u32,
}

impl DrawingContext {
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        let root_clip = Rect::new(0.0, 0.0, screen_w, screen_h);
        Self {
            instances: Vec::new(),
            commands: Vec::new(),
            clip_stack: Vec::new(),
            current_clip: root_clip,
            font_atlas: FontAtlas::new(),
            icon_atlas: IconAtlas::new(),
            motion: crate::motion::Animator::new(),
            thumbnails: crate::thumbnail::ThumbnailCache::new(),
            shaped: crate::shaped::ShapedBuffers::default(),
            transform_stack: Vec::new(),
            flatten_cache: std::collections::HashMap::new(),
            registered_textures: 0,
            tolerance: crate::path::DEFAULT_TOLERANCE,
        }
    }

    /// Clear per-frame geometry. Does NOT clear the atlases or the animator —
    /// glyphs, icons and in-flight tracks all persist across frames.
    pub fn clear(&mut self, screen_w: f32, screen_h: f32) {
        self.instances.clear();
        self.commands.clear();
        self.clip_stack.clear();
        self.current_clip = Rect::new(0.0, 0.0, screen_w, screen_h);
        self.shaped.clear();
        self.transform_stack.clear();
    }

    /// The clip a widget is currently drawing inside.
    ///
    /// MORROWIND-M: a widget in a scroll viewer is as tall as its content, so
    /// its own bounds say nothing about what can be seen. This is what a long
    /// list has to consult in order to draw only the rows that are visible.
    #[must_use]
    pub fn clip_rect(&self) -> Rect {
        self.current_clip
    }

    pub fn push_clip_rect(&mut self, rect: Rect) {
        self.clip_stack.push(self.current_clip);
        self.current_clip = self.current_clip.intersect(&rect);
    }

    pub fn pop_clip_rect(&mut self) {
        self.current_clip = self.clip_stack.pop().unwrap_or(self.current_clip);
    }

    /// Number of instances emitted this frame. Used by the Phase 27-I
    /// performance harness and by the idle-frame stability test.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    fn begin_command(&mut self) {
        self.begin_command_in(crate::shaped::Stream::Quad);
    }

    /// Open or extend a command in `stream`.
    ///
    /// Merges with the previous command only when **both** the clip region and
    /// the stream match. Merging across streams would put shaped vertices and
    /// quad instances in one run, and the pass has no way to draw that.
    fn begin_command_in(&mut self, stream: crate::shaped::Stream) {
        if let Some(last) = self.commands.last()
            && last.clip_rect == self.current_clip
            && last.stream == stream
        {
            return;
        }
        let offset = match stream {
            crate::shaped::Stream::Quad => self.instances.len() as u32,
            crate::shaped::Stream::Shaped => self.shaped.vertices.len() as u32,
        };
        self.commands.push(DrawCommand {
            clip_rect: self.current_clip,
            stream,
            instance_offset: offset,
            instance_count: 0,
        });
    }

    /// The native Styx entry point. Every other `push_*` builds on this.
    ///
    /// `texture_id` selects the atlas sampled when the primitive carries
    /// `FLAG_TEXTURED`; it no longer breaks the batch.
    pub fn push_primitive(&mut self, primitive: Primitive, texture_id: Option<u32>) {
        self.begin_command();
        let primitive = match texture_id {
            Some(id) => primitive.with_texture_layer(id),
            None => primitive,
        };
        self.instances.push(primitive);
        if let Some(cmd) = self.commands.last_mut() {
            cmd.instance_count += 1;
        }
    }

    // -- MORROWIND-D: the shaped stream ---------------------------------------
    //
    // Seam 4b's authoring surface. Everything below emits into `self.shaped`,
    // in paint order, interleaved with the quad stream.

    /// Set the flattening tolerance, in **device** pixels.
    ///
    /// Call this when DPI changes. Phase 27 already fixed a DPI correctness bug
    /// and the plan is explicit that it must not be reintroduced; the flatten
    /// cache is keyed by tolerance, so changing it invalidates exactly the
    /// entries that need it and keeps the rest.
    pub fn set_tolerance(&mut self, device_pixels: f32) {
        self.tolerance = device_pixels.clamp(0.05, 8.0);
    }

    /// The current flattening tolerance, in device pixels.
    #[must_use]
    pub fn tolerance(&self) -> f32 {
        self.tolerance
    }

    /// Register a texture and get the slot a shaped instance names.
    ///
    /// Slots 0, 1 and 2 are the font, icon and thumbnail atlases -- the three
    /// fixed bindings this replaces -- so existing call sites keep their
    /// meaning. Games get 3 and up.
    ///
    /// Returns `None` when the array is full rather than wrapping to a slot
    /// somebody else owns: a silently reused slot renders one game's sprite
    /// where another expects its own, which is a bug nobody would look for in
    /// the UI layer.
    pub fn register_texture(&mut self) -> Option<u32> {
        let slot = crate::shaped::RESERVED_TEXTURE_SLOTS + self.registered_textures;
        if slot >= crate::shaped::MAX_TEXTURE_SLOTS {
            return None;
        }
        self.registered_textures += 1;
        Some(slot)
    }

    /// How many texture slots a game has taken.
    #[must_use]
    pub fn registered_texture_count(&self) -> u32 {
        self.registered_textures
    }

    /// Push a transform, applying to every shaped push until [`Self::pop_transform`].
    ///
    /// Composes with whatever is already on the stack, so nesting works the way
    /// a scene graph does: a rotated panel containing a scaled icon gets both.
    ///
    /// **Quad primitives are unaffected.** The frozen 100-byte instance has no
    /// transform field, which is exactly the plan's §4.5 finding; a widget that
    /// wants to rotate emits shaped geometry, and that is why this stream exists.
    pub fn push_transformed(&mut self, transform: crate::shaped::ShapedInstance) {
        let composed = match self.transform_stack.last() {
            Some(parent) => compose(parent, &transform),
            None => transform,
        };
        self.transform_stack.push(composed);
    }

    /// Pop the innermost transform.
    pub fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    /// The transform a shaped push would currently receive.
    #[must_use]
    pub fn current_transform(&self) -> crate::shaped::ShapedInstance {
        self.transform_stack
            .last()
            .copied()
            .unwrap_or_else(|| crate::shaped::ShapedInstance::identity([255, 255, 255, 255]))
    }

    /// Flatten `path` at the current tolerance, reusing a cached result.
    ///
    /// Public because MORROWIND-F hit-tests against the same contours it draws,
    /// and re-flattening for the hit test would be both wasteful and a source of
    /// disagreement between what is drawn and what is clickable.
    pub fn flatten(&mut self, path: &crate::path::Path) -> std::rc::Rc<Vec<crate::path::Contour>> {
        let key = FlattenKey {
            path: path.clone(),
            tolerance_centi: (self.tolerance * 100.0).round() as u32,
        };
        if let Some(hit) = self.flatten_cache.get(&key) {
            return std::rc::Rc::clone(hit);
        }
        let contours = std::rc::Rc::new(path.flatten(self.tolerance));
        self.flatten_cache
            .insert(key, std::rc::Rc::clone(&contours));
        contours
    }

    /// Stroke a path.
    ///
    /// The node graph's wires, the timeline's curves, the spline editor's
    /// handles and every dashed selection outline are this call.
    pub fn push_stroke(
        &mut self,
        path: &crate::path::Path,
        stroke: &crate::path::Stroke,
        style: crate::shaped::ShapedInstance,
    ) {
        let contours = self.flatten(path);
        let mut triangles = crate::path::Triangles::new();
        for contour in contours.iter() {
            triangles.extend(crate::path::stroke_contour(contour, stroke));
        }
        self.push_shaped(style, &triangles);
    }

    /// Fill a path's closed contours.
    ///
    /// A self-intersecting contour draws nothing rather than overlapping
    /// triangles -- see [`crate::path::fill_contour`].
    pub fn push_path(&mut self, path: &crate::path::Path, style: crate::shaped::ShapedInstance) {
        let contours = self.flatten(path);
        let mut triangles = crate::path::Triangles::new();
        for contour in contours.iter() {
            triangles.extend(crate::path::fill_contour(contour));
        }
        self.push_shaped(style, &triangles);
    }

    /// Draw an already-tessellated shape.
    ///
    /// The primitive every other shaped entry point builds on, exposed because
    /// a caller that already has triangles -- a chart, a mesh preview, a
    /// generated gizmo -- should not have to round-trip through a `Path`.
    ///
    /// UVs are normalised over the geometry's own bounds, and only computed
    /// when the shape actually samples something, so an untextured stroke pays
    /// nothing for the convenience.
    pub fn push_shaped(&mut self, style: crate::shaped::ShapedInstance, triangles: &[Vec2]) {
        if triangles.len() < 3 {
            return;
        }
        let style = match self.transform_stack.last() {
            Some(parent) => compose(parent, &style),
            None => style,
        };
        let samples = style.flags & crate::shaped::SHAPED_TEXTURED != 0
            || style.mask != crate::shaped::NO_MASK;
        let uv_from = samples.then(|| crate::path::bounds(triangles));
        self.begin_command_in(crate::shaped::Stream::Shaped);
        let count = self.shaped.push_shape(style, triangles, uv_from);
        if let Some(cmd) = self.commands.last_mut() {
            cmd.instance_count += count;
        }
    }

    /// Clip a shaped style to a mask texture.
    ///
    /// The rectangular clip stack still applies and is still the cheap path;
    /// this is the circular-avatar, rounded-panel and masked-reveal case that
    /// `push_clip_rect` cannot express (plan §4.5).
    ///
    /// Returns the style rather than pushing a stack entry, because a mask
    /// belongs to a *shape*, not to a region: two shapes under one clip rect
    /// routinely want different masks, and a stack would make that the awkward
    /// case.
    #[must_use]
    pub fn push_mask(
        &self,
        style: crate::shaped::ShapedInstance,
        mask_slot: u32,
    ) -> crate::shaped::ShapedInstance {
        style.with_mask(mask_slot)
    }

    /// Shaped vertices emitted this frame.
    #[must_use]
    pub fn shaped_vertex_count(&self) -> usize {
        self.shaped.vertices.len()
    }

    /// Shaped instances emitted this frame.
    #[must_use]
    pub fn shaped_instance_count(&self) -> usize {
        self.shaped.instances.len()
    }

    /// Solid-colour filled rectangle.
    ///
    /// Square corners and no border, so the fragment shader resolves to full
    /// coverage inside and zero outside for any integer-aligned rect — the
    /// pre-Styx result, byte for byte.
    pub fn push_rect_filled(&mut self, rect: Rect, color: [u8; 4]) {
        self.push_primitive(Primitive::fill(rect, color), None);
    }

    /// Rounded, optionally gradient-filled, optionally bordered rectangle.
    pub fn push_round_rect(&mut self, rect: Rect, radius: f32, color: [u8; 4]) {
        self.push_primitive(Primitive::fill(rect, color).with_radius(radius), None);
    }

    /// Solid-colour border (outline only).
    ///
    /// Pre-Styx this was four separate filled rects. It is now one instance
    /// with an inset stroke band, which both removes three quads and lets the
    /// stroke follow a corner radius.
    pub fn push_rect_border(&mut self, rect: Rect, thickness: f32, color: [u8; 4]) {
        self.push_primitive(
            Primitive::fill(rect, [0, 0, 0, 0]).with_border(thickness, color),
            None,
        );
    }

    /// Rounded border following `radius`.
    pub fn push_round_rect_border(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        color: [u8; 4],
    ) {
        self.push_primitive(
            Primitive::fill(rect, [0, 0, 0, 0])
                .with_radius(radius)
                .with_border(thickness, color),
            None,
        );
    }

    /// Render a resolved [`crate::style::Paint`] into `rect`.
    ///
    /// One call, so a widget cannot get the layering wrong. The order is fixed
    /// and is the whole reason this helper exists: shadow and glow sit *behind*
    /// the surface, the inset sits inside it, and the selection rail sits on
    /// top of everything so it is never washed out by a gradient.
    pub fn push_paint(&mut self, rect: Rect, paint: &crate::style::Paint) {
        let radii = [paint.radius; 4];

        if let Some(elevation) = paint.elevation {
            self.push_drop_shadow_rounded(rect, radii, elevation);
        }
        if let Some(glow) = paint.glow {
            self.push_primitive(
                Primitive::glow(rect, radii, glow.radius, glow.color.bytes()),
                None,
            );
        }

        let mut fill = Primitive::fill(rect, paint.background)
            .with_radii(radii)
            .with_border(paint.border_thickness, paint.border);
        if let Some(gradient) = paint.gradient {
            fill = fill.with_gradient(gradient.to.bytes(), gradient.axis);
            fill.fill_a = gradient.from.bytes();
        }
        self.push_primitive(fill, None);

        if let Some(inset) = paint.inset {
            self.push_primitive(
                Primitive::inset_shadow(rect, radii, inset.blur, inset.color.bytes()),
                None,
            );
        }

        if let Some(rail) = paint.rail {
            let width = crate::theme::active().geometry.stroke_rail;
            self.push_primitive(
                Primitive::fill(Rect::new(rect.x, rect.y, width, rect.h), rail),
                None,
            );
        }
    }

    /// Vertical fade at the edge of a scrolling region.
    ///
    /// Phase 27-D, from the §2.4 audit: the Details panel cut off mid-row with
    /// nothing to say content continued. Drawn as a gradient from the panel
    /// colour to fully transparent, so it works over any content.
    pub fn push_scroll_fade(&mut self, rect: Rect, surface: [u8; 4], from_top: bool) {
        let transparent = [surface[0], surface[1], surface[2], 0];
        let (from, to, axis) = if from_top {
            (surface, transparent, [0.0, 1.0])
        } else {
            (transparent, surface, [0.0, 1.0])
        };
        let mut p = Primitive::fill(rect, from);
        p = p.with_gradient(to, axis);
        self.push_primitive(p, None);
    }

    /// Render a run of text using the font atlas.
    ///
    /// `origin` is the top-left corner of the text block (above the ascenders).
    /// Glyphs are rasterized on first use and cached in the atlas permanently.
    pub fn push_text(&mut self, text: &str, origin: Vec2, font_id: u8, px: f32, color: [u8; 4]) {
        self.push_text_tracked(text, origin, font_id, px, color, 0.0);
    }

    /// [`push_text`](Self::push_text) with extra advance between glyphs.
    ///
    /// Only the uppercase header role uses a non-zero value; letter-spacing is
    /// what makes an 11 px caps label read as a designed header rather than as
    /// shouting. Tracking is added after every glyph including the last, which
    /// matches how CSS `letter-spacing` measures, so
    /// [`crate::font::FontAtlas::measure_text_tracked`] can stay a pure
    /// `advance + tracking * count` calculation.
    ///
    /// Phase 27-B snaps the **text block origin** to whole pixels; glyphs are
    /// then placed exactly relative to it. Snapping each glyph quad instead —
    /// the first version of this — rounded every letter to its own subpixel
    /// offset and made the baseline visibly ragged. Advances are untouched, so
    /// measured text width is unchanged either way.
    pub fn push_text_tracked(
        &mut self,
        text: &str,
        origin: Vec2,
        font_id: u8,
        px: f32,
        color: [u8; 4],
        tracking: f32,
    ) {
        // Ascent: distance from top-of-line to baseline (positive).
        // Phase 27-B snapped each glyph quad to whole pixels *independently*,
        // which was wrong and visibly so: `ymin + px_h` differs per glyph, so
        // rounding each quad's top rounded every letter to its own subpixel
        // offset and the baseline visibly jittered. The block origin is snapped
        // once instead, and glyphs are placed exactly relative to it.
        let ascent = self.font_atlas.ascent(px, font_id);
        let origin_x = origin.x.round();
        let mut baseline_y = (origin.y + ascent).round();
        let mut cursor_x = origin_x;

        for ch in text.chars() {
            if ch == '\n' {
                cursor_x = origin_x;
                // Rounded too, so every line shares one pixel phase rather than
                // drifting down the paragraph.
                baseline_y += self
                    .font_atlas
                    .measure_text("Ag", px, font_id)
                    .y
                    .max(px)
                    .round();
                continue;
            }
            let Some(info) = self.font_atlas.get_or_rasterize(ch, px, font_id) else {
                cursor_x += px * 0.5 + tracking;
                continue;
            };
            // Zero-size glyphs (space, etc.) — advance cursor only.
            if info.px_w == 0.0 {
                cursor_x += info.advance + tracking;
                continue;
            }
            // Glyph top-left in screen space:
            //   x = cursor_x + xmin  (horizontal bearing)
            //   y = baseline_y - (ymin + px_h)  (freetype y-up → screen y-down)
            let gx = cursor_x + info.xmin;
            let gy = baseline_y - (info.ymin + info.px_h);
            let rect = Rect::new(gx, gy, info.px_w, info.px_h);
            let uv = [
                info.uv_min[0],
                info.uv_min[1],
                info.uv_max[0],
                info.uv_max[1],
            ];
            self.push_primitive(
                Primitive::glyph(rect, uv, color),
                Some(FONT_ATLAS_TEXTURE_ID),
            );
            cursor_x += info.advance + tracking;
        }
    }

    /// Drop shadow beneath `rect`, marking z-order for popups, drawers and
    /// modals.
    ///
    /// Pre-Styx this was six concentric hard-edged rectangles standing in for a
    /// blur, which banded visibly at the 12–32 px spreads the elevation tokens
    /// ask for. It is now a single instance with an analytic falloff.
    /// Panels never call this: elevation marks layering, not decoration.
    pub fn push_drop_shadow(&mut self, rect: Rect, elevation: crate::theme::Elevation) {
        self.push_drop_shadow_rounded(rect, [0.0; 4], elevation);
    }

    /// [`push_drop_shadow`](Self::push_drop_shadow) following a corner radius.
    pub fn push_drop_shadow_rounded(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        elevation: crate::theme::Elevation,
    ) {
        let alpha = (elevation.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        if alpha == 0 {
            return;
        }
        self.push_primitive(
            Primitive::shadow(
                rect,
                radii,
                [0.0, elevation.offset_y],
                elevation.spread,
                0.0,
                [0, 0, 0, alpha],
            ),
            None,
        );
    }

    /// Textured quad (used for image widgets and icon quads).
    ///
    /// `uv` is kept as four corners for source compatibility with the pre-Styx
    /// signature; the primitive stores the axis-aligned min/max it implies.
    pub fn push_textured_rect(
        &mut self,
        rect: Rect,
        uv: [Vec2; 4],
        color: [u8; 4],
        texture_id: u32,
    ) {
        let uv_rect = [uv[0].x, uv[0].y, uv[2].x, uv[2].y];
        self.push_primitive(Primitive::textured(rect, uv_rect, color), Some(texture_id));
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
                let uv = [us[col], vs[row], us[col + 1], vs[row + 1]];
                self.push_primitive(Primitive::textured(r, uv, color), Some(texture_id));
            }
        }
    }
}

/// Compose two 2x3 affines: `parent` applied after `child`.
///
/// Style fields come from `child`. A transform pushed onto the stack carries no
/// colour of its own, and inheriting one would make a nested shape silently
/// take its parent's fill.
fn compose(
    parent: &crate::shaped::ShapedInstance,
    child: &crate::shaped::ShapedInstance,
) -> crate::shaped::ShapedInstance {
    let [pa, pb, pc, pd, ptx, pty] = parent.xform;
    let [ca, cb, cc, cd, cx, cy] = child.xform;
    let mut out = *child;
    out.xform = [
        pa * ca + pc * cb,
        pb * ca + pd * cb,
        pa * cc + pc * cd,
        pb * cc + pd * cd,
        pa * cx + pc * cy + ptx,
        pb * cx + pd * cy + pty,
    ];
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{AA_PAD, FLAG_TEXT, FLAG_TEXTURED};

    fn ctx() -> DrawingContext {
        DrawingContext::new(800.0, 600.0)
    }

    #[test]
    fn filled_rect_is_a_single_plain_instance() {
        // The 27-A merge gate: the compatibility shim must not gain behaviour.
        let mut c = ctx();
        c.push_rect_filled(Rect::new(10.0, 20.0, 30.0, 40.0), [1, 2, 3, 255]);
        assert_eq!(c.instances.len(), 1);
        let p = c.instances[0];
        assert!(p.is_plain_fill());
        assert_eq!(p.rect, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(p.fill_a, [1, 2, 3, 255]);
        assert_eq!(p.expand, AA_PAD);
    }

    #[test]
    fn border_is_one_instance_not_four_rects() {
        // Pre-Styx this emitted four filled quads.
        let mut c = ctx();
        c.push_rect_border(Rect::new(0.0, 0.0, 20.0, 10.0), 1.0, [9, 9, 9, 255]);
        assert_eq!(c.instances.len(), 1);
        assert_eq!(c.instances[0].border_width, 1.0);
        assert_eq!(c.instances[0].fill_a, [0, 0, 0, 0]);
    }

    #[test]
    fn drop_shadow_is_one_instance_not_six_rings() {
        let mut c = ctx();
        c.push_drop_shadow(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            crate::theme::Elevation {
                offset_y: 4.0,
                spread: 16.0,
                alpha: 0.5,
            },
        );
        assert_eq!(c.instances.len(), 1);
        let p = c.instances[0];
        assert_eq!(p.shadow[2], 16.0, "spread token drives the blur radius");
        assert_eq!(p.shadow_color[3], 128);
    }

    #[test]
    fn fully_transparent_shadow_emits_nothing() {
        let mut c = ctx();
        c.push_drop_shadow(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            crate::theme::Elevation {
                offset_y: 0.0,
                spread: 8.0,
                alpha: 0.0,
            },
        );
        assert!(c.instances.is_empty());
        assert!(c.commands.is_empty());
    }

    #[test]
    fn commands_merge_while_clip_and_texture_match() {
        let mut c = ctx();
        c.push_rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), [255; 4]);
        c.push_rect_filled(Rect::new(1.0, 0.0, 1.0, 1.0), [255; 4]);
        assert_eq!(c.commands.len(), 1);
        assert_eq!(c.commands[0].instance_count, 2);
        assert_eq!(c.commands[0].instance_offset, 0);
    }

    #[test]
    fn a_texture_change_no_longer_breaks_the_batch() {
        // The 164-draw-call measurement that motivated moving the selector onto
        // the instance. A fill followed by a glyph inside one clip region is one
        // draw, and the glyph still records which atlas it samples.
        let mut c = ctx();
        c.push_rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), [255; 4]);
        c.push_textured_rect(
            Rect::new(0.0, 0.0, 4.0, 4.0),
            [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
            [255; 4],
            crate::icons::ICON_ATLAS_TEXTURE_ID,
        );
        assert_eq!(c.commands.len(), 1);
        assert_eq!(c.commands[0].instance_count, 2);
        assert_eq!(c.instances[0].texture_layer(), 0);
        assert_eq!(
            c.instances[1].texture_layer(),
            crate::icons::ICON_ATLAS_TEXTURE_ID
        );

        let total: u32 = c.commands.iter().map(|cmd| cmd.instance_count).sum();
        assert_eq!(total as usize, c.instances.len());
    }

    #[test]
    fn command_breaks_on_clip_change() {
        let mut c = ctx();
        c.push_rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), [255; 4]);
        c.push_clip_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        c.push_rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), [255; 4]);
        c.pop_clip_rect();
        assert_eq!(c.commands.len(), 2);
    }

    #[test]
    fn textured_rect_converts_corner_uvs_to_min_max() {
        let mut c = ctx();
        let uv = [
            Vec2::new(0.25, 0.5),
            Vec2::new(0.75, 0.5),
            Vec2::new(0.75, 0.9),
            Vec2::new(0.25, 0.9),
        ];
        c.push_textured_rect(Rect::new(0.0, 0.0, 8.0, 8.0), uv, [255; 4], 1);
        assert_eq!(c.instances[0].uv, [0.25, 0.5, 0.75, 0.9]);
        assert_eq!(c.instances[0].texture_layer(), 1);
        assert_ne!(c.instances[0].flags & FLAG_TEXTURED, 0);
        assert_eq!(c.instances[0].flags & FLAG_TEXT, 0, "icons are not glyphs");
    }

    #[test]
    fn nine_slice_still_emits_nine_cells() {
        let mut c = ctx();
        c.push_nine_slice(Rect::new(0.0, 0.0, 60.0, 60.0), 3, 0.25, [255; 4]);
        assert_eq!(c.instances.len(), 9);
    }

    #[test]
    fn clear_drops_geometry_but_keeps_atlases() {
        let mut c = ctx();
        c.push_rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), [255; 4]);
        c.font_atlas.dirty = true;
        c.clear(800.0, 600.0);
        assert!(c.instances.is_empty());
        assert!(c.commands.is_empty());
        assert!(c.font_atlas.dirty, "atlas survives the frame boundary");
    }

    // ── Phase 27-D / 27-E: depth ────────────────────────────────────────────

    #[test]
    fn push_paint_layers_shadow_then_fill_then_inset_then_rail() {
        // The order is the whole reason `push_paint` exists: a widget that
        // emitted these by hand could put the rail under a gradient or the
        // shadow over the fill, and both look like a rendering bug.
        use crate::primitive::{FLAG_GRADIENT, FLAG_INSET, FLAG_SHADOW};
        let mut c = ctx();
        let t = crate::theme::active();
        let paint = crate::style::Paint {
            rail: Some([1, 2, 3, 255]),
            ..crate::style::input(crate::style::VisualState::rest())
        }
        .at_elevation(t.elevation.popup);

        c.push_paint(Rect::new(0.0, 0.0, 100.0, 24.0), &paint);

        let flags: Vec<u32> = c.instances.iter().map(|p| p.flags).collect();
        assert_eq!(flags.len(), 4, "shadow, fill, inset, rail");
        assert_ne!(flags[0] & FLAG_SHADOW, 0, "shadow first, behind everything");
        assert_eq!(flags[1] & FLAG_SHADOW, 0, "then the fill");
        assert_ne!(flags[2] & FLAG_INSET, 0, "then the recession, inside it");
        assert_eq!(flags[3], 0, "and the rail on top, unwashed");
        assert_eq!(flags[1] & FLAG_GRADIENT, 0, "an input is never washed");
    }

    #[test]
    fn a_flat_paint_emits_exactly_one_instance() {
        // Depth is opt-in. A recipe that asked for none must cost nothing.
        let mut c = ctx();
        let paint = crate::style::tree_row(crate::style::VisualState::rest());
        c.push_paint(Rect::new(0.0, 0.0, 100.0, 24.0), &paint);
        assert_eq!(c.instances.len(), 1);
    }

    #[test]
    fn a_gradient_paint_carries_both_stops_in_the_right_order() {
        use crate::primitive::FLAG_GRADIENT;
        let mut c = ctx();
        let t = crate::theme::active();
        let paint = crate::style::button(crate::style::VisualState::rest());
        c.push_paint(Rect::new(0.0, 0.0, 100.0, 32.0), &paint);

        let fill = c
            .instances
            .iter()
            .find(|p| p.flags & FLAG_GRADIENT != 0)
            .expect("a resting button is washed with chrome_wash");
        assert_eq!(fill.fill_a, t.gradient.chrome_wash.from.bytes());
        assert_eq!(fill.fill_b, t.gradient.chrome_wash.to.bytes());
        assert_eq!(fill.grad_axis, [0.0, 1.0]);
    }

    #[test]
    fn the_focus_ring_is_the_only_state_that_glows() {
        use crate::primitive::{FLAG_GLOW, FLAG_SHADOW};
        let glowing = |paint: &crate::style::Paint| {
            let mut c = ctx();
            c.push_paint(Rect::new(0.0, 0.0, 80.0, 24.0), paint);
            c.instances
                .iter()
                .any(|p| p.flags & FLAG_GLOW != 0 && p.flags & FLAG_SHADOW != 0)
        };
        let rest = crate::style::button(crate::style::VisualState::rest());
        let focused = crate::style::button(crate::style::VisualState::rest().focused(true));
        assert!(!glowing(&rest), "a resting control must not glow");
        assert!(glowing(&focused), "a focused control must");
    }

    #[test]
    fn a_disabled_control_is_neither_lit_nor_lifted() {
        use crate::style::{Interaction, VisualState, button};
        let disabled = button(VisualState::with(Interaction::Disabled).focused(true));
        assert!(disabled.glow.is_none(), "disabled must not glow");
        assert!(disabled.elevation.is_none(), "disabled must not lift");
    }

    #[test]
    fn pressing_a_button_removes_its_lift() {
        use crate::style::{Interaction, VisualState, button};
        let rest = button(VisualState::rest());
        let pressed = button(VisualState::with(Interaction::Pressed));
        assert!(
            rest.elevation.is_some(),
            "a resting button sits above its panel"
        );
        assert!(
            pressed.elevation.is_none(),
            "a pressed button is pushed into it"
        );
    }

    #[test]
    fn an_input_is_recessed_and_a_button_is_raised() {
        use crate::style::{VisualState, button, input};
        assert!(input(VisualState::rest()).inset.is_some());
        assert!(input(VisualState::rest()).elevation.is_none());
        assert!(button(VisualState::rest()).elevation.is_some());
        assert!(button(VisualState::rest()).inset.is_none());
    }

    #[test]
    fn recipes_follow_the_active_theme() {
        // The Dawn acceptance gate: swapping the snapshot must repaint every
        // recipe without a single widget knowing it happened.
        use crate::theme::{ThemeId, active_id, set_active};
        let original = active_id();

        set_active(ThemeId::Nocturne);
        let dark = crate::style::button(crate::style::VisualState::rest());
        set_active(ThemeId::Dawn);
        let light = crate::style::button(crate::style::VisualState::rest());
        set_active(original);

        assert_ne!(dark.background, light.background);
        assert_ne!(dark.foreground, light.foreground);
        // Geometry is shared, so a swap never invalidates layout.
        assert_eq!(dark.radius, light.radius);
    }

    #[test]
    fn scroll_fade_runs_from_the_surface_to_fully_transparent() {
        use crate::primitive::FLAG_GRADIENT;
        let surface = [0x1C, 0x1E, 0x26, 0xFF];
        let mut c = ctx();
        c.push_scroll_fade(Rect::new(0.0, 0.0, 200.0, 12.0), surface, true);
        assert_eq!(c.instances.len(), 1);
        let p = c.instances[0];
        assert_ne!(p.flags & FLAG_GRADIENT, 0);
        assert_eq!(p.fill_a, surface, "opaque at the clipped edge");
        assert_eq!(p.fill_b[3], 0, "and fully transparent at the other");
        // The RGB must match so the fade does not tint the content it covers.
        assert_eq!(p.fill_b[0..3], surface[0..3]);
    }

    #[test]
    fn a_bottom_scroll_fade_runs_the_other_way() {
        let surface = [0x1C, 0x1E, 0x26, 0xFF];
        let mut c = ctx();
        c.push_scroll_fade(Rect::new(0.0, 0.0, 200.0, 12.0), surface, false);
        let p = c.instances[0];
        assert_eq!(p.fill_a[3], 0);
        assert_eq!(p.fill_b, surface);
    }

    #[test]
    fn every_glyph_in_a_run_sits_on_one_baseline() {
        // Reproduces the reported defect directly: "some letters are a bit
        // above others". Phase 27-B rounded each glyph quad's top, and because
        // `ymin + px_h` differs per glyph that rounded every letter to its own
        // subpixel offset. The baseline a glyph implies is `quad_top + px_h +
        // ymin`; if the placement is sound, every glyph in a run implies the
        // *same* baseline.
        let mut c = ctx();
        for cut in [
            include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        ] {
            c.font_atlas.add_font(cut).expect("bundled cut parses");
        }

        // Deliberately mixed: ascenders, descenders, x-height and digits, which
        // is exactly where per-glyph rounding shows up.
        for (font_id, px) in [(0u8, 13.0f32), (0, 11.0), (1, 12.0)] {
            c.clear(800.0, 600.0);
            c.push_text(
                "Agxy 0369 Hlip",
                Vec2::new(10.3, 20.7),
                font_id,
                px,
                [255, 255, 255, 255],
            );

            let mut baselines: Vec<f32> = Vec::new();
            for (i, ch) in "Agxy 0369 Hlip".chars().enumerate() {
                let _ = i;
                if ch == ' ' {
                    continue;
                }
                if let Some(info) = c.font_atlas.get_or_rasterize(ch, px, font_id) {
                    if info.px_h == 0.0 {
                        continue;
                    }
                    baselines.push(info.px_h + info.ymin);
                }
            }
            assert!(!baselines.is_empty(), "the run must rasterize");

            // Recover each drawn glyph's implied baseline from the draw list.
            let implied: Vec<f32> = c
                .instances
                .iter()
                .zip(baselines.iter())
                .map(|(p, offset)| p.rect[1] + offset)
                .collect();
            let first = implied[0];
            for (n, b) in implied.iter().enumerate() {
                assert!(
                    (b - first).abs() < 0.001,
                    "font {font_id} at {px}px: glyph {n} sits on baseline {b},                      not {first} — the run is not on one baseline"
                );
            }
        }
    }

    #[test]
    fn a_text_block_starts_on_a_whole_pixel() {
        // The bitmap atlas is sampled linearly, so a block landing on a half
        // pixel smears every stem. Snapping the *origin* keeps the grid in
        // register without disturbing the glyphs' relative placement.
        let mut c = ctx();
        c.font_atlas
            .add_font(include_bytes!("../assets/fonts/Inter-Regular.ttf"))
            .expect("bundled cut parses");
        c.push_text("Hi", Vec2::new(10.4, 20.6), 0, 13.0, [255; 4]);
        let first = c.instances.first().expect("a glyph was drawn");
        // `xmin` is the glyph's own bearing and may be fractional; the pen
        // origin it is measured from must not be.
        let bearing = c
            .font_atlas
            .get_or_rasterize('H', 13.0, 0)
            .map(|i| i.xmin)
            .unwrap_or(0.0);
        let pen_x = first.rect[0] - bearing;
        assert!(
            (pen_x - pen_x.round()).abs() < 0.001,
            "the pen origin must be whole-pixel, got {pen_x}"
        );
    }

    // -- MORROWIND-D: the two streams, and the ordering between them ---------

    /// **GHOSTFENCE's first row, in miniature.** A frame that draws no paths
    /// emits exactly the bytes it emitted before the shaped stream existed.
    ///
    /// The whole premise of a second stream is that the first is untouched. If
    /// this fails, the extension became a widening and Phase 27's 646-instance
    /// measurement no longer describes the shell.
    #[test]
    fn a_frame_with_no_shapes_is_unchanged() {
        let mut c = ctx();
        c.push_rect_filled(Rect::new(4.0, 4.0, 100.0, 24.0), [28, 30, 38, 255]);
        c.push_rect_border(Rect::new(4.0, 4.0, 100.0, 24.0), 1.0, [49, 53, 67, 255]);
        assert_eq!(c.instances.len(), 2);
        assert!(c.shaped.is_empty(), "no shapes, no shaped geometry");
        assert!(
            c.commands
                .iter()
                .all(|cmd| cmd.stream == crate::shaped::Stream::Quad),
            "every command from a quad-only frame is a quad command"
        );
    }

    /// Paint order survives two pipelines.
    ///
    /// The specific mistake Appendix A.3.3 warns about is bucketing -- all
    /// quads then all shapes -- which draws every panel over every wire and
    /// looks like a z-order bug in the widget tree rather than in the pass.
    #[test]
    fn interleaving_streams_preserves_paint_order() {
        use crate::shaped::{ShapedInstance, Stream};
        let mut c = ctx();
        let tri = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0)];

        c.push_rect_filled(Rect::new(0.0, 0.0, 10.0, 10.0), [1, 1, 1, 255]);
        c.push_shaped(ShapedInstance::identity([2, 2, 2, 255]), &tri);
        c.push_rect_filled(Rect::new(0.0, 0.0, 10.0, 10.0), [3, 3, 3, 255]);

        let streams: Vec<_> = c.commands.iter().map(|cmd| cmd.stream).collect();
        assert_eq!(
            streams,
            vec![Stream::Quad, Stream::Shaped, Stream::Quad],
            "the command list is in paint order, not bucketed by pipeline"
        );
        // The second quad run starts where the first left off, so the instance
        // buffer is still one contiguous array.
        assert_eq!(c.commands[0].instance_offset, 0);
        assert_eq!(c.commands[2].instance_offset, 1);
    }

    /// Consecutive shapes under one clip merge into one draw.
    #[test]
    fn consecutive_shapes_share_a_command() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        let tri = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0)];
        c.push_shaped(ShapedInstance::identity([1; 4]), &tri);
        c.push_shaped(ShapedInstance::identity([2; 4]), &tri);
        assert_eq!(c.commands.len(), 1);
        assert_eq!(c.commands[0].instance_count, 6, "six vertices, one draw");
        assert_eq!(c.shaped.instances.len(), 2, "two shapes, two styles");
    }

    /// A clip change breaks the run even within one stream.
    #[test]
    fn a_clip_change_breaks_a_shaped_run() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        let tri = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0)];
        c.push_shaped(ShapedInstance::identity([1; 4]), &tri);
        c.push_clip_rect(Rect::new(0.0, 0.0, 5.0, 5.0));
        c.push_shaped(ShapedInstance::identity([2; 4]), &tri);
        assert_eq!(c.commands.len(), 2);
        assert_eq!(
            c.commands[1].instance_offset, 3,
            "the second run starts at vertex 3"
        );
    }

    /// The transform stack composes and applies to shaped pushes.
    #[test]
    fn a_pushed_transform_applies_and_nests() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        let tri = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];

        c.push_transformed(ShapedInstance::identity([0; 4]).translated(Vec2::new(10.0, 0.0)));
        c.push_transformed(ShapedInstance::identity([0; 4]).translated(Vec2::new(0.0, 5.0)));
        c.push_shaped(ShapedInstance::identity([7; 4]), &tri);
        c.pop_transform();
        c.push_shaped(ShapedInstance::identity([8; 4]), &tri);
        c.pop_transform();
        c.push_shaped(ShapedInstance::identity([9; 4]), &tri);

        assert_eq!(
            c.shaped.instances[0].apply(Vec2::ZERO),
            Vec2::new(10.0, 5.0)
        );
        assert_eq!(
            c.shaped.instances[1].apply(Vec2::ZERO),
            Vec2::new(10.0, 0.0)
        );
        assert_eq!(c.shaped.instances[2].apply(Vec2::ZERO), Vec2::ZERO);
        // The fill is the child's, not the stack's: a transform carries no colour.
        assert_eq!(c.shaped.instances[0].fill_a, [7; 4]);
    }

    /// Flattening the same path twice hits the cache.
    #[test]
    fn the_flatten_cache_returns_the_same_allocation() {
        let mut c = ctx();
        let path = crate::path::Path::wire(Vec2::ZERO, Vec2::new(200.0, 40.0));
        let first = c.flatten(&path);
        let second = c.flatten(&path);
        assert!(
            std::rc::Rc::ptr_eq(&first, &second),
            "a wire does not change shape while the user pans"
        );
    }

    /// Changing the tolerance re-flattens rather than serving a stale result.
    ///
    /// This is the DPI case Phase 27 already fixed once. A cache keyed only by
    /// path would hand a 96-DPI flattening to a 192-DPI frame, and the curve
    /// would be visibly faceted on the display that needs it least.
    #[test]
    fn changing_the_tolerance_invalidates_the_cache() {
        let mut c = ctx();
        let path = crate::path::Path::wire(Vec2::ZERO, Vec2::new(200.0, 40.0));
        let coarse = c.flatten(&path);
        c.set_tolerance(0.1);
        let fine = c.flatten(&path);
        assert!(!std::rc::Rc::ptr_eq(&coarse, &fine));
        assert!(fine[0].points.len() > coarse[0].points.len());
    }

    /// Texture slots start after the engine's own atlases and run out honestly.
    #[test]
    fn registering_textures_starts_after_the_atlases() {
        let mut c = ctx();
        assert_eq!(c.register_texture(), Some(3));
        assert_eq!(c.register_texture(), Some(4));
        for _ in 0..crate::shaped::MAX_TEXTURE_SLOTS {
            if c.register_texture().is_none() {
                break;
            }
        }
        assert_eq!(
            c.register_texture(),
            None,
            "wrapping to a slot somebody else owns renders one texture where \
             another was expected"
        );
    }

    /// A stroke reaches the shaped stream through the public API.
    #[test]
    fn stroking_a_path_emits_shaped_geometry() {
        use crate::shaped::{ShapedInstance, Stream};
        let mut c = ctx();
        let path = crate::path::Path::wire(Vec2::ZERO, Vec2::new(120.0, 60.0));
        c.push_stroke(
            &path,
            &crate::path::Stroke::new(2.0),
            ShapedInstance::identity([200, 200, 210, 255]),
        );
        assert!(
            c.shaped_vertex_count() > 30,
            "a bowed wire is many triangles"
        );
        assert_eq!(
            c.shaped_instance_count(),
            1,
            "one shape, whatever its length"
        );
        assert_eq!(c.commands.last().unwrap().stream, Stream::Shaped);
        assert!(c.instances.is_empty(), "a stroke emits no quad instances");
    }

    /// Filling a closed path emits geometry; an open one emits none.
    #[test]
    fn filling_needs_a_closed_contour() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        c.push_path(
            &crate::path::Path::circle(Vec2::splat(20.0), 10.0),
            ShapedInstance::identity([255; 4]),
        );
        let closed = c.shaped_vertex_count();
        assert!(closed > 0);

        c.push_path(
            &crate::path::Path::wire(Vec2::ZERO, Vec2::new(50.0, 0.0)),
            ShapedInstance::identity([255; 4]),
        );
        assert_eq!(
            c.shaped_vertex_count(),
            closed,
            "an open contour has no interior to fill"
        );
    }

    /// UVs are only computed for shapes that actually sample something.
    #[test]
    fn an_untextured_shape_pays_nothing_for_uvs() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        let tri = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0)];
        c.push_shaped(ShapedInstance::identity([1; 4]), &tri);
        assert!(c.shaped.vertices.iter().all(|v| v.uv == [0.0, 0.0]));

        c.push_shaped(ShapedInstance::identity([1; 4]).with_texture(3), &tri);
        assert!(
            c.shaped.vertices[3..].iter().any(|v| v.uv != [0.0, 0.0]),
            "a textured shape gets UVs over its own bounds"
        );
    }

    /// Clearing a frame resets the shaped stream and the transform stack.
    ///
    /// An unpopped transform leaking into the next frame would rotate the whole
    /// UI by a little more every frame -- a spectacular bug, and an easy one to
    /// write.
    #[test]
    fn clearing_resets_both_streams() {
        use crate::shaped::ShapedInstance;
        let mut c = ctx();
        c.push_transformed(ShapedInstance::identity([0; 4]).rotated(0.3, Vec2::ZERO));
        c.push_shaped(
            ShapedInstance::identity([1; 4]),
            &[Vec2::ZERO, Vec2::X, Vec2::Y],
        );
        c.clear(100.0, 100.0);
        assert!(c.shaped.is_empty());
        assert_eq!(c.current_transform().xform, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn identical_frames_produce_identical_instance_bytes() {
        // phase_27 §10.3: an idle shell must not churn the draw list.
        let build = || {
            let mut c = ctx();
            c.push_rect_filled(Rect::new(4.0, 4.0, 100.0, 24.0), [28, 30, 38, 255]);
            c.push_rect_border(Rect::new(4.0, 4.0, 100.0, 24.0), 1.0, [49, 53, 67, 255]);
            c.push_drop_shadow(
                Rect::new(0.0, 0.0, 50.0, 50.0),
                crate::theme::Elevation {
                    offset_y: 2.0,
                    spread: 12.0,
                    alpha: 0.4,
                },
            );
            c
        };
        let a = build();
        let b = build();
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&a.instances),
            bytemuck::cast_slice::<_, u8>(&b.instances)
        );
        assert_eq!(a.commands, b.commands);
    }
}
