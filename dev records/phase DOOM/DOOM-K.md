# DOOM-K — fp16 in the terrain inner loop

**Status:** complete and **reverted**, 2026-08-30. The numbers are the
deliverable.

## The stage, and its own prediction

> **Exit:** kept **only** if it wins ≥5% on the dev GPU with a byte-comparable
> capture. Reverted otherwise, with the numbers recorded — 5.9's
> counter-evidence (FP32 23% faster on an RTX 3080 Mobile) makes a null result
> the expected outcome, and recording it is the deliverable either way.

It was a null result. It is recorded.

## What was converted, and why that region

`terrain_material.wgsl`'s inner loop is a thirty-two entry weight array built
per terrain pixel, normalised, and scanned once for the strongest four layers.
Half precision halves that array's footprint, which is the *only* mechanism by
which f16 could win here — the arithmetic is a handful of adds and compares, so
this is an occupancy experiment, not a rate one.

Splat weights are normalised to sum to one, so f16's three-ish decimal digits
are far more precision than a blend weight needs. What was at risk was the tie
break: two weights differing in the f32 mantissa can compare equal in f16 and
swap which layer wins.

## Two things the implementation established

**It is a compiled variant, not a uniform.** The measurement contract is
explicit that *"toggling a runtime uniform is not an occupancy experiment; only
a different compiled pipeline is"*, and f16 could not have been a uniform
anyway — `enable f16;` is file-scoped and the types differ. The variant is a
`//!if TERRAIN_F16` block resolved by `somnium_shader`.

**This was the first real use of MORROWIND-C's define machinery**, whose stated
exit criterion was that adding a define takes no edit to `renderer.rs`. It
didn't: the change was one entry in `shaders.rs`, the guarded block in the
`.wgsl`, and a variant selector in the shading pass. `enable` hoisting worked as
documented — the resolver lifted `enable f16;` out of the guarded block to the
top of the composed module.

**One trap, worth leaving behind for the next person.** Sharing the scan between
the two branches through `alias TerrainWeight = f32;` compiles, and then naga
rejects the call:

```text
Shader validation error: Function [5] 'rt_terrain_albedo' is invalid
  60 │     let selected = terrain_strongest_four(&weight);
     │                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ invalid function call
   = Argument 0 value [40] doesn't match the type [23]
```

An alias inside a `ptr<function, array<T, 32>>` parameter does not unify with
the array type the caller holds. Each branch has to carry its own concrete
signature.

## Measurement

Coastal ground — the terrain-heavy view, where shading is the dominant zone —
fixed camera and sun, `SOMNIUM_TIME_STATIC=1`, 1920×1032, 180 warm-up / 300
measured, RTX 5080 Laptop / Vulkan. Two repetitions **back to back**, because
between-session drift on this hardware is larger than within-run spread and the
effect being looked for is a few tenths of a millisecond.

| | f32 Shading | f16 Shading | delta | band | verdict |
|---|---:|---:|---:|---:|---|
| rep 1 | 11.9365 ± 0.4488 | 11.6165 ± 0.4392 | **−0.320 ms (−2.7%)** | ±0.628 | ~ noise |
| rep 2 | 11.4759 ± 0.3743 | 11.6319 ± 0.3170 | **+0.156 ms (+1.4%)** | ±0.490 | ~ noise |

**The sign is not stable between repetitions.** Both deltas are inside their
noise band, and neither is within sight of the 5% gate. GPU frame time moved
20.668 → 20.220 ms in rep 1 and 20.119 → 20.109 ms in rep 2, both `~ noise`.

- [`DOOM-K_coastal-ground_f32.somtime`](DOOM-K_coastal-ground_f32.somtime)
- [`DOOM-K_coastal-ground_f16.somtime`](DOOM-K_coastal-ground_f16.somtime)

## The run that was thrown away, and why

The first f16 run reported **Shading 11.94 → 3.91 ms and frame time −49.1%**.
That is not a result, it is a broken measurement, and it is recorded because the
way it was caught is worth more than the way it was made.

Environment variables do not persist between shell invocations here. The f16 run
inherited only the four variables set in its own command, so it lost
`SOMNIUM_MAXIMIZE`, the view, the static-scene flag and the fixed sun — and
rendered a different scene at a different resolution. **The `.somtime` header
said so plainly:**

```text
f32 baseline:  # render 1920x1032   draw_calls 66   Shading.frag 1981440
f16 first run: # render 1280x720    draw_calls 195  Shading.frag  921600
```

A 49% "win" that also halves the fragment count and triples the draw calls is
not a win. **Read the header before the numbers** — the resolution, the draw
count and the fragment count are the control, and a comparison whose controls
moved is not a comparison. The same audit found two *committed* Island runs
taken at the wrong resolution; both were re-run and their records corrected.

## Correctness, and the gate that could not be used

The stage asks for a "byte-comparable capture". **That gate is not available on
this harness**, and DOOM-I is why: two runs of one unchanged build differ at
frame 120 by 2.80% of pixels with a peak channel delta of 53. A capture
comparison cannot certify a change smaller than that, and the f16 tie-break risk
is exactly the kind of small, localised change it would hide.

Since the experiment was reverted, no correctness claim is needed and none is
made. Had it won, this gate would have had to be replaced before it could be
kept — which is itself worth recording, because the next precision experiment
will meet the same wall.

## Decision

**Reverted**, per the stage's own rule. The guarded block, the `TERRAIN_F16`
define and the variant selector are gone; two copies of a thirty-line scan that
can drift are not worth keeping for an experiment whose sign flips between
repetitions.

**Kept:** the adapter capability probes. `SHADER_F16_FEATURES` and
`SUBGROUP_FEATURES` are detected, requested when present, and logged either way,
following the same "detect, do not demand" pattern as ray tracing, timestamps
and BC compression. An experiment that cannot say whether the hardware would
even allow it has not started, and DOOM-L needs the sibling probe. This adapter
reports both available.

## Commands

```bash
SOMNIUM_TERRAIN_F16=1 SOMNIUM_TIME_STATIC=1 SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=300 SOMNIUM_MAXIMIZE=1 SOMNIUM_SUN_ELEVATION=45 SOMNIUM_SUN_AZIMUTH=120 SOMNIUM_TIME=f16.somtime SOMNIUM_TIME_COMPARE=f32.somtime cargo run --release -p hello_engine
```

The variable no longer does anything; the command is kept so the experiment can
be rebuilt from this record rather than rediscovered.
