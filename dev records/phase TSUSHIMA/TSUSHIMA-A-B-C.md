# TSUSHIMA-A, B and C — landed 2026-09-02

Against `b38ac74` (`dev`). A, B and C were done in one pass because B's bake is
also C's bake, and because A's whole purpose was to answer two questions that
decide what B and D are.

**Everything here ships on.** `SOMNIUM_TERRAIN_HORIZON=0` and
`SOMNIUM_TERRAIN_SKYVIS=0` are the A/B rails, not the defaults.

---

## Result, in one table

Coastal, `coastal-vista` rail, sun pinned at 8°, frame 240, 1920×1032.
Terrain radiance is the capture harness's own per-pixel mean over the
1,041,163 terrain pixels; `changed` is against the both-off control.

| build | terrain radiance | mean abs Δ | px changed |
|---|---:|---:|---:|
| both off (pre-phase) | 1566.80 | — | — |
| **B only** (horizon shadow) | 1440.18 | **126.79** | **140,279** (13.5%) |
| C only, as first written (`min`) | 1566.63 | 0.17 | 14,297 (1.4%) |
| C only, as shipped (product) | — | 6.59 *(on top of B)* | 296,281 (28.5%) |
| **B + C, shipped** | 1433.59 | — | — |

Sky and mesh pixels are byte-identical in every row (`mean_abs=0.0000`), which
is the check that neither feature leaked outside terrain.

Bake cost, measured: **101 ms** for 1024² from a 1025² heightfield in a release
unit test, **118–124 ms** in the engine on Coastal, **105 ms** on Island. Load
time, once, not frame time.

Captures: `TSUSHIMA-B_T3_off.png` / `TSUSHIMA-B_T3_on.png` are the pair the
13.5% is measured from. `TSUSHIMA-C_T3_product.png` is the shipped state.
`TSUSHIMA-BC_T5_island_on.png` is the second map.

---

## A — what the two questions turned out to be

### The vista rails exist because nothing looked at a vista

Both `-ground` rails sit at eye height inside the 100 m cascade range. That is
the one band where none of this phase's defects are visible: the shadows are
there, the detail is there, the terrain looks like terrain. Every capture the
phase argues from is a *vista*, and there was no rail for one — which is why
the defect survived this long.

`coastal-vista` and `island-vista` are `Hold` rails placed by a new `vista`
kit view: highest open point, camera 22 m above it, pitch −12°, yaw chosen by
sampling eight bearings and taking the one with the most land within 400 m. The
yaw search is there because a fixed yaw points at open water on one of the two
maps, and a capture of the sea is not a capture of terrain.

`SOMNIUM_SUN_ELEVATION` pins the sun's elevation and leaves its azimuth alone,
so the map's authored bearing still puts shadows where the terrain was built to
receive them. Applied before `sun::transmittance` reads `to_light.y`, so a
pinned low sun reddens exactly as an authored one would.

### §4.2, answered: the 100 m limit is real and it is the whole story

`SHADOW_DISTANCE` is `100.0` in `shadow/cascade.rs:23`. The traced-visibility
path (`restir_di.wgsl:135`, `tmax = 10000.0`) was **not** covering the gap in
the shipped configuration — the both-off capture has no cast shadow past the
near field, and turning the horizon map on changed 13.5% of terrain pixels.
If ray tracing had already been doing this job, that number would have been
near zero. It was not. **B was needed.**

### §4.3, not answered — and it stopped being A's question

The plan had A settle whether the ground receives aerial perspective when the
volumetric pass is off. That measurement was not taken: B turned out to be
large enough that finishing it was worth more than a diagnostic for D, and D is
not blocked on it — D can take its own capture when it starts. **This is
outstanding work, not a closed question.** It is the one item of A's list that
did not get done, and it is recorded here rather than quietly dropped.

### The pack audit was not run either

Same reason. It belongs to H, which is a long way off.

### One thing the plan asked for that already exists

A.1 specified a `tools/lookmetric` for terrain-pixel metrics. **It should not be
built.** `capture.rs` already carries a per-pixel terrain/mesh/sky mask and a
`CAPTURE-DIFF` that reports, per class, mean absolute difference and changed-
pixel count against a stored `.somcap`. Every number in the table above came out
of it. The plan asked for a tool because the clipmap audit had computed those
numbers by hand; it had computed them by hand because it did not know this
existed.

---

## B — the horizon map

`terrain/horizon.rs`. For each texel and each of eight compass azimuths, the
maximum elevation angle at which the terrain blocks the sky, packed as two
RGBA8 maps at 1024². Sampled in `shading.wgsl`'s terrain branch, interpolated
between the two bracketing azimuths, softened by `light.sun_angular_radius`,
and folded into `terrain_parallax_shadow_factor` — the channel the relief
self-shadow already uses, so there is one definition of how much direct sun
reaches a point.

Cross-faded in over `smoothstep(70, 100)` metres so the cascades keep the near
field. Inside 100 m they are strictly better: they see meshes, and the horizon
map only ever sees terrain.

### The march is multi-resolution, and it had to be

A naive eight-azimuth march that reaches across a 1 km terrain is ~2 billion
samples and does not finish inside a load. The occluder field is a
**max-downsampled pyramid** and the march strides by the mip it reads — unit
steps to eight texels, then doubling. ~40 samples per azimuth cross the whole
map. Max-downsampling rather than mean is what makes it safe: a mean lets a
thin ridge average away and stop casting, a max keeps every occluder and can
only over-shadow, by a margin far smaller than the sun's own penumbra at the
distances where coarse mips are read.

`the_march_reaches_across_the_whole_map` is the test that pins this: a 600 m
spike in one corner must still raise the horizon at the opposite corner.

### The bug that made the first measurement a lie

The first on/off capture pair was **byte-identical**. Terrain radiance agreed to
four decimal places, which is not a subtle effect — it is no effect.

`TerrainData::new` bakes against the heightmap the terrain is *created* with,
and that heightmap is flat. Relief lands afterwards, through
`load_heightmap_file` / `generate_relief` / `generate_island_relief`. So every
horizon angle was zero and every sky visibility was fully open, on both sides of
the A/B.

The macro tier had already solved this and said so in a comment three lines
above where the bake was added — *"Generated flat here and regenerated once
relief lands (see `macro_dirty`)"*. The bake now hooks the same
`mark_all_dirty` flag and rebakes in `rebuild_dirty_chunks`.

Deliberately **not** hooked to the sculpt brush: 100 ms per stroke would make
the brush unusable, and a slightly stale long shadow under an active brush is
not what anyone is looking at. All three `mark_all_dirty` callers are wholesale
heightmap replacements, so this costs one rebake per map load.

**The lesson is not "remember to rebake".** It is that an A/B whose two sides
agree exactly is evidence of a plumbing bug, not of a technique that does
nothing, and it should be read that way before the technique is blamed.

---

## C — sky visibility

The same bake also emits, per texel, the cosine-weighted fraction of sky the
point can see (`cos²` of each azimuth's horizon angle, averaged over eight) and
the average unoccluded direction. RGB is that direction, A is the visibility.

Measured on the shipped maps:

| map | horizon range | sky visibility min / mean / max |
|---|---|---|
| Coastal (105 m of relief) | 0° – 78.4° | 0.47 / 0.93 / 1.00 |
| Island (22 m of relief) | 0° – 45.2° | 0.80 / 0.99 / 1.00 |

Island's numbers are honest and small. It is a low rolling islet; there is very
little there to occlude anything, and C should barely show on it. Coastal is
where the term earns its place.

### `min` was wrong, and measuring it is what showed that

The plan said to combine sky visibility with the existing occlusion by `min`,
not by a product, on the argument that two occlusion terms describing the same
hemisphere should not both apply. It also said to measure both. Both were
measured:

- `min`: mean abs Δ **0.17**, 14,297 pixels (1.4%).
- product: mean abs Δ **6.59**, 296,281 pixels (28.5%).

`min` can only bite where baked sky visibility is *lower* than GTAO's answer,
which on this terrain is the floor of a deep valley and almost nowhere else.
It is 38× weaker and it is weaker in the wrong places.

The product is also the more defensible of the two, and the plan's argument was
simply wrong. The two terms are very nearly **independent**: GTAO searches a few
metres and cannot see a ridge line; the bake sees the ridge line and has no idea
a boulder is sitting next to you. They occlude different parts of the sky, and
the surviving fraction under two independent occluders is the product. `min` is
correct only when one term's occluders are a superset of the other's, which is
exactly what these two are not.

Worth keeping in mind for F: the terrain's ambient term is **25%** of its total
radiance at this camera and sun (sun-only 1394.70, ambient-only 469.43, debug
modes 2 and 3). That is large enough that anything scaling ambient matters, and
it is why `min`'s near-invisibility was a plumbing answer rather than a physical
one.

### The bent normal is composed, not replaced

First version overwrote `surface.bent_normal` with the landscape-scale
direction. That throws away GTAO's contact-scale answer for the same reason
`min` was wrong — the two see different occluders. It now sums the two unit
directions and renormalises, then pulls a quarter of the way back toward the
geometric normal so a deep valley still shades as ground rather than as a wall.

`evaluate_ibl_diffuse` already gathers along `surface.bent_normal` at a 0.75
mix and needed **no change at all**, which is most of why C was cheap.

---

## Where the code went

| File | What |
|---|---|
| `terrain/horizon.rs` | **new.** Bake, pyramid, packing, upload, rewrite. 8 tests. |
| `terrain/mod.rs` | Module, `HorizonGpu` on `TerrainData`, three texture ids, four `GpuTerrainMaterial` words, bake at creation, rebake on `horizon_dirty`. |
| `renderer.rs` | Three bindless registrations beside the macro map. |
| `shaders/terrain_material.wgsl` | Four struct fields, `terrain_horizon_shadow`, `terrain_sky_visibility`, `TERRAIN_PI`. |
| `shaders/shading.wgsl` | One call site in the terrain branch. |
| `material/pool.rs` | Layout assertion 2032 → 2048. |
| `tests/shaders_validate.rs` | WGSL span 2032 → 2048. |
| `examples/hello_engine/src/dreams_fixture.rs` | Two rails, two tests. |
| `examples/hello_engine/src/main.rs` | `vista` kit view, `-vista` dispatch, sun-elevation pin. |

### Two things the shader had to be told

**`PI` is not in scope in `terrain_material.wgsl`.** It comes from `brdf.wgsl`,
and this file is composed into two roots with different dependency sets:
`shading.wgsl` pulls `brdf.wgsl` in and `clipmap_gen.wgsl` does not. Borrowing
the name parses in one root and fails in the other. It has its own
`TERRAIN_PI`, which collides with neither. `every_composed_root_validates`
caught this immediately, which is the value of validating every root rather
than the one being worked on.

**Both lookups live in `shading.wgsl`, not in `evaluate_terrain_material`.**
They are properties of the heightfield, not of the material, so putting them in
the material function would mean writing them a second time in
`evaluate_clipmap_material` and a third in the virtual-texture path — and those
copies would drift. One call site covers all three paths.

### Why no pipeline override

Hex and POM are behind `override`s because they gate a multi-step march, where
leaving the code resident costs occupancy on every terrain pixel whether or not
the march runs. These are two texture fetches and a compare. The CPU unbinds
the maps to `-1` when the feature is off — the same sentinel `macro_map` has
always used — and the branch is coherent across a draw because every pixel of a
terrain reads the same material. This also avoids a tenth entry in
`ShadingSpec::constants` and a new variant in a budget `context.md` tracks.

---

## Tests

445 renderer lib (8 new in `horizon`), 21 shader validation, 11 hello_engine
(2 new in `dreams_fixture`). All pass.

The two fixture tests are there specifically to pin the *additive* property:
TSUSHIMA reuses DREAMS' rail fixture rather than building a second one, so
`coastal-ground` still settling on its anchor at frame 240 is what stops a
TSUSHIMA change silently invalidating a DREAMS evidence folder.

`horizon_bake_cost_at_shipped_size` prints rather than asserts. A speed
assertion on a shared machine is a flaky test; a printed number is a
measurement.

---

## Outstanding

- **§4.3 is unmeasured.** Whether the ground gets aerial perspective with the
  volumetric pass off. D's first capture, not a blocker.
- **The pack audit.** Belongs to H.
- **Sculpt does not rebake.** By choice, recorded above.
- **Island barely moves.** Expected from its relief, but it means the second
  map is not currently much of a check on this feature. A vista on Coastal is.
- **`horizon_takeover`'s 70–100 m cross-fade is untuned.** It matches
  `SHADOW_DISTANCE` because that is where the cascades stop, and it has not been
  looked at with an eye to whether the seam is visible.
