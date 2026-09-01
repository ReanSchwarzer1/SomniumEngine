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

/// What a rail does with the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RailKind {
    /// Sway a metre or so, then hold the anchor. The measurement rail: a
    /// stationary frame is what makes a timing comparable and a golden image
    /// reproducible.
    Hold { lateral_metres: f32 },
    /// Fly in a straight line at a fixed speed and never stop.
    ///
    /// The rail DREAMS-B did not have, and the reason its captures could not
    /// see the artifact this was added for. Every cache in the renderer that
    /// can fall behind a camera converges the moment the camera stops: the
    /// terrain clipmap's rings re-centre, the cascade cache's quadrants go
    /// clean, ReSTIR's history refills. A held camera therefore proves the
    /// steady state and nothing else, and a bug that only exists while
    /// something is chasing is invisible to it.
    Flyover { metres_per_second: f32 },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DreamRail {
    anchor: CameraPose,
    start_frame: u64,
    kind: RailKind,
}

impl DreamRail {
    pub(crate) fn named(name: &str, anchor: CameraPose, start_frame: u64) -> Option<Self> {
        let kind = match name {
            "coastal-ground" => RailKind::Hold {
                lateral_metres: 1.5,
            },
            "island-ground" => RailKind::Hold {
                lateral_metres: 1.0,
            },
            // Chosen to match what a person actually does to provoke this:
            // the editor's own speed slider reaches into the hundreds, and the
            // reports that prompted the rail were at 121 and 205 m/s.
            "coastal-flyover" => RailKind::Flyover {
                metres_per_second: 200.0,
            },
            "island-flyover" => RailKind::Flyover {
                metres_per_second: 200.0,
            },
            _ => return None,
        };
        Some(Self {
            anchor,
            start_frame,
            kind,
        })
    }

    /// Whether this rail is still moving at `frame`.
    ///
    /// A capture taken while this is false is a steady-state capture whatever
    /// the rail is called, which is the distinction the fixture was missing.
    pub(crate) fn moving_at(self, frame: u64) -> bool {
        match self.kind {
            RailKind::Hold { .. } => frame.saturating_sub(self.start_frame) < MOVING_FRAMES,
            RailKind::Flyover { .. } => true,
        }
    }

    pub(crate) fn pose(self, frame: u64) -> CameraPose {
        let elapsed = frame.saturating_sub(self.start_frame);
        let yaw = self.anchor.yaw.to_radians();
        match self.kind {
            RailKind::Hold { lateral_metres } => {
                if elapsed >= MOVING_FRAMES {
                    return self.anchor;
                }
                let t = elapsed as f32 / MOVING_FRAMES as f32;
                let phase = t * std::f32::consts::TAU;
                let right = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
                CameraPose {
                    position: self.anchor.position
                        + right * (phase.sin() * lateral_metres)
                        + Vec3::Y * ((phase * 0.5).sin() * 0.35),
                    yaw: self.anchor.yaw + phase.sin() * 2.0,
                    pitch: self.anchor.pitch + phase.cos().mul_add(0.75, -0.75),
                }
            }
            RailKind::Flyover { metres_per_second } => {
                // Frame-indexed at a nominal 60 Hz, like the hold rail, so a
                // slow machine lands the capture in the same place rather than
                // somewhere further down the track.
                let seconds = elapsed as f32 / 60.0;
                let pitch = self.anchor.pitch.to_radians();
                // The camera's own convention, from `EditorCamera::forward_vector`.
                let forward = Vec3::new(
                    yaw.cos() * pitch.cos(),
                    pitch.sin(),
                    yaw.sin() * pitch.cos(),
                )
                .normalize_or_zero();
                // Level flight: the heading follows the camera, the altitude
                // does not, or a downward pitch would fly it into the ground
                // and every late frame would be underground rather than
                // looking at terrain.
                let heading = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
                CameraPose {
                    position: self.anchor.position + heading * (metres_per_second * seconds),
                    ..self.anchor
                }
            }
        }
    }
}

/// How long a hold rail sways before it settles.
const MOVING_FRAMES: u64 = 120;

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
        assert!(DreamRail::named("coastal-flyover", anchor(), 7).is_some());
        assert!(DreamRail::named("coatsal-ground", anchor(), 7).is_none());
        assert!(DreamRail::named("coastal-flyver", anchor(), 7).is_none());
    }

    #[test]
    fn a_flyover_never_holds_and_covers_real_ground() {
        // The property the hold rails do not have, and the one every
        // cache-lag bug needs: at the capture frame the camera is still
        // moving, and it has travelled far enough that a ring or a cascade
        // centred where it started is nowhere near it.
        let rail = DreamRail::named("coastal-flyover", anchor(), 0).unwrap();
        assert!(rail.moving_at(240));
        let start = rail.pose(0).position;
        let at_capture = rail.pose(240).position;
        let travelled = start.distance(at_capture);
        assert!(
            travelled > 500.0,
            "240 frames at 200 m/s is 800 m; travelled {travelled}"
        );
        // Level: altitude is the anchor's, so a pitched-down camera does not
        // fly into the terrain before the capture frame.
        assert_eq!(at_capture.y, anchor().position.y);
    }

    #[test]
    fn a_hold_rail_still_holds() {
        // The measurement rails must not have changed: DREAMS-B's numbers were
        // taken with them and have to stay comparable.
        let rail = DreamRail::named("coastal-ground", anchor(), 10).unwrap();
        assert!(rail.moving_at(60));
        assert!(!rail.moving_at(240));
        assert_eq!(rail.pose(240), anchor());
    }

    #[test]
    fn a_flyover_is_frame_indexed_rather_than_wall_clock() {
        let rail = DreamRail::named("island-flyover", anchor(), 0).unwrap();
        let a = rail.pose(120).position;
        let b = rail.pose(240).position;
        let c = rail.pose(360).position;
        // Equal frame steps are equal distances, so a slow machine lands the
        // capture in the same place.
        let first = a.distance(b);
        let second = b.distance(c);
        assert!((first - second).abs() < 0.01, "{first} vs {second}");
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
