# Phase ALYX — Source 2

> *Half-Life: Alyx shipped with Hammer 2 and the Source 2 tools: every panel has a picture, every action has a
> face, and nothing a level designer touches is a bare list of words.*

> **Codename:** ALYX (Valve, Source 2, 2020). Chosen because Source 2's tool suite is the reference for a modern
> editor that is dense, quick and friendly at once. No Valve code, art, icons or layouts are copied; Unreal and Flax
> are ideas only (their licences); Godot, Fyrox, O3DE, Stride, Esoterica, Overload and the other MIT/Apache engines in
> `Downloads\GE\example_repo` may be studied and translated with attribution.
>
> **Status:** A–I and M in tree 2026-10-18 (uncommitted); welcome screen and the E follow-ups open. One track at a time, each shown in a native capture before the next.
> **Asked for by the developer, 18 Oct 2026:** "make the engine look more modern and usable ... lots of stuff that
> other modern engines have for qol or ui/ux, Somnium doesn't have ... the animation window is unusable, the material
> window is tacky, the mcp window can be better, more settings and preferences etc., better content browser, more
> icons ... anything that is not there i need it there in the engine". Icons and UI art through
> `Downloads\Craft_Source` (VectorCraft). Purely engine work; nothing for TSF.
>
> **Predecessors:** 26 / 26-Zeta (information architecture, Nocturne tokens, Tabler icons), 27 Hades (paint layer),
> CONTROL (schema Details, command registry), MORROWIND (animation, graphs, designer tools), PERSONA (Nocturne v2,
> workspaces, QoL; H–L and G acceptance still open). ALYX does not re-theme what PERSONA certified and keeps its
> standing user rules: dark panels, no grey gradient behind Details or material properties (rejected 6 Sep).

## 1. Where it stands — the "before" tour

Fourteen native captures of the editor as it is, taken 18 Oct 2026 with the TSF project open
(`SOMNIUM_AUDIT_UI_STATE`, which now also takes `cmd:<command id>[+<id>...]`), are in
[`phase ALYX/before/`](<phase ALYX/before/>). What they show:

| Surface | What a designer meets |
|---|---|
| Animation workspace | The viewport is replaced by a cramped node graph (clip → state → output) over an empty timeline and an unlabelled curve strip. No preview, no clip or skeleton list, no transport, no tracks or keys, tiny type, nodes cut off. "Create Animation Preview Rig" is the only way to see anything and it lives in another workspace. |
| Materials workspace | A text block ("No target selected ... Material graphs are planned for a later phase") and two text buttons. No preview, no slots, no presets. |
| Tools / MCP panel | One column of flat text buttons: Capture, Play, Stop, Pause, Step, Revoke MCP, then a repeated "Game / TSF · X — Undoable game preset — Create" list. No status, no clients, no activity, no permissions. |
| Menus | Long flat lists with no icons, no groups, no submenus. Window holds 20+ entries, "Float: …" items among workspaces. |
| Palette | No icons; every row says "Command"; a second-line shortcut overlaps the next row. |
| Preferences | A modal with labels made from field names ("Snap Translate M", "Tooltip Delay Ms"), two tabs, no category sidebar, few settings. |
| Content Drawer | Big folder tiles, no folder tree, one breadcrumb, nothing about the selection. |
| Viewport | Camera speed, Profiler, Resolution, Snap, Float. No transform-mode, space, view-mode or camera controls on screen. |
| Behaviour graph | A bare canvas with text buttons. |

## 2. What modern editors give that Somnium lacks

Studied in Unreal 5 (ideas), Unity 6 (ideas), Godot 4.7 (MIT source in `example_repo/godot-4.7.1-stable/editor`),
Source 2 Hammer (ideas), Fyrox's editor (MIT), O3DE (Apache) and Blender (behaviour):

- An icon on every command, asset type, component and panel; menus with sections, submenus and right-aligned shortcuts.
- A viewport toolbar: transform mode, local/world, snap toggles with values, view mode (lit, unlit, wireframe, buffers),
  camera (perspective/orthographic views, speed), show flags, stats.
- A content browser with a folder tree, breadcrumbs, type filter chips, grid/list with a size slider, thumbnails,
  favourites and recents, a context menu (new, rename, duplicate, delete, show in explorer, copy path), an item count
  and a selection footer.
- An inspector whose components have icons, enable toggles, a header menu (reset, copy, paste, remove) and an Add
  Component picker with search and categories.
- Preferences and Project Settings windows with a category tree, search, readable labels, units, and per-setting reset.
- A material editor with a large live preview (sphere, cube, plane; turntable), grouped parameters, texture slots with
  thumbnails, and assign/show-in-browser actions.
- An animation editor with an asset list, a preview viewport and transport (play, pause, loop, speed, scrub), a
  time ruler with tracks and keys, a curve view, a state-machine graph with readable nodes, and a parameters panel.
- An MCP / automation panel worth looking at: connection status, clients, permissions, activity feed, tool catalogue.
- An output log with level filters, search, timestamps, collapse, copy and clear; an undo history; notifications.

## 3. Tracks, in order

Each track ends with native captures against the "before" pictures, a record under `phase ALYX/`, and unit tests.

| Track | Deliverable |
|---|---|
| **ALYX-A Icons** | The icon family grows to cover every command, asset type, component, panel and the new windows: more Tabler (MIT) glyphs, and Somnium's own drawn in VectorCraft on the same 24-unit, 2-unit-stroke grid. Commands carry an icon. |
| **ALYX-B Menus and palette** | Icons and right-aligned shortcuts in menus; sections and submenus (Window → Panels, Float, Workspaces, Viewports); the palette's rows get icons and categories and lose the overlap. |
| **ALYX-C MCP / Authoring panel** | Status card, permissions, activity, icon actions, game tools as a tidy grid. |
| **ALYX-D Preferences** | Category sidebar, readable labels, units, more real settings (each wired), Project Settings alongside. |
| **ALYX-E Material editor** | Preview, grouped sections, texture slots with thumbnails, presets, assign and show-in-browser. |
| **ALYX-F Content browser** | Folder tree, breadcrumbs, filter chips, grid/list and size, context menu, footer. |
| **ALYX-G Animation editor** | The viewport kept as the preview; asset list; transport; timeline with ruler, tracks and keys; graph and parameters in their own panes; readable nodes. |
| **ALYX-H Viewport toolbar** | Transform mode, space, snap, view mode, camera, show flags. |
| **ALYX-M MCP for agents** | The developer, 18 Oct: "improve the overall mcp feature as well, since we will use it more and more with ai agents". Logs for agents, permissions, client setup, focused tools, resources, prompts. |
| **ALYX-I The rest** | Outliner icons and columns, component headers and Add Component, log filters, undo history, notifications, welcome screen. |

## 4. Rules

- Every new control does something real; nothing fake or placeholder ships.
- Nocturne v2 tokens only (`theme.rs`, `style.rs`); no raw colours in new widgets; dark panels.
- Icons are vector, in the 24-unit family, tinted at draw time; licences recorded beside them.
- Captures through `SOMNIUM_AUDIT_UI_STATE` and `SOMNIUM_CAPTURE_UI_PNG`, one native process at a time (the shared
  lock), never hello_engine.
- `somnium.d.luau` is regenerated by the editor when schemas change.

## 5. Progress

Records per track go in `phase ALYX/`. This table is updated as each lands.

| Track | State | Record |
|---|---|---|
| A Icons | in tree, seen | [A–D, M](<phase ALYX/ALYX-A_B_C_D_M.md>) |
| B Menus and palette | in tree, seen | same |
| C Authoring panel | in tree, seen | same |
| D Preferences | in tree, seen (rail clicks untried by hand) | same |
| M MCP for agents (added at the developer's request) | in tree; adapter tested, editor side unit-tested | same |
| E Material editor | in tree, seen; GPU live preview, texture slots and presets still open | [E](<phase ALYX/ALYX-E.md>) |
| F Content browser | in tree, seen; tree, chips, quick actions, footer, Copy Path / Duplicate / Recycle Bin | [F](<phase ALYX/ALYX-F.md>) |
| G Animation editor | in tree, seen; dock under the viewport, tabs, transport, scrub, Animation tools mode, readable timeline and nodes, frame-all | [G](<phase ALYX/ALYX-G.md>) |
| H Viewport toolbar | in tree, seen; Tools strip (move/rotate/scale/focus, lit by gizmo mode), View Mode button and menu | [H, I](<phase ALYX/ALYX-H_I.md>) |
| I The rest | in tree, seen; Add Component, component glyphs and Remove, ungrouped fields first; log/history/outliner found already done; welcome screen open | same |
