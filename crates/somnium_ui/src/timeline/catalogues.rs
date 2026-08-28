//! Built-in consumers proving that MORROWIND-L is a framework, not an
//! animation-only sequencer.

use super::{LaneArchetype, MediaKind, TimelineCatalogue, TrackArchetype};

/// Animation clips, events and weight channels.
#[must_use]
pub fn animation() -> TimelineCatalogue {
    let mut catalogue = TimelineCatalogue::new("somnium.animation.timeline");
    catalogue.register(
        TrackArchetype::new("animation.clip", "Animation", "Animation")
            .with_media(MediaKind::AnimationClip)
            .with_lane(
                LaneArchetype::new("weight", "Weight")
                    .with_range(0.0, 1.0, 1.0)
                    .with_unit("×")
                    .with_tooltip("Contribution of this animation track"),
            )
            .with_lane(
                LaneArchetype::new("speed", "Speed")
                    .with_range(0.0, 4.0, 1.0)
                    .with_unit("×")
                    .with_tooltip("Playback-rate multiplier"),
            )
            .with_tooltip("Animation clips and their continuous controls"),
    );
    catalogue.register(
        TrackArchetype::new("animation.event", "Animation Events", "Events")
            .with_media(MediaKind::Event)
            .with_tooltip("Named events emitted at authored times"),
    );
    catalogue
}

/// Non-animation consumer: MORROWIND-H's runtime UI motion.
#[must_use]
pub fn ui_motion() -> TimelineCatalogue {
    let mut catalogue = TimelineCatalogue::new("somnium.ui-motion.timeline");
    catalogue.register(
        TrackArchetype::new("ui.motion", "UI Motion", "Interface")
            .with_media(MediaKind::UiMotion)
            .with_lane(
                LaneArchetype::new("opacity", "Opacity")
                    .with_range(0.0, 1.0, 1.0)
                    .with_tooltip("Widget opacity over the sequence"),
            )
            .with_lane(
                LaneArchetype::new("offset-x", "Horizontal Offset")
                    .with_range(-4096.0, 4096.0, 0.0)
                    .with_unit("px")
                    .with_tooltip("Widget translation from its arranged position"),
            )
            .with_lane(
                LaneArchetype::new("offset-y", "Vertical Offset")
                    .with_range(-4096.0, 4096.0, 0.0)
                    .with_unit("px")
                    .with_tooltip("Widget translation from its arranged position"),
            )
            .with_tooltip("Track-based authoring for runtime UI motion"),
    );
    catalogue
}
