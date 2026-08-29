# MORROWIND-AC — Claude read-first handoff

> **Purpose:** learn the current Somnium renderer and return a scoped design
> report for MORROWIND-AC. This is a **read-only turn**. Do not edit, create,
> format, build, test, stage, or commit repository files. Stop after reporting
> findings and wait for explicit implementation approval.
>
> **Snapshot:** `dev` at `439b6b6` (`feat(renderer): stream terrain material
> pages`), 2026-08-29. Verify `git status` and `git log -5 --oneline`; later
> repository state is authoritative.

## 1. Mandatory skills

Read and apply these skills before the audit:

1. `C:\Users\adhir\.claude\plugins\cache\claude-code-skills\engineering-skills\2.9.0\skills\senior-architect\SKILL.md`
   — map dependencies and compare design alternatives.
2. `C:\Users\adhir\.agents\skills\codebase-design\SKILL.md`
   — keep complex GPU behaviour behind deep pass-module interfaces; use its
   terms *module*, *interface*, *seam*, and *adapter* precisely.

The Graphify result identifies `SomniumRenderer` as a high-betweenness bridge
(degree 169). Treat that as a warning: AC should deepen focused render-pass
modules, not add another large block of policy to `renderer.rs`.

## 2. Scope correction to verify

The old plan lists five AC items in `dev records/phase_MORROWIND.md` §8:
OIT, SMAA, subsurface scattering, contact shadows, and conditional deferred
decals. The current tree appears to reduce the unfinished scope to **OIT and
SMAA**, but verify every row rather than trusting this handoff:

| Planned item | Current evidence |
|---|---|
| Deferred decals | Already shipped by CONTROL-O. Read `dev records/phase CONTROL/CONTROL-O_decals.md` and `dev records/phase CONTROL/README.md`. AC must not build a second decal path. |
| Subsurface scattering | `context.md` row 24S marks transmission/SSS complete. `MaterialAsset::transmission` reaches `GpuMaterial` and `shading.wgsl::transmitted_light`. Decide whether this satisfies AC's deliberately terse wording; do not silently expand it into a skin diffusion system. |
| Contact shadows | `context.md` row 24X marks the screen-space depth march complete. It is exposed as **Contact Shadows** in generated Post Processing Details and documented in `docs/editor/lighting.md`. |
| OIT | Not found. The current forward transparency path CPU-sorts per object and uses ordinary alpha blending. |
| SMAA | Not found. FXAA, TAA, CAS, and FSR 3 already exist and interact. |

No `MORROWIND-AC.md` evidence record exists yet. Your report must end with a
verified scope table: **done / genuinely remaining / deliberately out**.

## 3. Efficient reading order

Use `rg` and Graphify as indexes, then read the named source files themselves.
Do not read all 386 KB of `context.md` linearly.

1. `context.md`: preamble/current phase; rows 21, 24F, 24S, 24X and 24AC;
   search for `transparent`, `FSR`, `TAA`, `FXAA`, `subsurface`, `contact
   shadows`, and `decal`.
2. `dev records/phase_MORROWIND.md`: §§3, 5.2, 6.6–6.7, AC under §8, and
   §§9–13 (prerequisites, GHOSTFENCE, acceptance, evidence).
3. Relevant completed records: `MORROWIND-A2.md`, `MORROWIND-C.md`,
   `MORROWIND-Z.md`, `MORROWIND-AB.md`, `MORROWIND-AD.md`, plus
   `dev records/phase CONTROL/CONTROL-D_material_authoring.md` and the
   CONTROL-O record above.
4. `ATTRIBUTION.md` clean-room rules and §13H license table; `CONTRIBUTING.md`.
5. Graphify: `graphify-out/.graphify_analysis.json` and
   `.graphify_semantic_new.json`. Relevant analysis communities are 195
   (FXAA), 237 (transparent pass), and 228 (shader system). Graphify was
   committed at `66d1ccf`, before AB/AD, so use it for structure—not current
   status.
6. Current code:
   - `crates/somnium_renderer/src/pass/transparent.rs`
   - `crates/somnium_renderer/src/shaders/transparent.wgsl`
   - `crates/somnium_renderer/src/pass/{fxaa,taa,postprocess,cas,fsr}.rs`
   - matching WGSL files and `pass/mod.rs`
   - `crates/somnium_renderer/src/renderer.rs` around transparent queue setup,
     frame-instance layout, pass 7.6, TAA, and post/present
   - `crates/somnium_renderer/src/{context,capability,shaders}.rs`
   - `crates/somnium_renderer/tests/shaders_validate.rs`
   - `crates/somnium_core/src/lib.rs` (`PostProcessComponent`),
     `reflect_registry.rs`, `editor_commands.rs`, and `app.rs`
   - `crates/somnium_asset/src/material.rs`
   - `docs/editor/{lighting,viewport}.md`

## 4. Current architecture to challenge, not rediscover

- `SomniumRenderer::submit` hides material routing from callers. `AlphaMode::Blend`
  commands enter `transparent_queue`; opaque commands enter the visibility path.
- Transparent instances occupy the tail of the shared instance buffer. They are
  sorted by object-origin distance, drawn after opaque terrain/water into the HDR
  target, depth-tested read-only, then resolved by TAA before exposure/bloom.
- The transparent shader currently implements only sun plus environment
  reflection—no clustered local-light loop or shadow-atlas lookup. Keep lighting
  parity separate from the OIT requirement unless evidence makes it necessary.
- FXAA is one LDR fullscreen pass after tone mapping and before editor chrome.
  It is skipped when TAA is effective or FSR succeeds. FSR is default-on and
  disables TAA/CAS; unsupported or unsafe FSR falls back to TAA. A third AA
  boolean could create checked no-ops, so inspect the state model before proposing
  SMAA's Details representation.
- The capability report code records storage-buffer/texture limits, but no
  generated hardware report is checked in. The plan's “PPLL likely” sentence is
  a hypothesis, not a decision.
- `docs/editor/lighting.md` already records that water/transparents lack an FSR
  reactive mask and may ghost. AC should name this interaction, not accidentally
  claim OIT fixes temporal reconstruction.

## 5. Questions the report must answer

### OIT seam

Compare at least:

- weighted-blended OIT as the portable path;
- per-pixel linked-list OIT as an optional exact(er) path;
- retaining sorted alpha only if it still has a clearly bounded role.

For each, state wgpu 30 requirements, target formats/bindings, pass count,
resize lifecycle, memory at 1920×1080 and 4K, deterministic overflow behaviour,
depth semantics, compositing order relative to water/TAA, and degradation on a
device that lacks the needed limits. Verify wgpu details from official docs or
local dependency source. Do not choose PPLL merely because the 2026 plan called
it likely.

### SMAA seam

Determine whether AC means SMAA 1x or a temporal variant. Map its edge,
blend-weight, and neighbourhood passes into the existing LDR FXAA slot. Account
for lookup/search textures and their licenses. Propose one truthful authored
state for None/FXAA/SMAA/TAA/FSR rather than independent toggles that can be
checked but ineffective—or explain why the present boolean model should remain.
Preserve the rule that editor UI, text, gizmos, and outlines are not blurred.

### Delivery surface

Forecast—not implement—the smallest file set, public interfaces, generated
Details fields, undo/serialization implications, Help updates, shader
registration/validation, and a Vvardenfell public-API exercise. Include tests
for CPU policy and resource sizing plus visual cases that cannot pass by
accident: intersecting transparent surfaces and thin diagonal geometry.

The eventual Track 7 completion must include matched `.somtime` runs on both
shipped maps, display-referred captures after tone mapping, GHOSTFENCE, workspace
library tests, `ATTRIBUTION.md`, `context.md`, Help, and a `MORROWIND-AC.md`
record. Flax is proprietary: it may inform architecture only. Implement from
public literature or permissively licensed primary sources; copy no code,
identifiers, constants, layouts, or comments.

## 6. Required response and stopping condition

Return at most about 1,200 words with:

1. verified residual-scope table;
2. present frame/data flow;
3. OIT options table and recommendation;
4. SMAA integration/state-model recommendation;
5. predicted file/test/evidence set;
6. unresolved blockers or measurements needed before coding.

Every claim must cite a repository path and symbol or section. **Completion is
the report, not a patch.** Make no repository change and wait for the user's
next instruction.
