# Phase TALOS — Coastal performance with a deliberate quality budget

> **Status, 2026-09-08:** Scope reduced by user request: keep the new graphics scalability controls and small rendering cleanups. The broader C–H optimization program and its 60 FPS acceptance target are deferred, not requirements for the current work. The original plan remains below as reference. See [implementation and evidence](<phase TALOS/README.md>).
> **Date:** 2026-09-06. **Audited revision:** d51e550 on dev.
> **Primary workload:** Coastal, at the user's 2K display/viewport, in motion as well as at rest.
> **Objective:** Substantially improve real frame throughput while retaining approximately 90% of the current visual experience.
> **Stack verified:** Rust 1.88 / edition 2024; wgpu and Naga 30.0.1 in Cargo.lock.
> **Supersedes:** This file's 2026-09-05 exact-equivalence-first proposal. Git retains that version.

## 1. Executive decision

**TALOS should buy a large reduction in Coastal frame time by reusing terrain material work and reducing expensive shading resolution where the image tolerates it.** Exact equivalence is no longer the objective. Fix actual rendering defects, preserve the scene's identity, and spend a limited quality budget on texture filtering, indirect-light detail, reconstruction, and small water frequencies.

The previous plan correctly identified cache defects, but made an exhaustive equivalence exercise the entry ticket to a small FFT optimization. That is the wrong priority for a user reporting roughly 25+ FPS at 2K. A 1% kernel win cannot close that gap. Conversely, accepting some approximation does not make stale wetness, rectangular terrain bands, broken history, or missing objects acceptable.

Proceed sequentially:

1. Establish the actual Coastal 2K baseline and effective settings.
2. Test the existing FSR/resolution path at fixed scales; repair only the integration defects exposed by those tests.
3. Correct and qualify the existing terrain clipmap for a performance configuration.
4. Re-measure. If the frame still misses its budget, choose the largest remaining opportunity: reduced-resolution GI, then ocean simulation or AO if their measured costs justify them.
5. Package the smallest successful combination into existing settings, verify motion and editing, and stop. A compute material resolve is an escalation if these approaches fail, not mandatory phase infrastructure.

**Existing Rust and WGSL may be edited, simplified, or replaced. New standalone files are not required.** Preserve a reproducible baseline for comparison during development, then remove failed experiments and superseded duplication. Do not build a second renderer, cache manager, quality framework, job system, or shader registry.

### Authority and scope

The user's current request controls this revision. The attached optimization brief and older phase records are evidence and historical proposals; their imperative wording does not authorize implementation. Their 100% parity rule, new-files preference, and prohibition on sample/detail reductions do not govern this revised plan. The older TSUSHIMA preference for appearance regardless of cost is likewise superseded for TALOS.

The revision above was originally authorized as plan-only work. The subsequent user request on 2026-09-06 explicitly starts TALOS and requests performance-minded defaults and editable graphics scalability, superseding that restriction and the PERSONA-first schedule. The A/B implementation and its remaining acceptance limits are recorded separately; the plan below retains the broader phase contract.

## 2. Success means a faster Coastal scene, not a favorable screenshot

Treat “2K” provisionally as **2560×1440 output**, not as a verified current internal resolution. Editor chrome can make the scene viewport shorter. Record physical window size, actual displayed viewport, DPI, internal scene target, and reconstruction output separately. A 2560×1392 scene is a distinct workload; a toolbar label is not its measurement.

### Performance targets

These are proposed engineering targets, not results or a guarantee:

| Gate | Requirement |
|---|---|
| Main target | Coastal ground and a repeatable Coastal walking route: mean GPU frame at most **16.67 ms**, aiming for **60 real rendered frames/s** under presentation conditions that permit it |
| Meaningful improvement | At least **40% lower mean GPU frame time** against the matched current baseline; approximately **2× throughput** is the preferred outcome |
| Motion/pacing | Route frame-wall p95 at most **22.2 ms**, p99 at most **33.3 ms**, with no new recurring compilation, resize, cache-fill, or allocation stalls |
| Secondary regression | Island and Coastal overview: no repeatable regression above **5%** at matched settings unless an explicit quality gain is documented and accepted |
| Quality | Pass the appearance and motion contract in §3 at the final combined settings |
| Honesty | If only a partial improvement lands, publish it as partial. Do not declare TALOS's performance objective met because an individual pass improved |

Approximately 25 FPS corresponds to 40 ms/frame, but the user's observation is not a matched GPU measurement. Moving from 40 to 16.67 ms would require about 58% less frame time. Use the actual baseline to compute the required saving.

A useful change that saves under 1% of the frame can accompany a nearby fix, but does not earn its own optimization project. A new substantial pass should normally demonstrate at least **5% net frame saving** on its admitted configuration; a small in-place kernel or ownership correction can qualify at **1%**. These are prioritization thresholds, not permission to reject correctness fixes.

### Historical evidence and its limits

GPU times below are historical RTX 5080 Laptop / Vulkan measurements. They establish where to investigate, not what d51e550 currently achieves.

| Record | Actual internal target | GPU frame | Shading | ReSTIR GI | Water prepass |
|---|---|---:|---:|---:|---:|
| PORTAL-0 final Coastal ground, cache off [R1] | 1920×1032 | 21.4390 | 11.5340 | 2.9059 | 2.3109 |
| PORTAL-0 final Coastal ground, cache on [R2] | 1920×1032 | 9.0959 | 1.6546 | 1.7976 | 1.9741 |
| DOOM-M Coastal ground, recorded defaults [R3] | 1920×1032 | 20.3849 | 11.6107 | 3.0327 | 2.2135 |
| DOOM-M Island ground, recorded defaults [R4] | 1920×1032 | 13.1794 | 6.2000 | 1.3807 | 2.2390 |
| PORTAL-0-A Coastal ground, “720” [R5] | 1280×688 | 10.0490 | 4.3894 | 0.9391 | 1.9071 |
| PORTAL-0-A Coastal ground, “2560” [R6] | **1920×1032** | 21.7695 | 11.7402 | 3.0521 | 2.3126 |

The filename in R6 is misleading: its header is not a 2560-wide render. DOOM-M also explicitly says its earlier DOOM-A baseline was 2560×1392, so the historical 38.392 → 20.385 ms comparison is **not** a measured optimization [R7].

The PORTAL cache pair is **6.97× shading throughput and 2.36× frame throughput**. Shading accounts for 9.8794 ms of the 12.3431 ms frame reduction. Other scopes moved too; do not attribute the entire difference to deleted terrain instructions or add the later shadow-cache gain to it.

Other useful denominators:

- PORTAL's terrain-only shading ablation was 11.461 ms against 11.463 ms unablated. That makes live terrain the first source target, not a general geometry rewrite [R8].
- DOOM-M static shadows cost 0.0023 ms. Moving casters and camera motion need separate evidence; another static shadow cache has no worthwhile denominator [R3–R4].
- Water reflection was only 0.1217 ms in R1, separately from the 2.3109 ms prepass. Reducing reflection rays cannot claim the FFT/prepass budget.
- R2 records Frame CPU 3.8587 ms, Surface acquire 0.0399 ms, and Frame wall 16.8980 ms. It is presentation-paced. CPU/GPU times are not additive, and acquisition is not useful CPU work.
- DOOM-J measures resource footprint and churn, **not DRAM bandwidth**. No bandwidth-versus-ALU conclusion follows from that inventory alone [R9].

Do not multiply all the historical speedups together. Cache reuse, reduced scene resolution, and reduced GI resolution overlap. Re-rank using the **remaining** frame after each accepted change.

## 3. Replace exact parity with an explicit appearance contract

“Approximately 90%” is the user's tolerance for modest visible compromise in exchange for major speed. It is **not** SSIM ≥ 0.90, 10% arbitrary changed pixels, or permission to remove 10% of the scene. No consulted paper supplies that conversion.

Keep a current **Reference** configuration and one proposed **Coastal Performance** configuration. These are comparison/settings bundles over existing controls, not new rendering backends. Record every changed setting and the visible compromise it buys.

| Area | Permitted experiment | Acceptance boundary |
|---|---|---|
| Scene reconstruction | Fixed internal scale 0.75 first, then about 0.67 if needed, reconstructed to the same 2K output | Readable near terrain, stable foliage edges, no persistent trails or night-edge failure; UI remains at display resolution |
| Terrain | Cached material, softer distant normals, filtered small texture detail; POM omission where its depth cue is unimportant | Keep painted material identity, cliffs, macro variation and landscape shape; no hard cache boundaries, missing data bands, or stale weather |
| GI | Half-width/half-height evaluation, fewer spatial reuse taps, smoother indirect detail; existing IBL/DDGI alternatives may be compared | Preserve important indirect color and contact cues; reject leaks, lagging illumination and black fallback surfaces |
| Water | Lower FFT resolution or reduced update frequency if the actual wave motion remains convincing | Preserve coastline, body coverage, large waves, reflection/refraction behavior and foam continuity |
| AO/shadows | Lower evaluation resolution or filter budget when measured worthwhile | Keep foliage grounding and terrain readability; no renewed contact-shadow dashes |
| Geometry and content | Existing LOD/distance controls may be tuned if geometry becomes a demonstrated bottleneck | No deletion of authored layers, unexplained foliage thinning, missing silhouettes, or shortened coastline to win a benchmark |

Retain the recent TSUSHIMA fixes for horizon/sky visibility, AO multiplication, foliage transmission visibility, relief variance and specular stability. These preserve large visual cues or prevent defects; disabling them is not the first quality trade.

Use paired stills and short sequences at identical scene states:

- Ground: sand/rock/grass junction, looking down and at grazing angles.
- Shore: water edge, moving boat, foam, bright reflections.
- Overview/cliff: near-to-far transitions, horizon silhouettes, cache rings.
- Low sun, overcast/wetness transition, and night.
- Slow walking, quick pan/turn, teleport/camera cut, paint/sculpt, and resize.
- Island as a smaller-scene check; keep its authored look as the comparison reference.

At normal output size and playback speed, the owner must find the compromise acceptable. Also inspect cropped problem areas and temporal sequences; a small but conspicuous band can hide in a whole-image average.

FLIP difference maps can support still-image review; FovVideoVDP can support sequence review if needed. Record display/viewing assumptions and local errors, and use them to locate regressions rather than inventing a universal score [L6–L7]. CHI research on frame-rate variation supports evaluating pacing alongside mean FPS [L8].

**Keep correctness and quality checks separate.** Existing exact/regression checks continue to apply to invariants and unchanged configurations. The repository's 0.2%/24 golden comparator must not be globally loosened. Intentional Performance image changes receive their own comparison and acceptance record. Current PERSONA UI golden mismatches are separate; the older claim that only sculpt-panel is outstanding is no longer a complete account [R10].

## 4. Current-code audit: what changes the plan

### 4.1 Terrain: a valuable existing cache with specific defects

The complete route is terrain upload → clipmap update/jobs → generation → material resolve → shared lighting. The existing files already own that route [R11–R14].

- **Specialization already exists.** ShadingSpec carries clipmap and live_terrain. The all-cached case deletes live material evaluation; adding the same override again is not an optimization. Mixed cached/live content keeps both paths and must be measured as such.
- **VT can own the cache.** reconcile_clipmaps combines the debug toggle with virtual-texturing ownership, subject to forced-off state. VT source placeholders are not a valid live reference. Log actual source residency and effective per-terrain modes.
- **Normal blending is wrong relative to the live formulation.** terrain_generate_texel starts n_ts at (0,0,1), then adds weighted normals. Live evaluation sums surface gradients. For one layer with normal (0.6,0,0.8), the live gradient is 0.75 while the generated/decoded gradient is 1/3: an 18.435° normal disagreement. Zeroing the accumulator alone still does not fix multilayer composition.
- **Wetness is cached without a complete dependency key.** Generation bakes wetness into albedo/roughness/alpha. Weather writes terrain.wetness, but TerrainClipmap::update receives camera position and edit_revision. A stationary cache can stay dry while weather changes.
- **Approximation is inherent.** Two Rgba8Unorm material outputs, differing footprints, nonlinear strongest-four/height blending, and no cached POM cannot generally reproduce live shading exactly. That is now an error budget to evaluate, not a reason to abandon reuse.
- **Missing coverage is a different problem.** Readiness, ring fallback, toroidal addressing, update ordering and per-record uniform slots remain correctness requirements. The single-layer normal counterexample does not prove the cause of the historical dark band.
- **Current defaults differ from old notes.** context.md records shipped map hex/POM off. Verify effective values, but do not advertise turning off already-disabled work as a new saving.

The current cache uses eight detail and four macro rings at 1024², two four-byte targets per ring: **96 MiB per terrain**, before source VT and other resources. It already has dirty rectangles, readiness, generation order and a shared per-frame texel budget. Extend these owners, not a speculative MaterialCacheDecision service. A small dependency key or decision enum inside the existing terrain module is sufficient until a second real caller demands more.

### 4.2 Resolution: infrastructure exists; its quality and resize costs matter

scene_size_for_preset, DynamicResolution, and FsrPass already exist [R15–R17]. FSR is the authored AA default where supported. At a 1:1 ratio it is reconstruction/AA work, not evidence that pixels were reduced.

The current dynamic-resolution controller has hysteresis, cooldowns and settled-sample handling, but changes allocate scene targets and reset temporal resources. FsrPass::resize replaces textures and previous-depth storage. Therefore:

- Start with fixed scale, not an oscillating controller.
- Use existing resolution caps when they produce the desired dimensions.
- If another ratio is needed, extend the existing sizing path rather than adding another controller.
- Admit automatic scale changes only after measuring their hitches. Subrect rendering into persistent maximum-size resources is a later change if allocation stalls are actually the blocker.

Source review found two reconstruction gaps and one stale description:

1. FsrDispatchInfo supplies neither reactive_mask nor transparency_and_composition.
2. The opaque velocity shader reconstructs static-point camera motion from depth. It has no previous object transform or deformation input; its comment that nothing moves is stale. **Water does overwrite velocity in its prepass**, so this is not an absence of all object/surface motion support.
3. The fsr.rs introductory Karis/RCAS description is stale. Current sanitize/output WGSL uses linear pre-exposed HDR and bounded output sharpening, with backend RCAS disabled. Do not “repair” it back to the old compressed input.

These are source-confirmed limitations, not fresh observed ghosting results. Fix the minimum failing class in the moving-scene gate. A reactive mask can reduce accumulation for animated transparency; it cannot replace missing opaque object motion.

### 4.3 GI: resolution and history are a credible second major opportunity

RestirGiPass owns two **48-byte reservoirs per scene pixel**, two full-resolution dispatches and an Rgba16Float output. The shader creates one initial bounce candidate, considers four spatial neighbors, and validates the selected connection. There is no many-samples-per-pixel knob whose simple reduction explains the whole cost [R18].

At an actual 2560×1440 scene target the reservoir pair alone is **337.5 MiB**; half width and half height makes it **84.375 MiB**. This is arithmetic from the allocation, not measured residency or bandwidth. It is a credible reason to investigate cost and working set together.

Temporal reuse currently reads gi_a[index] at the same screen index. It does not consume motion vectors or previous primary-surface guides. The Rust pass resets history for resize, toggles and material lighting changes, but that does not make same-index reuse valid during ordinary movement. Current velocity is also scheduled after GI.

A reduced-rate GI path needs a clear mapping between GI pixels and full-resolution depth/visibility, plus compatible temporal state. Reuse existing motion where valid or use previous matrices for static geometry, validate previous surfaces, and invalidate dynamic/disoccluded samples. Moving velocity earlier is a dependency change to audit, not a free wiring edit. Do not add temporal smoothing on top of invalid history.

The shared shading consumer performs a direct full-resolution textureLoad of restir_gi. Simply allocating a smaller output is incorrect. Initially reconstruct into the existing full-resolution radiance/validity output; keep its interface and lighting composition intact.

### 4.4 Water: distinguish simulation, rasterization, and reflection

Water prepass includes spectrum.record and surface rasterization. It is not a pure FFT timestamp. The spectrum already uses a workgroup-memory Stockham transform, a transpose, four packed spectra, and three 1024² cascades. On an update all cascades run; catch-up advances accumulated time but performs one transform set [R19–R20].

A paired-butterfly kernel is a valid small experiment, but should compete with the now-permitted **512² simulation** and cadence choices. Do not spend weeks proving byte-identical ocean arithmetic while a modest spectral-detail reduction may save much more.

Keep the optical/body definition initially: datum, depth, wave-speed controls and shoreline are scene semantics. Optimize the sampling/simulation budget first. Dropping small frequencies changes normal variance and foam; assess those rather than promising 4× whole-prepass speed from 4× fewer texels.

### 4.5 Geometry, post effects, Rust and system boundaries

Two-phase Hi-Z/visibility, GPU meshlet culling, dense indirect submission, dirty BLAS rebuilds, static shadow caching, existing render scratch and a shader registry already exist. The recorded low geometry and CPU terrain costs do not justify SIMD culling, triangle filtering, or a new staging allocator as the main Coastal intervention [R7–R9, R21].

GTAO currently traces and denoises at scene resolution. It is eligible for half-resolution evaluation if it remains significant after scene-resolution/cache/GI changes. Clouds, volumetrics, bloom, histogram and reflection work should be ranked from actual enabled scopes, not a list of effects the engine supports.

Ponytail/codebase-design conclusion: keep policy with the existing resource owner. SomniumRenderer schedules passes; TerrainClipmap owns material-cache state; RestirGiPass owns GI dimensions/history; WaterSpectrumPass owns spectrum resources; viewport_resolution owns scaling. Avoid adding independent state to EngineContext or a growing quality switch in every central type.

Rust review conclusion: do not introduce unsafe/SIMD, another async runtime, or a custom mapped staging ring without a measured owner-level problem. DOOM-J's Island allocation churn remains a conditional investigation. A mapped buffer is not usable as a GPU copy source indefinitely, and two slots do not prove safety under arbitrary GPU latency. If upload batching becomes material, inspect wgpu's installed StagingBelt first.

Keep the shared scene-global resource layout, Rg32Uint instance/primitive identity, 32 authored terrain layers, and the current 2,080-byte terrain material layout as the starting interfaces. New private pass resources do not require a second scene pool. Any necessary layout change must update producer, consumer and layout tests together; serialized scene/schema changes require compatibility handling. Source-file flexibility is not permission for mismatched GPU layouts or lost scene data.

## 5. Implementation sequence and falsifiers

The entries below define the phase contract; the first A/B slice is now in tree, with remaining acceptance and later stages open. Run one bounded experiment at a time. After each accepted change, refresh the cost table and stop implementing optional stages once §2 and §3 pass.

### TALOS-A — establish the Coastal contract

**Work:** Extend the existing timing/capture path only for missing facts: effective settings, both resolutions, a reproducible walking route, and the required percentile/window summaries. Existing .somtime has p99/hitch information; verify/add p95 rather than claiming it is already emitted. Reuse existing named views for stills. If recording a route is missing, use one small deterministic camera/time sequence, not a benchmark framework.

Prepare current Reference and one fixed-scale candidate. Keep the display viewport identical, DRS off, and scene simulation/time matched. Include the editor state that matters to the user, then distinguish its overhead from scene rendering.

**Exit:** A current, correctly labeled 2K baseline; cost breakdown; reference images/sequence; repeatability envelope; actual feature/capability settings. State whether the complaint is native shading, an existing reduced-resolution path, or a pacing/driver problem.

**Falsifier:** A mismatch in dimensions/settings or strong thermal/session drift invalidates the comparison. Repair the measurement rather than coding against it. No deep profiler extension is required if existing scopes answer the question.

### TALOS-B — use fewer scene pixels without losing the scene

**First experiment:** Existing FSR at fixed 0.75 scale, then approximately 0.67 only if more saving is needed. At 2560×1440 those inputs are 1920×1080 and approximately 1715×965; AMD's exact 1/1.5 Quality ratio is about 1707×960 before implementation rounding. Log actual results rather than using these example dimensions for an editor viewport of a different aspect.

At scale s the pixel count is s². A simple opportunity model is:
**new cost = fixed work + pixel work × s² + reconstruction/transition overhead.**
This is a model, not a measured speedup. FFT, CPU work and many shadow costs do not shrink with scene pixels.

Repair concrete reconstruction defects exposed by §3: correct mask wiring for unstable transparency, reset rules, and missing object/deformation motion where it visibly fails. Reuse current instance submission identity for previous state only if it is stable across sorting; otherwise introduce the smallest explicit per-object history mapping. Do not use the previous sorted instance index as identity. Keep jitter/depth conventions and the current linear HDR contract.

**Exit:** A fixed-scale candidate with useful net GPU saving and acceptable still/motion quality. Publish the internal/output sizes and full reconstruction cost. If it already achieves the target, finish acceptance rather than forcing further subsystems to change.

**Falsifier:** The scale was already active, quality fails at the allowed floor, reconstruction overhead erases the gain, or the workload is no longer dominated by pixels. Stop decreasing resolution; continue with material reuse. Do not force a lower floor merely to pass.

### TALOS-C — make the existing terrain cache a performance option

**Work, in this order:**

1. Add only the focused algebra checks needed for the normal fix: one-layer, two differing normals, flat input, and the existing z clamp. Check production composed WGSL with a small device fixture when implementation exists. Do not commission a general material-equivalence laboratory.
2. Correct generation/decoding to agree on gradient composition. Begin with current formats and compare the decoded result; expand storage only if visible error justifies its memory/bandwidth cost.
3. Separate slowly changing material content from weather response. Prefer caching **dry albedo/roughness plus material moisture response** and applying current wetness after sampling and cliff mixing. Alpha is a possible moisture channel; check both producer and consumer and the AO validity sentinel. Preserve the actual operation order, including cliff moisture. Paint/material/texture/VT changes invalidate content through existing revisions/events.
4. Verify coverage and updates during movement, painting, cold load and teleport. Existing generation budgets and readiness are the starting point. Use a valid coarser result while refining; if no valid coverage exists, use a deliberate live fallback with resident sources or complete coarse coverage before exposing the view. Never label a mean-color VT placeholder as a full live material.
5. Measure warm, moving and edited cache costs. Tune density/update budget only if refresh work or coarse appearance is the limiting issue. State the tradeoff; do not expand every ring to force exact parity.
6. Admit POM omission and different cache footprints in Performance where §3 passes. Reference remains live for content that needs the stronger detail. Avoid per-pixel live/cached blending that retains the full expensive call graph unless its measured quality benefit pays for it.

**Primary files:** terrain/clipmap.rs, pass/terrain_clipmap.rs, terrain_material.wgsl, clipmap_gen.wgsl, clipmap_shade.wgsl; the existing renderer selection/upload seams.

**Exit:** No stale wetness or invalid coverage; acceptable normal/material appearance in motion; substantial net savings after generation, fallback and VT work. Recommend the cache in the Performance bundle only for content that passed. Record a separate global-default recommendation at closure.

**Falsifier:** Moving refresh/fallback cost erases the saving, recurring seams remain, or quality is unacceptable. Keep the live route and B's independent gains. Do not turn “all terrains must be provably equivalent” into an indefinite blocker; qualify actual content and document unsupported cases.

### TALOS-D — reduce GI work if it is the next bottleneck

**Entry:** GI is a substantial residual cost or source of tail spikes on the accepted B/C settings. Keep ReSTIR DI separate: it is a smaller historical cost and suppresses other shadow work.

**Work:** Compare full GI with an explicit half-width/half-height GI candidate. Start with a stable representative depth/visibility sample per 2×2 block, explicit full↔GI coordinate transforms, GI-sized reservoirs and dispatch bounds, and depth/normal-aware reconstruction to the current output interface. Do not average IDs or interpolate across silhouettes. Use receiver/guide validity to fall back where no compatible sample exists.

Fix history addressing/rejection before relying on temporal reuse. Record previous surface position/normal as needed; correct reservoir transport must know the old receiver, not substitute the new position. Preserve sample accounting, visibility validation, radiance-versus-albedo convention and night/disabled alpha semantics. Clear both stages on cuts, map changes, resizing and invalid history.

Try reducing four spatial taps to two only after the resolution result is known, as a separate measured quality trade. Add a modest spatial/variance filter only if the saved ray/reuse work pays for it. SVGF and Bevy Solari are references for reconstruction/history responsibilities; TALOS does not adopt an entire denoiser SDK or radiance-cache architecture [L4, E2].

If ray-query GI remains too costly, compare the existing IBL and SDF-DDGI configurations as explicit alternatives. They are different illumination models; inspect them in shadowed terrain and near foliage, and never describe the small 4³ probe grid as equivalent full-scene GI.

**Exit:** At least 5% net frame improvement for a substantial new reconstruction path, lower GI working set, and acceptable motion/lighting at the combined settings. Report GI initial, spatial/visibility and reconstruction costs separately if needed to attribute the result.

**Falsifier:** Temporal leaks/lag persist, the upscale/filter consumes the saving, or B already made GI too cheap to matter. Reject the candidate or choose an accepted existing fallback; do not compensate with an ever-larger denoiser.

### TALOS-E — reduce ocean cost when its denominator survives

**Entry:** Water remains a material fraction of the frame after earlier changes. Split its prepass timing just enough to distinguish spectral update from surface rasterization. Record update/no-update frames and catch-up intervals.

**First experiment:** 512² instead of 1024² for the existing three-cascade simulation, at the same 50 Hz schedule and optical/body settings. Parameterize existing resource sizes, butterfly dispatches and row indexing together. Check power-of-two limits, workgroup bounds, transpose orientation, spectrum energy and output bindings. Do not simply change an allocation while leaving hardcoded shader strides.

Keep the same low-frequency sea state and evaluate the lost high-frequency energy, displacement, normal variance, whitecaps and foam. If one cascade visibly suffers, consider a mixed-resolution roster only when that additional scratch/layout complexity is justified; it is not the initial design.

If spatial reduction fails quality, test the existing-size paired-butterfly ownership from the former TALOS plan **in the current kernel**, with a temporary comparison variant. Each pair shares input loads/twiddle work and writes both outputs. Use a small CPU DFT oracle plus production device outputs; numerical tolerance and the appearance contract replace byte identity. No new permanent shader file is required.

A lower cadence, e.g. 25 Hz with interpolation, is another conditional quality experiment, not bundled with the first resolution change. Phase and foam evolution must use elapsed simulation time. Pause/hidden water can skip work only when no render/query consumer needs the updated state.

**Exit:** Meaningful net frame saving, stable wave/foam motion, no shoreline/body change, and bounded update spikes. Include extra interpolation buffers/copies in memory and timing.

**Falsifier:** Optical appearance, low-frequency waves or foam fails, or rasterization—not spectrum—is the actual cost. Reject the spectral change and investigate the measured part. Do not call the separate reflection pass the prepass.

### TALOS-F — finish with the smallest remaining change

This is a decision gate, not a bag of mandatory optimizations.

- If GTAO remains expensive, test half-resolution tracing with depth-aware reconstruction of AO **and bent normal**; retain all existing AO composition rules. Count the filter and full-resolution consumer cost.
- If live terrain remains dominant because C cannot qualify, test a compiled two-contributing-layer Performance evaluator while retaining all 32 authored layer IDs. Keep the current four-layer route for Reference; record changes at blend boundaries. Reducing the storage ABI or silently discarding the upper bank is not the same experiment.
- If allocation/resize hitches dominate, fix their existing owner. A reusable buffer or cached view/bind group is preferable to a general upload framework.
- Only after fixed settings pass, try the existing DynamicResolution controller with the validated floor and target. If it repeatedly resets history or reallocates at visible cost, ship the fixed configuration. Persistent maximum-size targets are an escalation only for a demonstrated transition bottleneck.

**Exit:** Pick at most the next measured opportunity, validate it, and return to the final gate. No implementation is required for a bullet whose entry condition is false.

**Falsifier:** Cost moved elsewhere or the candidate cannot clear noise/quality gates. Stop that experiment and preserve the record.

### TALOS-G — conditional material-compute escalation

**Entry:** Coastal still misses the target, live material evaluation still dominates, and cache/reconstruction/material-budget options have been exhausted or rejected on evidence.

Build one terrain-focused compute material candidate using explicit gradients and the current material logic, with the cheapest viable routing. Do not migrate sky, foliage, lighting, transparency and post simultaneously. Count classification, storage output bandwidth and its consumer pass.

The current analytic UV neighbor-barycentric path can extend to world-position gradients, but it differs from fragment quad derivatives at primitive boundaries. The pinned Naga frontend also recognizes quadSwapX/Y/Diagonal and quadBroadcast. Adapter feature/operation/stage support and pixel-to-lane mapping still require a bounded device probe; existing clipmap comments record a failed compute bindless sampling attempt that must be explained before another port [R22].

**Exit:** At least 10% total-frame saving over the best accepted live-material configuration, within §3. If a smaller shader change achieves it, prefer that.

**Falsifier:** Device/compiler path fails, added traffic consumes the saving, or occupancy does not improve. Keep the fragment implementation; do not ship a second resolver just because the port compiles.

### TALOS-H — accept, integrate and close

Choose the smallest combination meeting §2–§3. Keep a Reference option through existing settings, and expose the accepted Performance values through the existing schema/AA/viewport controls rather than parallel booleans. Distinguish authored map data from renderer quality overrides; preserve saved scene round trips.

Publish a before/after table with actual resolutions, mean/p95/p99, per-pass deltas, memory peak/steady state, effective options and known compromises. Include Coastal moving/ground/overview plus Island and low-light results. Report each optional sub-phase as accepted, rejected, or not needed.

A modest improvement is worth keeping but is not phase completion. If the target is missed, report the best achieved result, the remaining dominant cost and whether G was justified. No invented FPS estimate substitutes for that outcome.

## 6. Measurement and validation contract

No benchmarks run while authoring this plan. The following is the implementation protocol.

1. **Identity first.** Record revision, executable/build profile, adapter/driver, power mode, actual sizes, AA, DRS, clipmap/VT per terrain, ShadingSpec, RT/DI/GI state, view, sun, simulation state and residency. Pin release settings; do not compare debug and release.
2. **Use existing evidence infrastructure.** .somtime and captures are the baseline. Add missing p95/moving-route/config fields locally. Retain current GPU timestamps; add subscopes only where a combined scope prevents a decision.
3. **Separate attribution from user experience.** Fixed-scale paired runs isolate algorithms. Final runs use the actual chosen settings. Report GPU time, frame-wall time, Frame CPU and acquisition separately; record VSync/cap/VRR policy. If presentation limits wall FPS, report that fact without pretending reciprocal GPU time is observed display FPS. Frame generation does not count toward the target.
4. **Bound the run.** Prepare a deliberate editor session, not a loop of hello_engine launches. Release temporary captures and unused targets. Stop on unexpected continuing memory growth. Prior sessions exhausted system memory. New multi-window timing support, if needed, is a small extension—not an existing switch.
5. **Warm and pair.** After assets/pipelines settle, use the established 180-warmup/300-measured policy for initial static windows. Warm each configuration and repeat matched A/B windows in reversed order, e.g. A/B then B/A. Expand only if uncertainty changes the decision. A 300-frame sample is exploratory for p99; final route acceptance should cover at least 60 seconds per configuration and enough samples to expose recurring stalls.
6. **Match time, not just frame index.** Use identical camera paths and simulation times. Faster rendering changes ocean update fractions and stochastic histories. Cache edits, VT admissions, foam, exposure and reservoirs must not advance once for one candidate and twice for the other.
7. **Handle nondeterminism.** Existing unchanged-build captures differed by 2.80% in DOOM-I. Establish self-agreement; distinguish image noise from a candidate's change. Small algebra/device fixtures can be deterministic even when a full editor run is not. Do not raise the old global golden threshold to fit noise.
8. **Controls and uncertainty.** Report individual paired deltas and spread, not just within-run sigma. Untouched work counts must remain comparable. GPU scopes need not match to three decimals; correlated shifts across all passes suggest a confound. A null is an acceptable outcome, not permission to relabel noise as improvement.
9. **Count complete cost.** Include cache regeneration, full-resolution reconstruction, memory traffic, copies, fallbacks, compilation/prewarming, resize transitions and update/no-update frames. For micro-optimizations use the current residual frame as denominator.
10. **Validate changed behavior.** Run focused arithmetic/layout/composition tests as appropriate, then cargo test --workspace -j 1 and python tools/ghostfence/run.py. Report PASS/FAIL/SKIP accurately. GPU-free Naga validation does not prove device layouts, startup or appearance. Perform the bounded device/editor acceptance for any implemented renderer change. A plan-only edit needs link/diff review, not engine tests.
11. **Separate default decisions.** Leave defaults intact during experiments. At closure, recommend the accepted Performance bundle and any justified default change with evidence. Do not weaken the existing clipmap-off guard merely to make an experiment pass; update its intent only when an actual default change is authorized.

## 7. Research and history: what was used, and what was refused

### Git and Graphify

The renderer source has no changes between the earlier TALOS audit revision 3c4e33a and d51e550; core/editor wiring and context have changed. The old source findings therefore remain relevant, but their priority and constraints do not.

History reviewed includes:

| Commit / record | Consequence for TALOS |
|---|---|
| DOOM-M close-out, 2026-08-30 | Retain the shadow cache; respect measured nulls, unresolved GI tails and resolution mismatch |
| b1686fe, 1deee61, c005e27, f15a728, b5ffb1c, b38ac74 | Clipmap toggle/VT ownership, cold fallback, toroidal address and uniform-slot failures are distinct; a checkbox or stationary picture cannot close the cache audit |
| ef8e5cb through bfdc046 and 673d243 | Landscape visibility, relief/specular variance and macro appearance are recent investments worth preserving |
| 5c6ee4a, 9cafaaa, eb61dc3 | Ray-query driver/compilation incidents and lightweight hit-material evaluation; keep terrain_splat_core separate from the expensive raster call graph |
| c4473a5, fba14e0 | Foliage/material-lighting corrections precede this baseline |
| d51e550 and recent PERSONA records | Native editor context and visual goldens have moved since the original handoff |

Graphify's GRAPH_REPORT.md is dated **2026-08-27**, not current profiling evidence. It reports SomniumRenderer as a 169-edge hub and a major cross-community bridge. Use that to locate ownership and avoid central-type growth; do not infer execution time from graph degree. Current code wins over graph edges and stale comments.

### Reference mirror audit

Read-only root: C:/Users/adhir/Downloads/GE/example_repo. Some archives contain a second nested project directory; the actual paths below reflect that. These are scoped source inspections, not an exhaustive audit of every engine.

| ID | Exact local source inspected | What it contributes |
|---|---|---|
| E1 | o3de-development/o3de-development/Gems/Terrain/Code/Source/TerrainRenderer/TerrainClipmapManager.cpp | Existing clipmap resource/update ownership and differentiated channel formats. Header: Apache-2.0 OR MIT; root licensing read. Supports reuse; does not prove Somnium's cache quality. |
| E2 | bevy/bevy-main/crates/bevy_solari/src/realtime/restir_gi.wgsl | Temporal motion/previous-surface inputs and reservoir reuse. MIT root read; inspected shader starts with the ReSTIR course reference and imports. Its G-buffer/world-cache assumptions cannot be transplanted wholesale. |
| E3 | WickedEngine-master/WickedEngine/shaders/visibility_velocityCS.hlsl and visibility_surfaceCS.hlsl | Previous-surface velocity, wind-aware surface reconstruction and explicit quad organization. MIT root read; inspected file headers carry defines/includes, no separate restriction. |
| E4 | GodotOceanWaves-main/assets/shaders/compute/fft_compute.glsl | Stockham/workgroup-memory structure already present in Somnium. MIT root read; shader also credits its Stockham source. No need to “discover” shared memory again. |
| E5 | The-Forge-master/Common_3/Renderer/VisibilityBuffer2/VisibilityBuffer2.cpp | Multi-view/geometry-set filtering and batch ownership. Apache-2.0 root and file header read. Architectural reference only; no Coastal geometry bottleneck demonstrated. |

Follow ATTRIBUTION.md §14–15 for any later adopted pattern. No external source was copied or added to the workspace. No SDK adoption is proposed. The handoff's broad NRD/RTX licensing claims were not independently re-audited here; inspect the exact version/license if that decision ever becomes relevant. Published reconstruction methods suffice for the present plan.

### Primary literature and official sources checked 2026-09-06

These sources inform mechanisms and evaluation. Their published speedups are not Somnium predictions.

| ID | Source | Decision it supports |
|---|---|---|
| L1 | AMD, [FSR 3 quality modes](https://gpuopen.com/fidelityfx-super-resolution-3/) and [FSR upscaler integration](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-upscaler/) | Compare fixed internal ratios; honor motion, masks, HDR and history. Hardware/SDK/backend compatibility must be checked against the installed wgpu-ffx, not assumed from newer vendor SDKs. |
| L2 | Mikkelsen, [Surface Gradient–Based Bump Mapping Framework](https://jcgt.org/published/0009/03/04/), JCGT 2020 | Repair normal composition while acknowledging cache filtering/quantization error. |
| L3 | Turánszki, [Derivatives in compute shader](https://turanszkij.wordpress.com/2022/05/08/derivatives-in-compute-shader/), 2022, and [Wicked Engine graphics in 2024](https://turanszkij.wordpress.com/2024/12/10/wicked-engines-graphics-in-2024/) | Analytic neighbor reconstruction and material/lighting separation for conditional G; compute alone does not imply faster rendering. |
| L4 | Schied et al., [Spatiotemporal Variance-Guided Filtering](https://research.nvidia.com/labs/rtr/publication/schied2017spatiotemporal/), HPG 2017 | Sparse radiance requires valid temporal state and edge-aware reconstruction; guide a small GI-specific filter only if needed. |
| L5 | NVIDIA, [Advanced API Performance: Shaders](https://developer.nvidia.com/blog/advanced-api-performance-shaders/) | Investigate register pressure, memory access and generated variants; higher occupancy is not automatically lower time. |
| L6 | [FLIP research](https://developer.nvidia.com/blog/flip-a-difference-evaluator-for-alternating-images/) and [author implementation](https://github.com/NVlabs/flip), HPG 2020 onward | Localized perceptual difference maps for still-image acceptance; not “percent parity.” |
| L7 | Mantiuk et al., [FovVideoVDP](https://www.cl.cam.ac.uk/research/rainbow/projects/fovvideovdp/), SIGGRAPH 2021 | Sequence/display-aware quality evaluation when still comparisons miss temporal loss. |
| L8 | Liu, Kuwahara, Scovell and Claypool, [The Effects of Frame Rate Variation on Game Player Quality of Experience](https://web.cs.wpi.edu/~claypool/papers/frame-variation-chi-23/), CHI 2023 | Mean FPS alone misses experience; measure tails and motion pacing. The study does not establish this phase's exact numerical budgets. |
| L9 | [SIGGRAPH 2026 Advances course](https://advances.realtimerendering.com/s2026/index.html), ORCA / Variable Rate Ray Tracing abstracts | Current work emphasizes reuse and disocclusion-aware ray allocation. Abstracts reviewed; attempted VRRT PDF retrieval failed. No detailed algorithm or speed claim is attributed to unread slides. |
| L10 | [wgpu v30.0.1](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.1), [WGSL specification](https://www.w3.org/TR/WGSL/) and installed Naga source | Distinguish language specification, pinned compiler support and actual device features. No dependency upgrade is required by this plan. |

### Do not repeat these experiments without new evidence

| Idea | Decision |
|---|---|
| DOOM-C raster tile bins | Measured slower at every tested tile size. Compute has different costs, but is G's conditional experiment; classification is never literally free. |
| DOOM-E / per-pixel near/far sample branches | Prior null/regression remains. New quality permission changes what may be tried, not the hardware cost of divergent paths. Existing hex/POM-off settings may leave nothing to strip. |
| Atomic indirect compaction | Prior 66-object result was noise; reopen only for a larger actual draw workload. |
| Blanket f16 conversion | Prior results changed sign across repetitions. Try only a specific measured bottleneck; do not trade stability for an unmeasured register claim. |
| Subgroup histogram optimization | Historical whole-pass share was about 0.22%; irrelevant to closing a 25-FPS complaint. |
| New staging ring / SIMD terrain cull | No current major frame denominator. Fix proven owner-level churn when it causes hitches. |
| Neural texture/radiance cache, NRD integration, hardware VRS/SER, mesh-shader rewrite | Much larger integration/capability surface than the first accepted experiments. Reopen only with a concrete residual bottleneck and supported backend path. |
| Frame generation | Outside the measured real-render-throughput objective and not implemented by the current FSR wrapper. |
| Disable all RT/lighting or shrink the map | An ablation may locate cost. A stripped scene is not the requested result. Existing GI alternatives must pass the same appearance contract. |

## 8. Repository evidence index and audit limits

Paths are relative for portability. Symbol names are the stable locators; line numbers below refer to d51e550 and can drift.

| ID | Source / locator |
|---|---|
| R1 | [PORTAL-0 final Coastal ground](<phase PORTAL-0/PORTAL-0-final_coastal-ground.somtime>), header and GPU/CPU rows |
| R2 | [PORTAL-0 final Coastal ground, clipmap](<phase PORTAL-0/PORTAL-0-final_coastal-ground_clipmap.somtime>) |
| R3 | [DOOM-M Coastal final](<phase DOOM/DOOM-M_coastal-ground_final.somtime>) |
| R4 | [DOOM-M Island final](<phase DOOM/DOOM-M_island-ground_final.somtime>) |
| R5 | [PORTAL-0-A Coastal 720](<phase PORTAL-0/PORTAL-0-A_coastal-ground_720.somtime>), actual render header |
| R6 | [PORTAL-0-A Coastal “2560”](<phase PORTAL-0/PORTAL-0-A_coastal-ground_2560.somtime>), actual render header |
| R7 | [DOOM-M close-out](<phase DOOM/DOOM-M.md>), resolution warning, nulls, GI tails and open work |
| R8 | [PORTAL-0](phase_PORTAL-0.md), ablations and clipmap audit; [unablated record](<phase PORTAL-0/PORTAL-0-A_coastal-ground.somtime>) |
| R9 | [DOOM-J](<phase DOOM/DOOM-J.md>), footprint/churn versus bandwidth; [DOOM-I](<phase DOOM/DOOM-I.md>), capture self-agreement |
| R10 | [context.md](../context.md), current status, architecture, materials and PERSONA gate; [PERSONA E/F](<phase PERSONA/PERSONA-E_F.md>) |
| R11 | [renderer.rs](../crates/somnium_renderer/src/renderer.rs), reconcile_clipmaps at 623, update inputs at 3624, generation at 4202, ShadingSpec selection at 4378 |
| R12 | [terrain_material.wgsl](../crates/somnium_renderer/src/shaders/terrain_material.wgsl), ts_to_surfgrad, terrain_generate_texel, evaluate_terrain_material; [clipmap generation](../crates/somnium_renderer/src/shaders/clipmap_gen.wgsl), packing and validity sentinel |
| R13 | [clipmap_shade.wgsl](../crates/somnium_renderer/src/shaders/clipmap_shade.wgsl), evaluate_clipmap_material at 225; [terrain/clipmap.rs](../crates/somnium_renderer/src/terrain/clipmap.rs), constants, update, take_jobs, fill_gpu, gpu_bytes |
| R14 | [core/app.rs](../crates/somnium_core/src/app.rs), weather wetness assignment at 6883 and terrain slider at 9904; [terrain clipmap pass](../crates/somnium_renderer/src/pass/terrain_clipmap.rs), begin_frame/record uniform ownership |
| R15 | [viewport_resolution.rs](../crates/somnium_renderer/src/viewport_resolution.rs), scene_size_for_preset and DynamicResolution; [core/lib.rs](../crates/somnium_core/src/lib.rs), AA and dynamic-resolution defaults |
| R16 | [fsr.rs](../crates/somnium_renderer/src/pass/fsr.rs), resize, record, FsrDispatchInfo at 229; [sanitize](../crates/somnium_renderer/src/shaders/fsr_sanitize.wgsl) and [output](../crates/somnium_renderer/src/shaders/fsr_untonemap.wgsl) |
| R17 | [velocity.wgsl](../crates/somnium_renderer/src/shaders/velocity.wgsl), fs_main; [water.rs](../crates/somnium_renderer/src/pass/water.rs), record_prepass writes velocity; [global_pool.wgsl](../crates/somnium_renderer/src/shaders/global_pool.wgsl), Instance |
| R18 | [restir_gi.rs](../crates/somnium_renderer/src/pass/restir_gi.rs), RESERVOIR_BYTES, allocation at 329, record; [GI shader](../crates/somnium_renderer/src/shaders/restir_gi.wgsl), gi_primary_surface, initial_and_temporal, spatial_and_shade; [shading.wgsl](../crates/somnium_renderer/src/shaders/shading.wgsl), restir_gi consumer |
| R19 | [water_spectrum.rs](../crates/somnium_renderer/src/pass/water_spectrum.rs), MAP_SIZE, tick schedule and cascade loop; [spectrum shader](../crates/somnium_renderer/src/shaders/water_spectrum.wgsl), butterfly_precompute, fft_row and transpose |
| R20 | [renderer.rs](../crates/somnium_renderer/src/renderer.rs), Water prepass at 4742; [water.rs](../crates/somnium_renderer/src/pass/water.rs), spectrum.record within record_prepass |
| R21 | [classify.rs](../crates/somnium_renderer/src/pass/classify.rs), measured null; [shading.rs](../crates/somnium_renderer/src/pass/shading.rs), ShadingSpec; [raytrace.rs](../crates/somnium_renderer/src/pass/raytrace.rs), pending_blas; [shadow/cache.rs](../crates/somnium_renderer/src/shadow/cache.rs), invalidation; [gtao.rs](../crates/somnium_renderer/src/pass/gtao.rs), target sizes; [ddgi.rs](../crates/somnium_renderer/src/pass/ddgi.rs), PROBE_GRID |
| R22 | Installed Naga at C:/Users/adhir/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/naga-30.0.1/src/front/wgsl/lower/mod.rs:1232,3693,3716,3739; shader generation comments in R12. The enable-subgroups directive remains a separate pinned-compiler issue; language-spec subgroupElect does not prove frontend support. |
| R23 | [timing.rs](../crates/somnium_renderer/src/timing.rs), wall_samples and hitch output; [shader registry](../crates/somnium_renderer/src/shaders.rs); [shader composition](../crates/somnium_shader/src/compose.rs); [shader tests](../crates/somnium_renderer/tests/shaders_validate.rs) |
| R24 | [Graphify report](../graphify-out/GRAPH_REPORT.md), 2026-08-27 snapshot, hub listing; [development index](README.md); [attribution](../ATTRIBUTION.md), §14–15 |

The audit traced the central frame sequence and the terrain, cache, shading/BRDF, reconstruction, GI, water, culling/shadow and measurement seams relevant to the proposals. Development records were read for current status, negative experiments and provenance; this is not a claim that every line of every engine or unrelated editor subsystem was reviewed.

Skills applied in one agent: Ponytail from C:/Users/adhir/.codex/plugins/marketplaces/ponytail/skills/ponytail/SKILL.md; rust-pro and code-review-checklist from the Claude plugin marketplace; codebase-design from the installed skill. Their implementation/delegation suggestions were constrained by the user's explicit **plan only / no multiple agents** request.

Only this phase document is revised. The outcome is a performance-first plan with bounded quality tradeoffs, not a claim of achieved frame rate.
