# MORROWIND-E2b — GHOSTFENCE's golden row, taken

**Complete, 2026-08-25.** Track 0 / the gate. Not a sub-phase in the plan; the
plan assumes this exists, and three sub-phases have now been blocked on the fact
that it did not.

## Why this is worth its own record

`MORROWIND-A` built the golden-image machinery — a standard-library PNG codec, a
two-sided perceptual threshold, a diff writer — and then could not take a
reference, because taking one needs a GPU and a windowed run. The row has read
`SKIP` ever since:

```
SKIP  golden-images  no reference set yet - capture one with
      `SOMNIUM_CAPTURE_UI_PNG=<path> SOMNIUM_CAPTURE_FRAME=120
       SOMNIUM_CAPTURE_QUIT=1 cargo run -p hello_engine`
```

Three things were waiting on it:

1. **MORROWIND-G's shaper.** `cosmic-text` is chosen and sitting behind
   `SOMNIUM_UI_SHAPER`, default off, because Appendix A.5 requires the
   block-origin snapping rule to be A/B'd against a reference image and there
   was none. **The shaper is the largest single piece of unfinished Track 1
   work and this is its only blocker.**
2. **MORROWIND-D's paint contract.** GHOSTFENCE row 1 asserts instance *counts*
   are byte-identical. Counts do not catch a glyph that moved half a pixel.
3. **Every Track 7 sub-phase**, whose §8 acceptance is *".somtime on both maps,
   stddev reported, plus the golden image"* (Appendix A.7).

## The thing that had to be solved first

The capture is the whole swapchain **after** the UI pass — which is what makes
it evidence for the paint layer, and what makes whole-image comparison
worthless. It contains:

- a **ReSTIR-lit viewport**, stochastic by construction;
- an **fps counter**, twice (title bar and status bar);
- whatever **toast** happened to be up — the reference run had *"Unsaved work
  was recovered (autosave)"*, which depends on files in the content root.

A threshold cannot express *"ignore the viewport"*. So a golden entry now names
the chrome it is evidence **for**:

```json
{ "name": "menu-bar", "region": { "x": 0, "y": 0, "w": 430, "h": 32 } }
```

`golden.Region` crops both images before comparing, clamps to the capture rather
than crashing on a mis-typed rectangle, and sizes the diff image to the region
so a failure opens on the thing that changed.

## The three regions, and why each

| Region | What it covers | Why it earns its place |
|---|---|---|
| `menu-bar` 0,0 430x32 | logo, wordmark, six menu labels | Pure text on a flat fill — **the most glyph-sensitive area in the shell, and therefore the region that decides the shaper A/B**. Stops well left of the command palette, whose focus ring is not load-deterministic. |
| `sculpt-panel` 0,70 168x200 | six tool rows | The Phase 27 paint contract's rounded / washed / lifted primitives at shipping size, with an SDF icon and a label per row. |
| `toolbar` 0,38 540x30 | Save / Select / Landscape / Foliage + play glyphs | Icon-and-label pairs at toolbar size, where half a pixel of *icon* drift is visible and half a pixel of *text* drift is not — the two failure modes block-origin snapping trades between. |

Threshold: `±2` per channel, `0.2%` of pixels allowed to exceed it, `24` hard
ceiling. **Not byte-identity** — that is the right bar for a draw list, which
row 1 already asserts separately, and the wrong bar for a rasterised image where
a driver update can legitimately move a subpixel.

## Verified across two independent runs

The reference and the candidate are separate `cargo run` invocations, separate
processes, separate GPU submissions:

```
PASS  golden-images  3 image(s) within threshold
```

Measured, incidentally, while choosing the regions: the viewport region differs
by a peak of **6** across the two runs and the fps region by **0**. The scene is
more deterministic at frame 120 than assumed — ReSTIR has converged and the
frame rate happened to round the same. **The regions still exclude both**,
because "it was stable twice on this GPU" is not a contract, and a golden row
that fails on a driver update for a reason unrelated to the UI trains people to
ignore it.

## The gate can fail — asserted, not assumed

Three tests in `tools/ghostfence/test_ghostfence.py`, because a green row proves
nothing on its own:

- **a region ignores drift outside it** — half the image repainted white; the
  whole-image compare fails, the untouched half passes, and the region's pixel
  count actually shrank (a region that silently compared everything would pass
  this test without the last assertion).
- **a region still catches drift inside it** — sixteen changed pixels out of
  256, inside the region, fails; the same change outside it does not.
- **a region larger than the capture clamps** rather than crashing.

Nine tests in the file, 0 failures.

## One command, not six environment variables

```bash
python tools/ghostfence/capture.py              # candidate
python tools/ghostfence/capture.py --reference  # approve a new reference
```

`--reference` overwrites checked-in evidence, so it prints what it is about to
replace and refuses without `--yes`. A gate whose reference cannot be
regenerated in one command is a gate that is stale within a month, and the old
SKIP message asked a human to remember six things in the right order — which is
the same as asking them not to run it.

Frame 120 rather than frame 1: the shell has settled, thumbnails have decoded,
and every Phase 27 motion track has finished (`MAX_DURATION_MS` is 200 ms, so
anything started at load is long done).

## Files

```
+ dev records/phase MORROWIND/golden/editor_shell_1280x720.png   the reference
+ dev records/phase MORROWIND/golden/manifest.json               three regions, each with its reason
+ tools/ghostfence/capture.py                                    one command
~ tools/ghostfence/golden.py                                     Region, and region-aware compare + diff
~ tools/ghostfence/run.py                                        region in the manifest schema; a SKIP message that names the script
~ tools/ghostfence/test_ghostfence.py                            3 region tests
```

## What is now unblocked

**MORROWIND-G's shaper.** Land `cosmic-text` behind the flag, capture a
candidate with `SOMNIUM_UI_SHAPER=1`, and compare against `menu-bar`. The
A/B that Appendix A.5 asked for is now a command rather than an argument.
