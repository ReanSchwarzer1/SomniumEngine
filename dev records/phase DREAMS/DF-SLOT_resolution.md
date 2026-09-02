# The band, third mechanism: two uploads to one uniform slot

`DF-BAND_resolution.md` fixed the **miss** path (red). `DF-STALE_resolution.md`
fixed a **stale-texel** path (green) caused by losing a ring's true
displacement. Both records ended with the same instruction:

> Red means the fallback and this analysis applies. Green or blue means the
> cache served those pixels and the fault is downstream, which would be new
> information and worth a fresh record.

The pixels reported this time are **green**, and this is that record.

## What was still wrong

Hard-edged darker patches lying on the terrain, with Clipmap checked, near the
ground, surviving indefinitely after the camera stops. Reported against a build
that already carries both earlier fixes.

## The defect

`TerrainClipmapPass::record` is called twice per frame per terrain — once for
the detail stack, once for the macro stack — and each call did this:

```rust
queue.write_buffer(&self.params, 0, &bytes);
...
pass.set_bind_group(1, &self.bind, &[(i as u32) * PARAMS_STRIDE as u32]);
```

Both calls wrote **the same buffer at offset 0**. wgpu's own contract is
explicit about what that means:

> Calls to `write_buffer()` do *not* submit the transfer to the GPU
> immediately. They begin GPU execution only on the next call to
> `Queue::submit()`, **just before the explicitly submitted commands**.

So the two uploads did not take turns. Both landed before either render pass
ran, in call order, and the second won. **The detail generate pass executed
with the macro stack's uniforms**: its `rect_min`/`rect_max`, `center`,
`origin_uv`, `texels_per_m` and `ring`.

Nothing about the *commands* was wrong — the scissor and viewport come from the
job, not the uniform, so the detail rectangle was still the region rasterised.
The shader then compared those detail texels against the macro rectangle:

```wgsl
out.albedo = vec4<f32>(0.0);
out.surface = vec4<f32>(0.5, 0.5, 0.8, 1.0);
if tex.x < clipmap_gen.rect_min.x || ... { return out; }
```

The two rectangles do not coincide, so most of the scissored texels took that
early-out. What it writes is the trap: **alpha is 1.0**. `clipmap_tap_detail`
reads a non-zero alpha as *data* rather than as an ungenerated texel — that is
the whole point of the `1.0 / 255.0` floor `DF-BAND` added — so the ring keeps

- `albedo = 0` → black,
- `nxy = (0, 0)` → no detail normal, shaded by the geometric normal alone,
- `occlusion = 1.0` → valid,

inside an axis-aligned world-space rectangle with a hard edge. Nothing
re-dirties it, so it persists for as long as the ring holds its centre, and
debug view 34 reports it as a healthy detail-ring hit. Green.

That is every feature the original report listed, one more time, from a third
cause.

## Why it is intermittent

The collision needs both stacks to hand generate a job in the *same* frame.
Over the 126 generating frames of the repro below:

| | frames |
|---|---|
| detail jobs | 121 |
| macro jobs | 13 |
| **both, and therefore colliding** | **8** |

Eight corrupted batches in one flight, each permanent. It is common enough to
be seen every session and rare enough that most frames look fine, which is
exactly the shape of the report.

## The instrument that was missing

Debug view 34 answers *which stage produced this pixel*, and the answer here is
"a detail ring" — correct, and useless. Nothing could say **which rings were
ready**, how much work was queued, or whether the source-page cache was still
moving, so a starved stack and a healthy one rendered the same plausible blur.

`SOMNIUM_CLIPMAP_TRACE=1` now emits one `DF-TRACE` line per generating frame:

```
DF-TRACE frame=123 detail_ready="11111111" macro_ready="1111" detail_jobs=1
         macro_jobs=4 queued_texels=0 vt_resident=0 vt_pending=0 ...
```

Two things fell out of it immediately and are worth writing down:

- The stack **does** converge. Ready went to `11111111` / `1111` with zero
  queued texels by frame 123, and generation stopped entirely by frame 126.
  Whatever was wrong was not a starved cache.
- **Virtual texturing is not involved.** `vt_resident=0`, `vt_uploads=0` for
  the whole run: this scene's terrain never enables it, so the
  feedback/`invalidate` loop was never a suspect here at all.

## The rail the fixture did not have

`coastal-flyover` never stops, and `coastal-ground` never moves. A cache that
is merely *behind* and a cache holding material from somewhere else look
identical while the camera is moving; only the frames after a stop tell them
apart, and no rail produced those. `SOMNIUM_DREAMS_RAIL_STOP=<frames>` brakes a
flyover and holds its last position:

```
SOMNIUM_TERRAIN_CLIPMAP=1 SOMNIUM_TIME_VIEW=coastal-ground
SOMNIUM_DREAMS_RAIL=coastal-flyover SOMNIUM_DREAMS_RAIL_STOP=120
SOMNIUM_TIME_STATIC=1 SOMNIUM_CAPTURE_FRAME=240
SOMNIUM_VIEWPORT_RES=2 SOMNIUM_MAXIMIZE=1
```

200 m/s for 120 frames from walk height, then 120 stationary frames. Nothing in
the captured frame is a transient.

A rail is frame-indexed from the frame the map finished loading, and
`SOMNIUM_CAPTURE_FRAME` counts from process start. Those are two clocks, and a
slower load quietly moves the capture to a different point on the rail — which
is how a capture pair that reads like the same recipe turns out not to be the
same experiment. The rail now logs what it armed with:

```
DREAMS rail armed rail="coastal-flyover" start_frame=0
                  anchor=Vec3(-31.5, 24.696337, 8.5) yaw=-90 pitch=-8
```

Check it before comparing two captures.

## Fix

Each `record` call takes its own slice of the params buffer. A cursor rather
than one buffer per stack, because the collision is between *calls*: a second
terrain would have aliased the first the same way.

```rust
pub fn begin_frame(&mut self, device: &wgpu::Device, terrains: usize)
```

resets the cursor once per frame and sizes the buffer for the frame's worst
case — `MAX_TAKE_JOBS` per stack, two stacks, per terrain — before any write,
so growing it cannot discard params an earlier call has already made. Running
out of slots logs an error rather than dropping jobs silently: `take_jobs` has
already cleared their dirty rectangles, so a dropped job leaves texels marked
clean that nothing will ever generate.

## Evidence

Same rail, same stop, same capture frame, both builds. The clipmap-**off**
capture is **byte-identical** across the two builds, which is what makes the
pair a measurement rather than two screenshots.

| capture | what it shows |
|---|---|
| `DF-SLOT_bands_before.png` | straight-edged darker wedges across the dune |
| `DF-SLOT_bands_after.png` | the same frame, no edges |
| `DF-SLOT_clipmap_off.png` | the reference the cache is standing in for |
| `DF-SLOT_source34_after.png` | green everywhere — the cache serves it all |

**82,122 of 1,981,440 pixels (4.14%) changed, peak channel delta 76**, inside
y 349..840 — the terrain band, and only it.

## What this does not change

The DF-QUALITY verdict is untouched, and was checked rather than assumed. Its
static-camera capture is **byte-identical before and after this fix** (0
pixels, peak 0), because a held camera almost never collides the two stacks.

Sharpness is unmoved by design:

| | mean abs Laplacian |
|---|---|
| clipmap off | 5.295 |
| clipmap on, before | 1.702 |
| clipmap on, after | 1.658 |

The bands were a bug; the blur is the ring density, still measured at roughly
two thirds of the surface's high-frequency detail discarded, and still the
reason `terrain_clipmap` ships off. This fix is what makes the cache correct
for anyone who turns it on, not an argument for turning it on.

## Guards

| Test | Catches |
|---|---|
| `one_frame_never_hands_out_a_slot_twice` | two `record` calls naming the same uniform slot |
| `a_frames_capacity_covers_both_stacks_at_their_cap` | a frame budget too small for both stacks at `MAX_TAKE_JOBS` |
| `begin_resets_the_cursor_between_frames` | a cursor that grows without bound across frames |
| `a_flyover_can_brake_and_then_hold_its_last_position` | the fly-then-stop rail regressing to never stopping |
| `an_unset_stop_leaves_the_flyover_flying` | the DREAMS-B captures moving under an unset variable |

## Method note

The two earlier records both say the same thing and it held again: make the
renderer say which path a pixel took, rather than looking harder at the
picture. What actually moved this one forward was the negative from debug 34 —
*not a miss* — combined with a trace line proving the stack had converged. That
left "the cache was written with the wrong parameters" as the only surviving
shape, and the parameters are uploaded in exactly one place.

The sweep afterwards is worth recording too: every other pass whose `record` is
called more than once per frame either allocates a buffer per call (`bloom`),
writes one buffer per phase (`cull`), writes at a per-item stride
(`shadow::record_virtual`), or sits in a mutually exclusive match arm
(`shadow_pass.record`). The clipmap generate pass was the only site with this
defect.
