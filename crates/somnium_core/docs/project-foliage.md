# Project painted foliage

A project can extend the native foliage palette with `<content_root>/foliage.palette.json`.
The file is loaded once when the project opens. Reopen after editing the descriptor.
Legacy built-in kinds 0–24 keep their meanings; project kinds use stable IDs 128–255.
Missing IDs remain missing instead of silently becoming a different plant.

```json
{
  "version": 1,
  "entries": [{
    "kind": 128,
    "name": "Mixed woodland patch 1",
    "source": "assets/environment/groundcover_templates.gltf",
    "primitives": [0, 1, 2, 3, 4],
    "lod": {"source":"assets/environment/lod/groundcover.gltf", "primitives":[0,1,2,3,4], "distance":16},
    "brush": {"single": true, "scale_min": 1, "scale_max": 1}
  }]
}
```

`source` is relative to the project root and uses the existing contained project-path
resolver. `primitives` contains flattened renderable-node ordinals: exactly the
indices used by `ImportedMesh.node`, including all material parts of the chosen
variant. One source is loaded/uploaded once even when several entries select it.
All source node matrices and authored material values are retained. Optional
`local_transform` is a 16-number column-major invertible affine matrix, composed
before the source node matrix; omit it for identity. It can move an off-origin scan
to its intended planting anchor without changing the imported geometry.

The optional `brush` object accepts `single`, `density`, `layer`, `min_layer_weight`,
`max_tilt_deg`, `max_slope_deg`, `scale_min`, and `scale_max`. It changes future brush
strokes only. A mixed patch containing many plants should generally use single
placement or a low patch density. The Details brush and context-bar picker display
project names and translate their rows back to stable kinds.

Terrain save/load keeps the existing STER v4 height/splat binary and adds a companion:
`<scene>.terrain0.bin.foliage.json` (the terrain number matches the binary filename).

```json
{"version":1,"instances":[{"kind":128,"position":[2,0,-3],"yaw":0.7,"tilt":0,"scale":1}]}
```

Position is terrain-local, yaw and tilt are radians, and scale is positive and uniform.
The final matrix is `terrain × translation × yawY × tiltX × uniformScale × local_transform × sourceNode`.
A missing companion means no painted instances. Loading checks the document before
replacing the runtime collection; invalid floats, unsupported versions and oversized
files are rejected. Saving includes an empty companion when the last instance is
removed, so erased foliage cannot reappear on reload. Existing paint/erase undo uses
this same native vector, without making each plant or patch an ECS entity.

An optional `lod` selects a complete simplified mesh beyond a positive horizontal
terrain-local `distance`, in metres. Its `source` uses the same contained path rule
and its `primitives` select the far source's flattened nodes. Both sources share
the entry's local transform; author their origins and complete plant silhouettes
consistently. Import failure retains the near mesh. Omit `lod` for existing behavior.

Project entries keep all selected material parts of the chosen mesh until the terrain FoliageComponent's
cull distance, with its separate closer shadow cutoff. Legacy tree part-dropping at
lod/impostor distances does not apply to mixed project patches. The native renderer
still receives material-part draws and performs its existing visibility/meshlet
work; this migration does not claim hardware instancing or guarantee a frame rate.
A large patch should be sized for useful distance culling and measured in the scene.

Tests cover stable IDs/selected parts/local transforms and versioned instance pose
round-trip/validation. Native placement, selection labels, painting, save/reload and
frame time require the shared consolidated build and scene evidence.
