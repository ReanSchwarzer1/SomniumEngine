//! Oriented boxes and water rectangles behind [`Terrain`]. Exact enough for
//! tests and offline tools; the engine sweeps through Jolt instead.

use glam::{Quat, Vec3};

use crate::{Contact, Terrain};

/// An oriented box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solid {
    /// Centre.
    pub center: Vec3,
    /// Half extents along the box's own axes.
    pub half: Vec3,
    /// Orientation.
    pub rotation: Quat,
}

impl Solid {
    /// An axis-aligned box between two corners.
    #[must_use]
    pub fn between(min: Vec3, max: Vec3) -> Self {
        Self {
            center: (min + max) * 0.5,
            half: (max - min).abs() * 0.5,
            rotation: Quat::IDENTITY,
        }
    }

    /// A ramp `width` wide rising `degrees` toward +Z, its foot at `foot`
    /// (on the ground, centre of the lower edge), `length` along the slope.
    #[must_use]
    pub fn ramp(foot: Vec3, width: f32, length: f32, degrees: f32) -> Self {
        let a = degrees.to_radians();
        let rotation = Quat::from_rotation_x(-a);
        let thick = 0.5;
        let up = rotation * Vec3::Y;
        let along = rotation * Vec3::Z;
        Self {
            center: foot + along * (length * 0.5) - up * thick,
            half: Vec3::new(width * 0.5, thick, length * 0.5),
            rotation,
        }
    }

    fn closest(&self, p: Vec3) -> Vec3 {
        let local = self.rotation.inverse() * (p - self.center);
        self.center + self.rotation * local.clamp(-self.half, self.half)
    }

    /// The face normal nearest a point inside the box.
    fn face_normal(&self, p: Vec3) -> Vec3 {
        let local = self.rotation.inverse() * (p - self.center);
        let depth = self.half - local.abs();
        let axis = if depth.x <= depth.y && depth.x <= depth.z {
            Vec3::X * local.x.signum()
        } else if depth.y <= depth.z {
            Vec3::Y * local.y.signum()
        } else {
            Vec3::Z * local.z.signum()
        };
        self.rotation * axis
    }

    /// Closest points between a segment and the box: (on segment, on box).
    /// Distance to a convex set is convex along a segment, so a golden-section
    /// search finds the minimum.
    fn segment(&self, a: Vec3, b: Vec3) -> (Vec3, Vec3) {
        let distance = |t: f32| {
            let p = a.lerp(b, t);
            p.distance_squared(self.closest(p))
        };
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        const R: f32 = 0.618_034;
        for _ in 0..40 {
            let m1 = hi - (hi - lo) * R;
            let m2 = lo + (hi - lo) * R;
            if distance(m1) <= distance(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let p = a.lerp(b, (lo + hi) * 0.5);
        (p, self.closest(p))
    }
}

/// A rectangle of water on the ground plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    /// Smallest x and z.
    pub min: [f32; 2],
    /// Largest x and z.
    pub max: [f32; 2],
    /// Surface height.
    pub surface: f32,
}

/// Boxes and water.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Boxes {
    /// The solids.
    pub solids: Vec<Solid>,
    /// The water.
    pub water: Vec<Water>,
}

impl Boxes {
    /// Ground at height `y`, 200 m across.
    #[must_use]
    pub fn floor(y: f32) -> Self {
        Self::default().with(Solid::between(
            Vec3::new(-100.0, y - 1.0, -100.0),
            Vec3::new(100.0, y, 100.0),
        ))
    }

    /// Add a solid.
    #[must_use]
    pub fn with(mut self, solid: Solid) -> Self {
        self.solids.push(solid);
        self
    }

    /// Add an axis-aligned box.
    #[must_use]
    pub fn block(self, min: Vec3, max: Vec3) -> Self {
        self.with(Solid::between(min, max))
    }

    /// Add water.
    #[must_use]
    pub fn pool(mut self, min: [f32; 2], max: [f32; 2], surface: f32) -> Self {
        self.water.push(Water { min, max, surface });
        self
    }

    fn sweep_one(
        solid: &Solid,
        feet: Vec3,
        delta: Vec3,
        radius: f32,
        height: f32,
    ) -> Option<Contact> {
        let length = delta.length();
        if length < 1e-7 {
            return None;
        }
        let axis = (height - radius).max(radius);
        let closest = |t: f32| {
            let f = feet + delta * t;
            solid.segment(f + Vec3::Y * radius, f + Vec3::Y * axis)
        };
        let gap = |t: f32| {
            let (p, q) = closest(t);
            p.distance(q) - radius
        };
        let normal = |t: f32| {
            let (p, q) = closest(t);
            (p - q)
                .try_normalize()
                .unwrap_or_else(|| solid.face_normal(p))
        };
        if gap(0.0) <= 0.0 {
            // Already touching: a contact only when moving further in.
            let n = normal(0.0);
            return (n.dot(delta) < 0.0).then_some(Contact {
                fraction: 0.0,
                normal: n,
            });
        }
        // March a quarter of a radius at a time (nothing thinner slips through), then bisect.
        let steps = ((length / (radius * 0.25).max(0.01)).ceil() as usize).clamp(1, 2048);
        let mut lo = 0.0;
        for i in 1..=steps {
            let mut hi = i as f32 / steps as f32;
            if gap(hi) > 0.0 {
                lo = hi;
                continue;
            }
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                if gap(mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some(Contact {
                fraction: lo,
                normal: normal(hi),
            });
        }
        None
    }
}

impl Terrain for Boxes {
    fn sweep(&self, feet: Vec3, delta: Vec3, radius: f32, height: f32) -> Option<Contact> {
        self.solids
            .iter()
            .filter_map(|s| Self::sweep_one(s, feet, delta, radius, height))
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
    }

    fn water_surface(&self, x: f32, z: f32) -> Option<f32> {
        self.water
            .iter()
            .filter(|w| x >= w.min[0] && x <= w.max[0] && z >= w.min[1] && z <= w.max[1])
            .map(|w| w.surface)
            .reduce(f32::max)
    }
}
