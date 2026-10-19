//! The switch between two protagonists (Rimeborn's Caelan and Rhun, DUAL_PROTAGONISTS 3 and DP-1; 19 Oct 2026):
//! the camera lifts off the one being played, the other's cells stream in under the lift, and the camera falls onto
//! him; as the fall begins, control is his.
//!
//! The engine's part is the timing: [`Switch::begin`] holds the destination's cells (`cell_streaming::hold`) and starts
//! a [`Lift`]; [`Switch::update`] runs it, holding at the top until those cells are in (or a cap passes, so a broken
//! cell never strands the camera), and says once, in [`SwitchFrame::hand_over`], when control changes hands. What
//! changes hands is the game's: which fighter takes the input (`somnium_combat`'s `player`), the HUD's subject, and the
//! one left going to the life sim on his schedule. When the switch is done the hold is released and the rig snaps to
//! [`Switch::destination`].

use glam::Vec3;
use somnium_ecs::World;

use crate::camera_lift::{Lift, LiftPhase, LiftShape};
use crate::cell_streaming::{self, CellStreamingComponent};
use crate::isometric_camera::{CameraPose, IsometricCameraComponent, IsometricRig};

/// The streaming hold a switch keeps on its destination while it runs.
pub const HOLD_KEY: &str = "protagonist_switch";
/// The longest a switch waits at the top for its destination's cells, seconds (then it falls anyway).
pub const MAX_WAIT: f32 = 8.0;

/// A frame of a switch.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SwitchFrame {
    /// Where the camera is this frame (the rig yields to it while the switch runs).
    pub camera: CameraPose,
    /// Where the lift is.
    pub phase: LiftPhase,
    /// True on the one frame control passes to the arriving protagonist (the fall begins).
    pub hand_over: bool,
    /// The switch is over: snap the rig to [`Switch::destination`] and follow the arrived protagonist.
    pub done: bool,
}

/// One switch from where the rig looks now to the other protagonist.
#[derive(Clone, Debug)]
pub struct Switch {
    lift: Lift,
    handed: bool,
    waited: f32,
    released: bool,
}

impl Switch {
    /// Begin a switch to the protagonist standing at `to` (feet): hold his cells and lift off.
    pub fn begin(world: &mut World, rig: &IsometricRig, settings: &IsometricCameraComponent, to: Vec3) -> Self {
        Self::begin_shaped(world, rig, settings, to, LiftShape::default())
    }

    /// [`Self::begin`] with a lift of the game's own shape (its height, its legs' seconds).
    pub fn begin_shaped(
        world: &mut World,
        rig: &IsometricRig,
        settings: &IsometricCameraComponent,
        to: Vec3,
        shape: LiftShape,
    ) -> Self {
        cell_streaming::hold(world, HOLD_KEY, to);
        Self { lift: Lift::new(rig, settings, to, shape), handed: false, waited: 0.0, released: false }
    }

    /// The focus the rig snaps to when the switch is done.
    #[must_use]
    pub fn destination(&self) -> Vec3 {
        self.lift.destination()
    }

    /// One frame. The destination is ready when the scene streams no cells, when its hold's cells are all in, or
    /// after [`MAX_WAIT`] at the top.
    pub fn update(&mut self, world: &mut World, dt: f32) -> SwitchFrame {
        let streams = world.first_with::<CellStreamingComponent>().is_some_and(|(_, c)| c.enabled);
        if self.lift.phase() == LiftPhase::Holding {
            self.waited += dt.max(0.0);
        }
        let ready = !streams || self.waited >= MAX_WAIT || cell_streaming::hold_ready(world, HOLD_KEY);
        let (camera, phase) = self.lift.update(dt, ready);
        let hand_over = !self.handed && matches!(phase, LiftPhase::Falling | LiftPhase::Done);
        self.handed |= hand_over;
        let done = phase == LiftPhase::Done;
        if done && !self.released {
            cell_streaming::release(world, HOLD_KEY);
            self.released = true;
        }
        SwitchFrame { camera, phase, hand_over, done }
    }

    /// Give up the switch (the player was attacked as it began, a cut scene took over): the hold is released and
    /// control stays where it was.
    pub fn cancel(self, world: &mut World) {
        if !self.released {
            cell_streaming::release(world, HOLD_KEY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> (IsometricRig, IsometricCameraComponent) {
        let settings = IsometricCameraComponent::default();
        let mut rig = IsometricRig::new(&settings, Vec3::ZERO);
        rig.snap();
        (rig, settings)
    }

    /// Run until done; returns the frames.
    fn run(switch: &mut Switch, world: &mut World, mut step: impl FnMut(&mut World, &SwitchFrame)) -> Vec<SwitchFrame> {
        let mut frames = Vec::new();
        for _ in 0..60 * 30 {
            let frame = switch.update(world, 1.0 / 60.0);
            step(world, &frame);
            frames.push(frame);
            if frame.done {
                break;
            }
        }
        frames
    }

    #[test]
    fn without_streaming_it_lifts_hands_over_once_as_it_falls_and_lands() {
        let mut world = World::new();
        let (rig, settings) = rig();
        let mut switch = Switch::begin(&mut world, &rig, &settings, Vec3::new(4000.0, 0.0, -2000.0));
        let frames = run(&mut switch, &mut world, |_, _| {});
        assert!(frames.last().unwrap().done);
        let handovers: Vec<_> = frames.iter().filter(|f| f.hand_over).collect();
        assert_eq!(handovers.len(), 1, "control changes hands once");
        assert_eq!(handovers[0].phase, LiftPhase::Falling, "as the fall begins");
        let seconds = frames.len() as f32 / 60.0;
        assert!((3.0..=6.0).contains(&seconds), "{seconds}");
        assert!(!cell_streaming::hold_ready(&mut world, HOLD_KEY), "released when done");
    }

    #[test]
    fn with_streaming_it_waits_at_the_top_for_the_destinations_cells() {
        let mut world = World::new();
        world.spawn((CellStreamingComponent { enabled: true, ..Default::default() },));
        let (rig, settings) = rig();
        let to = Vec3::new(4000.0, 0.0, -2000.0);
        let mut switch = Switch::begin(&mut world, &rig, &settings, to);
        // The streamer brings the cells in after five seconds (it marks the hold ready, as the app does).
        let mut t = 0.0;
        let frames = run(&mut switch, &mut world, |world, _| {
            t += 1.0 / 60.0;
            if t > 5.0 {
                let mut holds: cell_streaming::Holds = world.take_cache();
                holds.mark_ready(&[], 600.0, |_| true);
                world.put_cache(holds);
            }
        });
        let handover = frames.iter().position(|f| f.hand_over).unwrap() as f32 / 60.0;
        assert!(handover > 5.0, "not before the cells are in: {handover}");
        assert!(frames.iter().any(|f| f.phase == LiftPhase::Holding));
        assert!(frames.last().unwrap().done);
    }

    #[test]
    fn a_destination_that_never_streams_in_does_not_strand_the_camera_and_a_cancel_lets_go() {
        let mut world = World::new();
        world.spawn((CellStreamingComponent { enabled: true, ..Default::default() },));
        let (rig, settings) = rig();
        let mut switch = Switch::begin(&mut world, &rig, &settings, Vec3::new(900.0, 0.0, 0.0));
        let frames = run(&mut switch, &mut world, |_, _| {});
        assert!(frames.last().unwrap().done, "it falls after the cap");
        let seconds = frames.len() as f32 / 60.0;
        assert!(seconds < MAX_WAIT + 6.0, "{seconds}");
        // Cancelled mid-lift: the hold goes, control stays.
        let switch = Switch::begin(&mut world, &rig, &settings, Vec3::new(900.0, 0.0, 0.0));
        switch.cancel(&mut world);
        let holds: cell_streaming::Holds = world.take_cache();
        assert_eq!(holds.points().count(), 0);
    }
}
