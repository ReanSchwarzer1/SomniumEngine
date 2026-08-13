# BC7 terrain packs — visual check (2026-08-13)

**Status:** encoder + runtime load **in engine**. Packs are local
(`assets/terrain/bc7/`, gitignored). 1.10 ms shading remains an exception.

## How to rebuild packs

```text
cargo run --release -p somnium_renderer --example encode_terrain_bc7
cargo run --release -p somnium_renderer --example encode_terrain_bc7 -- --force
```

Semantic mips (`terrain::mips`) then Intel ISPC BC7 via `intel_tex_2`
`alpha_basic_settings` (height and AO live in alpha). Hero 0–15 at 2048; extra
16–31 at 1024; procedural slots 16 and 24 included. This run: 30 photographed +
2 procedural, 175.8 s.

A/B switch after packs exist: `SOMNIUM_TERRAIN_FORCE_RGBA8=1`.

## Runtime (release, 1280×720, frame 240, RTX 5080 Laptop, Vulkan 610.74)

| Path | Residency log | Hero / extra |
|---|---|---|
| BC7 | `compressed=true` ~**213 MiB** | 2048 / 1024 |
| RGBA8 (`FORCE_RGBA8=1`) | `compressed=false` ~**341 MiB** | 1024 / 1024 (853 MiB 2048+1024 still over 700) |

Never both resident. wgpu `write_texture` copy size is a 4×4 block multiple on
the 2×2 / 1×1 mips.

## GPU timings (shading pass)

| View | BC7 2048+1024 | RGBA8 1024+1024 | XV-J RGBA8 (earlier) |
|---|---:|---:|---:|
| overview | **3.794 ms** | 4.027 ms | 3.951 ms |
| walk / eye | **5.250 ms** | — | 5.532 ms |
| shore dry | **3.227 ms** | — | 3.481 ms |
| cliff | 4.580 ms | 4.502 ms | 4.499 ms |

1.10 ms is **not met**. BC7 is a residency win (hero 2K back under budget), not
a shading-budget close.

## Visual A/B (tonemapped Reinhard PNG, this folder)

Same cameras as XV-J. First attempt was invalidated by a leftover
`SOMNIUM_SHADOW_DEBUG` in the process environment; recaptured with it unset.

Mean absolute 8-bit error (every other pixel, all channels):

| Pair | MAE |
|---|---:|
| BC7 vs RGBA8 overview | **0.162** |
| BC7 vs RGBA8 cliff | **0.599** |
| BC7 vs XV-J overview | 0.256 |
| RGBA8 vs XV-J overview | 0.128 |

Cliff is the height/AO/POM risk. Sub-1 MAE at 8-bit is within capture noise +
the 2048-vs-1024 mix; no blend halo or flattened relief showed up in the
sampled grids. Overview, walk, and shore match the XV-J look.

| File | Path |
|---|---|
| `phase_XV-BC7_overview_day.png` | BC7 live |
| `phase_XV-BC7_eye_day.png` | BC7 walk |
| `phase_XV-BC7_shore_day_dry.png` | BC7 shore |
| `phase_XV-BC7_cliff_day.png` | BC7 cliff |
| `phase_XV-RGBA8_overview_day.png` | `FORCE_RGBA8=1` |
| `phase_XV-RGBA8_cliff_day.png` | `FORCE_RGBA8=1` cliff |

Matching `.log` files have adapter, residency, and profiler lines.
`WaterComponent::great_lakes` untouched.
