# Terrain

Select a Terrain entity, then press **F6** (or use the Landscape toolbar button) to enter sculpt mode. The left **Sculpt** strip becomes the brush.

## Actor world partition streaming

Terrain Details includes a generated **Actor World Partition** panel. **Stream Actors**, **Actor Cell Size**, **Actor Load Radius**, and **Source Priority** control cell-owned actor streaming around the view that is actually rendered (the editor camera in Edit and the player camera in Play). **Manual Pin** can keep one actor cell resident while inspecting it. The read-only **Diagnostics** group reports wanted, loaded, and pending cells, resident actor count, and the current streaming status; these live counters are recomputed and are not stored in the scene.

This does **not** hide or stream the Terrain entity's mesh chunks. Coastal is one authored terrain resource, so its chunks continue through the terrain frustum/LOD pipeline and the visible landscape can span the whole map. World partition controls actors stored under `assets/world_partition`; terrain chunk residency/virtual terrain is a separate renderer feature.

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

## Virtual texture streaming

Terrains created with **Stream Source Pages** use the existing runtime material
clipmap as their runtime virtual texture. Its feedback step follows each dirty
clipmap rectangle, reads only material layers present in the covered splatmap
region (plus the cliff layer), and streams paired albedo/surface BC7 pages into
an exact **64 MiB** physical cache. The 32 authored material slots therefore do
not require 32 full-resolution GPU texture layers.

**Stream Source Pages** and **Cache Budget** are read-only because the physical
resources are chosen when the terrain is created; an ordinary existing terrain
continues to use its resident arrays. **Uploads Per Frame** is the live throttle.
The **Virtual Texture Diagnostics** group reports resident and pending pages,
hits, misses, and evictions. A cold page uses a resident parent mip and finally
the layer mean; page arrival automatically recomposes the affected runtime
clipmap rather than leaving that fallback baked in.

## Maps

**Game / Maps** in the Content Drawer. **Coastal** is the 1 km, 32-layer Appalachia launch landscape (256 chunks). **Island** is a 512 m ocean tile with a 16-layer hero bank (hex and parallax off; GPU splat format still 32 slots with 16–31 empty). Island shade is cheaper because fewer pixels hit ground and the compact pipeline scans 16 layers. Coastal stays heavier on the ground with the same options off — that is tile size and 32 published layers, not leftover POM. Soft Shadows on Post FX is PCSS (Help → **Lighting**).

## Frustum cull (Phase CR)

Select the **Camera** entity for **Frustum Cull** (default on). Off-screen terrain chunks skip the vis draw queue; they still cast into view when they overlap a cascade. Hold **RMB** and look away from the tile to see profiler `cpu-cull` rise — WASD while facing the coast will not. `[off]` / `[forced-off]` on that row means the CPU test is not running. `SOMNIUM_CPU_FRUSTUM=0` forces it off. **F10** remains the GPU 15B A/B.

## Debug views

Terrain **Dbg** 0–23 are material / shadow / splat probes. **24–31** are lighting: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

Water is a separate child entity. Reflection knobs live on that Water, not on Terrain — Help → **Water**.

## Water surface level

**Surface Level** on a Water entity is the height of the water plane in
terrain-local metres. The Great Lakes preset defaults to **15.0 m**, which is
the datum its shoreline was baked at — `assets/terrain/great_lakes/recipe.json`
records it as `"water_level_metres": 15`.

It defaulted to **16.1 m** until 2026-08-29, and that was 1.1 m above the bake.
Measured over all 4,194,304 mask cells against the shipped heightmap at the
default 105 m of relief:

| Datum | Dry cells under water | Wet cells above it |
|---|---:|---:|
| 14.0 | 0 | 235,688 |
| **15.0** | **3,545** | **3,625** |
| 16.1 | **108,719** | 1 |

At 16.1 a hundred and nine thousand cells of ground the shoreline calls dry sat
under the plane, which is why the water read as lying *on* the beach instead of
meeting it. At 14.0 the error just reverses. The residual few thousand cells at
15.0 are the antialiasing band.

Moving it now moves the shoreline with it. That is worth saying because it did
not used to: `assets/terrain/great_lakes/{mask,depth,shore_sdf}.png` are a
*shoreline*, solved once for a plane at 16.1 m, and editing Surface Level moved
the plane while leaving that coverage where it was baked. Lowering the datum put
the surface below a waterline that still thought it was at 16.1; raising it drew
water over ground that was now above it. Either way the number in Details and
the picture disagreed, and the beach got an edge that did not follow the terrain
it was meeting.

The baked depth field is what fixes it: depth below one datum is depth below
another plus a constant, so the wet set is re-derived by subtracting the shift
and the shoreline contour is re-solved from it. At **16.1 exactly** the baked
data is used untouched, so the shipped look is unchanged to the byte.

`SOMNIUM_WATER_LEVEL=14` overrides the datum on the default landscape for an
A/B.

**Open ocean** (`WaterComponent::ocean`, preset 2) is exempt: it is a fully wet
rectangle with no baked shoreline, and terrain depth already owns where it meets
the island.

### Where the water meets the shore

The surface fades out over the last **0.9 m** of depth difference against
whatever is behind it, rather than being cut off by the depth test.

That fade is not cosmetic polish, it is the shoreline. Coverage deliberately
extends the water **1.5 m under** the terrain and lets the depth test own the
visible intersection, so the waterline you see is the terrain's rasterised
silhouette against a flat plane — and a binary depth test against LOD-reduced
terrain gives that silhouette hard, axis-aligned steps. Before the fade the
shore was visibly staircased.

Worth knowing if you go looking for that staircase again: it is **not** the
water mesh (rebuilding it at 0.5 m instead of 2 m changes nothing) and **not**
the shore SDF (coverage cuts 1.5 m further out, under the terrain, so the
contour never draws that edge). Both were checked.
