# Phase VV — Halcyon

> *"…the halcyon calmed the waves, and for fourteen days the sea lay like glass."*
> A sea that lies like glass is a sea that shows you what is above it. This phase
> is about what the water reflects.

> **Codename:** Halcyon, after the mythological bird said to still the winter sea
> so it became a mirror
> **Status:** PLANNED — IMPLEMENTATION NOT STARTED
> **Start-here:** [`halcyon_context_handoff.md`](halcyon_context_handoff.md)
> **Plan date:** 2026-08-13
> **Project:** Somnium Engine
> **Target:** Rust 1.85, wgpu 29, winit 0.30
> **Depends on:** Phase IV-K (ocean fidelity, complete 2026-08-13) and Phase
> 24K/24L (ReSTIR DI/GI). Metaphor 26-A–I chrome is in the tree; do not rebuild
> it. A VV-A debug view may add a small inspector / View-menu binding.

The codename is thematic. No third-party source code is copied by this phase;
reference implementations named in section 12 inform an original Rust/WGSL
design and must be cited in `ATTRIBUTION.md` §1.7 as they are used.

## 1. Executive decision

Somnium's water reflects the world through a 28-step screen-space march
(`trace_ssr` in `water.wgsl`) and falls back to the prefiltered environment cube
wherever that march fails. This is the single largest remaining gap between the
engine's water and a photographic sea, because the cases screen-space tracing
cannot serve are exactly the cases a viewer notices: a hull reflected below the
waterline, a headland at the edge of the frame, anything behind the camera, and
anything the surface is looking *down* at rather than across.

Phase VV replaces that fallback hierarchy with hardware ray tracing. The engine
already builds a two-level acceleration structure every frame and already
queries it from two compute passes, so the acceleration structure is not the
work. The work is that reflections are a *specular* signal, and every piece of
existing ray-tracing infrastructure in the engine resolves *diffuse* ones.

The phase is staged so that a working, correct, unoptimised path lands first and
the architecture that makes it shippable lands second. Stage VV-B is the point
of no return; everything before it is reversible in an afternoon.

## 2. Goals

1. Reflect geometry that screen-space tracing structurally cannot: off-screen,
   behind-camera, and below-horizon.
2. Keep screen-space tracing as the near-field fast path where it is both
   cheaper and higher quality, and blend between the two on confidence rather
   than switching hard.
3. Degrade to exactly today's behaviour on hardware without
   `EXPERIMENTAL_RAY_QUERY`, with no visual regression and no new stalls.
4. Hold a per-frame budget that leaves the ocean playable at 1440p on the target
   GPU (section 11).
5. Produce a reflection signal stable enough that TAA is not asked to hide
   sampling noise it was never designed to remove.
6. Extend the same path to the transparent pass afterwards without a second
   architecture.

## 3. Non-goals

- Full path tracing, or replacing the visibility-buffer renderer.
- Ray-traced *refraction* through the water surface. It shares machinery with
  reflection and is a natural Phase VV+1, but it multiplies the ray budget and
  interacts with the volume-scattering model settled in IV-K.
- Caustics.
- Reflecting water in water. This requires water in the BLAS/TLAS, which is a
  meaningful cost for a surface that is re-tessellated and displaced every
  frame; see section 9.3.
- Replacing ReSTIR DI or GI, or unifying them with this path.
- A software ray-tracing fallback. Phase 24P is still unwritten and this phase
  must not quietly become it.
- Removing `trace_ssr`. It is the designed degrade path and stays.

## 4. Repository audit

Everything in this section was verified against the worktree on the plan date.
It is recorded because the most expensive failure mode for this phase is
planning against infrastructure that does not exist.

### 4.1 What exists and works

| Capability | Location | Notes |
|---|---|---|
| BLAS per mesh and per terrain chunk | `pass/raytrace.rs`, `renderer.rs` L837–846, L1640–1670 | Rebuilt only when dirty |
| TLAS rebuilt per frame | `renderer.rs` L1986–2010 | From `draw_queue` |
| Feature detection and gating | `context.rs` L52–60, L143, L224–227 | `wgpu::Features::EXPERIMENTAL_RAY_QUERY` |
| Inline ray query in WGSL | `restir_di.wgsl`, `restir_gi.wgsl`, `rt_debug.wgsl` | `enable wgpu_ray_query;` |
| Hit resolution against the global pool | `gi_trace()` in `restir_gi.wgsl` L157–221 | Instance → barycentrics → albedo |
| ReSTIR DI sun visibility | `pass/restir.rs` | Alpha channel signals "traced result exists" |
| ReSTIR GI indirect irradiance | `pass/restir_gi.rs` | Two entry points, temporal + spatial reuse |
| TAA that understands water | `taa.wgsl` L289–291 | Uses water velocity where coverage > 0.5 |
| Water writes velocity and coverage | `water.wgsl` L589–593, MRT targets 1 and 2 | Gated on `frame.history_valid` |

### 4.2 What does not exist

Do not plan around any of the following. None of it is present today.

- Any ray-traced reflection or refraction path, for water or anything else.
- Water geometry in the BLAS or TLAS. Water lives in `water_queue` and is never
  registered; the TLAS is built from `draw_queue` alone.
- Transparent geometry in the TLAS. `transparent_queue` is excluded.
- A shared "evaluate material at a ray hit and return outgoing radiance"
  function. `gi_trace()` returns albedo and normal only, and `gi_direct_at()`
  adds a Lambert sun term. Neither builds a `Surface` or calls `evaluate_brdf`.
- Any specular denoiser, or reservoir resampling for a specular signal.
- A reflection G-buffer or any separate reflection target.
- Fragment-stage ray query anywhere in the codebase. `RaytracePass::layout()`
  declares `FRAGMENT | COMPUTE` visibility for the acceleration structure, so it
  is permitted at the API level, but it is untested here.
- A software ray-tracing fallback.

### 4.3 Constraints that shape the design

**The TLAS holds at most 1024 instances** (`MAX_TLAS_INSTANCES`,
`raytrace.rs` L60) and silently drops the remainder. A reflection that traces
against a TLAS missing half the scene is worse than no reflection at all,
because the miss is not uniform — it is whichever draws happened to sort late.
This cap must be raised or made adaptive before VV-C, and the drop must become
observable rather than silent.

**The water pass is forward-shaded**, computing its reflection inline in
`fs_main`. There is no water G-buffer that a separate pass could consume, which
is the central architectural problem: a compute reflection pass needs surface
position, normal, and roughness, and today those exist only inside the fragment
that also needs the answer.

**The water pass already uses four bind groups** (`water.rs` L449–457) and the
global resource pool ReSTIR GI relies on for hit shading occupies `@group(0)`
with eleven-plus bindings. Reusing that pool from the water pass means either a
fifth bind group carrying the pool or an extracted subset.

**Water runs after opaque shading**, and copies the HDR target to `scene_color`
for refraction and SSR. The TLAS is therefore already built and valid by the
time water is recorded — reflection rays can be traced in the water pass without
reordering the frame.

## 5. Architecture

### 5.1 The reflection is deferred, not forward

The decisive choice is to stop computing water reflections inside the water
fragment shader.

The forward approach — an inline ray query in `fs_main` — is tempting because it
touches almost nothing. It also cannot be denoised, cannot be traced at reduced
resolution, cannot amortise across frames, and pays full cost on every fragment
including the ones foam has made completely rough. One ray per pixel of visible
ocean at 1440p is roughly two million rays per frame with no reuse.

The deferred approach splits the water pass in two:

```
  water depth prepass  →  water G-buffer  (position via depth, normal, roughness, coverage)
                                  ↓
  reflection compute pass  →  reflection texture  (ray query + temporal reuse, half-res)
                                  ↓
  water shading pass  →  HDR  (samples the reflection texture instead of tracing)
```

The prepass is cheap: the water surface is already being displaced in a vertex
shader, and target 1 of the existing MRT output is very nearly the G-buffer
already (`n.xz`, view depth, coverage). Roughness must be added, which means
moving the foam and roughness computation earlier — that is real work, but it is
mechanical and it is what makes everything downstream possible.

### 5.2 Rays are traced against confidence, not instead of SSR

Screen-space tracing is not a poor approximation of ray tracing; within its
domain it is *better*, because it reflects the fully shaded, post-lit, foam-and-
all HDR image rather than a re-shaded approximation of a triangle. The two are
combined per-pixel:

- march in screen space first;
- if it hits with high edge confidence, take it;
- otherwise trace a ray, shade the hit, and take that;
- if the ray misses, fall back to the environment cube.

This ordering also bounds the cost, because the rays that get traced are exactly
the ones screen space failed on, which is a minority of pixels in a typical
frame and a majority only when the camera is low to the water — which is also
when reflections matter most.

### 5.3 Hit shading

A reflection ray that hits a triangle needs outgoing radiance towards the water,
not albedo. This does not exist today and is the second-largest piece of work in
the phase.

The plan is to extract the hit-resolution half of `gi_trace()` into a shared
`rt_hit.wgsl` — instance lookup, barycentric interpolation, material fetch — and
then build a genuine `Surface` from it and call the existing `evaluate_brdf`
with the sun, plus `evaluate_ibl` for ambient. ReSTIR GI can then be refactored
onto the same hit resolution without changing its behaviour, which is how the
extraction pays for itself and how it gets tested.

Sun visibility at the hit point requires a second, shadow ray. Stage VV-D
evaluates whether that second ray is worth its cost or whether sampling the
cascaded shadow map at the hit position is sufficient at reflection fidelity.

### 5.4 Temporal reuse

A half-resolution reflection buffer with one ray per pixel is noisy on rough
water. The signal is reprojected using the water velocity the surface already
writes, and accumulated with a variance-driven blend factor. Reservoir
resampling in the manner of ReSTIR is explicitly *not* the first tool reached
for: specular reservoirs are far more delicate than diffuse ones because the
lobe is view-dependent, and a straightforward reprojected accumulation with a
disocclusion clamp is the correct starting point.

## 6. Stages

Each stage must build, pass `cargo test --workspace`, and leave the engine
shippable. No stage may be skipped on the grounds that a later one subsumes it.

### VV-A — Instrumentation and honesty (no visual change)

Make the current state measurable before changing it.

- Add a GPU timer scope around the water pass reflection work.
- Add a debug visualisation mode showing SSR hit/miss/confidence per pixel, so
  the actual failure rate of screen-space tracing in the shipped scenes is a
  number rather than an impression.
- Make the TLAS instance-cap overflow log once per frame instead of silently
  dropping draws.

**Exit:** the SSR miss rate for the default landscape and the ocean parity scene
is recorded in this document.

### VV-B — Water G-buffer and pass split

The architectural commitment.

- Split `WaterPass` into a prepass writing depth plus a packed G-buffer
  (`Rgba16Float`: octahedral normal, roughness, coverage) and a shading pass.
- Move foam and roughness derivation into the prepass; the shading pass reads
  them back rather than recomputing.
- The shading pass initially samples a reflection texture that the old
  `trace_ssr` fills, so this stage is a pure refactor with byte-identical output.

**Exit:** a frame capture before and after this stage differs by no more than
floating-point reassociation.

### VV-C — Ray-traced reflection compute pass

- New `pass/water_reflection.rs` and `shaders/water_reflection.wgsl`, compute,
  `enable wgpu_ray_query;`.
- Binds the TLAS, the water G-buffer, and the global resource pool.
- One ray per half-res pixel along the mirror direction; on hit, resolve
  geometry and return albedo only (no lighting yet) so that correctness of the
  ray path is visually unambiguous.
- Raise or make adaptive the 1024 TLAS instance cap.

**Exit:** reflections of off-screen geometry appear, flat-shaded, and the pass
is fully skipped when `EXPERIMENTAL_RAY_QUERY` is absent.

### VV-D — Hit shading

- Extract `rt_hit.wgsl` from `gi_trace()`; refactor ReSTIR GI onto it and prove
  GI output is unchanged.
- Build a `Surface` at the hit and evaluate sun plus IBL.
- Decide shadow ray versus cascade sample by measurement, and record the
  decision here.

**Exit:** ray-traced reflections are lit consistently with the raster path; a
reflected object and the object itself agree in colour.

### VV-E — Roughness-aware tracing

- Importance-sample the GGX lobe using the water's own `reflection_roughness`
  rather than tracing the mirror direction.
- Skip the ray entirely above a roughness threshold where the environment cube
  is indistinguishable, and use the ray budget saved on the pixels that need it.

**Exit:** foam-covered water costs no more than it does today.

### VV-F — Temporal accumulation and upsample

- Reproject with water velocity; accumulate with disocclusion and variance
  clamping.
- Bilateral upsample to full resolution using the G-buffer depth and normal.

**Exit:** no visible boiling on rough water while the camera is in motion.

### VV-G — Blend with screen-space tracing

- Combine per-pixel on SSR confidence as described in 5.2.
- Verify that the seam between the two is invisible in motion, which is the
  case that will expose any disagreement in exposure between the two paths.

### VV-H — Evidence, budgets, documentation

- Capture before/after evidence into `dev records/phase VV/`.
- Record measured costs against section 11.
- Update `context.md`, `ATTRIBUTION.md`, and this file's status.

## 7. Fallback matrix

| Hardware / setting | Reflection source |
|---|---|
| RT available, VV complete | SSR where confident, else traced ray, else env cube |
| RT available, `SOMNIUM_RT_REFLECT=0` | Exactly today's behaviour |
| No `EXPERIMENTAL_RAY_QUERY` | Exactly today's behaviour |
| RT available, TLAS overflowed | Traced result rejected for that frame, SSR and cube only, logged |

The third row is the one that matters for the phase's credibility: the engine
must look no worse than it does today on hardware that cannot ray trace, and
that must be verified on an actual non-RT adapter rather than assumed.

## 8. Risks

**Fragment versus compute ray query.** All existing ray tracing in the engine is
compute. The deferred architecture keeps it that way, which is a reason to
prefer it beyond performance.

**Exposure disagreement between paths.** SSR returns fully shaded HDR pixels;
ray hits return re-shaded approximations. If the two disagree, the blend in VV-G
will show a moving seam. This is the most likely source of a late, hard-to-
diagnose visual bug, and VV-D's acceptance test exists specifically to catch it
early.

**Water not being in the TLAS** means a reflected shoreline will not show the
water lapping against it. This is acceptable and expected; it is called out here
so it is not later mistaken for a bug.

**The 1024 instance cap** silently dropping geometry is a pre-existing
correctness problem that reflections will make visible rather than cause.

**ReSTIR GI regression** during the VV-D extraction. Mitigated by refactoring GI
onto the shared hit resolution first and proving output equivalence before
building anything new on it.

## 9. Open questions

1. Half resolution, quarter resolution, or adaptive by roughness?
2. Shadow ray at the hit, or cascade sample? Measure in VV-D.
3. Should the transparent pass share the reflection texture, and if so does it
   need its own G-buffer or can it reuse the water one?
4. Is water-in-TLAS worth revisiting once the surface is a camera-snapped
   clipmap with a stable topology?

## 10. Acceptance criteria

- A reflection of geometry that is entirely off-screen is visible and correct.
- Disabling ray tracing produces a frame identical to today's.
- No boiling, crawling, or ghosting on rough water under camera motion.
- ReSTIR GI output is unchanged by the VV-D refactor.
- Budgets in section 11 are met and recorded.

## 11. Budgets

To be filled with measurements during VV-A and VV-H. Targets:

| Item | Target |
|---|---|
| Reflection pass, 1440p, ocean parity scene | ≤ 2.0 ms |
| Additional VRAM | ≤ 32 MB |
| Frame time regression with RT disabled | 0 ms |

## 12. References

To be cited in `ATTRIBUTION.md` as each is actually used.

- Wright et al., **ReSTIR GI: Path Resampling for Real-Time Path Tracing**, NVIDIA 2021
- Kajiya-style specular importance sampling as described in Karis, **Real Shading in Unreal Engine 4**, SIGGRAPH 2013
- Stachowiak, **Stochastic Screen-Space Reflections**, SIGGRAPH 2015
- NVIDIA, **Ray Tracing Gems**, chapters on reflection denoising
- wgpu 29 ray query documentation: <https://docs.rs/wgpu/29.0.0/wgpu/struct.Features.html>
- Existing in-repo prior art: `pass/restir_gi.rs`, `shaders/restir_gi.wgsl`

## 13. Handoff rule

The next session starts at [`halcyon_context_handoff.md`](halcyon_context_handoff.md),
then this file, then verifies section 4 against the worktree before writing
code — that audit is dated 2026-08-13 and line numbers drift. Also re-read
`context.md`, `ATTRIBUTION.md` §1.7, and `dev records/phase_IV.md` section 14.
Phase IV-K closed on 2026-08-13, so the water pass is stable again, but the
shading it settled on is what VV-D has to reproduce through a traced ray.

Metaphor chrome (26-A–I plus the 2026-08-13 evening polish) is the shipping
editor. Do not restart it. Frozen: `WaterComponent::great_lakes`, XV terrain
contract, `context.md` §20.

Begin at VV-A. Do not begin at VV-C because it is the interesting one.

This file is a plan only. No Phase VV engine code was added as part of its
creation.
