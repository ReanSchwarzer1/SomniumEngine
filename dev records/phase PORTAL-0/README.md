# Phase PORTAL-0 — evidence

Produced 2026-08-29 against `dev` at `439b6b6`. Record: [`phase_PORTAL-0.md`](../phase_PORTAL-0.md).

Hardware for every file here: **NVIDIA GeForce RTX 5080 Laptop GPU / Vulkan,
driver 610.74**, render **1920×1032** (maximised, `SOMNIUM_VIEWPORT_RES=2`)
unless the filename says `_720`, release profile, **180 warm-up / 300 measured**.
A run on different hardware is a different measurement and belongs in its own
file, not silently replacing one of these.

| File | What it is |
|---|---|
| `PORTAL-0-A_<view>.somtime` | the baseline matrix — four canonical views |
| `PORTAL-0-A_coastal-ground_2560.somtime` | **kept as a negative result.** `SOMNIUM_VIEWPORT_RES=1` is a *cap*; the window is 1920 wide, so this is byte-identical in size to the 1920 run and is not a second resolution |
| `PORTAL-0-A_<view>_720.somtime` | 1280×688, the second resolution that does bind |
| `PORTAL-0-A_ablate-{sky,mesh,foliage,terrain}.somtime` | `SOMNIUM_SHADE_ABLATE`; terrain is 100.0% of Shading |
| `PORTAL-0-F_<view>_clipmap.somtime` | `SOMNIUM_TERRAIN_CLIPMAP=1`, everything else identical |
| `PORTAL-0-F_<view>_clipmap-{off,on}.png` | display-referred captures for DF §6.4's luminance gate |
| `PORTAL-0-after_<view>.somtime` | after B/C/D **and G**; superseded, kept because §G's reversal is argued from it |
| `PORTAL-0-final_<view>.somtime` | the shipped tree: B + C + D, G reverted. **These are the rows to compare against.** |

## Reproduce

```bash
SOMNIUM_TIME="dev records/phase PORTAL-0/x.somtime" \
SOMNIUM_TIME_LABEL="what this run is" \
SOMNIUM_TIME_VIEW=coastal-ground \
SOMNIUM_VIEWPORT_RES=2 SOMNIUM_MAXIMIZE=1 \
SOMNIUM_TIME_WARMUP=180 SOMNIUM_TIME_FRAMES=300 SOMNIUM_TIME_QUIT=1 \
cargo run --release -p hello_engine
```

## The one rule these files are here to enforce

**A `.somtime` standard deviation is a within-run number.** `coastal-ground`
`Shading` measured 11.463 ms in one session and 11.703–12.239 ms in another on
identical code, against a within-run σ of 0.47. Comparing two runs taken at
different times through that band will call drift a result. Anything decided on
a difference smaller than about 1 ms needs repetitions taken **back to back**,
which is how §G was settled.

A short warm-up produces the same class of lie in the other direction:
MORROWIND-AB's 20-frame runs reported `cpu Terrain` at 1.39 ms where 180 frames
report 0.031 ms. See `phase_PORTAL-0.md` §E.
