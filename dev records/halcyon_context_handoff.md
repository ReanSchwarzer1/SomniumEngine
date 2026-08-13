# Somnium Engine — Halcyon (Phase VV) Context Handoff

> **Purpose:** start-here after Phase VV-A–H. Remaining Halcyon work is evidence
> captures and §11 timings, not a re-implementation of A–H.  
> **Snapshot date:** 2026-08-13 evening (updated after VV-A–H)  
> **Branch:** `dev`  
> **Implementation status:** Phase IV **complete**; Phase XV **A–J complete** (1.10 ms shading exception; BC7 encoder + local packs); Phase 26 (Metaphor) **26-A–I shipped, phase remains open** as living chrome (26-J not started); Phase VV (Halcyon) **VV-A–H in tree** (live SSR miss-rate capture and GPU budget numbers still open)

This document supersedes [`post_IV_context_handoff.md`](post_IV_context_handoff.md) as the **start-here** file. Keep the post-IV handoff for IV/XV history; keep [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) for IV A–J / asset-license narrative. Do not treat either as the current entry point.

**Live contracts (do not silently retune):**

- Water: `WaterComponent::great_lakes` (datum **16.1 m**, optical `max_depth` **18.6 m**, Gerstner `wave_speed` **0.85**). CPU `sample_surface` is Gerstner-only; spectral FFT is GPU visual.
- Terrain: 32 global layers, sidecar v4, 1664-byte `GpuTerrainMaterial`, unique colour from splat, biome v3 / landscape v4, snow `relief * 0.48`, aerial hex/POM off > 80 m above ground. Canonical: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). Do not reintroduce a per-pixel terrain sample-count LOD.
- UI: Metaphor chrome is the shipping editor. Do not restart at 26-A. Do not implement 26-J unless the user asks. New `IconId` variants only at the **end** of the enum.

---

## 1. Read this first

A later session should read these **in order**:

1. **This file** — current engine state, frozen contracts, what VV shipped, what is still open.
2. [`phase_VV.md`](phase_VV.md) — plan plus the 2026-08-13 implementation record. **Do not re-implement VV-A–H.**
3. [`context.md`](../context.md) — living architecture. §6 / §14 water + ray tracing; §8 editor chrome; roadmap rows IV / XV / 26 / VV. **Do not rewrite §20** (Phase 14 heightmap history).
4. [`ATTRIBUTION.md`](../ATTRIBUTION.md) — reference boundaries. Halcyon citations are in §1.7.
5. [`phase_IV.md`](phase_IV.md) **§14 IV-K** — the water shading traced hits have to match.
6. Help: [`docs/editor/water.md`](../docs/editor/water.md) for the inspector knobs.

Optional (do not skip (1)–(6) for these):

- [`phase_26.md`](phase_26.md) — chrome contract. Living chrome, not a rebuild. 26-J still queued unless requested.
- [`post_IV_context_handoff.md`](post_IV_context_handoff.md) — IV/XV history and the 32-layer terrain freeze.
- [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) — live terrain numbers.

---

## 2. Current state in one page

| Track | Status | Contract |
|---|---|---|
| **IV Great Lakes water** | Closed 2026-08-13 | [`phase_IV.md`](phase_IV.md) §14. Finite wet-cell body, 3×1024² FFT, Jacobian foam, Atlas-style lighting. Clipmap body / HDRI / GPU spray **not** delivered. |
| **XV Appalachia terrain** | A–J complete 2026-08-13 | [`phase_XV.md`](phase_XV.md) · live [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) · gate [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md) |
| **26 Metaphor editor** | 26-A–I shipped; **phase open** | [`phase_26.md`](phase_26.md). Nocturne shell, docked Content Drawer, Iris, Help, immersive play, ComboBox overlay. 26-J / 26-H SDF / 26-D2 still queued. |
| **VV Halcyon** | **VV-A–H in tree** (2026-08-13) | [`phase_VV.md`](phase_VV.md). SSR + half-res RT + env cube on confidence. Kill switch `SOMNIUM_RT_REFLECT=0`. Evidence PNGs / §11 timings still open. |

**What shipped:** water G-buffer prepass, half-res GGX/mirror compute, `rt_hit.wgsl` (GI wraps `rt_trace`), cascade-shadow hit lighting (not a second ray), temporal mix, bilateral upsample, SSR/RT/env blend. Inspector: **RT Reflect**, **Reflect Debug**. Post FX: **RT Reflections**. Help: `docs/editor/water.md`.

**Still open on Halcyon:** live SSR miss-rate and before/after captures into `dev records/phase VV/`; profiler timings vs [`phase_VV.md`](phase_VV.md) §11. Ray-traced *refraction* is VV+1 and is not authorized by this handoff.

**Session estimate:** a follow-up Halcyon session should capture evidence, not rewrite the pass. VV-B already committed the architecture.

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
| `ssr_strength` | 1.0 | Near-field SSR mix; RT amount is `rt_reflect_strength` (also 1.0) |

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

**Halcyon living chrome (shipped):** Details **RT Reflect** / **Reflect Debug**; Post FX **RT Reflections**; Help `docs/editor/water.md`. Do not rebuild widgets, restyle Nocturne, or grow the icon atlas except a new `IconId` at the **end** of the enum.

### 3.5 Chronology (high level)

| When | Result |
|---|---|
| 2026-08-11 | 25M-2 night/shadow corrections |
| 2026-08-12–13 | Phase IV A–K, Great Lakes water, boat |
| 2026-08-13 morning | `b5e6052` IV close + Phase VV research plan (`phase_VV.md` created, no engine code) |
| 2026-08-13 | Phase XV A–J + BC7 (`43c3daa`, `8083328`) |
| 2026-08-13 | Phase 26 plan then 26-A–I (`719086c` … `ce16c31`) |
| 2026-08-13 evening | UI polish (`973b9a6`, `b5d1e57`) + immersive play, drawer tiles, ComboBox overlay, toolbar wiring |
| 2026-08-13 late | Phase VV-A–H (Halcyon) in tree; fragment sampled-texture limit fix (displacement vertex-only) |

---

## 4. What Halcyon is (and is not)

**Is (shipped):** replace SSR→env-cube as the *only* water reflection path with a blend of screen-space tracing (near field), hardware ray tracing (off-screen / behind camera / below horizon), and the environment cube (miss). Degrade to *exactly* today’s look without `EXPERIMENTAL_RAY_QUERY` or with `SOMNIUM_RT_REFLECT=0`.

**Is not:** path tracing, ray-traced refraction (VV+1), caustics, water-in-water, ReSTIR replacement, a software RT fallback (that is still 24P), a water retune, a terrain session, a Metaphor rebuild.

Stages (all in tree; each passed `cargo test --workspace`):

| ID | Work | Status |
|---|---|---|
| **VV-A** | GPU timer; SSR hit/miss debug viz; TLAS overflow log | Shipped (miss-rate number still open) |
| **VV-B** | G-buffer prepass + shading split | Shipped |
| **VV-C** | Half-res compute reflection | Shipped (lit, not left albedo-only) |
| **VV-D** | `rt_hit.wgsl`; sun + IBL; cascade shadow not a 2nd ray | Shipped |
| **VV-E** | GGX / skip foam (`roughness_skip` 0.72) | Shipped |
| **VV-F** | Reproject + accumulate + 2×2 upsample | Shipped |
| **VV-G** | Blend with SSR on confidence | Shipped |
| **VV-H** | Docs / ATTRIBUTION / Help / tests | Shipped except live evidence PNGs |

Fallback matrix and budgets: [`phase_VV.md`](phase_VV.md) §7 and §11. Kill switch: `SOMNIUM_RT_REFLECT=0`.

### 4.1 Infrastructure that already exists

| Capability | Location (verify; lines drift) |
|---|---|
| BLAS per mesh and terrain chunk | `pass/raytrace.rs`, `renderer.rs` |
| TLAS rebuilt per frame from `draw_queue` | `renderer.rs` |
| `EXPERIMENTAL_RAY_QUERY` gate | `context.rs` |
| Shared hit resolve | `shaders/rt_hit.wgsl` (`rt_trace`); GI wraps it |
| Water reflection compute | `pass/water_reflection.rs`, `shaders/water_reflection.wgsl` |
| Water G-buffer + shade | `water.wgsl` `fs_prepass` / `fs_main` |
| Water velocity + coverage | prepass MRT; TAA still uses coverage > 0.5 |

### 4.2 Still not in the tree

- Ray-traced refraction, caustics, water-in-water (water not in TLAS).
- Specular ReSTIR / a dedicated denoiser beyond temporal mix + upsample.
- Fragment-stage ray query (compute only).
- Software ray-tracing fallback (24P).
- Live evidence PNGs under `dev records/phase VV/`.

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

## 6. Next-session start checklist

1. Confirm branch `dev`. Read **this file**, then [`phase_VV.md`](phase_VV.md) §4.4, §6 shipped notes, §11, §13.
2. Do **not** re-implement VV-A–H.
3. Remaining authorized Halcyon work: live captures into `dev records/phase VV/` (after tonemap; do not invent PNGs) and filling §11 from the profiler.
4. VV+1 refraction, water-in-TLAS, or 24P only if the user asks.
5. Frozen: `WaterComponent::great_lakes` (especially `wave_speed` **0.85**). Displacement cascade BGL visibilities stay vertex-only (fragment sampled-texture limit).
6. `cargo fmt`, `cargo test --workspace` after any code change.

**If the user redirects to Metaphor:** follow [`phase_26.md`](phase_26.md) §13.2 / §20. Do not fold water reflections into a UI rebuild.

**If the user redirects to terrain:** live contract is [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). XV is closed; 25C/G/J/N/P remain parked.

---

## 7. Key paths

| Area | Path |
|---|---|
| Halcyon plan + shipped notes | `dev records/phase_VV.md` |
| Water pass / shader | `crates/somnium_renderer/src/pass/water.rs`, `shaders/water.wgsl` (`fs_prepass` / `fs_main`, `trace_ssr`) |
| Water reflection compute | `crates/somnium_renderer/src/pass/water_reflection.rs`, `shaders/water_reflection.wgsl` |
| Shared hit resolve | `crates/somnium_renderer/src/shaders/rt_hit.wgsl` |
| Water component / Great Lakes | `WaterComponent::great_lakes` (core); inspector in `somnium_ui` |
| Help | `docs/editor/water.md` |
| TLAS / BLAS | `crates/somnium_renderer/src/pass/raytrace.rs` (`MAX_TLAS_INSTANCES` 8192) |
| ReSTIR GI | `restir_gi.wgsl` (`gi_trace` → `rt_trace`) |
| Feature gate | `crates/somnium_renderer/src/context.rs` (`EXPERIMENTAL_RAY_QUERY`) |
| Skip chrome when immersive | `crates/somnium_renderer/src/renderer.rs` (`ui.is_immersive()`) |
| Editor seam | `crates/somnium_ui/src/editor_event.rs` ↔ `crates/somnium_core/src/app.rs` |
| Combo overlay (do not regress) | `crates/somnium_ui/src/widgets/combo_box.rs`, `popup.rs` |
| Evidence destination | `dev records/phase VV/` (create on first capture) |

---

## 8. Help / shortcuts already documenting immersive play

`docs/editor/viewport.md`, `docs/editor/shortcuts.md` (Esc), `docs/editor/welcome.md`, `docs/editor/outliner.md`, and **`docs/editor/water.md`** document reflections. `SOMNIUM_RT_REFLECT=0` is also in `phase_VV.md` §7.
