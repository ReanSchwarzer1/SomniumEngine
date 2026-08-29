# Phase PORTAL — Source

> *The Enrichment Center reminds you that the Companion Cube cannot speak. In
> the event that it does, please disregard its advice.*

> **Codename:** PORTAL (Valve, Source, 2007). Chosen for two reasons and both
> are load-bearing.
>
> 1. **Test chambers.** Portal is the game whose only progression mechanic is
>    "a sealed room you cannot leave until the thing is proven". That is what
>    this phase installs: every deficiency below closes against a gate that
>    fails, not against a session's confidence.
> 2. **`ConVar`.** Source's console-variable registry — one registered,
>    described, ranged, flagged place where every tunable lives — is the
>    reference architecture for the largest structural defect measured in this
>    tree: **96 `SOMNIUM_*` environment variables read from 91 scattered
>    `std::env::var` call sites, three of them inside `render()`.**
>
> And one joke that is also a finding: **the cake is a lie.** This repository
> has a `clippy` CI job that has never been able to fail — and `rust-doctor`,
> run for the first time on 2026-08-18, reports its own gate as
> `"status": "not-evaluated"` for exactly the same reason (§2.1).
>
> **Current status, 2026-08-29:** **PLAN ONLY.** No full PORTAL sub-phase has
> started. PORTAL-0 later completed a focused performance and health pass, but
> it does not make this phase complete. Before PORTAL-A, re-run the audit,
> remove work already closed by PORTAL-0, and rebase the sequence against
> completed CONTROL and the current MORROWIND tree.
>
> **Original snapshot:** 2026-08-18 on `dev` at `45a3df8` (`Update
> context.md`). Counts, dependency versions, defect locations, and sequencing
> below belong to that snapshot unless a revision note says otherwise.
> **Revision:** r2, 2026-08-18 — rust-doctor run against the tree (§2.1);
> PORTAL-D inverted from "version-control `dev records/`" to "migrate the
> record into `context.md`" at the user's direction (§5.10); the foliage-colour
> defect found already fixed and never struck from `context.md` §18 (§5.11).
> **Record:** this file. Evidence folder `dev records/phase PORTAL/` is created
> by **PORTAL-A**, not before. **Do not invent PNGs or `.somtime` files.**
> **Roadmap correction:** CONTROL is complete. MORROWIND is active and partial.
> PORTAL still owns engineering health, but the old instruction to land it
> before CONTROL is obsolete. Use [`../context.md`](../context.md#roadmap-order)
> for the current sequence.

**Frozen in the original 2026-08-18 plan.** Revalidate this list before
PORTAL-A. A PORTAL sub-phase that changes any confirmed item has gone wrong:
Great Lakes water numbers (datum 16.1 m, `max_depth` 18.6 m,
`wave_speed` 0.85); the XV 32-layer terrain contract and the 2032-byte
`GpuTerrainMaterial` layout; the Island `hero_bank_only` recipe; foliage LOD /
impostor / cull distances (45 / 90 / 120 m); DOOM's measured defaults (dynamic
resolution opt-in, tile binning and aerial terrain off, hex/POM off); the
Nocturne token sheet; rustc **1.88**. The recorded wgpu **29** value is
historical; the current gated version is wgpu **30**.

**The rule this phase is judged by, stated once — the gate rule:**

> A defect that is fixed but not gated is not fixed, it is *currently absent*.
> Every sub-phase below closes with a **command that fails before the change
> and passes after**, and that command runs in CI. A sub-phase that improves
> the code without leaving behind the gate that keeps it improved has failed,
> however clean the diff looks.

**And its corollary, which this revision exists to prove:** a defect that is
*fixed* but never struck from the record is worse than one still open, because
every future session pays to rediscover that it is gone. §5.11 is the worked
example — 40 minutes between the bug being written down and being fixed, and
ten days of it sitting in `context.md` §18 as "not yet investigated".

---

## 1. Executive decision

Somnium's engine is in good shape and its *record* of that engine is unusually
good. `dev records/` contains deterministic GPU timing runs with a standard
deviation per row, a pixel census, two documented negative results, and three
separate write-ups of an investigation that recorded its own wrong turns. That
is better engineering discipline than most shipping renderers have.

**The problem is that almost none of it is enforced by a machine, and none of
it is under version control.**

Measured in the tree on 2026-08-18, with the command that produced each number:

| Fact | Value | Command |
|---|---:|---|
| Workspace tests, all green | **826** passing, 0 failed, 0 ignored | `cargo test --workspace` |
| …of which CI actually runs | **~590** (`--lib` only) | `.github/workflows/ci.yml` |
| Integration tests CI never runs | **12 files, 5 529 lines** — including the 22-case script threat model | `find crates -path '*/tests/*.rs'` |
| CI triggers on push to | `main`, `master` | `.github/workflows/ci.yml` |
| Branch all work happens on | **`dev`** | `git branch` |
| `cargo fmt --all --check` | **198 diffs across 42 files** | `cargo fmt --all --check` |
| `cargo clippy --workspace --all-targets` | **228 warnings**, job cannot fail | `cargo clippy …` |
| **`rust-doctor` score** | **65 / 100, "Needs work"** (`authoritative: false`) | `npx rust-doctor@latest --json` |
| …its worst dimension | **dependencies 50**; reliability 75, maintainability 81 | ibid. |
| …its findings | **1 167 distinct / 1 328 occurrences**, 0 errors | ibid. |
| Crates with a lint policy | 5 of 11 — **not** the two largest | `grep '#!\[warn' crates/*/src/lib.rs` |
| Functions over 500 lines | **5** | function-length scan, §5.4 |
| Longest function | `Renderer::render` — **1 939 lines** | `renderer.rs:1884` |
| Highest cyclomatic complexity | `handle_editor_event` — **381** (cognitive **484**) | rust-doctor `structure::complex_function` |
| Largest pass-call parameter list | **21** | `pass/water.rs:755` |
| Distinct `SOMNIUM_*` environment variables | **96** | `grep -rho 'SOMNIUM_[A-Z0-9_]*'` |
| `std::env::var` call sites | **91** | `grep -r 'env::var'` |
| …inside `Renderer::render` (per frame) | **3** | `renderer.rs:2001, 2085, 3078` |
| `unsafe` in `somnium_physics/src/world.rs` | **22**, with **0** `SAFETY` comments | `grep -c` |
| `somnium_ui` test density | **3.7 tests / kloc**, 0 integration tests | 80 tests / 21 394 LOC |
| Public items in `somnium_renderer` + `somnium_ui` with no `missing_docs` gate | **1 410** | `grep -rh '^\s*pub \(fn\|struct\|…\)'` |
| **`dev records/` under version control** | **0 files** — and **staying that way** (§5.10) | `.gitignore:47` |
| **Engineering record living only on one machine** | **40 markdown files, 17 387 lines, 1.03 MB** + 108 data files, 751 KB | `find "dev records" -name '*.md'` |
| Unused declared dependencies | **11** | rust-doctor `cargo::unused_dependency` |
| Incompatible major duplicate crates | **9** (of 46 total duplicates) | rust-doctor `cargo::duplicate_major_versions` |
| Panic-on-index sites (`indexing_slicing`) | **604** (388 in the renderer) | rust-doctor |
| Coastal-ground frame vs the DOOM §9 budget | **29.4 ms** vs **≤ 16.6 ms** | `phase DOOM/README.md` |
| Open defects listed in `context.md` §18 | **5 listed, 4 actually open** (§5.11) | `context.md:4112` + `git log -S` |

The shape of it: **this project's verification is human, its record of that
verification is prose, and that prose is untracked.** All three are good prose
and none of them is backed. The `.somtime` baselines that
`dev records/README.md` says "**do not overwrite**" are protected by a sentence
in a markdown file, in a directory git has been told to ignore, on a machine
whose `target/` is 54 GB inside OneDrive.

PORTAL converts the prose into gates, and moves the durable half of it into the
one file that *is* tracked. It adds **no rendering feature and no editor
feature**. Every sub-phase either (a) makes an existing invariant
machine-checked, (b) removes a structure that makes the invariant hard to
check, (c) closes a defect that is already written down and still open, or
(d) moves a record from a place git ignores to a place git keeps.

---

## 2. What is measured, and what the instruments are worth

This section exists because one of the tools reached for while writing this
plan produced numbers that are **wrong**, and a plan that quoted them would
have sent a session chasing 12 523 phantom defects.

| Instrument | Verdict | Detail |
|---|---|---|
| `cargo test --workspace` | **Trustworthy.** Use as the gate. | 826 pass in ~40 s wall. |
| `cargo clippy` | **Trustworthy, uneven.** Signal depends on the crate's own `#![warn]`. | See §5.2 — the 153 `somnium_core` warnings are mostly `pedantic` noise; the 41 `somnium_renderer` ones are `clippy::all` and therefore *real*. |
| `cargo fmt --check` | **Trustworthy.** Binary answer. | 198 diffs. |
| `timing.rs` / `.somtime` | **Trustworthy, best-in-tree.** Stddev per row, `~ noise` verdict. | Already caught a wrong DOOM-F diagnosis (`phase DOOM/README.md`). |
| `capture.rs` / `.somcap` | **Trustworthy.** Per-class pixel diff with `mean_abs`. | DOOM-C parity to 2 px of 2 615 044. |
| **`git log -S`** | **Trustworthy, and underused.** | It is what proved §5.11. Any claim in `context.md` §18 can be dated against the commit that fixed it. This should be routine before any defect is worked. |
| `rust-doctor` 0.2.0 | **Useful, non-authoritative, needs configuration.** | §2.1 — real run, real findings, three real limitations. |
| `engineering-advanced-skills:tech-debt-tracker` `debt_scanner.py` | **Not usable on this repo. Do not wire it into a gate.** | Reported 13 820 items / health score 0. Verified false: **all 536 `todo_comment` hits match the substring `bug` inside `#[derive(Debug, …)]`**; it scans `.ttf` font binaries as source (4 of its top 12 "debt" files are fonts); `duplicate_code` (12 523 of 13 820) fires on ordinary Rust boilerplate. Actual `TODO/FIXME/HACK/XXX` count in `crates/`: **0**. |

**The one TODO in the tree** is honest and load-bearing:
`third_party/wgpu-ffx/src/lib.rs:25` — `// TODO: remove once GenerateReactive
and DebugView are wired up`. It is the cause of the FSR ghosting on water noted
in the post-Halcyon handoff §2. It is PORTAL-L's.

### 2.1 — rust-doctor, run 2026-08-18

Node LTS installed (v24.19.0 / npm 11.17.0). `npx rust-doctor@latest --json`,
full workspace scope, against rustc/cargo/clippy 1.88.0. **Exit 0.**

```
score 65 / 100  "Needs work"   model core-v2   authoritative: false
  security 100 · performance 100 · maintainability 81 · reliability 75 · dependencies 50
1 167 distinct findings / 1 328 occurrences · 0 errors · worst tier P1
```

Mechanically it drives `cargo clippy --workspace --no-deps -- -A clippy::all -W
<62 curated rules>`, then adds its own `structure::*` and `cargo::*` analyses on
top. So it is **not** a second opinion on clippy — it is a *curated selection*
of clippy plus a real structural pass.

**What it found, by weight:**

| Rule | Count | Verdict against this inventory |
|---|---:|---|
| `clippy::indexing_slicing` | **604** (388 renderer, 77 ui, 37 ecs) | **New, and partly real.** My audit counted `.unwrap()`; this counts panic-on-index too. Most are fixed-size arrays in trusted contexts. Scoped into PORTAL-K, not taken wholesale. |
| `structure::near_duplicate_function_body` | 114 | **New and credible** — unlike `debt_scanner`'s 12 523, these name a file and a node count (e.g. `light_units.rs`: 3 functions sharing a 44-node body). |
| `clippy::unwrap_used` | 95 | **Agrees** with §5.7's panic-boundary item. |
| `structure::oversized_unit` | 72 | **Agrees** with §5.4 — and adds what a line count misses: `impl Engine<G>` spans **2 927 lines**, `impl ApplicationHandler for Engine<G>` spans **1 324**. |
| `structure::unreasoned_allow_attribute` | 69 (+15 `crate_level_allow`) | **Agrees exactly** with PORTAL-B's "every surviving `#[allow]` gets a reason string". Independent corroboration. |
| `structure::duplicate_function_body` | 52 | New. |
| `structure::complex_function` | 49 | **Agrees and improves on §5.4** — see below. |
| `clippy::expect_used` | 33 | Agrees. |
| `cargo::path_dependency_outside_workspace` | 28 | **False positive.** It flags `somnium_ecs`, `somnium_renderer`, `somnium_asset` — all workspace members. Configure the rule off with a reason. |
| `clippy::too_many_arguments` | 12 | **Agrees** with §5.5. |
| `cargo::unused_dependency` | **11** | **New and immediately actionable** — see §5.12. Removes the need for `cargo-udeps`. |
| `cargo::duplicate_major_versions` | **9** | **Better than my number.** I counted 46 duplicates; 9 of them are *incompatible majors*. See §5.12. |
| `clippy::unreachable` | 7 (all `app.rs`) | Its only `correctness`-category findings. |

**The complexity numbers are the most valuable thing it produced**, because
they rank the same five functions §5.4 found by a different measure and add two
§5.4 missed:

| Function | Cyclomatic | Cognitive | In §5.4's line-count top 5? |
|---|---:|---:|---|
| `Engine<G>::handle_editor_event` | **381** | **484** | yes (1 739 lines) |
| `ApplicationHandler::window_event` | **77** | **124** | **no** — 336 lines |
| `ApplicationHandler::about_to_wait` | 69 | 101 | yes (792 lines) |
| `Engine<G>::apply_inspector_color` | **38** | **76** | **no** |
| `Engine<G>::apply_post_process` | 32 | 43 | no (129 lines) |
| `Engine<G>::submit_foliage` | 28 | 60 | no (155 lines) |
| `somnium_asset::load_gltf` | 25 | 27 | no (198 lines) |

A cyclomatic complexity of **381** in one function is the single most alarming
number this audit produced, and a line count did not surface it as different in
kind from the others. **PORTAL-I gains two targets** (`window_event`,
`apply_inspector_color`) on this evidence.

**Its three limitations, recorded so nobody over-trusts the 65:**

1. **The run is incomplete.** `"status": "incomplete"`, `"complete": false`.
   Its own Rust parser threw **11 parse errors across 3 files** —
   `somnium_core/src/scene_schema.rs` (1), `somnium_renderer/src/capture.rs`
   (3), `somnium_script_luau/src/lib.rs` (7) — and those files were **skipped
   entirely** by the structure pass. So the structural findings above are a
   *lower bound*, and the score is self-declared `"authoritative": false`.
2. **Its default gate cannot fail — the same defect as our clippy job.**
   `"gate": {"blocking": "error", "status": "not-evaluated"}`, because every
   one of the 1 167 findings is a *warning* and there are **0 errors**. Adopted
   at defaults, rust-doctor would be a second decorative green check.
   PORTAL-C must set `--blocking warn`, or promote categories explicitly in
   `rust-doctor.toml`, or it is theatre.
3. **At least one rule is wrong on this repo** (`path_dependency_outside_
   workspace`, 28 occurrences), and one is a matter of taste at this scale
   (`indexing_slicing`, 604). Both need a configured level and a written
   reason before the tool goes anywhere near a gate.

**Verdict: adopt, advisory-first, configured.** It found four things this audit
did not (the complexity ranking, 11 unused deps, the 9 incompatible majors, and
166 duplicate bodies) and independently corroborated four more. That earns it a
place. It does not earn it a blocking gate on day one — which is precisely the
mistake §5.1 says the existing clippy job already made.

---

## 3. Goals

1. **Every invariant this project already believes in is checked by a
   command**, and that command runs on `dev`.
2. **No structure in the tree is too large to review.** Specifically: no
   function over ~300 lines or cyclomatic complexity ~40, no pass call over ~8
   parameters.
3. **One registry for tunables.** 96 environment variables become one
   registered, described, defaulted, validated table — the prerequisite both
   for CI reproducibility and for Phase CONTROL's Seam 4.
4. **The durable record lives in a tracked file.** `dev records/` stays a local
   working folder; its engineering content migrates into `context.md`.
5. **The four genuinely open defects are closed or explicitly retired**, and
   the defect list stops carrying entries that were fixed ten days earlier.
6. **The refactors are provably invisible.** Byte-class-identical captures and
   `~ noise` on every `.somtime` zone, or the sub-phase does not land.

## 4. Non-goals

- **No new rendering feature.** No new pass, no new shader effect.
- **No editor reach.** Turning env vars into inspector controls is *Phase
  CONTROL Track 1*. PORTAL builds the registry CONTROL binds to, and stops.
- **No frame-time chase.** DOOM measured what is left (§5.13). PORTAL does not
  re-open Coastal shading. The one perf item it takes is the **ReSTIR GI
  variance**, because a max of 39.1 ms against a mean of 8.4 is a *correctness
  smell*, not a budget question.
- **No retuning of any frozen contract** (see the header block).
- **No dependency bumps for their own sake.** wgpu 29, rustc 1.88, mlua
  `=0.12.0` stay pinned. PORTAL removes the 11 *unused* declarations and audits
  the rest; it does not upgrade.
- **No `unsafe` removal for its own sake.** PORTAL requires that every `unsafe`
  block *justifies itself in a comment*; it does not require that the block
  disappear.
- **No git-tracking of `dev records/`.** Explicitly rejected by the user on
  2026-08-18. §5.10 is the migration, not the tracking.

---

## 5. The inventory

Fourteen deficiencies, each with the evidence that found it, the cost of
leaving it, and the sub-phase that owns it.

### 5.1 — D1: the CI gate is decorative

`.github/workflows/ci.yml`, four separate holes:

| Hole | Consequence |
|---|---|
| `on: push: branches: [main, master]` | All work happens on `dev`. **Pushes to `dev` run nothing.** CI fires only on PRs, and this repo's history is direct commits. |
| `cargo test --workspace --lib` | The 12 integration-test files — 5 529 lines, including `script_threat_model.rs` (22 sandbox-escape cases + a 28-case malformed corpus + every prefix of every case) — **have never run in CI.** The most security-relevant suite in the engine is the one CI skips. |
| clippy job with no `-D warnings`; comment says "tighten this once the workspace is fully clippy-clean" | It has never been able to fail. 228 warnings accumulated under it. **rust-doctor arrives with the identical defect at its defaults** (§2.1) — do not install it the same way. |
| No `cargo fmt --check` | 198 formatting diffs across 42 files landed, concentrated in the newest work (Phase 16 scripting, DOOM `timing.rs` / `census.rs`). |

**Owner: PORTAL-A.**

### 5.2 — D2: the lint policy is inverted

| Crate | LOC | `#![warn(clippy::pedantic)]` | `#![warn(missing_docs)]` | clippy | rust-doctor |
|---|---:|:---:|:---:|---:|---:|
| `somnium_renderer` | **37 741** | ✗ | ✗ | 41 | **606** |
| `somnium_ui` | **21 394** | ✗ | ✗ | 14 | 121 |
| `somnium_core` | 16 012 | ✓ | ✓ | 153 | 320 |
| `somnium_script` | 4 815 | ✓ | ✓ | 0 | 34 |
| `somnium_ecs` | 3 712 | ✓ | ✓ | 14 | 98 |
| `somnium_script_luau` | 2 793 | ✓ | ✓ | 0 | 41 |
| `somnium_voxel` | 1 000 | `clippy::all` only | ✗ | 0 | 5 |
| `somnium_asset` | 820 | ✗ | ✗ | 1 | 28 |
| `somnium_physics` | 442 | ✗ | ✗ | 1 | 4 |
| `somnium_physics_sys` | 146 | ✗ (FFI, correctly) | ✗ | 0 | 6 |
| `somnium_audio` | 93 | ✗ | ✗ | 3 | 2 |

**The 59 135 lines with the most GPU-contract-sensitive code in them are the
lines with no lint policy**, and rust-doctor's independent column agrees:
`somnium_renderer` alone carries **606 of its 1 328 occurrences**. The 153
`somnium_core` clippy warnings *look* like the worst crate and are mostly
`must_use_candidate` and `doc_markdown`; the 41 `somnium_renderer` warnings are
`clippy::all`-level and include real ones (`this if has identical blocks`,
`the loop variable i is used to index`, `casting the result of i32::abs() to
u32`).

There is no workspace `[lints]` table, so every policy is a per-file `#![warn]`
someone has to remember — and rust-doctor found **84 `#[allow]` attributes with
no stated reason** (69 item-level, 15 crate-level) sitting under that absence.

**Owner: PORTAL-B.**

### 5.3 — D3: formatting has drifted where the newest code is

198 diffs / 42 files. The distribution is the finding: `somnium_script*` (all
source files and all three test files), `somnium_core/src/script_*.rs`,
`somnium_renderer/src/timing.rs` and `census.rs` — i.e. **Phase 16 and Phase
DOOM**, the two most recent phases. Nothing gates it, so it grows with velocity.

**Owner: PORTAL-A** (the gate) **plus one mechanical commit.**

### 5.4 — D4: seven functions are too large to review

By line count:

| Lines | Location | Function |
|---:|---|---|
| **1 939** | `crates/somnium_renderer/src/renderer.rs:1884` | `Renderer::render` |
| **1 739** | `crates/somnium_core/src/app.rs:3572` | `App::handle_editor_event` |
| **999** | `crates/somnium_ui/src/lib.rs:3587` | `process_outgoing` |
| **792** | `crates/somnium_core/src/app.rs:1578` | `about_to_wait` |
| **506** | `crates/somnium_ui/src/icons.rs:444` | `rasterize` |

By complexity, which rust-doctor added on 2026-08-18 and which **finds two more
that line count missed** — `window_event` (336 lines, cyclomatic **77**) and
`apply_inspector_color` (cyclomatic **38**, cognitive **76**):

> `handle_editor_event` reaches **cyclomatic complexity 381 and cognitive
> complexity 484.**

For scale: 10 is the conventional "refactor this" threshold, and this project's
own `EditorEvent` enum has 48 variants. 381 is not "a big match statement".

rust-doctor also names the containers, which a per-function scan cannot see:
**`impl Engine<G>` spans 2 927 lines**, `impl ApplicationHandler for Engine<G>`
spans **1 324**, and `app.rs` is **5 530 lines** in total.

Out of 2 590 functions in the workspace, only 8 exceed 200 lines. So this is not
a diffuse style problem — it is **seven specific places**, and the top two are
the frame graph and the editor command dispatcher, i.e. the two places every
handoff document warns a new reader about.

`render()` is not badly written — it carries ~16 substantial ordering comments
("*After the shadow pass, whose atlas it samples for light shafts, and…*") that
are genuine architecture. The problem is that those comments are the **only**
representation of the ordering contract. Nothing fails if a pass moves.

**Owner: PORTAL-I**, gated by **PORTAL-E**.

### 5.5 — D5: pass calls pass 21 arguments

clippy `too_many_arguments`, `somnium_renderer` only (rust-doctor counts 12
across the workspace):

| Args | Location |
|---:|---|
| **21** | `pass/water.rs:755` `record_prepass` |
| **20** | `pass/water.rs:881` |
| **19** | `pass/shading.rs:868` |
| **17** | `pass/shading.rs:176` |
| **11** | `pass/water.rs:970` |
| 9 | `pass/volumetric.rs:373`, `pass/terrain_clipmap.rs:147` |
| 8 | `renderer.rs:4029` |

`record_prepass` takes `device, queue, encoder, depth_view,
global_view_proj_buffer, visibility_depth_texture_view, light_buffer,
shadow_atlas_view, shadow_sampler, env_view, env_sampler, scene_copy_view,
velocity_view, current_view_proj, current_time, geometry_vertex_buffer,
geometry_index_buffer, water_textures_bind_group, water_bodies, water_queue` —
twenty-one positional `&wgpu::*` handles.

**This is the mechanical cause of D4.** There are 38 passes in `pass/`. Because
each one's resource dependencies are expressed as a positional argument list
rather than as data, the only place that can know all of them is one function —
and that function is 1 939 lines. Two same-typed `&wgpu::TextureView` arguments
transposed at a call site is a bug the compiler cannot see and no test
currently catches.

**Owner: PORTAL-H.** This is the sub-phase that makes PORTAL-I possible.

### 5.6 — D6: 96 environment variables, 91 read sites, 3 per frame

96 distinct `SOMNIUM_*` names. `env::var` call sites: `somnium_core/src/lib.rs`
19, `renderer.rs` 14, `terrain/mod.rs` 10, `capture.rs` 6, `profiler.rs` 5,
`timing.rs` 4, `pass/taa.rs` 4, and 12 more files.

Three are read **inside `Renderer::render`**, i.e. once per frame:

```
renderer.rs:2001   SOMNIUM_SHADOW_DEBUG
renderer.rs:2085   SOMNIUM_RT_TERRAIN
renderer.rs:3078   SOMNIUM_CAPTURE_AFTER_WATER  (+ :3079 …_AFTER_TAA)
```

plus free functions `cpu_frustum_env_off()` (`renderer.rs:3984`) and
`cascade_cull_env_off()` (`:3988`) that re-read the environment on every call.
On Windows each is an OS lookup plus a `String` allocation. At three per frame
this is not a frame-time problem — **it is a correctness and reproducibility
problem**: an environment mutated mid-run silently changes behaviour halfway
through a capture, and nothing declares which of the 96 names exist, what they
accept, what they default to, or which are mutually exclusive.

The consequences are already documented elsewhere in this folder.
`terrain_shading_occupancy_2026-08-14.md` exists largely because *"runtime
uniforms do not delete WGSL"* had to be rediscovered, and `phase_CONTROL.md` §1
counts these 96 variables against **~18** with any editor control.

**Source's answer, and this phase's:** a `ConVar`-style registry. One
declaration per tunable carrying name, type, default, range, help string, and
flags (`DEV`, `CHEAT`, `STARTUP_ONLY`, `CAPTURE_AFFECTING`). Read once at
startup into a struct; `--help` and a `SOMNIUM_DUMP_VARS=1` listing fall out of
it; unknown `SOMNIUM_*` names in the environment become a **warning instead of
silence** (today, a typo'd kill switch is indistinguishable from the feature
being on).

**Owner: PORTAL-G.** Hands Phase CONTROL Seam 4 its data source.

### 5.7 — D7: test density is inverted against risk

| Crate | LOC | `#[test]` | tests / kloc | integration LOC |
|---|---:|---:|---:|---:|
| `somnium_script_luau` | 2 793 | 58 | **20.8** | 1 627 |
| `somnium_ecs` | 3 712 | 54 | 14.5 | 306 |
| `somnium_core` | 16 012 | 217 | 13.6 | 3 208 |
| `somnium_script` | 4 815 | 55 | 11.4 | 0 |
| `somnium_voxel` | 1 000 | 11 | 11.0 | 0 |
| `somnium_renderer` | 37 741 | 328 | 8.7 | **250** |
| `somnium_asset` | 820 | 6 | 7.3 | 0 |
| **`somnium_ui`** | **21 394** | **80** | **3.7** | **0** |
| **`somnium_physics`** | **442** | **1** | **2.3** | 138 |
| `somnium_audio` | 93 | 0 | 0.0 | 0 |

Two holes matter:

- **`somnium_ui`** — 21 394 lines, 3.7 tests/kloc, **zero integration tests**,
  and the crate whose history in `context.md` §8.5 is a list of layout bugs
  found by *looking at the screen*. `process_outgoing` (999 lines) and the
  layout engine are pure CPU and entirely testable.
- **`somnium_physics`** — one test, 442 lines, and **22 `unsafe` blocks with
  zero `SAFETY` comments** wrapping a C++ Jolt FFI.

`somnium_renderer` at 8.7/kloc is *fine* given the GPU boundary, but its 250
lines of integration test are one file (`shaders_validate.rs`, 14 tests).

**The panic surface**, from two instruments: 142 `.unwrap()` in
`somnium_core/src`, 65 in `somnium_ecs/src`, 39 in `somnium_renderer/src` (my
count); 95 `unwrap_used` + 33 `expect_used` + **604 `indexing_slicing`**
(rust-doctor's curated count, 388 of the indexing in the renderer). Most
indexing is a fixed-size array in a trusted context and is fine. The scoped
question — the only one PORTAL asks — is which of these are reachable from
**data the user supplied**.

**Owner: PORTAL-J** (`somnium_ui`), **PORTAL-K** (physics / asset / audio and
the panic boundary).

### 5.8 — D8: `unsafe` without `SAFETY`

| File | `unsafe` | `SAFETY` |
|---|---:|---:|
| `somnium_physics/src/world.rs` | 22 | **0** |
| `somnium_ui/src/node.rs` | 8 | **0** |
| `somnium_ecs/src/archetype.rs` | 22 | 7 |
| `somnium_ecs/src/world.rs` | 8 | 4 |
| `somnium_ecs/src/component.rs` | 3 | 1 |
| `somnium_ui/src/ui.rs` | 4 | 4 |

`somnium_ecs/src/archetype.rs` is the crate's raw-storage core and the one place
where a wrong invariant is a silent memory-safety bug across the whole engine;
it sits at 7 comments for 22 blocks. `somnium_physics/src/world.rs` has none at
all.

`clippy::undocumented_unsafe_blocks` mechanises this exactly.

**Owner: PORTAL-F.**

### 5.9 — D9: 1 410 undocumented public items

`somnium_renderer` exposes 743 public items, `somnium_ui` 667, neither under
`#![warn(missing_docs)]`. The four crates that *do* carry the lint are the four
smallest. There is no `cargo doc` build in CI, so a broken intra-doc link is
invisible.

The user-facing half is in good shape — `docs/editor/` has 10 help pages and
they are cited by the handoffs. It is the API surface that is undocumented.

**Owner: PORTAL-M.**

### 5.10 — D10: the record lives only on this machine

```
.gitignore:47   dev records/
```

`git ls-files "dev records" | wc -l` → **0**.

**Decision, taken by the user on 2026-08-18: `dev records/` stays out of
version control.** An earlier revision of this plan proposed tracking it; that
is withdrawn. The folder is a working directory — evidence imagery, capture
binaries, timing runs, session-scoped narrative — and it does not belong in the
repository. **The content does. `context.md` is the tracked file, and the
durable half of `dev records/` migrates into it.**

What is at risk today, and its size:

| Untracked | Volume |
|---|---:|
| Phase markdown (20 phase docs, 4 handoffs, plans, audits) | **40 files, 17 387 lines, 1.03 MB** |
| `.somtime` / `.audit.txt` / `.json` / `.log` data | **108 files, 751 KB** |
| Evidence imagery (`.png`, `.somcap`) | ~1 GB |

Including `DOOM-A_*.somtime` — the three baselines `phase DOOM/README.md` says
in bold "**do not overwrite**", with no history to recover them from.

**The migration is a distillation, not a concatenation, and the size is why.**
`context.md` is 4 594 lines / 349 KB today. Appending 17 387 lines of phase
markdown makes it ~22 000 lines / 1.37 MB — and the post-Halcyon handoff
instructs a new reader to *"read the entire `context.md`"*. A four-fold
`context.md` breaks the one document the project's onboarding depends on.

**The retention rule, stated once so the migration is decidable:**

> Migrate what a **future session must not rediscover**. Leave behind what a
> **past session needed to say**.

| Migrate into `context.md` | Leave in `dev records/` (local) |
|---|---|
| Frozen contracts and their numbers (water datum, terrain layout, LOD distances) | Sub-phase-by-sub-phase progress narrative |
| **Measured results** — the DOOM-A/B/F tables, the pixel census, CR occupancy, DF timings | The `.somtime` and `.somcap` files themselves |
| **Negative results and why** — DOOM-C slower at every tile size, DOOM-E invisible, the DOOM-F wrong diagnosis | Plan text superseded by what shipped |
| Root causes and the invariant each one established | Evidence PNGs and capture logs |
| Open-defect state with its ruled-out list (DF band artifact) | Restatements of `context.md` a plan made for self-containment |
| The method notes (`terrain_shading_occupancy`, the DF "do not reason from a screenshot" rule) | The four superseding handoffs' *narrative*; their live contracts migrate |

The `.somtime` **files** stay local; their **numbers** become tables in
`context.md`. That is the honest version of baseline protection under this
decision: the artifact can still be overwritten, but the record of what it said
is in git and reviewable in a diff. That is strictly better than today, where
both are on one disk.

**If the distilled result still exceeds a readable `context.md`** — the
estimate is +2 500 to +4 000 lines, taking it to ~7 000–8 500 — the fallback is
a tracked `docs/context/` set split by subsystem with `context.md` as the index,
**not** a 22 000-line single file. Decide at the halfway mark on the measured
size, and record the decision.

**Owner: PORTAL-D.** This is the highest expected-loss item in the inventory
and is sequenced early for that reason.

### 5.11 — D11: the defect list is stale, and nobody knew how stale

`context.md` §18 lists five open defects. **One of them was fixed forty minutes
after it was written down, ten days ago, and never struck.**

```
4aceadb  2026-08-08 20:12:50  Fix crash on disabling RT Direct Light; ReSTIR back to off
         ^ adds to context.md §18:
           "Foliage renders with wrong colours. Trees show salmon/pink,
            grass white. Not yet investigated."

5bdae99  2026-08-08 20:34:54  Fix cubes vanishing (cone sentinel), crash on toggling RT off

b9d1e68  2026-08-08 20:52:13  Fix GpuMaterial layout mismatch:
                              primitives and foliage decoded wrong
```

`b9d1e68`'s own message is unambiguous:

> WGSL aligns `vec3<f32>` to 16 bytes; Rust `repr(C)` aligns `[f32; 3]` to 4.
> `emissive: vec3<f32>` in the shader's Material therefore sat at offset 64
> with a 96-byte stride, against the CPU struct's offset 52 and 80-byte stride.
> **Material 0 decoded correctly and every material after it was read from the
> wrong bytes**, the error growing with the index — which is why the glTF
> helmet looked right while **editor primitives and foliage did not.**

A base colour read from the wrong bytes is exactly "trees salmon/pink, grass
white". The fix is **still in tree and load-bearing** —
`material/pool.rs:8-12` carries the constraint as a doc comment on the struct
(*"The WGSL mirror of this struct must not use `vec3<f32>`"*), and
`pool.rs:275` names it as "the failure mode that cost a whole session". It was
fixed in all four shaders declaring `Material`.

`b9d1e68` **did edit `context.md`** (42 lines changed) — it simply did not
remove the paragraph three sections away.

**Revised state of the list — 4 open, not 5:**

| # | Defect | State | Note |
|---|---|---|---|
| 1 | ~~Foliage renders with wrong colours~~ | **FIXED `b9d1e68`, 2026-08-08.** Strike from §18 with the commit cited. | User recalled it was fixed "wayyy earlier"; `git log -S` confirms. |
| 2 | **Editor primitives spawned at `on_init` do not appear** | **Probably stale too — verify first, do not investigate.** | `5bdae99`, 22 minutes after the note's sibling, fixed *"cubes being culled entirely"* — a cone sentinel of −1.0 passing the `cone.w > 1.0` guard, entering the test with a zero axis, `normalize(vec3(0))` → NaN → dropped. *"A cube's six face normals cancel exactly, so every cube in the engine hit it; planes survived."* The §18 note says the **plane and the cube** both fail, so this explains at most half. **Run `SOMNIUM_SHADOWTEST` once before spending anything on it.** |
| 3 | **BUG-013 water plane texture seams** | ⚪ Open | CPU mipmap wrap downsample leaving borders at tile edges. |
| 4 | **Clipmap dark-band / ribbon artifact** | ⚪ Open, UNRESOLVED | Six causes ruled out, five untested hypotheses ranked, and **the cheapest test — a clipmap-off capture at the same camera — has never been taken.** |
| 5 | **`GenerateReactive` unimplemented in `wgpu-ffx`** | ⚪ Open | `third_party/wgpu-ffx/src/lib.rs:25, 72, 179`. Water/transparents can ghost under FSR, which is default-on. |

**The finding underneath the finding.** Two of five entries are plausibly
resolved by commits made *the same evening they were written*, and nobody knew
until `git log -S` was run against them on 2026-08-18. The defect list is
append-only in practice. That is the same root cause as §5.14, and it is why
PORTAL-L opens with archaeology rather than debugging: **for every entry in
§18, `git log -S` the distinctive phrase before touching a debugger.** It cost
under two minutes per defect here and retired one outright.

**Owner: PORTAL-L.**

### 5.12 — D12: dependency hygiene is unmeasured

rust-doctor scores this dimension **50 / 100 — the worst of its five** — and
gives two lists that are immediately actionable:

**Nine crates resolved at incompatible major versions.** My raw count was 46
duplicates; this is the subset that actually matters (the rest are the usual
`objc2-*` / `ndk` / `jni` cross-platform spread):

`syn` 2.0.117 + 3.0.3 · `thiserror` 1.0.69 + 2.0.18 · `thiserror-impl` (same) ·
`nom` 7.1.3 + 8.0.0 · `ordered-float` 2.10.1 + 5.3.0 · `rustix` 0.38.44 +
1.1.4 · `bitflags` 1.3.2 + 2.11.1 · `webpki-roots` 0.26.11 + 1.0.9 ·
`rustc-hash` 1.1.0 + 2.1.2.

**Eleven declared dependencies referenced by no source in their package** —
this removes the need for a separate `cargo-udeps` pass:

| Package | Unused declaration |
|---|---|
| `somnium_asset` | `serde` |
| `somnium_audio` | `glam`, `tracing` |
| `somnium_ecs` | `rayon` |
| `somnium_ui` | `somnium_ecs` |
| `somnium_voxel` | `tracing` |
| `hello_engine` | `anyhow`, `tracing-subscriber`, `somnium_audio`, `somnium_ui`, `rand` |

`somnium_ecs` declared by `somnium_ui` and unused is the interesting one — it
means the UI crate's dependency graph claims a coupling that does not exist,
which is worth knowing before Phase CONTROL wires the reflection-driven Details
seam through exactly that boundary.

**Not flagged by rust-doctor, still real:** `glam` **0.19.0 and 0.29.3** both in
the graph — 0.19 pulled by `ilattice`, transitively from the voxel crate's
`block-mesh` / `ndshape`. Eleven of twelve workspace members use 0.29. The rule
only counts *major* version splits, so a `0.x` split slips past it; cargo still
treats them as incompatible, so there are two `Vec3` types that do not convert.

**And the gap neither tool covers:** **no `cargo-audit`, no `cargo-deny`, no
`deny.toml`.** For a repository whose `ATTRIBUTION.md` is 132 KB and whose
central legal claim is "patterns only, no source copied", having **no automated
licence check on 541 dependencies** is the inconsistency worth naming.

**Owner: PORTAL-C.**

### 5.13 — D13: one performance item, and it is not a budget question

DOOM closed with Coastal ground at **29.4 ms** against a §9 budget of
**≤ 16.6 ms**, and said honestly that the remaining cost is the terrain
material's 8 splatmap fetches plus the 32-wide scan, that distance does not
reduce it (DOOM-E proved this), and that the designed cheap path is Phase DF's
clipmap, gated on DF-E. **PORTAL does not reopen that.**

What PORTAL *does* take is the thing DOOM-A recorded and explicitly did not
schedule:

> `ReSTIR GI` on the overview is **8.427 ms with a standard deviation of 6.024
> and a maximum of 39.125**. The mean is not the story; something is
> occasionally costing an entire frame's budget in one pass.
> `Water prepass` ranges 0.636 → 3.254 ms on the same still camera.

A **still camera** producing a 4.6× spread in one pass is not a budget overrun,
it is non-determinism, and non-determinism is the thing that makes every other
gate in this phase unreliable. It is in scope as a *measurement* question: find
the source, or prove it is the instrument.

**Owner: PORTAL-N.**

### 5.14 — D14: documented facts that are no longer true

- **`context.md` §18 carries a defect fixed on 2026-08-08** (§5.11), and
  probably a second.
- `Cargo.toml` comment says engine code "targets 1.85"; `rust-toolchain.toml`
  pins **1.88**, and the post-Halcyon handoff already flags the drift.
- `context.md` §8.6 — "**No active regressions.** All major UI rendering… fully
  stabilized" — sits ~3 400 lines above §18's list of open defects.
- `context.md` §25P's foliage LOD text is superseded by the live behaviour
  recorded in the post-Halcyon handoff §4.3. The handoff says so explicitly,
  which means the primary architecture document is knowingly stale in at least
  one place.
- The handoff chain is four deep (`post_halcyon` → `halcyon` → `post_IV` →
  `post_25M2`), each superseding the last, and a new reader is told to read all
  of them. **PORTAL-D's migration is the opportunity to collapse this**, since
  the durable content of all four is going into `context.md` anyway.

**Owner: PORTAL-M**, with the handoff collapse handled inside **PORTAL-D**.

---

## 6. Sub-phases

Five tracks, fourteen sub-phases. **Every one closes with a command.**

### Track 1 — The Chamber (make the gate real)

#### PORTAL-A — the gate that can fail

Creates `dev records/phase PORTAL/`.

- `on:` gains `push: branches: [dev, main, master]`.
- New step `cargo fmt --all --check` — **and the mechanical formatting commit
  that makes it pass, landed separately and first**, so the 198-diff reformat
  never mixes with a behaviour change.
- `cargo test --workspace` replaces `--lib`. If a test needs a GPU it is marked
  `#[ignore]` with a reason and run in a separate `--ignored` job on the dev
  machine — not excluded by scoping the whole command.
- clippy becomes blocking at a level the tree passes (see PORTAL-B), with
  `-D warnings` on the crates that are clean and a ratchet for the rest.
- `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"`.
- A `concurrency:` group so a push during a run cancels the stale one.

**Exit:** a deliberately introduced bad format, a `dbg!`, and a failing
integration test each produce a red run **on `dev`**. Recorded as three run URLs
in `phase PORTAL/PORTAL-A_gate_proof.md`.

**Must not:** turn a test off to make the gate green. **Must not** add
rust-doctor here at its defaults — §2.1 limitation 2.

#### PORTAL-B — one lint policy, in one place

- Workspace `[workspace.lints.rust]` / `[workspace.lints.clippy]` in the root
  `Cargo.toml`; every member gains `[lints] workspace = true`. The eleven
  per-file `#![warn]` blocks are deleted in the same commit.
- Baseline: `clippy::all` = deny everywhere. `clippy::pedantic` = warn
  everywhere, with the four crates that already carry it staying at warn.
- The **41 `somnium_renderer` and 14 `somnium_ui` `clippy::all` warnings are
  fixed**, not allowed — they are the real ones.
- **All 84 unreasoned `#[allow]` attributes** (69 item-level, 15 crate-level,
  per rust-doctor) get a reason string or are deleted. This is the item where
  the two instruments agree most precisely, so it is the cheapest to verify.
- `clippy::undocumented_unsafe_blocks` = warn (deny in PORTAL-F).
- The existing 19 `#[allow(dead_code)]` are triaged: delete the code, or state
  why it stays.

**Exit:** `cargo clippy --workspace --all-targets -- -D warnings` exits 0;
rust-doctor's `unreasoned_allow_attribute` and `crate_level_allow` counts reach
**0**; the pedantic ratchet is documented in
`phase PORTAL/PORTAL-B_lint_policy.md` with every remaining `#[allow]` and its
reason.

**Must not:** blanket-`#[allow]` a category to reach zero.

#### PORTAL-C — configure the outside instruments

rust-doctor has now been run once (§2.1). This sub-phase turns a one-off number
into a standing instrument, and closes the dependency dimension it scored
**50 / 100** on.

- **`rust-doctor.toml` in the repo root**, with a written reason per deviation:
  - `cargo::path_dependency_outside_workspace` → **off** (28 false positives on
    genuine workspace members).
  - `clippy::indexing_slicing` → **warn, scoped**; the 604 hits are triaged in
    PORTAL-K, not here.
  - Categories promoted to `error` only once their count is 0, so the
    `--blocking` gate can actually bite. **Do not adopt the default
    `--blocking error`** — §2.1 limitation 2 shows it evaluates to
    `not-evaluated` on this tree and would be a second decorative check.
  - Record the target score. 65 → **≥ 85** is the phase budget (§8).
- **Fix the 11 unused dependencies** (§5.12). Each removal is verified by
  `cargo build --workspace --all-targets`. `cargo-udeps` is **not** needed.
- **Report the 3 parse failures upstream** — `scene_schema.rs`,
  `capture.rs`, `somnium_script_luau/src/lib.rs`, 11 errors — with a minimal
  repro. Until they parse, the structural findings are a lower bound and the
  score stays `authoritative: false`. This is worth doing: the tool is 0.2.0
  and the maintainer can act on a concrete repro.
- **`cargo-deny` with a `deny.toml`:** licence allow-list (MIT / Apache-2.0 /
  BSD / Zlib / Unicode), the 9 incompatible-major duplicates as a policy
  decision (allow with reason, or resolve), advisory database. This is the
  automated half of `ATTRIBUTION.md`'s claim, and it is the part rust-doctor
  does **not** cover.
- **`cargo-audit`** in CI, scheduled weekly rather than per-push.
- **`glam` 0.19 duplicate:** check whether `block-mesh` / `ndshape` /
  `ilattice` have a release on glam 0.29, or whether the voxel crate should own
  a thin conversion boundary. Record the decision; do not fork.
- **Explicitly rejected, again:** `debt_scanner.py` from `tech-debt-tracker`.
  §2 has the evidence. Recording the rejection is part of this sub-phase so
  nobody runs it again and files 12 523 tickets.

**Exit:** `rust-doctor.toml` and `deny.toml` in tree; `cargo deny check` green
in CI; rust-doctor score recorded before and after; the 11 unused deps gone;
`phase PORTAL/PORTAL-C_tooling.md` carries the rule-by-rule agreement table
from §2.1, the upstream parse-error report, and the glam decision.

### Track 2 — The Cube (the things you must not lose)

#### PORTAL-D — the record migrates into `context.md`

**`dev records/` stays untracked. Its durable content moves into the file git
already keeps.** Retention rule, volumes, and the fallback are in §5.10 —
read that section before starting; this is the sub-phase most likely to be done
badly by dumping rather than distilling.

Order, cheapest and highest-loss first:

1. **The measured numbers.** DOOM-A baselines, DOOM-B census, DOOM-C tile
   sweep, DOOM-E table, DOOM-F resolution results, CR occupancy, DF timings,
   XV-J compile gate. These are irreplaceable — they cost GPU hours and a
   deterministic harness to produce — and they compress to a handful of tables.
2. **The negative results**, with their reasoning intact: DOOM-C slower at
   every tile size and *why* the references use compute; DOOM-E's 925 changed
   pixels; the DOOM-F wrong diagnosis and the test that passed before and after
   the change. These are the highest-value paragraphs in the folder because
   they stop a future session repeating the work.
3. **The frozen contracts**, consolidated into one section rather than
   scattered across four handoffs.
4. **The open-defect state** — the DF band artifact's six ruled-out causes and
   five ranked hypotheses — merged into `context.md` §18 alongside PORTAL-L's
   revisions.
5. **The method notes**: `terrain_shading_occupancy_2026-08-14.md` in full
   (it is short and it is load-bearing), and the DF "do not reason from the
   shape of the artifact" rule.
6. **Collapse the four handoffs** (§5.14). Their live contracts are now in
   `context.md`; what remains is narrative. One `dev records/START_HERE.md`
   points at `context.md` and lists what is local-only.
7. **Measure `context.md` at the halfway mark.** If the projection exceeds
   ~8 500 lines, stop and take the `docs/context/` split (§5.10) instead of
   finishing into a 22 000-line file. Record the decision either way.

`dev records/README.md` is rewritten to say what it now is: a **local working
folder** for evidence and capture artifacts, whose engineering content lives in
`context.md`, and whose files are not backed up by git.

**Exit:** every measured table and negative result from `dev records/` appears
in `context.md`; a fresh reader can answer "what does Coastal ground cost and
why" without opening `dev records/`; `context.md`'s new line count is recorded
against the ~8 500 threshold; `START_HERE.md` exists and the four handoffs are
marked historical.

**Must not:** concatenate. **Must not** track `dev records/` in git. **Must
not** delete anything from `dev records/` — migration is a copy; the folder
stays as the local artifact store.

#### PORTAL-E — one command that proves a refactor changed nothing

**This is the sub-phase that makes Track 3 possible, and it must land before any
of it.**

The two instruments already exist and are good (`capture.rs`, `timing.rs`). What
does not exist is a single reproducible entry point, so today "prove the
refactor was invisible" is a PowerShell session someone reconstructs from a
README.

- A `cargo xtask verify` (a workspace `xtask` crate is the recommendation over a
  `.ps1`, because it is testable and cross-platform) that:
  - runs the fixed viewpoint matrix — `coastal-ground`, `coastal-overview`,
    `island`, `island-ground` — at fixed resolution, fixed frame count, fixed
    seed;
  - writes `.somcap` + `.somtime` per viewpoint;
  - compares against a named baseline set and prints the per-class `mean_abs` /
    changed-pixel counts and the per-zone `~ noise` verdicts;
  - **exits non-zero** if any pixel class changed beyond a stated tolerance or
    any zone moved outside the combined noise band.
- The tolerance is stated once, in the file, with the DOOM-C precedent as its
  justification: binned-vs-fullscreen parity was **2 pixels of 2 615 044**, so
  "invisible" already has a measured meaning in this project.
- It runs on the dev machine, **not** in CI (no adapter on the runner). CI's job
  is to check that the harness itself compiles and that its unit tests pass.
- **It reads its baselines from the tables PORTAL-D migrated into
  `context.md`**, or from local `.somtime` files verified against them. Under
  the no-tracking decision, the tracked table is the authority and the local
  file is the artifact.

**Exit:** `cargo xtask verify --baseline DOOM-A` reproduces the DOOM-A numbers
on an unmodified tree, and a deliberately altered constant in
`terrain_material.wgsl` makes it exit non-zero. Both recorded.

#### PORTAL-F — every `unsafe` justifies itself

- `clippy::undocumented_unsafe_blocks` from warn to **deny**.
- 30 blocks need comments: `somnium_physics/src/world.rs` (22),
  `somnium_ui/src/node.rs` (8); plus completing `somnium_ecs/src/archetype.rs`
  (15 of 22 uncommented) and `world.rs` (4 of 8).
- Each comment states **the invariant that makes the block sound**, not what the
  code does. `archetype.rs` is the priority: it is raw component storage and its
  invariants are the whole ECS's safety argument.
- Where writing the comment reveals the invariant is *not* actually guaranteed,
  that is a defect and gets a test. This is the expected outcome in at least one
  place and the sub-phase should budget for it.

**Exit:** `-D clippy::undocumented_unsafe_blocks` clean; any invariant found
unsound has a failing-then-passing test.

### Track 3 — The Companion (structure)

**Nothing in this track lands without PORTAL-E green on both sides of the diff.**
Each sub-phase's commit message quotes the verify output.

#### PORTAL-G — the `ConVar` registry

Source's pattern, adapted:

```rust
somnium_var! {
    SOMNIUM_RT_REFLECT: bool = true,
    help  = "Ray-traced water reflections (Halcyon VV).",
    flags = DEV | CAPTURE_AFFECTING,
}
```

- One registry, one read of the environment **at startup**, into a `Config`
  struct passed down. **Zero `env::var` calls inside `render()` or any per-frame
  path** — including `cpu_frustum_env_off()` and `cascade_cull_env_off()`.
- Unknown `SOMNIUM_*` in the environment → **warning at startup listing the
  nearest registered name.** Today a typo is silent, which is how a kill switch
  can appear not to work.
- `SOMNIUM_DUMP_VARS=1` prints the full table: name, type, default, current,
  help, flags. This is the missing documentation for 96 variables, and it cannot
  go stale.
- Flags carry meaning: `STARTUP_ONLY` variables that changed mid-run are
  reported rather than half-applied; `CAPTURE_AFFECTING` variables are written
  into the `.somcap` / `.somtime` header so a comparison against a baseline taken
  under different settings **fails loudly** instead of quietly.
- **Scope discipline:** PORTAL-G ships the registry and the data. It ships **no
  editor UI**. Phase CONTROL Seam 4 ("settings are data, environment variables
  are overrides") binds to it.

**Exit:** `grep -rn 'env::var' crates/*/src | grep SOMNIUM` returns only the
registry module. `SOMNIUM_DUMP_VARS=1` lists 96 entries, each with help text.
`cargo xtask verify` reports `~ noise` on every zone. A test asserts every
registered name is unique, and every one is either documented in `docs/editor/`
or flagged `DEV`.

#### PORTAL-H — passes take a struct, not twenty-one arguments

- Each pass's `record*` signature collapses to
  `fn record(&mut self, frame: &FrameResources, params: &XxxParams)`, where
  `FrameResources` holds the shared handles (device, queue, encoder, view
  buffers, depth, shadow atlas, env cube, samplers) and `XxxParams` holds the
  pass's own inputs.
- Named fields make a transposed `&TextureView` a compile error. Twenty-one
  positional arguments of four distinct types make it a runtime artifact.
- clippy `too_many_arguments` threshold set at 8 and enforced; the 17 `#[allow]`
  sites that currently suppress it are removed as they are fixed.
- **Not a frame-graph rewrite.** No automatic resource lifetime tracking, no
  barrier inference, no DAG scheduler. That is a phase of its own and PORTAL
  explicitly does not start it. This is a signature change.

**Exit:** no function in `somnium_renderer` exceeds 8 parameters without a
reasoned `#[allow]`; verify `~ noise`; captures byte-class-identical.

#### PORTAL-I — the seven complex functions

Order matters — cheapest and least contended first. **Targets 6 and 7 were added
by rust-doctor's complexity pass** (§2.1); a line-count scan alone would have
missed both.

1. `icons::rasterize` (506 lines) — pure CPU, SVG → atlas, easy to test.
2. `process_outgoing` (999, `somnium_ui`) — after PORTAL-J gives it a harness.
3. `apply_inspector_color` (cyclomatic **38**, cognitive **76**) — small enough
   to be a warm-up on the `app.rs` dispatcher family.
4. `window_event` (336 lines, cyclomatic **77**) — the input path.
5. `about_to_wait` (792 lines, cyclomatic 69) — the frame loop.
6. `handle_editor_event` (1 739 lines, **cyclomatic 381 / cognitive 484**) — 48
   `EditorEvent` variants; splits by variant family. **Coordinate with Phase
   CONTROL**, which also touches this dispatcher; PORTAL landing first gives
   CONTROL a smaller surface.
7. `Renderer::render` (1 939 lines) — **last, and only with PORTAL-E and
   PORTAL-H both landed.**

The containers matter too: `impl Engine<G>` at **2 927 lines** and `app.rs` at
**5 530** should end the sub-phase split across modules by concern, not left as
one impl block that happens to contain shorter functions.

For `render()` the target is not "shorter" but **"the ordering contract is
data"**: the ~16 ordering comments become an explicit, tested pass sequence — a
list a test can assert against ("water prepass is after visibility depth and
before shading", "overlays use the unjittered view-projection"). Every constraint
named in the post-Halcyon handoff §4.4 and DOOM §12 becomes an assertion.

**Exit:** no function over 300 lines or cyclomatic complexity 40 in the
workspace; rust-doctor `complex_function` and `oversized_unit` counts recorded
before and after; a test that fails if two passes are transposed;
`cargo xtask verify` `~ noise` and capture-identical on all four viewpoints,
output pasted into `phase PORTAL/PORTAL-I_parity.md`.

**Must not:** change a default, a uniform, or a shader in this sub-phase. If a
bug is found during the move it is recorded and fixed in a **separate** commit,
so the parity claim stays honest.

### Track 4 — Test chambers

#### PORTAL-J — `somnium_ui` gets a harness

21 394 lines at 3.7 tests/kloc, zero integration tests, and a documented history
of layout bugs found by eye. The layout engine is deterministic CPU code; there
is no reason it is untested.

- A headless harness: build a widget tree, run measure/arrange, assert rects.
- Regression tests for **every bug in `context.md` §8.5** — `RootControl`
  infinity, invalidation not propagating to ancestors, log-panel overlap. Those
  are three known-good test cases already written up in prose.
- `process_outgoing`'s message dispatch tested per message type.
- `resolve_content_target` already has its own tests (§17.20.1) and is the model
  to follow — a free function with its own test module, because it is the one
  place a typed string becomes a filesystem path.
- Target: **≥ 10 tests/kloc**, matching the ECS.

**Exit:** the three §8.5 bugs each have a test that fails against a reverted fix.
`somnium_ui` at ≥ 10 tests/kloc.

#### PORTAL-K — physics, asset, audio, and the panic boundary

- `somnium_physics`: 1 test → a real suite. Body creation/destruction, the
  heightfield path, the character controller's `grounded` heuristic (§17.19.2
  says it *is* a heuristic — that deserves a test pinning its stated behaviour),
  and the Jolt FFI lifetime assumptions PORTAL-F just wrote down.
- `somnium_asset`: 6 tests / 820 LOC over glTF import — the malformed-input path
  deserves the treatment `script_threat_model.rs` gives Luau. A truncated and
  adversarial glTF corpus. (`load_gltf` is also cyclomatic 25 / 201 lines.)
- `somnium_audio`: 0 tests. Small crate, but 3 of its 93 lines are clippy
  warnings; add smoke coverage.
- **Panic-boundary audit**, scoped by reachability rather than by count. The raw
  numbers are 142 `.unwrap()` in `somnium_core/src`, 65 in `somnium_ecs/src`, 39
  in `somnium_renderer/src`, plus rust-doctor's 95 `unwrap_used`, 33
  `expect_used` and **604 `indexing_slicing`**. Most are sound — a `Mutex` lock,
  a just-inserted key, a fixed-size array. **The only ones PORTAL converts to
  typed errors are those reachable from a file the user chose, a scene they
  loaded, or a script they wrote.** Priorities: `scene_schema.rs` (28 unwraps),
  `reflect_registry.rs` (26), `script_bridge.rs` (14), `terrain/heightmap.rs`
  (9), `terrain/splat.rs` (6). A malformed `.somnium` file must not take the
  editor down.
- The 7 `clippy::unreachable` in `app.rs` are reviewed in the same pass: each is
  either provably unreachable (documented) or a silent crash waiting on an enum
  variant someone adds later.

**Exit:** a corpus of malformed scenes, glTF files and heightmaps loads without a
panic; each crate above has a stated density and meets it; the reachable-panic
list is recorded with a decision per entry.

### Track 5 — Records and close-out

#### PORTAL-L — the open defects, starting with archaeology

**Step zero, before any debugging** — and it is what retired defect 1 in under
two minutes on 2026-08-18:

> For every entry in `context.md` §18, run `git log -S'<distinctive phrase>'`
> and `git log --grep` over the symptom, and read the messages of every commit
> made in the hours after the entry was written. **Two of five entries were
> plausibly fixed the same evening they were recorded.**

Then, for what survives:

| Defect | First move (cheapest test that eliminates a hypothesis) |
|---|---|
| ~~Foliage salmon/pink, grass white~~ | **Closed by archaeology.** `b9d1e68`, 2026-08-08 — `GpuMaterial` `vec3` alignment; every material past index 0 decoded from the wrong bytes. Fix still in tree with the constraint documented on the struct (`material/pool.rs:8`). **Action: strike from §18, citing the commit.** No debugging. |
| `on_init` primitives do not render | **Verify before investigating.** `5bdae99` fixed "cubes culled entirely" (cone sentinel −1.0 passing the `cone.w > 1.0` guard → `normalize(vec3(0))` → NaN) 22 minutes after the sibling note; planes were explicitly unaffected. Run `SOMNIUM_SHADOWTEST` once. If the cube now draws and the plane does not, the defect is **half its original size** and the plane is the whole remaining question. |
| BUG-013 water seams | CPU mipmap downsample at tile edges on repeating UVs; a unit test on the mip generator with a known tiling texture is a pure-CPU reproduction. |
| Clipmap dark bands | **Take the clipmap-off capture at the same camera that has never been taken.** The DF file's own first instruction. Then hypothesis 1 (foliage cards — one checkbox), then 4 (`SOMNIUM_CASCADE_CULL=0`, `SOMNIUM_CPU_FRUSTUM=0`). Save the frames into `phase DF/` *first*; the 2026-08-15 captures were lost to a conversation. |
| `GenerateReactive` | Wire it in `wgpu-ffx`, or record the decision not to and state what ghosting under FSR is accepted. It is the only real TODO in the tree and it affects a default-on path. |

Each surviving defect gets a decision: **fixed with a test**, or **retired with
a recorded reason**. None stays "not yet investigated".

**And a standing rule for §18, added by this sub-phase:** an entry carries the
commit that opened it, so the next archaeology pass is a date range instead of a
guess. A defect closed by a commit is struck **in that commit**, per §14.

**Must not:** reason from the shape of an artifact. The DF file records three
wrong attributions made in exactly that way. **Must not** start debugging
before step zero.

#### PORTAL-M — documentation stops drifting

- `#![warn(missing_docs)]` on `somnium_renderer` and `somnium_ui`, applied as a
  ratchet: module-level first, then the 1 410 items over the sub-phase's life,
  with the count committed so it can only go down.
- Fix §5.14's stale statements: the MSRV comment in `Cargo.toml`, `context.md`
  §8.6's "no active regressions", §25P's superseded foliage text.
- `context.md` §18 becomes the single defect list — now also carrying everything
  PORTAL-D migrated and PORTAL-L revised.
- `cargo doc --no-deps` with `-D warnings` in CI (from PORTAL-A).
- The handoff collapse is **PORTAL-D's**, not this sub-phase's, because the
  migration is what makes it possible.

**Exit:** `missing_docs` count committed and decreasing; `cargo doc` clean; no
statement in `context.md` contradicted by another section.

#### PORTAL-N — close-out, and the one measurement question

- **The ReSTIR GI variance** (§5.13): stddev 6.024, max 39.125, mean 8.427, on a
  *stationary* camera. Hypotheses to eliminate in order: (a) the profiler's own
  reservation/readback, since DOOM-A noted the reservation happens before the
  pass — eliminate by timing a pass known to be steady in the same run;
  (b) reservoir reuse / temporal history reset on a boundary condition; (c) a
  TLAS rebuild landing inside that scope; (d) driver or OS scheduling. Same
  treatment for `Water prepass` (0.636 → 3.254 ms). **The outcome may
  legitimately be "it is the instrument" — record it either way.**
- Re-run `cargo xtask verify` against DOOM-A and publish the full table. Every
  zone should read `~ noise`. **A phase that ends with the frame unchanged is the
  correct outcome for PORTAL** — anything else means a refactor moved something.
- Re-run `rust-doctor`; publish 65 → final, per dimension, and confirm the three
  parse errors are gone or still upstream.
- Update `context.md` (new §, PORTAL history), `ATTRIBUTION.md` (§13F for
  Source's ConVar pattern and rust-doctor), this file's §15, and
  `dev records/START_HERE.md` in **one commit**.

---

## 7. Sequencing, and why

```
A ──► B ──► C          (gates, policy, configured instruments)
│
├──► D                 (record migrates to context.md — early; highest loss)
│
└──► E                 (verify harness — BLOCKS all of Track 3)
        │
        ├──► F         (unsafe: independent, can run in parallel)
        │
        ├──► G ──► H ──► I     (registry → signatures → decomposition)
        │
        ├──► J ──► K   (test chambers; J unblocks I's step 2)
        │
        └──► L         (open defects; step zero is free, do it first)
                │
                └──► M ──► N   (docs, close-out)
```

Four ordering claims worth stating explicitly:

1. **PORTAL-E before any of Track 3.** Refactoring a 1 939-line frame graph
   without a one-command parity check is how this project's own history says
   things break — the FSR jitter contract, the depth convention, the unjittered
   overlay VP, and the cull invariant are *all* silent contracts inside that
   function. §17.7 and the DOOM write-ups exist because "it looked fine" was
   measured and turned out to be noise.
2. **PORTAL-G before PORTAL-H before PORTAL-I.** Removing env reads shrinks
   `render()`'s surface; collapsing 21-argument calls into structs is what makes
   the extraction in PORTAL-I mechanical rather than judgemental.
3. **PORTAL-L step zero can run today, before anything else**, and costs
   minutes. It already retired one defect and halved a second. Do it before
   scheduling any debugging effort against §18.
4. **PORTAL before Phase CONTROL.** CONTROL is a large editor phase that touches
   `handle_editor_event` (cyclomatic 381) and needs a settings-as-data layer
   (its Seam 4). PORTAL-G *is* that layer, and PORTAL-I makes the dispatcher
   reviewable. Landing CONTROL first means building the reach layer on top of
   structure PORTAL was going to change. If CONTROL must go first,
   **PORTAL-A / B / D / E should still land before it** — they are the cheap end
   of this phase and they protect everything CONTROL does afterwards.

---

## 8. Budgets

Targets to be argued with by measurement, not promises.

| Metric | Today | Target | Owner |
|---|---:|---:|---|
| `cargo fmt --check` diffs | 198 | **0**, gated | A |
| clippy warnings under `-D warnings` | 228 (job cannot fail) | **0** on `clippy::all`, ratcheted on pedantic | B |
| Unreasoned `#[allow]` attributes | 84 | **0** | B |
| **rust-doctor score** | **65** (`authoritative: false`) | **≥ 85**, and *authoritative* | C, I |
| rust-doctor parse failures | 3 files / 11 errors | **0** (fixed upstream or worked around) | C |
| Unused declared dependencies | 11 | **0** | C |
| Tests run in CI | ~590 of 826 | **826** | A |
| Functions > 500 lines | 5 | **0** | I |
| Functions > 300 lines | 8 | **0** | I |
| Max cyclomatic complexity | **381** | **≤ 40** | I |
| Max pass-call parameters | 21 | **≤ 8** | H |
| `env::var` calls outside the registry | 91 | **0** | G |
| `env::var` calls per frame | 3 (+2 helpers) | **0** | G |
| `unsafe` blocks without `SAFETY` | 30 | **0**, denied | F |
| `somnium_ui` tests / kloc | 3.7 | **≥ 10** | J |
| `somnium_physics` tests | 1 | **≥ 15** | K |
| **Measured results reachable from `context.md`** | ~0 (all in untracked `dev records/`) | **all DOOM/CR/DF/XV tables + negative results** | D |
| `context.md` size after migration | 4 594 lines | **≤ ~8 500**, or split to `docs/context/` | D |
| Licence / advisory audit | none | `cargo deny check` green in CI | C |
| Defects in §18 | 5 listed / 4 open | **0 open**, each entry carrying its opening commit | L |
| Undocumented public items (renderer + ui) | 1 410 | committed and **monotonically decreasing** | M |
| **Coastal-ground frame** | **29.4 ms** | **29.4 ms — unchanged** | phase |
| **`~ noise` on every `.somtime` zone** | — | **required at every Track 3 exit** | E |

The last two are the point. **PORTAL is a phase whose success condition includes
the frame time not moving.**

---

## 9. Measurement contract

Inherited from DOOM §8, restated because Track 3 depends on it:

1. **No claim without a command.** Every number in every PORTAL record carries
   the command that produced it.
2. **No win from a screen-capture frame delta.** This project measured 0.776 →
   2.018 ms across three runs of one identical build.
3. **A comparison inside the combined noise band is `~ noise`, not a win.**
4. **A refactor's claim is "nothing changed", and it is proved by
   `cargo xtask verify`**, not by review.
5. **A negative result is a result.** DOOM-C and DOOM-E are the model: built,
   correct, default off, measured slower, written up. If PORTAL-H's struct
   refactor costs measurable frame time, that is a finding, and it is recorded
   rather than hidden.
6. **Evidence is saved before it is discussed.** The DF band captures were lost
   to a conversation; that is written into `phase DF/` as a warning and applies
   here.
7. **A defect is dated against git before it is debugged.** §5.11 is the worked
   example: `git log -S` retired an entry that had sat open for ten days.
8. **A score is not a measurement.** rust-doctor's 65 is `authoritative: false`
   on an incomplete run. Quote it with that attached, every time.

---

## 10. Risks and controls

| Risk | Severity | Control |
|---|---|---|
| **Track 3 silently breaks a frame contract** (jitter, depth convention, unjittered overlay VP, cull invariant) | **High** | PORTAL-E lands first and is non-negotiable; every Track 3 commit pastes verify output; the contracts in post-Halcyon §4.4 / §6 become assertions in PORTAL-I |
| **PORTAL-D concatenates instead of distilling** and `context.md` becomes 22 000 lines | **High** | §5.10's retention rule; the halfway-mark size check against ~8 500 lines; the `docs/context/` fallback decided in advance rather than in a panic |
| **`dev records/` is lost before PORTAL-D runs** | **High** | D is sequenced immediately after A; nothing in the folder is deleted by any sub-phase; the user's own backup of the machine is the only other control and is outside this phase's reach |
| A formatting or lint commit hides a behaviour change | Medium | Mechanical commits land alone, never mixed; run `cargo xtask verify` after the reformat too |
| The registry (G) changes a default by accident | **High** | A test asserts the registered default of all 96 equals the value read today; `CAPTURE_AFFECTING` flags land in `.somcap` headers so a baseline mismatch fails loudly |
| **rust-doctor installed at defaults becomes a second decorative gate** | Medium | §2.1 limitation 2 is explicit; PORTAL-C sets blocking levels per category and only promotes a category once its count is 0 |
| A session chases rust-doctor's 604 `indexing_slicing` wholesale | Medium | PORTAL-K scopes the panic audit by *reachability from user data*, not by count; the rule is configured to `warn` in `rust-doctor.toml` with that reason written down |
| Making clippy blocking stalls all work | Medium | Ratchet, not a cliff: `clippy::all` denied (55 real warnings, one sitting), pedantic warn-only with a committed count |
| PORTAL-L defect 2 turns out to be the cull/instance invariant | Medium | That would be *good* — it is also DF hypothesis 4. Investigate both under one head; do not fix inside a Track 3 commit |
| Someone "fixes" the frame time during PORTAL | **High** | Explicit non-goal; §11; the budget table requires 29.4 ms to be **unchanged** |
| Phase CONTROL starts mid-PORTAL and both touch `handle_editor_event` | Medium | §7 sequencing; if CONTROL must start, A/B/D/E land first and PORTAL-I steps 6–7 are deferred |
| The 198-diff reformat collides with in-flight work | Low | Land it as the first commit of the phase, on a quiet tree, `git status` clean beforehand |

---

## 11. Must not do

1. Do not retune Great Lakes water, XV look numbers, the Island recipe, or
   foliage LOD / impostor / cull distances.
2. Do not shrink `TERRAIN_LAYER_COUNT` or the `GpuTerrainMaterial` layout — and
   do not reintroduce a `vec3` into any WGSL mirror of a `repr(C)` struct
   (`material/pool.rs:8`; this is what §5.11's defect was).
3. Do not turn Clipmap, tile binning, the aerial terrain pipeline, hex, POM, or
   World Cache on to win frame time. DOOM measured all of them.
4. Do not reintroduce per-pixel terrain sample-count LOD.
5. Do not put water or transparents in the TLAS; do not remove `trace_ssr`.
6. Do not change a default, a uniform, or a shader inside a Track 3 commit.
7. Do not turn a test off, `#[ignore]` it without a reason, or scope a CI command
   to make a gate green.
8. Do not blanket-`#[allow]` a clippy category to reach zero.
9. Do not overwrite `DOOM-A_*.somtime`, or any baseline — and once PORTAL-D has
   run, do not edit a migrated table in `context.md` to match a re-run.
10. Do not invent evidence PNGs or `.somtime` files.
11. Do not build editor UI for the `ConVar` registry — that is Phase CONTROL.
12. Do not start a frame-graph rewrite (resource lifetimes, barrier inference,
    DAG scheduling). PORTAL-H is a signature change and stops there.
13. Do not copy source from Valve/Source, UE5, id Tech, Fyrox, Flax, O3DE or
    Godot. Patterns only, cited in `ATTRIBUTION.md`.
14. Do not report a win from a screen-capture frame delta.
15. Do not wire `debt_scanner.py` into any gate (§2).
16. **Do not add `dev records/` to version control.** Decided 2026-08-18.
17. **Do not delete anything from `dev records/`.** PORTAL-D copies.
18. **Do not debug a §18 entry before running `git log -S` against it.**

---

## 12. Evidence plan

`dev records/phase PORTAL/`, created by PORTAL-A. **Local, untracked** — like
every other evidence folder — with its durable content migrating into
`context.md` at PORTAL-N per the rule PORTAL-D establishes.

| File | Sub-phase | Contents |
|---|---|---|
| `PORTAL-A_gate_proof.md` | A | Three deliberately-red CI runs plus the green baseline |
| `PORTAL-B_lint_policy.md` | B | Final lint table; every surviving `#[allow]` and its reason |
| `PORTAL-C_tooling.md` | C | rust-doctor before/after scores, the §2.1 rule-agreement table, `rust-doctor.toml` and `deny.toml` rationale, the upstream parse-error repro, the glam decision, the `debt_scanner` rejection |
| `PORTAL-D_migration.md` | D | What migrated, what stayed local, the `context.md` size check against ~8 500 lines, and the split decision |
| `PORTAL-E_verify.md` | E | Harness contract, tolerances, the deliberate-failure proof |
| `PORTAL-F_unsafe.md` | F | Each block's invariant; any unsoundness found |
| `PORTAL-G_convars.md` | G | The 96-entry table as generated by `SOMNIUM_DUMP_VARS=1` |
| `PORTAL-H_signatures.md` | H | Before/after parameter counts; verify output |
| `PORTAL-I_parity.md` | I | Per-viewpoint capture diffs and `.somtime` verdicts for each of the seven functions; complexity before/after |
| `PORTAL-J_ui_tests.md` | J | The §8.5 regression tests and what they pin |
| `PORTAL-K_boundaries.md` | K | The malformed-input corpora; the reachable-panic list with a decision per entry |
| `PORTAL-L_defects.md` | L | The archaeology results, then a decision per surviving defect |
| `PORTAL-N_closeout.md` | N | Final verify table vs DOOM-A; rust-doctor 65 → final; the ReSTIR variance verdict |

Captures after tonemapping, per `dev records/README.md`. `.somcap` and
`.somtime` files are saved **before** they are discussed.

---

## 13. Bibliography

Patterns only. No source copied. To be cited in `ATTRIBUTION.md` §13F, created by
PORTAL-A.

- **Valve, Source engine — `ConVar` / `ConCommand`.** The registered-tunable
  pattern: name, default, range, help string, flags (`FCVAR_CHEAT`,
  `FCVAR_ARCHIVE`, `FCVAR_REPLICATED`). From public documentation and the Valve
  Developer Community wiki; **no Source SDK source consulted or copied.** Basis
  for PORTAL-G's flags, the startup-only distinction, and the dump listing.
- **rust-doctor 0.2.0** — github.com/arthjean/rust-doctor. 62 curated rules
  across security / correctness / reliability / performance / maintainability /
  dependencies, driving `cargo clippy --workspace --no-deps -- -A clippy::all -W
  <rules>` plus its own `structure::*` and `cargo::*` analyses; local-only, no
  telemetry; `rust-doctor.toml`, `--rule id=level`, `--category cat=level`,
  `--blocking`, `--json`; GitHub Actions integration. **Run against this tree on
  2026-08-18: score 65, `authoritative: false`, 3 parse failures, default gate
  `not-evaluated`** — §2.1. Adopted advisory-first and configured.
- **`cargo-deny` and the RustSec advisory database** — licence allow-listing,
  duplicate-version policy, advisories. The automated half of
  `ATTRIBUTION.md`'s claim, and the part rust-doctor does not cover.
- **Rust API Guidelines** — `#[must_use]`, error types, and `missing_docs`
  expectations behind PORTAL-B and PORTAL-M.
- **The Rustonomicon, "Working with Unsafe"** — the invariant-not-behaviour form
  PORTAL-F requires of a `SAFETY` comment.
- **The `cargo xtask` pattern** (matklad, *Cargo Xtasks*) — a workspace member as
  the project's task runner, so PORTAL-E is compiled and testable rather than a
  shell script.
- **In-tree, and the real references:** `phase_DOOM.md` §8 (measurement
  contract), `phase DOOM/README.md` (the negative results and the
  wrong-diagnosis record), `terrain_shading_occupancy_2026-08-14.md` (uniforms do
  not delete WGSL), `phase DF/DF-OPEN_clipmap_band_artifact.md` (the method
  note), `post_halcyon_audit_handoff.md` §4.4 and §6 (the frame contracts
  PORTAL-I must assert), `context.md` §17.7 (why the profiler exists), §17.18.4–5
  (the threat-model and decision-record style PORTAL-K should copy), §18 (the
  defect list PORTAL-L revises), and commits `4aceadb` / `5bdae99` / `b9d1e68`
  of 2026-08-08 (the §5.11 archaeology).

---

## 14. Handoff rule

Implementation and tests are truth; this file is a plan written on 2026-08-18
against HEAD `45a3df8`. Line numbers drift — every one in §5 was read from the
worktree on that date and should be re-verified, not trusted.

A PORTAL sub-phase closes by updating **this file's §15 status table**,
`context.md`, and `ATTRIBUTION.md` **in the same commit** as the code. A
sub-phase that lands code and updates the record later has broken the rule the
rest of this folder is built on.

**And the rule §5.11 earned:** a commit that fixes a defect listed in
`context.md` §18 **strikes that entry in the same commit**. `b9d1e68` edited
`context.md` and still left its own fix recorded as an open bug three sections
away. That is the cheapest possible failure to prevent and it cost ten days.

**AI disclosure:** produced by reading `context.md`, `ATTRIBUTION.md`, every
markdown in `dev records/`, `docs/editor/`, `.github/workflows/ci.yml`, the
workspace `Cargo.toml` / `Cargo.lock`, and the source tree; and by running
`cargo clippy --workspace --all-targets`, `cargo test --workspace`,
`cargo fmt --all --check`, `npx rust-doctor@latest --json` (r2), `git log -S`
against `context.md` §18 (r2), a function-length scan over all
`crates/*/src/**/*.rs`, and `debt_scanner.py` from
`engineering-advanced-skills:tech-debt-tracker` (rejected — §2). Every number in
§1 and §5 comes from one of those runs on 2026-08-18.

---

## 15. Status

| Sub-phase | Status |
|---|---|
| PORTAL-A — the gate that can fail | **Not started** |
| PORTAL-B — one lint policy | **Not started** |
| PORTAL-C — configure the outside instruments | **Not started** (rust-doctor run once, §2.1) |
| PORTAL-D — the record migrates to `context.md` | **Not started** |
| PORTAL-E — the verify harness | **Not started** |
| PORTAL-F — every `unsafe` justifies itself | **Not started** |
| PORTAL-G — the `ConVar` registry | **Not started** |
| PORTAL-H — pass signatures | **Not started** |
| PORTAL-I — the seven complex functions | **Not started** |
| PORTAL-J — `somnium_ui` test chamber | **Not started** |
| PORTAL-K — boundaries and panics | **Not started** |
| PORTAL-L — the open defects | **Step zero done** — foliage colour retired (`b9d1e68`), `on_init` primitives halved (`5bdae99`); 4 entries remain |
| PORTAL-M — documentation | **Not started** |
| PORTAL-N — close-out | **Not started** |

### Revision history

| Rev | Date | Change |
|---|---|---|
| r1 | 2026-08-18 | Initial plan. 14 deficiencies, 14 sub-phases. rust-doctor listed as un-runnable (no Node). PORTAL-D proposed tracking `dev records/`. |
| **r2** | **2026-08-18** | Node installed; **rust-doctor run** — §2.1 added, score 65, two new §5.4 targets, 11 unused deps, 9 incompatible majors, three tool limitations. **PORTAL-D inverted** at the user's direction: `dev records/` stays untracked, content migrates into `context.md` (§5.10 rewritten with volumes and a retention rule). **§5.11 rewritten**: foliage-colour defect found fixed by `b9d1e68` on 2026-08-08 and never struck; `on_init` primitives likely half-fixed by `5bdae99`; archaeology added as PORTAL-L step zero and as measurement-contract rule 7. |
