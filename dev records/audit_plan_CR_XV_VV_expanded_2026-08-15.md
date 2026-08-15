# Consolidated audit and research plan — CR → XV → VV

> **Written:** 2026-08-15  
> **Status:** source reconnaissance and execution plan; the live GPU audit has not yet run  
> **Scope:** one master audit document integrating
> [`audit_plan_CR_XV_VV.md`](audit_plan_CR_XV_VV.md) with the newly reported
> post-processing, lighting-extra, water-reflection, path-tracing and probe
> failures. Execution remains strictly ordered **CR → XV → VV**.  
> **Change policy:** this document plans and prioritizes the audit. It does not
> authorize code fixes or invent live evidence.

---

## 1. Outcome of the research pass

The reported effects do not reduce to one broken Details checkbox. Source
inspection found a mixture of:

1. an intentional dependency that the UI does not communicate (**CAS is
   suppressed while FSR is enabled**);
2. probable lifecycle defects (**bloom's tone-map binding can retain the old
   bloom texture after resize**);
3. incomplete temporal systems (**water reflections, scene RT specular and the
   path tracer**);
4. lighting-composition errors that can plausibly bleach materials (**RT
   specular and probes**); and
5. two mathematical defects in volumetric integration/cascade selection that
   can prevent stable light shafts.

These are source-backed findings, not claims that a particular screenshot has
already been reproduced. The audit must separate these confidence levels:

| Mark | Meaning |
|---|---|
| **S — source-proven** | The code contains the stated state transition, equation or binding error. A live test determines impact, not existence. |
| **H — high-confidence symptom link** | The code defect closely matches the reported visual symptom, but a controlled A/B is still required. |
| **L — live-only** | Source inspection is inconclusive; the GPU capture/profiler is the evidence. |

### 1.1 Priority queue before implementation

| Priority | Track | Source finding | Confidence | Reported symptom it can explain |
|---|---|---|---|---|
| **P0** | Path tracer | One shared `aux` history is considered valid across lighting-extra modes; path history resets for camera translation only, not rotation, projection, scene/material/light changes or mode changes | S/H | Afterimages of everything; path tracer unusable |
| **P0** | Post FX control plane | The inspector records requested booleans, while effective renderer state has hidden dependencies/hardware gates. No effective-state feedback or end-to-end automated toggle test exists | S | Toggles look checked but appear inert |
| **P1** | Bloom | `PostProcessPass` clones `bloom_view` at construction; bloom recreates its views on resize, but post-process is not rebound to the new result | S/H | Bloom stops affecting the image after a viewport/window resize |
| **P1** | Volumetrics/shafts | Per-slice jitter uses `fract`; with two samples it can make the second sample move backward, producing negative `dt`. Cascade selection is passed ray distance `t`, not view-space z | S/H | Light shafts invisible or unstable; volumetrics suspect |
| **P1** | RT water reflections | One stochastic ray/pixel is accumulated with weak validation; “previous” depth/coverage is sampled from the current water G-buffer. Cascade atlas bounds are checked after applying the quadrant offset | S/H | Dancing black splotches and shimmering on water |
| **P1** | Scene RT specular | RT-hit radiance omits the engine BRDF/shadow/IBL contract, then replaces ambient according only to confidence and roughness; 85% same-UV history has no reprojection | S/H | Boat and terrain materials turn white/whitish |
| **P1** | SH probes | Probe irradiance derived from the environment is added on top of the existing environment IBL diffuse term | S/H | Entire scene becomes whitish with probes enabled |
| **P2** | CAS UX | `cas_enabled` is forced ineffective whenever FSR is on; FSR is on by default and owns RCAS sharpness | S | CAS checkbox appears to do nothing |
| **P2** | CR/XV/VV original debt | Instance-slot, bindless-index, mip/normal, fallback, evidence and budget gates from the original plan remain open | S/L | Foundational correctness and performance uncertainty |

Do not fix in this priority order blindly. First execute the ordered audit
below; CR and XV establish that instance/material inputs are trustworthy before
VV and the lighting extras are judged.

---

## 2. Engine model used by this audit

The current engine is a Rust/wgpu visibility-buffer renderer. `somnium_core`
owns the ECS and copies the first scene `PostProcessComponent` into renderer
state every frame. Opaque meshes and terrain write visibility/depth; the shared
shading pass resolves materials and lighting into HDR. Water is a later
specialized HDR path. Bloom, DoF and temporal/upscale passes operate before the
display-resolution tone map and editor overlays.

The relevant frame chain is:

```text
ECS Post Processing entity
  → app.rs apply_post_process (requested settings → effective pass flags)
  → CPU/GPU culling and visibility/depth
  → lighting extras / volumetrics / shared opaque shading
  → water prepass → half-res RT reflection/refraction → water shade
  → motion blur / TAA or FSR / DoF / bloom as configured
  → tone map + grading
  → FXAA / CAS when effective
  → gizmos, outline and editor UI
```

Two architectural consequences control this audit:

- **Runtime checkbox state is not effective state.** FSR disables Somnium TAA
  and CAS; ray-query effects are gated by adapter support; shafts require the
  volumetric pass; values of zero can make an enabled pass visually neutral.
- **The built-in `.somcap` capture is HDR and occurs before final display-only
  passes.** It cannot prove CAS/FXAA/display-resolution tone-map behavior. Those
  tracks require swapchain capture or a dedicated pass-output/debug counter.

Documentation drift discovered during reading must be corrected only after the
audit: current `GpuTerrainMaterial` is **2032 bytes**, while several older
handoffs still say 1664; older frame-order diagrams also omit later passes.

---

## 3. Shared defect taxonomy and method

Carry forward the original DF-derived classes:

| ID | Defect class | Audit question |
|---|---|---|
| **C1** | Silent early return / stale or zero target | If the pass does not write, what does its consumer read? |
| **C2** | Validity flag does not describe written data | Can `history_valid`/`ready` become true before every required pixel/resource is valid? |
| **C3** | Sign, handedness or reconstruction error | Do encoded normals, depth, velocities and matrices reconstruct in one convention? |
| **C4** | Compiled but unreachable work | Which WGSL paths remain in a PSO even when runtime flags make them unreachable? |
| **C5** | Redundant per-pixel work | Are identical intermediates recomputed or large arrays dynamically indexed? |
| **C6** | Precision/encoding mismatch | Is the stored quantity linear/perceptual and filtered in the correct space? |
| **C7** | Worst-case fallback guard | Does a common input accidentally invoke the full refresh/fallback? |
| **C8** | Documentation drift | Do sizes, defaults, ordering and evidence claims match HEAD? |
| **C9** | Hidden dependency/effective-state mismatch | Does the UI show requested state when another feature or capability suppresses it? |
| **C10** | Temporal ownership/invalidation error | Does each history have one producer, compatible guides and complete reset triggers? |
| **C11** | Lighting double-count or BRDF mismatch | Is radiance added/replaced exactly once and with the same material/energy convention as raster shading? |

For every hypothesis:

1. prove or eliminate it with source first;
2. choose the cheapest discriminator: unit test, pass counter, debug view, then
   controlled live A/B;
3. record requested state, effective state, pass execution and visible output as
   four separate facts;
4. never infer correctness from “the checkbox moved” or performance from a
   runtime uniform; and
5. label measurements with adapter, API, driver, resolution, render scale,
   FSR/TAA state, camera and frame count.

---

## 4. Stage 1 — Phase CR (Crysis)

**Primary records:** [`phase_CR.md`](phase_CR.md),
[`phase CR/CR-A_occupancy.md`](phase%20CR/CR-A_occupancy.md), and the CR section
of the original audit plan.

### 4.1 Contract

CR adds a default-on CPU camera-frustum early-out for terrain chunks while
preserving off-camera shadow casters through `shadow_only_queue`. GPU 15B/F10
remains independent. It also adds optional parallel CPU work at 512+ items,
per-cascade filtering and persistent scratch buffers.

### 4.2 Audit work

#### CR-1 — instance-slot alignment (**highest severity**)

- Prove the instance buffer layout is always
  `draw_queue + shadow_only_queue + transparent_queue`.
- Prove `transparent_base` uses the same layout on GPU-driven and CPU fallback
  paths.
- `cluster_args` are reordered single-sided then double-sided. Confirm all cull
  and visibility consumers use each argument's `first_instance`, never the
  argument's dispatch index.
- Add a focused round-trip test with at least one visible opaque, one
  shadow-only and one transparent draw. Assert geometry/material/transform IDs
  at the final consumer, not just argument counts.
- Repeat with GPU-driven disabled to cover the F10 fallback.

#### CR-2 — conservative culling and shadow continuity

- Test camera and cascade AABBs exactly on every plane, just inside/outside, at
  large coordinates and behind the camera.
- Record a slow camera orbit with a caster crossing a cascade boundary. Reject
  any one-frame shadow disappearance.
- Confirm camera frustum never removes a caster that intersects any cascade;
  confirm the cascade test itself remains conservative.

#### CR-3 — early returns and lifetime

- Sweep `CullPass::record`, visibility, shadow recording and surface-acquire
  failure paths for C1.
- On every exit, verify persistent vectors are cleared but capacity retained:
  `shadow_only_queue`, rebuilt chunks, cull AABBs, cluster args and shadow
  scratch.
- Ensure no stale indirect args survive an empty frame.

#### CR-4 — parallel path truth

- The rayon threshold is 512; the common terrain has 256 chunks. Establish a
  shipping/repro configuration that actually crosses the threshold.
- If none exists, mark the parallel branch “source-tested only / live
  unreachable” rather than implying it has production evidence.
- For 511/512/513 inputs, assert byte-identical ordering and results between
  serial and parallel paths.

#### CR-5 — defaults and control plane

- Verify `cpu_frustum_active()` is default on, `SOMNIUM_CPU_FRUSTUM=0` wins over
  UI state, and inspector refresh cannot flip it. This setter correctly consumes
  the checkbox value rather than treating it as another toggle; use that pattern
  later for the Post FX audit.
- Verify `SOMNIUM_CASCADE_CULL=0` separately.

### 4.3 CR exit gate

- Unit tests pass for slot layout, 511/512/513 parity and conservative planes.
- Looking away from terrain increases `cpu-cull` without changing shadow
  correctness.
- CR-A occupancy is re-measured on the current renderer; historical numbers are
  not copied forward.
- Only after these pass may XV image or performance evidence be treated as
  trustworthy.

**Frozen:** no camera-frustum culling of shadow casters, no multi-queue rewrite,
no foliage-distance retune, no Island recipe retune.

---

## 5. Stage 2 — Phase XV (Appalachia)

**Primary records:** [`phase_XV.md`](phase_XV.md),
[`phase XV/XV-Zeta_plan.md`](phase%20XV/XV-Zeta_plan.md),
[`phase XV/evidence/XV-J_compile_gate.md`](phase%20XV/evidence/XV-J_compile_gate.md),
and [`terrain_shading_occupancy_2026-08-14.md`](terrain_shading_occupancy_2026-08-14.md).

### 5.1 Live contract

32 global layers; strongest four locally; sidecar v4; biome v3 / landscape v4;
unique colour from splat; snow `relief * 0.48`; aerial hex/POM disabled above
80 m AGL; `GpuTerrainMaterial` 2032 bytes after DF; BC7 hero 2K + extra 1K
packs around 213 MiB; RGBA8 fallback; no per-pixel terrain sample-count LOD.

### 5.2 Original XV audit work

#### XV-1 — bindless index validity (**highest severity**)

`hero_bank_only` unbinds layer 16–31 textures to `-1`. Prove that no non-zero
weight for those layers can reach `terrain_sample_layer`, whose live path indexes
the bindless array without an explicit `>= 0` guard.

- Validate startup, Create → Terrain, sidecar v3→v4 migration, paint undo/redo,
  preset switch and dirty splat upload.
- Add a CPU mirror/test over every shipped splat texel: every selected layer has
  non-negative albedo/surface IDs, and every unbound layer has zero effective
  weight.
- Prefer an explicit shader guard/fallback after the audit even if current
  assets prove safe; asset invariants alone make future corruption silent.

#### XV-2 — semantic mip correctness

- Albedo: decode to linear, filter, re-encode for storage.
- Normals: filter vectors and renormalize; reconstruct z safely.
- Roughness: include normal variance/Toksvig behavior rather than box-filtering
  encoded roughness alone.
- AO/height: remain linear with documented reduction choice.
- Run BC7 versus `SOMNIUM_TERRAIN_FORCE_RGBA8=1` on the same camera and preserve
  a numeric diff plus material close-ups.

#### XV-3 — tangent-frame degeneracy

Both live and clipmap paths construct a tangent by normalizing X projected onto
the surface. This collapses for normals near ±X, exactly a common biplanar cliff
case. Add an axis-selection helper and tests over the six cardinal normals plus
near-cardinal epsilon cases. Validate no NaN reaches surface gradients,
roughness or the final HDR target.

#### XV-4 — redundancy and PSO reachability

- Verify DF's shared hex grid/taps and strongest-four changes remain active.
- Count actual taps for flat, transition and cliff pixels with hex/POM on/off.
- Inspect `terrain_projected_pbr`, parallax shadow and macro sampling for work
  whose result is discarded.
- Enumerate real `ShadingSpec` keys and cache hits. Reject per-frame PSO creation
  while moving between terrain zones.
- Remember: runtime toggles do not reduce shader occupancy. Only compare
  specialized PSOs when making performance claims.

#### XV-5 — live/clipmap material parity

The live path writes `terrain_wet_f0`; the clipmap path explicitly sets it to
zero. Compare albedo, normal, roughness, AO, wet F0 and macro contribution at
the same world points. Record deliberate differences; treat accidental ones as
DF/XV interaction defects, not artistic tuning opportunities.

#### XV-6 — gates that were never completed

- Strongest-four versus offline all-layer reference: CIEDE2000, normal angle and
  roughness errors.
- Base hex ≤24 material-map taps; steep biplanar ≤36; landscape/eye-level
  averages documented.
- Residency around 213 MiB BC7 and ≤700 MiB RGBA8, never both resident.
- Re-measure shading maximized/native. XV's 1.10 ms target was closed as an
  explicit exception, not a pass.

### 5.3 New XV regression fixture for “white terrain materials”

Do not immediately blame terrain textures when RT Specular or Probes bleach the
terrain. Add a fixed terrain-and-boat fixture and capture these isolated terms:

| Capture | RT Specular | Probes | Base IBL | Purpose |
|---|---:|---:|---:|---|
| XV-W0 | off | off | normal | Trusted raster material baseline |
| XV-W1 | on | off | normal | Scene RT specular delta |
| XV-W2 | off | on | normal | Probe-only delta |
| XV-W3 | off | on | zero/isolated debug | Determine whether probes replace or duplicate environment diffuse |
| XV-W4 | on | off | ambient/spec debug | Inspect whether white energy arrives through `lighting_aux` |

Use material-ID and term debug views so a lighting failure is not recorded as a
bad albedo/splat. Test terrain, boat paint/wood/metal and a neutral 18% grey
Lambertian object in the same frame.

### 5.4 XV exit gate

- No reachable negative bindless index and no tangent NaN.
- Live and clipmap differences are enumerated and intentional or filed.
- The white-material fixture proves whether terrain data is correct before the
  lighting-extra audit begins.
- Current timings, tap counts, residency and image diffs are recorded without
  retuning the frozen look.

**Frozen:** 32 layers, 2032-byte live struct, sidecar v4, biome/landscape
versions, snow and 80 m AGL cut, no per-pixel sample-count LOD.

---

## 6. Stage 3 — Phase VV (Halcyon) and the post-processing/lighting audit

**Primary records:** [`phase_VV.md`](phase_VV.md),
[`halcyon_context_handoff.md`](halcyon_context_handoff.md),
[`post_halcyon_audit_handoff.md`](post_halcyon_audit_handoff.md), and
[`phase_IV.md`](phase_IV.md) §14.

Run **VV core first**, then the all-toggle control-plane sweep, then each newly
reported effect. This keeps the requested CR → XV → VV ordering while producing
one integrated result.

### 6.1 VV core — original Halcyon audit

#### VV-1 — fallback/dummy/history correctness

- Inspect the dummy texture's clear value and prove alpha 0 means “no RT hit,”
  restoring SSR + environment.
- Test first frame, re-enable, resize, TLAS overflow and unsupported ray-query
  paths. No stale alpha-1 history may survive.
- Prove `SOMNIUM_RT_REFLECT=0` and a forced no-ray-query path are visually
  identical to the pre-RT SSR/environment path.
- Keep `trace_ssr` as the near-field and degradation path; keep VV+1 refraction
  default off.

#### VV-2 — water G-buffer conventions

- Validate packed `n.xz` reconstruction against the prepass normal, including
  above/below-water views. The missing y sign is a specific risk when the
  prepass flips the normal toward the viewer.
- Validate velocity sign and scale with the invariant `previous_uv = current_uv
  + velocity`.
- Capture depth, coverage, roughness, normal and velocity guides separately.

#### VV-3 — hit-lighting parity

Compare RT-hit shading to IV-K/raster shading term by term: BRDF, shadow,
sun/moon, local lights, IBL intensity, emissive, AO, atmosphere/volumetrics and
exposure domain. The confidence blend must not cross-fade between incompatible
radiance definitions.

#### VV-4 — open Halcyon evidence

- Measure SSR hit/miss/confidence on the default landscape.
- Measure reflection time at the named resolution and record render scale.
- Measure VRAM and prove zero reflection-pass frame cost when disabled.
- Capture post-tonemap before/after images under `dev records/phase VV/`.

### 6.2 RT water reflection shimmer and black-splotch audit

#### Source findings

1. Rough water uses one frame-varying GGX sample/pixel.
2. History retains only 12–28% of the previous result.
3. Disocclusion samples depth/coverage at `prev_uv` from the **current**
   `water_surface`, not a previous-frame water G-buffer.
4. No previous normal/roughness, hit distance, moments, variance or
   neighborhood clamp participates in history acceptance.
5. `sample_cascade_shadow` converts local cascade UV into atlas UV, then checks
   global `[0,1]` bounds. For cascades with an offset, an out-of-quadrant local
   UV can remain globally valid and sample a neighboring cascade.

This combination can alternate between a dark mis-shadowed hit and a bright
environment miss, producing the reported dancing black splotches.

#### Discriminators

Run the same locked camera in this order:

1. mirror rays only (disable roughness jitter);
2. unshadowed RT-hit debug;
3. cascade-index and local-UV-validity debug;
4. raw current radiance/hit alpha;
5. reprojection acceptance mask;
6. accumulated result;
7. final SSR/RT/environment composition.

If step 2 removes black splotches, fix local cascade bounds before tuning the
denoiser. If step 1 is stable but step 4 is noisy, the sampling/denoising path
is the owner. If raw current is stable but accumulated is not, history guides
are the owner.

#### Required architecture after the audit

- Validate local cascade UV before atlas mapping and clamp sampling to the
  selected quadrant.
- Retain a previous water guide set or reconstruct previous geometry from
  genuinely previous data.
- Validate history using previous depth, normal, roughness/coverage and, where
  available, reflected hit distance.
- Track moments/variance and clamp reprojected history to current neighborhood
  statistics; use confidence/sample count to choose the blend.
- Use depth/normal-aware half-res upsampling and preserve hit/miss confidence.

AMD's reflection denoiser is the minimum design reference: it consumes current
depth hierarchy, motion, normal, roughness, ping-pong radiance and variance, and
clips reprojected history with neighborhood statistics rather than relying on a
single depth comparison.

### 6.3 Post Process Details — audit every toggle end to end

The checkbox route currently exists: `CheckBoxMessage::Check` →
`EditorEvent::TogglePostFx` → selected `PostProcessComponent` →
`apply_post_process`. The audit must still prove every destination and consumer.

For each row record four values: **requested**, **effective**, **pass executed**,
**output changed**.

| Control | Effective dependency / consumer | Mandatory audit |
|---|---|---|
| Auto Exposure | Histogram/exposure buffer; overrides manual EV | Meter bright/dark cards, freeze adaptation, validate reset and compensation |
| Physical Camera | Derives EV from aperture/shutter/ISO; aperture also feeds DoF | Change each input by one stop; verify exact exposure relationship |
| Tonemapper | Post-process shader AgX/ACES/Reinhard | Numeric HDR ramp and saturated highlight fixture |
| Vignette | Post-process strength | Strength 0 and enabled-off must be bit-identical |
| Chromatic Aberration | Post-process offset | RGB edge pattern; capture after tone map |
| FXAA | Display LDR path | Edge target; verify pass placement before overlays |
| TAA | Forced off while FSR is on | UI must show “suppressed by FSR”; test only with FSR off |
| FSR | Adapter feature gates and context creation | Requested/effective status, reset on resize/toggle, render/display size and jitter |
| GTAO | GTAO target consumed by shading | Neutral clear on inactive/early return; AO-only debug |
| ReSTIR DI | Ray-query support; replaces shadow visibility where valid | Supported/unsupported and history reset matrix |
| ReSTIR GI | Ray-query support; feeds ambient | GI-only debug, light/material change invalidation |
| RT Reflections | Ray query, TLAS overflow, strength, kill switch | VV audit above |
| RT Refraction | Ray query; Great Lakes default off | Default/fallback and underwater direction |
| PCSS | Shared shading permutation/runtime flag | Shadow-edge fixture; do not infer speed change from a uniform |
| Contact Shadows | Screen-space depth path | Contact-only debug and horizon guard |
| CAS | Suppressed by FSR | §6.4 |
| Motion Blur | Velocity + shutter | Static image must be identical; known camera/object motion target |
| Bloom | Bloom chain + tone-map binding | §6.5 |
| Depth of Field | Focus distance + aperture | Focus ruler; physical/manual camera combinations |
| Volumetrics | Owns atmosphere/fog volume | §6.6 |
| Light Shafts | Requires volumetrics; shadow-tests in-scatter | §6.6 |
| World Cache | Ray query; volume RGB; conflicts with SDF alpha semantics | Isolated debug and mode-transition history reset |
| RT Specular | Ray query; shared `lighting_aux` | §6.7 |
| Path Tracer | Ray query; replaces raster result | §6.8 |
| Mesh SDF | Volume alpha; cache interaction | SDF-only AO debug; explicit mutual-exclusion policy |
| Probes | Ray query bake + SH buffer | §6.9 |
| Analytic Mips | Shading bit/permutation | Minification fixture and barycentric derivative validation |
| Cel Shading | Shared shading branch | Requested/effective state and F5 synchronization |

Also test singleton ownership. `apply_post_process` uses the **first** entity
with `PostProcessComponent`, while editor events mutate the **selected** one.
Assert exactly one such entity after startup, map load, New Scene, Create →
Terrain and save/load. If duplicates can exist, selection may edit a component
that never drives the renderer.

Replace flip-style events with explicit setters where programmatic sync or
duplicate delivery is possible. The CPU-frustum checkbox already demonstrates
the safer value-carrying event.

### 6.4 CAS appears to do nothing

This is presently an effective-state/UX failure, not evidence that the CAS
shader is dead:

```text
FSR default = on
app.rs: cas_pass.enabled = pp.cas_enabled && !pp.fsr_enabled
renderer: CAS also runs only when FSR did not resolve
```

AMD's FSR integration guidance says FSR has built-in RCAS and recommends
disabling separate CAS/menu options while FSR sharpening is active to avoid
double sharpening. The engine's suppression is reasonable; the unchecked UX is
not.

Audit/fix requirements:

- show CAS disabled with reason “FSR owns sharpening”; do not show an apparently
  active checkbox;
- expose FSR Sharp as the operative control while FSR is on;
- test CAS with `SOMNIUM_FSR=0`, TAA on, a static converged camera and a
  swapchain capture;
- verify sharpness/strength endpoints and prove overlays are not sharpened; and
- do not use `.somcap` as CAS evidence because it captures before CAS.

### 6.5 Bloom appears to do nothing

#### Source finding

Bloom creates a new `views[0]` on resize. `PostProcessPass` stores a clone of
the old `bloom_view`; renderer resize calls post-process resize before bloom
resize and never supplies the new bloom result. Subsequent tone-map bind-group
rebuilds keep using the stale view. The likely result is that bloom renders into
the new chain while tone mapping samples an abandoned zero/stale texture.

#### Audit

1. Add a pass-output debug that displays `BloomPass::result_view()` directly.
2. Compare fresh startup versus after maximize, manual resize, FSR scale change
   and immersive enter/exit.
3. Log/assert a bloom view generation ID in the tone-map bind group.
4. Use an emissive HDR card and sun glint; test intensity 0, 0.04, 0.5 and 1.0.
5. Confirm disabled clears/neutralizes the consumer and re-enable has no stale
   content.

Expected fix direction: make post-process bind bloom's current view after every
bloom resize, or make ownership/bind-group rebuilding explicit in one resize
function. Add a regression test around the resource generation change.

### 6.6 Volumetrics and light shafts

#### Source findings

- With two samples per slice, `fract(base + jitter)` can produce fractions in
  descending order. Then `dt = t - prev_t` becomes negative and
  `exp(-extinction * dt)` amplifies rather than attenuates.
- Cascade selection receives radial ray distance `t`; shadow cascade splits are
  view-space depth. Off-axis froxels therefore choose the wrong cascade.
- The shafts toggle has no independent output: it only changes shadowed
  in-scatter inside the enabled volumetric pass.
- `shaft_intensity` scales the sun/fog single-scatter terms broadly; audit
  whether the intended control is a shaft contrast/visibility amount rather
  than global volumetric brightness.

Bevy's reference offsets the ray origin once with jitter, advances samples
monotonically and selects the cascade using the sample's view-space z.

#### Audit

- Add assertions/debug reduction for `dt > 0` and finite transmittance on every
  sample.
- Visualize cascade selection in froxel space; compare radial `t` versus actual
  view z at center and screen corners.
- Use the missing acceptance scene: low sun, hard ridge/occluder, visible fog,
  static camera. Capture shafts off/on with volumetrics fixed on.
- Then test volumetrics off + shafts on and require the UI to display the
  dependency rather than implying a functioning independent pass.
- Validate camera translation/rotation history, resize and enable/disable reset.

### 6.7 RT Specular makes boat and terrain white

#### Source findings

- RT hit radiance is currently `albedo * NdotL * light.color + emissive`: no
  `/pi`, no material BRDF, no shadow, and no IBL/local-light parity. Phase IV-K
  already records that unnormalized diffuse under the physically scaled sun
  turned water white.
- The final shading pass does not add a material-filtered specular lobe. It
  replaces `ambient` with `lighting_aux` by confidence × `(1 - roughness)`, with
  no Fresnel, N·V or metalness weighting.
- History blends 85% of a five-tap same-UV previous image without motion
  reprojection or surface/hit validation.
- The aux output is multiplied by a shared intensity whose meaning changes when
  probes are active.

Bevy's scene specular reference samples a GGX VNDF direction, evaluates the
specular BRDF/PDF and **adds** the resulting specular radiance to the view
output. That is the appropriate structural comparison; Somnium's existing
shared raster BRDF remains the local convention to match.

#### Audit

- Capture raw aux RGB/alpha, raster ambient, raster specular and final composite
  on boat wood/paint/metal, terrain wet/dry layers and neutral spheres.
- Temporarily clamp/directly display hit radiance before history; if whitening
  exists there, fix energy/BRDF first.
- Disable history; if whiteness disappears only during motion, fix reprojection
  and invalidation.
- Validate no scene-specular contribution on water unless explicitly intended;
  Halcyon already owns water reflection.
- Energy gate: a diffuse hit uses albedo/π; specular uses the shared GGX/Fresnel
  convention; shadow/IBL/exposure domains agree with raster; no NaN/Inf and no
  unexplained near-60000 clamp.

### 6.8 Path tracer afterimages / not working

#### Source findings

`LightingExtraPass` has one `history_valid`, `frame`, `last_camera` and shared
`aux/aux_history` for specular and path modes. It resets path history only when
camera **position** changes more than 0.05 m. It does not reset for:

- camera rotation;
- projection/FOV/aspect/jitter changes;
- enabling path mode or switching specular ↔ path;
- TLAS topology/transform changes;
- material, light, environment, exposure or bounce-count changes; or
- a resize beyond the generic texture recreation.

The path shader then samples the previous result at the same UV with no
reprojection. A camera rotation therefore averages new rays with old objects at
the same pixels, exactly the reported afterimage. `history_valid` is also set
true at the end of any lighting-extra frame, even if only cache/probe work ran.

#### Required audit/fix design

- Give path tracing its own history validity, sample count and texture ownership.
- Reset on full camera view/projection change and every scene/light/material/env
  revision; reset on mode transition, resize, support loss and bounce changes.
- Clear accumulation explicitly on reset. Bevy's path tracer does this when its
  reset flag or camera `GlobalTransform` changes.
- Use an **unjittered** camera projection for a standalone reference path tracer,
  or define one temporal owner. Do not accumulate path samples, then feed them
  through unrelated TAA/FSR history without a tested compatibility policy.
- Define the mode matrix: path tracer should suppress raster-only lighting
  extras and normally suppress TAA/FSR, motion blur, GTAO and other temporal
  estimators; bloom/tone map may remain after it if documented.

Acceptance:

1. static camera converges monotonically;
2. a one-degree rotation clears immediately with zero ghost of the previous
   silhouette;
3. moving one object/light or changing one material resets affected reference
   accumulation (global reset is acceptable initially);
4. specular → path → specular cannot share radiance history;
5. path off restores raster on the next frame with no stale aux contribution.

### 6.9 Probes make everything whitish

#### Source finding

The probe bake projects the environment cubemap into L2 SH. Base
`evaluate_ibl` already adds environment diffuse/specular. When probes are on,
shading performs:

```text
ambient = evaluate_ibl(...)
ambient += SH_environment_irradiance * albedo * kd * AO
```

Thus the environment diffuse term is plausibly counted twice. In addition,
normalization must be verified: are stored coefficients radiance SH or already
cosine-convolved irradiance, and is the Lambertian albedo/π convention applied
exactly once?

#### Audit

- Uniform-white environment + 18% Lambertian sphere: compare analytic expected
  diffuse, base IBL only, probes only and both.
- Set base IBL to zero in a debug-only isolation, not as a shipping retune.
- Display each SH band/coefficient contribution and verify finite values.
- Decide semantics: probes **replace/blend the diffuse environment term** or
  store a local delta/occlusion correction. They should not add a second copy
  of the same distant environment.
- Split cache and probe intensity fields in GPU parameters; simultaneous flags
  must not change the meaning of one uniform.

The standard nine-coefficient SH irradiance model is a low-frequency
representation of diffuse distant illumination; it is not an additional light
source when the same environment diffuse has already been evaluated.

### 6.10 VV/post exit gate

- Every Details control has a requested/effective/executed/output record.
- CAS dependency is visible and CAS is proven with FSR off at the swapchain.
- Bloom survives every resize path and has a direct output capture.
- Volumetric integration has strictly positive steps and correct view-depth
  cascades; shafts are visible in the acceptance scene.
- Water reflections have valid previous guides/history rejection and no dancing
  dark splotches in static or moving shots.
- RT specular is BRDF/energy-correct and does not bleach the XV/boat fixture.
- Path tracing clears on all relevant revisions and shows no afterimages.
- Probes do not duplicate environment diffuse and do not whiten the scene.
- Original VV fallback, miss-rate, timing and VRAM gates are filled with real
  evidence.

**Frozen:** Great Lakes datum 16.1 m, optical max depth 18.6 m, Gerstner speed
0.85, water/transparents out of TLAS, `trace_ssr` retained, refraction default
off, no 24P software-RT expansion.

---

## 7. Evidence harness and run discipline

### 7.1 Build/static gate

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

Run focused tests first while iterating, then the workspace gate. Record the
exact commit, adapter and any environment overrides.

### 7.2 Controlled capture template

Use a fresh PowerShell process per A/B when possible. Otherwise remove every
environment override after the run; PowerShell environment variables persist
within the shell.

```powershell
$env:SOMNIUM_MAXIMIZE = '1'
$env:SOMNIUM_CAPTURE_FRAME = '180'
$env:SOMNIUM_CAPTURE = 'dev records/evidence/baseline.somcap'
$env:SOMNIUM_CAPTURE_PNG = 'dev records/evidence/baseline.png'
$env:SOMNIUM_CAPTURE_QUIT = '1'
cargo run --release --bin hello_engine

Remove-Item Env:\SOMNIUM_MAXIMIZE -ErrorAction SilentlyContinue
Remove-Item Env:\SOMNIUM_CAPTURE_FRAME -ErrorAction SilentlyContinue
Remove-Item Env:\SOMNIUM_CAPTURE -ErrorAction SilentlyContinue
Remove-Item Env:\SOMNIUM_CAPTURE_PNG -ErrorAction SilentlyContinue
Remove-Item Env:\SOMNIUM_CAPTURE_QUIT -ErrorAction SilentlyContinue
```

Do not put CAS/FXAA conclusions in an HDR `.somcap` report. Capture the
swapchain or instrument the actual pass output.

### 7.3 Temporal test motions

Use repeatable scripts/inputs for:

- static convergence for 256 frames;
- 1° camera yaw with zero translation;
- 0.10 m camera translation;
- object translation and material/light edits;
- resize, maximize/restore and immersive enter/exit; and
- feature transitions off→on, on→off and mode A→mode B.

For temporal artifacts preserve short frame sequences, not only final PNGs.

---

## 8. Deliverable format

Keep one audit result file for the whole requested sweep, appended to or derived
from this master plan, ordered **CR → XV → VV**. Each stage must contain:

1. headline finding;
2. findings table with severity, source/live confidence and status;
3. checked and correct;
4. ruled-out hypotheses and discriminator;
5. remaining live-only work with exact command;
6. validation run verbatim;
7. evidence links; and
8. AI disclosure separating source inference from measured behavior.

Stage gates remain independent even though the record is one file. If CR fails,
mark XV/VV evidence blocked rather than quietly interpreting images built from
untrusted instance slots. If XV fails, keep VV water work running where safe but
do not diagnose white terrain as a VV lighting defect until the XV baseline is
clean.

---

## 9. Reference basis

### Repository records and code

- [`context.md`](../context.md) — current architecture and phase state.
- [`implementation/context.md`](../implementation/context.md) — older core-crate
  architecture; useful history, not current pass ordering.
- [`audit_plan_CR_XV_VV.md`](audit_plan_CR_XV_VV.md) — original audit plan
  integrated here.
- [`phase_CR.md`](phase_CR.md), [`phase_XV.md`](phase_XV.md),
  [`phase_VV.md`](phase_VV.md), [`phase_IV.md`](phase_IV.md),
  [`phase_DF.md`](phase_DF.md), [`phase_26.md`](phase_26.md).
- Handoffs named in the request plus the terrain occupancy record. Where they
  disagree, current code and newer records win.
- Current source: `app.rs`, `renderer.rs`, `postprocess.rs`, `bloom.rs`,
  `lighting_extra.rs/.wgsl`, `volumetric.wgsl`, `water_reflection.rs/.wgsl`,
  `shading.wgsl`, CR culling/jobs/indirect code and XV terrain shaders/data.

### Local reference-engine source inspected

Under `C:\Users\adhir\Downloads\GE\example_repo`:

- Bevy Solari path tracer: camera-transform reset plus explicit accumulation
  clear.
- Bevy Solari scene specular: GGX VNDF sampling, BRDF/PDF evaluation and additive
  composition.
- Bevy volumetric fog: one jittered ray-origin offset, monotonic steps and
  view-space depth for cascade selection.
- AMD FidelityFX/Spartan and Unreal reflection-denoiser source were used as
  architecture comparisons only; no proprietary Unreal code is to be copied.

### Primary external references

- AMD, [FidelityFX Denoiser](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/denoiser/) — reflection inputs, variance-guided spatial filtering, temporal history clipping and disocclusion principles.
- AMD, [FidelityFX CAS](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/contrast-adaptive-sharpening/) — CAS modes, sharpness and color-space integration.
- AMD, [FSR Upscaling integration — RCAS](https://gpuopen.com/learn/ue-fsr/) — avoid simultaneous menu-enabled CAS and FSR sharpening.
- AMD, [FidelityFX FSR2 repository](https://github.com/GPUOpen-Effects/FidelityFX-FSR2) — temporal integration and post-process ordering guidance.
- AMD, [FidelityFX Denoiser repository](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser) — stochastic reflection denoiser reference.
- NVIDIA, [NRD](https://github.com/NVIDIA-RTX/NRD) — motion, normal/roughness, hit-distance and history-confidence expectations for reflection denoising.
- Bevy, [path tracer camera reset](https://github.com/bevyengine/bevy/blob/main/crates/bevy_solari/src/pathtracer/extract.rs) and [accumulation clear](https://github.com/bevyengine/bevy/blob/main/crates/bevy_solari/src/pathtracer/node.rs).
- Bevy, [scene specular GI](https://github.com/bevyengine/bevy/blob/main/crates/bevy_solari/src/realtime/specular_gi.wgsl) and [volumetric fog](https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/volumetric_fog/volumetric_fog.wgsl).
- Ramamoorthi and Hanrahan, [An Efficient Representation for Irradiance Environment Maps](https://graphics.stanford.edu/papers/envmap/) — nine-coefficient diffuse irradiance representation and its lighting semantics.

---

## 10. Non-negotiable accuracy rules

1. Do not call a source hypothesis “visually confirmed” without the controlled
   run.
2. Do not invent PNGs, timings, miss rates, memory figures or adapter behavior.
3. Do not retune water, terrain or foliage contracts to hide a correctness bug.
4. Do not diagnose from one beauty screenshot; capture the producer, guides,
   history acceptance and consumer.
5. Do not stack temporal systems without a declared owner and reset contract.
6. Do not equate a checked box with an executing pass.

**AI disclosure:** this plan was produced by reading the requested records,
current Somnium source, relevant local reference-engine implementations and the
primary references above. The P0/P1 source findings are code-level observations;
their exact on-screen contribution remains to be measured during the ordered
audit.
