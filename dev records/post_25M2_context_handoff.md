# Somnium Engine — Post-25M2 Context Handoff

> **Purpose:** historical next-session context for work performed after Phase 25M-2 (Phase IV A–J narrative)  
> **XV start-here:** for IV/XV history, use [`post_IV_context_handoff.md`](post_IV_context_handoff.md). **Current engine start-here:** [`halcyon_context_handoff.md`](halcyon_context_handoff.md) (Phase VV). This file remains the deep Phase IV A–J / asset-license record.  
> **Snapshot date:** 2026-08-13  
> **Branch at audit:** `dev`  
> **25M-2 boundary commit:** `4e56482`  
> **Audited HEAD:** `846dea7` (`Phase XV BGS and Godot ref update`) — docs after this snapshot include the 2026-08-13 Phase XV research expansion in `phase_XV.md` and the post-IV handoff  
> **Implementation status:** Phase IV complete (IV-A through IV-K, closed 2026-08-13). Phase XV is complete (see [`post_IV_context_handoff.md`](post_IV_context_handoff.md)). Phase 26 (Metaphor) **26-A–I shipped, phase remains open** — see [`phase_26.md`](phase_26.md). Phase VV (Halcyon) **VV-A–H in tree** — start at [`halcyon_context_handoff.md`](halcyon_context_handoff.md). This file stays the Phase IV A–J / asset-license record; do not treat the snapshot HEAD as current engine status.

## 1. Read this first

> For **Phase XV history**, start with [`post_IV_context_handoff.md`](post_IV_context_handoff.md). For **Halcyon (VV-A–H in tree; remaining work is evidence)**, start with [`halcyon_context_handoff.md`](halcyon_context_handoff.md).

The next session that still needs the Phase IV A–J / post-25M-2 narrative should read:

1. [`context.md`](../context.md) — current engine architecture and living phase history.
2. [`ATTRIBUTION.md`](../ATTRIBUTION.md) — exact reference/adaptation boundaries.
3. [`phase_25m2_completion_report.md`](phase_25m2_completion_report.md) — boundary state before this handoff.
4. [`phase_IV.md`](phase_IV.md) — completed Great Lakes terrain/water implementation and validation.
5. [`phase_XV.md`](phase_XV.md) — researched sixteen-material terrain plan; **XV-A–J later completed** (see the post-IV handoff). This file does not track that work.
6. [`assets/LICENSE.md`](../assets/LICENSE.md), [`assets/terrain/great_lakes/README.md`](../assets/terrain/great_lakes/README.md), and [`assets/models/gislinge_viking_boat/README.md`](../assets/models/gislinge_viking_boat/README.md) — shipped asset provenance.

The originally requested root documents `m2.md` and `m25.md` were not present during the Phase XV audit. The combined Phase 25M2 completion report above is the available substitute. Do not silently claim those absent files were read.

## 2. Current state in one page

### Shipped after 25M-2

- The accepted default moon illumination is `0.010 lux` in the scene, renderer, and fallback defaults.
- Daytime triangular shadow artifacts were traced to shadow-receiver logic, not the terrain heightmap:
  - ordinary geometry uses a true face normal for receiver bias;
  - smooth terrain uses its continuous interpolated geometric normal;
  - contact-shadow thickness is compared in linear view-space metres instead of nonlinear NDC depth.
- Motion Forge Pictures' Great Lakes source became the deterministic default terrain derivative.
- The default terrain has a finite, mask-clipped, depth-bearing water body represented as a separate ECS/Outliner entity.
- Startup and **Create → Terrain** use one versioned `DefaultLandscapePreset` and one `create_default_landscape` factory.
- Water supports deterministic CPU/GPU surface queries, Gerstner waves, optional two-cascade FFT waves, coherent optics, SSR fallback, foam, wet sand, underwater rendering, motion vectors, and camera containment.
- The shoreline no longer depends on coarse terrain triangles: it uses the full-resolution contour/SDF, under-bank guard coverage, shoreline LOD pinning, depth ownership, and contact foam.
- The accepted default water level is `16.1 m`.
- The default scene includes the licensed Gislinge Viking Boat with an independent physics proxy, distributed buoyancy, propulsion, Kelvin wake, and prop wash.
- Environment simulation advances in both Editing and Playing. Pause is the deliberate freeze operation.
- Play mode, including paused Play, hides editor-only gizmos, grid, selection outline, and terrain/foliage authoring overlays. Stop restores them.
- Phase IV evidence was moved under `dev records/phase IV/`; no development screenshots remain in the repository root.

### Researched but not shipped

Phase XV — **Appalachia** is a research-complete plan to expand terrain from eight to sixteen photoscanned/photogrammetry-quality PBR materials. All XV-A–XV-J milestones remain **PLANNED**. No Phase XV texture assets, shader changes, splat-layout changes, compression pipeline, or editor changes have been implemented.

## 3. Chronological change record

| Commit | Date | Result |
|---|---|---|
| `ed73500` | 2026-08-11 | Post-25M2 contact-shadow thickness correction, true receiver face normal for ordinary geometry, stable smooth terrain receiver path, and default moon illumination changed to `0.010`. |
| `e9a7016`, `99c3c11` | 2026-08-11 | Initial Phase IV research/architecture plan created and refined. |
| `d64e52e` | 2026-08-11 | Phase IV-A/B/C: FLOAT32 EXR import, Great Lakes deterministic bake, terrain shadow diagnosis, first-class ECS water body, hierarchy/serialization/undo integration. |
| `9864569` | 2026-08-11 | Phase IV-D/E: finite wet-cell surface, CPU/GPU query contract, water motion vectors/TAA, coherent optics, SSR/environment fallback, mips and inspector controls. |
| `2d6d800` | 2026-08-11 | Removed a validation PNG from the repository root in preparation for phase-scoped evidence storage. |
| `46d8aab` | 2026-08-11 | Phase IV-F/G/H: spectral FFT tier, foam/wet sand, underwater pass, shared landscape factory, evidence folders, broad renderer/ECS support. |
| `9361253` | 2026-08-11 | Phase IV-I/J: editor transport controls, continuous environment preview, buoyant Gislinge Viking Boat, wake/prop wash, shoreline improvements, documentation/attribution/evidence. |
| `997dd2e` | 2026-08-12 | Phase IV follow-up fixes: final shoreline/LOD/depth ownership, play-mode overlay suppression, scene/default reconciliation, default water datum `16.1`, documentation moved into `dev records`. |
| `42bd087` | 2026-08-12 | Phase XV research plan created. |
| `bfccbab`, `cfabdf0` | 2026-08-12 | Phase XV epigraph/codename and expanded research/specification updates. |
| `846dea7` | 2026-08-12 | Added Bethesda Game Studios and Godot 4.7.1 references and their concrete design consequences to Phase XV. |

## 4. Post-25M2 shadow and night corrections

### 4.1 Accepted moon value

The user selected `0.010 lux` from the night capture. It is now the authoritative default across all initialization layers. This is a project-authored visual calibration, not a value copied from an external engine.

### 4.2 Why the daytime terrain showed triangles

The height source was initially suspected because the screenshots showed large angular patches. Source analysis and analytic terrain tests showed that the Great Lakes FLOAT32 data was smooth. The defects aligned to render triangles because two shadow calculations were using the wrong geometric domain:

1. A receiver-bias normal labeled `geo_normal` was actually an interpolated vertex normal. On coarse ordinary triangles it could move the lookup behind the plane. Ordinary meshes now derive a true face normal for the shadow receiver. Terrain deliberately retains a stable continuous normal so every triangle does not become a separate bias plane.
2. The contact-shadow algorithm compared a thickness expressed in metres (`0.05 m`) with nonlinear NDC depth. Far from the camera that accepted blockers many metres thick and stamped triangle-shaped patches across the landscape. Scene depth is now reconstructed and compared with the ray in linear view-space metres.

### 4.3 Reference boundary

The screen-space contact-shadow structure was already derived from the Bend Studio pattern documented for Phase 24X. The post-25M2 fix is Somnium-specific unit correction and receiver-normal separation. Karis/Frostbite grazing-angle bias remains part of the broader shadow model; no external shader was copied for this correction.

## 5. Phase IV — completed implementation

### 5.1 IV-A: terrain truth and precision import

- Added FLOAT32 OpenEXR channel preservation. The old generic path would have converted the Great Lakes source to `Luma8`, leaving about 47 useful height levels and manufacturing roughly 1.9 m terraces at the proposed scale.
- Added real-codec precision tests, analytic flat/ramp/hill continuity tests, and debug modes for terrain LOD, triangle edges, geometric normal, receiver normal, shadow factor, and contact shadows.
- Established that the original angular artifact was a shadow/receiver problem rather than an image problem.
- Preserved the distinction between smooth terrain receiver normals and face-normal ordinary geometry.

### 5.2 IV-B: deterministic Great Lakes bake

- Added `crates/somnium_asset/examples/bake_great_lakes.rs`.
- Baked deterministic runtime derivatives:
  - `1025×1025` 16-bit height;
  - masked macro colour;
  - `2048×2048` water mask;
  - shoreline signed-distance field;
  - water depth/bathymetry;
  - recorded recipe and hashes.
- Runtime terrain uses approximately `105 m` total relief.
- The source's repeated flat water plateaus were treated as coverage hints, not lakebed depth. Somnium creates synthetic bathymetry up to `12 m` and documents that it is an authored approximation, not geographic bathymetry.
- Dry terrain has a minimum clearance above the extracted water surface to prevent coplanarity.
- The accepted runtime water datum was later raised to `16.1 m` after direct user visual inspection; the mask still prevents water spreading onto dry land.

### 5.3 IV-C: first-class ECS water body

- Expanded `WaterComponent` into stable authoring state with terrain relationship, kind, preset, bounds, datum, maximum depth, enablement, optics, and wave fields.
- Heavy mask/depth/SDF/simulation resources live in renderer-owned `WaterBodyRegistry`, not ECS columns.
- Terrain and Water are distinct selectable entities, with Water parented to Terrain.
- Added Outliner/inspector integration, hierarchy, composite creation, undo/redo, duplicate/delete, scene serialization, and renderer-resource reconciliation.
- Removed demo ownership of water textures and the old `WaterPlane` concept.

### 5.4 IV-D: finite surface and shared query contract

- Replaced the broad plane with a compact terrain-local 2 m grid containing only wet coarse cells.
- Fragment coverage still uses the full-resolution mask/depth/SDF so the coarse mesh cannot define the visible coastline or lose narrow inlets.
- The same four-band deterministic Gerstner parameters drive Rust and WGSL.
- `WaterBodyRegistry` supplies height, normal, depth, velocity, coverage, and containment queries without GPU readback.
- Shore depth attenuates displacement and derivatives in both CPU and GPU evaluation.
- Water writes an `Rgba16Float` surface-data target and replaces global `Rg16Float` velocity only where water exists.
- TAA consumes water motion vectors while opaque pixels retain depth reprojection.
- Distance/pixel-footprint filtering moves unresolved slope energy into roughness to prevent distant cross-hatching.

### 5.5 IV-E: coherent surface optics

- Dielectric water uses `F0 = 0.02037` with GGX sun/moon highlights and prefiltered environment reflection.
- Added bounded SSR with confidence/edge fade and mandatory environment fallback.
- Refraction rejects invalid foreground/no-backdrop candidates.
- Linear reconstructed path length drives RGB Beer–Lambert extinction and approximate Henyey–Greenstein single scattering.
- Shore SDF contributes depth-aware edge foam.
- Normal and ORM textures include full CPU-generated mip chains.
- Water optical/wave values serialize and are exposed in the Water inspector.
- Day and night post-TAA evidence was recorded.

### 5.6 IV-F: spectral waves, crest foam, and wet sand

- Retained Gerstner as deterministic baseline/low tier and CPU query contract.
- Added an optional deterministic two-cascade GPU inverse FFT:
  - `256²` over `192 m`;
  - `512²` over `53 m`.
- Uses fixed-seed wind spectrum evolution, bit reversal, radix-2 ping-pong inverse transforms, displacement, gradients, horizontal Jacobian, and foam history.
- Crest foam derives from horizontal folding/Jacobian; shoreline foam derives from SDF/depth.
- Foam accumulation/decay and wet-sand darkening use the same shared signal.
- Incommensurate patch lengths reduce obvious repetition.
- `SOMNIUM_WATER_SPECTRUM=0` keeps the Gerstner-only tier.

### 5.7 IV-G: underwater medium and partial submersion

- Renderer containment selects the finite body below the camera.
- A smooth per-pixel near-plane mask avoids a binary full-screen waterline switch.
- Reconstructs the submerged ray segment in HDR and applies RGB extinction, HG in-scattering, fog, sun/moon shafts, and depth/turbidity-faded receiver caustics.
- The water interface is visible from below and supports an underside/total-internal-reflection transition.
- The implementation deliberately did not translate the Brown–Conrady or stylized god-ray Shadertoy helpers cited inside Wicked Engine.

### 5.8 IV-H: one landscape preset everywhere

- Added versioned `DefaultLandscapePreset` and `create_default_landscape`.
- Normal startup and **Create → Terrain** consume the same terrain descriptor, Great Lakes source, relief, material threshold, transforms, water datum, camera, and post-processing defaults.
- Editor creation is one undoable `CreateLandscapeCmd` containing separate Terrain and Water snapshots.
- Added structural, undo/redo, deletion, and scene round-trip tests.

### 5.9 IV-I: simulation, vessel, wake, and final shoreline

- Added Opus Poly's 29,035-triangle Gislinge Viking Boat GLB with its embedded materials unchanged.
- The visual hierarchy is independent of a simple stable Jolt physics proxy hull.
- Eight hull samples apply distributed buoyancy, point drag, righting torque, and submerged propulsion at a fixed 60 Hz.
- Boat heading/speed drive analytic Kelvin-angle wake arms and prop-wash foam.
- Environment simulation runs in both Editing and Playing. This corrected the earlier behavior where water/boat preview appeared frozen until Play.
- Toolbar controls are Play, Pause/Resume, and Stop:
  - Play changes game/editor presentation but does not start environmental time from zero;
  - Pause freezes simulation time, physics, particles, and water while rendering continues;
  - Stop resets vessel pose/velocities and returns to live editor preview.
- Play and paused Play suppress all editor-only overlays. Stop restores them.
- Final shoreline treatment:
  - retains the `2048²` source contour;
  - bilinearly reconstructs and derivative-antialiases the SDF zero contour;
  - expresses foam width/SDF distance in metres;
  - adds broken-up depth-aware breakers and scene-depth contact foam;
  - uses three rotated normal-map frequency bands with distance fade;
  - adds a two-cell raster guard ring and `1.5 m` under-bank coverage dilation;
  - pins terrain chunks crossing the water datum to LOD 0, with neighbour relaxation;
  - lets opaque terrain depth hide dilated water, so coarse mesh facets do not own the shoreline.

### 5.10 IV-J: documentation and evidence

- Updated `context.md`, `ATTRIBUTION.md`, `assets/LICENSE.md`, Great Lakes provenance, and boat provenance.
- Moved the phase records and screenshots under `dev records`.
- Evidence naming is phase-specific; do not put screenshots in the repository root.
- Recorded compilation, unit, shader, scene, and renderer validation in `phase_IV.md`.

## 6. Phase IV reasoning and rejected alternatives

| Decision | Reasoning |
|---|---|
| Do not solve triangle shadows by merely swapping heightmaps | The discontinuities followed renderer triangles; the FLOAT32 source itself was smooth. Replacing art could hide but not correct the unit/normal defects. |
| Offline Great Lakes processing | Direct EXR loading was unsafe, runtime ambiguity was high, and deterministic derivatives are smaller and testable. |
| Generate bathymetry | The source contained water-surface plateaus but no real underwater terrain. A flat plateau cannot produce depth, refraction thickness, or underwater containment. |
| Keep water as a specialized pass | Water must read the opaque terrain colour/depth beneath it for refraction and thickness. It does not belong in the one-hit opaque visibility buffer. |
| Small ECS component, heavy renderer registry | Preserves archetype SoA behavior while giving each body stable authoring ownership and persistent GPU resources. |
| Finite wet-cell mesh plus full-resolution fragment mask | A huge plane wastes raster work; a coarse clipped mesh alone damages narrow shoreline detail. Combining both bounds cost without surrendering exact coverage. |
| Gerstner before FFT | Gerstner is deterministic, differentiable, cheap, and queryable. It established physics/optics contracts before the complex spectral tier. |
| Environment fallback is mandatory for SSR | SSR cannot see off-screen or hidden objects and therefore cannot be the only reflection source. |
| RGB underwater transport first | Fits the current HDR renderer and editable ECS parameters; full spectral transport was disproportionate to the phase. |
| Generated bathymetry is labeled synthetic | Prevents an art approximation from being misrepresented as geographic data. |
| Editor simulation is live | Environmental preview is an editor feature; Play should change player-facing presentation, not be the only way to see water move or boats float. Pause is the explicit freeze control. |
| Dilate water under terrain instead of changing the source height | Opaque terrain can safely cover a guarded surface. Editing licensed elevation data merely to hide raster/LOD cracks would be destructive and non-reproducible. |
| Pin shoreline terrain LOD | The visible intersection needs full-resolution terrain ownership; distant LOD triangles were the last source of square/triangular shoreline bites. |
| Separate render boat and collision hull | High-detail visual geometry is unsuitable for stable real-time buoyancy/collision sampling. |

Deferred Phase IV possibilities include a localized shallow-water solver, spray/bubbles/breaking-wave particles, broader interaction/ripple injection, and optional planar reflection. They were not required to complete the finite-lake foundation.

## 7. Phase IV assets and licensing

### Great Lakes terrain

- Author: Chris J Mitchell / Motion Forge Pictures.
- Source: <https://www.motionforgepictures.com/sdm_downloads/great-lakes-height-map/>.
- Catalog: <https://www.motionforgepictures.com/height-maps/>.
- Specific catalog grant: “CCO 1.0 Universal,” interpreted as the evident CC0 typo.
- CC0 legal reference: <https://creativecommons.org/publicdomain/zero/1.0/>.
- Important limitation: Motion Forge's general terms also describe downloads as personal/non-transferable. The more specific asset-page grant reasonably supports the committed processed derivatives, but the conflict remains documented rather than erased.
- Shipped repository content is processed runtime data with source/output hashes and transformations recorded in `assets/terrain/great_lakes/README.md`.

### Gislinge Viking Boat

- Author: Opus Poly.
- Source: <https://sketchfab.com/3d-models/gislinge-viking-boat-01098ad7973647a9b558f41d2ebc5193>.
- License: CC BY 4.0, <https://creativecommons.org/licenses/by/4.0/>.
- Shipped model: unchanged embedded render materials/textures; Somnium applies its own root scale, ECS integration, physics proxy, and buoyancy system.
- Hash, dimensions, source page, and render/physics boundary are recorded in `assets/models/gislinge_viking_boat/README.md`.

## 8. Phase IV complete reference inventory

These are pattern, algorithm, architecture, licensing, or visual references. Third-party source was inspected and independently translated; it was not copy/pasted.

### Primary papers, talks, and official documentation

| Reference | Phase IV use |
|---|---|
| Jerry Tessendorf, *Simulating Ocean Water* — <https://people.computing.clemson.edu/~jtessen/reports/papers_files/coursenotes2002.pdf> | Wind spectrum, deep-water dispersion, horizontal displacement, FFT foundation. |
| Mark Finch, *Effective Water Simulation from Physical Models*, GPU Gems — <https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models> | Gerstner displacement, analytic derivatives/normals, CPU/GPU parity foundation. |
| NVIDIA, *Rendering Water Caustics* — <https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-2-rendering-water-caustics> | Portable caustic quality tier and attenuation. |
| Nicolas Longchamps / Eidos Montréal, *From Shore to Horizon* — <https://media.gdcvault.com/gdc2017/Presentations/Longchamps_Nicolas_ShoreToHorizon_NOTES.pdf> | Shore SDF, coastal bands, wet sand, underwater plate, quality layering. |
| Bartłomiej Wroński / Ubisoft, *Assassin's Creed IV: Road to Next-Gen Graphics* — <https://www.gdcvault.com/play/1020397/Assassin-s-Creed-IV-Black> | ACIV water rendering goals, reflection/foam/LOD production context. |
| fxguide, *Assassin's Creed III: The tech behind (or beneath) the action* — <https://www.fxguide.com/fxfeatured/assassins-creed-iii-the-tech-behind-or-beneath-the-action/> | ACIII/IV production water systems, Beaufort/weather and layered appearance context. |
| Epic Games, *Single Layer Water* — <https://dev.epicgames.com/documentation/unreal-engine/single-layer-water-shading-model-in-unreal-engine> | Water pass ordering and surface/volume separation. |
| Epic Games, *Water Body Actors* — <https://dev.epicgames.com/documentation/en-us/unreal-engine/water-body-actors-in-unreal-engine> | First-class water-body ownership and editor authoring. |
| AMD GPUOpen, FidelityFX SSSR — <https://gpuopen.com/manuals/fidelityfx_sdk/techniques/stochastic-screen-space-reflections/> | Hierarchical/denoised SSR reference and limitations. |
| Monzon et al., *Real-Time Underwater Spectral Rendering* — <https://diglib.eg.org/items/1316f247-e9a8-48fe-8754-f3276191e6b5> | Physically grounded underwater absorption/scattering direction; Phase IV uses RGB approximation. |
| Jeschke et al., *Water Surface Wavelets* — <https://research.nvidia.com/labs/prl/shallow-water-simulation/> | Long-term localized interaction reference, deferred. |
| János Turánszki, *Underwater effect updates* — <https://youtu.be/DZaaUPLmJIQ> | Visual acceptance reference for waterline and underwater appearance. |
| Wicked Engine official repository — <https://github.com/turanszkij/WickedEngine> | License/provenance anchor for the locally inspected ocean, FFT, shoreline, and underwater source. |
| Standard Beer–Lambert, Henyey–Greenstein, dielectric Fresnel, and GGX models | Physical basis for extinction, approximate single scattering, water reflectance, and highlights; Somnium's path reconstruction and integration are original. |

### Local source references

| Reference | Inspected patterns / boundary |
|---|---|
| Wicked Engine `wiOcean.h/.cpp`, `wiFFTGenerator.*` | Persistent spectral resources, FFT scheduling, displacement/gradient ownership, optional CPU query/readback. MIT; Somnium scheduling and WGSL are original. |
| Wicked `oceanSimulatorCS.hlsl`, `oceanUpdateDisplacementMapCS.hlsl`, `oceanUpdateGradientFoldingCS.hlsl` | Spectrum evolution, displacement packing, gradients, horizontal Jacobian/folding. |
| Wicked `oceanSurfaceVS.hlsl`, `oceanSurfaceHF.hlsli`, `oceanSurfacePS.hlsl` | Projected-grid ideas, distance filtering, reflection/refraction, thickness, extinction, foam. |
| Wicked `underwaterCS.hlsl` | Pass placement, water interface, extinction, in-scattering. Shadertoy-cited helper paths were deliberately excluded. |
| Wicked `wiScene.cpp`, `wiRenderPath3D.cpp` | Lifecycle, ripple injection, water pass placement. |
| bevy_water `src/water.rs`, `wave.rs`, `water/material.rs` and WGSL shaders | Matched CPU/GPU wave queries, ECS authoring versus render state, tiled geometry and depth colour. MIT OR Apache-2.0. |
| Jolt `Samples/Tests/Water/WaterShapeTest.cpp`, `BoatTest.cpp` | Water sensor/broadphase ideas, sampled surface, distributed forces, drag, submerged propulsion. MIT. |
| CDLOD shaders/README and storage path | Sample convention, regular-grid LOD continuity, future distance morphing. MIT. |
| Unreal Water plugin `WaterBrushManager.cpp`, `WaterInfoMerge.usf` | Terrain/water coupling, distance-field smoothing, dilation, terrain-depth ownership at shore. No source copied. |
| Unreal Editor viewport transport convention | User-facing Play/Pause/Stop control language only. Somnium's live environmental preview, clock, reset semantics, and overlay suppression are original. |
| Falco Engine `Engine/Components/Water.*`, `StandardWater.shader` | Corroborating editor/reflection/refraction/foam/caustic patterns only. Provenance unresolved; no translation permitted. |
| Bend Studio screen-space shadow pattern | Existing contact-shadow structure. Post-25M2 change is Somnium's metre/linear-depth correction. |
| Karis 2013 / Frostbite PBR | Grazing-angle/slope-aware shadow bias context. |

## 9. Phase XV — research-only handoff

### 9.1 Codename and intent

Phase XV is **Appalachia**, inspired by Bethesda Game Studios' *Fallout 76* and the coincidental sixteen-material goal:

> *“Sixteen times the detail.”* — Todd Howard  
> Sixteen terrain materials. It had to be Appalachia.

The codename is thematic, while public BGS rendering and landscape-production principles are legitimate research references. Those ideas will be independently adapted and attributed; no Bethesda code or game assets are to be copied.

### 9.2 Current eight-layer baseline

Indices 0–7 are compatibility-locked:

| Index | Existing material | Label |
|---:|---|---|
| 0 | `aerial_grass_rock` | Grass |
| 1 | `forrest_ground_01` | Forest Floor |
| 2 | `aerial_rocks_04` | Rock / legacy cliff |
| 3 | `snow_02` | Snow |
| 4 | `leafy_grass` | Meadow |
| 5 | `brown_mud` | Mud |
| 6 | `coast_sand_rocks_02` | Sand / pebbled coast |
| 7 | `gravel_floor` | Gravel |

Current storage is two packed RGBA8 arrays per layer:

- albedo RGB + height A;
- normal XY + roughness + AO.

Current controls are two RGBA splatmaps and sidecar v2 `[u8; 8]`. The shader has height-aware blending, perceptual albedo, dominant-layer POM, derived macro colour, sparse weight gating, and practical hex tiling. It is fixed to eight layers in renderer/editor/serialization and ReSTIR GI code.

### 9.3 Proposed new materials

The eight proposed additions are Poly Haven CC0 candidates. They are researched, not downloaded or committed. XV-A first-party audit (2026-08-13; [`phase XV/XV-A_research.md`](phase%20XV/XV-A_research.md)) substituted two IDs before download:

| Index | Asset | Intended role | Source |
|---:|---|---|---|
| 8 | `aerial_sand` | Dry beach sand | <https://polyhaven.com/a/aerial_sand> |
| 9 | `coast_sand_01` | Damp shoreline sand | <https://polyhaven.com/a/coast_sand_01> |
| 10 | `dry_mud_field_001` | Dry earth/topsoil | <https://polyhaven.com/a/dry_mud_field_001> |
| 11 | `cracked_red_ground` | Red mineral clay | <https://polyhaven.com/a/cracked_red_ground> — substituted for `terrain_red_01` (crushed reddish gravel, overlapping layer 7 `gravel_floor`) |
| 12 | `sparse_grass` | Sparse grass/exposed soil | <https://polyhaven.com/a/sparse_grass> |
| 13 | `mossy_rock` | Wet/mossy mountain rock | <https://polyhaven.com/a/mossy_rock> |
| 14 | `rock_face_03` | Rugged vertical cliff | <https://polyhaven.com/a/rock_face_03> |
| 15 | `ganges_river_pebbles` | Talus/river stone | <https://polyhaven.com/a/ganges_river_pebbles> — substituted for `dry_riverbed_rock` (rock face, overlapping dedicated cliff) |

Candidate substitution is allowed only after a visual/channel audit and only with another unambiguously redistributable CC0 material. Update the manifest and attribution if any candidate changes. Failed IDs remain in the draft manifest `rejected_for_role` list.

### 9.4 Core architecture decision

- Keep direct editable weights in **four RGBA splatmaps** for sixteen global layers.
- Enforce no more than four non-zero stored channels per texel. Painting a fifth deterministically decays/removes the smallest weights and renormalizes to 255.
- Bilinear filtering can expose more candidates at boundaries; the shader selects the strongest four before expensive PBR sampling.
- Do not adopt an indexed ID/weight control map initially. O3DE proves it can work, but correct filtering requires neighbouring gathers, ID deduplication, and reweighting, making painting/migration substantially more complex.
- Sidecar v3 copies v2 layers 0–7 exactly and initializes 8–15 to zero.
- Keep two packed texture arrays. Prefer BC7 when wgpu exposes `TEXTURE_COMPRESSION_BC`; retain RGBA8 fallback and never keep both resident.
- Default runtime resolution stays 2K; 4K remains opt-in.
- Generate semantic mips offline:
  - filter albedo in linear space and encode sRGB;
  - average and renormalize normals;
  - preserve linear height/AO;
  - increase mip roughness from unresolved normal variance.
- Full-PBR cliff projection must cover albedo, normal, roughness, AO, and height. Use biplanar as default, triplanar as reference/debug.
- Projected height participates in material blending, but POM/parallax ray marching is disabled on the projected cliff branch.
- Blend layer transitions through weighted surface gradients and use RNM for shared microdetail.
- The default scene and **Create → Terrain** must consume one versioned biome/material preset.

### 9.5 Why top four

O3DE supports many global surface materials but evaluates only a few locally. Official docs say top-three blending; **local source** (`TerrainRenderer/TerrainDetailMaterialManager.cpp`) stores **top-two IDs** + relative blend and gathers neighbours. Somnium keeps four **direct** splatmaps (not indexed IDs) to preserve four-way junctions. It must be compared against both top three and an offline all-layer reference.

With two packed maps, three hex samples, and four selected layers, the base worst case is 24 material-map taps rather than the current 48. A steep projected path may reach 36. The plan also defines average-tap, timing, memory, and image-error gates.

### 9.6 Compression and memory reasoning

Approximate full-mip material-array residency for sixteen layers:

| Pack | 2K | 4K |
|---|---:|---:|
| Two RGBA8 arrays | 682.7 MiB | 2.67 GiB |
| Two BC7 arrays | 170.7 MiB | 682.7 MiB |

Four 2K RGBA8 control maps add about 21.3 MiB. BC7 keeps the two-sample-per-layer packing and avoids splitting normals/scalars into more texture reads. The fallback exists for adapters without BC support.

### 9.7 Godot 4.7.1 additions

The user added `C:\Users\adhir\Downloads\GE\example_repo\godot-4.7.1-stable` after the first Phase XV plan.

Godot has no native 3D terrain/splat-material subsystem in the inspected source, so it does not replace O3DE/Unreal/Wicked/Fyrox for layer architecture. Two general systems are valuable:

1. `BaseMaterial3D` makes world/object triplanar mapping a material-wide coordinate policy, with normalized powered-normal weights and adjustable sharpness across PBR/detail channels. It also explicitly disables height-map parallax with triplanar mapping. Somnium uses this as corroboration for full-channel projection, bounded sharpness, and no projected POM—not as normal-projection code to copy.
2. `Image::generate_mipmap_roughness` reconstructs normal Z, builds a summed-area table, measures average-normal length over the target mip footprint, estimates unresolved normal variance, and raises roughness before compression. XV-B now requires an independently implemented Godot-reference comparison fixture.

Godot is MIT-licensed. Any adapted pattern must be independently expressed and cited in `ATTRIBUTION.md`; substantial copied portions would require the MIT notice, but copying is not planned.

### 9.8 Bethesda Game Studios additions

- BGS's Fallout 4 graphics overview connects tactile, distinct PBR materials with weather-driven wet surfaces. Phase XV therefore validates the full material response, including wet/dry states, rather than albedo screenshots alone.
- BGS's GDC 2016 Fallout 4 modular-level-design talk supports treating the sixteen materials and biome preset as a reusable, versioned **landscape kit** that a small team can iterate across the entire world.
- Bethesda's Fallout 76 overview describes six readable Appalachian regions. Phase XV uses combinations of a reusable material palette to establish biome identity rather than assigning one unique texture to every biome.
- BGS environment artist Megan Sawyer describes landscape-team review and regionally meaningful flora. Foliage is out of Phase XV scope, but material metadata retains biome/moisture tags for a later coherent scatter system.

### 9.9 Phase XV milestones

All remain **PLANNED**:

| Milestone | Scope |
|---|---|
| XV-A | Baseline, provenance, license, hashes, landscape-kit review matrix. |
| XV-B | Manifest-driven fetch/pack pipeline, semantic mips, BC7/RGBA8 outputs, Godot-reference roughness fixture. |
| XV-C | Sixteen-layer CPU/editor storage, four-splat authoring, sidecar v3 migration. |
| XV-D | GPU layout, strongest-four selection, shared terrain/ReSTIR material helpers, debug modes. |
| XV-E | Conditional BC compression, residency, mip/specular stability. |
| XV-F | Full-PBR biplanar cliffs, triplanar reference, bounded sharpness, no projected POM. |
| XV-G | Deterministic biome preset, paint overrides, shared startup/Create terrain creation. |
| XV-H | Physical scale, surface-gradient normals, macro/meso/micro, moisture wetness, optional histogram-preserving A/B. |
| XV-I | Sixteen-material native editor palette and diagnostics (incl. wetness / projection-axis views). |
| XV-J | Verification, performance, migration, evidence, documentation, attribution. |

The next implementation session begins with XV-A **only when the user authorizes implementation**. It must not jump directly to downloading textures or changing array sizes without first capturing the baseline/provenance evidence.

### 9.10 Second research pass (2026-08-13)

After Phase IV water closed as the photographic reference surface, Phase XV research was expanded so terrain can meet the same bar. New attributable material (full URLs in `phase_XV.md` §5.5 / §15):

| Addition | Consequence for XV |
|---|---|
| Losasso/Hoppe Geometry Clipmaps; Strugar CDLOD | Stay in Phase 25C — XV must not rewrite mesh LOD to sell materials. |
| Ka Chen Far Cry 4 AVT; Hooker CoD GDC 2021; Étienne SIGGRAPH 2023 | Strengthen VT deferral; define AAA “done” checklist without adopting VT. |
| Terrain3D (MIT) wetness paint + autoshader/override | Wetness validation first-class; paint wetness deferred past v1; moisture affinity in manifest. |
| PlumeSplat / PVTUT / Hollow-TerrainSystem | Confirm array+height-blend default; VT prototypes as bibliography only. |
| Mikkelsen surface-gradient bump (JCGT 2020) + demo | Mandatory inter-layer / cliff normal composition; RNM for microdetail only. |
| Hnat et al. porous wetting (2006) | Dry/damp/wet = albedo darken + roughness drop + slight F0; Great Lakes shore fixture. |
| ambientCG CC0 API | Explicit fallback beside Poly Haven. |
| Water-parity bar | XV fails if materials only look good as flat albedo swatches next to shipping water. |

**Still no Phase XV code or textures.**

## 10. Phase XV reasoning and deferred alternatives

| Decision / alternative | Reasoning / status |
|---|---|
| Four direct splatmaps | Preserves hardware filtering, painting, undo, serialization, and simple v2 migration. |
| Indexed IDs + weights | Deferred; filtering and editor behavior require complex neighbourhood gather/dedup logic. |
| Runtime virtual texturing/detail clipmap | Deferred; valuable for massive worlds, but disproportionate for the current bounded 1 km editable terrain unless profiling fails. |
| BC7 two-array pack | Preferred balance of residency, four-channel preservation, and existing two-fetch layout. |
| Split BC5 normals/scalars | Rejected initially because it raises sample count and binding complexity. |
| Full multilayer POM | Rejected; divergent sampling cost is poor at blended transitions. Dominant-layer POM remains only on non-projected terrain. |
| LEAN mapping | Rejected for this phase due extra moments/storage; use normal-variance roughness compensation first. |
| Tessellation/true displacement | Rejected; it changes geometry, collision, shadows, and LOD beyond the material goal. |
| Texture bombing | Rejected initially because practical hex tiling already supplies anti-repetition. |
| Histogram-preserving randomized blending | Conditional experiment only if A/B evidence shows contrast washout. |
| Mix-Max transitions | Research reserve if existing height-aware blending is insufficient. |
| Mesh scatter, decals, foliage | Future phase. Photoscanned materials alone cannot provide all close-range natural detail, but adding scatter now would hide material-system defects. |
| Quixel/Megascans | Not selected. Current redistribution terms require a separate current-license review; Poly Haven CC0 already covers the roster. |

## 11. Phase XV complete reference inventory

### Asset quality, licensing, and candidates

- Poly Haven CC0 license: <https://polyhaven.com/license>.
- Poly Haven contribution/quality standards: <https://polyhaven.com/contribute>.
- Poly Haven API information: <https://polyhaven.com/el/our-api>.
- CC0 1.0 Universal: <https://creativecommons.org/publicdomain/zero/1.0/>.
- Vecchio et al., *MatSynth: A Modern PBR Materials Dataset*, CVPR 2024: <https://openaccess.thecvf.com/content/CVPR2024/html/Vecchio_MatSynth_A_Modern_PBR_Materials_Dataset_CVPR_2024_paper.html>.
- DICE, photogrammetry article: <https://www.ea.com/news/photogrammetry-and-star-wars-battlefront>.
- DICE, GDC 2016 photogrammetry slides: <https://media.gdcvault.com/gdc2016/Presentations/Brown_Kenneth_Hamilton_Andrew_PhotogrammetryStarWars.pdf>.
- ambientCG, CC0 fallback source: <https://ambientcg.com/>.
- Exact Poly Haven candidate URLs are in section 9.3.

### Terrain rendering and authoring

- Andersson, *Terrain Rendering in Frostbite Using Procedural Shader Splatting*, SIGGRAPH 2007: <https://advances.realtimerendering.com/s2007/Andersson-TerrainRendering%28Siggraph07%29-CourseNotes.pdf>.
- Losasso & Hoppe, *Geometry Clipmaps*, SIGGRAPH 2004 (added 2026-08-13): <https://hhoppe.com/geomclipmap.pdf>.
- Strugar, *CDLOD*, JGT 2009 (added 2026-08-13): <https://aggrobird.com/files/cdlod_latest.pdf>.
- Ka Chen, *Adaptive Virtual Texture Rendering in Far Cry 4*, GDC 2015 (added 2026-08-13): <https://www.gdcvault.com/play/1021761/>.
- JT Hooker, *Boots on the Ground: The Terrain of Call of Duty*, GDC 2021 (added 2026-08-13): <https://research.activision.com/publications/2021/09/boots-on-the-ground--the-terrain-of-call-of-duty>.
- Étienne, *Large Scale Terrain Rendering in Call of Duty*, SIGGRAPH 2023 Advances (added 2026-08-13): <https://advances.realtimerendering.com/s2023/Etienne%28ATVI%29-Large%20Scale%20Terrain%20Rendering%20with%20notes%20%28Advances%202023%29.pdf>.
- Mikkelsen, *Practical Real-Time Hex-Tiling*, JCGT 2022: <https://jcgt.org/published/0011/03/05/>.
- Burley, *On Histogram-Preserving Blending for Randomized Texture Tiling*, JCGT 2019: <https://jcgt.org/published/0008/04/02/>.
- O3DE Terrain Surface Materials List: <https://www.docs.o3de.org/docs/user-guide/components/reference/terrain/surface-material-list/>.
- O3DE Terrain Detail Material: <https://www.docs.o3de.org/docs/user-guide/components/reference/terrain/terrain-detail-material/>.
- O3DE Terrain Macro Material: <https://docs.o3de.org/docs/user-guide/components/reference/terrain/terrain-macro-material/>.
- O3DE terrain texture tutorial: <https://docs.o3de.org/docs/learning-guide/tutorials/environments/create-terrain-from-images/texture-terrain/>.
- Ubisoft, *Terrain Rendering in Far Cry 5*, GDC 2018: <https://www.gdcvault.com/play/1025261/Terrain-Rendering-in-Far-Cry>.
- Far Cry 5 slides: <https://media.gdcvault.com/gdc2018/presentations/TerrainRenderingFarCry5.pdf>.
- Ubisoft, *Ghost Recon Wildlands Terrain Technology and Tools*, GDC 2017: <https://www.gdcvault.com/play/1024029/-Ghost-Recon-Wildlands-Terrain>.
- Ghost Recon slides: <https://media.gdcvault.com/gdc2017/Presentations/WERLE_MARTINEZ_GRWterrainTechnologyTools.pdf>.
- Epic Games Runtime Virtual Texturing: <https://dev.epicgames.com/documentation/unreal-engine/runtimevirtual-texturing-quick-start-in-unreal-engine>.
- NVIDIA, GPU geometry clipmaps: <https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry>.
- NVIDIA, texture bombing: <https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-20-texture-bombing>.
- TokisanGames, Terrain3D (MIT; wetness paint; added 2026-08-13): <https://github.com/TokisanGames/Terrain3D>.
- DrewRidley, PlumeSplat (added 2026-08-13): <https://github.com/drewridley/plumesplat>.
- ACskyline, PVTUT (added 2026-08-13): <https://github.com/ACskyline/PVTUT>.

### Material filtering, projection, and wetness

- Mikkelsen, *Surface Gradient–Based Bump Mapping Framework*, JCGT 2020 (added 2026-08-13): <https://jcgt.org/published/0009/03/04/>.
- Mikkelsen, surfgrad demo (added 2026-08-13): <https://github.com/mmikk/surfgrad-bump-standalone-demo>.
- Hill, *Blending in Detail — Reoriented Normal Mapping*: <https://blog.selfshadow.com/publications/blending-in-detail/>.
- Toksvig, *Mipmapping Normal Maps*: <https://www.tandfonline.com/doi/abs/10.1080/2151237X.2005.10129203>.
- Olano and Baker, *LEAN Mapping*: <https://userpages.cs.umbc.edu/olano/papers/lean/>.
- *Triplanar Displacement Mapping for Terrain*, Eurographics 2020: <https://diglib.eg.org/server/api/core/bitstreams/b3af0317-e2d6-4e3a-8076-b415516eee87/content>.
- *Mix-Max: Content-Aware Real-Time Texture Transitions*, Eurographics 2024: <https://diglib.eg.org/items/50375852-f98b-4f60-ae25-4ae06ad038d1>.
- Hnat et al., *Real-time Wetting of Porous Media*, ICCVG 2006 (added 2026-08-13): <http://damien.porquet.free.fr/msi/iccvg06/iccvg06.pdf>.
- ambientCG docs (fallback; added 2026-08-13): <https://docs.ambientcg.com/>.

### Bethesda Game Studios sources

- BGS, *The Graphics Technology of Fallout 4*: <https://bethesda.net/tr-TR/news/the-graphics-technology-of-fallout-4>.
- Burgess and Purkeypile/BGS, *Fallout 4's Modular Level Design*, GDC 2016: <https://www.gdcvault.com/play/1022930/-Fallout-4-s-Modular>.
- GDC slides: <https://media.gdcvault.com/gdc2016/Presentations/Burgess_Joel_Modular%20Level%20Design.pdf>.
- BGS, *What is Fallout 76?*: <https://fallout.bethesda.net/en-EU/news/what-is-fallout-76>.
- BGS, *Meet Megan Sawyer — Senior Environment Artist*: <https://bethesda.net/tr-TR/news/meet-megan-sawyer-senior-environment-artist-at-bethesda-game-studios>.

### API and compression

- wgpu 29 `Features`: <https://docs.rs/wgpu/29.0.0/wgpu/struct.Features.html>.
- WebGPU `GPUFeatureName`: <https://gpuweb.github.io/types/types/GPUFeatureName.html>.

### Local engine source inspected for Phase XV

- O3DE `Gems/Terrain/Assets/Shaders/Terrain/TerrainDetailHelpers.azsli` — height-to-weight blending and bounded material-context handling.
- O3DE `TerrainDetailMaterialManager.cpp` — compact top-material ID/relative blend representation and neighbour gather/dedup.
- O3DE `AzFramework/SurfaceData/SurfaceData.h` — global surface-weight capacity.
- Bevy triplanar plugin `src/shaders/biplanar.wgsl` — explicit-gradient two-axis projection and axis-transition limitations.
- Wicked Engine `wiTerrain.cpp` — material arrays and sparse/virtual terrain organization.
- Unreal Landscape weightmap and Runtime Virtual Texture source — layer controls, caching, landscape/actor coupling.
- Fyrox terrain material/mask source — per-layer authoring stack.
- Godot `scene/resources/material.cpp` — full-channel world/object triplanar generation.
- Godot `doc/classes/BaseMaterial3D.xml` — triplanar cost/quality and height-parallax incompatibility.
- Godot `core/io/image.cpp` — `Image::generate_mipmap_roughness`.
- Godot `editor/import/resource_importer_texture.cpp` — normal/roughness association and processing order.
- Godot `LICENSE.txt` — MIT license.

## 12. Validation and evidence already recorded

Phase IV's authoritative counts and exact commands remain in `phase_IV.md`. Important evidence paths:

- `dev records/phase IV/IV-D-E/IV-D-E_day_post-TAA.png`
- `dev records/phase IV/IV-D-E/IV-D-E_night_post-TAA.png`
- `dev records/phase IV/IV-F-G-H/IV-F-G-H_surface_day.png`
- `dev records/phase IV/IV-F-G-H/IV-G_underwater_deep.png`
- `dev records/phase IV/IV-F-G-H/IV-G_waterline_transition.png`
- `dev records/phase IV/IV-I-J/IV-I-J_runtime_validation.png`
- `dev records/phase IV/IV-I-J/IV-I-J_shoreline_lod_validation.png`

The final Phase IV record reports successful formatting, workspace compilation/tests, 209 renderer tests, all 9 shader-module tests, targeted Clippy, and runtime captures on the user's adapter. Re-run current tests before future changes; historical passing counts are evidence, not a guarantee about a later worktree.

Phase XV has no runtime evidence because it has not been implemented. Future screenshots belong under `dev records/phase XV/evidence/phase_XV-<subphase>_<purpose>.png`.

## 13. Known issues outside the completed Phase IV claim

`context.md` still records these issues; they were not solved by the post-25M2 work and should not be silently folded into Phase XV:

- Foliage currently renders with wrong colours (trees salmon/pink, grass white); not yet investigated.
- Editor primitives spawned during `on_init` upload and receive gizmos but do not appear; cause unknown.
- `BUG-013` records water normal/mipmap texture seams as pending. Phase IV replaced much of the old plane path and added complete water mips, so reproduce this before assuming the note still applies.
- Earlier known limitations remain in other systems, including incomplete foliage realism/performance work and deferred renderer phases. Consult `context.md`, not this handoff, for the entire historical backlog.

These are context, not authorization to expand a Phase XV implementation into unrelated fixes.

## 14. Next-session start checklist

> Prefer [`post_IV_context_handoff.md`](post_IV_context_handoff.md) §10 for XV starts. Checklist below is retained for continuity.

1. Confirm the branch/HEAD and inspect changes made after `846dea7` / `2dec6bd`.
2. Read the authoritative files from the post-IV handoff section 1 (or this file’s section 1 for IV A–J depth).
3. Confirm Phase IV still builds before changing terrain-material layouts.
4. Begin **XV-A only** when authorized:
   - capture the existing eight-layer visual/performance/memory baseline;
   - freeze camera/adapter/reference scene details;
   - create the provenance/manifest schema;
   - re-verify each Poly Haven candidate, physical scale, channels, CC0 page, and hashes before committing assets;
   - define the dry/damp/wet/day/night biome review matrix including the Great Lakes shore fixture.
5. Preserve indices 0–7 and sidecar v2 behavior.
6. Do not download or commit Quixel/Megascans content.
7. Do not implement RVT, indexed splat IDs, LEAN mapping, full multilayer POM, tessellation, foliage scatter, geometry clipmaps, or CoD/AVT virtual texturing unless the Phase XV evidence gates explicitly justify reopening them — and only after the user authorizes XV implementation.
8. Update `context.md` and `ATTRIBUTION.md` after each completed XV subphase, not in advance.
9. Keep all future evidence under `dev records/phase XV/evidence/`.
10. Until the user says to implement XV, treat `phase_XV.md` as research/docs only: expand bibliography and decisions freely; do not touch terrain shaders, splat layout, or texture packs.

## 15. Accuracy rule for future sessions

Use the implementation and tests as truth, then reconcile documentation. This handoff is a snapshot at `846dea7`. Phase IV claims are implemented; Phase XV claims are researched decisions and acceptance targets. If a later session changes an architectural choice, material candidate, license interpretation, performance budget, or milestone order, update `phase_XV.md` and this handoff with the reason and evidence rather than allowing the documents to diverge.

---

**AI disclosure:** This handoff was reconstructed from the post-`4e56482` commit range, current source tree, living documentation, phase records, attribution index, asset provenance, and recorded evidence. It summarizes engineering reasoning and source-use boundaries; it does not replace the exact licenses or upstream source material linked above.
