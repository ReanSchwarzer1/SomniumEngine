//! The switch between two protagonists (Rimeborn's Caelan and Rhun, DUAL_PROTAGONISTS 3 and DP-1; 19 Oct 2026):
//! the camera lifts off the one being played, the other's cells stream in under the lift, and the camera falls onto
//! him; as the fall begins, control is his.
//!
//! The engine's part is the timing:
//! - [`Switch::begin`] holds the destination's cells (`cell_streaming::hold`), pins the streaming view at the place it
//!   leaves (so the crossing starts no readers for the land beneath it) and starts a [`Lift`];
//! - [`Switch::update`] runs it, holding high above the destination for as long as its cells are not in (the game
//!   shows a veil, by [`SwitchFrame::held_for`], and crossfades to its map by [`SwitchFrame::altitude`]), and says
//!   once, in [`SwitchFrame::hand_over`], when control changes hands.
//!
//! What changes hands is the game's: which fighter takes the input, the HUD's subject, the one left going to the life
//! sim on his schedule. The player asks for a switch with [`ACTION`] (hold Left Alt, rebindable: [`switch_map`]).

use glam::Vec3;
use somnium_ecs::World;
use somnium_input::{Action, ActionMap, Binding, ControlPath};

use crate::camera_lift::{Lift, LiftPhase, LiftShape};
use crate::cell_streaming::{self, CellStreamingComponent};
use crate::isometric_camera::{CameraPose, IsometricCameraComponent, IsometricRig};

/// The streaming hold a switch keeps on its destination while it runs (one of `cell_streaming::MAX_HOLDS`).
pub const HOLD_KEY: &str = "protagonist_switch";
/// The switch map's name.
pub const MAP: &str = "protagonist";
/// Its one action: held, the switch wheel shows (Tab is combat's Focus).
pub const ACTION: &str = "SwitchProtagonist";

/// The switch key: hold Left Alt. Rebind with `somnium_input::rebind::rebind(maps, MAP, ACTION, path)`.
#[must_use]
pub fn switch_map() -> ActionMap {
    ActionMap::new(MAP).action(Action::digital(ACTION).bind(Binding::single(ControlPath::keyboard("altleft"))))
}

/// A frame of a switch.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SwitchFrame {
    /// Where the camera is this frame (the rig yields to it while the switch runs).
    pub camera: CameraPose,
    /// Where the lift is.
    pub phase: LiftPhase,
    /// How high, 0 (the rig's view) to 1 (the top): crossfade to the world map from about 0.6.
    pub altitude: f32,
    /// Seconds waited at the top for the destination's cells: past a moment, the game shows its veil (a card on the
    /// map) until they are in.
    pub held_for: f32,
    /// True on the one frame control passes to the arriving protagonist (the fall begins).
    pub hand_over: bool,
    /// The switch is over: snap the rig to [`Switch::destination`] and follow the arrived protagonist.
    pub done: bool,
}

/// One switch from where the rig looks now to the other protagonist.
#[derive(Clone, Debug)]
pub struct Switch {
    lift: Lift,
    /// The hold the switch waits on, and whether it is the switch's own (released when done).
    key: Option<String>,
    owned: bool,
    handed: bool,
    held_for: f32,
    released: bool,
}

impl Switch {
    /// Begin a switch to the protagonist standing at `to` (feet): hold his cells, pin the view, lift off.
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
        let key = if cell_streaming::hold(world, HOLD_KEY, to) {
            Some(HOLD_KEY.to_owned())
        } else {
            // Both holds are taken: a joint mission should switch with `begin_on` and the other man's own hold.
            tracing::warn!("protagonist switch: no hold left for the destination; it is not waited for");
            None
        };
        let owned = key.is_some();
        Self::start(world, rig, settings, to, shape, key, owned)
    }

    /// A switch to a protagonist whose place the game already holds under `key` (a joint or parallel mission holds
    /// both men): it waits on that hold and leaves it held when done.
    pub fn begin_on(
        world: &mut World,
        rig: &IsometricRig,
        settings: &IsometricCameraComponent,
        to: Vec3,
        key: &str,
    ) -> Self {
        Self::start(world, rig, settings, to, LiftShape::default(), Some(key.to_owned()), false)
    }

    fn start(
        world: &mut World,
        rig: &IsometricRig,
        settings: &IsometricCameraComponent,
        to: Vec3,
        shape: LiftShape,
        key: Option<String>,
        owned: bool,
    ) -> Self {
        cell_streaming::pin_view(world, Some(rig.focus));
        Self { lift: Lift::new(rig, settings, to, shape), key, owned, handed: false, held_for: 0.0, released: false }
    }

    /// The focus the rig snaps to when the switch is done.
    #[must_use]
    pub fn destination(&self) -> Vec3 {
        self.lift.destination()
    }

    /// One frame. The destination is ready when the scene streams no cells, or when its hold's cells are all in (a
    /// cell that failed to load counts as in, so a broken file never strands the camera).
    pub fn update(&mut self, world: &mut World, dt: f32) -> SwitchFrame {
        let streams = world.first_with::<CellStreamingComponent>().is_some_and(|(_, c)| c.enabled);
        if self.lift.phase() == LiftPhase::Holding {
            self.held_for += dt.max(0.0);
        }
        let ready = !streams || self.key.as_deref().is_none_or(|key| cell_streaming::hold_ready(world, key));
        let (camera, phase) = self.lift.update(dt, ready);
        let hand_over = !self.handed && matches!(phase, LiftPhase::Falling | LiftPhase::Done);
        if hand_over {
            // The camera is over the destination now: it is the view again.
            cell_streaming::pin_view(world, None);
            self.handed = true;
        }
        let done = phase == LiftPhase::Done;
        if done {
            self.finish(world);
        }
        SwitchFrame { camera, phase, altitude: self.lift.altitude(), held_for: self.held_for, hand_over, done }
    }

    fn finish(&mut self, world: &mut World) {
        if !self.released {
            cell_streaming::pin_view(world, None);
            if let (true, Some(key)) = (self.owned, self.key.as_deref()) {
                cell_streaming::release(world, key);
            }
            self.released = true;
        }
    }

    /// Give up the switch (the player was attacked as it began, a cut scene took over): the view is unpinned, the
    /// switch's own hold released, and control stays where it was.
    pub fn cancel(mut self, world: &mut World) {
        self.finish(world);
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

    fn pinned(world: &mut World) -> Option<Vec3> {
        let holds: cell_streaming::Holds = world.take_cache();
        let pin = holds.pinned();
        world.put_cache(holds);
        pin
    }

    /// Run until done or `seconds` pass; returns the frames.
    fn run(
        switch: &mut Switch,
        world: &mut World,
        seconds: f32,
        mut step: impl FnMut(&mut World, &SwitchFrame),
    ) -> Vec<SwitchFrame> {
        let mut frames = Vec::new();
        for _ in 0..(seconds * 60.0) as usize {
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
        assert_eq!(pinned(&mut world), Some(rig.focus), "the view stays where it left while the camera is up");
        let frames = run(&mut switch, &mut world, 30.0, |_, _| {});
        assert!(frames.last().unwrap().done);
        let handovers: Vec<_> = frames.iter().filter(|f| f.hand_over).collect();
        assert_eq!(handovers.len(), 1, "control changes hands once");
        assert_eq!(handovers[0].phase, LiftPhase::Falling, "as the fall begins");
        let seconds = frames.len() as f32 / 60.0;
        assert!((3.0..=6.0).contains(&seconds), "{seconds}");
        assert!(frames.iter().any(|f| f.altitude == 1.0) && frames.last().unwrap().altitude == 0.0);
        assert!(!cell_streaming::hold_ready(&mut world, HOLD_KEY), "released when done");
        assert_eq!(pinned(&mut world), None);
    }

    #[test]
    fn with_streaming_it_holds_behind_the_veil_for_as_long_as_the_cells_take() {
        let mut world = World::new();
        world.spawn((CellStreamingComponent { enabled: true, ..Default::default() },));
        let (rig, settings) = rig();
        let mut switch = Switch::begin(&mut world, &rig, &settings, Vec3::new(4000.0, 0.0, -2000.0));
        // The cells take twenty seconds (the streamer marks the hold ready, as the app does).
        let mut t = 0.0;
        let frames = run(&mut switch, &mut world, 40.0, |world, _| {
            t += 1.0 / 60.0;
            if t > 20.0 {
                let mut holds: cell_streaming::Holds = world.take_cache();
                holds.mark_ready(&[], |_| true);
                world.put_cache(holds);
            }
        });
        let handover = frames.iter().position(|f| f.hand_over).unwrap() as f32 / 60.0;
        assert!(handover >= 20.0 - 1.0 / 60.0, "not before the cells are in, however long: {handover}");
        let veil = frames.iter().map(|f| f.held_for).fold(0.0, f32::max);
        assert!(veil > 15.0, "the veil's clock: {veil}");
        assert!(frames.last().unwrap().done);
    }

    #[test]
    fn a_joint_mission_switches_on_the_other_mans_hold_and_a_cancel_lets_go() {
        let mut world = World::new();
        world.spawn((CellStreamingComponent { enabled: true, ..Default::default() },));
        let (rig, settings) = rig();
        let rhun = Vec3::new(900.0, 0.0, 0.0);
        // Both men held by the game: the cap is reached.
        assert!(cell_streaming::hold(&mut world, "caelan", Vec3::ZERO));
        assert!(cell_streaming::hold(&mut world, "rhun", rhun));
        let mut switch = Switch::begin_on(&mut world, &rig, &settings, rhun, "rhun");
        let mut holds: cell_streaming::Holds = world.take_cache();
        holds.mark_ready(&[], |_| true);
        world.put_cache(holds);
        let frames = run(&mut switch, &mut world, 30.0, |_, _| {});
        assert!(frames.last().unwrap().done);
        assert!(cell_streaming::hold_ready(&mut world, "rhun"), "the game's hold stays held");
        // Cancelled mid-lift: the view is unpinned, control stays.
        cell_streaming::release(&mut world, "caelan");
        let switch = Switch::begin(&mut world, &rig, &settings, Vec3::new(50.0, 0.0, 0.0));
        assert!(pinned(&mut world).is_some());
        switch.cancel(&mut world);
        assert_eq!(pinned(&mut world), None);
        assert!(!cell_streaming::hold_ready(&mut world, HOLD_KEY));
    }

    #[test]
    fn the_switch_key_is_left_alt_and_rebinds() {
        let mut maps = vec![switch_map()];
        let bound = |maps: &[ActionMap]| maps[0].find(ACTION).unwrap().bindings[0].paths()[0].control.clone();
        assert_eq!(bound(&maps), "altleft");
        somnium_input::rebind::rebind(&mut maps, MAP, ACTION, ControlPath::keyboard("g")).unwrap();
        assert_eq!(bound(&maps), "g");
    }
}
