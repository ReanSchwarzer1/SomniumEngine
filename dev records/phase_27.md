# Phase 27 — Hades

> *"Every surface is lit. Every state has a face. Nothing is anonymous."*
> Phase 26 gave Somnium a professional **information architecture**. Nocturne
> Atelier gave it a professional **token layer**. Neither gave it a professional
> **surface** — because the rasterizer underneath cannot draw one. Hades rebuilds
> the paint layer, then repaints the shell on top of it.

> **Codename:** Hades (Supergiant Games, 2020). Chosen because Hades is the
> reference for a UI that is simultaneously *dense* and *beautiful*: every panel
> is lit and layered rather than flat-filled, motion is causal rather than
> decorative, the palette runs two temperatures (cool ground, warm signal), and
> the whole thing reads at a glance under heavy information load. That is exactly
> Somnium's problem statement. **Zero Supergiant art, fonts, motion curves,
> layout, or assets are copied.** The discipline only — the same contract Phase 26
> holds with Atlus.
> **Status:** Plan date 2026-08-18. **27-A, 27-B, 27-C, 27-E, most of 27-D and
> the DPI-correctness fix landed 2026-08-18**; **27-F partially landed** (the
> icon atlas now rasterizes at device resolution). The widget tree lays out in
> logical units, and the visible chrome — buttons, inputs, search fields,
> sliders, toasts and panel surfaces — now renders through `push_paint`, so the
> radius, chrome wash, elevation and focus glow the recipes describe actually
> reach the screen. **All 18 widgets are migrated** and the scroll-edge fade
> is wired. Measured on the real 1920x1080 shell: **56 rounded, 29 washed,
> 21 lifted, 5 recessed, 17 stroked** of 646 instances.
> 80 pre-existing tests pass unedited; suite at **181 green**.
> **27-G partially landed:** empty states, Search Everywhere and Content
> Drawer type badges. **The project picker is blocked** on an `EditorEvent`
> addition that §12.1 forbids; engine-rendered thumbnails and the remaining
> browser workflows are not started.
> **Still open:** the 27-D backdrop blur (needs `COPY_SRC`, which the surface
> only conditionally supports); 27-F's monogram route, optical ladder, `.ico`
> and splash; `cosmic-text`; the Geist gate; and all visual evidence.
> 27-H through 27-J not started. See §18.
> **Project:** Somnium Engine
> **Target:** Rust 1.85 docs / 1.88 effective, wgpu 29, winit 0.30
> **Depends on:** Phase 26-A–I and 26-Zeta-B–I in tree. No renderer, terrain,
> physics, or ECS dependency. Independent of Phase DF and Phase VV.
> **Supersedes nothing.** [`phase_26.md`](phase_26.md) remains the chrome
> *contract* (§3 constraints, §14 must-not-break).
> [`phase_26_Zeta.md`](phase_26_Zeta.md) remains the *identity* contract
> (§5 Nocturne, §6 token architecture, §8A redlines). This file is the
> **paint and craft** contract and inherits both.
> **On completion this file rewrites `phase_26.md`** — see §16.

---

## 0. How to use this document (handoff)

**Read in this order before writing code:**

1. **This file**, all of it. Especially §2 (the audit that motivates the phase),
   §3 (the framework decision — do not relitigate it without reading §3.6),
   §6 (the render contract), §9 (sub-phases), §12 (must-not-break), §14 (start
   checklist).
2. [`phase_26.md`](phase_26.md) §2.4 Nocturne, §3 constraints, §14 must-not-break.
3. [`phase_26_Zeta.md`](phase_26_Zeta.md) §5 identity, §6 token architecture,
   §8A.2 redlines, §8A.3 certified contrast pairs, §8A.4 the four-cue state
   grammar, §8A.6 decisions still owed.
4. [`context.md`](../context.md) §8 (`somnium_ui`), roadmap row 26.
5. [`ATTRIBUTION.md`](../ATTRIBUTION.md) §1.4–1.5, §13.13–13.18.
6. `crates/somnium_ui/src/draw.rs` and `src/ui_pass.wgsl` — **the two files this
   phase exists to replace.** Read them before anything else in the crate.
7. `crates/somnium_ui/src/theme.rs`, `style.rs`, `typography.rs` — the token,
   recipe and type layers that already exist and are **kept**.

**Authorized work:** `somnium_ui` paint, motion, typography, iconography, brand
assets, and the editor surfaces built on them. Plus **one** new UI-owned GPU pass
(§9.4 backdrop blur) that reads the swapchain. Renderer, terrain, water optics,
lighting, physics, ECS, and `EditorEvent` stay frozen except:

- inspector bindings to fields that already exist, and
- an **asset thumbnail render service** (§9.7-2) that calls the existing renderer
  through a narrow request/response API and adds no renderer feature.

`WaterComponent::great_lakes` stays frozen. Do not reintroduce per-pixel terrain
sample-count LOD. Do not touch `EditorEvent` variants.

---

## 1. Executive decision

The editor's information architecture is good. Its **surface** is not, and the
reason is mechanical, not aesthetic.

`DrawingContext` exposes exactly these primitives: `push_rect_filled`,
`push_rect_border`, `push_text`, `push_text_tracked`, `push_textured_rect`,
`push_nine_slice`, plus `push_drop_shadow`, which is six concentric hard-edged
translucent rectangles standing in for a blur. `ui_pass.wgsl` is a ~50-line
sample-and-tint shader with one texture binding and no geometry evaluation.

That primitive set cannot express a corner radius, an antialiased edge, a
gradient, a glow, a real shadow, a stroke, or a rounded clip. So the design
system above it is writing cheques the paint layer cannot cash: `theme.rs`
declares five radius tokens, `style.rs` carries `Paint::radius` through five
recipes, and **zero pixels are ever rounded.** `MotionTokens` declares four
durations; only `tooltip_delay_ms` is read, because there is no animation clock
in the crate at all.

Hades does four things, in this order:

1. **Rebuild the paint layer** (§9.1–9.4) — one instanced primitive-quad
   pipeline with analytic SDF evaluation, giving radius, antialiasing,
   gradients, borders, real shadows, glow, and inner shadow in a single
   pipeline family; real text rasterization quality; an animation driver; and
   a backdrop-blur pass for scrims and floating chrome.
2. **Expand the token layer** (§9.5) onto the new capabilities — elevation
   ladder, accent ramp, warm secondary, gradient and glow tokens, and a second
   complete theme (Dawn) to prove nothing is hard-coded.
3. **Finish the identity** (§9.6) — monogram route, DPI-correct icon atlas,
   optical ladder, splash, application icon.
4. **Build the surfaces a user sees first** (§9.7) — project picker, real
   Content Browser with engine-rendered thumbnails, viewport overlays, empty
   states, Search Everywhere.

Then close Phase 26's open interaction and harness debt (§9.8–9.9) and sign off
(§9.10).

Strategy is **strangler fig again**, and this is what makes the phase safe:
`push_rect_filled(rect, color)` stays as a thin wrapper that emits a primitive
quad with `radius = 0, border = 0`. **No widget call site changes on day one.**
Beauty is opted into recipe by recipe.

---

## 2. Audit as of 2026-08-18

Evidence: `dev records/phase 26/zeta_shell_after.png` (the current shell),
`crates/somnium_ui/src/` (21,394 lines, 80 tests green),
`crates/somnium_ui/assets/tokens/nocturne.tokens.json`.

### 2.1 What is already right — keep all of it

| Asset | Where | Verdict |
|---|---|---|
| Retained widget tree, generational `Handle`/`Pool`, bubbling `UiMessage`, two-pass measure/arrange | `ui.rs`, `pool.rs`, `node.rs` | Sound. Keep. |
| Correct colour contract — authored sRGB decoded to linear exactly once, straight alpha | `color.rs`, `ui_pass.wgsl`, Zeta-B | **Hard-won. Must survive §9.1 verbatim.** |
| Semantic token snapshot | `theme.rs` `NOCTURNE` | Keep, extend. |
| Component style recipes (component + `VisualState` → `Paint`) | `style.rs` | Keep. This is the correct seam for the new paint. |
| Five OFL cuts behind `FontRole`/`TextRole` | `typography.rs` | Keep. |
| `PropertyRow` measured grammar, revert gutter | `widgets/property_row.rs` | Keep. |
| Seven named workspaces | `workspace.rs` | Keep. |
| `EditorEvent` seam with `app.rs` | `editor_event.rs` | **Frozen.** |
| 27 widgets, `editor/` split, 80 tests | crate-wide | Keep. |

### 2.2 P0 finding — the token layer is writing cheques the paint layer cannot cash

```
theme.rs      GeometryTokens { radius_input, radius_chrome, radius_popup,
                               radius_modal, radius_tile }        <- declared
style.rs      Paint { radius: f32 }        (5 recipes set it)     <- plumbed
widgets/*.rs  grep '\.radius' -> 0 hits                           <- never drawn
draw.rs       no rounded-rect primitive exists                    <- cannot be drawn
```

The same shape holds for motion. `MotionTokens { press_ms: 90, hover_ms: 120,
popup_ms: 140, drawer_ms: 200 }` is declared, and the only consumer in the crate
is `TOOLTIP_DELAY_MS`. There is no frame delta, no easing function, no
interpolation, and no per-node animation state anywhere in `somnium_ui`.

This is why the shell reads as competent-but-flat. It is not a taste problem, and
no amount of retokenising fixes it.

### 2.3 Paint-layer capability gaps, ranked by visual impact

| # | Missing | Cost today | Impact |
|---|---|---|---|
| 1 | Antialiased edges | every edge is a hard pixel step | **Highest.** Single largest "hobby tool" tell. |
| 2 | Corner radius | tokens dead | High. 2–6 px is all that is needed; see §5.2. |
| 3 | Real shadows | 6 hard rings, visibly banded at drawer/modal spreads | High. Kills the elevation story. |
| 4 | Gradients | none | High. Flat fills at 12 surface tokens read as grey soup. |
| 5 | Motion | none | High. Every state change is an instant pop. |
| 6 | Text quality | fontdue, 1.5× supersample, no shaping, no subpixel X positioning, no gamma-correct blend | High. Light-on-dark text is anemic without stem compensation. |
| 7 | Glow / focus bloom | none | Medium. Focus ring is a 1 px hairline only. |
| 8 | Backdrop blur | none | Medium. Modal scrim is a flat 62 % black; the floating context bar has no separation from the render. |
| 9 | Inner shadow / inset | none | Medium. Input fields have no recession. |
| 10 | DPI-correct icons | fixed 32 px / 96 px atlas cells | Medium. Blurry at 150 % / 200 %. Open Zeta item. |
| 11 | Rounded clipping | scissor rect only | Low-medium. Blocks rounded thumbnails and rounded scroll regions. |

### 2.4 Screenshot critique — `zeta_shell_after.png`

Read honestly, worst first:

1. **Every rectangle is hard-edged and flat-filled.** Twelve surface greys
   separated only by 1 px hairlines. Nothing is lit; nothing recedes.
2. **The Content Drawer is the weakest surface in the product.** Seven identical
   80 px gold folder glyphs and two file glyphs. No thumbnails, no metadata, no
   badges, no density control, no breadcrumb. This is the surface a new user
   opens first after the viewport.
3. **Sliders are two flat bars.** No track recession, no thumb, no fill gradient,
   no hover state.
4. **The Details panel is cut off mid-row** with no fade, no scroll shadow, and
   no indication that content continues.
5. **The floating viewport context bar has no separation from the render** — it
   is a flat translucent slab. This is the one place a backdrop blur is
   unambiguously correct.
6. **No entry surface exists.** The editor opens straight into a scene. There is
   no project picker, no splash, no branded first frame. The single highest-
   leverage "wow" surface in the product does not exist.
7. **Icons are uniform monochrome outline at one weight.** Active/selected state
   is carried by fill colour alone in several places.
8. **Numeric field values render at `0.000` in JetBrains Mono against a flat
   input fill** with no recession — correct typographically, unfinished visually.
9. Positives, to be preserved: the 68 px pre-scene budget is genuinely tight and
   professional; the three command scopes are legible; the sculpt rail, outliner
   type icons, and status bar all read correctly; type hierarchy works.

### 2.5 Inherited open items from Phase 26

Carried into this phase rather than left dangling:

- Zeta §8A.6: monogram route unpicked; optical ladder, `.ico`, splash lockup
  uncut; Tabler manifest partly vendored; visual-regression sheet not produced.
- Zeta-G: Content Browser workflows (breadcrumbs, filters, density, async
  previews, drag/drop) not started; Details favourites/units/validation not
  started.
- Zeta-H: arrow traversal inside Outliner/Details; modal focus trap and return;
  AccessKit; colour-vision pass; reduced-motion; human keyboard-only walk.
- Zeta-I: token/raw-literal lint; licence and icon-manifest checks; component
  gallery scene; golden screenshot diffing.
- 26-H: `cosmic-text` shaping and bidi.
- 26-J: reflection-driven inspector. **Stays out** unless separately requested.

---

## 3. Framework decision — evaluated, recorded, closed

Four external toolkits were proposed. All four were evaluated against the actual
constraint set. **The recommendation is to keep and rebuild `somnium_ui`.** This
section records why, so the decision is not relitigated from vibes later.

### 3.0 The constraints any candidate must satisfy

| # | Constraint | Source |
|---|---|---|
| C1 | Composites into Somnium's existing wgpu swapchain, over the 3D viewport, in the same frame | `context.md` §8.1, `UiPass` `LoadOp::Load` |
| C2 | License-compatible with MIT **and** Apache-2.0, and imposes **no** obligation on downstream games built with the engine | `LICENSE-MIT`, `LICENSE-APACHE` |
| C3 | Windows-first, DPI-correct; Linux/macOS not blocked | `rust-toolchain.toml`, dev platform |
| C4 | Supports live-scrub `ValueChanging` / commit `ValueChanged` on 103 inspector fields without a frame of IPC latency | `widgets/numeric_field.rs`, `editor_event.rs` |
| C5 | Preserves the `EditorEvent` seam with `app.rs` unchanged | `phase_26.md` §14 |
| C6 | Lets a **game** built on Somnium draw its own UI with the same toolkit | `phase_26.md` §1 item 3, `runtime.rs` `UiCanvas` |
| C7 | Does not regress the Zeta-B single-sRGB-decode colour contract | `phase_26_Zeta.md` §2.2 |

### 3.1 Tauri v2 — rejected

WebView-based. Phase 12C **explicitly deleted** `wry`, `editor.html`,
`IpcMessage`, and the 400-line `handle_ipc_command` dispatcher (`context.md`
roadmap 12C). Reintroducing it fails C1 (the viewport must live under a
transparent WebView — fragile and platform-specific on Windows, with no reliable
per-pixel input passthrough), fails C4 (IPC on every scrub frame), and fails C6
(a shipped game cannot carry a WebView runtime). It also adds a Node toolchain to
a Rust engine. This would be a documented regression, not an upgrade.

### 3.2 GPUI — rejected as the shell, adopted as a technique reference

Apache-2.0, so C2 is fine. But GPUI owns its own window and its own renderer
(blade-graphics, not wgpu), which fails C1 outright — there is no supported path
to composite GPUI over an existing wgpu frame. It is pre-1.0 with frequent
breaking changes, and the standalone `gpui` crate still documents macOS and Linux
as its supported platforms, which fails C3.

**Adopt the idea, not the dependency.** GPUI's core rendering insight — one
instanced quad primitive carrying rect, corner radii, border, background and
shadow, evaluated analytically in the fragment shader — is exactly the technique
§9.1 implements. That is a published approach, not GPUI's property.

### 3.3 Slint — rejected on licensing, and it is a genuine loss

Technically the strongest candidate. `slint::wgpu_2x` plus
`Window::set_rendering_notifier()` really does support rendering Slint as an
underlay or overlay in a wgpu application, and `slint::Image::try_from<wgpu::Texture>()`
lets an externally rendered viewport become a Slint image. C1 is satisfiable.

**C2 is not.** Slint is offered under GPLv3, a royalty-free license, or a paid
commercial license. Somnium is dual MIT + Apache-2.0. Linking Slint means either
(a) the engine becomes GPLv3, which relicenses the project and every downstream
game, or (b) every downstream developer inherits Slint's royalty-free terms —
which require an attribution disclosure, exclude embedded targets, and put a
per-device fee on the commercial path. **An engine cannot impose licensing terms
on the games built with it.** That is disqualifying regardless of how good the
toolkit is.

Slint remains legitimate as an *internal, non-shipped* prototyping tool if fast
screen mockups are ever wanted. It cannot be in the shipped dependency graph.

### 3.4 Iced + libcosmic — rejected on cost/benefit, and libcosmic on platform

The only license-clean pair: Iced is MIT, libcosmic is MPL-2.0. `iced_wgpu`
genuinely supports manual `Engine` construction over a caller-owned `Device` and
`Queue`, so C1 and C2 both pass for Iced alone.

It fails on everything else:

- **libcosmic is a COSMIC-desktop application framework** — Wayland/Linux-first,
  wired to `cosmic-config` and panel/applet integration. On a Windows-first
  engine editor it is the wrong platform target (C3).
- Adopting Iced means replacing the retained tree, the `Pool`/`Handle` arena, the
  `UiMessage` bus, all 27 widgets, `PropertyRow`, the workspaces, and the
  `UiManager` state machine — **rewriting 21,394 tested lines to reach parity
  before gaining one pixel of beauty.**
- Iced's Elm architecture fights C4: a full-tree `view()` rebuild per message is
  the wrong shape for 103 live-scrubbed fields that must distinguish
  `ValueChanging` from `ValueChanged`.

**Adopt COSMIC's *theming concepts*, not the library.** Its container /
component / on-colour nesting — where each nested container derives its own
surface, component and text colours rather than picking greys from a flat list —
is a better model than Nocturne's flat 12-surface list, and §9.5 adopts it.
Configurable corner radii and its frosted-glass treatment inform §9.5 and §9.4.

### 3.5 The decision

**Keep `somnium_ui`. Rebuild its paint layer.** The framework is not the problem,
and every replacement costs a full rewrite plus, in three of four cases, a
blocking licence or platform failure. Somnium already owns the hard parts — the
retained tree, the layout algorithm, the message bus, the token layer, the
`EditorEvent` seam, and correct colour. What it lacks is roughly 600 lines of
shader and an animation clock.

**Ideas imported (technique references, zero code, zero dependency):**

| From | Idea adopted | Where |
|---|---|---|
| GPUI / Zed | Instanced primitive quad with analytic SDF evaluation | §9.1 |
| Inigo Quilez | `sdRoundedBox` signed-distance primitive | §9.1 |
| Evan Wallace | Fast analytic Gaussian approximation for rounded-rect shadow | §9.1 |
| COSMIC | Container/component/on-colour nesting; configurable radii; frosted glass | §9.4, §9.5 |
| Fluent 2 | Elevation ladder semantics | §9.5 |
| Blender / Godot / VS Code / JetBrains | Already benchmarked in Zeta §4; unchanged | — |

Every one of these lands in `ATTRIBUTION.md` as a technique citation **before**
the code that uses it is merged (§11).

### 3.6 What would reopen this decision

Only these. Nothing else.

- Slint relicensing its desktop path under MIT/Apache or an equivalent
  no-downstream-obligation permissive licence.
- GPUI shipping a stable, Windows-supported, wgpu-compositable standalone crate.
- A measured finding that §9.1's pipeline cannot hold the §10.6 performance
  budget after two optimisation attempts.

---

## 4. What Hades actually teaches

Supergiant's Hades is studied for **discipline**, exactly as Phase 26 studies
Atlus. Four transferable lessons:

1. **Every surface is lit, not filled.** Panels carry a subtle luminance gradient
   and a distinct edge, so depth is read before any border is parsed. Somnium's
   translation: a 2–4 % vertical wash on chrome bars and panel headers in
   **linear space**, plus an elevation ladder — not decorative gloss.
2. **Two temperatures.** A cool ground and a warm signal, so composition exists
   before colour meaning does. Somnium already has both halves and has not
   noticed: lunar indigo `#7A86FF` and the orphaned `folder: #C4A574`. §9.5
   promotes the warm value into a full secondary role.
3. **Motion is causal.** Things move because something happened, along the axis
   the causing action implies. Nothing loops, nothing breathes, nothing
   decorates. This matches Zeta §5.1 exactly and is the constraint on §9.3.
4. **Density and beauty are not in tension.** Hades shows dense stat blocks and
   still reads instantly, because hierarchy is carried by *depth and weight*, not
   by whitespace. Somnium must not solve beauty by adding padding — the 68 px
   pre-scene budget is a feature and §12 protects it.

### What must not be copied

Supergiant art, typefaces, colour scripts, frame ornaments, motion curves, sound,
layout, or any asset. Hand-painted illustrative chrome. Warm-gold-dominant
palettes — Somnium's ground stays nocturnal indigo-black. Ornamental borders. Any
UI that would make a screenshot resemble a Supergiant product.

---

## 5. Design principles for this phase

These extend Zeta §4's ten interface principles; they do not replace them.

1. **Depth over decoration.** New capability is spent on making hierarchy
   readable — elevation, recession, separation — never on ornament.
2. **Radius stays small.** Tokens remain 2 / 2 / 4 / 6 / tile. The beauty comes
   from antialiasing, depth, gradient and type. **Explicitly forbidden:** large
   uniform corner radii, card-grid layouts, oversized padding, heavy layered
   shadows on every surface, and purple-gradient hero treatments. A professional
   tool is not a landing page.
3. **Gradients are 2–6 %, in linear space, on chrome only.** Panels, headers,
   toolbars, primary buttons, the selection rail. Never on body content, never on
   inputs, never on text.
4. **Glow is focus and active state only.** One outer glow on the focus ring, one
   faint bloom on the armed mode button. Nothing else glows, ever.
5. **Motion is causal, ≤200 ms, and interruptible.** Reduced-motion disables all
   of it without changing layout.
6. **Idle frames are free.** A shell with no hover, no animation and no scrub must
   produce a byte-identical draw list two frames running. §10.3 asserts it.
7. **Every new visual is a token.** No raw literal reaches a widget. §9.9's lint
   enforces it mechanically.
8. **State is never colour-only** — Zeta §8A.4's four cues (hover wash, 1 px focus
   ring, 2 px selection rail + translucent fill, gutter dot) still bind.
9. **Two themes from day one.** Dawn (light) ships with Nocturne. A design system
   with one theme has untested seams.

---

## 6. The render contract (normative)

This section is the specification §9.1 implements. It is normative — a reviewer
checks the shader against this, not against taste.

### 6.1 Primitive quad instance

One instanced pipeline replaces the current vertex soup. Per-instance data:

```wgsl
struct Primitive {
    rect        : vec4<f32>,   // x, y, w, h  in logical px
    radii       : vec4<f32>,   // tl, tr, br, bl
    fill_a      : vec4<f32>,   // authored sRGB, straight alpha
    fill_b      : vec4<f32>,   // gradient stop B; == fill_a for flat
    grad_axis   : vec2<f32>,   // unit vector; (0,0) disables gradient
    border_color: vec4<f32>,
    border_width: f32,
    shadow      : vec4<f32>,   // offset_x, offset_y, blur, spread
    shadow_color: vec4<f32>,
    clip_rect   : vec4<f32>,   // rounded-clip bounds
    clip_radius : f32,
    flags       : u32,         // INSET | TEXTURED | GLOW | MASK_IS_COVERAGE
    tex_layer   : u32,
}
```

Fragment evaluation, in this order:

1. `d = sdRoundedBox(p - center, half_extent, radius_for_quadrant)`
2. Fill coverage: `smoothstep(aa, -aa, d)` where `aa = fwidth(d) * 0.5`.
   **Antialiasing is analytic — no MSAA, no supersampling, no extra target.**
3. Gradient: `mix(fill_a, fill_b, saturate(dot(p_norm, grad_axis)))`,
   **interpolated in linear space, after the sRGB decode, never before.**
4. Border: second distance band,
   `abs(d + border_width * 0.5) - border_width * 0.5`.
5. Shadow: a separate instance drawn behind, using the analytic blurred
   rounded-box approximation — **not** concentric rings.
6. Rounded clip: multiply coverage by the clip shape's own smoothstep.

### 6.2 Colour contract — inherited unchanged from Zeta-B

**This is the single highest-risk item in the phase.** The rules:

- Every colour that reaches the GPU is authored **sRGB bytes with straight alpha**.
- The shader decodes sRGB → linear **exactly once**, at the top of `fs_main`,
  before any interpolation and before any blend.
- Gradient interpolation, shadow accumulation and glow accumulation all happen
  **after** that decode, in linear.
- The swapchain encodes. Nothing else encodes.
- The `const OUTPUT_IS_SRGB` string substitution for non-sRGB surfaces is
  preserved, and its two existing tests in `pass.rs` must still pass unmodified.

### 6.3 Batching and state

- One pipeline, one instance buffer. Texture-binding changes and clip-stack
  changes are the only batch breaks.
- The font atlas and icon atlas keep their existing `texture_id` 0 and 1.
- Instance count, batch count and buffer bytes are exported per frame for §10.6.

### 6.4 Compatibility shims — day-one, non-negotiable

```
push_rect_filled(rect, color)  -> Primitive { radii: 0, border: 0, flat fill }
push_rect_border(rect, t, c)   -> Primitive { radii: 0, border: t, fill: transparent }
push_textured_rect(..)         -> Primitive { flags: TEXTURED }
push_nine_slice(..)            -> unchanged, still emits textured quads
push_drop_shadow(rect, elev)   -> Primitive { shadow: from elev }, one instance
push_text_tracked(..)          -> unchanged glyph path
```

**Zero widget call sites change in 27-A.** The 80 existing tests must pass against
the new pipeline with no edits. That is the merge gate for 27-A.

---

## 7. Sub-phase map

| ID | Name | Delivers | Gate |
|---|---|---|---|
| 27-A | **Styx** | Primitive-quad pipeline: SDF fill/border/radius/AA/gradient/shadow/glow/inset, rounded clip | 80 existing tests green and unedited; before/after captures identical except AA and shadows |
| 27-B | **Lethe** | Text quality: subpixel X positioning, gamma-correct blend, stem compensation, per-DPI atlas, `cosmic-text` shaping + bidi | 12 px capture at 100/125/150/200 % beats baseline; Geist/Inter decided |
| 27-C | **Charon** | Animation driver, easing tokens, reduced-motion, per-node invalidation | Idle frames byte-identical; no animation >200 ms |
| 27-D | **Erebus** | Elevation ladder on real shadows; backdrop-blur pass for modal scrim and floating context bar | Both separate legibly over bright and dark scenes; blur ≤0.15 ms |
| 27-E | **Asphodel** | Token expansion: container nesting, accent ramp, ember secondary, gradient/glow/inset tokens, **Dawn** light theme | Both themes pass WCAG AA on every certified pair |
| 27-F | **Nyx** | Monogram route picked; optical ladder; `.ico`; DPI-correct icon atlas; duotone active variants; splash | Icons crisp at 200 %; manifest and licence checks green |
| 27-G | **Elysium** | Project picker; real Content Browser with engine-rendered thumbnails; viewport overlays; empty states; Search Everywhere | Each surface passes its workflow checklist with real assets |
| 27-H | **Cerberus** | Zeta-H completion: arrow traversal, focus trap/return, AccessKit, colour-vision pass, target sizes | Human keyboard-only walk completes |
| 27-I | **Tartarus** | Token lint, licence + manifest checks, component gallery, golden screenshot diffing, perf harness | A new property lands with no raw literal and with automated evidence |
| 27-J | **Olympus** | Full sign-off; rewrite `phase_26.md`; update `context.md`, `ATTRIBUTION.md` | §10 matrix passes; §16 executed |

**27-A → 27-B → 27-C are strictly ordered.** 27-D..27-G may interleave once 27-A
lands. 27-H..27-J are terminal.

---

## 8. Non-negotiable decisions

1. **`somnium_ui` is not replaced.** §3 is closed; §3.6 lists the only reopeners.
2. **`EditorEvent` does not change.** No new variants, no changed payloads.
3. **The Zeta-B colour contract survives verbatim.** §6.2.
4. **No widget call site changes in 27-A.** §6.4.
5. **The 68 px pre-scene budget does not grow.** Beauty is not bought with space.
6. **Radius tokens stay at 2 / 2 / 4 / 6.** §5.2.
7. **One new GPU pass only** — the 27-D backdrop blur. No other renderer work.
8. **Both themes ship together.** Nocturne and Dawn, from 27-E onward.
9. **Reduced-motion is a first-class mode** delivered in 27-C, not deferred to
   27-H.
10. **No dependency is added without a licence check landing first** (§11).

---

## 9. Implementation sequence

### 9.1 — 27-A Styx: the primitive-quad pipeline

The foundation. Nothing else in the phase is possible first.

**Work**

- New `src/primitive.rs`: the `Primitive` instance struct (§6.1), `bytemuck::Pod`.
- Rewrite `ui_pass.wgsl`: instanced vertex stage emitting a unit quad expanded by
  `rect` plus shadow padding; fragment stage evaluating §6.1's ordered pipeline.
- `sdRoundedBox` with per-quadrant radius selection.
- Analytic antialiasing via `fwidth`. No MSAA, no extra render target.
- Gradient in linear space, after decode.
- Border as a second distance band.
- Shadow as a separate instance behind the quad, analytic blurred rounded box.
- Glow: outer additive band gated by `flags & GLOW`, used by the focus ring only.
- Inset shadow for input recession, gated by `flags & INSET`.
- Rounded clip: replace scissor-only clipping with per-instance rounded clip
  bounds; keep the scissor for the coarse case.
- `draw.rs`: all existing entry points become §6.4 shims. Add
  `push_primitive(Primitive)` as the new native path.
- `style.rs`: `Paint` gains `gradient: Option<(Color, Vec2)>`, `shadow`, `glow`,
  `inset`. `Paint::radius` finally reaches the GPU.

**Merge gate.** All 80 existing tests pass **with no edits to any test or widget**.
Capture the shell again through `SOMNIUM_CAPTURE_UI_PNG`; the diff against
baseline must be confined to edge antialiasing and the shadow rewrite. Any colour
shift on a flat fill is a §6.2 violation and blocks the merge.

**Tests to add**

- `sdRoundedBox` unit tests against known distances.
- Shader-source assertion that the sRGB decode appears exactly once and precedes
  all interpolation.
- Golden: flat `push_rect_filled` output byte-identical to the pre-Styx path.
- Gradient midpoint is the linear-space mean, not the sRGB mean.

### 9.2 — 27-B Lethe: text quality

Type is most of what "professional" means, and it is currently the weakest
technical layer.

**Work**

- **Gamma-correct glyph blending.** Coverage is currently blended in the sRGB
  domain, which makes light-on-dark text render thin. Blend coverage in linear,
  and apply a stem-darkening correction tuned for the Nocturne ground.
- **Subpixel X positioning.** ~~Rasterize three horizontal phases per glyph.~~
  **Measured and rejected for this phase.**
  `font::tests::measured_atlas_pressure_for_the_editor_type_inventory` packs the
  real inventory — ASCII at the five `TextRole` sizes across all five bundled
  cuts — and reports **2,375 glyphs at 47.7 % of the 1024² atlas**. Three phases
  triples the cached bitmaps, needing ~143 %, and the failure mode is silent: a
  refused glyph renders as a blank advance, not an error. 27-B ships
  **integer-snapped glyph quads** instead, which removes the same bilinear smear
  for the common case at zero atlas cost. Subpixel phases stay blocked on a
  2048² atlas or an evicting shelf allocator; the test asserts the conclusion, so
  it reopens automatically if the footprint ever changes.
- **Per-DPI atlas regeneration.** `FontAtlas::set_dpi_scale` already exists; make
  the icon atlas follow the same rule (§9.6).
- **`cosmic-text` (MIT) for shaping, bidi and fallback** — the open 26-H item.
  Gate behind a feature flag; keep the fontdue path as fallback for one release.
- **The Geist decision gate** from Zeta §8A.6: capture Inter vs Geist at 12 px,
  100 / 125 / 150 / 200 %, in the real Details column, and **decide**. Inter stays
  unless Geist demonstrably wins.

**Exit:** side-by-side capture sheet at four DPI scales; Geist/Inter decided and
recorded; no regression in `measure_text_tracked` layout results.

### 9.3 — 27-C Charon: motion

**Work**

- `src/motion.rs`: an `Animator` holding `(NodeHandle, PropertyId) -> Track`.
  Tracks are cubic-bezier or critically-damped spring; both, chosen per token.
- Frame delta threaded from `UiManager::end_frame`.
- **Only animating nodes invalidate.** A tick that advances no track must not
  dirty the tree.
- Easing tokens added to `MotionTokens`: `ease_standard`, `ease_decelerate`,
  `ease_accelerate`, `spring_press`.
- `reduced_motion: bool` on the theme snapshot. When set, every track completes
  instantly. **Layout must be identical either way** — this is asserted by test.
- Wire the four existing durations, finally: hover wash cross-fade 120 ms; press
  90 ms; popup scale-and-fade from its anchor 140 ms; drawer slide 200 ms.
- Toast enter/exit and stacking.

**Forbidden:** looping animation, idle breathing, parallax, spring overshoot on
anything a user scrubs, and any animation on a numeric field's value display.

**Exit:** idle frames byte-identical two frames running (test); reduced-motion
produces identical layout (test); no animation exceeds 200 ms.

### 9.4 — 27-D Erebus: elevation and backdrop blur

**Work**

- Elevation ladder replacing the three-level `ElevationTokens`:
  `canvas 0 | panel 1 | raised 2 | popup 3 | drawer 4 | modal 5 | toast 6`, each a
  real `(offset_y, blur, spread, alpha)` driving §9.1's shadow instance.
- **Panels still never cast** — Zeta's rule holds. Elevation marks z-order.
- **Backdrop blur pass.** One UI-owned pass: copy the swapchain region behind the
  target, two-pass separable blur at half resolution, sample it as the surface's
  backdrop. Applied to **exactly two** surfaces: the modal scrim, and the floating
  viewport context bar. Nothing else, ever.
- Scroll-edge fade on `ScrollViewer` so the Details panel stops cutting mid-row
  (audit §2.4 item 4).

**Exit:** modal and context bar separate legibly over a bright and a dark scene;
blur pass ≤0.15 ms at 1440p; disabling blur degrades to the current flat scrim
with no layout change.

### 9.5 — 27-E Asphodel: token expansion and the second theme

The design-system work proper. This is where the tokens file grows.

**Work**

1. **Container nesting** (from COSMIC). Replace the flat 12-surface list with a
   nesting rule: each container level derives its `surface`, `component`,
   `border`, and `on_*` text colours from its parent plus an elevation delta.
   Hierarchy then survives without a hairline on every edge.
2. **Accent ramp.** Derive indigo 50…900 once from `#7A86FF` in a perceptual
   space, so `hover` / `pressed` / `disabled` / `glow` / `selected_bg` are
   *computed* and consistent instead of five hand-picked hexes.
3. **Ember secondary.** Promote the orphaned `folder: #C4A574` into a full warm
   role ramp used for content and asset semantics. This gives the two-temperature
   composition of §4.2. **Ember never competes with indigo for focus or
   selection** — it carries asset identity only.
4. **Gradient tokens.** `chrome_wash` (2–4 %), `header_wash`, `accent_primary`,
   `rail_accent`, `viewport_vignette`. Linear space. §5.3's limits are hard.
5. **Glow tokens.** `focus_glow`, `armed_glow`. Two only.
6. **Inset tokens** for input recession.
7. **Motion tokens** finalised from 27-C.
8. **Dawn** — a complete light theme. Same recipes, different snapshot. Any recipe
   that cannot express Dawn is a hard-coded recipe and gets fixed.
9. **Contrast certification.** Extend Zeta §8A.3's certified pair table to cover
   every new pair, in **both** themes, at WCAG AA. Automated, not eyeballed.
10. `nocturne.tokens.json` gains a sibling `dawn.tokens.json` under a shared
    `$schema`, and `theme.rs` is verified against both by test.

**Exit:** both themes render the full component gallery; every certified pair
passes AA in both; a theme switch is one snapshot swap with no relayout.

### 9.6 — 27-F Nyx: identity completion

Closes Zeta §8A.6.

**Work**

- **Pick the monogram route.** Eclipse is recommended and all three SVGs are
  already vendored at `crates/somnium_ui/assets/brand/`. Decide, record, and cut
  the optical ladder: 16 / 24 / 32 / 48 / 64 / 128 / 256 plus the application
  icon, with simplified counters at 16 px.
- Windows `.ico`, splash lockup, About window, watermark, empty-state mark.
- **DPI-correct icon atlas.** Today `ICON_CELL = 32` and `ICON_CELL_LARGE = 96`
  are fixed, so a 16 px row icon at 200 % samples a 32 px cell upscaled — visibly
  soft. Regenerate the atlas through `resvg` at window DPI, mirroring
  `FontAtlas::set_dpi_scale`.
- **Duotone / filled active variants** for icons whose active state currently
  relies on colour alone.
- Complete the Tabler vendoring the manifest already expects; run the manifest
  check (27-I).
- Clear-space, minimum-size and forbidden-use rules written into
  `assets/brand/PROVENANCE.md`.

**Exit:** icons crisp at 100 / 125 / 150 / 200 %; the similarity board from Zeta
§5.2 produced; manifest and licence checks green.

### 9.7 — 27-G Elysium: the surfaces that carry the first impression

Ordered by how early a user sees them.

1. **Project picker / startup surface — highest leverage in the phase.** The
   editor currently opens straight into a scene. Build a branded entry: recent
   projects with captured thumbnails, templates, engine version, a quiet Nocturne
   ground with the Eclipse mark, one primary action. This is the first frame a new
   user sees and it does not exist today.
2. **Content Browser, properly** (Zeta-G's open item, audit §2.4 item 2). Folder
   tree, breadcrumb, back/forward, typed filter chips, grid/list density, type and
   status badges, metadata tooltip, inline rename, keyboard and multi-select,
   context actions, and **engine-rendered asset thumbnails** — a narrow
   request/response service that asks the existing renderer for a small offscreen
   preview of a mesh or material, disk-cached, generated async, falling back to
   type icons. This single change replaces seven identical gold folder glyphs with
   a real content surface. 26-D2 drag/drop lands here when the input contract is
   ready.
3. **Viewport overlays.** Selection outline, grid treatment, gizmo styling, stats
   overlay, and the context bar's blurred backdrop from 27-D.
4. **Empty states** for Outliner, Details, Content Browser, Output Log — mark, one
   sentence, one action. Plain Speech, per §13.
5. **Search Everywhere.** Upgrade `CommandPalette` to span commands, entities,
   assets, panels, settings, recent items and Help, with category prefixes and
   shortcut hints.
6. **Details polish**: favourites, units, validation, advanced disclosure,
   documentation links — the rest of Zeta-G.
7. **Slider, input and numeric-field redesign** on the new primitives (audit §2.4
   items 3 and 8): recessed track, filled range with the accent gradient, real
   thumb, hover and focus states, recessed input fills.

**Exit:** each surface passes its workflow checklist with real assets, real
entities, and real error states.

### 9.8 — 27-H Cerberus: interaction and accessibility completion

Closes Zeta-H's remaining list. No new scope.

- Arrow-key traversal **inside** Outliner and Details.
- Focus trap and focus return inside the modal.
- AccessKit: stable semantic IDs, roles, labels, values, states, actions.
- Colour-vision simulation pass across both themes.
- WCAG 2.2 target-size audit with documented dense-editor exceptions.
- Cursor shapes, disabled reasons, drag affordances, destructive confirmations.
- **A human keyboard-only walk of the whole editor.** No headless test substitutes
  for this; it stays open until a person does it.

### 9.9 — 27-I Tartarus: harness and lints

Closes Zeta-I's remaining list.

- **Token lint / raw-literal audit** — a widget containing a colour literal fails
  the build. This is what makes §5.7 real.
- Licence check and icon-manifest check in CI.
- **Component gallery scene** rendering every widget in every `VisualState`, in
  both themes, at four DPI scales.
- **Golden screenshot diffing** with deterministic thresholds — Zeta's deliverable
  8, still unproduced.
- **Performance harness** exporting per-frame instance count, batch count, buffer
  bytes, UI CPU build time, and UI GPU time against the §10.6 budget.
- Document how a new engine feature adds commands, icons, properties, Help,
  telemetry and states without editing unrelated UI files.

### 9.10 — 27-J Olympus: sign-off

- Run Phase 26 §14 must-not-break, Zeta §10, and this file's §10 and §12.
- Capture before/after sheets, component gallery, four DPI scales, both themes,
  and a short interaction video.
- Record CPU/GPU/draw/atlas metrics against the pre-Styx baseline.
- **Execute §16** — rewrite `phase_26.md`.
- Update `context.md` §8 and roadmap, `ATTRIBUTION.md`, `THIRD_PARTY_NOTICES.md`.
- **Keep every incomplete item open.** Do not declare Hades complete from
  screenshots. Phase 26 was honest about this and so is this file.

---

## 10. Acceptance matrix

### 10.1 Paint correctness

- [ ] sRGB decoded exactly once; shader-source test asserts it.
- [ ] Flat `push_rect_filled` output byte-identical to the pre-Styx path.
- [ ] Gradient midpoints are linear-space means.
- [ ] Straight alpha preserved end to end.
- [ ] Non-sRGB surface path still substitutes `OUTPUT_IS_SRGB = false`.

### 10.2 Visual system

- [ ] Radius tokens visibly rendered at 2 / 2 / 4 / 6.
- [ ] All edges antialiased; no hard pixel stepping at any DPI.
- [ ] Shadows are analytic, not ringed; no banding at drawer or modal spread.
- [ ] Gradients present on chrome only, within 2–6 %.
- [ ] Glow present on exactly two roles.
- [ ] Both themes complete; every certified pair passes WCAG AA in both.
- [ ] Icons crisp at 100 / 125 / 150 / 200 %.
- [ ] No surface uses a raw colour literal (lint-enforced).

### 10.3 Motion

- [ ] Idle frames byte-identical two frames running.
- [ ] No animation exceeds 200 ms.
- [ ] Reduced-motion produces identical layout.
- [ ] No looping, breathing, or decorative motion anywhere.

### 10.4 Workflow

- [ ] Project picker ships and is the first frame.
- [ ] Content Browser passes its full checklist with engine-rendered thumbnails.
- [ ] Search Everywhere spans all seven categories.
- [ ] Every panel has a designed empty state.

### 10.5 Interaction and accessibility

- [ ] Keyboard-only walk completes, verified by a human.
- [ ] Focus trap and return work in the modal.
- [ ] AccessKit roles present on every interactive widget.
- [ ] Colour-vision pass complete in both themes.
- [ ] No required state is colour-only.

### 10.6 Engineering and performance

Budget, measured at 1920×1080 and 2560×1440, against the pre-Styx baseline:

| Metric | Budget | Measured (27-A) |
|---|---|---|
| UI CPU build (draw-list construction) | ≤1.5 ms | not yet instrumented |
| UI GPU (primitive pass) | ≤0.40 ms | needs a GPU capture |
| Backdrop blur pass | ≤0.15 ms, and only when visible | 27-D |
| Batches per frame | **≤192** (was ≤8 — see below) | **146** |
| Instance-buffer bytes per frame | ≤256 KB | **61 KiB** |
| Instances per frame | — | **625** |
| Idle-frame allocations | 0 | draw list byte-identical (test) |
| Existing tests | 80 green, plus new ones; none edited to pass | 80 + 34 = **114 green** |

Instanced quads *do* reduce traffic: a 1 px border was 4 quads and is now 1
instance; a drop shadow was 6 and is now 1. The pre-Styx list spent 104 bytes per
quad (4 × 20 B vertices + 6 × 4 B indices) against Styx's 100 bytes per shape.

**The ≤8 batch figure in the first draft of this table was a guess, and it was
wrong by more than an order of magnitude.** It is corrected here from the real
shell. `UserInterface::draw_node` pushes a clip rect for every visible node, so
each batch break is a genuine clip transition rather than waste. Folding both
atlases into one bind group — which is why `DrawCommand` no longer carries a
texture id — took the count from 164 to 146; the remainder is the widget tree's
clipping structure. Collapsing it to a single draw means clipping per instance in
the fragment shader instead of by scissor. That is real work, scheduled for 27-D
where rounded clips are needed anyway, rather than a perf chase: 146
scissor-plus-draw pairs cost microseconds of command recording. If a *measured*
budget is missed after two optimisation attempts, §3.6 reopens.

---

## 11. Licensing and provenance

- **No dependency is added before its licence check lands.** Candidates and their
  licences, to be re-verified from upstream at adoption time and not from memory:
  `cosmic-text` (MIT), and nothing else planned.
- **Technique citations** for §3.5's imported ideas go into `ATTRIBUTION.md`
  *before* the code that uses them merges: SDF rounded-box primitives, the fast
  analytic rounded-rect shadow approximation, the instanced-quad UI rendering
  approach, and COSMIC's container/on-colour theming model. These are published
  techniques; **no source is copied from any of them.**
- **No Slint, GPUI, Iced, libcosmic, or Tauri code enters the tree.** §3 rejected
  them as dependencies; importing their source instead would be worse.
- All brand assets remain original geometry (Zeta §5.2). All fonts remain OFL,
  re-sourced from official upstreams with licences shipped.
- Tabler icons remain MIT, vendored per the manifest, only the icons used.
- `THIRD_PARTY_NOTICES.md` updated in 27-J.

---

## 12. Must-not-break

Additive to `phase_26.md` §14 and Zeta §10, which both remain in force.

1. `EditorEvent` — no new variants, no changed payloads, no changed semantics.
2. The Zeta-B colour contract (§6.2).
3. All 80 existing `somnium_ui` tests, **unedited**.
4. The 68 px pre-scene budget at 1920×1080.
5. Every `GridMessage` row index in the outer grid, including the two retired
   zero-height rows.
6. `PropertyRow`'s measured grammar: 14 px gutter, 46 % label column clamped
   96–176, ellipsis-with-tooltip, sub-240 px stacking.
7. The four-cue state grammar (Zeta §8A.4).
8. The seven named workspaces and their persisted layouts.
9. Esc layer order: modal → palette → popup → drawer → filter → selection.
10. Tab region order: application → mode → viewport context → rail → Outliner →
    Details → drawer → status.
11. The `UiCanvas` runtime path — a game must still be able to draw its own UI,
    and must gain the new primitives too.
12. Immersive play mode, Content Drawer dismiss-on-focus-loss, ComboBox root popup
    overlay, per-property revert in one undo step.

---

## 13. Copy and voice

All new user-facing strings follow Plain Speech: concise, direct, active voice,
sentence case for body/buttons/labels, Title Case for headings, no exclamation
marks, no "Please", no "Oops". Error messages say what happened, why if useful,
and what to do next. Empty states say what would be here and give one action.
Personality is allowed in exactly two places — the About window and the splash —
and nowhere else.

Existing expanded labels stay expanded (Zeta-G turned 72 abbreviations into words;
do not regress that).

---

## 14. Start checklist

Before writing any code:

1. Read §0's reading list in order. All of it.
2. Capture a fresh baseline: `SOMNIUM_CAPTURE_UI_PNG` at 1920×1080 and 2560×1440,
   100 / 125 / 150 / 200 %, on both a bright and a dark scene. Commit to
   `dev records/phase 27/baseline/`.
3. Record the pre-Styx §10.6 metrics. **Without this the budget is
   unfalsifiable.**
4. Run `cargo test -p somnium_ui` and record 80 green.
5. Confirm §3 is understood and §3.6 is not triggered.
6. Write the `ATTRIBUTION.md` technique citations for §9.1 **first**, before the
   shader.
7. Create `dev records/phase 27/` with `design/` and `evidence/` subdirectories
   mirroring `phase 26/`.
8. Start 27-A. Do not start 27-E, 27-F or 27-G first, however tempting — token and
   identity work built on the old rasterizer will be thrown away.

---

## 15. Risks and controls

| # | Risk | Likelihood | Control |
|---|---|---|---|
| 1 | Shader rewrite regresses the single-sRGB-decode contract | Medium | §6.2 normative; shader-source test; byte-identical flat-fill golden; before/after captures on the 27-A merge gate |
| 2 | Rounded corners plus gradients drift toward a consumer-app look | Medium | §5.2's explicit forbidden list; radius frozen at token values; gradients capped at 6 %; glow limited to two roles; component gallery reviewed in one sitting |
| 3 | Motion causes continuous redraw and burns a game engine's frame budget | Medium | 27-C's "only animating nodes invalidate" rule, asserted by an idle-frame byte-identity test |
| 4 | Scope creep from the backdrop blur into renderer work | Medium | §8 rule 7: exactly one new pass, UI-owned, two surfaces, degrades to flat scrim |
| 5 | Thumbnail service pulls renderer changes into a UI phase | Medium | Narrow request/response API only; no renderer feature; disk-cached; async; falls back to type icons |
| 6 | `cosmic-text` adoption destabilises layout measurement | Medium | Feature-flagged; fontdue path retained one release; `measure_text_tracked` results diffed |
| 7 | Dawn theme reveals dozens of hard-coded recipes late | High | Build Dawn in 27-E, not 27-J; a recipe that cannot express Dawn is a bug found early |
| 8 | Perf budget missed | Low-medium | §10.6 measured from a real baseline; two optimisation attempts, then §3.6 reopens honestly |
| 9 | Phase declared done from screenshots | Medium | §9.10 and Phase 26's own precedent: incomplete items stay open, in writing |
| 10 | The 21k-line rewrite temptation returns mid-phase | Low | §3 recorded with evidence; §3.6 lists the only reopeners |

---

## 16. What this phase does to `phase_26.md` on completion

`phase_26.md` currently opens with a long "this phase is not closed" status block
listing 26-H, 26-J, 26-D2, and the Zeta remainder. When 27-J passes:

1. **Rewrite the status block.** Move 26-H (SDF/shaping) to *closed by 27-B*,
   26-D2 (drag/drop) to *closed by 27-G*, and every Zeta-G/H/I open item to
   *closed by 27-G/H/I* — each with the sub-phase that closed it named.
2. **Keep 26-J open** unless a reflection inspector was separately requested. Do
   not close it by implication.
3. **Add a forward pointer** in §0 to this file, in the same shape as the existing
   pointer to `phase_26_Zeta.md`.
4. **Update §1's executive decision**, which still says the toolkit "only draws
   editor chrome" and that "no icon ever uses `push_textured_rect`" — both
   obsolete.
5. **Update §4's audit table** to the post-Hades state.
6. **Do not delete Phase 26's history.** The Metaphor and Nocturne contracts stay
   readable; they are the reason this phase could be surgical.
7. Mirror the same edits into `phase_26_Zeta.md` §8A.6 and §9's sub-phase status
   blocks, and into `context.md` §8 and roadmap row 26 — plus a new roadmap
   row 27.

---

## 17. Research sources and confidence

**Direct evidence — high confidence.** Read in tree on 2026-08-18:
`crates/somnium_ui/src/{draw,pass,theme,style,typography,color,icons,font}.rs`,
`ui_pass.wgsl`, `assets/tokens/nocturne.tokens.json`,
`dev records/phase 26/zeta_shell_after.png`, `phase_26.md`, `phase_26_Zeta.md`,
`context.md` §8. The dead-radius and no-motion findings (§2.2) are verified by
grep, not inferred.

**Primary documentation — high confidence for the framework decision (§3).**
Slint's wgpu integration modules and its three-way licence structure; libcosmic's
MPL-2.0 library licensing and its COSMIC-desktop application scope; GPUI's pre-1.0
status and documented platform support; `iced_wgpu`'s manual `Engine` integration
path. Each was checked against upstream documentation on 2026-08-18 and **must be
re-verified at adoption time** if §3.6 is ever triggered.

**Inference — must be validated in Somnium.** The §10.6 performance budget is
derived from the expected instance-count reduction, not measured; §14 step 3
exists precisely to make it falsifiable. The claim that analytic AA is sufficient
without MSAA holds for axis-aligned rounded rects and is standard practice, but
must be confirmed at 100 % DPI on a low-density display. The stem-darkening
constant in 27-B is empirical and needs capture-based tuning.

---

## 18. Implementation ledger

*(Append one dated entry per pass, in the shape used by `phase_26_Zeta.md` §15:
what was done, what it cost, what it revealed, and what stayed open. Do not
summarise optimistically.)*

### 2026-08-18 — 27-A Styx and 27-B Lethe, first pass

**Status: the pipeline is in and green; nothing is repainted yet.** 27-A's
foundation and 27-B's two mechanical text fixes are in the tree. No widget has
opted into a radius, gradient or elevation, so **the editor still looks the
same** apart from antialiased edges and the shadow rewrite. That is the intended
shape of the merge gate, not an unfinished job — §6.4 exists precisely so the
paint layer can land before any recipe changes.

**What landed**

- `primitive.rs` (new). `Primitive`, a 100-byte `Pod` instance carrying rect, uv,
  four corner radii, shadow, gradient axis, border, expand, four colours and
  flags, plus 12 vertex attributes and constructors for fill / textured / glyph /
  shadow / glow / inset.
- `ui_pass.wgsl` (rewritten). Instanced SDF shader: the unit quad is generated
  from `@builtin(vertex_index)`, and the fragment stage derives fill, gradient,
  border, shadow, glow, inset and analytic AA from one `sd_rounded_box` distance.
  No MSAA, no extra target.
- `pass.rs` (rewritten). Vertex+index buffers replaced by one instance buffer;
  `draw_indexed` replaced by `draw(0..6, instances)`; BG0 grew from a 64-byte
  ortho to an 80-byte `Globals` carrying `text_gamma`; `UiFrameStats` added.
- `draw.rs` (rewritten). Instance list replaces the vertex/index list.
  **Every historical `push_*` entry point kept its signature**, plus
  `push_primitive`, `push_round_rect`, `push_round_rect_border` and
  `push_drop_shadow_rounded`.
- `font.rs`. `render_scale` replaces `dpi_scale`; atlas diagnostics
  (`utilization`, `is_full`, `cached_glyph_count`) and a warn-once on exhaustion.
- `lib.rs`. Registered `primitive`; removed the incorrect scale-factor wiring;
  added the `styx_budget_tests` measurement module.
- `ATTRIBUTION.md` §13E. Six technique citations (Inigo Quilez's `sdRoundedBox`,
  the instanced-quad approach from GPUI, analytic AA, Evan Wallace's rounded-rect
  shadow, COSMIC's theming concepts, text coverage gamma) plus a supersession
  notice on §13.17. **No source copied; no dependency added.**

**The gate.** All **80 pre-existing tests pass unedited**; 34 new ones bring the
suite to **114 green**. `cargo build --workspace` is clean, and `cargo clippy`
reports **zero warnings in all four rewritten files** (the remaining crate
warnings are pre-existing, in `ui.rs`, `button.rs`, `color_picker.rs`,
`numeric_field.rs`, `popup.rs`, `parts.rs` and `lib.rs`).

**Four things the work revealed, none of them guessed**

1. **The ≤8 batches budget was wrong by 20×.** Measured 164 on the real shell.
   Folding both atlases into one bind group — selector moved onto the instance,
   `DrawCommand::texture_id` deleted, the white 1×1 texture retired — took it to
   **146**. The rest is the clipping structure, not waste. The §10.6 table is
   corrected and the fix belongs to 27-D. The instance buffer measured **61 KiB**,
   comfortably inside the 256 KB guess.
2. **`Primitive`'s first attribute array silently skipped `shadow_color`.** Every
   field after offset 92 would have been read as its neighbour. Caught before it
   compiled; `declared_attribute_formats_exactly_tile_the_instance` now walks the
   whole layout and fails on any gap or overlap instead of only checking the last
   offset.
3. **`FontAtlas::set_dpi_scale(window.scale_factor())` was actively harmful.**
   Layout is in physical pixels (`reposition_panels` passes `inner_size()`), so
   the scale factor multiplied on top of `SUPER_SAMPLE`: at 200 % a 13 px glyph
   rasterized at 39 px and was minified into a 13 px quad — softer, and 9× the
   atlas area per glyph. The wiring is removed and the replacement documented so
   it cannot come back by accident.
4. **Three-phase subpixel positioning does not fit.** Measured 2,375 glyphs at
   **47.7 %** of the atlas for the real type inventory; three phases needs ~143 %.
   27-B snaps glyph quads to whole device pixels instead. §9.2 records it and the
   test asserts it.

**A defect found and deliberately left open.** The UI lays out in physical
pixels, so at 200 % DPI the whole chrome is half its intended apparent size — a
36 px title bar occupies 36 device pixels regardless of scale. This is a genuine
bug, it is **not** fixed here, and it is larger than 27-B: making layout logical
means scaling every density token, converting scissor rects, and changing how
`app.rs` reports window size. `FontAtlas::set_render_scale` is the hook.
**Schedule it explicitly before 27-F**, which cannot deliver DPI-correct icons on
top of a DPI-incorrect layout.

**What is still open in 27-A / 27-B**

- No widget recipe uses radius, gradient, elevation, glow or inset yet, and
  `style::Paint` has not been extended. That is the next step and the point of
  the phase.
- Rounded clipping is specified in §6.1 but not implemented; clipping is still
  scissor-only, which is what holds batches at 146.
- `DEFAULT_TEXT_GAMMA = 1.18` is **empirical and untuned**. It needs the 27-B
  capture sheet at four DPI scales. `UiPass::set_text_gamma` makes it a runtime
  knob, and 1.0 reproduces pre-Styx text exactly.
- `SUPER_SAMPLE` is deliberately unchanged at 1.5. Whether 1.0 reads better now
  that quads land on the texel grid is a capture-sheet question, not a guess.
- `cosmic-text` shaping and bidi: not started.
- The Geist-versus-Inter decision gate: not run.
- **No visual evidence captured.** §14 step 2's baseline sheet and the
  before/after diff both need a GPU and a human at the keyboard. Until those
  exist, the claim that the visual diff is confined to antialiasing and the
  shadow rewrite is argued from the shader and the golden tests, **not**
  demonstrated.

### 2026-08-18 — DPI correctness, then 27-C, 27-E and most of 27-D

**The DPI fix came first because 27-F cannot sit on top of it.** The widget tree
was being fed `window.inner_size()`, which winit reports in **physical** pixels,
so at 200 % a 36 unit title bar occupied 36 device pixels and the whole chrome
rendered at half its intended apparent size. The tree now lays out in **logical
units** and the scale factor appears at exactly two boundaries: pointer positions
coming in (`UserInterface::to_logical`) and the scissor rect going out
(`UiPass::prepare`). The projection is built from the logical extent, so the GPU
stretches layout space across the framebuffer and every density token holds.

Two details that matter more than they look:

- The scissor converts by the **measured** ratio (`physical / logical`) rather
  than by the raw scale factor, so a rounded logical size cannot drift a clip
  region off the framebuffer edge.
- `FontAtlas::render_scale` is finally meaningful and is finally *set*. Phase 27-B
  had to remove the scale-factor wiring because layout was physical; with layout
  logical, a `px` glyph occupies `px * scale` device pixels and rasterizing at
  `px * scale * SUPER_SAMPLE` is correct. Changing it invalidates the glyph cache,
  which is why dragging a window between monitors now clears and rebuilds it.

`runtime.rs` got the same treatment, so a **game** canvas is DPI-correct too, not
just the editor.

**27-C Charon — the animation driver.** `motion.rs`: `Easing` (linear, standard,
decelerate, accelerate, critically damped spring), `Track`, `Animator`, and
`lerp_color`, which blends in linear space so a half-way hover wash is the
perceptual midpoint rather than the byte midpoint. The animator lives on
`DrawingContext` beside the atlases, because a widget receives `&mut DrawingContext`
in `draw()` and nothing else — which is also the moment it knows its own
interaction state. `UiManager::end_frame` ticks it from a real `Instant` delta,
clamped to 100 ms so a breakpoint or a minimised window cannot teleport every
track to its end state.

Two contracts are enforced in code rather than by review: `MAX_DURATION_MS` clamps
inside `start`, so no call site can exceed the 200 ms ceiling; and a finished
track is *removed*, so `tick` returns false and an idle shell stays byte-identical.

**27-E Asphodel — the token layer, and Dawn.** The elevation ladder went from
three ad-hoc levels to five ordered rungs. Added gradient, glow and inset token
groups; `text.emphasis` as a real role (it was the hard-coded `MOON` constant);
`ember` promoted from the orphaned `folder` swatch into the warm half of a
two-temperature palette; and `ramp_step`, which derives accent steps on linear
values instead of hand-picking hexes. **`DAWN` ships**, and every `style.rs` recipe
now reads `theme::active()` — zero `NOCTURNE` references remain in the recipe
layer, so a theme swap repaints the editor without a single widget knowing.

**27-D Erebus — partial.** `Paint` gained `gradient`, `elevation`, `glow` and
`inset`, and `DrawingContext::push_paint` renders a whole `Paint` in one call with
a fixed layer order (shadow, glow, fill+border, inset, rail) so a widget cannot
get it wrong. Recipes opted in: a resting button is washed and lifted, a pressed
button loses the lift so it reads as pushed *into* the surface, an input is
recessed, and a focused control glows. `push_scroll_fade` closes the §2.4 audit
item about the Details panel cutting off mid-row.

**Five things the work revealed**

1. **The certification test found six failing pairs in the shipped Nocturne
   palette.** `text.muted` and `status.error` were below AA on `raised`, `header`
   and `popup` — Zeta §8A.3 had certified them against `panel` only. Both were
   corrected (`#7E8698` → `#8C95AA`, `#E05A5A` → `#E67070`) and the test now
   covers every surface a text role can land on, in both themes.
2. **An absolute luminance delta is the wrong metric for a gradient cap.** The
   same perceptual wash measures dL 0.0036 on the Nocturne ground and 0.0631 on
   Dawn, so §5.3's "2-6 %" would have forced the light theme's wash to be
   invisible. Expressed as a **contrast ratio** instead, both themes land at
   1.05-1.08 for chrome and 1.27-1.37 for accent gradients — scale-invariant, and
   it means the same thing on both.
3. **`Animator::start` silently did nothing.** It took the origin from the
   *target*, so a first transition to 1.0 began at 1.0 and no track ever ran. The
   origin is now an explicit `rest` parameter and a test guards it, because the
   failure mode was a control that simply never animated with nothing to say so.
4. **`border.strong` is not a WCAG 1.4.11 component.** The first draft of the
   certification test held it to 3:1 and it failed in both themes at 2.04 and
   2.43. It is a divider and a panel seam — decorative structure, explicitly out
   of scope. The test now holds only actual state cues to 3:1, and says why.
5. **The Nocturne token sheet was already stale.** Nothing pointed at the JSON, so
   the WCAG corrections did not reach it. Both sheets are now generated from
   `theme.rs` and verified back against it by
   `json_sheets_match_the_shipped_snapshots`, which also fails on a key present in
   one and not the other.

**What is still open**

- **The backdrop blur is not built.** It needs the swapchain copied into a
  sampleable texture, and `somnium_renderer/src/context.rs:323` only adds
  `COPY_SRC` when the surface capabilities allow it — so the pass must degrade to
  the flat scrim on hardware that does not. That plus a two-pass separable blur is
  real GPU work that cannot be verified without a GPU, and half-built unverifiable
  GPU code is worse than none. It stays 27-D's remaining item.
- Rounded clipping (§6.1) is still unimplemented; clipping is scissor-only, which
  is what holds batches at 146.
- No widget *calls* `push_paint` yet — the recipes describe the depth and the
  helper renders it, but the ~86 existing call sites still use `push_rect_filled`
  directly. Migrating them surface by surface is the next visible step.
- Motion is driven but not yet *started* by any widget: nothing calls
  `Animator::start`, so the editor does not animate yet.
- `DEFAULT_TEXT_GAMMA`, `SUPER_SAMPLE`, `cosmic-text`, the Geist gate and **all
  visual evidence** remain exactly as §18's first entry left them.
- The DPI fix is verified by five unit tests but **has not been seen on a HiDPI
  display**. That needs a human at the keyboard at 125 / 150 / 200 %.

### 2026-08-18 (third pass) — the depth reaches the screen, and 27-F starts

**This pass exists because the editor still looked identical.** That was a real
gap, not a scheduling artefact: 27-A through 27-E built the pipeline, the tokens
and the recipes, but **nothing called `push_paint`**, so every recipe described a
radius, a wash and an elevation that no widget ever rendered.

**Widgets migrated.** `button` (the toolbar, mode strip, sculpt rail, menu rows,
drawer tiles and status controls all route through it), `text_box`, `search_box`,
the tooltip, `slider`, `toast`, and `border` — which is the shell's panel
workhorse and therefore most of the editor's surface area. The slider was
rebuilt to the §2.4 audit item: a recessed capsule track, an accent-gradient
filled range, and a real lifted handle instead of two flat bars.

**The measurement that made the gap concrete.** A new test counts capability use
in the real shell's draw list, because "does it look different" deserves a number
rather than a claim:

| | after 27-A/B | after the first migration | after `wash_from` |
|---|---|---|---|
| rounded | 0 | 49 | 49 |
| washed | 0 | **1** | **28** |
| lifted | 0 | 20 | 20 |
| recessed | 0 | 4 | 4 |
| stroked | 0 | 17 | 17 |

**One washed instance was the bug.** The migration rule said a caller-supplied
`widget.background` was "a deliberate flat choice" and suppressed the gradient.
That was wrong: those backgrounds were set in Zeta to pick a *surface token*,
long before recipes existed, so nearly every chrome button opted itself out of
the wash. `theme::wash_from` now derives the same relative wash from whatever
base a surface actually uses — lighter at the top, darker at the bottom, mixed on
linear values, calibrated to the ~1.05 contrast ratio the token gradients use —
and `wash_for_surface` applies it to `header`, `raised` and `popup` fills while
leaving content grounds flat. The test now asserts the canvas ground is **never**
washed, because a gradient on every surface is exactly the "lit like a toy"
failure §5.2 forbids.

**Hover now animates.** `Button::draw` starts a `HoverWash` track keyed on its own
node and cross-fades between the rest and hover recipes with `lerp_color`. This is
the first actual consumer of 27-C.

**27-F Nyx — the icon atlas is DPI-correct.** It was a fixed 1024² grid of 32 and
96 px cells, so a 16 logical-px row icon at 200 % sampled a 32 px cell blown up to
32 device px. The cell grid and the atlas dimensions now scale together by the
device ratio — which is what keeps **every normalised UV byte-identical**, so not
one call site changed. `UiPass` recreates the GPU texture when the dimensions
move. `set_render_scale` is a no-op at an unchanged ratio, so an ordinary resize
costs nothing while dragging to a HiDPI monitor rebuilds and re-uploads.

**Three findings**

1. **The scale ceiling is set by memory, not geometry.** Packing is
   scale-invariant, so the atlas fits at any ratio; the measured cost is what
   binds. 4.0 MiB at 100 %, 16.0 at 200 %, **36.0 at 300 %**, 64.0 at 400 %.
   `MAX_RENDER_SCALE` is **3.0** — Windows tops out at 300 % and 64 MiB of icon
   atlas is not a reasonable trade for the fraction of a display beyond it.
2. **A test I wrote took 106 seconds.** `uvs_are_identical_at_every_scale` built
   one atlas per icon per scale — 252 full resvg passes. It builds three now and
   runs in 1.55 s. Worth recording because the naive shape looked harmless.
3. **`border.rs` keeps its per-side strokes.** A panel seam is a hairline, not a
   rounded box, so `Border` gained the wash but not a radius. Rounding every
   panel would have been the fastest way to make a dense editor look like a
   consumer app.

**What is still open**

- **27-G is not meaningfully started.** No project picker, no Content Browser
  workflows, no engine-rendered thumbnails, no empty states, no Search
  Everywhere. This is now the largest remaining block of user-visible value and
  should be the next session's whole focus.
- 27-F's remaining items: the monogram route is still undecided, and the optical
  ladder, `.ico`, splash and duotone active variants are uncut.
- The 27-D backdrop blur, unchanged: still gated on `COPY_SRC`.
- Rounded clipping (§6.1); batches still 146.
- Widgets **not** yet migrated: `check_box`, `tab_control`, `combo_box`,
  `context_menu`, `command_palette`, `property_row`, `tree_view`, `scroll_viewer`,
  `color_picker`, `splitter`, `menu`, `grid`. They still draw flat fills directly.
- `push_scroll_fade` exists but **no scroll region calls it**, so the Details
  panel still cuts off mid-row.
- Motion has exactly one consumer (button hover). Popups, drawers and toasts
  still appear instantly.
- `DEFAULT_TEXT_GAMMA`, `SUPER_SAMPLE`, `cosmic-text`, the Geist gate and **all
  visual evidence** remain untouched. Nothing in this phase has been seen on a
  screen — every claim above is from tests and measured draw lists.

### 2026-08-18 (fourth pass) — the rest of the widget migration

**Every remaining widget now renders through the paint layer.** `check_box`,
`combo_box` (header and dropdown), `tab_control`, `tree_view`, `property_row`,
`scroll_viewer`, `context_menu`, `command_palette`, `splitter`, `canvas`,
`stack_panel`, `grid` and `menu`. The rule for every edit was **paint only** —
every rect expression was copied through verbatim, so no hit target, no layout
and no behaviour moved. The `must_not_break` matrix, including
`every_inspector_control_is_actually_hittable` and the six-tool terrain palette,
passes unchanged.

Notable per-widget decisions:

- **`splitter` stays flat.** A seam is not a control; rounding or lifting it
  would read as a floating bar between panels.
- **`border` keeps per-side strokes and gains no radius.** Rounding every panel
  is the fastest way to make a dense editor look like a consumer app.
- **The `property_row` modified dot was a 5 px square.** The design has always
  called it a dot; the pipeline can round it now, so it is one.
- **`tab_control`'s active tab rounds only its top corners**, so it reads as the
  panel surfacing through the strip rather than as a floating chip.
- **The `scroll_viewer` thumb is a capsule**, and `push_scroll_fade` is finally
  called — only while there is more to see, and only at the edge there is more
  on, so a short list stays clean. That closes the §2.4 audit item about the
  Details panel cutting off mid-row.
- **Containers route through `wash_for_surface`**, so chrome picks up the wash
  and content grounds stay flat.

**A regression the measurement caught.** Moving the combo header to the `button`
recipe dropped the shell's stroke count from 17 to 16: the recipe is flat and the
pre-Styx header had a hairline outline. An outline is how a combo says it opens
something, so the header keeps it explicitly. Without the capability counter this
would have been a silent loss of affordance.

**Motion has a second consumer, and `MotionKey` grew a `sub` index.** An Outliner
is one widget painting N rows in a loop, so a node-only key made every row share
a single hover track and fade together. `MotionKey::row(node, sub, property)`
gives each row its own, and a test asserts two rows of one widget animate
independently. Selection is deliberately excluded from the fade — a selected row
must not blink when the pointer crosses it.

**Two new regression guards**, because 18 `draw()` methods changed and the
failure mode that matters is a surface that quietly stopped painting — bounds
still correct, nothing rendered, and no layout test would notice:
`every_shell_region_still_paints_something_visible` checks five regions, and
`no_migrated_surface_became_fully_transparent` watches the proportion of
zero-alpha instances (measured 11 %, which is the ghost icon buttons at rest).

**A correction to the previous entry.** It reported clippy as clean across the
touched widgets. That check used a forward-slash path filter that cannot match
Windows paths, so it matched nothing and looked clean. Re-run properly, the crate
carries **20 warnings** — and a stash-and-compare against the pre-27 tree gives
**20** as well, so this phase has added none. The individual warnings are
pre-existing style lints in `lib.rs`, `ui.rs`, `popup.rs`, `numeric_field.rs`,
`color_picker.rs`, `parts.rs` and the `button.rs` test helper.

**Still open, unchanged in substance**

- **27-G is not started.** No project picker, no Content Browser workflows, no
  thumbnails, no empty states, no Search Everywhere.
- The **backdrop blur** remains gated: `UiPass::render` receives a
  `&TextureView`, not the surface texture, so copying the swapchain needs an API
  change up through `app.rs` on top of the conditional `COPY_SRC`. Deferred
  rather than half-built.
- Motion still only covers hover. Popups, drawers and the drawer slide are
  instant.
- 27-F's monogram route, optical ladder, `.ico`, splash and duotone variants.
- Rounded clipping; batches still 146.
- **No visual evidence.** Every number here is from tests and measured draw
  lists. Nothing in this phase has been seen on a screen.

### 2026-08-18 (fifth pass) — 27-G Elysium, partially

**Three of 27-G's seven items landed. One is blocked by this file's own
must-not-break rule, and that is worth stating plainly rather than working
around.**

**Empty states (§9.7-4).** `metaphor::EmptyState` plus five shipped instances,
and `parts::build_empty_state` renders mark, headline, sentence and action. The
Content Drawer is wired: a drawer with nothing in it used to be a blank grey
rectangle, which reads as broken rather than as empty.

A distinction the copy makes deliberately: **an empty folder and a filtered miss
are different situations**. `CONTENT` says "Import a model from the File menu";
`CONTENT_FILTERED` says "Clear the search box". Offering an import to someone who
mistyped a search would be the wrong instruction, and a test asserts the two
never converge. `empty_state_copy_follows_plain_speech` enforces §13 mechanically
— sentence-case bodies ending in a period, no exclamation marks, no "Please",
no "Oops".

**Search Everywhere (§9.7-5).** The palette searched only its own static command
list. It now spans **commands, entities, assets, panels and Help**, with the
category prefixes the plan asked for: `>` `@` `#` `:` `?`. A bare prefix lists the
whole category, so it browses as well as filters. Prefix matches rank above
contained matches, because typing "sa" surfacing "Toggle Grid Snap" above "Save
Scene" is the failure that makes a palette useless. Each row shows its category,
so a result says what kind of thing it is before it says how to reach it.

**The constraint that shaped the design.** `run_palette_command` dispatches on
**positional index 0..14**. Appending is safe; inserting or reordering silently
rebinds every command after the insertion, and nothing in the codebase would
notice. So the 15 static commands keep their exact positions, everything dynamic
is appended, and dynamic rows carry a `PaletteTarget` instead of relying on
position. Two tests pin it: one on the count, one on the **exact label at every
index**. That second test is the one that matters — a count check alone would pass
a reorder.

**Content Drawer type badges.** Each tile now carries its extension, or `ENGINE`
for the virtual primitives that do not exist on disk. Badges use **ember**, the
warm half of the two-temperature palette, which is exactly the asset-identity role
27-E promoted it into and which never competes with indigo for a state cue.

**What is blocked, and why I did not route around it**

The **project picker** — §9.7-1, and the item I called "highest leverage in the
phase" — needs an `EditorEvent::OpenProject`. §12.1 of this file lists
`EditorEvent` as must-not-break with no new variants, and `app.rs`
`handle_editor_event` dispatches on it directly. The engine has no project
concept at all today; it loads `scene.somnium` at startup. This is a real design
decision, not a small one, and quietly adding a variant to a contract I wrote
three passes ago would have been the wrong call. **It needs an explicit decision
to amend §12.1 before the surface can be built.**

Also not started: engine-rendered asset thumbnails (§9.7-2, authorized by §0 but
substantial), the rest of the Content Browser workflows (back/forward history,
filter chips, density control, inline rename, multi-select), viewport overlays
(§9.7-3), and Details polish (§9.7-6).

**Still open elsewhere, unchanged**

- Empty states exist for the Outliner, Details and Log but are **only wired into
  the Content Drawer**. The other three need persistent handles in the layout
  struct so the state can be toggled rather than rebuilt.
- The 27-D backdrop blur, still gated on `COPY_SRC` plus an API change to hand
  `UiPass` the surface texture rather than a view.
- 27-F's monogram route, optical ladder, `.ico`, splash and duotone variants.
- Motion covers button and Outliner-row hover; popups and drawers are instant.
- **No visual evidence.** 181 tests and measured draw lists; nothing seen on a
  screen.

**A defect this pass introduced and then caught.** The first full workspace run
after 27-G reported **one failing test** and could not be reproduced — every
crate passed individually and the next run was clean. That is the signature of a
race, not a flake to shrug at.

`theme::set_active` writes a **process-global** selector, and Rust runs unit
tests multi-threaded: two tests swap the theme while **46 call sites** read
`theme::active()`. A reader scheduled beside `theme_selection_round_trips` or
`recipes_follow_the_active_theme` could observe Dawn and assert against Nocturne.

The selector is now **thread-local under `cfg(test)`** and stays a global atomic
in production, where the editor swaps themes from one UI thread. That removes the
interference without weakening what the tests assert — a swap still changes what
`active()` returns, it just cannot be seen by a test running beside it.
`a_theme_swap_cannot_leak_into_a_test_running_beside_it` spawns a probe thread
and fails if the selector ever goes back to a process-global under test. Five
consecutive clean runs since.

**The lesson worth keeping:** a single unreproducible failure in a 900-test
workspace is evidence of shared mutable state, not noise. Every earlier
"transient" in this session was a genuine OneDrive link-file lock (LNK1104, a
*link* failure); this one was a *test* failure and had a real cause.

### 2026-08-18 (sixth pass) — UI functionality audit

Asked to audit the UI for things that do not work. Two real defects, both mine,
both from this phase.

**1. The scroll-edge fade was invisible.** `UserInterface::draw_node` paints a
control and *then* recurses into its children, so anything a container emits
from `Control::draw` lands **underneath** its own content. The fade added in
27-D was therefore correct in geometry and colour and rendered as nothing. The
previous ledger entry claiming it "closes the §2.4 audit item" was wrong.

Fixed with `Control::draw_over`, a post-children hook with an empty default
implementation, called from `draw_node` inside the same clip. `ScrollViewer`
moved its **scrollbar and both fades** there — the bar had the same problem and
was being covered by tall content. `draw_over_paints_after_every_child` uses a
probe control to pin the ordering, and
`a_scroll_viewer_paints_its_bar_above_the_content` guards the specific case.

**2. I deleted `ScrollViewer::handle_routed_message` while fixing 1, and every
test still passed.** The edit that split `draw` used index arithmetic over the
source rather than exact anchors, and swallowed the handler that follows it.
Mouse wheel, thumb drag and track click were all dead. It was caught only
because the orphaned `drag_anchor_y` / `drag_scroll0` / `clamp_scroll` produced
dead-code warnings.

Restored from HEAD and redone as a **rename plus an inserted no-op**, deleting
nothing. A sweep then compared the function count of every file modified this
session against HEAD; no other deletion exists.

**The real finding is the coverage gap.** A whole input handler vanished and 184
tests were green, because **nothing exercised scroll input**.
`input_contract_tests` now covers wheel scrolling, clamping at both ends, track
clicks, checkbox state, tree-row hover and slider value emission — asserted
through observable effects (where content ends up, what gets painted, what is
emitted), because `Control` exposes no downcast and test-only introspection
would have been the wrong thing to add.

Two process notes worth keeping:

- **Index arithmetic over source text is not a safe edit.** Every other patch
  this session used exact string anchors and none lost anything; the one that
  computed a region boundary destroyed a function.
- **My first draft of these tests hard-coded pointer coordinates** and one
  failed. The root centres its child, so the viewer sat at (50, 40) and the
  click missed the gutter. That was a test defect, not a widget defect, and the
  fix was to derive every point from live `screen_bounds()`.

Suite at **190 green**, clippy unchanged at the 20-warning baseline.

### 2026-08-18 (seventh pass) — the ragged baseline

**Reported from a screenshot: letters in the shell sat at slightly different
heights.** It is a 27-B defect and the cause is exact.

`push_text_tracked` snapped **each glyph quad's top** to a whole pixel. A
glyph's top is `baseline - (ymin + px_h)`, and `ymin + px_h` differs per glyph,
so every letter rounded to *its own* subpixel offset. Adjacent glyphs in one run
therefore sat on different baselines. Measured on the failing code: in a 13 px
Inter run, glyph 0 landed on baseline 33.0 and glyph 1 on 33.333 — a third of a
pixel of vertical stagger, repeated unevenly across every label in the editor.

**Fix:** snap the **block origin** once — `origin.x.round()` and
`(origin.y + ascent).round()` — and place every glyph exactly relative to it.
The line advance in the newline branch is rounded too, so lines in a paragraph
share one pixel phase instead of drifting. Glyph positions are no longer rounded
at all, so their relative geometry is preserved to the fraction.

**The tradeoff, stated honestly.** Individual glyphs can now land on fractional
*x* (their `xmin` bearing is scaled by `1 / SUPER_SAMPLE` and is rarely whole),
so horizontal stem crispness is very slightly softer than per-glyph x-snapping
would give. That is the right trade: uneven spacing and a ragged baseline are
far more visible than a fractionally soft stem, and the report proves it.

**Guarded, and the guard was verified against the bug.**
`every_glyph_in_a_run_sits_on_one_baseline` recovers each drawn glyph's implied
baseline from the draw list and asserts they agree, across three font/size
combinations and a string chosen for ascenders, descenders, x-height and digits.
`a_text_block_starts_on_a_whole_pixel` covers the snapping that remains. The
per-glyph rounding was temporarily reintroduced to confirm the test **fails**
on the original code — it does, naming the exact glyph and offset — because a
regression guard that passes on the broken version guards nothing.

**This is the second defect this phase that only a human looking at the screen
could have found.** The draw-order bug and this one both produced correct
geometry, correct colour and green tests. §14 step 2's capture sheet is not
optional polish; it is the only instrument that catches this class.

Suite at **192 green**, clippy unchanged at the 20-warning baseline.

### 2026-08-18 (eighth pass) — 27-G continued, and a numbering collision

**Details empty state, wired.** The screenshot showed POSITION / ROTATION / SCALE
at `0.000` beside a status bar reading "No selection" — which says "the selection
sits at the origin", not "there is none". `update_inspector` now toggles the
property stack against the empty state, and `UiManager::new` seeds the empty one
because nothing is selected at startup and `update_inspector` has not run yet.

**Browser workflows.** `ContentHistory` (back / forward with correct truncation
semantics), `ContentFilterKind` (seven type chips routed through the same
`icon_for_path` answer the tile shows, so a chip can never disagree with its
tile), `ContentDensity` (three steps; compact deliberately stays under the 40 px
large-cut threshold so it does not pay for the 96 px atlas cut), and multi-select
as a **set** rather than a range — the tiles wrap, so "everything between A and B"
has no stable meaning once the panel resizes. Rename already existed and was left
alone. Both navigation entry points — folder double-click and breadcrumb — now
route through `navigate_content`, so Back cannot point at a folder the user never
left.

**Details polish: units.** Transform fields carry `m`, `°` and `×`; a light's
Range is metres and its cone angles degrees. The unit rides on the field rather
than the section heading so the two cannot drift apart, and it is hidden while
editing so it can never be mistaken for part of the text being typed.

**A widget the 27-D sweep missed.** `NumericField` still drew flat bars for its
embedded scrub slider. It now matches the standalone `Slider` exactly. The shell
capability count moved from 56/29/21/5/17 to **119 rounded, 38 washed, 30 lifted,
23 recessed, 17 stroked** of 748 instances, which is how much of the inspector
that one widget accounts for.

**A second order-dependent global, same shape as the theme race.**
`typography::REGISTRY` is a process-global `OnceLock`. Two new tests passed in
isolation and failed in the full suite: once any earlier test calls the editor's
`load_fonts`, every role resolves through that mapping, so a fixture registering
only two faces asks for a `MonoStrong` id that does not exist,
`get_or_rasterize` returns `None`, and the field renders **zero** glyphs. The
fixture now loads all five cuts in the editor's order. **Worth generalising: any
test fixture that registers fonts must register the full set, or it is testing a
different program depending on execution order.**

**A numbering collision, flagged rather than papered over.** This file is
`phase_27.md` and ~57 code comments say "Phase 27-x", but `context.md` **already
uses Phase 27 for skeletal animation**. The two are unrelated. The roadmap row
added this pass is keyed by codename ("Hades") and states the conflict; renaming
either one is a decision for whoever owns the roadmap, not something to do
silently across 24 files.

**Project picker deferred, recorded in `context.md`.** It needs
`EditorEvent::OpenProject`, which §12.1 freezes, and the engine has no project
concept at all today. That is a design decision about what a project *is* in
Somnium.

**Still open in 27-G:** engine-rendered thumbnails, viewport overlays, and the
Outliner / Log empty states (built, but only Details and the Content Drawer are
wired). Suite at **203 green**, clippy at the 20-warning baseline.

### 2026-08-18 (ninth pass) — the Details panel stopped scrolling

**Reported: the Details panel could not scroll and showed no thumb, on most
entities.** I caused it, and it exposed a latent bug that had been sitting in
`ScrollViewer` since it was written.

`arrange_override` **assigned** `content_h` on every iteration instead of
accumulating a maximum:

```rust
for &ch in &widget.children {
    content_h = ds.y.max(final_size.y);   // overwrite, not max
}
```

With exactly one child that is correct, which is why it survived this long.
Phase 27-G added the Details empty state as a **second, trailing** child of the
same scroll viewer, so the short sibling won and the panel reported its content
as exactly viewport-height — no scroll range, and a thumb painted in its
inactive colour. Every entity with more properties than fit was affected.

**Fixed in two parts.** `arrange_override` now accumulates the tallest child and
arranges in a separate pass. Both `measure_override` and `arrange_override` skip
**hidden** children via a new `LayoutCtx::is_visible`, so a panel stacking a
content state against an empty state does not reserve room for both — without
that, hiding the property list would still have claimed 600 px of scroll range
it was not showing.

**Guarded, and the guard was verified against the bug.**
`a_short_sibling_does_not_make_tall_content_unscrollable` builds exactly the
Details shape — tall child, short trailing sibling — and
`a_hidden_child_reserves_no_scroll_height` covers the visibility half. The
`arrange` fix was temporarily reverted to confirm the first test **fails** on the
original code; it does, naming the assertion.

**The pattern, now three for three.** The invisible scroll fade, the ragged
baseline, and this: correct geometry, correct colour, green tests, and only
visible to someone looking at the running editor. This one is worse than the
other two, because the latent `arrange` bug was reachable by anyone adding a
second child to any scroll viewer and nothing in the suite covered it.

Suite at **205 green**, clippy at the 20-warning baseline.

### 2026-08-18 (tenth pass) — 27-G finished bar the deferred picker

**Outliner and Log empty states, wired.** Both were built two passes ago and left
unconnected. The Outliner flips on entity count; the Log retires its empty state
on the first line and never brings it back, because the log is append-only for
the session. Both sit as siblings inside their scroll viewers, which is only safe
because the ninth pass taught `ScrollViewer` to skip hidden children when sizing
content — wiring these before that fix would have reproduced the Details bug in
two more panels.

**Viewport overlay.** A selection readout pinned bottom-left of the render. The
status bar already carries scene-wide counts; this answers "what am I holding"
without the eye leaving the viewport, which is the only justification for an
overlay over another status slot. It hides entirely when nothing is selected
rather than reading "No selection" twice — an overlay that is always on has
stopped being an overlay. Bottom-left because the context bar owns the top and
the gizmo tends to sit centre-right.

**Deliberately not done here:** selection outlines, grid treatment and gizmo
styling are renderer work. §9.7-3 lists them under viewport overlays, but they
do not belong in `somnium_ui` and adding them would put scene rendering behind
the UI crate's boundary.

**Thumbnails, split honestly along the boundary.** `thumbnail.rs` owns a 1024²
atlas of 64 px cells, and two kinds of asset get two different answers:

- **Images decode here.** `png`, `jpg`, `tga` and friends are decoded and
  downscaled by the `image` crate already in the dependency graph. No renderer
  involved, so this half works end to end today. Aspect ratio is preserved on a
  transparent ground rather than stretched: a squashed preview is worse than a
  small one for telling two assets apart.
- **Meshes and scenes are requests.** `take_thumbnail_requests` /
  `deliver_thumbnail` / `fail_thumbnail` are the narrow API §0 authorised.
  `somnium_ui` owns no renderer and must not grow one, so the hook is exactly
  where the boundary is. An unanswered request leaves the tile on its type icon,
  which is a correct resting state.

Decoding is **budgeted, not threaded**: two assets per frame. A worker thread
would need the atlas behind a lock and buys little, whereas the budget turns
"opening a folder of 200 textures" from an unpredictable stall into a predictable
per-frame slice. Failures are recorded so a corrupt file is never re-decoded, and
a full atlas settles tiles on their type icons rather than asking every frame.

The atlas is bound as a **third texture** at group 1 binding 3, sampled through
the same per-instance layer selector as the font and icon atlases — so it costs
no extra batch. It is the one atlas whose RGB carries meaning rather than
coverage, so a delivered preview draws untinted.

**Two test defects of my own, both caught and both instructive.** The overflow
test asserted on a request queue I had never drained, so it was measuring 256
earlier requests rather than the overflow. And the `Image` unit fixture
constructed the struct literally, so adding a field broke it — the same
class of breakage a builder would have absorbed.

**27-G is now complete except the project picker**, which is deferred by decision
and recorded in `context.md`. Suite at **215 green**, clippy at the 20-warning
baseline, workspace clean.
