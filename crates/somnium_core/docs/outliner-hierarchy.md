# Outliner hierarchy cost

The editor previously rebuilt its sorted hierarchy every frame, even while the
world was unchanged. Its sort allocated two lowercase strings per comparison;
the following name map, tree walk, row facts, and UI snapshot made further copies.
`World::find_entity_by_index` was checked and is already constant time.

`outliner_hierarchy::OutlinerHierarchy` retains the flattened rows and their base
names. Every frame compares live entity index, generation, optional name, and parent
against the retained inputs. This linear validation does not depend on editor
commands, so direct game writes, undo, creation/deletion, reparenting and renaming
still take effect on the next frame. A changed hierarchy uses one lowercase sort
key per entity and preserves the previous stable ASCII-insensitive ordering.

Hidden/locked flags, component tags, prefab decorations, selection and Details
remain live each frame. Retained rows reset their decorations and reuse string/tag
capacity before those facts are read. The UI compares borrowed rows before cloning
a changed snapshot. The cache does not change filtering, collapsed state, hierarchy
depth saturation, or the existing handling of missing parents and rootless cycles.

## Bounded measurement, 2026-09-23

The ignored `benchmark_unchanged_outliner_hierarchy` test exercises the production
hierarchy cache and the previous algorithm retained as an equality oracle. It uses
synthetic mixed root/child lists with shuffled names, 100 iterations per size, and
asserts identical initial rows. A standalone `rustc --test` harness includes the
production module directly and the exact current `OutlinerRow` definition, avoiding
a second Cargo build while the native editor is running.

| Rows | Previous, opt-level 2 | Retained, opt-level 2 | Previous, opt-level 0 | Retained, opt-level 0 |
| ---: | ---: | ---: | ---: | ---: |
| 1,340 | 1.465 ms | 0.012 ms | 5.926 ms | 0.045 ms |
| 4,294 | 5.628 ms | 0.039 ms | 22.307 ms | 0.148 ms |
| 5,611 | 6.726 ms | 0.051 ms | 29.352 ms | 0.204 ms |

These are warm-cache hierarchy measurements, not whole-editor frame improvements.
They exclude ECS component lookup, live row facts, Details, UI rendering, and GPU
work. The optimization is algorithmic in both compiler settings; it does not change
the development profile. Native frame timing must establish the total improvement.

The normal regression test checks ordering, unnamed/orphan roots, immediate rename,
reparent/delete, recycled entity generation, emptied scenes, and reuse/reset of
row storage. Both standalone regression and benchmark passed. Run the shared-crate
tests with `cargo test -p somnium_core outliner_hierarchy::tests`; explicitly add
`-- --ignored --nocapture` for the benchmark. Test/profile output and the isolated
harness are retained in the private game's `source-assets/outliner-audit` folder.
