# PERSONA — designer QoL follow-up

Date: 2026-09-06. Base: `5a3ce87` (`more ui work`) plus this working tree. One agent, no delegation. Design-system and codebase-design guidance applied.

**Status: implementation and focused checks complete; user visual review and phase acceptance remain open.** This follows [E/F](PERSONA-E_F.md). Material graphs remain deferred. The six supplied screenshot paths were unavailable, so this pass used the user's descriptions and source inspection. No fresh captures or reference-image replacements were made, as requested.

## Changes

- **Advanced and Scripts:** legacy section labels, control heights and text alignment now follow the generated Details styling. Scripts appear before the Advanced fold. Attachment actions wrap in narrow panels.
- **Materials:** selecting a `.sommat` in Content, including a newly created material, exposes its existing generated color, texture and material properties in the Materials workspace. New Material and Save are available there. Texture selection retains the material target for Use Selected. Selecting a scene object returns to its material. Save uses the existing scene/material save boundary; the graph is not implemented.
- **Lighting:** the workspace now hosts existing selected light/environment properties and Point, Spot, Sun and Area creation actions. Both workspaces reuse the same property widgets and undo routes as Details. Changing workspace exits an active paint mode.
- **Content hover:** tiles change hover immediately, without accumulating fade trails; selected and disabled button paint no longer gets overwritten by hover interpolation.
- **Cancel and rename:** name dialogs no longer submit on field blur before Cancel receives its click. Enter still submits. Inline rename explicitly focuses its text widget, and Escape cancels the rename before other overlay handling.
- **E resource/settings:** terrain operation/layer and foliage resource are stored by name/path, with brush options, in `%APPDATA%/SomniumEngine/authoring_tools.json`. Settings restore on launch and save on committed changes, not every pointer update. Unknown versions, malformed files, unavailable resource identifiers and invalid numeric values recover to defaults or supported bounds. Foliage scale limits remain ordered.

Standalone material editing uses temporary asset sessions with the existing material document cache, field edits, undo and save machinery. These sessions are excluded from the Outliner, Select All and scene serialization. No second material format or parallel property editor was added.

The E resource contract currently covers built-in foliage resources and loaded terrain layers. Arbitrary imported paint resources and custom brush masks are not exposed as supported tools. Authored terrain/foliage continue using their existing stores; brush preferences are user settings and do not dirty the scene.

## Verification

- Core and UI suites passed: **1,198 passed, 0 failed, 1 ignored**, combining the core run with the final UI rerun (417 core unit/integration, 772 UI unit, 6 shader and 3 passing doc tests). Windows mapped test executables required bounded relinking; no tests were disabled. Existing compiler warnings remain.
- New regressions cover modal blur versus Enter submission, registered lighting/material actions and workspace visibility, standalone asset sessions staying out of serialized scenes, and brush preference serialization/fallbacks.
- The release editor was rebuilt with `cargo build -p hello_engine --release -j1`, including the final workspace paint-exit change.
- Generated census: **218,972 Rust/WGSL lines; 2,216 discovered tests**. This is a structural count, separate from executed tests.
- `git diff --check` passed. No new screenshot, GHOSTFENCE run or golden update was requested or performed.

## Visual feedback follow-up

The user confirmed the QoL functionality, but rejected the grey gradient visible behind Details and material properties. The new component/script containers and tool layout containers now use transparent backgrounds, restoring the existing dark panel grounds. Theme colors and property behavior are unchanged. No fresh screenshots were taken.

## Review boundary

The fixes above have source and automated evidence; their complete native pointer/keyboard journeys have not been rechecked in this pass. User review should cover material creation/edit/save/reload and texture selection, browser Cancel/rename/hover, Scripts access, and restored terrain/foliage settings. Existing floating-window journeys and the F OS/DPI matrix remain part of phase acceptance.

Visual-baseline approval belongs to the user. The earlier golden mismatch remains recorded in E/F; references are preserved. PERSONA is not marked bug-free or fully complete on the strength of these tests. Its zero-known-outstanding-editor-bug gate, including floating windows, remains binding.

## Expansion feedback — 2026-09-09

The user clarified that “texel viewer” means **viewport texel-density visualization**, not an asset texture viewer. This pass prioritizes their screenshots and usability feedback while the remaining H–L work stays open.

- Create and View share the bounded command-menu builder. Menus show at most 16 rows, fit the available space above/below the anchor, scroll by wheel/scrollbar and reveal keyboard focus. Short menus shrink to their contents. The duplicate Create builder is removed, preserving command IDs and routing.
- Preferences title, tabs, search, Modified only and Reset All align by measured control bounds. Transport buttons share the neighbouring mode-button height and centreline; icon/text groups, Output Log heading and Content breadcrumbs no longer depend on guessed top padding. Content navigation arrows are horizontally centred.
- Numeric values and units have separate clipped space and measured vertical alignment. Redundant decimal zeros compact only when the resting value would crowd its suffix (for example, `500.000ms`); stored values and text editing are unchanged.
- **View → Texel Density** appears next to Lit and is also available through the command palette. Mode 35 uses base-colour texture dimensions and world-space triangle/UV area to report mesh texels/metre, including rectangular textures, mirrored UVs and model scaling. Blue/cyan/green/yellow/red correspond to 128/256/512/1024/2048, interpolated logarithmically and clamped at the ends; grey means no applicable texture or degenerate UVs.
- Terrain reports the nominal authored density of the dominant painted layer, including when clipmap rendering is enabled. It does **not** measure cache residency, the final height-blended mixture or cliff-projection stretch. The persistent viewport legend states the terrain distinction. The legend sits above the bottom-left selection badge, clear of camera/snap controls, travels with the viewport subtree and paints beneath menus/dialogs. Exposure, grading, vignette and bloom do not alter this mode; Lit restores the normal post-process settings. The legacy numeric debug route updates the active mode/legend too.

Validation: the final UI suite passes **790 tests** (783 unit, 6 shader, 1 doc). Renderer validation passes **499 tests** (468 unit, 31 composed-shader/renderer guards), including all registered view branches and Naga validation. The menu regression covers the actual Create builder and View at 360/720 px height, the last command by wheel and keyboard focus, and compact short menus. Source/automated checks do not constitute native visual acceptance. Generated census: **220,314 Rust/WGSL lines; 2,230 discovered tests**. No screenshots or golden changes were made.

Release verification: `cargo build -p hello_engine --release -j1` passed after the final legend-placement correction (`target/persona-qol-release.log`). `cargo check -p somnium_ui -j1` passed after unused-import cleanup; `git diff --check` is clean. The editor executable is `target/release/hello_engine.exe`. Remaining H–L work and user visual acceptance stay open; no commit or push was made.


## Shared picker repair — 2026-09-09

User screenshots showed asset selectors painting over the whole editor and terrain lists extending almost to the bottom of the window. A headless regression reproduced both on base `28b65cc`: the asset popup painted its full-window click catcher, and 200 choices measured 4,800 px tall.

All combo selectors now use `editor::parts::picker_popup`: a transparent click catcher, one raised popup frame, fixed optional search/actions, and the existing ScrollViewer as the body. The cap is twelve dense rows plus the frame (290 px at the standard density), further constrained to the anchor's available window space. Normal selectors prefer 280 px width; asset selectors prefer 360 px, constrained to the host window. Short and filtered lists shrink. Terrain layers, material textures, reflected enums/assets and shell selectors use the same base. The scrollbar supports dragging and the wheel; long labels ellipsize and only visible rows paint.

The scroll offset clamps after content layout, resets when filtering changes results, and reveals the current row on opening. Repeated unchanged model snapshots do not pull the scrollbar back. Thumb dragging uses its actual travel rather than assuming the minimum thumb size.

Popup ownership is now exposed by the ComboBox itself. Editor dismissal discovers generated asset/enum/settings pickers through that link, so they participate in Escape, outside-click and sibling-dropdown handling. Removing a rebuilt Details/settings control also removes its root-parented popup. Content places updates go through the combo's messages instead of assuming its list is the popup's first child. Existing floating-window popup rehoming remains in use.

Validation: **789 UI unit tests passed**, zero failures. The two regression scenarios exercise the real generated asset picker and shared terrain/combo path, including 200 rows, narrow/bottom-right window placement, gutter hit testing, dragging to the final row, wheel return, selection events, stable model updates, pinned asset actions/search, filtering shrink/reset and popup cleanup. No new screenshot or golden replacement was taken. `cargo build -p hello_engine --release -j1` passed; the rebuilt executable contains these fixes (log: `target/persona-picker-build.log`).
