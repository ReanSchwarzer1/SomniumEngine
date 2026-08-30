# DOOM-J — bandwidth, formats, allocations

**Status:** complete, 2026-08-30. One measurable clause met and proved; two
closed without a change because the measurement did not support one.

## The three clauses

The 2026-08-16 stage asked for three things, and §16's rebase added the rule
that decides all three: *"Capture allocation/format inventory and change only
rows with measured traffic or churn."*

| Clause | Outcome |
|---|---|
| An allocation counter asserting zero buffer, texture and bind-group creation in a steady-state frame | **Built, and the assertion holds** for everything the engine owns |
| `RGB9E5` or `f16` for intermediate targets *where the census says bandwidth matters* | **No change.** The census says it does not — see below |
| Remove redundant full-resolution copies | **No change.** The inventory found none to remove |

## The counter

Two hundred and sixty-five `create_*` call sites is far too many to wrap, so the
counter is wgpu's own. `wgpu = { features = ["counters"] }` is on in every build
rather than behind a feature flag — this repository's own lesson is that *a gate
you have to opt into is a gate that is off* — and the cost is one relaxed atomic
per resource create and destroy, which in steady state is the number the gate
exists to prove is zero.

The counters are **gauges**: one increment per creation, one decrement per
destruction. That shape decides how they must be read. Comparing the endpoints
of a window would report "nothing changed" for a resource created every frame
and destroyed the next, which is precisely the churn worth finding. So the
sample is taken **every measured frame** and it is the per-frame delta that is
accumulated:

| Row | Meaning |
|---|---|
| `alloc_churn_frames` | Measured frames in which any live object count moved |
| `alloc_worst_frame_delta` | Largest single-frame movement |
| `churn_<object>` | Frames in which *that* counter moved — emitted only when non-zero |
| `alloc_net_*` | Endpoint difference, which is the leak question rather than the churn one |
| `live_*` | The inventory the gate is a gate over |

`churn_<object>` exists because the first run reported "68 of 300 frames
churned" and that is not something anybody can act on.

## What the first run found

Coastal ground, 180 warm-up / 300 measured, fixed camera and sun, 1920×1032.

| | Coastal | Island |
|---|---:|---:|
| `alloc_churn_frames` | 59 / 300 | 96 / 300 |
| `alloc_worst_frame_delta` | 1 | 1 |
| `churn_buffers` | **59** | **96** |
| `churn_textures`, `churn_texture_views`, `churn_bind_groups`, `churn_samplers` | **0** | **0** |
| `live_buffers` | 342 | 192 |
| `live_texture_views` | 594 | 430 |
| `live_bind_groups` | 181 | 116 |

**Textures, texture views, bind groups and samplers do not move at all** across
three hundred steady-state frames on either map. Exactly one buffer appears and
disappears, on about a fifth of frames.

Island churns *more* than Coastal while drawing 38 objects to Coastal's 66,
which rules out the obvious explanations — it is not proportional to draws, to
instances, or to scene size.

## Naming it

Counters say *that* a buffer moved. Only wgpu's allocator report says *which*,
because it carries the label each resource was created with, so
`SOMNIUM_ALLOC_TRACE=1` diffs the multiset of allocation names frame to frame.
It is opt-in: rebuilding that multiset once a frame is far too much work to
leave on.

```text
alloc churn frame=62 name=(wgpu internal) Staging before=54 now=53
alloc churn frame=63 name=(wgpu internal) Staging before=53 now=54
alloc churn frame=68 name=(wgpu internal) Staging before=54 now=53
alloc churn frame=69 name=(wgpu internal) Staging before=53 now=54
```

One name, oscillating by one on a fixed six-frame cycle: **wgpu's own staging
pool** for `Queue::write_buffer` and `write_texture`. No engine-labelled
resource is created or destroyed in a steady-state frame on either map.

The clause is therefore met, with its boundary stated: *the engine allocates
nothing in a steady-state frame; wgpu's staging pool breathes by one buffer, and
that is inside wgpu.*

## The inventory

`SOMNIUM_ALLOC_TRACE=1` also logs a one-shot breakdown by label on the last
measured frame — where the memory went does not change frame to frame. Coastal
ground, 601 allocations across 11 memory blocks, **1901.7 MiB allocated of
2368.0 MiB reserved**:

| Label | MiB | Objects |
|---|---:|---:|
| Scene Texture | 298.7 | 11 |
| **Global Vertex Buffer** | **256.0** | 1 |
| **Global Index Buffer** | **128.0** | 1 |
| Mesh BLAS | 127.9 | 278 |
| Terrain Albedo+Height / Surface BC7 (hero) | 85.4 + 85.4 | 2 |
| Shadow Atlas | 64.0 | 1 |
| Water FFT scratch | 64.0 | 1 |
| ReSTIR reservoirs (DI + GI A/B) | 56.3 + 42.2 + 42.2 | 4 |
| Terrain clipmap detail albedo / surface | 32.0 + 32.0 | 2 |
| Water spectral wind / displacement / gradient | 48.0 + 24.0 + 24.0 | 9 |
| Terrain Albedo+Height / Surface BC7 (extra) | 21.4 + 21.4 | 2 |
| Global Instance Buffer | 16.0 | 1 |

The 213.5 MiB of terrain BC7 arrays match the *"projected 213 MiB BC7"* the
terrain loader logs at startup, which is a useful independent check that the
report is measuring what it claims to.

**The largest single row is a deliberate trade, and it is the one that buys this
stage's result.** `VERTEX_POOL_BYTES` and `INDEX_POOL_BYTES` are fixed 256 MiB
and 128 MiB blocks allocated once at construction. Coastal's 336,364 triangles
use about 4 MiB of the index pool — roughly 3% — and 384 MiB is reserved so that
uploading geometry can never reallocate. That is *why* `churn_buffers` sees no
engine buffer: the pools cannot grow because they were never sized to the scene.
Shrinking them would trade this stage's exit criterion for footprint, and
nothing measured says footprint is the constraint. **Recorded, not changed.**

- [`DOOM-J_coastal-ground_inventory.somtime`](DOOM-J_coastal-ground_inventory.somtime)
- [`DOOM-J_island-ground_inventory.somtime`](DOOM-J_island-ground_inventory.somtime)

## Why no format changed

The stage proposed `RGB9E5` or `f16` intermediates *"where the census says
bandwidth matters"*. The census says the opposite, and has since DOOM-A:

> `Shading.frag = 3 563 520`, which is exactly 2560 × 1392. One fragment per
> pixel, no more. So the whole 25.8 ms is *cost per pixel*.

A frame that shades each pixel exactly once, whose dominant zone is per-pixel
shader work, is not the frame where narrowing an intermediate format wins.
DOOM-C already took the per-pixel-cost lever and measured it. Changing formats
here would be the speculative change §16 and the measurement contract both
forbid — *"a number without a control is not evidence"*, and there is no number
at all pointing at intermediate bandwidth.

The inventory found no redundant full-resolution copy to remove either. The 11
`Scene Texture` allocations are the render-target set, not duplicates of it.

**What this stage does not have is a bytes-per-frame counter.** `live_*_mib` is
a footprint, not traffic; nothing here measures bandwidth. The first clause was
provable with what wgpu exposes and the other two were not, and building a
bandwidth counter to justify a format change nothing has asked for would be the
same speculation from the other end. Stated as a gap rather than closed over.

## Commands

```bash
SOMNIUM_TIME_STATIC=1 SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=300 SOMNIUM_MAXIMIZE=1 SOMNIUM_SUN_ELEVATION=45 SOMNIUM_SUN_AZIMUTH=120 SOMNIUM_TIME="dev records/phase DOOM/DOOM-J_coastal-ground_inventory.somtime" cargo run --release -p hello_engine
```

```bash
SOMNIUM_ALLOC_TRACE=1 SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=60 SOMNIUM_TIME=trace.somtime cargo run --release -p hello_engine
```
