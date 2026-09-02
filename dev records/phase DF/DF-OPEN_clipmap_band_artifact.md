# OPEN — dark band / ribbon artifact with Clipmap on

> **Status (2026-09-02): three mechanisms identified, reproduced, and fixed;
> the clipmap still ships off by default, for the reason in
> `DF-QUALITY_clipmap_verdict.md` rather than for any of them.**
> See `dev records/phase DREAMS/DF-BAND_resolution.md` (the red, miss-path
> mechanism), `DF-STALE_resolution.md` (the green, stale-texel mechanism), and
> `DF-SLOT_resolution.md` (green again: the generate pass's uniforms, uploaded
> twice to one slot, so the detail stack was painted with the macro stack's
> rectangle).
>
> The third one was reported against a build carrying both earlier fixes, and
> it is the reason this file stayed open. It needs both stacks to hand generate
> a job in the same frame — 8 of 126 frames in the repro — and each collision
> is permanent, because what the shader writes on its early-out has alpha 1.0
> and therefore reads back as data.
>
> A **red** Clipmap Source band is the flat macro-map fallback in
> `evaluate_clipmap_material`, which sets `tap.nxy = vec2(0.0)` and so shades
> with no detail normal at all. That is the smooth interior, the hard edge and
> the wrong brightness, together.
>
> One trigger is fixed: on a cold or invalidated cache the detail stack took the
> whole generate budget and filled near-first, starving the macro stack that
> covers the view. Measured at frame 2 of `coastal-flyover`: 27.83% of the frame
> on the fallback, now 0.00%, converged frame unchanged.
>
> The surviving **green** patch was a different bug: a +768 ring move wrapped
> to the same origin as -256, and the dirty-strip code guessed the shorter move.
> It regenerated the overlap instead of the entering side, leaving valid stale
> texels indefinitely. The signed unwrapped displacement is now preserved. A
> frame-120 yaw jump followed by 120 stationary frames reproduces the old patch
> and shows it gone with the fix (9.5446% of pixels changed, peak delta 87).
>
> The hypotheses below are retained as the investigation history and are
> superseded for the reproduced persistent patch. Debug view 34 remains the
> first discriminator: red is a miss; green/blue is a cache hit.

## Symptom

Flat, dark, straight-edged bands lying on the terrain surface. Seen with
**Clipmap checked**. Two or three at a time, sometimes crossing most of the
view, sometimes tapering off screen.

Distinguishing features, from the captures:

- The band interior is **smooth** — the sand's ripple normal detail visible
  everywhere around it is absent or much weaker inside it.
- Edges are **straight and hard**, not soft or noisy.
- Brightness is wrong but not zero; it reads as an unlit or flat-shaded patch,
  not a black hole.
- Shape follows something planar, not the terrain contour. In one capture a
  band curved gently along its length while keeping straight ends.
- Present while flying (128–296 m/s) and while stationary.

The original captures live in the conversation record for 2026-08-15. The
deterministic frame-240 reproduction for the stale-cache mechanism is saved
under `target/clipmap-repro/` as `yaw-bug.png`, `yaw-fixed.png`, and
`yaw-diff.png`.

## Ruled out

Each of these was fixed and the artifact survived the fix, so none of them is
the cause:

| Ruled out | Why |
|---|---|
| **GTAO returning zero visibility** | Real bug, scene-wide, fixed (`context.md` §18, inverted `reconstruct_normal`). Artifact persists after the fix. |
| **Clipmap ready-bit vs ungenerated strips** | Fixed: `take_jobs` serves ready rings first and drops `ready` if a strip cannot be served. Artifact persists. |
| **Zero-alpha "never written" sentinel** | Was unsound (a real AO texel can be 0); generate now floors occlusion at 1/255. Artifact persists. |
| **Linear albedo in an 8-bit cache** | Real precision defect, fixed (perceptual storage). Not this. |
| **Ring picker jumping straight to macro** | Fixed: the tap now walks outward through the detail rings. Artifact persists. |
| **Terrain surface packs missing AO** | Disproved by `shipped_surface_packs_carry_occlusion_in_alpha` (passing test). |

## Historical gap (closed for the reproduced patch)

There was no clean clipmap-off comparison at the same camera. The one
clipmap-off capture taken was Dbg 8, which was entirely black because of the
GTAO bug, so it could not show whether the bands were present. The later source
debug and deterministic old-versus-fixed replay localized the persistent patch
to clipmap cache movement directly, so that comparison is no longer needed for
this mechanism:

```powershell
$env:SOMNIUM_TERRAIN_CLIPMAP = "0"; cargo run -p hello_engine --release
# ... then, in a NEW terminal or after Remove-Item Env:\SOMNIUM_TERRAIN_CLIPMAP
cargo run -p hello_engine --release
```

Same camera, normal shading, both runs. If the bands appear with the clipmap
**off**, everything above is a dead end and the subsystem is wrong.

## Historical hypotheses, superseded for the reproduced patch

1. **Foliage cards.** The shapes look like flat quads lying on the ground, and
   the scene has foliage at Cull 120 / LOD 45 / Impostor 90. `phase 25P`
   deleted a camera-facing plane impostor for being "a black triangle + FSR
   ghost" — the family resemblance is strong. Test: uncheck Foliage on the
   Terrain entity. **This is the cheapest test and it is not the clipmap.**
2. **Terrain chunk LOD seams / skirts.** Straight edges at chunk granularity,
   flat interior. LOD Morph is off in every capture. Test: Dbg 13 (chunk LOD)
   and Dbg 14 (triangle edges) aimed at a band.
3. **The clipmap ring-blend band.** `CLIPMAP_BLEND_TEXELS = 256` of a 1024 ring
   means the blend covers roughly the outer 77% of a ring's area, which is a
   large, geometrically-defined region. Test: Dbg 33 aimed **at** a band — if
   the band coincides with a ring index change, this is it.
4. **`shadow_only_queue` instances reaching the visibility pass.** Phase CR
   appends off-camera casters to the instance buffer *after* the vis draws.
   A mismatch between argument index and instance slot would draw geometry with
   another chunk's transform — which would look exactly like a flat sheet lying
   across the terrain. Test: `SOMNIUM_CASCADE_CULL=0` and `SOMNIUM_CPU_FRUSTUM=0`.
5. **Water wet-cell mesh.** The finite lake grid is a compact mesh clipped by a
   mask; a coverage failure could leave a flat quad above the sand. Test: the
   bands were seen well inland, which argues against it, but it is cheap to
   check by disabling the Water entity.

## Method note for whoever takes this

The three wrong attributions all came from reasoning about a screenshot instead
of measuring. What actually moved the investigation forward, every time, was a
debug view aimed at a specific question:

- Dbg 8 at the **boat** proved the occlusion fault was scene-wide rather than
  terrain-specific, which is what isolated GTAO.
- A unit test on the shipped asset alpha channel removed a whole branch of the
  hypothesis tree in one run.

Pick the hypothesis that a single toggle can eliminate, and eliminate it. Do
not reason from the shape of the artifact.
