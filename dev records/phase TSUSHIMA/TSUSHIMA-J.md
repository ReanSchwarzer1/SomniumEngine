# TSUSHIMA-J — the foliage that was never a card, and two things that failed in silence

Against `eb61dc3`. Three defects, reported together and unrelated except that
all three had been visible for a while and none of them said anything.

> "foliage looks EXTREMELY bad, and broken no matter the lighting conditions"
> "when i try to paint some foliage it doesnt paint (only happens with some)"
> "weird ui problems again for the viewport bar"

---

## J.1 — The curved-card normal, applied to things that are not cards

### What was wrong

`shading.wgsl` has carried a port of Spartan's "foliage curved normals" since
Phase 17E. A leaf drawn as a flat card shares one normal across its whole width
and lights as a plate, so the shader rotates the shading normal about the
blade's long axis by an angle taken from how far across the card the pixel sits.
Spartan carries a `width_percent` vertex attribute for that. Somnium substituted
`uv.x`, with a comment saying that on a foliage card `uv.x` **is** the distance
across the blade — "which is what makes this free here".

It is free, and it is only true of a card.

The gate was `MATERIAL_FLAG_FOLIAGE`, and `app.rs` sets that on every non-opaque
material of every palette entry. So the rotation was running on all of them.

### The measurement

`assets/foliage/grass_medium_01/grass_medium_01_2k.gltf`, read straight out of
the buffer:

| | |
|---|---|
| Primitives | 17, named `Plane.037` … `Plane.059` |
| Vertices | 44 to 7,044 each |
| `u` range, per primitive | 0.016 … 0.996 |
| Vertex-normal coherence | 0.216 … 0.883 |

Coherence is the length of the mean unit normal: 1.0 is a plate, 0 is a sphere.
Nothing here is above 0.89.

So the model is seventeen modelled clusters sharing **one atlas**, and `uv.x` is
the blade's address in the texture sheet rather than a position across it. The
rotation spans ±60°, so two blades whose art sits at opposite ends of the atlas
were being bent 120° apart from each other.

### Why it looked like a lighting bug and was not

A ground plane of scattered normals produces:

- **daylight** — blotches, because N·L varies per-blade for no physical reason;
- **night** — a sheet of white specular sparkle, because the only light left is
  environment specular and every normal is pointing somewhere else;
- **every distance and every sky** — the same, because the input is a texture
  coordinate and does not care about any of them.

That last property is the tell, and it is why no lighting change ever helped.
The engine already carries a roughness floor for foliage (25M-2), a specular
firefly filter (`90d2afd`), a micro-shadow exclusion (`5c4e0ff`) and a
normal-variance roughness widen — four separate attempts at symptoms of this.

### The fix, and the part that matters

`MATERIAL_FLAG_FOLIAGE_CARD` (bit 2), `foliage_card` on `LoadedMaterial` and
`MaterialAsset`. `FOLIAGE` keeps the two claims that are true of any vegetation
— two-sided transmitted light, and the roughness floor. `FOLIAGE_CARD` carries
the one claim that is about geometry, and only it reaches the rotation.

**It is authored and never inferred.** Inference is what put the bug here: the
flag was set from the `*_alpha_*` sidecar convention, which identifies a
*cut-out*, not a card. Geometry cannot separate them either — a crossed-quad
billboard is a genuine card and has exactly the normal spread of a modelled
tuft that is not one. Both would sit around 0.7 coherence. There is no honest
detector, so the flag is a material property with a default of `false` and a doc
comment that says what it costs to set it wrongly.

Nothing in the shipped palette sets it, which means the effect is off for every
asset the engine currently has. That is the correct answer for this content, not
a removal: an authored card asset turns it on and gets the effect it was written
for.

---

## J.2 — Vegetation lighting decided by the exporter

Found while reading J.1. The palette promoted a material to vegetation on
`alpha_mode == Blend`. Surveying all 25 entries:

| Export | Entries |
|---|---|
| `BLEND` | grass ×2, moss, island-tree leaves, quiver-tree leaves |
| `MASK` | dandelions, fern, nettles, shrub ×2 |
| `OPAQUE` + `*_alpha_*` sidecar | grass bermuda, fir sapling |
| `OPAQUE` | every rock, cliff, stone, boulder, stump, trunk, branch |

So ferns, shrubs, dandelions and nettles got **no** transmission, **no**
two-sidedness and **no** roughness floor. They were lit as solid dielectrics —
dark, flat, and missing every back face — because of which exporter their author
used. The sidecar route rescued two of the `OPAQUE` ones by accident.

The test is `!= Opaque` now. That still excludes bark, because on every
multi-material tree in this palette the trunk and branches are the opaque parts
and only the leaf material is not. A capability test that keys on an exporter's
habit is a coin toss wearing a rule's clothes.

---

## J.3 — A brush that refuses correctly, and says nothing

`FoliageBrush::min_layer_weight` (TSUSHIMA-I) is a hard threshold: pebbles need
Gravel painted under them, moss needs Mossy Rock, nettles need Mud. **Thirteen
of the twenty-five palette entries carry one.** Point any of them at a terrain
still painted Grass and every candidate is rejected.

The rejection is right, and it is the whole reason a gravel patch reads as
gravel instead of fading into a lawn. What was wrong was one line:

```rust
let added = foliage_paint::paint(...);
if added > 0 { info!("Foliage: painted {added} ..."); }
```

Zero placed meant zero output. No log line, no status, nothing. The user's
report — "only happens with some" — is precisely the shape of thirteen entries
out of twenty-five silently refusing.

`paint` returns a `PaintReport` now: placed, `too_steep`, `wrong_layer`,
`too_close`, and the strongest layer weight actually found under the brush.
`refused()` deliberately **excludes** `too_close`, because painting over ground
that is already covered is a stroke converging, which is the brush working. The
message names the layer and quotes both numbers:

> Foliage: Pebbles needs Gravel painted here — it wants a weight of 0.50 and the
> ground under the brush has at most 0.03. Paint Gravel with the terrain brush
> first, or lower "Min layer" in the Foliage Brush section.

Once per stroke, latched on mouse-down. A brush dabs on every mouse-move, so a
message per dab would have been its own bug.

**Min layer** joins the Foliage Brush section, because being told why is only
half an answer when the rule cannot be relaxed. Moss on a grass hillside is a
legitimate thing to want.

---

## J.4 — The viewport bar sliced controls in half

Reported as "Resoluti" with its dropdown gone.

### What the bar was

One `StackPanel` with `clip_to_bounds`, in a grid whose second column is
reserved for the actions. MORROWIND-J had already found half of this: a stack
does not shrink, it lays the overflow past the edge, so the newest control is
the one that disappears. The reserved actions column fixed the *end* of the bar.
It left everything before the end to be cut at the pixel — a label truncated
mid-word with the control it names gone, and nothing on screen to say so.

Only the snapping cluster could collapse. Everything else was a bare sibling.

### The two measurement faults

Measured with a layout harness at `ChromeLayout::default().resolved(1920, 1080)`:

| Cluster | Desired width |
|---|---:|
| Snapping | 482 px |
| Camera speed | ~330 px |
| Resolution | ~130 px |
| Day cycle | 169 px |
| Actions (reserved) | 75 px |

- `context_bar_full_width` was cached **once**, the first frame the snap cluster
  was visible. The day-cycle cluster is hidden until the scene has an
  Environment, so on almost every startup the number was learned **without it**
  and under-read the bar by all 169 px.
- `available` was the viewport width minus the actions column. The bar carries
  `Thickness::uniform(12.0)`, and those 24 px were never subtracted.

193 px of phantom room, and the stack quietly cut the difference off.

### The fix

Every control is now in a cluster with its own label — `speed_cluster`,
`res_cluster`, the existing snap and time clusters — so the unit of overflow is
a whole control-with-its-label rather than a pixel. `fit_context_bar` runs every
layout: it learns each cluster's width whenever that cluster happens to be
visible, sums what the owners want, subtracts the inset, and hides clusters in
order until it fits.

The order is what a person can still do without each one:

1. **Snapping** — the chevron beside it opens the same controls.
2. **Camera speed** — RMB + scroll wheel sets it, and its tooltip says so.
3. **Resolution**, then the **day cycle** — neither has a second route.

Two rules that are easy to get wrong and are now pinned by tests:

- **A cluster that has never been measured is never dropped.** A hidden node
  measures to zero, so dropping one for the zero it contributed itself would
  make its width unlearnable and it would never be seen again at any size.
- **Overflow may only subtract from what the owner wants.** "No Environment, so
  no clock to scrub" and "no room for the clock" are different reasons, kept in
  different arrays. Confusing them means the first wide window brings back a
  control that has nothing to control.

The decision is a free function, `context_bar_visibility(widths, wanted,
available)`, so the arithmetic is testable without standing up a shell.

---

## Tests

New:

- `foliage_paint`: `a_refused_dab_names_the_layer_that_refused_it`,
  `a_dab_on_a_cliff_blames_the_slope_and_not_the_layer`,
  `a_settled_stroke_is_not_a_refusal`.
- `zeta_layout_tests`: `every_context_bar_control_belongs_to_a_cluster_with_its_label`,
  `the_whole_context_bar_fits_a_1920_window`,
  `the_context_bar_sheds_clusters_in_order`,
  `overflow_can_only_subtract_from_what_the_scene_wants`,
  `an_unmeasured_cluster_is_kept_so_it_can_be_measured`.

Changed: `schema_is_complete_and_texture_slots_reject_non_textures` counts 18
material fields.

### Not proven here

None of J.1's or J.2's effect on the image is captured. The engine may not be
launched from this session, so the shader gate and the vegetation-flag change
are argued from the glTF measurements above and from the code path, not from a
frame. Both are one-line gates whose behaviour is fully determined by the flag,
which is the reason that argument is worth anything at all — but a capture is
still owed, and the honest place to say so is here.

`gltf_import_writes_editable_material_and_embedded_texture_siblings` fails on a
second run in the same process tree because its temp directory is named from a
process-local counter and accumulates PNGs. Pre-existing, unrelated, and it cost
half an hour to rule out.
