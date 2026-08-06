//! Phase 17F: painting foliage by hand instead of scattering it procedurally.
//!
//! Phase 17A filled the whole terrain the moment foliage was switched on, which
//! is the wrong model for authoring: a level artist wants to put grass in the
//! meadow and trees on the ridge, not everywhere at once. This module holds the
//! painted set and the brush that edits it.
//!
//! ## Spacing, not density-per-frame
//!
//! A brush stroke fires many times per second over the same ground, so "add N
//! per dab" would pile thousands of instances on one spot. Instead each
//! candidate must clear a **minimum spacing** from every instance already
//! painted. Painting over dense ground is then a no-op, and a stroke naturally
//! converges on the requested density rather than growing without bound.
//!
//! Spacing is derived from density so the two cannot disagree: at `d` instances
//! per square metre, the mean spacing of a packed layout is `1/sqrt(d)`.
//!
//! ## Determinism
//!
//! Candidate offsets, yaw and scale come from a counter-seeded hash rather than
//! a live RNG, so a recorded sequence of strokes replays identically — which is
//! what undo and scene reload need.

use glam::Vec3;

/// One painted instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintedFoliage {
    /// Index into the foliage palette — which mesh this is.
    pub kind: u8,
    /// Terrain-local position, already on the ground.
    pub position: Vec3,
    /// Rotation about Y, radians.
    pub yaw: f32,
    pub scale: f32,
}

/// Brush settings for a paint stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageBrush {
    /// Palette entry being painted.
    pub kind: u8,
    /// Brush radius in world units.
    pub radius: f32,
    /// Target instances per square metre inside the brush.
    pub density: f32,
    /// Place exactly one instance at the brush centre, ignoring density.
    /// This is how trees get placed — one at a time, where you point.
    pub single: bool,
    pub scale_min: f32,
    pub scale_max: f32,
    /// Reject ground steeper than this, in degrees.
    pub max_slope_deg: f32,
}

impl Default for FoliageBrush {
    fn default() -> Self {
        Self {
            kind: 0,
            radius: 6.0,
            density: 2.0,
            single: false,
            scale_min: 0.8,
            scale_max: 1.3,
            max_slope_deg: 40.0,
        }
    }
}

/// Ground query at a point, in terrain-local space.
#[derive(Debug, Clone, Copy)]
pub struct GroundSample {
    pub height: f32,
    /// Cosine of the slope: 1 is flat, 0 vertical.
    pub slope_cos: f32,
}

/// Minimum spacing implied by a density, in world units.
///
/// A packed layout at `d` per square metre averages `1/sqrt(d)` apart. Using
/// slightly less than that leaves room for the jitter to look natural rather
/// than gridded.
pub fn spacing_for_density(density: f32) -> f32 {
    if density <= 0.0 {
        return f32::MAX;
    }
    (1.0 / density).sqrt() * 0.85
}

/// Add instances under the brush, returning how many were placed.
///
/// `stroke_seed` should advance between dabs so a held brush keeps producing
/// new candidates rather than retrying the same rejected points.
pub fn paint(
    out: &mut Vec<PaintedFoliage>,
    brush: &FoliageBrush,
    center: [f32; 2],
    stroke_seed: u32,
    sample: impl Fn(f32, f32) -> GroundSample,
) -> usize {
    let slope_limit = brush.max_slope_deg.clamp(0.0, 90.0).to_radians().cos();
    let scale_lo = brush.scale_min.min(brush.scale_max).max(0.01);
    let scale_hi = brush.scale_min.max(brush.scale_max).max(0.01);
    let before = out.len();

    let place = |x: f32, z: f32, salt: u32, out: &mut Vec<PaintedFoliage>| {
        let g = sample(x, z);
        if g.slope_cos < slope_limit || !g.height.is_finite() {
            return;
        }
        let spacing = if brush.single {
            // A tree still should not land inside another tree, but it must not
            // be blocked by the grass around it either.
            0.5 * scale_lo
        } else {
            spacing_for_density(brush.density)
        };
        let sp_sq = spacing * spacing;
        if out.iter().any(|p| {
            p.kind == brush.kind
                && (p.position.x - x).powi(2) + (p.position.z - z).powi(2) < sp_sq
        }) {
            return;
        }
        let jy = unit_from(hash2(salt, 0x51_7C_C1_B7));
        let js = unit_from(hash2(salt, 0x27_22_0A_95));
        out.push(PaintedFoliage {
            kind: brush.kind,
            position: Vec3::new(x, g.height, z),
            yaw: jy * std::f32::consts::TAU,
            scale: scale_lo + js * (scale_hi - scale_lo),
        });
    };

    if brush.single {
        place(center[0], center[1], stroke_seed, out);
        return out.len() - before;
    }

    if brush.radius <= 0.0 || brush.density <= 0.0 {
        return 0;
    }

    // Try a number of candidates proportional to the brush area at the target
    // density. Rejection by spacing means a dab over covered ground places
    // nothing, so a held brush settles instead of stacking up.
    let area = std::f32::consts::PI * brush.radius * brush.radius;
    let attempts = ((area * brush.density).ceil() as u32).clamp(1, 4096);

    for i in 0..attempts {
        let salt = stroke_seed.wrapping_mul(0x9E37_79B9).wrapping_add(i);
        // Uniform over the disc: sqrt on the radius, or candidates bunch in the
        // middle and the brush paints a hot spot.
        let r = brush.radius * unit_from(hash2(salt, 0x85EB_CA6B)).sqrt();
        let a = unit_from(hash2(salt, 0xC2B2_AE35)) * std::f32::consts::TAU;
        place(center[0] + r * a.cos(), center[1] + r * a.sin(), salt, out);
    }
    out.len() - before
}

/// Remove instances within `radius` of `center`, returning how many went.
///
/// `kind` limits the erase to one palette entry; `None` erases everything,
/// which is what a plain erase stroke should do.
pub fn erase(
    out: &mut Vec<PaintedFoliage>,
    center: [f32; 2],
    radius: f32,
    kind: Option<u8>,
) -> usize {
    if radius <= 0.0 {
        return 0;
    }
    let r_sq = radius * radius;
    let before = out.len();
    out.retain(|p| {
        let inside = (p.position.x - center[0]).powi(2) + (p.position.z - center[1]).powi(2) <= r_sq;
        let matches = kind.is_none_or(|k| k == p.kind);
        !(inside && matches)
    });
    before - out.len()
}

fn hash2(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x8DA6_B343) ^ b.wrapping_mul(0xD824_2BA5);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h
}

fn unit_from(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_x: f32, _z: f32) -> GroundSample {
        GroundSample { height: 0.0, slope_cos: 1.0 }
    }

    fn brush() -> FoliageBrush {
        FoliageBrush { radius: 5.0, density: 2.0, ..Default::default() }
    }

    #[test]
    fn a_dab_places_instances_inside_the_brush() {
        let mut v = Vec::new();
        let n = paint(&mut v, &brush(), [10.0, 10.0], 1, flat);
        assert!(n > 0);
        assert_eq!(v.len(), n);
        for p in &v {
            let d = ((p.position.x - 10.0).powi(2) + (p.position.z - 10.0).powi(2)).sqrt();
            assert!(d <= 5.0 + 1e-3, "instance {d} outside a radius-5 brush");
        }
    }

    #[test]
    fn holding_the_brush_converges_instead_of_piling_up() {
        // The real failure mode: a stroke fires every frame, so without spacing
        // rejection one spot would accumulate thousands of instances.
        let mut v = Vec::new();
        for seed in 0..40 {
            paint(&mut v, &brush(), [10.0, 10.0], seed, flat);
        }
        let area = std::f32::consts::PI * 25.0;
        let spacing = spacing_for_density(2.0);
        let ceiling = (area / (spacing * spacing)) as usize + 20;
        assert!(v.len() < ceiling, "{} instances, expected under {ceiling}", v.len());
        assert!(v.len() > 20, "40 dabs only placed {}", v.len());
    }

    #[test]
    fn instances_keep_their_spacing() {
        let mut v = Vec::new();
        for seed in 0..20 {
            paint(&mut v, &brush(), [10.0, 10.0], seed, flat);
        }
        let min = spacing_for_density(2.0);
        for (i, a) in v.iter().enumerate() {
            for b in &v[i + 1..] {
                let d = ((a.position.x - b.position.x).powi(2)
                    + (a.position.z - b.position.z).powi(2))
                .sqrt();
                assert!(d >= min - 1e-3, "instances {d} apart, minimum {min}");
            }
        }
    }

    #[test]
    fn higher_density_packs_more_in() {
        let mut sparse = Vec::new();
        let mut dense = Vec::new();
        for seed in 0..25 {
            paint(&mut sparse, &FoliageBrush { density: 0.5, ..brush() }, [0.0, 0.0], seed, flat);
            paint(&mut dense, &FoliageBrush { density: 8.0, ..brush() }, [0.0, 0.0], seed, flat);
        }
        assert!(dense.len() > sparse.len() * 3, "{} vs {}", dense.len(), sparse.len());
    }

    #[test]
    fn single_mode_places_exactly_one_at_the_cursor() {
        // How trees get placed: one per click, where you point, not a scatter.
        let mut v = Vec::new();
        let b = FoliageBrush { single: true, ..brush() };
        let n = paint(&mut v, &b, [3.0, -4.0], 1, flat);
        assert_eq!(n, 1);
        assert_eq!(v[0].position.x, 3.0);
        assert_eq!(v[0].position.z, -4.0);
    }

    #[test]
    fn single_mode_does_not_stack_trees_on_one_spot() {
        let mut v = Vec::new();
        let b = FoliageBrush { single: true, ..brush() };
        for seed in 0..10 {
            paint(&mut v, &b, [3.0, -4.0], seed, flat);
        }
        assert_eq!(v.len(), 1, "a repeated click stacked {} trees", v.len());
    }

    #[test]
    fn different_kinds_do_not_block_each_other() {
        // Grass must be paintable under a tree, so spacing is per palette entry.
        let mut v = Vec::new();
        paint(&mut v, &FoliageBrush { kind: 0, single: true, ..brush() }, [0.0, 0.0], 1, flat);
        let n = paint(&mut v, &FoliageBrush { kind: 1, single: true, ..brush() }, [0.0, 0.0], 2, flat);
        assert_eq!(n, 1, "a different kind was blocked by an existing instance");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn steep_ground_rejects_the_brush() {
        let cliff = |_x: f32, _z: f32| GroundSample { height: 0.0, slope_cos: 0.1 };
        let mut v = Vec::new();
        assert_eq!(paint(&mut v, &brush(), [0.0, 0.0], 1, cliff), 0);
    }

    #[test]
    fn instances_sit_on_the_ground() {
        let hill = |x: f32, z: f32| GroundSample { height: x * 0.5 + z, slope_cos: 1.0 };
        let mut v = Vec::new();
        paint(&mut v, &brush(), [10.0, 10.0], 1, hill);
        for p in &v {
            assert!((p.position.y - (p.position.x * 0.5 + p.position.z)).abs() < 1e-3);
        }
    }

    #[test]
    fn candidates_spread_over_the_disc_rather_than_bunching_in_the_middle() {
        // Sampling radius uniformly instead of by its square root concentrates
        // points at the centre and the brush paints a hot spot.
        let mut v = Vec::new();
        for seed in 0..30 {
            paint(&mut v, &FoliageBrush { density: 4.0, ..brush() }, [0.0, 0.0], seed, flat);
        }
        let inner = v.iter().filter(|p| p.position.length() < 2.5).count();
        // The inner half-radius disc is a quarter of the area, so it should hold
        // roughly a quarter of the instances.
        let frac = inner as f32 / v.len() as f32;
        assert!((0.15..0.35).contains(&frac), "inner disc holds {frac:.2} of instances");
    }

    #[test]
    fn erase_removes_only_what_is_under_the_brush() {
        let mut v = Vec::new();
        paint(&mut v, &brush(), [0.0, 0.0], 1, flat);
        paint(&mut v, &brush(), [40.0, 0.0], 2, flat);
        let total = v.len();
        let removed = erase(&mut v, [0.0, 0.0], 5.0, None);
        assert!(removed > 0);
        assert_eq!(v.len(), total - removed);
        for p in &v {
            assert!(p.position.x > 20.0, "an instance near the origin survived");
        }
    }

    #[test]
    fn erase_can_target_one_palette_entry() {
        let mut v = Vec::new();
        paint(&mut v, &FoliageBrush { kind: 0, ..brush() }, [0.0, 0.0], 1, flat);
        paint(&mut v, &FoliageBrush { kind: 1, single: true, ..brush() }, [0.0, 0.0], 2, flat);
        let removed = erase(&mut v, [0.0, 0.0], 5.0, Some(1));
        assert_eq!(removed, 1);
        assert!(v.iter().all(|p| p.kind == 0), "erasing kind 1 took kind 0 with it");
    }

    #[test]
    fn a_stroke_replays_identically() {
        // Undo and scene reload both depend on this.
        let mut a = Vec::new();
        let mut b = Vec::new();
        for seed in 0..10 {
            paint(&mut a, &brush(), [5.0, 5.0], seed, flat);
            paint(&mut b, &brush(), [5.0, 5.0], seed, flat);
        }
        assert_eq!(a, b);
    }

    #[test]
    fn degenerate_brushes_do_nothing_instead_of_panicking() {
        let mut v = Vec::new();
        assert_eq!(paint(&mut v, &FoliageBrush { radius: 0.0, ..brush() }, [0.0, 0.0], 1, flat), 0);
        assert_eq!(paint(&mut v, &FoliageBrush { density: 0.0, ..brush() }, [0.0, 0.0], 1, flat), 0);
        assert_eq!(erase(&mut v, [0.0, 0.0], 0.0, None), 0);
        assert_eq!(spacing_for_density(0.0), f32::MAX);
    }
}
