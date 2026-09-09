//! Root displacement and events share unwrapped playback time. A frame may
//! cross any number of loop boundaries, including backwards.
use crate::{AnimationClip, ClipError, JointIndex, NO_PARENT, Playback, Pose, Skeleton};
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotion {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl RootMotion {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };
    pub fn compose(self, next: Self) -> Self {
        Self {
            translation: self.translation + self.rotation * next.translation,
            rotation: (self.rotation * next.rotation).normalize(),
        }
    }
    pub fn inverse(self) -> Self {
        let rotation = self.rotation.conjugate();
        Self {
            translation: rotation * -self.translation,
            rotation,
        }
    }
    fn power(self, exponent: i64) -> Self {
        let mut base = if exponent < 0 { self.inverse() } else { self };
        let mut count = exponent.unsigned_abs();
        let mut result = Self::IDENTITY;
        while count != 0 {
            if count & 1 != 0 {
                result = result.compose(base);
            }
            base = base.compose(base);
            count >>= 1;
        }
        result
    }
    pub fn is_finite(self) -> bool {
        self.translation.is_finite()
            && self.rotation.is_finite()
            && (self.rotation.length_squared() - 1.0).abs() < 1e-3
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MotionError {
    Clip(ClipError),
    InvalidRoot,
    InvalidTime,
    InvalidTransform,
    InvalidEvents,
    EventBudgetExceeded,
}
impl From<ClipError> for MotionError {
    fn from(e: ClipError) -> Self {
        Self::Clip(e)
    }
}

impl AnimationClip {
    /// Extract relative rigid displacement and remove root translation/rotation
    /// from the returned pose. Scale remains authored. Applying this displacement
    /// through collision consumes it: rejected movement must not accumulate as debt.
    pub fn sample_root_motion(
        &self,
        skeleton: &Skeleton,
        root: JointIndex,
        previous: f32,
        elapsed: f32,
        playback: Playback,
    ) -> Result<(Pose, RootMotion), MotionError> {
        if skeleton.parents().get(root as usize) != Some(&NO_PARENT) {
            return Err(MotionError::InvalidRoot);
        }
        let mut pose = self.sample(skeleton, elapsed, playback)?;
        let a = self.unwrapped_root(skeleton, root, previous, playback)?;
        let b = self.unwrapped_root(skeleton, root, elapsed, playback)?;
        let delta = a.inverse().compose(b);
        if !delta.is_finite() {
            return Err(MotionError::InvalidTransform);
        }
        pose.local[root as usize].translation = skeleton.rest()[root as usize].translation;
        pose.local[root as usize].rotation = skeleton.rest()[root as usize].rotation;
        Ok((pose, delta))
    }

    fn unwrapped_root(
        &self,
        skeleton: &Skeleton,
        root: JointIndex,
        elapsed: f32,
        playback: Playback,
    ) -> Result<RootMotion, MotionError> {
        let scaled = f64::from(elapsed) * f64::from(playback.time_scale());
        let duration = f64::from(self.duration());
        if !scaled.is_finite() || (scaled / duration).abs() > 1_000_000_000.0 {
            return Err(MotionError::InvalidTime);
        }
        let sample = |time| -> Result<RootMotion, MotionError> {
            let t = self.sample_local(skeleton, time)?.local[root as usize];
            let motion = RootMotion {
                translation: t.translation,
                rotation: t.rotation.normalize(),
            };
            if !motion.is_finite() {
                return Err(MotionError::InvalidTransform);
            }
            Ok(motion)
        };
        if !playback.looping() {
            return sample(scaled.clamp(0.0, duration) as f32);
        }
        let cycle = sample(self.duration())?.compose(sample(0.0)?.inverse());
        Ok(cycle
            .power((scaled / duration).floor() as i64)
            .compose(sample(scaled.rem_euclid(duration) as f32)?))
    }
}

/// Events at duration are authored at zero instead, giving each loop seam one
/// owner. Equal-time events retain authored order in both playback directions.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationEvent {
    pub time: f32,
    pub name: String,
    pub payload: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EventOccurrence {
    pub event: AnimationEvent,
    pub cycle: i64,
    pub playback_time: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EventTrack {
    duration: f32,
    events: Vec<AnimationEvent>,
}

impl EventTrack {
    pub fn new(duration: f32, events: Vec<AnimationEvent>) -> Result<Self, MotionError> {
        if !duration.is_finite()
            || duration <= 0.0
            || events.iter().any(|e| {
                !e.time.is_finite() || e.time < 0.0 || e.time >= duration || e.name.is_empty()
            })
            || events.windows(2).any(|w| w[0].time > w[1].time)
        {
            return Err(MotionError::InvalidEvents);
        }
        Ok(Self { duration, events })
    }
    pub fn duration(&self) -> f32 {
        self.duration
    }
    pub fn events(&self) -> &[AnimationEvent] {
        &self.events
    }
    /// Forward interval `(previous, elapsed]`, reverse `[elapsed, previous)`.
    /// The explicit output budget rejects the whole frame before allocation;
    /// it never silently drops gameplay hooks during a long frame.
    pub fn sample(
        &self,
        previous: f32,
        elapsed: f32,
        playback: Playback,
        budget: usize,
    ) -> Result<Vec<EventOccurrence>, MotionError> {
        let duration = f64::from(self.duration);
        let mut a = f64::from(previous) * f64::from(playback.time_scale());
        let mut b = f64::from(elapsed) * f64::from(playback.time_scale());
        if !a.is_finite()
            || !b.is_finite()
            || (a / duration).abs().max((b / duration).abs()) > 1_000_000_000.0
        {
            return Err(MotionError::InvalidTime);
        }
        if !playback.looping() {
            a = a.clamp(0.0, duration);
            b = b.clamp(0.0, duration);
        }
        if a == b || self.events.is_empty() {
            return Ok(Vec::new());
        }
        let forward = b > a;
        let mut ranges = Vec::with_capacity(self.events.len());
        let mut count = 0usize;
        for event in &self.events {
            let time = f64::from(event.time);
            let (mut first, mut last) = if forward {
                (
                    ((a - time) / duration).floor() as i64 + 1,
                    ((b - time) / duration).floor() as i64,
                )
            } else {
                (
                    ((b - time) / duration).ceil() as i64,
                    ((a - time) / duration).ceil() as i64 - 1,
                )
            };
            if !playback.looping() {
                first = first.max(0);
                last = last.min(0);
            }
            if first <= last {
                count = count
                    .checked_add((last - first + 1) as usize)
                    .ok_or(MotionError::EventBudgetExceeded)?;
                if count > budget {
                    return Err(MotionError::EventBudgetExceeded);
                }
                ranges.push((event, first, last));
            }
        }
        let mut result = Vec::with_capacity(count);
        for (event, first, last) in ranges {
            for cycle in first..=last {
                result.push(EventOccurrence {
                    event: event.clone(),
                    cycle,
                    playback_time: cycle as f64 * duration + f64::from(event.time),
                });
            }
        }
        result.sort_by(|a, b| {
            if forward {
                a.playback_time.total_cmp(&b.playback_time)
            } else {
                b.playback_time.total_cmp(&a.playback_time)
            }
        });
        Ok(result)
    }
}

/// Result of an actual capsule/shape sweep. Normal points out of the obstacle;
/// distance is represented as a fraction of the requested displacement.
#[derive(Clone, Copy, Debug)]
pub struct MotionHit {
    pub fraction: f32,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppliedMotion {
    pub requested: Vec3,
    pub applied: Vec3,
    pub blocked: Vec3,
}

/// Bounded collide-and-slide. The callback sweeps the caller's character shape
/// from the supplied world position. Invalid contact data is rejected, never
/// treated as an unobstructed path. The caller commits the returned position.
pub fn collide_and_slide(
    position: Vec3,
    displacement: Vec3,
    skin: f32,
    mut sweep: impl FnMut(Vec3, Vec3) -> Option<MotionHit>,
) -> Result<AppliedMotion, MotionError> {
    if !position.is_finite() || !displacement.is_finite() || !skin.is_finite() || skin < 0.0 {
        return Err(MotionError::InvalidTransform);
    }
    let mut applied = Vec3::ZERO;
    let mut remaining = displacement;
    let mut planes: Vec<Vec3> = Vec::new();
    for _ in 0..5 {
        let length = remaining.length();
        if length <= 1e-6 {
            break;
        }
        let Some(hit) = sweep(position + applied, remaining) else {
            applied += remaining;
            break;
        };
        if !hit.fraction.is_finite()
            || !(0.0..=1.0).contains(&hit.fraction)
            || !hit.normal.is_finite()
            || hit.normal.length_squared() < 1e-8
        {
            return Err(MotionError::InvalidTransform);
        }
        let normal = hit.normal.normalize();
        let step = remaining * (hit.fraction - skin / length).max(0.0);
        applied += step;
        remaining -= step;
        planes.push(normal);
        for plane in &planes {
            remaining -= *plane * remaining.dot(*plane).min(0.0);
        }
        if planes.iter().any(|p| remaining.dot(*p) < -1e-5) {
            let crease = planes[0].cross(normal).normalize_or_zero();
            remaining = crease * remaining.dot(crease);
            if planes.iter().any(|p| remaining.dot(*p) < -1e-5) {
                break;
            }
        }
    }
    Ok(AppliedMotion {
        requested: displacement,
        applied,
        blocked: displacement - applied,
    })
}
