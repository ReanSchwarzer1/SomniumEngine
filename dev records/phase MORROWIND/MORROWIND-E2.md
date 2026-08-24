# MORROWIND-E2 — the game UI hook

**Complete, 2026-08-25.** Track 1 (VIVEC). **Not in the plan**, and that is the
finding: §8 lists six sub-phases in Track 1 and none of them is *"let a game put
the UI on screen"*, because the plan assumed a runtime UI framework implies a
way to reach it. It did not.

## What was wrong

`somnium_ui::runtime::UiCanvas::render` takes a `&Window`, a `&wgpu::Device`, a
`&wgpu::Queue`, a `&mut CommandEncoder`, a `&TextureView` and a
`TextureFormat`. `EngineContext` (`crates/somnium_core/src/context.rs:80`) hands
a `GameApp` the world, physics, audio, the renderer, the selection, the camera,
the simulation clock, the script host **and the editor's `UiManager`** — and not
one of those six things.

So there was no expression a game could write that put a widget on screen.

The evidence is `examples/vvardenfell`, which for four sub-phases did this:

```rust
let layout = self.hud.layout(glam::Vec2::new(w as f32, h as f32));
println!("  health bar {:?}", layout.health_bar);
println!("  minimap    {:?}", layout.minimap);
println!("  crosshair  {:?}", layout.crosshair);
```

**MORROWIND-D, -E, -F and -G — a second instance stream, paths, strokes,
gradients, masks, bindless textures, canvas roots, anchors, safe areas,
directional navigation, shaped hit-testing, styled runs, BBCode, font fallback
and IME — and the second example printed its layout to stdout.**

`context.md` recorded the finding on the day MORROWIND-F closed and said
*"Next: that hook, then MORROWIND-H."* The next three commits were AE, AG and
AH. The hook waited a week.

## Why this is the second-example rule's best evidence so far

Nothing was failing. `somnium_ui` had 471 passing tests. The editor looked
exactly right — because the editor reaches the paint layer through `UiManager`,
which has always had the hook, in the form of `UiManager::end_frame` being
called by the renderer at pass 9. Every test that could have caught this was
written against the half of the system that already worked.

**The only mechanism in the plan that caught it was the rule that a second
program must exist**, and it caught it by being unable to do the obvious thing.
The preamble's claim — *"this is the only mechanism in the plan that reliably
catches an engine/game boundary that does not exist"* — is now measured rather
than argued.

## The design, and the one alternative that was rejected

**Rejected: a `UiCanvas` in `EngineContext`.** It is the smaller diff and it is
wrong. A game owns its canvases and there are several — a HUD, a pause menu, a
world-space name-plate over each of forty NPCs — and the engine has no business
knowing how many. Putting one in the context would have made the common case a
special case.

**Taken: the engine owns the *moment*, the game owns the trees.**

```rust
fn on_render(&mut self, ctx: &mut EngineContext) { /* build */ }
fn on_render_ui(&mut self, frame: &mut GameUiFrame) { frame.draw(&mut self.hud); }
```

Three pieces:

| Piece | Where | What it is |
|---|---|---|
| `GameUiFrame<'a>` | `somnium_ui::runtime` | the open encoder, view, device, queue, window and format, with `draw(&mut UiCanvas)` over them |
| `GameUi` | `somnium_ui::runtime` | the trait the renderer calls, blanket-implemented for `FnMut(&mut GameUiFrame)` so a game never writes an adapter |
| `GameUiAdapter` | `somnium_core::app` | six lines joining a boxed `GameApp` to that trait |

The adapter exists because **`somnium_renderer` cannot name a `GameApp`** —
`somnium_core` depends on the renderer, not the reverse. Rather than invert
that, the renderer takes a trait object from the crate they *both* depend on.

### Build in `on_render`, draw in `on_render_ui`

`GameUiFrame` deliberately carries no world, no physics and no time. The split
is not bureaucratic: `on_render_ui` runs with a half-recorded command encoder
open, and a callback that could mutate the world at that point is a callback
that can invalidate a buffer the encoder is already referencing. A game that
finds it needs the world in `on_render_ui` has found that it is building rather
than drawing.

### Where in the frame

Pass 9, in `render_with_game_ui`, **before** `UiManager::end_frame`:

- In the editor, a game's HUD belongs *under* the editor's panels.
- In immersive mode the editor's `end_frame` is skipped entirely and the game's
  UI is the only UI — the same code path, not a second one.
- It gets its own profiler zone, `Game UI`. A HUD that costs two milliseconds
  should read as a HUD that costs two milliseconds, not as an editor that got
  slower.

`render` is kept as `render_with_game_ui(.., None)` so a capture harness or a
test does not have to say it has no game UI.

### Input, and the ordering question that is not this sub-phase's to answer

`GameApp::on_os_event(&mut self, ctx, &WindowEvent) -> bool`, called after the
editor shell declines an event and before the editor's viewport tools see it.
It takes the raw `winit::event::WindowEvent` rather than the translated
`EngineEvent`, because `UiCanvas::process_os_event` takes a `WindowEvent` and a
translation layer in front of it would be a second event vocabulary for the
runtime UI to disagree with the editor UI in. `winit` types are already
re-exported from `somnium_core` (`MouseButton`, `KeyCode`; `WindowEvent` joins
them), so this leaks nothing that was not already public.

**The order — editor first, game second — is right while the game is a thing
inside a viewport and wrong once there is a play mode with input focus of its
own. That is MORROWIND-N's call**, and it is written into the doc comment so N
does not have to rediscover it.

## The second bug: anchors with no output

MORROWIND-E's `Canvas::place` resolves an `Anchoring` into a `Rect`. **Nothing
consumed the result.** There was no way to say "put this widget there", so the
anchoring system computed rectangles that no widget ever read, and the reason it
looked finished is that E's tests asserted on the rectangles rather than on the
tree.

Added: `UserInterface::place_node(handle, rect)` — the write half of
`screen_bounds` — and `UiCanvas::place_anchored(handle, &anchoring)` over it.
Both pin horizontal and vertical alignment, because a node handed a rectangle it
then centres itself inside of is not placed.

## The third bug: GHOSTFENCE could not run on Windows

```
UnicodeDecodeError: 'charmap' codec can't decode byte 0x90 in position 4243
TypeError: can only concatenate str (not "NoneType") to str
```

Three `subprocess.run(capture_output=True, text=True)` calls in
`tools/ghostfence/run.py` decoded cargo's UTF-8 output as the Windows ANSI code
page. One non-ASCII byte anywhere in a test name or a diagnostic killed the
reader thread, `proc.stdout` came back `None`, and the gate crashed before
printing a row. **The must-not-break gate has not been able to run since it was
written**, which is a worse failure than any row failing.

Fixed with `encoding="utf-8", errors="replace"` on all three, and the `None`
concatenation guarded. The gate now runs, and its first act was to fail the
`census` row correctly because the tree had changed.

## What `vvardenfell` does now

`HudTree` — one `UiCanvas`, four nodes, no editor chrome — built from the *same*
`Anchoring` values the layout tests already assert on, so the tests keep testing
what is on screen. The health bar is a track and a fill, because one node with a
background cannot show a value, and a HUD whose health bar cannot show a value
is a rectangle with a name. The fill drains over two minutes and wraps: the
slice has no combat to take health away, and pretending otherwise would be a lie
in the one program whose job is to not contain any.

Re-anchoring happens on a resize, not per frame — `place_anchored` invalidates
every ancestor's layout, and doing that sixty times a second to answer "no,
nothing moved" is how a HUD becomes a frame cost.

## Tests: 8 new, 0 failures

- **`somnium_ui::runtime`, 4** — an anchoring moving *the widget* and not only
  the rectangle (the bug this is named after); `place_node` pinning both
  alignments; a pinned widget keeping its distance from the right edge across a
  1080p→4K resize; a plain closure satisfying `GameUi` without an adapter type.
- **`vvardenfell`, 4** — the tree landing where the anchoring says for all three
  parts; the fill following the value; **an overfull bar clamping** rather than
  drawing past its track, which is the bug every health bar has once, when a
  heal overshoots; the tree re-anchoring on a resize.

Suite: `somnium_ui` 471 passed, `vvardenfell` 12 passed, GHOSTFENCE reports
**1,633 passed / 0 failed** against a floor of 945.

## GHOSTFENCE

```
PASS  census            MORROWIND-A_census.md matches the tree
PASS  toolchain         rustc 1.88, wgpu 30.0, winit 0.30
PASS  shader-budget     51 modules, 51 variants possible in total
PASS  one-job-system    no bare spawns; 2 exemptions, each with a reason
PASS  no-second-system  4 singleton symbols, each defined only where it is allowed
SKIP  golden-images     no reference set yet
PASS  tests             1633 passed, 0 failed (floor 945)
```

The `golden-images` row is the next piece of work and it is what unblocks the
shaper (Appendix A.5, item 2).

## Files

```
+ crates/somnium_ui/src/runtime/mod.rs   GameUiFrame, GameUi, place_anchored,
                                         place_node, 4 tests
~ crates/somnium_ui/src/ui.rs            UserInterface::place_node
~ crates/somnium_ui/src/lib.rs           re-export GameUi, GameUiFrame
~ crates/somnium_renderer/src/renderer.rs render_with_game_ui, the Game UI
                                         profiler zone, the empty-hook warning
~ crates/somnium_core/src/app.rs         on_render_ui, on_os_event, GameUiAdapter,
                                         the §3.2 event route
~ crates/somnium_core/src/lib.rs         re-export UiCanvas, GameUiFrame, WindowEvent
~ examples/vvardenfell/src/hud.rs        HudTree, 4 tests
~ examples/vvardenfell/src/main.rs       on_render / on_render_ui / on_os_event
~ tools/ghostfence/run.py                the gate can run on Windows
```
