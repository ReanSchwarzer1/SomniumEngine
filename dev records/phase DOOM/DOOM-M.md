# DOOM-M — close-out

**Status:** phase DOOM closed, 2026-08-30.

## What the phase turned out to be about

It was planned as an optimisation phase: shade binning, a job system, fp16,
subgroups, format changes. Most of that did not survive measurement. What
survived is the measuring.

Of the eleven stages that ran, **five produced instruments** (A the clock, B the
census, I the hitch metric, J the allocation inventory, and the per-frame job
zones H needed), **three produced changes that hold** (D's shadow cache, H's
scheduler migration, I's startup fix), and **three produced null results kept as
records rather than code** (C's shade bins, G's counted submission, K's fp16 and
L's subgroups). E is a default-off measured result.

That ratio is the phase's actual finding, and it only exists because the
instruments came first. Every null above is a *number*, not an opinion.

## Closing measurements

Both shipped maps, all defaults, fixed camera and sun,
`SOMNIUM_TIME_STATIC=1`, 1920×1032, 180 warm-up / 300 measured frames, RTX 5080
Laptop / Vulkan.

| Zone | Coastal ground | Island ground |
|---|---:|---:|
| **Frame** | **20.385 ± 1.968 ms** | **13.179 ± 0.836 ms** |
| Shading | 11.611 ± 0.504 | 6.200 ± 0.247 |
| ReSTIR GI | 3.033 ± 1.765 | 1.381 ± 0.060 |
| Water prepass | 2.214 ± 0.693 | 2.239 ± 0.805 |
| GTAO | 0.814 ± 0.126 | 0.500 ± 0.027 |
| FSR | 0.806 ± 0.167 | 0.724 ± 0.013 |
| **Shadows** | **0.0023 ± 0.0001** | **0.0023 ± 0.0021** |
| **unattributed** | **0.6%** | **1.1%** |

The unattributed gate is < 5% and both maps meet it, which is the DOOM-A
criterion that made every other number in the phase readable.

**The `Shadows` row is DOOM-D still working at close-out**: 0.0023 ms on a
static scene, against 0.9633 ms with `SOMNIUM_SHADOW_CACHE=0`.

- [`DOOM-M_coastal-ground_final.somtime`](DOOM-M_coastal-ground_final.somtime)
- [`DOOM-M_island-ground_final.somtime`](DOOM-M_island-ground_final.somtime)

### These are not comparable to the DOOM-A baselines, and that must be said

DOOM-A's Coastal baseline was taken at **2560×1392**; every run in this
continuation is at **1920×1032**, because "maximized Native" resolved to a
different display between the two sessions. That is a 46% difference in pixel
count on a frame the census showed to be per-pixel cost — so
`38.392 → 20.385 ms` is **not** a 47% improvement and must never be quoted as
one.

The continuation is internally consistent: D, G, H, I, J, K, L and M all ran at
1920×1032 with the same camera, sun and static-scene flag, and every A/B in the
phase is a matched pair inside that set. The §9 budgets were written against
DOOM-A's resolution and are therefore not closed against here; closing them
needs a baseline re-taken on the current display, which is a session's work and
is left named rather than guessed at.

## Kill switches, as shipped

Every experiment in the phase is off unless asked for, and every instrument is
opt-in except the two that cost nothing.

| Switch | Stage | Default | What it does |
|---|---|---|---|
| `SOMNIUM_SHADOW_CACHE=0` | D | **on** | Restores the uncached four-cascade path |
| `SOMNIUM_DRAW_COMPACTION=1` | G | off | Counted `multi_draw_indirect_count` submission |
| `SOMNIUM_SHADE_BINS=1` | C | off | Per-tile shading specialisation |
| `SOMNIUM_SHADE_ABLATE=<n>` | B/C | off | Shade one pixel class, black the rest. A measuring instrument that makes the image wrong on purpose |
| `SOMNIUM_CENSUS=1` | B | off | The pixel-classification compute pass |
| `SOMNIUM_ALLOC_TRACE=1` | J | off | Name what churns, and log the memory inventory by label |
| `SOMNIUM_VOXEL=1` | H | off | Spawn the voxel terrain at map load so a timing run can see chunk streaming |
| `SOMNIUM_TIME_STATIC=1` | D | off | Suppress the demo boat when a gate needs a genuinely static scene |
| `SOMNIUM_DYNRES`, `_TARGET_MS`, `_FLOOR` | F | off | Startup overrides for dynamic resolution; the real control is on the Camera in Details |
| `SOMNIUM_TIME…` | A | off | The timing harness itself |

Two things are on in every build and stated as such: **the shadow cache**, and
**wgpu's `counters` feature**, whose cost is one relaxed atomic per resource
create and destroy — in steady state, the number it exists to prove is zero.

## Help

- `docs/editor/lighting.md` — the shadow cache, what invalidates a cascade, and
  the `shadow_cascades_rendered` counter to watch (D).
- `docs/editor/viewport.md` — dynamic resolution: where the control is, why it
  is off by default, the quality floor, the dead band, and why the step grid is
  coarse (F).

## Deletions not made, and why

**The fragment shading path stays.** The plan permits deleting it *"only once
DOOM-C has been default-on through a full session without a revert"*. DOOM-C is
default-**off** with a measured null, so the condition was never approached, and
deleting the path the engine actually ships would have been the opposite of what
that sentence protects.

**C, E and G stay in tree as default-off experiments.** Each has a mechanism
that a different scene might reward — G in particular is waiting on a scale axis
KENSHI may provide — and each costs a branch that is never taken.

**K and L were deleted.** Their sites cannot become winners: fp16's sign flipped
between repetitions, and L's reduction runs in one workgroup out of seven
thousand seven hundred and forty. A default-off path that can never win is not
an experiment, it is a second copy of a function that will drift from the first.

## Attribution

`ATTRIBUTION.md` §13C already carries the phase's reconnaissance — Wicked
Engine's tile binning, UE5's Nanite shade binning, The Forge's filtered
visibility buffer. The continuation added no external source: D's cascade
invalidation, H's migration, I's mean-albedo fix, J's counter reading and K/L's
reverted experiments are all against this tree and wgpu's own API. §13C is
unchanged and correct.

## What the phase leaves open

Named, with the evidence that names them, so none of it has to be rediscovered:

- **Island's steady-state allocation churn.** 100 of 300 frames, one moving 75
  objects at once, five per-frame labels halving and doubling together. Named by
  `SOMNIUM_ALLOC_TRACE=1`, not attributed to a call site (J).
- **No bytes-per-frame counter.** The allocation half of J was provable and the
  bandwidth half was not; no format changed because nothing measured asked for
  one (J).
- **`build_histogram`'s two million shared-memory atomics per frame.** The
  subgroup transform for it is a same-bin ballot aggregation, and the whole pass
  is 0.22% of the frame (L).
- **naga 30 does not implement `enable subgroups;` and has no `subgroupElect`.**
  The toolchain, not the hardware, is what currently blocks a default subgroup
  path (L).
- **Captures are not deterministic at frame 120.** Two runs of one unchanged
  build differ by 2.80% of pixels. Virtual-texture page admission is the obvious
  suspect and was not investigated; until it is, no capture in this directory is
  a fixed reference (I, K).
- **The §9 budgets need a baseline re-taken at 1920×1032** before the phase's
  targets can be closed against (M, above).
- **`ReSTIR GI` on Coastal is 3.033 ± 1.765 ms with a 12.73 ms maximum.** The
  same instability DOOM-A recorded on the overview, still unexplained, still not
  scheduled.
