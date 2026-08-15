# OPEN — dark band / ribbon artifact with Clipmap on

> **Status: UNRESOLVED, not being worked.** Recorded 2026-08-15 so the next
> session does not restart the investigation from a screenshot.
> **Do not** treat any hypothesis below as established. Three separate
> attributions have already been made and all three were wrong.

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

Captures live in the conversation record for 2026-08-15; none were saved to
disk. **If this is picked up again, save the frames into this folder first.**

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

## Not yet tested — and the gap that matters most

**There is still no clean clipmap-off comparison at the same camera.** The one
clipmap-off capture taken was Dbg 8, which was entirely black because of the
GTAO bug, so it could not show whether the bands were present. Now that GTAO is
fixed, that comparison is cheap and is the first thing to do:

```powershell
$env:SOMNIUM_TERRAIN_CLIPMAP = "0"; cargo run -p hello_engine --release
# ... then, in a NEW terminal or after Remove-Item Env:\SOMNIUM_TERRAIN_CLIPMAP
cargo run -p hello_engine --release
```

Same camera, normal shading, both runs. If the bands appear with the clipmap
**off**, everything above is a dead end and the subsystem is wrong.

## Untested hypotheses, roughly ordered

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
