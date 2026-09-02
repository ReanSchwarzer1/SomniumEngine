# TSUSHIMA-D, E and F — landed 2026-09-02

Against `ef8e5cb`. All three ship **on**; each has an A/B rail.

Every number below is the capture harness's own `CAPTURE-DIFF`, coastal
`coastal-vista` rail, sun pinned at 8°, frame 240, 1920×1032, over the
1,041,163 terrain pixels. Sky and mesh are byte-identical in every row.

---

## D — KIRI: the plan's premise was wrong, and the real defect was one line

### What §2.3 asked, and what the answer was

The plan asked whether the ground receives aerial perspective when the
volumetric pass is off. Two findings, and the second one matters more:

1. **`max_distance()` returns 0.0 when the pass is disabled**
   (`pass/volumetric.rs:397`), so `volumetric_range.x > 0.0` fails and opaque
   geometry gets nothing. The mechanism is real.
2. **But the pass is on by default and does reach the ground.** Measured on
   `island-ground`: fog off 3924.62, fog on 3822.50, mean abs **102.12**, 53%
   of terrain pixels. So "the ground gets no aerial perspective" was simply
   not true, and the island capture that started this looked wrong for a
   different reason.

### The real defect

```wgsl
// volumetric.wgsl, before
step_scatter += vec3<f32>(fog * fog_phase) * sun_vis * sun_transmittance;
```

The fog medium was lit **by the sun alone**. The air terms one line above have
carried a `multiscatter` skylight term since they were written; the fog never
did.

That would be a small omission if fog were a small scatterer. It is not: at
the default density of 0.0008 /m the fog's optical depth over a few hundred
metres is ~0.24, against Rayleigh's ~0.004 at the same distance — **roughly
fifty to one**. So the entire distance cue on these maps was a sun-lit grey
wash, and distant ground desaturated toward grey and got *darker*, when the
thing everyone recognises as aerial perspective is ground going blue and
*lighter* as it recedes.

The fix is one term, and it is the same term the air already uses:

```wgsl
step_scatter += vec3<f32>(fog * fog_phase) * sun_vis * sun_transmittance
    + vec3<f32>(fog) * multiscatter;
```

A grey scatterer lit by blue skylight scatters blue. The two media now agree
about what is illuminating them.

| | terrain radiance |
|---|---:|
| fog off | 3924.62 |
| fog on, sun-lit (before) | 3822.50 |
| fog on, sky-lit (shipped) | **4870.11** |

**Flagged, not fixed:** this makes the same fog density deliver considerably
more light — 43% brighter terrain on the coastal vista. The 0.0008 default was
chosen when fog was sun-lit-only and therefore too dark, so it is now probably
too high. That is a tuning question for whoever looks at the fog next, and it
is a *better* problem than the one it replaced.

**Also still true, and not addressed:** past 1,200 m the last froxel slice is
held, so a 5 km ridge still gets 1,200 m of air, and 32 slices over 1,200 m is
37.5 m per slice. Neither showed on these maps.

---

## E — WHETSTONE: the normal that survives distance

`terrain/relief.rs` bakes a **mip-chained** terrain-space normal map at 1024².
Every level stores the filtered normal's XZ *and* the length of the
unnormalised mean that produced it.

Mips are generated on the CPU rather than by the hardware because the quantity
that must survive downsampling is the **unnormalised sum**. Four agreeing
normals sum to length 1; four disagreeing ones sum to much less, and that
shortfall is exactly the roughness the coarse level owes the surface. A
hardware mip of an already-normalised normal map throws that channel away, and
with it the only thing that makes the roughness widening possible.

The shader samples it with `textureSampleGrad` at the pixel's own footprint,
cross-fades it in over 50–100 m, and feeds the discarded length through
`widen_roughness_toksvig`.

| | terrain radiance | mean abs | px changed |
|---|---:|---:|---:|
| relief off | 1379.82 | — | — |
| **relief on** | 1410.22 | **68.53** | 524,484 (50.4%) |

The picture is the point rather than the number: gullies and folds appear in
the middle distance that were smooth blobs before. That band — roughly 1–5 m
of relief at 100–500 m — is the one the LOD stride was deleting.

### The double square root is not a typo

`D_GGX` takes **perceptual** roughness `r` and computes `a = r*r`, `a2 = a*a`,
so its `a2` is `r⁴`. The standard `alpha` is therefore `r²` and `alpha²` is
`r⁴`. NDF filtering happens in `alpha²`, so recovering `r` is
`pow(alpha2, 0.25)` — two roots. Getting this wrong is invisible in a still and
obvious in motion, which is the worst combination.

### Specular antialiasing

Tokuyoshi & Kaplanyan (I3D 2019), published constants `σ² = 1/(2π)`,
`κ = 0.18`. Behind `override enable_specular_aa` rather than a `shading_mode`
bit: it is two derivative instructions, and derivatives are the one thing worth
being able to compile out entirely.

**Placement is the whole correctness argument.** `dpdx` on a value produced
inside non-uniform control flow is undefined, and the terrain branch is a
storage read the compiler cannot prove uniform. It is therefore applied after
the terrain branch and after every other write to `surface.normal`, where every
path has already written it.

### One test failed against correct code

`disagreeing_normals_shorten_the_mean` first used a period-**two** sawtooth and
reported a mean length of exactly 1. The code was right: a central difference
reads `h[x+1] - h[x-1]`, those two samples share a parity, so a period-two
signal has a central difference of zero everywhere and produces a perfectly
flat normal field. The test now uses a period-four triangle. Recorded because
the first instinct was to go looking for a bug in the bake.

---

## F — FORGE: the BRDF, and why one switch was not enough

Three terms, three switches. `SOMNIUM_TERRAIN_BRDF=0` still turns all three off
together; `SOMNIUM_TERRAIN_BRDF_MS`, `_DIFFUSE` and `_MICROSHADOW` separate
them.

That separation was not planned — it was forced. Measured through a single
switch, F darkened terrain by 39%, which is the *opposite* of what energy
compensation does, and one number could not say which term was responsible.

Measured apart, against F-off at 2052.32:

| term | terrain radiance | Δ | mean abs | px changed |
|---|---:|---:|---:|---:|
| multiple-scattering compensation | 2084.90 | **+1.6%** | 32.57 | 614,983 |
| Hammon rough diffuse | 1848.57 | −9.9% | 207.83 | 588,443 |
| micro-shadowing | 1423.44 | −30.6% | 628.88 | 589,832 |
| all three | 1379.82 | −32.8% | 679.32 | 697,721 |

- **Multiscatter** behaves exactly as it should: small, brightening,
  roughness-dependent. It is the term the prompt was really asking about and it
  is the smallest of the three.
- **Hammon** darkens ~10% against Burley at high roughness, which is expected:
  Burley's `f90 = 0.5 + 2·VdotH²·roughness` exceeds 1 at high roughness and
  Hammon is the more conservative fit. What it buys is retroreflection.
- **Micro-shadowing** is large because at a grazing sun `N·L` is small over the
  whole landscape and the term is a function of `N·L`. That is the behaviour it
  exists for — it is what makes the relief read at a low sun — and at a high sun
  it will be far smaller. `override micro_shadow_opacity` is the dial; Unity
  HDRP ships the same control at zero and lets artists raise it.

### The AO it was being fed was wrong

Micro-shadowing first measured **−38.7%**. It was reading `surface.occlusion`,
which by that point included TSUSHIMA-C's landscape-scale sky visibility. That
answers a question nobody asked: sky visibility is what fraction of the sky a
valley floor can reach; micro-shadowing is what sub-pixel relief shadows. It
now reads a `micro_occlusion` snapshot taken before C folds in — worth 8
percentage points.

**The first attempt at that fix was silently wrong**, and this is the part worth
remembering. It assigned `micro_occlusion = min(micro_occlusion, surface.occlusion)`
*after* the terrain branch, which looks equivalent to snapshotting and is not:
the `min` re-picks the post-sky-visibility value and cancels the whole fix. The
measurement came back byte-identical — 1257.8627 both times — which is the same
signature as TSUSHIMA-B's flat-heightmap bug and was recognised the same way.
The assignment now happens *before* the branch so the branch can overwrite it.

### Sources

Everything transcribed, nothing recalled. Filament's compensation term (via the
local Bevy copy, which cites it); Hammon GDC 2017 slide 113 with the 1.05
derived on slide 108; Fdez-Agüera *JCGT* 8(1) for the IBL half, whose listing
was read from bruop.github.io/ibl; micro-shadowing from Uncharted 4 (Brinck &
Maximov, GDC 2016) via Unity HDRP's `ComputeMicroShadowing`, which carries the
attribution in its own comment.

`env_brdf_approx` now delegates to `env_brdf_ab` in `brdf.wgsl`. That split is
what made any of the multiscatter work possible: the old function computed the
pair one line before discarding it.

`brdf.wgsl` gates the terms on **private flags**, not on `cluster_params`,
because it is composed into five roots and not all of them bind the cluster
grid. They default to off, so a root that never sets them keeps exactly the
response it had. This is the same mistake `TERRAIN_PI` was added to avoid in
TSUSHIMA-B, caught before it compiled this time.

---

## Where the code went

| File | What |
|---|---|
| `shaders/volumetric.wgsl` | D: one skylight term on the fog medium. |
| `terrain/relief.rs` | **new.** E's bake, mip reduction, upload, rewrite. 4 tests. |
| `terrain/mod.rs` | E's `ReliefGpu`, texture id, two `GpuTerrainMaterial` words + pad, bake at creation and on `horizon_dirty`. |
| `renderer.rs` | E's bindless registration; F's three `shading_mode` bits and `brdf_term_enabled`. |
| `shaders/brdf.wgsl` | F: `env_brdf_ab`, `energy_compensation`, `micro_shadow`, `diffuse_hammon`, `l_dot_v`, three private flags. |
| `shaders/terrain_material.wgsl` | E: struct fields, `terrain_relief_normal`, `widen_roughness_toksvig`. |
| `shaders/shading.wgsl` | E: relief blend + specular AA. F: flag set, `evaluate_ibl_ms`, `micro_occlusion`, micro-shadow. |
| `material/pool.rs`, `tests/shaders_validate.rs` | Layout 2048 → 2064. |

449 renderer lib tests (4 new), 21 shader validation, 11 hello_engine. All pass.

---

## Outstanding

- **Fog density wants retuning** after D. Flagged above.
- **The 1,200 m froxel range and 32 slices** are untouched.
- **`micro_shadow_opacity` is untuned** at 1.0. It is the largest single lever
  in F and nobody has looked at it against a high sun.
- **`relief_takeover` is 100 m** to match B's cross-fade. Not tuned, just
  matched.
- **Per-layer F0 (F4) and Hammon's cheaper `G2` (F5) are not done.** F4 is a
  struct change plus authoring; F5 is a drop-in worth measuring on its own.
- **Island still barely moves** on B/C, and D/E/F have not been A/B'd on it
  separately — only sanity-checked (terrain radiance 1079.25, no artifacts).
