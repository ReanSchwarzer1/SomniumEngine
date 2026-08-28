//! MORROWIND-L — one reusable, archetype-driven track-and-media timeline.
//!
//! The model and editing surface are consumer-neutral. Animation, UI motion,
//! VFX and audio contribute catalogues; none of them owns a timeline widget.
//! Numeric channels use CONTROL-K's [`somnium_ecs::curve::Curve`], and the
//! retained control embeds CONTROL-K's curve editor for the selected channel.

pub mod archetype;
pub mod catalogues;
pub mod model;
pub mod serial;
pub mod surface;
pub mod widget;

pub use archetype::{LaneArchetype, MediaKind, TimelineCatalogue, TrackArchetype};
pub use model::{
    Channel, GroupId, Marker, MarkerId, MediaClip, MediaId, TimelineDocument, TimelineError, Track,
    TrackGroup, TrackId,
};
pub use serial::{TIMELINE_ASSET_VERSION, TimelineAssetError, from_json, to_json};
pub use somnium_ecs::curve::{Curve, CurveKey, Interpolation};
pub use surface::{TimelineHistory, TimelineSelection, TimelineSurface, TimelineView};
pub use widget::{
    TimelineEditor, TimelineEditorBuilder, TimelineEditorHandles, TimelineEditorMessage,
};

#[cfg(test)]
mod tests;
