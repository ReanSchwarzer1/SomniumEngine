//! Input actions (Phase MORROWIND, MORROWIND-AE, Seam 5).
//!
//! # What this replaces
//!
//! `crates/somnium_core/src/script_input.rs` has **54** `KeyCode::` arms and
//! `examples/hello_engine/src/main.rs` has **16**. Every one of them is a
//! hard-coded key that a player cannot rebind, and the engine has no concept of
//! an action at all — §4.6's grep for `gamepad` and `action_map` returned zero.
//!
//! Seam 5:
//!
//! > *"Keycodes appear in exactly one place: the device layer that resolves a
//! > `ControlPath` to a hardware control. Game code, script and UI see actions.
//! > Rebinding is a runtime operation over the same data."*
//!
//! # The layers, and why there are four
//!
//! ```text
//!  winit events  ->  Devices          keycodes live here, and only here
//!                    ControlPath      <Keyboard>/w, <Gamepad2>/leftStick
//!                    Processor        dead zone, invert, scale, normalise
//!                    Interaction      press, hold, tap, multi-tap
//!  game code     <-  ActionValue      Digital | Analog1D | Analog2D
//! ```
//!
//! Each boundary buys something specific:
//!
//! - **Path over keycode** so a binding is *data* — a file a player edits, a
//!   setting that survives a restart, a control scheme shipped as content.
//! - **Processor** so inverting a Y axis is a player preference rather than a
//!   code change, and so a dead zone is radial in one place instead of
//!   per-axis in twenty.
//! - **Interaction** so "tap to reload, hold to holster" is two bindings on one
//!   control and neither knows the other exists.
//! - **Action value** so movement code reads one `Vec2` and never learns
//!   whether it came from WASD or a stick.
//!
//! # Rebinding
//!
//! [`rebind`] is a runtime operation over the same `ActionMap` the game reads,
//! with conflict detection that reports what a change *would* break before it
//! is applied. See that module for why conflicts are reported rather than
//! prevented.

#![deny(missing_docs)]

pub mod action;
pub mod device;
pub mod path;
pub mod processor;
pub mod rebind;

pub use action::{
    Action, ActionKind, ActionMap, ActionStates, ActionValue, Binding, ControlSource,
    PRESS_THRESHOLD,
};
pub use device::{Devices, PadButton, PadState};
pub use path::{ControlPath, DeviceKind, PathError};
pub use processor::{Interaction, Phase, Processor, RawValue};
pub use rebind::{Conflict, RebindListener, conflicts_for};

use glam::Vec2;

/// The whole input system: devices, maps and evaluated state.
///
/// One per game. A caller feeds it window events, calls [`Self::update`] once a
/// frame, and reads actions by name.
#[derive(Debug, Default)]
pub struct InputSystem {
    devices: Devices,
    maps: Vec<ActionMap>,
    states: ActionStates,
}

impl InputSystem {
    /// An empty system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A system with the default maps, for a game that has not authored its own.
    ///
    /// Not a convenience: **the default map is documentation.** It shows the
    /// vocabulary a game is expected to bind, and it is what `vvardenfell` and
    /// the UI's navigation verbs read, so the shape is exercised rather than
    /// merely described.
    #[must_use]
    pub fn with_default_maps() -> Self {
        let mut system = Self::new();
        system.add_map(default_gameplay_map());
        system.add_map(default_ui_map());
        system
    }

    /// Add a map.
    pub fn add_map(&mut self, map: ActionMap) {
        self.maps.push(map);
    }

    /// Enable or disable a map by name. Returns whether it was found.
    ///
    /// How context switching works: opening a menu disables `gameplay` and
    /// enables `ui`, and every action handler stops needing to know whether a
    /// menu is open.
    pub fn set_map_enabled(&mut self, name: &str, enabled: bool) -> bool {
        match self.maps.iter_mut().find(|m| m.name == name) {
            Some(map) => {
                map.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// The maps, for rebinding and for saving to settings.
    #[must_use]
    pub fn maps(&self) -> &[ActionMap] {
        &self.maps
    }

    /// The maps, mutably.
    pub fn maps_mut(&mut self) -> &mut Vec<ActionMap> {
        &mut self.maps
    }

    /// The device layer, for a platform backend to feed.
    pub fn devices_mut(&mut self) -> &mut Devices {
        &mut self.devices
    }

    /// The device layer.
    #[must_use]
    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    /// Fold a window event in.
    pub fn handle_window_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        self.devices.handle_window_event(event)
    }

    /// Accumulate raw mouse motion.
    pub fn add_mouse_delta(&mut self, delta: Vec2) {
        self.devices.add_mouse_delta(delta);
    }

    /// Evaluate every enabled map. Once per frame, before game logic reads it.
    pub fn update(&mut self, dt: f32) {
        self.states.update(&self.maps, &self.devices, dt);
        // Deltas are consumed by the evaluation above and must not survive it,
        // or the camera keeps turning after the mouse stops.
        self.devices.end_frame();
    }

    /// Evaluated state.
    #[must_use]
    pub fn actions(&self) -> &ActionStates {
        &self.states
    }

    /// Whether an action is active.
    #[must_use]
    pub fn is_active(&self, action: &str) -> bool {
        self.states.is_active(action)
    }

    /// Whether an action became active this frame.
    #[must_use]
    pub fn just_activated(&self, action: &str) -> bool {
        self.states.just_activated(action)
    }

    /// An action as a 2D vector.
    #[must_use]
    pub fn vec2(&self, action: &str) -> Vec2 {
        self.states.vec2(action)
    }

    /// An action as one axis.
    #[must_use]
    pub fn axis(&self, action: &str) -> f32 {
        self.states.axis(action)
    }
}

/// The default gameplay map.
///
/// The vocabulary `hello_engine`'s sixteen inline keycodes describe, written
/// once as data.
#[must_use]
pub fn default_gameplay_map() -> ActionMap {
    ActionMap::new("gameplay")
        .action(
            Action::vector2("Move")
                .bind(Binding::Vector2 {
                    up: ControlPath::keyboard("w"),
                    down: ControlPath::keyboard("s"),
                    left: ControlPath::keyboard("a"),
                    right: ControlPath::keyboard("d"),
                    processors: vec![Processor::Normalize],
                })
                .bind(
                    Binding::single(ControlPath::gamepad("leftstick"))
                        .with(Processor::stick_dead_zone()),
                ),
        )
        .action(
            Action::vector2("Look")
                .bind(Binding::single(ControlPath::mouse("delta")).with(Processor::Scale(0.1)))
                .bind(
                    Binding::single(ControlPath::gamepad("rightstick"))
                        .with(Processor::stick_dead_zone())
                        .with(Processor::Scale(2.0)),
                ),
        )
        .action(
            Action::digital("Jump")
                .bind(Binding::single(ControlPath::keyboard("space")))
                .bind(Binding::single(ControlPath::gamepad("buttonsouth"))),
        )
        .action(
            Action::digital("Sprint")
                .bind(Binding::single(ControlPath::keyboard("shiftleft")))
                .bind(Binding::single(ControlPath::gamepad("leftstickpress"))),
        )
        .action(
            Action::digital("Interact")
                .bind(Binding::single(ControlPath::keyboard("e")))
                .bind(Binding::single(ControlPath::gamepad("buttonwest"))),
        )
        .action(
            Action::digital("Pause")
                .bind(Binding::single(ControlPath::keyboard("escape")))
                .bind(Binding::single(ControlPath::gamepad("start"))),
        )
}

/// The default UI map — the navigation verbs MORROWIND-F defined.
///
/// **This is what closes F.** F's item 5 is a forward dependency on this map;
/// `somnium_ui`'s `NavAction` had a hard-coded keyboard default, and the names
/// here are the ones it now resolves against.
#[must_use]
pub fn default_ui_map() -> ActionMap {
    ActionMap::new("ui")
        .action(
            Action::vector2("Navigate")
                .bind(Binding::Vector2 {
                    up: ControlPath::keyboard("arrowup"),
                    down: ControlPath::keyboard("arrowdown"),
                    left: ControlPath::keyboard("arrowleft"),
                    right: ControlPath::keyboard("arrowright"),
                    processors: vec![],
                })
                .bind(Binding::Vector2 {
                    up: ControlPath::gamepad("dpadup"),
                    down: ControlPath::gamepad("dpaddown"),
                    left: ControlPath::gamepad("dpadleft"),
                    right: ControlPath::gamepad("dpadright"),
                    processors: vec![],
                })
                .bind(
                    Binding::single(ControlPath::gamepad("leftstick"))
                        .with(Processor::stick_dead_zone()),
                ),
        )
        .action(
            Action::digital("Confirm")
                .bind(Binding::single(ControlPath::keyboard("enter")))
                .bind(Binding::single(ControlPath::keyboard("space")))
                .bind(Binding::single(ControlPath::gamepad("buttonsouth"))),
        )
        .action(
            Action::digital("Cancel")
                .bind(Binding::single(ControlPath::keyboard("escape")))
                .bind(Binding::single(ControlPath::gamepad("buttoneast"))),
        )
        .action(Action::digital("Next").bind(Binding::single(ControlPath::keyboard("tab"))))
        .action(
            Action::digital("Previous").bind(Binding::single(ControlPath::gamepad("leftshoulder"))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Press or release a key by name, through the public device API.
    fn key(input: &mut InputSystem, name: &str, down: bool) {
        assert!(
            input.devices_mut().set_key(name, down),
            "unknown key {name}"
        );
    }

    /// End to end: a key press becomes an action value, with no keycode
    /// anywhere above the device layer.
    #[test]
    fn a_key_press_becomes_a_named_action() {
        let mut input = InputSystem::with_default_maps();
        key(&mut input, "space", true);
        input.update(0.016);
        assert!(input.is_active("Jump"));
        assert!(input.just_activated("Jump"));

        key(&mut input, "space", false);
        input.update(0.016);
        assert!(!input.is_active("Jump"));
    }

    #[test]
    fn wasd_drives_move_as_a_vector() {
        let mut input = InputSystem::with_default_maps();
        key(&mut input, "w", true);
        key(&mut input, "d", true);
        input.update(0.016);
        let moved = input.vec2("Move");
        assert!(
            (moved.length() - 1.0).abs() < 1e-3,
            "diagonal is not faster"
        );
        assert!(moved.y < 0.0 && moved.x > 0.0);
    }

    /// **Context switching is the point of maps.**
    ///
    /// Escape is bound in both maps. Only the enabled one fires, and no action
    /// handler needs an "is a menu open" flag.
    #[test]
    fn disabling_a_map_switches_context() {
        let mut input = InputSystem::with_default_maps();
        key(&mut input, "escape", true);

        input.update(0.016);
        assert!(input.is_active("Pause"));
        assert!(input.is_active("Cancel"), "both maps are on by default");

        assert!(input.set_map_enabled("gameplay", false));
        input.update(0.016);
        assert!(!input.is_active("Pause"));
        assert!(input.is_active("Cancel"));
    }

    #[test]
    fn setting_an_unknown_map_reports_it() {
        let mut input = InputSystem::new();
        assert!(!input.set_map_enabled("nope", false));
    }

    /// The default maps cover the vocabulary the engine's inline keycodes did.
    #[test]
    fn the_default_maps_name_the_verbs_the_engine_hard_coded() {
        let input = InputSystem::with_default_maps();
        for verb in ["Move", "Look", "Jump", "Sprint", "Interact", "Pause"] {
            assert!(
                input.maps().iter().any(|m| m.find(verb).is_some()),
                "{verb} is missing from the default gameplay map"
            );
        }
        // And MORROWIND-F's navigation verbs, which is what closes F.
        for verb in ["Navigate", "Confirm", "Cancel", "Next", "Previous"] {
            assert!(
                input.maps().iter().any(|m| m.find(verb).is_some()),
                "{verb} is missing from the default UI map"
            );
        }
    }

    /// Both default maps survive a round trip, because bindings are a file.
    #[test]
    fn the_default_maps_round_trip_through_json() {
        for map in [default_gameplay_map(), default_ui_map()] {
            let json = serde_json::to_string_pretty(&map).expect("serialises");
            let back: ActionMap = serde_json::from_str(&json).expect("deserialises");
            assert_eq!(map, back, "{} did not round-trip", map.name);
        }
    }

    /// Mouse delta reaches `Look` and does not survive the frame.
    #[test]
    fn look_reads_the_mouse_and_then_forgets_it() {
        let mut input = InputSystem::with_default_maps();
        input.add_mouse_delta(Vec2::new(20.0, 0.0));
        input.update(0.016);
        assert!(input.vec2("Look").x > 0.0);

        input.update(0.016);
        assert_eq!(
            input.vec2("Look"),
            Vec2::ZERO,
            "the camera stops when the mouse does"
        );
    }

    /// An unknown key name is reported rather than silently ignored.
    #[test]
    fn an_unknown_key_name_is_rejected() {
        let mut input = InputSystem::new();
        assert!(!input.devices_mut().set_key("hyperspace", true));
    }
}
