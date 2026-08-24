//! Control paths (MORROWIND-AE, Seam 5).
//!
//! A `ControlPath` names a hardware control *without naming a keycode*:
//! `<Keyboard>/w`, `<Gamepad>/leftStick`, `<Mouse>/delta`. Everything above the
//! device layer speaks paths; only `device.rs` knows what a `KeyCode` is.
//!
//! # Why a string-shaped path and not an enum
//!
//! An enum of every control on every device is the obvious first design and it
//! is wrong for one reason that outweighs its type safety: **a binding is a
//! file a player edits.** A rebinding written to settings, a control scheme
//! shipped as data, a mod that adds a binding — all of them are text, and an
//! enum makes every one of them a serialisation problem with a migration every
//! time a device gains a control.
//!
//! The path is Unity's, because it is the reference §8 names and because its
//! shape has been through this: `<Device>/control` with an optional
//! `/subcontrol` for a stick's axes.
//!
//! Parsing is validated and the parsed form is what gets matched, so the
//! stringly-typed part is confined to the boundary rather than smeared through
//! the matcher.

use serde::{Deserialize, Serialize};

/// A device class a binding can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    /// Keys, by physical position rather than by label.
    Keyboard,
    /// Buttons, motion delta and scroll.
    Mouse,
    /// Sticks, triggers, buttons and d-pad.
    Gamepad,
    /// A device this build does not know. Kept rather than rejected so a
    /// settings file written by a newer build round-trips instead of losing the
    /// player's bindings for hardware this build cannot see.
    Unknown,
}

impl DeviceKind {
    /// Case-insensitive, for the same reason control names are: a settings
    /// file edited by hand says `<keyboard>` as readily as `<Keyboard>`, and
    /// rejecting one of them is a bad trade for a player editing a text file.
    fn parse(text: &str) -> Self {
        match text.to_ascii_lowercase().as_str() {
            "keyboard" => Self::Keyboard,
            "mouse" => Self::Mouse,
            "gamepad" => Self::Gamepad,
            _ => Self::Unknown,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Mouse => "Mouse",
            Self::Gamepad => "Gamepad",
            Self::Unknown => "Unknown",
        }
    }
}

/// Why a path string could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// No `<Device>` prefix.
    MissingDevice(String),
    /// A `<Device>` with no control after it.
    MissingControl(String),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDevice(s) => {
                write!(f, "`{s}` has no `<Device>` prefix; try `<Keyboard>/w`")
            }
            Self::MissingControl(s) => write!(f, "`{s}` names a device but no control"),
        }
    }
}

impl std::error::Error for PathError {}

/// A parsed reference to one hardware control.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ControlPath {
    /// Which class of device.
    pub device: DeviceKind,
    /// The control, lower-cased: `w`, `leftstick`, `delta`, `buttonsouth`.
    pub control: String,
    /// A component of the control, when it has them: `x`, `y`, `up`, `left`.
    pub component: Option<String>,
    /// The device index a multi-device setup pairs on.
    ///
    /// `None` means "any device of this kind", which is what a single-player
    /// binding wants. Player two's map sets it, and that is the whole of
    /// multi-device pairing at this layer.
    pub device_index: Option<u8>,
}

impl ControlPath {
    /// Parse `<Device>/control[/component]`.
    ///
    /// Case-insensitive on the control, because a settings file written by hand
    /// says `W` as often as `w` and rejecting one of them is a bad trade for a
    /// player editing a text file.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let trimmed = text.trim();
        let Some(rest) = trimmed.strip_prefix('<') else {
            return Err(PathError::MissingDevice(trimmed.to_string()));
        };
        let Some((device_text, rest)) = rest.split_once('>') else {
            return Err(PathError::MissingDevice(trimmed.to_string()));
        };

        // `<Gamepad2>/buttonSouth` pairs to the second pad.
        let (device_name, device_index) = split_index(device_text);
        let device = DeviceKind::parse(device_name);

        let rest = rest.strip_prefix('/').unwrap_or(rest);
        if rest.is_empty() {
            return Err(PathError::MissingControl(trimmed.to_string()));
        }
        let mut parts = rest.split('/');
        let control = parts.next().unwrap_or_default().to_ascii_lowercase();
        let component = parts.next().map(str::to_ascii_lowercase).filter(|s| !s.is_empty());

        Ok(Self {
            device,
            control,
            component,
            device_index,
        })
    }

    /// A keyboard key by name.
    #[must_use]
    pub fn keyboard(control: &str) -> Self {
        Self {
            device: DeviceKind::Keyboard,
            control: control.to_ascii_lowercase(),
            component: None,
            device_index: None,
        }
    }

    /// A gamepad control by name.
    #[must_use]
    pub fn gamepad(control: &str) -> Self {
        Self {
            device: DeviceKind::Gamepad,
            control: control.to_ascii_lowercase(),
            component: None,
            device_index: None,
        }
    }

    /// A mouse control by name.
    #[must_use]
    pub fn mouse(control: &str) -> Self {
        Self {
            device: DeviceKind::Mouse,
            control: control.to_ascii_lowercase(),
            component: None,
            device_index: None,
        }
    }

    /// Pair this path to a specific device index.
    #[must_use]
    pub fn on_device(mut self, index: u8) -> Self {
        self.device_index = Some(index);
        self
    }

    /// Whether this path accepts input from `index`.
    #[must_use]
    pub fn accepts_device(&self, index: u8) -> bool {
        self.device_index.is_none_or(|paired| paired == index)
    }
}

impl std::fmt::Display for ControlPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}", self.device.as_str())?;
        if let Some(index) = self.device_index {
            write!(f, "{index}")?;
        }
        write!(f, ">/{}", self.control)?;
        if let Some(component) = &self.component {
            write!(f, "/{component}")?;
        }
        Ok(())
    }
}

/// Split a trailing integer off a device name: `Gamepad2` -> `("Gamepad", 2)`.
fn split_index(text: &str) -> (&str, Option<u8>) {
    let digits = text.len() - text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return (text, None);
    }
    let (name, index) = text.split_at(text.len() - digits);
    (name, index.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyboard_path_round_trips() {
        let path = ControlPath::parse("<Keyboard>/w").unwrap();
        assert_eq!(path.device, DeviceKind::Keyboard);
        assert_eq!(path.control, "w");
        assert_eq!(path.component, None);
        assert_eq!(path.to_string(), "<Keyboard>/w");
    }

    #[test]
    fn a_component_is_parsed_and_printed() {
        let path = ControlPath::parse("<Gamepad>/leftStick/x").unwrap();
        assert_eq!(path.control, "leftstick");
        assert_eq!(path.component.as_deref(), Some("x"));
        assert_eq!(path.to_string(), "<Gamepad>/leftstick/x");
    }

    /// A settings file written by hand says `W` as often as `w`, and
    /// `<keyboard>` as readily as `<Keyboard>`.
    ///
    /// The first version of this test papered over the device half with an
    /// `unwrap_or_else` fallback, and the fallback was hiding that `<keyboard>`
    /// parsed as `Unknown` — which would have made a lower-cased settings file
    /// silently unbind every key in it.
    #[test]
    fn paths_are_case_insensitive_in_both_halves() {
        assert_eq!(
            ControlPath::parse("<Keyboard>/W").unwrap(),
            ControlPath::parse("<keyboard>/w").unwrap()
        );
        assert_eq!(
            ControlPath::parse("<GAMEPAD>/ButtonSouth").unwrap(),
            ControlPath::gamepad("buttonsouth")
        );
    }

    /// A device index is how player two's map differs from player one's.
    #[test]
    fn a_device_index_pairs_a_binding_to_one_pad() {
        let any = ControlPath::parse("<Gamepad>/buttonSouth").unwrap();
        let second = ControlPath::parse("<Gamepad2>/buttonSouth").unwrap();
        assert_eq!(any.device_index, None);
        assert_eq!(second.device_index, Some(2));
        assert_eq!(second.device, DeviceKind::Gamepad);
        assert_eq!(second.to_string(), "<Gamepad2>/buttonsouth");

        assert!(any.accepts_device(0), "an unpaired binding takes any pad");
        assert!(any.accepts_device(7));
        assert!(second.accepts_device(2));
        assert!(!second.accepts_device(1));
    }

    /// **An unknown device is kept, not rejected.**
    ///
    /// A settings file written by a newer build must round-trip through this
    /// one, or upgrading and downgrading silently loses a player's bindings for
    /// hardware this build cannot see.
    #[test]
    fn an_unknown_device_survives_a_round_trip() {
        let path = ControlPath::parse("<SteeringWheel>/pedal").unwrap();
        assert_eq!(path.device, DeviceKind::Unknown);
        assert_eq!(path.control, "pedal");
    }

    #[test]
    fn a_malformed_path_says_what_is_wrong() {
        assert!(matches!(
            ControlPath::parse("w").unwrap_err(),
            PathError::MissingDevice(_)
        ));
        assert!(matches!(
            ControlPath::parse("<Keyboard>").unwrap_err(),
            PathError::MissingControl(_)
        ));
        assert!(
            ControlPath::parse("w")
                .unwrap_err()
                .to_string()
                .contains("<Keyboard>/w"),
            "the error suggests the shape it wanted"
        );
    }

    #[test]
    fn whitespace_is_tolerated() {
        assert_eq!(
            ControlPath::parse("  <Keyboard>/space  ").unwrap(),
            ControlPath::keyboard("space")
        );
    }
}
