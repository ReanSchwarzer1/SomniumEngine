# DF-A — maximized-window timings (2026-08-14)

**Status: MEASURED.** Clipmap default remains **off** (walk luminance gate not met).

## Adapter freeze (live wgpu)

| Item | Value |
|---|---|
| Backend | Vulkan |
| Device | NVIDIA GeForce RTX 5080 Laptop GPU |
| vendor / device_id | 4318 / 11353 |
| Type | DiscreteGpu |
| Driver | NVIDIA 610.74 |
| GPU-driven / RT / timestamps / BC7 / FSR | yes |
| Binary | release `hello_engine`, capture frame 240, `SOMNIUM_MAXIMIZE=1` |
| Swapchain (maximized client) | **2560×1392** |
| Clipmap (this table except §3) | **off** |

XV-J froze the same adapter at a **1280×720** window. That understated pixel cost. Native maximized is the DF-A headline.

## 1. Native maximized (2560×1392), clipmap off

| View | FSR | Terrain px | Terrain lum | Shading | Frame | PNG |
|---|---|---|---|---|---|---|
| overview | on | 2 938 125 | 3171.8 | **50.838 ms** | 71.376 ms | `DF-A_overview_native_fsr.png` |
| overview | off | 2 938 106 | 3161.8 | **46.720 ms** | 65.364 ms | `DF-A_overview_native_nofsr.png` |
| walk / eye | on | 2 615 044 | 2923.1 | **49.677 ms** | 65.364 ms | `DF-A_walk_native_fsr.png` |
| walk / eye | off | 2 615 044 | 2917.1 | **48.851 ms** | 63.390 ms | `DF-A_walk_native_nofsr.png` |
| ridge-look | on | 1 599 155 | 6562.9 | **30.472 ms** | 41.282 ms | `DF-A_ridge_native_fsr.png` |
| ridge-look | off | 1 599 155 | 6543.7 | **29.801 ms** | 39.725 ms | `DF-A_ridge_native_nofsr.png` |

FSR does not cut work per terrain pixel. Overview and walk are both ~50 ms of shading at this resolution — vs XV-J **3.95 / 5.53 ms** at 1280×720.

## 2. Walk + FSR on, resolution sweep (window still 2560×1392)

| Preset | Scene | Terrain px | Shading | Frame |
|---|---|---|---|---|
| Native | 2560×1392 | 2 615 044 | **49.677 ms** | 65.364 ms |
| 2560×1440 cap | 2560×1392 (same as Native) | 2 615 044 | 49.125 ms | 64.870 ms |
| 1920×1080 | 1920×1044 | 1 470 974 | **29.149 ms** | 40.293 ms |
| 1600×900 | 1600×870 | 1 021 487 | **19.454 ms** | 28.855 ms |
| 1280×720 | 1280×696 | 653 737 | **12.623 ms** | 20.107 ms |

Shading scales with terrain pixel count, not with a 1280×720 window.

## 3. Debug mode 12 (taps), walk Native FSR on, HDR capture

`SOMNIUM_CAPTURE_AFTER_TAA` **unset** so luminance is the shader’s `taps / 36`.

| Terrain lum | × 36 | Mean taps / terrain pixel |
|---|---|---|
| 0.6023 | 21.68 | **~22** of a 36-tap worst case |

PNG: `DF-A_walk_native_fsr_taps12_hdr.png`.

## 4. Clipmap on (same Native maximized, FSR on) — not default

`SOMNIUM_TERRAIN_CLIPMAP=1`. Generate at frame 240 is idle (0.003 ms); first frames paid the full refresh.

| View | Shading off | Shading on | % of off | Terrain lum off | lum on | Δ lum |
|---|---|---|---|---|---|---|
| overview | 50.838 ms | **2.435 ms** | 4.8% | 3171.8 | 3175.0 | **+0.10%** |
| walk | 49.677 ms | **10.652 ms** | 21.4% | 2923.1 | 3963.0 | **+35.6%** |
| ridge | 30.472 ms | **7.345 ms** | 24.1% | 6562.9 | 6788.8 | **+3.4%** |

Overview hits the timing target (≤ 50% of off) and the 1% luminance gate. Walk shading is faster and must not regress — it doesn’t — but eye-level luminance is **not** within 1%, so **Clipmap stays default off** until that gate passes. Do not drop hex at the feet to chase the number; raise finest texels/m if the cache is too coarse.

PNGs: `DF-A_overview_native_fsr_clipmap.png`, `DF-A_walk_native_fsr_clipmap.png`, `DF-A_ridge_native_fsr_clipmap.png`.
