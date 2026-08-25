//! Consumer-provided timeline schema.

use std::collections::BTreeMap;

/// Media payload accepted by a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    AnimationClip,
    AudioClip,
    UiMotion,
    VfxEvent,
    Event,
    Custom(&'static str),
}

impl MediaKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnimationClip => "animation-clip",
            Self::AudioClip => "audio-clip",
            Self::UiMotion => "ui-motion",
            Self::VfxEvent => "vfx-event",
            Self::Event => "event",
            Self::Custom(id) => id,
        }
    }
}

/// One numeric channel lane supplied by a track archetype.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneArchetype {
    pub id: &'static str,
    pub title: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    pub tooltip: &'static str,
}

impl LaneArchetype {
    #[must_use]
    pub const fn new(id: &'static str, title: &'static str) -> Self {
        Self {
            id,
            title,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: "",
            tooltip: "",
        }
    }

    #[must_use]
    pub const fn with_range(mut self, min: f32, max: f32, default: f32) -> Self {
        self.min = min;
        self.max = max;
        self.default = default;
        self
    }

    #[must_use]
    pub const fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    #[must_use]
    pub const fn with_tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = tooltip;
        self
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.min.is_finite()
            && self.max.is_finite()
            && self.default.is_finite()
            && self.max > self.min
            && (self.min..=self.max).contains(&self.default)
    }
}

/// All data the shared control needs to construct one kind of track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackArchetype {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub lanes: Vec<LaneArchetype>,
    pub media: Vec<MediaKind>,
    pub tooltip: &'static str,
}

impl TrackArchetype {
    #[must_use]
    pub fn new(id: &'static str, title: &'static str, category: &'static str) -> Self {
        Self {
            id,
            title,
            category,
            lanes: Vec::new(),
            media: Vec::new(),
            tooltip: "",
        }
    }

    #[must_use]
    pub fn with_lane(mut self, lane: LaneArchetype) -> Self {
        self.lanes.push(lane);
        self
    }

    #[must_use]
    pub fn with_media(mut self, media: MediaKind) -> Self {
        self.media.push(media);
        self
    }

    #[must_use]
    pub fn with_tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = tooltip;
        self
    }
}

/// A feature's contribution to the one timeline.
#[derive(Clone, Debug)]
pub struct TimelineCatalogue {
    pub id: &'static str,
    tracks: BTreeMap<&'static str, TrackArchetype>,
}

impl TimelineCatalogue {
    #[must_use]
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            tracks: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, archetype: TrackArchetype) -> &mut Self {
        assert!(
            !archetype.id.is_empty(),
            "timeline archetype id must not be empty"
        );
        assert!(
            archetype.lanes.iter().all(LaneArchetype::valid),
            "timeline archetype {} has an invalid lane",
            archetype.id
        );
        assert!(
            self.tracks.insert(archetype.id, archetype).is_none(),
            "duplicate timeline archetype"
        );
        self
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&TrackArchetype> {
        self.tracks.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &TrackArchetype> {
        self.tracks.values()
    }
}
