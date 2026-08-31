# DOOM-L — subgroup operations

**Status:** complete and **reverted**, 2026-08-30. Two findings: a null on the
one site the instrument can resolve, and a toolchain limit that blocks the whole
technique regardless of hardware.

## Picking the site, which is most of the stage

> **Exit:** each site individually A/B'd; keep only the winners.

The 2026-08-16 stage named three candidates — *the classify reduction, the
terrain strongest-four scan, cluster assignment*. §16's rebase narrowed it:
*"Restrict the experiment to a reduction/shuffle with an existing scalar path
and measure occupancy/timing."*

Two of the three do not survive contact with the tree:

- **The terrain strongest-four scan is not a cross-lane problem.** Every lane
  scans its own thirty-two weights for its own pixel. There is nothing to
  reduce *across* lanes, so there is no subgroup operation to apply.
- **Cluster assignment is CPU-side.** `cpu Cluster cull` is 0.0167 ms of the
  frame; subgroups are a GPU instruction set.

Every workgroup-shared reduction actually in the shader tree, with the measured
cost of the pass containing it (Coastal ground, 180/300, 1920×1032):

| Shader | Pass | Cost ± σ | Verdict |
|---|---|---:|---|
| `auto_exposure.wgsl` | Auto exposure | **0.0455 ± 0.0013 ms** | **The only one measurable.** σ is 3% of the mean |
| `spd.wgsl` | Hi-Z | 0.0718 ± 0.0655 ms | σ is 91% of the mean — nothing under a doubling is resolvable |
| `water_spectrum.wgsl` | Water prepass | 2.1772 ± 0.7113 ms | σ is 33% of the mean |
| `census.wgsl` | Census | 0.0021 ms | DOOM-B's instrument, default off |
| `classify.wgsl` | — | — | DOOM-C, default off, already a measured null |

That table *is* the "where measured" the stage asked for. Only auto exposure has
both a real reduction and a spread tight enough to see a change through.

## What was built

`resolve_exposure` reduces 256 histogram bins to two sums with the canonical
tree: eight `workgroupBarrier`s and 255 shared-memory read-modify-writes. The
subgroup form is one `subgroupAdd` per lane with no shared memory and no
barrier, one elected write per subgroup, a single barrier, and a sum over eight
partials.

A `//!if SUBGROUP_REDUCE` variant, selected only when `SOMNIUM_SUBGROUP=1` *and*
the adapter reports `Features::SUBGROUP` — a compiled pipeline, not a uniform,
for the same reason DOOM-K's was.

Floating-point addition is not associative, so "the same result" needed stating
precisely: the tree sums in a fixed pairwise order and the subgroup path sums in
the hardware's order. The terms are `f32(count) * f32(bin)` — at most 256, all
non-negative, within a few orders of magnitude — and the result is divided,
`log2`'d and exponentially smoothed toward, so the two orders agree far inside
the precision that survives to the screen.

## Two things naga 30 will not do

**`enable subgroups;` is rejected outright:**

```text
Shader 'auto_exposure.wgsl' parsing error: the `subgroups` enable-extension is not yet supported
1 │ enable subgroups;
  │        ^^^^^^^^^ this enable-extension specifies standard functionality which is not yet implemented in Naga
```

The intrinsics work *without* the directive — they are gated on the device's
`Features::SUBGROUP` instead — so the guard has to live in Rust rather than in
the shader. That is the opposite of how WGSL is specified to work, and it means
the shader text alone cannot state its own requirement.

**`subgroupElect` does not exist.** naga 30's WGSL front end knows
`subgroupAdd`, `subgroupBallot`, `subgroupBroadcastFirst`, `subgroupShuffle`,
`subgroupInclusiveAdd` and friends, but not `subgroupElect`. With a fully
occupied workgroup and uniform control flow, `@builtin(subgroup_invocation_id)
== 0` is the same thing — but only under those conditions, which a future site
would have to re-establish for itself.

**This is the finding that outlives the experiment.** §16 required that *"no
default path may depend on it without a portable fallback"*. The toolchain is a
harder constraint than the hardware: this adapter reports `SUBGROUP` available
and 32–32 lane widths, and the shader language still cannot say so.

## Measurement

Coastal ground, fixed camera and sun, `SOMNIUM_TIME_STATIC=1`, 1920×1032,
180 warm-up / 300 measured, two back-to-back repetitions.

| | workgroup tree | subgroup | delta | band | verdict |
|---|---:|---:|---:|---:|---|
| rep 1 | 0.0449 ± 0.0014 ms | 0.0446 ± 0.0014 ms | −0.0003 ms | ±0.0020 | ~ noise |
| rep 2 | 0.0459 ± 0.0012 ms | 0.0448 ± 0.0015 ms | −0.0011 ms | ±0.0019 | ~ noise |

Both deltas are in the faster direction and both are inside the band. The larger
of them is **0.0011 ms against a 20 ms frame — 0.005%.**

The mechanism explains the size. `build_histogram` runs one workgroup per 256
pixels: about **7 740** workgroups on this frame. `resolve_exposure` runs
**one**. Removing seven barriers from one workgroup out of seven thousand seven
hundred and forty cannot show up in the pass, let alone the frame, and the
measurement agrees.

- [`DOOM-L_coastal-ground_workgroup.somtime`](DOOM-L_coastal-ground_workgroup.somtime)
- [`DOOM-L_coastal-ground_subgroup.somtime`](DOOM-L_coastal-ground_subgroup.somtime)

## Correctness

Tone-mapped frame-120 captures differ by **1.92% of pixels, peak channel delta
58, mean channel delta 0.0909**, against a same-build run-to-run baseline of
2.80% and peak 53. The difference is inside the instrument's own variance.

Unlike DOOM-K, the capture *is* adequate evidence here, and it is worth saying
why: a wrong reduction would produce a wrong exposure, and exposure multiplies
the entire image. The failure mode is a frame that is visibly too bright or too
dark — a mean channel delta in the tens, not 0.09. The check only had to
distinguish "the same picture" from "a differently exposed picture", and it can.

- [`DOOM-L_coastal-ground_workgroup.png`](DOOM-L_coastal-ground_workgroup.png)
- [`DOOM-L_coastal-ground_subgroup.png`](DOOM-L_coastal-ground_subgroup.png)

## Decision

**Reverted**, per *"keep only the winners"*. A default-off path at a site that
is 0.22% of the frame cannot become a winner later — no scale axis changes the
fact that the reduction runs in one workgroup — so unlike DOOM-G's counted
submission there is nothing to keep it for, and a second copy of a reduction is
a thing that drifts.

**Kept:** the `SUBGROUP_FEATURES` capability probe added alongside DOOM-K's.

**The site with the actual cost was not attempted, and that is deliberate.**
`build_histogram` performs one shared-memory atomic per pixel — about two
million per frame — and the subgroup transform for it is a same-bin ballot
aggregation: combine the lanes of a subgroup that landed in the same histogram
bin, then issue one atomic per distinct bin. That is a genuine optimisation with
a real mechanism, and it is bounded above by the whole pass: **0.0455 ms, or
0.22% of the frame.** The stage says keep only measured winners, and a win that
cannot exceed a fifth of a percent of the frame does not justify the
uniform-control-flow hazards of a ballot loop written against a front end that
does not implement the extension it belongs to.

## Commands

```bash
SOMNIUM_SUBGROUP=1 SOMNIUM_TIME_STATIC=1 SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=300 SOMNIUM_MAXIMIZE=1 SOMNIUM_SUN_ELEVATION=45 SOMNIUM_SUN_AZIMUTH=120 SOMNIUM_TIME=subgroup.somtime SOMNIUM_TIME_COMPARE=workgroup.somtime cargo run --release -p hello_engine
```

The variable no longer does anything; the command is kept so the experiment can
be rebuilt from this record rather than rediscovered.
