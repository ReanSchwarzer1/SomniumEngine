//! Animation LOD, opt-in per rig: far from the camera a rig is evaluated less
//! often (and its limb IK, foot placement and look-at dropped); past a cull
//! distance not at all, keeping its last pose.
//!
//! Add **Animation LOD** to an entity with **Animation**. Without it a rig is
//! evaluated every frame as before. Skipped time is owed, not lost: when a rig
//! is next evaluated it advances by everything it missed (capped at a second),
//! so a far crowd stays in step with the clock.

use somnium_ecs::Component;
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;

/// How a rig's evaluation thins with distance from the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationLod {
    /// Thin this rig's evaluation.
    pub enabled: bool,
    /// Within this (metres), every frame.
    pub full: f32,
    /// Out to this, every other frame (30 Hz at 60); beyond, at `far_rate`.
    pub far: f32,
    /// Evaluations a second beyond `far`.
    pub far_rate: f32,
    /// Beyond this, not evaluated at all (the last pose holds).
    pub cull: f32,
    /// Limb IK, foot placement and look-at only within this.
    pub detail: f32,
}

impl Component for AnimationLod {}

impl Default for AnimationLod {
    fn default() -> Self {
        Self {
            enabled: true,
            full: 20.0,
            far: 60.0,
            far_rate: 10.0,
            cull: 160.0,
            detail: 25.0,
        }
    }
}

/// What to do with a rig this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimationStep {
    /// Evaluate, advancing by this many seconds, with or without the detail
    /// (IK, feet, look-at).
    Evaluate {
        /// Seconds to advance (this frame plus what was owed).
        seconds: f32,
        /// Keep limb IK, foot placement and look-at.
        detail: bool,
    },
    /// Not this frame (the time is owed).
    Wait,
}

/// Most a skipped rig catches up in one evaluation.
pub const MAX_OWED: f32 = 1.0;

impl AnimationLod {
    /// The interval between evaluations at `distance`, or `None` past the cull.
    #[must_use]
    pub fn interval(&self, distance: f32) -> Option<f32> {
        if distance > self.cull {
            None
        } else if distance > self.far {
            Some(1.0 / self.far_rate.max(0.1))
        } else if distance > self.full {
            Some(1.0 / 30.0)
        } else {
            Some(0.0)
        }
    }

    /// Decide this frame for a rig at `distance` that is owed `owed` seconds
    /// (including this frame's). Resets `owed` when it evaluates.
    pub fn step(&self, distance: f32, owed: &mut f32) -> AnimationStep {
        *owed = owed.min(MAX_OWED);
        match self.interval(distance) {
            Some(interval) if *owed + 1e-6 >= interval => AnimationStep::Evaluate {
                seconds: std::mem::take(owed),
                detail: distance <= self.detail,
            },
            _ => AnimationStep::Wait,
        }
    }
}

/// Register the schema.
pub fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        AnimationLod as "somnium.AnimationLod", display "Animation LOD", version 1,
        fields {
            enabled { doc: "Thin this rig's evaluation with distance from the camera. Off (or no Animation LOD at all) evaluates every frame." },
            full { min: 0.0, unit: "m", doc: "Every frame within this." },
            far { min: 0.0, unit: "m", doc: "Every other frame out to this." },
            far_rate { min: 0.1, max: 60.0, unit: "Hz", display_name: "Far Rate", doc: "Evaluations a second beyond Far." },
            cull { min: 0.0, unit: "m", doc: "Not evaluated beyond this; the last pose holds." },
            detail { min: 0.0, unit: "m", doc: "Limb IK, foot placement and look-at only within this." },
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_every_frame_far_at_the_far_rate_beyond_the_cull_never() {
        let lod = AnimationLod::default();
        let frame = 1.0 / 60.0;
        let mut evaluated = |distance: f32| {
            let mut owed = 0.0;
            let mut seconds = 0.0;
            let mut count = 0;
            for _ in 0..60 {
                owed += frame;
                if let AnimationStep::Evaluate { seconds: s, .. } = lod.step(distance, &mut owed) {
                    count += 1;
                    seconds += s;
                }
            }
            (count, seconds)
        };
        let (near, near_s) = evaluated(5.0);
        let (mid, mid_s) = evaluated(40.0);
        let (far, far_s) = evaluated(100.0);
        let (gone, _) = evaluated(500.0);
        assert_eq!(near, 60);
        assert!((29..=31).contains(&mid), "{mid}");
        assert!((9..=11).contains(&far), "{far}");
        assert_eq!(gone, 0);
        // Time is owed, not lost.
        for s in [near_s, mid_s, far_s] {
            assert!((s - 1.0).abs() < 0.11, "{s}");
        }
    }

    #[test]
    fn detail_only_up_close() {
        let lod = AnimationLod::default();
        let mut owed = 0.5;
        assert!(matches!(lod.step(10.0, &mut owed), AnimationStep::Evaluate { detail: true, .. }));
        owed = 0.5;
        assert!(matches!(lod.step(50.0, &mut owed), AnimationStep::Evaluate { detail: false, .. }));
    }
}
