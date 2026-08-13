#!/usr/bin/env bash
# Phase XV-B — download CC0 terrain source textures from Poly Haven.
#
# Prefer the Rust fetcher, which reads assets/terrain/materials.json, sends
# User-Agent SomniumEngine-terrain-fetch/XV, and fail-closes on MD5:
#
#   cargo run --release -p somnium_asset --example fetch_terrain -- 4k
#
# This script is a thin convenience wrapper for Unix shells. It does **not**
# verify hashes — use the Rust example for a shipping fetch.
#
#   ./tools/fetch_terrain_textures.sh [resolution] [staging_dir]
#
# Everything here is CC0 <https://polyhaven.com/license>. Credit in
# assets/LICENSE.md. Rejected IDs terrain_red_01 and dry_riverbed_rock are
# not fetched.
set -euo pipefail

RES="${1:-4k}"
STAGING="${2:-assets/terrain/_source}"
BASE="https://dl.polyhaven.org/file/ph-assets/Textures/jpg/${RES}"
UA="SomniumEngine-terrain-fetch/XV"

MATERIALS=(
    aerial_grass_rock
    forrest_ground_01
    aerial_rocks_04
    snow_02
    leafy_grass
    brown_mud
    coast_sand_rocks_02
    gravel_floor
    aerial_sand
    coast_sand_01
    dry_mud_field_001
    cracked_red_ground
    sparse_grass
    mossy_rock
    rock_face_03
    ganges_river_pebbles
)

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
        curl -sS --fail --location --max-time 300 \
            -A "$UA" \
            -o "$out" "${BASE}/${mat}/${name}"
        total=$((total + 1))
    done
done

echo
echo "downloaded ${total} new file(s) into ${STAGING}"
echo "hashes were NOT verified — run the Rust fetch_terrain example for MD5/SHA-256"
du -sh "$STAGING"
echo "next: cargo run --release -p somnium_asset --example pack_terrain -- ${RES}"
