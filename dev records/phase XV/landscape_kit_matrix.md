# Phase XV-A landscape-kit review matrix

Frozen from `DefaultLandscapePreset::current()` — **not** F7 auto-splat (snow at 10 m).

| Item | Value |
|------|-------|
| Relief | 105 m (`DEFAULT_RELIEF_METRES`) |
| Snow / auto-splat height | ~65.1 m (`relief * 0.62`) |
| Terrain world size | 1024 m × 1024 m |
| Terrain translation | (−512, 0, −512) |
| Water local translation | (512, 15, 512) — shipping Great Lakes water |
| Default camera | position `(0, 150.75, 460.8)`, yaw −90°, pitch −22° |
| Runtime texture res | `SOMNIUM_TERRAIN_RES` default 2048 |
| Engine tiling | 0.25 / m (4 m repeat) for all layers until XV-H |

## Capture set

Evidence files belong under `dev records/phase XV/evidence/` as
`phase_XV-A_<purpose>.png` after tonemapping. Live GPU captures are still
pending an engine run; do not invent timings or images.

| Purpose | Camera | Lighting | What must be readable |
|---------|--------|----------|------------------------|
| `shore_day_dry` | default landscape cam | noon | wet sand → dry sand → meadow against the water body |
| `shore_day_damp` | eye-level at shoreline | noon | layer 9 vs 8 vs 4/0 |
| `shore_night` | same as shore_day_dry | moon / low IBL | same adjacency, no hue crash |
| `meadow_day` | default cam | noon | layers 0/4/12 |
| `forest_day` | under canopy-height cam | noon | layer 1 vs 0 |
| `mud_day` | lowland | noon | layer 5 vs 10 |
| `red_clay_day` | exposed bank | noon | layer 11 (not gravel 7) |
| `cliff_day` | looking at a steep face | noon | layer 14 biplanar, no albedo stretch |
| `talus_day` | cliff base | noon | layer 15 vs 14 vs 2 |
| `snow_day` | high ridge (~65 m+) | noon | layer 3 band |
| `cliff_triplanar_debug` | same as cliff_day | noon | `SOMNIUM_TERRAIN_TRIPLANAR=1` |
| `taps` | default cam | any | `SOMNIUM_SHADOW_DEBUG=12` |
| `discarded` | default cam | any | `SOMNIUM_SHADOW_DEBUG=18` |
| `selected` | default cam | any | `SOMNIUM_SHADOW_DEBUG=19` |

Dry / damp / wet columns for grass, mud, rock, and shore are **authoring
intent** for XV-H wetness. XV-A/F record the dry look; moisture affinity is in
`materials.json` but the global wetness scalar is not applied yet.

## Debug views (XV-D)

`SOMNIUM_SHADOW_DEBUG`:

- 12 — material-map taps / 36
- 18 — discarded strongest-four weight
- 19 — first three selected layer indices
- 20 — raw strongest-four weights (first three)
- `SOMNIUM_TERRAIN_TRIPLANAR=1` — triplanar reference for cliffs
