# MORROWIND-AB — the GI tier below ray query

**Implementation complete, 2026-08-28.** Track 7 (RED MOUNTAIN).

## Decision

Somnium chose **DDGI** over an offline lightmapper. The engine already owned a
camera-relative software SDF, L2 SH gather, temporal lighting infrastructure
and a generated scene-settings route. It did not own secondary lightmap UVs,
an atlas/cook format or a lightmap material binding. DDGI therefore adds the
portable tier without creating a second asset pipeline.

The old `probes` switch was not that tier: it projected the environment inside
`lighting_extra.wgsl`, whose root requires `wgpu_ray_query`, and its record path
returned without a TLAS. AB replaces the runtime with a separate `ddgi.wgsl`
root that contains neither ray-query enablement nor an acceleration structure.

## Shipped path

- A camera-relative 4×4×4 probe lattice traces the existing 64³ software SDF.
- Each probe integrates 64 Fibonacci-distributed rays into nine L2 SH
  coefficients. Eight probes update per frame by default; the deterministic
  ring scheduler visits all 64 without starvation.
- SDF hits receive directional and low-frequency environment irradiance,
  modulated by the runtime material base colour. Misses contribute zero: the
  normal IBL path already owns sky lighting, while DDGI adds geometry bounce.
  Temporal hysteresis damps update noise.
- Camera cell movement, topology/content revision, light changes and setting
  changes invalidate history. Validity is tracked per probe so the first sweep
  never blends unwritten coefficients. Probe spacing, update budget,
  hysteresis and intensity are clamped at the renderer boundary.
- The pass records after the SDF upload and before visibility-buffer shading,
  outside every ray-query/TLAS guard. Shading trilinearly gathers the SH grid.
- ReSTIR GI wins when both tiers are requested; the path tracer disables both.
  The pre-AB `probes` fields remain loadable and map to DDGI but are hidden from
  Details so there is one authored route.

Generated Details exposes `Portable DDGI`, `DDGI Intensity`, `DDGI Probe
Spacing`, `DDGI Update Budget` and `DDGI Hysteresis` under **Post Processing →
Global Illumination**. Schema reflection supplies editing, validation, undo,
scripting and scene persistence without a hand-written UI route.

DDGI is default off. That preserves existing scenes and keeps ReSTIR as the
explicit higher-quality option where ray query is available.

## Measurement

Matched 1920×1080 windowed runs used 20 warm-up frames and 40 measured frames.
Raw `.somtime` files sit beside this record. DDGI adds less than 0.35 ms in both
terrain views; the total-frame variation is larger than the feature cost, so
the scoped GPU row—not a faster-frame claim—is the decision evidence.

| View | Tier | GPU DDGI (ms) | GPU frame (ms) | CPU lighting-extra (ms) | Wall frame (ms) |
|---|---|---:|---:|---:|---:|
| Coastal Ground | off | 0.0042 ± 0.0055 | 23.7750 ± 3.5334 | 0.0441 ± 0.0011 | 23.9681 ± 3.7413 |
| Coastal Ground | DDGI | 0.1897 ± 0.1698 | 23.4785 ± 3.2042 | 0.2296 ± 0.1409 | 23.5613 ± 3.4469 |
| Island | off | 0.0031 ± 0.0036 | 20.0438 ± 4.8013 | 0.0290 ± 0.0008 | 19.7754 ± 4.0753 |
| Island | DDGI | 0.3499 ± 0.2337 | 18.4006 ± 4.2480 | 0.2023 ± 0.1184 | 18.8122 ± 3.6293 |

An early run rebuilt the 64³ SDF after every transform/light revision and cost
38–40 ms of CPU time. The measured path splits SDF topology/content invalidation
from probe-light invalidation, which is why the final CPU row remains bounded.

## Verification

- `somnium_core` library: **269/269** tests passed, including generated-Details
  metadata, the generic editable-field gate and an explicit DDGI scene round
  trip.
- DDGI policy/layout suite: **5/5** tests passed (clamps, fair scheduling,
  snapped-origin invalidation, per-probe validity and the
  64-probe/9-coefficient GPU contract).
- The renderer and shader registry compile checks pass; the composed shader
  validator accepts the ray-query-free DDGI root.
- The public `vvardenfell` slice compiled and ran through the reflected
  `PostProcessComponent`; no renderer-internal feature toggle is required.

## Known bounds

This first portable tier uses the software SDF's geometry distance rather than
hardware rays. It therefore inherits that field's mesh budget and coarse voxel
accuracy. It uses base colour rather than sampling the full textured material.
Transform and material changes rebuild after the revision is stable for one
frame; continuously animated geometry is intentionally omitted from repeated
64³ CPU rebuilds until it settles. It is a portable dynamic-diffuse fallback,
not an RTXGI-equivalent visibility-moment implementation.

## Reference boundary

Flax was architecture-only and proprietary; no Flax implementation detail was
copied. The clean-room boundary is recorded in `ATTRIBUTION.md` §13H.25.
