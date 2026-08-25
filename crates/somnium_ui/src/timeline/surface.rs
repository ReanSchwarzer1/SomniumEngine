//! Stateful editing surface and bounded local history for the shared timeline.

use super::{
    Channel, GroupId, MarkerId, MediaId, TimelineCatalogue, TimelineDocument, TimelineError,
    TrackId,
};
use somnium_ecs::curve::{Curve, CurveKey};

#[derive(Clone)]
struct HistoryEntry {
    label: &'static str,
    before: TimelineDocument,
    after: TimelineDocument,
}

#[derive(Clone)]
pub struct TimelineHistory {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    capacity: usize,
}

impl Default for TimelineHistory {
    fn default() -> Self {
        Self::new(128)
    }
}

impl TimelineHistory {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            capacity: capacity.max(1),
        }
    }

    pub fn apply(
        &mut self,
        document: &mut TimelineDocument,
        label: &'static str,
        edit: impl FnOnce(&mut TimelineDocument) -> bool,
    ) -> bool {
        let before = document.clone();
        if !edit(document) || *document == before {
            *document = before;
            return false;
        }
        self.record(before, document, label)
    }

    pub fn record(
        &mut self,
        before: TimelineDocument,
        document: &TimelineDocument,
        label: &'static str,
    ) -> bool {
        if before == *document {
            return false;
        }
        self.entries.truncate(self.cursor);
        self.entries.push(HistoryEntry {
            label,
            before,
            after: document.clone(),
        });
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
        true
    }

    pub fn undo(&mut self, document: &mut TimelineDocument) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        *document = self.entries[self.cursor].before.clone();
        true
    }

    pub fn redo(&mut self, document: &mut TimelineDocument) -> bool {
        let Some(entry) = self.entries.get(self.cursor) else {
            return false;
        };
        *document = entry.after.clone();
        self.cursor += 1;
        true
    }

    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        self.entries.iter().map(|entry| entry.label).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineView {
    pub origin: f32,
    pub pixels_per_second: f32,
    pub snap: f32,
}

impl Default for TimelineView {
    fn default() -> Self {
        Self {
            origin: 0.0,
            pixels_per_second: 80.0,
            snap: 1.0 / 30.0,
        }
    }
}

impl TimelineView {
    #[must_use]
    pub fn time_to_x(self, time: f32, left: f32) -> f32 {
        left + (time - self.origin) * self.pixels_per_second
    }

    #[must_use]
    pub fn x_to_time(self, x: f32, left: f32) -> f32 {
        self.origin + (x - left) / self.pixels_per_second.max(1.0)
    }

    pub fn zoom_at(&mut self, factor: f32, anchor_time: f32) {
        if !factor.is_finite() || factor <= 0.0 || !anchor_time.is_finite() {
            return;
        }
        let old = self.pixels_per_second;
        self.pixels_per_second = (old * factor).clamp(8.0, 2_048.0);
        let ratio = old / self.pixels_per_second;
        self.origin = anchor_time - (anchor_time - self.origin) * ratio;
    }

    pub fn pan_pixels(&mut self, pixels: f32, duration: f32) {
        if pixels.is_finite() {
            self.origin =
                (self.origin - pixels / self.pixels_per_second).clamp(0.0, duration.max(0.0));
        }
    }

    #[must_use]
    pub fn snapped(self, time: f32, duration: f32) -> f32 {
        let time = if time.is_finite() { time } else { 0.0 };
        let snapped = if self.snap.is_finite() && self.snap > 0.0 {
            (time / self.snap).round() * self.snap
        } else {
            time
        };
        snapped.clamp(0.0, duration.max(0.0))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimelineSelection {
    pub track: Option<TrackId>,
    pub channel: Option<usize>,
    pub media: Option<MediaId>,
    pub marker: Option<MarkerId>,
}

#[derive(Clone)]
pub struct TimelineSurface {
    document: TimelineDocument,
    pub catalogue: TimelineCatalogue,
    pub view: TimelineView,
    pub selection: TimelineSelection,
    pub playhead: f32,
    history: TimelineHistory,
    curve_gesture_before: Option<TimelineDocument>,
}

impl TimelineSurface {
    #[must_use]
    pub fn new(catalogue: TimelineCatalogue, duration: f32) -> Self {
        let document = TimelineDocument::new(catalogue.id, duration);
        Self {
            document,
            catalogue,
            view: TimelineView::default(),
            selection: TimelineSelection::default(),
            playhead: 0.0,
            history: TimelineHistory::default(),
            curve_gesture_before: None,
        }
    }

    #[must_use]
    pub fn with_document(
        catalogue: TimelineCatalogue,
        document: TimelineDocument,
    ) -> Result<Self, TimelineError> {
        if document.catalogue() != catalogue.id {
            return Err(TimelineError::UnknownArchetype);
        }
        Ok(Self {
            document,
            catalogue,
            view: TimelineView::default(),
            selection: TimelineSelection::default(),
            playhead: 0.0,
            history: TimelineHistory::default(),
            curve_gesture_before: None,
        })
    }

    #[must_use]
    pub fn document(&self) -> &TimelineDocument {
        &self.document
    }

    pub fn set_document(&mut self, document: TimelineDocument) -> Result<(), TimelineError> {
        if document.catalogue() != self.catalogue.id {
            return Err(TimelineError::UnknownArchetype);
        }
        self.document = document;
        self.playhead = self.playhead.clamp(0.0, self.document.duration());
        self.selection = TimelineSelection::default();
        self.history = TimelineHistory::default();
        self.curve_gesture_before = None;
        Ok(())
    }

    pub fn add_group(
        &mut self,
        title: impl Into<String>,
        parent: Option<GroupId>,
    ) -> Result<GroupId, TimelineError> {
        let title = title.into();
        let mut result = Err(TimelineError::IdExhausted);
        self.history
            .apply(&mut self.document, "Add Timeline Group", |document| {
                result = document.add_group(title, parent);
                result.is_ok()
            });
        result
    }

    pub fn add_track(
        &mut self,
        archetype: &str,
        title: impl Into<String>,
        group: Option<GroupId>,
    ) -> Result<TrackId, TimelineError> {
        let Some(schema) = self.catalogue.get(archetype) else {
            return Err(TimelineError::UnknownArchetype);
        };
        let channels = schema
            .lanes
            .iter()
            .map(|lane| Channel {
                lane: lane.id.to_string(),
                curve: Curve::from_keys(vec![
                    CurveKey::new(0.0, lane.default),
                    CurveKey::new(self.document.duration(), lane.default),
                ]),
            })
            .collect();
        let archetype = archetype.to_string();
        let title = title.into();
        let mut result = Err(TimelineError::IdExhausted);
        self.history
            .apply(&mut self.document, "Add Timeline Track", |document| {
                result = document.add_track(archetype, title, group, channels);
                result.is_ok()
            });
        result
    }

    pub fn add_media(
        &mut self,
        track: TrackId,
        kind: &str,
        source: impl Into<String>,
        start: f32,
        duration: f32,
    ) -> Result<MediaId, TimelineError> {
        let Some(authored_track) = self.document.track(track) else {
            return Err(TimelineError::UnknownTrack);
        };
        let supported = self
            .catalogue
            .get(&authored_track.archetype)
            .is_some_and(|schema| schema.media.iter().any(|media| media.as_str() == kind));
        if !supported {
            return Err(TimelineError::UnsupportedMedia);
        }
        let kind = kind.to_string();
        let source = source.into();
        let mut result = Err(TimelineError::IdExhausted);
        self.history
            .apply(&mut self.document, "Add Timeline Media", |document| {
                result = document.add_media(track, kind, source, start, duration);
                result.is_ok()
            });
        result
    }

    pub fn move_media(&mut self, media: MediaId, start: f32) -> bool {
        let start = self.view.snapped(start, self.document.duration());
        self.history
            .apply(&mut self.document, "Move Timeline Media", |document| {
                document.move_media(media, start)
            })
    }

    pub fn resize_media(&mut self, media: MediaId, start: f32, duration: f32) -> bool {
        if !duration.is_finite() {
            return false;
        }
        let document_duration = self.document.duration();
        let start = self.view.snapped(start, document_duration);
        let end = self.view.snapped(start + duration, document_duration);
        self.history
            .apply(&mut self.document, "Resize Timeline Media", |document| {
                document.resize_media(media, start, end - start)
            })
    }

    pub fn add_marker(
        &mut self,
        time: f32,
        label: impl Into<String>,
    ) -> Result<MarkerId, TimelineError> {
        let label = label.into();
        let time = self.view.snapped(time, self.document.duration());
        let mut result = Err(TimelineError::IdExhausted);
        self.history
            .apply(&mut self.document, "Add Timeline Marker", |document| {
                result = document.add_marker(time, label);
                result.is_ok()
            });
        result
    }

    pub fn move_marker(&mut self, marker: MarkerId, time: f32) -> bool {
        let time = self.view.snapped(time, self.document.duration());
        self.history
            .apply(&mut self.document, "Move Timeline Marker", |document| {
                document.move_marker(marker, time)
            })
    }

    pub fn add_keyframe(
        &mut self,
        track: TrackId,
        channel: usize,
        mut key: CurveKey,
    ) -> Result<usize, TimelineError> {
        key.t = self.view.snapped(key.t, self.document.duration());
        let mut result = Err(TimelineError::InvalidKey);
        self.history
            .apply(&mut self.document, "Add Timeline Key", |document| {
                result = document.add_keyframe(track, channel, key);
                result.is_ok()
            });
        result
    }

    pub fn move_keyframe(&mut self, track: TrackId, channel: usize, key: usize, time: f32) -> bool {
        let time = self.view.snapped(time, self.document.duration());
        self.history
            .apply(&mut self.document, "Move Timeline Key", |document| {
                document.move_keyframe(track, channel, key, time)
            })
    }

    pub fn remove_track(&mut self, track: TrackId) -> bool {
        let removed = self
            .history
            .apply(&mut self.document, "Delete Timeline Track", |document| {
                document.remove_track(track)
            });
        if removed && self.selection.track == Some(track) {
            self.selection = TimelineSelection::default();
            self.curve_gesture_before = None;
        }
        removed
    }

    pub fn select_channel(&mut self, track: TrackId, channel: usize) -> bool {
        if self
            .document
            .track(track)
            .and_then(|track| track.channels.get(channel))
            .is_none()
        {
            return false;
        }
        self.selection = TimelineSelection {
            track: Some(track),
            channel: Some(channel),
            ..TimelineSelection::default()
        };
        true
    }

    #[must_use]
    pub fn selected_curve(&self) -> Option<&Curve> {
        let track = self.selection.track?;
        let channel = self.selection.channel?;
        self.document
            .track(track)
            .and_then(|track| track.channels.get(channel))
            .map(|channel| &channel.curve)
    }

    pub fn set_selected_curve(&mut self, curve: Curve, live: bool) -> bool {
        let (Some(track), Some(channel)) = (self.selection.track, self.selection.channel) else {
            return false;
        };
        let before_edit = self.document.clone();
        if !self.document.set_curve(track, channel, curve) {
            return false;
        }
        if live {
            if self.curve_gesture_before.is_none() {
                self.curve_gesture_before = Some(before_edit);
            }
        } else {
            let before = self.curve_gesture_before.take().unwrap_or(before_edit);
            self.history
                .record(before, &self.document, "Edit Timeline Curve");
        }
        true
    }

    pub fn scrub(&mut self, time: f32) -> f32 {
        self.playhead = self.view.snapped(time, self.document.duration());
        self.playhead
    }

    pub fn undo(&mut self) -> bool {
        self.curve_gesture_before = None;
        self.history.undo(&mut self.document)
    }

    pub fn redo(&mut self) -> bool {
        self.curve_gesture_before = None;
        self.history.redo(&mut self.document)
    }

    #[must_use]
    pub fn history_labels(&self) -> Vec<&'static str> {
        self.history.labels()
    }
}
