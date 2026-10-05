//! Deterministic curve fitting. The output remains an ordinary AnimationClip,
//! so cooked reduced tracks use the exact same sampling/blending path.
use crate::{AnimationClip, ClipError, Keyframe, Skeleton};
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressionBudget {
    /// Maximum local translation error, in metres.
    pub translation: f32,
    /// Maximum rotation error, in radians.
    pub rotation: f32,
    /// Maximum Euclidean scale-vector error.
    pub scale: f32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompressionReport {
    pub source_keys: usize,
    pub retained_keys: usize,
    pub source_bytes: usize,
    pub retained_bytes: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub enum CompressionError {
    InvalidBudget,
    Clip(ClipError),
}

impl AnimationClip {
    /// Fit piecewise linear/spherical curves within independent channel error
    /// budgets. Endpoints, absent channels, clip identity and sync tracks remain
    /// unchanged. Zero error retains a channel verbatim. No quantization is used.
    ///
    /// Vector bounds hold continuously: on each original span both curves are
    /// linear, so maximum norm error occurs at an endpoint. Rotation bounds use
    /// adaptive geodesic Lipschitz bounds, conservatively retaining keys when a
    /// segment cannot be certified within twelve subdivisions.
    pub fn compress(
        &self,
        skeleton: &Skeleton,
        budget: CompressionBudget,
    ) -> Result<(AnimationClip, CompressionReport), CompressionError> {
        if [budget.translation, budget.rotation, budget.scale]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err(CompressionError::InvalidBudget);
        }
        let mut tracks = self.tracks().to_vec();
        let mut report = CompressionReport::default();
        for track in &mut tracks {
            report.source_keys +=
                track.translation.len() + track.rotation.len() + track.scale.len();
            report.source_bytes +=
                (track.translation.len() + track.scale.len()) * 16 + track.rotation.len() * 20;
            track.translation = reduce_vec(&track.translation, budget.translation);
            track.scale = reduce_vec(&track.scale, budget.scale);
            track.rotation = reduce_rotation(&track.rotation, budget.rotation);
            report.retained_keys +=
                track.translation.len() + track.rotation.len() + track.scale.len();
            report.retained_bytes +=
                (track.translation.len() + track.scale.len()) * 16 + track.rotation.len() * 20;
        }
        let clip = AnimationClip::new(
            self.id(),
            skeleton,
            self.duration(),
            tracks,
            self.sync_tracks().to_vec(),
        )
        .map_err(CompressionError::Clip)?;
        if skeleton.id() != self.skeleton() {
            return Err(CompressionError::Clip(ClipError::SkeletonMismatch));
        }
        Ok((clip, report))
    }
}

fn reduce_vec(keys: &[Keyframe<Vec3>], budget: f32) -> Vec<Keyframe<Vec3>> {
    if keys.len() < 3 || budget == 0.0 {
        return keys.to_vec();
    }
    reduce(keys, |a, b| {
        let duration = keys[b].time - keys[a].time;
        let mut worst = (0.0f32, a + 1);
        for i in a + 1..b {
            let alpha = (keys[i].time - keys[a].time) / duration;
            let error = keys[a]
                .value
                .lerp(keys[b].value, alpha)
                .distance(keys[i].value);
            if error > worst.0 {
                worst = (error, i);
            }
        }
        (worst.0 > budget).then_some(worst.1)
    })
}
fn reduce<T: Clone>(
    keys: &[Keyframe<T>],
    split: impl Fn(usize, usize) -> Option<usize>,
) -> Vec<Keyframe<T>> {
    let mut keep = vec![false; keys.len()];
    keep[0] = true;
    keep[keys.len() - 1] = true;
    let mut pending = vec![(0, keys.len() - 1)];
    while let Some((a, b)) = pending.pop() {
        if b <= a + 1 {
            continue;
        }
        if let Some(i) = split(a, b) {
            keep[i] = true;
            pending.push((i, b));
            pending.push((a, i));
        }
    }
    keys.iter()
        .zip(keep)
        .filter(|(_, retain)| *retain)
        .map(|(key, _)| key.clone())
        .collect()
}
fn angle(a: Quat, b: Quat) -> f32 {
    let relative = a.conjugate() * b;
    2.0 * Vec3::new(relative.x, relative.y, relative.z)
        .length()
        .atan2(relative.w.abs())
}
fn certified_rotation(a: Quat, b: Quat, c: Quat, d: Quat, budget: f32, depth: u8) -> bool {
    let ea = angle(a, c);
    let eb = angle(b, d);
    if ea.max(eb) > budget {
        return false;
    }
    let speed_bound = angle(a, b) + angle(c, d);
    if ea.max(eb) + speed_bound * 0.5 <= budget {
        return true;
    }
    if depth == 12 {
        return false;
    }
    let ab = a.slerp(b, 0.5);
    let cd = c.slerp(d, 0.5);
    certified_rotation(a, ab, c, cd, budget, depth + 1)
        && certified_rotation(ab, b, cd, d, budget, depth + 1)
}
fn reduce_rotation(keys: &[Keyframe<Quat>], budget: f32) -> Vec<Keyframe<Quat>> {
    if keys.len() < 3 || budget == 0.0 {
        return keys.to_vec();
    }
    reduce(keys, |a, b| {
        let qa = keys[a].value.normalize();
        let qb = keys[b].value.normalize();
        let duration = keys[b].time - keys[a].time;
        for i in a..b {
            let c = qa.slerp(qb, (keys[i].time - keys[a].time) / duration);
            let d = qa.slerp(qb, (keys[i + 1].time - keys[a].time) / duration);
            if !certified_rotation(
                keys[i].value.normalize(),
                keys[i + 1].value.normalize(),
                c,
                d,
                budget,
                0,
            ) {
                return Some((i + 1).min(b - 1).max(a + 1));
            }
        }
        None
    })
}
