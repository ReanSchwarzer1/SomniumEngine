# Somnium Engine — Post–Halcyon Audit Handoff

> **Purpose:** start-here for a **different model / session** that must
> (1) learn the engine, then (2) **audit everything from Phase VV (Halcyon)
> through HEAD**. Phase 26 (Metaphor) is in the tree and is **fine** — do not
> rebuild chrome, do not treat 26-A–I as the audit target.  
> **Snapshot date:** 2026-08-14  
> **Branch:** `dev`  
> **HEAD at write:** `2fd242d` (`foliage lod bug fix`) on top of `1cc33cd`
> (FSR 3). Later commits may exist; `git log` is truth.  
> **Toolchain:** `rust-toolchain.toml` pins **rustc 1.88**. Do not bump to 1.92.
> Workspace docs still say “target 1.85 / wgpu 29 / winit 0.30”; the *effective*
> MSRV is 1.88 (`image`, naga/wgpu).  
> **This file supersedes** [`halcyon_context_handoff.md`](halcyon_context_handoff.md)
> as the **current start-here**. Keep that file for VV-A–H history.

**Why an extra audit:** Halcyon through FSR landed fast, on a second model, in
the same window as lighting extras (24M–R), CDLOD morph (25C), analytic mips
(25N), foliage LOD (25P), and GPU cull fixes. Those paths touch jitter, depth
convention, instance counts, two-sided cone culling, and the present chain.
A fresh reader who already knows the architecture is more likely to catch
silent contracts than the session that wrote them.

---

## 0. Mandatory reading order (do this first)

**Do not open a shader or start “fixing” until this list is done.** Guessing
from a single file is how Great Lakes water, XV splat LOD, and FSR jitter were
almost broken.

1. **This file** — what to audit, what is frozen, what shipped after 26.
2. **The entire** [`context.md`](../context.md) — living architecture. Do not
   skip §3–§6, §12–§14 (frame graph), §17 (roadmap), §18 (bugs), §20 (Phase 14
   heightmap history — **do not rewrite it as XV**), §21 (GPU-driven).
3. **The entire** [`ATTRIBUTION.md`](../ATTRIBUTION.md) — reference boundaries.
   Halcyon is §1.7; FSR is §13B.8; XV is §1.6; Daggerfall is §1.8 (in engine,
   default off; clipmap audit is `phase_DF.md` §12). **No source copies.**
4. **Every markdown file under** [`dev records/`](.) — start with this folder’s
   [`README.md`](README.md), then:
   - [`phase_VV.md`](phase_VV.md) and [`halcyon_context_handoff.md`](halcyon_context_handoff.md)
   - [`phase_26.md`](phase_26.md) (chrome contract only)
   - [`phase_IV.md`](phase_IV.md) §14 (IV-K water freeze)
   - [`phase_XV.md`](phase_XV.md), [`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md),
     [`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md)
   - [`phase_DF.md`](phase_DF.md) — Daggerfall **in engine**, default off.
     Clipmap **audit** is §12 (separate session, other model). Do not retune
     clipmaps inside the Halcyon→HEAD audit unless the user redirects.
   - [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md)
     — Island vs Coastal fps, compact shading PSO, why inspector checkboxes
     did not drop Shading ms. **Read before “fixing” terrain frame time.**
   - historical handoffs (`post_IV_context_handoff.md`, `post_25M2_context_handoff.md`,
     `phase_25m2_completion_report.md`, `phase_IV.md`, XV research/evidence)
5. Help pages that document the live editor: [`docs/editor/`](../docs/editor/)
   (especially `viewport.md`, `lighting.md`, `water.md`, `terrain.md`).
6. Then, and only then, read the code listed in §5.

Optional depth after (1)–(6): `wgpu_api_gotchas.md`, `CONTRIBUTING.md`,
`third_party/wgpu-ffx/README.md`.

---

## 1. Frozen contracts (audit must not retune)

| Contract | Value | Why |
|---|---|---|
| Water | `WaterComponent::great_lakes`: datum **16.1 m**, optical `max_depth` **18.6 m**, Gerstner `wave_speed` **0.85** | Boat buoyancy samples Gerstner only; FFT is visual |
| Terrain look | 32 layers, strongest-four, sidecar v4, unique colour from splat, biome v3 / landscape v4, snow `relief * 0.48`, aerial hex/POM off **> 80 m AGL**. `GpuTerrainMaterial` **2032** bytes (XV **1664** body + DF clipmap fields). Do **not** shrink `TERRAIN_LAYER_COUNT` so Coastal “becomes Island.” Island publishes 16 layers (`hero_bank_only`); the GPU struct stays 32 slots. | [`XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) · [`phase_DF.md`](phase_DF.md) |
| Terrain LOD shader | **No per-pixel** `close` / `use_maps` / `layer_budget` sample branch | Compiled three paths into one shader; walking 20→27 ms |
| Clipmap default | Inspector **off**. Do not enable as the Coastal default to win fps. | Cheap shade path for a 1 km tile; DF audit is `phase_DF.md` §12 |
| Island look | Hex off, Parallax off, 16-layer hero bank. User signed off 2026-08-14. | Do not retune the recipe for frame time |
| Water / transparents in TLAS | **Out** | Halcyon non-goal; reflected shore will not show water lapping |
| `trace_ssr` | **Keep** | Degrade path and VV-G near-field |
| World Cache | **Default off** | Extra GPU on top of ReSTIR GI, not a speedup |
| Metaphor | Do not restart at 26-A; 26-J only if asked; new `IconId` **last** in the enum | Living chrome |
| Foliage LOD | User signed off 2026-08-14 — **no further LOD retune** unless they ask | Impostor is *not* a billboard |
| PowerShell | `;` not `&&` | Windows |
| `CARGO_TARGET_DIR` | Prefer repo `target/` | OneDrive |

---

## 2. Current state in one page

| Track | Status | Audit? |
|---|---|---|
| **26 Metaphor** | 26-A–I shipped; phase **open** as living chrome | **No extra audit** unless a post-26 UI regression is obvious |
| **VV Halcyon** | VV-A–H + VV+1 refraction in tree | **Yes — extra** |
| **24M–R / 24AB / 25C / 25N / 25P** | Started 2026-08-13 (`0d54c44`, `e240ad5`) | **Yes — extra** (defaults, sharing, correctness) |
| **FSR 3** | Default on (`1cc33cd`) | **Yes — extra** |
| **Foliage LOD + GPU cull** | Signed off (`2fd242d` and cull/Hi-Z follow-ups) | **Yes — extra** (do not “improve” LOD) |
| **IV / XV** | Closed | Regression-only |
| **Maps / Island look** | Coastal + Island recipes; Island look signed off | Regression-only (do not retune water or Island materials) |
| **Terrain shading occupancy** | Compact PSO in tree; Island **30+ fps**; Coastal ~**20 fps** on the ground with Hex/POM/PCSS off | **Read the notes — do not re-diagnose with inspector checkboxes.** [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md) |
| **DF Daggerfall** | **In engine**, default **off** — [`phase_DF.md`](phase_DF.md) | Own audit (§12). Do not implement or default-on during this audit |

**Still open (not an invitation to rewrite):**

- Halcyon live SSR miss-rate / before-after PNGs in `dev records/phase VV/`
- [`phase_VV.md`](phase_VV.md) §11 GPU timings vs profiler
- wgpu-ffx `GenerateReactive` unimplemented → water/transparents can ghost under FSR
- XV-J 1.10 ms shading budget **not met** (walk ~5.5 ms at 1280×720) — that is Daggerfall / a second aerial PSO, not a silent sample-count LOD
- Coastal ground fps after compact PSO (~20) vs Island (30+) — tile size / 32 published layers / full-screen terrain, **not** leftover POM. Notes: [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md)

---

## 3. Chronology after Phase 26

Phase 26 plan `719086c`, 26-A–I `ce16c31`, evening chrome `973b9a6` / `b5d1e57`.
**Everything below is the audit window.**

| When | Commit (approx.) | What landed |
|---|---|---|
| 2026-08-13 late | `e4c95b7` … `6fb3dff` | Phase VV Halcyon A–H |
| 2026-08-13 late | `70ab45e` | VV+1 RT refraction (default **off**) |
| 2026-08-13 night | `0d54c44` | 24M/N/O/P/Q/R/U/AB, 25C/N/P “trying to finish” |
| 2026-08-13 night | `e240ad5` | Disc/tube lights, packed mesh-SDF bake, SH probes, Details |
| 2026-08-14 | `17f9f69` | Bug fixes (post-start) |
| 2026-08-14 | `36b8fc7` | Viewport **Resolution** selector |
| 2026-08-14 | `1cc33cd` | **FSR 3 temporal upscale** (no frame gen) |
| 2026-08-14 | `2fd242d` | Foliage LOD: dummy impostor gone; later cull/cone/Hi-Z fixes in the same conversation |
| 2026-08-14 | worktree (check `git status`) | Maps (Coastal / Island). Compact shading PSO (`ShadingSpec` overrides). Uniform POM / ReSTIR-vis gates (necessary, **not** sufficient). Island **30+ fps**; Coastal still ~20 fps on the ground |

Line numbers in older plans drift. Verify against the worktree. The occupancy / PSO work may still be **uncommitted**.

---

## 4. What to audit (by subsystem)

### 4.1 Phase VV — Halcyon (extra)

Plan: [`phase_VV.md`](phase_VV.md). History: [`halcyon_context_handoff.md`](halcyon_context_handoff.md).

**Shipped:** water G-buffer prepass → half-res compute (`WaterReflectionPass`) →
shade; SSR / RT / env-cube blend on confidence; shared `rt_hit.wgsl`; GI wraps
`rt_trace`; TLAS cap 8192 with overflow log; foam skip; temporal mix + 2×2
upsample; inspector **RT Reflect** / **Reflect Debug**; Post FX **RT Reflections**.
VV+1 refraction is array layer 1, **default off**, IOR 1.333, water still not in
the TLAS.

**Audit questions:**

- Without `EXPERIMENTAL_RAY_QUERY` or with `SOMNIUM_RT_REFLECT=0`, is the image
  identical to SSR + env cube?
- Does VV-D hit lighting match IV-K (cascade shadow, not a second ray)?
- Displacement cascade BGL visibilities **vertex-only** (fragment sampled-texture
  limit 16) — still true after FSR bind groups?
- `trace_ssr` still present and used as near-field + degrade?
- Refraction default stays **off** on Great Lakes?

### 4.2 Lighting extras 24M–R, 24AB (extra)

Defaults in Help [`docs/editor/lighting.md`](../docs/editor/lighting.md).

| ID | What | Default | Shared-state traps |
|---|---|---|---|
| 24M | World-space radiance cache, 64³ camera clipmap | **Off** | Shares volume **alpha** with 24P |
| 24N | Scene-wide RT specular (not water) | **Off** | Must not fight Halcyon on water |
| 24O | Path tracer 1 spp accumulate | **Off** | Replaces raster while on |
| 24P | Mesh SDF 16³ bricks → 64³ cone trace | **Off** | Leave World Cache off while testing |
| 24Q | 4×4×4 SH L2 probes | **Off** | |
| 24R | Rect / disc / tube LTC | Create menu | |
| 24AB | Terrain Dbg 24–31 | | |
| 24U shafts | Shadow-tested in-scatter | On (Amt) | Do not retune fog to “see” shafts |

**Audit:** kill switches (`SOMNIUM_WORLD_CACHE`, `SOMNIUM_SPECULAR_GI`,
`SOMNIUM_PATH_TRACER`, `SOMNIUM_MESH_SDF`, `SOMNIUM_PROBES`) match inspector;
no default-on extra pass except what Help states.

### 4.3 25C / 25N / 25P (extra)

- **25C CDLOD morph** — packed in instance `_padding`; inspector **LOD Morph**
  default **off**. `SOMNIUM_LOD_MORPH=1`. Must not fight FSR jitter (morph is
  world-space).
- **25N analytic mips** — vis-buffer `textureSampleGrad`; **Analytic Mips**
  default on. Confirm barycentrics stay mesh-relative (`vertex_index / 3`).
- **25P foliage LOD** — **user accepted 2026-08-14. Do not change distances or
  part picking unless asked.**

  Live behaviour (not the old plan text in `context.md` §25P):

  - Dummy camera-facing **plane impostor deleted** (black triangle + FSR ghost).
  - **LOD** (default 45 m, horizontal): drop **leaf/cutout** parts (`is_leaf`),
    keep bark/branches.
  - **Impostor** (default 90 m, horizontal): keep solid parts only (same rule).
  - **Cull** (default 120 m, horizontal): skip the instance.
  - GPU: two-sided draws **disable normal-cone** cull (vis pipeline is
    `cull_mode: None`). Heavily instanced meshes skip cluster expansion but
    keep **one argument per instance** (cull shader used to smash
    `instance_count` to 0/1). Hi-Z skips candidates covering **> 25%** of the
    screen.

### 4.4 FSR 3 (extra)

Vendored `third_party/wgpu-ffx` + `third_party/wgpu-ffx-shaders-spv` (SPIR-V,
MSRV patched to 1.88). **Not** workspace members. Path dep from
`somnium_renderer`.

Pipeline: scene at **render res** → HDR (bloom/DoF/GTAO at render res) →
**Karis compress** → FSR → **untonemap** → tonemap at **display** → gizmos/UI.

| Item | Contract |
|---|---|
| Features | `FSR_FEATURES`: adapter format features, passthrough shaders, 16-bit norm, float32 filterable, `CLEAR_TEXTURE`; `max_storage_textures_per_shader_stage` ≥ 16 |
| Depth | Vis buffer `Depth32Float`, Less, clear 1.0, **0 = near, 1 = far**. **Not reverse-Z. Do not set `DEPTH_INVERTED`.** Sanitize copies depth to R32Float |
| Motion | `prev_uv - current_uv` (history lookup). Unjittered matrices. Scale **positive** `[render_w, render_h]` |
| Jitter | wgpu-ffx Halton pixels `[-0.5, 0.5]`. Bevy: `jitter_ndc = (jx*2/w, -jy*2/h)` **added to `proj.z_axis.xy`**. **Never** `translate * projection` on `perspective_rh` |
| Flags | FSR context `empty()` — colour already Karis 0–1. `pre_exposure: 1.0`. `reactive_mask: None` |
| Compress scale | `renderer.exposure` uniform **16 bytes, four f32s** — not the adapting meter. WGSL `vec3` pad would expect 32 |
| Overlays | Unjittered `view_proj` before gizmos/outlines/particles |
| TAA/CAS | Forced off while FSR on |
| Cull VP | **Unjittered** — jittered planes made foliage vibrate |
| Kill | `SOMNIUM_FSR=0` |

Known: water fully-reactive was wrong (showed jittered frame); removed. Inf
clamp alone was not enough (Lanczos undershoot on 100k lux HDR).

### 4.5 GPU cull (extra, tied to foliage)

`shaders/cull.wgsl`: do not replace a live `instance_count` with `1`. Phase 2
revival is a single instance because the CPU no longer batches `N` copies onto
one argument. Cone test `cone.w = 2` disables. Large screen-space AABB → not
occluded (CPU mirror in `culling.rs`).

### 4.6 Terrain shading occupancy (extra — already diagnosed)

Full notes: [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md).
Help: [`docs/editor/terrain.md`](../docs/editor/terrain.md) (Hex / Parallax /
Clipmap / Maps), [`docs/editor/lighting.md`](../docs/editor/lighting.md)
(**Soft Shadows** = PCSS).

**Do not spend the audit re-testing inspector checkboxes for Shading ms.**

Runtime uniforms do not delete WGSL. Occupancy is the union of every path still
in the compiled shading shader. Measured on Island (terrain selected): Shading
~41 ms of ~53 ms total, ~20 fps — same ballpark as Coastal ~18 fps. Uniform POM
skip, uniform ReSTIR sun-vis (`shading_mode` bit 4), and unchecking Parallax /
Soft Shadows / Contact **did not drop** frame time.

**What did:** `ShadingSpec` + `ensure_pipeline` overrides (`enable_hex`,
`enable_pom`, `enable_pcss`, `enable_contact`, `enable_clipmap`, `enable_debug`,
`terrain_scan`). Island (hex off, POM off, 16-layer scan) stays on
`ShadingSpec::COMPACT` → **30+ fps**. Coastal with Hex / Parallax / Soft Shadows
unchecked is still ~**20 fps on the ground**: 1024 m / 256 chunks / 32 published
layers / almost every pixel is terrain. Unchecking those boxes does not turn
Coastal into Island.

**Audit questions (regression only):**

- Island still loads compact (hex off, POM off, `hero_bank_only`,
  `terrain_scan = 16`)? Look signed off — do not retune the recipe.
- Coastal still 32 layers / 8 splatmaps? Do not shrink the GPU format.
- Clipmap still default **off**? Do not enable it as the Coastal default.
- Soft Shadows still the PCSS checkbox (no “PCSS” label)?
- `shading_mode` bit 4 still set when ReSTIR DI + TLAS are live?

If profiler Shading on Coastal is now near Island and Vis/Shadow ate the extra
~15 ms, that is the 256-chunk tile — not leftover POM.

---

## 5. Key paths (verify; lines drift)

| Area | Path |
|---|---|
| FSR | `crates/somnium_renderer/src/pass/fsr.rs`, `shaders/fsr_sanitize.wgsl`, `shaders/fsr_untonemap.wgsl` (ATTRIBUTION **§13B.8**) |
| Features | `crates/somnium_renderer/src/context.rs` (`FSR_FEATURES`) |
| Jitter / overlay VP | `crates/somnium_renderer/src/renderer.rs` `set_view`, `write_view_buffer` |
| Cull | `shaders/cull.wgsl`, `culling.rs`, cluster args in `renderer.rs` |
| Foliage submit / LOD | `crates/somnium_core/src/app.rs` `submit_foliage`, `ensure_palette_mesh` |
| Water RT | `pass/water.rs`, `pass/water_reflection.rs`, `shaders/water.wgsl`, `shaders/water_reflection.wgsl`, `shaders/rt_hit.wgsl` |
| Lighting extras | `pass/lighting_extra.rs`, `shaders/lighting_extra.wgsl` |
| Terrain material | `shaders/terrain_material.wgsl` (strongest-four, hex, POM, `enable_*` overrides) |
| Compact shading PSO | `pass/shading.rs` (`ShadingSpec`, `ensure_pipeline`); spec built in `renderer.rs` before Shading |
| Help | `docs/editor/viewport.md`, `lighting.md`, `water.md`, `terrain.md` |
| Occupancy notes | [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md) |

---

## 6. Must not break / must not do

1. Do not retune Great Lakes water or XV-Zeta look numbers.
2. Do not put water or transparents in the TLAS.
3. Do not remove `trace_ssr`.
4. Do not default World Cache on.
5. Do not reintroduce per-pixel terrain sample-count LOD.
6. Do not copy UE / O3DE / AMD shader source. Cite in `ATTRIBUTION.md`.
7. Do not invent evidence PNGs.
8. Do not “bring back” the foliage billboard impostor.
9. Do not collapse foliage instances into `instance_count > 1` without teaching
   the cull shader to preserve `N`.
10. Play / Pause / Stop / immersive Esc, foliage Type combo, terrain paint,
    boat buoyancy, FSR Sharp, Resolution combo stay working.
11. Do not implement or default-on [`phase_DF.md`](phase_DF.md) during the
    audit unless the user says to start Daggerfall. Clipmap stays inspector
    **off**; do not enable it as the Coastal default to win fps.
12. Do not shrink `TERRAIN_LAYER_COUNT` / `GpuTerrainMaterial` so Coastal
    matches Island’s 16 published layers. Island is `hero_bank_only`.
13. Do not “fix” Shading ms by flipping Hex / Parallax / Soft Shadows /
    Contact and calling it done. Those uniforms do not delete compiled paths;
    the compact PSO already does when they are off. See §4.6.

---

## 7. Suggested audit method

1. `git log --oneline 719086c..HEAD` (26 plan → now). Read the diffs for
   `fsr.rs`, `cull.wgsl`, `water_reflection.wgsl`, `app.rs` foliage, `lighting_extra`.
2. `cargo fmt`, `cargo test --workspace` (expect minutes).
3. Play Great Lakes: FSR on Native and 1080p; water SSR vs RT Reflect; foliage
   grove close/mid/far; World Cache left off.
4. Write findings as defects with file+reason. Do not drive-by refactors.

---

## 8. Accuracy rule

Implementation and tests are truth; this handoff is a snapshot at 2026-08-14
(occupancy addendum the same day). If architecture, defaults, or contracts
change, update **this file**, [`context.md`](../context.md), and
[`ATTRIBUTION.md`](../ATTRIBUTION.md) together.

**AI disclosure:** Reconstructed from the Halcyon handoff, `context.md` /
`ATTRIBUTION.md`, git history `ce16c31`→`2fd242d`, FSR/foliage session notes,
Help pages, and the 2026-08-14 Island/Coastal shading occupancy session. It
does not replace licenses or the full VV/XV/IV plans.
