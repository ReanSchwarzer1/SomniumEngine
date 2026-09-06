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
