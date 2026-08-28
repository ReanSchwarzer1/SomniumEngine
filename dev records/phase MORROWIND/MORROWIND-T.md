# MORROWIND-T — HLOD, impostors and floating origin

**Complete, 2026-08-25.** Track 4 (SILT STRIDER) closure, after MORROWIND-S.

`somnium_asset::world_bake::bake_hlod` transforms and merges a cell's proxy
geometry, validates triangle indices, normalizes transformed normals, applies a
deterministic triangle budget, area-weights a merged base colour and retains
sorted/deduplicated source `AssetId`s. The result is versioned cook data ready
to be indexed as S's derived cell content.

`bake_impostor` accepts offline square RGBA captures, encodes their directions
onto a signed octahedron, rejects duplicate or malformed views and packs a
deterministic square atlas independent of request order. Runtime selection sees
only the atlas and direction keys; capture remains offline.

## Floating-origin decision

Somnium selects the CPU integer-grid-plus-local-float design. `GlobalPosition`
stores exact i64 cells and a small f32 metre offset. `FloatingOrigin` rebases at
cell boundaries and produces camera-relative f32 positions without ever first
forming a huge f32 world coordinate. Authored positions are unchanged and the
operation is reversible. Tests retain centimetre differences around
10,000,000 metres.

Shader soft-double is declined for the current engine. It would spread special
arithmetic through shaders and solves planet-scale rendering that neither
shipped map requires. It remains a future tier rather than an irreversible
foundation.

## Verification

- Two HLOD/impostor unit tests pass: transformed merge/budget/material/dependency
  behavior and order-independent complete atlas packing.
- Two floating-origin tests pass: centimetre preservation at extreme distance
  and cell-boundary rebasing without authored-data mutation.
- Track 4's S gate already proves 100 unload/reload loops return entity count
  exactly; Q/R prove deterministic cook and budgeted residency.
- GHOSTFENCE: **7/7 rows passed**, including **1,835 tests passed, 0
  failed** and all 3 registered golden images within threshold.

## References

Terra and `bevy_terrain` were permissive pattern references only. No source,
constants or layouts were copied. The decision mapping is in
`ATTRIBUTION.md` §13H.22.
