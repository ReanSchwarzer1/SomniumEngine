# Lighting

These controls live on the **Post Processing** entity unless noted. Select that entity in the Outliner, then use Details. Expensive paths default **off**; turn them on from Details.

## Anti-aliasing

One control, **Anti-aliasing**, with six values. It used to be three separate checkboxes; the FXAA box was checked by default and never ran a pass, because FSR was also on and took precedence. There is one value now, and whatever it says is what runs.

| Value | What it does | Cost |
|---|---|---|
| **Off** | Nothing. The visibility buffer has no MSAA, so edges are hard-aliased. Useful as the reference when judging the others. | — |
| **FXAA** | One LDR pass over the tone-mapped image. Cheapest. Cannot tell an edge from a glyph, so it softens detail as well as edges. | 1 pass |
| **SMAA 1x** | Morphological. Finds edges, reconstructs how much of each pixel the silhouette covers, blends only that. Much less texture softening than FXAA. | 3 passes |
| **SMAA T2x** | SMAA 1x over a temporally resolved image — the subpixel jitter and velocity buffer TAA already uses. Best non-upscaling option. | 3 passes + TAA |
| **TAA** | Somnium's own temporal resolve, no morphological pass. | 1 pass |
| **FSR 3** | AMD FSR 3. **Default.** Also the upscaler: it reconstructs the viewport **Resolution** preset to the window, and picking anything else means that preset is blitted instead. **FSR Sharp** is RCAS (0–1, default 0.8). | 1 pass |

**SMAA Quality** — Low / Medium / High / Ultra (default) — only does anything on the two SMAA values. It trades edge sensitivity and how far the search walks along an edge: Low is roughly FXAA's cost, Ultra finds edges FXAA cannot see.

**There is no SMAA S2x or SMAA 4x**, and there will not be: both resolve MSAA subsamples, and shading from a visibility buffer means one triangle per pixel with no subsample coverage to resolve. They are named here so their absence reads as a decision rather than an omission.

Somnium CAS stays off while FSR is selected — RCAS already sharpens, and stacking the two rings edges. Checking **CAS** steps the mode down to TAA rather than leaving a control that does nothing.

`SOMNIUM_FSR=0` starts on TAA instead of FSR. Frame generation is not in the engine. Water and transparents may ghost under camera motion; that is a missing reactive mask, not a water shading bug, and no anti-aliasing choice fixes it.

## Transparency

**Order-Independent Transparency** is **off by default**, on the Post Processing entity.

Off, blended surfaces are sorted back-to-front by object origin and blended in that order. That is correct for separated panes of glass and wrong where two blended surfaces *of the same object* intersect — a per-object sort key cannot answer a per-pixel question.

On, the engine uses weighted-blended OIT (McGuire and Bavoil, 2013): every blended fragment accumulates into two targets with a depth-derived weight, and one resolve composites them. Draw order stops mattering. It is **approximate** — the weight function decides which fragment dominates, and it can be wrong for a nearly-opaque surface behind a transparent one — so this is a trade, not a strict upgrade. That is why it is authored and why it defaults off: turning it on changes what an existing scene draws.

Costs two extra full-resolution targets, 10 bytes per pixel (20.7 MB at 1080p, 82.9 MB at 4K), fixed regardless of how many layers overlap.

## World cache (24M)

**World Cache** is **off by default**. It does **not** make the frame cheaper.

It adds a 64³ clipmap splat of this frame's GI so shading can pick up extra bounce light. That is more GPU work on top of ReSTIR GI, not a substitute for it. Turn it on when you want the extra bounce, not for frame time. **Cache Amt** is the mix into ambient; **Cell m** is voxel size. `SOMNIUM_WORLD_CACHE=1` forces it on at startup.

## Scene specular (24N)

**RT Specular** traces glossy reflections for everything that is not water (water already has Halcyon RT). **Spec Rgh** is the roughness cutoff. Needs ray query. A 5-tap temporal mix is the denoiser. Default off (`SOMNIUM_SPECULAR_GI=1`).

## Path tracer (24O)

**Path Tracer** replaces the image with an accumulating 1-spp reference. **Bounces** is 1–8. History resets if the camera moves more than a few centimetres. Default off (`SOMNIUM_PATH_TRACER=1`). Needs ray query.

## Mesh SDF / portable DDGI (24P / MORROWIND-AB)

**Mesh SDF** cone-traces a 64³ clipmap. Static meshes bake a packed 16³ triangle SDF at upload (AABB fallback for voxels). Do not combine it with World Cache — they share the volume's alpha. Create a cube (it spawns in front of the camera) and leave World Cache off; contact darkens the ground around the mesh.

**Portable DDGI** traces that software SDF from a camera-relative 4×4×4 probe volume, so it works without ray query. **DDGI Intensity** controls the diffuse contribution; **Probe Spacing** is metres between probes; **Update Budget** is how many of the 64 probes refresh per frame; **Hysteresis** trades responsiveness for stability. It defaults off. ReSTIR GI wins if both GI tiers are checked, and the path tracer disables both. The old `SOMNIUM_PROBES=1` capture switch maps to DDGI for compatibility but is not a second authored control.

## Area lights (24R)

Create → **Area Light**, **Disc Light**, or **Tube Light**. New lights spawn a few metres in front of the camera (discs/rects/spots face into the view; tubes run across it).

- **Area Light** — **Half W** / **Half H** are metres from the centre of the rectangle; **Radius** still drives highlight size.
- **Disc Light** — **Radius** is the disc radius; forward is the emitting-plane normal.
- **Tube Light** — **Radius** is the tube cross-section; **Half W** is half-length along forward.

## Soft shadows / contact (PCSS)

**Soft Shadows** is percentage-closer soft shadows (PCSS). There is no separate “PCSS” label. **Contact Shadows** is the screen-space contact march. Both live on **Post Processing**. With **RT Direct Light** (ReSTIR DI) on, sun visibility already lives in the ReSTIR buffer, so the shading pipeline drops PCSS/contact from the compiled shader — unchecking the boxes alone does not delete that code until the pipeline rebuilds. Hex and Parallax are on the Terrain entity, not here (Help → **Terrain**).

The four-cascade atlas is persistent. Static quadrants keep their depth until
the sun, snapped camera cell, filtered caster set, or edited terrain geometry
affecting that cascade changes. Distant camera-driven updates are interleaved;
caster changes are immediate. The Profiler counter **shadow cascades** reports
how many of four were redrawn, so a static scene should settle at zero even
while **shadow casters** remains non-zero. `SOMNIUM_SHADOW_CACHE=0` is the
correctness kill switch and restores all four redraws every frame.

## Light shafts (24U)

**Light Shafts** shadow-tests the volume. **Shaft Amt** boosts the sun in-scatter when shafts are on (1 is unscaled air). Default on.

## Lighting debug (24AB)

Terrain **Dbg** 24–31: luminance, GI, cluster occupancy, world cache, specular aux, SDF, analytic mips, path-tracer aux.

## Analytic mips (25N)

**Analytic Mips** (default on) uses barycentric UV gradients so foliage does not pick an arbitrary mip across vis-buffer quads. `SOMNIUM_ANALYTIC_GRAD=0` kills it.

## Terrain LOD morph (25C)

On a terrain: **LOD Morph** (default off) and **Morph** (0–1, start of the blend; 0.7 is the last 30% of each LOD range). `SOMNIUM_LOD_MORPH=1` turns it on.

## Foliage LOD (25P)

**Cull**, **LOD**, and **Impostor** are **horizontal** metres. Past LOD, leaf/cutout parts drop; past Impostor, only solid bark/branches remain (not a billboard). Impostor `0` keeps every part.

## Profiler (29)

The overlay lists **GPU** pass times, then the **Graph** (pass order), then **CPU** zones (instances, cluster cull, foliage, lighting extra), then draw counters. Toggle **Profiler** on the viewport bar.
