# Phase KENSHI — OGRE

> *Kenshi does not fall over because any one thing is slow. It falls over
> because two hundred squads, a streamed world and a physics sim are all
> correct at once.*

> **Codename:** KENSHI (Lo-Fi Games, OGRE 3D, 2018). Load-bearing for three
> reasons. It is the open-world RPG whose identity **is** scale — hundreds of
> independent AI squads, each pathing, fighting and trading across a streamed
> continent, with no instancing of the simulation and no scripted set-pieces
> holding it together. It is the game that is **famous for falling over at that
> scale**, in exactly the way an engine falls over: not one hot function, but
> every subsystem being individually reasonable and collectively fatal. And it
> is built on **OGRE**, which has been sitting in `example_repo` for this
> project's entire life and has never been read for anything except HLMS.
>
> **Status:** **PLAN — nothing in tree, and the tree it plans against does not
> exist yet.** Written 2026-08-23. **This document is structurally different
> from its predecessors and §4 explains how**: `phase_CONTROL.md` and
> `phase_MORROWIND.md` open with a *measured* audit of the tree at a named
> commit. KENSHI cannot, because the things it measures — skinned crowds,
> streamed cells, GPU particles, navmesh agents — do not exist until MORROWIND
> ships. Its §4 is therefore a **prediction with its assumptions labelled**, and
> **KENSHI-A's first job is to replace it with measurements.**
>
> **Predecessor:** **Phase CONTROL completes in full, then Phase MORROWIND**
> (`phase_CONTROL.md` §9.1). KENSHI is third and is gated on MORROWIND's
> Tracks 4, 5, 6 and 7 being in tree — there is no scale to measure without
> streaming, animation, AI and particles. **Phase PORTAL** (engineering health,
> CI gates, the `.somtime` parity harness) should land before this phase and
> §9.3 says why it matters more here than anywhere else.
>
> **Record:** this file. Evidence folder `dev records/phase KENSHI/` is created
> by **KENSHI-A**, not before. **Do not invent PNGs, and do not invent
> `.somtime` rows** — this is the phase where a fabricated number does the most
> damage, because every decision in it is downstream of a measurement.
>
> **Do not copy source.** Patterns only, cited in `ATTRIBUTION.md` **§13I** —
> §13E/F belong to Phase 27, §13G to CONTROL, §13H to MORROWIND. The copyleft
> and proprietary rules in `phase_MORROWIND.md` §6.6 carry over unchanged and
> apply to Daemon (GPL), Luanti (LGPL) and Unreal (proprietary).

**The rule this phase is judged by, stated once — the no-speculative-fix rule:**

> **No optimisation is authorized until a measurement names it.** Track 3 is a
> list of fixes this plan *expects* to be needed; every one of them is
> **blocked** until a Track 2 sweep produces a row saying so. A KENSHI sub-phase
> that optimises something because it seemed slow has failed, even if the frame
> got faster — because the phase's product is not a faster frame, it is a
> **known** frame, and a fix applied without a measurement leaves the engine
> exactly as unknown as before.
>
> This is not pedantry. Phase DOOM built tile binning and an aerial terrain
> pipeline, both correct, and shipped both **default off** because they measured
> slower. That result is worth more than either feature, and it only exists
> because DOOM measured first. KENSHI is that discipline applied to a whole
> engine instead of one pass.

**Frozen by this phase** — a KENSHI sub-phase that changes any of these has gone
wrong:

- Every frozen contract inherited from 26/26-Zeta/27 (tokens, paint), XV
  (terrain), IV (water), DOOM (measured defaults), CONTROL (six seams) and
  MORROWIND (eight seams). **KENSHI adds no features to any of them.**
- **The `.somtime` v1 format is extended, never broken.** `timing.rs::parse`
  must keep reading every v1 file in `dev records/`, and the DOOM-A baselines
  stay readable and are never overwritten. §7 Seam 2.
- The visibility-buffer pass order and MORROWIND's Track 7 defaults.
- Public API shape. This phase changes how things are *scheduled and measured*,
  not what they are called. `examples/vvardenfell` compiles unedited at the end
  of it.

---

## 0. How to use this document (handoff)

1. **This file**, all of it — it is shorter than its two predecessors on
   purpose. Especially §4 (**read the labels: half of it is prediction**), §5,
   §7 (three seams), §8, and §9's gating.
2. [`phase_MORROWIND.md`](phase_MORROWIND.md) §14.8 (multi-threaded rendering,
   parked as "a phase of its own" — **this is that phase**), MORROWIND-W2, and
   MORROWIND-AD. Those three are Track 3's expected content.
3. [`phase_DOOM.md`](phase_DOOM.md) §15 and `dev records/phase DOOM/README.md`
   — the `.somtime` harness, the measured defaults, and the negative results.
   **KENSHI is DOOM's method at engine scale**; read DOOM before deciding you
   have a better idea about methodology.
4. [`context.md`](../context.md) §17.7 (Phase 29 — what the profiler is and what
   it admits it cannot see) and §12 (frame execution order).
5. `crates/somnium_renderer/src/profiler.rs` and `timing.rs`, **end to end**.
   Track 1 extends the first and Track 0 extends the second, and neither is
   readable from its API surface alone.
6. **Appendix A**, at the end — the code layer, for a cold session.

**Authorized work:** `profiler.rs`, `timing.rs`, `somnium_jobs`, the scheduling
of existing systems, the scale harness under `tools/`, and — **only where a
Track 2 row names it** — the internals of whatever that row indicts.

**Not authorized:** new engine features, new editor surfaces, new rendering
techniques, or any optimisation without a citation.

---

## 1. Executive decision

### 1.1 The finding, stated as a prediction because that is what it is

After MORROWIND, Somnium will contain: a visibility-buffer renderer with virtual
shadow maps, GPU particles and a GI tier; skeletal animation with blend trees;
a streamed, partitioned world with HLOD and impostors; navmesh agents with
behaviour trees; a runtime UI; and an asset cook with a residency budget.

**Every one of those will have been accepted on a single-feature measurement.**
MORROWIND's §11 row 7 requires a `.somtime` row per sub-phase on the two shipped
maps. That row proves virtual shadow maps did not regress *a scene with no
characters in it*. It says nothing about virtual shadow maps with four hundred
animated characters walking through eight streaming cells while particles burn
and a hundred agents repath.

**Nobody will have run that frame.** Not because it was skipped, but because
until MORROWIND's last track lands there is nothing to run it with.

### 1.2 The decision

**Build the instrument, build the load, find the wall, fix what the wall says.**
Four tracks, in that order, and the order is the phase:

- **Track 0 — THE HUB.** Determinism and the harness. A run that does not
  reproduce cannot be compared, and everything after this depends on comparison.
- **Track 1 — BEEP.** The profiler finished: CPU depth, memory, job queues,
  per-system attribution, capture-to-file, and a remote client.
- **Track 2 — WORLD'S END.** The sweep. Scale every axis until something
  breaks, and publish where.
- **Track 3 — SKELETON.** The fixes, each gated on a Track 2 row.

The phase's deliverable is **a document**: a table of where Somnium's limits
are, on named hardware, with the reasons. The faster frame is a side effect.

### 1.3 Why this and not the shipping phase

The honest alternative was a packaging-and-distribution phase — nothing in three
plan documents produces a distributable artifact, and that is a real hole.

**Scale first, for one reason:** you cannot ship what you have not measured, but
you can measure what you cannot ship. A packaging phase that ships a build which
falls to 12 fps at the content density the engine was designed for has produced
a worse outcome than no packaging phase at all, because now the number is public.
§14.1 keeps shipping as the named successor.

### 1.4 Why the codename is doing work

OGRE is in `example_repo` and has been read exactly once, for HLMS
(`ATTRIBUTION.md` §5). Two things in it are relevant here and neither is HLMS:
its **compositor** node/workspace system (`ogre-next-master/ogre-next-master/OgreMain/src/Compositor/`,
restored to the record in `phase_MORROWIND.md` §6.8.2) and its long institutional
memory about **scene-graph traversal costs at scale**, which is the problem OGRE
spent two major versions on and which Somnium has never had to think about
because it has never had ten thousand nodes.

---

## 2. Goals

1. **A run reproduces.** Same seed, same build, same input trace, same numbers —
   within a stated tolerance. Without this the rest of the phase is anecdote.
2. **A frame is fully attributed.** `unattributed` on both CPU and GPU under a
   stated threshold, memory accounted, jobs visible, and per-system costs
   attributable to the system rather than to a pass.
3. **The limits are known and written down.** For each axis — animated
   characters, streaming cells in flight, particle systems, navmesh agents,
   draw calls, resident bytes, UI widgets — the number at which the frame budget
   breaks, on named hardware, with the subsystem that broke first.
4. **The three parked items land, or are refused with a measurement.**
   Multi-threaded recording (MORROWIND §14.8), the pose task graph
   (MORROWIND-W2), virtual texturing (MORROWIND-AD). **Refusing one on evidence
   is a success**, on the DOOM tile-binning precedent.
5. **Nothing regresses.** GHOSTFENCE, inherited from MORROWIND §10, including
   its golden-image row.

---

## 3. Non-goals

1. **No new features.** Not one. If a Track 2 sweep reveals a missing capability
   rather than a slow one, that is a finding for the *next* phase and gets
   written down, not built.
2. **No packaging or distribution.** §14.1.
3. **No networking.** Still parked (MORROWIND §14.1). Note that determinism
   work in Track 0 makes it *cheaper* later; that is a side effect, not a goal,
   and no netcode is written.
4. **No console or mobile port**, though Track 2's harness is written so a
   second hardware target is a config entry rather than a rewrite (§7 Seam 2).
5. **No optimisation without a citation.** The judging rule. It is a non-goal
   because it is the thing this phase will be most tempted to do.
6. **No profiler rewrite.** Phase 29's design — deferred readback, three-deep
   ring, thirty-frame smoothing — is correct and is *extended*, not replaced.

---

## 4. The audit — half measured, half predicted, labelled throughout

**This section is structurally weaker than the equivalent section in
`phase_CONTROL.md` §4 or `phase_MORROWIND.md` §4, and pretending otherwise would
be the single most damaging thing this document could do.** Those two measured a
tree that existed. This one cannot. Every claim below is tagged:

- **[M]** measured on the tree at `7c0b66f`, 2026-08-23. Trustworthy.
- **[P]** predicted about the post-MORROWIND tree. **Unverified by construction.**
  KENSHI-A replaces every one of these with a measurement.

### 4.1 What the profiler has **[M]**

`crates/somnium_renderer/src/profiler.rs` provides:

- A `Timeline` of GPU scopes with `begin(name)` / `end()`, depth-carrying so the
  overlay can draw a frame graph (the Flax idea, `context.md` §17.7).
- `GpuProfiler` with deferred readback through a three-deep ring of mapped
  buffers, thirty-frame smoothing, and a guard against nonsense timestamps.
- `cpu_begin` / `cpu_end` for CPU zones.
- `FrameCounters` — draws, triangles, instances, TLAS instances — travelling
  *with* the timings, because "a pass took 0.9 ms" never says why.
- `total_ms()` and **`unattributed_ms()`**, printed instead of a total, which
  `context.md` §17.7 calls "the honest statement of how much of the frame the
  profiler still cannot see."
- Requires `TIMESTAMP_QUERY_INSIDE_ENCODERS`, detected and never demanded.

### 4.2 What the profiler does **not** have **[M]**

Read against what Track 2 will need:

| Missing | Consequence at scale |
|---|---|
| **Any memory accounting** | "Why is this build 6 GB" is unanswerable. There is no allocation tracking, no GPU-residency total, no per-subsystem budget. MORROWIND-R adds a residency panel for *assets*; nothing covers the rest. |
| **A CPU flame graph with real depth** | `cpu_begin`/`cpu_end` exist, but §17.7's own list of CPU zones is short (instances, cluster cull, foliage, lighting extra). With ten thousand agents the interesting cost is in gameplay, not in those four. |
| **Job-queue visibility** | Jobs do not exist yet; MORROWIND-B adds them and makes profiler reporting mandatory (`JobDesc::name`). This phase has to *render* that: queue depth, wait time, deadline misses, cancellations. |
| **Capture to file** | Thirty-frame smoothing answers "what is happening now." It cannot answer "what happened during that hitch four seconds ago." There is no ring buffer of frames and no capture trigger. |
| **Per-system attribution** | Costs are attributed to *passes*. At scale the question is "what do four hundred characters cost," which crosses animation, skinning, culling, shading and physics. Nothing aggregates along that axis. |
| **A remote client** | The profiler draws into the editor's own overlay, so it perturbs what it measures and cannot profile a headless or packaged run. |

### 4.3 What `.somtime` is **[M]**

`crates/somnium_renderer/src/timing.rs`: a v1 text format, `Row` and `Run`,
`parse`, `compare(before, after)`, driven by
`SOMNIUM_TIME=out.somtime SOMNIUM_TIME_COMPARE=before.somtime`, with
`unattributed_pct` and `frame_ms` accessors and `rows_from_scopes(kind, scopes)`
to build rows from profiler output. Deterministic runs with a stddev per row.

**It has no scale axis.** A `.somtime` file describes one configuration. There
is no way to express "this row, at 100 / 200 / 400 / 800 characters" and
therefore no way to state a *scaling curve*, which is the only interesting shape
in this phase. §7 Seam 2 extends the format to v2 for exactly this.

### 4.4 What will be unmeasured after MORROWIND **[P]**

The predicted list, and KENSHI-A's job is to turn it into a measured one:

| Axis | Why it is a wall candidate |
|---|---|
| Animated characters | Pose evaluation is a recursive tree walk until MORROWIND-W2 lands, which that sub-phase explicitly allows to be deferred. Plus skinning bandwidth, whichever design MORROWIND-U picked. |
| Streaming cells in flight | Load, GPU upload and entity re-instantiation all land on the main thread through `drain_completions(budget)`. A budget that is always exhausted is a queue that never drains. |
| Navmesh agents | Path queries are per-agent; avoidance is pairwise unless spatially partitioned. |
| Particle systems | GPU-simulated, so the CPU cost is submission and the GPU cost is fill rate — two different walls with the same name. |
| Draw calls / instances | The visibility buffer is GPU-driven, so this should scale well. **"Should" is the word this phase exists to delete.** |
| Resident bytes | MORROWIND-R sets a budget; nothing has tested what happens when it is genuinely full and eviction thrashes. |
| UI widgets | Track 1 of MORROWIND adds a game UI. A thousand-widget inventory screen has never existed. |
| Physics bodies | Jolt scales well and Somnium uses ~236 lines of it. Probably not the wall; measure anyway, because "probably" is how DOOM's tile binning got built. |

### 4.5 The one thing that is certain **[M]**

`crates/somnium_renderer/src/jobs.rs:3` says it in its own doc comment:
*"Parallel work is CPU-side only… **Record still happens on the render thread**."*

Whatever else the sweep finds, single-threaded command recording is a known
ceiling, it is documented in the source, and MORROWIND §14.8 parked the fix as
"a phase of its own." **This is that phase**, and Track 3's first candidate.

---

## 5. What "scale" means here

Three distinct failure modes, which need distinguishing because they have
different fixes and Track 2 must report which one it found:

**A cliff.** Performance is flat, then falls off. Almost always a capacity being
exceeded — a cache evicting, a buffer resizing, a budget exhausting, an atlas
overflowing. The fix is usually a bigger or smarter capacity, and the
*measurement* matters more than the fix because the cliff position is the number
worth publishing.

**A slope.** Cost rises linearly with N and the constant is too big. The fix is
optimisation in the ordinary sense. This is the mode everyone expects and the
least interesting one.

**A curve.** Cost rises faster than N — pairwise avoidance, an O(n²) broadphase,
a per-frame sort that was fine at fifty. The fix is algorithmic and the phase
should be *delighted* to find one, because it is the highest-value result per
line changed.

**Track 2 reports which of the three it saw, for every axis.** A row that says
"slow at 400 characters" without saying which shape is an unfinished row.

---

## 6. References

Shorter than its predecessors', because this phase is mostly method rather than
architecture. Verified by listing on 2026-08-23 unless marked.

- **`panda3d-master/panda/src/pstatclient/`** — `pStatClient.cxx`,
  `pStatClientImpl.cxx`, `pStatCollector`, `pStatClientControlMessage.cxx`. A
  **networked** profiler: the engine is a thin client streaming to a separate
  GUI application. This is Track 1's remote-client model and the reason it is
  worth building — you cannot attach an in-editor overlay to a packaged build,
  and an overlay perturbs the frame it measures.
- **`panda3d-master/panda/src/pipeline/`** — `pipelineCyclerTrueImpl.cxx`,
  `cycleData.cxx`, `cyclerHolder.cxx`. Pipeline cycling: one copy of every piece
  of scene state per pipeline stage, so App/Cull/Draw read different consistent
  snapshots without locks. **Track 3's design for MORROWIND §14.8**, already
  identified there and not re-litigated here.
- **`WickedEngine-master/WickedEngine/wiProfiler.cpp`** — already cited by
  Phase 29 for deferred readback; re-read for its CPU/GPU unification.
- **`FlaxEngine-master/Source/Engine/Profiler/`** (`ProfilerGPU.h`,
  `RenderStats.h`) — already cited for event depth and counters. Track 1 extends
  the counter set rather than inventing one.
- **`Esoterica-main/Code/Engine/Animation/TaskSystem/`** — pose evaluation as a
  task graph. **MORROWIND-W2's content**, inherited here if W2 was deferred.
- **`ogre-next-master/ogre-next-master/OgreMain/src/Compositor/`** — the
  node/workspace frame graph. Read **only** if a Track 2 row indicts pass
  scheduling; `phase_MORROWIND.md` §5.4 refuses a render graph and that refusal
  stands unless measured wrong.
- **`luanti-master/src/profiler.cpp`** and `src/threading/` (**LGPL, pattern
  only**) — a shipping streamed-world engine's own profiler and threading model.
- **`o3de-development/Gems/Profiler`** and `Code/Framework/AzCore/IO/Streamer/`
  — the streaming stack whose deadline model MORROWIND-B adopted; re-read for
  what it *reports*, which is the part MORROWIND did not take.
- **Public literature, not in the tree:** Tracy and Superluminal as the
  contemporary bar for what a frame profiler shows; `RenderDoc` for GPU capture.
  **Not verified for this document** — a Track 1 sub-phase should look at
  Tracy's wire protocol before designing Seam 3's, because it is the most
  battle-tested answer to the same problem and there is no reason to invent one.

---

## 7. The three seams

### Seam 1 — A run is `(build, scene, seed, input trace, hardware)` and nothing else

```rust
pub struct RunSpec {
    pub build: BuildId,          // git hash + profile + feature flags
    pub scene: SceneId,
    pub seed: u64,               // every RNG in the engine derives from this
    pub input: InputTrace,       // recorded, replayed frame-exactly
    pub hardware: HardwareId,    // adapter name + driver version
    pub scale: ScaleVector,      // Seam 2
}
```

Two consequences, both non-negotiable:

- **Every source of non-determinism is enumerated and controlled**: RNG seeding,
  frame timing (fixed timestep for the sim, decoupled from render), job
  completion order (`somnium_jobs`'s single-threaded mode from MORROWIND-B, or a
  deterministic merge), iteration order over hash maps, and float
  reassociation across thread counts.
- **Input is a trace, not a human.** A sweep that requires someone to walk the
  same path twice is not a sweep. This is also what makes a hitch reproducible,
  which is the difference between fixing it and describing it.

**Determinism is scoped honestly:** *reproducible on one machine with one build*
is the target. Cross-platform bit-exactness is not (§14.4).

### Seam 2 — `.somtime` v2 adds a scale axis, and reads every v1 file

```
# somtime v2
# label     crowd-sweep
# build     7c0b66f/release
# hardware  "NVIDIA RTX 4070" 552.22
# seed      0x5EED
# axis      characters 100 200 400 800 1600
pass          n=100    n=200    n=400    n=800    n=1600   stddev
shading       2.14     2.19     2.31     2.55     3.02     0.04
skinning      0.31     0.62     1.24     2.51     5.06     0.02
anim.eval     0.44     0.91     1.98     4.40     11.20    0.09   <- curve
```

Rules: **v1 files parse unchanged** (`timing.rs::parse` gains a version branch,
and the DOOM-A baselines are never rewritten); a v2 file with one column is
exactly a v1 file semantically; `compare` works within a version and refuses
across; and **the shape classification of §5 is computed, not eyeballed** — a
column set with a fitted exponent so `anim.eval` above is *reported* as
super-linear rather than noticed by a human.

### Seam 3 — The profiler emits a stream, and the overlay is one consumer

Today `GpuProfiler` owns both measurement and presentation. Track 1 splits them:

```rust
pub enum ProfileEvent {
    ZoneBegin { name: &'static str, depth: u8, t: u64, thread: ThreadId },
    ZoneEnd   { t: u64 },
    Counter   { name: &'static str, value: i64 },
    Alloc     { subsystem: SubsystemId, bytes: i64 },   // negative = free
    JobState  { queue: Priority, depth: u32, waited_us: u32 },
    FrameMark { index: u64 },
}
```

Consumers: the in-editor overlay (unchanged behaviour), a ring buffer for
capture-to-file, and a socket for the remote client. **The engine does not know
which are attached**, which is what stops the profiler perturbing the thing it
measures — the overlay stops being mandatory.

Panda3D's `pStatClient` is the model (§6). Look at Tracy's protocol before
designing the wire format.

---

## 8. Sub-phases

Fifteen, across four tracks. Every sub-phase closes with: an artefact, a
`.somtime` v2 file where it touches the frame, a GHOSTFENCE run, an
`ATTRIBUTION.md` §13I entry, and a `context.md` update.

Sub-phase names are Kenshi places.

### Track 0 — THE HUB (determinism and the harness)

#### KENSHI-A — Replace §4's predictions with measurements

**No other sub-phase starts until this exists.** §4 is half prediction and this
is where that debt is paid.

1. Re-run MORROWIND's census script against the post-MORROWIND tree; produce the
   real crate/line/test table and the real feature inventory.
2. **Measure every [P] row in §4.4** at whatever scale the shipped
   `examples/vvardenfell` reaches, and rewrite §4 in place with [M] tags.
3. Inventory what the profiler *actually* covers after MORROWIND — the
   `unattributed` figure on CPU and GPU is the headline number of this
   sub-phase and every later one is measured against it.
4. Create `dev records/phase KENSHI/`, open `ATTRIBUTION.md` §13I.
5. **Name the target hardware.** Every number in this phase is meaningless
   without it, and "the dev machine" is not a specification.

**Exit:** §4 contains no [P] tags.

#### KENSHI-B — Determinism (Seam 1)

Fixed-timestep simulation decoupled from render; one seeded RNG root with
per-system derivation; input recording and frame-exact replay; deterministic
iteration order wherever it affects simulation; `somnium_jobs` in deterministic
mode for replay runs.

**Exit:** the same `RunSpec` replayed ten times produces frame times whose
per-row stddev is under a stated threshold **and** identical simulation state at
the final frame. The second half is the real test; the first is noise
measurement.

#### KENSHI-C — `.somtime` v2 and the sweep harness (Seam 2)

The format, the parser branch, the v1 compatibility tests, the curve fitter, and
a `tools/sweep/` runner that takes a `RunSpec` plus an axis and produces one v2
file. Plus the CI hook — **and this is where Phase PORTAL's absence would hurt
most** (§9.3).

**Exit:** one command produces the crowd-sweep table in Seam 2's example.

#### KENSHI-D — The scale rig

The content. A generated world at parameterised density: N characters with
animation graphs and nav agents, M cells, K emitters, W UI widgets, P physics
bodies — each axis independently dialable, seeded, and **built only from public
crate APIs** so it inherits MORROWIND's second-example rule.

**This is content work and it is the sub-phase most likely to be
under-estimated.** A rig that is not representative produces confident wrong
answers; one that is hand-tuned to look good produces flattering ones. It should
be generated from a seed, not authored.

### Track 1 — BEEP (the profiler, finished)

*Beep follows you everywhere and comments on what he sees.*

#### KENSHI-E — The event stream (Seam 3)

Split measurement from presentation. Existing overlay behaviour is unchanged and
GHOSTFENCE's golden-image row proves it.

#### KENSHI-F — CPU depth and per-system attribution

Real nested CPU zones across gameplay, animation, physics, streaming and UI; and
an aggregation axis that is not the pass list, so "what do 400 characters cost"
is answerable across the five subsystems it touches.

#### KENSHI-G — Memory

Per-subsystem allocation tracking, GPU residency totals, and budgets with
overrun reporting. Closes §4.2's first row, which is the largest single hole in
the current profiler.

#### KENSHI-H — Job-queue visibility

Queue depth, wait time, deadline misses, cancellation counts, and
`drain_completions` budget exhaustion — rendered. MORROWIND-B made job names
mandatory for exactly this.

#### KENSHI-I — Capture, and the remote client

A ring buffer of N frames with a trigger (manual, or automatic on a frame over
budget), written to a file; and a socket consumer so a headless or packaged run
can be profiled. Panda3D's `pStatClient` is the model; look at Tracy's protocol
first.

**Exit:** a hitch that happened four seconds ago can be examined.

### Track 2 — WORLD'S END (the sweep)

*Where the map stops.*

#### KENSHI-J — The sweep, and the limits document

Run every axis in §4.4 to breaking point. For each: the number, the subsystem
that broke first, and **which of §5's three shapes it was.**

The deliverable is `dev records/phase KENSHI/limits.md`, and it is the phase's
actual product. It should be publishable — the kind of document that tells a
person choosing an engine what it does at their content density, which almost
no open-source engine provides.

#### KENSHI-K — The combined-load frame

The frame nobody has run: every axis simultaneously at 60% of its individual
limit. Interactions are the point — cache pressure, bandwidth contention, budget
competition between streaming uploads and skinning uploads. **Expect this to be
worse than the individual limits predict, and expect the reason to be
uninteresting and fixable.**

#### KENSHI-L — The triage

Rank findings by (frame time recovered ÷ cost) and by shape, since a curve
usually beats a slope. **This sub-phase produces Track 3's authorization list.
Nothing in Track 3 may proceed without an entry here.**

### Track 3 — SKELETON (the fixes, each gated)

*The ancient machines that still run.*

**Every sub-phase below is `BLOCKED` until KENSHI-L names it.** They are listed
because they are the plan's *expectations*, and writing expectations down is how
they stay falsifiable — if the sweep does not indict them, they do not happen,
and that is a result worth recording.

#### KENSHI-M — Multi-threaded recording *(expected; MORROWIND §14.8)*

Panda3D pipeline cycling: one copy of scene state per pipeline stage, App/Cull/
Draw reading consistent snapshots without locks. **The largest change in the
phase** and the one whose blast radius is every piece of scene state. If
KENSHI-L does not indict single-threaded recording, **do not do this**, and
record that `jobs.rs:3`'s documented ceiling was not the binding one — which
would be a genuinely surprising and valuable result.

#### KENSHI-N — The pose task graph *(expected; MORROWIND-W2)*

Pose evaluation as a job graph rather than a recursive walk (Esoterica's
`TaskSystem/`), plus clip compression. Indicted by an `anim.eval` row with a
super-linear shape — the exact case Seam 2's example shows.

#### KENSHI-O — Virtual texturing *(expected; MORROWIND-AD)*

Indicted by a resident-bytes cliff. MORROWIND called it "the most expensive item
and the most deferrable"; if the sweep shows the terrain material's memory is
the wall, it stops being deferrable.

#### KENSHI-P — Whatever else the sweep found

Deliberately unnamed. **If this sub-phase is empty at the end of the phase, the
sweep was not aggressive enough**, because an engine that scales exactly as its
authors predicted has not been measured, it has been confirmed.

---

## 9. Sequencing

### 9.1 The gate

```
Phase CONTROL complete ──► Phase MORROWIND complete ──► KENSHI
                                    │
   MORROWIND Tracks 4,5,6,7 ────────┘  (streaming, animation, AI, particles)
```

**KENSHI cannot start early in a reduced form.** A sweep with no crowds and no
streaming measures a scene Somnium already measures today. If MORROWIND ran only
§9.3's eleven-sub-phase cut, KENSHI's axes shrink to whatever landed, and
**KENSHI-A must say so in writing** rather than sweeping a diminished rig and
reporting the numbers as if they were the whole engine.

### 9.2 Internal order

`A → B → C → D` is strict: measurements need a harness, a harness needs
determinism, determinism needs the census. Track 1 (E–I) may run in parallel with
D. Track 2 needs Track 0 complete and Track 1 at least through G. Track 3 needs
KENSHI-L, without exception.

### 9.3 Why PORTAL matters more here than anywhere else

Phase PORTAL builds CI gates that can actually fail and a capture + `.somtime`
parity harness. **KENSHI produces more numbers than any previous phase and every
one of them rots.** A limits document that is not re-run by CI is accurate on the
day it is written and misleading a month later. If PORTAL has not landed, KENSHI-C
should build the minimum CI hook itself and say plainly that it is a stopgap
occupying PORTAL's territory.

---

## 10. Must-not-break

GHOSTFENCE, inherited from `phase_MORROWIND.md` §10 in full, plus:

| Invariant | Check | Owner |
|---|---|---|
| `.somtime` v1 files still parse | Every v1 file in `dev records/` round-trips; DOOM-A baselines byte-identical | KENSHI-C |
| The profiler does not perturb the frame | Frame time with the profiler disabled, enabled-overlay, and enabled-remote, all three reported | Track 1 |
| Determinism does not cost the frame | Fixed timestep and seeded RNG measured against the pre-B baseline | KENSHI-B |
| Public API unchanged | `examples/vvardenfell` compiles unedited | All |
| No feature was added | The census diff shows no new public capability | All |

---

## 11. Acceptance

| # | Row | Applies to |
|---|---|---|
| 1 | §4 contains no `[P]` tags | KENSHI-A |
| 2 | Ten replays agree within the stated tolerance, and end-state is identical | KENSHI-B |
| 3 | Every claim is a `.somtime` v2 row with a stddev and named hardware | All |
| 4 | Every Track 2 row states which of §5's three shapes it found | Track 2 |
| 5 | **No Track 3 sub-phase proceeded without a KENSHI-L entry** | Track 3 |
| 6 | A refusal on evidence is recorded as prominently as a fix | Track 3 |
| 7 | `limits.md` exists and is publishable | KENSHI-J |
| 8 | GHOSTFENCE passes | All |
| 9 | `ATTRIBUTION.md` §13I written; `context.md` updated | All |

---

## 12. Risks and controls

**12.1 The rig is not representative.** The central risk: a generated world that
is uniformly dense measures a game nobody makes. Control: KENSHI-D generates
from a seed with *clustered* density, and KENSHI-J reports the distribution it
used so a reader can judge it.

**12.2 The phase becomes an optimisation phase.** The failure mode the judging
rule exists to prevent. Control: acceptance row 5, and Track 3 sub-phases
literally marked `BLOCKED` in the document.

**12.3 Determinism is more expensive than it looks.** Fixed timestep touches
every system that currently reads a delta; hash iteration order is a long tail.
Control: KENSHI-B scopes to *one machine, one build* (§14.4), and if the cost
lands in the frame, §10's row makes it visible rather than silent.

**12.4 Pipeline cycling is a rewrite wearing a sub-phase's clothes.** KENSHI-M
touches every piece of scene state. Control: it is gated on KENSHI-L; it is
prototyped on **one** subsystem first with a measurement before generalising;
and the fallback — recording a subset of passes in parallel — is available and
much smaller.

**12.5 The numbers are hardware-specific and read as universal.** Control:
KENSHI-A names the hardware; every `.somtime` v2 header carries it; `limits.md`
leads with it.

**12.6 MORROWIND shipped reduced.** §9.1. Control: KENSHI-A states which axes
exist and which do not, and the limits document is explicit about its own scope.

---

## 13. Evidence plan

`dev records/phase KENSHI/`, created by KENSHI-A.

- **Track 0**: the rewritten §4; a determinism report (ten replays, per-row
  stddev, final-state hash); a v1-compatibility test log.
- **Track 1**: overlay before/after (golden-image identical); a capture file
  from a real hitch; a screenshot of the remote client attached to a headless
  run; the three-way profiler-overhead table.
- **Track 2**: **`limits.md`** — the phase's product. One `.somtime` v2 file per
  axis, plus the combined-load file. Curve plots per axis with the fitted shape.
- **Track 3**: per fix, the KENSHI-L citation that authorized it, before/after v2
  files, and the GHOSTFENCE run. **Refusals get the same treatment** — the DOOM
  precedent is that a negative result is evidence, not an absence of it.

---

## 14. Left open, deliberately

**14.1 Packaging, distribution and platform targets.** The named successor —
this is the phase after KENSHI. Nothing in CONTROL, MORROWIND, PORTAL or KENSHI
produces a distributable artifact: no export, no installer, no content patching
(Defold's `liveupdate`, flagged in MORROWIND-Q as *must not preclude*), no crash
reporting, no save migration across engine versions, no console or mobile target.
KENSHI's remote profiler (KENSHI-I) is the first piece of the tooling that phase
will need, which is a small argument for this order.

**14.2 Networking.** Still parked (MORROWIND §14.1). KENSHI-B's determinism work
makes it materially cheaper later — rollback netcode needs exactly a fixed
timestep, a seeded RNG and replayable input — but no netcode is written and none
should be started on the strength of that.

**14.3 Hot reload with live state migration** (MORROWIND §14.11) and
**sandboxed native gameplay code** (§14.9). Both still out.

**14.4 Cross-platform determinism.** KENSHI-B targets one machine and one build.
Bit-exact reproduction across platforms requires controlling float
reassociation, library versions and driver behaviour, and it is a phase-sized
problem that only matters if §14.2 ever happens.

**14.5 The render graph.** MORROWIND §5.4 refused one. That refusal stands
**unless a Track 2 row indicts pass scheduling specifically**, which is the only
evidence that would reopen it — and §6 says which OGRE files to read if it does.

---

## 15. Start checklist

1. Read this file, `phase_MORROWIND.md` §14.8 / W2 / AD, `phase_DOOM.md` §15,
   `context.md` §17.7, and `profiler.rs` + `timing.rs` end to end.
2. Confirm **Phase MORROWIND is complete**, and specifically that Tracks 4, 5, 6
   and 7 are in tree. If MORROWIND ran reduced, write down which axes exist
   before touching anything.
3. Run KENSHI-A. **Do not trust §4** — half of it was written before the tree it
   describes existed, and the tags say which half.
4. Name the target hardware, in writing, before the first measurement.
5. Confirm `dev records/phase KENSHI/` exists and contains no invented `.somtime`
   files. This phase is the one where a fabricated number does the most damage.
6. Re-read the judging rule at the top. **The temptation in this phase is to
   fix things.** The product is a known frame, not a fast one.

---

## 16. Research sources and confidence

**Measured on the tree at `7c0b66f`, 2026-08-23** (high confidence): everything
in §4.1, §4.2, §4.3 and §4.5 — the `profiler.rs` and `timing.rs` API surfaces,
the `.somtime` v1 environment variables, and the `jobs.rs:3` doc comment.

**Quoted from `context.md` §17.7** (high confidence, second-hand): the deferred
readback design, the `TIMESTAMP_QUERY_INSIDE_ENCODERS` requirement, the
thirty-frame smoothing, the silent-failure bug, and the sample frame breakdown.

**Verified by listing, 2026-08-23** (medium-high): `panda3d/panda/src/pstatclient/`
and `panda/src/pipeline/`; `ogre-next-master/ogre-next-master/OgreMain/src/Compositor/`.
Contents not read.

**Predicted, and tagged `[P]` throughout §4.4** (**low confidence by
construction**): every claim about the post-MORROWIND tree. This is not a
research failure; the tree does not exist. KENSHI-A converts them.

**Not verified at all**: Tracy's and Superluminal's designs, cited in §6 as
worth looking at before designing Seam 3's wire protocol. No source was consulted
for either; treat both as leads.

---

# Appendix A — Implementation reference

*The code layer, for a session picking this up cold. §§0–16 are the plan; where
the two disagree, §§0–16 win.*

## A.1 Read these six things first

| # | Path | Read for | Approx |
|---|---|---|---|
| 1 | `crates/somnium_renderer/src/profiler.rs` | `Timeline`, `GpuProfiler`, `ScopeResult`, `FrameCounters`, `cpu_begin`/`cpu_end`, `unattributed_ms`. **Track 1 extends every one of these** | 1,132 ln |
| 2 | `crates/somnium_renderer/src/timing.rs` | `Row`, `Run`, `parse`, `compare`, `TimingRun::from_env`, `rows_from_scopes`. **Track 0 extends this to v2** | 693 ln |
| 3 | `context.md` §17.7 | Why the profiler is shaped as it is, and the silent-failure bug | — |
| 4 | `dev records/phase DOOM/README.md` | The `.somtime` baselines that must never be overwritten, and DOOM's negative results | — |
| 5 | `crates/somnium_jobs/` *(exists after MORROWIND-B)* | `JobDesc::name`, `Priority`, `drain_completions`. Track 1's KENSHI-H renders what this reports | — |
| 6 | `crates/somnium_core/src/time.rs` | The current delta-time path — KENSHI-B replaces it with a fixed timestep | 237 ln |

## A.2 Seam 1 — determinism

```rust
// crates/somnium_core/src/determinism.rs
pub struct RunSpec {
    pub build: BuildId,        // git hash + cargo profile + enabled features
    pub scene: SceneId,
    pub seed: u64,
    pub input: InputTrace,
    pub hardware: HardwareId,  // adapter.get_info(): name, driver, backend
    pub scale: ScaleVector,
}

/// Every RNG in the engine derives from the root. Nothing calls `thread_rng`.
pub struct RngRoot(u64);

impl RngRoot {
    /// Per-system streams so adding a system does not shift another's sequence
    /// — the classic way a "deterministic" build stops reproducing.
    pub fn stream(&self, system: &'static str) -> Rng {
        Rng::seed_from_u64(self.0 ^ fnv1a(system.as_bytes()))
    }
}
```

**Fixed timestep**, with render decoupled:

```rust
// The sim advances in fixed steps; the renderer interpolates between the last
// two states. `accumulator` carries the remainder across frames.
const SIM_HZ: f64 = 60.0;
const SIM_DT: f64 = 1.0 / SIM_HZ;
const MAX_STEPS: u32 = 8;   // spiral-of-death guard: drop time, never stall

accumulator += frame_dt.min(0.25);
let mut steps = 0;
while accumulator >= SIM_DT && steps < MAX_STEPS {
    world.step(SIM_DT);      // <- the only place sim time advances
    accumulator -= SIM_DT;
    steps += 1;
}
let alpha = (accumulator / SIM_DT) as f32;   // render interpolation factor
```

**The non-determinism checklist**, which is the actual work of KENSHI-B — each
row is a real source and each has bitten a real engine:

| Source | Fix |
|---|---|
| `HashMap` iteration order feeding simulation | `BTreeMap`, or sort before iterating. `HashMap` is fine for lookup, never for order |
| `thread_rng` / `SystemTime` seeding | One `RngRoot`; grep for both and allow neither |
| Job completion order | `somnium_jobs` deterministic mode, or a stable merge on the main thread |
| Variable delta time | Fixed timestep above |
| Float reassociation across thread counts | Pin the sim to a fixed reduction order, or accept and scope to one machine (§14.4) |
| Entity id reuse | Generational handles already prevent this — verify, do not assume |
| Iteration over a `Vec` mutated by a parallel pass | Collect, sort by stable id, then apply |

**Verification, and it is the real acceptance test:**

```rust
#[test]
fn replay_is_deterministic() {
    let spec = RunSpec::fixture();
    let a = run_headless(&spec, 600);   // 10 s at 60 Hz
    let b = run_headless(&spec, 600);
    assert_eq!(a.state_hash, b.state_hash);          // <- the one that matters
    assert!(a.frame_times.rms_diff(&b.frame_times) < TOLERANCE); // noise, not correctness
}
```

`state_hash` is a stable hash over every simulated component, in `StableId`
order then `FieldId` order — reusing CONTROL Seam 1's vocabulary so it costs
almost nothing to write.

## A.3 Seam 2 — `.somtime` v2

The v1 parser must keep working, so version-branch at the header:

```rust
// crates/somnium_renderer/src/timing.rs
pub enum Run { V1(RunV1), V2(RunV2) }

pub fn parse(text: &str) -> Run {
    match text.lines().next() {
        Some("# somtime v2") => Run::V2(parse_v2(text)),
        _                    => Run::V1(parse_v1(text)),   // unversioned == v1
    }
}

pub struct RunV2 {
    pub label: String,
    pub build: String,
    pub hardware: String,
    pub seed: u64,
    pub axis: Axis,                 // name + the n values
    pub rows: Vec<RowV2>,
}

pub struct RowV2 {
    pub pass: String,
    pub ms: Vec<f32>,               // one per axis point; len == axis.points.len()
    pub stddev: f32,
    pub shape: Shape,               // computed, not authored
}
```

**Shape classification** — §5's three modes, computed so nobody eyeballs a
curve:

```rust
pub enum Shape {
    Flat,                  // exponent ~0: cost independent of n
    Linear   { per_unit: f32 },
    Super    { exponent: f32 },     // > 1.2 — the interesting one
    Cliff    { at: u32 },           // a step change between adjacent points
}

/// Log-log least squares gives the exponent; a cliff is detected first,
/// because a cliff fits a bad power law and would be misreported as `Super`.
pub fn classify(axis: &Axis, ms: &[f32], stddev: f32) -> Shape {
    if let Some(at) = detect_step(axis, ms, stddev) { return Shape::Cliff { at }; }
    let e = loglog_slope(axis.points(), ms);
    match e {
        e if e < 0.2 => Shape::Flat,
        e if e < 1.2 => Shape::Linear { per_unit: linear_fit(axis.points(), ms) },
        e            => Shape::Super { exponent: e },
    }
}
```

`detect_step` before the power fit is not a detail — it is the difference
between reporting "super-linear, exponent 1.9" (which sends someone hunting an
algorithm) and "cliff at n=512" (which sends them to look for a capacity).

**Compatibility test that must exist**, because §10 makes it an invariant:

```rust
#[test]
fn every_v1_file_in_dev_records_still_parses() {
    for path in glob("../../dev records/**/*.somtime") {
        let text = fs::read_to_string(&path).unwrap();
        assert!(matches!(parse(&text), Run::V1(_) | Run::V2(_)), "{path:?}");
    }
}
```

## A.4 Seam 3 — the profiler event stream

Split measurement from presentation without changing what the overlay draws:

```rust
// crates/somnium_renderer/src/profile/stream.rs
pub trait ProfileSink: Send {
    fn emit(&mut self, ev: ProfileEvent);
    fn frame_end(&mut self) {}
}

pub struct ProfileBus { sinks: Vec<Box<dyn ProfileSink>> }

impl ProfileBus {
    /// Hot path. With no sinks attached this is a length check and a return —
    /// which is what makes the profiler optional rather than mandatory, and is
    /// the whole point of the split (§10: "does not perturb the frame").
    #[inline]
    pub fn emit(&mut self, ev: ProfileEvent) {
        if self.sinks.is_empty() { return; }
        for s in &mut self.sinks { s.emit(ev.clone()); }
    }
}
```

Three sinks ship:

```rust
pub struct OverlaySink { /* existing GpuProfiler presentation, unchanged */ }

pub struct RingSink {
    frames: VecDeque<FrameRecord>,   // capacity N
    trigger: Trigger,                // Manual | OverBudget(Duration)
}

pub struct RemoteSink { stream: TcpStream, /* framed, length-prefixed */ }
```

`RingSink` is what answers "what happened during that hitch four seconds ago" —
the §4.2 gap that thirty-frame smoothing structurally cannot close. Its trigger
firing writes the whole ring to a file.

**Counters extend rather than replace `FrameCounters`.** The existing set —
draws, triangles, instances, TLAS instances — was chosen because "a pass time
says how long, never why" (`context.md` §17.7). At scale the missing ones are:
`characters_animated`, `poses_evaluated`, `cells_resident`, `cells_in_flight`,
`agents_repathed`, `particles_alive`, `widgets_laid_out`, `jobs_queued`,
`jobs_deadline_missed`, `bytes_uploaded`.

## A.5 The sweep harness

```
tools/sweep/
  spec/crowd.toml          # RunSpec + axis definition
  spec/streaming.toml
  spec/combined.toml
  src/main.rs              # runs headless, N reps per point, writes one .somtime v2
```

```toml
# tools/sweep/spec/crowd.toml
label    = "crowd-sweep"
scene    = "vvardenfell"
seed     = 0x5EED
reps     = 5              # repetitions per point; stddev comes from these
warmup   = 120            # frames discarded before measuring
measure  = 300            # frames measured
[axis]
name   = "characters"
points = [100, 200, 400, 800, 1600]
[fixed]                   # everything not being swept is pinned
cells      = 9
emitters   = 4
agents     = 50
```

Two rules that keep results honest and are easy to omit:

- **Warmup is discarded.** Shader variants compile on first use (MORROWIND-C),
  streaming fills, TAA converges, and caches warm. Measuring frame 1 measures
  none of the things this phase cares about.
- **Everything not swept is pinned**, and the pinned values are printed in the
  header. A sweep where two axes moved is not a sweep.

## A.6 How to tell this phase is being done wrong

The failure modes are specific and each has a tell:

| Tell | What went wrong |
|---|---|
| A Track 3 sub-phase is in progress and `limits.md` has no row indicting it | The judging rule was broken. Stop. |
| A `.somtime` v2 file has no `hardware` header | The number is unfalsifiable |
| Frame times improved and no `.somtime` was captured | An optimisation phase in disguise |
| KENSHI-P is empty | The sweep stopped at comfortable numbers |
| The determinism test asserts on frame times but not `state_hash` | Noise was measured; determinism was not |
| `unattributed` is unchanged from KENSHI-A | Track 1 added presentation, not coverage |
| A fix was applied and refused fixes are not recorded | DOOM's precedent — negative results are the evidence, not the absence of it |
