# Phase 26-Zeta — Nocturne Atelier

> **Status:** open. Zeta-B–I are substantially in tree as of 2026-08-16. Zeta-J sign-off and a human keyboard/interaction pass remain.
> **Prepared:** 2026-08-15 · **Last implementation pass:** 2026-08-16
> **Purpose:** take Somnium's existing Nocturne editor from functional custom UI to a coherent, professional product identity without replacing the retained-mode UI or breaking editor behavior
> **Design gate:** **passed.** The Claude Design package is delivered and vendored at [`phase 26/design/`](phase%2026/design/); its machine-readable contract is [`nocturne.tokens.json`](phase%2026/design/assets/tokens/nocturne.tokens.json). See §8A.
> **Controlling context:** [`phase_26.md`](phase_26.md) remains the behavioral and architectural contract. This document extends it; it does not reopen shipped Phase 26 features.

### Where things stand at a glance

| Sub-phase | State | Evidence |
|---|---|---|
| Zeta-0 baseline | partial — the capture path exists, the frozen before/after package does not | `SOMNIUM_CAPTURE_UI_PNG` |
| Zeta-B colour contract | **done** | `color.rs`, `pass.rs`, transfer tests |
| Zeta-C theme service + recipes | **done**; a raw-literal lint is still missing | `theme.rs`, `style.rs`, 7 state tests |
| Zeta-D typography | **done** for roles and weights; shaping/bidi/fallback remain | `typography.rs`, five bundled cuts |
| Zeta-E brand + icons | **done** — 67 Tabler + 16 Somnium + the Eclipse mark, rasterized by `resvg` | `icon_svg.rs`, coverage test |
| Zeta-F shell | **done** for scopes, the 68 px budget, collapse rules, workspaces and the status bar | layout + collapse + workspace tests |
| Zeta-G workflows | partial — Details grammar and revert are wired; browser/filter/preview workflows are not | `property_row.rs` |
| Zeta-H interaction / a11y | partial — Tab ring, Esc layer order, focus rings; no AccessKit, no human pass | `focus_stops`, `close_top_overlay` |
| Zeta-I maintainability | **done** for the split — `lib.rs` 6,554 → 3,938, five `editor/` modules; lints remain | `editor/` |
| Zeta-J sign-off | partial — automated §14 coverage lands; five items still need a human | `mod must_not_break` |

---

## 0. Executive decision

> **Historical.** §0 and §2 record the state Zeta was written against
> (2026-08-15), not the state of the tree. Six of the issues below are now
> addressed — see the status table at the top and the §15 ledger. They are kept
> verbatim because the reasoning is what justifies the design, and rewriting an
> audit after acting on it destroys the evidence that the action was warranted.

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

## 2. Somnium audit as of 2026-08-15 (historical)

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

Re-scored 2026-08-16 after the Zeta-B/C/D/F passes: colour/tokens 5/5,
typography 3/5 (roles and weights land; shaping, bidi and fallback do not),
component consistency 4/5, information architecture 4/5, layout/workspaces 3/5,
iconography 2/5 (unchanged — the runtime atlas is still procedural),
interaction 3/5, accessibility 2/5, maintainability 2/5 (unchanged — `lib.rs`
has not been split). **28/45.** The two categories that have not moved are
exactly the two whose next steps are named in Zeta-E and Zeta-I.

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
| Text shaping | [cosmic-text](https://github.com/pop-os/cosmic-text) | MIT / Apache-2.0 | **Spike**, then adopt if integration and editing tests pass. Still open — Zeta-D shipped weight hierarchy on `fontdue` first, because weights were the visible gap and shaping is the risky one |
| UI + mono faces | [Inter](https://rsms.me/inter/) 4.1, [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) 2.304 | SIL OFL 1.1 | **Adopted 2026-08-16.** Five static cuts, subset with `fontTools`, notices in `THIRD_PARTY_NOTICES.md`. Geist remains the preferred direction pending in-engine captures |
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

### 8.4 Design approval gate (passed)

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

## 8A. The delivered design package

The package is vendored at `dev records/phase 26/design/` so the redlines
cannot drift away from the repository that implements them.

| File | What it is |
|---|---|
| `Nocturne Atelier - System.dc.html` | Deliverables 1–3 and 5–7: identity board, three `S` routes with a similarity review, the token sheet, the component/state library, interaction annotations, redlines, asset list |
| `Nocturne Atelier - Editor Screens.dc.html` | Deliverable 4: S1 terrain + dense Details, S2 foliage + Content Browser states, S3 play/modal/palette/notifications, S4–S6 the 1280×720, 150 % and 3440×1440 cases |
| `assets/tokens/nocturne.tokens.json` | The machine-readable contract. `theme.rs` must stay a faithful transcription of it |
| `assets/brand/*.svg` | Eclipse / Horizon / Sigil routes, the 16 px micro cut, the reversed mark, the horizontal lockup |
| `assets/icons/somnium/*.svg` + `icon-manifest.json` | Sixteen original engine-specific icons on the Tabler 24×24 / 2 px grid |

Both HTML files render as static documents; the template placeholders
(`{{ … }}`) are filled by data arrays at the bottom of each file, which is
where the redline table, the contrast pairs and the interaction annotations
actually live. Read the `<script data-dc-script>` block, not just the render.

### 8A.1 The one number that drives the shell

The package's headline finding is not colour. It is that the pre-Zeta shell
gave four horizontal bands equal weight — title 36, menu 28, main toolbar 32,
viewport toolbar 26 — and spent **122 logical px** before the scene started.
Zeta folds those into three *scopes*, only two of which take layout space:

| Scope | Height | Contents | Takes layout space |
|---|---|---|---|
| Application | 36 | mark, wordmark, menus, command search, jobs, window controls | yes |
| Mode | 32 | save, Select / Landscape / Foliage, create, transport, play state | yes |
| Viewport context | 32 | camera, shading, snap, profiler, fullscreen | **no** — floats over the render at a 12 px inset |

**122 px → 68 px.** Floating the third scope is what buys the last 32: the
controls end up next to the thing they change instead of stacked above it.

### 8A.2 Redlines — sizes and collapse rules

| Region | Logical size | Collapse rule |
|---|---|---|
| Application scope | 36 px · 8 px inset | Below 1100 px the menu labels collapse to one ☰ popup; mark and window controls never collapse |
| Mode scope | 32 px · 30 px controls | Below 1400 px transport labels drop to icons; below 1100 px Create moves into an overflow ⋯ |
| Viewport context bar | 32 px · floats, 12 px inset | Below 1280 px it splits into two floating clusters, left and right |
| Left tool rail | min 120 · default 168 · max 280 | Under 120 px it becomes a 44 px icon-only rail with tooltips |
| Outliner | min 180 · default 300 px tall | Metadata column hides below 240 px wide; type icons never hide |
| Details | min 240 · default 340 px wide | Under 240 px property rows stack label over value at 40 px |
| Bottom drawer | min 140 · default 220 · max 60 % of height | Grid density steps 72 / 88 / 104 / 128 px; below 160 px tall it forces list mode |
| Status bar | 26 px | Items drop right to left: triangles → memory → objects → frame time. FPS and dirty state never drop |
| Splitters | 6 px hit, 1 px paint | Double-click resets to the workspace default; drag clamps to the min/max above |
| Hit targets | ≥ 24 × 24 logical | Where density forces 22 px, 4 px of separation on every side satisfies WCAG 2.2 target spacing |

**Details column grammar.** Label column is 46 % of panel width clamped to
96–176 px; the value column takes the rest minus a 14 px left gutter and an
8 px right inset. Vector rows split the value column into three equal cells
with a 4 px gap and the X/Y/Z tag inside the field at 10.5 px `text.muted`.
Labels never wrap — they ellipsise and gain a tooltip.

**Z-order.** 0 viewport passthrough (never hit-tested) · 1 docked panels,
drawers, status · 2 viewport overlays (context bar, profiler, gizmo HUD) ·
3 popups (menus, combo lists, context menus, colour) · 4 tooltips · 5 command
palette and notification centre · 6 modal scrim and dialog, focus trapped.

### 8A.3 Contrast pairs the package certifies

Every pair below is measured against `surface.panel` `#1C1E26`.

| Role | sRGB | Ratio |
|---|---|---|
| `text.primary` | `#D8DCE8` | 11.4:1 |
| `text.secondary` | `#9AA3B5` | 6.2:1 |
| `text.muted` | `#7E8698` | 4.5:1 |
| `text.disabled` | `#5C6478` | 2.6:1 — WCAG-exempt, inactive only |
| `focus.ring` / `accent.hover` / `text.link` | `#949CFF` | 6.0:1 |
| `accent.default` | `#7A86FF` | 4.8:1 |
| `accent.pressed` | `#5C68E0` | 3.4:1 |
| `border.strong` | `#4A4F5E` | 3.2:1 |
| `surface.selected` | `rgba(122,134,255,.16)` | 3.1:1 |
| `status.info` / `success` / `warning` / `error` | `#59B8D6` / `#5DCE9A` / `#E6B04A` / `#E05A5A` | 6.4 / 8.1 / 8.4 / 4.9:1 |
| `folder.sand` | `#C4A574` | 7.3:1 — folders only, never an accent |

### 8A.4 The state grammar, in four cues

The package is emphatic that the editor should not invent a fifth:

1. **hover wash** — never a border change, because a border change reflows the row;
2. **1 px focus ring** in `focus.ring`, composing with any other state;
3. **2 px selection rail** in `accent.selected_rail`, *always* paired with the
   translucent fill so selection survives a colour-vision simulation;
4. **gutter dot** for modified — the only modified cue, no italics, no recolour.

Press darkens rather than shifts: no control moves under the cursor.

### 8A.5 Interaction annotations that constrain implementation

- **Focus order.** application → mode → viewport context → left rail →
  viewport (one stop; Enter enters camera control, Esc leaves) → Outliner (one
  stop, arrows traverse) → Details (one stop per section) → drawer → status.
  A focus landing on a scrolled-out row scrolls it into view without animation.
- **Esc closes exactly one layer**, in order: modal → palette → popup → drawer
  → filter → selection.
- **Drag and drop.** Source dims to 60 %; the cursor carries a 104 px tile
  ghost; valid targets outline indigo dashed over an 18 % fill; invalid targets
  outline rose and put the *reason* in the status bar. Drop spawns at the
  picked surface point, never the origin.
- **Changed and revert.** A 5 px indigo dot in the 14 px gutter; clicking it
  reverts that property, the section header dot reverts the section, and revert
  is one undo step. Live scrubbing writes `ValueChanging` and never creates an
  undo entry; commit writes `ValueChanged` once.
- **Validation is in place, never modal.** An out-of-range value keeps the
  typed text, turns the border rose, and states the constraint at 11 px to the
  right of the field. Leaving an invalid field restores the last valid value
  and toasts what happened. Nothing is silently clamped.
- **Reduced motion.** Drawer travel, popup fade, toast slide and progress
  shimmer all resolve to 0 ms. Progress still advances numerically. No
  functionality depends on an animation completing.

### 8A.6 Decisions still owed to the package

- **Monogram route.** Eclipse (A) is recommended and all three SVGs are
  delivered, but the route has not been picked, so the optical ladder, the
  `.ico` set and the splash lockup have not been cut.
- **UI face.** Geist is the preferred direction; Inter is the shipped fallback
  and stays shipped until in-engine captures at 12 px / 100 / 125 / 150 / 200 %
  prove Geist equal or better in the Details column. Zeta-D shipped Inter.
- **Tabler.** The manifest expects `assets/icons/tabler/`; nothing is vendored
  there yet, so the runtime still draws the procedural `icons.rs` strokes.
- **The visual-regression sheet** (deliverable 8) was offered but not produced.

---

## 9. Implementation sequence after design approval

Order is deliberate: correct perception first, then infrastructure, then paint and workflows.

### Zeta-0 — Freeze and evidence baseline

> **Status: partial.** No frozen before-package exists. What does exist is a
> repeatable capture: `SOMNIUM_CAPTURE_UI_PNG=<file> SOMNIUM_CAPTURE_FRAME=120
> SOMNIUM_CAPTURE_QUIT=1 hello_engine` writes the swapchain *after* the UI pass,
> which is the first capture in the project that can show editor chrome at all.
> The remaining Zeta-0 work is to run it at every target size and scale and to
> record the CPU/GPU/draw/atlas numbers beside it.

- Capture deterministic before screenshots at all target sizes/scales.
- Record pixel probes for every current theme token.
- Record UI CPU time, GPU time, draw calls, vertices, atlas occupancy/rebuilds, allocations, and input latency.
- Record every must-not-break interaction on one audit scene.
- Inventory raw colours, sizes, radii, literal font sizes, icon usages, tooltip gaps, and unreachable keyboard controls.

**Exit:** repeatable capture/audit command and baseline package committed; no visual change.

**Remaining:** capture at 1280×720, 1920×1080, 2560×1440, ultrawide and
100/125/150/200 %; record UI CPU/GPU time, draw calls, vertices, atlas
occupancy and allocations; walk the §14 must-not-break inventory once and
record the result rather than assuming it.

### Zeta-B — Colour contract and compositing correctness

> **Status: done (2026-08-15).** `#1C1E26` reaches the framebuffer as
> `#1C1E26`; alpha is straight through the widget API and premultiplied at
> blend; transfer-function and shader-variant tests are in tree.

- Define `Srgb8`, linear float, and alpha types or equally explicit APIs.
- Decode authored sRGB vertex colours once before the sRGB surface.
- Verify white-mask, font-mask, thumbnail, and coloured texture semantics independently.
- Audit straight versus premultiplied alpha, especially translucent selection, text edges, popups, and overlapping panels.
- Add unit tests for transfer functions and screenshot pixel assertions for opaque/translucent swatches.
- Re-capture the editor without retuning Nocturne values.

**Exit:** `#1C1E26` renders as `#1C1E26` within capture tolerance; blend tests pass; no scene-render colour regression.

### Zeta-C — Theme service and style recipes

> **Status: done (2026-08-16) except the lint.** `theme.rs` is the typed
> immutable `NOCTURNE` snapshot and now carries the whole token sheet —
> `body_strong` / `mono_strong`, `gap_section`, `radius_tile`, the opacity
> ladder and the elevation table. `style.rs` is the new recipe layer: `button`,
> `primary_button`, `icon_button`, `input`, `tree_row`, `asset_tile`,
> `drop_target`, `popup` and `status`, each resolving a `VisualState`
> (interaction + focus + modified + invalid) into a `Paint`. `Button`,
> `NumericField` and `TreeView` consume it; seven tests pin the state grammar,
> including "selection is never carried by colour alone".

- Replace direct global constants with a typed theme resource and immutable frame snapshot.
- Implement palette, semantic, component, and state alias layers.
- Add semantic spacing/type/icon/motion tokens.
- Create recipes for Button, IconButton, Tab, MenuItem, TextField, NumericField, Combo, Checkbox, TreeRow, PropertyRow, Section, AssetTile, Tooltip, Popup, Toast, Scrollbar, Splitter, Drawer, and StatusItem.
- Remove widget-local literals or document approved exceptions.
- Add token/state gallery and hot-reload only if it is safe and bounded.

**Exit:** core widgets receive all paint/metric decisions from theme/style APIs; raw-colour audit is clean.

**Remaining:** migrate the widgets that still choose their own colours
(`check_box`, `combo_box`, `tab_control`, `scroll_viewer`, `splitter`, `toast`,
`color_picker`); add the raw-literal lint that makes the exit condition
enforceable rather than aspirational; add the token/state gallery scene.

### Zeta-D — Typography infrastructure

> **Status: done (2026-08-16) for roles and weight hierarchy; shaping remains.**
> This was the 1/5 category in the §2.3 audit and the single biggest reason the
> shell read as amateur: one bundled face meant hierarchy could only be
> expressed with size and colour.
>
> Five cuts now ship — Inter Regular / Medium / SemiBold and JetBrains Mono
> Regular / Medium, all OFL, subset from official upstreams and recorded in
> `THIRD_PARTY_NOTICES.md`. `typography.rs` owns two layers: `FontRole`
> (resolved once at startup through a process-wide `FontRegistry`, so the ~300
> builder call sites in `lib.rs` did not each need a new parameter) and
> `TextRole` (`display` / `title` / `section` / `section_caps` / `body` /
> `body_strong` / `label` / `caption` / `mono` / `mono_strong`).
> `TextBuilder::with_role` applies size, face, colour, tracking and case
> together, and `DrawingContext::push_text_tracked` plus
> `FontAtlas::measure_text_tracked` give the uppercase header role real
> letter-spacing that measures the width it draws.
>
> **On `tnum`.** The token sheet asks for tabular figures. `fontdue` applies no
> OpenType features, so the feature cannot be switched on. `mono_strong` routes
> numeric fields to JetBrains Mono instead, whose digits are one advance wide by
> construction — the redline's actual requirement ("a scrub never shifts the
> row") is met by the face rather than by a flag. Do not "fix" this by
> pre-padding numbers.

- Load approved UI Regular/Medium/Semibold and Mono Regular/Medium roles.
- Add correct kerning/shaping/fallback measurement through the approved text spike.
- Support tabular numerals and deterministic label/value alignment.
- Define truncation, ellipsis, wrapping, caret/selection, bidi, fallback, and missing-glyph behavior.
- Make atlases DPI-aware and observable; handle rebuild/exhaustion without a blank UI.
- Audit every 11–24 px role at 100/125/150/200% with ClearType-independent captures.

**Exit:** typography specimen and every editing control pass; no cursor/selection/measurement regressions.

**Remaining:** the `cosmic-text` spike for kerning, shaping, bidi and script
fallback; DPI-aware atlas observability and graceful behaviour when the atlas
fills; the 11–24 px capture audit at 100/125/150/200 % that decides Geist
versus Inter; a typography specimen scene.

### Zeta-E — Brand and icon asset pipeline

> **Status: done (2026-08-16).** Route **A (Eclipse)** is chosen — it is the
> package's own recommendation, and the diagonal channel between the two blades
> is what keeps it off the generic-tech-`S` pile. `somnium-s-eclipse.svg` is now
> the engine mark at runtime.
>
> The utility family is **Tabler** (MIT): 67 outline SVGs, one per `IconId`,
> vendored individually under `assets/icons/tabler/` rather than redistributing
> the 6,000-icon upstream, and renamed to the `IconId` they serve so the mapping
> is legible from the directory listing. The sixteen original Somnium icons sit
> beside them on the same 24 × 24 / 2 px grid.
>
> `icon_svg.rs` compiles every source in with `include_str!` and rasterizes it
> at startup through `resvg` (default features off — the text feature would drag
> a second font stack in beside `fontdue`). Only the alpha channel is kept, so
> the existing shader still tints each glyph with a semantic colour. Any
> `IconId` without a source keeps its procedural fallback, so a new variant
> degrades to hand-drawn art rather than to an empty cell. A test renders all 83
> and fails any that parses but marks fewer than a dozen pixels — a blank cell
> that replaced a working glyph is worse than a missing source.

- Land approved original `S` sources, optical variants, application icon, splash/title lockups, and provenance.
- Vendor only selected Tabler SVGs and their MIT license.
- Add Somnium-specific extension icons on the same grid and optical weight.
- Build deterministic DPI-aware alpha atlases with `resvg` or the approved equivalent.
- Replace old icons through an `IconId → asset` manifest; preserve semantic IDs.
- Audit tooltips and label equivalents.

**Exit:** no mixed icon family; brand and utility glyphs are crisp and distinct at every target scale.

**Remaining:** the atlas is a single 32 px cut, not the DPI ladder. 32 → 16 is
an exact 2:1 box filter and 32 → 20/24 is a mild downscale, so 100 % is correct;
at 200 % the 24 px action icons upscale 1.5× and go slightly soft. Regenerating
the atlas on a DPI change needs `IconId::uv_rect` to read runtime atlas
dimensions instead of the current consts. Also still owed: the optical ladder
(16/24/32/48/128/256), the `.ico` set and the splash lockup for the chosen
route.

### Zeta-F — Shell hierarchy and workspaces

> **Status: done (2026-08-16).**
>
> The three command scopes are real: application 36 with the mark, wordmark,
> menus, Ctrl+P search entry, help and window controls; mode 32 with Save and
> the three editing modes now carrying **labels** (the package forbids icon-only
> controls, and phase_26 §2.4 already required recognition over recall) grouped
> by hairline separators from the transport triple; and the viewport context bar
> reparented into the viewport as a translucent 32 px overlay inset 12 px.
> **The scene now starts at 68 px, down from 122.** A layout test pins both the
> budget and the bar's inset so a future edit cannot quietly redock it.
>
> The status bar became an instrument panel rather than a second label for the
> drawer button: drawer / log entry points, save state in words, the selected
> entity, and a right-aligned mono statistics cluster fed per frame from
> `app.rs`.

- Consolidate top chrome into global, mode, and viewport-context scopes.
- Keep the viewport largest in every default workspace.
- Convert Content Browser and Output Log to on-demand drawers that can promote to persistent docks.
- Build named Layout/Terrain/Foliage/Lighting/Materials/Animation/Debug/Play workspaces with Save/Reset.
- Make title/status items measured, actionable, and resilient to compact widths.
- Persist panels, selected tabs, drawer heights, tile density, and workspace state.

**Exit:** screenshots match approved redlines; compact/ultrawide layouts pass; persistence/reset are deterministic.

**Collapse rules** are a pure `CollapseRules::for_width` table, so the whole
responsive policy is readable in one place and testable without a window: the
play-state word drops at 1400, the object count at 1280, the command-search
field at 1100. A test walks every width from 2560 down to 600 and asserts the
policy is monotone — nothing reappears as the window narrows. Note the redline
says *transport* labels drop at 1400, not the mode command names: "icon-only
controls" is a forbidden motif and the package's own 1280 screen still spells
Select / Landscape / Foliage out. An earlier pass here got that backwards.

**Workspaces** are seven named presets in `workspace.rs` — Layout, Terrain,
Foliage, Lighting, Materials, Debug, Play — in the Window menu with Reset.
Presets resolve against the live window size rather than storing absolutes, then
clamp to the redline minimums, and a test asserts the viewport stays the largest
region in every preset at 1280, 1920 and 3440 wide.

**A real defect fell out of writing those tests.** `ChromeLayout`'s default
stored a 720 px viewport as an absolute, so on a 1920 px window the Details
column took everything left over — over a thousand pixels of inspector against a
720 px scene. `ChromeLayout::resolved` now clamps the *derived* right column into
the redline's 240–520 range, and a test proves a deliberate splitter drag inside
that range still round-trips unchanged.

**Remaining:** the ☰ menu collapse at 1100 px and the ⋯ overflow for Create; the
split context bar under 1280 px; the 44 px icon rail; drawer → dock promotion as
a 200 ms height reconcile; actionable status items (click the error count to open
the log); the second right column and split bottom row at ultrawide.

### Zeta-G — Professional workflow surfaces

> **Status: Details and revert done; the browser workflows are not.**
> The Details *grammar* landed with Zeta-C/D —
> every inspector row is now a `PropertyRow` that computes the 46 % label
> column, the 14 px modified gutter, ellipsis-with-tooltip and the sub-240 px
> stacking rule from the redline instead of a per-section `label_w`. That made
> the old 34 px label column obsolete, so 72 abbreviated labels became words
> ("Rng" → "Range", "Slp°" → "Max slope", "Abs Mag" → "Absorption mag.").
> Outliner rows go through the `tree_row` recipe and pick up per-row hover and
> a `body_strong` lift on selection.
>
> **Revert now works, with no `EditorEvent` change.** `Control` grew a
> `numeric_value` accessor so `UiManager` can read all 103 inspector fields back
> out of the tree once a frame rather than mirroring values at ~100 `set_value`
> call sites and hoping none was missed. The first value a field is seen holding
> becomes its baseline; the dot lights when the live value differs; clicking the
> dot writes the baseline back through the ordinary path, which is exactly one
> `ValueChanged` and therefore one undo step. The gutter grows a ring on hover
> and a pointer cursor, and only a *lit* dot is clickable — swallowing clicks on
> an unmodified row would make the gutter feel broken.
>
> One honest caveat on semantics. The design says the dot means "differs from
> the component default". The UI layer does not know component defaults, and
> inventing them would make the dot lie, so the baseline is reset on scene load,
> save, and selection change — the dot means **"unsaved edit to this
> property"**, which is the reading that pairs with the status bar's save state.
> Promoting it to true component-default semantics needs `app.rs` to publish
> defaults per field.
>
> Favourites, filters, breadcrumbs, async previews and drag/drop are not
> started.

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

**The revert wiring is the next concrete step, and needs no `EditorEvent`
change.** `PropertyRow` already renders and accepts `SetModified`; what is
missing is a producer. `UiManager` can hold the component-default value per
`InspectorField`, mark the dot when the live value differs, and on a gutter
click send the existing `SetInspectorValue { field, value: default, live:
false }` — one undo step, no new contract. Do this before favourites or
validation; it is the cue the package leans on hardest.

### Zeta-H — Interaction, accessibility, and motion

> **Status: partial.** `Button` tracks keyboard focus and draws the 1 px ring;
> `style.rs` guarantees selection is fill *and* rail so no required state is
> colour-only; the certified contrast pairs are transcribed in §8A.3.
>
> **Tab and Esc now follow the design's annotations.** Tab / Shift+Tab walk a
> region-level focus ring in the specified order — application → mode → viewport
> context → rail → Outliner → Details → drawer → status — skipping any stop that
> is hidden, and wrapping. It is deliberately region-level, not control-level:
> the annotation gives the viewport, the Outliner and each Details section *one*
> stop each and expects arrows to traverse inside them, so tabbing through 120
> property fields would be the wrong shape. A focused text field keeps Tab for
> itself.
>
> `close_top_overlay` was reordered to the package's sequence — **modal →
> palette → popup → drawer → filter → selection** — and extended past popups to
> actually reach the last three. The modal moved to the front because it is the
> only layer that traps focus; closing something underneath it would strand the
> keyboard. Esc now also clears a live search filter and, failing that, drops
> the selection.
>
> **Remaining:** arrow-key traversal *inside* the Outliner and Details; focus
> trapping and return inside the modal; AccessKit semantic IDs, roles and
> actions; a colour-vision simulation pass; reduced-motion; and a human
> keyboard-only walk, which no headless test substitutes for.

- Complete keyboard traversal, focus trapping/return, mnemonic/shortcut display, and visible focus rings.
- Give widgets stable semantic IDs, roles, labels, values, states, and actions in preparation for/through AccessKit.
- Ensure status is not colour-only; audit contrast and colour-vision simulations.
- Enforce [WCAG 2.2 target-size guidance](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html) through size or spacing exceptions appropriate to a dense desktop editor.
- Add reduced-motion mode, text/UI scaling, and screen-reader smoke tests when AccessKit lands.
- Audit cursor shapes, tooltips, disabled reasons, drag affordances, escape/cancel, and destructive confirmations.

**Exit:** keyboard-only audit completes; contrast/scale/focus gates pass; no input leak into the viewport.

### Zeta-I — Maintainability and regression harness

> **Status: split done (2026-08-16); lints remain.** `lib.rs` went from 6,554
> lines to 3,938. Everything that only *builds* widgets moved into `editor/`,
> one module per surface: `shell.rs` (1,349), `inspector.rs` (744),
> `parts.rs` (309), `help.rs` (231), `content.rs` (189). `lib.rs` keeps
> `UiManager` — the state machine, OS-event routing and the `EditorEvent` seam —
> so a change to how a surface *looks* no longer lands in the same file as a
> change to how the editor *behaves*.
>
> **Remaining:** the token/raw-literal lint, the licence and icon-manifest
> checks, the component gallery scene and golden screenshot diffing.

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

The design gate is passed and the package is vendored, so this list is no
longer "when the design arrives". A session picking Zeta up should:

1. Read the status table at the top of this document, then §8A, then
   [`phase_26.md`](phase_26.md) §14 (must-not-break) and `context.md` §8.
2. **Look at the editor before changing it.** Build and run:

   ```bash
   SOMNIUM_CAPTURE_UI_PNG="dev records/phase 26/zeta_shell_after.png" SOMNIUM_CAPTURE_FRAME=120 SOMNIUM_CAPTURE_QUIT=1 SOMNIUM_TERRAIN=1 cargo run -p hello_engine
   ```

   That is the only capture that includes chrome. Most of the remaining Zeta
   work is visible in it; none of it is visible in a `cargo test` run.
3. Do **not** restart at Zeta-B, C or D — they are in tree. Do not re-derive
   the token values; `theme.rs` is a transcription of
   `design/assets/tokens/nocturne.tokens.json` and the two must stay equal.
4. Take the next step from the "Remaining" block of whichever sub-phase you are
   in. If choosing freely, the ranked order by visible payoff is now:
   **(a)** the Content Browser workflows in Zeta-G — filters, breadcrumb
   history, async previews and drag-to-spawn are the largest surface still
   showing placeholder behaviour; **(b)** the remaining Zeta-F collapse steps
   (☰ at 1100 px, the split context bar at 1280 px, the 44 px icon rail);
   **(c)** Zeta-H's arrow-key traversal inside the Outliner and Details plus
   modal focus trapping; **(d)** Zeta-I's raw-literal and licence lints, which
   are what stop the token layer drifting back into decoration.
5. **Run the five `MANUAL_ONLY` checks in `mod must_not_break`.** They are the
   part of the `phase_26.md` §14 inventory no headless test can cover, and they
   have not been performed.
6. Keep the engine runnable and the Phase 26 must-not-break matrix green after
   every sub-phase, and re-capture rather than describing the result.
7. Add a dated §15 ledger entry that says what is *not* done as plainly as what
   is.

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

### 2026-08-16 — typography, recipes, Details grammar, floating context scope

Implemented against the vendored design package, with no `EditorEvent` change
and no renderer/gameplay change beyond one new capture hook.

**Evidence tooling first.** `SOMNIUM_CAPTURE_UI_PNG` copies the swapchain
*after* `UiManager::end_frame`, so the project can, for the first time, produce
a real screenshot of its own editor chrome. The pre-existing
`SOMNIUM_CAPTURE_DISPLAY_PNG` runs before the UI pass, which is exactly why the
2026-08-15 entry above had to disclaim its smoke PNG. Both share one readback
helper. Current capture: `dev records/phase 26/zeta_shell_after.png`.

**Zeta-D — typography.** Five bundled cuts replace the single Inter Regular:
Inter Regular / Medium / SemiBold and JetBrains Mono Regular / Medium, all SIL
OFL 1.1, subset from official upstreams with `fontTools`, licences and
modifications recorded in `THIRD_PARTY_NOTICES.md`. New `typography.rs` carries
`FontRole` (resolved once into a process-wide `FontRegistry`, with any cut that
fails to load aliased onto one that did) and the ten `TextRole`s from the token
sheet. `TextBuilder::with_role` applies size, face, colour, tracking and case in
one call; `push_text_tracked` / `measure_text_tracked` give the uppercase header
role letter-spacing that measures what it draws. Numeric fields default to
`mono_strong`, which is how the `tnum` requirement is satisfied under a
rasteriser that applies no OpenType features.

**Zeta-C — recipes.** `theme.rs` gained the remainder of the token sheet
(`body_strong`, `mono_strong`, `gap_section`, `radius_tile`, the opacity ladder,
the elevation table, `MOON`, and `with_alpha` / `scaled_alpha` / `flatten`).
New `style.rs` turns component + `VisualState` into `Paint` for button, primary
button, icon button, input, tree row, asset tile, drop target, popup and status.
`Button` (which now also tracks keyboard focus and draws the ring),
`NumericField` and `TreeView` consume it. Seven tests pin the grammar — notably
that every selected recipe emits the rail, so selection is never colour-only,
and that hover changes the wash and not the outline.

**Zeta-F — shell.** The viewport context scope was reparented out of the outer
grid and into the viewport as a translucent 32 px bar inset 12 px, taking the
pre-scene budget from 100 px to **68**; its grid row stays at index 3 at zero
height so every `GridMessage` row index is unchanged. Save and the three editing
modes carry labels and are grouped by hairline separators. The status bar became
a two-cluster instrument panel — drawer/log, save state in words, selection —
with a right-aligned mono statistics readout fed from `app.rs`. Details minimum
width went from 180 to the redline's 240.

**Zeta-G foundation.** `widgets/property_row.rs` implements the Details column
grammar as a measured widget: 14 px modified gutter, 46 %-of-width label column
clamped 96–176, ellipsis with a permanent full-text tooltip, and the sub-240 px
stack-and-grow rule. Every inspector row and the shared `build_property_row`
funnel go through it. Because the label column stopped being 34 px, 72
abbreviated labels became words — `Rng` → `Range`, `Slp°` → `Max slope`,
`Abs Mag` → `Absorption mag.`, `AO Rad` → `AO radius`. Section headers are now
26 px `surface.header` bands in tracked SemiBold caps.

Verification: `cargo test -p somnium_ui` passes 56/56 (was 39); the serial
`cargo test --workspace -j 1` gate passes; `cargo fmt --all` and
`cargo check --workspace` are clean; a live `hello_engine` run captures the
editor with chrome and exits cleanly.

Still open and not represented as complete: Zeta-E's `resvg` icon pipeline and
the monogram route, Zeta-F's collapse rules and workspaces, all of Zeta-G's
workflow surfaces including revert, Zeta-H's keyboard traversal and AccessKit,
Zeta-I's `lib.rs` split and lints, and Zeta-J. The Phase 26 must-not-break
inventory has not been re-walked interactively since this pass and remains the
controlling gate.

### 2026-08-16 (second pass) — icons, revert, workspaces, keyboard, the split

Closing the list the first pass left open. No `EditorEvent` variant was added
or changed; `InspectorField` gained `Hash` so it can key a map, which is not a
contract change.

**Zeta-E — one icon family, finally.** Route **A (Eclipse)** is chosen, so the
brand SVG is now the runtime engine mark. 67 Tabler outline icons (MIT) are
vendored one-per-`IconId` and renamed to the id they serve; the sixteen original
Somnium icons sit beside them on the same grid. `resvg` (default features off)
rasterizes all 83 at startup into the existing 32 px atlas cells, keeping only
alpha so the shader still tints by semantic colour. Icons without a source keep
their procedural fallback. A test renders every source and fails any that parses
but marks fewer than a dozen pixels.

**Zeta-G — revert.** `Control::numeric_value` lets `UiManager` read all 103
inspector fields back out of the tree once a frame, rather than mirroring values
at ~100 `set_value` call sites. First observation becomes the baseline; the dot
lights on difference; clicking it writes the baseline back through the ordinary
value path — one `ValueChanged`, one undo step. Baselines reset on scene load,
save and selection change, so the dot honestly means "unsaved edit to this
property" rather than claiming component-default semantics the UI layer cannot
know.

**Zeta-F — collapse rules and workspaces.** `CollapseRules::for_width` holds the
whole responsive policy as one pure table; a test walks 2560 → 600 px and proves
it is monotone. Seven named workspaces in `workspace.rs` reach the Window menu
with Reset, resolving against the live window size and clamping to the redline
minimums. Writing their tests surfaced a real defect: `ChromeLayout`'s default
stored a 720 px viewport as an absolute, so a 1920 px window gave the Details
column over a thousand pixels and the scene 720. `ChromeLayout::resolved` now
clamps the derived right column into the redline's 240–520 range while letting a
deliberate drag inside that range round-trip unchanged.

An earlier version of the collapse table also had the 1400 px rule backwards —
it dropped the *mode command* names rather than the transport's play-state word.
The package forbids icon-only controls and its own 1280 screen spells the mode
commands out; the table now matches.

**Zeta-H — Tab and Esc.** Tab / Shift+Tab walk a region-level focus ring in the
annotated order, skipping hidden stops and wrapping. `close_top_overlay` was
reordered to modal → palette → popup → drawer → filter → selection and extended
to actually reach the last three; the modal moved to the front because it is the
only layer that traps focus.

**Zeta-I — the split.** `lib.rs` 6,554 → 3,938 lines. Construction code moved to
`editor/{shell,inspector,parts,help,content}.rs`; `lib.rs` keeps `UiManager`.

**Zeta-J — automated §14 coverage.** A `must_not_break` test module asserts that
every `CreateKind` still has a Create row, that the transport / mode / file /
edit / profiler / drawer / log / help controls all still exist and are sized,
that the terrain palette still arms tools 0–5 in order, and that the viewport
stays the largest region at 1280 / 1920 / 2560 / 3440. Five inventory items
genuinely need a human at the keyboard — fly-cam feel, gizmo drag, terrain
sculpting, foliage painting, and viewport input passthrough — and are listed in
`MANUAL_ONLY` so the gap is countable rather than invisible.

Verification: `cargo test -p somnium_ui` 74/74 (was 56); serial
`cargo test --workspace -j 1` green; `cargo fmt --all` and
`cargo clippy -p somnium_ui` clean; a live `hello_engine` run captures the editor
with chrome and exits without a wgpu validation error.

**Still open.** DPI-aware icon atlas regeneration; text shaping and bidi
(`cosmic-text`); the ☰ / ⋯ collapse steps and the split context bar; Content
Browser workflows — filters, breadcrumb history, async previews, drag-to-spawn;
arrow-key traversal inside the Outliner and Details, modal focus trapping, and
AccessKit; the raw-literal and licence lints; the component gallery and golden
screenshot diffing; and the before/after evidence package at every target size
and scale. The five `MANUAL_ONLY` interaction checks have not been performed —
they need the user at the keyboard, and this document does not claim otherwise.

### 2026-08-16 (third pass) — polish from live review

Four items from looking at a real capture, plus one defect the audit exposed.

**Content Browser tiles were pixelated.** They draw at `ICON_DRAWER` = 80 px
from a 32 px atlas cell — a 2.5× upscale that no filtering recovers. The atlas
now carries **two cuts** of every glyph: the 32 px block for chrome (16/20/24 px,
where 32 → 16 is an exact 2:1 box filter) and a 96 px block below it for anything
drawn larger. `IconId::draw_quad` picks from the destination rect it already
receives, so no call site had to opt in. The atlas grew from 512² to 1024²
(4 MB); tests pin that the two blocks never overlap, that a 16 px row and an
80 px tile sample different cells, and that the large block actually carries
coverage — an allocated-but-unfilled block would show as *invisible* tiles
rather than pixelated ones.

**Chrome labels sat a pixel or two high.** `labeled_icon_button` and the sculpt
rail computed a top margin from an assumed 14 px line height. Inter's line box is
1.21 em, so the assumption was wrong for the Zeta type roles — and it was going
to be wrong again for any future role. Those margins are gone; the glyph and the
word both take `VerticalAlignment::Center`, which the arrange pass already
supported. The same fix applies to the title bar, the menu labels, the play-state
word and the status bar.

**The engine mark was too small.** `ICON_MARK` was aliased to `icon_action`
(24 px), so the brand read as one more toolbar glyph beside the wordmark. It is
now its own 30 px token, centred in the 36 px application band, with a comment
saying why it is not `icon_action` — a brand element is not a control.

**Details toggles and editables: audited, no dead wiring found.** All 37 toggle
handles and all 103 numeric bindings are referenced in `process_outgoing`; every
`InspectorField`, `PostFxToggle` and `ColorField` variant is handled in `app.rs`;
`CheckBoxMessage` separates `SetChecked` (engine → widget) from `Check` (widget →
engine), so an inspector refresh cannot re-fire a toggle. A new test arranges the
whole inspector — including the rows that are hidden until a light type or
foliage mode reveals them — and fails any control smaller than 8 × 8, because a
zero-sized control still draws its label and silently swallows clicks. It covers
the combos, checkboxes and colour swatches as well as the numeric fields.

The likely cause of what was observed is the fifth item:

**Defect — a persisted layout could not survive a change of window size.**
`ChromeLayout` stored the *viewport* width as an absolute. A file written on a
wide window held `viewport: 2040`, which on a 1280 px window derives a negative
Details column, clamps to the 240 px minimum, and pushes every property row
below the 240 px stacking threshold — so the value controls stacked and clipped,
which reads exactly like "the editable doesn't work". `ChromeLayout` now stores
`details`, the value that actually transfers between window sizes, with the
viewport derived from it; `serde(default)` keeps old files loading. A stored
column outside the 240–520 range is treated as belonging to a different window
and falls back to the shipped 340 default rather than pinning to a boundary the
user never chose. Splitter drags record the column, and three tests cover the
round-trip, the cross-monitor transfer and the legacy-file case.

Verification: `cargo test -p somnium_ui` 80/80; serial `cargo test --workspace
-j 1` green; `cargo fmt --all` and `cargo clippy -p somnium_ui --all-targets`
clean; live capture re-taken.

### 2026-08-16 (documentation pass)

Every living document now describes the editor that is actually in tree.

- **`README.md`** leads with the Eclipse lockup (dark/light via `<picture>`),
  the identity sentence, and a **real screenshot** — `media/editor.png`, captured
  from a running build rather than mocked up. A new *Editor design system* block
  lists what Nocturne Atelier actually provides; the licence section now names
  the bundled fonts and icons and states plainly that the Somnium name, the
  Eclipse mark and the engine-specific icons are **not** covered by the dual
  licence and have not been through trademark clearance.
- **The README lockups are generated, not hand-edited.** The brand sheet requires
  outlines rather than live text, and GitHub renders README SVGs through `<img>`,
  where Inter is unavailable and a `<text>` element would fall back to the
  reader's `system-ui` with unapproved metrics. `fontTools` converts the wordmark
  from the bundled Inter cuts to path data; the mark stays `#7A86FF`.
- **`context.md`** §8 now shows the floating context bar in the shell diagram and
  the 68 px scene origin, lists the new modules (`style`, `typography`,
  `workspace`, `icon_svg`, `editor/`), documents the collapse rules, the
  workspaces, the column-based layout persistence and the Tab/Esc contract, and
  records `SOMNIUM_CAPTURE_UI_PNG` as the only capture that shows chrome. The
  roadmap row and both phase summaries are rewritten.
- **`ATTRIBUTION.md`** §1.4 gains the rows where Somnium *diverges* from Unreal —
  a floating viewport bar, a gutter-dot revert whose baseline is honestly the
  last save rather than a component default, and Blender-style workspaces. §1.5
  replaces "no third-party SVG is vendored" with the Tabler + resvg reality and
  tabulates the five bundled font cuts.
- **`THIRD_PARTY_NOTICES.md`** covers Inter, JetBrains Mono, Tabler and resvg
  with upstream, version, files, modification and retrieval date.
- **Help pages** (`docs/editor/`): Shortcuts documents Tab traversal, the Esc
  layer order, the revert dot and workspaces; About is rewritten for the real
  type and icon story; Welcome explains the three command scopes; Outliner
  explains why selection is fill *and* rail.
- **`media/README.md`** records the capture command so screenshots stay
  reproducible.

**A defect fixed along the way.** `encode_png` emitted *stored* deflate blocks —
valid PNG, no compression — so every capture was `width × height × 3` on disk and
a 1280×720 screenshot cost 2.7 MB. It now uses the `image` encoder that was
already in the renderer's dependency tree, with the hand-rolled writer kept as a
fallback so a failure loses size rather than the capture. Same image, 448 KB.

### 2026-08-16 (lockup geometry)

The README lockup looked off-centre inside a `<p align="center">` that was in
fact centring it correctly. The cause was inside the SVG: a hand-set 250 × 64
viewBox around artwork that only occupied x 15–154, y 18–49. Roughly 40 % of the
width and half the height were empty, so the *box* centred while the visible
artwork sat about 115 px to the left at the chosen width — and the logo read as
small for the space it took.

`tools/build_lockup.py` now measures the viewBox from the artwork's own bounds,
with clear space set to one blade stroke (9.2 × 0.76 = 7.0 units) on all four
sides as the brand sheet specifies. The box is 153.08 × 45.21 and the aspect
moves from 3.91:1 to 3.39:1.

The generator moved out of a scratch directory and into `tools/` because
`PROVENANCE.md` instructs future maintainers to regenerate rather than hand-edit,
and an instruction to run a script that does not exist is not an instruction.
README width dropped 720 → 440, which renders the same artwork size as before
with the dead space gone.
