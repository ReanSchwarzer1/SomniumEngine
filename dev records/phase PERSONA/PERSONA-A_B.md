# PERSONA-A / B — baseline and Nocturne v2 foundation

**Date:** 2026-09-05. **Base:** `b988996` on `dev`; B evidence is the working tree on that base.
**Scope:** Native editor baseline/redlines and shared visual foundations. PERSONA as a whole remains open; C–H and the phase bug-closure gate are not complete. One agent; no added dependencies or renderer changes.

## A — native baseline and redlines

The existing capture hooks produced these baseline images before the source edits:

| Surface | Before | After |
|---|---|---|
| Shell, selected Post Processing, drawer open, 1280×720 | [A](A_shell_1280x720.png) | [B](B_shell_1280x720.png) |
| Same shell at 1920×1080 | [A](A_shell_1920x1080.png) | [B](B_shell_1920x1080.png) |
| Command palette, 1280×720 | [A](A_palette_1280x720.png) | [B](B_palette_1280x720.png) |
| Unsaved-changes dialog, 1280×720 | [A](A_modal-unsaved_1280x720.png) | [B](B_modal-unsaved_1280x720.png) |

Inputs: `hello_engine` coastal startup scene/camera; `SOMNIUM_AUDIT_SELECT_ENTITY=Post Processing`, `SOMNIUM_AUDIT_UI_STATE=shell|palette|modal-unsaved`, `SOMNIUM_AUDIT_WINDOW_SIZE` as listed, frame 120, capture-and-quit. Both passes use process-local `APPDATA=target/persona-ab/profile` to preserve personal window layouts. Default Nocturne/Compact/standard contrast. A uses the previously built release executable from `3c4e33a`; `b988996` added the phase documentation without engine changes. B shell/palette/modal use the fresh release build with the completed shared recipes; gallery images use the subsequent release build adding gallery-only viewport-overlay isolation. The gallery-only flag defaults off and does not affect normal editor captures.

**Limits:** Window dimensions are capture inputs/output extents; monitor DPI was not independently measured. Scene clock, asset streaming and recovered autosave were not isolated. The recovered-autosave toast is visible in both passes. Compare UI regions, not scene pixels or total FPS. These are review captures, not replacement goldens, performance measurements, or observed designer task timings.

Redlines refer to the 1280×720 baseline:

| Region | Finding and target | Ownership |
|---|---|---|
| Top bands, y0–68; tool/header strips | Repeated strong washes make every surface equally prominent. Flatten ordinary buttons/panels; reserve depth for layers and a named primary action. | B foundation delivered; C composition remains |
| Left column, x0–168 | Sculpt persists during object work. Replace with a contextual mode/tool arrangement while preserving command identities. | C |
| Right side, x940–1280 | Diagnostics consume the first visible Details content. Selected-object identity/properties must precede diagnostics. Retain stable selection and undo routing. | C/D |
| Drawer, y476–695 | 22-unit controls and large folder art have mismatched emphasis. Give frequent controls proper hit areas, then improve breadcrumbs, filters and result layout. | C/E |
| Palette, central popup | Disabled reasons visibly truncate at the right edge. Reserve/wrap reason text and keep keyboard actions reachable at minimum width. | C/D bug ledger |
| Fields, rows and selection | Required control borders are too faint; focus replaces error; active/inactive selection looks identical. Separate semantic roles and compose outlines. | B foundation delivered |
| Panel float controls and reset gutter | Tiny hit areas remain. Grow hit regions without changing panel ownership, reset semantics or glyph size. | C/D/F |

The survey's source/status reconciliation still applies: shaped text is enabled; floating panels, presets, persistence and semantic asset actions already exist. Arbitrary redocking and the recorded asset-assignment issue must not be silently labeled complete.

## B — implemented foundation

- Versioned Nocturne/Dawn token sheets (`0.3.0-persona`) and matching Rust snapshots. Nocturne uses blue-charcoal surfaces, brighter silver text, iris actions and a separate warm modified cue. Muted text is `#9EABC0`, brighter than the initial `#929DB3` proposal so essential text also clears 4.5:1 on raised/hover surfaces.
- Decorative separators and required control edges are different tokens. Opaque selected and inactive-selected fills no longer depend on the viewport behind them.
- Compact rows/commands are 24/28 units; Comfortable uses 28/32. Tree rows are 26/30. Active-theme metrics and typography replace fixed Nocturne reads throughout shared widgets and editor builders. Density is independent of DPI; existing literal toolbar/header sizes are still C work.
- Explicit Primary, Secondary, Quiet, Toggle and Destructive button variants. Ordinary controls and panel bodies are flat. Primary-action elevation/gradient remains intentional. Explicit button variants supply label foreground through a scoped draw context; that color cannot leak into sibling widgets. Explicit authored text colors remain overrides.
- Focus is a 2-unit outline. An inner validation edge survives focus; both strokes fit within the widget clip. Disabled controls do not retain a focus glow/ring. Tree selection keeps a rail when inactive and uses a quieter fill/foreground.
- High-contrast snapshots derive essential text/control/status colors from relevant semantic backgrounds. The accessibility setting feeds the same active-theme selection. No per-glyph contrast recomputation was added.
- Panel titles use the existing Section typography role. The fonts, retained tree, shader instance format, command registry, property schema, undo model and floating-panel ownership remain intact.

Runtime values live in `crates/somnium_ui/src/theme.rs`; exported design values live in `crates/somnium_ui/assets/tokens/{nocturne,dawn}.tokens.json`. Update them together: existing semantic-color parity now also checks density, geometry, typography and motion. Historical Zeta token packages are not the current edit target.

For startup verification, `SOMNIUM_UI_THEME=dawn` and `SOMNIUM_UI_DENSITY=comfortable` select the alternate snapshots before UI construction. Defaults remain Nocturne/Compact. This is not a new persisted density preference or a claim that every fixed editor dimension responds to live density changes.

## Native component gallery

Use `SOMNIUM_AUDIT_UI_STATE=persona-gallery` with the startup variables above and `SOMNIUM_AUDIT_HIGH_CONTRAST=0|1`. The gallery uses native retained controls and the real paint recipes. It is an audit surface, not a web mockup or another UI framework.

The state matrix shows Rest, Hover, Pressed, Selected, Inactive selection, Focus, Invalid, Focus + error, and Disabled for five button variants, Input, Tree row and Asset tile. Inapplicable combinations intentionally fall back to that recipe's supported state. Live text, numeric, mixed-value and button controls sit below it. The modified cue is currently a warm dot; the named reset affordance/diamond and workflow-specific busy/drop/error messages belong to subsequent property/asset slices.

| Theme | Compact | Comfortable |
|---|---|---|
| Nocturne | [Standard](B_gallery_nocturne_compact_hc0.png) · [High contrast](B_gallery_nocturne_compact_hc1.png) | [Standard](B_gallery_nocturne_comfortable_hc0.png) · [High contrast](B_gallery_nocturne_comfortable_hc1.png) |
| Dawn | [Standard](B_gallery_dawn_compact_hc0.png) · [High contrast](B_gallery_dawn_compact_hc1.png) | [Standard](B_gallery_dawn_comfortable_hc0.png) · [High contrast](B_gallery_dawn_comfortable_hc1.png) |

A state sheet proves native appearance, not all input interactions. Real designer journeys, mixed-script caret behavior and OS DPI transitions remain in the phase acceptance matrix.

## Validation and remaining gates

- `cargo test -p somnium_ui -j1`: **752 unit tests + 6 shader integration tests + 1 doc test passed**, zero failures. Includes six new PERSONA tests: exported metrics, theme/density/contrast combinations, clipped focus+validation composition, action/inactive-selection grammar, button label scope, and native gallery layout/viewport-overlay isolation. Existing detached-tree/popup/focus tests remain in this suite.
- `cargo build -p hello_engine --release -j1`: succeeded with existing dependency/documentation/dead-code warnings. During validation, intermittent Windows linker `LNK1104` failures affected test executable outputs; reruns succeeded. These were build-attempt failures, not hidden passing results.
- `python tools/ghostfence/run.py --fast`: **5 passed, 1 failed, 1 skipped** after refreshing the generated census. The failure is the preserved old golden reference: menu-bar **98.9826%**, sculpt-panel **99.8155%**, toolbar **99.9321%** changed beyond tolerance, against a 0.2% budget. Candidate is the new B shell capture; these figures must not be confused with the historical 5.3333% sculpt mismatch. Whole-workspace tests were skipped by `--fast`; the focused UI suite above ran separately.
- No reference images or thresholds were replaced. PERSONA-H must review and intentionally approve the eventual visual reference once composition and known bugs are resolved. This delivery is not a green whole-phase gate.
- The existing shell paint-budget test now asserts a ceiling of four ordinary gradients/shadows rather than obsolete minimum counts requiring decorative gloss. The fixture measured one gradient and one elevated primitive; rounded shapes, recessed fields and border checks remain. This is a paint-complexity check, not a CPU/GPU timing claim.
- `git diff --check` and local Markdown image/link existence checks pass. Capture review confirms shell/palette/modal changes and the native state matrix. No new multi-monitor, physical-DPI, designer-timing or UI p95 measurement is claimed.

## Bugs and floating-window contract

**Do not close PERSONA with known unresolved in-scope bugs.** This is a release criterion; it is not a claim that testing can prove the absence of every possible defect. A/B does not waive the criterion for later slices.

| Issue | Current status / required closure evidence |
|---|---|
| Focus overwrites validation border | Fixed in B; test asserts both distinct strokes stay inside the field clip. |
| Hidden-entity label lost its muted color during migration | Fixed before delivery; retained hidden color override alongside inactive selection. |
| Native gallery canvas produces unbounded sheet dimensions; viewport axes overlap samples | Fixed before delivery; finite full-window bounds and gallery-only overlay isolation, with native regression test and replacement captures. |
| Explicit action label does not inherit its recipe color | Fixed in B; subtree/sibling regression test. |
| Palette disabled-reason clipping | Reproduced in A. Keep open for C/D; verify minimum-width native capture and keyboard use after fix. |
| Unsaved-dialog last button meets/clips the right panel edge | Visible in both A and B at 1280×720. Open C layout work: reserve complete action widths and trailing inset. |
| Diagnostics obscure selected properties | Reproduced in A. Keep open for C/D; selected-object fields must be visible first. |
| Tiny float/reset/drawer actions | Open C/D/F layout work; test actual hit rectangles and keyboard alternatives. |
| Asset-to-Details dragging reportedly does nothing | Recorded issue, not freshly reproduced in this slice. Remains open until actual assignment and undo work in the native editor. |
| Historical sculpt golden mismatch | Reference stays unchanged; current gate results are recorded separately. |
| Floating windows | Existing detach/reattach, popup-host, capture/focus and panel-state tests remain mandatory. Shared recipes/metrics apply to these same widgets. Native multi-monitor DPI, narrow resize, titlebar drag, minimize/maximize, close/redock, monitor loss, restart restoration and focus/undo continuity are still F/H acceptance work. Never replace them with a screenshot enlargement or a static route test. |

Next implementation work is PERSONA-C: compose the shell and populated Details around the new foundation, then complete the D/E designer workflows. PERSONA retains priority over other phases.
