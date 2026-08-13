# Lighting

These controls live on the **Post Processing** entity unless noted. Select that entity in the Outliner, then use Details. Expensive paths default **off**; turn them on from Details.

## World cache (24M)

**World Cache** is **off by default**. It does **not** make the frame cheaper.

It adds a 64³ clipmap splat of this frame's GI so shading can pick up extra bounce light. That is more GPU work on top of ReSTIR GI, not a substitute for it. Turn it on when you want the extra bounce, not for frame time. **Cache Amt** is the mix into ambient; **Cell m** is voxel size. `SOMNIUM_WORLD_CACHE=1` forces it on at startup.

## Scene specular (24N)

**RT Specular** traces glossy reflections for everything that is not water (water already has Halcyon RT). **Spec Rgh** is the roughness cutoff. Needs ray query. A 5-tap temporal mix is the denoiser. Default off (`SOMNIUM_SPECULAR_GI=1`).

## Path tracer (24O)

**Path Tracer** replaces the image with an accumulating 1-spp reference. **Bounces** is 1–8. History resets if the camera moves more than a few centimetres. Default off (`SOMNIUM_PATH_TRACER=1`). Needs ray query.

## Mesh SDF / probes (24P / 24Q)

**Mesh SDF** cone-traces a 64³ clipmap. Static meshes bake a packed 16³ triangle SDF at upload (AABB fallback for voxels). Do not combine it with World Cache — they share the volume's alpha. Create a cube (it spawns in front of the camera) and leave World Cache off; contact darkens the ground around the mesh. **Probes** bakes a 4×4×4 SH L2 grid from the environment (and the world cache when that is on); **Probe Amt** scales the bake. Scrub it on a shadowed face — sunlight drowns the mix. Both default off (`SOMNIUM_MESH_SDF=1`, `SOMNIUM_PROBES=1`).

## Area lights (24R)

Create → **Area Light**, **Disc Light**, or **Tube Light**. New lights spawn a few metres in front of the camera (discs/rects/spots face into the view; tubes run across it).

- **Area Light** — **Half W** / **Half H** are metres from the centre of the rectangle; **Radius** still drives highlight size.
- **Disc Light** — **Radius** is the disc radius; forward is the emitting-plane normal.
- **Tube Light** — **Radius** is the tube cross-section; **Half W** is half-length along forward.

## Light shafts (24U)

**Light Shafts** shadow-tests the volume. **Shaft Amt** boosts the sun in-scatter when shafts are on (1 is unscaled air). Default on.

## Lighting debug (24AB)

Terrain **Dbg** 24–31: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

## Analytic mips (25N)

**Analytic Mips** (default on) uses barycentric UV gradients so foliage does not pick an arbitrary mip across vis-buffer quads. `SOMNIUM_ANALYTIC_GRAD=0` kills it.

## Terrain LOD morph (25C)

On a terrain: **LOD Morph** (default on) and **Morph** (0–1, start of the blend; 0.7 is the last 30% of each LOD range). `SOMNIUM_LOD_MORPH=0` kills it.

## Foliage LOD (25P)

**Cull**, **LOD**, and **Impostor** distances in metres. Identical parts are instanced automatically. Impostor `0` keeps the mesh.

## Profiler (29)

The overlay lists **GPU** pass times, then the **Graph** (pass order), then **CPU** zones (instances, cluster cull, foliage, lighting extra), then draw counters. Toggle **Profiler** on the viewport bar.
