# MORROWIND-AC — evidence

Produced 2026-08-29. Record: [`MORROWIND-AC.md`](../MORROWIND-AC.md).

RTX 5080 Laptop / Vulkan, driver 610.74, render **1920×1032** (maximised,
`SOMNIUM_VIEWPORT_RES=2`), release, **180 warm-up / 300 measured**.

| File | What it is |
|---|---|
| `AC_coastal-ground_aa-<mode>.somtime` | one run per `AntiAliasing` value. The table these produce is the proof that every authored value runs a pass and `off` runs none — which is the defect this sub-phase opened by fixing. |
| `AC_coastal-ground_smaa-<preset>.somtime` | the four SMAA quality presets |
| `AC_island_oit-{off,on}.somtime` | weighted-blended OIT against the sorted path |
| `AC_coastal-ground_aa-<mode>.png` | display-referred captures, after tone map and before editor chrome |
| `AC_island_oit-{off,on}.png` | as above |
| `AC_*_control.png` | **a second capture with settings identical to the `off` one.** These exist to measure how much the image moves for no reason at all, and they are the most important files here — see below. |

## Reproduce

```bash
SOMNIUM_AA=smaa SOMNIUM_SMAA_PRESET=ultra \
SOMNIUM_TIME="dev records/phase MORROWIND/AC/x.somtime" \
SOMNIUM_TIME_VIEW=coastal-ground SOMNIUM_VIEWPORT_RES=2 SOMNIUM_MAXIMIZE=1 \
SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=300 SOMNIUM_TIME_QUIT=1 \
cargo run --release -p hello_engine
```

`SOMNIUM_AA` takes `off|fxaa|smaa|smaa_t2x|taa|fsr`; `SOMNIUM_OIT=1` turns OIT
on. Both are Seam 4 overrides of a real authored field, not harness-only knobs.

## The captures cannot resolve anti-aliasing, and here is the proof

Every scene in this repository contains stochastic and animated passes — ReSTIR
GI, the FFT ocean, clouds. Two runs of the *same build with the same settings*
do not produce the same frame 240. Measured, at a channel tolerance of 2 over
every second pixel:

| Comparison | px changed | mean delta |
|---|---:|---:|
| **coastal-ground, `off` vs `off`** | **4.48%** | **0.683** |
| coastal-ground, `off` vs SMAA 1x | 4.43% | 0.591 |
| coastal-ground, `off` vs FXAA | 10.07% | 0.964 |
| **island, `off` vs `off`** | **63.50%** | **14.955** |
| island, sorted vs OIT | 63.49% | 14.955 |

Read the bold rows first. **The SMAA difference is smaller than the noise floor**
of its own scene, and the OIT difference is indistinguishable from it to three
decimal places — 314,515 pixels against a control of 314,536.

So: these PNGs are evidence that the passes **run**, and evidence of nothing
else. They do not show that SMAA improves an edge or that OIT fixes an
intersection. FXAA is the only one that clears its scene's floor, and only by
about 2×.

A visual gate for either feature needs a **deterministic** fixture — the
stochastic passes off, still water, no clouds — and that is owed alongside the
transparency content in the record's §7. Quoting any number from this table as
a quality result would be exactly the mistake `capture.rs`'s own header
describes: a metric that varied "from 0.776 to 2.018 across three runs of one
identical build".

## What these files cannot tell you

**OIT's cost.** The `oit-on` run measures +0.040 ms on the `Transparent` zone,
and essentially all of that is the fixed overhead — a two-target clear plus a
fullscreen resolve. PORTAL-0 measured the whole transparent pass at 0.004–0.017
ms across all four canonical viewpoints, because **neither shipped map contains
meaningful transparency**. The cost that scales with depth complexity, and the
image-quality claim that is the entire reason to prefer OIT, both need a fixture
with intersecting transparent surfaces. That fixture does not exist yet and is
owed; see the record's §7.

Per `dev records/phase PORTAL-0/README.md`: a `.somtime` σ is a within-run
number. The SMAA preset ladder here is monotonic across four runs, but the
gaps between adjacent presets are inside the run-to-run band — read the ladder,
not any single pair.
