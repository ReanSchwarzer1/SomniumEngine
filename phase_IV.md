# Phase IV — Great Lakes Landscape and Black Flag Water

**Project:** Somnium Engine  
**Status:** Research complete; implementation not started  
**Plan date:** 2026-08-11  
**Codename:** Black Flag  
**Target:** Rust 1.85, wgpu 29, winit 0.30

## 1. Outcome

Phase IV will replace the current demo terrain/water pairing with one shared, production-shaped landscape preset:

- the Motion Forge Pictures **Great Lakes** height field becomes the default terrain source;
- the terrain receives a real, finite, depth-bearing water body rather than a small generic plane;
- the water is a first-class ECS entity visible and editable in the Outliner;
- above-water, shoreline, partial-submersion, and underwater views use the same water data;
- application startup and **Create → Terrain** call one landscape factory, so they cannot drift;
- asset and reference provenance is recorded in `assets/LICENSE.md` and `ATTRIBUTION.md` before the new default ships.

This is a plan only. No Great Lakes files have been copied and no renderer behavior has been changed in this phase-planning pass.

## 2. Research conclusion

### 2.1 The current terrain artifact is not solved by swapping images alone

The supplied screenshots show discontinuities aligned to individual terrain triangles. The Great Lakes source is smooth: its FLOAT32 height channel has no NaN/Inf values, and 99.9% of adjacent-pixel changes are below approximately `0.00233`. The current artifacts therefore remain a renderer/geometry problem until disproved. Likely contributors are:

- LOD-dependent triangle-fan topology;
- geometric frequency exceeding the active LOD sampling rate;
- normal or receiver-bias discontinuities evaluated per face;
- vertical exaggeration after normalization;
- shadow/contact-shadow self-intersection.

The Great Lakes asset improves art direction and supplies natural basins, but Phase IV must first establish that a flat ramp and a smooth analytic hill render without triangle-aligned lighting or shadow discontinuities.

### 2.2 Direct EXR loading is currently unsafe

`Height Map.exr` is a 2048×2048, one-channel, 32-bit float OpenEXR. Somnium's current generic height-image path preserves `Luma16`, but sends every other dynamic image through `to_luma8()`. Because the source occupies only part of the nominal `0..1` range, direct loading would retain only about 47 distinct levels. At 90 m relief that is approximately 1.9 m per step—enough to manufacture severe visible terraces from a smooth source.

Phase IV must add a FLOAT32-aware import path and a regression test before changing `DEFAULT_HEIGHTMAP`.

### 2.3 The asset contains water surfaces, not underwater terrain

The height field contains large exact plateaus, including one value covering roughly 23% of the image, plus several smaller repeated levels. These are useful water-surface hints, but the package has no bathymetry or explicit mask. A flat plateau is not a lakebed.

The importer must derive persistent water masks and create a submerged bed beneath each selected water surface. Otherwise the surface and ground will be coplanar, water depth will be zero, and underwater rendering cannot work.

### 2.4 Water remains a specialized pass

Water must not replace the opaque terrain primitive in the visibility buffer. Refraction and water thickness need the terrain color and depth that exist beneath the surface. The correct architecture is a specialized pass after opaque shading and before TAA/translucency post effects.

## 3. Evidence and source grading

| Evidence | Grade | Use in Phase IV | Limitation |
|---|---|---|---|
| Local Wicked Engine ocean and underwater source | A — primary source | FFT displacement, projected grid, gradients, foam, refraction, extinction, underwater transition | HLSL/C++; patterns must be translated, never copied |
| Tessendorf, *Simulating Ocean Water* | A — foundational primary source | Spectral waves, dispersion, choppiness, optical principles | Ocean height fields do not model breaking waves or local fluid volume |
| NVIDIA GPU Gems water chapters | A — primary technical reference | Gerstner waves, analytic normals, depth attenuation, caustic tiers | Older hardware assumptions; math remains applicable |
| Eidos Montréal, *From Shore to Horizon* | A — production technical talk | Shore SDF, coastal waves, wet sand, underwater plate, quality layering | Its shipped approach did not support an underwater camera |
| Unreal Single Layer Water and Water Body documentation | A — official engine documentation | Pass ordering, water-body ownership, surface/volume separation | Architecture reference, not a code source |
| AMD FidelityFX SSSR | A — official implementation documentation | Hierarchical SSR quality tier and denoising | SSR still cannot see off-screen/hidden geometry |
| Monzon et al., *Real-Time Underwater Spectral Rendering* | A — peer-reviewed primary source | Physically grounded underwater attenuation/scattering presets | Full spectral transport is beyond the initial RGB implementation |
| Ubisoft ACIII/ACIV interviews and GDC material | A/B — primary talk plus production reporting | Beaufort-style controls, shallow/deep LODs, foam, SSR, systemic weather | Interviews describe outcomes more than exact algorithms |
| Supplied Wicked Engine video, *Underwater effect updates* | B — visual target | Partial-submersion and underwater look acceptance | No implementation explanation |
| Motion Forge first-party asset/catalog pages | A — provenance source | Great Lakes authorship, format, and asset-specific CC0 statement | General site EULA conflicts with the specific catalog statement |

## 4. Current Somnium baseline

### Terrain

- `TerrainComponent` is a lightweight ECS handle into renderer-owned `TerrainData`.
- The default descriptor is 16×16 chunks, 64 cells per chunk: 1025×1025 vertices across 1024 m.
- **Create → Terrain** and the demo both call `apply_default_relief`, but spawn logic is still duplicated.
- Height/splat data is renderer-owned and scene sidecars persist it.
- Terrain already participates in the visibility buffer and central opaque shading.

### Water

- `WaterComponent` exists in the ECS, but the demo manually attaches it to a generated 20 m plane.
- The current water pass already reads opaque depth, a pre-water HDR copy, sun/shadows, and the environment.
- It has depth tint, refraction, Fresnel-like blending, PCF shadows, and edge foam.
- It does not own a finite footprint, water mask, bathymetry, volume containment, motion vectors, persistent simulation state, or gameplay queries.
- Water is absent from Create-menu authoring, undo snapshots, scene serialization, and inspector plumbing.
- Texture binding is demo-global rather than water-body-owned.
- `WaterPass::record` currently creates material/instance buffers and bind groups for every water draw every frame; persistent per-body GPU resources are a prerequisite for FFT/history work.

## 5. Requirements

### Functional

1. Great Lakes-derived terrain and water appear in the normal default scene.
2. **Create → Terrain** creates the same landscape configuration through the same factory.
3. The Outliner contains a `Terrain` entity and a child `Water` entity.
4. Water is finite and follows an explicit mask; it cannot cover dry terrain outside its body.
5. Water has actual distance-to-bed data and a camera containment query.
6. Saving/loading, duplicate, delete, undo, and redo preserve both terrain and water without leaking renderer resources.
7. Water supports sun/sky reflection, refraction, absorption, scattering, shore/crest foam, and shadows.
8. Crossing the surface transitions continuously into an underwater medium.
9. Terrain sculpt changes can invalidate and rebuild dependent shoreline/depth data.
10. Wave height, normal, depth, and velocity are queryable for gameplay/physics.

### Non-functional

- Preserve the visibility-buffer architecture for opaque geometry.
- Use archetype ECS columns; heavy textures, grids, masks, FFT state, and readback live in renderer-owned storage.
- All depth/absorption math uses linear world/view distances, never raw nonlinear depth-buffer differences.
- No tessellation-shader dependency; wgpu/WGSL has no tessellation stage.
- Water quality tiers must work without optional float atomics or hardware ray tracing.
- Default rendering must remain deterministic under fixed seed/time for captures and tests.
- Water must provide motion vectors/reactive data so TAA does not smear foam, highlights, or refracted geometry.

## 6. Target architecture

### 6.1 ECS and renderer ownership

The existing `WaterComponent` should evolve rather than be discarded. `WaterBodyComponent` below is the conceptual target name; keep the public name or provide a serialized migration if renaming would break scenes. The component remains small and copyable, identifies renderer-owned `WaterBodyData`, and carries stable authoring settings:

- `water_id`;
- body kind (`Lake`, later `Ocean` and `River`);
- surface datum and finite XZ bounds;
- source terrain relationship;
- optical and wave preset identifiers;
- enabled/render/physics flags.

Optional SoA-friendly components hold editable parameter groups:

- `WaterOpticsComponent`: absorption, scattering, anisotropy, turbidity, roughness;
- `WaterWavesComponent`: wind, amplitude, wavelength range, choppiness, seed, simulation tier;
- `WaterFoamComponent`: shore width, crest threshold, accumulation, decay;
- `WaterInteractionComponent`: ripple/wake enablement and query quality.

Renderer-owned `WaterBodyData` contains:

- water coverage mask;
- signed shoreline-distance field;
- bathymetric/depth map;
- wet-sand band and shore normals;
- projected-grid or clipmap geometry;
- wave displacement/gradient/foam history textures;
- SSR/reflection history and TAA reactive data;
- CPU query cache/readback only while a caller requests it.

The `Water` entity is parented to its `Terrain` entity. Scene serialization stores stable authoring data and rebuilds runtime `water_id` handles on load.

### 6.2 Shared landscape factory

Add one engine-level landscape factory used by both startup and **Create → Terrain**:

```text
create_default_landscape()
  ├─ import/instantiate terrain preset
  ├─ apply material auto-splat + dry-land macro tint
  ├─ load or generate water masks/depth/SDF
  ├─ create Terrain ECS entity
  ├─ create child Water ECS entity
  └─ return one atomic editor command/result
```

The factory, not `hello_engine`, owns defaults. Undo/redo treats the terrain-water pair as one composite creation while retaining separate Outliner entities.

### 6.3 Render graph

```text
Opaque visibility + depth
        ↓
Fullscreen opaque shading → HDR A
        ↓
Water surface coverage/depth/normal/motion
        ↓
Water SSR / planar-reflection quality tier
        ↓
Water composite → HDR B
  samples HDR A + opaque depth + shadow/environment + water data
        ↓
Underwater medium + caustics + submerged light shafts
        ↓
TAA / bloom / tone map
        ↓
Native wgpu UI
```

HDR ping-pong is preferred over repeated full-resolution copies. The current explicit pre-water copy may remain as the first safe milestone, then be replaced only if profiling justifies it.

### 6.4 Finite surface geometry

The preferred Great Lakes design is a finite, camera-relative, terrain-aligned patch/clipmap clipped by body bounds and the persistent water mask. It is not a `MeshKind::Plane`:

- clipmap rings concentrate vertices near the camera while preserving a stable terrain/water correspondence;
- large waves displace vertices in world space;
- fine waves move into gradient/normal and roughness representation with distance;
- depth test plus mask prevents water over dry terrain;
- shore-wave amplitude fades with bathymetric depth;
- grid edge displacement is suppressed or skirted to avoid cracks.

Wicked/ACIII-style projected-grid rendering remains an explicit prototype and is likely preferable for a later infinite-ocean mode. Phase IV-D compares both approaches at grazing views, shorelines, partial submersion, and in GPU timings before locking the finite-lake implementation.

## 7. Great Lakes asset pipeline

### Source audit

| Source file | Technical use | SHA-256 |
|---|---|---|
| `Height Map.exr` | FLOAT32 source height and plateau classification | `d608ec2e62a40e38ff3a65180c6e017b14422496920a1f517d9aa691e2f252b9` |
| `Diffuse Map.exr` | Linear dry-land macro-colour source | `45cc8c1e4a2698ff01de2a441e8ad2cf822bf4bae29dfb95dc9a7a20a38dce17` |
| `Great Lakes.png` | Preview/provenance only | `7fad06f049f7503ea518a435d03833483ebfc0b539ffdebcccfc7b8c962add77` |

### Import rules

1. Decode `ImageLuma32F`, `ImageRgb32F`, and `ImageRgba32F` without 8-bit conversion.
2. Normalize the height source explicitly from its audited finite range, with logged min/max and an option for authored scale/offset.
3. Area-filter 2048 source samples to Somnium's 1025-vertex convention; preserve source corners and never confuse samples with cells.
4. Bake the runtime height into the engine terrain format at 16-bit or better.
5. Recompute normals from the final resampled/displaced height grid.
6. Treat the diffuse EXR as linear. Encode to sRGB before storing in an sRGB runtime texture.
7. Use diffuse only as low-frequency dry-land tint. Mask its baked blue water pixels from terrain macro colour.
8. Clamp addressing; the source is not seamless.
9. Detect selected repeated water levels offline and bake explicit masks.
10. Carve/synthesize smooth bathymetry below those masks with configurable shore slope and maximum depth; never modify dry terrain elevations.
11. Bake shoreline SDF, depth, shore normal/slope, and wet-sand band as sidecar data.
12. Commit optimized runtime derivatives. Raw EXRs may be retained as provenance only after the license record and repository-size decision are accepted.

## 8. Implementation milestones

### IV-A — Terrain truth pass and precision import

**Work**

- Add FLOAT32 EXR preservation and importer tests.
- Add debug views for terrain LOD, triangle edges, vertex normals, shadow factor, and contact-shadow hits.
- Test flat, ramp, sinusoidal hill, old default, and Great Lakes-derived height fields.
- Revisit the per-face shadow receiver normal; terrain must use a stable smooth macro normal or a slope-limited hybrid without creating per-triangle bias steps.
- Validate LOD wave-frequency filtering and vertical scale.

**Exit gate**

- No triangle-aligned discontinuity appears on analytic smooth terrain at the supplied camera/sun angles.
- FLOAT32 EXR ramp retains more than 8-bit precision after import.
- Adjacent chunk and LOD boundaries have matching positions and shading normals.

### IV-B — Great Lakes landscape bake

**Work**

- Add an offline/import-time Great Lakes recipe with recorded scale, offset, resampling kernel, and checksums.
- Produce optimized height and macro-colour derivatives.
- Detect/choose water plateaus and generate masks.
- Synthesize bathymetry and shoreline SDF/depth products.
- Tune terrain relief and camera placement from physical-looking slope targets, not full-range exaggeration.
- Auto-splat the existing eight terrain materials, then apply masked low-frequency macro colour.

**Exit gate**

- The terrain reads as smooth at eye level and landscape distance.
- Lake regions are explicit masks with non-zero, inspectable depth.
- No coplanar terrain/water surfaces remain.
- Reimport is deterministic from the source hashes and recipe.

### IV-C — First-class ECS water body

**Work**

- Evolve the demo's mesh-plus-`WaterComponent` convention into a finite water-body component and renderer-owned `WaterBodyData`; rename only with a scene migration.
- Add Water to the Create system if independent bodies are desired; **Create → Terrain** always creates the companion default water child.
- Add Outliner hierarchy, inspector groups, undo/redo, duplicate/delete, and scene serialization.
- Move water texture/resource creation out of `hello_engine` and into renderer asset ownership.
- Add lifecycle tests for create/delete/load and invalid parent terrain.

**Exit gate**

- Water is independently selectable in the Outliner.
- Scene round-trip reproduces parameters, mask/depth references, and parent relation.
- Deleting/undoing the landscape frees/recreates both terrain and water GPU state without leaks or stale handles.

### IV-D — Finite surface, depth, and query contract

**Work**

- Prototype terrain-aligned clipmap and projected-grid surfaces, then select the finite-lake path from captured correctness and timing evidence.
- Produce water coverage, surface depth, normals, and motion vectors.
- Add CPU/gameplay queries: `surface_height`, `surface_normal`, `depth`, `velocity`, and `contains_point`.
- Use deterministic Gerstner waves as the first queryable displacement layer.
- Add depth-based shore attenuation and Nyquist filtering against grid density.

**Exit gate**

- No water draws beyond mask/bounds.
- CPU and GPU surface queries agree within a documented tolerance.
- TAA remains stable during camera and wave motion.

### IV-E — Physically coherent surface optics

**Work**

- Use dielectric water `F0 ≈ 0.020` and GGX direct/environment specular.
- Validate refracted UVs against opaque depth; fall back to unperturbed sampling when displacement reveals invalid foreground.
- Apply Beer–Lambert RGB transmittance from reconstructed linear path length.
- Add approximate single scattering using named clear-freshwater, turbid-lake, coastal, and ocean presets.
- Implement SSR with environment fallback and edge/confidence fading.
- Evaluate low-resolution planar reflection as a high-quality lake option, not a mandatory default.
- Light water from the real sun/moon/environment and shared shadows.

**Exit gate**

- Shallow ground remains visible and progressively loses red/green energy with depth.
- Grazing water reflects the environment while normal incidence remains transmissive.
- SSR failure regions fade to an environment/planar fallback rather than holes.
- Day, dusk, and night captures remain finite and temporally stable.

### IV-F — Multi-scale waves, shoreline, and foam

**Work**

- Retain Gerstner as the deterministic baseline and low-end quality tier.
- Add optional 256²/512² Stockham inverse-FFT cascades based on a wind spectrum; use at least two spatial scales for the cinematic tier.
- Generate displacement, analytic/finite-difference gradients, and horizontal-displacement Jacobian.
- Derive crest foam from Jacobian/curvature and shore foam from SDF/depth/wave phase.
- Accumulate/decay foam temporally and generate a synchronized wet-sand band.
- Move unresolved distant wave energy into normal variance/roughness to prevent horizon sparkle.

**Exit gate**

- Calm/storm presets transition without discontinuities.
- Foam is limited to shoreline and breaking crests, persists briefly, and decays.
- No obvious FFT tile seam or short-period repetition is visible from the default camera path.

### IV-G — Underwater medium and partial submersion

**Work**

- Determine camera containment from the active water body and displaced surface.
- Build a per-pixel partial-submersion mask at the near plane; avoid a binary full-screen switch.
- Apply path-length absorption, in-scattering, sun-direction phase, and submerged fog in HDR.
- Render the water underside with refraction/reflection and total-internal-reflection behavior.
- Add depth-faded projected caustics as the portable baseline.
- Reuse volumetric shadow/light-shaft infrastructure for submerged shafts where stable.
- Implement original WGSL math for transition/distortion/light shafts; do not inherit Wicked's Shadertoy-cited Brown–Conrady and stylized god-ray helper code by accident.
- Keep RGB coefficients data-driven so a later spectral approximation can replace them without changing the ECS contract.

**Exit gate**

- Slowly crossing the surface has no full-screen pop, seam, or unbounded distortion.
- Underwater colour changes continuously with camera depth and object distance.
- The surface is visible from below and exhibits an intelligible Snell-window/TIR transition.
- Caustics disappear with depth/turbidity and do not project above water.

### IV-H — One default landscape everywhere

**Work**

- Route normal startup and **Create → Terrain** through `create_default_landscape`.
- Remove demo-owned water plane/texture setup.
- Make the default camera, water datum, terrain transform, material thresholds, and post-processing part of a versioned landscape preset.
- Make compound creation one undoable transaction while preserving two Outliner entities.
- Add a structural regression test comparing startup and Create-menu descriptors/entity graphs.

**Exit gate**

- Startup and UI-created landscapes have identical source recipe, terrain descriptor, water preset, masks, materials, and hierarchy.
- The old `WaterPlane` entity and hard-coded 20 m geometry no longer exist.

### IV-I — Interaction tier (after the visual/volume foundation)

**Work**

- Ripple-normal injection for rain, footsteps, projectiles, and small impacts.
- Wakes and Kelvin-style trails for moving bodies.
- Buoyancy sampling through the shared query API.
- Localized shallow-water solver near actors/shore only after profiling and stability tests.
- Spray, bubbles, and breaking-wave particles as effects; do not pretend a height field can represent overturning fluid.

**Exit gate**

- Interaction is optional and cannot destabilize the base surface or scene save/load.
- Server/CPU users can query deterministic water without requiring full GPU readback every frame.

### IV-J — Documentation, attribution, and completion evidence

- Update `context.md` after every completed sub-phase.
- Add pattern-level citations to `ATTRIBUTION.md` before translating each reference.
- Add asset provenance and transformations to `assets/LICENSE.md` before committing derivatives.
- Record fixed-seed screenshots and GPU timings for day/night, above/below surface, shore/open water, and all quality tiers.
- Run shader validation, renderer tests, scene round-trip tests, `cargo test --workspace`, and `cargo clippy` on touched crates.

## 9. Validation matrix

| Area | Required evidence |
|---|---|
| Terrain precision | FLOAT32 ramp/import test; histogram and min/max log; no 8-bit plateaus |
| Terrain topology | Fixed-camera flat/ramp/hill captures with triangle/LOD/normal/shadow debug overlays |
| Asset bake | Deterministic outputs from source hashes; verified water masks and bathymetry |
| ECS | Archetype query, Outliner selection, inspector edit, save/load, duplicate/delete, undo/redo |
| Water boundary | Dry pixels never receive water; shore mask/SDF debug view |
| Optics | Fresnel-angle sweep; linear-depth Beer–Lambert numeric tests; invalid-refraction fallback |
| Waves | Deterministic Gerstner query test; FFT energy/periodicity tests; no NaN/Inf |
| Temporal | Motion-vector and reactive-mask debug; moving camera/waves/foam under TAA |
| Underwater | Partial-submersion sweep; distance/depth attenuation; below-surface TIR/caustics |
| Lighting | Noon, sunset, moonlit night, shadowed shore, indoor/occluded water |
| Performance | GPU timestamp per water sub-pass at 1080p; memory report by quality tier |
| Portability | wgpu validation on supported backends without optional float atomics |

Initial GPU targets are provisional until a baseline is recorded on the selected adapter:

- Gerstner/default lake simulation: ≤ 1 ms;
- water coverage + composite: ≤ 2 ms;
- SSR/underwater/caustics combined: ≤ 3 ms at the default quality tier;
- no unbounded per-body allocations or per-frame bind-group creation.

If the current engine baseline cannot meet these absolute targets, Phase IV records both absolute time and percentage regression, then chooses quality defaults from measured data.

## 10. Decisions and trade-offs

### Gerstner first, FFT second

Gerstner waves are deterministic, cheap, differentiable, and easy to query for physics. FFT gives richer wind-driven spectra but adds compute passes, storage, tiling, readback, and debugging cost. The API supports both; the milestone order prevents FFT work from blocking ECS, depth, optics, or underwater correctness.

### Derived runtime assets, documented source

The downloaded package is about 65.4 MiB and the EXRs are uncompressed. Runtime derivatives are smaller and remove import ambiguity. Source EXRs may be retained only after repository-size and licensing hygiene are accepted.

### Automatic bathymetry is an authored approximation

The Great Lakes package cannot supply real lake depth. Phase IV-generated beds are visually plausible engine data, not geographic bathymetry. This must be documented in asset metadata and UI naming.

### SSR needs a fallback

SSR cannot see off-screen or hidden objects. It is a useful near-field reflection tier, never the sole reflection source. Environment reflection is mandatory; planar reflection is optional for the flagship lake preset.

### RGB underwater transport first

Three-channel absorption/scattering is not spectrally exact, but it is compatible with the current HDR pipeline and can be calibrated to measured water types. The data model keeps room for a later spectral approximation.

## 11. Attribution and license actions

The Motion Forge catalog specifically states that heightmaps on the page are provided under “CCO 1.0 Universal,” evidently a typo for CC0 1.0 Universal. The general site EULA simultaneously describes downloaded products as personal/non-transferable and restricts sharing originals. The specific first-party CC0 grant reasonably supports redistribution, but the conflict must be preserved in the provenance record.

Before any asset commit:

1. save a dated record/screenshot of the catalog's CC0 statement;
2. record the asset page, catalog page, CC0 legal code, access date, author, source hashes, and every transformation;
3. prefer processed runtime derivatives over raw downloads;
4. voluntarily provide both credits below;
5. optionally request written confirmation from Motion Forge for maximum certainty.

Planned asset credit:

> Great Lakes Height Map and Diffuse Map by Chris J Mitchell / Motion Forge Pictures. Source: https://www.motionforgepictures.com/sdm_downloads/great-lakes-height-map/. Provided under CC0 1.0 Universal: https://creativecommons.org/publicdomain/zero/1.0/.

Additional site-requested credit:

> Heightmaps Supplied by Motion Forge Pictures.

Reference implementations are architectural/pattern sources only. No third-party code is to be copied. Wicked Engine is MIT licensed; any translated pattern receives a path-level Pattern Index entry in `ATTRIBUTION.md`.

## 12. Primary implementation references

- Motion Forge Pictures, [Landscape Height Maps](https://www.motionforgepictures.com/height-maps/) and [Great Lakes Height Map](https://www.motionforgepictures.com/sdm_downloads/great-lakes-height-map/).
- Creative Commons, [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/).
- János Turánszki / Wicked Engine, local `wiOcean.*`, `wiFFTGenerator.*`, `ocean*.hlsl`, `underwaterCS.hlsl`, and [official repository](https://github.com/turanszkij/WickedEngine).
- János Turánszki, [*Underwater effect updates*](https://youtu.be/DZaaUPLmJIQ) — visual acceptance reference.
- Jerry Tessendorf, [*Simulating Ocean Water*](https://people.computing.clemson.edu/~jtessen/reports/papers_files/coursenotes2002.pdf).
- Mark Finch, [*Effective Water Simulation from Physical Models*](https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models).
- NVIDIA, [*Rendering Water Caustics*](https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-2-rendering-water-caustics).
- Nicolas Longchamps / Eidos Montréal, [*From Shore to Horizon*](https://media.gdcvault.com/gdc2017/Presentations/Longchamps_Nicolas_ShoreToHorizon_NOTES.pdf).
- Bartłomiej Wroński / Ubisoft, [*Assassin's Creed IV: Road to Next-Gen Graphics*](https://www.gdcvault.com/play/1020397/Assassin-s-Creed-IV-Black).
- fxguide, [*Assassin's Creed III: The tech behind (or beneath) the action*](https://www.fxguide.com/fxfeatured/assassins-creed-iii-the-tech-behind-or-beneath-the-action/).
- Epic Games, [*Single Layer Water*](https://dev.epicgames.com/documentation/unreal-engine/single-layer-water-shading-model-in-unreal-engine) and [Water Body Actors](https://dev.epicgames.com/documentation/en-us/unreal-engine/water-body-actors-in-unreal-engine).
- AMD GPUOpen, [FidelityFX Stochastic Screen Space Reflections](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/stochastic-screen-space-reflections/).
- Monzon et al. (2024), [*Real-Time Underwater Spectral Rendering*](https://diglib.eg.org/items/1316f247-e9a8-48fe-8754-f3276191e6b5), DOI `10.1111/cgf.15009`.
- Jeschke et al., [*Water Surface Wavelets*](https://research.nvidia.com/labs/prl/shallow-water-simulation/) — long-term interaction reference.

### Local reference file index

These files were read in the supplied repositories. They are pattern references, not copy sources.

**Wicked Engine — MIT, copyright Turánszki János**

- `New_Engines/WickedEngine-master/WickedEngine/wiOcean.h` and `wiOcean.cpp`: spectrum parameters, Phillips initialization, displacement resources, demand-gated readback, draw grids.
- `New_Engines/WickedEngine-master/WickedEngine/wiFFTGenerator.*`: 512² inverse-FFT compute scheduling.
- `New_Engines/WickedEngine-master/WickedEngine/shaders/oceanSimulatorCS.hlsl`: time evolution and height/horizontal-displacement packing.
- `.../oceanUpdateDisplacementMapCS.hlsl`: spatial displacement output.
- `.../oceanUpdateGradientFoldingCS.hlsl`: gradient and Jacobian folding/foam signal.
- `.../oceanSurfaceVS.hlsl`, `oceanSurfaceHF.hlsli`, and `oceanSurfacePS.hlsl`: projected grid, distance filtering, reflection/refraction, thickness, extinction, and foam.
- `.../underwaterCS.hlsl`: per-pixel waterline, Beer–Lambert attenuation, HG sun in-scattering, and HDR underwater composition.
- `.../wiScene.cpp` and `wiRenderPath3D.cpp`: ocean lifecycle, ripple injection, and pass placement.

Somnium should translate the FFT/optics/pass patterns but reject Wicked's global singleton ownership; a Somnium ECS entity owns each finite body's runtime state.

**Bevy Water — MIT OR Apache-2.0, Robert G. Jakabosky/bevy_water authors**

- `bevy-plugins/bevy_water-main/src/water.rs`, `wave.rs`, and `water/material.rs`.
- `bevy-plugins/bevy_water-main/assets/shaders/water_vertex.wgsl`, `water_fragment.wgsl`, and `water_functions.wgsl`.

Relevant patterns are matched CPU/WGSL wave queries, quality-dependent tiled geometry, crossfaded wave directions, and depth-driven Beer colour. Somnium already cites/adapts this reference for the current water pass; Phase IV extends that path.

**Jolt Physics — MIT, copyright Jorrit Rouwe**

- Workspace `example_repo/JoltPhysics-master/JoltPhysics-master/Samples/Tests/Water/WaterShapeTest.cpp`.
- Workspace `example_repo/JoltPhysics-master/JoltPhysics-master/Samples/Tests/Water/BoatTest.cpp`.

Relevant patterns are sensor/broadphase water volumes, shared sampled surface position/normal, deterministic contact order, buoyancy/drag impulses, and submerged-only propulsion.

**CDLOD — MIT, copyright Filip Strugar**

- `CDLOD-master/source/BasicCDLOD/Shaders/CDLODTerrain.vsh` and `CDLOD-master/README.md`.

Relevant future pattern: continuous distance morphing to a coarser regular grid. Somnium already has area-filtered import, coherent central-difference normals, and stitched indices, so morphing is a later LOD-quality option rather than a prerequisite for the Great Lakes import.

**Falco Engine — provenance unresolved**

- `New_Engines/FalcoEngine/.../Engine/Components/Water.*` and `StandardWater.shader` corroborate editor serialization, mirrored reflection, depth refraction, foam, and caustics.

No controlling top-level license was found in the supplied tree. Do not translate Falco source unless its provenance is resolved.

## 13. Definition of done

Phase IV is complete only when:

- the terrain triangle artifact is diagnosed and removed rather than hidden by new art;
- Great Lakes-derived runtime assets are reproducible and fully attributed;
- startup and **Create → Terrain** produce the same versioned terrain-water landscape;
- Terrain and Water are separate, persistent, editable ECS entities;
- water has a finite mask, real generated bed depth, stable surface motion, coherent optics, foam, and underwater rendering;
- above-water and underwater modes share one physical/authoring data model;
- all acceptance captures, timings, scene tests, shader validation, and workspace tests pass;
- `context.md`, `ATTRIBUTION.md`, and `assets/LICENSE.md` reflect the final implementation and provenance.

---

**AI disclosure:** This research plan was produced with AI-assisted source discovery, local source inspection, asset-format analysis, evidence synthesis, and drafting. Technical and licensing claims are linked to the cited first-party or primary sources; the Motion Forge license conflict is explicitly retained as a limitation rather than silently resolved.
