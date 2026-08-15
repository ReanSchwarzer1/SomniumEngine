# Audit plan — Phases CR, XV and VV

> **Written:** 2026-08-15, after the Phase DF audit
> ([`phase DF/DF-Audit_2026-08-15.md`](phase%20DF/DF-Audit_2026-08-15.md)).
> **Purpose:** finish the audit sweep. DF is done; CR, XV and VV are the
> remainder. With these three closed, every phase in the tree has been
> defect-hunted by a reader who did not write it.
> **Status:** plan only. No audit has started.

---

## 1. What the DF audit actually taught us

This is the most transferable part of the exercise and it should drive the
other three. The DF audit found seven defects. **None** of them was found by
looking at a rendered frame, and the one time a diagnosis was made from a
screenshot it was wrong three times running. What worked was reading code
against a specific question, and eliminating hypotheses with a single toggle or
a unit test.

Just as important: the largest defect found during the whole session
(GTAO returning zero visibility for the entire scene) was **not in Phase DF at
all**. It surfaced because a debug view was aimed at a non-terrain object. An
audit scoped to one phase will find bugs in others; that is a feature, and the
record should capture them wherever they land.

### 1.1 Defect taxonomy — hunt these in every phase

Each class below is one Somnium found in DF. They recur because they come from
the engine's shape, not from any one author's mistake.

| # | Class | The DF instance | How to hunt it |
|---|---|---|---|
| **C1** | **Silent early-return leaving a zero-filled or stale target** | `GtaoPass::record` returns if its bind groups are absent; nothing ever writes a neutral value. wgpu zero-fills, and zero means *fully occluded* / *nearest depth* / *no data* — always the most destructive reading | Grep every `record`/`dispatch` for early `return`. For each, ask what the consumer reads when the pass does not run. `restir_pass.clear_if_inactive` is the pattern that does this right — ask who lacks it |
| **C2** | **Validity flag that does not match what was actually written** | Clipmap `ready` was set per ring while a freshly-entered strip was still ungenerated | Find every "is this resource valid" bool. Ask what writes it, and whether the write can precede the data |
| **C3** | **Sign / handedness error in a reconstruction** | GTAO's `cross(dx, dy)` — the two screen axes disagree about handedness once `view_position` flips y | Trace each axis by hand from texel to view space. Check the result against a known-good invariant (`dot(normal, view_dir) > 0` for a visible surface) |
| **C4** | **Code compiled in for a path that cannot run** | hex and POM stayed in the shading PSO when the clipmap owned every terrain | For each pipeline, list what the spec compiles in and what the frame can actually reach. Runtime uniforms do **not** delete WGSL |
| **C5** | **Redundant per-pixel work** | Hex built its simplex grid and three taps once per *map* instead of once per *layer*; `strongest_four` ran 4×32 over a 32-entry `used` array with the weights passed by value | Look for identical arguments producing identical intermediates in sibling calls, and for `array<_, 32>` locals with dynamic indexing |
| **C6** | **Precision / encoding** | Linear albedo written to `Rgba8Unorm`; terrain albedo is 0.02–0.05 linear, so ~5 of 256 codes | For every 8-bit target, ask whether the stored quantity is perceptual or linear, and whether the mip chain filters it in the right space |
| **C7** | **A guard that promotes the small case to the worst case** | `expand_and_wrap` bailed to a full 1M-texel refresh whenever *either* expanded axis reached the ring size — which the common case always did | Find every `if (too big) { do everything }` and check what fraction of real inputs trip it |
| **C8** | **Documentation that has drifted from the tree** | `GpuTerrainMaterial` documented as 1664 bytes in three files; actual 2032 | Cross-check every size, default and file path a phase doc asserts |

### 1.2 Method

1. **Read first, in the order each section below gives.** Do not open a shader
   before the contract it implements.
2. **Form a hypothesis, then find the cheapest thing that eliminates it.** A
   unit test over shipped assets, a single env toggle, one debug view aimed at
   a specific object. Not a screenshot.
3. **Separate what the source proves from what needs the GPU.** Every DF
   finding was provable by reading; every *performance* claim was not. Say
   which is which in the record, explicitly.
4. **Do not fix while auditing unless the user asks.** DF's brief was read-only
   and the user lifted it; assume read-only until told otherwise.
5. **Record defects found outside the phase under audit.** They are the most
   valuable output, because nobody is looking for them.

---

## 2. Ordering, and why

**CR → XV → VV.**

- **CR first.** It is the smallest, it is pure CPU, and it decides *which
  instances exist and in what slot*. If instance-slot alignment is wrong, every
  image and every timing produced by the other two audits is suspect — this is
  the same class as the Phase 21 "fan of mirror-like shards" bug, where the
  instance buffer and the draw queue disagreed. Auditing CR last would mean
  re-doing work. It also owns hypothesis 4 in
  [`phase DF/DF-OPEN_clipmap_band_artifact.md`](phase%20DF/DF-OPEN_clipmap_band_artifact.md).
- **XV second.** It owns the shading cost the whole DF effort exists to reduce,
  and the DF fixes edited its shader (`terrain_material.wgsl`, `hextile.wgsl`).
  Re-running XV's gates is also what unblocks DF-E default-on, so this audit
  produces something DF needs.
- **VV last.** Most self-contained — it has its own passes, its own targets and
  a clean kill switch. Its remaining work is largely *evidence capture* rather
  than defect hunting, so it benefits from the other two being settled first.

Each audit is a session. Do not merge them: the DF audit was productive
precisely because it held one subsystem in view at a time.

---

## 3. Phase CR — Crysis (CPU occupancy and frustum early-out)

**Plan:** [`phase_CR.md`](phase_CR.md) · evidence
[`phase CR/CR-A_occupancy.md`](phase%20CR/CR-A_occupancy.md)

### 3.1 Read first

1. `phase_CR.md` in full — especially §2 (vis vs shadows) and §4 (non-goals).
2. `phase CR/CR-A_occupancy.md` — the measurement that motivated the phase.
3. `context.md` §21 (GPU-driven) and the Phase 15A/15B/15E rows in §17, because
   CR sits on top of them.
4. Then: `renderer.rs` (the terrain loop around the `draw_queue` /
   `shadow_only_queue` split, `rebuild_shadow_casters`, `clear_frame_queues`),
   `culling.rs`, `jobs.rs`, `pass/cull.rs`, `shaders/cull.wgsl`.

### 3.2 What to hunt

**Instance-slot alignment (highest severity).** This is the phase's real risk.

- `transparent_base = draw_queue.len() + shadow_only_queue.len()`. Confirm every
  consumer that indexes the instance buffer agrees with that layout.
- The indirect args are built from `draw_queue` only, while instances cover
  `draw_queue + shadow_only_queue + transparent_queue`. Confirm `first_instance`
  and `cull_aabbs` stay index-aligned with `cluster_args` **after** the
  single-sided/double-sided reorder in `push_cluster_args`. The reorder means
  argument order deliberately does not match draw order — verify the cull
  shader really does read the instance from `first_instance` and never from its
  own dispatch index.
- Ask what happens when `shadow_only_queue` is non-empty and `gpu_driven` is
  off (the F10 fallback path).

**C1 — silent early-outs.** `record_visibility`, `CullPass::record`,
`ShadowPass::record`: what does each do when its queue is empty, and what does
the next consumer read?

**CR-E cascade culling.** Cascades are fitted per frame from
`view_proj_unjittered.inverse()`. A caster sitting on a cascade boundary can
enter and leave `shadow_only_queue` frame to frame — check whether that
produces a visibly flickering shadow, and whether `aabb_in_any_frustum` is
conservative in the same direction as `chunk_in_frustum`.

**CR-D job pool.** `jobs.rs` goes parallel at **512+** items; the default tile
is **256 chunks**. Establish whether the parallel path is reachable in any
shipping configuration. If it is not, it is untested-in-practice code and the
audit should say so plainly rather than leaving it as implied capability.

**CR-F persistent buffers.** Confirm `clear()` (not reallocation) on every path
out of `render`, including the early return when the surface texture is lost.

**C8.** `phase_CR.md` claims "CPU frustum **default on**" and
`SOMNIUM_CPU_FRUSTUM=0`; verify both against `cpu_frustum_active`.

### 3.3 Gates / evidence

- Existing tests in `culling.rs` and `jobs.rs` — read them and ask what they do
  *not* cover (the straddling and behind-camera cases are covered; the
  shadow-only round trip may not be).
- Live: profiler `terrain chunks` row showing `vis / cpu-cull`, RMB looking
  away from the tile. CR-A's occupancy claim should be **re-measured** — it
  predates every shading change since.

### 3.4 Frozen

No camera-frustum culling of shadow casters. No wgpu multi-queue. Do not retune
foliage LOD or the Island recipe.

---

## 4. Phase XV — Appalachia (32-layer terrain materials)

**Plan:** [`phase_XV.md`](phase_XV.md) · live contract
[`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md) · gate
[`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md)

### 4.1 Read first

1. `phase XV/XV-Zeta_plan.md` — the live contract, not `phase_XV.md`'s plan text.
2. `phase XV/XV-A_codebase_map.md` — records where the plan and the code
   already disagreed once.
3. `terrain_shading_occupancy_2026-08-14.md` — the compact-PSO lesson.
4. The DF audit §7.2, because it already changed `hextile.wgsl` and
   `terrain_material.wgsl`.
5. Then: `shaders/terrain_material.wgsl`, `shaders/hextile.wgsl`,
   `terrain/textures.rs`, `terrain/splat.rs`, `terrain/blend.rs`,
   `terrain/mips.rs`, `terrain/macro_map.rs`, `shaders/rt_hit.wgsl`.

### 4.2 What to hunt

**Bindless index validity (highest severity).** `hero_bank_only` calls
`unbind_extra_bank`, which sets `albedo[i] = -1` / `surface[i] = -1` for layers
16–31 and clears splatmaps 4–7. `terrain_sample_layer` then does
`textures[albedo_map]` with an `i32` that can be **−1**. Establish whether a
splat weight for an unbound layer is reachable on Island — if it is, that is an
out-of-range bindless access. `terrain_fetch_splats` checks `if id >= 0` for
splatmaps; the layer maps appear not to.

**C6 — mip filtering space.** `post_IV_context_handoff.md` records that terrain
mips are "box-filter of **encoded bytes**". Filtering sRGB-encoded albedo as
raw bytes is the same class of error as the clipmap's linear-in-8-bit, and
`mean_linear_albedo` in the same file decodes properly — so the codebase knows
the distinction in one place and may not in the other. Check `terrain/mips.rs`
for: albedo filtered in linear then re-encoded; normals renormalised; height
and AO left linear; roughness raised from normal variance (Toksvig).

**C3 — tangent frame degeneracy.** `normalize(vec3(1,0,0) - geo_normal * geo_normal.x)`
collapses when `geo_normal` is near ±X, i.e. on a vertical wall facing east or
west. Check what `ts_to_surfgrad` / `resolve_surfgrad` do there, and whether
the biplanar cliff path (which is exactly where such normals occur) is
affected.

**C5 — remaining redundant work.** DF fixed hex and strongest-four. Still to
check: `terrain_projected_pbr` (does biplanar sample the same maps twice, the
way hex did?), `terrain_parallax_shadow`'s step count, and
`terrain_macro_sample` being called on paths that discard it.

**Live/clipmap divergence.** The live path applies `wetness_f0` via
`terrain_wet_f0`; `evaluate_clipmap_material` sets it to `0.0`. That is a real
difference in surface response between the two paths and it will show up in any
DF-E luminance gate. Decide whether it is intentional.

**C4.** Enumerate the `ShadingSpec` permutations a real session produces and
confirm none of them thrashes the PSO frame to frame (a recreate is a hitch).

**Gates never actually run** — record them as open rather than implied:
CIEDE2000 vs an offline all-layer reference; tap budget (≤24 base hex, ≤36
steep); residency ≈213 MiB; the 1.10 ms shading budget, which XV-J closed as an
explicit **exception**, not a pass.

### 4.3 Gates / evidence

- Re-measure at **maximized Native**, not 1280×720 — XV-J's freeze understated
  pixel cost, which is what `DF-A_timings.md` §Adapter freeze already notes.
- The DF §7.2 hex change should show up here; if it does not, that is a finding.

### 4.4 Frozen

32 layers, sidecar v4, `GpuTerrainMaterial` **2032** bytes, biome v3 /
landscape v4, snow `relief * 0.48`, aerial hex/POM cut at 80 m AGL. **No
per-pixel sample-count LOD** (XV-Zeta §11.1). Do not shrink
`TERRAIN_LAYER_COUNT`.

---

## 5. Phase VV — Halcyon (ray-traced water reflections)

**Plan:** [`phase_VV.md`](phase_VV.md) · history
[`halcyon_context_handoff.md`](halcyon_context_handoff.md) · audit brief
[`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md) §4.1

### 5.1 Read first

1. `post_halcyon_audit_handoff.md` §4.1 — the five questions already posed.
2. `phase_VV.md` §4.4 (what shipped), §6 stage notes, §7 (fallback matrix),
   §8 (risks — the exposure-disagreement warning is the one to take seriously),
   §11 (budgets, unfilled).
3. `phase_IV.md` §14 — IV-K's lighting model, which VV-D's hit shading must
   match.
4. Then: `pass/water.rs`, `pass/water_reflection.rs`, `shaders/water.wgsl`
   (`fs_prepass` / `fs_main` / `trace_ssr`), `shaders/water_reflection.wgsl`,
   `shaders/rt_hit.wgsl`, `pass/raytrace.rs`.

### 5.2 What to hunt

**C1 — the dummy and the history buffers.** `WaterReflectionPass::record`
returns a `bool`; on `false` the renderer binds `dummy_view()`. Establish what
the dummy actually contains, and confirm alpha 0 there means "no RT result" so
`fs_main` falls back to SSR + env cube. Then the harder cases: the **first
frame** after a resize (ping-pong history is freshly allocated, i.e.
zero-filled), and the frame after a **TLAS overflow** (RT is rejected — does
shade read a stale history that still claims alpha 1?).

**C2 — temporal validity.** The history mix uses water velocity plus a
depth/coverage disocclusion test. Ask what marks history valid, and whether
that flag can be set before the history has been written for a given pixel —
the exact shape of the clipmap `ready` bug.

**C3 — the water G-buffer packing.** `fs_prepass` stores `n.xz` and
reconstructs; check the sign convention for the reconstructed component and
whether it agrees between prepass, reflection compute and shade. This is the
same class as the GTAO normal.

**Exposure agreement (phase_VV §8's own top risk).** SSR returns fully-shaded
HDR pixels; RT hits return a re-shaded approximation using a cascade shadow
sample. If the two disagree, VV-G's confidence blend shows a moving seam. Read
`rt_hit.wgsl`'s shading against `shading.wgsl`'s and list every term one has
that the other does not (IBL intensity, aerial perspective, volumetrics,
emissive, local lights).

**C4.** Is the RT path compiled into `water.wgsl` when RT Reflect is off? Water
has no `ShadingSpec` equivalent — if the answer is "yes, always", that is the
same occupancy finding DF made, and worth stating even if it is not worth
fixing.

**Fallback matrix (phase_VV §7).** Rows 2 and 3 are the phase's credibility:
prove `SOMNIUM_RT_REFLECT=0` and a no-`EXPERIMENTAL_RAY_QUERY` adapter give an
image identical to today's. Row 3 needs an actual non-RT adapter or a forced
feature-off path.

**Standing contracts to re-verify.** Displacement cascade BGL visibilities are
**vertex-only** so the reflection sampled texture fits the fragment
sampled-texture limit of 16 — confirm that still holds after the FSR bind
groups landed. `trace_ssr` still present and used as near-field + degrade.
VV+1 refraction still **default off** on Great Lakes.

### 5.3 Gates / evidence — the phase's real debt

- **SSR miss-rate.** Reflect Debug = 1 colours SSR hits green, misses red,
  brightness = confidence. `phase_VV.md` §6 VV-A has been waiting on this
  number since the phase shipped. Capture it for the default landscape and
  record it. **Do not invent it.**
- **§11 budgets.** Reflection pass ≤ 2.0 ms at 1440p; VRAM ≤ 32 MB; zero frame
  cost with RT disabled. All three are `*open*` in the plan. Fill from the
  profiler's `Water reflection` scope.
- Before/after captures into `dev records/phase VV/` (the folder exists and is
  empty).

### 5.4 Frozen

`WaterComponent::great_lakes` — datum 16.1 m, optical `max_depth` 18.6 m,
Gerstner `wave_speed` **0.85**. Water and transparents stay **out** of the
TLAS. Do not remove `trace_ssr`. Do not start 24P (software RT).

---

## 6. Cross-cutting work, to do once rather than three times

- **C1 sweep.** One pass over every `pass/*.rs` `record` looking for early
  returns whose target is then read as if it were written. GTAO was one; there
  are ~35 passes. This is the single highest-value hour in the whole plan.
- **C8 documentation reconciliation.** Known drift already: `GpuTerrainMaterial`
  1664 vs **2032** (`context.md` front-matter, `halcyon_context_handoff.md`,
  `post_IV_context_handoff.md`); `post_halcyon_audit_handoff.md` §3 saying the
  occupancy/PSO work "may still be uncommitted" when it is in `aed1b08`;
  "target Rust 1.85" in `phase_IV.md` / `phase_VV.md` against the pinned 1.88;
  `context.md` §6.3/§13 documenting the packed `R32Uint` vis buffer that §18
  records as having become `Rg32Uint`.
- **A shared bench harness.** Every audit wants "same camera, one flag
  different, two captures". `SOMNIUM_CAPTURE` plus the maximize flag nearly
  does it. Writing that down once — with the **PowerShell** syntax, and the
  `Remove-Item Env:\…` cleanup — would prevent the session-persistent env-var
  trap that has already cost one confusing run.

---

## 7. Deliverables

Per phase, one file in that phase's folder, named `<PHASE>-Audit_<date>.md`,
containing:

1. **Headline** — the single most consequential finding, stated first.
2. **Findings table** — location, defect, severity, status.
3. **Checked and found correct** — explicitly, so the next reader does not
   re-audit it. DF's §2.1 is the model.
4. **Ruled out** — hypotheses eliminated and what eliminated them.
5. **Still open** — everything that needs a GPU, with the exact command.
6. **Validation** — what was run, verbatim.
7. **AI disclosure** — what was proved by reading versus by measurement.

Then update `phase_<X>.md` status and `context.md`, together, in the same
change.

## 8. What must not happen

1. No fixes during a read-only audit unless the user lifts the brief.
2. No invented evidence. If a number needs the GPU, it stays `*open*`.
3. No retuning of frozen contracts to make a gate pass.
4. No diagnosis from a screenshot. Aim a debug view at a specific question, or
   write a test.
5. No merging of the three audits into one session.
