//! Sight, occlusion, hearing falloff and bounded stimulus memory.
use crate::navigation::Triangle;
use glam::Vec3;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sense {
    Sight,
    Hearing,
}
#[derive(Clone, Copy, Debug)]
pub struct Stimulus {
    pub source: u64,
    pub position: Vec3,
    pub strength: f32,
    pub sensed_at: f64,
    pub sense: Sense,
}
#[derive(Clone, Copy, Debug)]
pub struct Observer {
    pub position: Vec3,
    pub forward: Vec3,
    pub sight_range: f32,
    pub half_angle_degrees: f32,
    pub hearing_range: f32,
}
impl Observer {
    pub fn see(
        &self,
        source: u64,
        target: Vec3,
        now: f64,
        mut occluded: impl FnMut(Vec3, Vec3) -> bool,
    ) -> Option<Stimulus> {
        if !self.position.is_finite()
            || !self.forward.is_finite()
            || !target.is_finite()
            || !now.is_finite()
            || self.forward.length_squared() < 0.0001
            || !self.sight_range.is_finite()
            || self.sight_range <= 0.0
            || !self.half_angle_degrees.is_finite()
            || !(0.0..=180.0).contains(&self.half_angle_degrees)
        {
            return None;
        }
        let delta = target - self.position;
        if delta.length() > self.sight_range
            || (delta.length_squared() > 0.0001
                && self.forward.normalize_or_zero().dot(delta.normalize())
                    < self.half_angle_degrees.to_radians().cos())
            || occluded(self.position, target)
        {
            return None;
        }
        Some(Stimulus {
            source,
            position: target,
            strength: (1.0 - delta.length() / self.sight_range).max(0.0),
            sensed_at: now,
            sense: Sense::Sight,
        })
    }
    pub fn hear(&self, source: u64, position: Vec3, loudness: f32, now: f64) -> Option<Stimulus> {
        if !position.is_finite()
            || !self.position.is_finite()
            || !loudness.is_finite()
            || loudness <= 0.0
            || !now.is_finite()
            || !self.hearing_range.is_finite()
            || self.hearing_range <= 0.0
        {
            return None;
        }
        let fraction = position.distance(self.position) / self.hearing_range;
        let strength = loudness * (1.0 - fraction).max(0.0).powi(2);
        (strength > 0.0).then_some(Stimulus {
            source,
            position,
            strength,
            sensed_at: now,
            sense: Sense::Hearing,
        })
    }
}
/// Geometry adapter usable by the headless server and cooked scene geometry.
/// Ignores intersections at the eye and at the target itself.
pub fn occluded(triangles: &[Triangle], from: Vec3, to: Vec3) -> bool {
    let ray = to - from;
    triangles.iter().any(|t| {
        let [a, b, c] = t.map(Vec3::from);
        let e1 = b - a;
        let e2 = c - a;
        let p = ray.cross(e2);
        let determinant = e1.dot(p);
        if determinant.abs() < 0.000001 {
            return false;
        }
        let inverse = 1.0 / determinant;
        let offset = from - a;
        let u = offset.dot(p) * inverse;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = offset.cross(e1);
        let v = ray.dot(q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let distance = e2.dot(q) * inverse;
        distance > 0.0001 && distance < 0.9999
    })
}
#[derive(Clone, Debug)]
pub struct StimulusMemory {
    capacity: usize,
    lifetime: f64,
    items: BTreeMap<(u64, Sense), Stimulus>,
}
impl StimulusMemory {
    pub fn new(capacity: usize, lifetime: f64) -> Result<Self, String> {
        if capacity == 0 || capacity > 65536 || !lifetime.is_finite() || lifetime <= 0.0 {
            return Err("invalid perception memory policy".into());
        }
        Ok(Self {
            capacity,
            lifetime,
            items: BTreeMap::new(),
        })
    }
    pub fn remember(&mut self, stimulus: Stimulus) {
        if !stimulus.position.is_finite()
            || !stimulus.strength.is_finite()
            || stimulus.strength < 0.0
            || !stimulus.sensed_at.is_finite()
        {
            return;
        }
        let key = (stimulus.source, stimulus.sense);
        if self
            .items
            .get(&key)
            .is_some_and(|old| old.sensed_at > stimulus.sensed_at)
        {
            return;
        }
        self.items.insert(key, stimulus);
        if self.items.len() > self.capacity {
            let oldest = self
                .items
                .iter()
                .min_by(|a, b| {
                    a.1.sensed_at
                        .total_cmp(&b.1.sensed_at)
                        .then_with(|| a.0.cmp(b.0))
                })
                .map(|(key, _)| *key)
                .unwrap();
            self.items.remove(&oldest);
        }
    }
    pub fn expire(&mut self, now: f64) {
        if now.is_finite() {
            self.items.retain(|_, v| now - v.sensed_at <= self.lifetime);
        }
    }
    pub fn strongest(&self, now: f64) -> Option<Stimulus> {
        if !now.is_finite() {
            return None;
        }
        self.items
            .values()
            .filter_map(|s| {
                let elapsed = now - s.sensed_at;
                (elapsed >= 0.0 && elapsed <= self.lifetime).then(|| Stimulus {
                    strength: s.strength * (1.0 - elapsed as f32 / self.lifetime as f32),
                    ..*s
                })
            })
            .max_by(|a, b| {
                a.strength
                    .total_cmp(&b.strength)
                    .then_with(|| b.source.cmp(&a.source))
            })
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
