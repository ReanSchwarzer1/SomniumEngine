# Asset Licenses

## test_scene.glb

**Asset:** Damaged Helmet (`DamagedHelmet.glb`)  
**Author:** theblueturtle_  
**License:** Creative Commons Attribution-NonCommercial 4.0 International (CC-BY-NC 4.0)  
**Source:** [Khronos glTF-Sample-Assets](https://github.com/KhronosGroup/glTF-Sample-Assets)  
**File used as:** `assets/test_scene.glb`

### How to obtain

1. Download `DamagedHelmet.glb` from the Khronos glTF Sample Assets repository:  
   `glTF-Sample-Assets/Models/DamagedHelmet/glTF-Binary/DamagedHelmet.glb`

2. Copy it to this directory and rename it `test_scene.glb`:
   ```
   assets/test_scene.glb
   ```

Any CC-BY-4.0 or CC0 glTF 2.0 `.glb` file may be placed here under the same filename.  
The engine will fall back to a procedural cube scene if the file is absent.

## Attribution requirement (CC-BY-NC)

If you redistribute a build that includes `DamagedHelmet.glb`, credit must be given to the
original author as specified by the CC-BY-NC 4.0 license terms.  See the full license at
<https://creativecommons.org/licenses/by-nc/4.0/>.

## assets/terrain/ — terrain materials (Phase 25K / XV)

**License:** CC0 1.0 Universal (public domain dedication)
**Source:** [Poly Haven](https://polyhaven.com/textures) — <https://polyhaven.com/license>
**Manifest:** [`terrain/materials.json`](terrain/materials.json)

CC0 imposes no attribution requirement; the credit below is given anyway, and
the licence is the reason this source was chosen over commercial libraries.

Sixteen shipping layers (indices 0–7 compatibility-locked from Phase 25K):

| # | id | display | authors |
|---|----|---------|---------|
| 0 | `aerial_grass_rock` | Grass | Rob Tuytel |
| 1 | `forrest_ground_01` | Forest Floor | Rob Tuytel |
| 2 | `aerial_rocks_04` | Rock (legacy cliff) | Rob Tuytel |
| 3 | `snow_02` | Snow | Rob Tuytel |
| 4 | `leafy_grass` | Meadow | Charlotte Baglioni |
| 5 | `brown_mud` | Mud | Rob Tuytel |
| 6 | `coast_sand_rocks_02` | Sand | Rob Tuytel |
| 7 | `gravel_floor` | Gravel | Matterfield (photography), Jenelle van Heerden (processing) |
| 8 | `aerial_sand` | Dry Beach Sand | Rob Tuytel |
| 9 | `coast_sand_01` | Damp Shoreline Sand | Rob Tuytel |
| 10 | `dry_mud_field_001` | Dry Earth | Rob Tuytel (photography), Rico Cilliers (processing) |
| 11 | `cracked_red_ground` | Red Mineral Soil | Amal Kumar |
| 12 | `sparse_grass` | Sparse Grass | Amal Kumar |
| 13 | `mossy_rock` | Mossy Rock | Rob Tuytel |
| 14 | `rock_face_03` | Cliff Face | Dario Barresi (photography), Rico Cilliers (processing) |
| 15 | `ganges_river_pebbles` | Talus / River Stone | Amal Kumar |

`terrain_red_01` and `dry_riverbed_rock` were rejected for role overlap and are
not shipping layers. Substitutes are `cracked_red_ground` and
`ganges_river_pebbles`. See `dev records/phase XV/XV-A_research.md`.

`aerial_rocks_04` remains the same texture the bgfx hex-tile example ships
with, so Phase 25F can still be judged against its own reference. Layer 14 is
the dedicated XV-F cliff face; layer 2 keeps its legacy meaning.

### How to obtain

The committed `*_albedo.png` / `*_surface.png` pairs are channel-packed and are
what the engine loads. To regenerate them from source:

```
cargo run --release -p somnium_asset --example fetch_terrain -- 4k
cargo run --release -p somnium_asset --example pack_terrain -- 4k
```

Pass `2k` to both steps for a quarter of the size at terrain viewing distances.
`_source/` is git-ignored — only the packed result is committed. The fetcher
fail-closes on MD5 mismatch and writes SHA-256 into `_source/FETCH_REPORT.json`.

| packed texture   | R        | G        | B         | A      |
|------------------|----------|----------|-----------|--------|
| `*_albedo.png`   | albedo R | albedo G | albedo B  | height |
| `*_surface.png`  | normal X | normal Y | roughness | AO     |

Normal Z is reconstructed in the shader; metalness is dropped because terrain
layers are dielectric. Runtime default is 2K (`SOMNIUM_TERRAIN_RES`); committed
packs are 4K. Semantic mips are generated at load.

## assets/terrain/great_lakes/ — default landscape (Phase IV-B)

**Asset:** Great Lakes Height Map
**Author:** Chris J Mitchell / Motion Forge Pictures
**License:** CC0 1.0 Universal, per the asset-specific height-map catalog statement
**Source:** <https://www.motionforgepictures.com/height-maps/> and
<https://www.motionforgepictures.com/sdm_downloads/great-lakes-height-map/>

Somnium commits only deterministic runtime derivatives, not the downloaded
OpenEXR package. Full source and output hashes, transformations, encodings, and
the note about the catalog's `CCO` typo versus the general site terms are kept in
[`great_lakes/README.md`](terrain/great_lakes/README.md). Credit is given
voluntarily.

## assets/terrain/heightmap.tbmp — terrain heightmap (Phase 25L)

**Asset:** `TestData/maintestdata/heightmap.tbmp` (4096×2048, 16-bit)
**Author:** Filip Strugar
**License:** MIT — Copyright (c) 2010 Filip Strugar
**Source:** [CDLOD](https://github.com/fstrugar/CDLOD)

This was the demo default through Phase 25M-2. Phase IV-B replaced it with the
Great Lakes derivatives above, but the file remains as a legacy/regression
heightmap. MIT requires the copyright notice above to travel with the file,
which is why it remains recorded here.

CDLOD is also the reference the terrain's chunked LOD scheme came from in
Phase 14 and the `.tbmp` decoder in Phase 25L — see ATTRIBUTION.md.

Override with `SOMNIUM_HEIGHTMAP=<path>` (16-bit PNG, any decodable image, or
another `.tbmp`). With the file absent the engine generates procedural FBM
relief instead, so the scene still has landscape.

## assets/models/gislinge_viking_boat/ - default water-interaction vessel (Phase IV-I)

- **Model:** Gislinge Viking Boat
- **Author:** Opus Poly
- **License:** Creative Commons Attribution 4.0 International (CC BY 4.0)
- **Original:** <https://sketchfab.com/3d-models/gislinge-viking-boat-01098ad7973647a9b558f41d2ebc5193>
- **License text:** <https://creativecommons.org/licenses/by/4.0/>

Attribution: “Gislinge Viking Boat” by Opus Poly, licensed under CC BY 4.0.
The original GLB and its embedded materials/textures are distributed unchanged.
Somnium applies a runtime centimetre-to-metre scale and keeps the 29,035
triangle render hierarchy separate from its low-frequency buoyancy proxy.
The exact source hash and integration notes are in
[`models/gislinge_viking_boat/README.md`](models/gislinge_viking_boat/README.md).
