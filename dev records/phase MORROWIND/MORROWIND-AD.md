# MORROWIND-AD — terrain virtual texturing

Completed 2026-08-28.

## Result

Terrain material sources can now be created in a streaming mode. The renderer
seeks 128×128 BC7 pages directly from the existing albedo/height and
normal/roughness/occlusion mip packs, installs each pair into a deterministic
2,048-slot LRU, and publishes one page-table entry only after both channels are
ready. The physical atlases are 64×32 pages and total exactly 64 MiB.

The existing Phase DF terrain composition clipmap is the runtime virtual
texture. Its dirty rectangles form the feedback pass: their toroidal footprint
selects splatmap layers, target mip, and coarser fallback mips. Feedback is
deduplicated, pending work survives later empty frames, uploads obey the
authored per-frame throttle, and page arrival invalidates the composition cache
for regeneration. A failed source read restores evicted ownership and requeues
the failed batch.

`GpuTerrainMaterial` remains exactly 2,032 bytes. The three bindless resource
ids and atlas width travel in the clipmap-generation uniform instead. Existing
terrains default to resident source arrays; streaming allocation is selected at
terrain creation, which is why **Stream Source Pages** and **Cache Budget** are
read-only in generated Details. **Uploads Per Frame** remains editable and the
panel reports resident/pending pages, hits, misses, and evictions.

`vvardenfell` exercises the public path with a real generated 512 m island and
the 64 MiB cache. No new archive format was added: the shipped BC7 mip packs are
already deterministic random-access sources.

## Verification

- `cargo fmt --all --check`
- `cargo check -p somnium_renderer -p somnium_core -p vvardenfell`
- `cargo test -p somnium_asset virtual_texture --lib`
- `cargo test -p somnium_renderer terrain::virtual_texture::tests --lib`
- shader validation for `clipmap_gen.wgsl` and the frozen terrain-material ABI
- Terrain Details schema/migration and scene-serialization tests
- workspace library test gate and GHOSTFENCE (recorded in the completing commit)

The sub-phase does not claim a new visual golden or performance win. It changes
source residency for the already-shipped composition path; the acceptance claim
is bounded allocation, deterministic streaming, fallback/retry correctness, and
an exercised public example.
