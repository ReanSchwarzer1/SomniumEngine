# Phase IV — Great Lakes Landscape and Black Flag Water

**Project:** Somnium Engine  
**Status:** IV-A through IV-J complete
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

IV-A through IV-J were implemented on 2026-08-11.

Live wgpu evidence is stored by phase under [`dev records/phase IV`](dev%20records/phase%20IV).
The IV-F/G/H release captures cover the default spectral surface, deep
underwater medium, and waterline transition; no PNG evidence is kept in the
repository root.

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

**Status: DONE — 2026-08-11**

Implemented direct FLOAT32 OpenEXR channel loading without an 8-bit conversion,
real-codec precision tests, continuous-normal analytic terrain tests, and shadow
debug modes for LOD, triangle edges, geometric normals, receiver-bias normals,
shadow factor, and contact shadows. The daytime triangle patches were isolated
to per-face receiver bias and fixed by using the interpolated terrain geometric
normal while retaining face normals for ordinary meshes.

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

**Status: DONE — 2026-08-11**

Added the deterministic `bake_great_lakes` importer and committed its 1025²
16-bit height, masked macro colour, 2048² lake mask, bathymetric depth, shoreline
SDF, and recipe products. Repeated runs produce identical hashes. The default
terrain now loads these derivatives, uses 105 m total relief, a 16.1 m water datum,
up to 12 m of synthetic bathymetry, and a 0.35 m minimum dry-ground clearance.

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

**Status: DONE — 2026-08-11**

Expanded `WaterComponent` into a stable, serializable handle with terrain
relationship, preset, kind, bounds, datum, maximum depth, enabled state, and
editable optics/wave fields. Renderer-owned `WaterBodyData` now loads the lake
mask/depth/SDF textures. The default scene and Create → Terrain spawn a separate
child `Water` entity; hierarchy, inspector editing, duplicate/delete,
composite undo/redo, scene serialization, and renderer-resource reconciliation
are covered by tests. IV-D replaced the temporary broad render mesh with an
explicit finite, mask-clipped surface.

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

**Status: DONE — 2026-08-11**

The renderer now builds a compact terrain-local 2 m lake grid containing only
coarse cells touched by the baked wet mask. The fragment pass samples the
full-resolution mask, depth, and shoreline SDF, so the coarse mesh cannot cut
out narrow inlets and dry pixels never receive water. `WaterBodyRegistry`
provides deterministic surface height, normal, depth, velocity, XZ coverage,
and point-containment queries from the same four-band Gerstner parameters used
by WGSL. Shore depth attenuates displacement and derivatives before both CPU
and GPU evaluation.

The water pass writes an `Rgba16Float` surface-data target and overwrites the
global `Rg16Float` velocity only where water is present. TAA consumes these
water motion vectors while retaining the established depth reprojection for
opaque pixels. Camera-distance and pixel-footprint filtering move unresolved
wave slope energy into roughness, eliminating distant cross-hatching in the
post-TAA validation capture.

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

**Status: DONE — 2026-08-11**

The finite pass now uses dielectric `F0 = 0.02037`, GGX sun/moon highlights,
prefiltered environment reflection, and bounded SSR with confidence/edge fade
and environment fallback. Refraction candidates are rejected when opaque depth
is missing or lies in front of the surface. Reconstructed path length drives
RGB Beer–Lambert extinction and approximate Henyey–Greenstein single
scattering; shore SDF supplies a depth-aware edge-foam term. Normal and ORM
textures carry a complete CPU-generated mip chain so minification cannot
reintroduce salt-and-pepper sparkle.

The ECS and scene format persist wavelengths, speed, steepness, absorption,
scattering, roughness, anisotropy, and SSR strength. The Water inspector exposes
the primary motion and reflection controls. Release-mode live wgpu captures at
day and `SOMNIUM_SUN_ELEVATION=-20` remained finite, mask-clipped, and stable;
`dev records/phase IV/IV-D-E/IV-D-E_day_post-TAA.png` and
`dev records/phase IV/IV-D-E/IV-D-E_night_post-TAA.png` record the post-TAA evidence.

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

**Status: DONE — 2026-08-11**

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

**Delivered.** An optional deterministic two-cascade GPU inverse FFT uses
256²/192 m and 512²/53 m grids. The compute chain evolves a wind spectrum,
performs bit-reversal and radix-2 ping-pong inverse transforms, and composes
RGBA16F displacement plus gradient/Jacobian history. ECS wind speed, spectral
blend, foam decay, and foam threshold drive the shared simulation. Crest foam
comes from horizontal-displacement folding, shore foam from the authored
SDF/depth, and the same signal darkens the wet-sand band. Incommensurate patch
lengths and retained distant normal variance avoid a single obvious repeat.
`SOMNIUM_WATER_SPECTRUM=0` preserves the deterministic Gerstner tier.

Release evidence:
[`IV-F-G-H_surface_day.png`](dev%20records/phase%20IV/IV-F-G-H/IV-F-G-H_surface_day.png).

### IV-G — Underwater medium and partial submersion

**Status: DONE — 2026-08-11**

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

**Delivered.** Renderer-side containment selects the finite active body and
tests the camera against the displaced CPU surface. A post-TAA HDR pass builds
a per-pixel near-plane submersion mask, reconstructs the submerged camera-to-
receiver ray segment, and applies RGB Beer–Lambert extinction, HG in-scattering,
sun/moon illumination, submerged fog, and bounded light-shaft modulation. The
two-sided surface orients its interface normal to the viewer and exposes the
underside/TIR transition. Portable caustics are restricted to submerged opaque
receivers and fade with receiver depth, path length, and turbidity. The WGSL is
original; Wicked's Shadertoy-cited Brown–Conrady and god-ray helpers were not
translated.

Release evidence:
[`IV-G_underwater_deep.png`](dev%20records/phase%20IV/IV-F-G-H/IV-G_underwater_deep.png)
and
[`IV-G_waterline_transition.png`](dev%20records/phase%20IV/IV-F-G-H/IV-G_waterline_transition.png).

### IV-H — One default landscape everywhere

**Status: DONE — 2026-08-11**

**Work**

- Route normal startup and **Create → Terrain** through `create_default_landscape`.
- Remove demo-owned water plane/texture setup.
- Make the default camera, water datum, terrain transform, material thresholds, and post-processing part of a versioned landscape preset.
- Make compound creation one undoable transaction while preserving two Outliner entities.
- Add a structural regression test comparing startup and Create-menu descriptors/entity graphs.

**Exit gate**

- Startup and UI-created landscapes have identical source recipe, terrain descriptor, water preset, masks, materials, and hierarchy.
- The old `WaterPlane` entity and hard-coded 20 m geometry no longer exist.

**Delivered.** `DefaultLandscapePreset` is a versioned recipe for terrain,
relief, material threshold, transforms, water datum, camera, and post process.
Both normal demo startup and **Create → Terrain** call
`create_default_landscape`; the editor wraps its returned Terrain and Water
snapshots in one undoable `CreateLandscapeCmd`. The finite water child remains
a separate Outliner/ECS entity. Structural, undo/redo, deletion, and scene
round-trip tests cover the graph and the serialized spectral/underwater values.
The legacy `WaterPlane` startup path and demo-owned water texture setup are gone.

### IV-I — Interaction tier (after the visual/volume foundation)

**Status: DONE — 2026-08-11**

The default landscape now spawns Opus Poly's 29,035-triangle Gislinge Viking
Boat as a first-class ECS entity with its original embedded materials and a
separate, low-frequency Jolt proxy hull. Eight distributed hull samples query the same
deterministic CPU water surface used by rendering and apply buoyancy, point
drag, righting torque, and submerged propulsion at a fixed 60 Hz. Vessel speed
and heading feed an original analytic Kelvin-angle wake and prop-wash foam path
in the water shader. Environment simulation runs in editor preview as well as
Play mode and requires no GPU readback.

The editor viewport toolbar now owns Play, Pause/Resume, and Stop controls.
One simulation clock gates Jolt steps, particle time, and water time. Editing
and Playing both advance it, Pause holds the exact state while rendering
continues, and Stop resets time, velocities, and the vessel pose before live
editor preview resumes. Rendering and physics therefore always sample the same
water time. A Play session, including its paused state, suppresses the grid,
transform and light gizmos, selection outline, and terrain/foliage authoring
cursors; Stop restores those editor-only overlays.

The shoreline readability fix is part of this milestone: the full 2048² source
contour replaces the old 1024² majority mask, its SDF zero contour is bilinearly
reconstructed and antialiased, SDF distance is evaluated in metres,
and a broken-up depth-aware breaker band blends into crest/wake foam. Three
rotated normal-map frequency bands with distance fade replace the former two
obvious high-frequency tiles. A two-cell raster guard ring plus a 1.5 m
under-bank coverage dilation keeps the water surface behind opaque terrain,
while terrain chunks whose height range crosses the water datum are pinned to
LOD 0. The full-resolution terrain, depth buffer, and fragment SDF therefore own
the visible coastline instead of coarse distance-LOD triangles. Scene-depth
contact foam bridges the final sub-pixel edge. This removes the remaining
square/triangular bites without altering the licensed source height field.

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

**Status: DONE — 2026-08-11**

The vessel is Opus Poly's CC BY 4.0 Gislinge Viking Boat.
`assets/models/gislinge_viking_boat/README.md` records the author, original
page, license, source hash, real-world dimensions, runtime scaling, and the
render-hierarchy/physics-hull boundary. `assets/LICENSE.md`,
`ATTRIBUTION.md`, and `context.md` carry the same provenance and implementation
record. No screenshots were added to the repository root; any later visual
acceptance captures belong under `dev records/phase IV/IV-I-J/`.

Validation covers formatting, workspace compilation/tests (including 209
renderer tests and all 9 shader-module tests), and targeted Clippy on the
touched crates. The post-TAA runtime capture
`dev records/phase IV/IV-I-J/IV-I-J_shoreline_lod_validation.png` records the
water-aware terrain LOD and contact-band result on the user's adapter.

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
- Opus Poly, [Gislinge Viking Boat](https://sketchfab.com/3d-models/gislinge-viking-boat-01098ad7973647a9b558f41d2ebc5193), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
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

## 14. Phase IV-K — Ocean Fidelity Pass (GodotOceanWaves Parity)

> [!WARNING]
> **STATUS: UNFIXED / IN-PROGRESS / UNFINISHED**
> 
> Phase IV-K (Ocean Fidelity Pass) is **NOT YET COMPLETE** and is marked as **UNFIXED**. While the mathematical foundation, 3-cascade patch sizing ($88\text{m}$, $57\text{m}$, $16\text{m}$), Jacobian whitecap folding, temporal foam accumulation, SSS/GGX lighting, and `sea_spray.png` asset attribution have been implemented, the visual output still requires further tuning and auditing to fully match the `GodotOceanWaves` reference.
> 
> **MANDATORY INSTRUCTIONS FOR THE NEXT AI MODEL / SESSION:**
> 1. Read and digest the **entire codebase** in `crates/` (`somnium_renderer`, `somnium_core`, `somnium_ui`, `somnium_asset`, etc.).
> 2. Read all documentation files completely, especially `READ THIS FIRST.md`, `dev records/phase_IV.md`, `context.md`, `ATTRIBUTION.md`, `implementation_plan.md`, and `walkthrough.md`.
> 3. Inspect reference repositories in `example_repo/` (specifically `GodotOceanWaves-main`, `WickedEngine`, `JoltPhysics`).
> 4. Review cited papers and video talks (Tessendorf *Simulating Ocean Water*, Horvath *Empirical Directional Wave Spectra*, GDC 2019 *Wakes, Explosions and Lighting: Interactive Water Simulation in Atlas*, Turánszki *Underwater effect updates*, SIGGRAPH '18 *Sea of Thieves Water*).
> 5. Perform a full code audit of all written code and proposed changes to check for any subtle errors or discrepancies, and iterate on the water material/spectrum WGSL shaders until visual parity with `GodotOceanWaves` is completely achieved before declaring IV-K finished.

### 14.1 Overview & Motivation
The ocean rendering pipeline in Somnium Engine underwent a major fidelity pass (Phase IV-K) aimed at closing the visual gap with Retr0's `GodotOceanWaves` repository and the GDC 2019 reference talk *"Wakes, Explosions and Lighting: Interactive Water Simulation in Atlas"*.

### 14.2 Mathematical Formulas & Spectral Physics

#### 1. TMA Spectral Synthesis & Dispersion
For each cascade $c \in \{0, 1, 2\}$, wave frequencies $\omega(k)$ and TMA spectrum energy $S(\omega)$ are evaluated:
$$\omega(k) = \sqrt{g \cdot k \cdot \tanh(k \cdot d)}$$
$$S(\omega) = \frac{\alpha g^2}{\omega^5} \exp\left(-1.25 \left(\frac{\omega_p}{\omega}\right)^4\right) \cdot 3.3^r \cdot \Phi_{Kitaigorodskii}(\omega, d)$$
where directional spreading $D(\omega, \theta)$ uses Hasselmann directional distribution combined with Longuet-Higgins normalization:
$$\Phi_{LH}(s) = \frac{1}{\sqrt{\pi}} \left(\frac{\sqrt{s}}{2} + \frac{1}{16\sqrt{s}}\right) \quad (s \ge 0.4)$$
Initial complex wave amplitudes $h_0(\mathbf{k})$ are sampled via Box-Muller Gaussian transformation:
$$h_0(\mathbf{k}) = \frac{1}{\sqrt{2}} (\xi_1 + i \xi_2) \sqrt{2 \cdot S(\omega) \cdot D(\omega, \theta) \cdot \frac{d\omega}{dk} \frac{\Delta k_x \Delta k_y}{k}}$$

#### 2. Spatial Derivatives & Jacobian Matrix
FFT compute output stores spatial displacements $(D_x, D_y, D_z)$. The horizontal derivative matrix is evaluated via finite differences:
$$J = \left(1 + \frac{\partial D_x}{\partial x}\right)\left(1 + \frac{\partial D_z}{\partial z}\right) - \left(\frac{\partial D_x}{\partial z}\right)\left(\frac{\partial D_z}{\partial x}\right)$$
When waves steepen and crests compress horizontally, $J$ drops below $1.0$.

#### 3. Additive Temporal Foam Accumulation
Fold amount $f$ is evaluated from the whitecap threshold $w_{cap} = 1.0 - \text{foam\_threshold}$:
$$f = \max(w_{cap} - J, 0.0)$$
Foam is accumulated additively with exponential decay per frame $\Delta t$:
$$F_t = \text{clamp}\left(F_{t-1} \cdot e^{-\gamma_{decay} \Delta t} + f \cdot \gamma_{grow} \Delta t, 0.0, 1.0\right)$$
where $\gamma_{grow} = \text{clamp}(\Delta t \cdot \text{foam\_amount} \cdot 35.0, 0.05, 2.5)$ and $\gamma_{decay} = \Delta t \cdot \max(0.5, 12.0 - \text{foam\_amount}) \cdot 1.15$.

#### 4. 3-Cascade Spectral Mapping
The 3 cascades use tile lengths matching GodotOceanWaves default configuration:
- **Cascade 0**: Tile length $L_0 = 88.0\text{ m}$, displacement scale $= 1.0$, normal scale $= 1.0$
- **Cascade 1**: Tile length $L_1 = 57.0\text{ m}$, displacement scale $= 0.75$, normal scale $= 1.0$
- **Cascade 2**: Tile length $L_2 = 16.0\text{ m}$, displacement scale $= 0.0$, normal scale $= 0.25$

Summed world displacement $\mathbf{D}_{world}$ and normal gradient $\mathbf{G}$:
$$\mathbf{D}_{world} = \mathbf{D}_0 \cdot 1.0 + \mathbf{D}_1 \cdot 0.75 + \mathbf{D}_2 \cdot 0.0$$
$$\mathbf{G}_{slope} = \frac{\mathbf{G}_0 \cdot 1.0 + \mathbf{G}_1 \cdot 1.0 + \mathbf{G}_2 \cdot 0.25}{1 + |\mathbf{G}_{x,z}|}$$
$$F_{accum} = F_0 + F_1 + F_2$$
$$F_{factor} = \text{smoothstep}(0.0, 1.0, F_{accum} \cdot 0.75) \cdot e^{-\text{dist} \cdot 0.0075}$$

#### 5. GDC 2019 / Godot Ocean Surface Lighting
- **Albedo Blend**: $\mathbf{A} = \text{mix}(\mathbf{C}_{water}, \mathbf{C}_{foam}, F_{factor})$
- **Roughness**: $R = (1.0 - \text{Fresnel}) \cdot F_{factor} + 0.4$
- **Fresnel**: 
$$\text{Fresnel} = \text{mix}\left(\frac{(1 - \mathbf{V} \cdot \mathbf{N})^{5 e^{-2.69 R}}}{1 + 22.7 R^{1.5}}, 1.0, 0.02\right)$$
- **Subsurface Scattering (SSS)**:
$$SSS_{height} = \max(0.0, h_{wave} + 2.5) \cdot \max(\mathbf{L} \cdot -\mathbf{V}, 0)^4 \cdot \left(0.5 - 0.5 (\mathbf{L} \cdot \mathbf{N})\right)^3$$
$$SSS_{near} = 0.5 (\mathbf{N} \cdot \mathbf{V})^2$$
$$\mathbf{C}_{diffuse} = \text{mix}\left(\frac{(SSS_{height} + SSS_{near}) \cdot \mathbf{C}_{sss}}{1 + \text{mask}} + 0.5(\mathbf{N} \cdot \mathbf{L}), \mathbf{C}_{foam}, F_{factor}\right) (1 - \text{Fresnel}) \mathbf{C}_{light}$$

### 14.3 Third-Party Provenance & Attribution
- **Source Repository**: `GodotOceanWaves` by 2Retr0 (https://github.com/2Retr0/GodotOceanWaves)
- **License**: MIT License / Creative Commons
- **Attributed Components**:
  1. `sea_spray.png` asset copied to `assets/ocean_pbr/sea_spray.png`
  2. TMA wave spectrum parameterization and 3-cascade patch scales (88m, 57m, 16m)
  3. Exponential additive foam feedback and Jacobian folding math
  4. GDC 2019 GGX/Smith ocean surface and SSS lighting formulation
- **Attribution File**: [`assets/ocean_pbr/README.txt`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/assets/ocean_pbr/README.txt)

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
