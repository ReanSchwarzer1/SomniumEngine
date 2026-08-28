//! Versioned deterministic timeline-asset serialization.

use super::{
    Channel, GroupId, Marker, MarkerId, MediaClip, MediaId, TimelineCatalogue, TimelineDocument,
    Track, TrackGroup, TrackId,
};
use serde::{Deserialize, Serialize};
use somnium_ecs::curve::{Curve, CurveKey, Interpolation};
use std::collections::{BTreeSet, HashSet};
use std::fmt;

pub const TIMELINE_ASSET_VERSION: u32 = 1;

#[derive(Debug)]
pub enum TimelineAssetError {
    Json(serde_json::Error),
    FutureVersion(u32),
    CatalogueMismatch { expected: String, found: String },
    DuplicateId(u32),
    UnknownGroup(u32),
    GroupCycle(u32),
    UnknownTrack(u32),
    UnknownArchetype(String),
    UnknownLane { track: u32, lane: String },
    UnsupportedMedia { track: u32, kind: String },
    InvalidDuration,
    InvalidTime,
    InvalidValue,
    InvalidNextId,
}

impl fmt::Display for TimelineAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid timeline JSON: {error}"),
            Self::FutureVersion(version) => write!(
                formatter,
                "timeline version {version} is newer than supported {TIMELINE_ASSET_VERSION}"
            ),
            Self::CatalogueMismatch { expected, found } => {
                write!(
                    formatter,
                    "timeline catalogue is {found}, expected {expected}"
                )
            }
            other => write!(formatter, "invalid timeline asset: {other:?}"),
        }
    }
}

impl std::error::Error for TimelineAssetError {}

impl From<serde_json::Error> for TimelineAssetError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Asset {
    #[serde(default)]
    version: u32,
    catalogue: String,
    duration: f32,
    #[serde(default)]
    next_id: Option<u32>,
    #[serde(default)]
    groups: Vec<AssetGroup>,
    #[serde(default)]
    tracks: Vec<AssetTrack>,
    #[serde(default)]
    media: Vec<AssetMedia>,
    #[serde(default)]
    markers: Vec<AssetMarker>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetGroup {
    id: u32,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetTrack {
    id: u32,
    archetype: String,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<u32>,
    #[serde(default)]
    channels: Vec<AssetChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetChannel {
    lane: String,
    #[serde(default)]
    keys: Vec<AssetKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetKey {
    time: f32,
    value: f32,
    #[serde(default)]
    in_tangent: f32,
    #[serde(default)]
    out_tangent: f32,
    #[serde(default = "linear")]
    interpolation: String,
}

fn linear() -> String {
    "linear".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetMedia {
    id: u32,
    track: u32,
    kind: String,
    source: String,
    start: f32,
    duration: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetMarker {
    id: u32,
    time: f32,
    label: String,
}

pub fn to_json(document: &TimelineDocument) -> Result<String, TimelineAssetError> {
    let mut groups: Vec<_> = document
        .groups()
        .iter()
        .map(|group| AssetGroup {
            id: group.id.0,
            title: group.title.clone(),
            parent: group.parent.map(|id| id.0),
        })
        .collect();
    groups.sort_by_key(|group| group.id);
    let mut tracks: Vec<_> = document
        .tracks()
        .iter()
        .map(|track| AssetTrack {
            id: track.id.0,
            archetype: track.archetype.clone(),
            title: track.title.clone(),
            group: track.group.map(|id| id.0),
            channels: track
                .channels
                .iter()
                .map(|channel| AssetChannel {
                    lane: channel.lane.clone(),
                    keys: channel
                        .curve
                        .keys()
                        .iter()
                        .map(|key| AssetKey {
                            time: key.t,
                            value: key.v,
                            in_tangent: key.in_tangent,
                            out_tangent: key.out_tangent,
                            interpolation: key.interpolation.as_str().to_string(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    tracks.sort_by_key(|track| track.id);
    // Channel order is semantic: the retained control selects lanes by their
    // archetype index. Tracks are deterministically ordered by id, but lanes
    // must remain in catalogue order across a save/load round trip.
    let mut media: Vec<_> = document
        .media()
        .iter()
        .map(|clip| AssetMedia {
            id: clip.id.0,
            track: clip.track.0,
            kind: clip.kind.clone(),
            source: clip.source.clone(),
            start: clip.start,
            duration: clip.duration,
        })
        .collect();
    media.sort_by_key(|clip| clip.id);
    let mut markers: Vec<_> = document
        .markers()
        .iter()
        .map(|marker| AssetMarker {
            id: marker.id.0,
            time: marker.time,
            label: marker.label.clone(),
        })
        .collect();
    markers.sort_by_key(|marker| marker.id);
    let asset = Asset {
        version: TIMELINE_ASSET_VERSION,
        catalogue: document.catalogue().to_string(),
        duration: document.duration(),
        next_id: Some(document.next_id),
        groups,
        tracks,
        media,
        markers,
    };
    let mut json = serde_json::to_string_pretty(&asset)?;
    json.push('\n');
    Ok(json)
}

pub fn from_json(
    json: &str,
    catalogue: &TimelineCatalogue,
) -> Result<TimelineDocument, TimelineAssetError> {
    let mut asset: Asset = serde_json::from_str(json)?;
    match asset.version {
        0 => asset.version = TIMELINE_ASSET_VERSION,
        TIMELINE_ASSET_VERSION => {}
        version => return Err(TimelineAssetError::FutureVersion(version)),
    }
    if asset.catalogue != catalogue.id {
        return Err(TimelineAssetError::CatalogueMismatch {
            expected: catalogue.id.to_string(),
            found: asset.catalogue,
        });
    }
    if !asset.duration.is_finite() || asset.duration <= 0.0 {
        return Err(TimelineAssetError::InvalidDuration);
    }

    let mut ids = BTreeSet::new();
    for id in asset
        .groups
        .iter()
        .map(|item| item.id)
        .chain(asset.tracks.iter().map(|item| item.id))
        .chain(asset.media.iter().map(|item| item.id))
        .chain(asset.markers.iter().map(|item| item.id))
    {
        if id == 0 || !ids.insert(id) {
            return Err(TimelineAssetError::DuplicateId(id));
        }
    }
    let group_ids: HashSet<_> = asset.groups.iter().map(|group| group.id).collect();
    for group in &asset.groups {
        if group
            .parent
            .is_some_and(|parent| !group_ids.contains(&parent))
        {
            return Err(TimelineAssetError::UnknownGroup(group.parent.unwrap_or(0)));
        }
        let mut cursor = group.parent;
        let mut visited = HashSet::new();
        while let Some(id) = cursor {
            if !visited.insert(id) || id == group.id {
                return Err(TimelineAssetError::GroupCycle(group.id));
            }
            cursor = asset
                .groups
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.parent);
        }
    }

    let track_ids: HashSet<_> = asset.tracks.iter().map(|track| track.id).collect();
    for track in &asset.tracks {
        let Some(schema) = catalogue.get(&track.archetype) else {
            return Err(TimelineAssetError::UnknownArchetype(
                track.archetype.clone(),
            ));
        };
        if track.group.is_some_and(|group| !group_ids.contains(&group)) {
            return Err(TimelineAssetError::UnknownGroup(track.group.unwrap_or(0)));
        }
        let mut lanes = HashSet::new();
        for channel in &track.channels {
            if !lanes.insert(channel.lane.as_str())
                || !schema.lanes.iter().any(|lane| lane.id == channel.lane)
            {
                return Err(TimelineAssetError::UnknownLane {
                    track: track.id,
                    lane: channel.lane.clone(),
                });
            }
            for key in &channel.keys {
                if !key.time.is_finite()
                    || !key.value.is_finite()
                    || !key.in_tangent.is_finite()
                    || !key.out_tangent.is_finite()
                    || key.time < 0.0
                    || key.time > asset.duration
                {
                    return Err(TimelineAssetError::InvalidValue);
                }
            }
        }
    }
    for clip in &asset.media {
        let Some(track) = asset.tracks.iter().find(|track| track.id == clip.track) else {
            return Err(TimelineAssetError::UnknownTrack(clip.track));
        };
        let schema = catalogue
            .get(&track.archetype)
            .ok_or_else(|| TimelineAssetError::UnknownArchetype(track.archetype.clone()))?;
        if !schema.media.iter().any(|kind| kind.as_str() == clip.kind) {
            return Err(TimelineAssetError::UnsupportedMedia {
                track: clip.track,
                kind: clip.kind.clone(),
            });
        }
        if !clip.start.is_finite()
            || !clip.duration.is_finite()
            || clip.start < 0.0
            || clip.duration <= 0.0
            || clip.start + clip.duration > asset.duration + f32::EPSILON
        {
            return Err(TimelineAssetError::InvalidTime);
        }
    }
    for marker in &asset.markers {
        if !marker.time.is_finite() || marker.time < 0.0 || marker.time > asset.duration {
            return Err(TimelineAssetError::InvalidTime);
        }
    }
    let max_id = ids.last().copied().unwrap_or(0);
    let next_id = asset.next_id.unwrap_or(max_id.saturating_add(1));
    if next_id <= max_id || next_id == 0 {
        return Err(TimelineAssetError::InvalidNextId);
    }

    let mut document = TimelineDocument {
        catalogue: asset.catalogue,
        duration: asset.duration,
        next_id,
        groups: asset
            .groups
            .into_iter()
            .map(|group| TrackGroup {
                id: GroupId(group.id),
                title: group.title,
                parent: group.parent.map(GroupId),
            })
            .collect(),
        tracks: asset
            .tracks
            .into_iter()
            .map(|track| Track {
                id: TrackId(track.id),
                archetype: track.archetype,
                title: track.title,
                group: track.group.map(GroupId),
                channels: track
                    .channels
                    .into_iter()
                    .map(|channel| Channel {
                        lane: channel.lane,
                        curve: Curve::from_keys(
                            channel
                                .keys
                                .into_iter()
                                .map(|key| CurveKey {
                                    t: key.time,
                                    v: key.value,
                                    in_tangent: key.in_tangent,
                                    out_tangent: key.out_tangent,
                                    interpolation: Interpolation::from_str_or_linear(
                                        &key.interpolation,
                                    ),
                                })
                                .collect(),
                        ),
                    })
                    .collect(),
            })
            .collect(),
        media: asset
            .media
            .into_iter()
            .map(|clip| MediaClip {
                id: MediaId(clip.id),
                track: TrackId(clip.track),
                kind: clip.kind,
                source: clip.source,
                start: clip.start,
                duration: clip.duration,
            })
            .collect(),
        markers: asset
            .markers
            .into_iter()
            .map(|marker| Marker {
                id: MarkerId(marker.id),
                time: marker.time,
                label: marker.label,
            })
            .collect(),
    };
    document.groups.sort_by_key(|group| group.id);
    document.tracks.sort_by_key(|track| track.id);
    document.media.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.track.cmp(&b.track))
            .then(a.id.cmp(&b.id))
    });
    document
        .markers
        .sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
    debug_assert!(
        document
            .tracks
            .iter()
            .all(|track| track_ids.contains(&track.id.0))
    );
    Ok(document)
}
