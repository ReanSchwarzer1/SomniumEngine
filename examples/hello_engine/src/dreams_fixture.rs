//! DREAMS' deterministic camera rail.
//!
//! The rail is a capture harness, not an editor camera feature. It follows a
//! frame-indexed loop for 120 frames and then holds its starting pose. A 180
//! frame timing warm-up therefore sees the same motion/history build-up while
//! every measured frame is stationary.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraPose {
    pub(crate) position: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DreamRail {
    anchor: CameraPose,
    start_frame: u64,
    lateral_metres: f32,
}

impl DreamRail {
    pub(crate) fn named(name: &str, anchor: CameraPose, start_frame: u64) -> Option<Self> {
        let lateral_metres = match name {
            "coastal-ground" => 1.5,
            "island-ground" => 1.0,
            _ => return None,
        };
        Some(Self {
            anchor,
            start_frame,
            lateral_metres,
        })
    }

    pub(crate) fn pose(self, frame: u64) -> CameraPose {
        const MOVING_FRAMES: u64 = 120;
        let elapsed = frame.saturating_sub(self.start_frame);
        if elapsed >= MOVING_FRAMES {
            return self.anchor;
        }
        let t = elapsed as f32 / MOVING_FRAMES as f32;
        let phase = t * std::f32::consts::TAU;
        let yaw = self.anchor.yaw.to_radians();
        let right = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
        CameraPose {
            position: self.anchor.position
                + right * (phase.sin() * self.lateral_metres)
                + Vec3::Y * ((phase * 0.5).sin() * 0.35),
            yaw: self.anchor.yaw + phase.sin() * 2.0,
            pitch: self.anchor.pitch + phase.cos().mul_add(0.75, -0.75),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> CameraPose {
        CameraPose {
            position: Vec3::new(10.0, 4.0, -3.0),
            yaw: -90.0,
            pitch: -12.0,
        }
    }

    #[test]
    fn named_rails_refuse_a_typo_instead_of_guessing() {
        assert!(DreamRail::named("coastal-ground", anchor(), 7).is_some());
        assert!(DreamRail::named("coatsal-ground", anchor(), 7).is_none());
    }

    #[test]
    fn the_rail_is_frame_indexed_and_holds_before_measurement() {
        let rail = DreamRail::named("island-ground", anchor(), 10).unwrap();
        assert_eq!(rail.pose(10), anchor());
        assert_ne!(rail.pose(40), anchor());
        assert_eq!(rail.pose(130), anchor());
        assert_eq!(rail.pose(240), anchor());
    }
}
