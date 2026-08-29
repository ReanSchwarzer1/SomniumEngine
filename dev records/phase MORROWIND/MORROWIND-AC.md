# MORROWIND-AC — transparency and anti-aliasing

> **Status:** in tree, 2026-08-29, against `dev` at `732057b` (PORTAL-0).
> Evidence: `dev records/phase MORROWIND/AC/`.
>
> Track 7 (RED MOUNTAIN). The plan (`phase_MORROWIND.md` §8) listed five items
> for this sub-phase; three were already done and are recorded as such in §1
> rather than rebuilt. What shipped is **OIT**, **SMAA**, and the thing that
> turned out to be blocking both: **one authored anti-aliasing value**.
>
> Hardware for every number: NVIDIA GeForce RTX 5080 Laptop GPU / Vulkan,
> driver 610.74, render 1920×1032, release, 180 warm-up / 300 measured frames.

---

## 1. Residual scope, verified

| Planned item | Verdict | Evidence |
|---|---|---|
| Deferred decals | **Done, out** | `pass/decal.rs`, `shading.wgsl::apply_decals`, binned through `cluster.rs`'s `ClusterVolume`. Shipped by CONTROL-O; §6.7 says AC drops it. |
| Contact shadows | **Done, out** | `shading.wgsl::contact_shadow`; `PostProcessComponent::contact_shadows_enabled`; `docs/editor/lighting.md`. |
| Subsurface scattering | **Done as scoped, out** | `MaterialAsset::transmission` → `GpuMaterial` → `shading.wgsl::transmitted_light`, authored under a `Transmission` group. A separable skin-diffusion profile is a different feature and is **declined**, not deferred: the plan's word was unqualified, and expanding it silently is how a terse line becomes a month. |
| **OIT** | **Shipped** | §3 |
| **SMAA** | **Shipped** | §4 |

---

## 2. The defect that had to be fixed first

`renderer.rs` resolved three authored booleans with a precedence chain:

```rust
let fxaa_active = self.fxaa_enabled && !self.taa_pass.enabled() && !fsr_ok;
```

`fxaa_enabled` defaulted **true** and `fsr_enabled` defaulted **true**, so that
expression was false in the shipped configuration. **FXAA has never run by
default**, while Details showed a checked box saying it did. Three booleans
describe eight states, of which five are reachable and one is a lie.

Adding SMAA to that slot would have added a second checkbox with the same
property, which is precisely what the AC handoff's §5 warned against. So the
state model went first.

`PostProcessComponent::{fxaa_enabled, taa_enabled, fsr_enabled}` are replaced by
one `aa: AntiAliasing` — `Off / FXAA / SMAA 1x / SMAA T2x / TAA / FSR 3` — plus
`smaa_preset: SmaaPreset`. Both are `FieldType::Enum` rows in generated Details,
using the vocabulary CONTROL-K already ships and the selector MORROWIND-Z
already precedents. `set_taa_enabled` and `set_fsr_enabled` are **deleted**: the
mutual exclusions they restored are now consequences of there being one value,
not invariants somebody maintains. `editor_commands.rs` loses the two arms that
re-applied them after a generic patch.

**Measured proof the defect is gone.** Every value produces a pass, and `Off`
produces none:

| `SOMNIUM_AA` | Frame | SMAA | TAA | FSR | Post + present |
|---|---:|---:|---:|---:|---:|
| `off` | 21.351 | — | — | — | 0.103 |
| `fxaa` | 21.698 | — | — | — | **0.162** |
| `smaa` | 21.675 | **0.198** | — | — | 0.255 |
| `smaa_t2x` | 21.762 | **0.191** | **0.271** | — | 0.254 |
| `taa` | 21.419 | — | **0.265** | — | 0.104 |
| `fsr` | 20.979 | — | — | **0.909** | 0.053 |

FXAA's ~0.06 ms shows up in `Post + present` because it has no scope of its own;
that it shows up at all is the point. Before this commit that row read 0.103 in
the default configuration, and the checked box bought nothing.

Two tests pin it: `exactly_one_resolve_is_active_in_every_mode` and
`the_default_does_not_claim_an_anti_aliasing_that_never_runs`.

---

## 3. OIT — weighted-blended

McGuire and Bavoil, *Weighted Blended Order-Independent Transparency* (JCGT
2013). `pass/oit.rs`, `shaders/oit_composite.wgsl`, and a second fragment entry
point `fs_oit` in `shaders/transparent.wgsl`.

**The plan's guess was wrong and is recorded as wrong.** §8 said per-pixel
linked lists were "the likely answer where the required atomics are available".
Against the tree they are not:

- A PPLL writes storage from a fragment shader, needing
  `DownlevelFlags::FRAGMENT_WRITABLE_STORAGE` (`wgpu-types-30.0.1/src/limits.rs`
  line 1106). Somnium queries **no** downlevel flag anywhere, and every atomic in
  the repository is in a `@compute` entry point (`auto_exposure`, `census`,
  `classify`, `spd`). It would be the engine's first portability cliff and would
  need a fallback — which would be this pass.
- Its node pool must be sized from an assumed layer depth. At eight layers that
  is ~199 MB at 1080p and **~796 MB at 4K**, and overflow is a dropped fragment.
  Nobody can justify that budget without content that does not exist.

Weighted-blended needs no feature and no guess: two targets, **10 bytes a
pixel** — 20.7 MB at 1080p, 82.9 MB at 4K — fixed regardless of depth
complexity, and no overflow behaviour to specify because there is no pool.

Both paths call one `shade()` in `transparent.wgsl`, so any difference between
the two images is a difference in *compositing* and not in lighting. Depth
semantics are unchanged: tested against opaque depth, never written. The
composite lands at the existing `Transparent` slot, before Motion Blur and TAA,
so the resolve enters the buffer the temporal passes already reconstruct.

**Default off**, authored on Post Processing. It is a trade, not an upgrade: the
sorted path is exact for separated panes and wrong only where two blended
surfaces of one object intersect; weighted-blended is right there and
approximate everywhere else. `phase_MORROWIND.md` §3 forbids changing what an
existing scene draws, so the sorted path stays the default and keeps
`sort_back_to_front` and its three tests.

**Measured, island** (the shipped map with the most transparency):

| | Frame | Transparent |
|---|---:|---:|
| OIT off | 17.747 | 0.0163 |
| OIT on | 17.802 | **0.0566** |

**Read this honestly: +0.040 ms is the fixed overhead, not the cost of OIT.** It
is a two-target clear plus a fullscreen resolve at 1920×1032. The *variable*
cost — the part that scales with depth complexity — **cannot be measured on
either shipped map**, because PORTAL-0 measured the whole transparent pass at
0.004–0.017 ms on all four viewpoints. There is almost nothing transparent in
Coastal or Island. A fixture with intersecting transparent surfaces is owed
before any claim about OIT's cost or its image quality, and it is owed as
content, not as code.

---

## 4. SMAA

Jimenez, Echevarria, Sousa and Gutierrez (Computer Graphics Forum 31(2), 2012).
`pass/smaa.rs`, `shaders/smaa.wgsl`. Three fullscreen LDR passes in the slot
FXAA occupies, before editor chrome so text and gizmos are never blended.

### 4.1 The variants, and the two that are refused

The plan says "SMAA" with no variant. The ladder is: 1x (spatial), T2x
(temporal), S2x (2× MSAA), 4x (S2x + T2x).

**1x and T2x ship. S2x and 4x are refused, for a structural reason.** Both
resolve MSAA subsamples. Somnium shades from a visibility buffer that stores one
triangle per pixel, and every render target in the frame is created with
`sample_count: 1`. Offering either would be a control that cannot work — the
exact defect §2 exists to remove. They are named in `SmaaPreset`'s doc comment
and in Help so the absence reads as a decision.

T2x is SMAA 1x over a temporally resolved image, reusing 24F's `jitter_ndc` and
24AD's velocity buffer rather than growing a second history.

### 4.2 Quality presets, in Details

`SmaaPreset` is `Low / Medium / High / Ultra` (default Ultra), a second
generated enum row. It trades edge sensitivity against search distance, and the
ladder is measurable rather than nominal:

| Preset | Threshold | Search steps | SMAA ms |
|---|---:|---:|---:|
| Low | 0.15 | 4 | 0.1451 ± 0.036 |
| Medium | 0.10 | 8 | 0.1511 ± 0.034 |
| High | 0.10 | 16 | 0.1560 ± 0.042 |
| Ultra | 0.05 | 32 | 0.1876 ± 0.052 |

Monotonic, and Low→Ultra is +29%.

### 4.3 What this SMAA is not

The three-pass structure, the luma edge test with local contrast adaptation, and
the along-edge search are the paper's. **The reference `AreaTex` and `SearchTex`
lookup tables are not vendored.** `smaa.wgsl::smaa_coverage` solves the same
quantity analytically from the reconstructed silhouette geometry — the offset of
the straight line whose endpoints are ±0.5 where the edge turns and 0 where it
does not, which is what those tables bake.

That substitution is deliberate. A generated data table carries the licence of
the distribution it ships in, and the plan's cited source was
`FlaxEngine-master/.../SMAA.cpp` — which MORROWIND-A's licence audit classifies
as **proprietary**, so it could not be read for anything.

The consequence, stated rather than discovered later: **the diagonal-pattern
pass and the sharp-corner rounding of full SMAA are not implemented.** Both are
separate refinements with their own tables. Near-45° edges go through the
orthogonal path alone and are slightly softer than reference SMAA would leave
them. Four unit tests pin the coverage solve's three shapes — a clean step
blending half along its length, a diagonal ramping through zero, an
unterminated run contributing nothing.

---

## 4.4 The visual gate is not met, and the captures say why

Display-referred captures were taken for every mode. **They cannot resolve
either feature**, and finding that out is the most useful thing they did.

A control — two runs of the same build with the same settings — moves the image
as much as the change does:

| Comparison | px changed | mean delta |
|---|---:|---:|
| coastal-ground, `off` vs `off` (control) | **4.48%** | **0.683** |
| coastal-ground, `off` vs SMAA 1x | 4.43% | 0.591 |
| coastal-ground, `off` vs FXAA | 10.07% | 0.964 |
| island, `off` vs `off` (control) | **63.50%** | **14.955** |
| island, sorted vs OIT | 63.49% | 14.955 |

The SMAA difference is *below* its scene's noise floor. The OIT difference
matches its control to three decimals — 314,515 pixels against 314,536. ReSTIR
GI, the FFT ocean and the clouds are all stochastic or animated, so frame 240 is
not the same frame twice.

**Therefore this record claims that the passes run, and claims nothing about
what they look like.** The timing table in §2 is real evidence — a pass that
takes 0.198 ms executed. The images are not. A visual gate needs a deterministic
fixture with the stochastic passes off, which is owed in §7 alongside the
transparency content, and the two are the same missing artefact.

This is the same trap `capture.rs`'s header records from Phase DOOM: a metric
that "varied from 0.776 to 2.018 across three runs of one identical build".
Controls are cheap and they are the only thing that distinguishes a result from
that.

---

## 5. Interaction AC does not fix

`docs/editor/lighting.md` records that water and transparents ghost under camera
motion because FSR has no reactive mask. **OIT does not fix that**, and neither
does any anti-aliasing choice — it is a temporal reconstruction problem, not a
compositing or an edge problem. Help now says so explicitly in both new
sections, because "we shipped OIT and the glass still smears" is the report this
sentence exists to pre-empt.

---

## 6. Gates

- `cargo test --workspace -j 1`: **1,889 passed, 0 failed** (floor 945;
  PORTAL-0 closed at 1,876).
- `shaders_validate`: 16 passed. `smaa.wgsl`, `transparent.wgsl` and
  `oit_composite.wgsl` are added to the composed-root list — `transparent.wgsl`
  specifically because it now has two fragment entry points sharing one
  `shade()`.
- GHOSTFENCE: census regenerated; `golden-images` still fails on `sculpt-panel`,
  **pre-existing and unrelated** — PORTAL-0 reproduced it on a clean `439b6b6`
  with every change stashed.

## 7. Owed

1. **A deterministic fixture** — intersecting transparent surfaces and thin
   diagonal geometry, with ReSTIR, clouds and wave motion off so two runs agree.
   It is owed twice over: without it, OIT's variable cost is unmeasured (§3) and
   *neither* feature has a valid visual gate (§4.4). One artefact closes both.
2. Matched `.somtime` runs on both shipped maps once that fixture exists.
3. A decision on whether SMAA's diagonal pass is worth implementing, which the
   fixture is also what would answer.
