# MORROWIND-D — the paint layer, part two (Seam 4b)

**Complete, 2026-08-24**, with the visual evidence owed and named below.
Track 1 (VIVEC). The first of the plan's two enabling primitives (§9.2).

## The premise, and the rule

`phase_MORROWIND.md` §4.5 states the problem in one paragraph: the UI's sole
instance type is an **axis-aligned rectangle in screen pixels**. No transform,
no stroke, rectangular clipping only, one linear gradient axis, and exactly
three bound textures. A rotated health bar, a radial menu, a zoomable node
graph, a curve editor's bezier handles and a world-space quest marker are all,
today, *inexpressible*.

And the rule that constrains the fix: **the 100-byte `Primitive` is frozen.**
Phase 27 measured 646 instances on the 1920x1080 shell against it. So this is a
*second* stream — its own pipeline, the same pass, the same blend state, ordered
by the existing `draw_over` rule.

`shaped::tests::the_frozen_quad_instance_is_untouched` and
`draw::tests::a_frame_with_no_shapes_is_unchanged` are the two tests that make
that a fact rather than an intention.

## What landed

| §8 item | Landed |
|---|---|
| 1. The second instance stream | `shaped.rs`: `ShapedInstance` (2x3 affine, stroke/fill, texture and mask slots, flags), `ShapedVertex`, `ShapedBuffers`. Own pipeline, own buffers, same pass. |
| 2. Paths and strokes | `path.rs`: line, polyline, quadratic and cubic bezier, arc; miter/round/bevel joins, butt/round/square caps, dashes with an animatable phase; ear-clipped fills. |
| 3. Arbitrary textures | The three fixed bindings are one `binding_array<texture_2d<f32>, 64>`, with font/icon/thumbnail at slots 0-2. `DrawingContext::register_texture` hands out 3.. ; `UiPass::set_texture` supplies the view. |
| 4. Masking | `ShapedInstance::with_mask` / `DrawingContext::push_mask` — clip to an alpha texture, not only a `Rect`. |
| 6. Gradients | Linear, **radial and angular**, all in the shape's local space so a rotated widget's gradient rotates with it. |
| 5. Render-to-texture | **Deferred to MORROWIND-E, deliberately.** See below. |

**Runtime artefact**, per the plan's named list: `push_path`, `push_stroke`,
`push_transformed` / `pop_transform`, `register_texture`, `push_mask`,
`push_shaped`, `flatten`, `set_tolerance`.

## Three decisions worth arguing with

### CPU tessellation, and why the question is closed

`bevy_vello` was read specifically to decide **against** a compute-based vector
rasteriser, and §8 asks for the reason to be written down so the question is not
reopened annually. It is: Somnium's vector needs are wires, curves, splines and
rotated widgets — thousands of *short strokes*, not glyph-density fills with
winding rules. A compute rasteriser is a second renderer inside the UI pass,
with its own scheduling, its own intermediate targets and its own interaction
with the frozen ordering, bought to gain correctness on cases Somnium does not
have. **Revisit when the UI needs arbitrary filled glyph outlines at speed, or
conflation-artifact-free overlapping fills.** Neither is on the roadmap.

### Geometry is per-vertex; style is per-instance

A rounded rect is one instance and no geometry — the quad pipeline evaluates it
analytically. A stroked bezier is a few hundred triangles and has no analytic
form. So `ShapedInstance` goes in a storage buffer, `ShapedVertex` carries a
local position plus *the index of the shape it belongs to*, and the vertex stage
looks its own style up. A run of a hundred wires is still **one draw call**.

The plan's Appendix A.3.3 sketches a purely per-instance form with a
`geom_offset`/`geom_len` into a shared buffer. That works and needs one draw per
shape, or a base-vertex trick per shape; carrying the index on the vertex is the
same information with neither. The sketch's *field list* is otherwise honoured.

### No fallback path for bindless, because the engine already requires it

Appendix A.3.3 says *"MORROWIND-D must probe and record which it got"* and names
a texture-atlas-page fallback for backends without binding arrays.

**It is not needed, and that is a finding rather than a shortcut.**
`somnium_renderer::context`'s `required_features` already demands
`TEXTURE_BINDING_ARRAY`, `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING`
and `PARTIALLY_BOUND_BINDING_ARRAY` — the bindless resource pool has depended on
them since Phase 24. A device that lacks them cannot create the renderer that
owns this pass. Writing a second texture path for a configuration the engine
cannot reach would be untested code guarding an impossible case.

## Two defects found, both by the same mechanism

MORROWIND-C's lesson was that one description of a shader beats two. The same
mechanism — a naga validation test — found both of these on its first run.

### 1. The WGSL mirror was 80 bytes against Rust's 64

`ShapedInstance` carried `grad: [f32; 4]` / `grad: vec4<f32>`. **Rust aligns
`[f32; 4]` to 4; WGSL aligns `vec4<f32>` to 16.** The gradient landed at offset
24 in one language and 32 in the other, and the struct's stride differed by 16
bytes — which decodes every instance after the first from the wrong offset and
renders as "everything after the first shape is garbage".

`somnium_ui` had **no shader validation at all** before this sub-phase; the
renderer has had it since Phase 25. Adding it (`crates/somnium_ui/tests/shaders_validate.rs`,
six tests) caught this before the code ever reached a GPU. The fix is that the
WGSL mirror spells both the affine *and* the gradient as loose scalars, so
neither has an alignment of its own to disagree about, and the test pins every
offset.

This is the third time in this phase that a duplicated description drifted:
the renderer's `format!` concatenations, its stale `naga` pin, and now this.

### 2. The shaped shader premultiplied alpha; the quad pipeline does not

Written from the plan's sketch, `fs_shaped` returned `vec4(rgb * a, a)` under a
comment claiming it matched the quad pipeline's blend state. It does not:
`ui_pass.wgsl` blends **straight** alpha (`SrcAlpha`/`OneMinusSrcAlpha`).
Premultiplying would have applied the alpha term twice and made every shaped
shape darker than the quad shape beside it — which looks exactly like a
colour-space bug and is not one.

Caught by reading the existing pipeline's blend descriptor before copying it,
which is the only reason it is a paragraph here rather than an afternoon later.

## Three more findings from the tessellator's own tests

- **The bow-tie fill did not draw nothing.** Ear clipping does not detect
  self-intersection: given a bow tie it finds ears and emits overlapping
  triangles. `fill_contour`'s documented contract said otherwise, so the
  contract is now kept by an explicit O(n²) segment-crossing check over
  contours of tens of points.
- **A closed stroke is legitimately *larger* than the open one through the same
  corners** — four joins against three, because its ends are a corner rather
  than ends. The first version of that test asserted the opposite.
- **A wire between two stacked ports bows about 6.75 px** with a 24 px handle
  floor, not the 10 px a first guess asserted. The number to defend is "not
  zero": a straight line there hides which way the edge runs. Blueprint and
  Godot look the same.

## Item 5, deferred with a reason

**Render-to-texture is not in this sub-phase, and is not a gap.** `begin_layer`
and MORROWIND-E's world-space canvas are the *same mechanism* — an offscreen
target that a later draw consumes as a texture — and §8's MORROWIND-E says the
world-space decision *"is recorded in this sub-phase"*, between
render-to-texture-then-quad and direct 3D submission.

Building half the mechanism now, against a decision E has not made, would mean
building it twice. `register_texture` is the seam it will plug into: an offscreen
target is a registered texture like any other, and nothing in this sub-phase
would change to accommodate it.

## Tests: 60 new, 366 in the crate, 0 failures

- **`path`, 21** — a cubic checked against the true curve rather than a segment count; tolerance proven to change the output (a tolerance accepted and ignored looks correct at the default and wastes an order of magnitude at high DPI); the miter limit stopping a spike; dash coverage at half for a 4-on/4-off pattern; both windings filled; a concave L filled by its own area rather than its convex hull's.
- **`shaped`, 15** — the frozen instance still 100 bytes and 12 attributes; rotation about a pivot leaving the pivot fixed; inversion round-tripping (MORROWIND-F hit-tests through it); a **singular transform returning `None` rather than a NaN**, because a NaN comparison is false and would make every hit test *miss* — an invisible, unclickable widget with nothing in the log; a zeroed instance sampling no texture, since slot 0 is the font atlas.
- **`draw`, 14** — paint order across streams (the bucketing mistake A.3.3 names); a clip change breaking a shaped run; the transform stack nesting and *not* inheriting colour; the flatten cache hitting on a repeat and missing on a DPI change; texture slots running out rather than wrapping.
- **`shaders_validate`, 6** — both shaders parse and validate, in both the sRGB and non-sRGB forms `UiPass` actually compiles; both declare the same array; the enable comes first; the struct mirror matches.

## The owed item

**No visual evidence.** Everything here is verified by parse, by layout, by
arithmetic and by ordering — none of which is a picture. What needs a windowed
GPU session:

```bash
cargo run -p hello_engine    # then draw a stroked wire and confirm it appears
```

and specifically **GHOSTFENCE's first row**: the 646-instance / 56-rounded /
29-washed / 21-lifted / 5-recessed / 17-stroked composition Phase 27 measured
must be byte-identical. The CPU half of that is asserted here; the *rendered*
half needs the golden reference that MORROWIND-A's runner is waiting for.

This is the fifth item queued behind one windowed session. The others are A2's
`.somtime` parity, A2's capability report, MORROWIND-A's first golden image, and
C's reload latency.

## Files

```
+ crates/somnium_ui/src/path.rs            flatten, stroke, fill, dash (21 tests)
+ crates/somnium_ui/src/shaped.rs          ShapedInstance/Vertex/Buffers (15 tests)
+ crates/somnium_ui/src/ui_shaped.wgsl     the second pipeline's shader
+ crates/somnium_ui/tests/shaders_validate.rs   somnium_ui's first shader tests
~ crates/somnium_ui/src/draw.rs            Stream on DrawCommand; the shaped API
~ crates/somnium_ui/src/pass.rs            bindless BG1, BG2, second pipeline,
                                           interleaved command walk, set_texture
~ crates/somnium_ui/src/ui_pass.wgsl       three fixed bindings -> one array
~ crates/somnium_ui/src/lib.rs             pub mod path, pub mod shaped
~ crates/somnium_ui/Cargo.toml             naga dev-dependency, pinned to wgpu's major
```
