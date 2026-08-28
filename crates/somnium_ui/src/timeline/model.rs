//! Durable consumer-neutral timeline document.

use somnium_ecs::curve::{Curve, CurveKey};
use std::fmt;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);
    };
}

id_type!(GroupId);
id_type!(TrackId);
id_type!(MediaId);
id_type!(MarkerId);

#[derive(Clone, Debug, PartialEq)]
pub struct TrackGroup {
    pub id: GroupId,
    pub title: String,
    pub parent: Option<GroupId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub lane: String,
    pub curve: Curve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub archetype: String,
    pub title: String,
    pub group: Option<GroupId>,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaClip {
    pub id: MediaId,
    pub track: TrackId,
    pub kind: String,
    pub source: String,
    pub start: f32,
    pub duration: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: MarkerId,
    pub time: f32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineDocument {
    pub(crate) catalogue: String,
    pub(crate) duration: f32,
    pub(crate) next_id: u32,
    pub(crate) groups: Vec<TrackGroup>,
    pub(crate) tracks: Vec<Track>,
    pub(crate) media: Vec<MediaClip>,
    pub(crate) markers: Vec<Marker>,
}

impl TimelineDocument {
    #[must_use]
    pub fn new(catalogue: impl Into<String>, duration: f32) -> Self {
        Self {
            catalogue: catalogue.into(),
            duration: valid_duration(duration),
            next_id: 1,
            groups: Vec::new(),
            tracks: Vec::new(),
            media: Vec::new(),
            markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn catalogue(&self) -> &str {
        &self.catalogue
    }

    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn set_duration(&mut self, duration: f32) -> bool {
        if !duration.is_finite() || duration <= 0.0 {
            return false;
        }
        self.duration = duration;
        for clip in &mut self.media {
            let minimum_span = duration.min(0.001);
            clip.start = clip.start.clamp(0.0, duration - minimum_span);
            clip.duration = clip.duration.min(duration - clip.start);
        }
        for marker in &mut self.markers {
            marker.time = marker.time.clamp(0.0, duration);
        }
        true
    }

    pub fn groups(&self) -> &[TrackGroup] {
        &self.groups
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn media(&self) -> &[MediaClip] {
        &self.media
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == id)
    }

    #[must_use]
    pub fn media_clip(&self, id: MediaId) -> Option<&MediaClip> {
        self.media.iter().find(|clip| clip.id == id)
    }

    fn allocate(&mut self) -> Result<u32, TimelineError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(TimelineError::IdExhausted)?;
        Ok(id)
    }

    pub fn add_group(
        &mut self,
        title: impl Into<String>,
        parent: Option<GroupId>,
    ) -> Result<GroupId, TimelineError> {
        if parent.is_some_and(|id| !self.groups.iter().any(|group| group.id == id)) {
            return Err(TimelineError::UnknownGroup);
        }
        let id = GroupId(self.allocate()?);
        self.groups.push(TrackGroup {
            id,
            title: title.into(),
            parent,
        });
        Ok(id)
    }

    pub(crate) fn add_track(
        &mut self,
        archetype: impl Into<String>,
        title: impl Into<String>,
        group: Option<GroupId>,
        channels: Vec<Channel>,
    ) -> Result<TrackId, TimelineError> {
        if group.is_some_and(|id| !self.groups.iter().any(|candidate| candidate.id == id)) {
            return Err(TimelineError::UnknownGroup);
        }
        let id = TrackId(self.allocate()?);
        self.tracks.push(Track {
            id,
            archetype: archetype.into(),
            title: title.into(),
            group,
            channels,
        });
        Ok(id)
    }

    pub fn remove_track(&mut self, track: TrackId) -> bool {
        let before = self.tracks.len();
        self.tracks.retain(|candidate| candidate.id != track);
        if self.tracks.len() == before {
            return false;
        }
        self.media.retain(|clip| clip.track != track);
        true
    }

    pub fn add_media(
        &mut self,
        track: TrackId,
        kind: impl Into<String>,
        source: impl Into<String>,
        start: f32,
        duration: f32,
    ) -> Result<MediaId, TimelineError> {
        if self.track(track).is_none() {
            return Err(TimelineError::UnknownTrack);
        }
        let (start, duration) = self.valid_clip_range(start, duration)?;
        let id = MediaId(self.allocate()?);
        self.media.push(MediaClip {
            id,
            track,
            kind: kind.into(),
            source: source.into(),
            start,
            duration,
        });
        self.sort_media();
        Ok(id)
    }

    pub fn move_media(&mut self, id: MediaId, start: f32) -> bool {
        let Some(index) = self.media.iter().position(|clip| clip.id == id) else {
            return false;
        };
        let duration = self.media[index].duration;
        if !start.is_finite() {
            return false;
        }
        self.media[index].start = start.clamp(0.0, (self.duration - duration).max(0.0));
        self.sort_media();
        true
    }

    pub fn resize_media(&mut self, id: MediaId, start: f32, duration: f32) -> bool {
        let Ok((start, duration)) = self.valid_clip_range(start, duration) else {
            return false;
        };
        let Some(clip) = self.media.iter_mut().find(|clip| clip.id == id) else {
            return false;
        };
        clip.start = start;
        clip.duration = duration;
        self.sort_media();
        true
    }

    pub fn add_marker(
        &mut self,
        time: f32,
        label: impl Into<String>,
    ) -> Result<MarkerId, TimelineError> {
        let Some(time) = self.valid_time(time) else {
            return Err(TimelineError::InvalidTime);
        };
        let id = MarkerId(self.allocate()?);
        self.markers.push(Marker {
            id,
            time,
            label: label.into(),
        });
        self.sort_markers();
        Ok(id)
    }

    pub fn move_marker(&mut self, id: MarkerId, time: f32) -> bool {
        let Some(time) = self.valid_time(time) else {
            return false;
        };
        let Some(marker) = self.markers.iter_mut().find(|marker| marker.id == id) else {
            return false;
        };
        marker.time = time;
        self.sort_markers();
        true
    }

    pub fn add_keyframe(
        &mut self,
        track: TrackId,
        channel: usize,
        key: CurveKey,
    ) -> Result<usize, TimelineError> {
        if !key.t.is_finite() || !key.v.is_finite() || key.t < 0.0 || key.t > self.duration {
            return Err(TimelineError::InvalidKey);
        }
        let channel = self
            .track_mut(track)
            .and_then(|track| track.channels.get_mut(channel))
            .ok_or(TimelineError::UnknownChannel)?;
        Ok(channel.curve.insert(key))
    }

    pub fn move_keyframe(&mut self, track: TrackId, channel: usize, key: usize, time: f32) -> bool {
        if !time.is_finite() {
            return false;
        }
        let duration = self.duration;
        let Some(channel) = self
            .track_mut(track)
            .and_then(|track| track.channels.get_mut(channel))
        else {
            return false;
        };
        let Some(existing) = channel.curve.keys().get(key).copied() else {
            return false;
        };
        channel
            .curve
            .move_key(key, time.clamp(0.0, duration), existing.v);
        true
    }

    pub fn set_curve(&mut self, track: TrackId, channel: usize, curve: Curve) -> bool {
        let duration = self.duration;
        if curve.keys().iter().any(|key| {
            !key.t.is_finite()
                || !key.v.is_finite()
                || !key.in_tangent.is_finite()
                || !key.out_tangent.is_finite()
                || key.t < 0.0
                || key.t > duration
        }) {
            return false;
        }
        let Some(target) = self
            .track_mut(track)
            .and_then(|track| track.channels.get_mut(channel))
        else {
            return false;
        };
        target.curve = curve;
        true
    }

    fn valid_time(&self, time: f32) -> Option<f32> {
        time.is_finite().then(|| time.clamp(0.0, self.duration))
    }

    fn valid_clip_range(&self, start: f32, duration: f32) -> Result<(f32, f32), TimelineError> {
        if !start.is_finite() || !duration.is_finite() || duration <= 0.0 {
            return Err(TimelineError::InvalidClipRange);
        }
        let minimum_span = self.duration.min(0.001);
        let start = start.clamp(0.0, self.duration - minimum_span);
        let duration = duration.min(self.duration - start);
        Ok((start, duration))
    }

    fn sort_media(&mut self) {
        self.media.sort_by(|a, b| {
            a.start
                .total_cmp(&b.start)
                .then(a.track.cmp(&b.track))
                .then(a.id.cmp(&b.id))
        });
    }

    fn sort_markers(&mut self) {
        self.markers
            .sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
    }
}

fn valid_duration(duration: f32) -> f32 {
    if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        10.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineError {
    IdExhausted,
    UnknownGroup,
    UnknownTrack,
    UnknownChannel,
    UnknownArchetype,
    UnsupportedMedia,
    InvalidTime,
    InvalidClipRange,
    InvalidKey,
}

impl fmt::Display for TimelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TimelineError {}
