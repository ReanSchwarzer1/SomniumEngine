//! Exact, allocation-free shape tests for hit detection.
//!
//! Combat hit volumes are deliberately not physics bodies: a sword's arc has to
//! hit the same targets on every machine and every frame rate, and it must not
//! push anything. Everything here is closed-form or a fixed-count sweep, so a
//! test is deterministic by construction. World blocking (walls, terrain) goes
//! through [`crate::CombatQuery`], which the host answers from Jolt.
//!
//! Segment-to-segment closest points follow the clamped-parameter method in
//! Ericson, *Real-Time Collision Detection* (2005), section 5.1.9; ray against
//! capsule solves the inflated cylinder and its end spheres directly.
use glam::{Quat, Vec3};

const EPS: f32 = 1e-6;

/// `v` with its height removed.
#[must_use]
pub fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Unit planar direction of `v`, or `fallback` when `v` is vertical or zero.
#[must_use]
pub fn planar_dir(v: Vec3, fallback: Vec3) -> Vec3 {
    flat(v).try_normalize().unwrap_or(fallback)
}

/// Closest point on segment `ab` to `p`, and its parameter in `[0, 1]`.
#[must_use]
pub fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> (Vec3, f32) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < EPS {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (a + ab * t, t)
}

/// Closest points between segments `p1q1` and `p2q2`.
#[must_use]
pub fn segment_segment(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    let (s, t);
    if a <= EPS && e <= EPS {
        return (p1, p2);
    }
    if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > EPS {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// A capsule: the points within `radius` of segment `ab`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    /// One end of the core segment.
    pub a: Vec3,
    /// The other end.
    pub b: Vec3,
    /// Radius around the segment.
    pub radius: f32,
}

impl Capsule {
    /// An upright capsule standing on `feet`, `height` tall overall.
    #[must_use]
    pub fn upright(feet: Vec3, height: f32, radius: f32) -> Self {
        let radius = radius.max(0.01);
        let height = height.max(radius * 2.0);
        Self {
            a: feet + Vec3::Y * radius,
            b: feet + Vec3::Y * (height - radius),
            radius,
        }
    }

    /// Distance from `p` to the surface; negative inside.
    #[must_use]
    pub fn distance(&self, p: Vec3) -> f32 {
        closest_on_segment(p, self.a, self.b).0.distance(p) - self.radius
    }

    /// Whether two capsules overlap; returns a contact point between them.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> Option<Vec3> {
        let (p, q) = segment_segment(self.a, self.b, other.a, other.b);
        let d = p.distance(q);
        (d <= self.radius + other.radius).then(|| {
            let toward = (q - p).try_normalize().unwrap_or(Vec3::Y);
            p + toward * (self.radius.min(d))
        })
    }

    /// The same capsule, moved.
    #[must_use]
    pub fn translated(&self, by: Vec3) -> Self {
        Self {
            a: self.a + by,
            b: self.b + by,
            radius: self.radius,
        }
    }
}

/// First parameter `t` in `[0, max_t]` at which the ray `origin + dir * t`
/// touches a sphere. `dir` must be unit length.
#[must_use]
pub fn ray_sphere(origin: Vec3, dir: Vec3, max_t: f32, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let c = oc.length_squared() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let b = oc.dot(dir);
    let h = b * b - c;
    if h < 0.0 || b > 0.0 {
        return None;
    }
    let t = -b - h.sqrt();
    (t >= 0.0 && t <= max_t).then_some(t)
}

/// First parameter `t` in `[0, max_t]` at which a ray touches a capsule
/// inflated by `extra` (a projectile's own radius). `dir` must be unit length.
#[must_use]
pub fn ray_capsule(
    origin: Vec3,
    dir: Vec3,
    max_t: f32,
    capsule: &Capsule,
    extra: f32,
) -> Option<f32> {
    let r = capsule.radius + extra.max(0.0);
    if capsule.distance(origin) <= extra.max(0.0) {
        return Some(0.0);
    }
    let ba = capsule.b - capsule.a;
    let oa = origin - capsule.a;
    let baba = ba.length_squared();
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if t.is_finite() && t >= 0.0 && t <= max_t && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    if baba > EPS {
        let bard = ba.dot(dir);
        let baoa = ba.dot(oa);
        let rdoa = dir.dot(oa);
        let oaoa = oa.length_squared();
        let k2 = baba - bard * bard;
        let k1 = baba * rdoa - baoa * bard;
        let k0 = baba * oaoa - baoa * baoa - r * r * baba;
        if k2.abs() > EPS {
            let h = k1 * k1 - k2 * k0;
            if h >= 0.0 {
                let t = (-k1 - h.sqrt()) / k2;
                let y = baoa + t * bard;
                if y > 0.0 && y < baba {
                    consider(t);
                }
            }
        }
    }
    for center in [capsule.a, capsule.b] {
        if let Some(t) = ray_sphere(origin, dir, max_t, center, r) {
            consider(t);
        }
    }
    best
}

/// An oriented box: walls, pillars, ward panes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    /// Centre.
    pub center: Vec3,
    /// Half extents in the box's own frame.
    pub half: Vec3,
    /// Orientation.
    pub rotation: Quat,
}

impl Obb {
    /// A box standing on `base`, facing `normal` (its thin axis), `width` wide,
    /// `height` tall and `thickness` deep.
    #[must_use]
    pub fn wall(base: Vec3, normal: Vec3, width: f32, height: f32, thickness: f32) -> Self {
        let n = planar_dir(normal, Vec3::Z);
        Self {
            center: base + Vec3::Y * height * 0.5,
            half: Vec3::new(width * 0.5, height * 0.5, thickness * 0.5),
            rotation: Quat::from_rotation_arc(Vec3::Z, n),
        }
    }

    fn to_local(&self, p: Vec3) -> Vec3 {
        self.rotation.conjugate() * (p - self.center)
    }

    /// Closest point on or in the box to `p`.
    #[must_use]
    pub fn closest(&self, p: Vec3) -> Vec3 {
        let local = self.to_local(p);
        self.center + self.rotation * local.clamp(-self.half, self.half)
    }

    /// Distance from `p` to the box; zero inside.
    #[must_use]
    pub fn distance(&self, p: Vec3) -> f32 {
        self.closest(p).distance(p)
    }

    /// Whether a capsule touches the box.
    ///
    /// ponytail: samples the core segment at nine points, exact for the upright
    /// capsules against upright walls combat uses; a slanted giant against a
    /// tilted box can miss by under a ninth of its length. Solve the
    /// segment-box distance exactly if large monsters need it.
    #[must_use]
    pub fn touches_capsule(&self, capsule: &Capsule) -> bool {
        (0..=8).any(|i| {
            let p = capsule.a.lerp(capsule.b, i as f32 / 8.0);
            self.distance(p) <= capsule.radius
        })
    }

    /// First hit of a ray against the box inflated by `extra`, with the
    /// outward normal. `dir` must be unit length.
    #[must_use]
    pub fn ray(&self, origin: Vec3, dir: Vec3, max_t: f32, extra: f32) -> Option<(f32, Vec3)> {
        let o = self.to_local(origin);
        let d = self.rotation.conjugate() * dir;
        let half = self.half + Vec3::splat(extra.max(0.0));
        let mut t_min = 0.0f32;
        let mut t_max = max_t;
        let mut normal = Vec3::ZERO;
        for axis in 0..3 {
            let (oa, da, ha) = (o[axis], d[axis], half[axis]);
            if da.abs() < EPS {
                if oa.abs() > ha {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / da;
            let mut t0 = (-ha - oa) * inv;
            let mut t1 = (ha - oa) * inv;
            let mut sign = -1.0;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
                sign = 1.0;
            }
            if t0 > t_min {
                t_min = t0;
                normal = Vec3::ZERO;
                normal[axis] = sign;
            }
            t_max = t_max.min(t1);
            if t_min > t_max {
                return None;
            }
        }
        if normal == Vec3::ZERO {
            // Started inside: report an immediate hit facing back along the ray.
            return Some((0.0, -dir));
        }
        Some((t_min, self.rotation * normal))
    }
}

/// A horizontal sector swept by a swing: the cheap, readable melee volume an
/// isometric camera wants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    /// Where the sector is centred (the attacker's chest, on the ground plane).
    pub origin: Vec3,
    /// Unit planar facing of the sector's middle.
    pub forward: Vec3,
    /// Reach in metres.
    pub radius: f32,
    /// Half the swept angle, radians.
    pub half_angle: f32,
    /// Lowest point reached, relative to `origin.y`.
    pub low: f32,
    /// Highest point reached, relative to `origin.y`.
    pub high: f32,
    /// Targets closer than this are not touched (a wave's trailing edge).
    pub inner: f32,
}

impl Arc {
    /// Where the arc touches a capsule, if it does.
    #[must_use]
    pub fn hits(&self, target: &Capsule) -> Option<Vec3> {
        let lo = target.a.y.min(target.b.y) - target.radius;
        let hi = target.a.y.max(target.b.y) + target.radius;
        if hi < self.origin.y + self.low || lo > self.origin.y + self.high {
            return None;
        }
        let height = (self.origin.y + (self.low + self.high) * 0.5).clamp(lo, hi);
        let axis = closest_on_segment(
            Vec3::new(self.origin.x, height, self.origin.z),
            target.a,
            target.b,
        )
        .0;
        let to = flat(axis - self.origin);
        let dist = to.length();
        if dist - target.radius > self.radius || dist + target.radius < self.inner {
            return None;
        }
        let contact_dist = (dist - target.radius).max(0.0);
        let dir = to.try_normalize().unwrap_or(self.forward);
        if dist > target.radius {
            let cos = self.forward.dot(dir).clamp(-1.0, 1.0);
            let slack = (target.radius / dist).min(1.0).asin();
            if cos.acos() > self.half_angle + slack {
                return None;
            }
        }
        Some(Vec3::new(
            self.origin.x + dir.x * contact_dist,
            height,
            self.origin.z + dir.z * contact_dist,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_segment_finds_the_crossing() {
        let (p, q) = segment_segment(
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, -1.0),
            Vec3::new(0.0, 1.0, 1.0),
        );
        assert!(p.distance(Vec3::ZERO) < 1e-5);
        assert!(q.distance(Vec3::Y) < 1e-5);
        // Parallel segments still give a valid pair.
        let (p, q) = segment_segment(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Y + Vec3::X);
        assert!((p.distance(q) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ray_capsule_hits_body_and_caps_and_misses_beside() {
        let c = Capsule::upright(Vec3::new(0.0, 0.0, 5.0), 1.8, 0.35);
        let t = ray_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 10.0, &c, 0.0).unwrap();
        assert!((t - (5.0 - 0.35)).abs() < 1e-4, "body hit at {t}");
        let t = ray_capsule(Vec3::new(0.0, 5.0, 5.0), -Vec3::Y, 10.0, &c, 0.0).unwrap();
        assert!((t - (5.0 - 1.8)).abs() < 1e-4, "top cap at {t}");
        assert!(ray_capsule(Vec3::new(1.0, 1.0, 0.0), Vec3::Z, 10.0, &c, 0.0).is_none());
        assert!(
            ray_capsule(Vec3::new(0.6, 1.0, 0.0), Vec3::Z, 10.0, &c, 0.3).is_some(),
            "a fat projectile grazes"
        );
        assert!(ray_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 3.0, &c, 0.0).is_none());
        assert_eq!(
            ray_capsule(Vec3::new(0.0, 1.0, 5.0), Vec3::Z, 3.0, &c, 0.0),
            Some(0.0),
            "starting inside is an immediate hit"
        );
    }

    #[test]
    fn obb_ray_reports_the_face_normal() {
        let wall = Obb::wall(Vec3::new(0.0, 0.0, 4.0), Vec3::Z, 4.0, 2.0, 0.5);
        let (t, n) = wall
            .ray(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 10.0, 0.0)
            .unwrap();
        assert!((t - 3.75).abs() < 1e-4);
        assert!(n.distance(-Vec3::Z) < 1e-4);
        assert!(
            wall.ray(Vec3::new(3.0, 1.0, 0.0), Vec3::Z, 10.0, 0.0)
                .is_none()
        );
        assert!(wall.touches_capsule(&Capsule::upright(Vec3::new(1.5, 0.0, 3.5), 1.8, 0.4)));
        assert!(!wall.touches_capsule(&Capsule::upright(Vec3::new(3.0, 0.0, 3.0), 1.8, 0.4)));
    }

    #[test]
    fn arcs_respect_reach_angle_and_height() {
        let arc = Arc {
            origin: Vec3::new(0.0, 1.0, 0.0),
            forward: Vec3::Z,
            radius: 2.0,
            half_angle: 60f32.to_radians(),
            low: -0.8,
            high: 0.8,
            inner: 0.0,
        };
        let at = |x: f32, z: f32| Capsule::upright(Vec3::new(x, 0.0, z), 1.8, 0.35);
        assert!(arc.hits(&at(0.0, 2.2)).is_some(), "reach plus body radius");
        assert!(arc.hits(&at(0.0, 2.5)).is_none());
        assert!(arc.hits(&at(0.0, -1.5)).is_none(), "nothing behind");
        assert!(arc.hits(&at(1.4, 1.0)).is_some(), "inside the angle");
        assert!(arc.hits(&at(1.8, 0.0)).is_none(), "outside the angle");
        assert!(
            arc.hits(&at(0.1, 0.1)).is_some(),
            "standing inside the swing"
        );
        let flier = Capsule::upright(Vec3::new(0.0, 4.0, 1.5), 1.0, 0.3);
        assert!(arc.hits(&flier).is_none(), "out of vertical reach");
    }

    #[test]
    fn capsule_overlap_and_distance() {
        let a = Capsule::upright(Vec3::ZERO, 1.8, 0.4);
        let b = Capsule::upright(Vec3::new(0.7, 0.0, 0.0), 1.8, 0.4);
        assert!(a.overlaps(&b).is_some());
        assert!(a.overlaps(&b.translated(Vec3::X)).is_none());
        assert!((a.distance(Vec3::new(1.4, 1.0, 0.0)) - 1.0).abs() < 1e-5);
    }
}
