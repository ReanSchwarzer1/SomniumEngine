//! The device layer (MORROWIND-AE, Seam 5).
//!
//! > *"Keycodes appear in exactly one place: the device layer that resolves a
//! > `ControlPath` to a hardware control."*
//!
//! **This is that place.** `winit::keyboard::KeyCode` appears in this file and
//! nowhere else in the crate, and the census grep that counts `KeyCode::` arms
//! is what keeps it honest.
//!
//! # Names, not discriminants
//!
//! A binding says `<Keyboard>/w`, and this file maps `"w"` to `KeyCode::KeyW`.
//! The mapping is by *name* rather than by casting a discriminant, because a
//! settings file is data that outlives the enum: winit adding a variant must not
//! silently repoint every binding after it.

use crate::{action::ControlSource, path::{ControlPath, DeviceKind}, processor::RawValue};
use glam::Vec2;
use std::collections::HashMap;
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

/// Live hardware state, keyed by control path.
///
/// Implements [`ControlSource`], so everything above it is testable against a
/// fake and this is the only thing that needs a window.
#[derive(Debug, Default)]
pub struct Devices {
    keys: HashMap<KeyCode, bool>,
    mouse_buttons: HashMap<MouseButton, bool>,
    mouse_delta: Vec2,
    scroll: f32,
    /// Per-pad axes and buttons, indexed by pad. Fed by a platform gamepad
    /// backend; nothing in the tree provides one yet, and the shape is here so
    /// that when one lands it plugs in without the action layer changing.
    pads: HashMap<u8, PadState>,
}

/// One gamepad's controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct PadState {
    /// Left stick, y-down to match screen space.
    pub left_stick: Vec2,
    /// Right stick, y-down.
    pub right_stick: Vec2,
    /// Left trigger, `0..=1`.
    pub left_trigger: f32,
    /// Right trigger, `0..=1`.
    pub right_trigger: f32,
    /// Bitset of pressed buttons, indexed by [`PadButton`].
    pub buttons: u32,
    /// Whether this pad is currently connected. Hot-plug flips it; bindings
    /// paired to a disconnected pad read as zero rather than as stuck-down,
    /// which is what stops a yanked cable leaving a character walking forever.
    pub connected: bool,
}

/// The buttons a pad reports, in the order the bitset indexes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PadButton {
    /// A / Cross. Confirm.
    South,
    /// B / Circle. Cancel.
    East,
    /// X / Square.
    West,
    /// Y / Triangle.
    North,
    /// LB / L1.
    LeftShoulder,
    /// RB / R1.
    RightShoulder,
    /// Select / Back / Share.
    Select,
    /// Start / Menu / Options.
    Start,
    /// Left stick pressed in.
    LeftStick,
    /// Right stick pressed in.
    RightStick,
    /// D-pad up.
    DpadUp,
    /// D-pad down.
    DpadDown,
    /// D-pad left.
    DpadLeft,
    /// D-pad right.
    DpadRight,
}

impl PadButton {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "buttonsouth" | "a" | "cross" => Self::South,
            "buttoneast" | "b" | "circle" => Self::East,
            "buttonwest" | "x" | "square" => Self::West,
            "buttonnorth" | "y" | "triangle" => Self::North,
            "leftshoulder" | "lb" => Self::LeftShoulder,
            "rightshoulder" | "rb" => Self::RightShoulder,
            "select" | "back" => Self::Select,
            "start" | "menu" => Self::Start,
            "leftstickpress" => Self::LeftStick,
            "rightstickpress" => Self::RightStick,
            "dpad/up" | "dpadup" => Self::DpadUp,
            "dpad/down" | "dpaddown" => Self::DpadDown,
            "dpad/left" | "dpadleft" => Self::DpadLeft,
            "dpad/right" | "dpadright" => Self::DpadRight,
            _ => return None,
        })
    }
}

impl PadState {
    /// Whether `button` is down.
    #[must_use]
    pub fn is_down(&self, button: PadButton) -> bool {
        self.buttons & (1 << button as u32) != 0
    }

    /// Set `button`'s state.
    pub fn set(&mut self, button: PadButton, down: bool) {
        let bit = 1 << button as u32;
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
    }
}

impl Devices {
    /// Empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a window event into the device state.
    ///
    /// Returns `true` when something changed, so a caller can skip re-evaluating
    /// actions on an event that touched nothing.
    pub fn handle_window_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return false;
                };
                // **Physical, not logical.** A binding to `<Keyboard>/w` must be
                // the same physical key on an AZERTY keyboard, or a French
                // player's WASD lands on ZQSD-shaped nonsense. Logical keys are
                // for text entry; bindings are positional.
                self.keys.insert(code, event.state == ElementState::Pressed);
                true
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_buttons
                    .insert(*button, *state == ElementState::Pressed);
                true
            }
            WindowEvent::CursorMoved { .. } => false,
            WindowEvent::MouseWheel { delta, .. } => {
                self.scroll += match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 120.0,
                };
                true
            }
            WindowEvent::Focused(false) => {
                // **Releasing everything on focus loss is not optional.** A key
                // released while the window is unfocused never reports, so a
                // player who alt-tabs mid-sprint comes back sprinting into a
                // wall with no way to stop.
                self.release_all();
                true
            }
            _ => false,
        }
    }

    /// Set a key's state by name.
    ///
    /// The path a replay, a remapping tool and a test all take. It exists
    /// because synthesising a `winit::event::KeyEvent` is not portable — its
    /// `platform_specific` field is a different type on every platform and is
    /// not `Default` on Windows — so a test that wanted to press a key had no
    /// way to. Naming it a real API rather than a `#[cfg(test)]` hook is the
    /// honest form: a replay system needs exactly this.
    ///
    /// Returns `false` for a name this build does not know.
    pub fn set_key(&mut self, name: &str, down: bool) -> bool {
        match key_code(name) {
            Some(code) => {
                self.keys.insert(code, down);
                true
            }
            None => false,
        }
    }

    /// Set a mouse button's state by name.
    pub fn set_mouse_button(&mut self, name: &str, down: bool) -> bool {
        match mouse_button(name) {
            Some(button) => {
                self.mouse_buttons.insert(button, down);
                true
            }
            None => false,
        }
    }

    /// Accumulate raw mouse motion, which does not arrive as a window event.
    pub fn add_mouse_delta(&mut self, delta: Vec2) {
        self.mouse_delta += delta;
    }

    /// Replace a pad's state. Called by a platform backend.
    pub fn set_pad(&mut self, index: u8, state: PadState) {
        self.pads.insert(index, state);
    }

    /// Mark a pad disconnected, zeroing its controls.
    pub fn disconnect_pad(&mut self, index: u8) {
        if let Some(pad) = self.pads.get_mut(&index) {
            *pad = PadState::default();
        }
    }

    /// Connected pad indices, ascending.
    #[must_use]
    pub fn connected_pads(&self) -> Vec<u8> {
        let mut out: Vec<_> = self
            .pads
            .iter()
            .filter(|(_, pad)| pad.connected)
            .map(|(index, _)| *index)
            .collect();
        out.sort_unstable();
        out
    }

    /// Clear per-frame deltas. Call after evaluating actions.
    ///
    /// Mouse delta and scroll are *deltas*: not clearing them makes the camera
    /// keep turning after the mouse stops, which reads as drift.
    pub fn end_frame(&mut self) {
        self.mouse_delta = Vec2::ZERO;
        self.scroll = 0.0;
    }

    /// Release every control.
    pub fn release_all(&mut self) {
        for down in self.keys.values_mut() {
            *down = false;
        }
        for down in self.mouse_buttons.values_mut() {
            *down = false;
        }
        self.mouse_delta = Vec2::ZERO;
        self.scroll = 0.0;
    }

    /// Every control currently pressed, as a path. Rebinding listens on this.
    #[must_use]
    pub fn pressed_paths(&self) -> Vec<ControlPath> {
        let mut out = Vec::new();
        for (code, down) in &self.keys {
            if *down && let Some(name) = key_name(*code) {
                out.push(ControlPath::keyboard(name));
            }
        }
        for (button, down) in &self.mouse_buttons {
            if *down && let Some(name) = mouse_button_name(*button) {
                out.push(ControlPath::mouse(name));
            }
        }
        for (index, pad) in &self.pads {
            if !pad.connected {
                continue;
            }
            for button in PAD_BUTTON_NAMES {
                if let Some(b) = PadButton::from_name(button)
                    && pad.is_down(b)
                {
                    out.push(ControlPath::gamepad(button).on_device(*index));
                }
            }
        }
        out.sort_by_key(ToString::to_string);
        out
    }
}

const PAD_BUTTON_NAMES: &[&str] = &[
    "buttonsouth",
    "buttoneast",
    "buttonwest",
    "buttonnorth",
    "leftshoulder",
    "rightshoulder",
    "select",
    "start",
    "leftstickpress",
    "rightstickpress",
    "dpadup",
    "dpaddown",
    "dpadleft",
    "dpadright",
];

impl ControlSource for Devices {
    fn read(&self, path: &ControlPath) -> RawValue {
        match path.device {
            DeviceKind::Keyboard => {
                let down = key_code(&path.control)
                    .and_then(|code| self.keys.get(&code).copied())
                    .unwrap_or(false);
                RawValue::Digital(down)
            }
            DeviceKind::Mouse => match path.control.as_str() {
                "delta" => RawValue::Analog2D(self.mouse_delta),
                "scroll" => RawValue::Analog1D(self.scroll),
                other => RawValue::Digital(
                    mouse_button(other)
                        .and_then(|b| self.mouse_buttons.get(&b).copied())
                        .unwrap_or(false),
                ),
            },
            DeviceKind::Gamepad => {
                let pads: Vec<&PadState> = self
                    .pads
                    .iter()
                    .filter(|(index, pad)| pad.connected && path.accepts_device(**index))
                    .map(|(_, pad)| pad)
                    .collect();
                // Unpaired bindings read whichever pad is pushed furthest, so a
                // single-player game works on whatever pad the player picks up.
                //
                // `zero` is the control's own shape, not `Digital(false)`: an
                // absent stick must still read as a 2D zero, or a `Vector2`
                // action bound to a disconnected pad receives a scalar and
                // `convert` turns it into `(0, 0)` by a different route than the
                // one the connected case takes. Two paths to the same value is
                // how they eventually stop agreeing.
                let pick = |f: fn(&PadState) -> RawValue, zero: RawValue| {
                    pads.iter()
                        .map(|pad| f(pad))
                        .max_by(|a, b| a.magnitude().total_cmp(&b.magnitude()))
                        .unwrap_or(zero)
                };
                let zero_2d = RawValue::Analog2D(Vec2::ZERO);
                let zero_1d = RawValue::Analog1D(0.0);
                match path.control.as_str() {
                    "leftstick" => pick(|p| RawValue::Analog2D(p.left_stick), zero_2d),
                    "rightstick" => pick(|p| RawValue::Analog2D(p.right_stick), zero_2d),
                    "lefttrigger" => pick(|p| RawValue::Analog1D(p.left_trigger), zero_1d),
                    "righttrigger" => pick(|p| RawValue::Analog1D(p.right_trigger), zero_1d),
                    name => match PadButton::from_name(name) {
                        Some(button) => RawValue::Digital(pads.iter().any(|p| p.is_down(button))),
                        None => RawValue::Digital(false),
                    },
                }
            }
            DeviceKind::Unknown => RawValue::Digital(false),
        }
    }
}

/// `"w"` -> `KeyCode::KeyW`.
///
/// By name, never by discriminant: a settings file outlives the enum, and winit
/// adding a variant must not silently repoint every binding after it.
#[must_use]
pub fn key_code(name: &str) -> Option<KeyCode> {
    Some(match name {
        "a" => KeyCode::KeyA,
        "b" => KeyCode::KeyB,
        "c" => KeyCode::KeyC,
        "d" => KeyCode::KeyD,
        "e" => KeyCode::KeyE,
        "f" => KeyCode::KeyF,
        "g" => KeyCode::KeyG,
        "h" => KeyCode::KeyH,
        "i" => KeyCode::KeyI,
        "j" => KeyCode::KeyJ,
        "k" => KeyCode::KeyK,
        "l" => KeyCode::KeyL,
        "m" => KeyCode::KeyM,
        "n" => KeyCode::KeyN,
        "o" => KeyCode::KeyO,
        "p" => KeyCode::KeyP,
        "q" => KeyCode::KeyQ,
        "r" => KeyCode::KeyR,
        "s" => KeyCode::KeyS,
        "t" => KeyCode::KeyT,
        "u" => KeyCode::KeyU,
        "v" => KeyCode::KeyV,
        "w" => KeyCode::KeyW,
        "x" => KeyCode::KeyX,
        "y" => KeyCode::KeyY,
        "z" => KeyCode::KeyZ,
        "0" => KeyCode::Digit0,
        "1" => KeyCode::Digit1,
        "2" => KeyCode::Digit2,
        "3" => KeyCode::Digit3,
        "4" => KeyCode::Digit4,
        "5" => KeyCode::Digit5,
        "6" => KeyCode::Digit6,
        "7" => KeyCode::Digit7,
        "8" => KeyCode::Digit8,
        "9" => KeyCode::Digit9,
        "space" => KeyCode::Space,
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "shiftleft" => KeyCode::ShiftLeft,
        "shiftright" => KeyCode::ShiftRight,
        "controlleft" => KeyCode::ControlLeft,
        "controlright" => KeyCode::ControlRight,
        "altleft" => KeyCode::AltLeft,
        "altright" => KeyCode::AltRight,
        "up" | "arrowup" => KeyCode::ArrowUp,
        "down" | "arrowdown" => KeyCode::ArrowDown,
        "left" | "arrowleft" => KeyCode::ArrowLeft,
        "right" | "arrowright" => KeyCode::ArrowRight,
        "f1" => KeyCode::F1,
        "f2" => KeyCode::F2,
        "f3" => KeyCode::F3,
        "f4" => KeyCode::F4,
        "f5" => KeyCode::F5,
        "f6" => KeyCode::F6,
        "f7" => KeyCode::F7,
        "f8" => KeyCode::F8,
        "f9" => KeyCode::F9,
        "f10" => KeyCode::F10,
        "f11" => KeyCode::F11,
        "f12" => KeyCode::F12,
        _ => return None,
    })
}

/// The inverse of [`key_code`], for reporting a pressed control during rebind.
#[must_use]
pub fn key_name(code: KeyCode) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
        "space", "enter", "escape", "tab", "backspace", "delete", "shiftleft", "shiftright",
        "controlleft", "controlright", "altleft", "altright", "arrowup", "arrowdown", "arrowleft",
        "arrowright", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    ];
    NAMES.iter().copied().find(|name| key_code(name) == Some(code))
}

fn mouse_button(name: &str) -> Option<MouseButton> {
    Some(match name {
        "leftbutton" | "left" => MouseButton::Left,
        "rightbutton" | "right" => MouseButton::Right,
        "middlebutton" | "middle" => MouseButton::Middle,
        _ => return None,
    })
}

fn mouse_button_name(button: MouseButton) -> Option<&'static str> {
    Some(match button {
        MouseButton::Left => "leftbutton",
        MouseButton::Right => "rightbutton",
        MouseButton::Middle => "middlebutton",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        for name in ["w", "space", "escape", "f5", "shiftleft", "arrowup"] {
            let code = key_code(name).unwrap_or_else(|| panic!("{name} maps"));
            assert_eq!(key_name(code), Some(name), "{name} round-trips");
        }
    }

    #[test]
    fn an_unknown_key_name_is_none_rather_than_a_guess() {
        assert_eq!(key_code("hyperspace"), None);
    }

    /// **Focus loss releases everything.**
    ///
    /// A key released while unfocused never reports, so a player who alt-tabs
    /// mid-sprint comes back sprinting into a wall with no way to stop.
    #[test]
    fn losing_focus_releases_every_control() {
        let mut devices = Devices::new();
        devices.keys.insert(KeyCode::KeyW, true);
        devices.mouse_buttons.insert(MouseButton::Left, true);
        assert!(devices.handle_window_event(&WindowEvent::Focused(false)));
        assert_eq!(
            devices.read(&ControlPath::keyboard("w")),
            RawValue::Digital(false)
        );
        assert_eq!(
            devices.read(&ControlPath::mouse("leftbutton")),
            RawValue::Digital(false)
        );
    }

    /// Deltas are cleared per frame, or the camera drifts after the mouse stops.
    #[test]
    fn deltas_do_not_survive_the_frame() {
        let mut devices = Devices::new();
        devices.add_mouse_delta(Vec2::new(4.0, -2.0));
        assert_eq!(
            devices.read(&ControlPath::mouse("delta")),
            RawValue::Analog2D(Vec2::new(4.0, -2.0))
        );
        devices.end_frame();
        assert_eq!(
            devices.read(&ControlPath::mouse("delta")),
            RawValue::Analog2D(Vec2::ZERO)
        );
    }

    /// A disconnected pad reads as zero, not as stuck-down.
    ///
    /// A yanked cable must not leave a character walking forever.
    #[test]
    fn a_disconnected_pad_reads_as_zero() {
        let mut devices = Devices::new();
        let mut pad = PadState {
            connected: true,
            left_stick: Vec2::new(1.0, 0.0),
            ..Default::default()
        };
        pad.set(PadButton::South, true);
        devices.set_pad(0, pad);
        assert_eq!(
            devices.read(&ControlPath::gamepad("leftstick")),
            RawValue::Analog2D(Vec2::new(1.0, 0.0))
        );

        devices.disconnect_pad(0);
        assert_eq!(
            devices.read(&ControlPath::gamepad("leftstick")),
            RawValue::Analog2D(Vec2::ZERO)
        );
        assert_eq!(
            devices.read(&ControlPath::gamepad("buttonsouth")),
            RawValue::Digital(false)
        );
        assert!(devices.connected_pads().is_empty());
    }

    /// An unpaired binding reads whichever pad is pushed furthest, so a
    /// single-player game works on whatever pad gets picked up.
    #[test]
    fn an_unpaired_binding_takes_the_strongest_pad() {
        let mut devices = Devices::new();
        devices.set_pad(
            0,
            PadState {
                connected: true,
                left_stick: Vec2::new(0.2, 0.0),
                ..Default::default()
            },
        );
        devices.set_pad(
            1,
            PadState {
                connected: true,
                left_stick: Vec2::new(0.9, 0.0),
                ..Default::default()
            },
        );
        assert_eq!(
            devices.read(&ControlPath::gamepad("leftstick")),
            RawValue::Analog2D(Vec2::new(0.9, 0.0))
        );
    }

    /// A paired binding ignores the other pad, which is how player two works.
    #[test]
    fn a_paired_binding_ignores_other_pads() {
        let mut devices = Devices::new();
        devices.set_pad(
            0,
            PadState {
                connected: true,
                left_stick: Vec2::new(0.9, 0.0),
                ..Default::default()
            },
        );
        devices.set_pad(1, PadState { connected: true, ..Default::default() });
        assert_eq!(
            devices.read(&ControlPath::gamepad("leftstick").on_device(1)),
            RawValue::Analog2D(Vec2::ZERO)
        );
    }

    /// **An absent control returns its own shape of zero.**
    ///
    /// `Digital(false)` for a stick would hand a scalar to a `Vector2` action,
    /// which `convert` widens by a different route than the connected case
    /// takes. Two paths to the same value is how they stop agreeing.
    #[test]
    fn an_absent_control_returns_the_right_shape_of_zero() {
        let devices = Devices::new();
        assert_eq!(
            devices.read(&ControlPath::gamepad("leftstick")),
            RawValue::Analog2D(Vec2::ZERO)
        );
        assert_eq!(
            devices.read(&ControlPath::gamepad("lefttrigger")),
            RawValue::Analog1D(0.0)
        );
        assert_eq!(
            devices.read(&ControlPath::gamepad("buttonsouth")),
            RawValue::Digital(false)
        );
    }

    /// Pad button aliases mean a binding file can say `a` or `cross`.
    #[test]
    fn pad_button_aliases_resolve_to_one_button() {
        assert_eq!(PadButton::from_name("a"), Some(PadButton::South));
        assert_eq!(PadButton::from_name("cross"), Some(PadButton::South));
        assert_eq!(PadButton::from_name("buttonsouth"), Some(PadButton::South));
    }

    #[test]
    fn pressed_paths_reports_what_a_rebind_would_capture() {
        let mut devices = Devices::new();
        devices.keys.insert(KeyCode::KeyJ, true);
        devices.keys.insert(KeyCode::KeyK, false);
        let paths = devices.pressed_paths();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].to_string(), "<Keyboard>/j");
    }

    /// An unknown device reads as zero rather than panicking, so a settings
    /// file from a newer build does not crash this one.
    #[test]
    fn an_unknown_device_reads_as_zero() {
        let devices = Devices::new();
        let path = ControlPath::parse("<SteeringWheel>/pedal").unwrap();
        assert_eq!(devices.read(&path), RawValue::Digital(false));
    }
}
