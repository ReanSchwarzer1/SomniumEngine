# Phase 26 — Metaphor

> *"A metaphor is not decoration. It is how a system teaches you to see."*
> Somnium's editor already has every authoring lever the engine ships — it just
> does not look or feel like a place you would want to spend a day. This phase
> rebuilds the UI framework and then the editor on top of it, so the chrome
> matches the renderer.

> **Codename:** Metaphor, after Atlus's *Metaphor: ReFantazio* (and the
> Persona-family discipline of treating UI as identity, not chrome). The name
> is thematic. **Zero Atlus art, fonts, motion, or layout is copied.**
> **Status:** 26-A through 26-I implemented 2026-08-13. UX polish (custom title
> bar, docked Content Drawer, click-away, named sculpt tools, wrapped Help,
> button hover/press, visible scrollbars, tile browser) landed the same day.
> Evening polish the same day: immersive play, Content Drawer 80 px tiles,
> ComboBox as a root popup overlay (Type / Tonemap), toolbar Select/Landscape/
> Foliage wiring, terrain palette selected fill, inspector search reapply,
> ScrollViewer zero-track thumb, ancestor layout invalidation on dirty measure.
> 26-H SDF/cosmic-text slipped (bundled bitmap Inter, supersampled + HiDPI
> raster). 26-J (reflection inspector) not started.
>
> **This phase is not closed.** The 26-A–I toolkit and Nocturne shell are the
> baseline. New renderer, terrain, lighting, animation, and gameplay features
> will keep needing inspector sections, menus, drawers, and Help pages. Treat
> Metaphor as living chrome, not a finished product.
> **Professional identity extension:** Phase 26-Zeta — Nocturne Atelier is
> specified in [`phase_26_Zeta.md`](phase_26_Zeta.md). The approved Claude
> Design package entered implementation on 2026-08-15: exact UI colour-space
> transfer, typed immutable Nocturne tokens, Eclipse-S/custom icon sources, and
> a behavior-preserving 36/32/32 top-shell vertical slice are in tree. The
> broader component/workspace, typography, accessibility, and sign-off phases
> remain open.
> **Next GPU track is not a re-implementation of Halcyon.** Phase VV-A–H is
> in the tree. **Start-here:**
> [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md). Remaining
> Halcyon work is evidence captures. Next *implementation* phase is Daggerfall
> ([`phase_DF.md`](phase_DF.md)) only when the user asks. Do not fold water
> reflections into a Metaphor rebuild.
> **Plan date:** 2026-08-13
> **Project:** Somnium Engine
> **Target:** Rust 1.85 docs / 1.88 effective, wgpu 29, winit 0.30
> **Depends on:** Phase 12 native UI complete; Phase 20 File Import; XV-I
> terrain palette; IV inspector water/vessel fields. **No GPU feature
> dependency.** Independent of Phase VV (Halcyon).
> **This file supersedes** the Iris-only colour-picker plan (commit `2dec6bd`).
> Colour pickers survive as **26-F Iris**, absorbed unchanged in contract.

**Information architecture** is Unreal Editor 5 (slots, density, Content
Drawer). **Paint** is Somnium's own system, **Nocturne** (§2.4) — lunar
indigo, Inter, cooler panels — so a screenshot is not a Starship clone.
Meshed with Godot's dock manager, Fyrox's retained widget/message pool
(already in tree), O3DE's editor-vs-runtime split, and Flax/Stride
content-browser layering. Interaction and IA only. **No Slate, UMG,
EditorStyle, or Starship source or art is copied** (UE EULA). Widget
implementation is original Rust on the existing Fyrox-inspired stack
(ATTRIBUTION §13.13–13.18).

---

## 0. How to use this document (handoff)

This file is the chrome contract for Metaphor. 26-A–I plus the 2026-08-13
UX polish (including immersive play and ComboBox overlay) are in the tree.
A later **UI** session should **extend** this chrome — not restart at 26-A.
A **new model** starts at
[`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) and must **not**
re-implement VV-A–H. Chrome only if a later feature needs a new inspector
field.

**Read in this order before writing code:**

1. **This file** — all of it, especially §2.4 Nocturne, §3 constraints, §4
   audit, §12.5 Help, §13 sub-phases, §14 must-not-break, §20 start checklist.
2. [`context.md`](../context.md) §8 (`somnium_ui`), roadmap row 26, §16
   messaging. **Do not rewrite §20** (Phase 14 heightmap history).
3. [`ATTRIBUTION.md`](../ATTRIBUTION.md) §1.4–1.5 (editor UX + colour picker),
   §13.13–13.18 (Fyrox UI port).
4. `crates/somnium_ui/src/theme.rs` — existing UE5-dark tokens.
5. `crates/somnium_ui/src/editor_event.rs` — the contract with `app.rs`.
6. `crates/somnium_ui/src/widgets/numeric_field.rs` — live (`ValueChanging`) vs
   commit (`ValueChanged`) pattern that Iris and every scrubbed control must
   mirror.
7. `crates/somnium_ui/src/lib.rs` — `UiManager`, `build_editor_layout`,
   inspector rebuilds. This is the file the chrome lives in today (~2.8k
   lines). The refactor migrates it; it is not deleted on day one.

**26-A–I are done.** Do not rebuild the toolkit. Remaining Metaphor work is
living chrome: 26-J (only if requested), 26-H SDF/shaping, 26-D2 drag-drop,
async thumbs, and **every new authoring surface** later phases need. Help
pages in `docs/editor/` grow with those surfaces.

**Authorized work:** UI and UX only. Renderer, terrain, water optics, lighting,
physics, and ECS layout stay frozen except **inspector bindings** to fields
that already exist. `WaterComponent::great_lakes` stays frozen. Do not
reintroduce per-pixel terrain sample-count LOD.

This file began as the Metaphor plan. Keep using it as the contract (`§3`,
`§14`) when adding UI for later features. Do not treat 26-I as “UI is done.”

---

## 1. Executive decision

`somnium_ui` already has a retained widget tree, two-pass measure/arrange,
generational `Handle` pool, bubbling `UiMessage`, focus, capture, and a
screen-space `UiPass`. What it does **not** have is a product:

- The editor is a 40 | \* | 280 grid with hand-positioned File/Create popups
  at `(52, 28)` and `(148, 28)` that miss the buttons after a resize.
- Checkboxes are `[ ]` / `[x]` buttons. Foliage type is a **cycler** (17G).
  Tonemapper is a cycler. Edit and View menus are inert labels.
- There is **no content browser**. Phase 20 Import was explicitly "placeholder
  for a real content browser (Phase 21)". `hello_engine` still posts
  `update_content_browser` JSON of **top-level `assets/` only** into
  `UiManager::send_message`, which is a **no-op stub** left from the wry
  removal (12C).
- `DrawingContext::push_textured_rect` exists; **no icon ever uses it**.
- Font is a 512² fontdue bitmap atlas loaded from `C:\Windows\Fonts\segoeui.ttf`
  (Arial / Liberation Sans fallback). No SDF, no shaping, no shipped font.
- A game built on Somnium cannot have a UI. The toolkit only draws editor
  chrome.

Metaphor does three things, in this order, without a big-bang blank editor:

1. **Harden the framework** so the missing widgets exist (splitter, tab,
   checkbox, tree, combo, image/icon, tooltip, context menu, 9-slice, status
   bar, anchored popup).
2. **Rebuild the editor on those widgets** (dogfood): Unreal-like shell,
   Content Drawer, iconography, inspector/outliner UX, colour picker.
3. **Open a runtime canvas API** so a *game* can draw UI with the same
   toolkit the editor uses.

Strategy: **strangler fig**. New widgets land first; chrome migrates panel by
panel; `EditorEvent` stays the seam with `app.rs`. Existing functionality is
re-hosted, not rewritten from a blank window.

---

## 2. Codename and visual brief

### 2.1 Why Metaphor

Atlus (Persona, *Metaphor: ReFantazio*) treats menus as a world: hierarchy,
iconography, restrained motion, every action having a face. Somnium studies
that **discipline** — clarity, type as structure, no anonymous chrome — and
stops there. **Do not** reproduce Persona/Metaphor colour scripts, all-out
attack layouts, menu 3D dioramas, or any Atlus asset.

### 2.2 Controlling *layout*: Unreal Editor 5

The **slot map** should read as a UE5 Level Editor at a glance, so anyone
who has used a DCC is not lost. The **paint** is Nocturne (§2.4), not
Starship.

| Slot | Unreal name | Somnium target |
|---|---|---|
| 1 | Menu Bar | File / Edit / Create / View / Window / Help — all live. **Help ?** on the right (F1) |
| 2 | Main Toolbar | Save, modes (Select / Landscape / Foliage), Create, Play/Pause/Stop — **with icons** |
| 3 | Viewport Toolbar | TRS, camera speed, view/shading, profiler |
| 4 | Level Viewport | Existing transparent passthrough region |
| 5 | Outliner | Hierarchical tree, type icons, search, context menu |
| 6 | Details | Category headers, property rows, real checkboxes/combos, colour swatches |
| 7 | Content Drawer | Temporary Content Browser; Ctrl+Space; dismiss on focus loss; Dock in Layout |
| 8 | Bottom Toolbar | Content Drawer button, Output Log drawer, status (FPS, selection, unsaved) |

Mesh, not copy, from other engines where Unreal is the wrong teacher:

- **Godot** — `EditorDockManager`: left/right/bottom slots, tabbed docks,
  layout save. Somnium v1: resizable splitters + tab strips, not a full
  docking graph. Inter as editor face (Godot 4.6, also Blender / GNOME).
- **Fyrox** — already our widget/message/pool lineage; editor asset preview
  cache is the thumbnail pattern.
- **Flax** — Content tree vs Content view vs preview cache (three-layer
  drawer).
- **O3DE** — Qt editor chrome is a *consumer* of widgets, separate from
  LyShine in-game canvases. Keep **one toolkit, two shells** (EditorShell vs
  GameCanvas).
- **Stride** — async thumbnail compiler (do not stall the UI thread on glTF).
- **Blender** — status-bar keymap hints; Help is a window, not a website.
- **Unity** — Scene overlays for “what can I do here”; Somnium uses F1 Help
  + viewport tooltips instead of a second overlay stack in v1.
- **VS Code / Linear** — command palette; muted selection fill + accent
  hairline rather than a solid primary slab.

### 2.3 What "like Unreal" does not mean

- Shipping Epic's EditorStyle / Starship SVGs or PNGs.
- Translating Slate C++ or UMG.
- Using Epic's `#0070E0` highlight or bundled Roboto as our face.
- Dear ImGui / egui as the editor (would throw away Phase 12).
- Qt, WPF, or a WebView regression.
- Atlus / Persona colour scripts, gold flourishes, or 3D menu dioramas.

### 2.4 Visual identity — Nocturne (locked)

**Name:** Nocturne. Somnium means dream; the chrome is a cool night interior
with one moonlight accent. Professional DCC density. Easy to scan. Own
thing.

**Two-second test.** A screenshot must be identifiable as Somnium, not
Unreal, Godot, or Blender:

| Engine | What you notice first | Somnium instead |
|---|---|---|
| Unreal Starship | `#0070E0` slabs, Roboto, 4 px rounds, `#C0C0C0` body | Lunar indigo hairline selection, Inter, 2 px rounds |
| Godot | Teal accent, warmer greys, looser density | Cooler panels, tighter 22 px rows |
| Blender | Orange accent, unique header | Indigo, Unreal-like slot map |
| VS Code | `#007ACC` + `#1E1E1E` | Cool blue-black `#14161C`, indigo not Microsoft blue |

**Surfaces** (cool blue-black, not today's neutral `#1E1E1E`):

| Token | Hex | Role |
|---|---|---|
| `BG_VOID` | `#14161C` | Window / menu bar / deepest well |
| `BG_CONTENT` | `#181A20` | Viewport surround, drawer well |
| `BG_PANEL` | `#1C1E26` | Outliner, Details, log |
| `BG_HEADER` | `#252830` | Toolbars, category headers |
| `BG_RAISED` | `#2A2D38` | Buttons, cards, combo chrome |
| `BG_INPUT` | `#12141A` | Numeric fields, search (recessed) |
| `BG_HOVER` | `#343848` | Hover wash |
| `BORDER_DARK` | `#0E1014` | Panel seams |
| `BORDER_MEDIUM` | `#3A3E4A` | Default stroke |
| `BORDER_FOCUS` | lunar indigo | Focus ring, 1 px |

Today's `theme.rs` (`BG_DARK 1E1E1E`, `ACCENT_BLUE 1A75D2`, white labels)
is a Phase 12 placeholder. **26-A replaces it with these tokens.** Keep
the old names as aliases for one sub-phase if call sites are many, then
delete the aliases.

**Accent — Lunar Indigo** (original; not Epic `#0070E0` / `#1A75D2`):

| Token | Hex | Role |
|---|---|---|
| `ACCENT` | `#7A86FF` | Focus, active tab, primary button, links |
| `ACCENT_HOVER` | `#949CFF` | Hovered accent |
| `ACCENT_PRESSED` | `#5C68E0` | Pressed |
| `ACCENT_DIM` | `#7A86FF` @ 22% | Selection fill behind a row |
| `ACCENT_BAR` | 2 px left strip | Selected outliner / asset row (hairline, not a slab) |

Folder tint in the Drawer: warm sand `#C4A574` (readable on cool panels).

**Type:**

| Token | Hex | Contrast on `BG_PANEL` |
|---|---|---|
| `TEXT_PRIMARY` | `#D8DCE8` | ~10:1 — labels, not `#FFFFFF` (glare) |
| `TEXT_SECONDARY` | `#9AA3B5` | ~6.5:1 — hints, breadcrumbs |
| `TEXT_DISABLED` | `#5C6478` | Inactive only (WCAG exempts disabled) |
| `TEXT_LINK` | `ACCENT` | Help hyperlinks |

- Face: **Inter** (SIL OFL 1.1), bundled. Same family Godot 4.6, Blender,
  and GNOME picked for UI. **Not** Roboto (Unreal's face), **not** Segoe
  (today's `C:\Windows\Fonts` hack).
- OpenType: `tnum` (inspector columns line up), `ss04` (serifed `I` / tailed
  `l`). Godot PR `#111140` is the citation for those features; we set them
  in our rasterizer, we do not copy Godot.
- Sizes: 11 px captions / 12 px chrome / 13 px body / 15 px panel titles.
- Line-height 1.35 in chrome, 1.5 in Help long-form.

**Geometry & motion:**

| Rule | Value | Why |
|---|---|---|
| Radius | 2 px chrome, 3 px popups, 0 px splitters | Tighter than Starship's 4 px default |
| Grid | 4 px | All padding is 4 / 8 / 12 |
| Menu bar | 28 px | Match today's height |
| Toolbar row | 24 px | Fitts: ≥24 px hit target |
| Inspector row | 22 px | DCC density |
| Icon | 16 px in rows, 20 px in toolbars | Lucide optical size |
| Tooltip delay | 400 ms | Apple HIG / Fluent ballpark; never instant |
| Motion | 100 ms ease-out | No bounce, no overshoot, no Atlus flourish |
| Focus | 1 px `ACCENT` ring | No 8 px glow |

**Engine mark:** original crescent enclosing a small geometric S, filled
`ACCENT`, 20 px in the menu bar. Not a copied moon, not Epic's sphere.

**Professional / easy (Nielsen mapped to chrome):**

| Heuristic | Somnium rule |
|---|---|
| Visibility of system status | Status bar always shows FPS, selection name, dirty save dot, active tool |
| Match the real world | Unreal slot names (Details, Content Drawer, Outliner) so DCC users transfer |
| User control | Esc / click-away closes the top popup, Help, palette, colour, unsaved; docked Content Drawer stays; Ctrl+Z for transforms |
| Consistency | One PropertyRow, one Combo, one CheckBox; no cyclers |
| Error prevention | Unsaved modal on New Scene; disabled Open until LoadScene is real |
| Recognition not recall | Icon + **label** on chrome (Play/Sculpt are not tooltip-only); hover/press/selected fills; F1 Help |
| Flexibility | Menu path and shortcut for every command |
| Aesthetic / minimal | One accent; no decorative gradients on panels |
| Recover | Undo; colour Cancel restores |
| Help | In-editor Help overlay (§12.5), not a browser tab |

**WCAG 2.1 AA** for enabled text and status colours. Disabled chrome may
sit near 3:1 (same exemption Starship documents). Do not ship
`TEXT_PRIMARY` as `#FFFFFF` on `#1E1E1E` just because it “pops”.

**starship-css / `EStyleColor`:** study the *token graph* (surface ladder,
foreground vs primary vs select, folder accent separate from CTA). **Do
not** copy their hex values. The tables above are the source of truth.

**Status colours** (own hex, not Epic's neon):

| Token | Hex |
|---|---|
| `STATUS_OK` | `#5DCE9A` |
| `STATUS_WARN` | `#E6B04A` |
| `STATUS_ERROR` | `#E05A5A` |

---

## 3. Hard constraints

1. **UI/UX only.** No terrain shader retune, no water `wave_speed` / optics
   change, no lighting, no physics, no ECS archetype redesign. Inspector may
   *bind* to existing component fields (including colours Iris exposes).
2. **Do not break existing editor functionality.** Re-host it. See §14 for
   the inventory. If a sub-phase cannot preserve a behaviour, it is not done.
3. **Improve what we already have first**, then add chrome. A prettier
   inspector that drops terrain paint is a failure.
4. **Content Drawer** shows `hello_engine` project files by default; a
   **Show Engine Content** toggle reveals engine files (UE `bShowEngineContent`).
5. **Every type has an icon** — engine, project, folders, scripts, meshes,
   textures, materials, scenes, lights, terrain, foliage, audio, shaders,
   unknown.
6. **Visual identity is Nocturne** (§2.4). Unreal-like slots; Somnium paint.
   Do not “fix” the accent back to Epic blue.
7. **Colour picker is in this phase** (26-F), absorbing the old Iris plan.
8. **Dogfood.** The editor is rebuilt on the new widgets. A framework with
   no editor consumer will rot the way 17G's cycler did.
9. **Cargo fmt / check / tests green** after every sub-phase.
10. **Evidence** under `dev records/phase 26/` — do not invent screenshots.
11. **ATTRIBUTION** updated when a reference is first used in code, not in
    advance of working widgets.

---

## 4. Repository audit (2026-08-13)

### 4.1 Framework (`crates/somnium_ui`)

| Piece | State |
|---|---|
| `UserInterface` | Two-pass measure/arrange, hit-test, message queue, viewport passthrough |
| Dirty flags | `measure_valid` / `arrange_valid`; `invalidate_ancestors()` exists (12 layout bug) |
| Pool | Fyrox-style `Handle<T>` + generation |
| Font | fontdue 0.7, 512×512 RGBA8 shelf atlas — **not SDF**, no kerning/shaping |
| Theme | Static UE5-dark constants; not a style resource |
| Draw | Rects, borders, text; `push_textured_rect` unused for icons |
| `UiPass` | Screen overlay, alpha blend, `LoadOp::Load`; bind groups: white 1×1 or font atlas only |
| Cargo.toml description | Still says **"egui-based UI system"** — stale since 12C |

**Widgets that exist:** Border, Button, Canvas (unused in editor), Grid,
NumericField, ScrollViewer, Slider, StackPanel, Text, TextBox (unused in
editor), Menu, Popup. Menu/Popup are **not** in `widgets/mod.rs` `pub use`.

**Explicitly stripped from Fyrox `widget.rs` (comment in source):** Matrix3
transforms, drag-drop, tooltips, context menus, Material, Reflect/Visit.

**Missing vs a real editor:** Checkbox, TreeView, Tab, Splitter, Image/Icon,
Tooltip, ComboBox/Dropdown, SearchBox, Breadcrumb, Thumbnail grid, Context
menu, Dock tabs, Status bar, Color picker, 9-slice, world-space canvas,
resizable panels, modal dialog, command palette.

### 4.2 Editor chrome today

`build_editor_layout` (`lib.rs` ~1491):

```
outer_grid  4 rows: 28px menu | 26px viewport toolbar | * main | 160px log
  menu: "Somnium Engine" + File (live) + Edit (inert) + Create (live) + View (inert)
        FPS text on the right (not actually updated — send_message is a stub)
  viewport toolbar: Camera Speed slider + Play/Pause/Stop text buttons
  main: 40px tool strip | transparent viewport | 280px right (outliner 200px + inspector)
  log: header + ScrollViewer, cap 200 lines then **silent stop** (not a ring buffer)
```

`context.md` §8 still diagrams **192 px** log and 3 outer rows — the code is
**160 px** and 4 rows. Fix the living diagram when 26-C lands.

Popups: File at `(52, 28)`, Create at `(148, 28)`, parented to root to escape
clip (Fyrox pattern, ATTRIBUTION popup note). **Breaks on resize.**

Outliner: flat `Button` list, rebuilt on selection/world change. No hierarchy,
no type icons, no visibility eye, no folders.

Inspector: ~8 sections, ~120 `InspectorField`s. Toggles are checkbox-shaped
Buttons. Foliage kind + tonemapper are cyclers. Terrain 32-layer palette is
a button grid (XV-I). Water leftover colours/dirs/underwater **not exposed**
(Iris). Lights: Col R/G/B floats + Kelvin.

### 4.3 `EditorEvent` surface (must keep working)

From `editor_event.rs`, drained in `somnium_core/src/app.rs`:

| Event | Notes |
|---|---|
| `SelectEntity` | Outliner + viewport pick |
| `CreateEntity(CreateKind)` | Cube/Sphere/Plane/Cylinder, 3 lights, Particle, Terrain, VoxelTerrain |
| `DeleteSelected` / `DuplicateSelected` | Delete key / Ctrl+D |
| `Undo` / `Redo` | Transform + light today; **post/water/vessel have no undo** |
| `PlaySimulation` / `PauseSimulation` / `StopSimulation` | Transport |
| `SaveScene` / `NewScene` | File menu + Ctrl+S / Ctrl+N |
| `LoadScene(path)` | **Stub** — logs "not yet fully implemented" |
| `SetInspectorValue { field, value, live }` | Live scrub vs commit |
| `SetTerrainTool` / `SetTerrainPaintLayer` / `ToggleTerrainPaint` / `ToggleTerrainHex` | Tools 0–5, 32 layers |
| `ToggleFoliage` / `ToggleFoliagePaint` / `ToggleFoliageErase` / `ToggleFoliageSingle` / `SelectFoliageKind` | 17F |
| `TogglePostFx` / `CycleTonemapper` | 15A1 + 24\* |
| `SetCameraSpeed` | Viewport slider |
| `ToggleProfiler` | Also gates GPU timestamps |
| `ImportModel` | `rfd` glTF/GLB |
| `ToggleShadingMode` | F5 |

Keyboard (must survive): Ctrl+Z/Y, Delete, WASD+RMB fly, W/E/R gizmos when
not flying, F5 shading, F6 terrain edit, F8 foliage enable, Shift speed,
brush keys while terrain edit is on, viewport passthrough when the cursor is
over the transparent region.

### 4.4 Content that the Drawer must show

`hello_engine` `list_assets_dir()` reads **only the top level of `assets/`**
and posts JSON into a dead IPC stub. Recursive inventory the Drawer will
actually use:

| Path | Kind |
|---|---|
| `assets/models/gislinge_viking_boat/` | glTF + textures + README |
| `assets/foliage/grass_medium_01/` | glTF foliage |
| `assets/foliage/grass_bermuda_01/` | glTF foliage |
| `assets/foliage/fir_sapling/` | glTF foliage |
| `assets/foliage/island_tree_02/` | glTF foliage |
| `assets/terrain/` | PNG albedo/surface, `materials.json`, Great Lakes sidecar, pack report |
| `assets/terrain/bc7/` | **gitignored** local BC7 packs — hide by default or mark derived |
| `assets/ocean_pbr/` | sea spray + README |
| `assets/LICENSE.md` | license text |
| `assets/test_scene.glb` | if present on disk |

There is no `engine/content/` tree today. Metaphor **creates** a virtual
`/Engine/` root (see §9). Do not dump `crates/` source into the browser.

### 4.5 Stale living-doc mismatches to fix when chrome lands

- `context.md` §8: 192 px log, 3-row outer grid, wry leftover in the crate
  tree diagram (`somnium_ui/` still described as wry in §2 in places).
- `somnium_ui/Cargo.toml`: "egui-based".
- ATTRIBUTION §1.4 still mentions `#right_panel` HTML ids from the wry era.

---

## 5. Research — every engine in `example_repo`

Workspace `SomniumEngine/example_repo` currently holds **JoltPhysics** (and an
empty Esoterica stub). The full ~6 GB study mirror used for this plan is
`C:\Users\adhir\Downloads\GE\example_repo`. Paths below are against that
mirror unless noted. **Pattern and IA only; no source translation.**

### 5.1 Unreal Engine 5 (controlling)

The Content Drawer is **not** a class named `ContentDrawer`. It is:

- `UStatusBarSubsystem` + `SWidgetDrawer` hosting a Content Browser instance.
- **Ctrl+Space** toggles it.
- **Dismiss on focus loss** when it is a drawer.
- **Dock in Layout** pins a full `SContentBrowser` into the tab layout.
- Same widget factory, two hosts.

Toggles live in `FContentBrowserInstanceConfig`:

- `bShowEngineContent`
- `bShowPluginContent`
- `bShowCppFolders`

Menus: `ContentBrowserMenuUtils.cpp`. Pieces: `SContentBrowser`, `SAssetView`,
`SPathView`, `SFilterList`, `FThumbnailManager` + per-type
`*ThumbnailRenderer`.

Colour: `SColorBlock`, `SColorPicker`, `FColorPickerArgs` (Iris / ATTRIBUTION
§1.5). Docking: `FTabManager` / `SDockTab`. Style: Starship / `EditorStyle`
— **do not copy art**.

Also on the same status bar: Output Log drawer; command console `~`.
UE 5.4 had a **Ctrl+Space does not close** bug — Somnium's toggle must be
a true toggle (open ↔ close), not open-only.

Official IA (Epic docs, UE 5.8 "Unreal Editor Interface"): Menu Bar, Main
Toolbar (Save, Modes, Content shortcuts, Play, Platforms), Viewport Toolbar
(TRS/snap, camera, view mode, scalability), Viewport, Outliner, Details,
Content Drawer, Bottom Toolbar (drawer, output log, DDC, source control).

**Somnium v1 cuts:** no Platforms menu, no DDC, no Live Coding, no Fracture/
Modeling/Animation modes, no Blueprint/Cinematics buttons. Modes that map
to existing work: **Select**, **Landscape** (terrain tools), **Foliage**.

### 5.2 Fyrox (already in Somnium)

`fyrox-ui`: retained tree, messages, pool, measure/arrange, popup-at-root.
`fyrox-ui` docking + `editor/src/asset` preview cache are the next citations
when 26-A/26-D land. Do not pull Fyrox's full editor; take widget patterns.

### 5.3 Flax

`Engine/UI/UICanvas` + `GUI/` — screen and world canvases (this is the
`context.md` §26.2 reference). Content: tree vs view vs `PreviewsCache`.
Editor is C#; we take layering, not the runtime.

### 5.4 Wicked Engine

`wiGUI` / `wiFont` — debug overlays and a font atlas, **not** editor chrome.
Useful for world-space 3D labels later; not a shell to copy.

### 5.5 Stride

WPF-like measure/arrange (`Stride.UI`). **Async thumbnail compiler** is the
lesson: never decode a 4K PNG or parse glTF on the UI thread for a 128² tile.

### 5.6 O3DE

Qt editor chrome **separate** from LyShine in-game UI. Prefab/LandscapeCanvas
are later phases (34/35). For Metaphor: keep EditorShell vs GameCanvas as two
consumers of `somnium_ui`. LyShine canvas/anchors inform 26-G.

### 5.7 Godot

`EditorDockManager`, `EditorFileSystem` (async scan + signals), FileSystem
dock, Inspector built from property info. Somnium v1 does **not** become
reflection-driven (that was 26.2c / NeoAxis — **follow-up**, not Metaphor v1).
Godot's FileSystem dock is the closest FOSS analog to a content browser.

### 5.8 Bevy

Taffy flexbox + ECS UI. Runtime-first, no editor. Optional later: Taffy as a
Flex container **beside** Grid, not a replacement this phase.

### 5.9 rbfx / Urho

Native UI + ImGui editor + RmlUI. **Do not triple-stack.** Deepen the Fyrox
lineage. RmlUI remains a candidate for *game* HTML/CSS UI in a later phase
if 26-G's retained canvas is not enough; it is not the editor.

### 5.10 Overload, Spartan, bgfx examples, Ogre

ImGui debug. Confirms ImGui is a profiler/debug overlay, not an editor
framework. Somnium already has a profiler overlay (Phase 29); leave it.

### 5.11 Jolt TestFramework

Tiny retained overlay + UI layer stack for modals. Modal layer (unsaved
dialog) is the takeaway.

### 5.12 Unity uGUI (if present in the mirror)

`RectTransform`, Canvas (Screen Space Overlay / Camera / World Space),
EventSystem. Directly useful for **26-G game canvases**, not the editor
shell.

### 5.13 The Forge, CDLOD, DXC, SwiftShader, Esoterica stub

No editor UI to study.

### 5.14 NeoAxis / Falco (from `context.md` §26.2b–c)

Reflection-driven inspector: components declare properties, editor generates
rows. **Correct long-term fix** for "every component needs hand-written
panel code" (why 17G shipped a cycler). Metaphor v1 still **hand-builds**
the inspector on better widgets (PropertyRow, Checkbox, Combo, ColorSwatch)
so we do not block the chrome rewrite on a reflection system. Track as
**post-Metaphor** (or a 26-J if the phase overruns and the user asks).

---

## 6. Research — papers, crates, icon packs

### 6.1 Layout

- Keep **two-pass Measure/Arrange** (WPF, Avalonia, Stride, Fyrox). It already
  works; Grid Strict/Auto/Stretch is enough for an Unreal-like shell.
- **Taffy / Yoga flex** — optional later container; do not replace Grid in
  26-A.
- **Cassowary** constraint layout — out of scope.

### 6.2 Text

- Valve, *Improved Alpha-Tested Magnification for Vector Textures and Special
  Effects* (SIGGRAPH 2007) — SDF.
- Chlumsky, *Shape Decomposition for Multi-channel Distance Fields* — MSDF.
- Rust: `fontdue` (current), `cosmic-text` + `rustybuzz` (shaping, wrapping,
  bidi), `msdfgen` bindings if 26-H happens.
- **Honest slip:** 26-H (SDF + shaping) may not close in the same calendar
  pass as the Drawer. Ship a **bundled OFL font** and a larger atlas in 26-A
  so Windows-font dependence dies even if SDF waits.

### 6.3 Game UI toolkits (runtime, not editor)

- RmlUi (HTML/CSS), Coherent Gameface, Noesis, Dear ImGui, egui, Slint,
  Iced, Floem, GPUI.
- Decision: **one retained toolkit**. Editor dogfoods it. Games get
  screen-space (then world-space) canvases on the same tree. Do not add
  egui as a second editor.

### 6.4 Icon packs (must be commercial-OK)

| Pack | License | Use |
|---|---|---|
| **Lucide** | ISC | **Primary.** Rasterize 16/20/32 into an atlas. |
| Phosphor | MIT | Fill gaps (duotone weights) if Lucide lacks a type. |
| Tabler | MIT | Fallback if coverage is thin. |
| Unreal EditorStyle / Starship | UE EULA | **Forbidden.** |
| Atlus / Persona / Metaphor assets | proprietary | **Forbidden.** |
| Font Awesome Pro | proprietary | Do not. |

Engine mark: **original** Somnium glyph (simple geometric S / moon — design
in 26-C, not a copied logo). Cite the chosen pack in ATTRIBUTION when the
atlas lands; keep LICENSE files next to the SVG sources.

### 6.5 Colour (Iris, unchanged)

See §11. UE `SColorBlock` / `SColorPicker` / `FColorPickerArgs`. Linear
storage, sRGB display. Kelvin sibling. Water abs/scatter = normalised swatch
+ magnitude.

---

## 7. Target information architecture

```
┌─ TitleBar (undecorated window) ── Somnium Engine ── fps ─ _ □ × ─┐
│  [S] File  Edit  Create  View  Window  Help                         │
├─ MainToolbar ────────────────────────────────────────────────────────┤
│  Save  |  Select Landscape Foliage  |  Create ▾  |  ▶  ❚❚  ■  |     │
├─ ViewportToolbar ────────────────────────────────────────────────────┤
│  T R S  |  snap  |  Camera speed ───── 5.0 m/s  |  Lit ▾  |  Prof   │
├─ L Sculpt ┬─ Viewport (passthrough) ─────────────┬─ Outliner ────────┤
│  Raise    │                                       │  tree + search    │
│  Lower …  │                                       ├─ Details ─────────┤
│           │                                       │  categories       │
├───────────┴───────────────────────────────────────┴───────────────────┤
│  Content Drawer tiles (docked; Output Log swaps this row)             │
├───────────────────────────────────────────────────────────────────────┤
│  [Content Drawer]  [Output Log]     status: entity · tool · 60 fps    │
└───────────────────────────────────────────────────────────────────────┘
```

Shipped default: Content Drawer **docked** in the bottom row (not a popup).
Click-away does **not** dismiss it. Output Log occupies the same slot when
its status-bar button is pressed.

Column widths: **resizable splitters** (left tools, right details). Bottom
log/drawer: **resizable**. Default roughly 40 | \* | 320, bottom 220 when
docked — not magic constants painted into Grid rows forever.

---

## 8. Framework architecture (what 26-A/B add)

Stay on the Fyrox-inspired `Control` trait. New widgets are new files under
`crates/somnium_ui/src/widgets/`, exported from `mod.rs`.

| Widget | Role |
|---|---|
| `Image` | Textured quad; icon atlas UVs or thumbnail texture |
| `Icon` | Named glyph from the atlas (`IconId`) |
| `Tooltip` | Delayed hover label; host on root canvas |
| `CheckBox` | Replaces `[x]` buttons |
| `ComboBox` | Replaces cyclers (foliage kind, tonemapper, view mode). Header stays one row; the list is a root-parented `Popup` + `ComboDropdown` so inspector siblings cannot paint over it. |
| `TreeView` | Outliner + content path tree |
| `TabControl` | Outliner/Details tabs; docked Content vs Log |
| `Splitter` | Drag to resize columns/rows |
| `StatusBar` + `StatusBarButton` | Bottom chrome; hosts drawers |
| `SearchBox` | Filter outliner / drawer / details |
| `Breadcrumb` | Drawer path |
| `ContextMenu` | Right-click outliner, assets, viewport (editor items only) |
| `NineSlice` | Draw helper; used by buttons/panels |
| `ColorSwatch` / `ColorPickerPopup` | 26-F |
| `PropertyRow` | Label + control + optional reset; Details consistency |
| `AssetTile` / `AssetListRow` | Drawer view modes |
| `ScrollViewer` | Already exists; add **horizontal** if missing |

**Popup anchoring:** Popup opens at `anchor_widget.screen_bounds().bottom_left`
(+ menu width), not hardcoded `(52, 28)`. File/Create/combo/colour all share
this.

**UiPass textures:** Today white 1×1 + font atlas. 26-A adds an **icon atlas**
bind group (id 1) and a small **thumbnail cache** (dynamic textures, ids 2+).
Do not explode bind-group switches; atlas first, then a handful of thumbnail
pages.

**Dirty tracking:** Keep measure/arrange flags. Add widget-level
`visual_dirty` so a Text change does not relayout the whole shell. Nice-to-have
in 26-A; required if profiler shows layout dominating.

**Input:** Focus + capture already exist. Add: tab order, Escape closes the
top popup, Ctrl+Space is owned by the Drawer (do not steal viewport Space if
we never used Space). RMB on viewport still flies; RMB on a panel is context
menu — hit-test must distinguish passthrough.

---

## 9. Content Drawer design

### 9.1 Two hosts, one widget

`ContentBrowser` is a widget. **Shipped default (2026-08-13):** the widget
is **docked** in outer-grid row 5. Ctrl+Space and the status-bar button
show/hide that row. Click-away does not dismiss it. Output Log swaps into
the same slot.

Original two-host design (still valid if we add an undocked popup later):

1. **Drawer** — rises from the status-bar "Content Drawer" button (and
   Ctrl+Space). Dismisses when it loses focus unless pinned.
2. **Docked** — "Dock in Layout" (or View → Content Browser) puts the same
   widget in the bottom tab strip beside Output Log. Drawer button still
   opens a *second* ephemeral instance (UE behaviour).

### 9.2 Roots

| Root | Default visible | On disk / virtual |
|---|---|---|
| **Project** (`/Game/`) | Yes | Recursive `assets/` of the running app (hello_engine CWD) |
| **Engine** (`/Engine/`) | **No** — Show Engine Content | Virtual + small on-disk `engine/content/` |

**Engine content is not `crates/`.** It is:

- **Virtual primitives:** Cube, Sphere, Plane, Cylinder (Create kinds as
  spawnable assets).
- **Editor chrome:** icon atlas, bundled font, engine mark.
- **Built-in materials / default PBR** if we already have engine-owned
  defaults (not the project's terrain PNGs).
- **View-only shaders** (optional, later): listing `.wgsl` under
  `crates/somnium_renderer/src/shaders` is tempting and wrong for v1 — those
  are source, not cooked content. Skip unless the user asks.

Project terrain/foliage/boat stay under `/Game/` even though they shipped
with the repo: they are the **sample project's** files, like UE's First
Person template Content.

`assets/terrain/bc7/` is a **derived** folder (gitignored). Default: hidden,
or shown greyed with a "generated" badge. Never treat BC7 packs as
authoritative sources (PNG/materials.json are).

### 9.3 Show Engine Content

A checkbox/toggle in the Drawer settings menu (funnel / eye menu, Unreal
pattern) and a matching View menu item. Off by default. When on, `/Engine/`
appears in the path tree. This is the UE `bShowEngineContent` analog.

No plugin system → **no Show Plugin Content** in v1.

### 9.4 View modes and filters

- Tiles (default) and List.
- Search filters name + extension.
- Type filters: Folder, Mesh (gltf/glb), Texture (png/exr/hdr), Material
  (json that is a material), Scene, Audio, Shader, Font, Other.
- Breadcrumb of the current path.
- Folder tree on the left (Flax/Godot/UE Path View).

### 9.5 Thumbnails

| Type | v1 | Later |
|---|---|---|
| Folder | Folder icon | — |
| PNG/JPEG | Async decode to 128² | Mip from GPU |
| glTF/GLB | Type icon | Offscreen preview (Stride-style compiler) |
| JSON material | Material icon | Sphere preview |
| Unknown | File icon | — |

Never block layout on IO. A background scanner (Godot `EditorFileSystem`
pattern) sends "listing ready" / "thumb ready" messages.

### 9.6 Interactions

- Double-click folder → enter; double-click mesh → **select in outliner if
  instanced**, or Import/spawn (decide in 26-D: v1 spawn at origin like
  File → Import, or select-only).
- Drag-drop into viewport → spawn at hit (late; 26-D2). If 26-A has not
  restored drag-drop on `Widget`, keep double-click spawn and do not fake
  drag.
- Right-click: Show in Explorer, Copy Path, Import (meshes), Dock in Layout.
- File → Import Model remains; it can also focus the imported asset in the
  Drawer.

### 9.7 hello_engine wiring

Delete the wry-era `send_message("update_content_browser", …)` call. The
browser scans `assets/` itself (or `somnium_asset` grows a listing helper).
`UiManager::send_message` stub can die once FPS and log no longer pretend
to be IPC.

---

## 10. Icon system

### 10.1 Atlas

One GPU texture, packed at build or first run from SVG → PNG at 16 / 20 / 32
(HiDPI: pick 32 and downscale, or 20@1x / 32@1.5 / 40@2). `IconId` enum
covers chrome + asset types. `Image` widget samples UVs.

### 10.2 Required `IconId` set (minimum)

**Chrome:** Engine mark, File, Edit, View, Window, Help, HelpCircle (?), Save, Undo, Redo,
Play, Pause, Stop, Translate, Rotate, Scale, Select mode, Landscape mode,
Foliage mode, Search, Filter, Settings, Dock, Close, Folder, FolderOpen,
Chevron, Visibility (eye), Add, Delete, Duplicate, Import, Profiler,
OutputLog, ContentDrawer.

**Create / entity types:** Cube, Sphere, Plane, Cylinder, DirectionalLight,
PointLight, SpotLight, Particle, Terrain, VoxelTerrain, EmptyEntity, Camera.

**Assets:** Mesh, Texture, Material, Scene, Audio, Shader, Font, Script
(`.rs` if ever shown), JSON, License, Unknown, Derived/BC7.

**Inspector sections:** Transform, Light, PostFx, Terrain, Water, Vessel,
Foliage.

**Status:** Ok, Warn, Error.

Every Create menu row, outliner row, asset tile, toolbar button, and
inspector category header shows an icon. Text-only chrome is a bug in 26-C+.

### 10.3 Attribution

Vendor SVGs stay in `assets/engine/icons/` (or `engine/content/icons/`) with
LICENSE. Rasterized atlas may be gitignored if generated; **sources committed**.
Never commit Unreal `Editor/Slate/Icons`.

---

## 11. Colour picker (Iris, absorbed as 26-F)

Contract copied from the superseded Iris plan so it is not lost.

### 11.1 Why

Lights still edit tint as `Col R/G/B`. Water deep/shallow/edge, absorption,
scattering, particle colours, and material base colour are **not in the
inspector**. Colour is a spatial quantity.

### 11.2 Widget

```
ColorProperty row
├── Label
├── ColorSwatch (click opens picker)
└── optional Alpha

ColorPickerPopup (one global instance, root canvas)
├── SV square
├── Hue strip
├── R G B | H S V | Hex
├── optional A
├── Recent colours (8)
└── Cancel  (OK = click-outside commit; Unreal bOnlyRefreshOnOk = false)
```

| Message | Semantics |
|---|---|
| `ColorChanging` | Live preview; no undo yet |
| `ColorChanged` | Commit; one undo step from colour at open |
| `ColorCancelled` | Restore colour at open |

Linear RGB(A) storage; swatch/spectrum use approximate sRGB (`pow(x, 1/2.2)`).
Hex edits sRGB bytes. Absorption/scattering: **normalised swatch + magnitude**
so `(0.22, 0.07, 0.03)` reads as a tint, not near-black.

Kelvin > 0 on lights **locks** the swatch to the derived tint (Unreal
temperature override). Intensity stays a separate float. HDR > 1.0 in the
spectrum is out of scope.

### 11.3 Adoption (still staged inside 26-F)

| Stage | Consumer |
|---|---|
| F0 | Widget + unit tests (linear ↔ sRGB ↔ hex; Cancel restores) |
| F1 | Lights (replace Col R/G/B) |
| F2 | Water deep/shallow/edge, abs/scatter, underwater toggle, wave dir X/Z |
| F3 | Particle start/end |
| F4 | Material base colour on selected mesh |

Post Temp/Tint/Lift/Gamma/Gain stay **numeric** (grading axes, not RGB
colours). Fog colour stays out until fog is an authored RGB. Eyedropper from
the viewport is later. Terrain layer tint is **25J's** problem; Iris owns
the widget, 25J owns terrain-specific fields.

Water **defaults** stay frozen (`great_lakes`). 26-F exposes authoring; it
does not retune the shipping body.

---

## 12. Inspector and Outliner UX (26-E)

Not a reflection system. Hand-built panels on `PropertyRow`:

- Real **CheckBox** for every PostFx / foliage / terrain hex / underwater
  toggle.
- **ComboBox** for foliage kind and tonemapper (retire 17G cycler).
- Category headers with icons, collapse, and a Details **search** that hides
  non-matching rows.
- Transform stays 3×3 numeric (Unreal uses a vector widget — optional later).
- Outliner becomes a **TreeView**: name, type icon, visibility eye (if we
  already have hide; if not, icon-only until a Hide event exists — do not
  invent renderer flags casually).
- Context menu: Duplicate, Delete, Rename (if names are writable), Focus.
- Keep every current `InspectorField`. Adding rows is allowed; removing
  bindings is not.

**Undo gap (in-scope UX if safe):** Post FX, water, and vessel currently
have no undo. Prefer extending the existing live/commit command path rather
than a new stack. If that risks terrain/transform undo, **leave it** and
document; do not break Ctrl+Z for transforms.

**Load Scene:** still a stub. Window/File can show a disabled "Open…" with
a tooltip rather than a silent no-op, unless 26-C wires a real loader
without touching asset cooking (Phase 28). Do not start Phase 28 here.

### 12.5 Editor Help (F1) — in scope

A **Help** menu and a **?** button on the **right of the menu bar** (next to
FPS) open the same overlay. **F1** is the shortcut (UE `OpenDocumentation`,
Unity/Godot/Blender convention). Esc / click-away / ? toggle closes it.

This is Nielsen's tenth heuristic in product form: searchable, task-focused
help *inside the editor*, not a browser tab.

**Do not dump `context.md` or phase plans into the overlay.** Those are
engineer records. Curate short pages from docs we already have, then keep
the pages next to the engine so Help cannot rot:

| Help page | Source to rewrite from (do not include raw) |
|---|---|
| Welcome | `README.md` opening + three commitments, in plain language |
| Viewport & camera | `README.md` “Editor controls”; RMB+WASD, Shift, speed slider |
| Selection & gizmos | README T/R/S; `context.md` §8 routing; L light gizmos |
| Outliner & Details | This plan §7 / §12; existing inspector sections |
| Create & Import | README Create menu + File → Import Model |
| Content Drawer | `docs/editor/content_drawer.md` (docked tiles; keep in sync as types grow) |
| Terrain | README F6 / 1–6 / `[` `]` / `-` `=`; XV-I palette; paint vs foliage |
| Foliage | 17F tools; F8; kind combo |
| Lights & Post FX | Inspector field list; F5 shading; Kelvin note |
| Water | `docs/editor/water.md` (SSR / RT Reflect / Reflect Debug; Post FX RT Reflections) |
| Play mode | Play / Pause / Stop; overlays hide |
| Shortcuts | Full table from README + `app.rs` (Ctrl+Z/Y/S/N/D, Delete, F5/F6/F8, F9/F10, brackets) |
| Profiler & log | Phase 29 overlay; output log drawer |

**On-disk:** `docs/editor/*.md` (committed, engine-owned prose). A tiny
loader turns markdown into Text/Bullet/Heading widgets. No WebView. Search
filters the TOC. Context-sensitive F1 in v1 can open the page that matches
the hovered panel (viewport → Viewport; Details → Details; Drawer → Drawer);
if hit-test is ambiguous, open Welcome.

**Help menu also lists:** Editor Help (F1), Keyboard Shortcuts (jumps to
that page), About Somnium (version + licenses). No Forums / AnswerHub
clones.

**About** cites `ATTRIBUTION.md` in one short paragraph and the MIT/Apache
choice from `README.md`. Do not paste the whole attribution file.

---

## 13. Sub-phases

Order is dependency. Do not skip A/B to "get to the Drawer".

| ID | Name | Deliverable | Done when |
|---|---|---|---|
| **26-A** | Kernel | Splitter, anchored Popup, Image/Icon atlas bind, Tooltip, NineSlice, **Nocturne tokens in `theme.rs`**, bundled **Inter**, `visual_dirty` if cheap, Cargo.toml description fix | File/Create popups follow the buttons on resize; chrome uses lunar indigo + Inter (look **will** change — that is the identity landing); a smoke Image draws an icon; `cargo test` green |
| **26-B** | Controls | CheckBox, ComboBox, TreeView, TabControl, ContextMenu, SearchBox, Breadcrumb, PropertyRow | Unit/widget tests; **not yet** swapped into the inspector (except a hidden harness if useful) |
| **26-C** | Shell | Unreal-like chrome + Nocturne paint: live Edit/View/Window/Help, **Help ? + F1 overlay** with curated `docs/editor/` pages, icon toolbar, status bar, resizable columns, FPS actually updates, log as ring buffer, Play/Pause/Stop as icons | Every §14 item still works; Edit menu Undo/Redo/Delete/Duplicate; View menu profiler/gizmos; F1 opens Help, Esc closes it; evidence PNG of idle editor **and** Help open |
| **26-D** | Content Drawer | Browser widget + drawer + Dock in Layout + Show Engine Content + type icons + search + tiles/list + async PNG thumbs | Ctrl+Space toggle (including close); click-away dismisses undocked drawer; project `assets/` recursive; engine root hidden until toggle; Import still works |
| **26-E** | Details / Outliner | Tree outliner, PropertyRows, CheckBox/Combo replacements, inspector search, section icons | No InspectorField lost; foliage/tonemapper are combos; terrain palette and paint still arm brushes |
| **26-F** | Iris | Colour widget + lights + water colours/leftovers + particles + material base | See §11; Cancel restores; Kelvin lock; water defaults unchanged |
| **26-G** | Runtime UI | Screen-space `UiCanvas` API usable from `hello_engine` without `UiManager` editor chrome; 9-slice; simple HUD/pause stub **optional** as dogfood | A game can build a widget tree and draw it through `UiPass` (or a second pass) without linking editor layout |
| **26-H** | Type | **Slipped.** Bundled bitmap Inter, 1.5× supersample + window DPI raster. No SDF/MSDF, no rustybuzz/cosmic-text. | Latin at 12–24 px is readable; shaping/kerning remain a later pass |
| **26-I** | Polish | Command palette (Ctrl+P), toasts, HiDPI scale, layout width persistence, modal unsaved, ATTRIBUTION, context.md §8. Evidence PNGs **not invented** — capture live into `dev records/phase 26/` | Palette/toasts/layout persist/unsaved modal work; screenshots optional |

**26-D2 (optional inside D):** drag-drop spawn from drawer to viewport.
Requires Widget drag-drop restored. If not ready, ship double-click spawn.

**26-J (explicitly out of v1 unless requested):** reflection-driven inspector
(NeoAxis/Falco). World-space 3D canvases can also wait until 26-G screen-space
is real.

### 13.2 Shipped vs still open (2026-08-13)

| Shipped | Still open |
|---|---|
| Toolkit A/B, Nocturne shell C, docked Content Drawer D (tiles, not a popup), Details/Outliner E, Iris F, `UiCanvas` G, bitmap Inter H (SDF slipped), palette/toasts/HiDPI/layout persist/unsaved I | 26-J reflection inspector; 26-H SDF/shaping; 26-D2 drag-drop spawn; async PNG thumbs |
| Custom title bar (engine mark, “Somnium Engine”, min/max/close) | Native OS chrome is gone on purpose; keep engine widgets if the bar grows |
| Click-away closes menus/Help/palette/colour/unsaved **and ComboBox lists** | Docked drawer does **not** close on click-away |
| Named Sculpt tools with selected/hover/press fills | New tools (foliage modes, voxel brushes, etc.) must ship as labelled buttons, not two-letter codes |
| F1 Help: wrapped pages + TOC (Welcome, Viewport, Shortcuts, Content Drawer, About, Outliner, Terrain, **Water**) | Add a Help page (or section) whenever a feature adds authoring UI |
| Visible scrollbars on Outliner, Details, Help, Drawer | Any new tall pane should use `ScrollViewer`; thumb uses `MIN_THUMB.min(track_h)` so a 0-px track cannot panic |
| Immersive play (toolbar after Play; `IconId::ImmersivePlay` last in the enum; Esc exits; restore maximized) | Do not insert new `IconId` variants except at the end of the enum |
| ComboBox header in-place; list is a root `Popup` + `ComboDropdown` (opaque, File-menu z-order) | Do not go back to expand-in-place lists inside a vertical inspector stack |
| Content Drawer `ICON_DRAWER = 80` (tiles ~112×120) | Keep tiles readable; do not silently shrink back to 48 px |
| Toolbar Select / Landscape / Foliage wired (`SetGizmoMode`, `ToggleTerrainEdit`, `ToggleFoliage`) | Foliage toolbar enables the component; it does **not** arm paint |
| Terrain palette `set_selected`; Details search reapplied after per-frame inspector writes | Filter must survive `update_inspector` |

**Open by design:** Metaphor does not end when 26-J lands. Each later phase
that adds an authoring lever (animation graphs, cooked assets, networking
debug, new post-fx, terrain material UI in 25J, …) is expected to extend
this chrome — inspector bindings, menus, Content Drawer types, and
`docs/editor/*.md` — rather than bolting on a one-off panel. Help is a
living product surface, not a one-shot dump.

### 13.1 Suggested calendar (not a promise)

Largest phase to date. A–C are the framework+shell (several sessions). D is
the headline (several). E and F can overlap after B (F needs Popup+NumericField
only, but should sit in PropertyRow). G–I after the editor is honest.

---

## 14. Must-not-break inventory

Re-test after **every** sub-phase that touches `lib.rs` or `app.rs`:

1. Viewport RMB fly-cam WASD/QE + Shift.
2. LMB pick entity; gizmo T/R/S (W/E/R when not flying).
3. Outliner select syncs inspector and gizmo.
4. Create menu: every `CreateKind` still spawns.
5. File → Import Model (`rfd`) still imports glTF/GLB and selects a node.
6. Save / New scene.
7. Undo/Redo for **transform and light**.
8. Play / Pause / Stop / **immersive play**; Play hides editor overlays; immersive fills the monitor (Esc restores).
9. Terrain: F6, tools 0–5, 32-layer palette, paint vs foliage mutual exclusion,
   hex toggle, sculpt.
10. Foliage: paint/erase/single/kind, F8 enable, density/seed/slope/scale.
11. Post FX toggles + tonemapper cycle + all numeric post fields.
12. Water inspector scalars that already exist (do not drop them); 26-F adds
    colours without changing defaults.
13. Vessel inspector fields.
14. Camera speed slider + readout.
15. Profiler overlay toggle.
16. F5 shading mode.
17. Output log still receives `append_log` (and after 26-C, as a ring buffer).
18. UI does not eat viewport input when the cursor is over the 3D hole.
19. After 26-C: **F1** opens Help, **Esc** closes it; ? button matches.

If Play-in-editor currently hides chrome, the Drawer must hide too (or the
game canvas from 26-G shows — do not leave editor docks up in Play unless
that is already today's behaviour).

---

## 15. Items the original request left implicit (now in scope)

These would have shown up as "why is the editor still awkward" halfway
through implementation:

| Item | Why |
|---|---|
| Splitters / resizable docks | Unreal columns are not 40/280 forever |
| Status bar | Drawer and log live here in UE |
| Output Log as sibling drawer | Same `SWidgetDrawer` pattern |
| Anchored popups | Retire `(52, 28)` / `(148, 28)` |
| Tooltips | Every icon-only button needs a name |
| Context menus | Outliner, assets, viewport |
| Outliner hierarchy + search | Flat button list does not scale |
| Details search + collapse | ~120 fields |
| Command palette | Faster than hunting menus |
| Modal unsaved | New Scene is dangerous without it |
| Bundled font | Stop reading `C:\Windows\Fonts` |
| HiDPI | Atlas + font size follow window scale |
| FPS actually updating | Stub `send_message` |
| Log ring buffer | Silent stop at 200 is a bug |
| Edit / View / Window / Help | Inert labels today |
| Help ? / F1 overlay | Nielsen #10; curated from README + editor controls |
| Nocturne identity | Unreal IA, Somnium paint; Inter + lunar indigo |
| Engine mark | "Somnium Engine" text is not an icon |
| Asset type registry | One map: extension → icon + filter + colour |
| Favorites / recents | Small; UE has both |
| Derived-data badge for `bc7/` | Avoid treating packs as sources |
| Keyboard: Ctrl+Space close | Do not repeat UE 5.4 toggle bug |
| Tab to cycle fields | Inspector UX |
| Disabled Open Scene | Honest about LoadScene stub |
| `Cargo.toml` crate description | Still says egui |
| Game screen canvas | §26.2 original ask; 26-G |
| 9-slice | Buttons/panels; also game UI |
| Notification toasts | Import finished, save failed |
| Progress on import | Large glTF stalls with no feedback today |
| Settings page (UI scale, theme) | Minimal; full preferences later |
| Layout persistence | Splitter positions in a tiny json |
| World-space canvas | After screen-space; 26-G+ |
| SDF / shaping | 26-H; may slip |
| Reflection inspector | Post-Metaphor |
| Drag-drop assets | 26-D2 |
| Eyedropper colour from viewport | Post-Iris |
| Console `~` | Nice; not blocking |
| Source control status | Out of scope |
| Localization | Phase 33, not here |
| 25J terrain material UI | Separate; reuse ColorSwatch when both exist |

---

## 16. Non-goals

- Rewriting the renderer, terrain, water, lighting, physics, or ECS.
- Dear ImGui / egui / Qt / WebView editor.
- Copying Unreal or Atlus art.
- Cooking / hot reload / streaming (Phase 28).
- Prefabs (Phase 34), sequencer (36), animation (27).
- Full docking graph with tear-off floating windows (Godot/VS-style). v1 is
  splitters + tabs inside the one OS window.
- Platforms / DDC / Live Coding / Blueprint / Cinematics toolbars.
- Colour grading suite, LUT authoring, HDR spectrum > 1.
- Show Plugin Content / Show C++ folders.
- Dumping engine `.rs` / `.wgsl` as Content.
- Per-pixel terrain LOD, water `wave_speed` retune, Quixel/Bethesda/AI
  assets.

---

## 17. Attribution and legal

| Source | Allowed | Forbidden |
|---|---|---|
| Unreal Engine | IA, interaction, naming analogies (Content Drawer, Details, Show Engine Content) under EULA study of local mirror | Slate/UMG/EditorStyle source, Starship icons, any Epic art |
| Fyrox | Continue MIT port lineage; cite new files (dock, asset preview) when used | Copying Fyrox editor wholesale |
| Godot, Flax, Stride, O3DE, Wicked, rbfx | Patterns | Source dumps |
| Lucide / Phosphor / Tabler | Icons + LICENSE | Rebrand as original art |
| Atlus | **Codename and UX values only** | Screenshots, fonts, layouts, motion |
| Valve SDF / Chlumsky MSDF | Algorithm papers | — |

Update ATTRIBUTION:

- §1.4 — native editor IA (replace wry HTML ids when 26-C lands).
- §1.5 — keep colour-picker table; retitle under Metaphor.
- New § when icons land (pack name, license, path).
- Fyrox §13.x additions per widget file, same style as 13.15/13.18.

---

## 18. Evidence

Directory: `dev records/phase 26/` (create when the first capture exists).

Suggested corpus (do not fabricate):

- Idle editor shell in Nocturne (26-C)
- Help overlay open on Shortcuts page (26-C)
- Content Drawer open, project files, engine toggle off/on (26-D)
- Outliner tree + Details with checkboxes/combos (26-E)
- Light colour picker over the viewport (26-F)
- Water colour rows (26-F)
- Resize: popups still aligned (26-A/C)
- Play mode: chrome policy matches today (26-C)

Logs beside PNGs if a capture path exists. `cargo test` / `cargo check
--workspace` noted in the sub-phase completion paragraph.

---

## 19. Risks

| Risk | Mitigation |
|---|---|
| Big-bang rewrite blanks the editor | Strangler: A/B widgets, then migrate; never delete `build_editor_layout` until the new shell is wired to the same events |
| Popup clip in ScrollViewer | Root-canvas host (already File menu pattern) |
| Drawer focus vs viewport | Hit-test: docked drawer consumes; click-away does **not** dismiss it. An undocked popup (if reintroduced) dismisses and passes pick |
| Ctrl+Space vs camera | Space is not a fly key today; still only handle Ctrl+Space when the editor shell is up |
| Thumbnail hitch | Async only; icon placeholder first |
| Icon atlas legal | Lucide/Phosphor; no UE |
| Layout perf with 32-layer palette + log | Ring-buffer log; virtualize log lines if needed; do not rebuild outliner every frame if not already required |
| Play mode still showing docks | Match current hide behaviour |
| Iris linear vs sRGB | Shared encode helper + unit test (mid-grey, saturated blue) |
| Undo every colour mouse-move | Changing ephemeral; Changed/Cancel only |
| Water abs as colour blows extinction | Normalised + magnitude (11.2) |
| Scope explosion (SDF, reflection, world canvas, drag-drop) | Slip 26-H/J/D2 explicitly rather than stalling the Drawer |
| `lib.rs` 2.8k-line god file | Split `editor/` modules as chrome migrates (shell, outliner, details, drawer) — mechanical, not a behaviour change |

---

## 20. Next-session start checklist

A Metaphor **implementation** session should:

1. Confirm branch `dev` and that this file is the chrome contract.
2. Read the status block + §13.2 (shipped vs still open) + §3 + §14.
3. `cargo check --workspace` **before** edits (known-good baseline).
4. User must have **authorized implementation**.
5. **Do not restart at 26-A.** 26-A–I plus UX polish are in the tree. Extend
   chrome for the feature in hand (inspector section, menu, drawer type,
   `docs/editor/` page). 26-J only if explicitly requested. 26-H SDF and
   26-D2 remain queued.
6. Keep `EditorEvent` stable; add variants only when a new command has an
   `app.rs` handler in the same change.
7. Do not retune `WaterComponent::great_lakes`. Do not rewrite `context.md`
   §20. Do not download Quixel/Megascans. Do not copy UE/Atlus art.
8. `cargo fmt`, tests, and a short completion note.
9. Update `context.md` §8 / roadmap row 26 and ATTRIBUTION §1.4–1.5 when
   chrome changes.
10. Evidence PNGs only from a real capture.

**Do not implement inside a Halcyon (VV) or terrain session** unless the user
redirects. Terrain/lighting/animation work that *needs* new inspector fields
is expected to add those fields — that is Metaphor staying open, not a
forbidden fold-in. A new session starts at
[`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) and must not
re-implement VV-A–H.

---

## 21. Bibliography (study paths)

Local mirror: `C:\Users\adhir\Downloads\GE\example_repo` (workspace
`example_repo` is Jolt-only).

**Layout / IA**

- Epic, *Unreal Editor Interface* (UE 5.8 docs) — slot map for §7.
- Epic, *Find Help and Answers* (UE4/5 Help menu + F1 `OpenDocumentation`).
- UE `Editor/StatusBar/`, `Editor/ContentBrowser/`, `Editor/Slate`,
  `AppFramework/.../SColorPicker` — pattern only.
- Unity Manual, *Overlays* — contextual Scene tools; we use Help + tooltips
  instead of a second overlay stack in v1.
- Godot *Customizing the editor* — docks, layouts, theme presets.
- Godot PR `#111140` — Inter + `tnum` / `ss04` for editor UI (features, not
  their font files unless we vendor Inter ourselves under OFL).
- Blender manual — keymap / status-bar hints; Help as a window.

**Visual tokens (structure, not hex)**

- UE `StyleColors.cpp` / `EStyleColor` via public discussion; **starship-css**
  (`yashabogdanoff/starship-css`) documents the Starship token graph
  (surface ladder, primary vs select, folder accent). Study the graph;
  **do not copy `#0070E0` or Roboto**.
- `FEditorStyle` / `FCoreStyle` API docs — named brushes/fonts as a pattern
  for Somnium `theme.rs` token names.
- IBM Carbon, Radix Colors — professional density and contrast discipline,
  not the look.
- WCAG 2.1 SC 1.4.3 contrast; disabled-component exemption.

**UX principles**

- Nielsen Norman Group, *10 Usability Heuristics* (1994/2020) — especially
  #1 status, #6 recognition, #10 help.
- Fitts's Law — ≥24 px toolbar hits.
- Apple HIG / Microsoft Fluent — tooltip delay ~400–700 ms.
- Hick's Law — one Create menu, not twelve empty toolbars.

**Type / icons / text**

- Inter (Rasmus Andersson, SIL OFL 1.1) — bundled editor face.
- Lucide (ISC); Phosphor (MIT); Tabler (MIT).
- Valve SDF 2007; Chlumsky MSDF thesis.
- `cosmic-text` / `rustybuzz` — shaping if 26-H happens.

**Widget / engine UI**

- Fyrox `fyrox-ui` (pool, widget, message, draw, popup, grid) — already
  ported; dock + asset preview next.
- Flax `Engine/UI/UICanvas`, `GUI/`, Content importers / PreviewsCache.
- Godot `editor/editor_dock_manager`, `editor/file_system`, `EditorHelp`.
- Stride `Stride.UI`, thumbnail compiler.
- O3DE LyShine vs Editor Qt split.
- Wicked `wiGUI`, `wiFont`.
- Unity uGUI Canvas / EventSystem (game UI).
- WPF Measure/Arrange (MSDN) — algorithm Somnium already runs.
- rbfx native UI / RmlUI — do not triple-stack.

**Help content we already own**

- `README.md` — editor controls, build, commitments.
- `CONTRIBUTING.md` — setup only; too contributor-facing for F1.
- `context.md` §8 — UI routing (rewrite, do not paste).
- `crates/somnium_core/src/app.rs` — canonical shortcut list.
- `ATTRIBUTION.md` — one-paragraph About, not the whole file.

---

## 22. One-page summary for the implementer

Metaphor is Phase 26. It **replaces** the Iris-only plan; Iris is 26-F.

Build widgets (A/B), land **Nocturne** tokens, rebuild the Unreal-*like*
editor with Somnium paint (C, including **F1 Help**), ship a Content
Drawer with project files + Show Engine Content (D), make Details/Outliner
feel like a product (E), add colour (F), then let games use the same UI (G).
Text quality (H) and polish (I) were the v1 close; **the UI phase is not
over.** New engine features keep needing new chrome. 26-J and SDF text are
still queued; so is every inspector/menu/Help addition those features bring.

The editor must still sculpt terrain, paint foliage, play the boat, and
import glTF on the day the Drawer ships. If it does not, the sub-phase is
not done.

---

**AI disclosure:** Research synthesis from Somnium's `somnium_ui` / editor
event audit, the local engine study mirror, Epic's public editor-interface
documentation, existing ATTRIBUTION Fyrox/UE notes, and the superseded Iris
plan. It does not replace licenses. No third-party source was copied into
the engine as part of writing this plan.
