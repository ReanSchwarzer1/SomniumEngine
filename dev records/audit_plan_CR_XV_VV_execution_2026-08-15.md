# CR → XV → VV audit execution report — 2026-08-15

This report records the implementation and verification run for
[`audit_plan_CR_XV_VV_expanded_2026-08-15.md`](audit_plan_CR_XV_VV_expanded_2026-08-15.md).
The work started from branch `dev` at `b8581aa`. The source tree was clean at
baseline. No frozen terrain, water, camera, layer-count, or material-layout
contract was retuned.

## Result

The reported failures were reproduced or source-isolated. Most were corrected;
path tracing remains an open visual failure:

- Post Process checkboxes now deliver one explicit value instead of two
  flip-style events. Inspector synchronization no longer mutates settings.
- CAS becomes the effective standalone sharpener by disabling FSR; FSR still
  owns RCAS when selected. A final-display capture proves CAS changes the image.
- Bloom's current result view is rebound after resize. A maximized-window
  display capture proves bloom reaches tone mapping.
- Volumetric samples advance monotonically, cascades use view-space depth, and
  shaft amount controls shadow contrast rather than global fog brightness.
- Water RT history now owns true previous guides, moments, variance, sample
  count, and hit distance. Cascade UVs are validated before atlas mapping;
  below-water normal reconstruction and guide-aware upsampling are explicit.
- Scene RT specular uses the shared BRDF/shadow/IBL conventions and replaces
  only the environment specular lobe. It no longer replaces all ambient light.
- Path tracing has a separate accumulation texture and resets for camera view,
  projection, scene, material, light, settings, resize, support, and mode
  changes. It suppresses unrelated temporal/raster estimators while active.
- Probes replace the environment diffuse lobe instead of adding a duplicate.
  Cache and probe intensities are independent.
- World Cache and Mesh SDF cannot claim incompatible semantics in the shared
  3-D volume at the same time.

**Current exception:** the path-tracing control flow and accumulation counter
were repaired, but the user's final engine retest still shows broken
boat/water rendering and artifacting. Path tracing is not accepted as working.
The current status table at the end of this report supersedes any earlier
capture-based visual conclusion.

No near-white clipping was measured in the RT specular, probes, path tracer,
light-shaft, or final water captures.

## Verification notation

| Mark | Evidence |
|---|---|
| **L** | Current release-engine live capture on the installed ray-query/BC7 adapter |
| **T** | Current automated test or WGSL validation |
| **S** | Current source/control-flow audit; used where a second hardware class or dedicated fixture was unavailable |

Source-only rows are deliberately not presented as reproduced visual results.

## Stage 1 — CR (Crysis)

### Changes

- Centralized the instance partition as visible → shadow-only → transparent.
  Shadow and transparent consumers use the same calculated bases.
- Added a final-consumer ID test that checks vertex offset, index offset,
  material, and transform for visible, shadow-only, and transparent draws on
  the GPU-driven and CPU-fallback issue paths.
- Added conservative plane-boundary coverage for all six plane directions at
  large world coordinates, including touching, inside, and outside cases.
- Added exact 511/512/513 serial-versus-parallel parity coverage.
- Audited the surface-acquire early return and normal frame end: both call the
  common queue clear, which retains vector capacity.

### Live occupancy

At 2560×1392 with the same scene and capture frame:

| State | Instances | terrain chunks | CPU-culled | shadow casters | Frame |
|---|---:|---:|---:|---:|---:|
| CR on | 197 | 175 | 81 | 260 | 19.137 ms |
| CR off | 278 | 256 | 0 | 260 | 19.246 ms |

The camera cull removed 81 terrain chunks while the shadow-caster count stayed
260. The isolated on/off display delta was MAE 0.0914 with no clipped whites;
the small non-zero delta is moving-water/shadow timing, not missing geometry.
Evidence: `cr_culling_on/off.*` and `cr_isolated_on/off.*`.

CR exit: **pass (L/T/S)**. The common 256-chunk terrain does not cross the
parallel threshold; that branch is correctly recorded as test-reachable at
511/512/513 rather than live-shipping evidence.

## Stage 2 — XV (Appalachia)

### Changes and audits

- Added an explicit non-negative bindless-layer guard before array indexing.
  Hero-only layers 16–31 cannot reach a negative texture ID.
- Replaced the projected-X tangent construction with stable axis selection in
  live and clipmap terrain paths.
- Preserved wetness/F0 semantics in the clipmap instead of forcing wet F0 to
  zero.
- Re-ran the existing semantic-mip tests: linear-light albedo, reconstructed
  and normalized normals, Toksvig roughness, and linear scalar channels pass.
- Re-ran strongest-four error, 2032-byte layout, packed-map, residency, and
  shading-specialization tests. The topology remains bounded at 24 base
  material-map taps and 36 on the full-PBR biplanar cliff path.

### BC7/RGBA8 A/B and occupancy

| State | Resident pack | Hero / extra | Frame | Shading |
|---|---|---:|---:|---:|
| BC7 | compressed asset pack | 2048 / 1024 | 19.165 ms | 11.667 ms |
| forced RGBA8 | generated fallback | 1024 / 1024 | 19.295 ms | 11.834 ms |

The display comparison measured MAE 0.2357; 2.972% of pixels changed by more
than one 8-bit level and neither image clipped white. The BC7 configuration is
the approximately 213 MiB shipping pack. The projected 2K RGBA8 allocation
exceeded the 700 MiB guard, so the fallback correctly selected 1K hero maps;
the audit log proves compressed and fallback packs were not simultaneously
resident. Evidence: `terrain_bc7.*` and `terrain_rgba8.*`.

The white-material discriminators localized the symptom to lighting extras,
not terrain data: RT specular changed 21.318% of pixels by more than one level
but kept mean luma 112.861 → 112.826; probes kept it 112.861 → 112.855. Neither
produced clipped-white pixels.

XV exit: **pass (L/T/S)**.

## Stage 3 — VV (Halcyon), post processing, and lighting extras

### Water RT reflections/refraction

The half-resolution history is now five layers:

1. reflection radiance/confidence;
2. refraction radiance/confidence;
3. previous water normal/depth/roughness guide;
4. reflection luma moments/sample count/hit distance; and
5. refraction luma moments/sample count/hit distance.

The old view exposed only two layers even though the shader needed a guide;
this made the guide access out of bounds. The current pass exposes all five.
History acceptance now checks previous depth, normal, roughness, hit/miss and
hit distance, then variance-clips the previous luminance and selects weight
from sample count. Re-enable and reflection/refraction mode edges invalidate
history. Final upsampling rejects depth/normal-discontinuous neighbours.

On the final water region mask (378,718 pixels), RT on/off at frame 97 measured
MAE 1.1433. Natural water animation measured MAE 0.3208 with RT off and 1.7904
with stochastic RT on between frames 96/97. Only 0.2289% of water pixels fell
more than 20 luma levels and none fell more than 40; there were no clipped-white
pixels or black dancing-splotch outliers in the inspected pair. The named
2560×1392 reflection pass cost was 0.313 ms; disabled cost was the profiler's
0.002 ms timestamp floor. Evidence: `water_final_*`, `water_rt_*`, and
`water_v2_*`.

The installed adapter supports ray query, so unsupported-device, TLAS-overflow,
and dummy-target behavior were source/validation audited: alpha zero selects
the retained SSR/environment path, and no disabled history is accepted.

### Post Process Details control ledger

All checkboxes use `CheckBoxMessage::Check(value)` →
`EditorEvent::SetPostFx(control, value)` → the selected singleton component.
Engine-to-widget synchronization is ignored as user intent. The renderer
normalizes zero/duplicate Post Process entities to one, preferring a selected
legacy duplicate before removing extras. New Scene creates the singleton.

| Control | Requested/effective/executed/output record | Proof |
|---|---|---|
| Auto Exposure | Explicit value; histogram owns exposure when on, manual EV when off; buffer consumed by post | T/S |
| Physical Camera | Explicit value; aperture/shutter/ISO derive EV and aperture feeds DoF | T/S |
| Tonemapper | Explicit enum/cycle; AgX/ACES/Reinhard index consumed in post shader | T/S |
| Vignette | Explicit value; strength zero and disabled both neutral in shader | T/S |
| Chromatic Aberration | Explicit value; display post offset branch | T/S |
| FXAA | Explicit value; LDR display pass before overlays | T/S |
| TAA | Explicit value; effective only with FSR/path off; owns fallback jitter/history | T/L |
| FSR | Explicit value; disables authored TAA/CAS; adapter smoke and final effective log | T/L |
| GTAO | Explicit value; neutral target when inactive; suppressed by path mode | T/S |
| ReSTIR DI | Explicit value; hardware-gated visibility, reset path retained; suppressed by path | T/S |
| ReSTIR GI | Explicit value; hardware-gated ambient target and invalidation; suppressed by path | T/S |
| RT Reflections | Explicit value; ray-query and kill-switch gated; final water output changes | L/T |
| RT Refraction | Explicit value; default off; shares the corrected history/fallback contract | T/S |
| PCSS | Explicit value; shared shading flag selects PCSS or single compare | T/S |
| Contact Shadows | Explicit value; shared shading flag selects guarded screen march | T/S |
| CAS | Enabling disables FSR; effective log says CAS=true/TAA=true/FSR=false; swapchain output changes | L/T |
| Motion Blur | Explicit value; velocity/shutter pass; suppressed by path; static path neutral | T/S |
| Bloom | Explicit value; current resized bloom view reaches tone map; maximized output changes | L/T |
| Depth of Field | Explicit value; focus/aperture consumer; suppressed by path | T/S |
| Volumetrics | Explicit owner value; off clears dependent shaft request; suppressed by path | L/T |
| Light Shafts | Enabling also enables volumetrics; shadow-contrast in-scatter consumer | L/T |
| World Cache | Explicit value; ray-query RGB volume; mutually exclusive with Mesh SDF | T/S |
| RT Specular | Explicit value; ray-query/specular-only replacement and isolated history | L/T |
| Path Tracer | Explicit value; ray-query replacement, isolated accumulation, mode suppression | L/T |
| Mesh SDF | Explicit value; distance-alpha volume; mutually exclusive with World Cache | T/S |
| Probes | Explicit value; SH diffuse replacement and separate intensity | L/T |
| Analytic Mips | Explicit value; shading permutation/derivative branch | T/S |
| Cel Shading | Explicit value; shared shading branch and F5/component synchronization | T/S |

Every row therefore has a requested setting, effective dependency, execution
consumer, and output/test record. The newly reported symptom rows have live
display evidence; the other rows are labeled T/S rather than being falsely
described as new visual captures.

### CAS and bloom

| A/B | Display MAE | Pixels changed >1 | Mean luma | Clipped white |
|---|---:|---:|---:|---:|
| CAS off → on | 1.3529 | 49.383% | 112.861 → 112.742 | 0% |
| Bloom off → on | 10.0501 | 100.000% | 112.861 → 122.911 | 0% |

CAS was captured at the swapchain, after tone mapping, with FSR off. Bloom was
captured after the maximize/resize path using the newly rebound result view.
Evidence: `post_neutral.*`, `post_cas_on.*`, `post_bloom_on.*`, and `fsr_on.*`.

### Volumetrics and shafts

- The two jittered strata are strictly ordered; no negative `dt` is possible.
- Cascade selection uses `-(view * world_position).z`, not radial ray length.
- Fog/shaft setting changes invalidate reprojection history.
- Shaft amount mixes shadow visibility and never multiplies globally lit fog.

In the aligned low-sun/ridge acceptance pair, shafts changed 15.310% of pixels
by more than one level (MAE 0.8826). 3.540% darkened by more than two levels,
only 0.024% brightened by that amount, and the peak localized luma reduction
was 32.3. This is the expected one-way removal of direct in-scatter behind an
occluder, not an exposure-normalized global fog boost. Evidence:
`shafts_aligned_off/on.*`.

### RT specular and probes

RT specular hit lighting now constructs the shared surface BRDF, traces sun
visibility, includes diffuse/specular environment terms in the same exposure
domain, clamps non-finite/negative energy, and replaces only the baseline
environment specular lobe. Its history is separate from path tracing and resets
on camera, projection, scene/material/light, settings, resize, and mode changes.

Probe SH irradiance now replaces environment diffuse while preserving cubemap
specular. Probe and cache intensity no longer overload one uniform.

| Mode | Display MAE vs raster | Pixels changed >1 | Mean luma before → after | Clipped white |
|---|---:|---:|---:|---:|
| RT specular | 1.1171 | 21.318% | 112.861 → 112.826 | 0% |
| probes | 7.9929 | 99.863% | 112.861 → 112.855 | 0% |

Both live images were visually inspected; neither boat nor terrain became
white. Evidence: `rt_specular_on.*`, `probes_on.*`, `post_neutral.*`.

### Path tracer — automated capture assessment, later superseded

The measurements below describe the original automated capture set only. The
later user acceptance retest found persistent visual artifacting, so these
numbers must not be interpreted as proof that path tracing works.

Path mode's effective log proves FSR, TAA, CAS, GTAO, volumetrics, shafts,
ReSTIR DI/GI, motion blur, and DoF were all false while `lighting_extra_flags`
was exactly `0x4`.

For a 20° audit rotation, the immediate reset image was MAE 5.8286 from an
independently started new-view image but MAE 22.6831 from the old view. The new
silhouette therefore owns the first post-rotation sample; old objects are not
retained as afterimages.

Fixed-camera convergence against the frame-96 image was monotonic:

| Accumulation frame | MAE to frame 96 |
|---:|---:|
| 8 | 1.2460 |
| 24 | 1.2155 |
| 48 | 1.1774 |

Frame 96 contained no clipped-white pixels. Evidence: `path_converge_*`,
`path_old_yaw.*`, `path_new_yaw.*`, and `path_yaw_jump_*`.

## Build and regression gate

Final commands:

```text
cargo fmt --all -- --check
git diff --check
cargo check --workspace
cargo test --workspace -j 1
cargo test -p somnium_renderer --test shaders_validate
cargo build --release --bin hello_engine
```

All pass. The workspace suite covers 428 current unit, integration, shader, and
doc tests (including 12 full WGSL module validations). Parallel workspace test
linking hit Windows `LNK1104` file locks on two different executables; the same
suite passed single-threaded, confirming an external link/file-lock race rather
than a failing test.

## Evidence index and reproducibility

All current artifacts are under
[`evidence/audit_2026-08-15/`](evidence/audit_2026-08-15/). The directory contains
display PNGs, HDR `.somcap` captures where relevant, effective-state/profiler
logs, and two PowerShell capture matrices. Display capture reads the final
swapchain before editor overlays; `.somcap` remains the pre-display HDR source
and was not used as CAS proof.

The evidence directory is intentionally untracked and large (91 files,
approximately 772 MiB) because the audit retained lossless 2560×1392 pairs. It should be
archived externally or selectively committed rather than added wholesale to
normal source history.

## Final disposition

CR, XV, VV, and the all-toggle control plane meet the automated gates stated
above. This disposition is superseded for path tracing by the focused user
acceptance retest below: path tracing remains visibly broken and is an open
rendering defect. The passing automated tests do not close that visual issue.

## 2026-08-15 focused regression follow-up

This follow-up covers the three defects reported after the first acceptance
pass: path-traced boat/water artifacting, dancing dark RT-reflection speckles,
and a broad black nighttime FSR result near the top of the image.

### Path tracing: still broken / open

The renderer was correctly invalidating accumulation when scene transforms
changed, but the editing demo continuously advances boat physics and water
state. That made the scene revision change every frame, so the offline path
tracer could never retain a second sample. Entering path mode now pauses the
simulation transport while preserving its prior state; leaving path mode
restores that exact state. The path also uses the sharp environment only for
primary misses and a filtered environment for indirect misses.

These changes repaired the accumulation counter, but they did **not** repair
the final rendered result. The user's post-fix engine retest still shows broken
path-traced boat/water output and objectionable artifacting. Path tracing must
therefore remain open; the earlier automated capture assessment was a false
acceptance.

Diagnostic data at a fixed 2560×1392 camera:

| Capture | Reported accumulated frames | What it proves |
|---|---:|---|
| `after_path_f2` | 1 | Initial accumulation state only |
| `after_path_f64` | 63 | The counter advances; visual correctness is **not** established |

The new transport-state unit test proves pause/restore behavior, including an
already-paused starting state. It does not validate image quality, material
transport, denoising, water integration, or temporal stability. Required
follow-up is a new visual diagnosis using the user's failing camera/material
case; do not treat the frame-63 capture or passing unit tests as closure.

### RT water reflections: fixed

The temporal validator compared hit distance across independent rough-GGX ray
samples. On rough water those samples are intentionally different, so valid
history was rejected. The temporal clip also pulled stable history toward each
new one-ray dark outlier, producing the reported dancing black speckles.

Distance validation is now retained for near-mirror reflections and bypassed
for rough stochastic reflections. Current radiance is winsorized against the
previous luminance moments before blending; stable history is no longer clipped
around a noisy current sample. The `after_water_rt_off/on` display pair shows
the RT result without the lake-wide dark speckle field.

### FSR at night: safely contained

Two corrections were made to the FSR integration itself:

- FSR now receives linear, pre-exposed HDR input with the HDR context flag and
  matching `pre_exposure` value.
- The embedded legacy RCAS stage is disabled. A bounded non-negative sharpen
  in the un-pre-exposure pass keeps the existing sharpness control functional.

The current experimental `wgpu-ffx` backend nevertheless continued to turn
low-luminance geometry black at both native 1:1 and 1600×870 → 2560×1392 after
those contract corrections. The retained `after_fsr_contract_*` captures
isolate that backend defect. Nighttime FSR requests therefore use TAA as a
contained fallback until the backend is replaced or upgraded; daylight FSR
continues to execute normally.

Runtime state proof:

| Scenario | Sun Y | Scene → swapchain | Effective state | Result |
|---|---:|---|---|---|
| Night, FSR requested | -0.309017 | 1600×870 → 2560×1392 | `fsr=false taa=true` | Full scene, no black band |
| Day, FSR requested | 0.573576 | 1600×870 → 2560×1392 | `fsr=true taa=false` | FSR active, full scene |

This is an explicit fallback rather than a false claim that the defective night
backend path is repaired. The authored FSR toggle remains requested and the
audit log reports the effective fallback truthfully.

### Follow-up validation

The final focused gate passed:

```text
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace -j1 --target-dir target\validation-remaining
cargo build --release -p hello_engine
git diff --check
```

The clean workspace run passed all 429 unit, integration, shader, and doc tests,
including 51 core tests, 283 renderer library tests, and 12 WGSL validation
tests. The ordinary debug target initially encountered transient Windows
`LNK1104` executable locks; the clean target completed without a test failure.

Focused captures, effective-state logs, and reproducibility scripts are under
[`evidence/audit_2026-08-15_remaining/`](evidence/audit_2026-08-15_remaining/).

### Current user-acceptance status

| Area | Status | Note |
|---|---|---|
| Path tracing | **Broken / open** | Persistent boat/water artifacting after the attempted accumulation fix |
| RT water reflections | Working | The reported dancing dark reflection speckles are no longer observed |
| Nighttime FSR | Working through fallback | Uses TAA below the horizon; daylight continues to use FSR |
| Other audited post-process controls | Working in the user's check | No additional regression reported in this retest |

The current release cannot be described as fully passing the expanded visual
audit while the path-tracing row remains open.
