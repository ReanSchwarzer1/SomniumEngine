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
    /// Lean away from vertical, radians, applied **after** `yaw` (Phase
    /// TSUSHIMA-I).
    ///
    /// One float rather than two, because `Ry(yaw) · Rx(tilt)` with a uniform
    /// yaw already leans in a uniformly random horizontal direction — the yaw
    /// that stops a mesh reading as a grid of clones is the same yaw that picks
    /// which way the lean points, and there is nothing left for a second angle
    /// to say.
    ///
    /// Zero for anything that grows: grass and trees are upright because they
    /// grew toward the light. A pebble has no such excuse, and a scatter of
    /// pebbles all sitting perfectly flat reads as placed rather than fallen —
    /// which is the whole failure this exists to fix.
    pub tilt: f32,
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
    /// Terrain layer this brush wants underneath it (Phase TSUSHIMA-I).
    pub layer: u8,
    /// Reject ground where `layer` is painted more weakly than this.
    ///
    /// **0 disables the test entirely**, which is what every pre-TSUSHIMA
    /// brush gets, so nothing that used to place now refuses to.
    ///
    /// This is the funnel's missing rejection. Slope keeps grass off a cliff
    /// and radius keeps it under the cursor, but nothing until now asked what
    /// the ground was *made of* — and it is the whole question for debris:
    /// pebbles belong on scree and gravel and nowhere else, and a scatter that
    /// ignores the splat puts them in the middle of a painted lawn.
    pub min_layer_weight: f32,
    /// Largest lean from vertical, in degrees. 0 keeps instances upright.
    pub max_tilt_deg: f32,
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
            layer: 0,
            // Both off by default: the defaults are the pre-TSUSHIMA brush
            // exactly, and the palette turns them on for the entries that want
            // them.
            min_layer_weight: 0.0,
            max_tilt_deg: 0.0,
        }
    }
}

/// Ground query at a point, in terrain-local space.
#[derive(Debug, Clone, Copy)]
pub struct GroundSample {
    pub height: f32,
    /// Cosine of the slope: 1 is flat, 0 vertical.
    pub slope_cos: f32,
    /// Splat weight of the brush's own layer here, `0..=1` (Phase TSUSHIMA-I).
    ///
    /// The caller decides which layer that is, because the caller is the one
    /// holding the brush. `TerrainData::surface_sample` has computed this all
    /// along and `ground_sample` was dropping it on the floor.
    pub layer_weight: f32,
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

/// What one dab did, and — when it did nothing — why.
///
/// # Why a report and not a count
///
/// `paint` returned `usize`, and the caller only spoke when it was non-zero.
/// So a brush that placed nothing was **completely silent**, and thirteen of
/// the twenty-five palette entries can place nothing for a reason no one can
/// see: they carry a `min_layer_weight` against a splat layer that has to be
/// painted on the ground first. Pebbles want gravel, moss wants mossy rock,
/// nettles want mud. Point one of them at a default grass terrain and the
/// cursor is over ground the brush will always refuse — and the editor's whole
/// answer was nothing at all.
///
/// The rejection is right. Silence about it is not, so the counts come back and
/// the caller can name the reason.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PaintReport {
    /// Instances actually added.
    pub placed: usize,
    /// Candidates on ground steeper than `max_slope_deg`.
    pub too_steep: usize,
    /// Candidates where `layer` was painted more weakly than
    /// `min_layer_weight`.
    pub wrong_layer: usize,
    /// Candidates rejected because an instance of the same kind was already
    /// within the spacing implied by density. This one is *success* for a held
    /// brush: it is how a stroke settles instead of piling up.
    pub too_close: usize,
    /// The strongest `layer` weight seen under any candidate this dab.
    ///
    /// The number a message needs: "wants 0.50, the ground here has 0.03" says
    /// what to do, where "nothing was placed" does not.
    pub best_layer_weight: f32,
}

impl PaintReport {
    /// A dab that placed nothing and was not merely already full.
    ///
    /// `too_close` is excluded deliberately — painting over ground that is
    /// already covered is the brush working, not failing.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.placed == 0 && (self.too_steep > 0 || self.wrong_layer > 0)
    }

    /// Whether the layer test is what stopped this dab.
    #[must_use]
    pub fn blocked_by_layer(&self) -> bool {
        self.refused() && self.wrong_layer >= self.too_steep
    }
}

/// Add instances under the brush, reporting what happened.
///
/// `stroke_seed` should advance between dabs so a held brush keeps producing
/// new candidates rather than retrying the same rejected points.
pub fn paint(
    out: &mut Vec<PaintedFoliage>,
    brush: &FoliageBrush,
    center: [f32; 2],
    stroke_seed: u32,
    sample: impl Fn(f32, f32) -> GroundSample,
) -> PaintReport {
    let slope_limit = brush.max_slope_deg.clamp(0.0, 90.0).to_radians().cos();
    let scale_lo = brush.scale_min.min(brush.scale_max).max(0.01);
    let scale_hi = brush.scale_min.max(brush.scale_max).max(0.01);
    let before = out.len();

    let tilt_limit = brush.max_tilt_deg.clamp(0.0, 90.0).to_radians();
    let mut report = PaintReport::default();
    let place = |x: f32,
                 z: f32,
                 salt: u32,
                 out: &mut Vec<PaintedFoliage>,
                 report: &mut PaintReport| {
        let g = sample(x, z);
        report.best_layer_weight = report.best_layer_weight.max(g.layer_weight);
        if g.slope_cos < slope_limit || !g.height.is_finite() {
            report.too_steep += 1;
            return;
        }
        // A hard threshold, not a probability. A probability would scatter a
        // thinning fringe of pebbles out across the grass, and the thing that
        // makes a gravel patch read as gravel is that it *stops*.
        if brush.min_layer_weight > 0.0 && g.layer_weight < brush.min_layer_weight {
            report.wrong_layer += 1;
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
            p.kind == brush.kind && (p.position.x - x).powi(2) + (p.position.z - z).powi(2) < sp_sq
        }) {
            report.too_close += 1;
            return;
        }
        let jy = unit_from(hash2(salt, 0x51_7C_C1_B7));
        let js = unit_from(hash2(salt, 0x27_22_0A_95));
        // `sqrt`, for the same reason the candidate radius above takes one: a
        // uniform angle puts as many instances near flat as near the limit, and
        // what a pile of debris actually looks like is mostly-settled with a
        // few propped up. The third salt keeps tilt independent of yaw and
        // scale, so raising the limit does not also reshuffle the field.
        let jt = unit_from(hash2(salt, 0x9E_37_79_B1)).sqrt();
        out.push(PaintedFoliage {
            kind: brush.kind,
            position: Vec3::new(x, g.height, z),
            yaw: jy * std::f32::consts::TAU,
            tilt: jt * tilt_limit,
            scale: scale_lo + js * (scale_hi - scale_lo),
        });
    };

    if brush.single {
        place(center[0], center[1], stroke_seed, out, &mut report);
        report.placed = out.len() - before;
        return report;
    }

    if brush.radius <= 0.0 || brush.density <= 0.0 {
        return report;
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
        place(
            center[0] + r * a.cos(),
            center[1] + r * a.sin(),
            salt,
            out,
            &mut report,
        );
    }
    report.placed = out.len() - before;
    report
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
        let inside =
            (p.position.x - center[0]).powi(2) + (p.position.z - center[1]).powi(2) <= r_sq;
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
        GroundSample {
            height: 0.0,
            slope_cos: 1.0,
            layer_weight: 1.0,
        }
    }

    fn brush() -> FoliageBrush {
        FoliageBrush {
            radius: 5.0,
            density: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_dab_places_instances_inside_the_brush() {
        let mut v = Vec::new();
        let n = paint(&mut v, &brush(), [10.0, 10.0], 1, flat);
        assert!(n.placed > 0);
        assert_eq!(v.len(), n.placed);
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
        assert!(
            v.len() < ceiling,
            "{} instances, expected under {ceiling}",
            v.len()
        );
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
            paint(
                &mut sparse,
                &FoliageBrush {
                    density: 0.5,
                    ..brush()
                },
                [0.0, 0.0],
                seed,
                flat,
            );
            paint(
                &mut dense,
                &FoliageBrush {
                    density: 8.0,
                    ..brush()
                },
                [0.0, 0.0],
                seed,
                flat,
            );
        }
        assert!(
            dense.len() > sparse.len() * 3,
            "{} vs {}",
            dense.len(),
            sparse.len()
        );
    }

    #[test]
    fn single_mode_places_exactly_one_at_the_cursor() {
        // How trees get placed: one per click, where you point, not a scatter.
        let mut v = Vec::new();
        let b = FoliageBrush {
            single: true,
            ..brush()
        };
        let n = paint(&mut v, &b, [3.0, -4.0], 1, flat);
        assert_eq!(n.placed, 1);
        assert_eq!(v[0].position.x, 3.0);
        assert_eq!(v[0].position.z, -4.0);
    }

    #[test]
    fn single_mode_does_not_stack_trees_on_one_spot() {
        let mut v = Vec::new();
        let b = FoliageBrush {
            single: true,
            ..brush()
        };
        for seed in 0..10 {
            paint(&mut v, &b, [3.0, -4.0], seed, flat);
        }
        assert_eq!(v.len(), 1, "a repeated click stacked {} trees", v.len());
    }

    #[test]
    fn different_kinds_do_not_block_each_other() {
        // Grass must be paintable under a tree, so spacing is per palette entry.
        let mut v = Vec::new();
        paint(
            &mut v,
            &FoliageBrush {
                kind: 0,
                single: true,
                ..brush()
            },
            [0.0, 0.0],
            1,
            flat,
        );
        let n = paint(
            &mut v,
            &FoliageBrush {
                kind: 1,
                single: true,
                ..brush()
            },
            [0.0, 0.0],
            2,
            flat,
        );
        assert_eq!(n.placed, 1, "a different kind was blocked by an existing instance");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn steep_ground_rejects_the_brush() {
        let cliff = |_x: f32, _z: f32| GroundSample {
            height: 0.0,
            slope_cos: 0.1,
            layer_weight: 1.0,
        };
        let mut v = Vec::new();
        assert_eq!(paint(&mut v, &brush(), [0.0, 0.0], 1, cliff).placed, 0);
    }

    #[test]
    fn instances_sit_on_the_ground() {
        let hill = |x: f32, z: f32| GroundSample {
            height: x * 0.5 + z,
            slope_cos: 1.0,
            layer_weight: 1.0,
        };
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
            paint(
                &mut v,
                &FoliageBrush {
                    density: 4.0,
                    ..brush()
                },
                [0.0, 0.0],
                seed,
                flat,
            );
        }
        let inner = v.iter().filter(|p| p.position.length() < 2.5).count();
        // The inner half-radius disc is a quarter of the area, so it should hold
        // roughly a quarter of the instances.
        let frac = inner as f32 / v.len() as f32;
        assert!(
            (0.15..0.35).contains(&frac),
            "inner disc holds {frac:.2} of instances"
        );
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
        paint(
            &mut v,
            &FoliageBrush { kind: 0, ..brush() },
            [0.0, 0.0],
            1,
            flat,
        );
        paint(
            &mut v,
            &FoliageBrush {
                kind: 1,
                single: true,
                ..brush()
            },
            [0.0, 0.0],
            2,
            flat,
        );
        let removed = erase(&mut v, [0.0, 0.0], 5.0, Some(1));
        assert_eq!(removed, 1);
        assert!(
            v.iter().all(|p| p.kind == 0),
            "erasing kind 1 took kind 0 with it"
        );
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
        assert_eq!(
            paint(
                &mut v,
                &FoliageBrush {
                    radius: 0.0,
                    ..brush()
                },
                [0.0, 0.0],
                1,
                flat
            )
            .placed,
            0
        );
        assert_eq!(
            paint(
                &mut v,
                &FoliageBrush {
                    density: 0.0,
                    ..brush()
                },
                [0.0, 0.0],
                1,
                flat
            )
            .placed,
            0
        );
        assert_eq!(erase(&mut v, [0.0, 0.0], 0.0, None), 0);
        assert_eq!(spacing_for_density(0.0), f32::MAX);
    }

    /// A dab that places nothing has to be able to say why.
    ///
    /// Thirteen of the twenty-five palette entries carry a layer requirement,
    /// and pointing one of them at ground that does not satisfy it is the
    /// ordinary case — not a mistake, and not something the editor may answer
    /// with silence. The counts are what let the caller name the reason and
    /// quote the number the ground actually has.
    #[test]
    fn a_refused_dab_names_the_layer_that_refused_it() {
        let sparse = |_x: f32, _z: f32| GroundSample {
            height: 0.0,
            slope_cos: 1.0,
            layer_weight: 0.03,
        };
        let b = FoliageBrush {
            radius: 6.0,
            density: 2.0,
            layer: 7,
            min_layer_weight: 0.5,
            ..Default::default()
        };
        let mut v = Vec::new();
        let r = paint(&mut v, &b, [0.0, 0.0], 1, sparse);

        assert_eq!(r.placed, 0);
        assert!(r.wrong_layer > 0, "the layer test rejected nothing");
        assert_eq!(r.too_steep, 0, "flat ground was called steep");
        assert!(r.refused() && r.blocked_by_layer());
        // The measured weight is the actionable half of the message.
        assert!((r.best_layer_weight - 0.03).abs() < 1.0e-6);
    }

    /// Steep ground and wrong ground are different answers.
    #[test]
    fn a_dab_on_a_cliff_blames_the_slope_and_not_the_layer() {
        let cliff = |_x: f32, _z: f32| GroundSample {
            height: 0.0,
            slope_cos: 0.1,
            layer_weight: 1.0,
        };
        let mut v = Vec::new();
        let r = paint(&mut v, &brush(), [0.0, 0.0], 1, cliff);
        assert!(r.refused());
        assert!(r.too_steep > 0);
        assert!(!r.blocked_by_layer(), "a cliff was reported as wrong ground");
    }

    /// Painting over ground that is already full is the brush working.
    ///
    /// `refused` has to exclude spacing, or a held stroke would report a
    /// failure every dab once it had converged — which is exactly when it is
    /// doing the right thing.
    #[test]
    fn a_settled_stroke_is_not_a_refusal() {
        let mut v = Vec::new();
        let mut saw_spacing = false;
        // A held brush over flat, unrestricted ground. Every dab that spacing
        // alone held back must report itself as working, not as refused —
        // whether it placed one stray instance into a gap or none at all.
        for seed in 0..60 {
            let r = paint(&mut v, &brush(), [0.0, 0.0], seed, flat);
            if r.too_close > 0 {
                saw_spacing = true;
            }
            assert!(
                !r.refused(),
                "dab {seed} on open ground reported a refusal: {r:?}"
            );
        }
        assert!(saw_spacing, "the stroke never packed tightly enough to test");
    }

    /// The rejection has to be a *cliff*, not a gradient. A probability would
    /// scatter a thinning fringe of debris across the neighbouring material,
    /// and the thing that makes a gravel patch read as gravel is that it stops.
    #[test]
    fn debris_stops_where_its_layer_does() {
        // A boundary down the middle: layer 3 is fully painted for x < 0 and
        // absent for x > 0.
        let split = |x: f32, _z: f32| GroundSample {
            height: 0.0,
            slope_cos: 1.0,
            layer_weight: if x < 0.0 { 1.0 } else { 0.0 },
        };
        let b = FoliageBrush {
            radius: 8.0,
            density: 3.0,
            layer: 3,
            min_layer_weight: 0.5,
            ..Default::default()
        };
        let mut v = Vec::new();
        // Centred on the boundary, so a brush that ignored the layer would
        // fill both halves.
        let n = paint(&mut v, &b, [0.0, 0.0], 1, split);
        assert!(n.placed > 0, "the painted half placed nothing at all");
        assert!(
            v.iter().all(|p| p.position.x < 0.0),
            "debris landed on ground its layer is not painted on"
        );
    }

    /// Zero is the pre-TSUSHIMA brush exactly, not "a threshold of zero".
    #[test]
    fn a_zero_layer_threshold_places_on_bare_ground() {
        let bare = |_x: f32, _z: f32| GroundSample {
            height: 0.0,
            slope_cos: 1.0,
            layer_weight: 0.0,
        };
        let mut v = Vec::new();
        assert!(paint(&mut v, &brush(), [0.0, 0.0], 1, bare).placed > 0);
    }

    /// A field of pebbles all sitting perfectly flat reads as placed rather
    /// than fallen, and every one leaning the same way reads as wind. The test
    /// is that the tilts are varied and bounded, and that an upright brush is
    /// still exactly upright.
    #[test]
    fn tilt_is_varied_bounded_and_off_by_default() {
        let mut upright = Vec::new();
        paint(&mut upright, &brush(), [0.0, 0.0], 1, flat);
        assert!(
            upright.iter().all(|p| p.tilt == 0.0),
            "default brush leaned"
        );

        let b = FoliageBrush {
            max_tilt_deg: 30.0,
            ..brush()
        };
        let mut v = Vec::new();
        paint(&mut v, &b, [0.0, 0.0], 1, flat);
        assert!(v.len() > 8, "not enough instances to say anything");
        let limit = 30.0_f32.to_radians();
        assert!(v.iter().all(|p| p.tilt >= 0.0 && p.tilt <= limit + 1e-6));
        let distinct = v
            .windows(2)
            .filter(|w| (w[0].tilt - w[1].tilt).abs() > 1e-6)
            .count();
        assert!(
            distinct > v.len() / 2,
            "tilt is not varying across instances: {distinct} of {}",
            v.len()
        );
        // And it is not correlated with yaw — the yaw picks the *direction* of
        // the lean, so a tilt that tracked it would lean every instance the
        // same way in world space.
        let sorted_by_yaw = {
            let mut c = v.clone();
            c.sort_by(|a, b| a.yaw.partial_cmp(&b.yaw).unwrap());
            c
        };
        let monotonic = sorted_by_yaw.windows(2).all(|w| w[0].tilt <= w[1].tilt);
        assert!(!monotonic, "tilt is a function of yaw");
    }
}
