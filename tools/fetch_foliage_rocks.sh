#!/usr/bin/env bash
# Phase TSUSHIMA-I — download the CC0 debris models the foliage palette's last
# two entries point at.
#
#   ./tools/fetch_foliage_rocks.sh [resolution]     # default 2k
#
# These are **not** committed. The four Phase 17E entries are, at about 101 MB;
# adding to that on every clone for two entries most projects will replace with
# their own content is the wrong default, and `ensure_palette_mesh` already
# degrades to one warning and no placement when a path is missing.
#
# Everything here is CC0 <https://polyhaven.com/license>. Credit in
# ATTRIBUTION.md §13.25.
#
# # Why the file API rather than a URL pattern
#
# The textures do not sit beside the glTF, and the `.bin` is shared from the 8k
# variant regardless of the resolution asked for. `tools/fetch_terrain_textures.sh`
# can build its URLs by hand because textures are a flat namespace; models are
# not, so the `include` map has to be followed. Guessing the layout gets a glTF
# that loads and renders untextured.
set -euo pipefail

RES="${1:-2k}"
DEST="assets/foliage"
UA="SomniumEngine-foliage-fetch/TSUSHIMA"
API="https://api.polyhaven.com/files"

# Matched to `FOLIAGE_PALETTE` in crates/somnium_core/src/app.rs. Changing one
# without the other gives a palette entry that warns forever.
ASSETS=(
    namaqualand_stones_01
    rock_moss_set_01
)

command -v python3 >/dev/null 2>&1 && PY=python3 || PY=python

for asset in "${ASSETS[@]}"; do
    echo "== $asset ($RES)"
    manifest=$(curl -fsSL -A "$UA" "$API/$asset")
    # One pass over the manifest emitting "<relative path>\t<url>\t<md5>" lines,
    # the glTF itself first. Parsing JSON in the shell is how a fetch script
    # ends up silently skipping a texture whose name contains a bracket.
    printf '%s' "$manifest" | "$PY" -c "
import sys, json
res = sys.argv[1]
asset = sys.argv[2]
d = json.load(sys.stdin)
try:
    g = d['gltf'][res]['gltf']
except KeyError:
    sys.exit(f'{asset}: no glTF at {res}')
rows = [(f'{asset}_{res}.gltf', g['url'], g.get('md5', ''))]
rows += [(k, v['url'], v.get('md5', '')) for k, v in g['include'].items()]
for r in rows:
    print('\t'.join(r))
" "$RES" "$asset" | while IFS=$'\t' read -r rel url md5; do
        out="$DEST/$asset/$rel"
        mkdir -p "$(dirname "$out")"
        if [ -f "$out" ] && [ -n "$md5" ]; then
            have=$(md5sum "$out" 2>/dev/null | cut -d' ' -f1 || true)
            if [ "$have" = "$md5" ]; then
                echo "   ok   $rel"
                continue
            fi
        fi
        echo "   get  $rel"
        curl -fsSL -A "$UA" -o "$out" "$url"
        # Fail closed. A truncated download that still parses is a mesh with a
        # hole in it, and that is much harder to notice than a missing file.
        if [ -n "$md5" ]; then
            have=$(md5sum "$out" | cut -d' ' -f1)
            if [ "$have" != "$md5" ]; then
                echo "   MD5 mismatch for $rel: got $have, want $md5" >&2
                rm -f "$out"
                exit 1
            fi
        fi
    done
done

echo
echo "Done. The Pebbles and Scree Rocks palette entries will load on next paint."
