# MORROWIND-S — world partition and cell ownership

**Complete, 2026-08-25; Hello Engine integration corrected 2026-08-25.**
Track 4 (SILT STRIDER), after MORROWIND-R.

## Hello Engine integration correction

The initial record proved the public partition API and Vvardenfell boundary but
did not connect that coordinator to the production engine loop. That omission
is now closed. A terrain carries a reflected `WorldPartitionComponent` whose
authored cell size, load radius, priority and manual pin appear in generated
Details. Wanted/loaded/pending cells, resident actors and status are visible
read-only diagnostics and are deliberately not serialized.

The engine-owned coordinator runs after `GameApp::on_render`, follows the same-
frame `renderer.camera_pos` used by the active editor/player view, and uses the
one shared job system. Cell-size edits drain and rebuild the old grid; terrain
deletion drains its streamed actors. Missing empty cells load as empty without
creating thousands of directories. Hello Engine's default and Create → Terrain
paths attach the component, so partitioning is usable without Vvardenfell or an
environment variable.

## Cells and want-state

`somnium_core::world_partition` maps double-precision world positions to a
configurable integer `CellCoord` spatial hash, including correct floor behavior
across the origin. A `StreamingSource` declares a stable source id, camera,
player or explicit-volume role, world position, radius/box and priority. The
sorted union of those sources is the runtime want-state. Editor pins are the
only override and use an undoable `CellPinCommand`.

`CellDiagnostic` supplies coordinate, load state, winning priority, pin and
actor count. `overlay_lines` produces world-aligned XZ grid segments with state
and pin data for the existing gizmo line pass; the editor does not own a second
streaming model.

## Storage, jobs and ownership

Each cell has a deterministic index containing sorted actor ids and sorted
cook-derived `AssetId`s. Every actor is a separate JSON file. Stale actor files
are removed when the authoritative index is rewritten, keeping diffs and file
ownership exact.

An actor file is the existing versioned schema scene document for exactly one
real ECS entity, not an opaque streaming DTO. The subset serializer preserves
registered components, scripts and retained unknown data. Loading resolves
references against both incoming actors and already-live neighboring cells.
`PersistentId` remains the durable cross-cell identity; `StableId` remains a
component-type name.

Cell reads and writes are priority/deadline `somnium_jobs` work. Moving a
source away cancels an obsolete load; returning while an unload is in flight
cancels that unload. ECS mutation stays on the main thread. Unload first
snapshots and persists live entities, and despawns only after successful
persistence. A cancelled or failed unload leaves the original live entities
untouched.

`examples/vvardenfell` creates a real named/transformed actor, serializes it
through the public schema subset API, stores it beside a derived cooked shader,
then loads the cell through the partition API and resolves the same
`PersistentId`.

## Verification

- Five focused world-partition tests pass, alongside 26 focused scene-schema
  tests.
- The cheat check unloads and reloads two real schema entities **100 times**.
  Every loaded baseline is 2, every unloaded baseline is 0, terminal job
  bookkeeping is pruned, final persistent ids are unique, and `Name` plus
  `Transform` are restored.
- Tests cover negative spatial hashing, one-file-per-actor deterministic
  storage, sorted derived assets, camera/volume want-state, undoable pins and
  cooperative obsolete-load cancellation.
- `cargo check -p vvardenfell` passes using public APIs.
- GHOSTFENCE: **7/7 rows passed**, including **1,831 tests passed, 0
  failed** and all 3 registered golden images within threshold.
- Whole-core strict pedantic clippy remains blocked by 213 pre-existing
  findings in older modules; the full core build and tests contain no S warning
  or failure.

## Reference boundary

Unreal World Partition (proprietary) and Luanti's active/static object lifecycle
(LGPL-2.1+) were architecture-only reads. No code, identifiers, constants,
layouts or comments were copied, and UE data layers/content bundles remain
explicitly refused. See `ATTRIBUTION.md` §13H.21.
