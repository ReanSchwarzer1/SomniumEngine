# The band artifacts: two cache failures, measured

Follows `DF-OPEN_clipmap_band_artifact.md`, which recorded three wrong
attributions and one instruction: *do not reason from the shape of the
artifact; pick the hypothesis a single toggle can eliminate.* The reason that
was hard to follow is that no instrument could see the answer.

## The instrument was blind

Debug 33 (Clipmap Ring) draws `terrain_clipmap_ring` as greyscale:

```wgsl
terrain_clipmap_ring = select(1.0, f32(ring) / f32(rings - 1), ring < rings);
```

The outermost detail ring is 7/7 = **1.0**. "No ring at all" is also **1.0**.
The one question worth asking — *did this pixel come from the cache?* — is the
one question mode 33 renders identically either way.

Debug 34 (Clipmap Source) is new and categorical, not a ramp:

| colour | meaning |
|---|---|
| green | a detail ring |
| blue | a macro ring |
| **red** | the flat macro-map fallback |
| yellow | the constant colour, when even the macro map is missing |

## A red band is the fallback

`evaluate_clipmap_material`, when neither stack can serve a pixel:

```wgsl
tap.albedo = m * m;        // the low-frequency unique-colour map
tap.roughness = 0.8;
tap.occlusion = 1.0;
tap.nxy = vec2<f32>(0.0);  // <- no detail normal
```

`nxy = 0` means the surface is shaded with the geometric normal and nothing
else. Against sand that carries a ripple normal everywhere around it, that is a
**smooth patch with a hard edge and slightly wrong brightness** — every
distinguishing feature the original report listed, in four lines of shader.

The fallback is not corrupt. It is the designed miss path, and it is far too
visible.

## Why pixels were reaching it

Measured on `coastal-flyover`, debug 34, `SOMNIUM_TIME_STATIC=1`:

| frame | fallback % of frame, before |
|---|---|
| 2 | **27.83** |
| 5 | 2.40 |
| 10 | 0.00 |

Frame 2 is a small sharp green disk surrounded by red. Two causes, both
ordering:

1. **Detail took the whole budget first.**
   ```rust
   let detail = clipmaps[i].take_jobs(true, &mut budget);
   let macro_jobs = clipmaps[i].take_jobs(false, &mut budget);
   ```
   One shared `MAX_GEN_TEXELS` = 1024². On a cold cache the detail stack
   exhausted it every frame, so the macro stack — the only one that covers the
   whole view — was starved for the ten-odd frames detail took to fill.

2. **Detail filled near-first even when nothing was on screen.**
   `DETAIL_GEN_ORDER = [3, 2, 1, 0, 4, 5, 6, 7]` paints the 8 m disk underfoot
   first. That is right for a ring already being sampled, whose dirty
   rectangles are the thin strip that just slid into view. It is wrong for a
   ring that is not ready, because a not-ready ring is skipped by the picker
   entirely — nothing on screen improves until it finishes, and what the screen
   needs first is *some* data everywhere.

## Fix: coverage before sharpness, but only while cold

```rust
fn gen_order(is_detail: bool, ready_pass: bool) -> &'static [usize] {
    match (is_detail, ready_pass) {
        (true, true)  => &DETAIL_GEN_ORDER,  // [3,2,1,0,4,5,6,7] near-first
        (true, false) => &DETAIL_COLD_ORDER, // [7,6,5,4,3,2,1,0] coarsest-first
        (false, _)    => &MACRO_GEN_ORDER,   // already coarsest-first
    }
}
```

and the budget goes to macro first while `macro_covers_view()` is false.

The ready path is untouched, so walking still sharpens underfoot first.

## Result

| frame | fallback % before | after | detail % | macro % |
|---|---|---|---|---|
| 2 | 27.83 | **0.00** | 0.00 | 52.35 |
| 5 | 2.40 | **0.00** | 38.46 | 4.92 |
| 10 | 0.00 | 0.00 | 38.43 | 5.26 |
| 240 | 0.00 | 0.00 | 38.39 | 6.32 |

Frame 2 is now blue everywhere: coarse, correct, and carrying normals, which
sharpens to detail by frame 5.

**The converged frame is unchanged**: 38 of 921,600 pixels differ by more than
2, peak channel delta 5 — TAA dither, not a visual change. This is a scheduling
fix, so the steady state must not move, and it does not.

Evidence: `DF-BAND_before_f2.png` / `DF-BAND_after_f2.png` (and `_f5`), with
`_before_normal` / `_after_normal` for the converged pair.

## A green band was stale cache data

The artifact that survived the cold-cache fix was a second mechanism. Debug 34
showed the large rectangular patch as **green**, not red: the detail stack was
returning a cache hit, but the texels belonged to an old world position.

`update_ring` knew the exact signed movement (`dx_tex`, `dy_tex`) but discarded
it after wrapping the ring origin. `toroidal_dirty_rects` then reconstructed the
shortest displacement between the two wrapped origins. For a 1024-texel ring,
a real +768 move and a -256 move have the same wrapped endpoint. The old code
therefore refreshed the 256-texel overlap side and left the 768 entering
columns untouched. Their non-zero material alpha kept them looking valid, so
the miss-path scheduling fix could never repair them after the camera stopped.

The dirty-strip interface now carries the original unwrapped signed delta.
Wrapping is used only to place the update rectangles in the texture. This is
the same separation used by O3DE's clipmap bounds updater: update width is
derived from the actual center displacement before the affected world region
is mapped into wrapped texture quadrants.

Evidence:

- The regression test first failed with only x=766..1024 and x=0..2 queued for
  a +768 slide; an entering texel at x=100 was not dirty. It now checks +X,
  -X, +Y, and -Y displacements larger than half the ring.
- A permanent low-altitude replay changes yaw by 90 degrees at frame 120, then
  holds still for another 120 frames. With the old algorithm, the large
  hard-edged terrain patch remains at frame 240. With the signed delta, it is
  gone.
- The old and fixed frame-240 captures differ at 189,120 of 1,981,440 pixels
  (9.5446%) with a peak channel delta of 87. Evidence is under
  `target/clipmap-repro/` (`yaw-bug.png`, `yaw-fixed.png`, `yaw-diff.png`).
- Fresh STF-off and start-on-then-off captures were byte-identical in this
  clipmap path. STF was correlated with the report, not the cause of this
  reproduced patch.

## What this does and does not settle

Settled: two visually similar cache artifacts and their triggers are measured
and fixed. Red is the flat fallback caused by missing coverage; green or blue
can be stale valid cache data caused by losing a ring's true displacement.

Not claimed: that every dark terrain mark in every scene is a clipmap defect.
The source debug remains the discriminator. A future red band is a coverage
miss; a future green or blue band means a cache hit and needs its world-to-cache
mapping checked. The flyover held 0.00% fallback at every frame past 10, and
the persistent yaw-jump patch is gone after 120 stationary frames.

**How to check, instead of guessing from a screenshot:** turn on Clipmap Source
in the debug view menu and look at the band. Red means the fallback and this
analysis applies. Green or blue means the cache served those pixels and the
fault is downstream, which would be new information and worth a fresh record.

## Guards

| Test | Catches |
|---|---|
| `a_cold_cache_paints_coverage_before_sharpness` | the order regressing to near-first while cold, or either order ceasing to be a permutation |
| `every_registered_debug_view_has_a_branch_in_the_shader` | a debug view registered in `somnium_ui` with no branch in `shading.wgsl`, which renders as the ordinary lit image |

The second replaces `assert_eq!(codes.last(), Some(&33))` — a hard-coded number
that could not tell a missing branch from a new view, and failed on the next
view added either way. Verified by deleting the mode 34 branch: it names
`clipmap_source`.
