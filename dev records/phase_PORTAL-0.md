# Phase PORTAL-0 — measure, then delete

> **Codename:** PORTAL-0 (Valve, Source). The half-life of a performance claim
> is one session, and Source shipped with `+showbudget` because Valve did not
> trust its own memory either.
>
> **Status:** **A, B, C, D, E, F, G run 2026-08-29** against `dev` at `439b6b6`
> (`feat(renderer): stream terrain material pages`). **B, C and D are in tree.
> E and G are negative results and are not.** F is a decision with its gates
> measured; the default flip itself is not taken here and is stated in §F.
>
> **Position:** before MORROWIND-AC, per
> `MORROWIND-PRE-AC_PERFORMANCE_CLAUDE_READ_FIRST.md` §7. It starts no AC work
> and redesigns nothing around a speculative optimisation.
>
> Evidence: `dev records/phase PORTAL-0/`. Hardware for every number below:
> NVIDIA GeForce RTX 5080 Laptop GPU / Vulkan, driver 610.74, render
> **1920×1032** (maximised, `SOMNIUM_VIEWPORT_RES=2`), release profile,
> 180 warm-up frames and 300 measured.

---

## 0. What this phase learned

Two of the seven sub-phases produced **negative** results, and they are the two
worth reading first.

1. **The audit's `Terrain` CPU-variance finding did not reproduce** (§E). The
   1.39 ms mean and 8.32 ms maximum it was built on came from a 20-frame
   warm-up. At the documented 180-frame warm-up the same zone measures
   **0.031 ms, σ 0.002**. The finding was an artefact of the warm-up, not a
   defect. Nothing was changed.
2. **The WGSL terrain micro-pass made the shader slower** (§G). Three
   repetitions each, back to back, same binary otherwise: with the change
   `Shading` measured 12.484 / 12.466 / 12.589 ms; without it, 12.239 / 12.032
   / 11.703 ms. The two sets do not overlap. It was reverted.

And one large positive one:

3. **The terrain clipmap — built in Phase DF, audited 2026-08-15, default off
   ever since, blocked on nothing but a remeasure — takes Coastal ground from
   21.4 ms to 9.4 ms a frame, and it passes DF §6.4's look gate** (§F).

---

## A — the baseline matrix

Twelve runs, `dev records/phase PORTAL-0/PORTAL-0-A_*.somtime`.

| View | Frame | Shading | Water prepass | ReSTIR GI |
|---|---:|---:|---:|---:|
| `coastal-ground` | 21.445 | 11.463 | 2.319 | 2.898 |
| `coastal-overview` | 23.959 | 10.648 | 2.793 | 4.471 |
| `island` | 18.762 | 7.545 | 2.639 | 2.412 |
| `island-ground` | 17.945 | 8.053 | — | — |
| `coastal-ground` @ 1280×688 | 10.049 | 4.389 | — | — |
| `island` @ 1280×688 | 8.165 | 2.364 | — | — |

**Ablation** (`SOMNIUM_SHADE_ABLATE`, coastal-ground, against Shading 11.463):

| Class | ms | share |
|---|---:|---:|
| terrain | 11.461 | **100.0%** |
| sky | 0.169 | 1.5% |
| mesh | 0.111 | 1.0% |
| foliage | 0.113 | 1.0% |

DOOM-B measured 97.6% for terrain in 2026-08-16. It is now indistinguishable
from the whole pass. The classes do not sum to 100% because each ablation still
pays the pass's own fixed cost; the reading is "terrain is everything", which is
the same reading DOOM-B took and is now stronger.

**Three corrections to the pre-AC audit**, each forced by these runs:

- The audit ranked `gpu_instance_from_cmd` using AB's `Instances` figure of
  0.178 ms. At the proper warm-up that zone is **0.019 ms**. The mechanism the
  audit identified is real and the cross-scene ratio confirms it (§D), but it
  was never worth 0.75% of a frame and is not claimed as one.
- `SOMNIUM_VIEWPORT_RES=1` (2560×1440) does **not** produce a 2560-wide render:
  the value is a cap, the window is 1920 wide, and the run is byte-identical in
  size to the 1920 one. There is no second high resolution available on this
  machine, so 1280×688 is the second point instead.
- Between-run spread is larger than within-run σ. `coastal-ground` `Shading`
  measured 11.463 in one session and 11.703–12.239 in another, on the same
  code. **A `.somtime` σ is a within-run number and must not be used as the
  band for a cross-session comparison.** §G is decided on repetitions taken
  back to back for exactly this reason.

**Evidence debt, closed and left open.** CONTROL-C's thumbnail fix still has no
post-fix `.somtime` — it needs the editor-driver harness, not a viewport run,
and it stays open. Not previously recorded and found here: **`assets/scripts/
somnium.d.luau` was stale.** MORROWIND-AD took the Terrain schema to version 2
and never regenerated the declarations, so eight virtual-texturing fields were
missing from the Luau type surface with nothing failing. Running the engine
regenerated it; the regenerated file is in this commit.

---

## B — an honest frame, and CPU zones that exist

**In tree.**

The audit's central instrumentation finding was that `cpu Frame wall` had been
read as CPU work when it is a tick-to-tick interval under
`PresentMode::AutoVsync`, and that the engine had five CPU zones, all inside the
renderer. Both are fixed.

- `GpuProfiler::frame_cpu_ms` — the engine's frame body, from the top of
  `about_to_wait` to just before `TimeState::wait_for_frame_budget`. Timed with
  a plain `Instant` and handed over at the end of the frame, because a profiler
  scope spanning the render call would still be open when `end_frame` harvests
  the stack.
- `GpuProfiler::surface_acquire_ms` — time blocked in
  `Surface::get_current_texture`, which is where Fifo puts the presentation
  wait.
- `cpu_raw_results()` beside `cpu_results()`. `cpu_end` writes an EMA into the
  latter because the overlay is unreadable without one; `timing.rs` now
  accumulates the former. **Every `.somtime` CPU row written before this commit
  reported the standard deviation of the smoother rather than of the work.**
- Five new zones in `somnium_core::app`: `Editor panels`, `Jobs & assets`,
  `Scene submit`, `Environment`, and the existing `Foliage` correctly nested
  under `Scene submit`.

What the three frame rows say, which nothing could say before:

| coastal-ground | clipmap **off** | clipmap **on** |
|---|---:|---:|
| `gpu Frame` | 21.44 | 9.10 |
| `cpu Frame wall` | 21.43 | 16.90 |
| `cpu Frame CPU` | 21.18 | 3.86 |
| `cpu Surface acquire` | **16.80** | **0.04** |

Clipmap off, the CPU spends 16.8 of its 21.2 ms blocked waiting for the GPU:
**the frame is GPU-bound and no CPU work in it can matter.** Clipmap on, the
CPU does 3.86 ms, blocks for 0.04, and the 16.90 ms wall is the vsync interval.
Two configurations whose `Frame wall` differ by 4.5 ms for completely unrelated
reasons — which is precisely the confusion the old single row invited.

Named zones on coastal-ground total 0.39 ms of the 4.39 ms of real CPU work
(`Editor panels` 0.203, `Jobs & assets` 0.068 with a 1.67 ms maximum,
`Environment` 0.041, `Terrain` 0.034, `Cluster cull` 0.024, `Lighting extra`
0.010, `Instances` 0.007, `Scene submit` 0.006). The ~4 ms residual is command
recording, UI layout and paint, editor event drain, and wgpu submission. That
residual is now a visible number instead of an absence.

---

## C — nine dead dependencies, a dead triple, and a gate that could not see

**In tree.**

MORROWIND-A's census had carried a nine-row `UNREFERENCED` column since
2026-08-24 with its own warning that a grep is not a resolver. Each was checked
against every spelling Cargo could give it before deletion; `cargo check
--workspace --all-targets` passes without them.

`rayon` (`somnium_ecs`), `pollster` (`somnium_renderer`), `base64`
(`somnium_core` — the census's only hit was the substring in the field name
`thumbnail_png_base64`), `tracing` (`somnium_audio`, `somnium_voxel`,
`somnium_script_luau`), `anyhow` and `rand` (`hello_engine`), and `anyhow`
(workspace).

Also removed: the **`egui` / `egui-wgpu` / `egui-winit` triple**. `phase_
MORROWIND.md` §4.7 found it dead, the census carried it as *"exempt — DEAD"* and
left its removal *"to Phase PORTAL's CI gates rather than smuggling a cleanup
into a census"*. This is that phase. Somnium has drawn its own editor since
Phase 12.

`Cargo.lock` loses 44 lines. The census now reports **0 unreferenced**, and its
generator's prose was corrected in the same commit so the report does not
describe an exemption that no longer exists.

**The gate could not see the one real offender.** GHOSTFENCE's `one-job-system`
row matched `thread::spawn` and nothing else, so `somnium_voxel/src/world.rs:267`
has been calling `rayon::spawn` — a detached task on rayon's *global* pool,
which is a second background scheduler by any reading of §11 row 12 — through
the whole of MORROWIND without the gate noticing. The pattern now also matches
`rayon::spawn`, `thread::Builder` and `ThreadPoolBuilder`. Data-parallel rayon
(`par_iter`, `for_each_mut`) is deliberately not matched; `somnium_jobs`'
manifest already argues why fork-join inside a frame is a different problem.

**The voxel call is exempted, not fixed.** Routing it through `somnium_jobs`
means threading a `&mut JobSystem` through `VoxelWorld::update`, and that is a
public API change that belongs at a MORROWIND seam rather than inside a
performance commit. The exemption says so and calls it owed work.

---

## D — two confirmed CPU mechanisms

**In tree.** Both were confirmed by prediction before they were fixed, which is
the only reason either is here: neither is worth a millisecond today.

**`gpu_instance_from_cmd` was O(draws × chunks).** It recovered a terrain
chunk's packed LOD word by scanning every chunk of every terrain for every draw
command. Coastal is 16×16 chunks over 89 draws; Island is 8×8 over 56. That
model predicts a 6.4× cost ratio between the scenes; the measured ratio was
5.4×, where a per-draw model predicts 1.6×. The word is now recorded in the
terrain loop, which already holds the chunk, and read back by hash.

| `cpu Instances` | before | after | |
|---|---:|---:|---:|
| coastal-ground | 0.0193 ± 0.0015 | 0.0065 ± 0.0019 | **3.0×** |
| island | 0.0050 ± 0.0006 | 0.0033 ± 0.0007 | 1.5× |

The ratio between the two scenes tracks chunk count, which is the point: the
old shape was quadratic in terrain size, and MORROWIND-S and -T exist to make
terrains larger.

**The UI cloned its whole child list twice per node per frame.**
`update_global_visibility` and `draw_node` are the two traversals that visit
every node every frame, and both cloned the child `Vec` at each node purely to
end the borrow before recursing. Both now walk by index. The real editor shell
is **1,042 nodes**, so this is 2,084 heap allocations a frame that no longer
happen.

New test `somnium_ui::tests::measured_cpu_cost_of_a_shell_frame`, in the shape
of `somnium_script_luau/tests/budgets.rs` — report always, assert only in
release, quote a p95. There was no CPU number for an editor frame at all before
it. Median of `perform_layout` + `draw` over the real shell, three release runs
each:

- before: 0.054, 0.054, 0.054 ms
- after: **0.043, 0.043, 0.043 ms** — **−20%**

The other five `children.clone()` sites in `ui.rs` are deliberately untouched:
they are structural (`remove_node`, `clear_children`) or event-driven
(`pick_node`, `collect_focusable`, the a11y snapshot), and two of them mutate
the tree while walking it, where the clone is what makes the walk correct.

---

## E — the terrain CPU variance does not exist

**Nothing in tree. The finding was wrong.**

The audit ranked this from MORROWIND-AB's `cpu Terrain` row: mean 1.39 ms,
σ 1.81, maximum 8.32, on a stationary camera. That run used
`SOMNIUM_TIME_WARMUP=20`. `timing.rs`' own documentation says the warm-up
exists to discard exactly this — pipeline creation, the first clipmap ring fill,
FSR and TAA history — and its default is 180 for that reason.

At 180 frames of warm-up the same zone on the same viewpoint measures **0.0309
ms, σ 0.0022, maximum 0.0361**. There is no variance to explain. The AB row was
measuring the warm-up transient the harness was designed to exclude, and the
audit should have caught that the run's own header said `20 warmup`.

No work was done. This section exists so nobody re-derives the finding.

---

## F — the clipmap decision

**Measured. The default flip is not taken in this commit; see the end.**

`phase_DF.md` §12.3 has carried one open item since 2026-08-15:
*"**DF-E default-on** — Blocked on §6.4 gates at maximized Native with the
post-audit look. Nothing in the audit could measure this; it needs a live
capture."* That capture is taken.

`SOMNIUM_TERRAIN_CLIPMAP=1`, everything else identical, 300 frames after 180:

| View | Frame off → on | Shading off → on |
|---|---|---|
| `coastal-ground` | 21.445 → **9.400** (−56%) | 11.463 → **1.724** (−85%) |
| `coastal-overview` | 23.959 → **14.191** (−41%) | 10.648 → **2.566** (−76%) |
| `island` | 18.762 → **12.190** (−35%) | 7.545 → **2.621** (−65%) |

Every other zone moves only in the direction the smaller shading cost implies
(`ReSTIR GI` −0.97, `Shadows` −0.34, `FSR` −0.17 on coastal-ground). All three
views land inside the vsync interval: `Frame wall` 16.92–16.94 ms, σ 0.49.

**DF §6.4's acceptance gates, both met:**

- *Eye-level mean luminance vs clipmap-off ≤ 1%.* Display-referred captures
  (`SOMNIUM_CAPTURE_DISPLAY_PNG`, frame 240), Rec.709 luma over the lower half
  of the frame, which is the ground:

  | View | eye-level | full frame |
  |---|---:|---:|
  | `coastal-ground` | **+0.12%** | −0.15% |
  | `coastal-overview` | **−0.15%** | −0.04% |
  | `island` | **+0.49%** | +0.25% |

- *Walk shading ms ≤ clipmap-off.* 11.463 → 1.724 ms.

Captures are `PORTAL-0-F_<view>_clipmap-{off,on}.png`.

**What is not claimed.** Mean luminance is the gate DF §6.4 names and it is not
a structural comparison; these are three stationary viewpoints, not a walk; and
DF's other two open items (POM on clipmap height, the CIEDE2000 fixture) are
untouched. **The default is left off in this commit.** Two documents carry a
standing instruction against flipping it —
`terrain_shading_occupancy_2026-08-14.md` (*"Do **not** enable it as the Coastal
default"*) and `phase_DF.md` §12 — and although both conditioned that on gates
that now pass, a shipped default that changes what an existing scene draws is
the user's call and not a side effect of a measurement phase. **The
recommendation is to take it.** It is a one-line change to
`TerrainClipmap::default_enabled` plus a `context.md` entry, and it is the
largest single frame-time result in the repository's history.

---

## G — the WGSL terrain micro-pass, rejected

**Nothing in tree. The change was slower.**

`terrain_strongest_four` computes the four winning weights as `b0..b3` and
discarded them, so both callers read them back with `weight[selected[s]]` — a
dynamic index into a 32-element function-scope array, eight times per terrain
pixel. Returning them instead looked free.

Two things went wrong, and the second is the one that matters.

**It did not compile the obvious way.** A `struct TerrainStrongest { index:
array<u32,4>, weight: array<f32,4> }` return makes `selected[s]` an array
reached through a member access, and naga rejects a dynamic index into one:
`Invalid access into expression [45], indexed: false`. Binding it to `var`
rather than `let` does not help. The working form is an out-pointer
(`out_w: ptr<function, array<f32,4>>`), which keeps the return type and every
existing `selected[s]` expression exactly as they were. `rt_hit.wgsl` is a third
caller and was missed on the first pass; `shaders_validate` caught it.

**It was 4% slower.** Three repetitions each, back to back, same binary except
for the two shader files:

| | rep 1 | rep 2 | rep 3 | mean |
|---|---:|---:|---:|---:|
| with the change | 12.484 | 12.466 | 12.589 | 12.513 |
| without | 12.239 | 12.032 | 11.703 | 11.991 |

The sets do not overlap — the fastest with-change run is slower than the slowest
without. The likely mechanism is that an out-pointer `array<f32,4>` forces four
values into addressable function-scope memory that previously lived as scalars
the compiler could keep in registers, trading eight indexed reads for a
guaranteed spill. Reverted.

This is DOOM-C's lesson a second time: **the plausible per-pixel saving lost to
the thing the compiler was already doing.** The 32-element `weight` array stays,
and the comment above it that explains why the previous rewrite of this function
was a win remains accurate.

---

## Gates

`python tools/ghostfence/run.py`:

| Row | Result |
|---|---|
| census | **PASS** — regenerated; 186,244 lines, 1,843 test markers, **0 unreferenced dependencies** |
| toolchain | PASS — rustc 1.88, wgpu 30.0, winit 0.30 |
| shader-budget | PASS — 53 modules, 53 variants |
| one-job-system | PASS — widened pattern, 4 exemptions each with a reason |
| no-second-system | PASS |
| tests | **PASS — 1,876 passed, 0 failed** (floor 945; MORROWIND closed at 1,835) |
| golden-images | **FAIL — pre-existing, not caused here** |

**The golden-images failure is older than this phase and is a gate that
silently stopped working.** `sculpt-panel` fails at 1,792 of 33,600 pixels
(5.33%, budget 0.2%) with a peak channel delta of 37 against a ceiling of 24.
The identical failure — same pixel count, same peak — reproduces on a clean
`439b6b6` with every PORTAL-0 change stashed, so nothing here caused it.
`menu-bar` and `toolbar` pass.

The reference was taken at MORROWIND-E2b. MORROWIND-AD then added the
virtual-texturing block to Terrain Details, which is what the sculpt panel
draws — the same sub-phase that left `somnium.d.luau` stale. It went unnoticed
because the row reports `SKIP` when no candidate image exists, and no candidate
had been generated since; a gate that skips by default is a gate that is off.
**This needs a decision that is not a performance decision**: either the
reference is re-taken because AD's new rows are correct, or AD changed chrome it
should not have. Recorded here, not resolved here.

---

## What is owed

1. The clipmap default flip (§F), if the recommendation is accepted.
2. The `sculpt-panel` golden reference, re-taken or investigated.
3. `rayon::spawn` in `somnium_voxel` routed through `somnium_jobs`, at a seam.
4. CONTROL-C's post-fix thumbnail `.somtime`.
5. A moving-camera hitch experiment. Nothing in the tree can produce one, and
   `Jobs & assets`' 1.67 ms maximum against a 0.068 ms mean is the first
   evidence that there is something there to find.
