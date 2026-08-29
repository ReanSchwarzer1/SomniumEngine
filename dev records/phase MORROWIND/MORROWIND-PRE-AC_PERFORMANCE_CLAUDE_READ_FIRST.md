# MORROWIND pre-AC performance pass — Claude read-first handoff

> **Purpose:** audit the whole Somnium Engine for measurable performance work,
> accidental complexity, and code-generation residue, then propose a safe,
> benchmark-driven execution phase. Run this handoff **before**
> `MORROWIND-AC_CLAUDE_READ_FIRST.md`.
>
> This is a **read-only turn**. Do not edit, create, delete, format, build, test,
> benchmark, stage, or commit repository files. Return the report described in
> §8, stop, and wait for explicit implementation approval.
>
> **Snapshot checked 2026-08-29:** `dev` at `439b6b6`
> (`feat(renderer): stream terrain material pages`). Verify `git status` and
> `git log -5 --oneline`; newer repository state is authoritative. Treat the AC
> handoff as planning, not implementation evidence.

## 1. Mandatory skills

Read these skill files completely before the audit and apply their relevant
parts. If a path moved, locate the same named skill under `.claude\plugins`,
`.codex\plugins`, or `.agents`; do not substitute a generic prompt.

1. `C:\Users\adhir\.claude\plugins\marketplaces\claude-code-skills\engineering\skills\performance-profiler\SKILL.md`
   — measurement-first profiling and before/after evidence.
2. `C:\Users\adhir\.claude\plugins\marketplaces\agentic-awesome-skills\skills\rust-pro\SKILL.md`
   — Rust ownership, allocation, cache locality, concurrency, SIMD, and
   documented `unsafe` boundaries.
3. `C:\Users\adhir\.claude\plugins\cache\claude-code-skills\engineering-skills\2.9.0\skills\senior-architect\SKILL.md`
   — subsystem/dependency map and explicit trade-offs.
4. `C:\Users\adhir\.codex\plugins\cache\claude-cowork\engineering\1.2.0\skills\tech-debt\SKILL.md`
   — evidence-based debt classification and ranking.
5. `C:\Users\adhir\.agents\skills\codebase-design\SKILL.md`
   — preserve or create deep module seams instead of growing bridge types.

Use the repository's rules over any generic skill example. This is a Rust/wgpu/
WGSL engine; Node, Python, web-service, and database examples in a skill are not
recommendations for Somnium. Reuse the repository's Graphify/census outputs;
do not run skill-bundled report generators during this read-only turn.

## 2. What “unslopify” means here

Translate the request into falsifiable findings. Do not label code “AI slop”
because it is long, unfamiliar, centralized, or heavily commented. A candidate
must cite a path and symbol and show at least one of:

- repeated or contradictory policy, shotgun surgery, shallow forwarding layers,
  dead code/dependencies, speculative abstractions, or an oversized public seam;
- accidental work in a hot path: allocation, clone, string formatting, hashing,
  sorting, locking, ECS traversal, resource creation, upload, copy, clear,
  dispatch, draw, texture sample, divergent branch, or redundant conversion;
- comments that merely restate syntax, stale claims, placeholder prose, or
  generated-looking duplication that obscures invariants;
- a data layout or algorithm whose cost mechanism can be measured against a
  credible alternative.

Keep useful rationale, safety, ABI, shader-layout, and workaround comments.
Do not propose mass comment deletion, repository-wide renaming, formatting
churn, or abstraction replacement as “optimization.” Maintainability cleanup
and runtime improvement are separate columns; never claim speed without a
benchmark or profile that can disprove the claim.

“Unique implementation” means a Somnium-specific solution is welcome when it
is simpler or measurably faster for this workload. Novelty is not an acceptance
criterion. Prefer standard, well-understood algorithms when they win.

## 3. Efficient source-of-truth reading order

Use `rg`, Graphify, and the generated census as indexes. Read only the source
needed to verify a claim; do not consume `context.md` linearly.

Authority order is current source plus enforced tests/layout assertions, then
the current `context.md` preamble and generated evidence, then completion
records, then historical plan prose. Historical sections intentionally preserve
superseded values: never promote one to a current claim without source evidence.

1. `context.md`: preamble/current-state entries, §§2–16 architecture/frame
   order/layouts, performance decisions, and searches for `profil`, `timing`,
   `allocation`, `stream`, `cull`, `job`, `JIT`, `SIMD`, and `somtime`.
2. `dev records/phase_MORROWIND.md`: §§3–6, §8, and §§9–13, especially Track 7
   timing and GHOSTFENCE contracts. Read
   `dev records/phase MORROWIND/README.md`, then scan
   headings plus status/decision/verification sections across completed records
   so every lane in §5 has evidence. Read A, B, C, Q, R, S, T, Z, AB, AD—and any
   record owning a serious candidate—more deeply, rather than ingesting every
   record linearly.
3. Performance history, using `rg` for baseline, decision, deferred, default,
   and exit sections before opening surrounding prose:
   `phase_DOOM.md`, `phase_CR.md`, `phase_DF.md`, `phase_XV.md`,
   `phase XV/XV-Zeta_plan.md`, `phase_VV.md`,
   `post_halcyon_audit_handoff.md`,
   `terrain_shading_occupancy_2026-08-14.md`, and `phase_CONTROL.md`.
4. Generated structural evidence:
   `dev records/phase MORROWIND/MORROWIND-A_census.md`,
   `graphify-out/GRAPH_REPORT.md`, and `graphify-out/graph.json`. The local
   `.graphify_analysis.json` and `.graphify_semantic_new.json` are optional
   ignored derivatives, not portable sources of truth.
5. Measurement/gates: `crates/somnium_renderer/src/{profiler,timing}.rs`,
   `examples/hello_engine/src/main.rs` around the Track-7 harness,
   `crates/somnium_core/src/time.rs`, `crates/somnium_jobs/`, and
   `tools/ghostfence/run.py`. Also read the existing script performance harness
   at `crates/somnium_script_luau/tests/budgets.rs` and its measured record at
   `dev records/phase 16/16-B_budgets.md`.
6. Workspace and legal contracts: root/crate `Cargo.toml` files,
   `CONTRIBUTING.md`, `ATTRIBUTION.md`, and relevant tests/docs.
7. Finally sample every runtime lane in §5. Follow call/data flow around likely
   costs rather than reading every file.

The tracked Graphify export was committed at `66d1ccf`, before MORROWIND-AB/AD.
It is a structural map, not current-status proof. `GRAPH_REPORT.md` reports
high-degree nodes including `UiManager` (224), `Widget` (201),
`SomniumRenderer` (169), `component_registry()` (100), `Engine<G>` (91), and
`WidgetBuilder` (91), with `SomniumRenderer` a high-betweenness bridge (0.259).
These are change-risk and module-seam clues—not runtime hotspots. Treat inferred
semantic edges as hypotheses and verify every finding in current source.

## 4. Facts to verify, not rediscover blindly

- The generated census reports 186,029 Rust/WGSL lines and 1,842 test markers
  across the surveyed crates. Renderer (59,490), UI (57,203), and core (31,703)
  are 79.8% of that crate subtotal; examples and tools are outside it.
- The repository contains 55 WGSL files / 14,287 lines. The largest are
  `shading.wgsl` (2,079), `water.wgsl` (1,171), and
  `terrain_material.wgsl` (1,165). Size alone does not prove GPU cost.
- The census flags nine dependencies as unreferenced by grep. Its own warning is
  binding: verify through Cargo targets/features/build scripts/macros before
  recommending removal.
- `profiler.rs` already has a three-slot deferred readback ring, CPU scopes,
  counters, and smoothed/unsmoothed GPU samples. GPU timestamps and pipeline
  statistics are adapter-feature dependent. CPU zones are EMA-smoothed before
  `timing.rs` accumulates them, so do not describe them as raw CPU samples.
- `.somtime v1` deliberately measures stationary views after warm-up and writes
  mean, standard deviation, minimum, maximum, and sample count—not percentiles.
  It excludes streaming, clipmap recentering, LOD transitions, and flythrough
  hitches by design. Checked-in Z and AB A/B pairs exist.
- Root `Cargo.toml` deliberately optimizes image decoding packages in dev and
  deliberately withholds script JIT/codegen pending a benchmark. Treat both as
  recorded decisions, not omissions.
- The ordinary documented library gate is `cargo test --workspace --lib`.
  GHOSTFENCE instead runs `cargo test --workspace -j 1`: this workspace's
  OneDrive location can make parallel linking fail with transient `LNK1104`.
- MORROWIND-AD preserves `GpuTerrainMaterial` at exactly 2,032 bytes. Numerous
  layouts, defaults, serialized fields, golden images, and visual behaviours are
  frozen contracts. Read their records before suggesting layout or shader work.
- There is no general workspace-wide microbenchmark suite or Cargo `[[bench]]`
  target in the current tree. There is a dedicated criterion-free release-test
  harness for Luau at `crates/somnium_script_luau/tests/budgets.rs`; reuse its
  pattern where appropriate before proposing Criterion or another dependency.
- Owed captures or interactive timings recorded by earlier phases are evidence
  debt, not newly discovered optimizations. Close the relevant debt before
  claiming a speedup.

## 5. Audit lanes

Sample all lanes, then go deep only where evidence supports it:

1. **GPU/frame graph/WGSL:** pass timings, bandwidth, attachment load/store,
   clears/copies, intermediate textures, overdraw, dispatch dimensions, sampling,
   divergence, occupancy, resource/bind-group churn, and pipeline compilation.
   Inspect render order and visual dependencies before suggesting pass fusion.
2. **Renderer CPU/submission:** extraction, visibility/culling, queue building,
   sorting, per-frame allocations, uploads, staging, instance/material updates,
   lock scope, and CPU↔GPU synchronization.
3. **Editor/UI/input:** layout and drawing traversal, message routing, clones and
   strings per frame, hit testing, list/tree virtualization, Details generation,
   thumbnail work, and editor idle cost.
4. **Assets/cooking/streaming:** file I/O, decoding, hashing, import/cook caching,
   scene load/save, virtual-texture paging, job granularity, back-pressure, and
   redundant CPU/GPU representations.
5. **ECS/script/animation/physics/audio/jobs:** iteration/data layout, query
   rebuilds, FFI boundaries, allocations, scheduling, queue wait, contention,
   deadline-sensitive audio work, and avoidable synchronization.
6. **Startup/build/test/dependencies:** startup latency, shader/pipeline creation,
   feature weight, compile/link bottlenecks, duplicate dependencies, and tests
   whose structure—not assertions—causes disproportionate cost.

For Rust, consider arenas, reuse, SoA/AoS changes, tighter types, small-vector or
hash choices, batching, parallelism, SIMD, and `unsafe` only after measuring the
actual mechanism and workload threshold. For WGSL, quantify invocations,
bandwidth, samples, branches, register pressure/occupancy proxies, and visual
error. “Fewer lines” is not a GPU metric.

## 6. Baseline plan Claude must design—but not run yet

Define reproducible scenarios before proposing an execution order:

- canonical `coastal-ground` and `island` stationary cameras, fixed resolution/
  configuration, warm-up, sample count, hardware/adapter/capability report,
  release profile, and a development/editor profile where iteration speed matters;
- a separate moving-camera/hitch experiment—do not present it as `.somtime`
  steady-state evidence—plus editor idle, heavy terrain/foliage,
  water/transparency, Details editing, asset drawer/thumbnails, scene load/save,
  VT streaming, and at least one script/animation/job-pressure scenario
  supported by current fixtures;
- CPU wall/frame percentiles where separate instrumentation can produce them,
  unsmoothed GPU total/pass rows, hitch count,
  queue wait/job drops, startup/load/import/cook latency, peak RAM/VRAM where a
  trustworthy tool exists, and allocation/call-stack evidence for CPU claims;
- for credible build/startup candidates: cold/warm compile time, link time,
  artifact size, and shader/pipeline startup cost;
- matched before/after `.somtime` files and display-referred captures whenever a
  frame/render path changes, plus GHOSTFENCE.

State which current tools provide each metric, what instrumentation is missing,
and the smallest way to add it later. `.somtime v1` does not provide p50/p95/p99
or a moving-camera hitch trace. Do not reduce resolution, scene content, quality,
feature defaults, assertions, or correctness to manufacture a win.

## 7. Ranking and future execution gates

For each candidate provide: path/symbol; observed evidence; hypothesized cost
mechanism; validation measurement; affected scenario; expected impact; inaction
risk; implementation/regression risk; effort; confidence; contract/tests at
risk; and whether it is performance, maintainability, or both.

Score Impact, Inaction Risk, and Effort from 1–5 and show
`priority = (Impact + Inaction Risk) × (6 - Effort)`. Here Impact means measured
runtime/user or developer-loop cost; Inaction Risk means the cost of leaving it
unresolved. Never feed implementation risk into the formula. The score informs
ordering but does not override prerequisites or safety. Separate quick verified
removals from architectural changes and experimental algorithms.

Propose a large phase as small reversible waves. Each eventual slice must be one
coherent commit and follow:

`baseline → focused change → same-scenario after measurement → correctness and
visual gates → docs/evidence/census if affected → commit`

Guardrails for every proposed wave:

- no new `unsafe` without a measured need, a smaller audited boundary, written
  safety invariants, Miri/sanitizer strategy where applicable, and tests;
- no SIMD, thread pool/job system, parallelism, dependency, cache, render graph,
  shader framework, reflection layer, or UI framework added without proving the
  current mechanism and break-even workload;
- no visual/default/ABI/serialization/public-API change smuggled into a speedup;
  public API expansion belongs at a deliberate MORROWIND/Vvardenfell seam;
- no copied proprietary implementation. Use primary public literature, official
  docs, or permissively licensed references; record attribution and implement
  cleanly for Somnium;
- do not start MORROWIND-AC work in this phase or redesign AC around speculative
  optimizations. The AC read-first handoff follows after this audit is accepted.

## 8. Required response and stopping condition

Return at most about **1,800 words**, with repository path/symbol or record-section
citations for every material claim:

1. a one-page architecture/performance map and current measurement capabilities;
2. the fixed baseline matrix to run during implementation;
3. a ranked top backlog (roughly 8–12 candidates), grouped into safe execution
   waves and clearly separating measured evidence from hypotheses;
4. objective code-quality/“slop” findings, including items that should *not* be
   touched because they encode an invariant or deliberate trade-off;
5. Rust and WGSL opportunities, with alternatives and validation methods;
6. frozen contracts, likely regression gates, and source/license constraints;
7. unknowns or measurements required before selecting the first implementation
   slice; and
8. a recommended phase name and sub-phase/commit sequence, explicitly placing it
   before MORROWIND-AC.

Completion means every audit lane was sampled and each top candidate names a
cost mechanism, a falsifying measurement, and a bounded change seam. **The
deliverable is the report, not code or a patch.** Stop and wait for approval.
