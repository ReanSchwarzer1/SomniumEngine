# CONTROL-C — asset seam verification (2026-08-23)

Implemented: immutable queried `AssetDbSnapshot`; two-sample debounce and
mtime/content-hash invalidation; bounded priority/FIFO cancellable jobs with
progress and status-bar Cancel; off-thread texture/mesh previews; visible-range
promotion; a 750 microsecond atlas-apply budget; warm hash-addressed disk cache;
frequency-declared generator fall-through; drawer search/sort/chips/history/
breadcrumbs/multi-select/in-place rename; and the schema Asset picker with
search, thumbnails, mask filtering, None, Edit, Locate and Make Unique.

## Deterministic gates

```text
cargo test -p somnium_asset --lib -j1       15 passed
cargo test -p somnium_ui --lib -j1          225 passed
cargo test -p somnium_core jobs::tests -j1   4 passed
```

`thumbnail::tests::sixty_png_frame_contract_performs_zero_ui_thread_decodes`
queues 60 PNGs, proves the frame pump decodes zero, and drains the eight visible
requests first. Separate tests cover scroll promotion/deduplication, LRU,
second-run cache hits, touched-file hash stability, debounce, and mesh crop.

No post-C live 1280x720 `.somtime` capture was taken, so this record does not
claim a measured live redline. It proves deterministically that the former
232–260 ms PNG inflate cannot run on the UI thread. A future hardware capture
should compare with `CONTROL-A_terrain_open.somtime`.
