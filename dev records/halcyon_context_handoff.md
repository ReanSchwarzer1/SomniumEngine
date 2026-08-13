# Somnium Engine — Halcyon (Phase VV) Context Handoff

> **Purpose:** start-here for a Phase VV implementation session.  
> **Snapshot date:** 2026-08-13 evening  
> **Branch:** `dev`  
> **HEAD at handoff:** `b5d1e57` (`more fixes ro ui`) plus the same-evening ComboBox overlay / immersive-play / drawer-tile work already in the tree  
> **Implementation status:** Phase IV **complete**; Phase XV **A–J complete** (1.10 ms shading exception; BC7 encoder + local packs); Phase 26 (Metaphor) **26-A–I shipped, phase remains open** as living chrome (26-J not started); Phase VV (Halcyon) **planned — no engine code yet**

This document supersedes [`post_IV_context_handoff.md`](post_IV_context_handoff.md) as the **start-here** file. Keep the post-IV handoff for IV/XV history; keep [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) for IV A–J / asset-license narrative. Do not treat either as the current entry point.

**Live contracts (do not silently retune):**

- Water: `WaterComponent::great_lakes` (datum **16.1 m**, optical `max_depth` **18.6 m**, Gerstner `wave_speed` **0.85**). CPU `sample_surface` is Gerstner-only; spectral FFT is GPU visual.
- Terrain: 32 global layers, sidecar v4, 1664-byte `GpuTerrainMaterial`, unique colour from splat, biome v3 / landscape v4, snow `relief * 0.48`, aerial hex/POM off > 80 m above ground. Canonical: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). Do not reintroduce a per-pixel terrain sample-count LOD.
- UI: Metaphor chrome is the shipping editor. Do not restart at 26-A. Do not implement 26-J unless the user asks. New `IconId` variants only at the **end** of the enum.

---

## 1. Read this first

A Halcyon session should read these **in order**:

1. **This file** — current engine state, frozen contracts, what Metaphor just shipped, how to start VV-A.
2. [`phase_VV.md`](phase_VV.md) — the controlling plan (architecture, stages VV-A–H, budgets, non-goals). **Begin at VV-A. Do not begin at VV-C.**
3. [`context.md`](../context.md) — living architecture. §6 / §14 water + ray tracing; §8 editor chrome; roadmap rows IV / XV / 26 / VV. **Do not rewrite §20** (Phase 14 heightmap history).
4. [`ATTRIBUTION.md`](../ATTRIBUTION.md) — reference boundaries. Cite Halcyon sources in §1.7 **as they are used**; do not copy shader source.
5. [`phase_IV.md`](phase_IV.md) **§14 IV-K** — the water shading Halcyon has to reproduce through a traced ray.
6. Verify [`phase_VV.md`](phase_VV.md) **§4 against the worktree** before writing code. That audit is dated 2026-08-13; line numbers drift.

Optional (do not skip (1)–(6) for these):

- [`phase_26.md`](phase_26.md) — chrome contract if VV-A needs a debug toggle or Help line. Living chrome, not a rebuild.
- [`post_IV_context_handoff.md`](post_IV_context_handoff.md) — IV/XV history and the 32-layer terrain freeze.
- [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) — live terrain numbers.

---

## 2. Current state in one page

| Track | Status | Contract |
|---|---|---|
| **IV Great Lakes water** | Closed 2026-08-13 | [`phase_IV.md`](phase_IV.md) §14. Finite wet-cell body, 3×1024² FFT, Jacobian foam, Atlas-style lighting. Clipmap body / HDRI / GPU spray **not** delivered. |
| **XV Appalachia terrain** | A–J complete 2026-08-13 | [`phase_XV.md`](phase_XV.md) · live [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) · gate [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md) |
| **26 Metaphor editor** | 26-A–I shipped; **phase open** | [`phase_26.md`](phase_26.md). Nocturne shell, docked Content Drawer, Iris, Help, immersive play, ComboBox overlay. 26-J / 26-H SDF / 26-D2 still queued. |
| **VV Halcyon** | **Not started** | [`phase_VV.md`](phase_VV.md). Water still reflects via `trace_ssr` (28-step march) + env cube. |

**Largest remaining water fidelity gap:** off-screen / behind-camera / below-horizon reflections. The engine already builds a per-frame TLAS and traces it from ReSTIR DI and GI; those paths resolve a *diffuse* signal. Halcyon is the first *specular* ray path.

**Session estimate:** one Halcyon session should finish **VV-A** (instrumentation, SSR debug viz, TLAS overflow log) and leave the engine shippable. VV-B is the architectural commit; do not skip A to “get to rays.”

---

## 3. Everything in the tree until now

### 3.1 Renderer, physics, ECS (frozen for Halcyon)

Visibility-buffer renderer, archetype ECS, Jolt, Kira, voxel world, GPU-driven draws, ReSTIR DI/GI, GTAO, TAA, post-fx stack, foliage, terrain-in-visibility-buffer (25A) and terrain BLAS (25B). Do not retune lighting, terrain materials, or water optics unless a Halcyon stage’s exit criteria require it (VV-D hit shading must *match* IV-K, not replace it).

Parked GPU rows that are **not** Halcyon: 23 transparent GPU cull, 24AA/AB, 24P (software RT — must not quietly become VV), 24Q, 16 scripting, IV-K clipmap/HDRI/spray.

### 3.2 Phase IV — Great Lakes water (closed)

Default landscape is the Motion Forge Pictures Great Lakes height derivative plus a finite ECS `Water` parented to Terrain. Water is a specialized HDR pass (not visibility-buffer opaque): wet-cell mesh, full-res mask/SDF/depth, Gerstner CPU queries, optional spectral FFT, coherent optics, underwater medium, shoreline contact foam, motion vectors.

**IV-K** raised the spectral path to GodotOceanWaves-class fidelity: **3×1024²** FFT cascades, Jacobian foam, GDC 2019 *Atlas*-style lighting (with Somnium deviations). Evidence: `dev records/phase IV/IV-K/`.

| Field | Value | Why it is frozen |
|---|---|---|
| `surface_level` | 16.1 m | User-accepted water datum |
| `max_depth` | 18.6 m | Optical path, not bathymetry |
| `wave_speed` | **0.85** | Buoyancy samples Gerstner only; FFT is visual |
| `spectrum_blend` | 0.64 | Visual FFT crossfade |
| `ssr_strength` | 1.0 | Today’s reflection mix; Halcyon blends on confidence later |

Gislinge Viking Boat ships with `BuoyantVessel`. Environment sim runs in Editing and Playing; Pause freezes; Play hides editor overlays. Immersive play (Metaphor, same evening) hides chrome and goes borderless fullscreen; **Esc** exits.

**IV-K not delivered (do not “finish” inside Halcyon):** K-1 ocean clipmap body, K-6 GPU sea spray (abandoned), K-7 HDRI + Filmic.

### 3.3 Phase XV — Appalachia terrain (closed)

32 global photogrammetry PBR layers, eight splatmaps, strongest-four, unique-colour macro, full-PBR biplanar cliffs, Terrain Paint vs Foliage Paint, biome v3 / landscape v4, aerial hex/POM via `gpu_material_for_camera` (80 m). Live look signed off 2026-08-13.

**XV-J** compile gate + PNG corpus: `dev records/phase XV/evidence/`. Adapter freeze: NVIDIA GeForce RTX 5080 Laptop GPU, Vulkan, driver 610.74. Release overview shading **3.951 ms**, walk **5.532 ms** (1.10 ms budget is an explicit exception). BC7 encoder ships (`encode_terrain_bc7`); local packs load at 2048+1024 (~213 MiB). `SOMNIUM_TERRAIN_FORCE_RGBA8=1` for A/B.

Terrain is in the TLAS (25B). Water is **not**. Transparent geometry is **not**. That is why a reflected shoreline will not show water lapping against it — expected, not a Halcyon bug ([`phase_VV.md`](phase_VV.md) §8).

### 3.4 Phase 26 — Metaphor (open, shell shipped)

26-A–I landed 2026-08-13: toolkit (splitter, popup, checkbox, combo, tree, tabs, search, icons), Nocturne paint, Unreal-like slot map, docked Content Drawer, Details/Outliner, Iris colour pickers, `UiCanvas`, bitmap Inter (SDF slipped), command palette / toasts / HiDPI / layout persist / unsaved modal, custom title bar, F1 Help (`docs/editor/`), button hover/press, visible scrollbars.

**Same-evening chrome (still Metaphor, already in the tree):**

| Change | Where | Notes |
|---|---|---|
| Immersive play | Toolbar after Play; `IconId::ImmersivePlay` **last** in the enum; `EditorEvent::ToggleImmersiveViewport` | Play + hide overlays + borderless fullscreen; skip `ui.end_frame` while immersive (`renderer.rs`); Esc exits; restore maximized if that was the prior state |
| Content Drawer tiles | `theme::ICON_DRAWER = 80`; tiles ~112×120 | Was 48 px icons in 96×92 |
| ComboBox overlay | `combo_box.rs` header + `ComboDropdown` as root `Popup` | Type / Tonemap lists are opaque panels over the inspector, not expand-in-place ghosts. Click-away / Esc close via the transient-overlay path (same as File) |
| Toolbar Select / Landscape / Foliage | Were `let _ = icon_tool_button(...)` | Select → `SetGizmoMode(0)`; Landscape → `ToggleTerrainEdit`; Foliage → `ToggleFoliage` (enables the component, does not arm paint) |
| Terrain palette selected fill | `ButtonMessage::set_selected` | Active layer reads as selected |
| Details search | `refresh_inspector_filter()` after per-frame inspector writes | Filter was being wiped every frame |
| ScrollViewer thumb | `MIN_THUMB.min(track_h)` | `clamp(24, 0)` panicked after immersive → Esc → Drawer |
| Layout invalidation | `UserInterface::update` calls `invalidate_ancestors` when a widget dirties measure | Combo/popup open actually relayouts |
| Popup width | `PopupPlacement::AnchorBelow` at least as wide as the anchor | Dropdown matches the Type/Tonemap header |

**Still queued (do not start in a Halcyon session unless the user redirects):** 26-J reflection inspector, 26-H SDF/shaping, 26-D2 drag-drop spawn, async PNG thumbs.

**Halcyon may add living chrome** when a stage needs it (VV-A SSR hit/miss debug view, a post-fx or View-menu toggle, one Help line). That is Metaphor staying open, not a phase fold-in. Do not rebuild widgets, restyle Nocturne, or grow the icon atlas except a new `IconId` at the **end** of the enum.

### 3.5 Chronology (high level)

| When | Result |
|---|---|
| 2026-08-11 | 25M-2 night/shadow corrections |
| 2026-08-12–13 | Phase IV A–K, Great Lakes water, boat |
| 2026-08-13 morning | `b5e6052` IV close + Phase VV research plan (`phase_VV.md` created, no engine code) |
| 2026-08-13 | Phase XV A–J + BC7 (`43c3daa`, `8083328`) |
| 2026-08-13 | Phase 26 plan then 26-A–I (`719086c` … `ce16c31`) |
| 2026-08-13 evening | UI polish (`973b9a6`, `b5d1e57`) + immersive play, drawer tiles, ComboBox overlay, toolbar wiring |

---

## 4. What Halcyon is (and is not)

**Is:** replace SSR→env-cube as the *primary* water reflection path with hardware ray tracing, keep SSR as the near-field fast path, blend on confidence, degrade to *exactly* today’s look without `EXPERIMENTAL_RAY_QUERY`.

**Is not:** path tracing, ray-traced refraction (VV+1), caustics, water-in-water, ReSTIR replacement, a software RT fallback (that is still 24P), a water retune, a terrain session, a Metaphor rebuild.

Stages (do not skip; each must `cargo test --workspace` and leave the engine shippable):

| ID | Work | Visual change? |
|---|---|---|
| **VV-A** | GPU timer on water reflection; SSR hit/miss/confidence debug viz; TLAS cap overflow logs once/frame | Debug viz only |
| **VV-B** | Split `WaterPass` into G-buffer prepass + shading; shading still samples old `trace_ssr` | Byte-identical (reassociation) |
| **VV-C** | Half-res compute reflection, mirror ray, albedo-only hits | First new look (flat-shaded reflected geo) |
| **VV-D** | Extract `rt_hit.wgsl` from `gi_trace()`; sun + IBL at the hit; prove GI unchanged | Lit reflections |
| **VV-E** | GGX / roughness-aware; skip rough foam | Cost, not a new trick |
| **VV-F** | Reproject + accumulate + bilateral upsample | Stability |
| **VV-G** | Blend with SSR on confidence | Final mix |
| **VV-H** | Evidence under `dev records/phase VV/`, budgets, docs | — |

Fallback matrix and budgets: [`phase_VV.md`](phase_VV.md) §7 and §11. Kill switch: `SOMNIUM_RT_REFLECT=0` must restore today’s behaviour.

### 4.1 Infrastructure that already exists

| Capability | Location (verify; lines drift) |
|---|---|
| BLAS per mesh and terrain chunk | `pass/raytrace.rs`, `renderer.rs` |
| TLAS rebuilt per frame from `draw_queue` | `renderer.rs` |
| `EXPERIMENTAL_RAY_QUERY` gate | `context.rs` |
| Inline ray query | `restir_di.wgsl`, `restir_gi.wgsl`, `rt_debug.wgsl` |
| Hit → albedo (`gi_trace`) | `restir_gi.wgsl` |
| Water velocity + coverage MRT | `water.wgsl` |
| TAA uses water velocity | `taa.wgsl` |

### 4.2 Infrastructure that does not exist

No ray-traced reflection/refraction. No water or transparents in the TLAS. No shared “shade a ray hit” module (`gi_trace` returns albedo/normal; `gi_direct_at` is Lambert sun). No specular denoiser. No water G-buffer. No fragment-stage ray query in use (layout permits it, untested). **TLAS cap 1024 instances, silent drop** — raise or make observable before VV-C.

Water already copies HDR to `scene_color` for refraction/SSR and runs **after** opaque shading, so the TLAS is valid when water records. Do not reorder the frame for VV-A/B.

---

## 5. Must not break / must not do

1. Do not retune `WaterComponent::great_lakes` (especially `wave_speed`).
2. Do not rewrite `context.md` §20 as if it were XV.
3. Do not reintroduce per-pixel terrain sample-count LOD.
4. Do not put water in the BLAS/TLAS in VV-A–G (non-goal; §9.3 of the plan).
5. Do not remove `trace_ssr` — it is the degrade path and the VV-G near-field.
6. Do not start 24P (software RT) inside this phase.
7. Do not restart Metaphor at 26-A or implement 26-J.
8. Do not copy UE/Atlus/GodotOceanWaves shader source. Cite in `ATTRIBUTION.md` §1.7 when a reference is actually used.
9. Do not invent evidence PNGs. Captures after tonemapping into `dev records/phase VV/`.
10. Play / Pause / Stop / immersive Esc, foliage Type combo, terrain paint, boat buoyancy, and ReSTIR GI output (especially around VV-D) stay working.

---

## 6. Next-session start checklist (Halcyon)

1. Confirm branch `dev`. Read **this file**, then [`phase_VV.md`](phase_VV.md) §1–8 and §13.
2. `cargo check --workspace` **before** edits (known-good baseline).
3. User must have **authorized implementation**. This handoff is not itself a water retune.
4. Re-verify [`phase_VV.md`](phase_VV.md) §4 against `water.wgsl`, `water.rs`, `raytrace.rs`, `restir_gi.wgsl`, `renderer.rs` (TLAS build vs water record order).
5. **Begin at VV-A.** Timer + SSR debug viz + TLAS overflow log. Record the SSR miss rate for the default landscape in `phase_VV.md` when you have it.
6. If VV-A needs a View-menu or F-key debug overlay, add the smallest Metaphor binding (`EditorEvent` + Help/shortcuts line). Do not restyle chrome.
7. `cargo fmt`, `cargo test --workspace`. Keep the engine shippable every stage.
8. Update `phase_VV.md` status, this file’s HEAD note, `context.md` roadmap row VV, and `ATTRIBUTION.md` §1.7 when code lands.

**If the user redirects to Metaphor:** follow [`phase_26.md`](phase_26.md) §13.2 / §20. Do not fold Halcyon water into a UI session.

**If the user redirects to terrain:** live contract is [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). XV is closed; 25C/G/J/N/P remain parked.

---

## 7. Key paths

| Area | Path |
|---|---|
| Halcyon plan | `dev records/phase_VV.md` |
| Water pass / shader | `crates/somnium_renderer/src/pass/water.rs`, `crates/somnium_renderer/shaders/water.wgsl` (`trace_ssr`) |
| Water component / Great Lakes | `WaterComponent::great_lakes` (core); inspector bindings in `somnium_ui` |
| TLAS / BLAS | `crates/somnium_renderer/src/pass/raytrace.rs` |
| ReSTIR GI hit | `crates/somnium_renderer/shaders/restir_gi.wgsl` (`gi_trace`) |
| Feature gate | `crates/somnium_renderer/src/context.rs` (`EXPERIMENTAL_RAY_QUERY`) |
| Skip chrome when immersive | `crates/somnium_renderer/src/renderer.rs` (`ui.is_immersive()`) |
| Editor seam | `crates/somnium_ui/src/editor_event.rs` ↔ `crates/somnium_core/src/app.rs` |
| Combo overlay (do not regress) | `crates/somnium_ui/src/widgets/combo_box.rs`, `popup.rs` |
| Evidence destination | `dev records/phase VV/` (create on first capture) |

---

## 8. Help / shortcuts already documenting immersive play

`docs/editor/viewport.md`, `docs/editor/shortcuts.md` (Esc), `docs/editor/welcome.md` already mention the immersive Play-adjacent button. A VV-A debug view should add one line there if it is user-facing; a `SOMNIUM_*` env flag can stay docs-only in `phase_VV.md`.
