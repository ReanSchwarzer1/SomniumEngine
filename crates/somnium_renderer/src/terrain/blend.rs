//! Height-weighted material blending (Phase 25E).
//!
//! Splat weights say *how much* of each material is here. They say nothing
//! about which one is *on top*. Normalising them cross-fades: at the seam
//! between rock and gravel every pixel is 50% of each, which is a colour that
//! exists in neither material and reads as a smudge. What actually happens on
//! the ground is that gravel settles into the rock's crevices — the boundary
//! follows the rock's own relief, not a straight line through weight space.
//!
//! The fix is to fold each material's height map into its weight before
//! deciding the winner, then blend only across a narrow band around it. Two
//! parameters per material control that, and they are what makes this an
//! authoring feature rather than one global look:
//!
//! - [`LayerBlend::height_scale`] — how much of the layer's own relief enters
//!   its weight. Rock crevices are deep; wet mud is nearly flat.
//! - [`LayerBlend::blend_width`] — the width of the transition band. Narrow is
//!   a hard interlocking edge (gravel in cracks); wide is a soft drift (snow).
//!
//! # Reference
//!
//! O3DE's `TerrainDetailHelpers.azsli` — `AppendHeightToWeight` and the
//! depth-blend loop in `GetDetailSurface`
//! (`example_repo/o3de-development/Gems/Terrain/Assets/Shaders/Terrain/`).
//! Defaults there are `m_heightBlendFactor = 0.5` and
//! `m_heightWeightClampFactor = 0.1` (`TerrainDetailMaterialManager.h`).
//!
//! [`blend_weights`] is a mirror of the WGSL in `terrain_material.wgsl`. It
//! exists because the properties that matter here — that a barely-present
//! material cannot steal a pixel, that equal heights degrade to a plain blend,
//! that the result sums to one — are cheap to state and impossible to check by
//! looking at a hillside. The GPU side is checked by the capture A/B; this
//! checks that the algorithm being ported is the right one.

use super::textures::TERRAIN_LAYER_COUNT;

/// Per-layer height-blend parameters.
#[derive(Clone, Copy, Debug)]
pub struct LayerBlend {
    /// How much of this layer's height map is added to its splat weight.
    ///
    /// The height map is `[0, 1]` and weights are `[0, 1]`, so this is
    /// effectively "how many splat-weight points is full relief worth". At 1.0
    /// a material's crevices can hand the pixel to whatever is beside it even
    /// where its own weight dominates.
    pub height_scale: f32,
    /// Width of the transition band, in weight units.
    ///
    /// Only materials within this much of the winner contribute at all. Small
    /// values give an interlocking edge that follows the height maps; large
    /// values approach the old cross-fade.
    pub blend_width: f32,
    /// Splat weight at which this layer's height counts in full.
    ///
    /// Below it the height contribution scales down linearly, which is the
    /// whole point: a material with 3% coverage and a tall height map would
    /// otherwise out-rank one with 60% coverage and a flat one, and a texture
    /// nobody painted would appear in the middle of a field. O3DE authors this
    /// as a threshold and uploads its reciprocal; so do we.
    pub min_weight: f32,
}

/// Per-layer blend parameters, keyed to `textures::LAYER_MATERIALS`.
///
/// These are material properties, not tuning constants — the numbers describe
/// what the photographed surface is. Rock and gravel are hard-edged with deep
/// relief; snow and mud are soft and nearly flat.
pub const LAYER_BLENDS: [LayerBlend; TERRAIN_LAYER_COUNT as usize] = [
    // 0 aerial_grass_rock — grass over stone; some relief, ordinary edge.
    LayerBlend { height_scale: 0.5, blend_width: 0.50, min_weight: 0.10 },
    // 1 forrest_ground_01 — leaf litter sits in real layers.
    LayerBlend { height_scale: 0.7, blend_width: 0.35, min_weight: 0.10 },
    // 2 aerial_rocks_04 — deep crevices, and the edge of a rock is an edge.
    LayerBlend { height_scale: 1.0, blend_width: 0.15, min_weight: 0.08 },
    // 3 snow_02 — drifts. Soft boundary, and its own micro-relief should not
    //   be what decides where the snow line falls.
    LayerBlend { height_scale: 0.35, blend_width: 0.60, min_weight: 0.15 },
    // 4 leafy_grass — coarser than layer 0, so a little more relief.
    LayerBlend { height_scale: 0.6, blend_width: 0.45, min_weight: 0.10 },
    // 5 brown_mud — wet and smooth; nothing to interlock with.
    LayerBlend { height_scale: 0.3, blend_width: 0.55, min_weight: 0.10 },
    // 6 coast_sand_rocks_02 — sand fills, pebbles poke through.
    LayerBlend { height_scale: 0.5, blend_width: 0.35, min_weight: 0.10 },
    // 7 gravel_floor — the case this phase exists for: gravel settling into
    //   the cracks of whatever it meets.
    LayerBlend { height_scale: 0.9, blend_width: 0.15, min_weight: 0.08 },
];

/// Reciprocal of `min_weight`, which is the form the shader multiplies by.
///
/// Guarded the same way O3DE guards it, because a zero threshold means "height
/// always counts in full" and must not divide to infinity.
pub fn weight_clamp(min_weight: f32) -> f32 {
    1.0 / min_weight.max(0.0001)
}

/// Below this weight a layer cannot change the result and is not sampled.
///
/// Mirrors `LAYER_WEIGHT_EPSILON` in `terrain_material.wgsl`.
pub const WEIGHT_EPSILON: f32 = 0.002;

/// Fold each layer's height into its weight, then blend across a narrow band.
///
/// `weights` are normalised splat weights, `heights` the layers' own height
/// maps at this texel. Returns weights that sum to one.
#[must_use]
pub fn blend_weights(
    weights: &[f32; TERRAIN_LAYER_COUNT as usize],
    heights: &[f32; TERRAIN_LAYER_COUNT as usize],
    params: &[LayerBlend; TERRAIN_LAYER_COUNT as usize],
) -> [f32; TERRAIN_LAYER_COUNT as usize] {
    let live = |i: usize| weights[i] >= WEIGHT_EPSILON;

    // AppendHeightToWeight. The clamp is the part that stops a sliver of a
    // material with a tall height map from out-ranking the material that is
    // actually painted here.
    let mut w = [0.0f32; TERRAIN_LAYER_COUNT as usize];
    for i in 0..TERRAIN_LAYER_COUNT as usize {
        if !live(i) {
            continue;
        }
        let p = params[i];
        let height = heights[i] * p.height_scale;
        w[i] = weights[i] + height * (weight_clamp(p.min_weight) * weights[i]).min(1.0);
    }

    // Depth blend: the winner's band sets the floor, and `min_depth` lets a
    // wide-blending material widen the band for everything it touches — which
    // is what keeps a soft material soft against a hard one.
    let mut max_w = 0.0f32;
    let mut min_depth = f32::NEG_INFINITY;
    for i in 0..TERRAIN_LAYER_COUNT as usize {
        if !live(i) {
            continue;
        }
        max_w = max_w.max(w[i]);
        min_depth = min_depth.max(w[i] - params[i].blend_width.max(0.001));
    }

    let mut out = [0.0f32; TERRAIN_LAYER_COUNT as usize];
    let mut total = 0.0f32;
    for i in 0..TERRAIN_LAYER_COUNT as usize {
        if !live(i) {
            continue;
        }
        let local_min = min_depth.max(max_w - params[i].blend_width.max(0.001));
        out[i] = ((w[i] - local_min) / (max_w - local_min).max(1e-4)).max(0.0);
        total += out[i];
    }

    if total > 0.0 {
        for v in &mut out {
            *v /= total;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = TERRAIN_LAYER_COUNT as usize;

    fn uniform(scale: f32, width: f32, min_weight: f32) -> [LayerBlend; N] {
        [LayerBlend { height_scale: scale, blend_width: width, min_weight }; N]
    }

    fn sum(w: &[f32; N]) -> f32 {
        w.iter().sum()
    }

    #[test]
    fn the_result_always_sums_to_one() {
        let mut weights = [0.0; N];
        weights[0] = 0.6;
        weights[2] = 0.3;
        weights[7] = 0.1;
        let mut heights = [0.0; N];
        heights[0] = 0.2;
        heights[2] = 0.9;
        heights[7] = 0.55;
        let out = blend_weights(&weights, &heights, &LAYER_BLENDS);
        assert!((sum(&out) - 1.0).abs() < 1e-5, "{out:?}");
    }

    #[test]
    fn equal_heights_leave_the_ordering_alone() {
        // With nothing to separate them by relief the blend must not invent an
        // order — the material with more splat coverage still wins.
        let mut weights = [0.0; N];
        weights[0] = 0.7;
        weights[3] = 0.3;
        let heights = [0.5; N];
        let out = blend_weights(&weights, &heights, &uniform(1.0, 0.5, 0.1));
        assert!(out[0] > out[3], "{out:?}");
    }

    #[test]
    fn a_sliver_with_a_tall_height_map_cannot_steal_the_pixel() {
        // The bug this phase's clamp exists for. Layer 2 covers 4% of the texel
        // and is at full relief; layer 0 covers 96% and is in a hollow. Without
        // the weight clamp, 0.04 + 1.0 beats 0.96 + 0.0 outright.
        let mut weights = [0.0; N];
        weights[0] = 0.96;
        weights[2] = 0.04;
        let mut heights = [0.0; N];
        heights[2] = 1.0;
        let out = blend_weights(&weights, &heights, &LAYER_BLENDS);
        assert!(
            out[0] > out[2],
            "a 4% layer took the pixel from a 96% one: {out:?}"
        );
    }

    #[test]
    fn without_the_clamp_that_sliver_does_steal_it() {
        // The companion to the test above: proof the clamp is load-bearing
        // rather than incidentally satisfied by the other parameters.
        let mut weights = [0.0; N];
        weights[0] = 0.96;
        weights[2] = 0.04;
        let mut heights = [0.0; N];
        heights[2] = 1.0;
        // min_weight 0 ⇒ height always counts in full.
        let out = blend_weights(&weights, &heights, &uniform(1.0, 0.15, 0.0));
        assert!(out[2] > out[0], "{out:?}");
    }

    #[test]
    fn a_narrow_band_is_harder_than_a_wide_one() {
        let mut weights = [0.0; N];
        weights[0] = 0.55;
        weights[2] = 0.45;
        let mut heights = [0.0; N];
        heights[0] = 0.30;
        heights[2] = 0.55;

        let narrow = blend_weights(&weights, &heights, &uniform(1.0, 0.05, 0.1));
        let wide = blend_weights(&weights, &heights, &uniform(1.0, 0.90, 0.1));
        let spread = |o: [f32; N]| (o[0] - o[2]).abs();
        assert!(
            spread(narrow) > spread(wide),
            "narrow {narrow:?} wide {wide:?}"
        );
    }

    #[test]
    fn relief_can_flip_two_evenly_matched_materials() {
        // Equal coverage, so the height maps decide — this is the seam, and it
        // is exactly where a normalised splat blend gives 50/50 mud.
        let mut weights = [0.0; N];
        weights[2] = 0.5;
        weights[7] = 0.5;
        let mut heights = [0.0; N];
        heights[2] = 0.1; // rock in a crevice
        heights[7] = 0.9; // gravel piled up
        let out = blend_weights(&weights, &heights, &LAYER_BLENDS);
        assert!(out[7] > out[2], "{out:?}");

        // …and the same pair with the relief reversed swaps the winner, which
        // is what makes the boundary follow the rock rather than the splatmap.
        heights[2] = 0.9;
        heights[7] = 0.1;
        let flipped = blend_weights(&weights, &heights, &LAYER_BLENDS);
        assert!(flipped[2] > flipped[7], "{flipped:?}");
    }

    #[test]
    fn a_layer_below_the_sampling_epsilon_contributes_nothing() {
        // The shader does not sample those layers at all, so their `heights`
        // entry is garbage; the CPU mirror must ignore them the same way.
        let mut weights = [0.0; N];
        weights[0] = 0.999;
        weights[5] = 0.001;
        let mut heights = [0.0; N];
        heights[5] = 1.0;
        let out = blend_weights(&weights, &heights, &LAYER_BLENDS);
        assert_eq!(out[5], 0.0, "{out:?}");
        assert!((out[0] - 1.0).abs() < 1e-5, "{out:?}");
    }

    #[test]
    fn every_authored_layer_is_usable() {
        for (i, p) in LAYER_BLENDS.iter().enumerate() {
            assert!(p.blend_width > 0.0, "layer {i} has no transition band");
            assert!(p.min_weight > 0.0, "layer {i} would divide by zero");
            assert!(
                (0.0..=1.0).contains(&p.height_scale),
                "layer {i} height_scale {} is outside the height map's range",
                p.height_scale
            );
        }
    }
}
