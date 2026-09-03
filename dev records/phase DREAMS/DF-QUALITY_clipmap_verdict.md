# The clipmap's artifacts are fixed. Its output is still worse than not using it.

Two band mechanisms were found and fixed (`DF-BAND_resolution.md`, the miss
path; `DF-STALE_resolution.md`, the stale-texel path). Toggling the cache on
still produced a wrong-looking surface, so this measures what is left.

> **A third band mechanism was found on 2026-09-02** — the generate pass's
> uniforms, uploaded twice to one slot (`DF-SLOT_resolution.md`). It does not
> touch anything below. The capture this record is built on was re-taken
> against the fixed build and is **byte-identical**: 0 of 1,981,440 pixels
> differ, peak channel delta 0. A held camera almost never collides the two
> stacks, which is exactly why that defect survived this measurement and had to
> be found from a moving one.

## Repro: the checkbox, not the environment variable

A switch thrown at frame 200 is not the same experiment as the same switch set
before startup. The second decides how the terrain loads its textures and starts
every cache warm; the first invalidates a running cache under a camera that has
been moving. The artifact was always reported on the *toggle*, and there was no
way to capture that without synthetic mouse input.

`SOMNIUM_AUDIT_TOGGLE_FRAME` / `SOMNIUM_AUDIT_TOGGLE_SWITCH` flip any render
switch mid-run through the same three statements as
`Engine::toggle_render_switch`.

```
SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_STATIC=1
SOMNIUM_CAMERA_YAW=0 SOMNIUM_CAMERA_PITCH=-15
SOMNIUM_AUDIT_TOGGLE_FRAME=120 SOMNIUM_AUDIT_TOGGLE_SWITCH=terrain_clipmap
SOMNIUM_CAPTURE_FRAME=240 SOMNIUM_VIEWPORT_RES=2 SOMNIUM_MAXIMIZE=1
```

**First result, and a good one:** toggled-on at frame 120 and on-from-startup
are byte-identical at frame 240 (0.00% of pixels differ, peak 1). The toggle
path converges exactly on the startup path. Whatever remains is not a
toggle-ordering bug, and both earlier fixes hold.

## The cache is serving everything, and what it serves is a blur

Debug 34 (Clipmap Source) after the toggle:

| stage | share of frame |
|---|---|
| a detail ring | 99.74% |
| a macro ring | 0.26% |
| **the fallback** | **0.00%** |

No misses at all. The miss path is genuinely fixed, and nothing here is stale
(the camera is held for 120 frames and the image is stable).

But against direct shading at the same camera:

| | mean abs Laplacian over the terrain |
|---|---|
| clipmap **off** (direct shading) | **7.05** |
| clipmap **on** | **0.90** |

**The cache discards 87% of the surface's high-frequency detail.** 82.49% of
pixels differ from the direct image, peak channel delta 160. Compare
`DF-QUALITY_clipmap_off.png` with `DF-QUALITY_clipmap_on.png`: the gravel
grain, the grass texture and the dark drainage lines are all present in one and
absent from the other.

## Why, and why it is not a bug

The clipmap caches *composited, shaded* material into rings of fixed texel
density: 512 texels/m over a 1 m radius, halving each ring out to 4 texels/m at
128 m. Direct shading samples 2048² hero layers through the sampler's own mip
chain at whatever density the surface actually needs.

Past the innermost metre or two, the ring is simply a coarser representation
than the thing it is standing in for. Nothing is corrupt; there is less of it.

That is exactly what `TerrainClipmap::env_default_enabled` has been saying since
the cache was written — *"off until DF-E gates pass"*. The gates are the ring
density and coverage. They were never passed, and the toggle table quietly
overrode the constructor's opinion, which is how the cache came to be on in
shipped builds at all.

## Verdict

**Keep it off.** It is now off by default in both places, with
`the_clipmap_ships_off_in_both_places` holding them together.

The two bug fixes stay in. They are what makes the cache correct when someone
switches it on, and they are prerequisites for any future DF-E work — but
correctness was never the reason to have it on, and at this ring configuration
it costs more picture than it saves frame time.

Turning it on is a deliberate act: the Clipmap checkbox, or
`SOMNIUM_TERRAIN_CLIPMAP=1`.

## What DF-E would actually have to change

Not listed as a plan, only so the next session does not re-measure this:

- Ring density. 512 texels/m over a 1 m radius is below what the near ground
  needs at walk height, so even the finest ring loses detail.
- Or cache something other than final composited material — a splat/weight
  cache re-shaded per pixel keeps the detail and still saves the blend.

Either is a phase, not a fix.
