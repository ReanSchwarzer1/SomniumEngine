# TSUSHIMA-H, first half — the specular fireflies

Landed 2026-09-02 across `557bcd3`, `551765f`, `f53d7e2`, `90d2afd`, `5c4e0ff`
and `bfdc046`. H is not finished: this covers the artifact work only. The macro
octaves the plan asks for are still open.

Reported as white sparkles on terrain, then found on painted foliage too. Three
separate defects were behind what looked like one bug, and a fourth report from
the same screenshots turned out not to be a lighting bug at all.

---

## The pixels said what was firing

`.somcap` HDR captures put the answer in two numbers.

The maximum was exactly `60000.0`, which is the shading pass's own output
clamp, so the offending pixels were not merely bright: they were saturating.
Their colour was `(60000, 60000, 29056)`. That ratio is the sun's, not the
sky's. So: direct sun specular, unbounded.

## Defect 1 — an antialiasing filter that ran and was thrown away

Geometric specular AA sat above the terrain branch in `shading.wgsl`. Terrain
then overwrote both the shading normal and the roughness, so every terrain
pixel computed the widened roughness and discarded it.

A comment three lines up claimed the filter ran last. Only the comment did.

Found by Codex during a parallel attempt. The fix is to move the call below
every writer of normal and roughness, and
`specular_aa_runs_after_every_normal_and_roughness_writer` in
`tests/shaders_validate.rs` now reads the shader source and fails if anything
moves back above it.

## Defect 2 — the lobe had no upper bound at all

Specular AA widens the lobe by the normal variance it can see. It cannot see:

- a lobe that is narrow because the surface is genuinely smooth, and
- sub-pixel geometry, which is the whole of a grass card.

So the filter alone does not close this. `clamp_specular_lobe` in `brdf.wgsl`
bounds the luminance of the direct specular response.

The first version used a flat ceiling of a quarter of incident light. That was
enough for terrain (isolated peaks 25 -> 4) and nowhere near enough for
foliage: a quarter of 100,000 lux is 25,000 against a scene mean near 3,000.
The amplifier is Fresnel. A leaf card seen edge-on has `v_dot_h` near zero,
Fresnel goes to 1, and the lobe returns twenty-odd times what the same leaf
returns face-on.

The shipped ceiling scales with roughness instead:

```wgsl
let ceiling = mix(0.08, 0.5, smoothstep(0.45, 0.0, roughness));
```

A dielectric's integrated specular reflectance is 3-5% at the roughnesses
ground and leaves actually have, so 0.08 is about twice it. A lobe peak may
exceed its own integral, but not by much on a surface this rough. It opens to
half of incident light as roughness approaches zero, where a mirror
legitimately does return most of what hits it.

### Why 2x and not 1x

Both were measured on the coastal rail:

| Ceiling | Mean | p99 | Saturating pixels |
|---|---|---|---|
| 1x integrated reflectance | — | **-15%** | 0 |
| 2x integrated reflectance | **-0.06%** | -0.5% | 0 |

1x removes the fireflies and takes a visible bite out of legitimate highlights.
2x removes them and costs essentially nothing.

## Defect 3 — micro-shadowing had no business on foliage

With the sparkles gone, the grass read as a flat orange wash. Micro-shadowing
was multiplying direct light by the foliage occlusion map, which encodes a
tuft's own interior shade at card scale and is already doing that job on the
ambient term. The grass was being darkened twice.

Foliage is now excluded, gated on `(material.flags & 2u) == 0u`.

This is the third time this term has been corrected, always the same way. It is
a hard cutoff, `saturate(N·L + 2·ao² − 1)`, and a hard cutoff is only as well
behaved as the field it thresholds. It has previously been wrongly fed GTAO
(which turned screen-space wobble into visible edges in sunlight) and baked sky
visibility (which answered a question about valleys with an answer about
grain). It takes the material AO map and nothing else.

## Not a defect — the white splotches are in the albedo

Reported in the same screenshots and circled as still broken. They are not a
lighting artifact and not a TSUSHIMA regression.

`SOMNIUM_SHADING_DEBUG=9` renders albedo alone. The splotches are there,
unchanged, with every TSUSHIMA feature switched off. Evidence:
`TSUSHIMA-H_white_is_albedo.png` and `TSUSHIMA-H_white_survives_all_off.png`.

They come from the source texture packs. That makes them content work, which is
what the rest of H is for, and no shading change will move them. Worth stating
plainly so a later session does not go looking in the BRDF again.

---

## Evidence

| File | Shows |
|---|---|
| `TSUSHIMA-FIX_coastal-vista.png` | Terrain after all three fixes |
| `TSUSHIMA-FIX_coastal-ground.png` | Ground-level, same |
| `TSUSHIMA-H_white_is_albedo.png` | Debug mode 9: the splotches without any lighting |
| `TSUSHIMA-H_white_survives_all_off.png` | The splotches with every TSUSHIMA rail off |
| `TSUSHIMA-H_rolloff_OFF.png` / `_ON.png` | The specular ceiling A/B |

## Still open in H

- Macro octaves, which is the work H was actually planned for.
- The albedo splotches in the source packs.
- `micro_shadow_opacity` is untuned at 1.0.
