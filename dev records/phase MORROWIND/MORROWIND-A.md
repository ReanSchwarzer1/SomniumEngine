# MORROWIND-A — the engine census

**Complete, 2026-08-24.** Track 0 (BALMORA). The plan's rule is that *"no code
in any other sub-phase is written until this exists"*, so this is the first
thing in the phase and it deliberately ships no engine capability at all.

## What §8 asked for, and what landed

| # | Asked | Landed |
|---|---|---|
| 1 | The census script, checked in beside its output | `tools/census/generate.py` (511 ln) → `MORROWIND-A_census.md`. Lines and tests per crate, absent-system greps, WGSL inventory, schema count, **public-API surface per crate**, and dependency justification. |
| 2 | The Fyrox diff — every `fyrox-ui/src/` module against `somnium_ui/src/` | `MORROWIND-A_fyrox_diff.md`. All ~66 modules, four verdicts, one-line reasons. |
| 3 | The license audit, including the Flax question | `MORROWIND-A_license_audit.md`. **The Flax question is answered and the answer changes the plan.** |
| 4 | Evidence folder, `ATTRIBUTION.md` §13H, `context.md` §17.6 | All three. §13E/F/G untouched. |
| 5 | `examples/vvardenfell` as an empty program | `examples/vvardenfell/` — 1 dependency, builds, opens a window, draws nothing. |
| — | §10: *"MORROWIND-A builds it as the first GHOSTFENCE row"* | `tools/ghostfence/` — the gate, a standard-library PNG codec, a two-sided perceptual comparator, and six tests of the comparator. |

## The four findings worth carrying forward

### 1. §4 was already 27,329 lines stale

The plan measured the tree on 2026-08-23 at **113,892 lines / 945 tests**. On
2026-08-24 the census measures **141,221 lines / 1,211 tests** — Phase CONTROL
landed in between. Nothing is wrong with §4; it is simply the failure mode the
plan predicted, one day after it predicted it. Every figure in the phase record
now comes from a script, and GHOSTFENCE fails when the checked-in output no
longer matches the tree.

The shape of the finding survives intact: **the top three crates are 85.5%** of
the tree against §4's 85.1%, `somnium_audio` is still **93 lines with zero
tests**, `material/hlms.rs` is still 29 lines, `renderer/jobs.rs` is still 75.
The imbalance did not improve; it got 27,000 lines worse in the same direction.

### 2. Flax is proprietary, and the plan leaned on it

`FlaxEngine-master/LICENSE.md` is one line pointing at the Flax Engine EULA.
§6.6 listed three strict references and implied Flax was in the permissive set;
it is not. §6.5 takes Flax's `UICanvas` as the reference for Seam 4a, and §6.2
takes `Source/Editor/Surface/` as a reference for Seam 8a.

Neither is blocked, because neither is a transcription — "a UI root declares its
coordinate space" is an idea with permissive expressions in Godot and Unreal
both. But the rule changes: **MORROWIND-K reads Godot's `GraphEdit` and Fyrox's
`absm/` as primary**, with Flax demoted, and **MORROWIND-E must not name Flax's
identifiers in shipped code.**

A second, more general finding came out of the same read, and it is the one to
remember: **two of the five strict references were only detectable by reading
past the root license file.** Flax's root file is a pointer to a EULA. Daemon's
root file is BSD-3-Clause over a tree whose `gl_shader.cpp` — the exact file
MORROWIND-C wants — carries its own GPL-2.0-or-later header. *Check the header
of the file you are actually reading.*

### 3. `somnium_ui/src/widget.rs` is 217 lines against Fyrox's 2,148

That is not efficiency. It is the visibility, enabled, opacity, z-index,
tooltip, context-menu, hit-test-override and layout-transform machinery Somnium
never needed because the editor shell is a fixed arrangement of always-visible
panels. Track 1 needs most of it. The diff puts **19 Fyrox modules and roughly
18,000 lines of already-solved problem** onto named MORROWIND sub-phases, which
is §6.1's claim surviving measurement.

### 4. Eleven unreferenced dependencies, and the census now names them

The plan called out the dead `egui` triple as a symptom of nothing forcing the
dependency list to justify itself. The census generalises that check and finds
eleven more candidates: `glam` and `tracing` in `somnium_audio` (a 93-line
crate that imports a maths library it never calls), `serde` and `base64` in
`somnium_core`, `rayon` in `somnium_ecs`, `pollster` in `somnium_renderer`,
`tracing` in `somnium_script_luau` and `somnium_voxel`, `anyhow` and `rand` in
`hello_engine`, and workspace-wide `anyhow`.

**None is removed here.** §4.7 assigns dependency hygiene to Phase PORTAL's CI
gates, and a census that quietly edits manifests is not a census. They are now
*visible*, with a named verdict each, which is the difference this sub-phase is
allowed to make.

## GHOSTFENCE, and what it says today

`python tools/ghostfence/run.py --fast`:

```
  PASS  census            MORROWIND-A_census.md matches the tree
  FAIL  toolchain         Cargo.toml declares wgpu '29.0', frozen line says '30.0'
  PASS  one-job-system    no bare spawns; 2 exemptions, each with a reason
  FAIL  no-second-system  somnium_core/src/jobs.rs defines JobRegistry;
                          renderer/material/hlms.rs defines MaterialSystem
  SKIP  golden-images     no reference set yet - <capture command>
  SKIP  tests             --fast
```

**Two red rows, and both are correct.** The gate is deliberately failing on
exactly the three things Track 0's remaining sub-phases fix:

- `toolchain` is red because `FROZEN_TOOLCHAIN` in the gate already says
  wgpu 30 and `Cargo.toml` still says 29. **MORROWIND-A2 turns it green.**
- `no-second-system` is red because `JobRegistry` still lives in
  `somnium_core` and `MaterialSystem` still exists. **MORROWIND-B and -C turn
  it green** by moving the first and deleting the second.

A gate that passed on the day it was written would not be a gate. Writing the
frozen line as the *destination* rather than the current state is what makes
A2's bump a green light rather than a paperwork exercise.

The two `SKIP` rows are the honest ones. Golden images need a windowed GPU run;
the gate prints the capture command instead of claiming a pass. `--strict`
promotes both to failures, which is what a release gate wants.

## The comparator is tested, not asserted

`tools/ghostfence/test_ghostfence.py` — six cases, all green:

- a PNG round-trip, because every other case is meaningless if the codec loses data;
- identical images passing at `Threshold::exact()`;
- **one pixel moving 245 levels failing**, which is the widget-drifted-a-pixel case a mean-only comparator sleeps through;
- ±1 encoder noise passing, because a gate that cries wolf gets switched off;
- a whole image drifting 8 levels failing on the *fraction* budget while staying under the `max_channel` ceiling — the tone-map-drift case, proving the two thresholds catch opposite shapes;
- a size change failing rather than crashing.

## Not done here, deliberately

- **No golden reference images.** They need a GPU session. §13's *"Do not invent PNGs"* is the rule and it is followed.
- **No dependency removals.** Phase PORTAL owns that (§4.7).
- **No seam reconciliation beyond the two audits.** §12.8 asks MORROWIND-A to reconcile §7's seams against what CONTROL shipped; the one that mattered — CONTROL's `JobRegistry` versus Seam 1 — is settled in `crates/somnium_core/src/jobs.rs:3`, which already says *"The public surface is intentionally narrow so Phase MORROWIND can move this module into `somnium_jobs` without changing call sites."* CONTROL did what Appendix A.6 asked. MORROWIND-B moves it.

## Files

```
+ tools/census/{__init__.py,generate.py}
+ tools/ghostfence/{__init__.py,png.py,golden.py,run.py,test_ghostfence.py}
+ examples/vvardenfell/{Cargo.toml,src/main.rs}
+ dev records/phase MORROWIND/{README.md,MORROWIND-A.md,MORROWIND-A_census.md,
                               MORROWIND-A_fyrox_diff.md,MORROWIND-A_license_audit.md}
~ Cargo.toml            (workspace member: examples/vvardenfell)
~ ATTRIBUTION.md        (§13H opened; §13E/F/G untouched)
~ context.md            (§17.6 numbering retired; MORROWIND roadmap entry updated)
```

**Exit condition — "the census command reproduces §4 without a human editing a
table" — met.** `python tools/census/generate.py` regenerates every figure, and
`--check` fails when the checked-in report drifts.
