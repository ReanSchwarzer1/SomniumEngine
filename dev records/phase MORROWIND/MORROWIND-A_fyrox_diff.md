# MORROWIND-A — the Fyrox diff

`phase_MORROWIND.md` §6.1 calls this "the highest-leverage reference in the
tree, by a wide margin" and makes a systematic module-by-module diff MORROWIND-A's
first deliverable. This is that diff: every module in
`example_repo/fyrox/Fyrox-master/fyrox-ui/src/` against
`crates/somnium_ui/src/`, with a verdict and a one-line reason.

Measured 2026-08-24. Fyrox line counts are `wc -l` per file, or the sum of
`*.rs` directly inside a directory for the `name/` rows.

**The copy rule applies here exactly as it applies everywhere else**: Fyrox is
MIT (`fyrox/Fyrox-master/LICENSE.md`, "Copyright (c) 2019-present Dmitry
Stepanov and Fyrox Engine contributors"), and that permits reading, not
transcribing. Somnium's fork has diverged far enough that transcription would
not work even if it were allowed — Phase 27 replaced the paint layer entirely
and Phase 26-Zeta replaced the style layer, so a Fyrox widget's `draw` method
speaks a contract Somnium no longer has.

## The four verdicts

| Verdict | Meaning |
|---|---:|
| **present** | Somnium already has this, from the original fork or from a later phase. Read only if a specific gap appears. |
| **adapt** | Somnium does not have it and a named sub-phase needs it. The Fyrox module is the reference to read *first*. |
| **refuse** | Deliberately not taken. Either a Somnium phase already decided otherwise, or the module solves a problem Somnium solves elsewhere. |
| **defer** | Real, small, and nothing needs it yet. Revisit when a track asks. |

## Summary

| Verdict | Modules | Fyrox lines |
|---|---:|---:|
| present | 34 | 15,041 |
| adapt | 19 | 18,027 |
| refuse | 10 | 8,015 |
| defer | 5 | 1,417 |

**The finding, in one line:** of Fyrox's ~66 UI modules, **19 map onto named
MORROWIND sub-phases** and between them are roughly **18,000 lines of already-solved
problem** — dominated by `dock/` (2,033), `text_box.rs` (1,805), the rich-text
pair (1,884), `widget.rs` (2,148) and `message.rs` (1,450). §6.1's claim that
Fyrox "already built half of Track 1 and Track 2" survives measurement.

The second finding is less comfortable and worth stating: **`somnium_ui/src/widget.rs`
is 217 lines against Fyrox's 2,148.** That is not a 10× efficiency; it is the
visibility, enabled, opacity, z-index, tooltip, context-menu, hit-test-override
and layout-transform machinery that Somnium's widget simply does not have,
because the editor shell never needed it. Track 1 needs most of it, and
MORROWIND-D/E/F is where that debt comes due.

---

## The diff

### Track 1 — VIVEC (the runtime UI)

| Fyrox module | Lines | Somnium counterpart | Verdict | Reason |
|---|---:|---|---|---|
| `screen.rs` | 217 | `runtime.rs` (`UiCanvas`, 141 ln) | **adapt → MORROWIND-E** | Somnium's `UiCanvas` wraps a `UserInterface` for screen space and stops there. Seam 4a needs a root that *declares* its space — `Screen { scaler }`, `World { .. }`, `Overlay { camera }` — and `screen.rs` is the screen-space half of that already written. |
| `vector_image.rs` | 361 | — | **adapt → MORROWIND-D** | Vector paths inside the widget tree. This is Seam 4b's `ShapedInstance` consumer and the reason a node-graph wire or a curve handle is drawable at all. |
| `nine_patch.rs` | 664 | `draw.rs:push_nine_slice` | **adapt → MORROWIND-D** | Somnium has the *draw call* and no widget and no texture to feed it (census §4.5). The widget is the smaller half; the binding array is the real work. |
| `bbcode.rs` | 428 | — | **adapt → MORROWIND-G** | A markup parser producing styled runs. Small, self-contained, and exactly the shape rich text needs. |
| `formatted_text.rs` + `formatted_text/` | 1,884 | `font.rs` (476), `typography.rs` (264) | **adapt → MORROWIND-G** | Run-based layout with per-run style. Somnium's font layer rasterises glyphs; it has no concept of a run, which is why shaping has nowhere to land today. |
| `font/` | 935 | `font.rs` | **adapt → MORROWIND-G** | Atlas management and eviction. Somnium's atlas is fixed-size; a CJK fallback face overflows it. |
| `text_box.rs` | 1,805 | `widgets/text_box.rs` | **adapt → MORROWIND-G** | Selection, multi-line, and IME. A.4 lists `text/ime.rs` as new; this is the reference. |
| `navigation.rs` | 194 | — | **adapt → MORROWIND-F** | Directional focus navigation between widgets. 194 lines, and it is the whole of gamepad UI navigation's geometry problem. |
| `widget.rs` | 2,148 | `widget.rs` (217), `node.rs` (292) | **adapt → MORROWIND-D/E/F** | See the finding above. Visibility, opacity, z-index, tooltip and context-menu attachment, hit-test override, layout transform. |
| `message.rs` | 1,450 | `message.rs` (162) | **adapt → MORROWIND-F** | Somnium's message vocabulary is thin because the editor drives most state directly. Focus, capture and routing need more of it. |
| `animation.rs` | 318 | `motion.rs` (548) | **adapt → MORROWIND-H** | Somnium's motion layer is a token-driven easing system (Phase 27-C). Fyrox's is track-based over widget properties, which is what H's "track mode" means. |
| `image.rs` | 427 | `widgets/image.rs` | **adapt → MORROWIND-D** | The widget exists; it can only reference the three fixed atlases. The adaptation is the *texture slot*, not the widget. |

### Track 2 — THE CONSTRUCTION SET

| Fyrox module | Lines | Somnium counterpart | Verdict | Reason |
|---|---:|---|---|---|
| `dock/` | 2,033 | `workspace.rs` (244), `layout_persist.rs` (181) | **adapt → MORROWIND-J** | Tiles, splitters, floating windows, and the serialised layout. Somnium's shell is a fixed arrangement with persisted splitter positions — a different and much smaller thing. |
| `window.rs` | 1,394 | — | **adapt → MORROWIND-J** | Floating, resizable, modal-capable windows. Nothing in Somnium floats. |
| `messagebox.rs` | 480 | `widgets/toast.rs` (partial) | **adapt → MORROWIND-J** | Modal dialogs with a result. CONTROL-J's scene lifecycle needs "save before closing?" and currently has no modal to ask in. |
| `list_view.rs` | 656 | — | **adapt → MORROWIND-M** | A virtualised list. §8's MORROWIND-M requires 100k rows; nothing in the tree virtualises. |
| `tree.rs` | 1,126 | `widgets/tree_view.rs` | **adapt → MORROWIND-M** | Somnium's tree is real but not virtualised. The adaptation is recycling, not the widget. |
| `absm/` | 409 | — | **adapt → MORROWIND-K/V** | An animation state-machine editor built on Fyrox's own graph surface. This is the *second consumer* §A.7 demands before Seam 8 counts as proven. |
| `test.rs` | 242 | — | **adapt → GHOSTFENCE** | A headless UI test harness. Somnium has 315 `somnium_ui` tests and, before this sub-phase, zero image assertions; this is worth reading for how a UI tree gets driven without a window. |

### Present — already in the tree

| Fyrox module | Lines | Somnium counterpart | Note |
|---|---:|---|---|
| `lib.rs` | 4,029 | `lib.rs` (7,565), `ui.rs` (2,605) | The tree, the message loop and the pool are the original fork. |
| `curve/` | 2,178 | `widgets/curve_editor.rs` | CONTROL-K. **MORROWIND does not build a curve editor** (§6.7). |
| `color/` | 1,963 | `widgets/color_picker.rs`, `color.rs` | CONTROL-D/K. |
| `inspector/` | 1,547 | `editor/inspector*.rs` | CONTROL-B generates rows from `ComponentSchema`; the census counts 28 schemas. |
| `menu.rs` | 1,373 | `widgets/menu.rs` | CONTROL-A2's 52-command registry generates the six menus. |
| `numeric.rs` | 823 | `widgets/numeric_field.rs` | |
| `grid.rs` | 799 | `widgets/grid.rs` | |
| `text.rs` | 730 | `widgets/text.rs` | |
| `tab_control.rs` | 727 | `widgets/tab_control.rs` | |
| `node/` | 713 | `node.rs` | The `Control` trait and the node enum. |
| `scroll_bar.rs` | 677 | `widgets/scroll_viewer.rs` | |
| `popup.rs` | 639 | `widgets/popup.rs` | |
| `key.rs` | 537 | `commands.rs` (1,872) | CONTROL-A2. |
| `scroll_viewer.rs` | 493 | `widgets/scroll_viewer.rs` | |
| `check_box.rs` | 482 | `widgets/check_box.rs` | |
| `dropdown_list.rs` | 472 | `widgets/combo_box.rs` | |
| `button.rs` | 467 | `widgets/button.rs` | |
| `control.rs` | 465 | `node.rs` | |
| `scroll_panel.rs` | 397 | `widgets/scroll_viewer.rs` | |
| `vec.rs` | 388 | `editor/property_editors/` | CONTROL-B. |
| `border.rs` | 370 | `widgets/border.rs` | |
| `log.rs` | 359 | `log.rs` (767) | CONTROL-I. |
| `rect.rs` | 326 | `types.rs` | |
| `wrap_panel.rs` | 325 | `widgets/wrap_panel.rs` | |
| `range.rs` | 323 | `widgets/slider.rs` | |
| `matrix.rs` | 307 | `editor/property_editors/` | |
| `stack_panel.rs` | 300 | `widgets/stack_panel.rs` | |
| `searchbar.rs` | 300 | `widgets/search_box.rs` | |
| `thumb.rs` | 205 | `widgets/splitter.rs` | |
| `toggle.rs` | 179 | `widgets/check_box.rs` | |
| `thickness.rs` | 175 | `types.rs` | |
| `canvas.rs` | 157 | `widgets/canvas.rs` | Already a direct port; the file header cites the Fyrox path. |
| `dropdown_menu.rs` | 126 | `widgets/context_menu.rs` | |
| `alignment.rs` | 72 | `types.rs` | |

### Refuse — deliberately not taken

| Fyrox module | Lines | Reason |
|---|---:|---|
| `file_browser/` | 2,965 | CONTROL-C shipped the asset seam and the content drawer. A second file browser is a second system, and §11 row 12 forbids exactly this shape of duplication. |
| `draw.rs` | 1,154 | **Phase 27 replaced the paint layer.** Fyrox emits a command list of tessellated geometry; Somnium emits a 100-byte instance evaluated analytically in the shader. Reading this file for Seam 4b would import the wrong mental model. |
| `style/` | 917 | Phase 26-Zeta owns theming, with certified contrast pairs. §Not-authorized in the plan preamble: "no re-theming anything Zeta certified". |
| `utils.rs` | 574 | A grab-bag. Nothing in it is a system. |
| `bit.rs` | 510 | A bitfield property editor. CONTROL-B's schema-generated Details is the answer to that shape of problem. |
| `decorator.rs` | 409 | Fyrox decorates by swapping brushes on state change; Zeta's token layer resolves state to a token. Same problem, incompatible answers, and Somnium's is the one that is certified. |
| `input.rs` | 324 | Widget-level input handling. **Seam 5 puts input behind an action map** (MORROWIND-AE); routing raw device state into widgets is what that seam exists to stop. |
| `uuid.rs` | 187 | Somnium identifies components by `StableId(&'static str)` and entities by generational handle. A third identity scheme is a liability. |
| `brush.rs` | 89 | Superseded by Phase 27's colour contract. |
| `loader.rs` | 75 | Resource loading belongs to `somnium_asset` and Seam 2, not to the UI crate. |

### Defer — real, small, nothing needs it yet

| Fyrox module | Lines | Revisit when |
|---|---:|---|
| `expander.rs` | 326 | A panel needs collapsible sections that `property_row.rs` does not already give. |
| `selector.rs` | 318 | Track 2 needs a segmented control. |
| `path.rs` | 254 | A path *field* is needed outside the content drawer. |
| `progress_bar.rs` | 232 | CONTROL's status-bar job chip stops being enough — e.g. MORROWIND-Q's cook. |
| `build.rs` | 195 | Never, probably; listed so nobody re-checks. |

---

## What a later session should do with this

1. **Do not read Fyrox modules marked `present`.** They are already answered and
   reading them invites a rewrite of something that works.
2. **Read the `adapt` module before designing the sub-phase**, not after. Nine
   of the nineteen are under 700 lines and readable in an hour.
3. **`refuse` rows are decisions, not backlog.** Re-opening one requires a
   reason recorded in the sub-phase that re-opens it.
4. Fyrox has moved since Somnium forked. Where its module and Somnium's
   disagree about a contract Phase 26/26-Zeta/27 froze, **Somnium wins** and
   the Fyrox module is read for the part above that contract only.
