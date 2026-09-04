# TSUSHIMA-G, the rest of H, and I — landed 2026-09-03

Against `d3f8f44`. Everything here ships **on**; each feature has an A/B rail.

Three sub-phases in one pass, because they are the same file asking the same
question — what the ground is made of at the scales between one texel and one
map — and because two of them turned out to want the same march.

---

## G — WEAVE: the boundary the brush cannot paint

### What was wrong

`sel_w` came straight out of the splat texture. A 4 m brush paints a 4 m-smooth
transition, and height blending only ever re-ranks layers *within* the band that
smooth weight already defines, so the widest tool in the editor set the smallest
feature size on the ground. Every material boundary on both maps was an oval.

### Where the perturbation went, and why that is the whole design

Inside `terrain_unpack_splats`, and **before** `terrain_strongest_four`.

Before selection is the point. A perturbation applied afterwards can only wobble
an edge the four winners have already drawn. Applied before, it changes *which*
four win — and that is the difference between an interlocked edge and a wobbly
oval.

Inside the unpack is the other half. There are **three** call sites, not two:
the live path, the clipmap generate path, and `rt_hit.wgsl`'s bounce-ray albedo.
The third was found by grepping after the first two were already edited, and it
is exactly the one a later session would have missed — a traced reflection that
disagreed with the surface it reflects about where gravel stops is a bug nobody
would think to look for in a splat function.

Putting it in the unpack also puts it where the invariant lives. That function's
job is "the weights, normalised to sum to one", and the perturbation has to
preserve the second half of that sentence. It renormalises: the `w(1-w)`
envelope is symmetric but the noise sample is not, so a naive perturbation
leaves `discarded = 1 - kept` drifting by roughly a seventh, and `discarded` is
a debug channel whose entire job is reporting what selection threw away.

### The envelope, and the per-layer offset

```wgsl
w + terrain_weight_noise(local_xz, i, scale) * amount * w * (1.0 - w) * 4.0
```

`4*w*(1-w)` is zero at both ends and one at w = 0.5. A fully-painted area and a
bare area both stay exactly put, and only the transition band moves. Without it,
noise punches holes in the middle of solid ground and an author who painted a
road gets gravel in it.

The per-layer offset is what makes this break a boundary rather than move one.
Perturb every layer with the same field and the weights rise and fall together,
the ranking between them never changes, and the whole edge translates.

### World position, never UV

A noise field indexed by anything derived from the camera crawls. The mirror of
that mistake — indexing a *dither* by UV rather than by screen position — is
written up at length in this same file's stochastic-filtering comment. Here the
world is the stable frame, because the boundary is a property of the ground.

### The noise primitive

`terrain_value_noise`, on the integer lattice, smoothstep-interpolated, over an
integer bit-mix hash rather than `fract(sin(dot(p, k)) * n)`. Two reasons the
one-liner is wrong here and not merely unfashionable:

- Its period collapses once the argument is large, and world coordinates on
  these maps reach a couple of thousand metres.
- Its failure mode is diagonal banding at low frequency, which is exactly the
  band both G and H exist to supply.

Smoothstep and not a straight lerp, for a related reason: linear interpolation
of a value lattice has a discontinuous derivative at every cell edge, and a
discontinuous derivative in a field that perturbs a *boundary* is a straight
line in the boundary. A grid, in the feature added to remove grids.

`bitcast<u32>` and not `u32(...)` on the lattice index — a value conversion of a
negative i32 is not defined to wrap, and `local_xz` is negative for anything off
the terrain's positive quadrant.

### Cost

The perturbation loop skips weights that are exactly 0 or 1. Splat weights are
sparse — two or three layers meet at any texel — so this is two or three
two-octave evaluations a pixel, not thirty-two, and the branch is coherent
across a warp because neighbouring texels agree about which layers are painted
there.

### Flagged, not fixed

The parallax march reads these weights one step later to pick its dominant
layer. A strength large enough to change which layer dominates will make POM's
chosen height map flicker at a boundary. It has not been looked at with a
near-ground camera and parallax on.

---

## H — INK: the octaves, and one of them driven by something real

### The octaves

`terrain_macro_octaves`, at roughly 1 km, 100 m and 10 m, **multiplied** rather
than lerped so they compose — a kilometre-wide band and a ten-metre mottle
should both be visible in the same square metre, and a lerp lets the finer one
erase the coarser wherever it is strong. Centred on 1.0, so strength 0 is the
exact identity, which is the property the macro map's 0.5 fallback already has.

Applied between `terrain_macro_blend` and the squaring — the same
approximately-perceptual space the existing blend works in. Below the squaring
it would be a second, independent gain on linear radiance fighting the blend
rather than composing with it, and the overlay and linear-light modes are
defined against a perceptual operand.

In **both** paths, live and clipmap-generate. A terrain shades through whichever
of the two the distance picks, and a clipmap ring that disagreed with live
terrain about the tint would draw a visible ring on the ground at the handover.

Frequencies are constants and only the strengths are uniforms. The three bands
are the design — they are the bands the material was missing — and the strengths
are what an author actually reaches for.

### The sky-visibility drive, and where the plan was improved on

Section 3 H item 2 asks for one octave driven by something meaningful, and names
C's sky visibility. That is the difference between noise and a landscape:
sheltered ground holds water and the organic matter that comes with it and reads
damp and green; open ground drains and bleaches.

Sky visibility rather than slope, because slope cannot tell a valley floor from
a plain and sky visibility can — the two have the same normal and very different
drainage.

**It is not in `terrain_material.wgsl`.** The plan put it there, in perceptual
space beside the octaves. It is in `shading.wgsl`, at the one call site where
`sky_vis` is already in hand, for the reason that file already gives about B and
C: the quantity is a property of the heightfield, not of the material, and
putting it in the material would mean a second fetch of the same texture in the
live path plus a third and fourth copy of the rule in the clipmap and
virtual-texture paths. The cost of the move is that the tint multiplies linear
albedo rather than the perceptual value, which is a reparametrisation of the
strength and nothing else.

The remap is not `1 - sky_vis`, and its lower edge is not the map's minimum
either. `smoothstep(1.0, 0.88, sky_vis)`: the upper edge is exactly 1.0 so
genuinely open ground is the exact identity, and the lower edge sits just below
TSUSHIMA-C's measured mean of 0.93, which is where the pixels that differ from
open ground actually are. The first cut read C's *range* instead of its
distribution and was effectively invisible; see the measurements below.

The tint's luminance is about 0.94, so it is mostly a hue shift and only
slightly a darkening. C already darkens sheltered ground through `occlusion`,
and that is a transport term where this one is what the ground is made of. Both
are true; stacking two full-strength darkenings on the same pixels would read as
a painted-on shadow.

### What is still open in H

- **Item 3, re-injecting contrast with distance**, is untouched.
  `terrain_detail_fade` still solves aliasing by removing signal. The plan's
  argument is that E's filtered normal and F2's roughness contrast make much of
  that fade recoverable, and that is a measurement, not an edit.
- **Item 4, de-lighting the packs.** The pack audit already concluded the albedo
  is mostly in range and the white splotches are content. De-lighting is an
  asset-pipeline job, not a renderer change.

---

## I — GRAVEL: the contact scale

### I.2 — the cliffs get parallax back

`evaluate_terrain_material` disables POM wherever `cliff_blend >= 0.05`, and the
stated reason was correct: that march is UV-space, walking the terrain's
world-XZ parametrisation, and on a vertical face that parametrisation is
degenerate. Godot makes the same exclusion.

The consequence was that the steepest ground in the scene — the ground a player
most often stands right next to — was the one surface with no depth at all.

The fix is not to make the XZ march work on a cliff. It is to march in the frame
the projection already samples in. Each plane's texture coordinate is two world
axes scaled by the tiling, so a displacement in it has an exact physical meaning
and the ray resolves into it with two dot products and no tangent frame at all:

| plane | coordinate | ray, resolved |
|---|---|---|
| X | `p.zy` | `(v.z, v.y, v.x * sign(n.x))` |
| Y | `p.xz` | `(v.x, v.z, v.y * sign(n.y))` |
| Z | `p.xy` | `(v.x, v.y, v.z * sign(n.z))` |

**One march, two frames.** `terrain_parallax_march` was extracted from
`terrain_parallax_offset` and now serves both. It takes a coordinate rather than
a metre, and the heightfield caller converts by multiplying its start point and
its step by the tiling and dividing the result back — exact, because scaling a
linear march and its linear refinement by one non-zero scalar commutes.

That refactor is the reason to do this at all rather than paste a second march:
the single-lookup POM refinement is the subtle half and the half nobody
re-reads, and two copies is two chances for one of them to keep a bug the other
lost. The same move let `terrain_parallax_shadow` stop passing `tiling` down to
every height fetch.

Step count and depth are computed **inside** `terrain_projected_pbr`, from
`tm.parallax_steps` and `terrain_detail_fade`, rather than passed in. There are
two callers — live terrain and the clipmap's shading pass — and a rule carried
at both would eventually stop matching at one. Planes contributing under 5% are
not marched; a full march to move a 3% contribution is a march for nothing.

The heightfield POM stays excluded on cliffs. Two parallax solutions applied to
one pixel would displace it twice.

**Not done:** the projected path has no parallax *self-shadow*. The heightfield
one does, and it is what makes relief read as lit rather than merely displaced.

### I.1 — the funnel had no idea what the ground was made of

The plan says to scatter debris "through the existing foliage rejection funnel,
which already does slope, layer-weight, radius and distance culling."

It does not do layer-weight. `paint` tests slope, radius and spacing and nothing
else. `TerrainData::surface_sample` has computed a layer weight all along and
`ground_sample` was **throwing it away** — sampling layer 0 and returning only
height and slope.

That is the whole question for debris. Slope keeps grass off a cliff and radius
keeps it under the cursor, but nothing asked what the ground was; a scatter that
ignores the splat puts pebbles in the middle of a painted lawn.

`GroundSample` now carries the weight, `FoliageBrush` carries `layer` and
`min_layer_weight`, and the test is a **hard threshold, not a probability**. A
probability scatters a thinning fringe of pebbles across the neighbouring
material, and the thing that makes a gravel patch read as gravel is that it
stops. Zero disables the test entirely, so every pre-TSUSHIMA brush behaves
exactly as it did.

**Tilt.** `PaintedFoliage` gains one angle. A field of pebbles all sitting
perfectly flat reads as placed rather than fallen. One float and not two,
because `Ry(yaw) * Rx(tilt)` with a uniform yaw already leans in a uniformly
random horizontal direction — the yaw that stops a mesh reading as a grid of
clones is the same yaw that picks which way the lean points. It is
`sqrt`-distributed for the same reason the candidate radius is: a uniform angle
puts as many instances near the limit as near flat, and a real pile is mostly
settled with a few propped up. Zero for anything that grew, because grass and
trees are upright because they grew toward the light and a pebble has no such
excuse.

### The palette stopped being a pair of strings

`FOLIAGE_PALETTE` is now `[FoliageEntry; 6]`, carrying the brush's defaults per
entry. Phase 17F expressed "trees are placed one at a time" as
`single = kind >= 2` at the selection site — true of a four-entry palette, and
false the moment a fifth entry was added. The hints belong to the *thing*: a
tree is placed singly because it is a tree, and a pebble leans because it is a
pebble.

Both places that change `kind` — the selection event and the numeric brush field
— now apply the entry's defaults. Two ways to set one field that behave
differently is how a slider ends up painting pebbles with a tree's brush.

Debris goes on **Gravel (layer 7)** and **Talus (layer 15)**.

### The meshes

`tools/fetch_foliage.sh` downloads all twenty-five palette entries through the
Poly Haven file API, MD5-verified and fail-closed, and they are committed. About
264 MB at 2k, against the 104 MB the four Phase 17E entries cost.

The script follows the **file API** rather than building URLs.
`fetch_terrain_textures.sh` can build its own because textures are a flat
namespace; models are not — the textures do not sit beside the glTF and the
`.bin` is shared from the 8k variant whatever resolution is asked for. Guessing
the layout gets a glTF that loads and renders untextured.

**It destroyed data once, and the fix is the interesting part.** The first cut
had Python emit one tab-separated line per file and read it with `IFS`. Python
emits CRLF on Windows, `read` leaves the CR on the last field, and the last
field is the MD5 — so every hash compared unequal, every already-correct file
was re-downloaded, and every fresh download was then deleted as corrupt. It took
the four committed Phase 17E assets with it before anyone read the log. They
came back with `git checkout`, which is the only reason this is a paragraph and
not a disaster.

Two changes came out of it, and neither is a `tr` in the pipeline — any fix
spelled with a backslash escape is one careless edit away from breaking the same
way and just as quietly:

- `tools/polyhaven_files.py` writes **three plain lines per record** — path,
  URL, MD5 — in binary. There is no delimiter to get wrong and no newline
  translation to happen.
- The download goes to `<file>.part` and is moved into place **only after** its
  hash agrees. Writing straight to the target means a bad download has already
  destroyed what was there before anything checks it.

A third came out of it too: `.gitattributes` now marks `assets/**/*.gltf` as
`-text`. glTF is JSON, so git was rewriting its line endings on checkout, which
made the bytes on disk differ from the bytes the publisher served and the MD5
check fail forever on exactly the file it most wants to verify.

---

## Where the code went

| File | What |
|---|---|
| `shaders/terrain_material.wgsl` | G: `terrain_noise_hash`, `terrain_value_noise`, `terrain_weight_noise`, `terrain_perturb_weights`, new unpack signature. H: `terrain_macro_octaves` in both paths. I: `terrain_parallax_march` extracted, `terrain_projected_offset`, per-plane POM in `terrain_projected_pbr`. Four new struct fields. |
| `shaders/shading.wgsl` | H2: the sky-visibility tint, at C's call site. |
| `shaders/rt_hit.wgsl` | G: the bounce-ray albedo goes through the same unpack. |
| `terrain/mod.rs` | G/H: four `GpuTerrainMaterial` words and their settings; `ground_sample` takes a layer and returns its weight. |
| `terrain/foliage_paint.rs` | I: layer-weight rejection, tilt. 3 new tests. |
| `somnium_core/src/app.rs` | I: `FoliageEntry`, two palette entries, `apply_foliage_palette_defaults`, tilt in the instance transform. |
| `tools/fetch_foliage.sh`, `tools/polyhaven_files.py` | **new.** The palette fetcher. |
| `.gitattributes` | **new.** Keeps git from rewriting glTF line endings. |
| `material/pool.rs`, `tests/shaders_validate.rs` | Layout 2064 to 2080. 4 new shader source tests. |

### The layout budget

G's two scalars land in the pad TSUSHIMA-B/C left behind, so the struct does not
grow for them at all. H's octave `vec4` takes it 2064 to 2080. Every
`array<vec4<_>>` keeps its offset, and both layout tests now pin the two new
offsets as well as the size.

---

## Tests

Four new shader source tests, all of the same kind and all for the same reason —
this phase has now shipped two filters that were computed into a local and then
discarded, both with a comment above them saying otherwise:

- `weight_noise_is_applied_before_strongest_four` — the perturbation is inside
  the unpack, and every call site unpacks before it selects. It pins the number
  of call sites in the composed root at two, so a path changing shape fails
  loudly rather than quietly losing the feature.
- `macro_octaves_land_between_the_macro_blend_and_the_squaring` — in both paths.
- `cliff_parallax_reaches_the_projected_sampler` — each plane samples the
  coordinate it marched, and the UV-space march is still excluded on cliffs.
- `one_parallax_march_serves_both_frames` — the march is declared once and
  neither wrapper has grown a loop of its own.

Three new Rust tests in `foliage_paint`: `debris_stops_where_its_layer_does`,
`a_zero_layer_threshold_places_on_bare_ground` and
`tilt_is_varied_bounded_and_off_by_default` — the last of which also asserts the
tilt is *not* a function of yaw, since the yaw is what chooses the lean's
direction and a tilt that tracked it would lean every instance the same way in
world space.

Full workspace after the change: 453 renderer lib, 26 shader validation, 741
`somnium_core`, 10 `hello_engine`, and the rest. All pass, no failures.

---

## Measured

`hello_engine`, Coastal, sun pinned at 8 degrees, frame 240, 1280x720, HDR
`.somcap`, all against a reference with G, H1 and H2 all off.

**`coastal-ground`**, over 705,355 terrain pixels. Two off-runs back to back are
**byte-identical**, so every number here is signal:

| | terrain radiance | mean abs | px changed |
|---|---:|---:|---:|
| all off | 821.56 | — | — |
| G, weight noise | 825.60 | 19.90 | 409,458 (58.0%) |
| H1, macro octaves | 785.25 | 37.83 | 655,273 (92.9%) |
| H2, sky-visibility tint | 816.16 | 5.40 | 67,786 (9.6%) |
| **all three, shipped** | **784.14** | **47.93** | **633,801 (89.9%)** |

**`coastal-vista`**, over 483,830 terrain pixels. This rail has a **noise floor**
— two off-runs are not identical, mean abs 1.37 over 44,151 pixels — so read
everything below against that, not against zero:

| | terrain radiance | mean abs | px changed |
|---|---:|---:|---:|
| all off | 841.00 | — | — |
| *noise floor* | *841.00* | *1.37* | *44,151 (9.1%)* |
| G, weight noise | 846.50 | 51.21 | 281,780 (58.2%) |
| H1, macro octaves | 852.89 | 13.59 | 159,454 (33.0%) |
| H2, sky-visibility tint | 837.33 | 4.49 | 103,795 (21.5%) |
| **all three, shipped** | **854.45** | **54.06** | **303,064 (62.6%)** |

Sky pixels are byte-identical in every row, which is the check that nothing
leaked outside terrain.

### G is *larger* at a distance, and that is the interesting part

Mean absolute change of 19.90 near the ground and 51.21 at a vista, on the same
share of pixels (58%). The obvious expectation is the opposite — a boundary
perturbation should matter most where you can see the boundary.

`FAR_LAYER_EPSILON` explains it. Past `detail_fade_end` the gate on a layer's
weight rises from 0.002 to 0.2, so a far pixel is almost always resolving to a
*single* material. Near the ground G reshuffles a blend between two layers and
moves the pixel a little; far away it changes which single layer wins outright
and moves the pixel from one material to another. Same feature, different
mechanism, and the far one is per-pixel much larger.

### H's two halves pull opposite ways, which is why they got two rails

The octaves **brighten** (852.89 against 841.00 at the vista): a perturbation
centred on 1.0 is symmetric in perceptual space, and squaring back to linear
gives it a mean above one. The tint **darkens** (837.33). Measured through a
single switch they would have partly cancelled and the net would have said
nothing about either — which is exactly what TSUSHIMA-F's single BRDF switch
did, reporting a 39% darkening that turned out to be three terms, one of them
brightening. The second rail was added after the first measurement showed the
same shape.

### The tint was tuned wrong first, and the measurement said so

The first cut used `smoothstep(0.99, 0.78, sky_vis)` with a strength of 0.35,
and moved **1,604 of 705,355 pixels**. Effectively nothing.

The mistake was reading TSUSHIMA-C's *range* instead of its *distribution*. C
reported 0.47 to 1.00 with a mean of 0.93, and 0.78 was picked as "most of the
way to the minimum". But 0.47 is one valley floor: almost every visible pixel
sits within a few hundredths of 1.0, so a remap stretched down to the minimum
was multiplying by about two percent across the whole frame.

A strength sweep confirmed the term was working and simply small — the response
is linear, 0.76 / 2.16 / 6.47 / 21.45 mean abs at strengths 0.35 / 1 / 3 / 10.
So the fix is the remap, not the gain: the upper edge is now exactly 1.0, so
genuinely open ground is the exact identity, and the lower edge is 0.88, just
below the mean, which is where the pixels that differ from open ground actually
are. With the strength at 0.6 that is 5.40 mean abs over 67,786 pixels — a
term you can find, and still a hue shift rather than a repaint.

**`SOMNIUM_TERRAIN_SKYVIS_TINT` is a float, not a flag**, because of this: `=0`
is the A/B off and any other value replaces the strength. The useful question
about a term this subtle is not whether it is on but how hard it has to push
before it says anything, and that should not cost a rebuild to ask.

### I.2 is unmeasured

Cliff parallax has no capture. Isolating it needs a build with the projected
march neutralised to compare against, and the phase stopped before that ran. It
is pinned by `cliff_parallax_reaches_the_projected_sampler` and by
`one_parallax_march_serves_both_frames`, which is a proof that it runs and
reaches the sampler, not a picture of it doing something worth having.

Terrain parallax also ships **off** (`parallax_scale` defaults to 0;
`SOMNIUM_TERRAIN_PARALLAX=1` turns it on), so nothing about the default image
changed here either way.

### Two things about the harness, for the next session

**`SOMNIUM_TERRAIN=1` does not get you the Coastal map.** It selects the 256 m
heightmap smoke test, and the map path is the `flat_terrain` branch — the
opposite of what the name suggests and of what `capture.rs`'s own doc comment
shows. Set `SOMNIUM_TIME_VIEW=coastal-vista` and leave `SOMNIUM_TERRAIN` unset.
Half an hour of this phase's A/B numbers were measured on the smoke test before
the log line `Heightmap terrain smoke test active` was noticed.

**The first process to render a rail after an idle gap is different from the
ones after it** — a different DPI (2560x1392 rather than the pinned 1280x720)
and cold residency. Back-to-back runs of one rail are byte-identical; the first
is not. Every batch here throws away two warm-up runs per rail, and that is what
turned the `coastal-ground` noise floor from garbage into exactly zero. It is
the same shape as the `.somtime` lesson: the noise band is within-run, and an
A/B has to be back to back.

---

## The A/B rails

- `SOMNIUM_TERRAIN_WEIGHT_NOISE=0` — G.
- `SOMNIUM_TERRAIN_MACRO_OCTAVES=0` — H1, the octaves.
- `SOMNIUM_TERRAIN_SKYVIS_TINT` — H2, the tint. A **float**: `=0` is off, any
  other value is the strength. Separate from H1 because the two pull in
  opposite directions and one switch cannot attribute a change to either.
- I.2 rides the existing parallax switch (`tm.parallax_steps`) and the
  `enable_pom` override, deliberately: it is the same feature as the heightfield
  march and there is no configuration in which one is wanted and the other is
  not.

Unlike B, C and E, "off" here is a strength of zero rather than an unbound
texture — there is no texture. The identity is exact: the perturbation loops do
not run and the octave product is 1.

---

## Outstanding

- **I.2 has no capture**, and neither does anything in I. The cliff parallax,
  the layer threshold, the densities and both tilt limits are reasoned and
  test-pinned rather than looked at.
- **No PNG evidence** was written for G or H; the numbers above are from HDR
  `.somcap` diffs only.
- **H item 3** — `terrain_detail_fade` still removes signal rather than
  re-injecting contrast. Unmeasured.
- **The projected parallax has no self-shadow.**
- **POM's dominant-layer flicker** under G at a boundary, near the ground.
- **No editor controls**, following B through F. Every switch here is an
  environment variable and a default.
- **The two debris meshes are not fetched**, so nothing has been seen with a
  pebble on it.
- **`micro_shadow_opacity` is still untuned at 1.0**, from F.

---

# The editor half — 2026-09-03, same day

G, H and I landed as environment variables and defaults, following B through F.
That is fine for a phase being measured and wrong for a phase being *used*: the
person judging this work has to be able to reach it. This is the pass that made
it reachable, plus the content it needs to be worth reaching.

## The palette went from four entries to twenty-five

Twenty-one more CC0 Poly Haven models — grass and flowers, ferns and shrubs and
moss, a quiver tree, deadwood, seven grades of rock from pebbles to boulders,
and three cliff faces. All committed, all MD5-verified. See "The meshes" above
for what the fetch script had to learn the hard way.

**`FoliageEntry` grew three named shapes rather than twenty-five copies of
eleven fields.** `cover`, `cover_on`, `prop` and `debris` are the four ways a
thing sits on ground, and the constructor name carries the reason: a tree is a
`prop` because it is placed one at a time, a pebble is `debris` because it leans
and refuses ground made of something else. Twenty-five struct literals would
have said the same thing twenty-five times and drifted the first time anyone
added a field.

**`max_slope_deg` became per-entry**, and a cliff face is why. The brush default
is 40 degrees, which is right for grass and refuses to put a rock wall anywhere
a rock wall belongs. `FoliageEntry::prop` takes it as an argument; the cliffs
pass 90.

Three tests pin the palette, all against silent failures:

- `the_palette_matches_the_fetch_script` reads the script's own asset list and
  fails both ways — an entry the script cannot fetch, and a download nothing
  uses. Without it a typo in a path is a brush that warns "not installed"
  forever, and nobody can tell that from a download that was never run.
- `the_combo_box_lists_the_palette` pins the UI's display names to the palette.
  `somnium_ui` sits below `somnium_core` in the dependency graph and therefore
  keeps its own copy of the names; this is what stops the copy drifting.
- `every_palette_entry_can_actually_paint` checks the things that fail by
  placing nothing: an inverted scale range, a spread brush at zero density, a
  layer index past the 32-layer table.

## The Details panel got the phase

Six sliders and three checkboxes under Terrain Tools, all defaulting on:

| Control | What it drives |
|---|---|
| Horizon shadow | B's baked horizon map, bound or unbound |
| Baked sky visibility | C's map and the landscape bent normal |
| Relief normal | E's mip-chained normal |
| Sky visibility | C's ambient strength |
| Relief takeover | E's hand-over distance, metres |
| Splat noise | G's peak weight displacement |
| Splat noise scale | G's coarse octave, cycles per metre |
| Macro octaves | H1's three bands, one multiplier |
| Damp tint | H2's sky-visibility hue shift |

**The bool-beside-a-float pairs collapsed into single floats.** B, C and E each
spelled their rail as an enable flag next to a strength, which is two things the
editor slider and the environment can disagree about for one dial. Every
TSUSHIMA-G/H/I rail is now one float whose zero is an *exact* identity in the
shader — the perturbation loops do not run, the octave product is 1 — so the
slider's left end is the pre-phase image rather than a faint version of the new
one. `env_strength` is the shared parser.

The three baked maps stayed checkboxes, and that is not an inconsistency: they
are enabled by their texture being **bound**, and unbinding is exactly the
pre-phase behaviour where a strength of zero would still be a fetch and a
multiply by one.

**The three toggles dispatch through one table.** Two separate message pumps
route inspector checkboxes and each is a chain of `if destination == handle`
blocks kept in step by hand. Three more blocks each is where that stops
happening, so `tsushima_toggle` is looked up once and both sites call it.

## Create → Terrain (Empty)

`Create -> Terrain` built the finished coastline preset: relief, an altitude
splat and a lake. That is the right thing when someone wants a scene in one
click and the wrong thing when they want to author one — the first two brush
strokes are then spent undoing a preset, and the water body has to be hunted
down and deleted.

The old entry is now labelled **Landscape**, and **Terrain (Empty)** is the
other half: flat at the datum, layer 0 everywhere, no water, no camera move.

`brush::fill_layer` is why it is layer 0 and not nothing. An all-zero splatmap
is not an empty terrain — the shader normalises the weights, finds nothing to
normalise, and falls back to the layer mean albedo, which is a flat grey plate
that reads as a bug rather than as ground waiting to be painted.

## Four kinds of water, and a river that follows a spline

`WaterComponent::body_kind` was documented as "0 = lake. Reserved for ocean and
river body types" since Phase 13. It is now a real `WaterBodyKind`, and the four
have genuinely different optics and dynamics rather than four names for one
look:

| Kind | Coverage | What separates it |
|---|---|---|
| Lake | baked mask | short fetch, clear enough to see the bed near shore |
| Ocean | wet rectangle | long swell, strong red absorption, low scattering |
| Sea | wet rectangle | shelf water: greener, ~2x the scattering, choppier |
| River | **swept channel** | shallow, turbid, almost flat |

The separation that matters is optical, not geometric, which is why coverage is
a *different* field: a lake and a sea share a coverage rule and look nothing
alike, and a channel ribbon serves a river and an irrigation ditch equally.
`the_water_kinds_are_optically_distinct` pins the orderings that carry the
physics — scattering rises ocean < sea < river with suspended load, and
wavelength falls the other way with fetch.

### The channel bake

`WATER_PRESET_CHANNEL` rasterises a ribbon: distance to the spline's polyline,
wet within `half_width`, a rounded bed deepest at the centreline, and a signed
shore distance encoded exactly as the baked lake's PNG is, so one shader contour
serves both. Everything downstream — the datum reprojection, the mask
decimation, `finite_mesh` — then works on a river without knowing one exists.

**Distance is to the segments, not the control points.** A point beside the
middle of a long straight reach is metres from the river and a hundred metres
from either vertex; a mask built from vertex distance is a string of beads with
dry gaps between them.

### Why editing it needs no dirty flag

The path lives in the descriptor as a fixed `[[f32; 2]; 16]`, and
`ensure_water_body` already compares descriptors for equality before rebaking.
So a control point the author drags simply *is* a different descriptor, and the
rebake follows. There is no dirty flag, and therefore no dirty flag anyone can
forget to set — which is the failure the terrain macro tier and TSUSHIMA-B's
horizon bake were both corrected for.

Fixed capacity rather than a `Vec` is what buys that: the descriptor has to stay
`Copy` and `PartialEq`. A spline longer than sixteen points is **decimated, not
truncated** — truncating loses the downstream half of a long river, and water
that stops in the middle of a valley looks like a bake bug rather than a
capacity one.

`channel_descriptor` lives in `landscape.rs` and is called from two places:
entity creation, which needs a baked mask before it can allocate a mesh, and the
per-frame refresh in `App`. One definition, so the two cannot disagree.

## Where this code went

| File | What |
|---|---|
| `somnium_core/src/app.rs` | `FoliageEntry` shapes and 25 entries; the TSUSHIMA slider arms; `toggle_terrain_baked_map`; `water_descriptor`; the Empty Terrain and four water Create handlers. 3 palette tests. |
| `somnium_core/src/landscape.rs` | `create_empty_terrain`, `create_water_body`, `channel_descriptor`. 4 tests. |
| `somnium_core/src/lib.rs` | `WaterBodyKind`, `sea()`, `river()`, `half_width`. |
| `somnium_renderer/src/water_body.rs` | `WATER_PRESET_CHANNEL`, `bake_channel`, `distance_to_polyline`, `Default`. 6 tests. |
| `somnium_renderer/src/terrain/mod.rs` | `env_strength`; the rails collapsed to floats. |
| `somnium_renderer/src/terrain/brush.rs` | `fill_layer`. |
| `somnium_ui/src/editor_event.rs` | 6 `TerrainToolField` variants, 3 toggle events, 5 `CreateKind` variants. |
| `somnium_ui/src/editor/inspector.rs`, `lib.rs`, `commands.rs`, `metaphor.rs` | Rows, handles, sync, dispatch, menu commands, icons. |
| `tools/fetch_foliage.sh`, `tools/polyhaven_files.py`, `.gitattributes` | **new.** |

## Outstanding, still

- **I.2 has no capture, and no further attempt should be made from a normal
  session.** Isolating cliff parallax needs a reference build with the projected
  march neutralised, and getting there means repeated `hello_engine` runs. Those
  runs took the machine to 15 GB resident and 97% memory. Whatever the capture
  harness leaks or simply holds at Coastal's size, an A/B that costs eight
  process launches is not a thing to run on a working machine. If this number is
  wanted, profile the engine's own footprint first.
- **The river does not carve its terrain.** It floats a ribbon at its datum; a
  channel through high ground will clip. Carving the heightfield along the
  spline is the obvious next move and is a terrain edit, not a water one.
- **No flow.** A river's `wave_dir_a` is authored rather than taken from the
  spline tangent, so the surface does not move downstream. The path is right
  there in the descriptor, so this is a small change nobody has made yet.
- **`micro_shadow_opacity` is still untuned at 1.0**, from F.
