# MORROWIND-A2 — the wgpu 30 bump

**Code complete, 2026-08-24. One acceptance item is owed and named below.**
Track 0 (BALMORA). **This sub-phase adds no feature.** It exists because §12.4
requires a toolchain bump to be taken alone, and because §6.9.1 found wgpu 30.0.0
shipped 2026-07-01 while Somnium sat on 29.

wgpu **29.0.3 → 30.0.1**, plus the vendored `third_party/wgpu-ffx` and
`wgpu-ffx-shaders-spv`, which pinned 29 independently.

## The plan predicted two breaking changes. Neither applied. Six others did.

§8's item 1 named "the two known breaking changes, both mechanical and both
wide": `push_constant` becoming `immediate` across 48 shader files, and
`subgroup_min_size` / `subgroup_max_size` moving from `Limits` to `AdapterInfo`.

**Both are real in wgpu 30 and neither touched Somnium**, for the same reason in
both cases — the tree never used the feature:

```
grep -rn "push_constant" --include=*.wgsl crates   ->  0
grep -rn "subgroup"      --include=*.rs   crates   ->  0
```

The third prediction — `EXPERIMENTAL_RAY_QUERY` absorbing the separate
acceleration-structure feature — was already true in wgpu 29, and
`context.rs:52` says so in a comment written during Phase 24J.

What *did* break was a different set, and every one of them is mechanical:

| # | Change | Sites | Fix |
|---|---|---:|---|
| 1 | `RenderPipelineDescriptor.vertex.buffers` is now `&[Option<VertexBufferLayout>]` | 5 | Wrap the literal in `Some(..)`. `gizmo.rs`, `light_gizmo.rs`, `water.rs` ×2, `somnium_ui/pass.rs`. |
| 2 | `SurfaceTexture::present()` moved to `Queue::present(surface_texture)` | 1 | `renderer.rs:4033`. Presentation is now ordered against submitted work explicitly rather than by the texture's lifetime. |
| 3 | `Buffer::get_mapped_range{,_mut}` returns `Result<_, MapRangeError>` | 9 | `.expect(..)` naming the precondition each site already established — either `map_async` + a blocking poll, or `mapped_at_creation: true`. An `Err` here is a programming error, so `expect` is the honest handling and the message says *which* precondition. |
| 4 | `RequestAdapterOptions` gained `apply_limit_buckets` | 1 | `false`. Bucketing rounds reported limits so untrusted content cannot fingerprint the GPU; Somnium is a native editor and wants the real numbers — the bindless pool and the terrain pack sizing both read them. |
| 5 | `SurfaceConfiguration` gained `color_space` | 1 | `SurfaceColorSpace::Auto`, which is supported for every format the surface reports and reproduces 29's behaviour exactly. **Choosing an HDR space is a rendering decision with its own evidence, and A2 adds no feature.** |
| 6 | `CreateShaderModuleDescriptorPassthrough.num_workgroups` became a per-entry-point `entry_points: Cow<[PassthroughShaderEntryPoint]>` | 1 | `third_party/wgpu-ffx/src/pass.rs`. FSR's SPIR-V has one entry point, `main`; `workgroup_size` is Metal-only and this path is SPIR-V, so it keeps the zeroes 29 passed. |

**The lesson, which is worth more than the bump:** the plan's list of breaking
changes came from a changelog read at a distance, and it was wrong in both
directions — it named two things that did not apply and missed six that did. A
version bump is a compile, not a reading exercise. §6.9.1's own caveat —
*"the changelog's 'fully supported on Vulkan' is a claim, not a measurement"* —
generalises further than it was written to.

## Probe, do not trust

§8 item 2 asks for a capability report on the actual target hardware.
`crates/somnium_renderer/src/capability.rs` is it: thirteen probes, each
carrying **the sub-phase that is waiting on the bit**, plus the subgroup sizes
(now on `AdapterInfo`) and the seven limits that bound a MORROWIND design.

It logs one summary line at startup and writes the full table when
`SOMNIUM_CAPABILITY_REPORT` names a path:

```bash
SOMNIUM_CAPABILITY_REPORT="dev records/phase MORROWIND/MORROWIND-A2_capabilities.md" \
  cargo run -p hello_engine
```

Three tests keep the report honest, and they are about the report rather than
about the GPU — a capability probe is one of the few things whose *content* can
be tested without hardware:

- `every_probe_names_who_wants_it` — the failure mode is a report that grows into a dump of every feature wgpu has. A capability nobody is waiting for is noise, and noise is what makes a report stop being read.
- `probes_are_distinct` — no bit probed twice under two names.
- `shipped_requirements_are_probed` — `TEXTURE_BINDING_ARRAY`, `EXPERIMENTAL_RAY_QUERY` and `TEXTURE_COMPRESSION_BC` are in the table. A report listing only what MORROWIND *wants* would let a regression in a shipped requirement pass unnoticed on new hardware.

Three named consumers read the result rather than re-deriving it:
**MORROWIND-U** needs `ACCELERATION_STRUCTURE_BINDING_ARRAY` before choosing
between skin-to-buffer and skin-in-shader (A.5 risk 1);
**MORROWIND-Z/AA/AD** want `EXPERIMENTAL_MESH_SHADER`, the native form of what
`meshlet.rs` plus `cull.wgsl` emulate today; anything wave-level wants the
subgroup sizes.

## What §8 item 4 asked, and the answer

> *"Record what 30 unlocks, and build none of it here."*

Recorded, none built. `capability.rs`'s `wanted_by` column **is** that record,
and it lives beside the probe rather than in a document, so a sub-phase that
picks up mesh shaders finds the note at the point of use. Mesh shaders in
particular are a Track 7 investigation with its own measurement, not something
to start while a version bump is still settling.

## The owed item, stated plainly

**§8 item 3 — `.somtime` parity on both shipped maps — is not done.** It needs a
windowed GPU run and this session had none. It is the whole acceptance test for
this sub-phase, so **A2 is code-complete and not closed**:

```bash
# Both shipped maps, before and after. A version bump that changes the frame is
# a regression until explained.
cargo run -p hello_engine --release   # with the .somtime harness enabled
```

The same run produces the capability report and, if taken at a fixed frame,
GHOSTFENCE's first golden reference. **One windowed session closes three owed
items at once**, which is the argument for doing it before MORROWIND-B rather
than after.

This is the same debt Phase CONTROL carries for its Track 2 and Track 3
evidence, and it is recorded the same way: named, with the command, rather than
quietly skipped.

## The test suite on wgpu 30

`cargo test --workspace -j 1`: **1,231 passed, 0 failed, across 40 suites**,
including the three new `capability` tests. Nothing regressed.

(`-j 1` is not superstition here. This workspace lives on OneDrive and a
parallel link step reliably trips `LNK1104` on a file the sync client is
holding; the first attempt at this run died exactly that way, on
`somnium_ecs`'s `archetype_migration` test binary, with nothing wrong in the
code. GHOSTFENCE's `tests` row uses `-j 1` for the same reason, and says so.)

## GHOSTFENCE

`toolchain` was **red before this sub-phase and is green after it**, without the
gate being edited:

```
  PASS  census     MORROWIND-A_census.md matches the tree
  PASS  toolchain  rustc 1.88, wgpu 30.0, winit 0.30
```

MORROWIND-A wrote `FROZEN_TOOLCHAIN` in `tools/ghostfence/run.py` as the
*destination* rather than the current state, so A2's job was to make reality
agree with a line that already existed. `no-second-system` stays red for B and C.

## Files

```
+ crates/somnium_renderer/src/capability.rs      (the probe, 13 capabilities, 3 tests)
~ Cargo.toml                                     wgpu 29.0 -> 30.0
~ Cargo.lock                                     wgpu 29.0.3 -> 30.0.1, naga 30.0.1
~ third_party/wgpu-ffx/Cargo.toml                29 -> 30 (dep + dev-dep)
~ third_party/wgpu-ffx/src/pass.rs               num_workgroups -> entry_points
~ crates/somnium_renderer/src/lib.rs             pub mod capability
~ crates/somnium_renderer/src/context.rs         apply_limit_buckets, color_space, probe call
~ crates/somnium_renderer/src/renderer.rs        Queue::present; one mapped-range
~ crates/somnium_renderer/src/capture.rs         two mapped-ranges
~ crates/somnium_renderer/src/profiler.rs        one mapped-range
~ crates/somnium_renderer/src/pass/{census,gizmo,postprocess,light_gizmo,water}.rs
~ crates/somnium_ui/src/pass.rs                  vertex buffer layout -> Some
~ context.md, project_somnium.md, third_party/*/README.md   frozen line
~ dev records/phase_MORROWIND.md                 preamble frozen line
```

Historical records (`dev records/phase 26/`, `phase XV/`, the handoffs, and
`ATTRIBUTION.md`'s Phase 12/15/24 entries) still say wgpu 29 and **are left
alone** — they describe what was true when they were written, which is what a
record is for.
