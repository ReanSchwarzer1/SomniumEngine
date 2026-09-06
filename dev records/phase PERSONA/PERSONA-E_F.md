# PERSONA E/F — authoring tools and floating workspace resilience

Date: 2026-09-06. Base: `a51ad30` (`more ui work`) plus the current working tree. Single agent, no delegated work. Builds on [C/D](PERSONA-C_D.md); design-system, Rust, codebase-design and local code-review guidance applied.

**Status: first E/F implementation slice in tree; acceptance remains open.** This includes the reported missing foliage painter and additional defects found during native review. Full PERSONA retains its zero-known-outstanding-editor-bug gate, including floating windows.

## Authoring tools

- Landscape and Foliage have contextual left panels showing the selected target, supported controls and eligibility feedback. The old always-present sculpt column is hidden. Existing foliage controls were moved into this panel with their original handles and event routes; the kind picker now has a popup. Both authoring workspaces reserve vertical space for their controls, with the Content Drawer one click away.
- The Foliage toolbar command, palette command and F8 now activate painting. Previously `editor.foliage.edit` dispatched the component visibility toggle. The visibility checkbox is separate and now uses the generic schema edit route, including undo and scene dirty tracking.
- Hidden foliage is explained above the brush. Painting while visibility is off produces a refusal message and does not silently add invisible instances; Visible stays available to correct it.
- Landscape exposes existing operation, radius, strength, hardness and loaded paint-layer controls. Core owns values and eligibility; invalid, hidden or locked targets explain why painting is unavailable. Brush values reject non-finite input and clamp to supported ranges.
- Landscape and Foliage are mutually exclusive. Select/Finish exits both. Esc restores the active terrain stroke snapshot without adding an undo entry; completed edits use the existing stroke/undo route.

### Resource and settings boundary

This slice exposes the engine's existing built-in foliage kind palette and loaded terrain layer palette. It does not pretend that imported arbitrary assets or brush masks are supported. Brush options remain session-owned core state; authored terrain and foliage use the existing terrain sidecars and scene data. A richer persistent tool-settings/resource contract and its save/reload journey remain required by E before E can be accepted. No second UI authoring model was added.

## Floating windows

- Header Float becomes Dock, using the same retained panel and property handles. Docking the viewport also restores the primary render destination. Closing/docking saves placement.
- Per-panel physical desktop positions and logical client sizes persist in `%APPDATA%/SomniumEngine/floating_windows.json`. Writes debounce; focus loss and close flush. Missing/corrupt data recovers to defaults. Removed-monitor/off-screen placements return to a live monitor; negative desktop origins and monitor scale are accounted for.
- Minimum logical sizes: Details 320×360, Outliner 300×240, Output Log 480×240, viewport 480×320. The log toolbar wraps and keeps its return-to-dock action separate.
- Native narrow-window review found a deeper layout defect: an empty grid track retained its last arranged width during measurement. After shrinking, headers requested the old width, hiding Dock and trailing log actions. Measurement now clears that cached extent. The regression lays out a docked panel, detaches it wide, shrinks it and checks action bounds **and clipping**, using the shipped fonts.
- Detached panels now paint their own opaque background. Previously their transparent roots depended on dock ancestors, leaving black swapchain areas. The viewport retains its rendered scene underneath.
- Viewport axes no longer paint or accept hits through a modal.

## Verification

- `cargo test -p somnium_ui -p somnium_core -j1`: **1,193 passed, 0 failed, 1 ignored** (770 UI unit, 6 shader, 414 core unit/integration, 3 doc). Existing warnings remain visible. Windows `LNK1104` required relinking mapped test outputs; no tests were disabled. The old foliage preset expectation was updated because its drawer now deliberately starts closed.
- Regression coverage includes F8/registry routing, context switching and model-refresh silence, hidden-foliage feedback, modal axis hits, placement recovery at different scales/negative origins, and real-font wide-to-narrow panel clipping.
- Generated census: **218,374 Rust/WGSL lines; 2,211 discovered tests**. These structural counts are not the executed-suite total.
- The release editor was rebuilt with `cargo build -p hello_engine --release -j1`. The UI suite was repeated after the final Float/Dock label change. Native capture evidence is distinct from actual pointer/keyboard journeys and hardware monitor transitions.

## Native evidence

Release engine, coastal scene, Nocturne/Compact, frame 120 with capture-and-quit. Main captures use 1280×720 and isolated `%APPDATA%` under `target/persona-ef/profile-<tag>`. Select `Terrain` for authoring and `Post Processing` for shell/Details/modal checks. `SOMNIUM_AUDIT_UI_STATE=persona-foliage` runs the same command registry entry as F8; `persona-terrain` enters Landscape.

| Capture | Reviewed result |
|---|---|
| [Foliage](EF_foliage.png) | Entire supported brush panel visible at 720 high; hidden-foliage explanation and Visible control available. |
| [Landscape](EF_landscape.png) | Operation, radius, strength, hardness and Finish remain together. |
| [Unavailable target](EF_foliage-unavailable.png) | Selecting Post Processing explains that a Landscape is required; no unexplained inert brush. |
| [Palette](EF_palette.png) | Viewport axes no longer overlap the modal; disabled explanations visible. |
| [Shell](EF_shell.png) | Final shell candidate for GHOSTFENCE comparison. |
| [Floating Details](EF_floating-recovery-details.png), [Outliner](EF_floating-recovery-outliner.png), [Log](EF_floating-recovery-log.png), [viewport](EF_floating-recovery-viewport.png) | Minimum-size native panels; opaque backgrounds, visible Dock actions and wrapped log controls. |

Floating startup seeded each saved desktop position to `(50000, 50000)` and each size to its minimum. Windows recovered onto the available display at scale 1.5: Details 480×540 physical, Outliner 450×360, Log 720×360 and viewport 720×480. This is native OS rendering at 150%, not a resized image; it does not prove the full interactive cross-monitor matrix. The profile retained corrected coordinates and logical sizes. A second launch using that same profile, with no `SOMNIUM_FLOAT` override, reopened all four panels at the same sizes and shut down cleanly: [restart Details](EF_floating-restart-details.png), [Outliner](EF_floating-restart-outliner.png), [Log](EF_floating-restart-log.png), [viewport](EF_floating-restart-viewport.png). This verifies persisted panel membership and geometry; the audit explicitly selects Post Processing and does not prove selection or pending-edit restoration across restart.

## Repository gate

`python tools/ghostfence/run.py --fast`: **5 passed, 1 failed, 1 skipped**. Census, toolchain, shader budget, single job system and no second system pass. Full workspace tests are skipped by `--fast`; the relevant core/UI suite and final UI rerun above were run separately. `git diff --check` passes.

The preserved old golden images fail against the final native shell candidate: menu bar **98.9826%**, sculpt panel **99.9970%**, toolbar **99.9753%** changed, against a 0.2% budget. The redesigned regions differ intentionally, but that is not a passing visual gate. Reference images were not replaced. Matched visual acceptance and approval of any new baseline remain open in G. The historical 5.3333% sculpt mismatch is an older measurement, not this run's result.

## Bug ledger and open acceptance

| Reproduction | Fix / verification boundary |
|---|---|
| Foliage Mode or F8 changes visibility instead of entering paint mode; brush hidden under old controls | Shared command corrected; contextual panel reachable. Registry/F8 regression and native command-driven capture. |
| Paint while foliage visibility is off gives no visible result | Explicit panel hint and refusal; Visible remains enabled and uses schema undo. Native hidden-state capture and presentation regression. |
| Select can leave painting active | Both paint modes cleared by Select; native toolbar state follows core mode. Full paint/undo/save/reload journey remains open. |
| Shrink a floated panel; Dock/log actions disappear | Empty-track measurement reset; real-font detach/resize/clip regression. Native minimum-size recapture confirms wrapped log actions; explicit Float/Dock labels replace ambiguous header glyphs. |
| Floating panel background becomes black | Paint detached ground before its subtree; viewport exempt. Native recapture confirms opaque panel backgrounds. |
| Viewport axes overlap command palette | Suppress modal overlay drawing and hits; regression and native recapture. |
| Historical asset drag appears inert | C/D model-driven picker feedback is fixed. Both drag and Use Selected with undo still require native end-to-end proof. |
| Monitor/DPI/state continuity | Geometry unit coverage and native off-screen startup test are partial evidence. Header drag, focus loss mid-drag, redock/reopen/restart, real monitor removal and 100/125/150/200% transitions remain acceptance work. |

The five designer journeys, richer E resource/settings contract, remaining F OS matrix, G accessibility/performance/human review and preserved-golden approval remain open. No zero-bug, full E/F completion, frame-time improvement or hardware DPI transition claim is made.
