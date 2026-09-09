# PERSONA expansion — motion, material and moments

> **Audience:** the implementing agent (GPT-6 Astra). This is a specification and a
> research brief, not a completion record. Implementation status is tracked in §21.
> **Date:** 2026-09-09. **Base:** `b58e788` (`more fixes`), branch `dev`.
> **Parent:** [`phase_PERSONA.md`](../phase_PERSONA.md). Slices A–F and the QoL
> follow-up are in tree; G (finish and accept) is open.
> **This document adds slices H–L before G closes**, because G is a *finish* pass
> and there is not yet enough shipped surface quality for it to finish.
> **Sole goal:** make the editor dramatically more beautiful and more professional
> without breaking a single thing that currently works.

---

## 0. How to use this document

1. Read §1 (goal), §2 (what already exists), §3 (why it is not beautiful yet) and
   §4 (**the contract you must amend rather than violate**) before writing code.
   §4 is the part most likely to be skipped and most likely to cause a rejected
   delivery.
2. §5–§7 are research. They exist so you do not re-derive them and do not add a
   dependency the codebase does not need. **The recommended dependency count for
   this entire expansion is zero.** §6 explains why, library by library.
3. §8–§12 are the work: five slices, each with multiple implementation options, an
   explicit recommendation, and an exit condition.
4. §13 is the choreography table — the actual list of what animates, when, for how
   long, and with what reduced-motion fallback. Treat it as the spec.
5. §14 is the QoL backlog. §15 is how not to break things. §16 is evidence and
   acceptance. §17 is rejected ideas. §18 is sources. §19 is the skills applied.

**Do one slice at a time and land it.** A half-wired motion system is worse than
none: it makes the editor look inconsistent rather than unfinished.

---

## 1. The goal, stated so it can be checked

The editor should read as a **precision instrument that is also expensive** — the
way Linear, Raycast, Blender 4.x, or the Wicked Engine editor read. Not as a
landing page, not as a consumer app, and not as a debug tool with a dark theme.

Three properties, in priority order:

| Property | What it means concretely | How it is judged |
|---|---|---|
| **Continuity** | Nothing teleports. Every state change the user causes is connected to its result by motion along the axis the action implies. | §13 fully wired; no surface in the shell changes state instantly except numeric values. |
| **Materiality** | Surfaces look lit and machined, not filled. Corners are optically correct, edges catch light, gradients do not band, layers separate. | §10; matched captures at 100 % and 200 %. |
| **Moments** | A handful of deliberate, memorable beats — first paint, mode arm, save, job completion, drop accepted. | §11; each is one-shot, named, and skippable. |

**Non-goal: ornament.** Every item below has a job. If you cannot name the job in
one sentence, it does not ship. The Atlus/Hades reading in
[`phase_27.md`](../phase_27.md) §4 and [`phase_PERSONA.md`](../phase_PERSONA.md) §1
is still the frame: strong identity, decisive feedback, legible hierarchy — not
imported style.

---

## 2. What already exists (do **not** rebuild any of this)

The single most important section for avoiding wasted work. Somnium owns far more
of the machinery than the current editor's appearance suggests.

### 2.1 The animation engine is already built — and almost entirely unused

`crates/somnium_ui/src/motion.rs` (1,465 lines) ships:

| Capability | Symbol | Notes |
|---|---|---|
| Timed tracks with easing | `Animator::start`, `Easing::{Linear,Standard,Decelerate,Accelerate,Spring}` | `Easing::apply` guarantees `f(0)==0`, `f(1)==1` — that is what lets reduced motion land on the exact same value. |
| Real spring systems | `Motion::Spring`, `Spring::{critical,SNAPPY,GENTLE,WOBBLY}`, `Spring::overshoots()` | Stiffness/damping, not a normalised curve. `MAX_SPRING_MS = 4_000` (`motion.rs:335`). |
| Authored curves | `Easing::Curve(CurveId)`, `Animator::register_curve` | Backed by CONTROL-K's curve editor and `somnium_ecs::curve::Curve`. |
| Delayed starts | `Animator::start_delayed` | |
| **Staggering** | `Animator::start_staggered`, `Animator::play_staggered` | Already there. Nothing calls it. |
| **Multi-property transitions** | `Transition::with`, `with_delayed`, `reversed()` | A declarative enter/exit pair. Nothing calls it. |
| Per-row keys | `MotionKey::row(node, sub, property)` | Solves the "one widget paints N rows" problem. |
| Reduced motion | `Animator::set_reduced_motion` | Timing only; end state identical. Test at `src/a11y/tests.rs:369`. |
| Idle discipline | `Animator::tick` returns `false`; finished tracks retire | Tests `motion.rs:1276`, `motion.rs:1286`. |
| Colour interpolation | `motion::lerp_color` | |

Ticked once per frame at `src/lib.rs:2410`, with `dt_ms.min(100.0)` so a stalled
frame cannot teleport tracks.

**Consumers, in the entire crate:**

```
src/widgets/button.rs:127     hover wash
src/widgets/tree_view.rs:167  hover wash
```

That is it. Two. Popups, drawers, modals, toasts, tabs, workspaces, selection,
focus, drag/drop, folds, tiles, jobs and floating windows all change state
instantly. **This is the largest gap between what Somnium can do and what it looks
like.** Slice H is mostly wiring, not engineering.

### 2.2 The paint pipeline

Two pipelines, one pass, drawn last, straight to the swapchain view
(`src/pass.rs`, `UiPass::render`).

**Quad pipeline** — `src/primitive.rs`, `src/ui_pass.wgsl`. One instanced unit
quad, SDF rounded box (`sd_rounded_box`, `ui_pass.wgsl:127`), 100-byte instance,
12 vertex attributes. Flags (`primitive.rs:28–39`):

```
FLAG_TEXTURED  1<<0    FLAG_TEXT      1<<1    FLAG_SHADOW    1<<2
FLAG_GLOW      1<<3    FLAG_INSET     1<<4    FLAG_GRADIENT  1<<5
```

Per-instance: `rect`, `uv`, `radii[4]` (per corner), `shadow[offset_x, offset_y,
blur, spread]`, `grad_axis`, `border_width`, `expand`, `fill_a`, `fill_b`,
`border_color`, `shadow_color`, `flags`. Analytic blurred rounded-box shadow, inner
shadow, outer glow and a two-stop linear gradient are already there.

**Shaped pipeline** — `src/shaped.rs`, `src/ui_shaped.wgsl`. Triangulated paths
with a 2×3 affine and — importantly — **three gradient kinds already compiled in**:

```
GRAD_LINEAR 1u   GRAD_RADIAL 2u   GRAD_ANGULAR 4u   TEXTURED 8u   COVERAGE 16u
```

plus a clip-mask slot (`mask: u32`). A radial "pointer light" and an angular
"sweep" are therefore **already expressible today** through the shaped pipeline,
before any shader change (§10.4, §10.5).

**Path machinery** — `src/path.rs` (1,241 lines): flattening, tolerance control,
stroking, plus `DrawingContext::{push_path, push_stroke, push_mask, push_shaped}`.
There is no reason to add a vector rasterizer crate.

Also present: `push_scroll_fade` (`draw.rs:509`), nine-slice (`draw.rs:761`), a
transform stack (`push_transformed`/`pop_transform`), and a clip stack.

### 2.3 The token system

`src/theme.rs` (1,389 lines) — `NOCTURNE` and `DAWN` snapshots plus a high-contrast
derivation. Exported to `crates/somnium_ui/assets/tokens/{nocturne,dawn}.tokens.json`
at `$meta.version = "0.3.0-persona"`, source of truth declared as
`theme.rs :: NOCTURNE`, verified by
`theme::token_sheet_tests::json_sheets_match_the_shipped_snapshots`.

Existing groups: `semantic` (surface / text / border / accent / focus / selection /
signal), `typography`, `density` (Compact 24/28, Comfortable 28/32), `geometry`,
`motion_ms`, `opacity`, `elevation` (7-level ladder), `gradient` (`chrome_wash`,
`header_wash`, `accent_primary`, `rail_accent`), `glow` (`focus`, `armed` — exactly
two, by contract), `inset` (`input`), `ember`.

```jsonc
"motion_ms": { "press": 90, "hover": 120, "popup": 140, "drawer": 200, "tooltip_delay": 400 }
"radius":    { "input": 4, "chrome": 5, "popup": 8, "modal": 10, "tile": 6 }
"stroke":    { "hairline": 1, "focus": 2, "rail": 2 }
```

Paint recipes live in `src/style.rs`: `button`, `primary_button`, `icon_button`,
`input`, `tree_row`, `asset_tile`, `drop_target`, `popup`, `action_button` with
`ButtonVariant`, over a composable `VisualState { interaction, focused, modified,
invalid, inactive }`. **All new visual states go through here.**

### 2.4 Renderer capability to reuse instead of writing

| Existing pass | File | Why it matters here |
|---|---|---|
| Bloom | `crates/somnium_renderer/src/pass/bloom.rs` | A 6-level progressive downsample + tent upsample chain. **Same family as dual Kawase** — the backdrop blur in §10.3 should reuse its shape, not invent one. |
| SPD | `pass/spd.rs` | FidelityFX single-pass downsampler, 6 mips per dispatch. A blur pyramid nearly for free. |
| Present | `pass/present.rs` | Already owns an offscreen `src_texture` + `src_view` upscaled to the swapchain. **This is the way around the `COPY_SRC` blocker** that stalled 27-D. |
| Grain, DoF, CAS, TAA/SMAA/FXAA | `pass/` | Scene-side only. Do not apply any of them to UI (§17). |

### 2.5 A latent defect found while surveying — fix it in H

`Animator::forget_node` and `Animator::animating_nodes` are **called from tests
only**. Nothing calls `forget_node` when a node is destroyed. Finished tracks retire
themselves, so the leak is bounded to *unfinished* tracks — but node indices come
from a pool and are recycled, so a recycled node can inherit a stranger's in-flight
track. Wire `forget_node` into node destruction and add a regression.
`animating_nodes()` allocates a fresh `Vec`; if you wire selective invalidation, do
not call it per frame without reusing a buffer.

---

## 3. Why it does not look beautiful yet — the honest gap list

Evidence key: **S** = read in current source; **V** = visible in committed PERSONA
captures; **P** = design judgement.

| # | Gap | Evidence | Consequence |
|---|---|---|---|
| G01 | Motion engine wired to 2 of ~40 interactive surfaces | S: `motion.rs` consumers | Everything pops. The editor feels like a form, not an instrument. This alone accounts for most of the "cheap" impression. |
| G02 | No entrance choreography anywhere | S: `play_staggered` unused | A folder of 200 tiles appears as one hard cut; a Details rebuild is a flash. |
| G03 | No backdrop blur; modal scrim is a flat 62 % black slab | S: `modal_scrim: rgba(0A,0B,0F,9E)`; `phase_27.md:31` records 27-D still open, blocked on surface `COPY_SRC` | Modals and the floating context bar do not separate from a bright scene. |
| G04 | Corners are circular arcs, not continuous curvature | S: `sd_rounded_box`, `ui_pass.wgsl:127` | Reads subtly "web default". A superellipse term is ~3 lines and is the cheapest large upgrade available. |
| G05 | 2–6 % washes band on dark surfaces; no dither | S: `GradientTokens` + 8-bit sRGB output | Visible steps across headers and toolbars at exactly the sizes the chrome uses. |
| G06 | No pointer-relative lighting | S: no such uniform in `ui_pass.wgsl` | Surfaces look printed. Wicked's editor gets much of its life from exactly this (§7). |
| G07 | No busy / indeterminate language | S: no looping track is permitted, none exists | PERSONA §4's state grammar demands "no endless anonymous spinner" but ships no alternative. |
| G08 | Icons are static alpha rasters | S: `icon_svg.rs`, atlas cells | No state morph (play↔pause, chevron↔caret, lock↔unlock). |
| G09 | Focus ring, selection rail and drop target are instant and static | S: `style.rs` recipes are pure functions of state | The strongest identity cues in the product are the ones that never move. |
| G10 | No designed moments | S/P | Launch, save, bake completion, mode arm are silent. Nothing is memorable. |
| G11 | `MotionProperty` has 5 members: no `OffsetX`, `ScaleY`, `Rotation`, `Flash`, `Stroke` | S: `motion.rs:39` | Several §13 items cannot be expressed until this closed enum is deliberately widened. |

---

## 4. The contract you must amend, not violate

**Read this before writing a line.** Somnium has a written, tested motion and
decoration contract. A "cool effects" pass that ignores it is a broken delivery
even if it looks good.

### 4.1 What is currently forbidden, and where

From [`phase_27.md`](../phase_27.md):

| Rule | Location | Text |
|---|---|---|
| Motion is causal | §5.5, §4.3 | "Nothing loops, nothing breathes, nothing decorates." |
| Explicit forbidden list | §9.3 | "looping animation, idle breathing, parallax, spring overshoot on anything a user scrubs, and any animation on a numeric field's value display" |
| Duration ceiling | §5.5 + `motion.rs:32` | `MAX_DURATION_MS = 200.0`, enforced inside `Animator::start`, not trusted to call sites |
| Idle frames are free | §5.6, §10.3 | Byte-identical draw list two frames running |
| Radius stays small | §5.2 | "Explicitly forbidden: large uniform corner radii, card-grid layouts, oversized padding, heavy layered shadows on every surface, and purple-gradient hero treatments." |
| Gradients | §5.3 | 2–6 %, linear space, chrome only. Never on body content, inputs, or text. |
| Glow | §5.4 | "Exactly two roles exist and no more may be added" — focus ring, armed mode. |
| Every visual is a token | §5.7 | No raw literal reaches a widget. |

The tests that enforce them:

```
src/lib.rs:9980     an_idle_shell_rebuilds_a_byte_identical_draw_list
src/draw.rs:1424    identical_frames_produce_identical_instance_bytes
src/lib.rs:9861     the_shell_actually_uses_the_new_paint_capabilities   (paint budget)
src/motion.rs:1276  an_idle_animator_reports_no_work_and_does_none
src/motion.rs:1286  a_finished_track_is_retired_so_idle_frames_stay_idle
src/a11y/tests.rs:369  reduced_motion_reaches_the_animator
theme::token_sheet_tests::json_sheets_match_the_shipped_snapshots   (token parity)
```

### 4.2 The amendment to propose — in writing, in slice H, before coding

Append a `> Amended by PERSONA-H` note at the head of `phase_27.md` §9.3 and §5.2–
§5.5. Do not edit the historical prose; append. The amendment is narrow and
enumerated:

| ID | Amendment | Justification | New guard |
|---|---|---|---|
| **A1** | Looping motion is permitted **only** while a real, named, external condition is true: (a) an indeterminate job is running, (b) a recording/live capture is active, (c) a thumbnail is decoding (skeleton shimmer). Each loop must stop within one frame of its condition ending. | §9.3 exists to forbid *decorative* loops and keep idle frames free. A job spinner is causal: the cause is the job. | New test `an_idle_shell_has_zero_live_motion_tracks`: build the shell with no jobs, tick 60 frames, assert `Animator::is_idle()`. Strictly stronger than today's guarantee; add it regardless. |
| **A2** | The 200 ms ceiling stays for anything the user is directly manipulating. Add a second named ceiling `MAX_TRAVEL_MS = 320` for exactly: modal enter, drawer travel, workspace transition, floating dock/undock. | Emil Kowalski's standard puts modals/drawers at 200–500 ms and everything else under 300 ms; Material 3 puts emphasised container transitions at 400–500 ms. 200 ms for a full-height drawer reads as a snap. **The codebase already broke the 200 ms rule for springs** — `MAX_SPRING_MS = 4_000` (`motion.rs:335`) — so the ceiling is already category-dependent in practice; A2 makes that honest. | `Animator::start` takes a category, or gains a second entry point; a test asserts no interaction-feedback track exceeds 200 ms. |
| **A3** | Two more glow roles: `glow.busy` (indeterminate sweep) and `glow.drop_valid`. Total four. | §5.4's "two only" predates drag/drop and jobs. Four named roles is still a closed set. **Preferred alternative if you would rather not amend:** express the drop target as an animated *border* + fill, not a glow, keeping the count at three. | Token parity covers the new roles; a lint keeps the count ≤4. |
| **A4** | Gradients may be used off-chrome for exactly three more purposes: the scroll-edge fade (already shipped), the veil behind a blurred surface, and the dither ramp. | Legibility devices, not decoration. | Paint-budget test updated with *named exemptions* rather than a raw count. |
| **A5** | `MotionProperty` widens by four members: `OffsetX`, `ScaleY`, `Flash`, `Stroke`. It remains a closed enum. | §11's flash cue and §13's shake/fold/rail entries cannot be expressed otherwise. | Each addition carries a doc comment naming its consumer; the enum stays closed. |

**Still forbidden after the amendment, permanently:** idle breathing on anything,
parallax, decorative particles, animated backgrounds, spring overshoot on a scrubbed
control, any animation of a numeric value display, animated brand marks in the
chrome, and motion that changes layout between reduced-motion modes.

---

## 5. Research — motion systems worth stealing rules from

None of these ship as code. They ship as *numbers and rules* that stop the
choreography table from being invented from scratch.

### 5.1 Emil Kowalski — the most directly applicable ruleset

[`emilkowalski/skills` → `review-animations/STANDARDS.md`](https://github.com/emilkowalski/skills/blob/main/skills/review-animations/STANDARDS.md),
[animations.dev](https://animations.dev/).

| Rule | Value |
|---|---|
| Button press feedback | 100–160 ms |
| Tooltips, small popovers | 125–200 ms |
| Dropdowns, selects | 150–250 ms |
| Modals, drawers | 200–500 ms |
| Global ceiling | "UI animations stay under 300 ms" |
| Ease-out (entries/exits) | `cubic-bezier(0.23, 1, 0.32, 1)` |
| Ease-in-out (on-screen movement) | `cubic-bezier(0.77, 0, 0.175, 1)` |
| Drawer (iOS) | `cubic-bezier(0.32, 0.72, 0, 1)` |
| **Never `ease-in` on UI** | it delays the exact moment the user is watching |
| Never `scale(0)` for entries | use `scale(0.9–0.97)` + `opacity` |
| Press feedback | `scale(0.97)`, 160 ms ease-out |
| Popovers scale from the trigger; modals from centre | transform-origin discipline |
| Stagger | 30–80 ms between items — "longer delays feel slow" |
| Spring bounce | keep subtle, 0.1–0.3 |
| Only animate transform + opacity | never layout properties |

**Somnium mapping.** `Easing::Decelerate` is currently
`1-(1-t)²` — a quadratic ease-out, weaker than `(0.23,1,0.32,1)`. Slice H should add
a real cubic-bezier evaluator (or bake the three curves above as authored
`Curve`s through the existing `Animator::register_curve` route — **that path needs
no new easing code at all**, which is the lazier and better answer).

### 5.2 Material Design 3 — token structure and the spring model

[Motion overview](https://m3.material.io/styles/motion/) ·
[Easing and duration tokens](https://m3.material.io/styles/motion/easing-and-duration/tokens-specs)

Duration ladder (ms): Short 50/100/150/200 · Medium 250/300/350/400 · Long
450/500/550/600 · ExtraLong 700–1000.

Easing tokens:

| Token | Control points | Use |
|---|---|---|
| `EasingEmphasizedDecelerate` | (0.05, 0.7, 0.1, 1.0) | entering |
| `EasingEmphasizedAccelerate` | (0.3, 0.0, 0.8, 0.15) | exiting |
| `EasingEmphasized` | (0.2, 0.0, 0.0, 1.0) | bidirectional |
| `EasingStandardDecelerate` | (0.0, 0.0, 0.0, 1.0) | simple enter |
| `EasingStandardAccelerate` | (0.3, 0.0, 1.0, 1.0) | simple exit |
| `EasingStandard` | (0.2, 0.0, 0.0, 1.0) | basic state shift |

The important structural idea for Somnium is **M3 Expressive's split between
`spatial` and `effects` springs**: spatial springs (position, size, rotation,
corner radius) may overshoot; effects springs (colour, opacity) never do. Somnium
has `Spring` but no such policy. Encode it: `Spring::SPATIAL_*` may overshoot,
`Spring::EFFECT_*` must satisfy `!overshoots()`, asserted by test. That single rule
prevents the most common way a spring system starts looking cheap.

### 5.3 IBM Carbon — the "productive vs expressive" split

[Carbon motion](https://carbondesignsystem.com/elements/motion/overview/) ·
[IBM/motion](https://github.com/IBM/motion)

Two families: **productive** (fast, for microinteractions that show the UI is
responsive) and **expressive** (slower, for moments where the system interrupts the
user). Duration scales *non-linearly with travel distance and size change*.

**Somnium mapping.** This is the closest published system to a DCC's needs, and it
resolves the A2 argument cleanly: an editor is a productive tool with a small
number of expressive moments (§11). Adopt the vocabulary directly — name the two
token groups `motion_ms.productive.*` and `motion_ms.expressive.*` — and adopt the
distance-scaled duration idea for drawer/panel travel so a 200 px slide and a
900 px slide do not take the same time.

### 5.4 Apple HIG and Fluent 2 — for the parts a DCC shares with an OS

- [Apple HIG — Motion](https://developer.apple.com/design/human-interface-guidelines/motion):
  motion must communicate, must be interruptible, must respect Reduce Motion by
  substituting a cross-fade rather than removing meaning.
- [Fluent 2 — Motion](https://fluent2.microsoft.design/motion) and
  [design tokens](https://fluent2.microsoft.design/design-tokens): raw vs semantic
  alias separation, already cited in [`phase_PERSONA.md`](../phase_PERSONA.md) §9.
  Fluent's duration/curve tokens are the closest structural match to Somnium's
  existing `motion_ms` block and should guide its expansion.

### 5.5 W3C, for the floor

- [WCAG 2.2 — Non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast)
- [WCAG 2.2 — Target size (minimum)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum)
- [Animation from interactions](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions)
  — the reduced-motion requirement, and why "disable" is the wrong answer and
  "substitute" is the right one.

### 5.6 The research literature — what is actually established

Design-system numbers are conventions. These are measurements. They matter because
they tell you which of the pretty ideas below are *load-bearing* and which are
taste.

| Work | Finding | What it decides here |
|---|---|---|
| Card, Robertson & Mackinlay, **"The Information Visualizer, an Information Workspace"**, CHI 1991 | The three human response-time constants: **0.1 s** (perceived as instantaneous / cause-and-effect), **1 s** (unbroken flow of thought), **10 s** (attention lost). | This is the real justification for the duration ladder, and it is stronger than any design-system table. Press feedback must land inside 0.1 s. Any operation over 1 s must show progress (§11.3). |
| Robertson, Mackinlay & Card, **"Cone Trees"**, CHI 1991 | Animated transitions let the viewer preserve **object constancy** — you track *which* thing moved instead of re-reading the whole view. | The core argument for animating list reorder, tree expand, panel dock and workspace switch rather than cutting. |
| Tversky, Morrison & Bétrancourt, **"Animation: can it facilitate?"**, IJHCS 57(4), 2002 — [PDF](https://www.cs.ubc.ca/~tmm/courses/cpsc533c-04-spr/readings/tversky.pdf) | Animation only helps when it satisfies the **Congruence** principle (the motion matches the conceptual change) and the **Apprehension** principle (it is slow and clear enough to be perceived). Decorative or over-complex animation measurably hurts. | This is §4's "motion is causal" rule, with evidence. Cite it in the amendment: the contract is not arbitrary conservatism. |
| Heer & Robertson, **"Animated Transitions in Statistical Data Graphics"**, InfoVis 2007 — [PDF](https://idl.cs.washington.edu/files/2007-AnimatedTransitions-InfoVis.pdf) · [project page](http://vis.stanford.edu/papers/animated-transitions) | Animated transitions significantly improve graphical perception; **staged** transitions (decomposing one change into ordered sub-transitions) outperform doing everything at once. | Directly supports staging a Details rebuild: fade out → reflow → fade in, rather than one cross-fade. |
| Chevalier, Dragicevic & Franconeri, **"The Not-so-Staggering Effect of Staggered Animated Transitions on Visual Tracking"**, TVCG 2014 — [PDF](http://www.cs.toronto.edu/~fchevali/fannydotnet/resources_pub/pdf/notsostaggering-infovis14.pdf) | Staggering start times **does not** measurably improve object tracking, and can hurt at high stagger amounts. | **Read this before over-investing in stagger.** Stagger in this document is justified on *aesthetic* and *load-perception* grounds only, never on "it helps you track things". Keep stagger small (≤30 ms/item, ≤240 ms total) and never stagger anything the user must track. |
| Dragicevic, Bezerianos, Javed, Elmqvist & Fekete, **"Temporal Distortion for Animated Transitions"**, CHI 2011 | Slow-in/slow-out and similar temporal distortions measurably aid tracking versus linear timing. | Why `Easing::Linear` is banned for anything but a looping indicator. |
| Bartram, Ware & Calvert, **"Moticons: detection, distraction and task"**, IJHCS 58(5), 2003 | Motion is an extremely strong preattentive channel — it is *detected* in the periphery far better than colour or shape, which is exactly why it is distracting when misused. | The argument for A1's narrowness: a looping element anywhere on screen steals attention from the viewport continuously. One busy indicator, bounded, is the budget. |

Practical rendering references (articles, not papers, but load-bearing):

- Evan Wallace, **[Fast Rounded Rectangle Shadows](https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/)** — closed-form Gaussian×box convolution via `erf`, sampled in the second axis. This is what Figma and Zed use.
- Raph Levien, **[Blurred rounded rectangles](https://raphlinus.github.io/graphics/2020/04/21/blurred-rounded-rects.html)** — a cheaper closed-form approximation of the same thing.
- Marius Bjørge, **"Bandwidth-Efficient Rendering"**, SIGGRAPH 2015 — the dual-filter (dual Kawase) blur. [Reference implementation](https://github.com/Baedrick/Dual-Kawase-Blur-Demo).
- Jorge Jimenez, **"Next Generation Post Processing in Call of Duty: Advanced Warfare"**, SIGGRAPH 2014 — the progressive downsample + tent upsample chain. **Somnium already implements this in `pass/bloom.rs`.**
- Ryan Juckett, **[Damped Springs](https://www.ryanjuckett.com/damped-springs/)** — the analytic damped-harmonic solution with cached coefficients. Relevant because `motion.rs`'s `Spring` should be checked against it for stability at large `dt`.
- Björn Ottosson, **[Oklab: a perceptual color space for image processing](https://bottosson.github.io/posts/oklab/)** and Raph Levien's [critique](https://raphlinus.github.io/color/2021/01/18/oklab-critique.html) — why hover/pressed ramps and gradient stops should interpolate in Oklab/Oklch, not sRGB or linear RGB.
- Christoph Peters, **[Free blue noise textures](https://momentsingraphics.de/BlueNoise.html)** — if you choose textured dither over an ordered Bayer matrix (§10.5).
- Inigo Quilez, **[2D distance functions](https://iquilezles.org/articles/distfunctions2d/)** — the reference for every SDF shape you may want beyond the rounded box.

---

## 6. Research — libraries, and why the answer is "add none"

The user asked for library research. Here it is, honestly evaluated. The verdict
column is what matters.

### 6.1 The example the user cited: tachyonfx

[ratatui/tachyonfx](https://github.com/ratatui/tachyonfx) ·
[docs.rs](https://docs.rs/tachyonfx) · [ratatui.rs/ecosystem/tachyonfx](https://ratatui.rs/ecosystem/tachyonfx/)

A shader-like effects library for **terminal** UIs: it mutates terminal cells after
widgets render. Not applicable as a dependency — Somnium is a GPU-rasterised
retained UI, not a cell grid.

**What is worth taking is its API shape and its vocabulary**, which is genuinely
good and maps onto Somnium's `Transition`:

- Composition combinators: `parallel()`, `sequence()`, `delay()`, `repeat()`,
  `ping_pong()`, `with_duration()`, `prolong_start()`, `prolong_end()`,
  `never_complete()`, `freeze_at()`.
- Effect verbs: `fade_from`/`fade_to`, `hsl_shift`, `coalesce`, `dissolve`,
  `sweep_in`/`sweep_out`, `slide_in`/`slide_out`, `expand`, `stretch`, `translate`.
- **Spatial patterns** that control *when each cell in a region starts*: radial,
  sweep, wave, diagonal, checkerboard.

That last one is the real idea. Somnium's `play_staggered` takes a flat node list
and a fixed delay. Generalising the delay to a **spatial function of the node's
rect** — `radial from the click point`, `sweep left-to-right`, `diagonal` — turns
one API into every list/grid entrance in the editor for about 40 lines. Do that in
slice H (`StaggerPattern`), and credit the idea.

### 6.2 Animation crates evaluated

| Crate | What it is | Verdict | Why |
|---|---|---|---|
| [`bevy_tweening`](https://docs.rs/bevy_tweening/) / [`bevy_tween`](https://docs.rs/bevy_tween/) | Tweening plugins for Bevy | **Reject** | ECS-coupled to Bevy. Somnium has its own ECS and its own animator. |
| [`keyframe`](https://crates.io/crates/keyframe), [`interpolation`](https://crates.io/crates/interpolation), [`simple-easing`](https://crates.io/crates/simple-easing) | Easing/lerp helpers | **Reject** | `motion.rs` already has easings and springs. Adding a crate to lerp an `f32` is a dependency for four lines. |
| [`spanda`](https://github.com/aarambh-darshan/spanda) | `no_std` animation engine: 38+ easings, keyframe tracks, timelines, damped-harmonic springs, motion paths | **Reject as a dependency; read its timeline API** | Its sequence/timeline composition is the one thing `Transition` lacks. Borrow the shape, not the code. |
| [Animato](https://lib.rs/crates/animato-tween) (`animato-*` family) | Easing, tweens, timelines, springs, motion paths, perceptual colour interpolation, stagger patterns, GPU batch evaluation | **Reject as a dependency; note the perceptual-colour point** | `motion::lerp_color` currently lerps in sRGB. Lerping a hover wash in Oklab instead removes the muddy midpoint. That is a 20-line change in one function, not a dependency. |
| [`rive-rs`](https://github.com/rive-app/rive-rs) (Rive runtime, wgpu/Bevy) | GPU vector animation with **state machines** and two-way interactivity | **Reject for chrome. Reconsider only for a splash/brand moment** | Pulls a second vector renderer and a second authoring tool. Somnium already has `path.rs` + the shaped pipeline. If an animated brand mark ever becomes a hard requirement, this is the correct tool for exactly that one asset — not for the editor. |
| [`velato`](https://github.com/linebender/velato) + [`vello`](https://github.com/linebender/vello) | Lottie playback on a compute-based vector renderer | **Reject** | Vello is an entire GPU rasterizer. For a handful of animated icons, morph the existing paths. |
| [`lyon`](https://github.com/nical/lyon) | Path tessellation | **Reject** | `src/path.rs` already flattens and strokes with a tolerance knob. |
| [`taffy`](https://github.com/DioxusLabs/taffy) | Flexbox/grid layout | **Reject** | Layout is not the problem and a layout swap would break every saved workspace. |
| [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (Zed) | Rust GPU UI framework | **Not a dependency — a reference** | Its quad shader (corner radii, borders, shadows) is architecturally the same as `ui_pass.wgsl`, and [its blog post](https://zed.dev/blog/videogame) documents the approach. Its [backdrop-blur design discussion](https://github.com/zed-industries/zed/discussions/47429) states the exact conclusion §10.3 reaches: *"introduce a BlurRegion primitive that tells the renderer to split the draw list, render-to-texture, blur, then resume."* |
| [`bevy_ui_render`](https://github.com/bevyengine/bevy/tree/main/crates/bevy_ui_render) | Bevy's wgpu UI renderer | **Not a dependency — the closest readable reference** | Same language, same API, same problem. Read `ui.wgsl` (SDF border + inset-rounded-box + its comment that `fwidth`-based AA caused artifacts), `gradient.wgsl` (**linear + radial + conic**, with interpolation in sRGB / linear / **Oklab / Oklch** / HSL and short/long hue paths), and `box_shadow.wgsl` (Evan Wallace's erf technique). Every one of those is directly portable to Somnium's `Primitive`. |
| [`fyrox-ui`](https://github.com/FyroxEngine/Fyrox/tree/master/fyrox-ui) | Fyrox's retained-mode Rust UI | **Not a dependency — the closest architectural reference** | A retained widget tree with a message bus, a `Brush` enum (Solid / LinearGradient / RadialGradient with stop lists), a centralised `Style` resource of `StyleProperty` values, and a `Decorator` widget that holds one brush per state. Its most interesting idea is `animation.rs`: **a UI `AnimationPlayer` node that plays the engine's ordinary animation tracks against reflected widget properties** — see §9 option C. |
| [Makepad](https://github.com/makepad/makepad) | Rust UI with per-widget shaders and a DSL animator | **Reference only** | Worth reading for how a shader-per-widget system keeps animation declarative. Adopting it means replacing the UI framework — explicitly out of scope per [`phase_PERSONA.md`](../phase_PERSONA.md) §10. |
| [Slint](https://slint.dev/) | Declarative UI with built-in animations | **Reference only** | Same reason. |

### 6.3 The verdict

**Add zero dependencies.** Everything §8–§13 requires is either already in
`somnium_ui`, already in `somnium_renderer`, or is a few dozen lines of new code in
`motion.rs` / `style.rs` / `ui_pass.wgsl`. Adding a crate here would trip GHOSTFENCE's
"no second system" row and would buy nothing.

If you disagree with any single verdict above, state which one and why in your
delivery record — do not silently add a dependency.

---

## 7. Reference engines studied — what each one actually does

Local reference collection: `C:/Users/adhir/Downloads/GE/example_repo`. **No code is
copied.** These are techniques, read and summarised, cited so the implementation
can be attributed if it later borrows an architectural idea (per
[`ATTRIBUTION.md`](../../ATTRIBUTION.md)). Folder names are local checkouts, not
verified upstream revisions.

Read order if you only have time for three: **Blender (§7.5) for what a
professional DCC widget actually looks like, Bevy (§7.6) for shader technique you
can port line-for-line, Wicked (§7.1–7.4) for the lighting model that makes an
editor feel alive.**

---

### 7.1 Wicked Engine — the signature effect: a rotating angular highlight

`WickedEngine/wiGUI.cpp`, `Widget::Update` / `Widget::Render`:

- Every widget carries `angular_highlight_timer += dt`.
- On render, when `angular_highlight_width > 0`, it draws an expanded copy of the
  widget behind itself, with the same corner rounding, using
  `angular_softness_direction = normalize(vec2(sin(t), cos(t)))` and an outer angle
  of `π·0.6`.
- The result is a **soft conic wedge of light that rotates slowly around the border
  of the focused widget.** It is the thing that makes Wicked's editor look alive
  without looking busy.
- A second variant blinks it: `Lerp(0, 0.24, abs(sin(resize_blink_timer * 4)))` on
  resize handles — a bounded attention cue, not decoration.

**Somnium translation.** `ui_shaped.wgsl` already has `GRAD_ANGULAR` with a centre
and a start angle in local space. An angular sweep ring is buildable **today**
through `push_path` + `ShapedInstance` with no shader edit. Use it for exactly two
roles: the indeterminate busy state (§11.3) and the armed-tool ring (§13). Under A1
it loops only while its condition holds.

### 7.2 Pointer-tracked highlight

`wi::image::Params` (`WickedEngine/wiImage.h`) carries `HIGHLIGHT` with
`highlight_color`, `highlight_spread`, `highlight_pos` (screen-UV). `Widget::Update`
feeds every sprite the current pointer position, so surfaces light up *relative to
the cursor* rather than uniformly.

**Somnium translation.** One `vec2` in the existing UiPass uniform block plus a
`FLAG_HIGHLIGHT` radial term in `ui_pass.wgsl` — about 12 lines total, one uniform
upload per frame, zero extra instances. This is the highest beauty-per-line item in
the whole document (§10.4).

### 7.3 The rest of the `wi::image::Params` feature set, as a checklist

Worth reading in full as "what a game-engine UI shader offers that Somnium's does
not yet":

| Feature | Somnium status | Recommendation |
|---|---|---|
| `corners_rounding[4]` with per-corner radius | Have it (`radii[4]`) | — |
| `border_soften` (0–1 alpha softening at the edge) | Not present | Consider as a cheaper alternative to squircles (§10.2 option C) |
| `gradient` = None / Linear / **LinearReflected** / **Circular** | Linear only in the quad pipeline | Add `LinearReflected` for the inner-light hairline; circular is already in the shaped pipeline |
| `mask_alpha_range_start/end` | Mask slot exists in shaped only | This is how you get a **wipe reveal** for free — animate the range, not the geometry |
| `saturation`, `fade`, `opacity`, `intensity` | Partial | `fade` = the disabled treatment; already covered by `opacity.disabled` |
| `distort` via mask RG on a background sample | Not present | **Reject.** Refraction on a productivity tool is decoration. |
| `WobbleAnim` (per-corner sine displacement) | Not present | **Reject** for chrome — this is exactly the "idle breathing" §9.3 forbids. |
| `MovingTexAnim` (scrolling texture) | Not present | Use for one thing only: a determinate progress stripe, if a progress bar needs texture. Prefer not. |
| Typewriter font animation (`font.anim.typewriter`, used on Wicked's loading screens) | Not present | **Consider for one place only:** the load/bake status line (§11.4). Never for UI labels. |

### 7.4 What not to take from Wicked

Its widget-per-sprite architecture, its immediate-mode window stacking, its
loading-screen art direction, and its "everything wobbles" sprite defaults. Somnium
is a retained-tree editor with a written restraint contract; Wicked is a
demo-friendly engine editor. Take the *lighting model*, leave the *ornament*.

---

### 7.5 Blender (via `upbge-master`) — the most important DCC reference

`source/blender/editors/interface/`. Blender's UI is the most widely respected
dense-editor UI in the industry, and its source is unusually instructive.

**7.5.1 The widget model — `interface_widgets.cc` (6,794 lines) and
`gpu_shader_2D_widget_base.bsl.hh`.** Blender draws a widget as **one instanced
primitive with four composited masks**, structurally identical to Somnium's
`Primitive`:

1. an **inner fill** with a vertical two-stop gradient (`shade_dir`, `shadetop`/`shadedown` deltas on the theme colour),
2. an **outline** band,
3. an **emboss** — a light 1 px line offset *downward* by the line width, and
4. an optional **tria** (the little glyph: arrow, checkmark, dropdown chevron), batched in the same call.

The shader resolves all of them from one SDF:

```
masks.x = smoothstep(-aa_radius, aa_radius, sdf);                        // inner
masks.y = smoothstep(-aa_radius, aa_radius, sdf - line_width);           // outline
masks.z = smoothstep(-aa_radius, aa_radius, sdf + line_width * emboss);  // emboss
```

and the emboss band is faded out near the corners and suppressed on the upper half:

```
emboss_size = upper_half ? 0.0 : min(1.0, uv_sdf.x / (corner_rad * ratio));
```

**This is the single most transferable recipe in the whole reference collection.**
That one extra band — a ~6–10 % white line along the bottom inner edge of a
control, fading into the corners — is most of the difference between "flat
rectangle with a fill" and "machined component". It is a theme colour in Blender
(`TH_WIDGET_EMBOSS`), which means it is a **token**, satisfying §5.7. See §10.2.

**7.5.2 State is a blend, not a swap.** `widget_state()` /
`widget_color_blend_from_flags()` blend the base colour *toward* a state colour by
a factor, rather than substituting a different colour per state. A blend factor is
directly animatable; a colour swap is not. Somnium's `style.rs` currently returns a
different `Paint` per `VisualState` — closer to a swap. Consider expressing state
as `(base_paint, blend_target, factor)` so the factor is the animated quantity.

**7.5.3 Blender animates panels, at 300 ms.** `interface_panel.cc:65`:

```c
#define ANIMATION_TIME     0.30
#define ANIMATION_INTERVAL 0.02
```

Panel open/close and drag-realign both animate over 300 ms
(`PANEL_STATE_ANIMATION`, `do_animate()`), and are explicitly suppressed while
other interaction is being handled ("Don't animate while handling other
interaction"). **This is a hard data point from a shipping professional DCC that
exceeds Somnium's 200 ms ceiling**, and is the strongest single argument for
amendment A2. The "don't animate during other interaction" rule is worth copying
verbatim as a policy.

**7.5.4 What not to take.** Blender's colour theme, its specific proportions, its
tooltip behaviour, and its immediate-mode `uiBlock` architecture.

---

### 7.6 Bevy — portable shader technique in the same language

`crates/bevy_ui_render/src/`. Same language, same graphics API, same problem.

| File | What to take |
|---|---|
| `ui.wgsl` (228 lines) | `sd_rounded_box` and, importantly, **`sd_inset_rounded_box`** — the correct way to compute an inner border with per-corner radii, which Somnium currently approximates with `border_width`. Also note the comment on its `antialias()` helper: *"Using the fwidth(distance) was causing artifacts, so just use the distance."* Somnium's `ui_pass.wgsl:173` uses `fwidth(d) * 0.5`; if you see shimmer at fractional DPI, this is the known cause and the known fix. |
| `gradient.wgsl` (258 lines) | Linear, **radial** and **conic** gradients in a UI shader, plus interpolation in sRGB / linear / **Oklab / Oklch** / HSL with short-and-long hue paths (`mix_hsl`, `mix_hsl_long`, `oklab_to_linear_rgb`). The conic path is `conic_distance(dir, point, start)` — that is the Wicked angular sweep, in WGSL, ready to read. |
| `box_shadow.wgsl` (98 lines) | Evan Wallace's shadow: a 4th-degree polynomial `erf` approximation, closed-form along x, numerically integrated along y with a Gaussian weight. **Strictly more correct than Somnium's current `smoothstep(-soft, soft, d_sh)` approximation** (`ui_pass.wgsl:197`). Whether the difference is visible at Somnium's 8–32 px elevation blurs is worth one A/B before spending the ALU. |
| `bevy_ui/src/interaction_states.rs`, `focus.rs` | A clean separation of interaction state from paint, and an accessibility-aware focus model. |

---

### 7.7 Fyrox — the closest architectural sibling

`fyrox/Fyrox-master/fyrox-ui/src/`. A retained-mode Rust UI with a message bus —
the same shape as `somnium_ui`.

- **`brush.rs`**: `Brush::{Solid, LinearGradient{from,to,stops}, RadialGradient{center,stops}}`. Somnium's `Primitive` supports exactly two stops on one linear axis. An **N-stop brush** is the natural next step and is what makes a "polished" gradient possible (a 3-stop wash with a soft midpoint does not band the way a 2-stop does).
- **`style/mod.rs`**: a centralised `Style` resource of `StyleProperty::{Number, Thickness, Color, Brush, …}` that widgets read by name, with per-widget overrides. Somnium's `theme.rs` + `style.rs` already occupy this space; the fyrox model is worth reading for how it handles *inheritance and overrides* if PERSONA ever wants user-editable themes.
- **`decorator.rs`**: one widget holding `normal_brush` / `hover_brush` / `pressed_brush` / `selected_brush`. A clean, boring, correct state model.
- **`animation.rs`**: **the interesting one.** Fyrox exposes an `AnimationPlayer` *widget node* that runs the engine's ordinary animation `Track`s against reflected UI properties (`ValueBinding`, `BoundValueCollection`). The UI reuses the scene animation system rather than owning a second one. Somnium has all three pieces already — `somnium_ecs::curve::Curve`, `Animator::register_curve`, and `reflect_registry.rs`. See §9 option C for whether that is worth doing here (recommendation: **not yet**).

---

### 7.8 s&box — how a shipping engine models backdrop blur

`engine/Sandbox.Engine/Systems/UI/Render/PanelRenderer.Backdrop.cs`. s&box's UI is
CSS-driven, and it implements `backdrop-filter` for real. The design is exactly
what GPUI's discussion proposes and what §10.3 recommends:

During draw-list construction, a panel with a backdrop filter appends a
**descriptor** to the current render layer rather than drawing anything:

```csharp
target.Backdrops.Add( new BackdropDrawDescriptor( rect ) {
    Radii, Opacity, Brightness, Contrast, Saturate, Sepia, Invert, HueRotate,
    BlurScale, OverrideBlendMode, IsLayered
} );
```

The renderer then processes the collected backdrop regions as a batch — split the
list, resolve the target, filter the regions, resume. **Take the descriptor
pattern.** It keeps the widget code ignorant of render-graph mechanics, which is
what lets the blur degrade to a flat scrim (§10.3 option D) without touching a
single widget. Note also the filter set: blur is only one of seven; a **saturate +
brightness** adjustment behind a scrim is much cheaper than a blur and gets a
surprising amount of the separation.

---

### 7.9 Godot — theme architecture and an easing catalogue

- `scene/resources/style_box_flat.cpp`: `corner_radius` per corner, `corner_detail` (1–20 tessellation segments), `anti_aliasing_size`, `skew`, `shadow_size`, `border_blend`. It is a **mesh-based** approach — Godot tessellates the rounded box rather than evaluating an SDF. Somnium's SDF approach is better; the transferable part is the **token vocabulary** (a separate anti-aliasing size knob is a good idea, and Somnium has no equivalent).
- `editor/themes/editor_theme_manager.cpp` (already cited in [`phase_PERSONA.md`](../phase_PERSONA.md) §9): independently configurable spacing, base colour, accent colour, contrast, icon saturation, corner radius and border width. The lesson stands — **density, colour and geometry are separate user-facing dimensions.**
- `scene/animation/tween.h`: 11 transition types (`LINEAR, SINE, QUINT, QUART, QUAD, EXPO, ELASTIC, CUBIC, CIRC, BOUNCE, BACK, SPRING`) × 4 ease types (`IN, OUT, IN_OUT, OUT_IN`). Somnium ships 5 easings + springs + authored curves, which is *enough*; do not expand to 44 combinations. Included here so you can see the ceiling and deliberately stop below it.

---

### 7.10 Flax — the editor-workflow reference

`Source/Editor/GUI/`, already cited in [`phase_PERSONA.md`](../phase_PERSONA.md) §9
for `ContentWindow.Navigation.cs`. Flax's editor is C# with a custom retained UI and
is worth reading for *workflow* rather than rendering: `AssetPicker.cs`,
`ContextMenu/`, `CurveEditor.*`, `PropertiesList.cs` and
`CustomEditors/GUI/DraggablePropertyNameLabel.cs` (drag a property *label* to
create a binding — a QoL idea worth stealing, see §15).

---

### 7.11 Unity uGUI — the state-transition taxonomy, and one number

`Runtime/UGUI/UI/Core/Selectable.cs` and `ColorBlock.cs`.

- `Transition { ColorTint, SpriteSwap, Animation }` — the three ways a control can
  express state, named. Somnium does ColorTint only, which is correct for a DCC.
- `ColorBlock.defaultColorBlock` ships `fadeDuration = 0.1f` — **100 ms** for a
  state colour cross-fade, in the engine with the largest install base. Somnium's
  `hover_ms = 120` is in the same place. That agreement is worth recording: the
  hover number is not a guess.

---

### 7.12 Spartan, Overload, Esoterica, Prowl — the "professional look" sanity check

- **Spartan Engine** (`source/editor/ImGui/ImGui_Style.h`): `WindowRounding 0`,
  `ChildRounding 0`, `PopupRounding 0`, `FrameRounding 1`, `GrabRounding 1`,
  `FrameBorderSize 0`, `WindowPadding (6,4)`, `ItemSpacing (6,3)`. One of the
  best-looking open-source engine editors, and it is **almost square with tight
  spacing**. This independently corroborates [`phase_27.md`](../phase_27.md) §5.2's
  "radius stays small" and is a useful counterweight to every web-design skill that
  will tell you to use `rounded-[2rem]`. Somnium's 4/5/6/8/10 ladder is already
  slightly *more* rounded than this; do not increase it.
- **Prowl** (`Prowl.Editor/Theming/EditorThemeData.cs`): the theme is built from
  **7-stop colour ramps per hue family** (`Neutral, Purple, Blue, Red, Green, Amber,
  Ink`), each with a bright C500 primary and user overrides — the Radix/Tailwind
  scale model applied to an engine editor. Somnium already has `theme::ramp_step`
  (`theme.rs:838`) implementing exactly this idea, **and nothing uses it** (§10.6).
  Prowl also ships a `Nebula` animated editor background as one of three background
  styles — an example of a "moment" that is opt-in and confined to empty space.
- **Overload** and **Esoterica**: Dear ImGui editors. Low transferable value for
  rendering; useful only as a reminder of what the default looks like, and why
  Somnium's custom paint is worth the effort.

---

## 8. The work — five slices, and how to choose

| Slice | Name | Delivers | Rough size | Depends on |
|---|---|---|---|---|
| **PERSONA-H** | *Charon II* — motion wiring | The animator reaches every surface; §14's table implemented; the contract amendment written; the `forget_node` fix | Large but low-risk; mostly call sites | Nothing |
| **PERSONA-I** | *Erebus II* — surface, material and light | Emboss, squircle option, dither, pointer light, backdrop blur, Oklab ramps | Medium; touches two shaders and the token sheets | Nothing (independent of H) |
| **PERSONA-J** | *Moments* — stateful feedback | Busy/indeterminate language, drop feedback, change flash, save/complete beats, empty states | Medium | H (needs tracks), I (needs the sweep) |
| **PERSONA-K** | *QoL* — §15's backlog | Whichever items you and the user pick | Sized by selection | Independent |
| **PERSONA-L** | *Proof* — performance, accessibility, evidence | Budgets measured, a11y matrix, one batched capture run, records updated | Small but mandatory | All of the above |

**H and I are independent and either can go first.** If you want the fastest visible
win, do **I first** — emboss + dither + pointer light is roughly a day and changes
every screenshot. If you want the largest total improvement, do **H first**,
because continuity is what makes an editor feel professional and it is the biggest
single gap (§3, G01).

### 8.1 Decisions that are yours to make

Every option table below carries a **recommendation**, but the recommendation is
advice, not an instruction. Where you disagree, say so in the delivery record with
one line of reasoning and take the other option. The decisions genuinely open:

1. **Order.** H-then-I, or I-then-H (§8 above).
2. **The easing upgrade route** — bake curves vs. add a bezier evaluator vs. adopt
   Fyrox's reflected-track model (§9.1).
3. **Squircles or not** (§10.1). This is a taste call with a real cost; the user
   should probably see an A/B before it lands.
4. **Backdrop blur route** — four options, very different costs (§10.3).
5. **How much of the choreography table to attempt in one pass** (§14). Half the
   table, landed and consistent, beats all of it, landed inconsistently.
6. **Which QoL items** (§15). That list is deliberately longer than one slice.

Where a decision changes what the user sees, prefer to build the cheap version,
capture it, and ask — rather than building the expensive version and defending it.

---

## 9. PERSONA-H — *Charon II*: make the animator reach the editor

**The premise:** almost none of this is new engineering. `motion.rs` already does
the hard part. H is (a) one contract amendment, (b) three small additions to the
engine, and (c) roughly thirty call sites.

### 9.1 Better curves — three routes

Somnium's `Easing::Decelerate` is `1-(1-t)²`, a quadratic ease-out. Every reference
in §5 uses something sharper: `cubic-bezier(0.23, 1, 0.32, 1)` (Emil),
`(0.05, 0.7, 0.1, 1.0)` (M3 emphasised decelerate). The difference is very visible —
the sharp curves are what make motion read as "fast and settled" rather than "slow
and gluey".

| Option | How | Cost | Risk |
|---|---|---|---|
| **A — bake authored curves** *(recommended)* | Build the three or four bezier curves as `somnium_ecs::curve::Curve` at theme-load time and register them via the existing `Animator::register_curve`. Reference them by `CurveId` from the motion tokens. | ~40 lines. **No new easing code, no new enum variant, no shader change.** | Near zero. The `Easing::Curve` path already exists and is tested. |
| **B — add a real cubic-bezier evaluator** | `Easing::Bezier(f32, f32, f32, f32)` with Newton-Raphson + bisection fallback for `x → t`. | ~60 lines plus tests for monotonicity and endpoint exactness. Makes `Easing` bigger than `Copy`-friendly 4 bytes. | Low, but you must prove `f(0)==0, f(1)==1` exactly or reduced motion drifts. |
| **C — adopt Fyrox's reflected-track model** | UI animation becomes engine animation tracks bound to reflected widget properties (§7.7). | Large. Touches `reflect_registry`, the UI node model and serialisation. | High, and it buys authoring flexibility this phase does not need. **Reject for now**; note it as the route if user-authorable UI animation is ever a requirement. |

Whichever route: keep the existing named easings working. They are used by the two
shipped call sites and by tests.

### 9.2 Spring policy — adopt M3's spatial/effects split

Add to `motion.rs`:

- `Spring::SPATIAL_{FAST,DEFAULT,SLOW}` — may overshoot; used for position, size,
  scale, fold height, dock geometry.
- `Spring::EFFECT_{FAST,DEFAULT,SLOW}` — must satisfy `!overshoots()`; used for
  colour, opacity, wash, glow.
- A test asserting every `EFFECT_*` constant returns `false` from the existing
  `Spring::overshoots()`.

This is ~30 lines and it is the difference between springs looking expensive and
springs looking cheap. Cross-check the integrator against
[Ryan Juckett's analytic solution](https://www.ryanjuckett.com/damped-springs/) for
stability at the 100 ms `dt` clamp — a naive semi-implicit Euler spring can visibly
misbehave on a stalled frame, and `lib.rs:2410` explicitly allows a 100 ms step.

### 9.3 Stagger patterns — generalise the delay

Today `start_staggered(nodes, …, stagger_ms)` applies `i * stagger_ms`. Generalise
the delay to a function of the node's rect, the way tachyonfx does with its spatial
patterns (§6.1):

```rust
pub enum StaggerPattern {
    Index,                    // current behaviour
    Radial { origin: Vec2 },  // outward from the click / drop point
    Sweep  { dir: Vec2 },     // left-to-right, top-to-bottom, diagonal
}
```

~40 lines, and it covers every list and grid entrance in the editor. **Constrain it
hard on the evidence in §5.6:** stagger is aesthetic, not functional — cap at
**30 ms per item and 240 ms total**, and never stagger content the user is trying
to track.

### 9.4 Widen `MotionProperty` (amendment A5)

Add `OffsetX`, `ScaleY`, `Flash`, `Stroke`. Each with a doc comment naming its
consumer, per the existing style of that enum. Keep it closed.

### 9.5 Fix the node-recycling defect (§2.5)

Call `Animator::forget_node` from node destruction. Regression: destroy a node with
a live track, allocate a new node that reuses the index, assert it starts at rest.

### 9.6 A `motion::policy` module — where the "when" lives

The one genuinely new piece of design. Right now a widget that wants motion has to
know a duration, an easing and a key. That is how you end up with two call sites in
1,465 lines. Give widgets a vocabulary instead:

```rust
// somnium_ui::motion::policy
pub fn hover(ctx, key, on: bool);
pub fn press(ctx, key, down: bool);
pub fn enter_popup(ctx, node, anchor: Anchor);
pub fn exit_popup(ctx, node);
pub fn travel(ctx, node, axis: Axis, distance: f32);   // distance-scaled, per Carbon
pub fn select(ctx, key, on: bool, focused: bool);
pub fn flash(ctx, key, kind: FlashKind);
pub fn busy(ctx, node, active: bool);                  // the only looping entry point
```

Each resolves tokens, category ceilings, reduced motion and the spring policy in one
place. Widgets then call `policy::enter_popup(...)` and cannot get it wrong. This is
what makes §14 tractable — thirty call sites of one line each, instead of thirty
bespoke animations.

**Exit condition for H.** Every row of §14 marked *H* is implemented through
`motion::policy`; `an_idle_shell_has_zero_live_motion_tracks` passes; reduced motion
produces identical layout for every new property (extend the existing assertion);
the contract amendment is written into `phase_27.md`; `cargo test -p somnium_ui -j 1`
is green.

---

## 10. PERSONA-I — *Erebus II*: surface, material and light

Ordered by **beauty gained per line of code**, best first.

### 10.1 The emboss line — highest value, lowest risk

From Blender (§7.5.1). A 1 px light line along the *inner bottom edge* of raised
controls, faded into the corners, suppressed on the upper half.

Somnium's shader already computes `d` and `radius`; this is one more `smoothstep`
and one more colour. Add `border.emboss` to the token sheets (Nocturne ~8 % white,
Dawn ~40 % white toward the surface, high-contrast off) and a `FLAG_EMBOSS`.

**Apply to:** raised buttons, primary buttons, toggles at rest, tabs, the mode rail.
**Never apply to:** panel bodies, inputs (they are recessed — they get the existing
`inset` instead), text, or anything flat by contract.

Expected cost: ~15 lines of WGSL, ~10 lines of Rust, two token entries. Expected
effect: large. This is the "machined hardware" cue.

### 10.2 Corner shape — squircle or not

| Option | How | Cost | Notes |
|---|---|---|---|
| **A — leave circular** | Nothing | 0 | Defensible. Spartan, Blender and Godot all ship circular corners and look professional. |
| **B — superellipse in the SDF** *(recommended to prototype, user decides)* | Replace the corner term's `length(q)` with a power metric: `pow(pow(qx,n) + pow(qy,n), 1/n)`, `n≈4` ≈ Figma corner-smoothing 0.6. Add `radius.smoothing` as a token so `n` is authored, not hard-coded. | ~3 lines WGSL + 1 token. One `pow` per fragment, only inside the corner region. | Continuous curvature — the iOS/Figma look. Most visible at the larger radii (popup 8, modal 10, tile 6) and nearly invisible at input 4. See [the superellipse math](https://squircle.js.org/blog/math-behind-squircles) and [Figma corner smoothing](https://help.figma.com/hc/en-us/articles/360050986854-Adjust-corner-radius-and-smoothing). |
| **C — `border_soften` instead** | Wicked's approach (§7.3): soften alpha near the edge by a normalised amount rather than changing the shape. | ~4 lines | Cheaper and subtler. Gets some of the optical benefit without changing geometry. Worth trying first if B looks too "consumer". |

**Do not** increase the radius values themselves. §5.2 forbids it and §7.12 shows
the professional consensus is *smaller*, not larger.

### 10.3 Backdrop blur — the 27-D debt, four routes

The blocker on record (`phase_27.md:31`) is that the swapchain surface only
conditionally supports `COPY_SRC`. **Three of the four routes below do not need it.**

| Option | How | Cost | Verdict |
|---|---|---|---|
| **A — sample the present offscreen** *(recommended)* | `pass/present.rs` already owns `src_texture` + `src_view`, an offscreen the scene is rendered into before upscale. Make it unconditional (or add a small dedicated `ui_backdrop` target), give `UiPass` its view, add `FLAG_BACKDROP` sampling a blurred mip. | Moderate. No new pass if you sample an SPD-built mip; one small pass if not. | Sidesteps the `COPY_SRC` blocker entirely. Reuses `pass/spd.rs`. |
| **B — reuse the bloom chain** | `pass/bloom.rs` is already a progressive downsample + tent upsample (Jimenez); run the same shaders over an LDR copy to build a blur pyramid. | Moderate; mostly plumbing. | Zero new algorithm, zero new shader family. Strong second choice, and the right one if A's target lifetime turns out to be awkward. |
| **C — a dedicated dual-Kawase pass** | Implement [Bjørge 2015](https://github.com/Baedrick/Dual-Kawase-Blur-Demo) fresh at half resolution. | Highest. New pass, new shaders, new budget line. | Only if A and B both fail. GHOSTFENCE's "one shader system" row will scrutinise this. |
| **D — no blur; scrim + filter + noise** *(the fallback that must always work)* | Keep the flat scrim; add a **saturate-down + brightness-down** of the backdrop (s&box exposes seven filters, not just blur — §7.8) or, with no backdrop sampling at all, a subtle vertical wash plus dither. | Very small. | **Build this first regardless.** It is the degradation path §9.4 of `phase_27.md` already requires ("disabling blur degrades to the current flat scrim with no layout change"), and it is most of the perceived separation for a fraction of the work. |

**Architecture, whichever route:** adopt the s&box / GPUI **descriptor** pattern.
Widgets append a `BackdropRegion { rect, radii, blur, saturate, brightness }` to the
draw list; the pass splits, filters and resumes. Widgets never learn about render
targets, and option D is a one-line change in the pass rather than a change in every
widget.

**Apply to exactly two surfaces**, per the existing contract: the modal scrim and
the floating viewport context bar. If you want a third, argue for it in the record.

### 10.4 Pointer light — the cheapest "alive" cue

From Wicked (§7.2). One `vec2` of pointer position in the existing UiPass uniform
block, a `FLAG_HIGHLIGHT` bit, and a radial term added to the fill:

```wgsl
// pseudo
let d_ptr = distance(in.world_pos, u.pointer_px) / u.highlight_spread;
rgb += highlight_color * (1.0 - smoothstep(0.0, 1.0, d_ptr)) * strength;
```

**Zero extra instances, one uniform upload per frame.** Apply at very low strength
(2–4 %) to: the toolbar band, panel headers, and the tile grid. **Not** to text,
inputs, or the viewport.

Caveat: this makes the draw output depend on pointer position, which will interact
with `an_idle_shell_rebuilds_a_byte_identical_draw_list`. Since the pointer is a
*uniform*, not per-instance data, the instance bytes stay identical — check that
the test compares instances and not the rendered image, and note the result either
way.

Alternative, if the uniform route is unwanted: the shaped pipeline's existing
`GRAD_RADIAL` can paint the same highlight as an instance, at the cost of one draw
per highlighted surface.

### 10.5 Dither — stop the washes banding

Somnium's 2–6 % gradients across 40–200 px in 8-bit sRGB produce visible steps. Two
options, both ~5 lines:

| Option | How | Notes |
|---|---|---|
| **A — ordered Bayer 8×8** *(recommended)* | An 8×8 matrix in constants; `± 0.5/255` amplitude applied just before output, **only when `FLAG_GRADIENT` is set**. | No texture, no atlas slot, deterministic — which matters for the byte-identical draw-list test and for golden images. |
| **B — blue noise texture** | Sample a tileable blue-noise texture ([free set](https://momentsingraphics.de/BlueNoise.html)). | Perceptually better, costs an atlas slot and a texture fetch, and makes goldens noisier. |

Background: [How to (and how not to) fix color banding](https://blog.frost.kiwi/GLSL-noise-and-radial-gradient/).
Deterministic dither is the right call for a tool that ships pixel-comparison tests.

### 10.6 Derive the accent ramp, in a perceptual space

`theme::ramp_step(base, 50..900)` exists at `theme.rs:838`, is tested at
`theme.rs:1193`, and **is used by nothing**. The accent's hover / pressed /
selected / glow values are still hand-picked hexes. Two problems, one fix:

1. Wire the ramp into the accent-derived tokens, as `phase_27.md` §9.5.2 originally
   asked ("derived rather than five hand-picked hexes"), and as Prowl does (§7.12).
2. `ramp_step` currently mixes toward white/black in **linear RGB**, which
   desaturates and shifts hue as it lightens. Move it to **Oklch** — vary `L`, hold
   `C` and `h` — per [Ottosson](https://bottosson.github.io/posts/oklab/) and the way
   [Bevy's gradient shader](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ui_render/src/gradient.wgsl)
   offers `Oklab`/`Oklch` interpolation. Do the same in `motion::lerp_color`, so an
   animated hover wash does not pass through a muddy midpoint.

Small change, systemic effect: every state colour in both themes becomes
consistent, and the existing contrast certification can then be *computed* per step
instead of re-eyeballed.

### 10.7 Multi-stop gradients

Fyrox's `Brush` (§7.7) carries stop lists; Somnium's `Primitive` carries two stops
on one axis. A 3-stop wash (dark → slightly-lighter → dark) is what makes a chrome
band look lit rather than tilted. Either widen the instance (costly — the 100-byte
size assert, the vertex attrs and the WGSL struct must move together, see §16) or
paint 3-stop washes through the **shaped pipeline**, which already supports
gradients and would cost one extra draw for a handful of chrome bands.
**Recommendation: shaped pipeline; do not widen `Primitive` for this.**

### 10.8 Optional: better shadow math

Somnium approximates the rounded-box shadow with `smoothstep(-soft, soft, d_sh)`
(`ui_pass.wgsl:197`) and says so honestly in its comment. Bevy/Figma/Zed use the
`erf` closed form (§5.6). **A/B it before spending the ALU** — at 8–32 px blurs the
difference may not be visible, and the existing comment already claims it is close
enough. If you do switch, [Raph Levien's version](https://raphlinus.github.io/graphics/2020/04/21/blurred-rounded-rects.html)
is cheaper than Wallace's.

**Exit condition for I.** Emboss and dither shipped and tokenised; the squircle and
blur decisions made explicitly (either implemented or recorded as declined with a
reason); pointer light shipped or declined; token sheets bumped to `0.4.0-persona`
with both themes and high contrast regenerated together; contrast certification
re-run; one batched capture run showing before/after at 1280×720 and 1920×1080.

---

## 11. PERSONA-J — *Moments*: stateful feedback the product currently lacks

H makes existing state changes continuous. J adds the states the editor does not
express at all. Each item names its job.

### 11.1 The change flash — "what did that just do?"

**Job:** after undo, redo, a reset-to-default, an asset assignment or an external
change, the affected row is not visibly the thing that changed.

One-shot `Flash` track on the property row background: accent at ~12 % → 0 over
240 ms, `Accelerate`. No layout change, no movement, no sound. This is the single
most useful new cue in the document for a designer working with undo, and it
directly closes the recurring "did the drag actually do anything?" confusion in the
[C/D](PERSONA-C_D.md) and [E/F](PERSONA-E_F.md) ledgers.

Reduced motion: hold the flash colour for 240 ms, then cut. Still visible, no ramp.

### 11.2 Drag and drop — continuous feedback

Current state: semantic routes exist and work; the *feedback* is the recorded
weakness. Three cues, all one-shot:

1. **Drag start** — the payload chip scales `0.92 → 1.0` with `SPATIAL_FAST` and fades in. It should feel picked up.
2. **Over a valid target** — the target's border animates to `border.focus` over 120 ms and holds. Under amendment A3 this is a border, not a glow — no looping pulse.
3. **Refused** — the cursor chip shakes: `OffsetX ±3 px`, three cycles, 160 ms total, one-shot. Paired with the existing refusal text. This is the standard "no" and it is *bounded*, so it does not violate A1.
4. **Accepted** — the target flashes (§11.1) and the chip collapses toward the field over 140 ms.

### 11.3 Busy and indeterminate — the one permitted loop

**Job:** PERSONA §4's state grammar demands "no endless anonymous spinner" but ships
no alternative. Two states:

- **Determinate** (progress known): a thin 2 px accent bar under the job's row, width driven by real progress. No animation beyond the width lerp. Preferred whenever a fraction exists.
- **Indeterminate** (progress unknown): the Wicked-style **angular sweep** (§7.1) around the job chip — a conic wedge rotating at ~1 rev/1.4 s. Built either on the shaped pipeline's existing `GRAD_ANGULAR` (no shader change) or as a `FLAG_ANGULAR` quad instance.

Both must name the operation and offer Cancel where supported — the existing Jobs
surface already has the model. The loop starts when the job starts and stops within
one frame of it ending (A1). One at a time: if three jobs run, one sweep on the
group, not three.

Card et al.'s 1 s constant (§5.6) sets the trigger: **do not show any busy
indicator before 1 s.** A spinner that flashes for 200 ms is worse than no spinner.

### 11.4 Skeletons for thumbnail decode

`thumbnails.pump()` already decodes a bounded number of previews per frame. While a
tile's preview is pending, paint a skeleton with a slow diagonal shimmer sweep
(one pass per ~1.2 s), stopping the instant the preview lands. Same A1 justification
as the busy sweep: the condition is real and external.

Cheaper alternative if you would rather not loop at all: a static placeholder with a
one-shot cross-fade when the real preview arrives. **This is the more conservative
choice and it is defensible** — decide and record which.

### 11.5 Arrival — first paint and mode arm

- **First paint.** The editor currently cuts from nothing to a full shell. One 240 ms scrim lift over the assembled shell (opacity only) turns a hard cut into an arrival. One-shot, at startup, skipped under reduced motion and skipped entirely in capture mode so it cannot pollute goldens.
- **Mode arm** (Landscape, Foliage). The armed glow token already exists and is already used. Give it a 120 ms fade-in and pair it with the contextual rail sliding in on its axis. Leaving a mode reverses it.
- **Save.** The dirty dot becomes a check with a 180 ms `pop-in` (`scale 0.8 → 1`, `SPATIAL_FAST`), then settles. This is the one place a small overshoot is welcome: it is a success beat, not a control the user is manipulating.
- **Job complete.** A single flash on the job row plus the toast that already exists. Nothing more.

### 11.6 Icon state morphs

`icon_svg.rs` rasterises Tabler SVG sources into an atlas, alpha only. Three routes:

| Option | How | Cost | Verdict |
|---|---|---|---|
| **A — cross-fade two atlas cells** *(recommended)* | Animate `Opacity` between the two existing glyphs. | ~10 lines, no new machinery | Covers play↔pause, lock↔unlock, eye↔eye-off. Boring and correct. |
| **B — rotate the instance** | For chevron↔caret, rotate rather than swap — it is the same glyph. The shaped pipeline has a full affine; the quad pipeline does not. | Small, via shaped | Correct for expand/collapse, which is exactly where a rotation reads as *meaning* ("this opened downward"). |
| **C — true path morphing** | Interpolate flattened contours in `path.rs`. | Large; correspondence problems between differing point counts | **Reject** unless a specific icon pair demands it. |

**Exit condition for J.** Every J row of §14 implemented; the busy indicator is
reachable from a real job and provably stops; `an_idle_shell_has_zero_live_motion_tracks`
still passes with the shell open on a scene with no jobs; the change flash is
reachable from undo, reset and assignment; reduced-motion equivalents recorded for
each.

---

## 12. PERSONA-K — QoL

The backlog is §15. It is deliberately longer than one slice: pick with the user,
land what you pick, leave the rest listed. **Do not** attempt all of it and land
half.

---

## 13. PERSONA-L — proof

Nothing here is optional, and it is small if H–K were done carefully.

- **Performance.** Matched UI CPU/GPU zones at the same scene, size, frame count and warm-up. Existing budget: **UI p95 ≤ +10 %** ([`phase_PERSONA.md`](../phase_PERSONA.md) §8). Blur, if built: **≤0.15 ms at 1440p** (`phase_27.md` §12). New: assert the animator allocates nothing per frame in the steady state, and that a shell with 200 content tiles hovering costs no more than N tracks.
- **Accessibility.** Reduced motion across every new property; high-contrast snapshots regenerated; contrast certification re-run over the derived ramp (§10.6); target sizes unchanged or improved; AccessKit tree unchanged in structure by any animation.
- **Evidence.** §17.
- **Records.** Update this file's status, `phase_PERSONA.md` §7's delivery paragraph, and the `phase_27.md` amendment note. Run `git diff --check` and validate the document links.

---

## 14. The choreography table — the actual spec

`P` = the `motion::policy` entry point from §9.6. Slice column says where it lands.
**Reduced motion for every row is: same end state, zero duration**, unless the row
says otherwise. Durations assume Compact; do not scale them with density.

### 14.1 Interaction feedback (≤200 ms ceiling — unchanged by the amendment)

| # | Surface | Trigger | Property | Duration / motion | Slice |
|---|---|---|---|---|---|
| 1 | Button, icon button, toggle | hover in / out | `HoverWash` | 120 / 90 ms, `Decelerate` — **shipped** | — |
| 2 | Tree row | hover | `HoverWash` | 120 ms — **shipped** | — |
| 3 | Any pressable | press down / up | `PressWash` + `ScaleY` 0.985 | 90 ms, `EFFECT_FAST` spring. Emil: `scale(0.97)`; 0.985 is gentler and right for a dense tool | H |
| 4 | Tile, asset tile | hover | `HoverWash` + emboss strength | 120 ms | H |
| 5 | Focus ring | focus gained / lost | `Opacity` + `Stroke` 1→2 px | 90 ms `Decelerate`. **Never moves layout.** | H |
| 6 | Selection (tree, list, grid) | select | `Flash` on fill + rail `ScaleY` 0→1 | fill 120 ms, rail 90 ms `Decelerate` from the row's leading edge | H |
| 7 | Selection loses panel focus | focus moves | fill cross-fade to `selection.inactive` | 120 ms `EFFECT_DEFAULT` | H |
| 8 | Checkbox / toggle | toggle | check `Scale` 0.6→1 + `Opacity` | 140 ms, `SPATIAL_FAST` (small overshoot allowed) | H |
| 9 | Tab | activate | underline `OffsetX` + `ScaleY` | 160 ms `Standard`, travels from old tab to new — object constancy (§5.6) | H |
| 10 | Numeric field | scrub | **nothing** | Forbidden by contract. The value must be instantaneous. | — |
| 11 | Numeric field | commit / reject | `Flash` (accept) or shake `OffsetX ±3`, 3 cycles | 240 / 160 ms one-shot | J |
| 12 | Slider | drag | knob follows pointer directly; **no smoothing** | 0 ms — smoothing a dragged control feels broken | — |
| 13 | Invalid field | validation fails | shake `OffsetX ±3`, 3 cycles + error edge fade-in | 160 ms one-shot. Reduced motion: edge only, no shake | J |

### 14.2 Travel (`MAX_TRAVEL_MS` = 320 — requires amendment A2)

| # | Surface | Trigger | Property | Duration / motion | Slice |
|---|---|---|---|---|---|
| 14 | Popup, combo box, context menu, `search_box` results | open | `Opacity` 0→1 + `Scale` 0.96→1 **from the anchor** | 140 ms `Decelerate`. Never `scale(0)` (Emil) | H |
| 15 | Same | close | `Opacity` only, no scale | 100 ms `Accelerate` — exits are faster than entrances | H |
| 16 | Menu bar menu | open while another is open | cross-fade, no scale | 90 ms — reopening a sibling should feel instant | H |
| 17 | Content drawer | open / close | `OffsetY` travel | **distance-scaled**, 200–320 ms `Decelerate` / 180 ms `Accelerate` (Carbon §5.3) | H |
| 18 | Modal | open | scrim `Opacity` 120 ms; dialog `Scale` 0.98→1 + `Opacity` 180 ms from centre | `Decelerate`; scrim leads | H |
| 19 | Modal | close / Esc | both `Opacity`, 120 ms `Accelerate` | Esc must feel immediate | H |
| 20 | Toast | enter | `OffsetY` from below + `Opacity`, `SPATIAL_DEFAULT` spring | ~260 ms | H |
| 21 | Toast stack | one dismissed | remaining toasts reflow `OffsetY` | spring, `SPATIAL_FAST` — object constancy | H |
| 22 | Details section | fold / unfold | height via `ScaleY` + content `Opacity` | **300 ms**, matching Blender's `ANIMATION_TIME` (§7.5.3). Suppressed while another interaction is in flight, also per Blender | H |
| 23 | Workspace switch | preset change | content cross-fade 120 ms + 8 px directional slide **of content only** | Chrome never moves; only the changing region does | H |
| 24 | Panel float / dock | Float, Dock, drag-release | geometry spring, `SPATIAL_DEFAULT` | Must not fight the OS window animation — test on Windows before committing | H |
| 25 | Splitter drag | release | **nothing** — it lands where released | Smoothing a splitter is the classic mistake | — |
| 26 | Scroll | wheel | existing behaviour + the shipped edge fade | Do **not** add scroll smoothing: it breaks precision in a DCC and is not requested | — |

### 14.3 Choreographed groups (aesthetic; capped per §5.6)

| # | Surface | Trigger | Pattern | Cap | Slice |
|---|---|---|---|---|---|
| 27 | Content tile grid | folder change **only** (never scroll, never resize) | `Opacity` + `OffsetY` 6 px, `StaggerPattern::Sweep` | 24 ms/tile, **240 ms total**, first ~10 tiles only then batch the rest | H |
| 28 | Details rebuild | selection change | **staged** (Heer & Robertson §5.6): fade out 80 ms → reflow → fade in 120 ms | 200 ms total | H |
| 29 | Outliner expand | expand a node | children `Opacity` + `OffsetY`, `StaggerPattern::Index` | 16 ms/row, 160 ms total | H |
| 30 | Command palette | open | popup enter (row 14) + results `StaggerPattern::Index` | 12 ms/row, 120 ms total. **Results must be usable before the animation finishes** — keyboard input is never gated on motion | H |

### 14.4 Moments (§11)

| # | Surface | Trigger | Effect | Slice |
|---|---|---|---|---|
| 31 | Property row | undo / redo / reset / external assign | change flash, 240 ms one-shot | J |
| 32 | Drag payload | drag start / refuse / accept | scale-in / shake / collapse | J |
| 33 | Drop target | hover valid | border → `border.focus`, 120 ms, hold | J |
| 34 | Job chip | indeterminate job > 1 s | angular sweep, **looping while the job lives** | J |
| 35 | Tile | preview pending | skeleton shimmer, or static placeholder + cross-fade on arrival | J |
| 36 | Shell | first paint | 240 ms scrim lift, one-shot, skipped in capture mode | J |
| 37 | Mode rail | arm / disarm | armed glow 120 ms + rail slide on its axis | J |
| 38 | Save indicator | save completes | dot → check, `pop-in` 180 ms | J |
| 39 | Expand chevron | expand / collapse | 90° rotation, 140 ms (shaped pipeline) | J |

---

## 15. The QoL backlog

Grouped, roughly in descending value. Each is independent; pick with the user.

### 15.1 Editing

1. **Expression evaluation in numeric fields** — type `3*2+1`, `w/2`, `45deg`. Blender, Maya, Houdini and Unreal all do this and designers expect it. Highest-value single QoL item in the list.
2. **Modifier-scaled scrubbing** — `Shift` = ×0.1 precision, `Ctrl` = snap to step, both shown in the status bar while held.
3. **Copy / paste property values**, including across entities of the same component, with one undo entry.
4. **Alt-drag to duplicate** in the Outliner and the viewport gizmo.
5. **Multi-field edit from a mixed selection** — already partly supported; make "type once, apply to all" explicit and undoable as one gesture.
6. **Drag a property label onto another property** to copy the value (Flax does this — §7.10).
7. **Right-click context menus everywhere**, with the same items as the keyboard route. Any action reachable by mouse must be reachable by keyboard and vice versa.

### 15.2 Navigation and search

8. **Fuzzy scoring with recency** in the command palette; recently used commands rank first. The palette already exists; this is scoring, not UI.
9. **Quick search in every list** — type-to-filter in the Outliner, Content, and Details with a consistent shortcut.
10. **Clickable breadcrumb segments** in Content, with a dropdown of siblings per segment.
11. **Back / forward** with mouse buttons 4/5 in Content.
12. **"Reveal in Explorer" / "Copy path"** on every asset context menu.
13. **Sticky group headers** while scrolling Details.
14. **Focus follows selection** — selecting in the Outliner scrolls Details to the top, and selecting a Details section highlights its Outliner origin.

### 15.3 Feedback and clarity

15. **Shortcut hints in every tooltip** — the command registry already knows the binding; surfacing it makes the shortcut discoverable at the moment of use.
16. **Undo toast with an Undo button** for destructive or non-obvious operations.
17. **A status bar that says what the current tool will do** on click, and why it cannot act when it cannot.
18. **Consistent `Esc` discipline**, documented: cancel the drag, then close the popup, then clear the search, then deselect — in that order, one level per press.
19. **Hover-delay previews** for assets (the existing tooltip delay token, applied to a thumbnail popover).
20. **Empty states with a next action** everywhere — already begun in 27-G; finish it.

### 15.4 Layout and windows

21. **Tear-off tabs** — drag a tab out to float it. The floating machinery already exists; this is the missing gesture.
22. **Remember scroll position and expansion per entity type**, so re-selecting a light returns you to where you were.
23. **Double-click a splitter to reset it** to the preset width.
24. **A visible "layout modified" state** with an explicit Save Layout / Reset Layout, so a user who nudged a splitter is not silently persisted into a layout they did not choose.
25. **Zoom the whole UI** (`Ctrl +/-`), independent of OS DPI — Blender and Unreal both have it, and it is the single most requested accessibility affordance in dense tools.

### 15.5 Content browser

26. **Drag-to-reorder array elements** in Details with a live insertion indicator.
27. **Rubber-band multi-select** in the tile grid and the Outliner.
28. **Filter chips that show counts** and can be inverted.
29. **A "recently modified" virtual folder**, from data the browser already tracks.

---

## 16. How not to break things

This section is the difference between a delivery that lands and one that gets
reverted. It is not general advice; every rule comes from something specific in
this repository.

### 16.1 The three-places rule for `Primitive`

If you add a field to `Primitive`, **three things must move together or the GPU
silently reads garbage**:

1. the struct in `src/primitive.rs` (and `const _: () = assert!(size_of::<Primitive>() == 100)` — update the number),
2. `Primitive::VERTEX_ATTRS` (offsets are hand-written and must stay correct),
3. the `InstanceIn` struct in `src/ui_pass.wgsl`.

The same applies to `ShapedInstance` (64 bytes, `% 16 == 0`) and `ui_shaped.wgsl`.
**Prefer adding a flag bit over adding a field.** All of §10's effects except the
backdrop fit in existing fields plus a flag.

### 16.2 Tokens move in lockstep

`theme.rs` is declared the source of truth in `$meta.source_of_truth`. Any token
change must update, in one commit: `theme.rs` (Nocturne **and** Dawn **and** the
high-contrast derivation), both `assets/tokens/*.tokens.json`, and
`$meta.version` → `0.4.0-persona`. `json_sheets_match_the_shipped_snapshots` will
catch you; the contrast certification will catch you second.

### 16.3 The guard tests, and the honourable way past each

| Test | What it will do | What you must do |
|---|---|---|
| `an_idle_shell_rebuilds_a_byte_identical_draw_list` | Fail if anything animates at rest | **Do not weaken it.** Add `an_idle_shell_has_zero_live_motion_tracks` beside it. If pointer light makes it fail, check whether it compares instances or pixels (§10.4) and record the finding. |
| `the_shell_actually_uses_the_new_paint_capabilities` | Counts gradients / shadows / insets / borders against a ceiling | Update it to count **named exemptions** (emboss, dither, backdrop) rather than raising a raw number. A raw number raise is how the "flatten the washes" work of PERSONA-B gets silently undone. |
| `identical_frames_produce_identical_instance_bytes` | Fail on any per-frame nondeterminism | The animator's `HashMap` iteration order is nondeterministic — that is fine today because values are looked up by key, not iterated into the draw list. **Keep it that way.** Never iterate the animator to build draw order. |
| `reduced_motion_reaches_the_animator` | Only checks the flag reaches the animator | Extend to assert *layout equality* for each new `MotionProperty`, which is the actual contract. |
| Golden images | Already failing: menu bar **98.98 %**, sculpt panel **99.997 %**, toolbar **99.975 %** vs a 0.2 % budget | **Do not replace them.** Approval belongs to the user in PERSONA-G. Record the new numbers; do not silently re-baseline. |
| `python tools/ghostfence/run.py --fast` | Enforces "one shader system", census, shader budget | A new blur pass will be examined against the "one shader system" row. Reusing bloom/SPD (§10.3 A/B) avoids the argument entirely. |

### 16.4 Things that must not change

One command registry. One schema editing route. One undo gesture per user gesture.
One UI renderer. The sRGB / straight-alpha contract. Floating-panel ownership. The
primary-viewport redirect (never a second scene render). Game-owned canvas styling.
Panel bodies stay flat. Panels never cast shadows. No UI code retaining ECS borrows
across an input gesture.

### 16.5 Working constraints in this environment

These come from recorded operational experience, not preference:

- **Do not run `hello_engine` repeatedly.** Repeated native captures have driven this machine to ~15 GB / 97 % memory. **Batch every capture you need into one run** that emits all states, using the existing `SOMNIUM_AUDIT_*` / `SOMNIUM_CAPTURE_*` hooks with an isolated `APPDATA`. Prefer `cargo test -p somnium_ui -j 1` and shader-source tests for iteration.
- **Use `-j 1`** on Windows. `LNK1104` on mapped test executables is a file-lock artefact, not a code failure; rerun the same command.
- **A green `cargo test` does not prove the editor starts.** If you cannot verify a change natively, say so before committing, not after.
- **Land the work in the editor path, not only in the proof slice.** `hello_engine` is where the user evaluates this.
- Regenerating `somnium.d.luau` is unrelated here, but if you touch the type registry, remember it is generated and will go stale silently.

### 16.6 Sequencing rule

Land H's `motion::policy` module **before** any of §14's call sites. If call sites
come first they will each invent their own durations and the result is exactly the
inconsistency this document exists to remove.

---

## 17. Evidence and acceptance

### 17.1 What counts

- **One batched native capture run** per slice (see §16.5), producing: shell at 1280×720 and 1920×1080, Details populated, drawer open, palette, modal, a floating panel, an empty state, a busy state, and the component gallery — in Nocturne and Dawn, standard and high contrast, Compact and Comfortable.
- **Motion cannot be captured by a still.** For each animated row in §14, either record a short screen capture, or capture the **mid-transition frame** deterministically by driving the animator to `t = 0.5` in a test and rendering. The second is reproducible and belongs in the repository; the first is for the user.
- Every capture records: revision, command, scene, window size, DPI, selected entity, workspace, theme, density, contrast.

### 17.2 What does not count

A passing test suite. A screenshot of a static shell. A claim that something "feels
better". A golden image you replaced yourself.

### 17.3 The gate that still binds

[`phase_PERSONA.md`](../phase_PERSONA.md) §8's **zero known outstanding editor bugs**
requirement applies to everything this expansion touches, including bugs it
discovers in passing. §2.5's node-recycling defect is already one; find it a place
in the ledger. Every defect found during H–L is fixed and its reproduction rechecked
before the slice closes, regardless of severity.

---

## 18. Rejected ideas — and why, so they are not re-proposed

| Idea | Why not |
|---|---|
| Glassmorphism everywhere; blurred panel bodies | Blur is expensive, hurts text legibility, and `phase_27.md` §9.4 already scoped it to exactly two surfaces. Two is the budget. |
| Large corner radii, card-grid layouts, generous padding | `phase_27.md` §5.2 forbids it; §7.12 shows the professional consensus runs the other way; and [`phase_27.md`](../phase_27.md) §4.4's density argument means beauty must not be bought with whitespace. |
| Film grain / noise over the UI | The renderer has `pass/grain.rs`, but grain over text is a legibility regression. Dither (§10.5) is ±0.5/255 and invisible; grain is not. |
| Idle breathing, pulsing, floating, parallax | §9.3, and Bartram et al. (§5.6): peripheral motion is preattentive and continuously steals attention from the viewport. |
| Animated brand mark in the chrome | Ornament with no job. If a brand moment is wanted, it belongs on a splash, once. |
| Scroll smoothing / inertia in editor lists | Breaks precision selection in a dense tool. Not requested. |
| Sound design | Out of scope; not requested; a DCC that beeps is a DCC that gets muted. |
| Rive / Lottie runtimes | §6.2. A second vector renderer for a handful of icons. |
| Replacing the UI framework (Makepad, Slint, GPUI) | Explicitly out of scope, [`phase_PERSONA.md`](../phase_PERSONA.md) §10, and would discard every shipped PERSONA slice. |
| Refraction / distortion behind panels (Wicked's `DISTORTION_MASK`) | Decoration on a precision tool. |
| A theme marketplace / user-authored themes | [`phase_PERSONA.md`](../phase_PERSONA.md) §10 excludes it. §10.6's derived ramp is the useful 5 % of it. |
| Raising the paint-budget test's number to make room | §16.3. Name exemptions instead. |

---

## 19. Sources

**Local reference collections** (not upstream-verified revisions; no code copied):
`C:/Users/adhir/Downloads/GE/example_repo/` — `WickedEngine-master`,
`new and shiny/upbge-master` (Blender UI), `bevy/bevy-main`,
`fyrox/Fyrox-master`, `sbox-public-master`, `godot-4.7.1-stable`,
`New_Engines/FlaxEngine-master`, `uGUI-main`, `SpartanEngine-master`,
`new and shiny/Prowl-main`, `New_Engines/Overload-main`, `New_Engines/Esoterica-main`.

**Design systems and practitioner rules**
- [Emil Kowalski — review-animations STANDARDS.md](https://github.com/emilkowalski/skills/blob/main/skills/review-animations/STANDARDS.md) · [animations.dev](https://animations.dev/)
- [Material Design 3 — Motion](https://m3.material.io/styles/motion/) · [Easing and duration tokens](https://m3.material.io/styles/motion/easing-and-duration/tokens-specs)
- [IBM Carbon — Motion](https://carbondesignsystem.com/elements/motion/overview/) · [IBM/motion](https://github.com/IBM/motion)
- [Apple HIG — Motion](https://developer.apple.com/design/human-interface-guidelines/motion)
- [Fluent 2 — Motion](https://fluent2.microsoft.design/motion) · [Design tokens](https://fluent2.microsoft.design/design-tokens)
- [WCAG 2.2 — Non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast) · [Target size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum) · [Animation from interactions](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions)

**Research**
- Card, Robertson & Mackinlay, *The Information Visualizer, an Information Workspace*, CHI 1991
- Robertson, Mackinlay & Card, *Cone Trees*, CHI 1991
- Tversky, Morrison & Bétrancourt, *Animation: can it facilitate?*, IJHCS 2002 — [PDF](https://www.cs.ubc.ca/~tmm/courses/cpsc533c-04-spr/readings/tversky.pdf)
- Heer & Robertson, *Animated Transitions in Statistical Data Graphics*, InfoVis 2007 — [PDF](https://idl.cs.washington.edu/files/2007-AnimatedTransitions-InfoVis.pdf) · [project](http://vis.stanford.edu/papers/animated-transitions)
- Chevalier, Dragicevic & Franconeri, *The Not-so-Staggering Effect of Staggered Animated Transitions on Visual Tracking*, TVCG 2014 — [PDF](http://www.cs.toronto.edu/~fchevali/fannydotnet/resources_pub/pdf/notsostaggering-infovis14.pdf)
- Dragicevic et al., *Temporal Distortion for Animated Transitions*, CHI 2011
- Bartram, Ware & Calvert, *Moticons: detection, distraction and task*, IJHCS 2003

**Rendering technique**
- [Evan Wallace — Fast Rounded Rectangle Shadows](https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/)
- [Raph Levien — Blurred rounded rectangles](https://raphlinus.github.io/graphics/2020/04/21/blurred-rounded-rects.html) · [An interactive review of Oklab](https://raphlinus.github.io/color/2021/01/18/oklab-critique.html)
- Bjørge, *Bandwidth-Efficient Rendering*, SIGGRAPH 2015 — [dual-Kawase reference](https://github.com/Baedrick/Dual-Kawase-Blur-Demo)
- Jimenez, *Next Generation Post Processing in Call of Duty: Advanced Warfare*, SIGGRAPH 2014
- [Björn Ottosson — Oklab](https://bottosson.github.io/posts/oklab/)
- [Christoph Peters — Free blue noise textures](https://momentsingraphics.de/BlueNoise.html) · [How to (and how not to) fix color banding](https://blog.frost.kiwi/GLSL-noise-and-radial-gradient/)
- [Inigo Quilez — 2D distance functions](https://iquilezles.org/articles/distfunctions2d/)
- [Ryan Juckett — Damped Springs](https://www.ryanjuckett.com/damped-springs/)
- [Squircle.js — the superellipse math](https://squircle.js.org/blog/math-behind-squircles) · [Figma corner smoothing](https://help.figma.com/hc/en-us/articles/360050986854-Adjust-corner-radius-and-smoothing)
- [Zed — Leveraging Rust and the GPU to render UIs at 120 FPS](https://zed.dev/blog/videogame) · [GPUI backdrop blur discussion](https://github.com/zed-industries/zed/discussions/47429)

**Libraries surveyed (all rejected as dependencies — §6)**
- [tachyonfx](https://github.com/ratatui/tachyonfx) · [docs](https://docs.rs/tachyonfx) · [ratatui ecosystem page](https://ratatui.rs/ecosystem/tachyonfx/)
- [bevy_tweening](https://docs.rs/bevy_tweening/) · [bevy_tween](https://docs.rs/bevy_tween/) · [spanda](https://github.com/aarambh-darshan/spanda) · [animato](https://lib.rs/crates/animato-tween)
- [rive-rs](https://github.com/rive-app/rive-rs) · [velato](https://github.com/linebender/velato) · [vello](https://github.com/linebender/vello) · [lyon](https://github.com/nical/lyon)
- [bevy_ui_render](https://github.com/bevyengine/bevy/tree/main/crates/bevy_ui_render) · [fyrox-ui](https://github.com/FyroxEngine/Fyrox/tree/master/fyrox-ui) · [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) · [Makepad](https://github.com/makepad/makepad) · [Slint](https://slint.dev/)

**Somnium documents**
[`phase_PERSONA.md`](../phase_PERSONA.md) · [`PERSONA-A_B.md`](PERSONA-A_B.md) ·
[`PERSONA-C_D.md`](PERSONA-C_D.md) · [`PERSONA-E_F.md`](PERSONA-E_F.md) ·
[`PERSONA-QoL.md`](PERSONA-QoL.md) · [`phase_27.md`](../phase_27.md) ·
[`phase_26_Zeta.md`](../phase_26_Zeta.md) · [`ATTRIBUTION.md`](../../ATTRIBUTION.md)

---

## 20. Skills applied

From the installed Codex marketplace cache
(`C:/Users/adhir/.codex/plugins/marketplaces/`):

| Skill | How it was used | Where it was overridden |
|---|---|---|
| `agentic-awesome-skills/ui-motion` | Its use-case → motion map and its two anti-rules — **"one seed per product"** and **"never delay the payload"** — shape §14. The second is why row 10 forbids animating a numeric value and why row 30 requires the palette to accept keystrokes before its animation finishes. | Its React/Framer-Motion specifics do not apply. Its `pulse-beat`, `shimmer` and `tilt-3d` recommendations are rejected or narrowed by §4's amendment. |
| `agentic-awesome-skills/high-end-visual-design` | Its "double-bezel" nesting idea informs §10.1's emboss; its performance guardrails (animate transform/opacity only; blur only on fixed surfaces) are consistent with §16. | **Substantially overridden.** Its banned-font list, `rounded-[2rem]` radii, `py-24` macro-whitespace and "Awwwards-tier" framing are landing-page rules and are the opposite of what a dense DCC needs — see §7.12 and `phase_27.md` §5.2. Recorded here so the conflict is deliberate rather than accidental. |
| `agentic-awesome-skills/anti-ui-slop` | Its finish gate is adopted directly: no decorative effects without a product reason, every visible control has a real outcome, required states implemented and reachable, and the result must not look like a generic default. §11's states and §18's rejections come from applying it. | Its UIZZE catalogue is a web/iOS reference set; the reference corpus here is the local engine collection instead. No content left this machine. |
| `design/design-system`, `design/design-critique` | Token structure, semantic-vs-raw separation, and the gap analysis method in §3. | — |
| `agentic-awesome-skills/rust-pro`, `codebase-design` | §9.6's policy-module shape, §16.1's three-places rule, the closed-enum discipline for `MotionProperty`, and the ownership rules in §16.4. | — |
| `engineering/code-review` | §16's guard-test analysis and §17's evidence standard. | — |
| `ponytail` (active in this session) | Why §6 concludes **zero new dependencies**, why §10 is ordered by effect-per-line, and why §12 says pick a subset of the QoL backlog rather than attempting all of it. | — |

---

## 21. Status

2026-09-09, base `0246ad2`: [PERSONA-I's Nocturne finish](PERSONA-I.md) is implemented, release-built and reviewed in exactly three native captures under the user's updated screenshot allowance. Both token sheets are **0.5.0-persona**. It delivers authored dark chrome, emboss/dither, explicit popup/modal frames, component/group hierarchy, centered-dialog and tab presentation, changed-row feedback and readable toasts. Squircle, blur, pointer light and other rendering alternatives are explicitly decided in that record. Tests: 1,109 UI/core unit tests plus six UI shader checks passed. Following positive user feedback, the header gradient and control emboss were strengthened for visibility on frequently used chrome; 25 targeted theme checks passed and the release was rebuilt. The three captures precede this final token adjustment.

Earlier [H passes](PERSONA-H.md) supply shared hover/press/toggle policy, anchored-popup continuity, node cleanup and idle/interaction regressions. The [QoL pass](PERSONA-QoL.md#expansion-feedback--2026-09-09) supplies bounded scrolling Create/View menus, alignment and viewport texel density.

Full expansion closure remains open: H's drawer travel, sibling-menu switching, focus/selection/group/floating choreography; J's remaining drag/job/thumbnail/arrival/icon feedback; the broader K backlog; matched GPU performance certification, OS/DPI/floating journeys and G's user acceptance. The delivered visual pass does not silently mark those rows complete. Flat dark panel bodies and immediate Content hover remain binding user preferences. No goldens were replaced.
