//! MORROWIND-V — the renderer-neutral animation runtime.
//!
//! Authored graphs compile to [`AnimGraphAsset`]: a versioned, UI-neutral node
//! array. Evaluation ends at [`Pose`]. The renderer still sees only palette
//! matrices, preserving Seam 7.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use glam::{Quat, Vec2, Vec3};

use crate::{JointIndex, Pose, Skeleton, SkeletonId};

// ── time, clips and sync tracks ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

impl<T> Keyframe<T> {
    #[must_use]
    pub const fn new(time: f32, value: T) -> Self {
        Self { time, value }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformTrack {
    pub joint: JointIndex,
    pub translation: Vec<Keyframe<Vec3>>,
    pub rotation: Vec<Keyframe<Quat>>,
    pub scale: Vec<Keyframe<Vec3>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    InvalidDuration,
    NonFiniteElapsed,
    NonFiniteScale,
    StationaryPlayback,
}

/// Validated playback policy. Private fields prevent a NaN scale from being
/// introduced after construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    looping: bool,
    time_scale: f32,
}

impl Playback {
    pub const LOOPING: Self = Self {
        looping: true,
        time_scale: 1.0,
    };
    pub const ONCE: Self = Self {
        looping: false,
        time_scale: 1.0,
    };

    pub fn new(looping: bool, time_scale: f32) -> Result<Self, TimeError> {
        if !time_scale.is_finite() {
            return Err(TimeError::NonFiniteScale);
        }
        Ok(Self {
            looping,
            time_scale,
        })
    }

    #[must_use]
    pub const fn looping(self) -> bool {
        self.looping
    }

    #[must_use]
    pub const fn time_scale(self) -> f32 {
        self.time_scale
    }

    pub fn local_time(self, elapsed: f32, duration: f32) -> Result<f32, TimeError> {
        if !elapsed.is_finite() {
            return Err(TimeError::NonFiniteElapsed);
        }
        if !duration.is_finite() || duration <= 0.0 {
            return Err(TimeError::InvalidDuration);
        }
        let scaled = elapsed * self.time_scale;
        if !scaled.is_finite() {
            return Err(TimeError::NonFiniteElapsed);
        }
        Ok(if self.looping {
            scaled.rem_euclid(duration)
        } else {
            scaled.clamp(0.0, duration)
        })
    }

    fn elapsed_for_local(self, local: f32, duration: f32, near: f32) -> Result<f32, TimeError> {
        if !local.is_finite() || !near.is_finite() {
            return Err(TimeError::NonFiniteElapsed);
        }
        if self.time_scale == 0.0 {
            return Err(TimeError::StationaryPlayback);
        }
        let base = local / self.time_scale;
        if !self.looping {
            return Ok(base);
        }
        let period = duration / self.time_scale.abs();
        Ok(base + ((near - base) / period).round() * period)
    }
}

impl Default for Playback {
    fn default() -> Self {
        Self::LOOPING
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SyncMarker {
    pub name: String,
    pub time: f32,
}

impl SyncMarker {
    #[must_use]
    pub fn new(name: impl Into<String>, time: f32) -> Self {
        Self {
            name: name.into(),
            time,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncError {
    EmptyName,
    InvalidDuration,
    TooFewMarkers,
    InvalidMarker,
    DuplicateMarker,
    NonFiniteTime,
    NonFinitePhase,
}

/// A validated semantic cycle. Duration is owned by the track, so mapping
/// cannot accidentally use a different or invalid duration.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncTrack {
    name: String,
    duration: f32,
    markers: Vec<SyncMarker>,
}

impl SyncTrack {
    pub fn new(
        name: impl Into<String>,
        duration: f32,
        markers: Vec<SyncMarker>,
    ) -> Result<Self, SyncError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(SyncError::EmptyName);
        }
        if !duration.is_finite() || duration <= 0.0 {
            return Err(SyncError::InvalidDuration);
        }
        if markers.len() < 2 {
            return Err(SyncError::TooFewMarkers);
        }
        let mut previous = -1.0;
        let mut names = HashSet::new();
        for marker in &markers {
            if marker.name.trim().is_empty()
                || !marker.time.is_finite()
                || marker.time < 0.0
                || marker.time >= duration
                || marker.time <= previous
            {
                return Err(SyncError::InvalidMarker);
            }
            if !names.insert(marker.name.as_str()) {
                return Err(SyncError::DuplicateMarker);
            }
            previous = marker.time;
        }
        Ok(Self {
            name,
            duration,
            markers,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    #[must_use]
    pub fn markers(&self) -> &[SyncMarker] {
        &self.markers
    }

    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.markers.len() == other.markers.len()
            && self
                .markers
                .iter()
                .zip(&other.markers)
                .all(|(a, b)| a.name == b.name)
    }

    pub fn phase_at(&self, time: f32) -> Result<f32, SyncError> {
        if !time.is_finite() {
            return Err(SyncError::NonFiniteTime);
        }
        let time = time.rem_euclid(self.duration);
        let count = self.markers.len();
        let index = self
            .markers
            .iter()
            .rposition(|marker| marker.time <= time)
            .unwrap_or(count - 1);
        let start = self.markers[index].time;
        let end = if index + 1 < count {
            self.markers[index + 1].time
        } else {
            self.markers[0].time + self.duration
        };
        let sample = if time < start {
            time + self.duration
        } else {
            time
        };
        Ok((index as f32 + (sample - start) / (end - start)) / count as f32)
    }

    pub fn time_at_phase(&self, phase: f32) -> Result<f32, SyncError> {
        if !phase.is_finite() {
            return Err(SyncError::NonFinitePhase);
        }
        let scaled = phase.rem_euclid(1.0) * self.markers.len() as f32;
        let index = (scaled.floor() as usize).min(self.markers.len() - 1);
        let fraction = scaled - index as f32;
        let start = self.markers[index].time;
        let end = if index + 1 < self.markers.len() {
            self.markers[index + 1].time
        } else {
            self.markers[0].time + self.duration
        };
        Ok((start + (end - start) * fraction).rem_euclid(self.duration))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClipId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    InvalidDuration,
    DuplicateJoint,
    JointOutOfRange,
    InvalidKeyframes,
    DuplicateSyncTrack,
    SyncDurationMismatch,
    SkeletonMismatch,
    Time(TimeError),
    Sync(SyncError),
}

impl From<TimeError> for ClipError {
    fn from(value: TimeError) -> Self {
        Self::Time(value)
    }
}

impl From<SyncError> for ClipError {
    fn from(value: SyncError) -> Self {
        Self::Sync(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationClip {
    id: ClipId,
    skeleton: SkeletonId,
    duration: f32,
    tracks: Vec<TransformTrack>,
    sync_tracks: Vec<SyncTrack>,
}

impl AnimationClip {
    /// Construct against the actual skeleton, so bad joint indices fail here.
    pub fn new(
        id: ClipId,
        skeleton: &Skeleton,
        duration: f32,
        tracks: Vec<TransformTrack>,
        sync_tracks: Vec<SyncTrack>,
    ) -> Result<Self, ClipError> {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(ClipError::InvalidDuration);
        }
        let mut joints = HashSet::new();
        for track in &tracks {
            if track.joint as usize >= skeleton.len() {
                return Err(ClipError::JointOutOfRange);
            }
            if !joints.insert(track.joint) {
                return Err(ClipError::DuplicateJoint);
            }
            if !valid_vec3_keys(&track.translation, duration)
                || !valid_quat_keys(&track.rotation, duration)
                || !valid_vec3_keys(&track.scale, duration)
            {
                return Err(ClipError::InvalidKeyframes);
            }
        }
        let mut names = HashSet::new();
        for sync in &sync_tracks {
            if sync.duration != duration {
                return Err(ClipError::SyncDurationMismatch);
            }
            if !names.insert(sync.name.as_str()) {
                return Err(ClipError::DuplicateSyncTrack);
            }
        }
        Ok(Self {
            id,
            skeleton: skeleton.id(),
            duration,
            tracks,
            sync_tracks,
        })
    }

    #[must_use]
    pub fn id(&self) -> ClipId {
        self.id
    }

    #[must_use]
    pub fn skeleton(&self) -> SkeletonId {
        self.skeleton
    }

    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    #[must_use]
    pub fn tracks(&self) -> &[TransformTrack] {
        &self.tracks
    }

    #[must_use]
    pub fn sync_tracks(&self) -> &[SyncTrack] {
        &self.sync_tracks
    }

    #[must_use]
    pub fn sync_track(&self, name: &str) -> Option<&SyncTrack> {
        self.sync_tracks.iter().find(|track| track.name == name)
    }

    pub fn local_time(&self, elapsed: f32, playback: Playback) -> Result<f32, ClipError> {
        playback
            .local_time(elapsed, self.duration)
            .map_err(ClipError::Time)
    }

    pub fn sample(
        &self,
        skeleton: &Skeleton,
        elapsed: f32,
        playback: Playback,
    ) -> Result<Pose, ClipError> {
        self.sample_local(skeleton, self.local_time(elapsed, playback)?)
    }

    pub fn sample_local(&self, skeleton: &Skeleton, time: f32) -> Result<Pose, ClipError> {
        if skeleton.id() != self.skeleton {
            return Err(ClipError::SkeletonMismatch);
        }
        if !time.is_finite() {
            return Err(ClipError::Time(TimeError::NonFiniteElapsed));
        }
        let time = time.clamp(0.0, self.duration);
        let mut pose = skeleton.rest_pose();
        for track in &self.tracks {
            let local = &mut pose.local[track.joint as usize];
            local.translation = sample_vec3(&track.translation, time, local.translation);
            local.rotation = sample_quat(&track.rotation, time, local.rotation);
            local.scale = sample_vec3(&track.scale, time, local.scale);
        }
        Ok(pose)
    }
}

fn valid_times<T>(keys: &[Keyframe<T>], duration: f32) -> bool {
    let mut previous = -1.0;
    keys.iter().all(|key| {
        let valid =
            key.time.is_finite() && key.time >= 0.0 && key.time <= duration && key.time > previous;
        previous = key.time;
        valid
    })
}

fn valid_vec3_keys(keys: &[Keyframe<Vec3>], duration: f32) -> bool {
    valid_times(keys, duration) && keys.iter().all(|key| key.value.is_finite())
}

fn valid_quat_keys(keys: &[Keyframe<Quat>], duration: f32) -> bool {
    valid_times(keys, duration)
        && keys
            .iter()
            .all(|key| key.value.is_finite() && key.value.length_squared() > 1e-8)
}

fn key_segment<T>(keys: &[Keyframe<T>], time: f32) -> Option<(usize, usize, f32)> {
    if keys.is_empty() {
        return None;
    }
    if time <= keys[0].time {
        return Some((0, 0, 0.0));
    }
    let last = keys.len() - 1;
    if time >= keys[last].time {
        return Some((last, last, 0.0));
    }
    let upper = keys.partition_point(|key| key.time <= time);
    let lower = upper - 1;
    Some((
        lower,
        upper,
        (time - keys[lower].time) / (keys[upper].time - keys[lower].time),
    ))
}

fn sample_vec3(keys: &[Keyframe<Vec3>], time: f32, fallback: Vec3) -> Vec3 {
    key_segment(keys, time).map_or(fallback, |(a, b, t)| keys[a].value.lerp(keys[b].value, t))
}

fn sample_quat(keys: &[Keyframe<Quat>], time: f32, fallback: Quat) -> Quat {
    key_segment(keys, time).map_or(fallback, |(a, b, t)| {
        keys[a]
            .value
            .normalize()
            .slerp(keys[b].value.normalize(), t)
    })
}

// ── triangulated blend spaces ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriangulationError {
    TooFewPoints,
    NonFinitePoint,
    DuplicatePoint,
    NoTriangles,
    BadIndex,
    DegenerateTriangle,
    UnusedPoint,
    NonManifoldEdge,
    Disconnected,
    InvalidBoundary,
    CrossingEdges,
    OverlappingTriangles,
    NonFiniteSample,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Triangulation2D {
    points: Vec<Vec2>,
    triangles: Vec<[u16; 3]>,
    boundary: Vec<(usize, usize, usize)>,
}

impl Triangulation2D {
    pub fn new(points: Vec<Vec2>, triangles: Vec<[u16; 3]>) -> Result<Self, TriangulationError> {
        if points.len() < 3 {
            return Err(TriangulationError::TooFewPoints);
        }
        if points.iter().any(|point| !point.is_finite()) {
            return Err(TriangulationError::NonFinitePoint);
        }
        if points
            .iter()
            .enumerate()
            .any(|(index, point)| points[index + 1..].iter().any(|other| point == other))
        {
            return Err(TriangulationError::DuplicatePoint);
        }
        if triangles.is_empty() {
            return Err(TriangulationError::NoTriangles);
        }
        let mut referenced = vec![false; points.len()];
        let mut edges: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for (triangle_index, triangle) in triangles.iter().enumerate() {
            let [a, b, c] = triangle.map(usize::from);
            if a >= points.len() || b >= points.len() || c >= points.len() {
                return Err(TriangulationError::BadIndex);
            }
            if a == b || b == c || c == a || cross(points[a], points[b], points[c]).abs() < 1e-6 {
                return Err(TriangulationError::DegenerateTriangle);
            }
            for item in [a, b, c] {
                referenced[item] = true;
            }
            for (from, to) in [(a, b), (b, c), (c, a)] {
                edges
                    .entry((from.min(to), from.max(to)))
                    .or_default()
                    .push(triangle_index);
            }
        }
        if referenced.iter().any(|used| !used) {
            return Err(TriangulationError::UnusedPoint);
        }
        if edges.values().any(|owners| owners.len() > 2) {
            return Err(TriangulationError::NonManifoldEdge);
        }
        for first in 0..triangles.len() {
            let first_indices = triangles[first].map(usize::from);
            for second_indices in triangles[first + 1..]
                .iter()
                .map(|triangle| triangle.map(usize::from))
            {
                let shared: Vec<usize> = first_indices
                    .iter()
                    .copied()
                    .filter(|index| second_indices.contains(index))
                    .collect();
                if shared.len() == 3 {
                    return Err(TriangulationError::OverlappingTriangles);
                }
                if shared.len() == 2 {
                    let first_opposite = first_indices
                        .iter()
                        .copied()
                        .find(|index| !shared.contains(index))
                        .expect("validated triangle has three distinct vertices");
                    let second_opposite = second_indices
                        .iter()
                        .copied()
                        .find(|index| !shared.contains(index))
                        .expect("validated triangle has three distinct vertices");
                    let first_side =
                        cross(points[shared[0]], points[shared[1]], points[first_opposite]);
                    let second_side = cross(
                        points[shared[0]],
                        points[shared[1]],
                        points[second_opposite],
                    );
                    if first_side * second_side >= 0.0 {
                        return Err(TriangulationError::OverlappingTriangles);
                    }
                    continue;
                }
                let first_contains_second = second_indices.iter().copied().any(|index| {
                    !shared.contains(&index)
                        && point_strictly_inside_triangle(
                            points[index],
                            points[first_indices[0]],
                            points[first_indices[1]],
                            points[first_indices[2]],
                        )
                });
                let second_contains_first = first_indices.iter().copied().any(|index| {
                    !shared.contains(&index)
                        && point_strictly_inside_triangle(
                            points[index],
                            points[second_indices[0]],
                            points[second_indices[1]],
                            points[second_indices[2]],
                        )
                });
                if first_contains_second || second_contains_first {
                    return Err(TriangulationError::OverlappingTriangles);
                }
            }
        }
        let unique_edges: Vec<(usize, usize)> = edges.keys().copied().collect();
        for (index, &(a, b)) in unique_edges.iter().enumerate() {
            for &(c, d) in &unique_edges[index + 1..] {
                if [a, b].contains(&c) || [a, b].contains(&d) {
                    continue;
                }
                if segments_cross(points[a], points[b], points[c], points[d]) {
                    return Err(TriangulationError::CrossingEdges);
                }
            }
        }
        let mut adjacency = vec![Vec::new(); triangles.len()];
        for owners in edges.values().filter(|owners| owners.len() == 2) {
            adjacency[owners[0]].push(owners[1]);
            adjacency[owners[1]].push(owners[0]);
        }
        let mut seen = vec![false; triangles.len()];
        let mut queue = VecDeque::from([0]);
        while let Some(index) = queue.pop_front() {
            if seen[index] {
                continue;
            }
            seen[index] = true;
            queue.extend(adjacency[index].iter().copied());
        }
        if seen.iter().any(|seen| !seen) {
            return Err(TriangulationError::Disconnected);
        }
        let boundary: Vec<(usize, usize, usize)> = edges
            .iter()
            .filter(|(_, owners)| owners.len() == 1)
            .map(|(&(a, b), owners)| (a, b, owners[0]))
            .collect();
        let mut degrees = vec![0usize; points.len()];
        for &(a, b, _) in &boundary {
            degrees[a] += 1;
            degrees[b] += 1;
        }
        if boundary.len() < 3
            || degrees
                .iter()
                .enumerate()
                .any(|(index, degree)| referenced[index] && *degree != 0 && *degree != 2)
        {
            return Err(TriangulationError::InvalidBoundary);
        }
        // A valid authored blend space covers the point-set hull exactly.
        // Matching only vertex degrees admits an inner boundary cycle (a hole)
        // and concave outer boundaries, making hull clamping order-dependent.
        let hull = convex_hull_indices(&points);
        if hull.len() < 3 {
            return Err(TriangulationError::InvalidBoundary);
        }
        let authored_edges: HashSet<(usize, usize)> = boundary
            .iter()
            .map(|&(a, b, _)| (a.min(b), a.max(b)))
            .collect();
        let hull_edges: HashSet<(usize, usize)> = hull
            .iter()
            .copied()
            .zip(hull.iter().copied().cycle().skip(1))
            .take(hull.len())
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        if authored_edges != hull_edges {
            return Err(TriangulationError::InvalidBoundary);
        }
        Ok(Self {
            points,
            triangles,
            boundary,
        })
    }

    #[must_use]
    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    #[must_use]
    pub fn triangles(&self) -> &[[u16; 3]] {
        &self.triangles
    }

    /// Inside the hull, use one authored triangle. Outside, project to the
    /// nearest hull edge so only local hull samples contribute.
    pub fn weights(&self, point: Vec2) -> Result<([usize; 3], [f32; 3]), TriangulationError> {
        if !point.is_finite() {
            return Err(TriangulationError::NonFiniteSample);
        }
        for triangle in &self.triangles {
            let indices = triangle.map(usize::from);
            let weights = barycentric(
                point,
                self.points[indices[0]],
                self.points[indices[1]],
                self.points[indices[2]],
            );
            if weights.iter().all(|weight| *weight >= -1e-5) {
                return Ok((indices, normalise_weights(weights)));
            }
        }
        let (_, projected, owner) = self
            .boundary
            .iter()
            .map(|&(a, b, owner)| {
                let projected = project_segment(point, self.points[a], self.points[b]);
                (point.distance_squared(projected), projected, owner)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("validated triangulation has a boundary");
        let indices = self.triangles[owner].map(usize::from);
        Ok((
            indices,
            normalise_weights(barycentric(
                projected,
                self.points[indices[0]],
                self.points[indices[1]],
                self.points[indices[2]],
            )),
        ))
    }
}

fn convex_hull_indices(points: &[Vec2]) -> Vec<usize> {
    let mut ordered: Vec<usize> = (0..points.len()).collect();
    ordered.sort_by(|&a, &b| {
        points[a]
            .x
            .total_cmp(&points[b].x)
            .then_with(|| points[a].y.total_cmp(&points[b].y))
            .then_with(|| a.cmp(&b))
    });
    let mut lower = Vec::new();
    for &index in &ordered {
        while lower.len() >= 2
            && cross(
                points[lower[lower.len() - 2]],
                points[lower[lower.len() - 1]],
                points[index],
            ) < 0.0
        {
            lower.pop();
        }
        lower.push(index);
    }
    let mut upper = Vec::new();
    for &index in ordered.iter().rev() {
        while upper.len() >= 2
            && cross(
                points[upper[upper.len() - 2]],
                points[upper[upper.len() - 1]],
                points[index],
            ) < 0.0
        {
            upper.pop();
        }
        upper.push(index);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    (ab_c * ab_d < -1e-8 && cd_a * cd_b < -1e-8)
        || (ab_c.abs() < 1e-6 && point_on_segment(c, a, b))
        || (ab_d.abs() < 1e-6 && point_on_segment(d, a, b))
        || (cd_a.abs() < 1e-6 && point_on_segment(a, c, d))
        || (cd_b.abs() < 1e-6 && point_on_segment(b, c, d))
}

fn point_on_segment(point: Vec2, a: Vec2, b: Vec2) -> bool {
    let t = (point - a).dot(b - a) / (b - a).length_squared();
    (-1e-6..=1.0 + 1e-6).contains(&t)
}

fn point_strictly_inside_triangle(point: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    barycentric(point, a, b, c)
        .iter()
        .all(|weight| *weight > 1e-6)
}

fn project_segment(point: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
    a.lerp(b, t)
}

fn barycentric(point: Vec2, a: Vec2, b: Vec2, c: Vec2) -> [f32; 3] {
    let area = cross(a, b, c);
    [
        cross(point, b, c) / area,
        cross(a, point, c) / area,
        cross(a, b, point) / area,
    ]
}

fn normalise_weights(mut weights: [f32; 3]) -> [f32; 3] {
    for weight in &mut weights {
        *weight = weight.max(0.0);
    }
    let total = weights.iter().sum::<f32>();
    weights.map(|weight| weight / total)
}

// ── layers and standalone blend evaluators ─────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct BoneMask {
    skeleton: SkeletonId,
    weights: Vec<f32>,
}

impl BoneMask {
    pub fn new(skeleton: &Skeleton, weights: Vec<f32>) -> Result<Self, LayerError> {
        if weights.len() != skeleton.len() {
            return Err(LayerError::MaskMismatch);
        }
        if weights.iter().any(|weight| !weight.is_finite()) {
            return Err(LayerError::NonFiniteWeight);
        }
        Ok(Self {
            skeleton: skeleton.id(),
            weights: weights
                .into_iter()
                .map(|weight| weight.clamp(0.0, 1.0))
                .collect(),
        })
    }

    #[must_use]
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PoseLayer<'a> {
    pub pose: &'a Pose,
    pub weight: f32,
    pub mask: Option<&'a BoneMask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerError {
    PoseMismatch,
    MaskMismatch,
    NonFiniteWeight,
}

pub fn layer_poses(base: &Pose, layers: &[PoseLayer<'_>]) -> Result<Pose, LayerError> {
    let mut output = base.clone();
    for layer in layers {
        if !layer.weight.is_finite() {
            return Err(LayerError::NonFiniteWeight);
        }
        if layer.pose.skeleton != output.skeleton || layer.pose.local.len() != output.local.len() {
            return Err(LayerError::PoseMismatch);
        }
        if let Some(mask) = layer.mask {
            if mask.skeleton != output.skeleton || mask.weights.len() != output.local.len() {
                return Err(LayerError::MaskMismatch);
            }
        }
        for index in 0..output.local.len() {
            let mask = layer.mask.map_or(1.0, |mask| mask.weights[index]);
            output.local[index] = output.local[index].blend(
                layer.pose.local[index],
                (layer.weight * mask).clamp(0.0, 1.0),
            );
        }
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
pub struct BlendSample1D<'a> {
    pub position: f32,
    pub clip: &'a AnimationClip,
    pub playback: Playback,
}

#[derive(Clone, Copy, Debug)]
pub struct BlendSample2D<'a> {
    pub position: Vec2,
    pub clip: &'a AnimationClip,
    pub playback: Playback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendError {
    Empty,
    NonFinitePosition,
    Unordered,
    SkeletonMismatch,
    InvalidLeader,
    MissingSyncTrack,
    IncompatibleSyncTracks,
    Triangulation(TriangulationError),
    Clip(ClipError),
}

impl From<ClipError> for BlendError {
    fn from(value: ClipError) -> Self {
        Self::Clip(value)
    }
}

impl From<SyncError> for BlendError {
    fn from(value: SyncError) -> Self {
        Self::Clip(ClipError::Sync(value))
    }
}

impl From<TriangulationError> for BlendError {
    fn from(value: TriangulationError) -> Self {
        Self::Triangulation(value)
    }
}

#[derive(Clone, Debug)]
pub struct Blend1D<'a> {
    samples: Vec<BlendSample1D<'a>>,
    sync_leader: usize,
}

impl<'a> Blend1D<'a> {
    pub fn new(samples: Vec<BlendSample1D<'a>>, sync_leader: usize) -> Result<Self, BlendError> {
        validate_1d_samples(&samples, sync_leader)?;
        Ok(Self {
            samples,
            sync_leader,
        })
    }

    pub fn sample(
        &self,
        skeleton: &Skeleton,
        parameter: f32,
        elapsed: f32,
        sync_track: Option<&str>,
    ) -> Result<Pose, BlendError> {
        if !parameter.is_finite() {
            return Err(BlendError::NonFinitePosition);
        }
        let phase = sync_track
            .map(|name| standalone_phase(&self.samples, self.sync_leader, elapsed, name))
            .transpose()?;
        let upper = self
            .samples
            .partition_point(|sample| sample.position <= parameter);
        if upper == 0 {
            return sample_standalone_1d(skeleton, self.samples[0], elapsed, sync_track, phase);
        }
        if upper == self.samples.len() {
            return sample_standalone_1d(
                skeleton,
                self.samples[self.samples.len() - 1],
                elapsed,
                sync_track,
                phase,
            );
        }
        let left = self.samples[upper - 1];
        let right = self.samples[upper];
        let weight = (parameter - left.position) / (right.position - left.position);
        let a = sample_standalone_1d(skeleton, left, elapsed, sync_track, phase)?;
        let b = sample_standalone_1d(skeleton, right, elapsed, sync_track, phase)?;
        Ok(blend_pose(&a, &b, weight))
    }
}

#[derive(Clone, Debug)]
pub struct Blend2D<'a> {
    samples: Vec<BlendSample2D<'a>>,
    triangulation: Triangulation2D,
    sync_leader: usize,
}

impl<'a> Blend2D<'a> {
    pub fn new(
        samples: Vec<BlendSample2D<'a>>,
        triangles: Vec<[u16; 3]>,
        sync_leader: usize,
    ) -> Result<Self, BlendError> {
        if samples.is_empty() {
            return Err(BlendError::Empty);
        }
        if sync_leader >= samples.len() {
            return Err(BlendError::InvalidLeader);
        }
        if samples
            .iter()
            .any(|sample| sample.clip.skeleton != samples[0].clip.skeleton)
        {
            return Err(BlendError::SkeletonMismatch);
        }
        let triangulation = Triangulation2D::new(
            samples.iter().map(|sample| sample.position).collect(),
            triangles,
        )?;
        Ok(Self {
            samples,
            triangulation,
            sync_leader,
        })
    }

    pub fn sample(
        &self,
        skeleton: &Skeleton,
        parameter: Vec2,
        elapsed: f32,
        sync_track: Option<&str>,
    ) -> Result<Pose, BlendError> {
        let (indices, weights) = self.triangulation.weights(parameter)?;
        let phase = sync_track
            .map(|name| standalone_phase_2d(&self.samples, self.sync_leader, elapsed, name))
            .transpose()?;
        let mut poses = Vec::with_capacity(3);
        for index in indices {
            let sample = self.samples[index];
            poses.push(sample_standalone(
                skeleton,
                sample.clip,
                sample.playback,
                elapsed,
                sync_track,
                phase,
            )?);
        }
        Ok(blend_three(&poses, weights))
    }
}

fn validate_1d_samples(samples: &[BlendSample1D<'_>], leader: usize) -> Result<(), BlendError> {
    if samples.is_empty() {
        return Err(BlendError::Empty);
    }
    if leader >= samples.len() {
        return Err(BlendError::InvalidLeader);
    }
    if samples.iter().any(|sample| !sample.position.is_finite()) {
        return Err(BlendError::NonFinitePosition);
    }
    if samples
        .windows(2)
        .any(|pair| pair[0].position >= pair[1].position)
    {
        return Err(BlendError::Unordered);
    }
    if samples
        .iter()
        .any(|sample| sample.clip.skeleton != samples[0].clip.skeleton)
    {
        return Err(BlendError::SkeletonMismatch);
    }
    Ok(())
}

fn standalone_phase(
    samples: &[BlendSample1D<'_>],
    leader: usize,
    elapsed: f32,
    name: &str,
) -> Result<f32, BlendError> {
    standalone_phase_common(
        samples[leader].clip,
        samples[leader].playback,
        elapsed,
        name,
        samples.iter().map(|sample| sample.clip),
    )
}

fn standalone_phase_2d(
    samples: &[BlendSample2D<'_>],
    leader: usize,
    elapsed: f32,
    name: &str,
) -> Result<f32, BlendError> {
    standalone_phase_common(
        samples[leader].clip,
        samples[leader].playback,
        elapsed,
        name,
        samples.iter().map(|sample| sample.clip),
    )
}

fn standalone_phase_common<'a>(
    leader: &AnimationClip,
    playback: Playback,
    elapsed: f32,
    name: &str,
    clips: impl Iterator<Item = &'a AnimationClip>,
) -> Result<f32, BlendError> {
    let track = leader
        .sync_track(name)
        .ok_or(BlendError::MissingSyncTrack)?;
    if clips.into_iter().any(|clip| {
        clip.sync_track(name)
            .is_none_or(|other| !track.is_compatible_with(other))
    }) {
        return Err(BlendError::IncompatibleSyncTracks);
    }
    Ok(track.phase_at(leader.local_time(elapsed, playback)?)?)
}

fn sample_standalone_1d(
    skeleton: &Skeleton,
    sample: BlendSample1D<'_>,
    elapsed: f32,
    sync_track: Option<&str>,
    phase: Option<f32>,
) -> Result<Pose, BlendError> {
    sample_standalone(
        skeleton,
        sample.clip,
        sample.playback,
        elapsed,
        sync_track,
        phase,
    )
}

fn sample_standalone(
    skeleton: &Skeleton,
    clip: &AnimationClip,
    playback: Playback,
    elapsed: f32,
    sync_track: Option<&str>,
    phase: Option<f32>,
) -> Result<Pose, BlendError> {
    let time = match (sync_track, phase) {
        (Some(name), Some(phase)) => clip
            .sync_track(name)
            .ok_or(BlendError::MissingSyncTrack)?
            .time_at_phase(phase)?,
        _ => clip.local_time(elapsed, playback)?,
    };
    Ok(clip.sample_local(skeleton, time)?)
}

fn blend_pose(a: &Pose, b: &Pose, weight: f32) -> Pose {
    let mut output = a.clone();
    let ok = a.blend_into(b, weight, &mut output);
    debug_assert!(ok);
    output
}

fn blend_three(poses: &[Pose], weights: [f32; 3]) -> Pose {
    let mut output = poses[0].clone();
    let mut total = weights[0];
    for index in 1..3 {
        if weights[index] <= 0.0 {
            continue;
        }
        if total <= 0.0 {
            output = poses[index].clone();
            total = weights[index];
        } else {
            output = blend_pose(
                &output,
                &poses[index],
                weights[index] / (total + weights[index]),
            );
            total += weights[index];
        }
    }
    output
}

// ── typed parameters ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParameterValue {
    Bool(bool),
    Float(f32),
    Int(i64),
    Trigger(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterType {
    Bool,
    Float,
    Int,
    Trigger,
}

impl ParameterValue {
    #[must_use]
    pub const fn ty(self) -> ParameterType {
        match self {
            Self::Bool(_) => ParameterType::Bool,
            Self::Float(_) => ParameterType::Float,
            Self::Int(_) => ParameterType::Int,
            Self::Trigger(_) => ParameterType::Trigger,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterDefinition {
    pub name: String,
    pub default: ParameterValue,
}

impl ParameterDefinition {
    #[must_use]
    pub fn new(name: impl Into<String>, default: ParameterValue) -> Self {
        Self {
            name: name.into(),
            default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParameterSchemaId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterError {
    EmptyName,
    Duplicate(String),
    Unknown(String),
    TypeMismatch(String),
    NonFinite(String),
    SchemaMismatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterSchema {
    id: ParameterSchemaId,
    definitions: BTreeMap<String, ParameterValue>,
}

impl ParameterSchema {
    pub fn new(
        id: ParameterSchemaId,
        definitions: Vec<ParameterDefinition>,
    ) -> Result<Self, ParameterError> {
        let mut values = BTreeMap::new();
        for definition in definitions {
            if definition.name.trim().is_empty() {
                return Err(ParameterError::EmptyName);
            }
            validate_parameter(&definition.name, definition.default)?;
            if values
                .insert(definition.name.clone(), definition.default)
                .is_some()
            {
                return Err(ParameterError::Duplicate(definition.name));
            }
        }
        Ok(Self {
            id,
            definitions: values,
        })
    }

    #[must_use]
    pub fn id(&self) -> ParameterSchemaId {
        self.id
    }

    #[must_use]
    pub fn ty(&self, name: &str) -> Option<ParameterType> {
        self.definitions.get(name).map(|value| value.ty())
    }

    #[must_use]
    pub fn instantiate(&self) -> ParameterSet {
        ParameterSet {
            schema: self.id,
            values: self.definitions.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterSet {
    schema: ParameterSchemaId,
    values: BTreeMap<String, ParameterValue>,
}

impl ParameterSet {
    #[must_use]
    pub fn schema(&self) -> ParameterSchemaId {
        self.schema
    }

    pub fn set(&mut self, name: &str, value: ParameterValue) -> Result<(), ParameterError> {
        validate_parameter(name, value)?;
        let previous = self
            .values
            .get(name)
            .ok_or_else(|| ParameterError::Unknown(name.to_string()))?;
        if previous.ty() != value.ty() {
            return Err(ParameterError::TypeMismatch(name.to_string()));
        }
        self.values.insert(name.to_string(), value);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<ParameterValue> {
        self.values.get(name).copied()
    }

    pub fn trigger(&mut self, name: &str) -> Result<(), ParameterError> {
        self.set(name, ParameterValue::Trigger(true))
    }

    fn consume_trigger(&mut self, name: &str) {
        if matches!(self.values.get(name), Some(ParameterValue::Trigger(true))) {
            self.values
                .insert(name.to_string(), ParameterValue::Trigger(false));
        }
    }
}

fn validate_parameter(name: &str, value: ParameterValue) -> Result<(), ParameterError> {
    if matches!(value, ParameterValue::Float(number) if !number.is_finite()) {
        return Err(ParameterError::NonFinite(name.to_string()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Bool {
        parameter: String,
        value: bool,
    },
    Float {
        parameter: String,
        op: CompareOp,
        value: f32,
    },
    Int {
        parameter: String,
        op: CompareOp,
        value: i64,
    },
    Trigger {
        parameter: String,
    },
}

impl Condition {
    fn expected_type(&self) -> ParameterType {
        match self {
            Self::Bool { .. } => ParameterType::Bool,
            Self::Float { .. } => ParameterType::Float,
            Self::Int { .. } => ParameterType::Int,
            Self::Trigger { .. } => ParameterType::Trigger,
        }
    }

    fn parameter(&self) -> &str {
        match self {
            Self::Bool { parameter, .. }
            | Self::Float { parameter, .. }
            | Self::Int { parameter, .. }
            | Self::Trigger { parameter } => parameter,
        }
    }

    fn validate(&self, schema: &ParameterSchema) -> bool {
        schema.ty(self.parameter()) == Some(self.expected_type())
            && !matches!(self, Self::Float { value, .. } if !value.is_finite())
    }

    #[must_use]
    pub fn matches(&self, parameters: &ParameterSet) -> bool {
        match self {
            Self::Bool { parameter, value } => {
                parameters.get(parameter) == Some(ParameterValue::Bool(*value))
            }
            Self::Float {
                parameter,
                op,
                value,
            } => {
                matches!(parameters.get(parameter), Some(ParameterValue::Float(actual)) if compare_f32(actual, *value, *op))
            }
            Self::Int {
                parameter,
                op,
                value,
            } => {
                matches!(parameters.get(parameter), Some(ParameterValue::Int(actual)) if compare_i64(actual, *value, *op))
            }
            Self::Trigger { parameter } => {
                parameters.get(parameter) == Some(ParameterValue::Trigger(true))
            }
        }
    }
}

fn compare_f32(a: f32, b: f32, op: CompareOp) -> bool {
    match op {
        CompareOp::Equal => a == b,
        CompareOp::NotEqual => a != b,
        CompareOp::Less => a < b,
        CompareOp::LessEqual => a <= b,
        CompareOp::Greater => a > b,
        CompareOp::GreaterEqual => a >= b,
    }
}

fn compare_i64(a: i64, b: i64, op: CompareOp) -> bool {
    match op {
        CompareOp::Equal => a == b,
        CompareOp::NotEqual => a != b,
        CompareOp::Less => a < b,
        CompareOp::LessEqual => a <= b,
        CompareOp::Greater => a > b,
        CompareOp::GreaterEqual => a >= b,
    }
}

// ── compiled graph asset ───────────────────────────────────────────────────

pub const ANIM_GRAPH_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnimNodeId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct NodeBlendSample1D {
    pub position: f32,
    pub node: AnimNodeId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeBlendSample2D {
    pub position: Vec2,
    pub node: AnimNodeId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerWeight {
    Constant(f32),
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeLayer {
    pub node: AnimNodeId,
    pub weight: LayerWeight,
    pub mask: Option<BoneMask>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnimNode {
    Clip {
        clip: ClipId,
        playback: Playback,
    },
    Blend1D {
        parameter: String,
        samples: Vec<NodeBlendSample1D>,
        sync_track: Option<String>,
        sync_leader: usize,
    },
    Blend2D {
        parameter_x: String,
        parameter_y: String,
        samples: Vec<NodeBlendSample2D>,
        triangles: Vec<[u16; 3]>,
        sync_track: Option<String>,
        sync_leader: usize,
    },
    Layer {
        base: AnimNodeId,
        layers: Vec<NodeLayer>,
    },
    Cache {
        source: AnimNodeId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimGraphError {
    InvalidVersion,
    DuplicateClip,
    SkeletonMismatch,
    EmptyNodes,
    UnknownNode,
    ForwardReference,
    UnknownClip,
    UnknownParameter(String),
    ParameterType(String),
    InvalidBlend,
    InvalidTriangulation(TriangulationError),
    InvalidLayer,
    MissingSyncTrack(String),
    IncompatibleSyncTracks(String),
    ParameterSchemaMismatch,
    NonFiniteTime,
    Clip(ClipError),
    Layer(LayerError),
}

impl From<ClipError> for AnimGraphError {
    fn from(value: ClipError) -> Self {
        Self::Clip(value)
    }
}

impl From<LayerError> for AnimGraphError {
    fn from(value: LayerError) -> Self {
        Self::Layer(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimGraphAsset {
    id: GraphId,
    version: u32,
    skeleton: SkeletonId,
    clips: Vec<AnimationClip>,
    nodes: Vec<AnimNode>,
    /// Validated once at asset construction; Blend2D evaluation is a hot path
    /// and must not rebuild authored topology every frame.
    blend2d: HashMap<AnimNodeId, Triangulation2D>,
    parameters: ParameterSchema,
    output: AnimNodeId,
}

impl AnimGraphAsset {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: GraphId,
        version: u32,
        skeleton: &Skeleton,
        clips: Vec<AnimationClip>,
        nodes: Vec<AnimNode>,
        parameters: ParameterSchema,
        output: AnimNodeId,
    ) -> Result<Self, AnimGraphError> {
        if version == 0 {
            return Err(AnimGraphError::InvalidVersion);
        }
        if clips.iter().any(|clip| clip.skeleton != skeleton.id()) {
            return Err(AnimGraphError::SkeletonMismatch);
        }
        let clip_ids: HashSet<ClipId> = clips.iter().map(AnimationClip::id).collect();
        if clip_ids.len() != clips.len() {
            return Err(AnimGraphError::DuplicateClip);
        }
        if nodes.is_empty() {
            return Err(AnimGraphError::EmptyNodes);
        }
        if output.0 as usize >= nodes.len() {
            return Err(AnimGraphError::UnknownNode);
        }
        let blend2d = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match node {
                AnimNode::Blend2D {
                    samples, triangles, ..
                } => Some(
                    Triangulation2D::new(
                        samples.iter().map(|sample| sample.position).collect(),
                        triangles.clone(),
                    )
                    .map(|triangulation| (AnimNodeId(index as u32), triangulation))
                    .map_err(AnimGraphError::InvalidTriangulation),
                ),
                _ => None,
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let asset = Self {
            id,
            version,
            skeleton: skeleton.id(),
            clips,
            nodes,
            blend2d,
            parameters,
            output,
        };
        for index in 0..asset.nodes.len() {
            asset.validate_node(index, skeleton)?;
        }
        Ok(asset)
    }

    #[must_use]
    pub fn id(&self) -> GraphId {
        self.id
    }

    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn skeleton(&self) -> SkeletonId {
        self.skeleton
    }

    #[must_use]
    pub fn nodes(&self) -> &[AnimNode] {
        &self.nodes
    }

    #[must_use]
    pub fn parameters(&self) -> &ParameterSchema {
        &self.parameters
    }

    #[must_use]
    pub fn output(&self) -> AnimNodeId {
        self.output
    }

    fn clip(&self, id: ClipId) -> Result<&AnimationClip, AnimGraphError> {
        self.clips
            .iter()
            .find(|clip| clip.id == id)
            .ok_or(AnimGraphError::UnknownClip)
    }

    fn validate_ref(&self, owner: usize, reference: AnimNodeId) -> Result<(), AnimGraphError> {
        let reference = reference.0 as usize;
        if reference >= self.nodes.len() {
            return Err(AnimGraphError::UnknownNode);
        }
        if reference >= owner {
            return Err(AnimGraphError::ForwardReference);
        }
        Ok(())
    }

    fn require_float(&self, parameter: &str) -> Result<(), AnimGraphError> {
        match self.parameters.ty(parameter) {
            Some(ParameterType::Float) => Ok(()),
            Some(_) => Err(AnimGraphError::ParameterType(parameter.to_string())),
            None => Err(AnimGraphError::UnknownParameter(parameter.to_string())),
        }
    }

    fn validate_node(&self, index: usize, skeleton: &Skeleton) -> Result<(), AnimGraphError> {
        match &self.nodes[index] {
            AnimNode::Clip { clip, .. } => {
                self.clip(*clip)?;
            }
            AnimNode::Blend1D {
                parameter,
                samples,
                sync_track,
                sync_leader,
            } => {
                self.require_float(parameter)?;
                if samples.is_empty()
                    || *sync_leader >= samples.len()
                    || samples.iter().any(|sample| !sample.position.is_finite())
                    || samples
                        .windows(2)
                        .any(|pair| pair[0].position >= pair[1].position)
                {
                    return Err(AnimGraphError::InvalidBlend);
                }
                for sample in samples {
                    self.validate_ref(index, sample.node)?;
                }
                if let Some(name) = sync_track {
                    self.validate_sync_nodes(
                        name,
                        samples.iter().map(|sample| sample.node),
                        samples[*sync_leader].node,
                    )?;
                }
            }
            AnimNode::Blend2D {
                parameter_x,
                parameter_y,
                samples,
                triangles: _,
                sync_track,
                sync_leader,
            } => {
                self.require_float(parameter_x)?;
                self.require_float(parameter_y)?;
                if samples.is_empty() || *sync_leader >= samples.len() {
                    return Err(AnimGraphError::InvalidBlend);
                }
                for sample in samples {
                    self.validate_ref(index, sample.node)?;
                }
                if !self.blend2d.contains_key(&AnimNodeId(index as u32)) {
                    return Err(AnimGraphError::InvalidBlend);
                }
                if let Some(name) = sync_track {
                    self.validate_sync_nodes(
                        name,
                        samples.iter().map(|sample| sample.node),
                        samples[*sync_leader].node,
                    )?;
                }
            }
            AnimNode::Layer { base, layers } => {
                self.validate_ref(index, *base)?;
                for layer in layers {
                    self.validate_ref(index, layer.node)?;
                    match &layer.weight {
                        LayerWeight::Constant(weight) if !weight.is_finite() => {
                            return Err(AnimGraphError::InvalidLayer);
                        }
                        LayerWeight::Parameter(parameter) => self.require_float(parameter)?,
                        LayerWeight::Constant(_) => {}
                    }
                    if let Some(mask) = &layer.mask {
                        if mask.skeleton != skeleton.id() || mask.weights.len() != skeleton.len() {
                            return Err(AnimGraphError::InvalidLayer);
                        }
                    }
                }
            }
            AnimNode::Cache { source } => self.validate_ref(index, *source)?,
        }
        Ok(())
    }

    fn validate_sync_nodes(
        &self,
        name: &str,
        nodes: impl Iterator<Item = AnimNodeId>,
        leader: AnimNodeId,
    ) -> Result<(), AnimGraphError> {
        let (leader_clip, _) = self.sync_anchor(leader, name)?;
        let leader_track = leader_clip
            .sync_track(name)
            .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?;
        for node in nodes {
            self.validate_forced_sync_node(node, name, leader_track, &mut HashSet::new())?;
        }
        Ok(())
    }

    /// Validate every branch that evaluation can visit while a parent forces a
    /// sync phase. Checking only a blend leader or layer base is insufficient:
    /// the forced phase is recursively propagated to all samples and layers.
    fn validate_forced_sync_node(
        &self,
        node: AnimNodeId,
        name: &str,
        expected: &SyncTrack,
        visited: &mut HashSet<AnimNodeId>,
    ) -> Result<(), AnimGraphError> {
        if !visited.insert(node) {
            return Ok(());
        }
        let item = self
            .nodes
            .get(node.0 as usize)
            .ok_or(AnimGraphError::UnknownNode)?;
        match item {
            AnimNode::Clip { clip, .. } => {
                let track = self
                    .clip(*clip)?
                    .sync_track(name)
                    .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?;
                if !expected.is_compatible_with(track) {
                    return Err(AnimGraphError::IncompatibleSyncTracks(name.to_string()));
                }
            }
            AnimNode::Blend1D { samples, .. } => {
                for sample in samples {
                    self.validate_forced_sync_node(sample.node, name, expected, visited)?;
                }
            }
            AnimNode::Blend2D { samples, .. } => {
                for sample in samples {
                    self.validate_forced_sync_node(sample.node, name, expected, visited)?;
                }
            }
            AnimNode::Layer { base, layers } => {
                self.validate_forced_sync_node(*base, name, expected, visited)?;
                for layer in layers {
                    self.validate_forced_sync_node(layer.node, name, expected, visited)?;
                }
            }
            AnimNode::Cache { source } => {
                self.validate_forced_sync_node(*source, name, expected, visited)?;
            }
        }
        Ok(())
    }

    fn validate_forced_sync_pair(
        &self,
        source: AnimNodeId,
        target: AnimNodeId,
        name: &str,
    ) -> Result<(), AnimGraphError> {
        let (source_clip, _) = self.sync_anchor(source, name)?;
        let expected = source_clip
            .sync_track(name)
            .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?;
        self.validate_forced_sync_node(source, name, expected, &mut HashSet::new())?;
        self.validate_forced_sync_node(target, name, expected, &mut HashSet::new())
    }

    fn sync_anchor(
        &self,
        node: AnimNodeId,
        name: &str,
    ) -> Result<(&AnimationClip, Playback), AnimGraphError> {
        let item = self
            .nodes
            .get(node.0 as usize)
            .ok_or(AnimGraphError::UnknownNode)?;
        match item {
            AnimNode::Clip { clip, playback } => {
                let clip = self.clip(*clip)?;
                clip.sync_track(name)
                    .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?;
                Ok((clip, *playback))
            }
            AnimNode::Blend1D {
                samples,
                sync_leader,
                ..
            } => self.sync_anchor(samples[*sync_leader].node, name),
            AnimNode::Blend2D {
                samples,
                sync_leader,
                ..
            } => self.sync_anchor(samples[*sync_leader].node, name),
            AnimNode::Layer { base, .. } => self.sync_anchor(*base, name),
            AnimNode::Cache { source } => self.sync_anchor(*source, name),
        }
    }

    fn sync_phase(
        &self,
        node: AnimNodeId,
        elapsed: f32,
        name: &str,
    ) -> Result<f32, AnimGraphError> {
        let (clip, playback) = self.sync_anchor(node, name)?;
        let time = clip.local_time(elapsed, playback)?;
        clip.sync_track(name)
            .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?
            .phase_at(time)
            .map_err(|error| AnimGraphError::Clip(ClipError::Sync(error)))
    }

    fn elapsed_at_phase(
        &self,
        node: AnimNodeId,
        phase: f32,
        name: &str,
        near: f32,
    ) -> Result<f32, AnimGraphError> {
        let (clip, playback) = self.sync_anchor(node, name)?;
        let local = clip
            .sync_track(name)
            .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?
            .time_at_phase(phase)
            .map_err(|error| AnimGraphError::Clip(ClipError::Sync(error)))?;
        playback
            .elapsed_for_local(local, clip.duration, near)
            .map_err(|error| AnimGraphError::Clip(ClipError::Time(error)))
    }

    pub fn evaluate(
        &self,
        skeleton: &Skeleton,
        parameters: &ParameterSet,
        elapsed: f32,
        generation: u64,
        cache: &mut PoseCache,
    ) -> Result<Pose, AnimGraphError> {
        self.evaluate_node_in_lane(
            self.output,
            skeleton,
            parameters,
            elapsed,
            generation,
            EvaluationLane::Output,
            cache,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_node(
        &self,
        node: AnimNodeId,
        skeleton: &Skeleton,
        parameters: &ParameterSet,
        elapsed: f32,
        generation: u64,
        cache: &mut PoseCache,
    ) -> Result<Pose, AnimGraphError> {
        self.evaluate_node_in_lane(
            node,
            skeleton,
            parameters,
            elapsed,
            generation,
            EvaluationLane::Output,
            cache,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_node_in_lane(
        &self,
        node: AnimNodeId,
        skeleton: &Skeleton,
        parameters: &ParameterSet,
        elapsed: f32,
        generation: u64,
        lane: EvaluationLane,
        cache: &mut PoseCache,
    ) -> Result<Pose, AnimGraphError> {
        let evaluation = EvaluationId { generation, lane };
        cache.begin_evaluation(self.id, self.version, evaluation);
        self.evaluate_node_inner(node, skeleton, parameters, elapsed, evaluation, cache, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_node_inner(
        &self,
        node: AnimNodeId,
        skeleton: &Skeleton,
        parameters: &ParameterSet,
        elapsed: f32,
        evaluation: EvaluationId,
        cache: &mut PoseCache,
        forced_phase: Option<(&str, f32)>,
    ) -> Result<Pose, AnimGraphError> {
        if skeleton.id() != self.skeleton {
            return Err(AnimGraphError::SkeletonMismatch);
        }
        if parameters.schema != self.parameters.id {
            return Err(AnimGraphError::ParameterSchemaMismatch);
        }
        if !elapsed.is_finite() {
            return Err(AnimGraphError::NonFiniteTime);
        }
        let item = self
            .nodes
            .get(node.0 as usize)
            .ok_or(AnimGraphError::UnknownNode)?;
        match item {
            AnimNode::Clip { clip, playback } => {
                let clip = self.clip(*clip)?;
                let time = if let Some((name, phase)) = forced_phase {
                    clip.sync_track(name)
                        .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?
                        .time_at_phase(phase)
                        .map_err(|error| AnimGraphError::Clip(ClipError::Sync(error)))?
                } else {
                    clip.local_time(elapsed, *playback)?
                };
                Ok(clip.sample_local(skeleton, time)?)
            }
            AnimNode::Blend1D {
                parameter,
                samples,
                sync_track,
                sync_leader,
            } => {
                let value = parameter_float(parameters, parameter)?;
                let phase = self.resolve_phase(
                    forced_phase,
                    sync_track.as_deref(),
                    samples[*sync_leader].node,
                    elapsed,
                )?;
                let upper = samples.partition_point(|sample| sample.position <= value);
                let (a, b, weight) = if upper == 0 {
                    (&samples[0], &samples[0], 0.0)
                } else if upper == samples.len() {
                    let last = &samples[samples.len() - 1];
                    (last, last, 0.0)
                } else {
                    let a = &samples[upper - 1];
                    let b = &samples[upper];
                    (a, b, (value - a.position) / (b.position - a.position))
                };
                let a_pose = self.evaluate_node_inner(
                    a.node, skeleton, parameters, elapsed, evaluation, cache, phase,
                )?;
                if a.node == b.node {
                    return Ok(a_pose);
                }
                let b_pose = self.evaluate_node_inner(
                    b.node, skeleton, parameters, elapsed, evaluation, cache, phase,
                )?;
                Ok(blend_pose(&a_pose, &b_pose, weight))
            }
            AnimNode::Blend2D {
                parameter_x,
                parameter_y,
                samples,
                triangles: _,
                sync_track,
                sync_leader,
            } => {
                let point = Vec2::new(
                    parameter_float(parameters, parameter_x)?,
                    parameter_float(parameters, parameter_y)?,
                );
                let triangulation = self
                    .blend2d
                    .get(&node)
                    .ok_or(AnimGraphError::InvalidBlend)?;
                let (indices, weights) = triangulation
                    .weights(point)
                    .map_err(AnimGraphError::InvalidTriangulation)?;
                let phase = self.resolve_phase(
                    forced_phase,
                    sync_track.as_deref(),
                    samples[*sync_leader].node,
                    elapsed,
                )?;
                let mut poses = Vec::with_capacity(3);
                for index in indices {
                    poses.push(self.evaluate_node_inner(
                        samples[index].node,
                        skeleton,
                        parameters,
                        elapsed,
                        evaluation,
                        cache,
                        phase,
                    )?);
                }
                Ok(blend_three(&poses, weights))
            }
            AnimNode::Layer { base, layers } => {
                let base = self.evaluate_node_inner(
                    *base,
                    skeleton,
                    parameters,
                    elapsed,
                    evaluation,
                    cache,
                    forced_phase,
                )?;
                let mut poses = Vec::with_capacity(layers.len());
                for layer in layers {
                    let pose = self.evaluate_node_inner(
                        layer.node,
                        skeleton,
                        parameters,
                        elapsed,
                        evaluation,
                        cache,
                        forced_phase,
                    )?;
                    let weight = match &layer.weight {
                        LayerWeight::Constant(weight) => *weight,
                        LayerWeight::Parameter(parameter) => {
                            parameter_float(parameters, parameter)?
                        }
                    };
                    poses.push((pose, weight, layer.mask.as_ref()));
                }
                let borrowed: Vec<PoseLayer<'_>> = poses
                    .iter()
                    .map(|(pose, weight, mask)| PoseLayer {
                        pose,
                        weight: *weight,
                        mask: *mask,
                    })
                    .collect();
                Ok(layer_poses(&base, &borrowed)?)
            }
            AnimNode::Cache { source } => {
                let key = PoseCacheKey::new(
                    evaluation.generation,
                    evaluation.lane,
                    self.id,
                    self.version,
                    node,
                );
                if let Some(pose) = cache.get(key) {
                    return Ok(pose.clone());
                }
                let pose = self.evaluate_node_inner(
                    *source,
                    skeleton,
                    parameters,
                    elapsed,
                    evaluation,
                    cache,
                    forced_phase,
                )?;
                cache.insert(key, pose.clone());
                Ok(pose)
            }
        }
    }

    fn resolve_phase<'a>(
        &self,
        forced: Option<(&'a str, f32)>,
        authored: Option<&'a str>,
        leader: AnimNodeId,
        elapsed: f32,
    ) -> Result<Option<(&'a str, f32)>, AnimGraphError> {
        if forced.is_some() {
            return Ok(forced);
        }
        authored
            .map(|name| {
                self.sync_phase(leader, elapsed, name)
                    .map(|phase| (name, phase))
            })
            .transpose()
    }
}

fn parameter_float(parameters: &ParameterSet, name: &str) -> Result<f32, AnimGraphError> {
    match parameters.get(name) {
        Some(ParameterValue::Float(value)) => Ok(value),
        Some(_) => Err(AnimGraphError::ParameterType(name.to_string())),
        None => Err(AnimGraphError::UnknownParameter(name.to_string())),
    }
}

// ── mandatory composite pose cache key ─────────────────────────────────────

/// A fixed evaluation lane keeps simultaneous state-machine samples distinct
/// without folding lane bits into caller generations. Consequently, no future
/// caller generation can alias a source or target transition sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvaluationLane {
    Output,
    StateSource,
    StateTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct EvaluationId {
    generation: u64,
    lane: EvaluationLane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PoseCacheKey {
    generation: u64,
    lane: EvaluationLane,
    graph: GraphId,
    graph_version: u32,
    node: AnimNodeId,
}

impl PoseCacheKey {
    #[must_use]
    pub const fn new(
        generation: u64,
        lane: EvaluationLane,
        graph: GraphId,
        graph_version: u32,
        node: AnimNodeId,
    ) -> Self {
        Self {
            generation,
            lane,
            graph,
            graph_version,
            node,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PoseCache {
    poses: HashMap<PoseCacheKey, Pose>,
}

impl PoseCache {
    #[must_use]
    pub fn get(&self, key: PoseCacheKey) -> Option<&Pose> {
        self.poses.get(&key)
    }

    pub fn insert(&mut self, key: PoseCacheKey, pose: Pose) -> Option<Pose> {
        self.poses.insert(key, pose)
    }

    /// Keep one generation per fixed lane and discard stale hot-reload
    /// versions. Normal graph evaluation calls this automatically, bounding
    /// retention to one generation's cache nodes per lane for each graph.
    fn begin_evaluation(&mut self, graph: GraphId, graph_version: u32, evaluation: EvaluationId) {
        self.poses.retain(|key, _| {
            if key.graph != graph {
                return true;
            }
            if key.graph_version != graph_version {
                return false;
            }
            key.lane != evaluation.lane || key.generation == evaluation.generation
        });
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.poses.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.poses.is_empty()
    }
}

// ── state machines over arbitrary pose nodes ───────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MachineId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationState {
    pub id: StateId,
    pub name: String,
    pub node: AnimNodeId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateTransition {
    pub from: StateId,
    pub to: StateId,
    pub conditions: Vec<Condition>,
    pub blend_seconds: f32,
    pub sync_track: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateMachine {
    id: MachineId,
    definition_version: u32,
    graph: GraphId,
    graph_version: u32,
    states: Vec<AnimationState>,
    transitions: Vec<StateTransition>,
    initial: StateId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateMachineError {
    InvalidVersion,
    NoStates,
    DuplicateState,
    UnknownState,
    UnknownNode,
    InvalidBlendTime,
    InvalidCondition,
    InvalidSyncTrack,
    ParameterSchemaMismatch,
    InvalidDelta,
    MachineMismatch,
    VersionMismatch,
    GraphMismatch,
    Graph(AnimGraphError),
}

impl From<AnimGraphError> for StateMachineError {
    fn from(value: AnimGraphError) -> Self {
        Self::Graph(value)
    }
}

impl StateMachine {
    pub fn new(
        id: MachineId,
        definition_version: u32,
        graph: &AnimGraphAsset,
        states: Vec<AnimationState>,
        transitions: Vec<StateTransition>,
        initial: StateId,
    ) -> Result<Self, StateMachineError> {
        if definition_version == 0 {
            return Err(StateMachineError::InvalidVersion);
        }
        if states.is_empty() {
            return Err(StateMachineError::NoStates);
        }
        let ids: HashSet<StateId> = states.iter().map(|state| state.id).collect();
        if ids.len() != states.len() {
            return Err(StateMachineError::DuplicateState);
        }
        if !ids.contains(&initial)
            || transitions
                .iter()
                .any(|transition| !ids.contains(&transition.from) || !ids.contains(&transition.to))
        {
            return Err(StateMachineError::UnknownState);
        }
        if states
            .iter()
            .any(|state| state.node.0 as usize >= graph.nodes.len())
        {
            return Err(StateMachineError::UnknownNode);
        }
        for transition in &transitions {
            if !transition.blend_seconds.is_finite() || transition.blend_seconds < 0.0 {
                return Err(StateMachineError::InvalidBlendTime);
            }
            if transition
                .conditions
                .iter()
                .any(|condition| !condition.validate(&graph.parameters))
            {
                return Err(StateMachineError::InvalidCondition);
            }
            if let Some(name) = &transition.sync_track {
                let source = states
                    .iter()
                    .find(|state| state.id == transition.from)
                    .ok_or(StateMachineError::UnknownState)?;
                let target = states
                    .iter()
                    .find(|state| state.id == transition.to)
                    .ok_or(StateMachineError::UnknownState)?;
                graph
                    .validate_forced_sync_pair(source.node, target.node, name)
                    .map_err(|_| StateMachineError::InvalidSyncTrack)?;
                let (source_clip, source_playback) = graph
                    .sync_anchor(source.node, name)
                    .map_err(|_| StateMachineError::InvalidSyncTrack)?;
                let (target_clip, target_playback) = graph
                    .sync_anchor(target.node, name)
                    .map_err(|_| StateMachineError::InvalidSyncTrack)?;
                if source_clip.sync_track(name).is_none()
                    || target_clip.sync_track(name).is_none()
                    || source_playback.time_scale == 0.0
                    || target_playback.time_scale == 0.0
                {
                    return Err(StateMachineError::InvalidSyncTrack);
                }
            }
        }
        Ok(Self {
            id,
            definition_version,
            graph: graph.id,
            graph_version: graph.version,
            states,
            transitions,
            initial,
        })
    }

    #[must_use]
    pub fn id(&self) -> MachineId {
        self.id
    }

    #[must_use]
    pub fn definition_version(&self) -> u32 {
        self.definition_version
    }

    #[must_use]
    pub fn states(&self) -> &[AnimationState] {
        &self.states
    }

    #[must_use]
    pub fn transitions(&self) -> &[StateTransition] {
        &self.transitions
    }

    fn state(&self, id: StateId) -> Result<&AnimationState, StateMachineError> {
        self.states
            .iter()
            .find(|state| state.id == id)
            .ok_or(StateMachineError::UnknownState)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveTransition {
    index: usize,
    source_time: f32,
    target_time: f32,
    elapsed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateMachinePlayer {
    machine: MachineId,
    machine_version: u32,
    graph: GraphId,
    graph_version: u32,
    current: StateId,
    state_time: f32,
    active: Option<ActiveTransition>,
}

impl StateMachinePlayer {
    #[must_use]
    pub fn new(machine: &StateMachine) -> Self {
        Self {
            machine: machine.id,
            machine_version: machine.definition_version,
            graph: machine.graph,
            graph_version: machine.graph_version,
            current: machine.initial,
            state_time: 0.0,
            active: None,
        }
    }

    #[must_use]
    pub fn current(&self) -> StateId {
        self.current
    }

    #[must_use]
    pub fn state_time(&self) -> f32 {
        self.state_time
    }

    #[must_use]
    pub fn is_transitioning(&self) -> bool {
        self.active.is_some()
    }

    fn validate_binding(
        &self,
        machine: &StateMachine,
        graph: &AnimGraphAsset,
    ) -> Result<(), StateMachineError> {
        if self.machine != machine.id {
            return Err(StateMachineError::MachineMismatch);
        }
        if self.machine_version != machine.definition_version {
            return Err(StateMachineError::VersionMismatch);
        }
        if self.graph != graph.id || machine.graph != graph.id {
            return Err(StateMachineError::GraphMismatch);
        }
        if self.graph_version != graph.version || machine.graph_version != graph.version {
            return Err(StateMachineError::VersionMismatch);
        }
        Ok(())
    }

    pub fn advance(
        &mut self,
        machine: &StateMachine,
        graph: &AnimGraphAsset,
        parameters: &mut ParameterSet,
        delta_seconds: f32,
    ) -> Result<(), StateMachineError> {
        self.validate_binding(machine, graph)?;
        if parameters.schema != graph.parameters.id {
            return Err(StateMachineError::ParameterSchemaMismatch);
        }
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err(StateMachineError::InvalidDelta);
        }
        if let Some(active) = &mut self.active {
            let transition = machine
                .transitions
                .get(active.index)
                .ok_or(StateMachineError::VersionMismatch)?;
            active.source_time += delta_seconds;
            active.target_time += delta_seconds;
            active.elapsed += delta_seconds;
            if active.elapsed >= transition.blend_seconds {
                if let Some(name) = &transition.sync_track {
                    let source = machine.state(transition.from)?;
                    let target = machine.state(transition.to)?;
                    let phase = graph.sync_phase(source.node, active.source_time, name)?;
                    active.target_time =
                        graph.elapsed_at_phase(target.node, phase, name, active.target_time)?;
                }
                self.current = transition.to;
                self.state_time = active.target_time;
                self.active = None;
            }
            return Ok(());
        }

        self.state_time += delta_seconds;
        let Some((index, transition)) =
            machine
                .transitions
                .iter()
                .enumerate()
                .find(|(_, transition)| {
                    transition.from == self.current
                        && transition
                            .conditions
                            .iter()
                            .all(|condition| condition.matches(parameters))
                })
        else {
            return Ok(());
        };
        for condition in &transition.conditions {
            if let Condition::Trigger { parameter } = condition {
                parameters.consume_trigger(parameter);
            }
        }
        if transition.blend_seconds == 0.0 {
            let source = machine.state(transition.from)?;
            let target = machine.state(transition.to)?;
            let target_time = if let Some(name) = &transition.sync_track {
                let phase = graph.sync_phase(source.node, self.state_time, name)?;
                graph.elapsed_at_phase(target.node, phase, name, 0.0)?
            } else {
                0.0
            };
            self.current = transition.to;
            self.state_time = target_time;
        } else {
            self.active = Some(ActiveTransition {
                index,
                source_time: self.state_time,
                target_time: 0.0,
                elapsed: 0.0,
            });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &self,
        machine: &StateMachine,
        graph: &AnimGraphAsset,
        skeleton: &Skeleton,
        parameters: &ParameterSet,
        generation: u64,
        cache: &mut PoseCache,
    ) -> Result<Pose, StateMachineError> {
        self.validate_binding(machine, graph)?;
        if parameters.schema != graph.parameters.id {
            return Err(StateMachineError::ParameterSchemaMismatch);
        }
        let Some(active) = &self.active else {
            let state = machine.state(self.current)?;
            return graph
                .evaluate_node(
                    state.node,
                    skeleton,
                    parameters,
                    self.state_time,
                    generation,
                    cache,
                )
                .map_err(StateMachineError::Graph);
        };
        let transition = machine
            .transitions
            .get(active.index)
            .ok_or(StateMachineError::VersionMismatch)?;
        let source = machine.state(transition.from)?;
        let target = machine.state(transition.to)?;
        let target_time = if let Some(name) = &transition.sync_track {
            let phase = graph.sync_phase(source.node, active.source_time, name)?;
            graph.elapsed_at_phase(target.node, phase, name, active.target_time)?
        } else {
            active.target_time
        };
        let source_pose = graph.evaluate_node_in_lane(
            source.node,
            skeleton,
            parameters,
            active.source_time,
            generation,
            EvaluationLane::StateSource,
            cache,
        )?;
        let target_pose = graph.evaluate_node_in_lane(
            target.node,
            skeleton,
            parameters,
            target_time,
            generation,
            EvaluationLane::StateTarget,
            cache,
        )?;
        Ok(blend_pose(
            &source_pose,
            &target_pose,
            (active.elapsed / transition.blend_seconds).clamp(0.0, 1.0),
        ))
    }
}
