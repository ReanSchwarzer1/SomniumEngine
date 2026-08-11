# Great Lakes terrain derivatives

These runtime files were baked from the Motion Forge Pictures **Great Lakes
Height Map** by Chris J Mitchell. The original OpenEXR files are not committed.

- Catalog: <https://www.motionforgepictures.com/height-maps/>
- Asset page: <https://www.motionforgepictures.com/sdm_downloads/great-lakes-height-map/>
- Accessed: 2026-08-11
- Asset-page license statement: CC0 1.0 Universal (written as “CCO” on the
  catalog page)
- License text: <https://creativecommons.org/publicdomain/zero/1.0/legalcode.en>

Motion Forge's general site terms are less permissive than the height-map
catalog's specific CC0 statement. Somnium relies on the specific grant and
keeps this provenance record because the page uses the `CCO` typo. Attribution
is supplied voluntarily even though CC0 does not require it.

## Audited source files

| File | SHA-256 |
|---|---|
| `Height Map.exr` | `d608ec2e62a40e38ff3a65180c6e017b14422496920a1f517d9aa691e2f252b9` |
| `Diffuse Map.exr` | `45cc8c1e4a2698ff01de2a441e8ad2cf822bf4bae29dfb95dc9a7a20a38dce17` |
| `Great Lakes.png` | `7fad06f049f7503ea518a435d03833483ebfc0b539ffdebcccfc7b8c962add77` |

## Runtime derivatives

| File | Encoding | SHA-256 |
|---|---|---|
| `height.png` | 1025×1025, 16-bit height samples | `90325eb716efc1f5c7e98da291eab8a33a694e63b8f20c45bd8735336c5d5842` |
| `macro_color.png` | 512×512 sRGB RGBA; water alpha is zero | `ea07cbabff6b87b320af416cf2fee8237c5191e9addce621b96e4d6444f5d089` |
| `water_mask.png` | 2048×2048, 8-bit wet/dry mask | `d832a6a8b21a0846a0b843423e6789a10363a422fd2aa9473eeafc241764fec8` |
| `water_depth.png` | 2048×2048, 16-bit normalized 0–12 m depth | `3bdb89cb735c0aeee1fa38ce386ec82221bf211d74a9b25dc51dfa059efccedd` |
| `shore_sdf.png` | 2048×2048, signed shore distance encoded over ±128 cells | `9b46d9cf87408cdc003bf7afba3f08e9e0e0f478595e0811957d8c44d3a17312` |

`recipe.json` records the audited float range, plateau tolerance, water datum,
bathymetry settings, output dimensions, and source hashes. The bake makes dry
terrain at least 0.35 m higher than the 15 m water datum, so no terrain/water
surface remains coplanar. Selected lake plateaus receive a smooth synthetic bed
up to 12 m deep. Phase IV-I retains the source's full 2048² wet/dry contour
instead of majority-downsampling it to 1024²; the water shader reconstructs a
bilinear zero contour from the SDF, giving the shoreline 0.5 m authored samples
plus screen-space antialiasing.

Rebuild deterministically from the downloaded source directory:

```powershell
cargo run --release -p somnium_asset --example bake_great_lakes -- `
  "C:\Users\adhir\Downloads\Great Lakes" `
  "assets\terrain\great_lakes"
```
