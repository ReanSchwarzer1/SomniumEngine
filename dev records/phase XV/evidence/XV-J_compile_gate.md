# XV-J — verification record (2026-08-13)

**Status: COMPLETE** with the explicit exceptions in §11 below.
GPU PNGs, wgpu adapter freeze, and release profiler timings are in this folder.

## Toolchain

| Item | Value |
|---|---|
| rustc / cargo | 1.88.0 (project target remains Rust 1.85, wgpu 29, winit 0.30) |
| Binary | release `hello_engine`, capture frame 240, 1280×720, vsync on |
| Capture | `SOMNIUM_CAPTURE_PNG` + `SOMNIUM_CAPTURE_AFTER_TAA=1` + `SOMNIUM_CAPTURE_QUIT=1` |

## Adapter freeze (live wgpu, not WMI)

From `phase_XV-J_overview_day.log`:

| Item | Value |
|---|---|
| Backend | Vulkan |
| Device | NVIDIA GeForce RTX 5080 Laptop GPU |
| vendor / device_id | 4318 / 11353 |
| Type | DiscreteGpu |
| Driver | NVIDIA 610.74 |
| GPU-driven (MDI) | yes |
| Hardware RT | yes |
| Timestamp profiler | yes |
| `TEXTURE_COMPRESSION_BC` | **yes** (packs absent → RGBA8) |

WMI still reports a second Intel Arc 140T; wgpu selected the NVIDIA discrete GPU.

## Compile gate (unchanged from earlier this day)

```text
cargo fmt --all -- --check          # pass
cargo check --workspace             # pass
cargo test --workspace              # 321 + CIEDE2000 fixture, 0 failures
cargo test -p somnium_renderer --test shaders_validate   # 9 pass
cargo run -p somnium_asset --example pack_terrain -- --validate-only
                                    # 30 photographed layers, 0 missing
```

Added this session: sidecar migrate helper tests, aerial-LOD fixtures,
`pack_terrain --validate-only`, offline CIEDE2000 strongest-four vs full blend,
`SOMNIUM_KIT_VIEW` / `SOMNIUM_CAPTURE_QUIT` harness.

## Residency (logged at load + capture)

Projected 2048+1024 RGBA8 = **853 MiB** (over 700). Runtime dropped both banks
to **1024** (**~341 MiB**). Capture line:

`compressed=false from_assets=false hero=1024 extra=1024`

`from_assets=false` because layers 16 and 24 are procedural (30/32 photographed).
RGBA8 was uploaded; BC7 packs are not in tree. Never both resident.

## GPU timings (release, 1280×720, frame 240)

| View | Shading | Frame | Notes |
|---|---:|---:|---|
| overview day | **3.951 ms** | 10.841 ms | aerial hex/POM off (~150 m up); ReSTIR GI 2.56 ms; water 2.46 ms |
| overview night | 3.926 ms | — | `SOMNIUM_SUN_ELEVATION=-20` |
| eye / walk | **5.532 ms** | — | kit `walk`, 1.7 m AGL, hex on |
| shore day dry | 3.481 ms | — | waterline, height 16.10 m |
| shore day wet | 3.333 ms | — | `SOMNIUM_TERRAIN_WETNESS=1` |
| shore night | 3.140 ms | — | |
| cliff day | 4.499 ms | — | ridge ~95 m |
| snow day | 4.401 ms | — | high ridge ~104 m |
| forest day | **8.036 ms** | — | close hex+POM |
| taps / discarded / selected | ~3.75–3.82 ms | — | debug views, default cam |

§10.1 median **1.10 ms** is **not met**. The earlier ~20 ms overview number was
an unoptimized debug session; release overview shading is ~4 ms. Further cut is
a second aerial PSO + BC7, not a per-pixel sample-count branch.

## PNG corpus (tonemapped Reinhard from HDR, this folder)

| File | Purpose |
|---|---|
| `phase_XV-J_overview_day.png` | default landscape cam, noon, water in frame |
| `phase_XV-J_overview_night.png` | same, moon |
| `phase_XV-J_eye_day.png` | walking height |
| `phase_XV-J_shore_day_dry.png` | Great Lakes shore, shipping water |
| `phase_XV-J_shore_day_wet.png` | same, wetness 1 |
| `phase_XV-J_shore_night.png` | same, night |
| `phase_XV-J_cliff_day.png` | steep ridge / cliff |
| `phase_XV-J_snow_day.png` | high ridge (snow mixed with rock at this look) |
| `phase_XV-J_forest_day.png` | forest-floor weights |
| `phase_XV-J_taps.png` | `SOMNIUM_SHADOW_DEBUG=12` |
| `phase_XV-J_discarded.png` | debug 18 |
| `phase_XV-J_selected.png` | debug 19 |

Matching `.log` files hold the adapter line, kit-view placement, and profiler dump.

## Sparse selection (§10.3)

CPU fixture `strongest_four_stays_inside_ciede2000_budget_against_full_blend`
passes (2–4 way + weak fifth). Not a GPU-sampled photogrammetry ΔE.

## §11

| # | Result |
|---|---|
| 1–8, 12–13, 15–16 | Met |
| 3 | Met: v2 copies 0–7, no four-nonzero on migrate |
| 9 | **Exception:** BC supported; packs absent; RGBA8 only |
| 10 | **Exception:** 341 MiB @1K (under 700). Shading 3.95 ms overview / 5.53 ms walk, not 1.10 ms |
| 11, 14 | Met by the PNG corpus above (wet/dry shore included) |

Histogram-preserving blend stays deferred. `WaterComponent::great_lakes` untouched.
