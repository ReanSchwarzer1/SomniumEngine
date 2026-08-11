# Phase XV — Appalachia

> *“Sixteen times the detail.”* — Todd Howard  
> Sixteen terrain materials. It had to be Appalachia.

> **Codename:** Appalachia, after the setting of Bethesda Game Studios' *Fallout 76*  
> **Status:** RESEARCH COMPLETE — IMPLEMENTATION NOT STARTED  
> **Plan date:** 2026-08-12  
> **Project:** Somnium Engine  
> **Target:** Rust 1.85, wgpu 29, winit 0.30

The codename is thematic only. No Bethesda code, artwork, textures, names, or other game assets will be used.

## 1. Executive decision

Phase XV expands Somnium's terrain from eight to sixteen photogrammetry-quality PBR materials while keeping the current editable heightfield, visibility-buffer renderer, four-channel packed material format, and hex-tiling system.

The implementation should use:

- sixteen globally available terrain materials;
- no more than four locally active materials per splat texel;
- four RGBA control/splat textures, retaining direct editable weights;
- GPU selection of the strongest four weights before expensive PBR sampling;
- two packed texture arrays per material: albedo + height and normal XY + roughness + AO;
- offline, semantics-aware mip generation and BC7 compression, with an uncompressed fallback;
- physically scaled material metadata rather than a single arbitrary UV scale;
- full-PBR biplanar projection on steep cliffs, including normals, height, roughness, and AO;
- the existing hex randomization and macro variation, tuned so the two do not compound into visible colour drift;
- a deterministic biome-rule preset shared by the default scene and **Create > Terrain**, with manual paint remaining authoritative.

This is an extension of the proven Phase 25 terrain path, not a replacement renderer. Runtime virtual texturing, geometry clipmaps, tessellation, and unlimited material stacks remain outside the initial scope unless profiling demonstrates that the bounded design cannot meet the acceptance budgets in section 11.

## 2. Goals

1. Increase the terrain palette from eight to sixteen production-quality, redistributable materials.
2. Cover visually distinct terrain classes: dry and wet sand, meadow and sparse grass, forest floor, soil and mud, gravel and talus, dry and mossy rock, vertical cliff, and snow.
3. Preserve old scenes exactly for layers 0–7 and make the sidecar migration automatic.
4. Improve close-range realism through physical scale, correct semantic mips, specular anti-aliasing, and consistent normal blending.
5. Remove stretched or incomplete cliff shading by projecting the entire material, not albedo alone.
6. Keep shader cost bounded even though the global palette doubles.
7. Make provenance, source URLs, source hashes, processing parameters, and licenses reproducible.
8. Give artists useful authoring and diagnostic tools without hiding the actual material budget.

## 3. Non-goals

- Replacing the Great Lakes heightfield or changing terrain geometry/LOD topology.
- Building a world-scale sparse virtual-texture system in this phase.
- Adding tessellation, mesh shaders, micropolygon displacement, or true geometry displacement.
- Sampling POM for every blended layer; POM remains dominant-layer-only.
- Adding foliage, rocks, decals, roads, or procedural object scatter. These are valuable later, but are separate systems.
- Importing Quixel/Megascans assets or any asset whose redistribution rights are not unambiguous for a public engine repository.
- Generating source materials with AI.
- Copying source code from any reference engine. Reference implementations inform an original Rust/WGSL design and must be cited in `ATTRIBUTION.md` during implementation.

## 4. Context and repository audit

### 4.1 Required project context

The following project records were reviewed before this plan was written:

- `context.md`
- `ATTRIBUTION.md`
- `dev records/phase_25m2_completion_report.md`
- `assets/LICENSE.md`
- the Phase 25 terrain implementation and shader files

The originally referenced root files `m2.md` and `m25.md` are not present in the current worktree. The available combined Phase 25M2 completion report was used instead; this absence must not be silently treated as successful discovery in later sessions.

### 4.2 Current Somnium baseline

Somnium already has a mature eight-layer terrain path:

| Layer | Current asset | Editor label | Compatibility rule |
|---:|---|---|---|
| 0 | `aerial_grass_rock` | Grass | Preserve index and interpretation |
| 1 | `forrest_ground_01` | Forest Floor | Preserve index and interpretation |
| 2 | `aerial_rocks_04` | Rock | Preserve as the legacy cliff layer unless explicitly migrated |
| 3 | `snow_02` | Snow | Preserve index and interpretation |
| 4 | `leafy_grass` | Meadow | Preserve index and interpretation |
| 5 | `brown_mud` | Mud | Preserve index and interpretation |
| 6 | `coast_sand_rocks_02` | Sand | Preserve index and interpretation |
| 7 | `gravel_floor` | Gravel | Preserve index and interpretation |

Current material storage is two RGBA8 array textures per layer:

- **Albedo/height:** RGB albedo, A height.
- **Surface:** normal X/Y, roughness, AO.

The control data is two RGBA splatmaps and `SplatTexel = [u8; 8]`. The sidecar format is version 2. The shader uses sparse weight gating, explicit derivatives, height-aware blending, perceptual albedo blending, a dominant-layer POM path, derived macro colour, and practical hex tiling. The committed source pack is 4K; 2K is the default runtime resolution and 4K is opt-in through `SOMNIUM_TERRAIN_RES=4096`.

The measured Phase 25D terrain shader cost fell from 0.973 ms to 0.883 ms in its reference view. The current landscape and eye-level material-map averages are approximately 11.44 and 12 taps respectively. Those measurements define the comparison baseline; they are not portable absolute guarantees for every GPU.

### 4.3 Identified gaps

- All CPU, GPU, editor, serialization, and ReSTIR GI paths assume exactly eight layers.
- A naïve increase to sixteen would double the existing worst-case material fetch count from 48 to 96.
- The steep-slope projection is albedo-only, uses implicit derivatives, assigns a fixed roughness, and does not project the cliff normal, height, or AO.
- Layer normals are linearly averaged and normalized, which loses detail as dissimilar normals blend.
- Current mip generation box-filters encoded channel bytes without respecting albedo colour space, normal length, height semantics, or normal-driven roughness variance.
- Physical real-world material size is not a first-class manifest property.
- Sixteen uncompressed 2K material pairs would consume about 682.7 MiB including full mip chains; 4K would be about 2.67 GiB.
- The same fixed eight-layer logic is duplicated in terrain lighting/GI code, creating drift risk.

## 5. Research method and conclusions

Research prioritized source code in the supplied reference repositories, official engine documentation, original papers, conference material, and first-party asset/license pages. Secondary tutorials were not used as architectural authority. SIGCHI proceedings were searched, but no directly applicable real-time terrain-material technique was found; the relevant evidence came from SIGGRAPH, JCGT, Eurographics, GDC, CVPR, and engine source/documentation.

### 5.1 Reference-engine findings

| Reference | Source finding | Somnium decision |
|---|---|---|
| O3DE Terrain | The Surface Material List exposes many materials globally but blends only the strongest local materials. Official documentation states that the top three weights are normalized; local source also uses a compact top-two ID/relative-blend detail texture and gathers/deduplicates neighbouring IDs. | Adopt “many globally, few locally,” but retain four direct editable weights rather than switching to indexed IDs. Evaluate strongest four initially and compare strongest three. |
| O3DE detail/macro materials | Separates large unique macro variation from tiled detail material and uses bounded material selection. | Keep Somnium's derived macro map and tiled PBR detail; do not add another clipmap until a measured need exists. |
| DICE/Frostbite | Procedural shader splatting combines a low-frequency unique colour source with tiled detail; terrain rules include slope and height, and dynamic flow avoids hidden-material cost. DICE also stresses that close terrain needs coherent scans and supporting surface detail. | Add deterministic biome suggestions and preserve early rejection of inactive layers. Do not pretend textures alone replace later scatter/decals. |
| Far Cry 5 | A virtual-texture solution collapses a very large material sample into a small runtime sample budget. | Record as a scalability alternative, not the default for Somnium's bounded editable terrain. |
| Ghost Recon Wildlands | Large-world biome diversity is supported with virtual texture and specialized authoring tools. | Borrow the clear biome/authoring separation, not the world-scale streaming architecture. |
| Unreal Landscape/RVT | Runtime Virtual Texturing caches complex landscape shading and lets landscape/other actors share surface data. | Defer RVT. Introduce a benchmark gate so it can be reconsidered if sixteen direct layers fail cost or residency targets. |
| Wicked Engine | Terrain uses material arrays and sparse/virtual atlas techniques for larger scopes. | Continue material arrays; avoid introducing an atlas/VT subsystem without a world-size requirement. |
| Fyrox | Terrain layers are individually masked and authored as a stack. | Preserve intuitive layer-based authoring while keeping Somnium's packed one-pass control textures. |
| Bevy biplanar reference | Demonstrates biplanar sampling with explicit gradients and the reduction from three projections to two, with known axis-switch discontinuities. | Use full-PBR biplanar projection as the default steep-slope path, retain triplanar as a debug/quality fallback, and test seam hysteresis. |
| Bethesda Game Studios / Fallout 4 | Bethesda's first-party graphics overview describes a physically based deferred renderer designed to make materials visually distinct, plus a material system that changes world surfaces when rain arrives. | Judge all sixteen materials by their light response—not albedo alone—and verify wet/dry weather states without erasing the identity of sand, soil, grass, and rock. |
| Bethesda Game Studios / GDC 2016 | Joel Burgess and Nathan Purkeypile describe reusable modular art kits and an iterative level-design workflow as the production foundation that allowed a comparatively small content team to build Fallout 4's enormous world. | Treat the sixteen-material palette and biome preset as a reusable **landscape kit**: versioned, previewable, composable, and repeatedly reviewed across the whole terrain rather than tuned as isolated textures. |

### 5.2 Research-to-design decisions

| Topic | Evidence | Decision |
|---|---|---|
| Anti-tiling | Mikkelsen's practical hex tiling preserves derivatives and limits randomised repetitions; Somnium already has this implementation. | Keep and extend the existing path. Do not layer texture bombing on top. |
| Histogram preservation | Burley's histogram-preserving blending addresses contrast/colour loss under randomized tiling. | Add only after an A/B evaluation shows measurable contrast loss; it requires preprocessing and is not automatically justified. |
| Normal composition | Reoriented Normal Mapping (RNM) preserves detail better than linear normal averaging. | Blend transition normals through weighted surface gradients, then apply shared microdetail to the result with RNM. |
| Specular shimmer | Toksvig filtering and LEAN mapping account for unresolved normal variance. | Generate per-mip roughness compensation offline using normal variance. Reject LEAN's additional storage for this phase. |
| Cliff mapping | Full triplanar displacement research maps colour, detail normals, and displacement consistently. | Project every PBR channel at cliffs. Use biplanar by default for cost; keep triplanar for verification/fallback. |
| Photogrammetry | DICE's photogrammetry workflow and MatSynth both highlight scale, material completeness, cleanup, and metadata. | Treat “photoscanned” as a quality/provenance requirement, not a marketing label. Validate seams, scale, lighting neutrality, and all PBR channels. |
| Compression | wgpu exposes BC texture compression only when the adapter supports `TEXTURE_COMPRESSION_BC`. | Produce BC7 runtime packs and request the feature conditionally. Keep a deterministic RGBA8 fallback and never keep both resident. |
| Virtual texturing | Unreal, Far Cry 5, Ghost Recon, O3DE, and Wicked demonstrate its value at large scale. | Defer until a profiler-backed gate is crossed. Somnium currently has a bounded 1 km terrain and a working direct-array path. |

### 5.3 The Bethesda/Appalachia landscape connection

The Appalachia codename now has a practical design link as well as the “sixteen times the detail” joke:

- Bethesda Game Studios' Fallout 4 graphics overview says its technology choices were selected jointly by the art and engineering teams against specific artistic and performance goals. Its PBR renderer was intended to make surfaces feel tactile and materially distinct, while rain could alter world-surface response. Phase XV follows that discipline by giving every terrain category a characteristic albedo, normal scale, roughness range, height response, and wet-state response, then testing those properties within a fixed GPU budget.
- In the GDC 2016 session **Fallout 4's Modular Level Design**, BGS presented reusable art kits and iteration as a way to create a large, varied open world efficiently. The direct analogue here is not modular architecture but a modular landscape kit: sixteen stable materials, one manifest, shared biome rules, consistent debug views, and repeated whole-world review.
- Bethesda's official Fallout 76 overview describes Appalachia as six visually distinct regions, including the Forest, Toxic Valley, and Ash Heap. Somnium should similarly require readable biome identities from combinations of a reusable palette rather than equating variety with one unique texture per biome.
- BGS senior environment artist Megan Sawyer describes a landscape team that reviews its work weekly and uses regionally meaningful flora—such as West Virginia's rhododendron—to ground Fallout 76. Foliage remains outside Phase XV, but the material manifest should retain biome and moisture tags so a later scatter system can extend the same environmental identity instead of inventing a disconnected one.

This does **not** mean copying Creation Engine technology or Fallout's art direction. It contributes production principles: distinct physical materials, weather-aware validation, reusable landscape building blocks, strong regional identity, and regular landscape-scale review.

## 6. Proposed sixteen-material palette

All eight current indices remain unchanged. The eight new candidates are Poly Haven assets published under CC0. Their final acceptance is contingent on an implementation-time visual and channel audit; any substitution must also be CC0, must preserve the intended terrain role, and must update the manifest and attribution record.

| Index | Asset identifier | Editor role | Approx. physical width | Source |
|---:|---|---|---:|---|
| 0 | `aerial_grass_rock` | Grass | Existing metadata | Existing licensed asset |
| 1 | `forrest_ground_01` | Forest Floor | Existing metadata | Existing licensed asset |
| 2 | `aerial_rocks_04` | Rock / legacy cliff | Existing metadata | Existing licensed asset |
| 3 | `snow_02` | Snow | Existing metadata | Existing licensed asset |
| 4 | `leafy_grass` | Meadow | Existing metadata | Existing licensed asset |
| 5 | `brown_mud` | Mud | Existing metadata | Existing licensed asset |
| 6 | `coast_sand_rocks_02` | Sand / pebbled coast | Existing metadata | Existing licensed asset |
| 7 | `gravel_floor` | Gravel | Existing metadata | Existing licensed asset |
| 8 | `aerial_sand` | Dry beach sand | 15 m | <https://polyhaven.com/a/aerial_sand> |
| 9 | `coast_sand_01` | Damp shoreline sand | 15 m | <https://polyhaven.com/a/coast_sand_01> |
| 10 | `dry_mud_field_001` | Dry earth / topsoil | 3 m | <https://polyhaven.com/a/dry_mud_field_001> |
| 11 | `terrain_red_01` | Red mineral soil | 2 m | <https://polyhaven.com/a/terrain_red_01> |
| 12 | `sparse_grass` | Sparse grass and exposed soil | 2 m | <https://polyhaven.com/a/sparse_grass> |
| 13 | `mossy_rock` | Wet/mossy mountain rock | 3 m | <https://polyhaven.com/a/mossy_rock> |
| 14 | `rock_face_03` | Rugged vertical cliff | 2.7 m | <https://polyhaven.com/a/rock_face_03> |
| 15 | `dry_riverbed_rock` | Talus / river stone | 2 m | <https://polyhaven.com/a/dry_riverbed_rock> |

Poly Haven's official license places its assets under CC0, permitting commercial use, modification, and redistribution without required attribution. Somnium should still provide voluntary credit, direct asset-page URLs, the CC0 link, an access date, and hashes for every downloaded source file. The implementation must fetch through the documented API or asset URLs with an identifying User-Agent and must not obscure provenance.

Fallback sourcing, if a candidate fails the quality review, may use another Poly Haven asset or an ambientCG CC0 material. A source's general reputation is not sufficient: the exact asset page and license must be captured.

## 7. Target architecture

### 7.1 Reproducible material manifest

Add a versioned `assets/terrain/materials.json` as the source of truth. Each entry should contain:

- stable numeric layer index and stable string ID;
- editor display name and terrain-role tags;
- source page, author/organization, license identifier and license URL;
- source-file URLs, expected byte sizes, and SHA-256 hashes;
- source colour-space and normal convention (OpenGL or DirectX);
- measured physical width/height in metres;
- per-layer UV scale multiplier and optional rotation offset;
- channel mapping and processing parameters;
- height normalization range and neutral-height convention;
- macro tint limits, microdetail response, wetness affinity, and cliff suitability;
- generated output hashes for 2K and 4K packs.

The fetch tool must fail closed on hash mismatch, missing channels, changed resolution, or an unknown license. Generated packs should be reproducible from the manifest and cached source files.

### 7.2 Control representation: four direct splatmaps

Use four RGBA8 control textures for sixteen weights:

```text
Splat 0: layers  0–3
Splat 1: layers  4–7
Splat 2: layers  8–11
Splat 3: layers 12–15
```

This deliberately retains direct weights rather than adopting an indexed ID/weight texture. Direct splats preserve hardware bilinear filtering, current brush behaviour, undo semantics, and simple migration. An indexed representation requires neighbour gathers plus ID deduplication to filter correctly, as demonstrated by O3DE's implementation, and would make authoring more fragile.

The editor invariant is **at most four non-zero stored channels per texel**. When a brush or biome rule adds a fifth channel, the smallest channels should decay deterministically, then all surviving channels should be renormalized to 255 with a stable remainder rule. Filtering can expose more than four candidates at pixel boundaries; the shader selects the strongest four and renormalizes them before material sampling.

Sidecar version 3 migration:

- copy version 2 layers 0–7 byte-for-byte;
- initialize layers 8–15 to zero;
- preserve the exact normalized rendering of old scenes;
- store material-manifest version and hash;
- retain a one-way v2-to-v3 migration test fixture.

### 7.3 Bounded shader evaluation

The fragment path should:

1. sample four control textures;
2. assemble and normalize sixteen scalar weights;
3. select the strongest four with a deterministic compare/swap network;
4. reject negligible weights before texture work;
5. evaluate only the surviving material entries;
6. height-blend and renormalize the selected set;
7. evaluate dominant-layer POM only when its existing distance/angle gates pass;
8. blend normals through a surface-gradient representation;
9. apply optional shared microdetail with RNM;
10. write the same material interpretation to the terrain visibility shading and ReSTIR GI paths through shared WGSL helpers.

With two packed maps, three hex samples, and four selected layers, the base worst case is 24 material taps rather than the current 48. Full-PBR biplanar projection on a steep dominant cliff may bring the bounded worst case to 36 material taps. Control-map, macro-map, and gated POM fetches are reported separately so profiling cannot hide them.

The implementation must A/B strongest-three and strongest-four selection against a full sixteen-layer offline reference. Three may ship only if its transition error and visible junction quality meet the same thresholds.

### 7.4 Material arrays, compression, and mips

Continue using the two packed material arrays. Build two runtime variants:

- **Preferred:** BC7 for both arrays when the adapter exposes wgpu `TEXTURE_COMPRESSION_BC`.
- **Fallback:** RGBA8 arrays using the current universal upload path.

The device feature is conditional; requesting an unsupported feature is an error. Only one variant may be resident.

Approximate full-mip residency for sixteen layers:

| Runtime pack | 2K | 4K |
|---|---:|---:|
| Two RGBA8 arrays | 682.7 MiB | 2.67 GiB |
| Two BC7 arrays | 170.7 MiB | 682.7 MiB |
| Four RGBA8 2K control maps | 21.3 MiB | 85.3 MiB |

The default remains 2K. The 4K mode remains an explicit high-quality option.

Offline mips must be channel-aware:

- decode albedo to linear, filter, then encode to sRGB;
- average tangent-space normals, renormalize, and repack XY;
- preserve linear height and AO semantics;
- increase per-mip roughness using the lost normal-vector length/variance (Toksvig-style specular anti-aliasing);
- perform deterministic edge wrapping for tileable sources;
- validate alpha and scalar ranges after compression.

The packer should output a machine-readable report with per-layer range statistics, seam error, normal-length error, compression PSNR/SSIM for colour, and scalar-channel maximum error. BC7 height quality must be inspected explicitly because height lives in alpha and influences blending/POM.

### 7.5 Full-PBR cliff projection

Replace the current albedo-only cliff projection with a shared projected-material function that returns:

- albedo;
- tangent/world-space normal contribution;
- roughness;
- AO;
- height.

Default to biplanar projection with explicit gradients to control cost and avoid derivative errors inside divergent branches. Blend projection axes continuously and add hysteresis/smoothing around the axis switch. A triplanar mode should remain available as a debug/reference quality path. Height blending and projected normal orientation must use the same coordinates, or cliff material boundaries will swim.

Slope transitions should be broad enough to avoid a visible contour around the whole landscape. Layer 14 is the preferred dedicated cliff face, while layer 2 retains legacy compatibility. Artists can override cliff assignment through painted weights.

### 7.6 Multi-scale appearance

Each material should contribute at three distinct scales:

- **Macro:** the existing derived unique colour map and conservative biome tint; no high-frequency normals.
- **Meso:** physically scaled photoscanned PBR material with hex randomization.
- **Micro:** a subtle shared or per-category detail normal applied through RNM and faded by distance/roughness.

Physical-size metadata determines the base tiling rate. Per-layer artistic scale is a bounded multiplier, not a replacement for metres. Random rotation/offset must preserve normal orientation. Macro colour variance should be energy-limited and evaluated alongside hex tiling so the combined system does not create synthetic blotches.

Histogram-preserving hex blending is an optional quality experiment. It should be enabled only if objective image statistics and side-by-side captures show that the current randomization visibly washes out contrast.

### 7.7 Biome rules and manual authoring

The default terrain preset should calculate material suggestions from stable, inspectable inputs:

- normalized elevation and world-space height;
- slope and curvature;
- distance above/below the water level;
- a low-frequency moisture field;
- a low-frequency temperature/exposure field;
- deterministic seeded noise only for boundary breakup.

Example intent:

- wet sand close to the waterline, transitioning to dry sand above it;
- mud and sparse grass in damp low slopes;
- meadow/grass on moderate, well-lit slopes;
- forest floor in sheltered/moist regions;
- gravel, talus, and rock as slope/curvature rises;
- rock-face projection on cliffs;
- snow at high elevation, modulated by slope and exposure.

Procedural results must be bakeable into the same four splatmaps. Manual paint is an additive/override authoring layer and must survive rule regeneration through an explicit “rebuild base / preserve overrides” operation. The default scene and **Create > Terrain** must instantiate the same versioned preset rather than duplicating constants.

## 8. Implementation phases

All subphases are **PLANNED**. Completing a subphase requires its acceptance evidence and documentation update, not only compiling code.

### XV-A — Baseline and provenance gate

**Work**

- Freeze reference scenes, camera transforms, adapter details, shader timings, tap counts, and memory measurements.
- Define a landscape-kit review matrix covering each intended biome identity in dry, wet, day, and night conditions.
- Add the sixteen-entry manifest schema and fill existing-layer provenance gaps.
- Validate the eight proposed new asset pages, channel sets, real-world sizes, licenses, and source hashes.
- Record any evidence images under `dev records/phase XV/evidence/` using `phase_XV-A_<purpose>.png` names.

**Exit criteria**

- Reproducible baseline report exists.
- Every candidate has an exact first-party source/license record.
- No new texture binary has entered the repository without its manifest entry.

### XV-B — Deterministic asset pipeline

**Work**

- Make the fetcher manifest-driven with hashes, identifying User-Agent, retries, and fail-closed validation.
- Extend the packer to sixteen layers and physical-scale metadata.
- Implement semantic mip generation, normal renormalization, and Toksvig-style roughness compensation.
- Emit 2K/4K RGBA8 and BC7 variants plus a validation report.

**Exit criteria**

- Two clean builds from the same inputs produce byte-identical outputs.
- Seam, channel-range, normal, and compression checks pass.
- The output report accounts for every input and transformation.

### XV-C — Sixteen-layer data model and migration

**Work**

- Expand CPU splat storage, painting, blending, undo, editor commands, save/load, and tests to sixteen layers.
- Add four-channel-group helpers so loops are not copied four times.
- Implement sidecar v3 and exact v2 migration.
- Enforce four stored non-zero channels per texel with deterministic normalization.

**Exit criteria**

- All sixteen layers can be painted, undone, serialized, reloaded, and inspected.
- Golden v2 scenes render identically after migration for layers 0–7.
- Quantized weights always total 255 where terrain is valid.

### XV-D — GPU layout and sparse evaluation

**Work**

- Add two more splat bindings and sixteen material metadata entries using WGSL-safe `vec4` packing.
- Implement deterministic strongest-four selection before expensive sampling.
- Consolidate terrain and ReSTIR GI material evaluation into shared helpers.
- Add debug modes for raw weights, selected indices, discarded weight, and tap count.

**Exit criteria**

- Sixteen layers are visible and correct in all shading paths.
- No path accidentally evaluates all sixteen PBR materials.
- Base hex worst case is at most 24 material-map taps, reported separately from control/macro/POM.

### XV-E — Compression and specular stability

**Work**

- Detect and request BC support conditionally in wgpu 29.
- Load BC7 packs when supported and RGBA8 otherwise, never both.
- Validate semantic mips under minification, glancing light, day/night, and camera motion.
- Tune roughness compensation to reduce distant sparkle without flattening close detail.

**Exit criteria**

- Default compressed material residency is at most 200 MiB at 2K.
- Uncompressed fallback is at most 700 MiB at 2K.
- No visible mip seams, hue shifts, dark normal mips, or new distant specular shimmer.

### XV-F — Full-PBR mountain and cliff materials

**Work**

- Implement explicit-gradient biplanar projection for all packed channels.
- Correct projected normal orientation and blend it through surface gradients.
- Add triplanar reference/debug mode and axis-switch diagnostics.
- Tune cliff slope masks and dedicated rock-face selection.

**Exit criteria**

- No albedo stretching or fixed-roughness cliff patches.
- Normal, roughness, AO, height, and albedo remain spatially aligned.
- Steep-path worst case is at most 36 material-map taps.
- Axis transitions are not visible in the cliff test corpus.

### XV-G — Biome preset and shared terrain creation

**Work**

- Implement deterministic elevation/slope/curvature/water/moisture/exposure rules.
- Bake results into the four direct splatmaps while respecting local sparsity.
- Add paint overrides and an explicit rebuild policy.
- Make the default scene and **Create > Terrain** consume one versioned preset.

**Exit criteria**

- Same seed, heightmap, water level, and preset produce identical splat hashes.
- Waterline, beach, grassland, soil, mountain, cliff, talus, and snow regions read coherently.
- Manual overrides survive a base-rule rebuild when requested.

### XV-H — Macro, meso, and micro fidelity

**Work**

- Apply per-material physical scale and bounded artistic multipliers.
- Calibrate dry and wet responses so each material remains distinct under weather-driven roughness and colour changes.
- Integrate shared/category microdetail through RNM with distance fade.
- Retune macro variation and hex randomization as one system.
- A/B histogram-preserving blending; ship it only if evidence justifies its preprocessing and runtime cost.

**Exit criteria**

- No obvious grid repetition in landscape, eye-level, and moving-camera captures.
- Scanned features have plausible real-world scale.
- Normal detail remains present through transitions without over-sharpening.
- Macro colour does not wash out or exaggerate the source material.

### XV-I — Editor experience and diagnostics

**Work**

- Expand the native wgpu material palette to sixteen named thumbnails.
- Show physical scale, source/license, memory state, and material role in the inspector.
- Add solo-layer, weight heatmap, selected-four, cliff projection, mip, and residency debug views.
- Surface unsupported compression fallback and manifest mismatch clearly in the output log.

**Exit criteria**

- All sixteen materials are discoverable and paintable without memorizing indices.
- The artist can identify why a material was selected and what was discarded.
- Debug UI remains native wgpu and does not introduce an opaque WebView background.

### XV-J — Verification, attribution, and handoff

**Work**

- Run formatting, build, unit/integration tests, shader validation, migration fixtures, and performance captures.
- Test day/night, wet/dry shoreline, distant landscape, eye-level, extreme cliff, four-way junction, and old-scene cases.
- Update `context.md`, `ATTRIBUTION.md`, `assets/LICENSE.md`, and this file with actual results and Pattern Index entries.
- Store evidence only under `dev records/phase XV/evidence/` with phase-specific names.

**Exit criteria**

- Every section 11 acceptance criterion passes or has an explicit, approved exception.
- Attribution covers assets, papers, engine patterns, modifications, and access dates.
- This document changes from planned milestones to an evidence-backed completion record.

## 9. Expected implementation touch points

This list is planning guidance, not permission to perform unrelated refactors.

| Area | Expected files |
|---|---|
| Asset provenance | `assets/terrain/materials.json`, `assets/LICENSE.md`, `ATTRIBUTION.md` |
| Fetch/pack tools | `tools/fetch_terrain_textures.sh`, `crates/somnium_asset/examples/pack_terrain.rs` |
| Terrain storage/upload | `crates/somnium_renderer/src/terrain/textures.rs`, `terrain/mod.rs`, `terrain/blend.rs`, `terrain/brush.rs` |
| Terrain shading | `crates/somnium_renderer/src/shaders/terrain_material.wgsl`, `restir_gi.wgsl`, shared terrain-material WGSL helper if introduced |
| Editor commands/UI | `crates/somnium_core/src/editor_commands.rs`, `app.rs`, existing native wgpu palette/inspector code |
| Tests/docs | renderer/asset tests, `context.md`, `ATTRIBUTION.md`, `assets/LICENSE.md`, this plan |

Before editing, the implementing session must re-open the current files and check for changes made after this plan date. File names and layouts are not contractual APIs.

## 10. Performance and quality budgets

### 10.1 Sampling and frame cost

- Maximum four expensive material evaluations per pixel.
- Base hex path: at most 24 material-map taps.
- Steep biplanar path: at most 36 material-map taps.
- Landscape average: at most 12 material-map taps.
- Eye-level average: at most 18 material-map taps.
- Median terrain shader target: at most 1.10 ms in the exact Phase 25 reference adapter, resolution, and camera corpus.
- No more than a 20–25% median regression from the captured pre-Phase-XV baseline without an approved image-quality justification.
- Report control, macro, projected-material, and POM taps separately.

### 10.2 Memory and disk

- Preferred 2K BC7 material arrays: at most 200 MiB resident.
- RGBA8 fallback 2K material arrays: at most 700 MiB resident.
- Never hold preferred and fallback packs simultaneously.
- Four 2K RGBA8 control maps: approximately 21.3 MiB including mips.
- 4K material mode stays opt-in and must log its projected residency before allocation.

### 10.3 Sparse-selection accuracy

Create an offline sixteen-layer reference that evaluates every non-zero layer, then compare strongest-three and strongest-four outputs over a corpus containing two-, three-, four-, and filtered five-plus-way junctions.

- Albedo difference: median CIEDE2000 < 1.0, 95th percentile < 3.0.
- Discarded normalized weight: 95th percentile < 0.05 in shipped/default splat data.
- Normal angular difference: median < 1 degree, 95th percentile < 4 degrees.
- Roughness absolute error: median < 0.015, 95th percentile < 0.05.
- No temporal index popping while the camera moves across a filtered junction.

### 10.4 Visual acceptance corpus

Capture consistent before/after frames and short camera paths for:

- default overview;
- eye-level grass/soil transition;
- dry-to-wet beach and waterline;
- mud/sparse-grass lowland;
- gravel/talus/rock mountain transition;
- vertical cliff under glancing day light;
- cliff at night and in wet conditions;
- snowline;
- long-distance minification and camera motion;
- two-, three-, and four-material junctions;
- migrated version 2 scene.

Evidence images must not be placed in the repository root. Use `dev records/phase XV/evidence/phase_XV-<subphase>_<purpose>.png`.

## 11. Phase acceptance criteria

Phase XV is complete only when all of the following are true:

1. Sixteen stable materials are listed, paintable, serialized, and rendered.
2. Every shipped material has exact provenance, license, access date, source hash, and physical-size metadata.
3. Version 2 terrain data migrates automatically and layers 0–7 preserve their appearance.
4. The default scene and **Create > Terrain** use the same versioned sixteen-layer preset.
5. Stored splat texels contain at most four non-zero channels and normalize deterministically.
6. Shader material cost is bounded by strongest-four selection before PBR sampling.
7. Full-PBR cliff projection eliminates stretched albedo and fixed roughness, with aligned normals/height/AO.
8. Offline mips are colour-, normal-, height-, AO-, and roughness-aware.
9. BC7 is used only when supported; the RGBA8 fallback renders equivalently within the defined compression tolerances.
10. Default 2K residency and shader timing meet section 10 budgets.
11. No obvious grid tiling, axis seam, transition pop, mip seam, or distant specular shimmer remains in the acceptance corpus.
12. Terrain and ReSTIR GI use the same material indexing and interpretation.
13. Native editor UI exposes all materials and the required diagnostic modes.
14. `cargo fmt --check`, relevant `cargo check`/tests, asset validation, sidecar migration tests, and WGSL/Naga validation pass.
15. Living documentation and attribution are updated with actual results rather than planned claims.

## 12. Risks and mitigations

| Risk | Consequence | Mitigation |
|---|---|---|
| Fixed eight-layer assumptions survive in a secondary path | GI or debug views disagree with the main renderer | Centralize constants/metadata and share WGSL evaluation helpers; search all shader and CPU paths before declaring XV-D complete. |
| WGSL storage alignment mistakes | Corrupt material metadata on some adapters | Store scalar groups in `vec4`-aligned records and add byte-layout assertions; follow wgpu 29 layout rules. |
| Direct control-map growth | More memory and bandwidth | Four control maps add little relative to material arrays; sample once and select before PBR work. Profile them separately. |
| Strongest-four filtering changes at bilinear boundaries | Colour/normal popping | Stable compare ordering, minimum-weight hysteresis where needed, transition corpus, moving-camera tests, and direct full-reference comparison. |
| Four-channel authoring limit removes a desired fifth material | Lost subtle blend | Show discarded weight in editor; deterministic decay; permit artist to choose the survivors. Revisit indexed sparse control only with evidence. |
| BC7 damages height/AO | Blend/POM changes or halos | Per-channel error reports, cliff/transition A/B tests, and per-map fallback if an encoder mode cannot meet thresholds. |
| Normal compression/mips shimmer | Sparkling or flattened mountains | Renormalized normal mips plus normal-variance roughness compensation and glancing-motion tests. |
| Biplanar axis transition appears | Diagonal seam on cliffs | Smooth weights/hysteresis, explicit gradients, a triplanar reference mode, and an adversarial rotated-cliff fixture. |
| Physical scale metadata is wrong | Materials look miniature or gigantic | Cross-check first-party scale, measure repeating features, expose bounded per-layer correction, record the correction in manifest. |
| Macro and hex variation compound | Muddy or artificial colours | Tune jointly with measured luminance/chroma limits; histogram-preserving option only after A/B evidence. |
| Asset/API/license changes | Non-reproducible or legally unclear source | Exact URLs, hashes, cached metadata, access dates, CC0 record, and fail-closed fetch. Never silently replace an asset. |
| Disk/VRAM doubles | Slow startup or allocation failure | BC7 preferred pack, default 2K, projected-memory log, conditional 4K, one resident variant. |
| Biome rules overwrite manual work | Artist data loss | Separate baked base and manual override operations, explicit rebuild confirmation, undo and serialization tests. |
| Texture realism exceeds surrounding scene fidelity | Terrain feels disconnected from vegetation/water/objects | Keep calibrated colour/scale; record scatter/decals as future work rather than hiding the mismatch with aggressive shading. |

## 13. Deferred and rejected alternatives

### 13.1 Runtime virtual texturing / detail clipmap

**Deferred.** It is proven in Unreal, Far Cry, Ghost Recon, O3DE, and Wicked, but it adds page management, cache invalidation, editor feedback, residency debugging, and more complex asset builds. Reconsider only if the direct-array implementation misses the frame or memory budgets after XV-D/XV-E, or if the engine expands beyond its bounded terrain scope.

### 13.2 Indexed material IDs plus compact weights

**Deferred.** This can reduce control data and is used effectively by O3DE-like selection systems, but correct bilinear filtering requires neighbouring ID gathers, deduplication, and reweighting. Four direct RGBA splats are simpler, preserve authoring semantics, and are small relative to material arrays.

### 13.3 LEAN mapping

**Rejected for Phase XV.** It improves normal/specular filtering but requires additional moments/storage. Toksvig-style offline roughness compensation is a better first step for the existing two-map budget.

### 13.4 Full multilayer POM

**Rejected.** It multiplies divergent sampling cost and offers poor value at transition pixels. Keep the current dominant-layer distance/angle-gated approach.

### 13.5 Tessellation or true displacement

**Rejected.** It changes terrain geometry, LOD, collision, and shadow behaviour and is not needed to achieve the material goals. Height remains for blend/POM cues.

### 13.6 Texture bombing on top of hex tiling

**Rejected initially.** It duplicates anti-repetition work and can raise sampling and derivative complexity. Preserve the already integrated JCGT hex path.

### 13.7 Content-aware Mix-Max transitions

**Research reserve.** Eurographics 2024 shows promising content-aware texture transitions. It should be tested only if current height-aware blending fails on the expanded material set; it is not required up front.

### 13.8 Mesh scatter, decals, and foliage

**Future phase.** DICE correctly notes that terrain materials alone do not create a natural close-range environment. Stones, debris, roots, grass, and biome scatter are an important follow-up, but including them here would hide whether the sixteen-layer material system itself is correct and performant.

## 14. Verification commands for the implementation session

Exact package names should be confirmed against the then-current workspace. The expected verification family is:

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo run -p somnium_asset --example pack_terrain -- --validate-only
```

In addition, run the repository's WGSL/Naga validation tests, sidecar v2-to-v3 golden fixtures, deterministic pack/hash tests, sparse-selection offline comparison, adapter capability tests, and the GPU timing/tap-count capture suite. No completion claim may rely on a single screenshot.

## 15. Sources and reference index

All web sources were accessed on 2026-08-12 unless otherwise noted.

### 15.1 Asset quality, datasets, and licensing

- Poly Haven, **License — CC0**: <https://polyhaven.com/license>
- Poly Haven, **Contribute / asset standards**: <https://polyhaven.com/contribute>
- Poly Haven, **API information**: <https://polyhaven.com/el/our-api>
- Creative Commons, **CC0 1.0 Universal**: <https://creativecommons.org/publicdomain/zero/1.0/>
- Vecchio et al., **MatSynth: A Modern PBR Materials Dataset**, CVPR 2024: <https://openaccess.thecvf.com/content/CVPR2024/html/Vecchio_MatSynth_A_Modern_PBR_Materials_Dataset_CVPR_2024_paper.html>
- Electronic Arts/DICE, **Photogrammetry and Star Wars Battlefront**: <https://www.ea.com/news/photogrammetry-and-star-wars-battlefront>
- DICE, **Photogrammetry and Star Wars Battlefront**, GDC 2016 slides: <https://media.gdcvault.com/gdc2016/Presentations/Brown_Kenneth_Hamilton_Andrew_PhotogrammetryStarWars.pdf>

### 15.2 Terrain rendering and authoring

- Bethesda Game Studios, **The Graphics Technology of Fallout 4**: <https://bethesda.net/tr-TR/news/the-graphics-technology-of-fallout-4>
- Burgess and Purkeypile, Bethesda Game Studios, **Fallout 4's Modular Level Design**, GDC 2016 session: <https://www.gdcvault.com/play/1022930/-Fallout-4-s-Modular>
- Burgess and Purkeypile, Bethesda Game Studios, **Fallout 4's Modular Level Design**, GDC 2016 slides: <https://media.gdcvault.com/gdc2016/Presentations/Burgess_Joel_Modular%20Level%20Design.pdf>
- Bethesda Game Studios, **What is Fallout 76?** (six-region Appalachia overview): <https://fallout.bethesda.net/en-EU/news/what-is-fallout-76>
- Bethesda Game Studios, **Meet Megan Sawyer — Senior Environment Artist**: <https://bethesda.net/tr-TR/news/meet-megan-sawyer-senior-environment-artist-at-bethesda-game-studios>
- Andersson, **Terrain Rendering in Frostbite Using Procedural Shader Splatting**, SIGGRAPH 2007: <https://advances.realtimerendering.com/s2007/Andersson-TerrainRendering%28Siggraph07%29-CourseNotes.pdf>
- Mikkelsen, **Practical Real-Time Hex-Tiling**, JCGT 2022: <https://jcgt.org/published/0011/03/05/>
- Burley, **On Histogram-Preserving Blending for Randomized Texture Tiling**, JCGT 2019: <https://jcgt.org/published/0008/04/02/>
- O3DE, **Terrain Surface Materials List**: <https://www.docs.o3de.org/docs/user-guide/components/reference/terrain/surface-material-list/>
- O3DE, **Terrain Detail Material**: <https://www.docs.o3de.org/docs/user-guide/components/reference/terrain/terrain-detail-material/>
- O3DE, **Terrain Macro Material**: <https://docs.o3de.org/docs/user-guide/components/reference/terrain/terrain-macro-material/>
- O3DE, **Texture Terrain with Macro and Detail Materials**: <https://docs.o3de.org/docs/learning-guide/tutorials/environments/create-terrain-from-images/texture-terrain/>
- Ubisoft, **Terrain Rendering in Far Cry 5**, GDC 2018: <https://www.gdcvault.com/play/1025261/Terrain-Rendering-in-Far-Cry>
- Ubisoft, **Terrain Rendering in Far Cry 5**, slides: <https://media.gdcvault.com/gdc2018/presentations/TerrainRenderingFarCry5.pdf>
- Ubisoft, **Ghost Recon Wildlands Terrain Technology and Tools**, GDC 2017: <https://www.gdcvault.com/play/1024029/-Ghost-Recon-Wildlands-Terrain>
- Ubisoft, **Ghost Recon Wildlands Terrain Technology and Tools**, slides: <https://media.gdcvault.com/gdc2017/Presentations/WERLE_MARTINEZ_GRWterrainTechnologyTools.pdf>
- Epic Games, **Runtime Virtual Texturing Quick Start**: <https://dev.epicgames.com/documentation/unreal-engine/runtimevirtual-texturing-quick-start-in-unreal-engine>
- NVIDIA, **Terrain Rendering Using GPU-Based Geometry Clipmaps**, GPU Gems 2: <https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry>
- NVIDIA, **Texture Bombing**, GPU Gems: <https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-20-texture-bombing>

### 15.3 Material filtering and projection

- Hill, **Blending in Detail — Reoriented Normal Mapping**: <https://blog.selfshadow.com/publications/blending-in-detail/>
- Toksvig, **Mipmapping Normal Maps**, Journal of Graphics Tools 2005: <https://www.tandfonline.com/doi/abs/10.1080/2151237X.2005.10129203>
- Olano and Baker, **LEAN Mapping**, I3D 2010: <https://userpages.cs.umbc.edu/olano/papers/lean/>
- **Triplanar Displacement Mapping for Terrain**, Eurographics 2020: <https://diglib.eg.org/server/api/core/bitstreams/b3af0317-e2d6-4e3a-8076-b415516eee87/content>
- **Mix-Max: Content-Aware Real-Time Texture Transitions**, Eurographics 2024: <https://diglib.eg.org/items/50375852-f98b-4f60-ae25-4ae06ad038d1>

### 15.4 API and compression

- wgpu 29, **Features**: <https://docs.rs/wgpu/29.0.0/wgpu/struct.Features.html>
- WebGPU, **GPUFeatureName**: <https://gpuweb.github.io/types/types/GPUFeatureName.html>

### 15.5 Local reference source inspected

- `C:\Users\adhir\Downloads\GE\example_repo\o3de-development\o3de-development\Gems\Terrain\Assets\Shaders\Terrain\TerrainDetailHelpers.azsli`
- `C:\Users\adhir\Downloads\GE\example_repo\o3de-development\o3de-development\Gems\Terrain\Code\Source\TerrainRenderer\Components\DetailMaterial\TerrainDetailMaterialManager.cpp`
- `C:\Users\adhir\Downloads\GE\example_repo\o3de-development\o3de-development\Code\Framework\AzFramework\AzFramework\SurfaceData\SurfaceData.h`
- `C:\Users\adhir\Downloads\GE\example_repo\bevy-plugins\bevy_triplanar_splatting-main\src\shaders\biplanar.wgsl`
- `C:\Users\adhir\Downloads\GE\example_repo\New_Engines\WickedEngine-master\WickedEngine\wiTerrain.cpp`
- Unreal Landscape weightmap and Runtime Virtual Texture source under `C:\Users\adhir\Downloads\GE\example_repo\UnrealEngine-release\UnrealEngine-release`
- Relevant Fyrox terrain material/mask source under `C:\Users\adhir\Downloads\GE\example_repo`

## 16. Handoff rule

The next session should begin by re-reading `context.md`, `ATTRIBUTION.md`, the available Phase 25M2 record, this plan, and the current terrain/reference source. It should then implement XV-A first. The roster and architecture above are the researched default, but measured evidence may change an implementation detail. Any such change must be recorded here with its reason, benchmark or visual evidence, and attribution impact.

This file is a research and implementation plan only. No Phase XV engine code or texture assets were added as part of its creation.
