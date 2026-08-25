# MORROWIND-U — skinned meshes and GPU skinning

**Items 1, 2 and 4 complete; item 3's design decided and argued, not measured.
2026-08-25.** Track 5 (DWEMER), and the first sub-phase of Block 2. §8's four
items, and the honest state of each is below rather than at the end.

`MORROWIND-A`'s census found **zero occurrences of `bone` or `armature`** in
141,221 lines. Somnium could render an open world with ray-traced global
illumination and could not move a character's arm.

## Item 1 — skeleton, skin binding and vertex weights from glTF ✅

New crate **`somnium_anim`**, 1,041 lines. It depends on `glam` **and nothing
else**, which is Seam 7's whole point: the renderer never sees a `Pose`.

```text
  somnium_anim          somnium_renderer
  ────────────          ────────────────
  Skeleton   ─┐
  Pose       ─┼──> [Mat4] ──> SkinningPalettes ──> skinning.wgsl
  (blend tree)┘     ^
                    └── the only thing that crosses
```

The renderer takes a flat array of matrices and does not know whether they came
from a clip, a blend tree, a ragdoll, an IK solver or a test that typed them in.
That is what lets MORROWIND-V add blend trees and MORROWIND-W add IK without the
renderer learning anything.

### The invariant everything rests on

**`parents[i] < i`.** `Pose::to_model_space` is one forward pass with no
recursion, no stack and no visited set — correct *only* because a parent always
precedes its children.

**A glTF file is under no obligation to store them that way.** So
`Skeleton::new` topologically sorts (Kahn's algorithm over a forest, which also
detects the cycle a hand-edited file can contain) and **returns the remap
alongside the skeleton**, because vertex joint indices in the source refer to
the *old* order.

Returning the remap rather than applying it internally is deliberate:
`somnium_anim` does not know what a vertex is, and a caller that forgets to
remap gets a compile error rather than a silently scrambled character. The
importer uses it, and there is a test that a well-ordered skeleton comes back
with an identity remap so the reorder cannot quietly churn.

**Siblings keep their authored order.** A skeleton whose joint order changed
between runs would invalidate every cooked clip that indexes into it, so the
worklist pushes children reversed to cancel the `pop`.

### `SkinBinding`, and the bug in dropping an influence

Four influences, which is what glTF guarantees in one set. A vertex with more is
truncated to its four heaviest **and renormalised** — losing the fifth influence
is invisible, and *not* renormalising is not: the vertex shrinks toward the
origin by exactly the weight that was dropped.

A vertex with *no* influences binds to joint 0 at weight 1, never to four zero
weights, which would put it at the origin and read as a spike through the mesh.
Ties break on the lower joint index so a cooked asset is byte-identical between
runs.

### The importer

`somnium_asset` gained skin import. Two decisions worth stating:

- **`LoadedMesh::skin` is `Option<Skin>`, a parallel array, not a wider
  `Vertex`.** `Vertex` is 32 bytes in `GeometryPool`'s shared buffer, which
  every pass reads and which ray tracing reads positions straight out of.
  Widening it for four joints and four weights would cost 24 bytes on **every**
  vertex in the world — terrain, foliage, props — to serve the handful that are
  skinned.
- **Joint names are index-suffixed, always.** glTF does not require unique node
  names and `Skeleton::find` is by name, so a duplicate would make lookup
  silently pick the first. Suffixing only on collision would be prettier and
  would make the name depend on file order; suffixing never is worse. Stated in
  the code rather than discovered later.

## Item 2 — the palette upload (Seam 7) ✅

`somnium_renderer/src/skinning.rs`. `SkinningPalettes` holds no GPU objects:
the buffers belong to the pass, and this is the part that can be tested without
a device — which for a system whose failure mode is an out-of-bounds palette
read is the part worth testing hardest.

Registration **refuses** rather than trusting:

| Refusal | Why it matters |
|---|---|
| `SkinDoesNotFitSkeleton` | a vertex naming a joint the skeleton lacks reads past the palette on the GPU — a garbage matrix on most drivers, a hang on some |
| `TooManyJoints` | reported with both numbers, so the message is actionable |
| `BindingCountMismatch` | bindings and vertices disagreeing means one of them is from a different mesh |
| `PosedVertexBudget` | the honest cost of the design has a ceiling rather than a hope |
| `InstanceBudget` | ditto |

`set_palette` is **all or nothing**: a half-written palette is a character with
some joints from this frame and some from the last, which reads as a limb
tearing off.

### `posed_bounds` — the piece that makes this correct rather than merely working

The pool's stored AABB for a posed span was computed from the **rest** pose and
is wrong the moment the character moves. `cull.wgsl` would test a box the
geometry has walked out of, and the character would vanish at the edge of the
screen.

The conservative answer, at `O(joints)` rather than `O(vertices)`: take the rest
AABB's eight corners, transform them by **every** palette matrix, union. It
over-estimates — a joint that moves only the left hand expands the box as if it
moved the whole mesh — and over-estimating is the safe direction for a cull
test. A palette containing a NaN reports **no bounds** rather than a NaN box,
because a NaN box passes every cull test *and* every reject.

### The dispatch, and what it wastes

`(widest_instance / 64, instance_count, 1)`, ragged: a thread past its
instance's vertex count returns early. One dispatch per instance would be one
bind and one call per character, and a thousand characters is a thousand of
each.

`dispatch_waste()` reports the cost of that shape, and it is a real finding for
KENSHI rather than a hypothetical: one 6,400-vertex hero plus two 64-vertex
crowd members wastes **more than 60%** of the launched threads. There is a test
asserting both that and that a uniform scene wastes nothing.

## Item 3 — skinning inside a visibility buffer: **decided, not measured**

§8 asks for a measurement on a thousand-character scene before choosing. **That
measurement was not run.** What follows is the argument that was made instead,
stated as an argument so a later session can overturn it with the measurement.

**Chosen: skin-to-buffer.** A compute pass writes posed vertices into a
transient slice of the *same* `GeometryPool`, and every consumer downstream
keeps reading the buffer it always read.

**Skin-in-shader** — applying the palette in the visibility pass's vertex stage
— costs no extra memory and needs three things:

1. conservative meshlet bounds, because `cull.wgsl` would be testing bounds
   computed from an unposed mesh;
2. **a BLAS rebuild anyway**, because ray tracing does not go through the vertex
   stage at all and would trace against the rest pose — a character casting a
   ray-traced shadow of its T-pose;
3. teaching every consumer of the pool that positions may be a function of a
   palette. Measured on the tree:
   `grep -rl "vertices\[" src/shaders` returns **eight modules besides
   `skinning.wgsl`** — `visibility`, `shading`, `shadow`, `transparent`,
   `outline`, `rt_hit`, `restir_gi`, `lighting_extra` — and the count only goes
   up.

(2) and (3) are most of skin-to-buffer's cost without its property that nothing
downstream changes. **Appendix A.5 predicted skin-to-buffer and set the rule
"if the measurement is ambiguous, take the simple one"**; the argument here is
that it is not ambiguous — skin-in-shader is strictly more work *and* leaves ray
tracing wrong until the BLAS rebuild it was supposed to avoid gets written.

**The cost of the choice, so a future comparison has a number to beat:**
`posed_bytes()` — 32 bytes per posed vertex, resident, plus one
read-modify-write per frame. A thousand characters at 8,000 vertices each is
**256 MB**. `SkinBudget` defaults to 2,000,000 posed vertices (**64 MB**, about
250 characters), and KENSHI's crowd phase is where that number gets argued with.

**What is missing, named rather than implied:** no thousand-character scene
exists to measure on, because nothing in the tree animates yet. The measurement
becomes possible after MORROWIND-V, and it is the right place to run it — which
is a reason to run it then, not a reason to have skipped it here.

**The risk floor A.5 asked for — a separate forward pass — was not prototyped
either**, and it is not needed for the chosen design: skin-to-buffer's failure
mode is memory, not correctness, and the budget is the mitigation.

## Item 4 — `SKINNED` is a permutation, and it turned out not to be needed ✅ (with a finding)

MORROWIND-C registered `define::SKINNED_BIT` and left a comment saying
MORROWIND-U would add the `//!if SKINNED` blocks.

**It did not, and that is the right answer.** Skin-to-buffer means the raster
path never learns that skinning exists — a posed vertex in the pool is
indistinguishable from a static one, which is the property the whole design was
chosen for. A `SKINNED` variant of `shading.wgsl` would be a variant that
differs in nothing.

The define stays registered, and this is written down so a reader does not
conclude it was forgotten. The first real user of a permutation will be
MORROWIND-Z or MORROWIND-AC; MORROWIND-C's exit criterion — *"adding a `SKINNED`
define adds a variant without editing `renderer.rs`"* — was met by C itself and
is unaffected.

## The GPU pass

`pass/skin.rs`. One compute dispatch, **before culling**, because a cull pass
reading a half-written posed span would flicker.

Two things it has to get right that are easy to miss:

- **The pool buffer's identity.** `GeometryPool` reallocates when it grows, and
  a bind group holding the old buffer would skin into memory nothing reads — a
  character that simply stops animating, with no error anywhere. wgpu 30 removed
  `Buffer::global_id`, so identity is tracked by size: the pool only grows, so a
  different size is a different buffer.
- **The empty group 0.** `skinning.wgsl` includes `global_pool.wgsl` for the
  `Vertex` declaration alone and binds nothing in group 0, but a pipeline
  layout's groups are positional — group 1 cannot be the first entry.

`skin_vertices` is one array parallel to the **whole pool**, indexed by the same
vertex index the shader uses for the pool, rather than one buffer per mesh with
an indirection.

### The normal, and why it is not the inverse transpose

The shader uses the skinning matrix's own upper 3x3. That is exact for rigid
joints (rotation plus translation) and wrong only in proportion to non-uniform
scale — which riggers avoid precisely because it breaks normals everywhere.
Every shipping engine makes this trade; §A.3.4 says to write it down rather than
leave a reader wondering whether it was an oversight, and the comment is in the
shader.

## Tests: 47 new, 0 failures

- **`somnium_anim`, 22** — the reorder and its remap; a stable order left alone;
  siblings keeping authored order; a cycle, a self-parent, an out-of-range
  parent and mismatched lengths all rejected; a chain accumulating; a rotated
  parent carrying its children; **a rest pose producing an identity palette**
  (with the inverse binds built from the rest rather than left as identity, so
  the test asserts the composition and not a coincidence); a wrong-skeleton pose
  refused *and writing nothing*; a short output buffer refused; blend endpoints
  exact; **rotation blending at a constant rate where a lerp would not**, with
  the quarter-point checked because slerp and lerp agree at the midpoint of a
  90° arc and a test at the midpoint would prove nothing; influences kept
  heaviest-first and renormalised; a fifth influence dropped with the rest still
  summing to one; a vertex with no influences binding to joint 0; zero-weight
  influences not taking a slot; deterministic tie-breaking; a skin naming a
  joint past the skeleton refused.
- **`somnium_renderer::skinning`, 25** — f16 round-trip within 1e-3 and exact on
  0/0.25/0.5/0.75/1; **a NaN weight staying a NaN** rather than becoming a large
  number; joint packing matching the shader's `& 0xffff` / `>> 16`; all five
  refusals; distinct palette regions; a palette writing only its own instance's
  joints; a wrong-length palette writing nothing; posed bounds at rest, following
  a moving joint, covering a rotated joint, and `None` for a NaN palette;
  dispatch shape and the workgroup boundary; **the ragged dispatch reporting what
  it wastes**; and two tests reading `skinning.wgsl` — struct field order and
  workgroup size against the Rust, and that the shader still writes into the
  shared pool, which is the decision asserted where somebody changing it will
  see it.

`somnium_renderer` 361 passed; workspace **1,741 passed, 0 failed**.

## GHOSTFENCE

```
PASS  census            MORROWIND-A_census.md matches the tree
PASS  toolchain         rustc 1.88, wgpu 30.0, winit 0.30
PASS  shader-budget     52 modules, 52 variants possible in total
PASS  one-job-system    no bare spawns; 3 exemptions, each with a reason
PASS  no-second-system  4 singleton symbols, each defined only where it is allowed
PASS  golden-images     3 image(s) within threshold
PASS  tests             1741 passed, 0 failed (floor 945)
```

**No `.somtime` row.** This sub-phase adds a pass that does nothing in any
shipped scene — neither map contains a skinned mesh — so a timing run would
measure the branch that returns early. The row that matters is the one
MORROWIND-V's walk cycle will carry, and it is named here so its absence is a
decision rather than an omission.

## What is left of MORROWIND-U

Named rather than implied:

1. **The frame integration.** `SkinPass::record` is written and tested for
   layout and dispatch shape; nothing in `renderer.rs` calls it yet, because
   nothing produces a palette until MORROWIND-V and calling it with an empty
   `SkinningPalettes` is the no-op it already handles.
2. **The measurement in item 3**, once there is something to measure.
3. **The slice.** `vvardenfell` gets *"one skinned character with a walk cycle"*
   per its own table, and a walk cycle is a clip — MORROWIND-V.

## Files

```
+ crates/somnium_anim/                     Transform, Skeleton, Pose, SkinBinding,
                                           Skin, the parents<child invariant, 22 tests
+ crates/somnium_renderer/src/skinning.rs  SkinningPalettes, SkinVertex, SkinBudget,
                                           posed_bounds, dispatch, f16 packing, 25 tests
+ crates/somnium_renderer/src/shaders/skinning.wgsl   the compute kernel
+ crates/somnium_renderer/src/pass/skin.rs            the dispatch
~ crates/somnium_asset/src/lib.rs          glTF skin import, LoadedMesh::skin,
                                           LoadedScene::skeletons
~ crates/somnium_renderer/src/shaders.rs   register skinning.wgsl
~ crates/somnium_renderer/tests/shaders_validate.rs   it composes and validates
~ Cargo.toml, crates/*/Cargo.toml          the new crate and its edges
```
