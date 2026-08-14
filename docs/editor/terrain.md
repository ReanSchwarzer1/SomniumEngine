# Terrain

Select a Terrain entity, then press **F6** (or use the Landscape toolbar button) to enter sculpt mode. The left **Sculpt** strip becomes the brush.

## Brushes

Click a tool so it highlights. Keys **1–6** pick the same tools.

- **Raise / Lower** — add or carve height
- **Smooth / Flatten** — soften or level
- **Noise** — add variation
- **Paint** — splat a material layer (use the layer list in Details)

**[ ]** change brush size. **- =** change strength. The Details terrain section shows the active layer and hex-tiling toggle.

## Foliage

**F8** opens foliage paint on a selected terrain. Pick a kind in Details, then paint, erase, or place a single instance. Foliage only grows on its paint layer and refuses steep ground. **Cull** / **LOD** / **Impostor** are **horizontal** metres: past LOD leaf/cutout parts drop; past Impostor only solid parts remain (there is no camera-facing billboard). Impostor `0` keeps every part.

## LOD morph

**LOD Morph** (default off) and **Morph** (0–1, start of the blend) remove the ridge pop between clipmap LODs. `SOMNIUM_LOD_MORPH=1` turns it on.

## Material clipmaps (Phase DF)

**Clipmap** (default off until the DF-E gates pass) bakes strongest-four + hex + height-blend into nested caches centred on the ground the camera is looking at (clamped to 8 m, so walking still keeps the player inside a dense ring). Shade bilinear-loads the toroidal cache — not the anisotropic wrap sampler — and blends rings over 256 texels so ring edges do not streak. Cliffs stay live (biplanar). Generate paints the cache as a color attachment (same path as live layer sampling); shade reads that array. Toggle Clipmap off/on once after updating to rebuild; the first second fills rings incrementally. `SOMNIUM_TERRAIN_CLIPMAP=1` forces on; `=0` forces off.

Dbg **32** is clipmap albedo, **33** is ring index (0 = finest).

## Debug views

Terrain **Dbg** 0–23 are material / shadow / splat probes. **24–31** are lighting: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

Water is a separate child entity. Reflection knobs live on that Water, not on Terrain — Help → **Water**.
