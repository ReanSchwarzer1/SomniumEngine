//! The lift: the isometric camera rising off one place, crossing high above the world and falling onto another
//! (Rimeborn's protagonist switch and its "meanwhile" cuts, DUAL_PROTAGONISTS 3 and DP-3; 19 Oct 2026).
//!
//! It is drawn with the rig's own [`pose`]: it starts from the rig's view of the start and ends on the rig's view of the
//! destination, so neither end jumps. Between them the view draws back to a height and steepens to look down, crosses,
//! and comes down again. The top stretches to hide streaming: a lift that reaches the destination before the game says
//! it is ready ([`Lift::update`]'s `ready`) holds there, high up, until it is.

use glam::Vec3;

use crate::isometric_camera::{CameraPose, IsometricCameraComponent, IsometricRig, pose};

/// Where a lift is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiftPhase {
    /// Drawing back and up off the start.
    Rising,
    /// High up, crossing to above the destination.
    Crossing,
    /// High above the destination, waiting for it to be ready.
    Holding,
    /// Coming down onto the destination. The arriving side takes control as this begins.
    Falling,
    /// On the destination, at the rig's own view: the rig takes over.
    Done,
}

/// The lift's shape: its height and how long each leg takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiftShape {
    /// How far back the view draws at the top, metres (the Lid's height, as the switch is told).
    pub height: f32,
    /// How steeply it looks down at the top, degrees.
    pub top_pitch: f32,
    /// Seconds to rise.
    pub rise: f32,
    /// Seconds to fall.
    pub fall: f32,
    /// Metres a second across.
    pub cross_speed: f32,
    /// The fewest seconds a crossing takes, however short.
    pub min_cross: f32,
    /// The most seconds a crossing takes, however long.
    pub max_cross: f32,
}

impl Default for LiftShape {
    /// A 3 to 6 second switch at game scale (DUAL_PROTAGONISTS 3).
    fn default() -> Self {
        Self { height: 900.0, top_pitch: 80.0, rise: 1.3, fall: 1.4, cross_speed: 4000.0, min_cross: 0.6, max_cross: 2.5 }
    }
}

/// One lift from the rig's view of `from` to its view of `to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lift {
    from: Vec3,
    to: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    shape: LiftShape,
    cross: f32,
    /// Seconds into the current phase.
    t: f32,
    phase: LiftPhase,
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Lift {
    /// A lift from where `rig` looks now to where it will look when it follows `to` (feet, as `IsometricRig::follow`
    /// takes them).
    #[must_use]
    pub fn new(rig: &IsometricRig, settings: &IsometricCameraComponent, to: Vec3, shape: LiftShape) -> Self {
        let to = to + Vec3::Y * settings.focus_height;
        let along = Vec3::new(to.x - rig.focus.x, 0.0, to.z - rig.focus.z).length();
        let cross = (along / shape.cross_speed.max(1.0)).clamp(shape.min_cross, shape.max_cross.max(shape.min_cross));
        Self {
            from: rig.focus,
            to,
            yaw: rig.yaw,
            pitch: settings.pitch,
            distance: rig.distance,
            shape,
            cross,
            t: 0.0,
            phase: LiftPhase::Rising,
        }
    }

    /// Where it is.
    #[must_use]
    pub fn phase(&self) -> LiftPhase {
        self.phase
    }

    /// The focus the rig should snap to when the lift is done (the destination raised by the rig's focus height).
    #[must_use]
    pub fn destination(&self) -> Vec3 {
        self.to
    }

    /// Seconds it takes when the destination is ready at once (no hold).
    #[must_use]
    pub fn seconds(&self) -> f32 {
        self.shape.rise + self.cross + self.shape.fall
    }

    /// How high it is, 0 (the rig's own view) to 1 (the top, crossing or holding), eased as the camera moves. At the
    /// top the camera sees far past the cells streamed around either place, so a game crossfades to its world map
    /// from about 0.6 up and back out below it on the fall (DUAL_PROTAGONISTS 3).
    #[must_use]
    pub fn altitude(&self) -> f32 {
        match self.phase {
            LiftPhase::Rising => ease(self.t / self.shape.rise.max(1e-3)),
            LiftPhase::Crossing | LiftPhase::Holding => 1.0,
            LiftPhase::Falling => 1.0 - ease(self.t / self.shape.fall.max(1e-3)),
            LiftPhase::Done => 0.0,
        }
    }

    /// Advance by `dt`; `ready` says whether the destination can be shown (its cells are in). Returns the camera pose
    /// and the phase it is now in.
    pub fn update(&mut self, dt: f32, ready: bool) -> (CameraPose, LiftPhase) {
        self.t += dt.max(0.0);
        loop {
            let length = match self.phase {
                LiftPhase::Rising => self.shape.rise,
                LiftPhase::Crossing => self.cross,
                LiftPhase::Holding => {
                    if !ready {
                        // Waiting does not bank time: the fall starts from its top when the destination is ready.
                        self.t = 0.0;
                        break;
                    }
                    0.0
                }
                LiftPhase::Falling => self.shape.fall,
                LiftPhase::Done => break,
            };
            if self.t < length {
                break;
            }
            self.t -= length;
            self.phase = match self.phase {
                LiftPhase::Rising => LiftPhase::Crossing,
                LiftPhase::Crossing => LiftPhase::Holding,
                LiftPhase::Holding => LiftPhase::Falling,
                LiftPhase::Falling | LiftPhase::Done => LiftPhase::Done,
            };
            if self.phase == LiftPhase::Holding && !ready {
                self.t = 0.0;
                break;
            }
        }
        (self.pose_now(), self.phase)
    }

    fn pose_now(&self) -> CameraPose {
        let (from, to, low, high) = (self.from, self.to, self.distance, self.shape.height.max(self.distance));
        let (pitch, top) = (self.pitch, self.shape.top_pitch);
        let blend = |a: f32, b: f32, k: f32| a + (b - a) * k;
        match self.phase {
            LiftPhase::Rising => {
                let k = ease(self.t / self.shape.rise.max(1e-3));
                pose(from, self.yaw, blend(pitch, top, k), blend(low, high, k))
            }
            LiftPhase::Crossing => {
                let k = ease(self.t / self.cross.max(1e-3));
                pose(from.lerp(to, k), self.yaw, top, high)
            }
            LiftPhase::Holding => pose(to, self.yaw, top, high),
            LiftPhase::Falling => {
                let k = ease(self.t / self.shape.fall.max(1e-3));
                pose(to, self.yaw, blend(top, pitch, k), blend(high, low, k))
            }
            LiftPhase::Done => pose(to, self.yaw, pitch, low),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig_at(focus: Vec3) -> (IsometricRig, IsometricCameraComponent) {
        let settings = IsometricCameraComponent::default();
        let mut rig = IsometricRig::new(&settings, focus);
        rig.follow(focus, &settings);
        rig.snap();
        (rig, settings)
    }

    fn close(a: CameraPose, b: CameraPose) -> bool {
        // `angle_between` is NaN for equal rotations (acos just past 1); the dot product is not.
        a.position.distance(b.position) < 1e-3 && a.rotation.dot(b.rotation).abs() > 1.0 - 1e-6
    }

    #[test]
    fn it_leaves_and_lands_on_the_rigs_own_view_and_rises_between() {
        let (mut rig, settings) = rig_at(Vec3::new(0.0, 0.0, 0.0));
        let to = Vec3::new(3000.0, 12.0, -1500.0);
        let start = rig.update(0.0, &settings);
        let mut lift = Lift::new(&rig, &settings, to, LiftShape::default());
        let (first, phase) = lift.update(0.0, true);
        assert_eq!(phase, LiftPhase::Rising);
        assert!(close(first, start), "no jump off the start: {first:?} vs {start:?}");
        let mut highest: f32 = 0.0;
        let mut phases = vec![phase];
        let mut seconds = 0.0;
        while lift.phase() != LiftPhase::Done {
            let (camera, phase) = lift.update(1.0 / 60.0, true);
            highest = highest.max(camera.position.y);
            if phases.last() != Some(&phase) {
                phases.push(phase);
            }
            seconds += 1.0 / 60.0;
            assert!(seconds < 10.0, "it finishes");
        }
        assert_eq!(phases, [LiftPhase::Rising, LiftPhase::Crossing, LiftPhase::Falling, LiftPhase::Done], "ready at once: no hold");
        assert!((3.0..=6.0).contains(&seconds), "a 3 to 6 second switch: {seconds}");
        assert!(highest > 600.0, "up to the Lid's height: {highest}");
        // Where it lands is where the rig, following the destination, looks.
        let (end, _) = lift.update(0.0, true);
        rig.follow(to, &settings);
        rig.snap();
        assert!(close(end, rig.update(0.0, &settings)), "no jump onto the destination");
        assert_eq!(lift.destination(), to + Vec3::Y * settings.focus_height);
    }

    #[test]
    fn it_holds_high_above_the_destination_until_the_destination_is_ready() {
        let (rig, settings) = rig_at(Vec3::ZERO);
        let to = Vec3::new(500.0, 0.0, 0.0);
        let mut lift = Lift::new(&rig, &settings, to, LiftShape::default());
        for _ in 0..600 {
            lift.update(1.0 / 60.0, false);
        }
        let (held, phase) = lift.update(1.0 / 60.0, false);
        assert_eq!(phase, LiftPhase::Holding, "ten seconds unready: still up there");
        let above = held.position - (to + Vec3::Y * settings.focus_height);
        assert!(above.y > 600.0, "high over the destination: {above}");
        // Ready: it falls, and lands after the fall's time.
        let (_, phase) = lift.update(1.0 / 60.0, true);
        assert_eq!(phase, LiftPhase::Falling);
        let mut t = 0.0;
        while lift.update(1.0 / 60.0, true).1 != LiftPhase::Done {
            t += 1.0 / 60.0;
        }
        assert!((t - LiftShape::default().fall).abs() < 0.05, "{t}");
    }

    #[test]
    fn altitude_rises_to_one_holds_there_and_comes_back_down() {
        let (rig, settings) = rig_at(Vec3::ZERO);
        let mut lift = Lift::new(&rig, &settings, Vec3::new(2000.0, 0.0, 0.0), LiftShape::default());
        assert_eq!(lift.altitude(), 0.0);
        let mut last = 0.0;
        let mut peaked = false;
        while lift.phase() != LiftPhase::Done {
            lift.update(1.0 / 60.0, true);
            let a = lift.altitude();
            assert!((0.0..=1.0).contains(&a));
            if lift.phase() == LiftPhase::Rising {
                assert!(a >= last, "it only rises on the rise");
            }
            peaked |= a == 1.0;
            last = a;
        }
        assert!(peaked);
        assert_eq!(lift.altitude(), 0.0, "landed: the rig's own view");
    }

    #[test]
    fn a_short_hop_still_crosses_for_a_moment_and_a_long_one_is_capped() {
        let (rig, settings) = rig_at(Vec3::ZERO);
        let near = Lift::new(&rig, &settings, Vec3::new(5.0, 0.0, 0.0), LiftShape::default());
        let far = Lift::new(&rig, &settings, Vec3::new(40_000.0, 0.0, 0.0), LiftShape::default());
        let shape = LiftShape::default();
        assert!((near.seconds() - (shape.rise + shape.min_cross + shape.fall)).abs() < 1e-5);
        assert!((far.seconds() - (shape.rise + shape.max_cross + shape.fall)).abs() < 1e-5);
    }
}
