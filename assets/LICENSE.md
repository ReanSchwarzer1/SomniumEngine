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

## assets/terrain/ — terrain materials (Phase 25K)

**Assets:** `aerial_grass_rock`, `leafy_grass`, `forrest_ground_01`, `brown_mud`,
`aerial_rocks_04`, `snow_02`, `coast_sand_rocks_02`, `gravel_floor`
**Author:** Rob Tuytel, Rico Cilliers and contributors
**License:** CC0 1.0 Universal (public domain dedication)
**Source:** [Poly Haven](https://polyhaven.com/textures) — <https://polyhaven.com/license>

CC0 imposes no attribution requirement; the credit above is given anyway, and the
licence is the reason this source was chosen over the larger commercial libraries.

`aerial_rocks_04` is deliberately the same texture the bgfx hex-tile example
ships with, so Phase 25F can be judged against the material its own reference
was tuned on.

### How to obtain

The committed `*_albedo.png` / `*_surface.png` pairs are channel-packed and are
what the engine loads. To regenerate them from source:

```
./tools/fetch_terrain_textures.sh 4k     # downloads 32 maps (~300 MB) to _source/
cargo run --release -p somnium_asset --example pack_terrain
```

Pass `2k` to both steps for a quarter of the size at terrain viewing distances.
`_source/` is git-ignored — only the packed result is committed.

| packed texture   | R        | G        | B         | A      |
|------------------|----------|----------|-----------|--------|
| `*_albedo.png`   | albedo R | albedo G | albedo B  | height |
| `*_surface.png`  | normal X | normal Y | roughness | AO     |

Normal Z is reconstructed in the shader; metalness is dropped because terrain
layers are dielectric.

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
