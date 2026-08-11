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
| `height.png` | 1025×1025, 16-bit height samples | `737062867d6af1a153df69f41cdb258296f9a2c759e49bf59a7d6852489f768f` |
| `macro_color.png` | 512×512 sRGB RGBA; water alpha is zero | `b60371ec104c13bdcff143ea2d3312e3b42d13324b05192c02fef2d780d3485c` |
| `water_mask.png` | 1024×1024, 8-bit wet/dry mask | `3e237185204389989d107430fb5a00d22818ed9d8fee72e0210e638e4fab66f0` |
| `water_depth.png` | 1024×1024, 16-bit normalized 0–12 m depth | `585bcfecbdbdd5e6aa02ea4a8e2f3478b3393596028f2f08717cabf1dab82a9c` |
| `shore_sdf.png` | 1024×1024, signed shore distance encoded over ±128 cells | `5ec68ca47a1de0067b4f10f119d10935b51b4026f1dc54c698f1506ec6398085` |

`recipe.json` records the audited float range, plateau tolerance, water datum,
bathymetry settings, output dimensions, and source hashes. The bake makes dry
terrain at least 0.35 m higher than the 15 m water datum, so no terrain/water
surface remains coplanar. Selected lake plateaus receive a smooth synthetic bed
up to 12 m deep.

Rebuild deterministically from the downloaded source directory:

```powershell
cargo run --release -p somnium_asset --example bake_great_lakes -- `
  "C:\Users\adhir\Downloads\Great Lakes" `
  "assets\terrain\great_lakes"
```
