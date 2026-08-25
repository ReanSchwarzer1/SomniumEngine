# MORROWIND-Q — deterministic native asset cook

**Complete, 2026-08-25.** Track 4 (SILT STRIDER), after MORROWIND-K and
MORROWIND-V.

## Format and identity contract

`somnium_asset::cook` writes one independently addressable artifact per asset.
The common little-endian envelope contains magic, format and cooker versions,
kind, the existing path-derived `AssetId`, SHA-256 source/recipe/payload hashes,
sorted dependency ids and recipe hashes, and a checked payload length. Mesh,
texture, audio, scene, prefab, shader and material payloads have distinct
magics and extensions. Decoding rejects unknown versions, kind mismatches,
unsorted or duplicate dependencies, unreasonable sizes, every truncated
prefix, trailing bytes and hash corruption.

Text input has canonical line endings; JSON prefab/material input is
canonicalized structurally. Absolute paths, output locations, cache locations
and filesystem timestamps never enter artifacts or manifests. The manifest is
sorted by `AssetId` and deterministically serialized.

## Incremental cook and dependency contract

The cook plan is validated as a complete DAG before output. Missing edges,
duplicate identities and cycles are errors. A recipe hashes format/cooker
version, native kind, `AssetId`, canonical source bytes and sorted direct
dependency recipe hashes. Tests demonstrate the required invalidation:
changing a texture recooks it and its material, while an unrelated mesh is a
cache hit. A cooker-version change invalidates every recipe.

The library submits a named `asset.cook` job with explicit priority and
deadline through the single `somnium_jobs` system. `tools/assetcook` is a thin
plan-driven CLI over that library, not a second implementation.

## Runtime and second-example boundary

`AssetResolver` has development and build representations behind one `load`
method. Both return the same `LoadedNativeAsset`, the same `AssetId` and the
same native payload. The build-mode test removes the source tree before load,
proving shipped resolution has no source-file dependency.

`examples/vvardenfell` cooks a real repository shader through the public job
API and resolves its build artifact without reaching into asset or renderer
internals. Derived files live under `target/`.

Artifacts are immutable independent blobs selected by a replaceable manifest.
A future live-update system can download new blobs and atomically replace that
manifest without changing the format. **MORROWIND-Q does not implement network
delivery, patch policy or post-ship live update.**

## Verification

- `somnium_asset`: focused unit suite passes, including distinct native
  families, round-trip/truncation/corruption, DAG validation, reverse-closure
  invalidation, clean-directory determinism, dev/build parity, cooker-version
  invalidation and job priority/deadline.
- `somnium_assetcook`: check and strict no-dependency clippy pass.
- `vvardenfell`: check passes through public `somnium_asset` and
  `somnium_jobs` APIs.
- GHOSTFENCE: **7/7 rows passed**, including **1,816 tests passed, 0 failed**
  against the repository floor of 945. The golden-image row compared all 3
  registered images within threshold. The first Windows console print hit a
  cp1252 encoding limitation after six rows passed; the completed tests row was
  rerun under Python UTF-8 mode and passed.

## Reference boundary

Defold's root `LICENSE.txt` is the Defold License 1.0, not a permissive license;
its game-engine-product commercialisation restriction places it in the strict
tier. It was read for architecture only. No Defold code, names, constants,
schemas, binary layouts or directory structure were copied. The independent
mapping is recorded in `ATTRIBUTION.md` §13H.19 and the phase license audit.
