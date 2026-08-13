# Phase XV-A — Baseline and provenance research

> **Status:** RESEARCH COMPLETE — no engine code, shaders, splat-layout changes, or new texture binaries  
> **Access date:** 2026-08-13  
> **API User-Agent used:** not sent by the docs fetcher; XV-B must send `SomniumEngine-terrain-fetch/XV` on every Poly Haven and ambientCG request  
> **Controlling plan:** [`phase_XV.md`](../phase_XV.md)

This record is the XV-A provenance gate. It freezes the eight-layer baseline *contract*, fills missing layer 0–7 metadata, audits the eight proposed Poly Haven additions, records two role substitutions, defines the landscape-kit matrix, and publishes the `materials.json` schema. Live GPU timings, tap counts, adapter identity, and SHA-256 of downloaded bytes remain for the first authorized implementation session (they require running the engine and fetching files).

No new texture binary entered the repository.

---

## 1. Eight-layer baseline freeze (code contract)

Re-measure timings on the user’s adapter before claiming XV-D/E budgets. Historical Phase 25D numbers are comparison hints only.

### 1.1 Scene and camera

| Item | Value | Source |
|---|---|---|
| Terrain extent | 1024 m × 1024 m | `TerrainDescriptor::default`: 16×16 chunks × 64 cells × 1.0 m |
| Vertices | 1025 × 1025 | |
| Relief | 105 m | `DEFAULT_RELIEF_METRES` |
| Water datum | 16.1 m | `DEFAULT_WATER_LEVEL_METRES` / `WaterComponent::great_lakes` |
| Optical `max_depth` | 18.6 m | IV-K authored body — do not retune |
| Gerstner `wave_speed` | 0.85 | Boat buoyancy contract |
| Default camera | `(0.0, 150.75, 460.8)`, yaw −90°, pitch −22° | `DefaultLandscapePreset`: `y = relief*1.15+30`, `z = depth*0.45` |
| Eye-level camera | local XZ `(348.16, 423.6)`, world `(-163.84, ground+1.6, -88.4)`, yaw −90°, pitch −6° | `SOMNIUM_TERRAIN_EYE=1`: `(wx*0.34, wz*0.40+14)` |
| Night capture | `SOMNIUM_SUN_ELEVATION=-20` | Phase IV evidence convention |
| Moon | 0.010 lux | Post-25M2 accepted default |
| Runtime pack | 2K default; 4K via `SOMNIUM_TERRAIN_RES=4096` | `textures.rs` |
| Hex tiling | on unless `SOMNIUM_HEXTILE=0` | |
| Layer tiling (all eight) | **0.25 repeats/m = 4 m tile** | `TerrainLayer.tiling`, not physical size |

### 1.2 Historical GPU numbers (must remeasure)

| Metric | Recorded | Caveat |
|---|---|---|
| Terrain shader median | 0.883 ms (was 0.973 ms) | Phase 25D reference view/adapter |
| Landscape material-map taps | ~11.44 | same |
| Eye-level material-map taps | ~12.00 | same |
| Adapter / driver / resolution | **not frozen this session** | capture on the XV-A implementation run |

### 1.3 Packed residency (eight layers, current)

Two RGBA8 arrays, eight layers, full mip chain:

| Pack | Formula | 2K | 4K |
|---|---|---:|---:|
| Current eight-layer RGBA8 | 16 maps × *N*² × 4 × 4/3 | 341.3 MiB | 1.33 GiB |
| Planned sixteen-layer RGBA8 | 32 maps | 682.7 MiB | 2.67 GiB |
| Planned sixteen-layer BC7 | 32 maps / 4 | 170.7 MiB | 682.7 MiB |

Packed `*_albedo.png` / `*_surface.png` for layers 0–7 **are git-tracked** under `assets/terrain/`. Sources in `_source/` are gitignored. Committed packed PNGs have **no recorded SHA-256** in-tree today.

### 1.4 Shader / data-model freeze (do not change in XV-A implementation)

- `TERRAIN_LAYER_COUNT = 8`, sidecar v2, two RGBA splatmaps, `SplatTexel = [u8; 8]`
- Two packed arrays: albedo RGB + height A; normal XY + roughness + AO; DirectX normals (`nor_dx`)
- Cliff path uses layer 2 (`cliff_layer`), albedo-heavy projection — XV-F work
- All CPU/GPU/editor/ReSTIR paths assume eight layers
- `auto_splat` paints only indices 0–7 from slope/height/noise

---

## 2. Layers 0–7 — provenance gaps filled

`assets/LICENSE.md` currently credits “Rob Tuytel, Rico Cilliers and contributors” as a lump. First-party Poly Haven `/info` on 2026-08-13 disagrees for several assets. Physical size was never a first-class property; every layer tiles at 4 m regardless of the scan.

License for all eight: **CC0 1.0** via <https://polyhaven.com/license> (confirmed 2026-08-13). Poly Haven API Terms additionally require a unique User-Agent and API-service credit when the *live API* is used; the assets themselves remain CC0.

`files_hash` below is Poly Haven’s asset-level listing hash from `/info`. Per-file MD5s for the packer quartet (`Diffuse`, `nor_dx`, `arm`, `Displacement`) at 4K JPG are in [`materials.draft.json`](materials.draft.json). **SHA-256 is unknown until XV-B downloads.** Normal convention: **DirectX** (`nor_dx`). Colour: sRGB diffuse.

| Idx | ID | Display | Authors (first-party) | Physical size | PH category / notes | Moisture | Cliff |
|---:|---|---|---|---:|---|---:|---:|
| 0 | `aerial_grass_rock` | Grass | Rob Tuytel | **15.0 m** | Mossy & lichened rock; aerial grass-over-stone | 0.55 | 0.35 |
| 1 | `forrest_ground_01` | Forest Floor | Rob Tuytel | **2.0 m** | Leaf litter | 0.70 | 0.05 |
| 2 | `aerial_rocks_04` | Rock / legacy cliff | Rob Tuytel | **80.0 m** | Aerial cliff scan — why eye-level cliffs stretch | 0.25 | 0.60 |
| 3 | `snow_02` | Snow | Rob Tuytel | **2.0 m** | Fresh snow | 0.15 | 0.00 |
| 4 | `leafy_grass` | Meadow | **Charlotte Baglioni** | **2.0 m** | Lush grass; not Tuytel | 0.60 | 0.05 |
| 5 | `brown_mud` | Mud | Rob Tuytel | **1.30 m** | API scale string `1,3`; dimensions 1300 mm | 0.95 | 0.00 |
| 6 | `coast_sand_rocks_02` | Sand / pebbled coast | Rob Tuytel | **15.0 m** | Coastal *rock*, not fine sand | 0.45 | 0.25 |
| 7 | `gravel_floor` | Gravel | **Matterfield** photo, **Jenelle van Heerden** process | **2.25 m** | Driveway gravel | 0.20 | 0.10 |

Channel set: every existing layer exposes `Diffuse`, `nor_dx`, `nor_gl`, `arm`, `Displacement`, `AO`, `Rough` at 1K–8K JPG. Packer already consumes the first four of those.

**Tiling implication.** Physical tile for layer 2 is 80 m; the engine currently forces 4 m. XV-H must store `physical_width_m` and a bounded `uv_scale_multiplier` (today’s 4 m / 80 m = 0.05 for layer 2, 4 m / 15 m = 0.267 for aerial grass/sand-rock). Do not silently switch shipping 0–7 to 1:1 physical scale — that would change old scenes.

---

## 3. Proposed layers 8–15 — first-party audit

All eight proposed Poly Haven IDs **exist**, are **CC0**, ship the packer quartet at **2K and 4K JPG**, and use **DirectX** normals. None were downloaded.

### 3.1 Pass as proposed

| Idx | ID | Role | Authors | Size | `surface_use` | Moisture | Cliff | 4K Diffuse MD5 |
|---:|---|---|---|---:|---|---:|---:|---|
| 8 | `aerial_sand` | Dry beach | Rob Tuytel | 15.0 m | ground (aerial) | 0.35 | 0.00 | `421927b11af0314bb237cc584f89aaf0` |
| 9 | `coast_sand_01` | Damp shoreline | Rob Tuytel | 15.0 m | ground (aerial); tags include `damp` | 0.90 | 0.00 | `c4fcd9ecc7ce8cb98a152c6cc4583674` |
| 10 | `dry_mud_field_001` | Dry earth | Rob Tuytel photo, Rico Cilliers process | 3.0 m | ground | 0.40 | 0.00 | `028b08c8b0837b9a76b1f08d75c6b333` |
| 12 | `sparse_grass` | Sparse grass | **Amal Kumar** | 2.0 m | ground; extra **Mask** map | 0.55 | 0.00 | `569f9a6a0cf566c88d281bc81f832e97` |
| 13 | `mossy_rock` | Mossy mountain rock | Rob Tuytel | 3.0 m | ground | 0.85 | 0.55 | `c9374b25a241c712aa86c2d24ec5329a` |
| 14 | `rock_face_03` | Vertical cliff | Dario Barresi photo, Rico Cilliers process | **2.70 m** | **wall** | 0.20 | **1.00** | `50105980b509029b9040f43b1f6ddc71` |

`sparse_grass` ships a `Mask` channel. XV-B must inspect whether diffuse already composites grass over soil. If the mask is cutout foliage, packing RGB without it is correct for a ground layer; if RGB contains a chroma-key background, reject and substitute.

`rock_face_03` is the only new asset tagged `surface_use: wall`. That is the dedicated cliff face XV-F needs. Layer 2 stays the legacy cliff for migrated scenes.

### 3.2 Role failures — substitute before download

| Planned idx | Planned ID | Why it fails the intended role | Recommended substitute | Why |
|---:|---|---|---|---|
| 11 | `terrain_red_01` | First-party description is **coarse reddish gravel / crushed aggregate**, same class as layer 7 `gravel_floor` | **`cracked_red_ground`** | Category *Cracked Dry Mud & Clay*; 2.0 m; Amal Kumar; CC0; packer quartet at 2K/4K |
| 15 | `dry_riverbed_rock` | Category *Rough Rock Faces*; `surface_use: ground` but description is weathered **stone faces**, overlapping layer 14 | **`ganges_river_pebbles`** | Category *River Pebbles*; 2.16 m; Amal Kumar; 16K available; established download count |

Keep the failed IDs in the manifest `rejected_for_role` list with hashes so a later visual A/B can reopen them. Do not download them as shipping layers.

Substitute 4K Diffuse MD5s:

- `cracked_red_ground`: `1cf22d90a7cfff2bf9b31f94dd7545d8` (`files_hash` `e26ccacd19dd06f9f75ce04602ded00370a96b61`)
- `ganges_river_pebbles`: `1d8b5ac8e9a6900fdb743a210ae1005d` (`files_hash` `304428504a6db1d8ccb1e164f7bce3686086375f`)

Runner-up if `ganges_river_pebbles` fails visual audit: `river_small_rocks` (2.9 m, soil + broken stones, Tuytel) or newer `dry_river_pebbles` (2.0 m, exact name, low download count as of this access).

Runner-up if `cracked_red_ground` is too cracked for a general red-soil band: `red_dirt_mud_01` (1.5 m, compacted red mud + gravel, Tuytel).

### 3.3 Recommended shipping roster (indices 0–7 locked)

| Idx | Asset ID | Editor role |
|---:|---|---|
| 0–7 | unchanged | compatibility-locked |
| 8 | `aerial_sand` | Dry beach sand |
| 9 | `coast_sand_01` | Damp shoreline sand |
| 10 | `dry_mud_field_001` | Dry earth / topsoil |
| 11 | **`cracked_red_ground`** | Red mineral clay |
| 12 | `sparse_grass` | Sparse grass / exposed soil |
| 13 | `mossy_rock` | Wet/mossy mountain rock |
| 14 | `rock_face_03` | Rugged vertical cliff |
| 15 | **`ganges_river_pebbles`** | Talus / river stone |

---

## 4. ambientCG CC0 fallbacks

License: CC0 1.0, <https://docs.ambientcg.com/license/> (2026-08-13). Shortlinks `https://ambientcg.com/a/<Id>`. API v3: `https://ambientCG.com/api/v3/assets`. Prefer **surface-photogrammetry** over fully procedural. Dimensions on the API are millimetres when present.

| Role | Fallback ID | URL | Notes |
|---|---|---|---|
| Dry / beach sand | `Ground054` | https://ambientcg.com/a/Ground054 | Photogrammetry, ~3.5 m, tags beach/sand/dirt; 190k downloads |
| Damp sand | `Ground080` | https://ambientcg.com/a/Ground080 | Photogrammetry, beach/yellow sand |
| Forest / damp ground | `Ground037` | https://ambientcg.com/a/Ground037 | Photogrammetry, 2.1 m, moss/woodland |
| Mud / riverbed soil | `Ground106` | https://ambientcg.com/a/Ground106 | Photogrammetry, mud/riverbed/soil |
| Grass | `Grass005` | https://ambientcg.com/a/Grass005 | Short lawn; **procedural bitmap elements** — last resort |
| Mossy/cliff rock | `Rock063` | https://ambientcg.com/a/Rock063 | Photogrammetry, tags cliff/mossy/eroded (2026-02-22) |
| Snow | `Snow015` | https://ambientcg.com/a/Snow015 | Photogrammetry, dirty/melting snow |

Use Poly Haven first. ambientCG is only if a Poly Haven candidate fails the channel/seam/scale audit. XV-B fetcher should speak both APIs with the same User-Agent.

---

## 5. `materials.json` schema

Draft files (not yet `assets/terrain/materials.json`):

- [`materials.schema.json`](materials.schema.json)
- [`materials.draft.json`](materials.draft.json)

Schema rules:

- Manifest version integer; `sidecar_version` for terrain saves is 3 when sixteen layers ship.
- Each layer: stable `index` 0–15, stable string `id`, `display_name`, `role_tags`, `biome_tags`.
- Provenance: `source` (`polyhaven` \| `ambientcg` \| `legacy`), `page_url`, `license` + `license_url`, `authors`, `access_date`, `files_hash` (vendor listing hash), `physical_width_m`, `physical_height_m`, `colour_space`, `normal_convention`.
- Packer maps: `diff`, `nor_dx`, `arm`, `disp` with per-resolution `url`, `bytes`, `md5`. `sha256` filled only after a verified download.
- Processing: height range, UV multiplier, moisture_affinity `[0,1]`, cliff_suitability `[0,1]`, blend/parallax seeds from `LAYER_BLENDS` for 0–7.
- Fail closed: hash mismatch, missing required map, unknown license, resolution change.
- Generated 2K/4K pack output hashes are empty until XV-B.

Copy the draft into `assets/terrain/materials.json` only when implementation is authorized. Do not commit source JPGs.

---

## 6. Landscape-kit review matrix

Capture **after tonemapping**. Path: `dev records/phase XV/evidence/phase_XV-A_<id>.png`. None captured this session.

Lighting columns: **Noon** (default sun) and **Night** (`SOMNIUM_SUN_ELEVATION=-20`, moon 0.010 lux).  
Moisture columns: **Dry** (global wetness 0), **Damp** (~0.4), **Wet** (~0.85). v1 wetness is a global scalar × per-layer `moisture_affinity` (Hnat-style albedo darken + roughness drop + slight F0 lift). No painted wetness channel.

| ID | Viewpoint | Camera | Materials that must read | Required moisture | Notes |
|---|---|---|---|---|---|
| `overview` | Default landscape | §1.1 default | whole kit readable as regions | dry + wet | Identity of biomes, not albedo swatches |
| `shore_parity` | Great Lakes waterline | eye-height on beach | 9 → 8 → 4 (damp sand → dry sand → meadow) with **shipping water in frame** | dry, damp, wet | **Water-parity fixture.** Fail XV if sand looks like a poster next to IV-K water |
| `grass_soil` | Eye hillside | `SOMNIUM_TERRAIN_EYE=1` | 0, 4, 10, 12 | dry, wet | Grass vs earth scale |
| `forest` | Sheltered slope | eye | 1, 13 | damp, wet | Forest floor vs mossy rock |
| `lowland_mud` | Low basin | eye | 5, 10, 12 | dry vs wet | Mud must stay mud when wet |
| `red_clay` | Exposed bank | eye | 11 | dry, wet | Substitution check vs gravel |
| `mountain` | Steep transition | landscape + eye | 7, 15, 2, 14 | dry | Gravel → talus → legacy rock → cliff |
| `cliff_day` | Vertical face, glancing sun | close | 14, 13 | dry | No stretched albedo; full PBR (XV-F) |
| `cliff_night_wet` | Same cliff | close | 14 | wet + night | Specular identity |
| `snowline` | High elevation | landscape | 3 vs 2/14 | dry | Soft snow edge |
| `junction_4` | Painted four-way | close | any four | dry | Strongest-four vs popping (XV-D) |
| `minify` | Distant + moving camera | overview path | all | dry | Mip/Toksvig (XV-E) |
| `migrated_v2` | Old eight-layer sidecar | default | 0–7 only | dry | Byte-identical look after v3 (XV-C) |

Solo-layer captures (one material painted 255) for all sixteen under noon dry are required before XV-J so the kit can be reviewed as Bethesda-style reusable blocks.

---

## 7. Fetch / pack implications for XV-B

Current `tools/fetch_terrain_textures.sh` hardcodes eight names and URL pattern  
`https://dl.polyhaven.org/file/ph-assets/Textures/jpg/${RES}/${mat}/${mat}_${map}_${RES}.jpg`  
with maps `diff nor_dx arm disp`. That pattern still matches every audited asset, including substitutes. Replace the hardcoded list with the manifest; add User-Agent; verify MD5 then SHA-256; fail closed.

`pack_terrain.rs` likewise hardcodes eight names and box-filters encoded bytes. XV-B adds semantic mips (linear albedo, renormalized normals, Toksvig roughness) and sixteen outputs.

Poly Haven `/files` MD5 is the **vendor** hash. Manifest stores it now; SHA-256 is computed locally after download and becomes the fail-closed runtime check.

---

## 8. XV-A exit criteria

| Criterion | Result |
|---|---|
| Reproducible baseline report | **This file** freezes cameras, tiling, residency math, and historical timings. Live adapter/GPU/tap capture is **deferred** to the first authorized engine run. |
| Every candidate has first-party source/license | **Yes** — CC0 Poly Haven pages + `/info` + `/files` MD5s dated 2026-08-13. Two role substitutions recorded with hashes. |
| No new texture binary without a manifest entry | **Yes** — nothing downloaded. Packed 0–7 were already committed. |

**Not done (needs implementation authorization):** `assets/terrain/materials.json` installed as the engine source of truth; GPU baseline PNGs; SHA-256; any fetch/pack/shader work.

---

## 9. Sources accessed 2026-08-13

- Poly Haven license: <https://polyhaven.com/license>
- Poly Haven API: <https://polyhaven.com/el/our-api> — unique User-Agent required; API-service attribution required; assets remain CC0
- Poly Haven `/info/{id}` and `/files/{id}` for layers 0–15 plus `cracked_red_ground`, `ganges_river_pebbles`, `red_laterite_soil_stones`, `red_dirt_mud_01`, `red_mud_stones`, `dry_river_pebbles`, `river_small_rocks`
- ambientCG license: <https://docs.ambientcg.com/license/>
- ambientCG API v3: <https://docs.ambientcg.com/api/v3/assets/>
- CC0 1.0: <https://creativecommons.org/publicdomain/zero/1.0/>
