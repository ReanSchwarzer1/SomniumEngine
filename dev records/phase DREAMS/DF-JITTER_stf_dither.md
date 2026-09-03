# The terrain jitter is stochastic filtering, and it was indexed wrong

Measured, not inferred. Camera stationary, `SOMNIUM_TIME_STATIC=1`, coastal map
from 250 m looking down at -35 degrees, comparing consecutive frames 239 and
240. A still camera over frozen terrain should produce two identical frames.

```
SOMNIUM_MAP=coastal SOMNIUM_TIME_STATIC=1
SOMNIUM_CAMERA_POS=0,250,0 SOMNIUM_CAMERA_YAW=45 SOMNIUM_CAMERA_PITCH=-35
SOMNIUM_CAPTURE_FRAME=239 / 240
```

## Ablation

| ablation | pixels moving by more than 2 | mean abs delta |
|---|---|---|
| none (baseline) | 1.99% | 0.513 |
| `SOMNIUM_DREAMS_STF=0` | **0.44%** | 0.194 |
| `SOMNIUM_DREAMS_GRAIN=0` | 1.98% | 0.515 |
| `SOMNIUM_CLOUD_JITTER=0` | 1.99% | 0.513 |
| `SOMNIUM_AERIAL=0` | 1.99% | 0.513 |

Stochastic texture filtering is the whole of it. Shared grain, cloud jitter and
aerial perspective are not involved, which also clears the two DREAMS-B
switches of everything except this one.

## The defect: the dither was indexed by texture coordinate

```wgsl
let tile = vec2<i32>(i32(floor(uv.x * 64.0)) & 63, i32(floor(uv.y * 64.0)) & 63);
```

Two things follow, and both are wrong.

**It moves when the camera does not.** TAA jitters the sample position by a
fraction of a pixel every frame. That moves `uv`, which moves
`floor(uv * 64.0)` across a tile boundary, which flips the decision. A flipped
decision here is a whole mip level, so a stationary camera over static terrain
changed 2% of the frame every frame.

**It gives the technique nothing to resolve against.** One 64x64 tile spread
over *texture* space means every pixel inside a tile shares one decision. The
premise of filtering stochastically is that neighbours disagree so the error
averages out; identical neighbours leave nothing for TAA's neighbourhood to
average, and the result is blotches flipping in unison rather than noise.

The comparison is `grain_masks` in GTAO, which indexes the same shared atlas by
screen coordinate and frame:

```wgsl
textureLoad(grain_masks, grain_coord, i32(params.frame & 63u), 0).r
```

Terrain reached for `grain_words`, a flat 64x64 copy in a uniform buffer with no
temporal dimension, and then indexed it by surface position.

## Fix

Index by screen position. `terrain_screen_pixel` is published at the top of
`fs_main` alongside the other `terrain_*` privates, and the per-layer shift
stays: it decorrelates the two to four layers blended at one pixel, and the
layer index is a property of the surface, so it costs no temporal stability.

| | pixels moving | mean delta |
|---|---|---|
| STF, UV-indexed (before) | 1.99% | 0.513 |
| STF, screen-indexed (after) | **1.06%** | 0.369 |
| STF off | 0.44% | 0.194 |

Halved, and correct. Not finished.

## The residual is the technique, and the technique is not buying anything

Replacing the stochastic choice with a plain fractional LOD:

```wgsl
return textureSampleLevel(textures[map], default_sampler, uv, lod);
```

| | pixels moving | mean delta |
|---|---|---|
| hardware trilinear | **0.43%** | 0.193 |
| STF off | 0.44% | 0.194 |

**Hardware trilinear lands exactly on the STF-off floor.** So every remaining
moving pixel is the stochastic mip choice itself, and nothing else.

That matters because `textureSampleLevel` with a fractional level *is*
trilinear, performed by the sampler, at no extra cost. Stochastic mip selection
replaces one hardware-filtered tap with one unfiltered tap and saves nothing —
the sampler was never going to issue two.

Stochastic filtering earns its keep where the hardware cannot filter for you:
filtering after a nonlinear decode, texture-space blurs, filters wider than the
sampler offers. Choosing between two mips a hardware sampler already blends is
the one case where it is pure cost. Measured here: 2.5x the temporal
instability of trilinear, for a tap the sampler was doing anyway.

## Recommendation

Default `dreams_stf` off, the same way `terrain_clipmap` now is, and keep the
STF machinery for a filter the sampler cannot do. That is a DREAMS-B design call
rather than a defect, so it is written down here rather than taken.

The screen-indexing fix stands either way: it is correct wherever the dither is
used, and the UV indexing was a bug on its own terms.
