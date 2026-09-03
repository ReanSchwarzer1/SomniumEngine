# The band, second mechanism: a wrapped origin does not carry a displacement

Credit: Codex isolated this one. This record verifies it independently and
keeps the reasoning.

`DF-BAND_resolution.md` found the clipmap's **miss** path and fixed one trigger
for it. It was not the whole artifact, and that record said so: *red means the
fallback and this analysis applies; green or blue means the cache served those
pixels and the fault is downstream.*

The pixels in the reported artifact are **green**. The cache served them. It
served the wrong thing.

## The defect

`update_ring` computes a true signed displacement in texels, then throws it
away and asks `toroidal_dirty_rects` to re-derive it from two wrapped origins:

```rust
ring.origin = [wrap_i(old_origin[0] + dx_tex, size), ...];
for rect in toroidal_dirty_rects(old_origin, ring.origin, size) { ... }
//                                            ^^^^^^^^^^^ the delta is gone
```

and that function guessed the shortest wrap:

```rust
fn wrap_delta(d: i32, size: i32) -> i32 {
    let mut d = ((d % size) + size) % size;
    if d > size / 2 { d -= size; }   // +768 becomes -256
    d
}
```

**Two wrapped origins do not determine a displacement.** On a 1024-texel ring,
`+768` and `-256` land on the same origin. The first exposes 768 new columns;
the second exposes 256. Choosing the shorter one queued 258 columns and left
766 holding material generated for the *old* world position.

Those texels are fully written, so their occlusion is non-zero, so
`clipmap_tap_detail` accepts them and `terrain_clipmap_source` reports a detail
ring. **Debug 34 draws the defect green.** The miss-path fix could not reach it,
and neither could any hypothesis phrased in terms of missing data.

Stale-but-valid is also why it *persists when stationary*: nothing re-dirties a
strip once the camera stops.

## Where it bites

The bug needs a one-frame slide in `(size/2, size)` — `(512, 1024)` texels.
Below that the shortest wrap is the true delta; at or above `size`,
`update_ring` already calls `mark_full`.

| ring | texels/m | 512 texels | 1024 texels |
|---|---|---|---|
| 0 | 512 | 1.0 m | 2.0 m |
| 1 | 256 | 2.0 m | 4.0 m |
| 2 | 128 | 4.0 m | 8.0 m |
| 3 | 64 | 8.0 m | 16.0 m |
| 7 | 4 | 128 m | 256 m |

At 60 Hz a focus moving 2 to 4 m per frame puts **ring 1** in that window on
every frame. That is 121 to 240 m/s, and the speeds in the original reports were
**121 and 205 m/s**. A large yaw change does it in one frame at any speed, by
swinging the 8 m look-ahead.

## Fix

Pass the displacement, because it is the thing that determines the answer:

```rust
pub fn toroidal_dirty_rects(old_origin: [i32; 2], delta: [i32; 2], size: i32) -> Vec<ClipRect>
```

`new_origin` is derived inside from `old_origin + delta`, so the two can no
longer disagree, and `wrap_delta` is deleted rather than left as a trap.

O3DE's `ClipmapBounds` does the same thing: update width comes from the
unwrapped centre difference, and wrapping happens only when mapping that
world-space strip into texture quadrants. Somnium wrapped first and inferred
second.

## Evidence

Camera held, yaw jumped 90 degrees at frame 120, captured at frame 240, so 120
frames stationary. Nothing here is a transient.

| capture | what it shows |
|---|---|
| `DF-STALE_yaw_bug.png` | a hard-edged wedge of the previous view's material across the hillside |
| `DF-STALE_yaw_fixed.png` | the same frame, regenerated |

**189,120 of 1,981,440 pixels (9.54%) exceed ±2, peak channel delta 87.**
Reproduced twice independently, landing on the same pixel count, peak, and
bounding box (x 192 to 1776, y 170 to 534).

```
SOMNIUM_TERRAIN_CLIPMAP=1 SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_STATIC=1
SOMNIUM_CAMERA_YAW=0 SOMNIUM_CAMERA_PITCH=-15
SOMNIUM_AUDIT_YAW_JUMP_FRAME=120 SOMNIUM_AUDIT_YAW_JUMP_DEGREES=90
SOMNIUM_CAPTURE_FRAME=240 SOMNIUM_VIEWPORT_RES=2 SOMNIUM_MAXIMIZE=1
```

`SOMNIUM_VIEWPORT_RES=2` and `SOMNIUM_MAXIMIZE=1` are load-bearing. At the
default viewport the same jump moves the focus too little to enter the window
and the comparison comes back byte-identical, which is exactly how a real defect
gets written down as "not reproducible".

## Guard

`a_slide_larger_than_half_the_ring_refreshes_the_entering_side` drives
`update_ring` through a 768-texel move in all four directions and asserts the
entering texel is queued. Against the old code it reports the actual failure:

```
queued rectangles were [ClipRect { x: 766, y: 0, w: 258, h: 1024 },
                        ClipRect { x: 0, y: 0, w: 2, h: 1024 }]
```

258 columns for a 768-column move.

## Method note

Both mechanisms were found by making the renderer say which path a pixel took,
not by looking harder at the picture. Debug 34 answered "is this a miss?" with
*no*, and that negative is what pointed at stale data. A screenshot cannot
distinguish "no data" from "wrong data": both are a flat patch with a hard edge.

## The clipmap now ships off

Requested after both fixes landed, and the codebase already half-agreed:
`TerrainClipmap::env_default_enabled` has said *"off until DF-E gates pass"*
since the cache was written, while `debug_toggles` said on. Two defaults for one
switch, and since `apply_debug_toggles` force-writes the field from the toggle,
the toggle won and the ring constructor's opinion never mattered.

`terrain_clipmap` is out of `default_for`. `SOMNIUM_TERRAIN_CLIPMAP=1` turns it
on, and so does the Clipmap checkbox.

**Virtual texturing had to follow it.** In VT mode `load_bc7_layers` registers
4x4 placeholders for the legacy layer arrays and only the rings carry the real
pages, so VT with the clipmap off is terrain shaded from eight mean colours.
Terrain construction now asks for VT only when the clipmap is on; otherwise it
loads `load_bc7_resident_layers`, the arrangement that predates the cache:

```
terrain: projected 213 MiB BC7 (0-15 at 2048, 16-31 at 1024)
```

Full hero resolution, real arrays. `DF-STALE_default_off.png` is the same
yaw-jump frame with the new default: detail texture throughout, no band, and
the dark patch that survived in `DF-STALE_yaw_fixed.png` is gone as well.

`the_clipmap_ships_off_in_both_places` asserts the two defaults agree, so they
cannot drift apart again.

Both fixes above stay in. They are what makes the cache correct when it is
switched on, which is now a deliberate act rather than the default.
