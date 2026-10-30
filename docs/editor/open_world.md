# Open worlds

A world larger than one scene holds is built offline into **cells**. One small persistent scene carries what is always there: the sky, the camera, the player and the terrain windows. The cells stream around the view. Rimeborn's Tyffir (about 12 x 12 km, 160,000 entities) is built this way by its `tools/compose_tyffir.py`.

## Cell Streaming

Add **Cell Streaming** to any entity in the persistent scene and point **Manifest** at a JSON file in the project:

```json
{"version": 1, "cells": [
  {"file": "cells/s_3_-2.scene.json", "min": [768, -512], "max": [1024, -256]},
  {"file": "cells/l_0_0.scene.json", "min": [0, 0], "max": [1024, 1024], "reach": 1500}
]}
```

Each cell is an ordinary scene file. `min` and `max` are the XZ bounds of what it holds, in world metres. `reach` is how near the view must come before the cell loads, and it overrides **Load Radius**; use it for cells of big landmarks that are seen from afar.

| Field | Does |
|---|---|
| Load Radius | Cells whose bounds come this near the view load (600 m) |
| Unload Margin | A loaded cell goes once the view is this much further than its reach (100 m) |
| Cells Per Frame | Most cells put into the world in one frame |
| Loaded Cells, Pending Cells, Streamed Entities, Status | What it is doing now (read-only) |

How a cell comes in:
- A worker thread reads and parses the cell's file.
- For each mesh source the cell uses that is not on the GPU yet, it decodes the glTF and prepares the upload: texture keys, cached BC7 mip chains, mesh clusters.
- The main thread spends about 6 ms a frame on the GPU writes.
- Once every mesh the cell needs is uploaded, its entities appear, all at once.

When a scene opens, or Play begins, the cells around the view are taken in whole before the first frame shows, as a loading screen would. After that it is a few each frame.

Streamed entities are never saved into the persistent scene. **Stop** does not bring back cells that streamed out during Play.

Meshes no loaded cell has used for 30 seconds are released, freeing their geometry, ray-tracing structures and textures. Meshes the persistent scene itself uses are never released.

## Terrain Streaming

A 4 km terrain at 2 m cells takes about 210 MB of the 512 MB vertex pool, so a world of several terrain windows cannot hold them all at once. Add **Terrain Streaming** to a terrain entity and its geometry is resident only while the camera is within **Geometry Distance** of it. It comes back 64 chunks a frame, nearest first. While none of its geometry is resident, its splat maps (the ground-layer weights, about 134 MB for a 4 km window) leave the GPU too, and they come back with its first chunk. Its heights, the weights on the CPU, its foliage and its colliders stay loaded throughout.

Where windows overlap (Rimeborn's are 4,096 m on a 4,000 m grid), give each window's **Own Area**: x0, z0, x1, z1 in terrain-local metres, splitting each overlap down its middle. Generated ground cover grows only inside it, so the overlap is not covered twice. All zero means the whole terrain. Rimeborn's composer fills it in.

The vertex pool is 512 MB unless the environment variable `SOMNIUM_GEOMETRY_POOL_MB` asks for more, up to the device's limit. Rimeborn asks for 1,536 MB.

## Ground cover

In the project foliage palette (`foliage.palette.json`), give a kind a `"ground_cover": {"density": 0.2, "radius": 80}` and that kind is generated around where the view looks instead of being painted and stored:
- "where the view looks" is the ground under the middle of the screen: an isometric camera stands 60 m or more behind what it shows, so cover grown round the camera itself would leave the far half of the frame bare. A view looking level keeps the camera's own place;
- it is made per 16 m cell from the kind's brush rules: its splat layer and minimum weight, its slope limit, scale and tilt;
- `"align_to_ground": true` tilts each instance to the slope under it, for patches of grass that should neither float nor sink on a hillside; without it they stand upright, as flowers do;
- the same cell always gets the same tufts;
- a terrain edit regenerates them.

Use it for grass and flowers. They only matter near the camera, and painting them over a whole open world would fill the instance budget.

Ground cover kinds stay out of the ray-traced scene: thousands of small patches would fill its instance cap, and a frame over the cap loses its ray-traced reflections. They still draw and cast shadow maps. They also cast no screen-space contact shadows, which from a high camera read as dark scribbles in the grass. A palette kind's `"ray_traced": true` or `false` overrides that either way; painted kinds are traced unless they say `false`.

## Captures with nobody watching

A screensaver or a locked session takes the window's swap-chain images away. Frames then wait about a second each for an image that never comes, and a capture never finishes. With `SOMNIUM_RENDER_UNPRESENTED=1` the renderer draws such a frame into an offscreen image of the window's size instead: captures and tours finish, and nothing is presented until the window can show again. In that mode it asks the swap chain again only every 120 frames, and frame generation stays off. Without the variable, a frame with no image is skipped as before.
