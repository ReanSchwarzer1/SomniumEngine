# Phase 26-Zeta — Nocturne Atelier

> **Status:** implementation started 2026-08-15 — Zeta-B and the first safe C/E/F shell vertical slice are in tree; the phase remains open
> **Prepared:** 2026-08-15
> **Purpose:** take Somnium's existing Nocturne editor from functional custom UI to a coherent, professional product identity without replacing the retained-mode UI or breaking editor behavior
> **Design gate:** Claude Design produces and the user approves the identity, token system, component library, and annotated editor screens before visual implementation begins
> **Controlling context:** [`phase_26.md`](phase_26.md) remains the behavioral and architectural contract. This document extends it; it does not reopen shipped Phase 26 features.

---

## 0. Executive decision

Somnium does **not** need an ImGui migration or another surface-only recolor. It already has the right broad information architecture and a capable custom Rust/wgpu widget stack. The professionalism gap is caused by six system-level issues:

1. **The UI colour pipeline is presently wrong.** Nocturne's sRGB byte values are sent as if they were linear values to an sRGB swapchain. The target encodes them again. The intended panel colour `#1C1E26` therefore appears as approximately `#5D606C`, which exactly matches the pale grey in the supplied Somnium screenshot. Palette design must not be judged or retuned until this is corrected.
2. **Tokens exist, but not yet as a design system.** Raw constants coexist with widget-local colours, font sizes, padding, radii, and state decisions.
3. **Typography has one bundled regular face and bitmap glyph placement.** It lacks weight hierarchy, kerning/shaping, robust fallback, and semantic text styles.
4. **Icons are a small hand-rasterized line set.** They are useful proof-of-concept assets, but do not yet form a broad, optically consistent editor language.
5. **The shell gives too many horizontal bands equal visual weight.** Global commands, editing mode, viewport tools, diagnostics, and status need clearer scope and hierarchy.
6. **Professional editor states and workflows are incomplete.** Details, Outliner, Content Browser, jobs/status, search, focus, keyboard access, changed/revert state, empty/error/loading states, and persisted workspaces must feel like one product.

The direction is **Nocturne Atelier**: a precise night-time creative instrument, not a generic grey developer tool and not a Lumina/Unreal clone. Keep the lunar indigo identity and the viewport-first editor model; rebuild the paint, type, icons, state grammar, and workflow surfaces around an original Somnium `S` mark.

### Non-negotiable decisions

- Keep `somnium_ui`; extend it rather than changing to Dear ImGui, egui, Qt, Taffy, or another UI framework.
- Study Lumina's systems, not its pixels, logo, colours, or source implementation.
- Do not describe the production logo as “Unreal's logo with an S.” The Unreal mark is not an attributable font template. Commission an original `S` with a distinct silhouette.
- Correct colour-space handling before approving any palette.
- Require a complete design system and state sheet, not one attractive hero mockup.
- Preserve every [`phase_26.md`](phase_26.md) must-not-break item and the current `EditorEvent` seam.

---

## 1. Request, document precedence, and scope

The current request supplies the product goal: make the editor genuinely professional, establish an original identity, research Lumina and other mature editors, investigate attributable fonts/icons/packages, define an original `S` mark, and prepare a plan whose first delivery phase is a Claude Design engagement.

The attached Phase 26 document supplies constraints and engine facts, not a separate instruction to redo all of Phase 26. Its controlling points are:

- Nocturne remains the base visual language: cool night surfaces and lunar indigo, with Unreal-like information slots but original Somnium paint.
- The editor is a custom retained-mode Rust/wgpu UI inspired by Fyrox patterns, not ImGui.
- 26-A–I shipped; 26-H shaping/SDF, 26-J reflection-driven inspection, 26-D2 drag/drop spawn, and async thumbnails remain open.
- UI work must preserve viewport input, selection, gizmos, scene creation/import, undo/redo, Play/Pause/Stop, terrain/foliage, post effects, water/vessel workflows, profiler controls, UI input capture, and Help.

### In scope

- Brand system and original `S` monogram.
- Correct colour-management contract and semantic tokens.
- Typography roles, font selection, shaping/fallback plan, and DPI behavior.
- One coherent icon family plus Somnium-specific extensions.
- Editor shell hierarchy, panels, drawers, status, workspaces, command/search surfaces.
- Details, Outliner, Content Browser, console/log, menus, popups, notifications, and common widget states.
- Accessibility semantics, keyboard/focus, contrast, scaling, and reduced-motion behavior.
- Visual regression, interaction audit, performance evidence, licensing, and provenance.

### Out of scope for Zeta unless separately approved

- Replacing `somnium_ui` with ImGui/egui/Qt.
- Copying Lumina or Unreal assets, marks, layouts, colours, or code.
- Shipping a node editor, complete arbitrary multi-window docking platform, or runtime game-UI authoring system merely because reference editors have one.
- Combining the visual refactor with unrelated renderer or gameplay work.
- Trademark clearance. A professional search by the owner remains necessary before public release.

---

## 2. Current Somnium audit

### 2.1 Architecture and assets inspected

| Area | Current implementation | Consequence for Zeta |
|---|---|---|
| UI framework | `crates/somnium_ui`, retained widget tree, measure/arrange, message routing, hit testing, wgpu `UiPass` | Sufficient base; preserve it |
| Editor assembly | `crates/somnium_ui/src/lib.rs`, 5,830 lines | Too many visual/workflow responsibilities in one file; split by surface during Zeta |
| Tokens | `theme.rs`, 77 lines of colour and size constants | Good seed, but needs palette → semantic → component aliases and live theme state |
| Text | `fontdue`, bundled `Inter-Regular.ttf`, bitmap atlas | Readable Latin baseline, but no true font hierarchy, shaping, kerning, or robust fallback |
| Icons | `icons.rs`, 719 lines, procedural strokes in a 512² atlas | Original and safe, but too limited and optically inconsistent for a full editor |
| Renderer | `ui_pass.wgsl` multiplies texture and `Unorm8x4` vertex colour | Must establish the correct sRGB/linear/premultiplied-alpha contract |
| Widgets | Buttons, menus, tabs, text/numeric fields, combo, colour picker, scroll, splitter, tree, toast, canvas | Enough primitives to style; several still own local visual decisions |
| Layout | Viewport, top chrome, mode/sculpt controls, right Outliner/Details, bottom Content Drawer | Familiar, but needs stronger command scoping and vertical economy |

### 2.2 P0 finding — verified double sRGB encoding

The swapchain deliberately prefers an sRGB format in `crates/somnium_renderer/src/context.rs`. UI colours are stored as authored sRGB bytes, for example:

```text
BG_VOID    #14161C
BG_CONTENT #181A20
BG_PANEL   #1C1E26
```

The vertex layout exposes those bytes as normalized floats. `ui_pass.wgsl` returns `in.color * tex_sample` directly. An sRGB render target expects **linear** fragment output and then performs the display encoding. Passing `#1C1E26` channel values as though they were linear produces:

```text
raw byte values interpreted as linear: 28, 30, 38
sRGB target encoding result:            93, 96, 108
observed screenshot panel:               #5D606C
```

The predicted and observed values are identical. This is not merely an aesthetic opinion; it is a render-contract defect.

**Zeta rule:** do not darken the token hex values to compensate. Define whether each colour/texture is authored in sRGB or linear, decode authored sRGB colours exactly once before the sRGB target, test straight versus premultiplied alpha, and capture pixel probes. Only then may Claude Design evaluate or modify Nocturne.

### 2.3 Design-system maturity audit

This is a heuristic design-system score, not a quality judgment on engine functionality.

| Category | Maturity | Evidence | Zeta target |
|---|---:|---|---:|
| Information architecture | 3/5 | Familiar viewport/Outliner/Details/Drawer structure exists | 4/5 |
| Colour/tokens | 2/5 | Useful Nocturne constants, but wrong colour-space path and local overrides | 5/5 |
| Typography | 1/5 | One regular bitmap font; no semantic weight/mono/fallback system | 4/5 |
| Iconography | 2/5 | Original atlas and IDs, but limited coverage/optical system | 4/5 |
| Component consistency | 2/5 | Strong widget base; state recipes and component aliases are incomplete | 5/5 |
| Interaction/feedback | 2/5 | Core actions, popups, toasts, Help work; busy/error/focus/revert state is uneven | 4/5 |
| Layout/workspaces | 2/5 | Persisted shell exists; contextual drawers/workspaces are incomplete | 4/5 |
| Accessibility | 1/5 | HiDPI work exists; no complete semantic tree/keyboard/contrast gate | 4/5 |
| Maintainability | 2/5 | Custom toolkit is coherent; main editor assembly and literals are concentrated | 4/5 |

**Overall baseline: 17/45.** The editor is functionally substantial, but the design language is not yet enforced as infrastructure.

### 2.4 Supplied-screenshot critique

What already works:

- The scene remains the dominant visual target.
- The left tool rail and right Outliner/Details produce an understandable editing model.
- Lunar indigo is recognizable and can become a distinctive accent.
- The custom title bar and native UI give Somnium a credible technical foundation.

What currently reads as unfinished:

- Double-encoded dark tokens make every surface look washed grey, eliminating depth and hierarchy.
- Four similar top bands compete instead of separating application, mode, viewport, and diagnostic commands.
- Lavender is spread across too much chrome, so selection and focus lose salience.
- Outliner and Details rows lack enough density, hierarchy guides, modified/revert cues, metadata, and state distinction.
- Content tiles use large areas with little information; previews, type identity, breadcrumb/history, filters, and density modes need a coherent workflow.
- The crescent mark is pleasant but does not yet behave as a full brand system.

---

## 3. What Lumina actually teaches

The current [Lumina repository](https://github.com/MrDrElliot/LuminaEngine) describes an experimental/educational engine hand-crafted over roughly three years and licensed Apache 2.0; it is not simply a one-year skin. Its [official editor documentation](https://luminagameengine.com/manual/editor/) and supplied screenshots show a compact, viewport-first editor.

The local source explains the result:

| System | Local evidence | Transferable lesson |
|---|---|---|
| Semantic palette | `Engine/Source/Runtime/Source/Tools/UI/ImGui/EditorColors.h`; style derived in `ImGuiRenderer.cpp` | Widgets consume roles, not random greys |
| Typography hierarchy | Lexend regular/bold at multiple tiers, JetBrains Mono for code, merged icon font | Font roles and weight do more work than a fashionable family name |
| DPI handling | Style and font scaling from monitor/resolution state | Test 100%, 125%, 150%, and 200%; do not scale only glyphs |
| Application chrome | Custom title/menu/status composition and measured hit regions | Stock framework chrome is not the source of polish |
| Workspace docking | Default viewport/inspector composition, dock classes, persistence | Excellent defaults and reset matter before unlimited docking |
| Bottom drawers | Content Browser and Output Log can open temporarily, resize, close, or promote to docks | Preserve viewport space without hiding high-frequency tools |
| Content Browser | Tree, history, breadcrumb, search, filters, thumbnails, tile zoom, multi-select, rename, drag/drop | Treat assets as a professional workflow, not a file grid |
| Property language | Reusable property table and reflection-generated inspector | Standardize every property row and eventually land Phase 26-J |
| Microinteractions | Hover/disabled/tooltips, full-edge window controls, drawer easing | Specify all states; animate only spatial/causal changes |

Lumina uses [Dear ImGui](https://github.com/ocornut/imgui), Material Design Icons, Lexend, JetBrains Mono, ImGuizmo, ImPlot, and a node editor, but Lumina is **not stock ImGui**. It has thousands of lines of editor-specific shell, widgets, thumbnails, docking, input routing, and property infrastructure. Dear ImGui also documents limitations around accessibility and internationalization. Migrating Somnium would exchange known behavior for a new platform project without producing Lumina's system automatically.

### What must not be copied

- The `L` logo, title-screen composition, palette values, texture assets, or pixel-identical skin.
- Unreal's circled `U` silhouette or Lumina's mark with one letter changed.
- Lumina source snippets without an explicit Apache-2.0 reuse decision, retained notices, and marked modifications.
- Font/icon binaries from the checkout. Re-source every adopted asset from its official upstream and ship its license.
- Lumina's very small ultrawide text, dense icon-only controls, or colour-only states.

---

## 4. External editor benchmark conclusions

Primary documentation from mature tools converges on the following:

- [Unreal Editor](https://dev.epicgames.com/documentation/unreal-engine/unreal-editor-interface?lang=en-US) validates Somnium's general shell: main commands, viewport-local tools, hierarchical [Outliner](https://dev.epicgames.com/documentation/unreal-engine/outliner-in-unreal-engine?lang=en-US), context Details, and an on-demand Content Drawer.
- [Godot's Inspector](https://docs.godotengine.org/en/stable/tutorials/editor/inspector_dock.html) is a stronger target for changed-value visibility, revert actions, property search, favorites, documentation, and compatible drag/drop.
- [Blender workspaces](https://docs.blender.org/manual/en/latest/interface/window_system/workspaces.html) show that task-specific saved layouts are more useful than one universal arrangement; its [status bar](https://docs.blender.org/manual/en/latest/interface/window_system/status_bar.html) also carries jobs, hints, scene statistics, and memory.
- [VS Code's UI model](https://code.visualstudio.com/docs/editing/userinterface) shows how one command surface can search commands, files/assets, symbols/entities, panels, settings, and recent items while menus remain discoverable.
- [JetBrains tool windows](https://www.jetbrains.com/help/idea/tool-windows.html) and [Search Everywhere](https://www.jetbrains.com/help/idea/searching-everywhere.html) demonstrate contextual edge tools, actionable status, keyboard focus, notifications, and unified search.
- Microsoft's [Fluent tokens](https://fluent2.microsoft.design/design-tokens), [layout](https://fluent2.microsoft.design/layout), [typography](https://fluent2.microsoft.design/typography), and [iconography](https://fluent2.microsoft.design/iconography) are useful system references even though Somnium should retain original paint.

### Zeta interface principles

1. **Viewport first.** At 1920×1080, normal top chrome should target roughly 72–88 logical pixels before the viewport, subject to the approved design and accessibility scale.
2. **Three command scopes.** Application commands live in global chrome; editing-mode commands live in a mode strip; camera/shading/snapping/diagnostics live against the viewport.
3. **Accent is punctuation.** Lunar indigo marks focus, selection, active modes, primary actions, and links—not whole panels.
4. **Semantic surfaces, not arbitrary greys.** Every panel, input, row, popup, selection, and border consumes a named role.
5. **One compact density grammar.** Use a 4 px base grid, 24–28 px dense rows, 28–32 px chrome rows, 16 px row icons, and 18–20 px toolbar icons, finalized through prototypes.
6. **Typography is navigation.** Use weight, size, spacing, and alignment to expose hierarchy; use sentence case and tabular numerals.
7. **Selection is one system.** Viewport, Outliner, Details, Content Browser, and search synchronize selection and visibly distinguish hover, keyboard focus, selected, active edit, disabled, hidden, locked, dirty, warning, and error.
8. **Status is actionable.** Save/dirty, selected object, mode, Play state, import/build/shader jobs, errors, FPS/frame time, objects, triangles, CPU/GPU/memory should open the relevant tool when clicked.
9. **Search is a backbone.** One command surface searches commands, entities, assets, panels, settings, recent items, and Help with category prefixes and shortcut hints.
10. **Professional defaults before arbitrary customization.** Ship excellent Layout, Terrain, Foliage, Lighting, Materials, Animation, Debug, and Play workspaces; persist and reset them.

---

## 5. Nocturne Atelier visual identity

### 5.1 Identity sentence

**Somnium is a precise instrument for constructing impossible worlds: nocturnal, calm, technical, and quietly cinematic.**

This means:

- Deep blue-black neutral surfaces, not washed mid-grey.
- Cool lunar indigo as a controlled active signal.
- Crisp seams and measured density rather than rounded-card decoration everywhere.
- A small amount of asymmetric “lunar cut” geometry as an ownable motif.
- Excellent legibility and technical numerics; no faux-futuristic body text.
- Motion only for spatial continuity, progress, state transition, and causal feedback.

### 5.2 Original `S` monogram brief

The production prompt must be:

> Design an original single-colour `S` monogram for Somnium Engine that preserves the existing lunar/crescent heritage. Form the S from two counter-rotating crescent blades or precision-cut ribbons separated by a diagonal negative-space channel. Reuse a controlled 12–15° chamfer as a broader Somnium motif. The silhouette must remain recognizable at 16 px. Avoid Unreal's enclosing ring or shield, central-U construction, serif proportions, outline silhouette, chrome, bevels, and gradients.

Three routes Claude must explore:

1. **Eclipse S — recommended.** A small upper crescent opens right; a larger lower crescent opens left; their negative space reads as `S`.
2. **Horizon S.** Two restrained angular ribbons offset around a clean diagonal horizon cut; flatter and more technical.
3. **Somnium Sigil.** A free-standing high-contrast blade/calligraphic `S` with one lunar notch; no surrounding badge.

Required logo delivery:

- Canonical one-colour SVG and reversed version.
- 16 px micro mark with simplified counters.
- 24, 32, 48, 64, 128, 256, and application-icon optical variants.
- Horizontal wordmark and mark-only lockups.
- Construction grid, safe area, minimum size, clear-space rules, and forbidden uses.
- Light/dark, monochrome, Windows icon, splash, title-bar, empty-state, and watermark applications.
- Similarity board beside Unreal, Lumina, Unity, Godot, and common technology `S` marks.
- Provenance record for every typeface/reference used.

The symbol should be drawn from first principles. An OFL typeface may seed wordmark exploration—the [SIL OFL FAQ](https://openfontlicense.org/ofl-faq/) allows fonts to be used in logos—but Somnium's mark should become original vector geometry, not a typed glyph.

### 5.3 Typeface research and prototype decision

Claude should prototype three complete screens with each of these systems rather than judging isolated specimen sheets:

| Direction | UI | Mono | Display/wordmark | Verdict |
|---|---|---|---|---|
| **Astral Precision** | [Geist Sans](https://github.com/vercel/geist-font) Regular/Medium/Semibold | Geist Mono | None or custom wordmark | Preferred if 12–14 px Windows rasterization passes |
| **Cinematic Accent** | [Inter](https://rsms.me/inter/) Regular/Medium/Semibold | [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | [Oxanium](https://github.com/sevmeyer/oxanium) Medium, only for rare display/wordmark use | Safest continuation of Nocturne |
| **Technical Heritage** | [IBM Plex Sans](https://github.com/IBM/plex), optional Condensed labels | IBM Plex Mono | Custom wordmark | Strong dense readability; must avoid looking like Carbon |

All listed fonts are available under SIL OFL 1.1 from official sources. Use static TTF cuts first: UI Regular/Medium/Semibold plus Mono Regular/Medium. Keep display faces out of inspectors, menus, console, and long labels. Use [Noto](https://notofonts.github.io/) as lazy script fallback rather than packing the full collection into the default atlas.

**Decision gate:** Geist is the preferred new identity direction, but Inter remains the fallback until actual Somnium glyph captures at 100/125/150/200% DPI prove Geist equal or better in dense fields.

### 5.4 Icon system

Adopt [Tabler Icons](https://github.com/tabler/tabler-icons) as the leading prototype: MIT licensed, 6,184 SVGs in the current official repository, drawn primarily on a 24×24 grid with 2 px strokes and available outline/filled variants. It offers broad coverage with simpler compliance than mixed web packs.

Rules:

- One production family only. Do not mix Tabler, Lucide, Phosphor, Material, and hand icons on one surface.
- Canonical sources remain SVG; import only icons Somnium uses.
- Visible sizes: 16 px rows, 20 px toolbars, 24 px large actions; validate optical centering at each scale.
- Outline means available; filled may mean selected/active only when the pair remains optically compatible.
- Default icons are neutral. Accent and status colour communicate state/meaning, never decoration.
- Every nonstandard icon gets a tooltip and a text-labelled menu/palette equivalent.
- Build an original Somnium extension on Tabler's grid for terrain sculpt, foliage, material graph, probes, ray tracing, water, camera/entity types, and engine-specific assets.
- The brand mark is not part of the utility icon family.

[Phosphor](https://github.com/phosphor-icons/core) is the stylistic fallback if Claude demonstrates that Tabler's outline language is too light; constrain Phosphor to Regular + Fill. Do not use Material Symbols as the primary identity because the result is strongly associated with Google/Material.

---

## 6. Design-system architecture target

### 6.1 Token layers

```text
authored palette (sRGB/OKLCH provenance)
        ↓
semantic roles (surface, text, border, accent, status, focus)
        ↓
component aliases (button.*, tree_row.*, input.*, tab.*, popup.*)
        ↓
state overrides (hover, pressed, focus, selected, disabled, busy, error)
        ↓
linear GPU values + explicit alpha contract
```

Minimum semantic roles:

```text
surface.window       surface.canvas      surface.panel
surface.header       surface.raised      surface.input
surface.popup        surface.modal_scrim surface.hover
surface.selected

text.primary         text.secondary      text.muted
text.disabled        text.inverse        text.link

border.subtle        border.default      border.strong
focus.ring

accent.default       accent.hover        accent.pressed
accent.selected_bg   accent.selected_rail

status.info          status.success      status.warning
status.error         status.busy
```

Every token must store:

- semantic name;
- source colour space and serialized sRGB value;
- linear runtime value or deterministic conversion path;
- intended foreground/background pairs;
- contrast test results;
- light/dark applicability;
- provenance and design rationale.

No widget may introduce a new raw hex, font size, radius, spacing value, or animation duration without adding a token or documented one-off exception.

### 6.2 Typography roles

Final values come from Claude's approved design, but the implementation needs named roles:

```text
display             20–24 px / semibold (rare empty/splash states)
title               16–18 px / semibold
section             13–14 px / semibold
body                13–14 px / regular
body_strong         13–14 px / medium
label               12–13 px / medium
caption             11–12 px / regular
mono                12–13 px / regular
mono_strong         12–13 px / medium
numeric             tabular numerals, right-aligned where appropriate
```

Text infrastructure must support kerning, ligatures where appropriate, bidi, fallback, ellipsis, selection/caret, and deterministic measurement. [cosmic-text](https://github.com/pop-os/cosmic-text) is the primary research candidate because it provides pure-Rust shaping, layout, bidi, fallback, editing, ligatures, and colour-emoji support under MIT/Apache-2.0. Integrate it only after a narrow spike proves compatibility with Somnium's atlas/render model; do not casually replace working input controls.

### 6.3 Geometry, spacing, and motion

- Base grid: 4 logical px.
- Dense row: prototype 24/26/28 px.
- Menu/tab/toolbar: prototype 28/30/32 px.
- Panel inset: 8 px; group gap: 12 px.
- Icon hit area: at least 24×24 logical px or sufficient separation.
- Corners: restrained 2–4 px on inputs/popups; editor seams remain edge-to-edge.
- Hairlines: physical-pixel aware; never rely on subpixel strokes that disappear at scaling factors.
- Motion: 90–160 ms for hover/press only when useful; 160–240 ms for drawer/spatial transitions; instant for high-frequency editing. Honor reduced motion.

### 6.4 Component state contract

Claude and implementation must cover, at minimum:

```text
rest, hover, pressed, keyboard-focus, selected,
active-edit, disabled, read-only, busy, loading,
drag-source, drop-valid, drop-invalid,
dirty/modified, warning, error, success,
empty, truncated, overflow, compact, HiDPI
```

This applies to buttons, icon buttons, tabs, menus, tooltips, text/numeric fields, combo boxes, checkboxes, sliders, tree rows, property rows, section headers, asset tiles, breadcrumbs, filter chips, popups, modal dialogs, toasts, splitters, scrollbars, drawers, title bar, status items, and viewport overlays.

---

## 7. Attributable package decision

| Need | Candidate | License | Zeta decision |
|---|---|---|---|
| SVG rasterization | [resvg](https://github.com/linebender/resvg) | MIT / Apache-2.0 | **Adopt after spike** for reproducible build-time or startup alpha-mask atlases |
| Colour correctness | [palette](https://github.com/Ogeon/palette) | MIT / Apache-2.0 | **Adopt or reproduce narrowly** for explicit conversion, OKLCH exploration, and contrast tests |
| Text shaping | [cosmic-text](https://github.com/pop-os/cosmic-text) | MIT / Apache-2.0 | **Spike**, then adopt if integration and editing tests pass |
| wgpu text renderer | [glyphon](https://github.com/grovesNL/glyphon) | MIT / Apache-2.0 / zlib | Optional only if cosmic-text cannot feed the existing atlas cleanly |
| Accessibility | [AccessKit](https://github.com/AccessKit/accesskit) | MIT / Apache-2.0 plus Chromium-derived BSD portions | Plan semantic IDs/roles/actions now; integrate in a bounded later sub-phase |
| Layout replacement | [Taffy](https://github.com/DioxusLabs/taffy) | MIT | Reject for Zeta; current retained layout exists |
| Vector GPU renderer | [Vello](https://github.com/linebender/vello) | MIT / Apache-2.0 | Reject for Zeta; too large and still evolving |
| Immediate-mode migration | [Dear ImGui](https://github.com/ocornut/imgui) | MIT | Reject as a visual-refactor dependency |

Lowest-risk icon path:

```text
official Tabler SVG + original Somnium SVG extension
        ↓ resvg at controlled scale
DPI-aware monochrome alpha-mask atlases
        ↓ semantic tint in existing UI shader
current Somnium UiPass
```

Do not download assets from “free font,” theme, or icon aggregation sites. GitHub/Figma availability alone is not a reuse license.

---

## 8. Zeta-A — Claude Design engagement (first execution phase)

No visual implementation begins until this phase is approved.

### 8.1 Context packet to give Claude

Provide:

- This document and [`phase_26.md`](phase_26.md).
- Current Somnium screenshot and a **second screenshot after the colour pipeline is corrected**.
- Lumina screenshots as reference only, clearly labelled “do not copy.”
- Current `theme.rs`, `icons.rs`, widget inventory, layout screenshots, and working video/GIF of interactions.
- Engine capabilities and the Phase 26 must-not-break list.
- 1920×1080, 2560×1440, 4K, ultrawide, and 125/150/200% scale constraints.
- The identity sentence and original `S` brief above.
- Exact fonts/icons under consideration with upstream license links.

### 8.2 Claude prompt

> Design Phase 26-Zeta “Nocturne Atelier” for Somnium Engine. Preserve the established cool blue-black/lunar-indigo language and the viewport-first DCC mental model, but make it feel like a mature, original professional tool. Do not copy Lumina, Unreal, or any reference skin. Develop an original crescent-derived S monogram and a complete editor design system. Work in semantic tokens and show every important interaction state. Optimize for dense technical use at 1920×1080 through 4K and 100–200% UI scale. The result must be implementable in Somnium's existing retained-mode Rust/wgpu UI.

### 8.3 Required deliverables

1. **Identity board:** principles, mood, permitted/forbidden motifs, three `S` routes, wordmark directions, and similarity review.
2. **Token sheet:** colour roles with sRGB values and intended pairs, typography roles, spacing, radii, strokes, elevation, opacity, motion, and density.
3. **Component library:** every core widget and all states from §6.4.
4. **High-fidelity editor screens:**
   - default scene and empty project;
   - terrain selected with dense Details;
   - foliage/landscape mode;
   - Content Browser drawer, docked, grid, list, filtering, loading, missing preview, and drag/drop;
   - Output Log with info/warn/error and active background job;
   - Play, Pause, stopped, unsaved, modal, command search, Help, and notification center;
   - compact 1280×720 failure/overflow case;
   - 1920×1080, 2560×1440, ultrawide, and a 150% scaled variant.
5. **Interaction annotations:** keyboard focus order, shortcuts, hover/press/drag behavior, drawer transitions, selection synchronization, changed/revert behavior, validation, and reduced motion.
6. **Asset package:** original SVGs, selected official font files, icon manifest, custom icon SVGs, licenses, provenance, and optical-size exports.
7. **Implementation redlines:** logical sizes, grid, label/value columns, min/max panel sizes, responsive collapse rules, and z-order.
8. **Visual-regression sheet:** one deterministic page showing every component and state.

### 8.4 Design approval gate

The user signs off only when:

- It is recognizable as Somnium with the title and logo hidden.
- The `S` is clearly original and readable at 16 px.
- Screens show real engine data and failure/loading states, not placeholder-perfect content.
- The token sheet covers every displayed colour and metric.
- Normal text pairs target at least 4.5:1 contrast; meaningful control/state graphics target 3:1, following [WCAG 2.2 text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html) and [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
- The design works at 1280×720 and 200% scaling without functional loss.
- Nonstandard icons have names/tooltips and every status is understandable without colour alone.
- No implementation requires changing the renderer/game/editor event contract merely to match the mockup.

---

## 9. Implementation sequence after design approval

Order is deliberate: correct perception first, then infrastructure, then paint and workflows.

### Zeta-0 — Freeze and evidence baseline

- Capture deterministic before screenshots at all target sizes/scales.
- Record pixel probes for every current theme token.
- Record UI CPU time, GPU time, draw calls, vertices, atlas occupancy/rebuilds, allocations, and input latency.
- Record every must-not-break interaction on one audit scene.
- Inventory raw colours, sizes, radii, literal font sizes, icon usages, tooltip gaps, and unreachable keyboard controls.

**Exit:** repeatable capture/audit command and baseline package committed; no visual change.

### Zeta-B — Colour contract and compositing correctness

- Define `Srgb8`, linear float, and alpha types or equally explicit APIs.
- Decode authored sRGB vertex colours once before the sRGB surface.
- Verify white-mask, font-mask, thumbnail, and coloured texture semantics independently.
- Audit straight versus premultiplied alpha, especially translucent selection, text edges, popups, and overlapping panels.
- Add unit tests for transfer functions and screenshot pixel assertions for opaque/translucent swatches.
- Re-capture the editor without retuning Nocturne values.

**Exit:** `#1C1E26` renders as `#1C1E26` within capture tolerance; blend tests pass; no scene-render colour regression.

### Zeta-C — Theme service and style recipes

- Replace direct global constants with a typed theme resource and immutable frame snapshot.
- Implement palette, semantic, component, and state alias layers.
- Add semantic spacing/type/icon/motion tokens.
- Create recipes for Button, IconButton, Tab, MenuItem, TextField, NumericField, Combo, Checkbox, TreeRow, PropertyRow, Section, AssetTile, Tooltip, Popup, Toast, Scrollbar, Splitter, Drawer, and StatusItem.
- Remove widget-local literals or document approved exceptions.
- Add token/state gallery and hot-reload only if it is safe and bounded.

**Exit:** core widgets receive all paint/metric decisions from theme/style APIs; raw-colour audit is clean.

### Zeta-D — Typography infrastructure

- Load approved UI Regular/Medium/Semibold and Mono Regular/Medium roles.
- Add correct kerning/shaping/fallback measurement through the approved text spike.
- Support tabular numerals and deterministic label/value alignment.
- Define truncation, ellipsis, wrapping, caret/selection, bidi, fallback, and missing-glyph behavior.
- Make atlases DPI-aware and observable; handle rebuild/exhaustion without a blank UI.
- Audit every 11–24 px role at 100/125/150/200% with ClearType-independent captures.

**Exit:** typography specimen and every editing control pass; no cursor/selection/measurement regressions.

### Zeta-E — Brand and icon asset pipeline

- Land approved original `S` sources, optical variants, application icon, splash/title lockups, and provenance.
- Vendor only selected Tabler SVGs and their MIT license.
- Add Somnium-specific extension icons on the same grid and optical weight.
- Build deterministic DPI-aware alpha atlases with `resvg` or the approved equivalent.
- Replace old icons through an `IconId → asset` manifest; preserve semantic IDs.
- Audit tooltips and label equivalents.

**Exit:** no mixed icon family; brand and utility glyphs are crisp and distinct at every target scale.

### Zeta-F — Shell hierarchy and workspaces

- Consolidate top chrome into global, mode, and viewport-context scopes.
- Keep the viewport largest in every default workspace.
- Convert Content Browser and Output Log to on-demand drawers that can promote to persistent docks.
- Build named Layout/Terrain/Foliage/Lighting/Materials/Animation/Debug/Play workspaces with Save/Reset.
- Make title/status items measured, actionable, and resilient to compact widths.
- Persist panels, selected tabs, drawer heights, tile density, and workspace state.

**Exit:** screenshots match approved redlines; compact/ultrawide layouts pass; persistence/reset are deterministic.

### Zeta-G — Professional workflow surfaces

**Outliner**

- Hierarchy guides; type icons; badges; hidden/locked/dirty/error states; multi-select; keyboard traversal; typed filters; context actions; viewport focus.

**Details**

- Fixed label/value grammar; searchable sections; modified rail/dot; per-property and per-section revert; favorites; units; validation; asset compatibility highlight; advanced disclosure; documentation link.
- Use this styling work to prepare, not fake, Phase 26-J's reflection-driven inspector. Land reflection separately when its schema is ready.

**Content Browser**

- Folder/catalog tree, breadcrumb, back/forward history, typed filter chips, adjustable grid/list density, async previews, type/status badges, metadata tooltip/preview, inline rename, keyboard/multi-select, context actions, and 26-D2 drag/drop when the input contract is ready.

**Log/status/search**

- Persistent job/error record; filter/search; timestamps/categories; command palette spanning commands/assets/entities/panels/settings/help; actionable status widgets and cancellable progress.

**Exit:** each surface passes its workflow checklist with real assets/entities and error states.

### Zeta-H — Interaction, accessibility, and motion

- Complete keyboard traversal, focus trapping/return, mnemonic/shortcut display, and visible focus rings.
- Give widgets stable semantic IDs, roles, labels, values, states, and actions in preparation for/through AccessKit.
- Ensure status is not colour-only; audit contrast and colour-vision simulations.
- Enforce [WCAG 2.2 target-size guidance](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html) through size or spacing exceptions appropriate to a dense desktop editor.
- Add reduced-motion mode, text/UI scaling, and screen-reader smoke tests when AccessKit lands.
- Audit cursor shapes, tooltips, disabled reasons, drag affordances, escape/cancel, and destructive confirmations.

**Exit:** keyboard-only audit completes; contrast/scale/focus gates pass; no input leak into the viewport.

### Zeta-I — Maintainability and regression harness

- Split `UiManager` editor construction by shell/viewport/Outliner/Details/Content/Log/Help/overlays while preserving event ownership.
- Add component gallery and golden screenshot scenes.
- Add token lints, raw-literal audit, license/provenance checks, icon manifest checks, and deterministic screenshot diff thresholds.
- Document how a new engine feature adds commands, icons, properties, Help, telemetry, and states without editing unrelated UI files.

**Exit:** a new sample property/tool can be added through documented APIs with no new visual literal and with automated evidence.

### Zeta-J — Full sign-off

- Run the full Phase 26 must-not-break matrix and new visual/accessibility matrix.
- Capture approved before/after sheets, component gallery, target resolutions/scales, and a short interaction video.
- Record CPU/GPU/draw/atlas metrics against baseline.
- Update `context.md`, `ATTRIBUTION.md`, `THIRD_PARTY_NOTICES.md`, licenses, and Phase 26 status honestly.
- Keep any incomplete item open; do not declare Zeta complete from screenshots alone.

---

## 10. Acceptance matrix

### 10.1 Identity and visual system

- Original `S` passes 16 px, monochrome, silhouette, and similarity review.
- Editor remains recognizable as Somnium when the title/logo are hidden.
- Corrected tokens render to expected captured sRGB values.
- One dominant accent; semantic status colours never replace the accent.
- Every displayed visual value maps to a token or documented exception.
- One icon family; custom icons conform to its grid/weight.
- Typography roles, weights, baselines, numeral alignment, and truncation are consistent.

### 10.2 Layout and workflow

- Verify 1280×720, 1920×1080, 2560×1440, 4K, ultrawide, and 100/125/150/200% scaling.
- Viewport remains the largest region in every default workspace.
- Panel positions/sizes/tabs, drawers, workspaces, and asset density restore across restart.
- Outliner/viewport/Details/Content selection stays synchronized.
- Content Browser passes tree/breadcrumb/history/search/filter/grid/list/preview/context/keyboard/drag tests.
- Details passes search/modified/revert/favorite/unit/validation/asset-drop/advanced tests.
- Save, dirty, import, build, shader compile, Play, warning, error, and progress are visible and discoverable.

### 10.3 Interaction and accessibility

- Every interactive component has rest, hover, pressed, keyboard focus, and disabled state.
- No unlabeled nonstandard toolbar icon lacks a tooltip.
- Normal text reaches 4.5:1; large text and meaningful control/state visuals reach 3:1.
- No required state is communicated by colour alone.
- Keyboard-only path covers menus, toolbar, viewport escape, Outliner, Details, Content, drawers, modals, Help, and command search.
- At 200% text/UI scale, no required content or function is lost.
- Reduced motion disables nonessential animation.

### 10.4 Engineering and performance

- Full Phase 26 must-not-break inventory passes.
- Scene colour/tone mapping is unchanged by UI colour fixes.
- UI CPU/GPU p50/p95, draw calls, vertices, allocations, and atlas occupancy are recorded before/after.
- No unexplained regression above the approved baseline budget; optimize or explicitly waive with evidence.
- No atlas-full blank UI, missing glyph, stale texture, one-frame input leak, or layout jump under stress.
- Visual regression sheet covers components/states, not just an assembled hero screen.

---

## 11. Licensing and provenance

Create `THIRD_PARTY_NOTICES.md` and a distributable `licenses/` directory before new assets ship.

| Asset/code class | Required treatment |
|---|---|
| OFL fonts | Ship copyright and OFL text; do not sell font files alone; rename modified font software when Reserved Font Name rules apply |
| MIT/ISC icons/code | Preserve copyright and license notices in source/distribution |
| Apache-2.0 code/assets | Preserve license and supplied NOTICE; mark modified reused files where required |
| AccessKit | Preserve MIT/Apache choice and Chromium-derived BSD notice material |
| Lumina | Research reference only by default; its Apache license does not grant branding/trademark rights |
| Figma/community assets | Do not use without an explicit, recorded license from the actual asset owner |

Record for every asset:

```text
name, version/commit, upstream URL, author/copyright,
license/SPDX, local source path, modifications,
retrieval date, required notices, shipped files
```

Practical license planning is included here; it is not legal advice. The Somnium name and final `S` require owner-led trademark clearance before public branding investment.

---

## 12. Risks and controls

| Risk | Control |
|---|---|
| Designing around the washed screenshot | Correct and capture the colour path before palette approval |
| “Make it like Lumina/Unreal” becomes copying | Use pattern matrix, original geometry, similarity review, and explicit red lines |
| Framework migration consumes the phase | Keep `somnium_ui`; reject architecture changes without a separate evidence-backed decision |
| Pretty mockup omits real states | Require component/state gallery, error/loading/compact screens, and interaction annotations |
| Futuristic font harms readability | Keep display face rare; approve UI fonts only from in-engine 12–14 px captures |
| Mixed icon packs look inconsistent | Select one family and build a controlled custom extension |
| Theme constants remain cosmetic | Add layered typed tokens, recipes, lints, and raw-literal audit |
| Text-stack rewrite breaks editing | Narrow spike, golden measurement/editing tests, staged adoption, rollback path |
| Docking scope explodes | Excellent named default workspaces/drawers first; arbitrary platform docking later |
| Accessibility postponed indefinitely | Design semantic IDs/roles/actions during component work; make gates release criteria |
| Visual work breaks renderer/editor tools | Deterministic must-not-break scene and measured before/after evidence at every sub-phase |
| Licenses get lost in copied assets | Source only from official upstream; automated manifest and notices check |

---

## 13. Research sources and confidence

### Direct evidence — high confidence

- Somnium source: `crates/somnium_ui`, `crates/somnium_renderer/src/context.rs`, `context.md`, `ATTRIBUTION.md`, and [`phase_26.md`](phase_26.md).
- Supplied Somnium/Lumina screenshots.
- Local Lumina source at `C:\Users\adhir\Downloads\LuminaEngine-main\LuminaEngine-main`.
- [Lumina repository/license](https://github.com/MrDrElliot/LuminaEngine) and [official editor manual](https://luminagameengine.com/manual/editor/).
- Official upstream repositories linked in §§5–7.

### Primary product documentation — high confidence for supported patterns

- Unreal, Godot, Blender, VS Code, JetBrains, Fluent, and W3C links in §§4 and 8–10.

### Inference — must be validated in Somnium

- Geist as the preferred final UI family.
- Tabler as the final visual fit rather than only the licensing/coverage leader.
- Exact density, spacing, motion, and surface values.
- cosmic-text/glyphon integration cost and performance.
- AccessKit coverage for every custom widget.

The reference research establishes what mature editors support; it does not prove that every pattern is right for Somnium. Claude prototypes and instrumented in-engine tests decide the final system.

---

## 14. Start checklist

When the user returns with Claude Design output:

1. Read this document, [`phase_26.md`](phase_26.md), `context.md` §8/§16/§17.6, and the latest handoff.
2. Validate the design package against §8.3 before coding.
3. Run Zeta-0 and preserve its evidence.
4. Fix Zeta-B colour correctness before adjusting palette values.
5. Land one vertical slice—theme + type + icons + states for a small shell region—and audit it before broad rollout.
6. Proceed in the Zeta-C → J dependency order unless measured evidence justifies a change.
7. Keep the engine runnable and the Phase 26 must-not-break matrix green after every sub-phase.

**Definition of done:** Somnium's editor is a coherent, original, accessible, measurable product system—not merely a darker screenshot—and its identity, components, workflows, implementation rules, evidence, and third-party provenance are all documented and reproducible.

---

## 15. Implementation ledger

### 2026-08-15 — approved-design vertical slice

Implemented without changing the `EditorEvent` contract:

- exact IEC 61966-2-1 sRGB transfer and one-decode UI shader contract, with a
  linear-surface shader variant and straight-alpha preservation;
- typed immutable `NOCTURNE` snapshot plus palette, semantic, typography,
  density, geometry, and motion roles; common widget-local colour literals were
  migrated to semantic aliases;
- approved token JSON, Eclipse-S responsive SVG set, sixteen original Somnium
  icon SVGs and manifest/provenance, plus procedural atlas counterparts for the
  terrain and specialist glyphs used by the current renderer;
- application / mode / viewport-context command scopes at 36 / 32 / 32 px,
  with menus rehosted into the application bar and a visible Ctrl+P search
  entry wired to the existing command palette;
- regression tests for colour transfer, theme alpha, shader surface variants,
  atlas coverage, the 100 px shell budget, Create actions, and primary
  transport controls.

Verification: `cargo test -p somnium_ui` passes 39/39; the serial
`cargo test --workspace -j 1` gate passes; a live `hello_engine` frame-5 smoke
run exits cleanly with no wgpu validation error. Its renderer capture is
`dev records/phase 26/zeta_runtime_smoke.png`; the current capture hook runs
before editor chrome, so UI geometry and wiring evidence comes from the layout
tests rather than a fabricated UI screenshot.

Still open and not represented as complete: Zeta-D typography/shaping,
workspace presets and broader workflow rebuilds, complete interaction-state
and accessibility coverage, visual golden/performance evidence, and Zeta-J
sign-off. The Phase 26 must-not-break inventory remains the controlling gate.
