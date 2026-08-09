#!/usr/bin/env bash
# Phase 25K — download the CC0 terrain source textures from Poly Haven.
#
# Downloads four maps per material into a staging directory. The engine does not
# read these directly: `cargo run -p somnium_asset --example pack_terrain`
# channel-packs them into the two textures per material that the terrain shader
# actually samples. See assets/LICENSE.md.
#
#   ./tools/fetch_terrain_textures.sh [resolution] [staging_dir]
#
# Resolution defaults to 4k (Poly Haven also serves 8k, 2k, 1k).
#
# Everything here is CC0 <https://polyhaven.com/license> — no attribution is
# legally required, which is the whole reason this source was chosen over the
# larger commercial libraries. It is credited in assets/LICENSE.md regardless.
set -euo pipefail

RES="${1:-4k}"
STAGING="${2:-assets/terrain/_source}"
BASE="https://dl.polyhaven.org/file/ph-assets/Textures/jpg/${RES}"

# Eight materials covering what a terrain is actually made of. Two grasses
# because a single grass tiled over a kilometre is the artefact this phase
# exists to remove, and `aerial_rocks_04` because it is the texture the bgfx
# hex-tile demo ships with — so 25F can be judged against its own reference.
MATERIALS=(
    aerial_grass_rock    # grass over rock, the default ground
    leafy_grass          # second grass, coarser
    forrest_ground_01    # forest floor
    brown_mud            # wet soil
    aerial_rocks_04      # rock, and the cliff layer
    snow_02              # snow
    coast_sand_rocks_02  # sand
    gravel_floor         # gravel
)

# Poly Haven's suffixes. `arm` is ambient-occlusion / roughness / metalness
# packed into RGB, which the engine already understands by filename convention
# from Phase 17I. `disp` is the height map that Phase 25E needs and that the
# procedural layers could only fake in the albedo's alpha channel.
MAPS=(diff nor_dx arm disp)

mkdir -p "$STAGING"
total=0
for mat in "${MATERIALS[@]}"; do
    for map in "${MAPS[@]}"; do
        name="${mat}_${map}_${RES}.jpg"
        out="${STAGING}/${name}"
        if [ -s "$out" ]; then
            echo "have  ${name}"
            continue
        fi
        echo "get   ${name}"
        # --fail so a 404 is an error rather than an HTML file saved as a jpg,
        # which would otherwise surface much later as a decode failure.
        curl -sS --fail --location --max-time 300 -o "$out" "${BASE}/${mat}/${name}"
        total=$((total + 1))
    done
done

echo
echo "downloaded ${total} new file(s) into ${STAGING}"
du -sh "$STAGING"
echo "next: cargo run --release -p somnium_asset --example pack_terrain"
