# Phase DOOM — id Tech

> *id Tech's reputation was never one clever trick. It was refusing to spend a
> cycle on anything the player could not see, and measuring before believing.*

> **Codename:** DOOM (id Tech 6/7/8)
> **Status:** PLAN (2026-08-16). Nothing in the tree.
> **Predecessor:** Phase CR (Crysis) established *where* the frame goes.
> CR-A's verdict — **GPU-bound, shading dominates** — is this phase's premise.
> **Record:** this file. Evidence folder `dev records/phase DOOM/` is created by
> DOOM-A, not before (do not invent PNGs or timing files).
> **Do not copy source** from id Tech, UE5, Wicked, CryEngine or Frostbite.
> Patterns are cited in `ATTRIBUTION.md` §13C.

**Frozen by this phase** (a DOOM sub-phase that changes any of these has gone
wrong): Great Lakes water datum 16.1 m / optical 18.6 m / Gerstner 0.85; the XV
32-layer look and `GpuTerrainMaterial` layout; foliage LOD and cull distances;
the Island recipe (signed off); clipmap inspector default **off** (Phase DF owns
that decision); no per-pixel terrain sample-count LOD; rustc 1.88; wgpu 29
single queue.

**The fidelity rule, stated once:** every stage below is judged by
`capture.rs` — the HDR target read back at a fixed frame index, before tone
mapping and TAA. A stage that claims "no visual change" must prove it with a
byte-comparable capture, and a stage that *does* change pixels must say in
advance which pixels and why. "Looks the same to me" is not an exit criterion in
this phase.

---

## 1. Executive decision

Somnium is GPU-bound on one pass. CR-A measured **~50.8 ms of shading inside a
71.4 ms frame** at maximized Native on an RTX 5080 Laptop; the 2026-08-14
occupancy session measured **Shading ~41.2 ms of ~52.8 ms** on Island before the
compact PSO, and **~20 fps on the ground in Coastal** after it. Everything else
in the frame — culling, instance build, terrain LOD, physics, UI — is
microseconds by comparison.

That session also proved the mechanism, and it is the single most important fact
in this plan:

> Runtime uniforms do **not** delete WGSL from the compiled module. Occupancy is
> the union of every path still in the shader. Flipping Hex / Parallax / Soft
> Shadows off changed nothing until `ShadingSpec` turned them into pipeline
> `override`s and compiled a smaller shader.

`ShadingSpec` fixed that **per frame, for the whole screen**. But a Coastal
ground frame contains sky, water, foliage, meshes and terrain at every distance,
and one PSO must be wide enough for the most expensive of them. Every sky pixel
in that frame is currently scheduled with the register budget of a 32-layer
terrain pixel that may also hex-tile and parallax-march.

**Phase DOOM's thesis: take `ShadingSpec` from per-frame to per-tile.** Classify
the visibility buffer into 8×8 tiles, bin those tiles by what they actually
contain, and run one indirect compute dispatch per bin against a specialization
of the shading shader that contains only that bin's code. This is what Wicked
Engine, UE5 Nanite and id Tech 8 all do, arrived at independently, and it is the
natural completion of the visibility-buffer architecture Somnium already
committed to in §6.1 of `context.md`.

Everything else in this phase is either (a) the measurement that makes the claim
falsifiable, (b) removing work the classification cannot remove — shadows,
pixels, draws — or (c) spending the idle CPU and RAM on work that currently
sits on the frame's critical path.

---

## 2. Goals

1. **Coastal, standing on the ground, maximized Native: 60 fps with the current
   image.** Today ~20. This is the headline and it is deliberately hard.
2. **Shading ms falls proportional to what a tile actually contains**, not to
   the worst material in the scene.
3. **Shadows cost nothing when nothing moved.**
4. **A frame's cost is knowable.** The profiler's `unattributed` row drops below
   5% of frame time, and a timing run is reproducible enough to A/B a 3% change.
5. **The idle cores do useful work** — parallel encode, background streaming,
   off-critical-path bakes — measured by frame time and hitch count, never by
   Task Manager percentage.
6. **No visual regression anywhere.** Not "close enough". Captures.

---

## 3. Non-goals

- **Async compute / multi-queue.** wgpu 29 exposes one queue. id Tech 8 saves
  ~0.5 ms this way and we cannot. Do not fake it with a second `Device`.
- **Copying source** from id Tech, UE5, Wicked, CryEngine, Frostbite. Patterns
  only, cited.
- **Nanite, geometry clipmaps, or rewriting the 25C morph.**
- **Turning terrain Clipmap default on.** DF-E owns that gate and it needs a
  live remeasure the DOOM sessions must not quietly perform instead.
- **Retuning the look** — water numbers, XV 32-layer format, foliage LOD, Island
  recipe, unique colour. If a DOOM stage improves fps by changing what the
  renderer draws, it has failed.
- **Per-pixel `close` / `use_maps` / `layer_budget` branching.** XV-Zeta §11.1
  measured walking go 20 → 27 ms. The answer to "different pixels need different
  work" in this phase is a **different dispatch**, never a different branch.
- **Raising CPU utilisation as an objective.** CR-A: "Inflating % on 256 chunks
  is a failure." Frame time is the score.
- **Mesh shaders.** `EXPERIMENTAL_MESH_SHADER` exists in wgpu 29 and is
  research-only here; 15D–15F already give per-cluster granularity through
  indirect draws.

---

## 4. What the frame actually costs today

Consolidated from `phase CR/CR-A_occupancy.md`, `phase DF/DF-A_timings.md`,
`terrain_shading_occupancy_2026-08-14.md` and `phase_DF.md` §4. **All of it is
from before the DF audit fixes**, which is itself a reason DOOM-A exists.

| Measurement | Value | Source |
|---|---:|---|
| Coastal, Native max, clipmap off — Shading | **50.838 ms** | DF-A overview |
| Coastal, Native max, clipmap off — frame | **71.376 ms** | DF-A overview |
| Island before compact PSO — Shading / frame | **41.2 / 52.8 ms** | 2026-08-14 |
| Island after compact PSO | **30+ fps** | 2026-08-14 |
| Coastal on the ground, Hex/POM/PCSS off | **~20 fps** | 2026-08-14 |
| Task Manager CPU on `hello_engine` | **~1.5%** | CR-A |
| Working set | **~553 MB** | CR-A |
| Profiler `unattributed` (debug, 720p) | **0.317 of 2.354 ms (13%)** | §17.7 |

Per visible terrain pixel today (`terrain_material.wgsl`, `phase_DF.md` §4):
8 splatmap `textureSampleGrad`, a 32-wide unpack loop, a 32-wide strongest-four
scan, then four layers × (albedo + surface) — ×3 again if hex tiling is on — plus
macro, projected maps, unique colour, and optionally a ~24-step POM march with
self-shadowing. Coastal runs the 32-wide scan because it publishes the extra
bank; Island runs 16.

**Known blind spots, which DOOM-A closes before anything else is believed:**

- The profiler does not bracket culling, the second visibility phase, ReSTIR
  DI/GI setup, IBL, or the editor overlays (§17.7).
- There is no counter for *how many pixels* are terrain vs mesh vs sky vs water.
  Every conclusion about Coastal so far has been inferred from "almost every
  pixel is ground."
- CR-A's GPU numbers are second-hand from DF-A, and DF-A's look is stale
  (`phase_DF.md` §12).
- Screen-capture frame deltas are useless here: 0.776 → 2.018 across three runs
  of one identical build (§18). `capture.rs` exists precisely because of this;
  DOOM needs its timing equivalent.

---

## 5. Repository and literature audit

Verified 2026-08-16 against the local trees and public material. No engine
source was copied. Local paths are given so the next session can re-read rather
than re-search.

### 5.1 Already in Somnium — do not rebuild

| Piece | Where | Use in DOOM |
|---|---|---|
| Compact shading PSO via WGSL `override` | `pass/shading.rs` `ShadingSpec` / `ensure_pipeline` | **The** foundation of DOOM-C. Per-tile specs are more of these. |
| GPU timestamps + counters, deferred readback | `profiler.rs` | DOOM-A extends it; does not replace it. |
| Deterministic HDR capture + A/B compare | `capture.rs` | The fidelity gate for every stage. |
| Indirect draw, GPU frustum cull, Hi-Z, meshlets, per-cluster args | `indirect.rs`, `pass/cull.rs`, `pass/hiz.rs`, `meshlet.rs` (15A–15F) | DOOM-G builds on the arg buffer; keeps or explicitly replaces the "arg *i* is instance *i*" invariant. |
| CPU frustum cull + cascade caster cull + persistent buffers | Phase CR (CR-B/E/F) | Unchanged. DOOM-H parallelises it, does not redesign it. |
| Clustered local lights (counting sort into flat buffers) | `cluster.rs` (13C) | Already the right shape; DOOM-L may add subgroup reductions. |
| Resolution presets + FSR 3 | `viewport_resolution.rs`, `pass/fsr.rs` | DOOM-F makes the scale continuous and automatic. |
| Terrain material clipmap cache | `terrain/clipmap.rs` (Phase DF) | The designed cheap terrain path. **DF owns it.** DOOM-E is the non-clipmap fallback and must not pre-empt DF-E. |
| Rayon helper at 512+ items | `jobs.rs` (CR-D) | DOOM-H replaces the single helper with a real pool. |

### 5.2 Wicked Engine — visibility tile binning (primary architecture for DOOM-C)

**Local:** `example_repo/New_Engines/WickedEngine-master/WickedEngine/shaders/`
`visibility_analyzeCS.hlsl`, `visibility_shadeCS.hlsl`, `visibility_surfaceCS.hlsl`,
and `ShaderInterop_Renderer.h`.

This is the closest working analogue to what Somnium needs, in an engine with
the same overall shape (bindless, visibility buffer, compute shading):

- `VISIBILITY_BLOCKSIZE = 8` — 8×8 pixel tiles.
- `SHADERTYPE_BIN_COUNT = 12` — tiles are binned by shader type, and each bin
  gets its own indirect dispatch of `visibility_shadeCS`.
- `visibility_analyzeCS` additionally detects **primitive-uniform tiles** with
  `WaveActiveAllTrue` + one `InterlockedAnd` per wave, and routes them to a
  `PRIMITIVEID_UNIFORM` permutation that reads the primitive from the tile
  record instead of per-pixel from the texture — the whole triangle setup
  becomes scalar for that tile.
- Tiles are dispatched as a flat 1-D group with `remap_lane_quads(groupIndex)`
  restoring 2×2 quad adjacency, so `QuadReadAcrossX/Y` still works for
  derivatives in compute.

The last point matters more than it looks: Somnium's shading pass is a
**fragment** shader and gets quad derivatives free. Moving to compute means
either analytic gradients (25N already computes them for the terrain path) or an
explicit quad-swizzle. This is the main technical risk in DOOM-C and §10 records
it as such.

### 5.3 UE5 — Nanite shade binning

**Local:** `example_repo/UnrealEngine-release/.../Engine/Shaders/Private/Nanite/NaniteShadeBinning.usf`
(and `NaniteShadeCommon.ush`, `Renderer/Private/Nanite/NaniteShading.cpp`).

Same idea at production scale, and it contributes two details Wicked does not:

- Binning is **four passes** — `COUNT`, `RESERVE`, `SCATTER`, `VALIDATE` — rather
  than one atomic append. Counting first, reserving contiguous ranges, then
  scattering gives deterministic tile ordering, which matters when you want two
  runs of the same build to produce the same capture.
- `SHADING_BIN_TILE_SIZE_BITS` is 3 or 5 depending on `BINNING_TECHNIQUE`: the
  tile size is a *tuned* parameter, not 8 by revelation. Somnium should measure
  8 vs 16 vs 32 rather than assume.
- Explicit `INVALID_BIN0..3` sentinels per pixel of a quad, so a coarse-rate
  quad and an empty quad are distinguishable. Somnium's `vis_data == 0u` sky
  sentinel already gives half of this.

### 5.4 id Tech 8 — Sousa, SIGGRAPH 2025, *FAST AS HELL*

Public slides: `advances.realtimerendering.com/s2025/content/SOUSA_SIGGRAPH_2025_Final.pdf`.
Not about our bottleneck (it is a GI talk) but three transferable patterns:

- **Async indirect dispatch per shader type** to update the world radiance
  cache — the same bin-then-dispatch-indirect structure as 5.2/5.3, applied to
  cache entries rather than screen tiles. Confirms the pattern is the industry's
  general answer to "different pixels need different shaders."
- **Interleaved updates**: one irradiance cascade and one local volume per
  frame, never all of them. Somnium's four shadow cascades are the same shape of
  problem (DOOM-D).
- **Cost table honesty**: every technique reported per platform, serial and
  async, with the note that async saved ~0.5 ms. This is the format DOOM-A's
  timing harness should emit.
- Also confirmed: `RGB9E5` over `R11G11B10F` for intermediate colour (no green
  shift, no loss of pure white) — relevant to DOOM-J.

### 5.5 id Tech 7 — Geffroy, SIGGRAPH 2020, *Rendering the Hellscape of DOOM Eternal*

Public slides: `advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf`
(too large to fetch inline; read from the browser if needed). The relevant
takeaway is the **hybrid cluster + tile binning** rework: pure clustered binning
put distant small lights and decals into single large clusters, and the fix was
to hybridise rather than to increase resolution everywhere. Somnium's `cluster.rs`
is pure froxel clustering and will hit the same wall if local light counts grow —
noted as a future item, **not** scheduled in DOOM.

### 5.6 The Forge — the filtered and culled visibility buffer

**Local:** `example_repo/The-Forge-master/Common_3/Renderer/VisibilityBuffer2/`
(`VisibilityBuffer2.cpp`, `Shaders/FSL/TriangleFiltering.h.fsl`,
`VisibilityBufferShadingUtilities.h.fsl`) and the `VisibilityBuffer` sibling.

The Forge is already Somnium's cited source for the visibility buffer itself
(ATTRIBUTION §2). What has *not* been taken is **triangle filtering**: a compute
pre-pass that rejects back-facing, degenerate, small-primitive and frustum-external
triangles into a filtered index buffer *before* rasterization, for several views
(camera + shadow cascades) at once. Somnium culls at instance and cluster
granularity (15B/15D/15E) and never below.

Scheduled as **DOOM-G2, research-and-measure only**: the visibility pass already
measures 0.053 ms in the §17.7 table, so triangle filtering has almost nothing to
win *there*. Its real value would be in the shadow pass across four cascades,
which is why it appears under DOOM-D rather than as a headline.

### 5.7 Wihlidal, GDC 2016 — *Optimizing the Graphics Pipeline with Compute*

The origin of 5.6's approach (Frostbite): cluster triangles by spatial coherence,
compute an optimal bounding cone per cluster, cull clusters then triangles in
compute, and reuse the result across multiple views in a frame. Somnium's
`meshlet.rs` already implements the cluster + normal-cone half of this
(Morton-order clustering, cone axis + cutoff, `-1` cutoff for non-cone-like
clusters). The unimplemented half is multi-view reuse.

### 5.8 Drobot, 2017 — *Improved Culling for Tiled and Clustered Rendering*

Cited by id Tech 8 for its **flat bit array** group-shared representation: cull
into a `groupshared uint bucket[N]` bitfield rather than a compacted list, which
keeps results ordered (important when the consumer needs stable ordering, e.g.
decals) and avoids per-item atomics. Directly applicable to DOOM-C's classify
pass and to `cluster.rs`.

### 5.9 AMD GPUOpen / NVIDIA — occupancy and precision

- *Occupancy explained* and *Register pressure* (GPUOpen): occupancy is set by
  the VGPR/SGPR high-water mark of the whole compiled shader, which is the
  formal statement of what the 2026-08-14 session found empirically.
- *First Steps When Implementing FP16* (GPUOpen): packed `f16x2` halves VGPR
  footprint and can double ALU throughput.
- **Counter-evidence, and the reason DOOM-K is gated:** Interplay of Light's
  fp16 experiments measured an FP16 FidelityFX resolve at **110 VGPR vs 83 for
  FP32**, and FP32 running **23% faster on an RTX 3080 Mobile**. Somnium's dev
  machine is an RTX 5080 Laptop. fp16 is an experiment with a real chance of
  reverting, not a plan item.

### 5.10 wgpu 29 — what the API actually permits

Audited against `wgpu-types-29.0.3/src/features.rs` and `naga-29.0.3`.

| Capability | wgpu 29 | Somnium today | DOOM use |
|---|---|---|---|
| `SUBGROUP` / `SUBGROUP_BARRIER` | ✅ | not requested | DOOM-L: classify reductions, strongest-four scan |
| `SHADER_F16` (`enable f16;`, naga → `SHADER_FLOAT16`) | ✅ | not requested | DOOM-K, gated |
| `MULTI_DRAW_INDIRECT_COUNT` | ✅ | not requested | DOOM-G: culled draws leave the command stream |
| `PIPELINE_STATISTICS_QUERY` | ✅ | not requested | DOOM-A: invocation counts answer "why" |
| `SHADER_INT64` | ✅ | no | not needed |
| `EXPERIMENTAL_MESH_SHADER` | ✅ | no | research only, non-goal |
| Multiple queues / async compute | ❌ | — | non-goal |
| Variable rate shading | ❌ | — | emulate per-tile in DOOM-C if ever needed |
| Multi-threaded encoders | ✅ (separate `CommandEncoder` per thread, ordered submit) | one encoder, one submit (`renderer.rs:2192`/`3257`) | DOOM-H |

Every one of these must be **detected, never demanded** — the same rule §17.7
established for `TIMESTAMP_QUERY_INSIDE_ENCODERS` and 24J for ray query. An
adapter without subgroups still runs, one dispatch slower.

### 5.11 Skimmed, and why they are not the spine

- `bevy-plugins/bevy_terrain-main` — virtual-texture terrain in wgpu; overlaps
  Phase DF, not DOOM.
- `terra-main`, `CDLOD-master` — geometry LOD; Somnium's chunk LOD is settled.
- `New_Engines/FlaxEngine-master` — already mined for the profiler (§17.7).
- `godot-4.7.1-stable`, `o3de-development`, `stride-master` — clustered lighting
  and terrain clipmaps, both already cited by earlier phases.
- `SpartanEngine-master` — cited for BRDF (ATTRIBUTION §11); its Vulkan
  descriptor management is worth a later read for DOOM-J, not now.

---

## 6. Architecture

### 6.1 One rule

**Delete work; do not hide it.** A uniform `if` around expensive code is not a
deletion — DXC and Naga flatten it and the occupancy is unchanged. The only
deletions that count are:

1. **Fewer pixels** run the shader (DOOM-F, and bins that early-out whole tiles).
2. **A smaller shader** runs on a given pixel (DOOM-C, DOOM-E — a different
   *pipeline*, never a different *branch*).
3. **Fewer invocations of a pass at all** (DOOM-D: cascades that did not change).
4. **The work happens on another thread or another frame** (DOOM-H, DOOM-I).

Every stage below has to say which of the four it is. A stage that cannot is not
an optimization, it is a hope.

### 6.2 Shading becomes compute, and `ShadingSpec` becomes per-tile

```text
                 vis_buffer (Rg32Uint: instance+1, primitive)
                              │
                    ┌─────────▼──────────┐
                    │  DOOM-C1 classify   │  1 thread / pixel, 8×8 groups
                    │  (compute)          │  subgroup reduce → bin id
                    └─────────┬──────────┘
                              │  per-bin tile lists + IndirectDispatchArgs
        ┌──────────┬──────────┼──────────┬───────────┬──────────┐
        ▼          ▼          ▼          ▼           ▼          ▼
     bin 0      bin 1      bin 2      bin 3       bin 4      bin 5
      SKY       MESH      TERRAIN    TERRAIN    FOLIAGE     MIXED
              (opaque)    (near)      (far)     (alpha)   (fallback)
        │          │          │          │           │          │
        └──────────┴──────────┴────┬─────┴───────────┴──────────┘
                                   ▼
                         HDR Rgba16Float (storage)
```

Each bin is one `dispatch_workgroups_indirect` against a distinct pipeline
compiled from `shading.wgsl` with a distinct `ShadingSpec`. The `override`
mechanism already in `pass/shading.rs` — `enable_hex`, `enable_pom`,
`enable_pcss`, `enable_contact`, `enable_clipmap`, `enable_debug`,
`terrain_scan`, `enable_live_terrain` — is exactly the specialization vocabulary
this needs. DOOM-C adds bin-specific values rather than a new mechanism.

Consequences worth stating up front:

- **Sky.** `vis_data == 0u` tiles run a shader containing one cubemap sample and
  `sky_detail`. On a Coastal overview that is a large fraction of the screen
  currently scheduled at terrain register pressure.
- **Meshes.** No terrain code in the module at all — no 8 splat samples, no
  32-wide scan, no `evaluate_terrain_material`.
- **Terrain-near vs terrain-far** is the *bin* form of the aerial PSO the
  2026-08-14 notes asked for: "a second *aerial* PSO that drops unique-colour /
  two-layer maps **without** a per-pixel sample-count branch." A bin is not a
  branch. This is DOOM-E.
- **MIXED** exists because a tile straddling terrain and sky must be correct.
  The fallback bin is today's full shader. Correctness first; if MIXED turns out
  to dominate the screen, the tile size is wrong and DOOM-C4 tunes it (5.3).

**Derivatives are the risk.** The fragment shader gets quad derivatives free.
Phase 25N already computes analytic UV gradients for the visibility-buffer
terrain path precisely because implicit `dpdx` across a vis-buffer 2×2 quad
straddles unrelated triangles — so the hard case is largely solved. Where a
genuine quad is still needed, Wicked's `remap_lane_quads` + `QuadReadAcrossX/Y`
pattern applies, which in WGSL means `quadSwapX`/`quadSwapY` under the subgroup
feature. DOOM-C2 exists to prove parity on a single bin before six bins are
built on top of an unproven assumption.

### 6.3 Bin taxonomy, and how a pixel's bin is known

Classification must be cheap and must not read material textures. It has exactly
the vis buffer, the instance array and the material array:

| Bin | Test |
|---|---|
| SKY | `vis_data == 0u` |
| TERRAIN_NEAR | `instances[i].flags & TERRAIN` and tile centre depth < `far_split` |
| TERRAIN_FAR | `instances[i].flags & TERRAIN` and beyond it |
| FOLIAGE | material has alpha-test / two-sided flag |
| MESH | anything else opaque |
| MIXED | tile disagrees |

The class is written on the CPU, where the draw queue already knows whether a
command is terrain, foliage or mesh (`renderer.rs` builds all three), so the
classify shader is one buffer load per pixel plus a subgroup reduction and never
touches a material texture.

**Where the byte lives is a real decision, not a detail.** `GpuInstanceData` is
80 bytes — `model_matrix`, `material_id`, `mesh_vertex_offset`,
`mesh_index_offset`, `_padding` — and `_padding` is **already claimed by the
25C vertex morph** (`context.md` §17.5). So DOOM-C1 must either bit-pack the
class alongside the morph flag in that word, with both fields documented at the
declaration, or widen the struct to 96 bytes and update
`context.md` §13's layout table in the same commit. Do not silently reuse
`_padding` and leave 25C to discover it.

### 6.4 Shadows: cache what did not move

Four cascades into a 4096² `Depth32Float` atlas, every frame, unconditionally.
CR-E added cascade-volume caster culling, so the *draw list* per cascade is
already correct — but the atlas is still fully re-rendered when nothing in it
changed.

DOOM-D makes each cascade's re-render conditional on invalidation:

- The sun direction moved more than a per-cascade angular epsilon (distant
  cascades tolerate more).
- A caster whose AABB overlaps that cascade moved, was created, or was deleted.
- The cascade's snapped centre crossed a texel (cascades are already fitted from
  `view_proj_unjittered` and bounding-sphere snapped, §18 — that snapping is
  what makes a cache possible at all).
- The cascade has never been rendered.

Plus **staggering**, from id Tech 8's interleaved volume updates (5.4): when
several distant cascades invalidate in the same frame, spread them across
frames rather than paying for all four at once.

A static camera looking at a static scene under a static sun should show
`Shadows ≈ 0.00 ms` in the profiler. That is the exit criterion, and it is also
the bug detector: a cascade that keeps invalidating with nothing moving means the
epsilon or the snap is wrong.

### 6.5 Pixels: a controller that reads the profiler

`viewport_resolution.rs` offers five fixed presets and FSR 3 is default on. The
gap is that nothing closes the loop. DOOM-F adds a controller that reads the
profiler's smoothed frame time, targets a user-set frame budget, and moves a
*continuous* scale factor within a user-set floor, with hysteresis and a rate
limit so it cannot oscillate or pump.

This is the one stage that deliberately trades fidelity, so it is the one stage
that must be **off by default**, explicitly opt-in on the Camera or Post
Processing entity, with the floor visible in the UI. "Locked 60" is a setting a
user chooses, not something the engine does behind their back.

### 6.6 CPU: what parallelism can and cannot buy

Be honest about the arithmetic. At ~50 ms of GPU shading and ~1.5% CPU, moving
CPU work to other cores **cannot** shorten the frame. CR-A already ruled on
this and CR-D's 512-item rayon threshold is the correct conclusion for 256
chunks. Nothing in DOOM overturns it.

What CPU parallelism buys is different and still worth having:

- **Headroom.** DOOM-C, D and F are aiming at a ~3× GPU reduction. At 16 ms the
  serial encode of ~25 profiled passes on one thread stops being free, and the
  256-chunk assumption stops holding when the tile grows.
- **Hitches.** The DF audit found a single frame consuming a 1 048 576-texel
  clipmap generate. `MAX_GEN_TEXELS`, BC7 encoding, glTF parse, terrain sidecar
  IO and voxel meshing are all frame-visible today. A hitch is a frame-time
  failure even when the average is fine.
- **Determinism.** A worker pool that produces byte-identical captures is a
  prerequisite for trusting every other measurement in this phase.

So DOOM-H's exit criterion is **encode time down, frame time not up, captures
identical** — not a Task Manager number.

### 6.7 RAM: caches with a measured hit rate

Working set is ~553 MB. CR-A: "Do not allocate RAM to raise this number."
DOOM only spends RAM where a cache has a *measured* hit rate:

- The shadow atlas cache (DOOM-D) costs nothing new — it reuses the atlas.
- Per-bin tile lists (DOOM-C) are `tile_count × 8 B` per bin; at 2560×1392 and
  8×8 tiles that is ~55 k tiles, so under 3 MB for all bins together.
- Persistent staging and bind-group caches (DOOM-J) trade a few MB for the
  elimination of per-frame allocation.
- Streaming queues (DOOM-I) get a stated byte budget, not "as much as fits."

Any DOOM stage proposing more than ~64 MB must justify it with a hit rate.

---

## 7. Stages

Ordered by dependency. **A and B are gates**: nothing after them ships without
their numbers, because every alternative ordering in this project's history
ended in a session spent flipping switches that could not move.

### DOOM-A — The clock (no visual change)

Make a frame's cost knowable and a measurement repeatable.

1. Bracket the unbracketed: culling, visibility phase 2, ReSTIR DI/GI, IBL,
   grid/gizmo/outline/UI. `unattributed` must fall below 5% of frame.
2. Add `PIPELINE_STATISTICS_QUERY` (detected, not demanded) for fragment and
   compute invocations per pass — the counter that says *why* a pass is slow,
   in the spirit of Flax's `RenderStats.h` already cited in §17.7.
3. **Timing harness**, the direct analogue of `capture.rs`: `SOMNIUM_TIME=out.somtime`
   replays a fixed camera path (scripted keyframes, fixed dt, fixed frame count,
   TAA/FSR state pinned), collects per-zone GPU ms and counters, and writes a
   table. `SOMNIUM_TIME_COMPARE=before.somtime` prints per-zone deltas with the
   run-to-run spread, so a 3% change is distinguishable from noise.
4. Three canonical viewpoints baked into the harness, matching the existing
   evidence vocabulary: **Coastal-ground**, **Coastal-overview**, **Island**.

**Exit:** `unattributed` < 5%; two runs of one build agree within 2% on every
zone above 0.5 ms; a baseline `.somtime` for all three viewpoints committed as
the phase's reference. **No pixel changes** — `capture.rs` byte-identical.

### DOOM-B — The pixel census (no visual change)

Answer the question no existing measurement answers: where do the 40 ms go, by
*pixel class*, not by pass.

1. A debug pass counting pixels per prospective bin (§6.3) into an atomic
   buffer, surfaced in the profiler alongside the existing counters.
2. An A/B by ablation: temporarily compile shading with only one class's code
   live and the rest returning a constant, and time it. Ugly output, honest
   numbers, and exactly how §17.7 measured 25D.

**Exit:** a table per viewpoint — pixel share and measured ms share for sky,
mesh, terrain-near, terrain-far, foliage, water. **This table decides whether
DOOM-E and DOOM-F are worth building at all**, and it is the phase's central
piece of evidence.

### DOOM-C — Tile-classified compute shading

The marquee. Four sub-steps, each independently revertable.

- **C-1 — classify pass.** Compute, 8×8 groups, writes per-bin tile lists and
  `IndirectDispatchArgs`. UE5's count/reserve/scatter ordering (5.3) so tile
  order is deterministic; Drobot flat bit arrays (5.8) in group-shared memory;
  subgroup reductions where available, a group-shared fallback where not.
  Ships **unused** — the fragment path still draws. Verified by reading the bin
  buffers back in a test and by a debug view tinting tiles by bin.
- **C-2 — compute shading at one bin (parity gate).** Port `fs_main` to a
  compute entry point writing an `rgba16float` storage texture, with a single
  bin containing every tile. This deliberately buys **nothing**; it exists to
  prove the port is exact. **Exit: `capture.rs` byte-identical to the fragment
  path.** If derivatives are wrong, this is where it shows, on a change small
  enough to debug.
- **C-3 — real bins.** SKY, MESH, TERRAIN, FOLIAGE, MIXED, each with its own
  `ShadingSpec`. **Exit: capture-identical, and Shading ms down on
  Coastal-ground and Coastal-overview.**
- **C-4 — uniform-tile fast path and tile-size tuning.** Wicked's
  primitive-uniform detection (5.2), plus measuring 8 vs 16 vs 32 px tiles
  (5.3). **Exit: measured; keep whichever wins; record the losers.**

Kill switch `SOMNIUM_SHADE_COMPUTE=0` restores the fragment path for the life
of the phase. The fragment path is not deleted until DOOM-M.

### DOOM-D — Shadow cache

Per-cascade invalidation (sun epsilon, caster movement, snap crossing, never
rendered) plus staggered re-render (§6.4). Optionally the multi-view half of
The Forge / Wihlidal triangle filtering (5.6/5.7) if DOOM-A shows the shadow
pass is geometry-throughput-bound rather than invocation-bound.

**Exit:** static camera + static sun + static scene ⇒ `Shadows` ≈ 0 ms; sun
sweep and moving casters show no shadow lag in a capture sequence;
`SOMNIUM_SHADOW_CACHE=0` A/B.

### DOOM-E — The aerial terrain bin

Only if DOOM-B says TERRAIN_FAR is a real share. A TERRAIN_FAR `ShadingSpec`
that drops unique colour, the second layer bank and macro detail at distance,
compiled as its own pipeline — the "second aerial PSO" the 2026-08-14 notes
asked for, expressed as a bin.

**Coordination with Phase DF, non-negotiable:** clipmap is the *designed* cheap
terrain path and DF-E owns whether it goes default-on. DOOM-E is what a
non-clipmap frame gets. A DOOM session must not turn clipmap on to win Coastal
fps, and must not retune the clipmap to make DOOM-E look better.

**Exit:** capture diff confined to terrain pixels beyond the split, mean
luminance delta within the DF §6.4 style tolerance; Shading ms down on
Coastal-overview.

### DOOM-F — Dynamic resolution (opt-in)

Continuous scale inside FSR 3, driven by the profiler, with a user floor,
hysteresis and rate limiting (§6.5). Default **off**.

**Exit:** with a 16.6 ms target and a 67% floor, Coastal-ground holds 60 fps and
the scale is visibly stable (no pumping) across a scripted flythrough.

### DOOM-G — Draw submission

1. `MULTI_DRAW_INDIRECT_COUNT` (detected) so culled draws leave the command
   stream instead of executing as zero-instance draws.
2. **The invariant question, answered explicitly.** 15A's contract is "argument
   *i* is instance *i*; culling writes `instance_count = 0`, never removes
   entries." A draw-count path changes that. DOOM-G must either keep the array
   uncompacted and only shorten the count from the tail, or introduce a
   compacted argument array with a parallel index remap — and whichever it
   picks, `context.md` §21 and the notes in `indirect.rs` get updated in the
   same commit. Silently breaking that invariant will cost a later session a
   day.
3. **G-2, research only:** triangle filtering across camera + cascades (5.6).
   Written up, not implemented, unless DOOM-A/D show it pays.

**Exit:** draw count on a Coastal look-away falls; capture identical; the
invariant is restated in the docs.

### DOOM-H — A real job system

Replace `jobs.rs`'s single helper with a scoped worker pool over the existing
rayon dependency:

- Parallel: terrain LOD classify, frustum bits, instance fill, cluster
  assignment, foliage cull — all with the CR-D threshold preserved as a
  *default*, now tunable and measured rather than assumed.
- **Parallel command encoding.** wgpu permits one `CommandEncoder` per thread
  with ordered submit. Record independent pass groups (shadow cascades, the
  water chain, the post chain) on workers and submit in order.

**Exit:** CPU encode zone falls measurably; frame time flat or better; captures
byte-identical (a nondeterministic pool is a bug, not a tuning choice); the
serial path stays available and unit-tested, as CR-D established.

### DOOM-I — Off the critical path

BC7 encoding, glTF parse, terrain sidecar IO, voxel meshing, and clipmap
generate staging move onto the pool behind a per-frame upload budget, so no
single frame does a 1M-texel generate.

**Exit:** a **hitch metric** in the timing harness — no frame above 2× the
median across a scripted flythrough — improving against the DOOM-A baseline.

### DOOM-J — Bandwidth, formats, allocations

Measured, not speculative: `RGB9E5` or `f16` for intermediate targets where the
census says bandwidth matters (id Tech 8's format note, 5.4); remove redundant
full-resolution copies; an allocation counter that asserts zero buffer, texture
and bind-group creation in a steady-state frame.

**Exit:** bytes/frame down, ms down or flat, steady-state allocation count zero.

### DOOM-K — fp16 (experiment, expected to be 50/50)

`SHADER_F16` + `enable f16;` in the terrain material inner loop only.

**Exit:** kept **only** if it wins ≥5% on the dev GPU with a byte-comparable
capture. Reverted otherwise, with the numbers recorded — 5.9's counter-evidence
(FP32 23% faster on an RTX 3080 Mobile) makes a null result the expected
outcome, and recording it is the deliverable either way.

### DOOM-L — Subgroup operations

Where measured: the classify reduction, the terrain strongest-four scan,
cluster assignment. Feature-detected with a group-shared fallback that is
unit-tested to produce identical results.

**Exit:** each site individually A/B'd; keep only the winners.

### DOOM-M — Close out

Evidence in `dev records/phase DOOM/`; `.somtime` baselines and finals;
`context.md` §17 and §21 updated; `ATTRIBUTION.md` §13C; Help pages
(`docs/editor/viewport.md` for dynamic resolution, `docs/editor/lighting.md`
for the shadow cache); the fragment shading path deleted only once DOOM-C has
been default-on through a full session without a revert.

---

## 8. Measurement contract

Restated because this project has been burned twice (§18, and the
2026-08-14 occupancy notes):

1. **Screen-capture frame deltas are not evidence.** 0.776 → 2.018 across three
   runs of one build.
2. **A number without a control is not evidence.** §17.7's 25D table is the
   model: every pass the change should not touch identical to the third decimal.
3. **Task Manager CPU % is not a score.** Frame time is.
4. **Toggling a runtime uniform is not an occupancy experiment.** Only a
   different compiled pipeline is.
5. **Every stage states its class** from §6.1 — fewer pixels, smaller shader,
   fewer invocations, or different thread — before it is implemented.

---

## 9. Budgets

Aspirational, from the DOOM-A baseline. These are targets to be argued with by
measurement, not promises.

| Zone | Coastal-ground today | DOOM target | Owner |
|---|---:|---:|---|
| Shading | ~40–50 ms | **≤ 12 ms** | C, E |
| Shadows | unisolated | **≤ 1 ms** (static: ~0) | D |
| Visibility + Hi-Z | small | unchanged | — |
| Water | ~3.6 ms | unchanged | frozen |
| Everything else | unmeasured | measured | A |
| **Frame** | **~50–71 ms** | **≤ 16.6 ms** | phase |
| CPU encode | unmeasured | measured, then ≤ 2 ms | A, H |
| Working set | ~553 MB | ≤ 700 MB | J |

---

## 10. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| **Derivatives in compute** break terrain/foliage mip selection | High | C-2 is a parity gate before any bin work; 25N analytic gradients already cover the terrain path; `quadSwapX/Y` under subgroups for the rest |
| MIXED tiles dominate, and binning wins nothing | Medium | C-4 tile-size sweep (8/16/32); DOOM-B's census predicts this before C-3 is built |
| Classify cost exceeds the saving | Medium | C-1 ships unused and is timed on its own first |
| Shadow cache shows stale shadows | High | Invalidation is conservative by default; `SOMNIUM_SHADOW_CACHE=0`; a capture sequence over a sun sweep is the gate |
| Parallel encode makes captures nondeterministic | High | Byte-identical capture is DOOM-H's exit criterion, not a nice-to-have |
| A DOOM session "fixes" Coastal by turning clipmap on | High | Explicit non-goal; DF-E owns it; called out in §3 and DOOM-E |
| `rgba16float` storage-texture support is thinner than the render-attachment path on some backend | Medium | Detected at init with a fragment fallback — the same pattern as ray query and timestamps |
| fp16 regresses on the dev GPU | Low | Expected; DOOM-K is gated and reverting is a valid outcome |
| Breaking 15A's "arg *i* is instance *i*" invariant silently | High | DOOM-G must restate the invariant in `context.md` §21 and `indirect.rs` in the same commit |

---

## 11. Inspector, Help, kill switches

| Control | Where | Default |
|---|---|---|
| `SOMNIUM_SHADE_COMPUTE=0` | env | compute on once C-3 lands |
| `SOMNIUM_SHADOW_CACHE=0` | env | cache on once D lands |
| `SOMNIUM_TIME=<file>` / `SOMNIUM_TIME_COMPARE=<file>` | env | off |
| Profiler bin overlay (tile bin tint) | viewport debug | off |
| **Dynamic Resolution** + target ms + floor % | Camera entity details | **off** |
| **Shadow Cache** checkbox | Lighting details | on |
| Job threads (0 = auto) | Post Processing / engine settings | auto |

Help pages: `docs/editor/viewport.md` (dynamic resolution, profiler bins),
`docs/editor/lighting.md` (shadow cache and what invalidates a cascade).

---

## 12. Must not do

- Turn Clipmap default on to win Coastal fps.
- Reintroduce per-pixel `close` / `use_maps` / `layer_budget` sample-count LOD.
- Shrink `TERRAIN_LAYER_COUNT` or the `GpuTerrainMaterial` layout.
- Retune Great Lakes water, XV look, foliage LOD, or the Island recipe.
- Report a win from a screen-capture frame delta.
- Add a runtime uniform and call it an occupancy fix.
- Copy source from id Tech, UE5, Wicked, CryEngine or Frostbite.
- Ship a stage whose capture diff was never taken.

---

## 13. Bibliography

Cite in `ATTRIBUTION.md` as each is actually used.

**Local trees (read 2026-08-16, cited in ATTRIBUTION §13C):**
- Wicked Engine — `visibility_analyzeCS.hlsl`, `visibility_shadeCS.hlsl`,
  `visibility_surfaceCS.hlsl`, `ShaderInterop_Renderer.h`
- Unreal Engine 5 — `Nanite/NaniteShadeBinning.usf`
- The Forge — `Common_3/Renderer/VisibilityBuffer2/` (located; contents to be
  read by DOOM-G2)

**Papers, talks and vendor material:**
- Burns & Hunt, *The Visibility Buffer: A Cache-Friendly Approach to Deferred
  Shading*, JCGT 2013 — the architecture Somnium already implements
- Engel, *The Filtered and Culled Visibility Buffer*, GDC Europe 2016
- Wihlidal, *Optimizing the Graphics Pipeline with Compute*, GDC 2016 / GPU Zen 1
- Drobot, *Improved Culling for Tiled and Clustered Rendering*, SIGGRAPH 2017
- Sousa, *The Devil Is In The Details: idTech 666*, SIGGRAPH 2016
- Geffroy, *Rendering the Hellscape of DOOM Eternal*, SIGGRAPH 2020 Advances
- Sousa, *FAST AS HELL — idTech 8 Global Illumination*, SIGGRAPH 2025 Advances
- Olson & Assarsson, *Clustered Deferred and Forward Shading*, HPG 2012
- Chen, *Adaptive Virtual Texture Rendering in Far Cry 4*, GDC 2015 (DF context)
- AMD GPUOpen — *Occupancy Explained*, *Register Pressure*, *First Steps When
  Implementing FP16*, *RDNA Performance Guide*
- NVIDIA — *Advanced API Performance: Shaders*
- Pettineo, *Half The Precision, Twice The Fun: Working With FP16 In HLSL*
- Interplay of Light, *Experimenting with fp16 in shaders* (the counter-evidence
  for DOOM-K)

---

## 14. Handoff rule

1. Read `context.md` §6, §12, §14, §17.7, §21; `phase_CR.md`;
   `terrain_shading_occupancy_2026-08-14.md`; `phase_DF.md` §3, §4, §12; and
   this file, before writing any code.
2. **Do DOOM-A and DOOM-B first, in that order.** Not "start on the interesting
   one." Every prior attempt to shorten this pass without a clock ended in a
   session of switch-flipping that could not move.
3. Do not restart at DOOM-A inside a DF or Halcyon session, and do not do DF's
   clipmap work inside a DOOM session.
4. One sub-phase per commit; `capture.rs` diff and `.somtime` delta in the
   commit message.
5. `context.md` and `ATTRIBUTION.md` after every sub-task, per
   `feedback_approach.md` §2.
