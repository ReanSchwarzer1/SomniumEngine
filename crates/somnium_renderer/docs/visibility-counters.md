# Visibility diagnostics

`renderer.profiler.counters` describes the current completed renderer frame;
`last_counters` retains the prior frame. Reading a count does not enable GPU
readback. These additions change observability only.

| Field | Meaning |
| --- | --- |
| `foliage.candidates` | Painted placements visited on enabled terrain foliage components. |
| `foliage.distance_culled` | Rejected by the existing horizontal terrain-local distance test. |
| `foliage.unavailable_mesh` | Placement could not resolve its palette geometry. |
| `foliage.scale_culled` | Authored falloff reduced scale to zero or below. |
| `foliage.submitted_instances` | Placements that emitted at least one material part. |
| `foliage.lod_instances` | Placements selecting a project's complete simplified far mesh. |
| `foliage.submitted_parts` | Material-part draws passed to the renderer before GPU culling. |
| `foliage.shadow_parts` | Those parts offered as shadow casters before shadow size/cascade culling. |
| `gpu_draw_arguments` | GPU indirect arguments, including whole-mesh fallbacks and expanded meshlets. |
| `gpu_meshlet_arguments` | Arguments specifically produced by meshlet expansion. |
| `gpu_visible_arguments` | Optional actual surviving argument count from the existing cull readback. |
| `water_draws` / `water_bound_draws` | Queued water commands versus commands with a loaded body descriptor. |

GPU visibility must be `null` when `gpu_visible_arguments` is `None`; the CPU
`draw_calls`, `instances` and `triangles` counters do not measure GPU survivors.
The optional visible count combines whole-mesh and meshlet arguments; it is not
a meshlet-only count. `SOMNIUM_CULL_STATS=1` already enables synchronous GPU
readback and can disturb timing. Normal runs leave it disabled.

Native foliage uses horizontal distance rejection before material-part
submission. The renderer then applies its usual GPU frustum and Hi-Z tests.
A mesh used more than eight times in a frame intentionally uses whole-mesh
arguments instead of expanding every copy into many meshlet arguments. Project
palette kinds 128–255 retain every selected material part until distance culling;
the legacy tree leaf-dropping LOD rules only apply to built-in kinds below 128.
Project entries may supply an optional simplified `lod` source and distance;
`foliage.lod_instances` records its selection. Without it, the full mesh remains.
This does not claim early CPU frustum rejection of whole painted patches.

Authoring diagnostics also expose live bindless texture slots, GPU allocation
bytes, terrain span reservations, and actual terrain texture bank format/size/VT
presence. Invalid backend allocation counts are null. Scene resets release terrain
views and reusable geometry reservations; imported shared assets remain cached.
Texture mip selection, terrain source-page streaming and mesh LOD are separate
systems. A bank reporting `virtual:false` is not streaming virtual source pages.

`shadow_casters` counts eligible casters, while `shadow_cascades_rendered` counts
atlas quadrants actually redrawn. A stationary frame can have many casters and
zero redraws because the atlas is cached. The existing `SOMNIUM_SHADOW_CACHE=0`
switch disables this cache for comparison. Screen-space contact shadow settings
are independent of the cascaded shadow atlas.

Water prepass timing includes the spectrum simulation. It is not evidence of
visible surface pixels: missing body descriptors, terrain depth, view framing or
coverage can leave little shading work. The OPEN preset (2) supplies full wet
coverage and has no distance cutoff. Water mesh UVs must span 0–1 across the body
bounds, since the shader uses them to reconstruct local water coordinates. The
surface shader uses the actual mesh/model height; `surface_level` alone does not
move an arbitrary imported mesh.
