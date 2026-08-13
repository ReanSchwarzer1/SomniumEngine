# Somnium Engine — Post–Phase IV Context Handoff

> **Purpose:** IV/XV history after Phase IV closed. **Not the current start-here.**  
> **Current start-here:** [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md). Halcyon history: [`halcyon_context_handoff.md`](halcyon_context_handoff.md).  
> **Snapshot date:** 2026-08-13 (original); **current as of 2026-08-13 evening:** XV-A through XV-J are **complete**. 1.10 ms shading remains an exception. BC7 encoder + local packs recorded. Metaphor 26-A–I plus evening chrome (immersive play, ComboBox overlay, drawer tiles) are in the tree.  
> **Branch at audit:** `dev`  
> **Phase IV closed at:** `b5e6052` (`Phase IV completion + Phase VV Research`) and subsequent boat/Iris docs commits  
> **Audited HEAD:** `2dec6bd` (`color picker phase plan`) — later HEAD is on the Halcyon handoff  
> **Implementation status:** Phase IV **complete**; Phase XV **A–J complete** (1.10 ms exception recorded; BC7 encoder + local packs); Phase 26 (Metaphor) **26-A–I shipped, phase remains open** (new UI as later features land; 26-J not started); Phase VV (Halcyon) **VV-A–H in tree** (live SSR miss-rate capture still open) — start at the Halcyon handoff

**Current live contract** (supersedes “research-complete / not implemented”
below): 32 global layers, sidecar v4, 1664-byte `GpuTerrainMaterial`, unique
colour from splat, biome v3 / landscape v4, snow `relief * 0.48`, aerial hex/POM
off > 80 m above ground. Canonical: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).
Do not reintroduce a per-pixel terrain sample-count LOD. Do not retune
`WaterComponent::great_lakes`.

This document supersedes [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) as the **IV/XV history** file. For a new session, start at [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md). Keep the 25M-2 handoff for historical Phase IV A–J detail and the post-25M-2 shadow/night corrections.


---

## 1. Read this first

The next session that still needs IV/XV history should read these files **in order**. A new model starts at [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) instead.

1. [`context.md`](../context.md) — living architecture (XV-A–Zeta in engine; §20 is still Phase 14).
2. [`ATTRIBUTION.md`](../ATTRIBUTION.md) — reference/adaptation boundaries (§1.5 Metaphor chrome / colour picker; §1.6 XV A–Zeta).
3. [`phase_IV.md`](phase_IV.md) — completed Great Lakes water/terrain record, especially **§14 IV-K**.
4. **This file** — post-IV contracts and XV history. Live numbers: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).
5. [`phase_XV.md`](phase_XV.md) — full plan (XV-A–J). XV-J is complete.
6. [`assets/LICENSE.md`](../assets/LICENSE.md), [`assets/terrain/great_lakes/README.md`](../assets/terrain/great_lakes/README.md), and existing terrain material provenance under `assets/terrain/` / `assets/LICENSE.md`.

Optional depth (do not skip (1)–(5) for these):

- [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) — Phase IV A–J narrative and asset license conflict notes.
- [`phase_26.md`](phase_26.md) (Metaphor) / [`phase_VV.md`](phase_VV.md) — parallel tracks. Current start-here: [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md).

The root files `m2.md` and `m25.md` are still absent. Use [`phase_25m2_completion_report.md`](phase_25m2_completion_report.md) only if you need the 25M-2 boundary; it is not required to begin XV-A.

---

## 2. Current state in one page

### Shipped — Phase IV closed

- Default landscape is Motion Forge Pictures' Great Lakes height derivative + finite water body (ECS Water parented to Terrain).
- Water is a specialized HDR pass (not visibility-buffer opaque): wet-cell mesh + full-res mask/SDF/depth, Gerstner CPU queries, optional spectral FFT, coherent optics, underwater medium, shoreline contact foam, motion vectors.
- **IV-K** raised the spectral path to GodotOceanWaves-class fidelity: **3×1024²** FFT cascades, Jacobian foam, GDC 2019 *Atlas*-style lighting (with Somnium deviations). Evidence: `dev records/phase IV/IV-K/`.
- Authored shipping body: `WaterComponent::great_lakes` (see §4.2). Water datum **16.1 m**; optical `max_depth` **18.6 m**.
- Gislinge Viking Boat ships with `BuoyantVessel`, distributed buoyancy, Kelvin wake / prop wash. Environment sim runs in Editing and Playing; Pause freezes; Play hides editor overlays.
- Phase IV evidence lives under `dev records/phase IV/` only — never repository root.

### Shipped after IV-K close (same “post-IV” window)

- Boat sway restored: Gerstner `wave_speed` must stay **0.85** on the Great Lakes body (FFT is visual-only on CPU; Speed 0.2 froze the vessel).
- Vessel inspector fields (buoyancy / drag / yaw damp / thrust / draft / righting) and missing water inspector fields (`spectrum_blend`, anisotropy, caustic, edge; Wind/Foam/Whitecap drive spectral cascades).
- Improved buoyancy (up-biased force, angular drag, righting, draft).

### Parallel tracks

| Phase | Codename | Status | Plan |
|---|---|---|---|
| **XV** | Appalachia | **A–J complete** | [`phase_XV.md`](phase_XV.md) · live: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) · gate: [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md) |
| **26** | Metaphor | **26-A–I shipped**; chrome stays open as later features need UI. 26-J not started. | [`phase_26.md`](phase_26.md) |
| **VV** | Halcyon | **VV-A–H in tree** (2026-08-13). SSR + half-res RT + env cube. Kill switch `SOMNIUM_RT_REFLECT=0`. Evidence PNGs / §11 timings still open. | [`phase_VV.md`](phase_VV.md) · audit: [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) |
| **DF** | Daggerfall | **Plan only** (2026-08-14). Nested material clipmaps. | [`phase_DF.md`](phase_DF.md) |

**Parity bar for XV:** IV-K water is the photographic reference surface. Terrain fails XV if it only looks good as flat albedo swatches next to that water. Live look 2026-08-13 passed that bar for hue/seams/snow; XV-J GPU PNGs are in `phase XV/evidence/`.

### Session estimate

XV-A–J are done. **Phase VV (Halcyon) VV-A–H is in the tree.** Do not
re-implement A–H. Remaining Halcyon work is live captures and profiler
timings. Metaphor remains open as living chrome (do not rebuild 26-A).
Start-here: [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md).

---

## 3. Chronological change record (post–25M-2 → post-IV)

| Commit | Date | Result |
|---|---|---|
| `8c50382` | 2026-08-12 | Created `post_25M2_context_handoff.md`. |
| `42bd087` … `846dea7` | 2026-08-12 | Phase XV research plan + BGS/Godot references. |
| `8a7ed8c` … `45957ff` | 2026-08-12 | Phase IV-K partial / audit work. |
| `5c3c204` | 2026-08-12 | IV-K flagged for extensive audit. |
| `b5e6052` | 2026-08-13 | **Phase IV completion** + Phase VV research plan. |
| `d9a7c5b` | 2026-08-13 | Boat sway restored (`wave_speed` contract). |
| `11587c1` | 2026-08-13 | Vessel + water inspector editables. |
| `2dec6bd` | 2026-08-13 | Phase 26 Iris colour-picker plan. |
| *(local docs)* | 2026-08-13 | Phase XV research expansion (§5.5 wetness / CoD AVT / clipmaps / open-source) — commit if still dirty. |

Earlier IV-A through IV-J commits remain in [`post_25M2_context_handoff.md`](post_25M2_context_handoff.md) §3.

---

## 4. Phase IV closed claim (what XV must not regress)

### 4.1 Closed milestones

IV-A … IV-J (Great Lakes bake, ECS water, finite surface, optics, spectral/underwater, landscape factory, vessel/shoreline, docs) plus **IV-K** (ocean fidelity). Full narrative: [`phase_IV.md`](phase_IV.md).

### 4.2 IV-K shipped vs not

| Item | Result |
|---|---|
| K-2 … K-5, K-8 | **Done** — cascade params, 1024² FFT, Jacobian foam, Atlas-style lighting, evidence/docs. |
| K-1 ocean clipmap body | **Deferred** — finite wet-cell grid remains. |
| K-6 GPU sea spray | **Abandoned** — asset/attribution retained for a later attempt. |
| K-7 HDRI + Filmic | **Not attempted** — keep procedural sky + AgX. |

### 4.3 Authored Great Lakes water (do not casually retune)

| Field | Value | Contract |
|---|---|---|
| `surface_level` | 16.1 m | User-accepted water datum |
| `max_depth` | 18.6 m | Optical path (not bathymetry) |
| `clarity` / `amplitude` | 1.0 / 0.57 | |
| `roughness` / `ssr_strength` | 0.02 / 1.0 | |
| `wave_length_a/b`, `wave_steepness` | 18 / 11, 0.42 | Gerstner look |
| **`wave_speed`** | **0.85** | **Buoyancy samples Gerstner only** |
| `spectrum_blend` | 0.64 | Visual FFT crossfade |
| `wind_speed` / foam / whitecap | 6.5 / 4.5 / 0.54 | Drive spectral cascades |

CPU `sample_surface` remains **Gerstner-only**. Spectral FFT is GPU visual. Changing Speed to “hide” Gerstner will break the boat again.

### 4.4 Terrain baseline XV inherits

Eight PBR layers, two packed RGBA8 arrays each, two RGBA splatmaps, sidecar **v2**. GPU / auto-splat / sidecar already have 8; **paint/UI still clamp to layers 0–3**. Indices **0–7 are compatibility-locked**. Steep projection is still **albedo-heavy / incomplete PBR** — that is an XV-F problem, not something to “fix” by rewriting LOD. ReSTIR GI is **mean albedo × splat weights only**, not the full PBR path. Hex samples the surface pack as colour (`hex_sample_normal` unused). `GpuTerrainMaterial` is **448 bytes**, not 256. Runtime default 2K via `SOMNIUM_TERRAIN_RES`; committed packs are 4K; mips are box-filter of encoded bytes. `context.md` §20 is still Phase 14 / 4-layer — not live terrain API; do not rewrite it as if XV shipped.

Freeze cameras from `DefaultLandscapePreset` (~65 m snow band), **not** F7 (`auto_splat(..., 10.0)`).

Approximate Phase 25D baseline to beat or justify: terrain shader ~0.883 ms median in its reference corpus; landscape/eye-level ~11–12 material taps. Those are historical comparison hints. Live adapter/GPU/tap/memory freeze is still blocked on implementation authorization; do not invent new timings.

Full plan-vs-code list: [`phase XV/XV-A_codebase_map.md`](phase%20XV/XV-A_codebase_map.md).

---

## 5. Parallel phases (do not fold into XV)

### 5.1 Phase 26 — Metaphor

26-A–I plus the 2026-08-13 UX polish shipped (Nocturne shell, docked Content
Drawer, Iris 26-F, custom title bar, F1 Help, immersive play, ComboBox overlay).
**The UI phase is not over:** new engine features keep needing inspector fields,
menus, drawers, and `docs/editor/` pages. Do not restart at 26-A. Queued: 26-J
(only if requested), 26-H SDF, 26-D2 drag-drop. Contract: [`phase_26.md`](phase_26.md).
**Independent** of XV and of Halcyon except where a VV stage needs a debug toggle.

### 5.2 Phase VV — Halcyon

Ray-traced specular water reflections **shipped as VV-A–H** (2026-08-13):
SSR near-field + half-res RT + env cube on confidence. Kill switch
`SOMNIUM_RT_REFLECT=0`. Water/transparents stay out of the TLAS. Remaining:
live miss-rate PNGs and [`phase_VV.md`](phase_VV.md) §11 timings. Do not
re-implement A–H. Start-here: [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md).
VV history: [`halcyon_context_handoff.md`](halcyon_context_handoff.md).

An XV session that starts changing water reflection architecture or Metaphor UI widgets without user direction is out of scope.

---

## 6. Phase XV — research-complete handoff

### 6.1 Intent

**Appalachia:** eight → sixteen photogrammetry-quality PBR materials so the ground can match IV-K water under the same lighting.

> *“Sixteen times the detail.”* — Todd Howard  
> Sixteen terrain materials. It had to be Appalachia.

Public BGS production principles and Godot/O3DE/etc. references inform design; **no Bethesda assets or code**. No Quixel/Megascans. No AI-generated source materials.

### 6.2 Architecture decisions (locked unless evidence forces change)

- Sixteen **global** materials; **≤4** stored non-zero weights per splat texel; shader strongest-four before PBR sampling.
- **Four RGBA splatmaps** (direct weights), not indexed ID/weight maps. O3DE local source stores **top-two IDs** (docs say “top three”); manager path is `TerrainRenderer/TerrainDetailMaterialManager.cpp`. Keep Somnium’s direct splats.
- Sidecar **v3**: copy v2 layers 0–7 exactly; zero 8–15.
- Two packed arrays per layer; prefer **BC7** when `TEXTURE_COMPRESSION_BC` exists; RGBA8 fallback; never both resident.
- Default **2K**; 4K opt-in.
- Semantic offline mips (linear albedo, renormalized normals, Toksvig-style roughness from normal variance; Godot-reference comparison fixture in XV-B).
- **Full-PBR biplanar** cliffs (albedo, normal, roughness, AO, height); projected POM **off**; triplanar debug/reference.
- **Surface-gradient** normal composition mandatory; RNM for shared microdetail only.
- Manifest moisture affinity + **global wetness** in v1; painted wetness channel deferred.
- One versioned biome preset for startup and **Create → Terrain**; manual paint overrides survive rebuild when requested.
- Great Lakes shore + shipping water = **water-parity fixture**.

### 6.3 Proposed new layers (not downloaded)

| Index | Asset | Role | Source |
|---:|---|---|---|
| 8 | `aerial_sand` | Dry beach | <https://polyhaven.com/a/aerial_sand> |
| 9 | `coast_sand_01` | Damp shore | <https://polyhaven.com/a/coast_sand_01> |
| 10 | `dry_mud_field_001` | Dry earth | <https://polyhaven.com/a/dry_mud_field_001> |
| 11 | `cracked_red_ground` | Red mineral clay | <https://polyhaven.com/a/cracked_red_ground> — XV-A substitution: `terrain_red_01` is crushed reddish gravel, overlapping layer 7 `gravel_floor` |
| 12 | `sparse_grass` | Sparse grass | <https://polyhaven.com/a/sparse_grass> |
| 13 | `mossy_rock` | Mossy rock | <https://polyhaven.com/a/mossy_rock> |
| 14 | `rock_face_03` | Vertical cliff | <https://polyhaven.com/a/rock_face_03> |
| 15 | `ganges_river_pebbles` | Talus | <https://polyhaven.com/a/ganges_river_pebbles> — XV-A substitution: `dry_riverbed_rock` is a rock face, overlapping dedicated cliff layer 14 |

CC0 via Poly Haven; ambientCG is the audited fallback. XV-A first-party audit
and hashes: [`phase XV/XV-A_research.md`](phase%20XV/XV-A_research.md).

### 6.4 Milestones

| ID | Scope | Est. sessions | Status |
|---|---|---:|---|
| XV-A | Baseline, provenance, landscape-kit matrix, codebase map | 1–2 | **IN ENGINE** 2026-08-13 |
| XV-B | Manifest fetch/pack, semantic mips, BC7/RGBA8, Godot roughness fixture | 2 | **IN ENGINE** 2026-08-13; BC7 encoder same day (`encode_terrain_bc7`) |
| XV-C | Sixteen-layer CPU/editor, four-splat, sidecar v3. **Superseded by Zeta-C (32 / v4)** | 1–2 | **IN ENGINE** 2026-08-13 |
| XV-D | GPU layout, strongest-four, shared indexing with GI. GI is mean-albedo only | 1–2 | **IN ENGINE** 2026-08-13 |
| XV-E | Compression residency, specular stability | 1 | **IN ENGINE** 2026-08-13; BC7 packs local, visual A/B recorded |
| XV-F | Full-PBR biplanar cliffs, surface-gradient projection | 1–2 | **IN ENGINE** 2026-08-13 |
| XV-G | Biome preset + paint overrides + Create Terrain | 1–2 | **IN ENGINE** 2026-08-13 (biome **v3**) |
| XV-H | Physical scale, gradients, wetness response, hex/macro | 2 | **IN ENGINE** 2026-08-13 |
| XV-I | Sixteen-material UI + diagnostics (incl. wetness / projection) | 1–2 | **IN ENGINE** 2026-08-13 (palette now 32) |
| XV-Zeta | 32 layers, unique colour, paint UX, aerial LOD, biome v3 | 1 | **IN ENGINE** 2026-08-13 |
| XV-J | Verification, attribution, handoff | 1 | **COMPLETE** 2026-08-13 (1.10 ms exception; BC7 follow-up same day) |

**A–J are done.** GPU corpus and wgpu freeze: [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md).

### 6.5 Second research pass consequences (2026-08-13)

Recorded in [`phase_XV.md`](phase_XV.md) §5.5 / §9.10 of the older handoff:

1. Surface-gradient blending is mandatory language for XV-F/H.
2. Wetness validation is first-class (Hnat porous wetting model; Terrain3D paint-wetness is UX reference only).
3. Geometry clipmaps / CDLOD / CoD AVT stay **out of XV** (25C / future world-scale).
4. Debug views are acceptance criteria for XV-I before XV-J closes.
5. Water adjacency (Great Lakes shore) is required evidence.

### 6.6 XV-A codebase-map corrections (2026-08-13)

Do not treat the older plan wording as live API. Details: [`phase XV/XV-A_codebase_map.md`](phase%20XV/XV-A_codebase_map.md).

- Paint/UI clamp to **0–3**; GPU/auto-splat/sidecar already have 8.
- Terrain3D / PlumeSplat / Mikkelsen surfgrad demo are **not** in `example_repo` (web/GitHub only).
- F7 snow height is **10 m** (debug); Create → Terrain / preset snow cap is **`relief * 0.48` (~50.4 m)** as of landscape v4. Historical XV-A freeze used `* 0.62` (~65 m).

### 6.7 Explicitly deferred / rejected for XV

| Item | Status |
|---|---|
| Runtime VT / Far Cry AVT / CoD super-terrain | Deferred |
| Indexed material IDs | Deferred |
| LEAN mapping | Rejected (Toksvig first) |
| Full multilayer POM | Rejected |
| Tessellation / true displacement | Rejected |
| Texture bombing on hex | Rejected initially |
| Mix-Max transitions | Research reserve |
| Mesh scatter / foliage / decals | Future phase |
| Geometry clipmaps / CDLOD rewrite | Out of XV → 25C |
| Painted wetness control map | Deferred past v1 |

---

## 7. Budgets XV must meet (summary)

From [`phase_XV.md`](phase_XV.md) §10 — historical Phase 25D numbers are comparison hints only. Live baseline capture is still blocked on implementation authorization:

- ≤4 expensive material evals/pixel; base hex ≤24 material-map taps; steep biplanar ≤36.
- Landscape avg ≤12 taps; eye-level ≤18; median terrain shader ≤1.10 ms on the Phase 25 reference corpus, ≤20–25% regression vs pre-XV baseline without approved justification.
- 2K BC7 materials: original 16-layer budget 200 MiB; live 32-layer mixed 2048+1024 is **~213 MiB**. RGBA8 fallback ≤700 MiB; never both.
- Strongest-four vs offline all-layer reference: CIEDE2000 / normal / roughness error gates in §10.3.

---

## 8. Evidence already on disk

### Phase IV (do not relocate)

- `dev records/phase IV/IV-D-E/` — day/night post-TAA
- `dev records/phase IV/IV-F-G-H/` — surface, underwater, waterline
- `dev records/phase IV/IV-I-J/` — runtime + shoreline LOD
- `dev records/phase IV/IV-K/` — `ivk_before_shading.png`, `ivk_after_shading.png`, `ivk_authored_water_body.png`

### Phase XV (A–J complete)

- Path: `dev records/phase XV/evidence/phase_XV-<subphase>_<purpose>.png`
- Captures **after tonemapping** only.
- XV-J record: [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md).
- BC7 A/B: [`phase XV/evidence/XV-BC7_visual_check.md`](phase%20XV/evidence/XV-BC7_visual_check.md) (`phase_XV-BC7_*`, `phase_XV-RGBA8_*`).
- Live contract: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md).

Historical test counts in `phase_IV.md` are evidence of a past worktree, not a guarantee. Re-run `cargo fmt`, `cargo check`, and relevant tests before claiming XV-A ready.

---

## 9. Known issues outside Phase IV / XV

Do **not** silently expand XV to fix these (`context.md` remains authoritative for the backlog):

- Foliage colour wrong (trees salmon/pink, grass white) — investigate separately.
- Editor primitives from `on_init` may upload/gizmo but not appear.
- `BUG-013` water normal/mip seams — reproduce before assuming it still applies post–IV-K.
- 25M / 25M-2E / 25C / 25G / 25J / 25N / 25P and other planned rows remain open; they are not XV.

---

## 10. Next-session start checklist

XV-A–J are done. Metaphor 26-A–I plus evening chrome are in the tree (phase
open, do not rebuild). **Halcyon VV-A–H is in the tree.**

A **new model** starts at [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md)
(entire `context.md`, entire `ATTRIBUTION.md`, every markdown under `dev records/`).
VV-A–H history is [`halcyon_context_handoff.md`](halcyon_context_handoff.md):

1. Confirm branch `dev`. Read that handoff, then [`phase_VV.md`](phase_VV.md) §4.4, §6 shipped notes, §11, §13.
2. Do **not** re-implement VV-A–H.
3. Remaining authorized Halcyon work: live captures into `dev records/phase VV/` (after tonemap; do not invent PNGs) and filling §11 from the profiler.
4. Frozen: `WaterComponent::great_lakes` (especially `wave_speed` **0.85**).
5. Living chrome only if a later feature needs a new inspector field. 26-J only if requested.

**If the session is Metaphor instead**, follow [`phase_26.md`](phase_26.md) §13.2
(shipped vs still open) and do not fold water reflections into it.

Historical XV notes (phase complete): do not download Quixel/Megascans or
generate materials with AI; do not retune `WaterComponent::great_lakes`
`wave_speed` away from 0.85 without an explicit buoyancy plan. Full XV
contract: [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md). BC7:
[`phase XV/evidence/XV-BC7_visual_check.md`](phase%20XV/evidence/XV-BC7_visual_check.md).
Research record: [`phase_XV.md`](phase_XV.md).

---

## 11. Accuracy rule

Implementation and tests are truth; docs follow. This handoff is a snapshot at HEAD `2dec6bd` plus the 2026-08-13 XV research expansion. Current start-here is [`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md).

- Phase IV claims are **implemented** (with K-1 deferred, K-6 abandoned, K-7 skipped — stated, not implied).
- Phase XV **A–J are complete**; 1.10 ms shading remains a recorded exception (2026-08-13). BC7 encoder + local packs: [`phase XV/evidence/XV-BC7_visual_check.md`](phase%20XV/evidence/XV-BC7_visual_check.md).
- If a later session changes architecture, candidates, licenses, budgets, or milestone order, update [`phase_XV.md`](phase_XV.md) **and this handoff** (or a dated successor) rather than letting them diverge.

---

**AI disclosure:** Reconstructed from Phase IV completion records, IV-K section, post-IV boat/inspector commits, Phase XV research plan (including 2026-08-13 expansion), Iris/VV plans, living `context.md` / `ATTRIBUTION.md`, and on-disk evidence paths. It summarizes engineering contracts and source-use boundaries; it does not replace upstream licenses or the full bibliography in `phase_XV.md` §15.
