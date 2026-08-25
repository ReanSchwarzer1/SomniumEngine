# MORROWIND-R — residency and cooked hot reload

**Complete, 2026-08-25.** Track 4 (SILT STRIDER), after MORROWIND-Q.

## Residency contract

`somnium_asset::residency::ResidencyManager` is the one policy owner. Every
request is keyed by the existing `AssetId` plus LOD, records its requester,
priority and optional deadline, and returns an `AssetHandle` containing a
type-correct placeholder before I/O starts. Resolver work runs through
`somnium_jobs`; its main-thread completion queues bytes for installation.

`process_frame` consumes no more than the configured upload bytes. An upload
larger than one frame's allowance spans frames, and the stable handle is swapped
only after the complete payload has arrived. Consumers therefore observe the
old complete revision or the new complete revision, never a partial value.

The configured byte budget is a hard bound. Installation evicts the
deterministic least-recently-used key, and eviction atomically restores that
handle's placeholder. Mesh LODs are separate keys; non-mesh LOD requests are
rejected. A coarse mesh LOD can be resident while LOD 0 is absent.

## Hot reload and diagnostics

`CookedAssetWatcher` applies one polling/debounce shape to all seven Q kinds.
A change is decoded and checked for the expected kind and `AssetId` before it
enters the upload queue. Reload tickets reject stale completions. A failed
reload leaves the old resident value and revision published, following
MORROWIND-C's transactional shader rule.

`ResidencySnapshot` is the UI-neutral residency-panel source. Its sorted rows
state what is loaded, current lifecycle, LOD, bytes, why it is retained, every
requester, last-use frame, revision and last error, plus total resident and
queued bytes against both budgets.

`examples/vvardenfell` uses only public APIs to cook a real shader, request it
through the build resolver, observe the immediate placeholder, drain the shared
job completion and install it within the upload budget.

## Verification

- `somnium_asset`: **45 tests passed**, including 10 focused residency tests.
- Strict no-dependency clippy for the asset library passed.
- Focused tests cover hard byte bounds, multi-frame uploads, LRU eviction,
  independent mesh LOD, immediate placeholder/atomic swap, failed reload,
  stale-ticket suppression, every cooked kind and complete diagnostics.
- `cargo check -p vvardenfell` passes through the public Q/R boundary.
- GHOSTFENCE: all seven rows passed, including **1,826 tests passed, 0
  failed** and all 3 registered golden images within threshold. The first test
  row hit a transient Windows `LNK1104` executable lock and passed on immediate
  isolated rerun.

## Reference boundary

The implementation uses standard cache algorithms and Somnium's own Q
resolver, B job system and C hot-reload precedent. Flax was architectural
context only and remains strict/proprietary under MORROWIND-A's audit; no Flax
code, identifiers, constants, layout or file structure was copied. See
`ATTRIBUTION.md` §13H.20.
