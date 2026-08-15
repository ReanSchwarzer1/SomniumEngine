# Terrain

Select a Terrain entity, then press **F6** (or use the Landscape toolbar button) to enter sculpt mode. The left **Sculpt** strip becomes the brush.

## Brushes

Click a tool so it highlights. Keys **1–6** pick the same tools.

- **Raise / Lower** — add or carve height
- **Smooth / Flatten** — soften or level
- **Noise** — add variation
- **Paint** — splat a material layer (use the layer list in Details)

**[ ]** change brush size. **- =** change strength. The Details terrain section shows the active layer plus **Hex Tiling**, **Parallax**, and **Clipmap**.

## Foliage

**F8** opens foliage paint on a selected terrain. Pick a kind in Details, then paint, erase, or place a single instance. Foliage only grows on its paint layer and refuses steep ground. **Cull** / **LOD** / **Impostor** are **horizontal** metres: past LOD leaf/cutout parts drop; past Impostor only solid parts remain (there is no camera-facing billboard). Impostor `0` keeps every part.

## LOD morph

**LOD Morph** (default off) and **Morph** (0–1, start of the blend) remove the ridge pop between clipmap LODs. `SOMNIUM_LOD_MORPH=1` turns it on.

## Hex, parallax, clipmap

**Hex Tiling** and **Parallax** live on the selected Terrain (not Post FX). Hex is the anti-repeat tap; Parallax is POM (there is no separate “POM” label). Unchecking them zeros the uniforms **and** rebuilds the shading pipeline without those paths — a hitch once, then cheaper shade. They do not change the 32-slot GPU splat format.

**Clipmap** (default **off** until the DF-E gates pass) is the cheap shade path for a 1 km tile: it bakes strongest-four + hex + height-blend into nested caches centred on the ground the camera is looking at (clamped to 8 m). Shade bilinear-loads the toroidal cache and blends rings so edges do not streak. Generate paints at most one 1024² ring per frame (the 8 m ring first); shade skips rings that have not finished a full generate. Cliffs stay live (biplanar). POM is not marched on the cache. Toggle Clipmap off/on once after updating to rebuild. `SOMNIUM_TERRAIN_CLIPMAP=1` forces on; `=0` forces off. Leave it off on **Coastal** unless you are running the Daggerfall audit.

Dbg **32** is clipmap albedo, **33** is ring index (0 = finest).

## Maps

**Game / Maps** in the Content Drawer. **Coastal** is the 1 km, 32-layer Appalachia launch landscape (256 chunks). **Island** is a 512 m ocean tile with a 16-layer hero bank (hex and parallax off; GPU splat format still 32 slots with 16–31 empty). Island shade is cheaper because fewer pixels hit ground and the compact pipeline scans 16 layers. Coastal stays heavier on the ground with the same options off — that is tile size and 32 published layers, not leftover POM. Soft Shadows on Post FX is PCSS (Help → **Lighting**).

## Frustum cull (Phase CR)

Select the **Camera** entity for **Frustum Cull** (default on). Off-screen terrain chunks skip the vis draw queue; they still cast into view when they overlap a cascade. Hold **RMB** and look away from the tile to see profiler `cpu-cull` rise — WASD while facing the coast will not. `[off]` / `[forced-off]` on that row means the CPU test is not running. `SOMNIUM_CPU_FRUSTUM=0` forces it off. **F10** remains the GPU 15B A/B.

## Debug views

Terrain **Dbg** 0–23 are material / shadow / splat probes. **24–31** are lighting: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

Water is a separate child entity. Reflection knobs live on that Water, not on Terrain — Help → **Water**.
