//! Phase 17B: turning a heightmap into a physics heightfield.
//!
//! Jolt's `HeightFieldShape` wants a square grid of `N x N` samples where `N` is
//! a power of two. A Somnium terrain is `chunks * cells_per_chunk + 1` vertices
//! per side — 513 for the default 16x32 layout — so the heightmap is resampled
//! onto a power-of-two grid rather than handed over directly.
//!
//! Resampling loses a little fidelity, which is the right trade here: the
//! collider only has to feel like the visible ground, and a 512-sample field
//! over a 1 km terrain still resolves every 2 m. Going the other way — padding
//! up to 1024 — would quadruple the memory for detail nothing can feel.

use glam::Vec3;

/// Largest heightfield side Jolt is asked for.
///
/// 512x512 is 262 144 samples, one megabyte as `f32`. Past that the build cost
/// of the shape starts to show on every terrain edit, and the extra resolution
/// is far below what a rigid body can notice.
pub const MAX_HEIGHTFIELD_SAMPLES: u32 = 512;

/// Choose the heightfield resolution for a terrain with `vertices_per_side`.
///
/// Rounds **down** to a power of two so the collider is never finer than the
/// mesh it approximates, and clamps to [`MAX_HEIGHTFIELD_SAMPLES`]. Jolt
/// requires at least 2.
pub fn sample_count_for(vertices_per_side: u32) -> u32 {
    if vertices_per_side < 2 {
        return 2;
    }
    let capped = vertices_per_side.min(MAX_HEIGHTFIELD_SAMPLES);
    // Largest power of two <= capped.
    let pow2 = 1u32 << (31 - capped.leading_zeros());
    pow2.max(2)
}

/// World units between adjacent samples, and the vertical scale.
///
/// Jolt maps sample `(x, z)` to `offset + scale * (x, height, z)`, so the
/// horizontal scale is the spacing and the vertical scale stays 1 — the samples
/// are already in world units.
pub fn heightfield_scale(world_size: [f32; 2], sample_count: u32) -> Vec3 {
    let n = sample_count.max(2) as f32;
    // `n - 1` spans, not `n`: the last sample sits on the far edge.
    Vec3::new(world_size[0] / (n - 1.0), 1.0, world_size[1] / (n - 1.0))
}

/// Resample a terrain onto a square grid, row-major with X varying fastest.
///
/// `height_at` takes terrain-local X and Z in world units.
pub fn resample(
    sample_count: u32,
    world_size: [f32; 2],
    height_at: impl Fn(f32, f32) -> f32,
) -> Vec<f32> {
    let n = sample_count.max(2);
    let scale = heightfield_scale(world_size, n);
    let mut out = Vec::with_capacity((n * n) as usize);
    for z in 0..n {
        for x in 0..n {
            let h = height_at(x as f32 * scale.x, z as f32 * scale.z);
            // A non-finite sample would poison Jolt's tree build. Treat it as
            // ground level rather than letting it through.
            out.push(if h.is_finite() { h } else { 0.0 });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_terrain_resolves_to_a_power_of_two() {
        // 16 chunks x 32 cells + 1 = 513 vertices per side.
        let n = sample_count_for(513);
        assert_eq!(n, 512);
        assert!(n.is_power_of_two());
    }

    #[test]
    fn the_resolution_never_exceeds_the_mesh_it_approximates() {
        // Rounding up would invent detail the heightmap does not have.
        assert_eq!(sample_count_for(100), 64);
        assert_eq!(sample_count_for(255), 128);
        assert_eq!(sample_count_for(256), 256);
    }

    #[test]
    fn the_resolution_is_capped() {
        assert_eq!(sample_count_for(4096), MAX_HEIGHTFIELD_SAMPLES);
        assert!(sample_count_for(100_000) <= MAX_HEIGHTFIELD_SAMPLES);
    }

    #[test]
    fn tiny_terrains_still_produce_a_legal_field() {
        // Jolt rejects anything below 2 samples per side.
        for v in [0, 1, 2, 3] {
            let n = sample_count_for(v);
            assert!(n >= 2, "{v} vertices gave {n} samples");
            assert!(n.is_power_of_two());
        }
    }

    #[test]
    fn the_grid_spans_the_whole_terrain() {
        let n = 512;
        let scale = heightfield_scale([1024.0, 1024.0], n);
        // The last sample must land exactly on the far edge, or the collider
        // stops short of the visible ground.
        assert!((scale.x * (n - 1) as f32 - 1024.0).abs() < 1e-3);
        assert_eq!(scale.y, 1.0, "samples are already in world units");
    }

    #[test]
    fn a_non_square_terrain_gets_non_square_spacing() {
        let scale = heightfield_scale([1024.0, 512.0], 256);
        assert!((scale.x - scale.z * 2.0).abs() < 1e-3);
    }

    #[test]
    fn resampling_produces_a_square_row_major_grid() {
        let out = resample(8, [16.0, 16.0], |x, z| x + z * 100.0);
        assert_eq!(out.len(), 64);
        // Row-major with X varying fastest: index 1 is one step in X.
        let step = 16.0 / 7.0;
        assert!((out[1] - step).abs() < 1e-3, "{}", out[1]);
        assert!((out[8] - step * 100.0).abs() < 1e-2, "{}", out[8]);
    }

    #[test]
    fn heights_are_sampled_in_world_units_not_indices() {
        // A constant-gradient terrain: the corner sample must equal the terrain
        // size, not the sample count.
        let out = resample(16, [64.0, 64.0], |x, _z| x);
        assert!((out[15] - 64.0).abs() < 1e-3, "corner height {}", out[15]);
    }

    #[test]
    fn non_finite_heights_are_replaced_rather_than_passed_to_jolt() {
        // A NaN would poison the shape's tree build and take the whole physics
        // system with it.
        let out = resample(4, [8.0, 8.0], |x, _z| if x > 0.0 { f32::NAN } else { 1.0 });
        assert!(out.iter().all(|h| h.is_finite()));
    }
}
