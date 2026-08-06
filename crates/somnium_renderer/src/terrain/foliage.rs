//! Phase 17A: scattering foliage across a heightmap terrain.
//!
//! Placement is a **jittered grid**: the terrain is divided into cells sized so
//! one instance per cell gives the requested density, and each cell contributes
//! a single candidate placed randomly inside it. That is stratified sampling —
//! it gives even coverage without the clumps and bald patches of independent
//! uniform sampling, at a fraction of the cost of true Poisson-disc.
//!
//! Every candidate is derived by hashing its cell coordinate together with the
//! seed, so nothing depends on iteration order or on any RNG state carried
//! between calls. Re-scattering the same terrain always produces the same
//! result, which matters because the instance list is rebuilt whenever the
//! terrain is sculpted and foliage that reshuffled on every edit would be
//! unusable.
//!
//! Candidates are rejected on slope and on the paint layer underneath, so grass
//! follows the layer it was painted onto and stops at cliffs.

use glam::Vec3;

/// What the terrain looks like at one point, in terrain-local space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSample {
    /// Terrain height at the point.
    pub height: f32,
    /// Cosine of the slope angle: `1` is flat ground, `0` a vertical wall.
    /// Stored as a cosine so the slope test is a comparison, not a `acos`.
    pub slope_cos: f32,
    /// Weight of the foliage layer in the splatmap, `0..=1`.
    pub layer_weight: f32,
}

/// Placement rules for one foliage kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageParams {
    /// Candidates per square world unit, before rejection. The number that
    /// survives is lower wherever slope or layer weight rules the ground out.
    pub density: f32,
    /// Seed for the placement hash. Changing it reshuffles the whole layout.
    pub seed: u32,
    /// Reject ground steeper than this, in degrees.
    pub max_slope_deg: f32,
    /// Splatmap layer that grows this foliage.
    pub layer: u8,
    /// Minimum weight of `layer` required, `0..=1`.
    pub min_layer_weight: f32,
    /// Uniform scale is drawn from this range.
    pub scale_min: f32,
    pub scale_max: f32,
    /// Lifts the instance so a mesh whose origin sits at its base is not
    /// half-buried. Applied after the height lookup.
    pub ground_offset: f32,
    /// Hard ceiling on the instance count.
    ///
    /// Reached by **coarsening the grid**, not by stopping partway through it.
    /// Truncating would pile every instance into whichever corner is visited
    /// first and leave the rest of the terrain bare.
    pub max_instances: usize,
}

impl Default for FoliageParams {
    fn default() -> Self {
        Self {
            density: 0.25,
            seed: 1,
            max_slope_deg: 35.0,
            layer: 0,
            min_layer_weight: 0.5,
            scale_min: 0.7,
            scale_max: 1.4,
            ground_offset: 0.0,
            max_instances: 20_000,
        }
    }
}

/// One placed instance, in terrain-local space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageInstance {
    pub position: Vec3,
    /// Rotation about Y, radians. Random so a single mesh does not read as a
    /// grid of clones.
    pub yaw: f32,
    pub scale: f32,
}

/// Scatter foliage across `world_size` (X and Z extents in world units).
///
/// `sample` is called once per surviving grid cell with terrain-local X/Z.
pub fn scatter(
    params: &FoliageParams,
    world_size: [f32; 2],
    sample: impl Fn(f32, f32) -> SurfaceSample,
) -> Vec<FoliageInstance> {
    if params.density <= 0.0
        || params.max_instances == 0
        || world_size[0] <= 0.0
        || world_size[1] <= 0.0
    {
        return Vec::new();
    }

    // One candidate per cell, so cell area = 1 / density.
    let mut cell = (1.0 / params.density).sqrt();

    // Coarsen until the grid fits under the cap. Growing the cell keeps the
    // distribution uniform, where stopping partway through the grid would not.
    // The closed form gets close in one step; the loop then trims the overshoot
    // from rounding each axis up, which alone can cost a whole extra row and
    // column (a 500-instance budget otherwise lands on 23x23 = 529 cells).
    let area = world_size[0] * world_size[1];
    let cap_cell = (area / params.max_instances as f32).sqrt();
    if cap_cell > cell {
        cell = cap_cell;
    }
    let (cells_x, cells_z) = loop {
        let cx = (world_size[0] / cell).ceil().max(1.0) as u64;
        let cz = (world_size[1] / cell).ceil().max(1.0) as u64;
        if cx.saturating_mul(cz) <= params.max_instances as u64 {
            break (cx as u32, cz as u32);
        }
        cell *= 1.05;
    };

    let slope_limit = params.max_slope_deg.clamp(0.0, 90.0).to_radians().cos();
    let scale_lo = params.scale_min.min(params.scale_max);
    let scale_hi = params.scale_min.max(params.scale_max);

    let mut out = Vec::new();
    for cz in 0..cells_z {
        for cx in 0..cells_x {
            // Four independent values from one cell, by salting the hash.
            let h = hash3(cx, cz, params.seed);
            let jx = unit_from(h);
            let jz = unit_from(hash3(cx, cz, params.seed ^ 0x9E37_79B9));
            let jy = unit_from(hash3(cx, cz, params.seed ^ 0x85EB_CA6B));
            let js = unit_from(hash3(cx, cz, params.seed ^ 0xC2B2_AE35));

            let x = (cx as f32 + jx) * cell;
            let z = (cz as f32 + jz) * cell;
            // The last row and column of cells can overhang the terrain when
            // the world size is not a whole number of cells.
            if x > world_size[0] || z > world_size[1] {
                continue;
            }

            let s = sample(x, z);
            // `slope_cos` falls as the ground steepens, so the test is `<`.
            if s.slope_cos < slope_limit || s.layer_weight < params.min_layer_weight {
                continue;
            }

            out.push(FoliageInstance {
                position: Vec3::new(x, s.height + params.ground_offset, z),
                yaw: jy * std::f32::consts::TAU,
                scale: scale_lo + js * (scale_hi - scale_lo),
            });
        }
    }
    out
}

/// Integer hash of a cell coordinate and seed. Based on the finalizer from
/// MurmurHash3, which mixes well enough that the four salted draws per cell are
/// visually independent.
fn hash3(x: u32, z: u32, seed: u32) -> u32 {
    let mut h = x
        .wrapping_mul(0x8DA6_B343)
        ^ z.wrapping_mul(0xD824_2BA5)
        ^ seed.wrapping_mul(0xF950_9C21);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h
}

/// Map a hash to `[0, 1)`.
fn unit_from(h: u32) -> f32 {
    // 24 bits keeps the result exactly representable in f32.
    (h >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat, fully-planted ground: nothing is ever rejected.
    fn open_ground(_x: f32, _z: f32) -> SurfaceSample {
        SurfaceSample { height: 0.0, slope_cos: 1.0, layer_weight: 1.0 }
    }

    fn params() -> FoliageParams {
        FoliageParams { density: 1.0, seed: 7, ..Default::default() }
    }

    #[test]
    fn the_same_seed_always_produces_the_same_layout() {
        // The instance list is rebuilt on every terrain edit. If placement
        // shifted each time, sculpting would make the foliage crawl.
        let a = scatter(&params(), [40.0, 40.0], open_ground);
        let b = scatter(&params(), [40.0, 40.0], open_ground);
        assert!(!a.is_empty());
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_seed_produces_a_different_layout() {
        let a = scatter(&params(), [40.0, 40.0], open_ground);
        let b = scatter(&FoliageParams { seed: 8, ..params() }, [40.0, 40.0], open_ground);
        assert_eq!(a.len(), b.len(), "seed must not change the cell count");
        assert_ne!(a, b);
    }

    #[test]
    fn density_drives_the_instance_count() {
        let sparse = scatter(&FoliageParams { density: 0.25, ..params() }, [40.0, 40.0], open_ground);
        let dense  = scatter(&FoliageParams { density: 4.0,  ..params() }, [40.0, 40.0], open_ground);
        // 16x the density over the same area, so roughly 16x the instances.
        let ratio = dense.len() as f32 / sparse.len() as f32;
        assert!((10.0..24.0).contains(&ratio), "ratio {ratio} from {} and {}", sparse.len(), dense.len());
    }

    #[test]
    fn steep_ground_grows_nothing() {
        let steep = |_x: f32, _z: f32| SurfaceSample {
            height: 0.0,
            slope_cos: 0.2, // ~78 degrees
            layer_weight: 1.0,
        };
        assert!(scatter(&params(), [40.0, 40.0], steep).is_empty());
    }

    #[test]
    fn the_slope_limit_is_honoured_exactly_at_the_boundary() {
        let p = FoliageParams { max_slope_deg: 45.0, ..params() };
        let just_under = |_x: f32, _z: f32| SurfaceSample {
            height: 0.0,
            slope_cos: 44.0f32.to_radians().cos(), // shallower than the limit
            layer_weight: 1.0,
        };
        let just_over = |_x: f32, _z: f32| SurfaceSample {
            height: 0.0,
            slope_cos: 46.0f32.to_radians().cos(), // steeper
            layer_weight: 1.0,
        };
        assert!(!scatter(&p, [40.0, 40.0], just_under).is_empty());
        assert!(scatter(&p, [40.0, 40.0], just_over).is_empty());
    }

    #[test]
    fn foliage_only_grows_on_its_own_paint_layer() {
        let unpainted = |_x: f32, _z: f32| SurfaceSample {
            height: 0.0,
            slope_cos: 1.0,
            layer_weight: 0.1,
        };
        let p = FoliageParams { min_layer_weight: 0.5, ..params() };
        assert!(scatter(&p, [40.0, 40.0], unpainted).is_empty());
    }

    #[test]
    fn instances_sit_on_the_ground_plus_the_offset() {
        let hilly = |x: f32, z: f32| SurfaceSample {
            height: x * 0.1 + z * 0.2,
            slope_cos: 1.0,
            layer_weight: 1.0,
        };
        let p = FoliageParams { ground_offset: 0.25, ..params() };
        for i in scatter(&p, [40.0, 40.0], hilly) {
            let expected = i.position.x * 0.1 + i.position.z * 0.2 + 0.25;
            assert!((i.position.y - expected).abs() < 1e-3);
        }
    }

    #[test]
    fn instances_stay_inside_the_terrain() {
        for i in scatter(&params(), [40.0, 25.0], open_ground) {
            assert!((0.0..=40.0).contains(&i.position.x), "x {}", i.position.x);
            assert!((0.0..=25.0).contains(&i.position.z), "z {}", i.position.z);
        }
    }

    #[test]
    fn scale_and_yaw_stay_in_range() {
        let p = FoliageParams { scale_min: 0.5, scale_max: 2.0, ..params() };
        let out = scatter(&p, [40.0, 40.0], open_ground);
        assert!(out.len() > 100, "need a decent sample");
        for i in &out {
            assert!((0.5..=2.0).contains(&i.scale), "scale {}", i.scale);
            assert!((0.0..std::f32::consts::TAU).contains(&i.yaw), "yaw {}", i.yaw);
        }
        // A single fixed yaw would make the field read as cloned billboards.
        let distinct = out.windows(2).filter(|w| w[0].yaw != w[1].yaw).count();
        assert!(distinct > out.len() / 2, "yaw is not varying");
    }

    #[test]
    fn an_inverted_scale_range_is_accepted() {
        let p = FoliageParams { scale_min: 2.0, scale_max: 0.5, ..params() };
        for i in scatter(&p, [40.0, 40.0], open_ground) {
            assert!((0.5..=2.0).contains(&i.scale), "scale {}", i.scale);
        }
    }

    #[test]
    fn the_cap_coarsens_the_grid_rather_than_truncating_it() {
        let p = FoliageParams { density: 100.0, max_instances: 500, ..params() };
        let out = scatter(&p, [100.0, 100.0], open_ground);
        assert!(out.len() <= 500, "cap exceeded: {}", out.len());
        assert!(out.len() > 250, "cap wasted most of its budget: {}", out.len());
        // Truncation would fill one edge and leave the far side empty, so check
        // the far quadrant is populated too.
        let far = out.iter().filter(|i| i.position.x > 50.0 && i.position.z > 50.0).count();
        assert!(far > out.len() / 8, "far quadrant nearly empty: {far} of {}", out.len());
    }

    #[test]
    fn degenerate_inputs_produce_nothing_instead_of_panicking() {
        assert!(scatter(&FoliageParams { density: 0.0, ..params() }, [40.0, 40.0], open_ground).is_empty());
        assert!(scatter(&FoliageParams { density: -1.0, ..params() }, [40.0, 40.0], open_ground).is_empty());
        assert!(scatter(&params(), [0.0, 40.0], open_ground).is_empty());
        assert!(scatter(&params(), [40.0, -5.0], open_ground).is_empty());
        assert!(scatter(&FoliageParams { max_instances: 0, ..params() }, [40.0, 40.0], open_ground).is_empty());
    }

    #[test]
    fn coverage_is_even_across_the_terrain() {
        // Stratification is the whole reason for the jittered grid: independent
        // uniform sampling leaves visible clumps and bald patches.
        let out = scatter(&FoliageParams { density: 1.0, ..params() }, [40.0, 40.0], open_ground);
        let mut quadrants = [0usize; 4];
        for i in &out {
            let q = (i.position.x > 20.0) as usize + 2 * (i.position.z > 20.0) as usize;
            quadrants[q] += 1;
        }
        let expected = out.len() as f32 / 4.0;
        for (q, n) in quadrants.iter().enumerate() {
            let ratio = *n as f32 / expected;
            assert!((0.8..1.2).contains(&ratio), "quadrant {q} holds {n}, expected ~{expected}");
        }
    }
}
