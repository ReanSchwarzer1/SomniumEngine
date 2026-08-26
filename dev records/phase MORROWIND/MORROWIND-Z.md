# MORROWIND-Z — virtual shadow maps

**Complete, 2026-08-26.** Track 7 (RED MOUNTAIN), after MORROWIND-A2.

## Shipped path

Somnium now ships a sparse directional virtual-shadow path beside the existing
CSM path:

- a stable virtual address over four 16,384-texel clip levels, split into
  128-texel pages;
- demand from visible opaque, terrain and water receivers, with neighbourhood
  expansion, a bounded 64-page frame budget and deterministic LRU allocation;
- a persistent 1,024-page physical depth atlas and shader-visible page table;
- light/caster revision invalidation, cache hits for unchanged pages, and
  coarser-parent lookup when a fine page is missing;
- per-page light projection and scissored physical-tile raster, retaining
  untouched cached depth;
- exact page-table sampling in opaque/terrain and water shading, with an
  explicit CSM fallback for missing pages or unavailable resources;
- `VSM Pages`, scheduled-page and resident-page profiler/timing evidence.

`Light > Shadows > Technique` is an editable generated-Details enum. Cascaded
and Virtual survive snapshot/scene round trips. GPU resources are allocated
lazily only after an authored light requests Virtual. The unattended
`SOMNIUM_VIRTUAL_SHADOWS=1` switch is classified as a capture/timing harness;
it overrides the effective demo sun without replacing the authored Details
route.

The CSM-only small-caster threshold is disabled for effective VSM, because
page caching—not projected-radius rejection—owns that cost. VSM demand includes
visible receivers even when they do not themselves cast shadows.

## Measured default

Both shipped maps were measured at 1280x720 on an NVIDIA GeForce RTX 5080
Laptop GPU / Vulkan, driver 610.74. Each row contains 120 samples after 30
warm-up frames and reports standard deviation.

| Map/view | CSM frame | VSM frame | VSM page pass | Physical pages |
|---|---:|---:|---:|---:|
| Coastal / ground | 10.1395 ± 0.8451 ms | 12.4056 ± 0.6212 ms | 1.8868 ± 0.1907 ms | 21 scheduled / 21 resident |
| Island | 8.9626 ± 1.0720 ms | 9.6246 ± 0.4988 ms | 1.3734 ± 0.1427 ms | 45 scheduled / 53 resident |

CSM therefore remains the measured default for these two small shipped scenes.
Virtual remains a per-light quality/scaling choice; it is not silently enabled
when its additional page pass loses on frame cost.

Canonical evidence is committed beside this record:

- `MORROWIND-Z_coastal-ground_{csm,vsm}.somtime` and matching display-referred
  PNG captures;
- `MORROWIND-Z_island_{csm,vsm}.somtime` and matching display-referred PNG
  captures.

The matched captures show stable terrain/water presentation with no missing-page
black regions or atlas-edge seams. They are captured after tonemapping.

## Verification

- `somnium_renderer`: **371/371** library tests passed.
- Sparse VSM allocator/cache/clipmap suite: **10/10** passed after the final
  receiver-demand and caster-policy fixes.
- Core generated-Details enum and scene-roundtrip regressions passed.
- `cargo check -p hello_engine` and `cargo build -p hello_engine` passed.
- Four windowed timing runs and four matched capture runs completed on the
  adapter named above; both VSM rows contain a non-zero `VSM Pages` scope and
  non-zero physical-page counters.
- GHOSTFENCE's full workspace row passed **1,859 tests, 0 failed**. After
  regenerating the source census, the fast structural rerun reported **5
  passed, 0 failed**; only the pre-existing editor-shell golden candidate and
  intentionally skipped duplicate test row were skipped.

## Reference boundary

Implementation followed J. Stephano's public *Sparse Virtual Shadow Maps*
write-up (`https://ktstephano.github.io/rendering/stratusgfx/svsm`) for the
sparse-page/frame-marker model. Unreal's VSM source was architecture-only under
its proprietary licence. A fresh search on 2026-08-25 again found no mature
production Rust/wgpu VSM implementation to adapt. Clean-room details are in
`ATTRIBUTION.md` §13H.24.
