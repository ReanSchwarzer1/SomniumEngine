# PERSONA-H — Charon II

Date: 2026-09-09. Base: `b58e788`. Status: first implementation pass in tree; full H and visual acceptance remain open.

## Decision before implementation

Start H before I. Under expansion §8.1, ship a coherent first subset: shared interaction policy, button press/hover, checkbox glyph transitions, and node lifecycle cleanup. Preserve the user's instant Content hover preference and flat dark panel bodies. No new dependencies, renderer changes or native captures in this pass. The earlier user instruction to avoid captures remains in force; native visual acceptance stays pending.

Use the existing bounded spring-shaped easing for 90 ms press feedback and existing named easing for hover/toggle. The more extensive authored-curve, spatial-stagger and travel work will follow with its consumers, rather than introducing unused machinery. No job loop or new glow role ships here. The phase-27 amendment records the expansion permissions without claiming them implemented.

## Delivered

- `motion::policy` owns hover, press and checkbox timing. Repeated draws observe the requested destination instead of restarting a track. Retargets begin at the current presentation value; first-seen model state is already settled.
- Buttons, icon buttons and toggle buttons use the same 90 ms bounded press wash and 0.985 vertical face compression. Labels and click targets stay stationary. Checkbox glyphs scale/fade over 140 ms; underlying values and mixed-state reporting update immediately. This first pass deliberately uses non-overshooting timed easing instead of adding unused spring presets.
- Existing button and TreeView hover now use policy. Content tiles retain their explicit immediate-hover opt-out, including when interrupting a transition. Primary-button gradient stops blend with the fill so the gradient cannot hide pressed feedback.
- Node destruction clears live and settled animation state recursively. A stale generational handle cannot clear a new node that recycled its index.
- MotionProperty adds the four named expansion properties. ScaleY has its first consumer; OffsetX, Flash and Stroke remain reserved for the later H/J consumers. No travel category, loop or new glow implementation is claimed here.
- Both token sheets move to `0.4.0-persona`, adding only press/toggle scale and toggle duration. Dawn and high contrast inherit the same metrics; parity tests cover them. Surface colors, panel gradients, density and radii are unchanged.

## Bug ledger

| Defect at `b58e788` | Reproduction and fix | Evidence |
|---|---|---|
| Node-index reuse retains unrelated motion, including settled state | Start a node track, destroy its subtree, allocate a replacement at that index. `remove_node` now calls `forget_node` for each valid descendant before freeing it. | Regression checks recycled indices, descendant settled values and stale handles. |
| Hover restarts on every redraw | Hover a button/tree row while drawing each frame. The old draw code repeatedly reset elapsed time. Policy starts only when the requested target changes. | Mid-transition tree input test reaches the exact hover color and idle at 120 ms while redrawing; rapid press reversal/hold tests settle within the feedback ceiling. |

## Verification and remaining work

`cargo test -p somnium_ui -j1`: **785 passed, zero failed** (778 unit, 6 shader-source, 1 doc). This includes the new 60-frame idle-shell guard, unchanged byte-identical idle paint checks, reduced-motion endpoints, live widget hit/layout and accessibility-value checks, node lifecycle checks, token parity and existing contrast/density coverage. The old tree-hover test now samples midpoint and completion instead of assuming an instantaneous fill.

`cargo build -p hello_engine --release -j1`: passed. Generated census: 219,590 Rust/WGSL lines and 2,225 discovered tests. `git diff --check` and expansion/H record links passed. No native launch, screenshot, motion capture or golden rewrite was performed, following the user's earlier capture preference. This is automated/source evidence, not native visual acceptance. No commit or push was made.
 Full H's remaining focus/selection/tab, popup/modal/drawer, toast, fold, workspace/floating and group choreography remain open, as do I–L and G acceptance.

## Second pass — anchored popup continuity

The shared `Popup` path now provides 140 ms opacity/0.96-scale entry from its anchor and a 100 ms opacity-only exit. If placement flips above the anchor, the visual origin follows the adjoining edge. Closing and reopening mid-transition continue from the current presentation value. This covers anchored menus/dropdowns built with Popup; centered modals, the palette and bottom-centered drawers keep their current behavior until their dedicated H pass. Sibling-menu switching still needs the special 90 ms cross-fade policy from row 16.

Logical visibility, focus handling and accessibility remain on the existing immediate route. A closing popup is paint-only and cannot intercept pointer input. Its retained children keep their last layout for the brief exit; no frame-image cache, duplicate panel or extra renderer is introduced. Popup entry accepts clicks and text at final layout coordinates from its first frame.

A scoped draw-list transform handles quads, shaped geometry, all alpha channels and scissor rectangles together, then separates the next sibling's batch. Only transitioning popup scopes pay this work; settled and reduced-motion draws use the original path. Numeric/model values and the primitive/shader layouts are unchanged. The popup scale and close duration are exported in both version-0.4 token sheets and tested for parity. No panel colors or hover preferences changed.

Verification: **788 passed, zero failed** via `cargo test -p somnium_ui -j1` (781 unit, 6 shader, 1 doc). Three added regressions cover mixed-stream/scissor isolation, live typing plus close/reopen/hit behavior, and exact settled-vs-reduced-motion paint/layout in Nocturne and Dawn. Existing docking, popup rehosting, idle and contrast tests remain green. Windows linker locks required retrying the same test command; no tests were skipped. Release rebuild passed with `cargo build -p hello_engine --release -j1`. Census: 220,004 Rust/WGSL lines and 2,228 discovered tests. Diff whitespace and H/expansion links passed. No native capture or golden replacement was performed; actual motion/floating-window visual acceptance remains open.
