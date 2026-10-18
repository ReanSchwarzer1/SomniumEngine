#!/usr/bin/env bash
# Both games stay whole: run after ANY engine change, Rimeborn change or TSF change (the developer, 18 Oct 2026:
# no engine or Rimeborn change may interfere with TSF, and no TSF change with Rimeborn).
#
#   bash tools/check_games.sh            # from the main checkout (games/ is git-ignored, so worktrees lack it)
#   JOBS=1 bash tools/check_games.sh     # if OneDrive locks the linker (LNK1104)
#
# Read-only for TSF, and checked: TSF's own tests write into its folder (the tutorials-seen save in saves/, the asset
# index cache, a document fixture under evidence/), so the fixture test is skipped, the other two files are backed up
# before TSF's tests and put back byte for byte after, and any other TSF file the run changes fails the check by
# name. Its ignored audits (they write evidence) stay ignored. Exit 0 only if everything passes.
set -u
cd "$(dirname "$0")/.."
JOBS="${JOBS:-4}"
status=0
run() {
    echo "== $1"
    shift
    local log
    log=$(mktemp)
    local ok=1
    "$@" >"$log" 2>&1 || ok=0
    if [ "$ok" -eq 0 ] && grep -q "1104" "$log"; then
        # LNK1104: OneDrive held a file the linker wanted (memory note: use -j 1). Once more, single job.
        local args=() prev=""
        for a in "$@"; do if [ "$prev" = "-j" ]; then args+=("1"); else args+=("$a"); fi; prev="$a"; done
        echo "   (linker lock; retrying with -j 1)"
        ok=1
        "${args[@]}" >"$log" 2>&1 || ok=0
    fi
    if [ "$ok" -eq 1 ]; then
        grep -E "^test result" "$log" | head -3
    else
        grep -E "^error|FAILED|panicked|^test result" "$log" | head -40
        echo "!! FAILED: $*"
        status=1
    fi
    rm -f "$log"
}
run "engine core" cargo test -p somnium_core -j "$JOBS" --quiet
run "engine workspace builds" cargo check --workspace --all-targets -j "$JOBS" --quiet
TSF=games/TheSomnusFracture
KEEP=(saves/player-tutorials.somsave assets/.somnium/asset-index.json Cargo.lock)
backup=$(mktemp -d)
for f in "${KEEP[@]}"; do
    if [ -e "$TSF/$f" ]; then mkdir -p "$backup/$(dirname "$f")" && cp -p "$TSF/$f" "$backup/$f"; fi
done
marker="$backup/.marker"
touch "$marker"
run "TSF" cargo test --manifest-path "$TSF/Cargo.toml" --target-dir target -j "$JOBS" --quiet -- \
    --skip private_component_document_draft_and_conflicts_share_live_schemas
changed=$(find "$TSF" -type f -newer "$marker" -not -path "*/target/*" 2>/dev/null \
    | grep -v -F -e "$TSF/saves/player-tutorials.somsave" -e "$TSF/assets/.somnium/asset-index.json" -e "$TSF/Cargo.lock")
if [ -n "$changed" ]; then
    # Something else in TSF changed while its tests ran: a TSF session (or the developer) is working in it. Its writes
    # may include the two files below, so nothing is put back; look before landing.
    echo "!! FAILED: TSF files changed during the check (a TSF session may be working; nothing was put back):"
    echo "$changed" | head -20
    status=1
else
    for f in "${KEEP[@]}"; do
        [ -e "$backup/$f" ] || continue
        if ! cmp -s "$backup/$f" "$TSF/$f"; then
            echo "   (put back TSF $f, which its own tests rewrote)"
        fi
        cp -p "$backup/$f" "$TSF/$f"
    done
fi
rm -rf "$backup"
run "Rimeborn" cargo test --manifest-path games/Rimeborn/Cargo.toml --target-dir target -j "$JOBS" --quiet
[ "$status" -eq 0 ] && echo "== both games whole" || echo "== SOMETHING BROKE: fix it before landing"
exit "$status"
