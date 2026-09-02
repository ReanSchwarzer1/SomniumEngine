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

---

# Artifact report and fixes — 2026-09-02, same day

Reported from a live editor session: blotches, triangular facets, a regular
scale-like ripple, a washed grey cast, and possible jitter. Bisected with the
per-feature rails at `coastal-vista`, default sun.

## 1. Micro-shadowing was the blotches and the facets

Turning micro-shadowing off alone removed them; turning the horizon map, sky
visibility or the relief normal off changed nothing. So it was F1, and the
cause was the input, not the term.

`micro_shadow` is a **hard cutoff** — `saturate(N.L + 2*ao^2 - 1)`. It was being
fed `surface.occlusion`, which by then carried **GTAO**. Feeding a screen-space
estimate into a hard cutoff turns every wobble in that estimate into a visible
edge *in direct sunlight*; feeding it an interpolated vertex normal makes the
cutoff trace the mesh triangulation. Between them that is exactly the reported
pair: blotches following GTAO, triangular facets following the mesh, both worst
on open sunlit hillsides where there is no micro-relief to justify either.

It now reads the **material's own AO map and nothing else** — the terrain layer
AO, or a mesh's occlusion map. That is what the term was designed against: a
texture-scale record of relief below the pixel footprint. GTAO belongs to the
ambient term, where it already is.

This is the *second* correction to this input. The first took sky visibility
out. The through-line is the same both times: a hard cutoff is only as
well-behaved as the field it thresholds.

## 2. The grey wash was D over-delivering, and the fog default was the cause

Measured at `coastal-vista`, default sun: fog off 2515.70, fog on **6521.35**.
The fog was contributing 60% of terrain radiance — a glow, not a distance cue.

The D term itself is right. The density was not: 8e-4 /m was chosen against a
fog medium lit *only by the sun*, which was much too dark for its density.
Lighting it by the sky as well exposed how much fog that number actually asks
for. It was always too high; the sun-only lighting was hiding it.

Retuned to 2e-4 in both `PostProcessSettings::default` and
`FogSettings::default`, kept in step deliberately: two defaults that disagree is
how a loaded scene ends up looking different from a fresh one. Terrain radiance
is now 3741.63 against 2515.70 fog-off — a third of the frame, which reads as
distance.

This was listed as "flagged, not fixed" an hour earlier. It is now fixed
because a measurement said it had to be, which is the right reason.

## 3. The ripple was a real bug in the horizon bake, in two parts

Both found by writing a test that a perfectly uniform slope's uphill horizon
angle must equal its slope angle, everywhere.

**Part one: two samplings of one field.** Level 0 of the occluder pyramid was a
*max* over the heightfield cells a texel covers, while the ground was a
*bilinear* centre. On any slope the max sits above the centre, so every texel
saw its neighbours as taller than they are. Both are now the same bilinear
field; the max survives only in the pyramid's *reduction*, from mip 1 outward,
which is where it was actually needed — that is what stops a thin ridge
averaging away and ceasing to cast.

**Part two, and the larger one: the cell's max was credited with the wrong
distance.** A mip-`m` cell spans `2^m` texels and stores the tallest point
anywhere in it, which may be most of a cell further away than the march's
current position. Dividing that height by the near position's distance
overstates the angle, by an amount that cycles with `s mod 2^m` — a ripple with
a period of exactly the cell size, which is what a scale pattern is.

Fixed by attributing the height to the cell's **far edge**. Two reasons that is
the right edge: it is *exact* wherever the ground is locally linear (a monotonic
slope puts its cell maximum at the far edge, so height over distance is just the
slope, at every mip), and where it is wrong it **under**-shadows. A missing
sliver of shadow at three hundred metres is invisible; a ripple of false shadow
is what got reported. The cell centre was tried first and still left a spread.

Measured on a uniform 26.6 degree slope, uphill horizon (true answer 75/255):

| attribution | reading |
|---|---|
| as shipped | 75..90 |
| cell centre | 75..79 |
| **cell far edge** | **75..75** |

Unit-stride marching also went from 8 texels to 16. The near field is where a
wrong horizon angle is a *visible* wrong shadow, and the whole march is still
around fifty samples.

### The test was wrong before it was right

`a_smooth_slope_does_not_ripple` first read azimuth 4 — **downhill** — got 0
everywhere, and passed its spread check trivially while measuring nothing. It
now reads azimuth 0 and asserts the value against the true slope angle as well
as the spread, so it cannot pass by looking at flat data again.

## 4. Not reproduced

**Jitter.** Not seen in any still capture, and the reported session was flying
at 175 m/s where motion blur and TAA are both in play. Candidates if it
persists: specular AA reading screen-space derivatives that move with TAA's
sub-pixel jitter, and the relief normal crossing a mip boundary. Both are
testable with a held camera and a frame-to-frame diff; neither has been.

## Test count

450 renderer lib (9 in `horizon`, one new), 21 shader validation, 11
hello_engine, 385 across `somnium_core`. One core test was constructing
`TerrainTextureIds` field by field and broke on the new fields; it now uses
`..Default::default()`, because a test about which ids get unbound has no
opinion about the rest.

---

# The white splotches are albedo, not lighting

Reported after the fixes above, with the bright patches circled on a mid-field
hillside. **They are not a TSUSHIMA regression.** Three tests, in order of how
decisive they are:

1. **Bisect.** Present with `SOMNIUM_TERRAIN_BRDF_DIFFUSE=0`, with
   `SOMNIUM_TERRAIN_BRDF_MS=0`, and with `SOMNIUM_TERRAIN_RELIEF=0`.
2. **All off.** Present with `SOMNIUM_TERRAIN_BRDF=0 SOMNIUM_TERRAIN_RELIEF=0
   SOMNIUM_TERRAIN_HORIZON=0 SOMNIUM_TERRAIN_SKYVIS=0` — every feature this
   phase added, disabled together.
3. **Debug mode 9, raw albedo.** The patches are *in the albedo*, at full
   strength, with no lighting applied at all.

So this is content: a near-white layer being placed on ridges and slope breaks.
The 32-layer table has four candidates — `Snow` (3), `Limestone` (21),
`Light Dune` (27) and `Hard Snow` (31) — and debug mode 19 confirms the pale
regions are their own splat selection rather than a blend artifact.

That makes it **TSUSHIMA-H's** problem (colour, calibrated and varied) and the
pack audit's, not F's. It is worth saying plainly because the natural reading
of "new artifacts appeared during a lighting phase" is that the lighting phase
caused them, and here it did not — better lighting simply stopped hiding them.
The pre-phase captures have the same patches; they were flatter and greyer and
read as part of the wash.

## What was not reproduced

The **dense field of small bright glints** in the second report image. Not seen
at `coastal-vista` or `coastal-ground`, at sun elevations 8, 15, 25, 35 or the
default. Two things about that scene are not in any capture here: it is much
closer to the ground, and its ground looks wet. Terrain wetness defaults to 0
(`SOMNIUM_TERRAIN_WETNESS`), and the wetness path lowers roughness
(`wetness_gloss` 0.55) and raises F0 — which is exactly the configuration that
turns a rough surface into one that can throw specular fireflies. That is a
hypothesis, not a finding; it needs the camera and the scene state that
produced it.

---

# The specular fireflies — found and fixed

Reported on terrain and then, decisively, **on painted foliage as well**. That
second screenshot is what cracked it: nothing terrain-specific — not the splat,
not the layer albedo, not the relief normal, not the horizon map — can explain
glints on grass blades.

## What they actually were, measured

HDR capture (`.somcap`, linear), `coastal-ground`, default sun and **default
wetness**, over 1,520,748 terrain pixels:

| | before | after |
|---|---:|---:|
| mean | 3057.8 | 3056.7 |
| p99 | 6840.5 | 6840.5 |
| p99.9 | 8416.4 | 8415.9 |
| **max** | **60000.0** | **20689.0** |
| px over 30,000 | 38 | **0** |
| px over 50,000 | 38 | **0** |
| isolated peaks (>4x their 8 neighbours) | 25 | **4** |

`60000.0` is not a coincidence: it is the shading pass's own output clamp
(`shading.wgsl`, `min(result, vec3(60000.0))`). Individual pixels were being
driven past the HDR ceiling and pinned there, while their neighbours a fraction
of a degree away returned a few thousand. Their RGB read `(60000, 60000, 29056)`
— warm and channel-clamped, i.e. **sunlight**, not sky.

## Two fixes, and only one of them was the firefly

### 1. Specular antialiasing was inert on terrain — my bug

TSUSHIMA-E placed the filter **above** the terrain branch. Terrain then
overwrote `surface.normal` and `surface.roughness` outright, the relief normal
overwrote them again, and decals again after that. Every terrain pixel computed
the filter and discarded it.

The comment at the call site said it ran "after the terrain branch and after
every other write to `surface.normal`". That was the intent. The code did the
opposite, and the record in this file repeated the claim.

It now runs after terrain, relief, wetness and decals, immediately before `f0`
is derived. `specular_aa_runs_after_every_normal_and_roughness_writer` is a
source-order test that fails if any future writer lands below it — a source
test rather than an image test because a discarded roughness widening looks
exactly like a surface that never needed one, right up until it aliases in
motion.

**Credit where it is due: Codex found this one.** It is a real defect and it was
mine.

### 2. The lobe itself needed a bound — this is what removed the glints

Specular AA is the principled fix and it cannot be the whole one. It widens the
lobe by the **normal** variance it can see. It cannot see a lobe that is narrow
because the surface is genuinely smooth, and it cannot see sub-pixel geometry at
all — which is exactly why grass blades sparkle where terrain does not.

`clamp_specular_lobe` bounds one light's specular response to **a quarter of the
light arriving**. That is an energy statement, not a tuned number: a dielectric's
normal-incidence reflectance is around 4%, rising toward 1 only at grazing where
the lobe is widest and least prone to this. A quarter is several times more
headroom than any real highlight needs — which the measurements confirm, since
the mean moved 0.04% and p99 did not move at all.

Applied before the sun's illuminance is multiplied in, so the ceiling is a
fraction of the light actually arriving rather than an absolute number that
would mean different things at noon and at dusk. Luminance-scaled, not clamped
per channel, because clamping channels independently turns a white firefly into
a coloured one.

## What was ruled out along the way

- **The albedo.** Foliage diffuse textures audited: max 0.553 linear, nothing
  above 0.6. Clean. The earlier terrain-albedo work (a per-layer p98 knee and a
  shader roll-off) measured no change at its default and is **not** in tree; it
  was targeting baked bright texels, and the fireflies were not that.
- **The environment cube.** A clamp on specular IBL radiance relative to the
  sky's own mean changed the capture **byte for byte** — so the spike was not
  arriving through the IBL path. That was worth knowing and cost one build.
- **The roughness floor.** The packed surface maps' roughness channel ranges
  from 0.37 to 0.93; nothing is near the 0.05 floor, so the floor was not
  letting mirrors through.
