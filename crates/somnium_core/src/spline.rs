//! Authored splines, and the shapes that follow them.
//!
//! # Why the engine needs one primitive rather than several
//!
//! The immediate need was a shoreline: a sea whose surf should be audible
//! *wherever the player is near the water*, not from one point in the middle
//! of it. A point emitter cannot express that — a coastline is a kilometre
//! long and two metres wide, and the only way to cover it with points is to
//! place fifty of them and keep them in sync with the terrain forever.
//!
//! But "a path an author draws and the engine follows" is not a sound
//! problem. It is the same primitive a road, a river, a fence line, a patrol
//! route and a camera rail all need, and each of those would otherwise arrive
//! with its own point list, its own serialization and its own editor handles.
//! So the spline is a component in its own right, it knows nothing about
//! audio, and the emitter reads it the same way anything else will.
//!
//! # Catmull-Rom, and why the curve is sampled rather than solved
//!
//! Control points are what an author places; a Catmull-Rom spline through
//! them is what they mean. It interpolates its control points — the curve
//! passes exactly through what was placed, which a Bezier does not — and it
//! needs no tangent handles, which is the whole reason it is the usual choice
//! for level-editor paths.
//!
//! Nearest-point queries against the analytic curve would need a numerical
//! solve per segment per query. Sampling to a polyline and taking the nearest
//! point on that is a few hundred dot products, is exact for the polyline the
//! gizmo actually draws, and has an error bounded by the sampling density.
//! The audio system asks this question once per emitter per frame, so "cheap
//! and visibly correct" beats "exact and hard to reason about".

use glam::Vec3;
use somnium_ecs::component_schema;
use somnium_ecs::reflect::{ComponentSchema, TypeRegistry};
use somnium_ecs::{Component, Entity, World};

/// Samples generated per control-point segment.
///
/// The bound on how far the sampled polyline can sit from the true curve, and
/// the cost of every nearest-point query, are both this number. Twelve is
/// under a centimetre of error on a ten-metre segment and twelve dot products
/// to search it.
pub const SAMPLES_PER_SEGMENT: usize = 12;

/// An authored path through space.
///
/// Points are in the entity's **local** space, so moving or rotating the
/// entity carries the whole path with it, exactly as it would carry a mesh.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SplineComponent {
    /// Control points, in order, in entity-local space.
    pub points: Vec<Vec3>,
    /// Join the last point back to the first. A shoreline around an island is
    /// closed; a river is not.
    pub closed: bool,
}

impl Component for SplineComponent {}

impl SplineComponent {
    /// A straight run of `count` points along local +X, `spacing` apart.
    ///
    /// What `Create -> Spline` produces: something visible and draggable
    /// rather than an empty component whose gizmo draws nothing.
    #[must_use]
    pub fn straight(count: usize, spacing: f32) -> Self {
        let count = count.max(2);
        #[allow(clippy::cast_precision_loss)]
        let half = (count - 1) as f32 * spacing * 0.5;
        Self {
            #[allow(clippy::cast_precision_loss)]
            points: (0..count)
                .map(|i| Vec3::new(i as f32 * spacing - half, 0.0, 0.0))
                .collect(),
            closed: false,
        }
    }

    /// How many segments the curve has: one fewer than the points, or one per
    /// point when it is closed.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        match self.points.len() {
            0 | 1 => 0,
            n if self.closed => n,
            n => n - 1,
        }
    }

    /// The control point at `index`, wrapping for a closed spline and
    /// clamping for an open one.
    ///
    /// Clamping is what gives an open spline sensible tangents at its two
    /// ends without inventing phantom control points an author cannot see.
    fn control(&self, index: isize) -> Vec3 {
        let len = self.points.len() as isize;
        if len == 0 {
            return Vec3::ZERO;
        }
        let index = if self.closed {
            index.rem_euclid(len)
        } else {
            index.clamp(0, len - 1)
        };
        self.points[index as usize]
    }

    /// The point at parameter `t` within `segment`, in local space.
    ///
    /// Uniform Catmull-Rom with the standard tension of one half.
    #[must_use]
    pub fn sample_segment(&self, segment: usize, t: f32) -> Vec3 {
        let i = segment as isize;
        let (p0, p1, p2, p3) = (
            self.control(i - 1),
            self.control(i),
            self.control(i + 1),
            self.control(i + 2),
        );
        let t = t.clamp(0.0, 1.0);
        let (t2, t3) = (t * t, t * t * t);
        0.5 * ((2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
    }

    /// The whole curve as a local-space polyline.
    ///
    /// One point for the start plus [`SAMPLES_PER_SEGMENT`] per segment, so
    /// consecutive segments share their join rather than duplicating it.
    #[must_use]
    pub fn polyline(&self) -> Vec<Vec3> {
        let segments = self.segment_count();
        if segments == 0 {
            return self.points.clone();
        }
        let mut out = Vec::with_capacity(segments * SAMPLES_PER_SEGMENT + 1);
        out.push(self.sample_segment(0, 0.0));
        for segment in 0..segments {
            for step in 1..=SAMPLES_PER_SEGMENT {
                #[allow(clippy::cast_precision_loss)]
                out.push(self.sample_segment(segment, step as f32 / SAMPLES_PER_SEGMENT as f32));
            }
        }
        out
    }

    /// The point on the curve closest to `target`, both in world space.
    ///
    /// `model` is the entity's world transform. Returns `None` for a spline
    /// with nothing on it, which is a real state — a freshly created one
    /// whose points an author has not placed yet.
    #[must_use]
    pub fn closest_point(&self, model: glam::Mat4, target: Vec3) -> Option<Vec3> {
        let polyline = self.polyline();
        if polyline.is_empty() {
            return None;
        }
        let world: Vec<Vec3> = polyline
            .iter()
            .map(|point| model.transform_point3(*point))
            .collect();
        if world.len() == 1 {
            return Some(world[0]);
        }
        let mut best = world[0];
        let mut best_distance = f32::MAX;
        for pair in world.windows(2) {
            let candidate = closest_on_segment(pair[0], pair[1], target);
            let distance = candidate.distance_squared(target);
            if distance < best_distance {
                best_distance = distance;
                best = candidate;
            }
        }
        Some(best)
    }
}

/// The point on the segment from `a` to `b` closest to `p`.
fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let length_squared = ab.length_squared();
    if length_squared <= f32::EPSILON {
        return a;
    }
    a + ab * ((p - a).dot(ab) / length_squared).clamp(0.0, 1.0)
}

/// The schema. Both fields are authored and both are saved.
pub(crate) fn spline_schema() -> ComponentSchema {
    component_schema! {
        SplineComponent as "somnium.Spline", display "Spline", version 1,
        fields {
            points { doc: "Control points in entity-local space. The curve passes through every one." },
            closed { doc: "Join the last point back to the first." },
        }
    }
}

/// Register the component. Called by `component_registry`.
pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(spline_schema());
}

/// The world-space point at which an entity's sound should be heard.
///
/// A point emitter is heard where it is. A spline emitter is heard at the
/// nearest point of its path, which is what makes one emitter cover a whole
/// shoreline: walk along the beach and the surf stays beside you, walk inland
/// and it fades with distance from the water rather than from a marker
/// somewhere out at sea.
///
/// Returns the entity's own position when it has no spline, so a caller never
/// has to ask which kind it is.
#[must_use]
pub fn audible_position(world: &World, entity: Entity, model: glam::Mat4, listener: Vec3) -> Vec3 {
    let own = model.w_axis.truncate();
    world
        .get::<SplineComponent>(entity)
        .and_then(|spline| spline.closest_point(model, listener))
        .unwrap_or(own)
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::reflect::{FieldFlags, StableId};

    fn line() -> SplineComponent {
        SplineComponent {
            points: vec![
                Vec3::new(-10.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
            ],
            closed: false,
        }
    }

    /// Catmull-Rom interpolates: the curve goes *through* the points an
    /// author placed. A Bezier would not, and an author who drags a point and
    /// watches the curve miss it has been given the wrong primitive.
    #[test]
    fn the_curve_passes_through_every_control_point() {
        let spline = SplineComponent {
            points: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(4.0, 3.0, 0.0),
                Vec3::new(9.0, 1.0, 2.0),
                Vec3::new(12.0, 0.0, -3.0),
            ],
            closed: false,
        };
        for (index, point) in spline.points.iter().enumerate() {
            let sampled = if index == 0 {
                spline.sample_segment(0, 0.0)
            } else {
                spline.sample_segment(index - 1, 1.0)
            };
            assert!(
                sampled.distance(*point) < 1e-4,
                "control point {index} at {point:?} but the curve is at {sampled:?}"
            );
        }
    }

    /// An open spline has one fewer segment than points; a closed one has the
    /// same number, because the join back to the start is a segment too.
    #[test]
    fn closing_a_spline_adds_the_returning_segment() {
        let mut spline = line();
        assert_eq!(spline.segment_count(), 2);
        spline.closed = true;
        assert_eq!(spline.segment_count(), 3);
    }

    /// Degenerate splines are a real authoring state, not a bug: `Create`
    /// makes one before any point is placed. Nothing may panic on them.
    #[test]
    fn an_empty_or_single_point_spline_is_harmless() {
        let empty = SplineComponent::default();
        assert_eq!(empty.segment_count(), 0);
        assert!(empty.polyline().is_empty());
        assert_eq!(empty.closest_point(glam::Mat4::IDENTITY, Vec3::ONE), None);

        let single = SplineComponent {
            points: vec![Vec3::new(1.0, 2.0, 3.0)],
            closed: false,
        };
        assert_eq!(single.segment_count(), 0);
        assert_eq!(
            single.closest_point(glam::Mat4::IDENTITY, Vec3::ZERO),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    /// The shoreline case. Standing beside the middle of the run, the nearest
    /// point is beside you — not the centre of the spline, and not an end.
    #[test]
    fn the_nearest_point_tracks_along_the_path() {
        let spline = line();
        for x in [-8.0_f32, -3.0, 0.0, 4.5, 9.0] {
            let listener = Vec3::new(x, 0.0, 6.0);
            let near = spline
                .closest_point(glam::Mat4::IDENTITY, listener)
                .expect("a three-point spline has a nearest point");
            assert!(
                (near.x - x).abs() < 0.6,
                "standing at x={x} the nearest water should be near x={x}, got {near:?}"
            );
            assert!(
                (near.distance(listener) - 6.0).abs() < 0.6,
                "and six metres away, got {:.2}",
                near.distance(listener)
            );
        }
    }

    /// Past the end of an open spline the nearest point is the end itself —
    /// the curve does not extrapolate, so walking off the end of a beach
    /// leaves the surf behind you rather than following you inland.
    #[test]
    fn beyond_the_end_the_nearest_point_is_the_end() {
        let spline = line();
        let near = spline
            .closest_point(glam::Mat4::IDENTITY, Vec3::new(40.0, 0.0, 0.0))
            .unwrap();
        assert!(
            (near.x - 10.0).abs() < 0.5,
            "clamped to the end, got {near:?}"
        );
    }

    /// Points are local, so the entity's transform carries the whole path.
    /// A shoreline that stayed at the world origin when its entity moved
    /// would not be a component, it would be a global.
    #[test]
    fn the_path_moves_and_turns_with_its_entity() {
        let spline = line();
        let model = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::new(100.0, 5.0, -20.0),
        );
        let near = spline
            .closest_point(model, Vec3::new(100.0, 5.0, -20.0))
            .unwrap();
        assert!(
            near.distance(Vec3::new(100.0, 5.0, -20.0)) < 0.5,
            "the middle of the run should sit on the entity's origin, got {near:?}"
        );
    }

    /// The nearest point on a closed loop is reachable from outside it and
    /// from inside it alike — an island shoreline is heard from the water as
    /// well as from the hill in the middle.
    #[test]
    fn a_closed_loop_is_nearest_from_both_sides() {
        let ring = SplineComponent {
            points: (0..8)
                .map(|i| {
                    let a = std::f32::consts::TAU * f32::from(i as u8) / 8.0;
                    Vec3::new(20.0 * a.cos(), 0.0, 20.0 * a.sin())
                })
                .collect(),
            closed: true,
        };
        let inside = ring.closest_point(glam::Mat4::IDENTITY, Vec3::ZERO).unwrap();
        assert!(
            (inside.length() - 20.0).abs() < 2.0,
            "from the centre the ring is about 20 m away, got {:.2}",
            inside.length()
        );
        let outside = ring
            .closest_point(glam::Mat4::IDENTITY, Vec3::new(60.0, 0.0, 0.0))
            .unwrap();
        assert!(
            (outside.distance(Vec3::new(60.0, 0.0, 0.0)) - 40.0).abs() < 2.0,
            "and about 40 m from a point 60 m out"
        );
    }

    /// `audible_position` is the one call an emitter makes, and it has to
    /// answer for an entity with no spline as well as one with.
    #[test]
    fn an_entity_without_a_spline_is_heard_where_it_is() {
        let mut world = World::new();
        let plain = world.spawn((crate::Transform::default(),));
        let model = glam::Mat4::from_translation(Vec3::new(3.0, 4.0, 5.0));
        assert_eq!(
            audible_position(&world, plain, model, Vec3::ZERO),
            Vec3::new(3.0, 4.0, 5.0)
        );

        let shore = world.spawn((crate::Transform::default(), line()));
        let heard = audible_position(&world, shore, glam::Mat4::IDENTITY, Vec3::new(7.0, 0.0, 9.0));
        assert!(
            (heard.x - 7.0).abs() < 0.6 && heard.z.abs() < 0.6,
            "heard at the nearest point of the path, got {heard:?}"
        );
    }

    #[test]
    fn the_schema_is_registered_and_both_fields_are_saved() {
        let mut registry = TypeRegistry::new();
        register(&mut registry);
        let schema = registry
            .by_stable_id(StableId::new("somnium.Spline"))
            .expect("the spline must be registered");
        for field in &schema.fields {
            assert!(
                field.flags.contains(FieldFlags::SERIALIZE),
                "`{}` has to survive a save and load — a path an author drew \
                 and the engine forgot is the worst possible outcome",
                field.name
            );
        }
    }
}
