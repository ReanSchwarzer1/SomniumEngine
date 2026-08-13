# Phase XV-A — Codebase map (research gate)

> **Status:** XV-A research freeze, 2026-08-13. **Live engine truth is now XV-Zeta** —
> [`XV-Zeta_plan.md`](XV-Zeta_plan.md) (32 layers, sidecar v4, 1664-byte GPU
> material, biome v3, landscape v4). Do not treat the eight-layer / version-1
> numbers below as current API.  
> **Date:** 2026-08-13  
> **Purpose:** file-level “codebases understood” briefing before XV-A/XV implementation.

Independent translation only. Reference engines are cited by path and idea; nothing is lifted.

---

## 1. Somnium terrain architecture (current truth)

### 1.1 What actually ships

A 1024 m heightfield (`TerrainDescriptor` default: 16×16 chunks × 64 cells × 1 m) with **eight** photographed PBR layers, **two** RGBA8 splatmaps, hex tiling, height-aware blend, dominant-layer POM, derived/authored macro colour, and visibility-buffer shading. Terrain is **inside** the vis-buffer / `shading.wgsl` path (Phase 25A-2). `context.md` §20 still describes the retired Phase 14 standalone pass (4 layers, 3 arrays, own `TerrainPass`) and must not be treated as live API.

Default landscape: Great Lakes height + authored macro + child water at `DEFAULT_WATER_LEVEL_METRES = 16.1`. That shore is the XV water-parity fixture. Do not rewrite water.

### 1.2 Data flow: JPG → pack → GPU arrays → splat → fragment → ReSTIR

```
Poly Haven JPG (diff / nor_dx / arm / disp)
    tools/fetch_terrain_textures.sh
        → assets/terrain/_source/{mat}_{map}_{res}.jpg   (gitignored)
    cargo run -p somnium_asset --example pack_terrain
        → assets/terrain/{mat}_albedo.png   RGB sRGB albedo + A height
        → assets/terrain/{mat}_surface.png  RG normal XY, B roughness, A AO
TerrainLayerTextures::load_or_generate
        SOMNIUM_TERRAIN_RES (default 2048; 4096 opt-in, power-of-two ≥ 256)
        Lanczos3 resize of committed 4K PNG → size
        build_mip_chain: per-channel box filter of encoded bytes
        albedo array: Rgba8UnormSrgb
        surface array: Rgba8Unorm
SomniumRenderer::create_terrain
        publishes each array layer as a bindless texture_2d view
        splat 0–3 + splat 4–7 + macro map
CPU splat: Vec<SplatTexel>  SplatTexel = [u8; 8]
        paint / auto_splat / sidecar v2
        Splatmap::upload_dirty de-interleaves [0..4] / [4..8] into two RGBA8 textures (no mips)
shading.wgsl  (material.terrain_index >= 0)
        evaluate_terrain_material in terrain_material.wgsl
        metallic forced 0; occlusion multiplied with GTAO
restir_gi.wgsl
        gi_terrain_albedo: mean_albedo[8] × splat weights only
        (no height blend, hex, cliff, POM, or macro)
```

Packed channel contract (also `assets/LICENSE.md`):

| Texture | R | G | B | A |
|---|---|---|---|---|
| `*_albedo.png` | albedo R | albedo G | albedo B | height |
| `*_surface.png` | normal X | normal Y | roughness | AO |

Normal Z is reconstructed as `sqrt(max(1 - dot(nxy, nxy), 0))`. Metalness from Poly Haven `arm.b` is dropped (terrain is dielectric). Source normals are **`nor_dx`** (DirectX); there is no documented Y-flip or OpenGL conversion.

### 1.3 CPU / GPU / editor / GI / packer roles

| Piece | File | Role |
|---|---|---|
| Layer count, names, assets, splat GPU | `crates/somnium_renderer/src/terrain/textures.rs` | `TERRAIN_LAYER_COUNT = 8`, `LAYER_MATERIALS`, `SplatTexel`, load/mips/upload |
| TerrainData, sidecar v2, GPU material | `crates/somnium_renderer/src/terrain/mod.rs` | 448-byte `GpuTerrainMaterial`, tiling 0.25, `cliff_layer: 2` |
| Height blend CPU mirror | `crates/somnium_renderer/src/terrain/blend.rs` | `LAYER_BLENDS`, `parallax_depth` metres |
| Paint / auto_splat | `crates/somnium_renderer/src/terrain/brush.rs` | **Paint still clamps to layer 0–3** |
| Macro colour | `crates/somnium_renderer/src/terrain/macro_map.rs` | 512² derived/authored overlay |
| Visibility shading | `shaders/terrain_material.wgsl` + `shading.wgsl` | Full composite |
| Hex tiling | `shaders/hextile.wgsl` | Colour hex; `hex_sample_normal` exists but is **unused** by terrain |
| GI bounce albedo | `shaders/restir_gi.wgsl` | Mean albedo × splat |
| Packer | `crates/somnium_asset/examples/pack_terrain.rs` | 8 materials, no semantic mips, no BC7 |
| Fetch | `tools/fetch_terrain_textures.sh` | No hashes, no User-Agent identity beyond curl |
| Editor commands / undo | `crates/somnium_core/src/editor_commands.rs` | `SplatTexel` in undo payloads |
| Editor loop / keys | `crates/somnium_core/src/app.rs` | F6/F7, `,`/`.` cycle **% 4**, paint clamp **min(3)** |
| Inspector | `crates/somnium_ui/src/lib.rs` | Paint + Tile 0–3 + Relief only |
| Default scene cameras | `crates/somnium_core/src/landscape.rs` | Shared Create → Terrain recipe |
| Bindless publish | `crates/somnium_renderer/src/renderer.rs` | Per-layer `texture_2d` views of arrays |
| Layout lock | `material/pool.rs` + `tests/shaders_validate.rs` | `size_of::<GpuTerrainMaterial>() == 448` |

There is **no** `assets/terrain/materials.json`. Provenance for layers 0–7 is a single CC0 paragraph in `assets/LICENSE.md` (authors, Poly Haven, no per-file URL/hash/physical size/moisture tag).

### 1.4 UV scale / tiling

`TerrainLayer.tiling` is **UV repeats per metre**, default **0.25** for every layer (a 4 m tile). Inspector can edit only layers 0–3. Physical width is not a first-class property. Hex tiling is on unless `SOMNIUM_HEXTILE=0`.

### 1.5 2K vs 4K

Committed packs are 4K PNG. Runtime default is 2K. `SOMNIUM_TERRAIN_RES=4096` loads full size. Resize is Lanczos3 **before** the box-filter mip builder. No BC7 path exists (`TEXTURE_COMPRESSION_BC` is unused).

### 1.6 DefaultLandscapePreset cameras (`landscape.rs`)

`DEFAULT_LANDSCAPE_VERSION = 1`.

| Field | Value |
|---|---|
| Terrain | `TerrainDescriptor::default()` → 1024×1024 m |
| Relief | `DEFAULT_RELIEF_METRES = 105` |
| Auto-splat snow band | `relief * 0.62` ≈ **65.1 m** |
| Terrain translation | `(-512, 0, -512)` |
| Water local | `(512, 16.1, 512)` Great Lakes body |
| Camera position | `(0, relief * 1.15 + 30, depth * 0.45)` ≈ `(0, 150.75, 460.8)` |
| Yaw / pitch | −90° / −22° |

Startup and **Create → Terrain** share this recipe. F7 auto-splat does **not**: it calls `auto_splat(t, 10.0)`.

### 1.7 Cliff / triplanar path (current)

`terrain_triplanar_albedo` in `terrain_material.wgsl`:

- Axis weights: `pow(abs(n), 4)`, then normalize.
- Samples **albedo RGB only** with `textureSample` (implicit derivatives), three planes.
- No hex, no surface pack, no height, no AO, no projected normal.
- After composite: `albedo = mix(albedo, cliff, cliff_blend)`; **`roughness = mix(roughness, 0.8, cliff_blend)`**.
- `cliff_blend = smoothstep(0.45, 0.7, 1 - abs(geo_normal.y))`.
- `cliff_layer` is hardcoded **2** (`aerial_rocks_04`).
- Those three taps are **not** added to `terrain_taps`.

This is the albedo-only gap XV-F must replace. POM still runs on the top-down UV path even on steep pixels; Godot’s triplanar/height incompatibility is the reason XV wants projected POM off.

### 1.8 Mip generation (what XV-B must replace)

`build_mip_chain` in `textures.rs`: 2×2 box filter of **encoded u8 bytes**, independently per channel. Explicitly not alpha-weighted (height lives in albedo alpha). Consequences:

- Albedo is filtered in sRGB byte space, not linear.
- Packed normal XY is averaged as colours, not renormalized unit vectors.
- Height and AO are averaged as bytes (acceptable as linear-ish, but no range validation).
- Roughness is box-filtered with **no** Toksvig / normal-variance compensation.
- Packer writes full-res PNG only; mips are built at **load**, after optional downscale.

Godot’s `Image::generate_mipmap_roughness` is the named validation reference for XV-B (see §3.4), not a source lift.

### 1.9 Editor paint / undo / splat invariants

**Paint (`apply_paint`):**

- Increases target channel, renormalizes so weights **aim** at sum 255: `(wi * 255 + sum/2) / sum`.
- Rounding can leave the quantized sum ≠ 255.
- **`paint_layer.min(3)`** — layers 4–7 cannot be painted.
- No “at most four non-zero channels” rule. All eight can be live.

**Keyboard / UI:** `,` / `.` cycle `paint_layer % 4`; inspector Paint is `.min(3)`; only Tile 0–3 exist. No thumbnail palette.

**Auto-splat:** writes all eight from slope/height/noise. Can populate many simultaneous non-zero channels. Layer order matches `LAYER_NAMES`. Snow uses the caller’s `snow_height`.

**Undo:** stroke-start snapshots the **full** splat CPU buffer; command stores only the dirty rect of `SplatTexel`. Restore writes the rect and `mark_dirty`. Changing `SplatTexel` width is an undo ABI change (`editor_commands.rs`).

**Sidecar v2:** magic `STER` (`0x5354_4552`), version **2 only** (v1 refused). Layout: magic, version, height verts X/Z, splat W/H, f32 heightmap, then `width*height*8` splat bytes. Load requires matching descriptor dimensions. No material-manifest hash.

**New splat:** all weight on layer 0 (grass).

### 1.10 Visibility vs GI (shared vs duplicated)

Shared:

- Same `terrain_materials` storage buffer (`@group(0) binding 11`).
- Same bindless splat / mean-albedo slots.
- `restir_gi.wgsl` concatenates `hextile.wgsl` + `terrain_material.wgsl` but **does not call** `evaluate_terrain_material`.

Duplicated / drifted:

| | Visibility | GI |
|---|---|---|
| Splat fetch | `textureSample` | `textureSampleLevel(..., 0)` |
| Weights | 8-way normalize | same 8-way (loop `0..4` on lo+hi) |
| Colour | hex + height blend + perceptual albedo + macro + cliff | `layer_albedo[i]` means only |
| Normal / rough / AO | full | unused (diffuse bounce) |

GI is *intentionally* cheaper, but it still hardcodes two splatmaps and eight `layer_albedo` slots. XV-D must keep indexing identical even if GI stays mean-based.

Foliage defaults to splat **layer 0** (`FoliageParams.layer`). Expanding layers does not by itself move grass.

---

## 2. Hardcoded 8-layer inventory

Constants / types:

- `TERRAIN_LAYER_COUNT = 8`, `SplatTexel = [u8; 8]`
- `LAYER_NAMES`, `LAYER_MATERIALS`, `RECIPES` (procedural fallback)
- `LAYER_BLENDS[8]`
- `GpuTerrainMaterial`: `layer_tiling/albedo_maps/surface_maps/height_scale/blend_width/weight_clamp/parallax` all `[f32|i32; 8]`; `layer_albedo: [[f32;4]; 8]`; `splat_map` + `splat_map_hi` only
- WGSL `TERRAIN_LAYERS = 8u`; `array<vec4<f32>, 2>` packing (must become **4** vec4s for 16, still 16-byte aligned — never `array<f32, 16>`)
- `array<f32, 8>` / `array<TerrainLayerSample, 8>` in `evaluate_terrain_material`
- `TERRAIN_MAX_TAPS = 48` (8 layers × 2 maps × 3 hex taps)
- `gi_terrain_albedo` `for i in 0u .. 4u` + `layer_albedo[i + 4u]`
- Packer / fetch: 8 material names
- Sidecar: splat block size `* 8`; version 2
- `Splatmap::upload_dirty`: split at index 4
- Inspector: 4 tiling fields; paint `% 4` / `.min(3)`
- Tests: `blend.rs` uses `N = TERRAIN_LAYER_COUNT`; `pool.rs` offset 132 = `splat_map_hi`

Not 8-wide but related:

- `cliff_layer: 2`
- Default tiling `0.25`
- Detail fade 60 → 400 m; POM 24 + 8 shadow steps
- Splat textures: `mip_level_count: 1`

WGSL packing trap (already documented, still load-bearing): a bare `array<f32, N>` has **16-byte stride**. Per-layer scalars must stay `array<vec4<f32>, N/4>`. `shaders_validate.rs` asserts struct **span 448**. Header comments that still say “256 bytes” are stale.

---

## 3. example_repo pattern map

Root: `C:\Users\adhir\Downloads\GE\example_repo`.

### 3.1 O3DE — many globally, few locally (indexed IDs)

**Plan path miss:** `TerrainDetailMaterialManager.cpp` is **not** under `Components\DetailMaterial\`. Actual:

`o3de-development\o3de-development\Gems\Terrain\Code\Source\TerrainRenderer\TerrainDetailMaterialManager.cpp`  
(+ `.h`)

Confirmed present:

- `...\Assets\Shaders\Terrain\TerrainDetailHelpers.azsli`
- `...\Code\Framework\AzFramework\AzFramework\SurfaceData\SurfaceData.h`

**What it teaches XV**

- `SurfaceData.h`: `MaxSurfaceWeights = 16` tagged weights per surface point (authoring-side list, sorted high→low). Not the GPU blend count.
- `UpdateDetailTexture`: stores **top two** material IDs + 8-bit relative blend per texel (`DetailMaterialPixel`). Docs’ “top three normalized” is **not** what this source writes.
- `GetDetailSurface`: `Load`s four neighbouring ID texels (up to 8 ID slots), bilinear position weights, **deduplicates by ID**, then `AppendHeightToWeight` + depth-band blend. This is exactly why XV keeps **direct RGBA splats** — indexed IDs cannot hardware-filter without this gather/dedup.
- Height blend: `m_heightBlendFactor` default 0.5 (band width), `m_heightWeightClampFactor` default 0.1 uploaded as reciprocal. Somnium already mirrors this in `blend.rs` / `terrain_append_height`.
- Macro/detail colour blend modes (Multiply / Lerp / LinearLight / Overlay) already ported in `macro_map.rs`.

**Borrow:** “many globally, few locally,” height-append clamp, perceptual albedo.  
**Do not borrow:** ID+blend control texture.

### 3.2 Bevy triplanar — `bevy_triplanar_splatting` `biplanar.wgsl`

Path exists: `bevy-plugins\bevy_triplanar_splatting-main\src\shaders\biplanar.wgsl`

**Ideas:** `calculate_biplanar_mapping(p, n, k)` — explicit `dpdx`/`dpdy` of world position; major + median axes (drop the minor); remap weights so ~0.577 (`1/sqrt(3)`) maps to 0 to hide the (±1,±1,±1) axis switch; `pow(w, k/8)` sharpness; `textureSampleGrad` per plane. `biplanar_texture_splatted` skips zero-weight layers.

**Borrow:** explicit gradients + two-plane default + axis-switch hysteresis.  
**Keep Somnium’s** orientation-correct normals / surface-gradient compose; do not translate this shader text.

### 3.3 Wicked Engine — `wiTerrain.cpp`

Path exists: `New_Engines\WickedEngine-master\WickedEngine\wiTerrain.cpp`  
Header: `wiTerrain.h`

**Ideas:** `MATERIAL_COUNT = 4` (`BASE / SLOPE / LOW_ALTITUDE / HIGH_ALTITUDE`) — a **tiny** material array, then a **sparse virtual atlas** (`VirtualTextureAtlas`, residency/feedback/page buffers, GPU tile allocate). Comments: no-sparse fallback with extra BC copies; Metal sparse+BC workaround.

**Borrow:** continue Somnium’s material **arrays**.  
**Avoid:** atlas / page-table / feedback loop. Matches XV deferral of RVT. Wicked is not a 16-layer splat reference.

### 3.4 Godot 4.7.1-stable — MIT, patterns only

All cited files exist. `LICENSE.txt` is MIT (Juan Linietsky / Ariel Manzur / contributors).

| File | Idea for XV |
|---|---|
| `scene/resources/material.cpp` | Generated `triplanar_texture`; `uv1_power_normal = pow(abs(n), sharpness)` then normalize; **same projected UV for albedo, metallic, roughness/ORM, normal, bent-normal, AO, emission, detail**. World vs object triplanar flags. Height mapping **disabled** when triplanar is on (warn + ignore). |
| `doc/classes/BaseMaterial3D.xml` | Triplanar cost (3 reads, not crisp); `uv1_triplanar_sharpness` clamped 0–150; heightmap incompatible with triplanar. |
| `core/io/image.cpp` `Image::generate_mipmap_roughness` | Reconstruct N.z; summed-area table of normals; per mip texel, mean normal length `r`; if `r < 1`, `kappa = (3r - r³)/(1-r²)`, `variance = 0.25/kappa`; `roughness = sqrt(r² + min(3·var, 0.4²))`. |
| `editor/import/resource_importer_texture.cpp` | Associates roughness with a **source normal** (`roughness/src_normal`); runs roughness-mip **after** normal mip gen; optional invert-Y. |

Godot has **no** native 3D splat-terrain subsystem. It does not replace O3DE/Unreal/Fyrox for layer management.

**Borrow:** full-channel projection policy; POM off on cliffs; Toksvig-style limiter as XV-B **comparison fixture**. Independently re-express in Rust.

### 3.5 Unreal Landscape weightmaps + RVT

Tree is nested: `UnrealEngine-release\UnrealEngine-release\Engine\...`

**Weightmaps:** `Engine\Source\Runtime\Landscape\Classes\LandscapeComponent.h` — `WeightmapTextures` array; each layer alloc has `WeightmapTextureIndex` + `WeightmapTextureChannel` (0–3). Four layers per RGBA texture, **variable number of textures** as layers grow. Edit path in `LandscapeEditLayers.cpp` treats channel `< 4` as the packing unit. This is the same “direct weights, 4 per RGBA” idea XV wants (four textures for 16 layers), without Unreal’s per-component allocation complexity.

**RVT:** `Engine\Source\Runtime\Engine\Classes\VT\RuntimeVirtualTexture.h` — tile count/size/border, material type (`BaseColor_Normal_Specular` etc.), compression, page-table packing, adaptive page tables, continuous page updates, producer priority. Renderer: `Engine\Source\Runtime\Renderer\Private\VT\` (scene proxy, invalidate, preload). Niagara also samples landscape via RVT.

**Why XV defers RVT:** it is a second renderer (page table, invalidation, residency, editor feedback, compressed physical tiles). Somnium’s terrain is 1 km and already has a working direct-array path. Reconsider only if XV-D/E miss budgets.

### 3.6 Fyrox — layer mask stack

`fyrox\Fyrox-master\fyrox-impl\src\scene\terrain\mod.rs` — `Layer { material, mask_property_name, ... }`. Comment: as many layers as you want, each slightly slower; 1–5 typical. Each layer binds its **own mask texture**. Somnium’s comments already contrast this with packed RGBA.

**Borrow:** artist-facing layer stack / paint-one-layer mental model.  
**Keep:** packed control maps, not one mask texture per layer.

Brush falloff in Somnium already ports Fyrox `1 - d/r` + hardness remap.

### 3.7 Optional refs in this tree

| Name | Present? |
|---|---|
| **CDLOD** | **Yes** — `CDLOD-master\` (paper + DX9 source). Already Somnium’s LOD citation. Out of XV (Phase 25C). |
| Terrain3D | **No** |
| PlumeSplat | **No** |
| Mikkelsen `surfgrad-bump-standalone-demo` | **No** |
| Hollow-TerrainSystem / PVTUT | **No** |

Those four remain web/plan citations only (`phase_XV.md` §5.5.3). Do not pretend they were audited on disk.

### 3.8 Water refs (glance only)

Present: `GodotOceanWaves-main\`, `bevy-plugins\bevy_water-main\`.  
XV fixture is the **shipping Great Lakes water body** beside wet→dry sand→meadow. Not a water rewrite (Phase VV owns reflections).

---

## 4. Recommended start order (still not implementing)

XV-A itself is baseline + provenance, **not** the 16-layer data model. When implementation is authorized:

### XV-A (this phase’s actual work)

1. Freeze cameras from `DefaultLandscapePreset` + record adapter / `SOMNIUM_TERRAIN_RES` / hex/macro/parallax env. Re-measure taps and shader time; do not trust 0.883 ms blindly.
2. Add `assets/terrain/materials.json` schema; fill layers 0–7 provenance gaps from `LICENSE.md` + fetch/packer (hashes, physical size, `nor_dx`, moisture affinity). Do not download the eight new binaries until the manifest is fail-closed.
3. Landscape-kit matrix including Great Lakes shore under shipping water (dry/damp/wet × day/night).
4. Evidence path: `dev records/phase XV/evidence/phase_XV-A_<purpose>.png`.

### After XV-A, file order for C→D (when authorized)

Do **CPU/editor/sidecar before GPU**, or GI and undo will disagree.

1. `textures.rs` — raise `TERRAIN_LAYER_COUNT`, `SplatTexel`, `LAYER_*` tables (indices 0–7 frozen).
2. `brush.rs` — **remove `.min(3)`**; four-nonzero decay + remainder-to-255; `auto_splat` 16-wide but sparse.
3. `editor_commands.rs` + `app.rs` + `somnium_ui` — paint 0–15, sixteen tiles/thumbnails, undo still carries `SplatTexel`.
4. `mod.rs` sidecar **v3** (copy v2 bytes 0–7, zero 8–15) + `GpuTerrainMaterial` vec4×4 packing + two new splat bindings. Update `pool.rs` / `shaders_validate.rs` **in the same change**.
5. `blend.rs` — 16 `LAYER_BLENDS`; keep CPU/WGSL algorithm identity.
6. `terrain_material.wgsl` — strongest-four before sampling; share helpers with GI.
7. `restir_gi.wgsl` — same splat count / `layer_albedo` length / indexing.
8. `pack_terrain.rs` + `fetch_terrain_textures.sh` — manifest-driven (XV-B can precede C if packs are needed for A visuals of new materials; do not commit binaries without manifest).

Cliff (XV-F) and semantic mips (XV-B) should not start until the 16-wide CPU/GPU structs compile and v2 scenes migrate.

---

## 5. Risks

**GI drift.** Two splatmaps and eight means are duplicated in `gi_terrain_albedo`. A third/fourth splat or 16 `layer_albedo` slots missed here bounces the wrong ground colour into every interior.

**WGSL alignment.** Expanding `[f32; 8]` → `[f32; 16]` without vec4 packing will 16-byte-stride. Struct is 448 B today; 16 layers add another 8 floats × several fields plus 8 mean-albedo vec4s. Assert offsets again. Stale “256-byte” comments in WGSL/Rust are already wrong.

**Cliff projection.** Albedo-only, implicit derivatives, fixed roughness 0.8, taps uncounted. Biplanar must cover albedo/normal/rough/AO/height, `SampleGrad`, suppress POM, compose normals as surface gradients. Axis switch at (±1,±1,±1) needs Bevy/Inigo remap. Layer 14 is the planned cliff face; layer 2 must keep legacy meaning.

**Mip semantics.** Current box-filter of sRGB albedo + packed normals is exactly the shimmer/dark-mip XV-B exists to kill. Runtime mip build after Lanczos downscale also means 2K and 4K packs do not share mip bytes.

**Hex normals.** `hex_sample_normal` counter-rotates; `terrain_sample_layer` hex-samples the **whole surface pack as colour**. Packed XY normals are not unrotated. XV-H surface-gradient work should not inherit this.

**Authoring vs GPU.** GPU already has 8 layers; **paint/UI still 4**. Auto-splat can write >4 live channels; XV-C’s four-nonzero stored limit will change default Great Lakes looks unless migration preserves 0–7 appearance and only decays 8–15 (zeros) plus future paint.

**Tap accounting.** `terrain_taps` counts gated layer maps only. Splat, macro, POM height marches, and cliff triplanar are excluded. XV budgets want those reported separately.

---

## 6. Plan vs live code (contradictions)

| `phase_XV.md` / handoff assumption | Live code |
|---|---|
| Eight layers are paintable | `apply_paint` / keys / inspector clamp to **0–3**. Layers 4–7 exist via auto-splat + sidecar only. |
| Stored texel already sparse-ish | No four-nonzero cap. Auto-splat can light many channels. |
| Sidecar v2 ready for v3 migration | v2 only; unknown versions refused; no manifest hash field. |
| `context.md` §20 is the terrain architecture | §20 is Phase 14 (4 layers, outside vis-buffer). Live path is 25A-2 + 25L. |
| `GpuTerrainMaterial` “256 bytes” (WGSL comment) | **448 bytes**, tested. |
| O3DE manager at `...\Components\DetailMaterial\TerrainDetailMaterialManager.cpp` | Actual: `...\TerrainRenderer\TerrainDetailMaterialManager.cpp`. |
| O3DE “top three weights” as the local source of truth | Local source stores **top two IDs** + blend; shader gathers neighbours. |
| Terrain3D / PlumeSplat / surfgrad demo / PVTUT “if present” | **Not in this example_repo.** CDLOD **is**. |
| Unreal root `UnrealEngine-release\UnrealEngine-release` | Confirmed nested that way. |
| Create Terrain and F7 share biome constants | Create uses `auto_splat_height ≈ 65 m`; **F7 uses 10 m**. |
| Packer material order = layer index order | Packer/fetch list order differs from `LAYER_MATERIALS`; files are named so load is still correct. |
| Hex tiling handles normals | Colour `hex_sample` used for surface pack; `hex_sample_normal` unused. |
| Cliff “albedo-heavy” | Confirmed: albedo mix + roughness 0.8; no PBR channels. |
| BC7 when BC feature present | No BC7/BC feature request anywhere. |
| `materials.json` as source of truth | File **absent**. |
| Inspector sixteen thumbnails | Numeric Paint + four tiling fields. |
| Shared WGSL helpers for terrain and GI | Concatenated modules; **separate** evaluation (`evaluate_terrain_material` vs `gi_terrain_albedo`). |

None of these block XV-A baseline/provenance. They **must** be in the implementing session’s first file-open, because §9 of the plan is guidance, not an API.

---

## Appendix A — Layer 0–7 lock

| i | `LAYER_MATERIALS` | Editor name | Role |
|--:|---|---|---|
| 0 | `aerial_grass_rock` | Grass | Default / foliage layer |
| 1 | `forrest_ground_01` | Forest Floor | |
| 2 | `aerial_rocks_04` | Rock | **`cliff_layer`** |
| 3 | `snow_02` | Snow | |
| 4 | `leafy_grass` | Meadow | Not brush-paintable today |
| 5 | `brown_mud` | Mud | Not brush-paintable today |
| 6 | `coast_sand_rocks_02` | Sand | Not brush-paintable today |
| 7 | `gravel_floor` | Gravel | Not brush-paintable today |

## Appendix B — Env knobs (baseline capture)

`SOMNIUM_TERRAIN_RES`, `SOMNIUM_HEXTILE`, `SOMNIUM_TERRAIN_MACRO`, `SOMNIUM_TERRAIN_DETAIL_FADE`, `SOMNIUM_TERRAIN_HEIGHT_BLEND`, `SOMNIUM_TERRAIN_PARALLAX`, `SOMNIUM_HEIGHTMAP`.
