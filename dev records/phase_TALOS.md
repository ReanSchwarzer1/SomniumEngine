# Phase TALOS — prove equivalence, then reuse the work

> **Status:** Research and implementation proposal; no renderer changes, device tests, or new performance measurements.
> **Date:** 2026-09-05. **Source revision:** `3c4e33a`, branch `dev`.
> **Codename:** The Talos Principle: establish what a mechanism actually guarantees before building on it.
> **Scope:** An independent thesis responding to `somnium_optimization_prompt_v2.md`, not execution of its example tracks.
> **Stack checked:** Rust 1.88; manifest `wgpu = "30.0"`; lockfile resolves both wgpu and Naga to **30.0.1** ([Cargo.toml, line 78](../Cargo.toml), [Cargo.lock, lines 2260 and 4925](../Cargo.lock)).

## 1. Executive decision

**Somnium's largest opportunity is making expensive material work safely reusable. Its immediate obstacle is semantic equivalence, not a missing compute classifier.** The clipmap has demonstrated substantial speed, but its current representation is not generally equivalent to the live material. This audit identifies a concrete normal-composition mismatch and a cache invalidation gap; neither requires another exploratory engine run to establish the premise.

The next implementation should be a small material-equivalence fixture, followed by a narrowly scoped correction to the clipmap's normal algebra. A cache eligibility/dependency contract comes next. Reconsider the default only after those changes survive component, image, and moving-scene gates. The default decision remains separate.

An independent performance experiment is justified in **the ocean's FFT row kernel**: assign both outputs of a butterfly to the same invocation so their input loads and twiddle lookup can be shared. This changes execution ownership rather than wave resolution, simulation rate, or sample count. It benefits either terrain configuration and has a much smaller correctness surface than a new denoiser or a fullscreen compute resolve. Its speedup is a hypothesis, not a promised result.

These are sequential, bounded decisions. TALOS does not commission all four example tracks, migrate the renderer to compute, or turn every uncertainty into a new profiling system.

## 2. Evidence and denominators

All GPU values below are milliseconds on the RTX 5080 Laptop / Vulkan at **1920×1032**, with 180 warm-up and 300 measured frames. Values are historical observations, not fresh measurements of this revision. “Off” and “on” in the PORTAL rows identify that experiment's clipmap setting; the DOOM rows identify its recorded defaults.

| Record / map / viewpoint | GPU frame | Shading | ReSTIR GI | Water prepass | Shadows |
|---|---:|---:|---:|---:|---:|
| PORTAL-0 final, Coastal ground, clipmap off [R1] | 21.4390 | 11.5340 | 2.9059 | 2.3109 | 0.9370 |
| PORTAL-0 final, Coastal ground, clipmap on [R2] | 9.0959 | 1.6546 | 1.7976 | 1.9741 | 0.5581 |
| DOOM-M final, Coastal ground, recorded defaults [R3] | 20.3849 | 11.6107 | 3.0327 | 2.2135 | 0.0023 |
| DOOM-M final, Island ground, recorded defaults [R4] | 13.1794 | 6.2000 | 1.3807 | 2.2390 | 0.0023 |

The PORTAL pair gives **6.97× shading throughput and 2.36× frame throughput**, not a 7× frame improvement. Shading falls by 9.8794 ms while frame time falls by 12.3431 ms. The remaining 2.4637 ms is movement in other work; it cannot be booked as direct elimination of terrain instructions. Nor can the later DOOM shadow-cache saving be added to this older clipmap result to manufacture a new baseline.

Attribution matters elsewhere too:

- PORTAL water **reflection** is only 0.1217 ms off / 0.1029 ms on, separately from prepass [R1–R2, line 42]. A reflection denoiser does not attack the 2 ms ocean simulation denominator.
- On the PORTAL clipmap-on rail, CPU frame body is 3.8587 ms and acquisition is 0.0399 ms; wall time is 16.8980 ms [R2, lines 53–55]. This is presentation-paced, not evidence that the CPU has “no slack.” GPU and CPU budgets are not additive. CPU optimization can still matter for hitches, but these records do not make staging a frame-throughput priority.
- DOOM-M already measures static shadows at 0.0023 ms. Triangle filtering or another static shadow optimization has essentially no opportunity on that particular workload [R3–R4]. Dynamic casters are a different workload.
- DOOM-J explicitly says its inventory measures footprint, not bandwidth. Pixel counts and ALU-heavy source justify a priority; they do **not** prove DRAM bandwidth or texture latency irrelevant ([DOOM-J, lines 174–192](<phase DOOM/DOOM-J.md>)).

For a new optimization, use `saving / matched total GPU frame`, including added passes, cache refresh, copies, and fallback execution. **1% is an admission threshold, not a prediction:** about 0.204 ms on DOOM-M Coastal, 0.132 ms on DOOM-M Island, and 0.091 ms on the historical PORTAL clipmap-on rail.

## 3. Corrections to the handoff

### 3.1 The obvious clipmap specialization already exists

`ShadingSpec` has both `clipmap` and `live_terrain`. Renderer construction starts with both false, marks the cache path for clipmapped terrain, and retains live evaluation only when a queued terrain needs it. The shader tests `!enable_live_terrain` before its per-material cache flag. Thus the all-cached case already removes the live branch; the all-live case can remove the cache branch. A third boolean does not eliminate a newly discovered cost [R5].

The mixed case genuinely needs both behaviors. A single frame-wide constant cannot replace a per-terrain decision unless geometry/material routing changes. Such routing has classification and dispatch costs and would need a new, measured mixed-terrain denominator. It is not an immediate Coastal optimization.

Also, “default off” is not a complete description of effective state. `reconcile_clipmaps` enables caches if either the debug toggle or virtual-texturing ownership requests them, unless the environment forces them off. The default guard test remains meaningful, but does not prove that every default scene executes live terrain [R6]. Future records must log **effective per-terrain state, VT state, and the actual ShadingSpec**, not infer them from an unset environment variable.

### 3.2 Naga's quad restriction is stale for the pinned package

The installed Naga **30.0.1** WGSL frontend lowers `quadSwapX`, `quadSwapY`, `quadSwapDiagonal`, and `quadBroadcast`. Its validator includes `QUAD_FRAGMENT_COMPUTE` under `Capabilities::SUBGROUP`; its SPIR-V backend emits quad operations [T1]. The current WGSL specification also defines quad swaps and `subgroupElect` [L1].

This is a **source-confirmed compiler path**, not a completed device capability test. An implementation still needs the adapter's subgroup operation support, requested wgpu features, valid stage/control flow, and correct pixel-to-lane mapping. Subgroup lanes are not automatically screen neighbors. Nevertheless, “analytic reconstruction is the only door” is false for this pinned source.

The `enable subgroups;` directive remains explicitly unimplemented in Naga's extension parser. `subgroupElect` is absent from the pinned frontend even though it is in the language specification. These are separate questions. The checked v30.0.1 release notes list platform fixes, not a reason to upgrade the engine for this phase [T1, L2]. No claim is made about untested upstream trunk.

### 3.3 Explicit gradients make a port possible, not identical

The existing UV path evaluates perspective-correct barycentrics at neighboring pixels. The same expression can reconstruct world position:

```text
P(x,y) = sum_i bary_i(x,y) * transformed_vertex_position_i
Gx = P(x+1,y).xz - P(x,y).xz
Gy = P(x,y+1).xz - P(x,y).xz
```

Use the existing NDC Y convention and the same morphed/skinned geometry, perspective division, jitter, and viewport dimensions [R7]. These are finite differences of this triangle's interpolant, not automatically the values returned by fragment `dpdx/dpdy`. Primitive boundaries, fine/coarse derivative selection, degenerate triangles and grazing projections matter. Changing gradients may improve correctness, but it changes texture footprints. It requires a separate fidelity decision before any compute speedup can be credited.

## 4. The material cache does not currently preserve the live function

### 4.1 A reproducible mathematical counterexample: normal strength

Live evaluation accumulates weighted **surface gradients** and resolves against the geometric normal. Cache generation starts `n_ts` at `(0,0,1)`, adds weighted tangent normals, then normalizes. The consumer decodes that normal and converts it to a surface gradient [R8–R10].

Choose a flat surface, one surviving layer of weight 1, constant tangent normal `(0.6,0,0.8)`, no cliffs, and no parallax. Ignore quantization and filtering; they are not needed for the counterexample.

```text
Live gradient magnitude       = 0.6 / 0.8 = 0.75
Generated direction           = normalize((0,0,1) + (0.6,0,0.8))
Decoded cache gradient        = 0.6 / 1.8 = 1/3
Live resolved world normal    = (-0.6, 0.8, 0)
Cached resolved world normal  = (-1/sqrt(10), 3/sqrt(10), 0)
Angular disagreement          = 18.43495 degrees
```

The generator attenuates even a single layer's bump. Starting the accumulator at zero fixes that single-layer bias but **does not** make weighted normal blending equal weighted gradient blending for multiple layers. The correct intermediate must reproduce the live gradient combination, including its `max(n.z, 0.2)` rule. Mikkelsen's surface-gradient framework supplies the relevant composition model [L3]; the numerical counterexample above is derived from Somnium's code.

This demonstrates non-equivalence. It does **not** establish the cause of the historic rectangular dark band, whose recorded addressing/readiness/uniform-slot defects are distinct.

### 4.2 The cache key omits an input that changes during play

Generation bakes wetness into albedo, roughness, and alpha. The consumer reads that baked wetness for F0. Weather writes `terrain.wetness` every frame, whereas `TerrainClipmap::update` receives camera position and `edit_revision`; its full refresh decision uses initialization/revision. The weather assignment does not bump that revision [R8, R10–R12].

Therefore, a ready stationary cache can retain old wetness until another event regenerates it. This is a statically identified dependency gap, not a reproduced screenshot finding. Paint/sculpt revisions and VT-arrival invalidation already exist and should be reused; a second general cache manager is unnecessary.

Refreshing the entire cache whenever weather changes would restore freshness at the expense of the reuse that made the cache fast. The architectural question is whether to cache **dry material plus moisture response**, applying current weather after sampling. That is not a one-line move: live terrain combines cliff material and moisture before wetness, while the cached path currently mixes projected cliffs after the baked response. Factor the actual equations in their existing order and test them; do not assume the operations commute.

### 4.3 Representation and coverage impose additional limits

The cache stores two `Rgba8Unorm` targets, including packed normal XY, roughness, AO and wetness. Live output has not undergone that cache quantization. Clipmap generation and live shading also use different sampling footprints; strongest-four selection and height blending are nonlinear. In general:

```text
filter(material_evaluation(inputs)) != material_evaluation(filter(inputs))
```

Consequently a corrected cache cannot promise general bit identity merely by sharing helper functions. Preserve the existing formats initially and measure their error; format expansion is a separate decision with a memory budget [R8, R9, R13].

POM is another explicit exception: the cache consumer sets `parallax_shadow = 1`, while the live path can march and shadow it [R8, R10]. A cache setting must not silently discard authored POM. Cliff evaluation already exists in the consumer and must be retained.

Finally, exhausted update budgets can intentionally make a ready ring fall back to a coarser ring; complete misses can become constant material [R10, R13]. Those behaviors prevent invalid reads but do not satisfy a no-detail-loss requirement. A warm stationary luminance comparison misses all of them. PORTAL-0 itself limits its evidence to three stationary viewpoints and mean luminance ([PORTAL-0 §F](phase_PORTAL-0.md)).

## 5. Proposed architecture

### 5.1 Cache eligibility follows proof and actual content

Keep the existing clipmap resources, queue order, and shader registry. Add a small renderer-owned companion, provisionally `terrain/cache_contract.rs`, only after the first fixture establishes its needed inputs. Its output should be a decision such as:

```text
MaterialCacheDecision
  effective_mode: Live | Cached | Mixed
  reason: unsupported feature | stale dependency | incomplete coverage | eligible
  content_epoch: u64
```

This is proposed CPU metadata, **not an addition to the 2,080-byte GPU material ABI**. The renderer uses the decision consistently when uploading `clipmap_enabled` and selecting existing `ShadingSpec` fields. Begin conservatively at whole-terrain granularity. Per-pixel eligibility would retain both costly shader paths and needs its own occupancy experiment.

Classify dependencies by semantics:

| Dependency | Correct response |
|---|---|
| Source maps, tiling, painted weights/noise, height-blend parameters | Invalidate affected cached content; use existing revision/resource events where sufficient |
| New VT source page | Recompose affected results; current full invalidation is the starting behavior |
| Camera movement / toroidal remap | Existing dirty rectangles, guards, and readiness; verify producer-before-consumer ordering |
| Weather / wetness | Initially ineligible if stale; consider a proven dry-material factorization instead of repeated full rebakes |
| View-dependent POM | Live route until an independently equivalent implementation exists |
| Light/camera parameters used only after material evaluation | Do not invalidate material content |

Selection must never describe a cache as eligible before generation is ordered ahead of its consumers. “Recorded for this frame” and “completed on the GPU” need not require a CPU wait: same-queue ordering is sufficient. Multiple view recordings must preserve distinct uniforms and content interpretation; the per-frame slot fix is a precedent, not proof for arbitrary multi-view reuse.

No new global scene bind group is needed. A fixture may use a private layout. Note that the existing standalone ocean compute shader already has its own `@group(0)` layout; the scene-global convention does not mean every shader binds the scene pool [R14].

### 5.2 An independent ocean experiment: own a butterfly pair

The ocean already runs a shared-memory Stockham transform. Proposing “move FFT into workgroup memory” or “avoid a dispatch per FFT stage” would rebuild existing code. Each row uses 256 invocations, 1,024 complex values and a 16 KiB ping-pong array. The scheduler processes all three cascades on an update; comments claiming one cascade per frame are stale [R14–R15].

At each radix-2 stage, the precomputed entries for outputs `w0` and `w1` contain identical input indices and opposite twiddles. Today separate output work reads those inputs and computes their products twice:

```text
y0 = upper + complex_mul(lower, twiddle)
y1 = upper + complex_mul(lower, -twiddle)
```

Propose a companion `shaders/water_fft_pair.wgsl`: each invocation owns two butterfly pairs per stage at N=1,024. Read one pair's factors and inputs once; write its two distinct outputs. Preserve the stage barriers, row load/store layout, transform sign and existing transpose orientation. Reuse the current butterfly buffer and scratch allocation. The experiment needs no subgroup feature, new texture, or precision reduction.

First preserve the two signed products explicitly. Only share the complex product as `p` and `-p` if device tests verify equivalence: floating-point contraction, signed zero and reassociation make algebra alone insufficient for byte parity. Mapping both outputs together can still reduce repeated loads without that second change.

**Exact integration seam:** create the companion pipeline in `WaterSpectrumPass`; choose it at the two existing row dispatches, lines 428–438. Reuse group 0 binding 1 (FFT read/write storage), binding 2 (butterfly storage) and binding 3 (80-byte uniform, 256-byte dynamic stride). Keep the layout's other entries unchanged. Register the new WGSL once in `shaders.rs`. A launch-time experiment selector chooses the baseline unless explicitly enabled [R14].

This preserves all three 1,024² cascades, the fixed 50 Hz update policy, tick catch-up, normal/foam output and rendering consumers. It is an original kernel ownership change informed by Stockham/shared-memory FFT literature, not a port of a CUDA implementation [L4].

**Payoff model:** `net_saving = updated_frame_fraction × row_FFT_saving − overhead`. The water prepass is 10.9% of DOOM-M Coastal and 17.0% of Island; its row-transform share is not separately recorded. A 10% whole-prepass reduction would mean about 0.22 ms, or 1.1% / 1.7% of those frames. That is a useful target, **not evidence that pairing will achieve it**. Two products instead of one, longer register lifetimes, or compiler reuse of existing loads can erase the gain.

## 6. Lettered sub-phases and stopping rules

### TALOS-A — make material equivalence testable

**Deliverable:** A small fixture using composed production WGSL and synthetic constant/patterned inputs. Evaluate live and generated/decoded material from the same immutable inputs; compare albedo, normal, roughness, AO, wetness/F0 and POM shadow separately. An offscreen device test is preferable to repeatedly launching the editor. Use full-precision diagnostic outputs so target quantization cannot hide algebraic errors; also compare actual cache-format outputs.

Start with the single-layer counterexample, then unequal-normal two-layer blends, four-way height blends, zero strengths, wetness endpoints, steep cliffs, and multiple footprints. Pair scalar analytical checks with execution of real shaders; source-order assertions alone do not prove the math.

**Exit:** The current shader produces the predicted single-layer discrepancy, the live oracle matches the analytic case, and repeated fixture evaluation is deterministic. The fixture must fail against the known mismatch. Existing default-off tests remain unchanged.

**Falsifier:** If actual shader output disagrees with the counterexample, resolve the setup/coordinate discrepancy before altering code. If deterministic comparison cannot be established, later parity claims stay blocked. A is a correctness gate; it claims no frame saving.

### TALOS-B — correct the cached normal algebra

**Deliverable:** An original companion helper that forms the same weighted gradient as live terrain and encodes a compatible normal for the existing cache consumer. Hook it only into generation; retain existing packing initially. Merely deleting the initial flat normal is insufficient.

**Exit:** Single- and multi-layer gradient tests pass within a predeclared numerical tolerance before encoding; cache-format error is reported separately. Verify normal variance and downstream specular response, not just average colour. A targeted existing-file hook is justified by the proven discrepancy and should be reviewed as such.

**Falsifier:** If the packed representation loses required gradient range/variance, stop the default recommendation. Do not quietly widen formats, flatten normals, or call the quantization invisible. Specify the smallest additional representation only in a subsequent decision. This fixes fidelity on the opt-in rail; the historic 9.1 ms frame is not guaranteed afterward.

### TALOS-C — close cache dependencies and eligibility

**Deliverable:** The minimal contract in §5.1, a dependency audit covering actual writers, and replay fixtures for cold start, diagonal ring crossing, teleport, paint/undo, weather changes, VT arrival and alternating views. Use current ready masks, pending-texel counters and slot allocator. Add only information they cannot express.

**Exit:** No stale dependency may be sampled as current. Unsupported POM and unavailable fine coverage retain the correct live behavior. Produce a matrix stating exactly which scene/feature combinations pass structural and motion parity, their effective cache state and their timing. Recommend a default only for a demonstrated supported configuration; do not flip it in this phase.

**Falsifier:** If maintaining fidelity requires continuous rebakes, excessive live fallback, or a mixed shader that removes the speed advantage, reject general default-on. Keep the live rail and report the constrained cache result. An audit is successful even when the old speedup cannot be retained under the stronger quality contract.

### TALOS-D — paired-output FFT, one candidate

**Deliverable:** The complete companion kernel and selector described in §5.2, plus a device test comparing the same input row under both pipelines. Use impulse, DC, conjugate pairs, random finite data and the real spectrum; include the full transform/unpack chain and a matched foam history sequence. Preserve the existing transposed output convention even if its explanatory comment is questionable.

**Exit:** No material numerical/image regression; no new GPU allocation; at least **1% matched total GPU-frame reduction** on the admitted primary rail, beyond observed paired-run variation, with no significant regression on the other map/rail. Time row transforms and the whole prepass so movement in the combined scope can be attributed. Recording brackets are part of the completed candidate before timing it.

**Falsifier:** Exact results fail, the paired owner increases stalls/register pressure, or the full-frame gain is below the gate. Stop after this one mapping; retain a measured null record and remove the experimental runtime path. A matrix of workgroup sizes, radices and fused passes is not authorized by this proposal.

D is independent of B/C's cache outcome. If the cache cannot meet the invariant, D remains the first bounded performance implementation worth considering.

## 7. Techniques deliberately not scheduled

| Technique | Decision and reopening condition |
|---|---|
| Terrain metadata prepass | Deferred. It moves the same scan into another invocation and adds metadata traffic. Four exact weights plus packed indices are at least 20 B/pixel before other outputs: about 37.8 MiB at 1920×1032, written and read each frame. Reopen only with an occupancy/duplication model that repays the entire extra pass on the relevant rail. |
| Whole-resolve compute port | Technically less blocked than the brief says, but too large for the first experiment. Reopen after gradient parity and a measured live-terrain shader bottleneck; quad availability alone is not a speedup. |
| New staging ring | Defer until Island churn is traced to its owner and correlated with a hitch. Texture-view/bind-group churn cannot be fixed by an upload allocator. If warranted, wrap wgpu 30.0.1's existing `StagingBelt`; it already handles suballocation, unmapping and remapping after submission. Two persistently mapped buffers are not automatically safe for arbitrary GPU latency [L5]. |
| SIMD CPU culling | Retain the existing decision. Portable SIMD is not stable on the pinned toolchain; `Vec3A` is not eight-object SIMD. The recorded terrain CPU scope is 0.034 ms [R1, line 52], below the frame admission threshold even if entirely removed. |
| Shared bilateral denoiser / fewer rays | No parity-preserving evidence. Reflection and GI have different guides/history and estimators. Reducing samples needs quality and temporal validation; it does not reduce spectral FFT work. |
| fp16 / texture compression / neural materials | No demonstrated present bottleneck that justifies their representation error and integration surface. DOOM's nulls remain relevant; any renewed experiment needs its own reason and oracle. |
| Shadow/triangle-filtering redesign | No priority on the static DOOM-M workload. Reopen with geometry-heavy, moving-caster evidence, not the obsolete 0.937 ms row. |

## 8. Literature audit and what it actually supports

The selected sources answer mechanism questions; their benchmark numbers are not transferred to Somnium.

- **Clipmaps:** Asirvatham and Hoppe describe nested, toroidally addressed windows and incremental updates. That supports amortization and border/transition audits, not equivalence of a nonlinear shaded-material cache to per-pixel evaluation [L6].
- **Normal composition:** Mikkelsen's surface-gradient framework supports composition in a common gradient representation. TALOS uses it to check the representation Somnium already chose for live terrain [L3].
- **FFT execution:** Govindaraju et al. describe shared-memory Stockham FFTs and data-layout/transpose tradeoffs. Somnium already implements the broad pattern; the proposed paired ownership is a smaller local hypothesis [L4].
- **Occupancy:** NVIDIA's shader guidance ties register allocation to occupancy/spilling; AMD's occupancy guide distinguishes several resource limits. These do not mean every runtime branch executes both sides or that higher occupancy always wins. Describe union-of-path resource pressure as a compiler/hardware hypothesis until generated-code or profiler evidence establishes it [L7–L8].
- **Blackwell-specific evidence:** Nsight Graphics documents Blackwell compute hardware-event tracing; NVIDIA's RTX 50 tools article describes expanded counters. If a completed candidate changes timing unexpectedly, a short bounded trace can distinguish register/shared-memory/latency constraints. Do not import data-center Blackwell or AMD register budgets as RTX 5080 Laptop limits [L9–L10].
- **2026 developments:** The SIGGRAPH 2026 course abstracts include ORCA's within-frame radiance cache and variable-rate ray tracing with disocclusion-aware ray allocation. Those are relevant future GI directions, but change a much larger estimator/reconstruction system than this phase. I reviewed the published abstracts, not their large slide decks, and derive no algorithm or speedup claim from them [L11].

No reference-engine source was transcribed or imported. The reference mirror was not exhaustively audited for this document. Repository attribution rules (§14–15) were read; any later adoption must inspect the exact reference file's license and cite the mechanism used. No NRD/RTX SDK licensing assumption is needed for these proposals.

## 9. Measurement and fidelity contract

**This document runs no engine benchmark.** Existing records are sufficient to select hypotheses. Device correctness fixtures and post-implementation timing belong to the implementation phase.

1. **Identity first.** Record revision, binary/profile, adapter/driver, actual render dimensions, map/view, sun, camera matrices, simulation tick, random seeds, effective clipmap/VT state, ShadingSpec, draw/triangle counts, and scene/resource residency. An environment label alone is insufficient.
2. **Separate tests by purpose.** Compare material outputs before lighting for algebra, decoded cache outputs for quantization, full HDR/display images for appearance, and sequences for motion/history. Component tests do not replace integrated fidelity checks.
3. **Establish self-agreement.** DOOM-I reports 2.80% differing pixels between unchanged-build captures, far above the 0.2% gate [R16]. Use identical immutable inputs and independent cloned temporal state for paired evaluation. Do not let one candidate advance foam, reservoirs, VT admission or exposure before the other. Resolve nondeterminism before evaluating a 0.2% claim; never raise the threshold to match the noise.
4. **Golden gate.** Retain the repository's 0.2% failing-pixel budget and peak channel ceiling of 24 with its existing comparator. Record maximum localized errors and normal/specular behavior too: a thin dark band can be serious even if its global pixel fraction is small. General bit identity and perceptual acceptance are different claims. The known sculpt-panel UI failure is reported separately, never used to excuse terrain or water failures.
5. **Timing session.** After a complete candidate passes correctness, prepare one bounded editor session with baseline and candidate pipelines precreated. Prefer planned in-process A/B windows to repeated startup; this window-switching/reset support is **proposed harness work**, not an existing `SOMNIUM_*` switch. Warm each rail twice, then collect at least two back-to-back paired 180/300 windows with reversed order. Release temporary captures/resources between windows; stop on memory growth. Do not run an unattended process-launch loop.
6. **State-dependent cost.** Preserve the 50 Hz ocean schedule. Record updates, skipped frames and catch-up ticks; report both update-frame FFT cost and wall-time-weighted prepass cost. Faster rendering changes the fraction of frames that simulate, so per-frame averages alone can mislead. Cache timing must include cold fills, edits and motion, as well as warm steady state.
7. **Controls and uncertainty.** Report individual paired deltas, within-run variance and between-pair spread. Require stable untargeted work counts and explain timing movement in control passes. Requiring unrelated GPU timings to match to the third decimal is not a physically credible universal rule—the historical clipmap pair itself violates it. If all passes move together, attribution is inconclusive until the confound is resolved. Do not use within-run sigma as a confidence interval for sessions with different thermal/power state.
8. **Validation after code.** Run `cargo test --workspace -j 1`, then `python tools/ghostfence/run.py`, reporting PASS/FAIL/SKIP exactly. GPU-free shader validation does not prove bind layouts or engine startup. Perform the device fixture and the deliberate editor acceptance session before claiming end-to-end success. Do not weaken the clipmap default guard or golden thresholds.

## 10. Final recommendation and scope boundary

Implement **TALOS-A first**, then the smallest TALOS-B correction the fixture supports. The counterexample is concrete enough to make that work reviewable. Treat cache dependency/coverage closure as the condition for recommending broader reuse; reject unsupported configurations rather than advertising the historical 7× shading ratio as a general solution.

For new throughput work, admit **one paired-output ocean FFT candidate**. Its denominator exists on both maps and either cache setting, its data is deterministic, and it preserves the current simulation and visual model. Its practical expected outcome is either a modest measured win or a cheap, well-explained null.

No runtime file, shader, default, dependency, or golden reference was changed while preparing this document. No subagents were launched. The research skill's primary-source/citation workflow was used directly; its delegation step was overridden by the user's explicit single-agent requirement.

## 11. Evidence index

Repository links are relative for portability. Line numbers refer to `3c4e33a`; historical measurements retain their own recorded dates and conditions.

| ID | Source and exact locator |
|---|---|
| R1 | [PORTAL-0 final Coastal ground](<phase PORTAL-0/PORTAL-0-final_coastal-ground.somtime>), header lines 1–7; GPU lines 19, 28, 30–31, 41–43; CPU lines 52–55 |
| R2 | [PORTAL-0 final Coastal ground, clipmap](<phase PORTAL-0/PORTAL-0-final_coastal-ground_clipmap.somtime>), same row locations |
| R3 | [DOOM-M Coastal final](<phase DOOM/DOOM-M_coastal-ground_final.somtime>), lines 19, 28, 30–31, 41 |
| R4 | [DOOM-M Island final](<phase DOOM/DOOM-M_island-ground_final.somtime>), lines 19, 28, 30–31, 42 |
| R5 | [renderer.rs](../crates/somnium_renderer/src/renderer.rs), lines 4377–4429; [shading.wgsl](../crates/somnium_renderer/src/shaders/shading.wgsl), lines 1634–1648; [pass/shading.rs](../crates/somnium_renderer/src/pass/shading.rs), lines 14–98 and test at 1581 |
| R6 | [renderer.rs](../crates/somnium_renderer/src/renderer.rs), lines 605–636; [default guard](../crates/somnium_renderer/tests/shaders_validate.rs), line 413 |
| R7 | [shading.wgsl](../crates/somnium_renderer/src/shaders/shading.wgsl), `vis_barycentric` line 354; positions 1329–1331; gradients 1358–1365; hit point 1389; world derivatives 1630–1631 |
| R8 | [terrain_material.wgsl](../crates/somnium_renderer/src/shaders/terrain_material.wgsl), gradient helpers 222–229; generation 1222–1331; live blend 1476–1488; POM 1397–1426; wetness 1520–1554 |
| R9 | [clipmap_gen.wgsl](../crates/somnium_renderer/src/shaders/clipmap_gen.wgsl), generation/encoding lines 63–100 |
| R10 | [clipmap_shade.wgsl](../crates/somnium_renderer/src/shaders/clipmap_shade.wgsl), lines 225–363 |
| R11 | [app.rs](../crates/somnium_core/src/app.rs), weather assignment 6655–6673; terrain slider assignments 9529–9568 |
| R12 | [renderer.rs](../crates/somnium_renderer/src/renderer.rs), update inputs 3610–3635; VT arrival invalidation 4290–4295 |
| R13 | [terrain/clipmap.rs](../crates/somnium_renderer/src/terrain/clipmap.rs), format 68; revision gate 341–353; budget/readiness 378–450; GPU flags 487–516 |
| R14 | [pass/water_spectrum.rs](../crates/somnium_renderer/src/pass/water_spectrum.rs), parameters 9–46; tick schedule 310–344; cascade dispatches 416–442; layout binding resources 535–570 |
| R15 | [water_spectrum.wgsl](../crates/somnium_renderer/src/shaders/water_spectrum.wgsl), bindings 61–67; paired outputs 237–255; row transform 258–300; transpose 302–328; unpack/foam 333–378 |
| R16 | [DOOM-I correctness](<phase DOOM/DOOM-I.md>), paragraph beginning “A tone-mapped capture cannot settle this”; [PORTAL-0](phase_PORTAL-0.md), §F and Gates |

**T1 — locally inspected dependency source:** under `C:/Users/adhir/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`: `naga-30.0.1/src/front/wgsl/lower/mod.rs:1232,3693,3716,3739`; `src/valid/mod.rs:291,596`; `src/back/spv/subgroup.rs:205`; `src/front/wgsl/parse/directive/enable_extension.rs:166`. Source inspection establishes implemented paths; no fresh compiler/device probe was executed.

| ID | Primary literature / official documentation, checked 2026-09-05 |
|---|---|
| L1 | [W3C WGSL](https://www.w3.org/TR/WGSL/), §17.12.7 `subgroupElect`, §17.13 quad operations, synchronization/memory semantics. Language specification is distinct from implementation support. |
| L2 | [wgpu v30.0.1 release notes](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.1) |
| L3 | Morten S. Mikkelsen, [Surface Gradient–Based Bump Mapping Framework](https://jcgt.org/published/0009/03/04/paper-lowres.pdf), JCGT 9(3), 2020, especially surface-gradient formulation and composition |
| L4 | Govindaraju, Lloyd, Dotsenko, Smith and Manferdelli, [High Performance Discrete Fourier Transforms on Graphics Processors](https://www.microsoft.com/en-us/research/publication/high-performance-discrete-fourier-transforms-on-graphics-processors/), SC 2008; publication summary reviewed |
| L5 | [wgpu 30.0.1 StagingBelt](https://docs.rs/wgpu/30.0.1/wgpu/util/struct.StagingBelt.html); local `wgpu-30.0.1/src/util/belt.rs`, especially lifecycle and `finish_and_recall_on_submit` |
| L6 | Asirvatham and Hoppe, [Terrain Rendering Using GPU-Based Geometry Clipmaps](https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry), GPU Gems 2, chapter 2, 2005 |
| L7 | NVIDIA, [Advanced API Performance: Shaders](https://developer.nvidia.com/blog/advanced-api-performance-shaders/), register allocation/spilling guidance |
| L8 | François Guthmann / AMD, [Occupancy explained](https://gpuopen.com/learn/occupancy-explained/), updated 2024; architectural concepts, not NVIDIA numerical limits |
| L9 | NVIDIA, [Nsight Graphics GPU Trace overview](https://docs.nvidia.com/nsight-graphics/UserGuide/gpu-trace-overview.html), hardware events and bounded trace memory |
| L10 | NVIDIA, [Nsight tools on GeForce RTX 50](https://developer.nvidia.com/blog/build-apps-with-neural-rendering-using-nvidia-nsight-developer-tools-on-geforce-rtx-50-series-gpus/), Blackwell counter capabilities |
| L11 | [SIGGRAPH 2026 Advances course](https://advances.realtimerendering.com/s2026/index.html), ORCA and Variable Rate Ray Tracing abstracts; scoped literature update, not an implementation reference |
