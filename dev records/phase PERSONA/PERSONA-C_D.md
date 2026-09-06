# PERSONA C/D — workspace and everyday controls

Date: 2026-09-05–06. Base: `a51ad30` (`more ui work`), plus the current working tree. One agent; no delegated work. The design-system, codebase-design, Rust and code-review guidance informed this work. Existing PERSONA research and native A/B evidence supplied the design direction; no duplicate internet survey was needed.

**Status: C/D implementation in tree; native visual review complete for the captures below. Designer-journey acceptance remains open.** PERSONA remains the priority before other phases. This record does not close the five designer journeys, the floating-window OS/DPI matrix, or the phase-wide zero-known-bug gate.

## What changed

- Shell: workspace selector uses the eight existing presets. Layout starts with a 48 px tool rail; Tools opens the authoring rail and preserves the Details width (the E/F follow-up makes its controls contextual). Terrain opens that rail; unrelated workspaces collapse it. Dock-tree projection and persisted layout accept the smaller rail. Outliner reserves more height for Details in short windows.
- Details: selected-object identity stays above scrolling properties. Repeated Name metadata follows authored properties. Generated component/group sections precede the collapsed Advanced renderer/tool controls. Section folds persist; searches temporarily reveal matching folded sections without discarding saved folds.
- Details QoL: All, Modified and Pinned filters, search across schema names/labels/groups/help, and Clear. Pins and reset have separate 24 px targets. Focus a row and press P to pin or Backspace to reset through the existing schema/undo route. Mixed rows can reset even without a modified dot. Pins use component StableId plus schema field name: FieldId is positional in this engine, so persisting that numeric index would misidentify fields after reordering.
- Content: navigation and breadcrumb above full-width results; visible kind chips; explicit Name/Type/Largest/Newest sort and Compact/Comfortable/Large tile menus. Favorites and the last 12 distinct locations are available through Places. These preferences and property pins/folds live in `%APPDATA%/SomniumEngine/persona.json`. Invalid/missing preferences recover to defaults; settings-dialog presentation is separate and does not overwrite main Details preferences.
- Asset feedback: generated asset pickers now follow the actual scene value after drop, Use Selected, reset or undo. An assigned value remains representable when picker search excludes it. Feedback names the field and asset only after the model changes. Existing semantic refusal reasons and alternative assignment routes remain in use. This fixes a stale display path; it does not by itself explain every historical drag report.
- Live Details refresh also updates enum, text and color displays. Mixed numeric/text/combo state refresh preserves active edits and emits no authoring event.
- Outliner: All/Visible/Hidden/Locked scope chips reuse existing query syntax; labels ellipsize before hidden/locked badges. Jobs show the current job/progress, the number of additional jobs, and a named Cancel tooltip.
- Palette: each result has a separate explanation line, with room for disabled reasons. Unsaved dialog reserves additional action width. Float header buttons have larger hit rectangles.

All controls remain in the native retained UI. Shared widget handles, schema bindings, commands, selection, undo and the job system keep their existing owners. Floating panels retain the same widgets and use the existing popup-host routes.

## Validation

- `cargo test -p somnium_ui -j1`: **760 unit + 6 shader + 1 doc = 767 passed**, zero failures. Includes existing drag/drop, focus, detach/reattach, popup-host, generated-field and dock-tree regressions.
- New coverage: filter/fold interactions without rebuilding live rows; pin identity across field reindexing/renaming; distinct pin/reset keyboard events; bounded/serializable Recent; populated Details geometry at both target sizes; mixed refresh preserving an active numeric edit; filtered asset choices retaining a newly assigned value and reset/undo to None.
- The first test run exposed the 48 px rail conflicting with dock-tree minimums and a narrow-window floor; these were fixed and the workspace tests pass. A search expectation was corrected because roughness also matches its texture field.
- Windows temporarily refused the mapped UI test executable (`LNK1104`); rerunning the same suite succeeded. Existing warnings were not suppressed.
- `cargo build -p hello_engine --release -j1`: passed. Native captures listed below. The later E/F record supersedes the test and census totals for the combined tree.

## Native evidence

Captures use the release engine, the existing coastal scene, Post Processing selected, Nocturne/Compact, frame 120, and an isolated APPDATA under `target/persona-cd/profile-<state>-<size>`. The engine capture hook exits after writing. Startup overrides use existing UI routes. These are native captures, not web mockups or resized images.

| Capture | Reviewed result |
|---|---|
| [1280×720 shell](CD_shell_1280x720.png) | Exposure value/toggles visible before the drawer; responsive Outliner height; selected kind/filter state and navigation readable. |
| [1920×1080 shell](CD_shell_1920x1080.png) | Exposure, tone mapping and color groups visible together; complete drawer tile labels/metadata. |
| [Search](CD_persona-search_1280x720.png) | Bloom search leaves the two matching rows, preserves sticky identity and Clear. |
| [Palette](CD_palette_1280x720.png) | Disabled reasons have complete second lines. Review also found viewport axes painting over the modal edge; fixed in the E/F follow-up and recaptured there. |
| [Unsaved dialog](CD_modal-unsaved_1280x720.png) | Save, Don't Save and Cancel fully contained with trailing inset. |

The first capture put repeated Name metadata before authored fields; the corrected captures above replace that intermediate image. Native FPS text is not a performance benchmark.

## Bug and acceptance ledger

| Item | Evidence / remaining work |
|---|---|
| Selected properties buried under diagnostics | Generated fields now precede Advanced; layout regression passes at 1280×720 and 1920×1080. Native 1280/1920 review passed for property visibility. |
| 48 px rail expands back to 80/120 px or shrinks the viewport | Dock projection, minimums and toggle sizing corrected; preset round-trip and narrow-window tests pass. |
| Stale asset picker after external assignment/reset/undo | Model-driven picker refresh added; filtered-current-value regression passes. Native drag and Use Selected plus undo still need a full journey. |
| Stale mixed/enum/text/color display | Snapshot refresh now reaches these controls. Active numeric edit preservation has regression coverage. |
| Palette reason and unsaved-dialog action clipping | Native clipping review passed; modal axis overlap was then fixed in E/F. |
| Foliage painter not discoverable (user report during follow-up) | C/D collapsed the old tool section. E/F reparents it into the contextual rail and corrects the pre-existing Foliage Mode/F8 command misrouting; see E/F evidence. |
| Reported asset dragging does nothing | Remains an open investigation until both native assignment routes and undo are exercised. Static routing tests and the stale-display fix are not closure. |
| Floating windows | Existing shared-widget tests pass. Native float/redock, focus loss, narrow resize, restart, monitor loss and 100/125/150/200% DPI checks remain required. |
| Five designer journeys | Find/edit/save/reopen; both assignment routes; terrain/foliage authoring; multi-selection/mixed-axis/reset/undo; workspace recovery require end-to-end proof. No timing or human-usability pass is claimed. |
| Remaining planned work | The [E/F first slice](PERSONA-E_F.md) implements contextual authoring and workspace repairs. E resource/settings and journey acceptance, F OS/DPI matrix, and G finish/acceptance remain open. This does not defer defects in shipped behavior. |

Full PERSONA closure still requires **zero known outstanding editor bugs in the audited scope**, including floating windows, after the documented reproductions are rechecked. A build or screenshot alone cannot satisfy that gate.
