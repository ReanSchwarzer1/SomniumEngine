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

## Debug views

Terrain **Dbg** 0–23 are material / shadow / splat probes. **24–31** are lighting: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

Water is a separate child entity. Reflection knobs live on that Water, not on Terrain — Help → **Water**.
