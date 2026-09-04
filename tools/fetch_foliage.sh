#!/usr/bin/env bash
# Phase TSUSHIMA-I — download the CC0 nature models the foliage palette points
# at.
#
#   ./tools/fetch_foliage.sh [resolution] [asset ...]    # default 2k, all
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
#
# # Why the manifest arrives as three lines per file and not one
#
# The obvious shape is one tab-separated line per file read with `IFS`. It was
# written that way and it destroyed data: Python emits CRLF on Windows, `read`
# leaves the CR on the last field, the last field is the MD5 — so every hash
# compared unequal, every already-correct file was re-downloaded, and every
# fresh download was then deleted as corrupt. It took the four committed
# Phase 17E assets with it before anyone read the log.
#
# The fix is not a `tr` in the pipeline. Any fix spelled with a backslash escape
# is one careless edit away from breaking the same way and just as quietly.
# `tools/polyhaven_files.py` writes three plain lines per record in binary —
# path, URL, MD5 — which have no delimiter to get wrong.
#
# # Keeping this in step with the palette
#
# The list below must match `FOLIAGE_PALETTE` in `crates/somnium_core/src/app.rs`.
# `the_palette_matches_the_fetch_script` reads both and fails if they drift,
# because the failure mode otherwise is a palette entry that warns "not
# installed" forever and nobody knows whether that is a missing download or a
# typo.
set -euo pipefail

RES="${1:-2k}"
shift || true
DEST="assets/foliage"
UA="SomniumEngine-foliage-fetch/TSUSHIMA"
API="https://api.polyhaven.com/files"
HERE="$(cd "$(dirname "$0")" && pwd)"

ALL=(
    # Phase 17E, already in the repository. Listed so the script can repair one.
    grass_medium_01
    grass_bermuda_01
    fir_sapling
    island_tree_02
    # TSUSHIMA-I: grass, flowers and ground cover.
    grass_medium_02
    moss_01
    fern_02
    shrub_02
    shrub_03
    nettle_plant
    dandelion_01
    # Rocks and debris, pebble scale upward.
    namaqualand_stones_01
    rock_moss_set_01
    rock_moss_set_02
    rock_07
    rock_09
    stone_01
    namaqualand_boulder_03
    # Cliff faces. Big single props, placed one at a time.
    rock_face_01
    rock_face_02
    namaqualand_cliff_01
    # Trees and deadwood.
    quiver_tree_01
    tree_stump_01
    dead_tree_trunk
    dry_branches_medium_01
)

if [ "$#" -gt 0 ]; then
    ASSETS=("$@")
else
    ASSETS=("${ALL[@]}")
fi

if command -v python3 >/dev/null 2>&1; then PY=python3; else PY=python; fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

failed=0
for asset in "${ASSETS[@]}"; do
    echo "== $asset ($RES)"
    if ! curl -fsSL -A "$UA" "$API/$asset" -o "$WORK/manifest.json"; then
        echo "   manifest fetch failed" >&2
        failed=$((failed + 1))
        continue
    fi
    if ! "$PY" "$HERE/polyhaven_files.py" \
        "$WORK/manifest.json" "$RES" "$asset" "$WORK/records"; then
        failed=$((failed + 1))
        continue
    fi

    while IFS= read -r rel && IFS= read -r url && IFS= read -r md5; do
        [ -n "$rel" ] || continue
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
        # Download beside the target and move it into place only once the hash
        # agrees. Writing straight to the target means a bad download has
        # already destroyed whatever was there before anything checks it.
        tmp="$out.part"
        if ! curl -fsSL -A "$UA" -o "$tmp" "$url"; then
            echo "   download failed: $rel" >&2
            rm -f "$tmp"
            failed=$((failed + 1))
            continue
        fi
        # Fail closed. A truncated download that still parses is a mesh with a
        # hole in it, and that is much harder to notice than a missing file.
        if [ -n "$md5" ]; then
            have=$(md5sum "$tmp" | cut -d' ' -f1)
            if [ "$have" != "$md5" ]; then
                echo "   MD5 mismatch for $rel: got $have, want $md5" >&2
                rm -f "$tmp"
                failed=$((failed + 1))
                continue
            fi
        fi
        mv -f "$tmp" "$out"
    done < "$WORK/records"
done

echo
if [ "$failed" -gt 0 ]; then
    echo "$failed file(s) had problems." >&2
    exit 1
fi
echo "Done. Every palette entry will load on next paint."
