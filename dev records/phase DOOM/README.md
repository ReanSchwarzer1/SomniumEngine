# Phase DOOM — evidence

Plan: [`../phase_DOOM.md`](../phase_DOOM.md).

`.somtime` files are deterministic GPU timing runs written by
`crates/somnium_renderer/src/timing.rs` (DOOM-A). Each row carries a **standard
deviation** beside its mean, and a comparison calls anything inside the combined
band `~ noise` rather than a win. That exists because a screen-capture frame
delta in this project once varied from 0.776 to 2.018 across three runs of one
identical build, and a whole session went into the variance instead of the
change.

```bash
SOMNIUM_MAXIMIZE=1 SOMNIUM_TIME=after.somtime SOMNIUM_TIME_VIEW=coastal-ground \
  SOMNIUM_TIME_COMPARE="dev records/phase DOOM/DOOM-A_coastal-ground.somtime" \
  cargo run --release -p hello_engine
```

| Variable | Meaning |
|---|---|
| `SOMNIUM_TIME=<path>` | enable a run and write the table here (parent directory is created) |
| `SOMNIUM_TIME_VIEW` | `coastal-overview`, `coastal-ground`, `island`, `island-ground` — picks the map **and** the camera |
| `SOMNIUM_TIME_COMPARE=<path>` | diff against an earlier run and log the verdict per zone |
| `SOMNIUM_TIME_WARMUP` / `SOMNIUM_TIME_FRAMES` | default 180 / 300 |
| `SOMNIUM_TIME_LABEL` | free text recorded in the header |
| `SOMNIUM_TIME_QUIT=0` | keep the window open after the run (default is to exit) |
| `SOMNIUM_TIME_STATIC=1` | suppress the example's dynamic boat when a gate requires a truly static scene |
| `SOMNIUM_VOXEL=1` | spawn the voxel terrain at map load, so chunk streaming is visible to a timing run (DOOM-H) |
| `SOMNIUM_MAP=coastal\|island` | startup map, independent of a timing run |

The viewpoints are **stationary**. Every number this project already has —
DF-A's overview and walk, XV-J's kit views — was taken from a still camera, and
a still camera keeps terrain streaming, clipmap recentring and LOD transitions
out of the measurement. A flythrough is the hitch experiment (DOOM-I), not this
one.

## Baselines — 2026-08-16, pre-DOOM tree

`DOOM-A_coastal-ground.somtime`, `DOOM-A_coastal-overview.somtime`,
`DOOM-A_island.somtime`. RTX 5080 Laptop, Vulkan, release `hello_engine`.
Coastal runs are maximized Native (2560×1392); Island is the default 1280×720
window. **Do not overwrite these** — they are what every later sub-phase is
measured against.

| | Coastal ground | Coastal overview | Island |
|---|---:|---:|---:|
| Frame | **38.392** | **34.143** | 7.652 |
| Shading | **25.769** | **15.048** | 2.322 |
| ReSTIR GI | 3.866 | 8.427 | 0.717 |
| Water prepass | 2.619 | 3.101 | 2.041 |
| Water shade | — | 1.721 | 0.691 |
| GTAO | 1.464 | 0.942 | 0.213 |
| FSR | 1.444 | 1.459 | 0.391 |
| Shadows | — | 1.074 | 0.312 |
| **unattributed** | **0.37%** | **0.43%** | **1.74%** |

Before DOOM-A the same row was **13%** (`context.md` §17.7). Gate is < 5%.

## What the baselines say

**Shading is 67% of a Coastal ground frame.** Not "dominant" as an impression —
25.769 of 38.392 ms, with a standard deviation of 0.677.

**It is not overdraw and it is not wasted invocations.** The pipeline-statistics
counter reads `Shading.frag = 3 563 520`, which is exactly 2560 × 1392. One
fragment per pixel, no more. So the whole 25.8 ms is *cost per pixel*, and the
only lever that can move it is a smaller shader on a given pixel — which is
DOOM-C's thesis, now measured rather than argued.

**Reproducibility.** Two consecutive runs of one identical build on Island:
Frame 7.652 → 7.661 (+0.1%), Shading 2.322 → 2.341 (+0.8%). **Every** zone was
reported `~ noise`. The harness does not manufacture wins.

**Two things worth a look that were not being looked at**, both newly visible
because the scopes did not exist before:

- `ReSTIR GI` on the overview is **8.427 ms with a standard deviation of 6.024
  and a maximum of 39.125**. The mean is not the story; something is
  occasionally costing an entire frame's budget in one pass. Not investigated.
- `Water prepass` ranges 0.636 → 3.254 ms on the same still camera.

Neither is scheduled. They are recorded here so the next session does not
rediscover them, and so nobody reads the Coastal means as steady.

---

# DOOM-D — per-cascade shadow cache

Completed 2026-08-30. The persistent CSM atlas now redraws only invalidated
quadrants, with explicit light/view/caster revisions, staggered distant view
updates, an observable `shadow_cascades_rendered` counter, and
`SOMNIUM_SHADOW_CACHE=0` restoring the old four-cascade path.

Matched 180-warm-up / 300-frame Coastal-ground runs at 1920×1032 measured the
static cache at **0.0028 ms and 0/4 cascades**, versus **0.9633 ms and 4/4**
with the kill switch, both with 218 filtered casters. Tone-mapped captures at
the same deterministic frame are visually equivalent.

Implementation details, commands, failure archaeology, and evidence links:
[`DOOM-D.md`](DOOM-D.md).

---

# DOOM-G — counted draw submission

Completed as a measured, default-off experiment on 2026-08-30. The cull shader
can compact survivors into single-/double-sided partitions and the visibility
pass consumes GPU-authored counts when `SOMNIUM_DRAW_COMPACTION=1` and the
adapter supports it. Dense args remain authoritative for culling and IDs.

On the current 66-object Coastal-ground fixture, counted submission changed
combined cull + visibility from **0.3270 to 0.3198 ms** — a 0.0072 ms difference
well inside the runs' spread. Dense submission therefore remains the default.
See [`DOOM-G.md`](DOOM-G.md) for the architecture, uniform-layout failure caught
by the first live run, determinism caveat, matched timings, and parity captures.

---

# DOOM-H — one scheduler

Completed 2026-08-30. The stage as planned in 2026-08-16 — build a worker pool
over rayon — was obsolete: MORROWIND-B already shipped `somnium_jobs`. H became
migration and proof instead.

Voxel chunk meshing was the last thing in the workspace still detaching onto
rayon's global pool, a debt GHOSTFENCE had been carrying as a stated exemption
since PORTAL-0-C. It now goes through `somnium_jobs` with cancellation on
despawn, retry on a refused submission, and retry on worker failure; the
exemption is gone and the `one-job-system` row passes on its own terms.

`SOMNIUM_VOXEL=1` spawns the voxel terrain at map load so a timing run can see
the work at all. Streaming ~118 chunks at warm-up 0 held the budgeted
main-thread install (`Jobs & assets`) to a **1.83 ms maximum against its 2 ms
budget**, with frame means `~ noise` against a no-voxel control.

Full audit, the status-bar regression the migration exposed and how it was
fixed, and the matched runs: [`DOOM-H.md`](DOOM-H.md).

---

# DOOM-I — off the critical path

Completed 2026-08-30. The stage's own exit criterion was a hitch metric, so that
was built first: `hitch` rows carrying the run's median, p99, worst, the count of
frames over **2x the median**, and where in the run those were.

It found its own blind spot immediately — the first tick had no previous tick, so
the largest stall in any session was never recorded — and then it found the
stall. **Steady state was already hitch-free** on both maps; the whole problem
was startup, at 8.2 s, of which **6.9 s was thirty-two average colours**.
`mean_albedo_from_sources` decoded every layer's surface map as well as its
albedo, Lanczos3-resized each 2048 source to 256 before averaging, and did both
serially.

Coastal startup **8210.8 -> 1574.3 ms (-80.8%)**; Island 2070.8 ms; median frame
and hitch count unchanged. The worst mean-albedo disagreement over the shipped
packs is 0.0184 in linear albedo.

Method, the instrumentation added to the map build, the correctness audit, and
why a tone-mapped capture could not settle it: [`DOOM-I.md`](DOOM-I.md).

---

# DOOM-J — bandwidth, formats, allocations

Completed 2026-08-30. One clause met and proved, two closed without a change
because the measurement did not support one.

`wgpu`'s internal counters are now on in every build, sampled every measured
frame, and reported as `alloc_churn_frames`, `alloc_worst_frame_delta`,
`churn_<object>` and a `live_*` inventory. The per-frame delta is what is
accumulated, because an endpoint comparison reports "nothing changed" for a
resource created every frame and destroyed the next.

On **Coastal** the criterion holds: one object moves on any churning frame, it
is always a buffer, and `SOMNIUM_ALLOC_TRACE=1` names it `(wgpu internal)
Staging`. On **Island** it does not: 100 of 300 frames churn, four move a texture
view and a bind group, and one frame moves 75 objects at once — five unrelated
per-frame labels halving and doubling together. Named, not attributed.

The inventory is 1901.7 MiB allocated of 2368.0 MiB reserved on Coastal, and its
largest row is the trade that buys the result: 384 MiB of fixed geometry pools
at roughly 3% occupancy, which cannot churn because they were never sized to the
scene. Recorded, not changed. No format changed either — the census has said
since DOOM-A that the frame is per-pixel shader cost at exactly one fragment per
pixel, not intermediate bandwidth.

Full inventory, the naming trace, and the bandwidth-counter gap stated as a gap:
[`DOOM-J.md`](DOOM-J.md).

---

# DOOM-K — fp16 in the terrain inner loop

Completed and **reverted** 2026-08-30, which is what the stage prescribed for a
result under 5%.

The thirty-two entry splat-weight array and the scan over it were compiled at
half precision behind a `//!if TERRAIN_F16` block — the first real use of
MORROWIND-C's define machinery, which worked as documented, including hoisting
`enable f16;` out of the guard. Two back-to-back repetitions on Coastal ground:

| rep | f32 Shading | f16 Shading | delta | verdict |
|---|---:|---:|---:|---|
| 1 | 11.9365 ± 0.4488 | 11.6165 ± 0.4392 | −0.320 ms | ~ noise |
| 2 | 11.4759 ± 0.3743 | 11.6319 ± 0.3170 | +0.156 ms | ~ noise |

**The sign flips between repetitions.** Reverted; the adapter capability probes
are kept, and this adapter reports both `SHADER_F16` and `SUBGROUP` available.

The record also carries a broken run and how it was caught — a 49% "win" that
was a lost `SOMNIUM_MAXIMIZE`, found by reading the `.somtime` header rather
than its numbers — and the naga trap that an alias inside a
`ptr<function, array<T, 32>>` parameter does not unify: [`DOOM-K.md`](DOOM-K.md).

---

# DOOM-L — subgroup operations

Completed and **reverted** 2026-08-30.

Most of the stage was picking the site. Two of the three the plan named do not
survive contact with the tree — the terrain strongest-four scan is per-lane with
nothing to reduce across lanes, and cluster assignment is CPU-side. Of the five
workgroup reductions actually in the shader tree, only auto exposure has a
spread tight enough to resolve a change (0.0455 ± 0.0013 ms); Hi-Z's σ is 91% of
its mean.

Its 256-wide tree — eight barriers, 255 shared read-modify-writes — became one
`subgroupAdd` per lane, one elected write, one barrier. Two back-to-back
repetitions: −0.0003 ms and −0.0011 ms, both inside the band. The mechanism says
why: `build_histogram` runs ~7 740 workgroups and `resolve_exposure` runs one.

**The finding that outlives the experiment is a toolchain one.** naga 30 rejects
`enable subgroups;` as unimplemented and does not know `subgroupElect`. The
hardware reports subgroups available at 32–32 lanes; the shader language cannot
say so. [`DOOM-L.md`](DOOM-L.md).

---

# DOOM-M — close-out

Phase DOOM closed 2026-08-30. Final runs on both shipped maps, all defaults,
1920×1032, 180/300: **Coastal 20.385 ± 1.968 ms** and **Island 13.179 ± 0.836
ms**, unattributed 0.6% and 1.1% against a 5% gate, with `Shadows` at 0.0023 ms
— DOOM-D still working at close-out.

**These do not compare to the DOOM-A baselines.** Those were taken at 2560×1392
and every continuation run is at 1920×1032, a 46% difference in pixel count on a
frame the census showed to be per-pixel cost. `38.392 → 20.385` is not a 47%
improvement and must not be quoted as one.

Of eleven stages: five produced instruments, three produced changes that hold,
three produced nulls kept as records rather than code. The kill-switch
inventory, the deletions deliberately not made, and seven named open threads:
[`DOOM-M.md`](DOOM-M.md).

---

# DOOM-B — the pixel census

`SOMNIUM_CENSUS=1` adds a compute pass that classifies every pixel of the
visibility buffer into the bins DOOM-C would dispatch separately, reducing
through group-shared counters so the global atomic traffic is one add per bin
per workgroup. It costs **0.083 ms** and does not move the frame
(38.328 vs 38.392 ms — inside the noise band).

`SOMNIUM_SHADE_ABLATE=sky|mesh|foliage|terrain` compiles a shading PSO that
returns black for every other class. The image is wrong on purpose; the point is
that the timer can then attribute cost to a class. Files:
`DOOM-B_coastal-ground.somtime`, `DOOM-B_coastal-overview.somtime`.

## Where the pixels are

2560×1392 = 3 563 520 pixels.

| bin | Coastal ground | | Coastal overview | |
|---|---:|---:|---:|---:|
| sky | 948 476 | 26.62% | 625 070 | 17.54% |
| mesh | 0 | 0.00% | 332 | 0.01% |
| foliage | 0 | 0.00% | 0 | 0.00% |
| terrain < 100 m | 2 275 352 | **63.85%** | 0 | 0.00% |
| terrain 100–400 m | 249 935 | 7.01% | 1 957 376 | **54.93%** |
| terrain > 400 m | 89 757 | 2.52% | 980 742 | 27.52% |
| **terrain, all** | **2 615 044** | **73.38%** | **2 938 118** | **82.45%** |

Caveat, stated because it will otherwise be misread: **water is not a bin.**
Water shades in its own pass and never writes the visibility buffer, so water
pixels are counted as whatever is behind them — usually `sky`. The `sky` row is
"pixels the shading pass treats as background", not "pixels that look like sky".

## What each class costs to execute

Shading ms, Coastal ground, one class at a time:

| ablation | Shading | pixels shaded |
|---|---:|---:|
| none (normal) | **25.672** | 3 563 520 |
| terrain only | 25.231 | 2 615 044 |
| sky only | 0.247 | 948 476 |
| mesh only | 0.172 | 0 |
| foliage only | 0.173 | 0 |

`mesh only` and `foliage only` shade **nothing** in this view, so 0.172 ms is the
floor: the cost of running the pass and returning black for all 3.5 million
pixels. Subtracting it:

- **terrain ≈ 25.06 ms** on 73.4% of pixels
- **sky ≈ 0.075 ms** on 26.6% of pixels
- the two together plus the floor come to 25.31 ms against a measured 25.67 —
  the remaining **≈ 0.36 ms** is the occupancy tax, which is what separate
  pipelines recover on top of the execution savings.

## The finding

**Terrain is 97.6% of the shading pass. Sky, at a quarter of the screen, is
0.3% of it.**

That reorders Phase DOOM, and it contradicts part of the plan's §1:

- **Binning alone is worth ≈ 0.4 ms, not 13.** On Coastal ground the whole
  prize for separating sky from terrain is the 0.075 ms of sky execution plus
  the 0.36 ms occupancy tax — about **1.5%** of the shading pass. On the
  overview it is ≈ 0.44 ms, about 2.9%. DOOM-C is still worth building, but as
  the *mechanism* that lets terrain pixels at different distances run different
  pipelines without the forbidden per-pixel branch — not as the headline win.
- **The lever is cost per terrain pixel**, and it is measurable:

| view | terrain px | terrain ms | **ns / terrain pixel** |
|---|---:|---:|---:|
| Coastal ground (64% under 100 m) | 2 615 044 | 25.06 | **9.58** |
| Coastal overview (0% under 100 m) | 2 938 118 | 14.33 | **4.88** |

A terrain pixel at walking height costs **1.96×** one seen from the overview,
on more pixels' worth of evidence than any previous comparison in this project.

> **Correction, added after DOOM-E ran.** This paragraph originally called that
> ratio "the size of the prize DOOM-E is chasing". It is not. The gap is
> `gpu_material_for_camera`'s existing 80 m aerial cut and the detail fade
> *already working* — realised, not unrealised. DOOM-E measured it: dropping hex
> and parallax past a distance changed 925 pixels out of 2 938 110 and cost
> 2.3 ms. See "DOOM-C and DOOM-E — two negative results" below. The error was
> comparing two viewpoints and attributing the difference to headroom.

- **Pixel count is the other lever, and it is linear.** Shading is exactly one
  fragment per pixel with no overdraw, so DOOM-F's resolution scale multiplies
  the 25.7 ms directly. At a 67% scale (0.45× the pixels) shading falls to about
  11.6 ms with nothing else changed.
- **Shadows are 0.958 ms**, not the several the plan assumed. DOOM-D's whole
  ceiling on this viewpoint is about 2.5% of the frame.

Recorded rather than acted on: `mesh` and `foliage` are empty in both Coastal
views, so neither says anything yet about a scene with characters or dense
vegetation in frame. Do not generalise the bin table beyond these two
viewpoints.

---

# DOOM-F — dynamic resolution

**Off by default**, on the Camera entity next to Frustum Cull: a
**Dynamic Resolution** checkbox, a **Target ms** field and a **Res floor %**
field. `SOMNIUM_DYNRES=1`, `SOMNIUM_DYNRES_TARGET_MS`, `SOMNIUM_DYNRES_FLOOR`
for headless runs.

Built because DOOM-B measured shading at exactly one fragment per pixel with no
overdraw, which makes pixel count a linear lever on the frame's dominant cost.
The controller reads the profiler's smoothed GPU `Frame` scope — **not** the CPU
frame delta, which `TimeState`'s hybrid limiter and vsync both pin near the
budget whatever the GPU is doing. That is a real dependency: with the profiler
unavailable the controller switches itself off and says so rather than sitting
silently at native.

Its shape is a ±10% dead band (the Coastal frame's own standard deviation is
2.5%), sixteenth-of-a-scale quantisation, one step per adjustment, and
asymmetric cooldowns — 15 frames down, 45 up, both measured against the
profiler's 30-frame smoothing window, because reacting faster than the number
finishes moving is how a resolution controller starts pumping.

## Measured, Coastal ground, maximized Native, target 16.67 ms

| | base | floor 67% | floor 45% |
|---|---:|---:|---:|
| render size | 2560×1392 | **1714×932** | **1600×870** |
| Frame | 38.392 | **19.937** | **17.690** |
| Shading | 25.769 | **11.329** | 9.923 |
| ReSTIR GI | 3.866 | 2.046 | — |
| GTAO | 1.464 | 0.649 | — |
| FSR | 1.444 | 0.998 | — |
| Shadows | 0.954 | 0.971 | — |

**Frame −48.1%, Shading −56.0%** at the 67% floor. DOOM-B predicted "about
11.6 ms at a 67% scale from the linear model"; the measurement is **11.329**.

`Shadows` is the control: +1.8%, inside the noise band. A shadow atlas does not
scale with the viewport, and a run where it *had* moved would have meant the
controller was changing something it should not.

The two runs demonstrate both halves of the contract:

- At a **67% floor** it settles on the floor at 19.94 ms — above the 16.67
  target, because no scale it is permitted to choose can reach it. Correct
  behaviour, and the floor is the user's to lower.
- At a **45% floor** it settles at 62.5% — *above* its floor — because 17.69 ms
  is already inside the dead band. It stops when it is close enough instead of
  chasing the last 6%.

## Fidelity

This is the one part of Phase DOOM that trades image quality for speed, which is
why it is opt-in with the floor on screen. The trade is not a new one: at the
67% floor it lands on 1714×932, within a few percent of the **1600×900** preset
the viewport toolbar has always offered, reconstructed by the same FSR 3 path.
No `capture.rs` gate applies — a scaled frame is *supposed* to differ.

## A defect the tests caught before it shipped (DOOM-F)

`it_settles_instead_of_pumping` failed on the first run: the controller settled
at scale 0.688 costing 22.18 ms against a 16.67 ms target and then refused to
move again. The guard rejecting changes smaller than half a step was eating the
final clamp onto `min_scale` — 0.67 sits between the sixteenths 0.625 and
0.6875, so the floor was never reachable in a whole step and every subsequent
adjustment was quantised to the same unreachable value. The guard is now a bare
inequality; quantisation and the one-step clamp already do the anti-jitter work
it was there for.

---

# DOOM-C and DOOM-E — two negative results

Both were built, both are correct, and **both are default off because they were
measured slower or invisible.** This section is the record of why, so nobody
spends another session rediscovering it.

## DOOM-C — tile-classified shading

`SOMNIUM_SHADE_BINS=1`, or **Post FX → Shade Bins**.

A compute pass (`pass/classify.rs`, `shaders/classify.wgsl`) splits the screen
into tiles, votes on each tile's class through group-shared memory, and appends
it to one of six bins with an indirect draw whose instance count is the bin's
tile count. The shading pass then draws one instanced quad per tile against a
pipeline compiled for exactly that bin — `ShadingSpec` moved from per-frame to
per-tile.

The plan called for compute dispatches, following Wicked Engine's
`visibility_shadeCS` and UE5's Nanite shade binning. Porting `fs_main` to
compute would have meant replacing every derivative-dependent intrinsic in a
1600-line shader — `textureSample` on four mesh maps, `textureSampleCompare` in
the PCF filter, `dpdx`/`dpdy` on terrain world position, `fwidth` in the star
field and the moon limb — so this drew instanced quads instead and kept the
fragment shader byte-for-byte.

**It is correct.** With the aerial split disabled so both paths run identical
code, the binned image matched the fullscreen one to **2 pixels out of
2 615 044** (`mean_abs` 0.0165).

**It is slower at every tile size:**

| tile | Shading, Coastal ground, maximized Native |
|---|---:|
| fullscreen triangle | **24.851 ms** |
| 8 px | 32.533 ms |
| 16 px | 27.820 ms |
| 32 px | 26.967 ms |
| 64 px | 26.131 ms |

Per-primitive setup falls monotonically as tiles grow and approaches the
fullscreen cost **from above** — it never crosses. Larger tiles simultaneously
make the classification worse by sending more tiles to `MIXED`. There is no tile
size at which this wins, and DOOM-B had already capped the whole prize at
~0.4 ms, so it could never have paid for a fraction of its own overhead.

**This is the answer to why the references use compute**, and it was not
obvious from reading them: a dispatch has no vertex shader, no primitive setup
and no rasterizer, so binning is free there and costs 1.3–7.7 ms here.

Getting DOOM-C to pay would mean the compute port after all. That is a real
option and the classifier is the half of it that already exists — but it is
chasing 0.4 ms, and the census says the money is elsewhere.

## DOOM-C's one lasting fix

The parity run started at **12 684** differing pixels, not 2. The cause was that
both vertex shaders produce the screen UV analytically but not to the last bit:
interpolating across a screen-sized triangle and across an 8-pixel quad give
answers a ULP apart, which is invisible everywhere except at a threshold — a mip
level, a hex-tile cell edge, a parallax step count.

`fs_main` now **derives** the UV from `clip_pos.xy`, which is the exact pixel
centre in every path, instead of taking it from the interpolator. That is a
correctness improvement independent of binning and it stays regardless of what
happens to the rest of DOOM-C.

## DOOM-E — the aerial terrain pipeline

**Terrain details → Aerial LOD**, with **Aerial dist m** and
**Aerial 16 layers**. `SOMNIUM_AERIAL=1`, `SOMNIUM_AERIAL_SPLIT`,
`SOMNIUM_AERIAL_HERO=1`.

Not tiles: two fullscreen draws of the same triangle emitted at the split's
clip-space depth, separated by a depth test against the visibility pass's own
depth buffer (`Greater` keeps the near half, `LessEqual` the far half plus sky).
Complete coverage, no overlap, and early-Z rejects the half each pipeline does
not own before a fragment runs. One large triangle each, so none of DOOM-C's
primitive overhead.

| config | Coastal ground | Coastal overview | terrain pixels changed |
|---|---:|---:|---:|
| off (baseline) | 25.040 | 15.187 | — |
| hex + parallax dropped | 24.981 | **17.482** | 925 of 2 938 110 |
| …and the layer scan cut to 16 | **24.231** | **14.137** | 1 067 237 of 2 938 110 |

**Dropping hex tiling and the parallax march buys nothing and costs 2.3 ms.**
925 changed pixels with a mean absolute difference of 0.71 is invisible — which
is the tell: `gpu_material_for_camera` already switches both off above 80 m, so
there was nothing left to delete and the second full-screen pass was pure
overhead.

**Cutting the layer scan to the hero bank is the only version that pays** —
−1.05 ms (−6.9%) on the overview, −0.81 ms (−3.2%) on the ground — and it is a
**real look change**: 36% of terrain pixels move, with a mean absolute
difference of 105. Coastal publishes 32 layers and the aerial pipeline would
stop reading half of them.

## The census reading that has to be corrected

DOOM-B's write-up said a terrain pixel costs 9.58 ns at walking height against
4.88 ns from the overview and called that 1.96× "the size of the prize DOOM-E is
chasing".

**That was wrong, and DOOM-E is what proved it.** The gap is not headroom; it is
`gpu_material_for_camera`'s existing 80 m aerial cut and the detail fade
*already working*. The engine was measured against itself and the win was
counted twice. Comparing two viewpoints and attributing the difference to
unrealised potential is the mistake, and it is worth naming because the census
is otherwise a good instrument and will be used again.

What is left after that correction: the per-terrain-pixel cost that distance
does **not** reduce — the 8 splatmap fetches, the 32-wide unpack and
strongest-four scan, and four layers of albedo + surface. Cutting the scan is
the only thing measured to move it, and it costs image quality. The designed
cheap path for that cost remains Phase DF's clipmap, which is still default off
and still gated on DF-E.

## Defaults after DOOM-C and DOOM-E

Verified by a full run against the pre-DOOM baseline: Frame 38.392 → 37.596,
Shading 25.769 → 24.934, **every zone reported `~ noise`**. Nothing in the
default path changed.

---

# DOOM-F follow-up — the controller oscillated

Reported as "the dynamic res sliders might not be doing anything". They were
doing something; the something was wrong.

Both fields reach the renderer. Verified through the environment path, which is
the same code the Details fields drive:

| floor | target | render size |
|---|---:|---|
| 50% | 8 ms | 1280×696 — exactly the floor |
| 85% | 8 ms | 2176×1182 — exactly the floor |

**But at a 33 ms target the controller flipped between two resolutions
forever:**

```text
frame_ms=37.62  →  step down to 2400x1304 (scale 0.9375)
frame_ms=28.99  →  step up   to 2560x1392 (scale 1.0)
…repeating every ~1.5 s
```

Neither decision is wrong in isolation. 37.62 is above the high band and 28.99
is below the low band. **No reachable scale lands inside the band**: one
sixteenth of scale moves this frame by about 23% while the dead band is ±10%.
On screen that reads as the resolution refusing to settle, which is what
"the slider does nothing" looks like from the outside.

## The wrong diagnosis, recorded because it cost a cycle

The first fix assumed 28.99 was a *stale* reading — the profiler's 30-frame
average still holding the previous resolution plus a resize transient — and
lengthened the down cooldown from 15 to 30 frames so a decision could not be
taken on a half-fresh window. A test was written to model a lagging average.

**The test passed before the change as well as after**, which is the tell that
the model was wrong, and running the engine confirmed it: with the longer
cooldown the oscillation was unchanged. 28.99 ms is simply what 2400×1304 costs.
No cooldown can fix a gap the quantisation cannot land inside — it only changes
how slowly the controller flips across it.

The longer cooldown and the settle-then-reset of the profiler window were both
kept: reacting faster than the instrument settles is still wrong, and the reset
is still correct. They were just not this bug.

## The fix

The controller remembers the lowest scale it has measured **over** budget and
refuses to climb back to it, so it settles on the safe side of a gap it cannot
straddle. The memory is dropped when a frame comes in under 70% of the target,
which means the scene got cheaper rather than the controller being wrong —
without that, walking out of an expensive view would pin the resolution low
forever.

Measured on Coastal ground, maximized Native, after the fix:

| target | scale changes before settling | settled at | frame |
|---|---:|---|---:|
| 33 ms | **1** | 2400×1304 | 29.56 |
| 25 ms | 3 | 2080×1130 | 22.64 |
| 16.67 ms | 6 | 1714×932 (the 67% floor) | 16.83 |

`it_does_not_oscillate_across_a_gap_no_scale_can_land_in` is built from the two
real measurements rather than invented ones, and it **fails with 129 direction
reversals** when the over-budget memory is removed.

Settling slightly below the budget rather than hunting for it is the intended
answer: a scale that oscillates is more objectionable than one a little lower
than strictly necessary.
