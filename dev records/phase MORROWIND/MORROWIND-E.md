# MORROWIND-E — the canvas (Seam 4a)

**Complete, 2026-08-24**, with one integration gap found and named below.
Track 1 (VIVEC).

## What §8 asked for, and what landed

| # | Asked | Landed |
|---|---|---|
| 1 | `Canvas` roots: `Screen { scaler }`, `World { transform, size, billboard }`, `Overlay { camera }` | `runtime/canvas.rs`. `Overlay` takes a `world_anchor` rather than a camera — see below. |
| 2 | Anchors: min/max, offsets, pivot, stretch — the RectTransform vocabulary, **without discarding the arrange pass** | `runtime/anchor.rs`. Layered beside measure/arrange, not over it. |
| 3 | Scaling: constant pixel, scale-with-resolution, constant physical size; **plus safe area** | `CanvasScaler` (three modes) and `SafeArea`. |
| 4 | The world-space decision, **recorded in this sub-phase** | Made and written down in `canvas.rs`'s module docs. |
| 5 | Layers and sorting | `Layer` with `HUD`/`MENU`/`OVERLAY`/`DEBUG`, stable within a layer. |
| — | **Slice:** a HUD and a floating name-plate | `examples/vvardenfell/src/hud.rs`, 8 tests. |

## The world-space decision, made

§8 asks for a choice between render-to-texture-then-quad and direct 3D
submission, "recorded in this sub-phase". **The decision is render-to-texture.**

| | Render-to-texture | Direct 3D submission |
|---|---|---|
| Compositing with the visibility buffer | A textured quad. Nothing new. | Needs depth, ordering, and a second projection path in both UI shaders. |
| Text crispness | Resampled once. | Crisp at any angle. |
| Cost | One target per canvas. | None extra. |
| Blast radius | `UiPass` gains an offscreen target; the shaders are untouched. | **`ui_pass.wgsl` is the frozen Hades paint contract.** |

The last row decides it. Direct submission means teaching the frozen quad
shader about a view-projection matrix and a depth test for a feature only
world-space canvases use, and Phase 27's contract is the thing this entire track
is built not to disturb. Resampled text on a name-plate is a real cost and a
small one; re-opening the paint contract is neither.

**And it closes MORROWIND-D's deferral honestly.** D deferred `begin_layer`
saying render-to-texture and the world-space canvas are one mechanism and that E
had not yet decided. It has now: an offscreen target is a **registered texture**
(`DrawingContext::register_texture`), sampled through the same bindless array a
game sprite uses. Nothing in D changes to accommodate it.

The mitigation for resampled text is per-canvas — raise
`world_pixels_per_unit` for the one canvas that needs it — and the slice has a
test showing 50 → 200 px/unit quadruples the target rather than changing the
architecture.

## One design change against the plan's sketch

§8 writes the third mode as `Overlay { camera }`. It is implemented as
`Overlay { world_anchor: Vec3 }`.

A camera reference in a canvas is a lifetime and an ownership question — which
camera, owned by whom, valid for how long — for information the canvas needs
only at projection time. `project_overlay(view_projection, viewport)` takes the
matrix as an argument instead, so the canvas stays a plain `Copy` value and the
caller supplies whichever camera it is currently rendering. The *anchor* is the
part that belongs to the canvas, and that is what it holds.

## Anchors are not alignment, and that is the point

The tree already has `HorizontalAlignment` / `VerticalAlignment` / `Thickness`
resolved during arrange. Anchors are not a replacement, and the module says so
at length because "we already have alignment" is the obvious objection:

- Alignment says *"put me at the top-right of whatever space I was given"*. It
  is a preference resolved inside a parent's arrange.
- An anchor says *"my top-right corner is 20 px in from the parent's top-right
  corner, and stays there when the parent resizes"*.

**Alignment cannot express stretch at all.** "16 px from the left edge and 16 px
from the right edge, whatever the width" has no alignment value, and it is what
a health bar, a title bar and a full-screen dim overlay all need. Anchoring
computes a rect the existing arrange pass then uses; the arrange pass is
untouched, which is what §8's "without discarding Fyrox's arrange pass" asks.

## Four things that are only wrong on somebody else's hardware

Each of these has a test, because none of them is visible on the machine the
code is written on.

- **The safe area arrives in viewport pixels and must be divided by the canvas
  scale.** Skipping that division is correct at 1:1 and wrong on every scaled
  canvas. `the_safe_area_is_converted_into_canvas_units` checks an 88 px notch
  becomes 44 canvas units at 4K.
- **The match blend is logarithmic, not linear.** On an ultrawide the two axis
  factors differ a lot, and a linear blend at 0.5 is not the middle of them —
  the geometric mean is what "halfway between two ratios" means.
  `the_match_blend_is_geometric` pins width-4 / height-1 to 2.0, not 2.5.
- **A point behind the camera projects to a mirrored position in front of it.**
  A marker for an objective behind the player would appear ahead of them,
  pointing at nothing. `project_overlay` returns `None` for `w <= 0` and for
  anything outside the depth range, and the doc says callers must treat that as
  "do not draw".
- **Centring by the top-left corner leaves a widget half its own size off
  centre**, which reads as a rounding bug. `Anchoring::centred` exists so that
  is unwritable, and `centring_uses_the_pivot` shows the two results differing
  by exactly half the size.

Two more, from the degenerate cases: insets larger than the parent **clamp to
zero rather than inverting** (a negative rect flips inside out and the widget
reappears mirrored somewhere unexpected), and a zero viewport or zero reference
produces a finite layout rather than a NaN.

## The finding: a game still cannot draw its HUD

**`EngineContext` has no UI hook.** It hands a game `world`, `physics`, `audio`,
`render_ctx` and `renderer` — and no way to submit a widget tree. `UiCanvas`
exists and can lay out and draw, but a `GameApp` has no encoder, no target view
and no place in the frame to call it.

So the slice exercises the canvas API through `somnium_ui` directly and asserts
its layout, which is a real use of the public surface and is exactly what the
second-example rule is for. It is not a rendered HUD, and this record does not
claim one.

This is §4.5's finding restated one layer out: the plan measured that the UI
cannot *draw* a game, and this sub-phase found that the engine cannot *offer* it
the chance. It belongs to Track 1 and it is small — an owned `UiCanvas` on the
app, a field on `EngineContext`, a draw after the editor's pass. **MORROWIND-F
needs the same hook** to route input into a game canvas, so the two should land
together rather than F discovering it again.

## Tests: 34 new, 0 failures

- **`anchor`, 9** — a top-left pin reproducing a plain position (so adding the field changes nothing until someone sets it); a bottom-right pin surviving a resize; the pivot; stretch insetting from both edges; mixed axes; over-inset collapsing; nesting against a non-zero parent origin.
- **`canvas`, 17** — the three scalers; the geometric blend; a degenerate viewport; the safe area converted into canvas units and honoured by anchored children; a world canvas ignoring the viewport (which is why a name-plate does not reflow on resize); overlays behind the camera and past the far plane refusing to project; billboarding replacing *only* the rotation (adopting the camera's translation would fly the plate to the camera, its scale would resize it); layer ordering stable within a layer.
- **`vvardenfell::hud`, 8** — the same HUD at 1080p and 4K; the minimap in its corner on an ultrawide; the crosshair actually centred; the health bar's insets held at four resolutions; a notch moving both; the name-plate world-sized; the crispness trade; and a test that the whole HUD builds from one crate's public API.

## Files

```
+ crates/somnium_ui/src/runtime/anchor.rs   Anchors, Offsets, Pivot, Anchoring
+ crates/somnium_ui/src/runtime/canvas.rs   Canvas, modes, scalers, SafeArea, Layer
+ examples/vvardenfell/src/hud.rs           the slice's HUD and name-plate
R crates/somnium_ui/src/runtime.rs -> runtime/mod.rs   (per Appendix A.4's file map)
~ crates/somnium_ui/src/runtime/mod.rs      UiCanvas owns a Canvas; layout_for,
                                            place, apply_canvas, with_canvas
~ examples/vvardenfell/src/main.rs          builds the HUD and reports its layout
~ examples/vvardenfell/Cargo.toml           somnium_ui, glam
```
