# TSUSHIMA-K — three faults in how foliage receives light

Landed as `c4473a5` and `fba14e0` against `1f5dc36`. Found and fixed by Codex.

J restored the foliage normals. That is what made these visible: once the
geometry was lit correctly, three transport faults stopped hiding behind
scattered normals and became the thing you noticed instead.

All three are in `shading.wgsl`, and none of them is foliage code. They are
about foliage being the first material here that is thin, translucent and
densely self-occluding.

---

## K.1 — Sun transmission ignored sun visibility

Phase 24S added `transmitted_light` for the two-sided leaf lobe, after the
shadowed direct term and multiplied by nothing:

```wgsl
transmitted = transmitted_light(surface, light_dir, light_color, material.transmission);
```

Transmission says which side of a thin surface light leaves. It says nothing
about whether the light got there. So a leaf behind a trunk, a hill or a cloud
still glowed as though the sun were directly behind it, and it glowed hardest at
low sun angles, which is when the most terrain is in shadow.

The fix is one multiply:

```wgsl
transmitted = transmitted_light(
    surface, light_dir, light_color, material.transmission) * shadow_factor;
```

`shadow_factor` is the single number the engine already folds cascades, cloud
shadow, terrain horizon shadow and micro-shadowing into. Using it means
transmission respects every occluder the reflected term does, including any
added later.

Both reference engines already do this. Bevy applies a separate transmitted
shadow lookup for its diffuse-transmission lobe, and Unreal shadows
subsurface and two-sided foliage transmission through the same terms as the
direct lobe. Somnium was the outlier by omission: nothing in 24S argues for an
unshadowed term.

Guard: `sun_transmission_uses_sun_visibility` scans the composed shader between
`// Transmitted sunlight` and `let gi_texel` for the multiply, with whitespace
stripped so reformatting cannot break it.

---

## K.2 — The material AO map erased GTAO

```wgsl
if material.occlusion_map >= 0 {
    surface.occlusion = textureSample(...).r;   // assignment, not composition
    micro_occlusion = surface.occlusion;
}
```

`surface.occlusion` already held GTAO by that point, from line 1499. The
assignment threw it away, so every material carrying an occlusion texture
silently lost screen-space ambient occlusion.

Every Poly Haven foliage entry carries one, so foliage lost all of it. That is
why painted grass read as sitting on top of the terrain rather than in it. The
contact darkening where a tuft meets the ground is GTAO's job, and an authored
map cannot do it: the map knows the tuft's own interior shade and nothing about
what the tuft is standing on.

The second assignment is why the first one looked reasonable. `micro_occlusion`
has to stay material-only, because micro-shadowing is a hard cutoff on direct
light, and TSUSHIMA-F1 already established that feeding it a screen-space
estimate turns every wobble into a visible edge in sunlight. Two rules pulling
opposite ways on one texture fetch.

Sample once into a local, compose into ambient, keep the material-only copy:

```wgsl
let material_occlusion = select(
    textureSample(...).r,
    textureSampleGrad(...).r,
    analytic_grad,
);
surface.occlusion *= material_occlusion;
micro_occlusion = material_occlusion;
```

The `select` also collapses the old `if analytic_grad { } else { }`, which is
what makes "sampled once" something a test can check.

Guard: `material_occlusion_multiplies_gtao_without_polluting_micro_occlusion`
asserts the local exists, ambient multiplies, micro takes the material-only
value, and `surface.occlusion = textureSample` never comes back.

---

## K.3 — Moon transmission had nothing to shadow it

Phase 25M-2 added directional moonlight and gave it the sun's transmission lobe:

```wgsl
if is_foliage && material.transmission > 0.0 {
    moonlight += transmitted_light(surface, moon_dir, moon_color, material.transmission);
}
```

K.1's fix cannot reach this. The sun has `shadow_factor`. The moon has no
directional shadow receiver in the shading pass at all, so there is nothing to
multiply by.

Night exposure makes that much worse than it sounds. Auto-exposure is scaled to
a scene lit only by the moon, so an unshadowed lobe on an isolated back-facing
grass texel is amplified into a bright green pinprick. The user reported daytime
foliage looking right after K.1 and K.2 while night still looked wrong. The
pinpricks are what that was.

So the lobe is gone, with the reason left in place:

```wgsl
// Do not add the thin-leaf transmission lobe here. Unlike the sun,
// the moon has no directional shadow receiver yet, so transmission
// would make every isolated back-facing grass texel glow through
// terrain and neighbouring blades. Reflected moonlight remains.
```

Deferred, not refused. Transmitted moonlight returns when the pass can trace
visibility from the receiver toward the moon.

### K.3b — moonlight also predated the firefly bound

Found in the same block. Moonlight went through `evaluate_brdf` while the sun
went through `evaluate_brdf_area` wrapped in `clamp_specular_lobe`, so the
direct-lobe energy bound from TSUSHIMA-H never applied to it. Night is the worst
place to skip that bound, because exposure is highest exactly where sub-pixel
leaf cards can catch a narrow Fresnel peak.

```wgsl
moonlight = clamp_specular_lobe(
    evaluate_brdf_area(surface, moon_dir, light.sun_angular_radius),
    surface.roughness,
) * moon_color;
```

Reusing `sun_angular_radius` is not a shortcut. The sun and the moon subtend
almost the same angle from Earth, about half a degree, which is why total solar
eclipses work at all.

Guards: `night_foliage_does_not_add_unshadowed_moon_transmission` checks the
block still produces reflected light and no longer mentions `transmitted_light`.
`moonlight_uses_the_bounded_area_brdf` pins the whole composed expression.

---

## What connects them

Three faults, three phases apart, and the same mistake each time. A term was
added in the right place and the factor qualifying it was not.

Transmission arrived without visibility. Material AO arrived without
composition. Moonlight arrived without the bound the sun already had.

None of those is a wrong formula. Each is a correct formula missing the part
that says when it applies, and all three stayed invisible until foliage showed
up, because foliage is the first content here dense enough for one bad texel to
become a field of them.

J.3 landed on the same rule from the other side: a filter and the message
explaining it are one feature. Here it is a term and its qualifier. Either way,
shipping half of it produces something that behaves correctly in the case you
tested.

---

## Verification

| Check | Result |
|---|---|
| `shaders_validate` | 31 passed, including naga validation of every composed WGSL root |
| `somnium_renderer` lib | 498 passed |
| Release `hello_engine` | Builds |
| Fixed-frame capture | Ran, but no preset covers the painted-foliage scene the report came from |

The visual claim rests on the user's own before and after: daylight correct
after K.1 and K.2, night correct after K.3. No pinned capture reproduces that
camera and time of day. Saying so here is better than a phase record claiming a
golden image it does not have.
