# MORROWIND-C — the shader system (Seam 3)

**Complete, 2026-08-24**, with one interactive measurement owed and named below.
Track 0 (BALMORA). New crate `somnium_shader`, zero workspace dependencies.
`crates/somnium_renderer/src/material/hlms.rs` is **deleted**.

## What was there before

```rust
pub struct MaterialSystem {
    /// Cached pipelines mapped by their configuration hash.
    _pipeline_cache: HashMap<u64, wgpu::RenderPipeline>,
}
// In a full implementation, this would take a material descriptor,
// hash it, check the cache, and if missing, construct the WGSL
// shader source, compile it, and create the wgpu::RenderPipeline.
```

Twenty-nine lines under a doc comment describing Ogre-Next's HLMS, an
underscore-prefixed field no code read, and a trailing comment beginning *"In a
full implementation, this would…"*. **The reference architecture was documented
and never built.** This sub-phase built it and deleted the file.

Composition, meanwhile, lived at thirteen pass construction sites as `format!`
calls over `include_str!`. The order of those calls was load-bearing, invisible
from the shader, and duplicated — `shading.rs`, `restir_gi.rs`,
`lighting_extra.rs` and `water_reflection.rs` each carried their own copy of an
overlapping list. `restir_gi.rs`'s comment said *"`tests/shaders_validate.rs`
pins this exact concatenation"*, which describes a convention rather than a
mechanism, and §4.3 of the plan is right that this blocks Track 5: skinning is a
permutation, and there was nowhere to put one.

## What §8 asked for, and what landed

| # | Asked | Landed |
|---|---|---|
| 1 | Composition: named includes, cycle detection, resolved-source cache; **the 48 existing shaders migrate; `shading.wgsl` is the acceptance case** | `somnium_shader::compose`. **All 51 shaders** now go through the registry; thirteen were composed roots and all thirteen carry `//!include` headers. Zero `include_str!("../shaders/…")` calls remain in pass code. |
| 2 | Permutation: `ShaderKey`, hash-keyed cache, compile-time-registered defines | `somnium_shader::cache` + `define::SKINNED` registered and keyed. A `//!if` on an unregistered name is an **error**, not a silently disabled block. |
| 3 | Hot reload: watcher, module-granular invalidation, atomic swap, **toast with the naga diagnostic, never a silent revert** | `somnium_shader::watch` + `ShaderSystem::apply_reload` + `SomniumRenderer::reload_shaders` + a 4 Hz pump in `app.rs` that toasts. Pipeline rebuild is wired for the shading pass; see *Coverage* below. |
| 4 | Variant budget: a build-time report | `tools/shadercook/generate.py`, and a **GHOSTFENCE row** so it is a gate rather than a document. |
| 5 | Ahead-of-time: a `tools/` cooker | `tools/shadercook/`, doing the half that is honest for wgpu — see *On AOT* below. |
| — | **Exit: `hlms.rs` is deleted** | Deleted. |
| — | **Exit: adding a `SKINNED` define adds a variant without editing `renderer.rs`** | Proven by `a_skinned_variant_is_a_distinct_key_without_touching_the_renderer`. |
| — | **Exit: editing `brdf.wgsl` updates the running editor in under a second** | Built end to end; the *measurement* needs a GPU session. |

## The finding: naga 29 was hiding a wgpu 30 incompatibility

**This is the most important thing in this record.**

MORROWIND-A2 bumped wgpu 29 → 30 and reported 1,231 tests green. It left
`crates/somnium_renderer`'s `naga` **dev-dependency** on 29. `tests/shaders_validate.rs`
is the only thing in the tree that parses WGSL without a GPU, so for one whole
sub-phase **every shader was being validated by a different front end from the
one that would compile it.**

Bumping that dev-dependency to 30 as part of this sub-phase turned eleven of
sixteen shader tests red, all with the same message:

```
error: the `wgpu_binding_array` enable extension is not enabled
   ┌─ wgsl:70:37
   │
70 │ @group(0) @binding(4) var textures: binding_array<texture_2d<f32>>;
```

wgpu 30 requires `binding_array<…>` to sit behind an explicit `enable`
directive; wgpu 29 accepted it without one. Four files declare binding arrays —
`global_pool.wgsl`, `shadow.wgsl`, `transparent.wgsl`, `visibility.wgsl` — and
**every one of them would have failed at pipeline creation on the first frame.**
The engine, as committed at the end of A2, could not have started.

Three things follow, and they are worth more than the fix:

1. **A2's acceptance was not wrong, it was incomplete.** "1,231 tests pass" was
   true. The test that could have caught this was running a stale compiler.
2. **A version bump has to move every copy of that version.** `naga` is in the
   dependency graph twice — once through wgpu, once pinned in `dev-dependencies`
   — and only one of them was bumped. The dev-dependency is now removed in
   favour of the regular one, with a comment saying why, so there is one naga.
3. **This is the same shape as the `format!` duplication this sub-phase
   removes.** Two copies of a thing that must agree, with nothing enforcing that
   they do. It showed up twice in one day, in two unrelated places.

`enable wgpu_binding_array;` was added to the four files, and the resolver
hoists and de-duplicates it, so every module composing `global_pool.wgsl`
inherits it without repeating it.

## The second finding: a test validating a shader nobody builds

`the_transparent_module_validates` checked `{BRDF}\n{TRANSPARENT}`.
`TransparentPass::new` compiles `transparent.wgsl` **alone**, and the module
calls none of `brdf.wgsl`'s three functions. The test had been over-approximating
for as long as it had existed, and nothing could have noticed, because the test
and the pass each described the shader separately.

Making the test resolve through the same registry the pass uses found it in one
run. **There is now one description of what a shader is made of**, which is the
argument for Seam 3 stated as a defect rather than as a principle.

## The directive language, and where it stops

```wgsl
//!include "brdf.wgsl"
//!if SKINNED
//!include "skinning.wgsl"
//!endif
```

Four rules, and the fourth is the one that matters:

1. `//!include` takes one quoted module name.
2. `//!if` / `//!else` / `//!endif` are whole lines and **do not nest**.
3. A condition is one registered define name, optionally negated.
4. **Anything else is an error, not a best guess.**

The reasoning is the one that keeps a build system from growing a scripting
language: when a conditional wants to be cleverer than this, the honest answer
is a second module, and a resolver that permits the clever version removes the
pressure to write the honest one. `nesting_is_refused_with_a_reason` asserts the
error message says *"write a second module instead"*.

**`//!if SKINED` is a compile error.** This is the single most valuable check in
the composer: under a string-keyed permutation system the typo compiles cleanly,
produces a variant with the block missing, and is found weeks later by somebody
wondering why skinned meshes render untransformed.

### `enable` hoisting is not a convenience

WGSL `enable` directives are file-scoped and must precede every other
declaration. `restir_gi.wgsl` and `lighting_extra.wgsl` both declare
`enable wgpu_ray_query;`, and the old arrangement satisfied the rule by
concatenating those two files **first** — with a comment in each pass and each
test explaining why. That is a rule somebody has to remember, in four places,
forever.

The resolver lifts every `enable` and `requires` line out of every included
module, de-duplicates them, and emits them first. Get this wrong and the error
surfaces as a naga parse failure pointing at line 1 of a file nobody edited;
`enable_directives_are_hoisted_to_the_top_of_a_composed_module` is the check
that it is right.

## Hot reload, and the rule about failure

> *"a visible toast on failure with the naga diagnostic — **never a silent
> revert to the old pipeline**."*

The path is: a 250 ms poll of modification times → naga parses each dependent
variant → on success the module is installed and affected pipelines rebuild → on
failure **nothing is installed** and the diagnostic goes to a toast.

Four decisions worth recording:

- **Polling, not an OS notification API.** `somnium_shader` depends on wgpu and
  nothing else; the alternative is an inotify / `ReadDirectoryChangesW` /
  FSEvents dependency tree for a debug-only feature. Fifty-odd `stat` calls four
  times a second is unmeasurable, and it cannot *miss* an edit, because it
  compares state rather than consuming events.
- **A vanished file is not an edit.** Editors that save by rename make a file
  briefly absent; treating that as a change would invalidate every dependent
  variant and recompile them against a file about to be replaced.
- **naga became a regular dependency.** wgpu reports shader errors through an
  *async* error scope, and the whole point of this feature is a synchronous
  diagnostic in front of somebody who is looking at the viewport. naga is
  already in the graph through wgpu, so this costs no build time.
- **A toast, not a log line.** An author using hot reload is watching the
  viewport, not a terminal. A failed edit that only writes to stderr is
  indistinguishable from an edit that did nothing.

### Coverage, stated rather than implied

`ShadingPass::reload` rebuilds its shader module and pipeline in place, and
clears the lazily-built bin and split pipelines so they rebuild on demand.
`brdf.wgsl` composes into `shading.wgsl`, so **the plan's named exit case is the
one that is wired.**

**The other passes report their reload and keep their existing pipeline.** The
toast says how many variants are awaiting a pass-side reload, so the gap is
visible rather than mistaken for a shader that did not take.
`ShadingPass::reload` is the pattern — store the pipeline layout and format,
rebuild the module, re-run `make_pipeline`, and mutate nothing until the new
pipeline exists. Each remaining pass is a fifteen-line addition and none of it
can be verified without a GPU, which is why it is staged rather than guessed at.

## On AOT, and why the cooker does not cook

§8 item 5 asks for a cooker that *"compiles the shipped variant set at build
time so a release build has no first-use hitch."*

**wgpu has no offline pipeline cache that survives a driver update.** A cooked
pipeline binary is a file that is correct until the user installs a graphics
driver and then silently wrong, which is a worse failure than the hitch it
avoids. The honest AOT step for wgpu is warming the in-process cache at load,
which `ShaderSystem::request` exists for and which Track 4's cook will drive.

So `tools/shadercook/` does the half that is real and says what it is not doing:
it enumerates the variant space from the tree — every module, its transitive
includes, and every define any of them branches on — and fails when a module's
space exceeds 128. Today: **51 modules, 51 possible variants**, because no
shader has a `//!if` yet. `water_reflection.wgsl` composes six modules,
`clipmap_gen.wgsl` four, `census.wgsl` and `classify.wgsl` three each.

That report is a **GHOSTFENCE row**, so "the key is too coarse, split the
module" is a build failure rather than advice in a document.

## Tests: 34 in `somnium_shader`, plus the renderer's 16 shader tests

The composer, cache and watcher are GPU-free by design, which is what lets the
interesting half be tested at all. Named for what each catches:

| Test | The failure it catches |
|---|---|
| `a_diamond_include_emits_the_shared_module_once` | WGSL has no include guards; `shading` and `restir_gi` both want `brdf`, and a duplicated struct is a redefinition error. |
| `a_cycle_is_reported_as_a_path_not_a_stack_overflow` | The alternative is a stack overflow inside the resolver, which says nothing about which two files disagree. |
| `an_unregistered_define_is_an_error` | `//!if SKINED` producing a variant with the block quietly missing. |
| `enable_directives_are_hoisted_and_deduplicated` | The most confusing failure the composer can produce. |
| `invalidation_hits_dependents_and_nothing_else` | Clearing the cache would work and would rebuild every pipeline, turning "edit a file and see it" into "edit a file and wait". |
| `dependencies_are_tracked_per_variant_not_per_module` | `//!if SKINNED` means two variants of one module have different dependencies; per-module tracking invalidates both or neither, and both are wrong. |
| **`a_broken_reload_reports_naga_and_keeps_the_old_source`** | **Appendix A.7's named check.** Asserts all three: the diagnostic survives with its location, the old source is still what a pipeline would be built from, and the cache still holds the variant. |
| `a_reload_that_breaks_composition_is_also_non_destructive` | A bad `//!include` fails at a different layer from a syntax error; both must reach the same non-destructive path or one of them is a black screen. |
| `a_missing_file_is_not_a_change` | Save-by-rename recompiling against a file that is about to be replaced. |
| `every_registered_module_composes` | The migration's own regression test: resolves all 51 shaders, so a bad `//!include` fails in `cargo test`. |
| `the_ray_query_roots_compose_the_same_modules_they_used_to` | The three roots whose concatenation order changed still compose the same *set*. |

`apply_reload` takes a validator closure precisely so the failure contract is
testable without a GPU — naga is not linked into `somnium_shader`, and a
contract about failure handling that needs an adapter to test is a contract
nobody tests.

## The owed item

**"Editing `brdf.wgsl` updates the running editor in under a second" is not
measured.** Every part of it exists and is unit-tested — the watcher fires, the
resolver recomposes, naga validates, the pipeline rebuilds, the toast appears —
but the end-to-end latency claim needs the editor open on a GPU.

```bash
cargo run -p hello_engine            # then edit crates/somnium_renderer/src/shaders/brdf.wgsl
```

The same session should also do A.7's negative case: introduce a deliberate WGSL
syntax error and confirm the toast shows naga's message **while the viewport
keeps drawing**.

This is now the fourth item waiting on one windowed session, alongside A2's
`.somtime` parity, A2's capability report, and MORROWIND-A's first golden image.

## Files

```
+ crates/somnium_shader/{Cargo.toml,src/lib.rs,src/compose.rs,src/cache.rs,src/watch.rs,src/tests.rs}
+ crates/somnium_renderer/src/shaders.rs        the registry: 51 modules, watch paths, defines
+ tools/shadercook/generate.py                  the variant budget, and what AOT means for wgpu
- crates/somnium_renderer/src/material/hlms.rs  29 lines, deleted
~ crates/somnium_renderer/src/shaders/*.wgsl    13 roots gain //!include headers;
                                                4 gain `enable wgpu_binding_array;`
~ crates/somnium_renderer/src/pass/*.rs         30 files: composition and include_str! removed,
                                                `shaders: &Shaders` threaded through
~ crates/somnium_renderer/src/pass/shading.rs   + ShadingPass::reload
~ crates/somnium_renderer/src/renderer.rs       Shaders replaces MaterialSystem; reload_shaders
~ crates/somnium_renderer/src/material/mod.rs   hlms removed, with a note saying what was there
~ crates/somnium_renderer/Cargo.toml            somnium_shader; naga 29 dev -> 30 regular
~ crates/somnium_renderer/tests/shaders_validate.rs  validates composed sources, not copies
~ crates/somnium_core/src/app.rs                pump_shader_reload, 4 Hz, toasts
~ tools/ghostfence/run.py                       + shader-budget row
~ Cargo.toml                                    workspace member + dependency
```
