//! Runtime rebinding with conflict detection (MORROWIND-AE, Seam 5).
//!
//! > *"Rebinding is a runtime operation over the same data."*
//!
//! The same `ActionMap` the game reads, mutated in place. There is no second
//! representation for "the bindings the player chose" — which is what stops the
//! two drifting, and is why a rebind takes effect on the next frame rather than
//! on the next restart.
//!
//! # Conflicts are reported, not prevented
//!
//! The tempting design refuses a binding that collides with an existing one.
//! It is wrong, and every game that ships it gets bug reports:
//!
//! - **A collision across maps is usually intentional.** Escape is Pause in
//!   gameplay and Cancel in the menu, and only one map is enabled at a time. A
//!   preventer either forbids that or needs to understand map enablement, which
//!   it cannot, because enablement is a runtime property.
//! - **A collision within a map is sometimes intentional too.** Sprint on
//!   `shiftleft` and Crouch on `shiftleft` is a mistake; Confirm on `enter`
//!   *and* `space` is not, and neither is a left-handed player deliberately
//!   stacking two verbs on one key they never use together.
//! - **Refusing leaves the player stuck.** They wanted the key, the game said
//!   no, and now they must remember which other action has it. Reporting lets
//!   the UI say "this will unbind Crouch — continue?", which is the interaction
//!   every shipped rebinding screen actually uses.
//!
//! So [`conflicts_for`] answers "what would this break", and the caller decides.

use crate::{
    action::{ActionMap, Binding},
    path::ControlPath,
};

/// An existing binding that would collide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The map holding the colliding action.
    pub map: String,
    /// The colliding action.
    pub action: String,
    /// The control both want.
    pub path: ControlPath,
    /// Whether the collision is inside the map being rebound.
    ///
    /// A cross-map collision is usually intentional — Escape is Pause and
    /// Cancel — so a UI should present the two differently rather than treating
    /// every conflict as an error.
    pub same_map: bool,
}

/// Every existing binding that uses `path`, excluding `action` in `map`.
///
/// Excluding the action being rebound matters: rebinding Jump from Space to
/// Space is not a conflict with itself, and reporting one makes the UI show a
/// warning for a no-op.
#[must_use]
pub fn conflicts_for(
    maps: &[ActionMap],
    map_name: &str,
    action_name: &str,
    path: &ControlPath,
) -> Vec<Conflict> {
    let mut out = Vec::new();
    for map in maps {
        for action in &map.actions {
            if map.name == map_name && action.name == action_name {
                continue;
            }
            let uses = action
                .bindings
                .iter()
                .flat_map(Binding::paths)
                .any(|bound| collides(bound, path));
            if uses {
                out.push(Conflict {
                    map: map.name.clone(),
                    action: action.name.clone(),
                    path: path.clone(),
                    same_map: map.name == map_name,
                });
            }
        }
    }
    out
}

/// Whether two paths refer to the same physical control.
///
/// An unpaired path collides with a paired one on the same control, because an
/// unpaired binding reads *every* device of that kind — so `<Gamepad>/a` and
/// `<Gamepad2>/a` genuinely do fight over pad two.
fn collides(a: &ControlPath, b: &ControlPath) -> bool {
    a.device == b.device
        && a.control == b.control
        && a.component == b.component
        && match (a.device_index, b.device_index) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

/// Replace an action's bindings with a single control.
///
/// Returns the conflicts the change created, *after* applying it — the caller
/// asked for the change and the report is what it now has to resolve, not a
/// veto. Use [`conflicts_for`] first when the UI wants to confirm before
/// committing.
pub fn rebind(
    maps: &mut [ActionMap],
    map_name: &str,
    action_name: &str,
    path: ControlPath,
) -> Result<Vec<Conflict>, RebindError> {
    let conflicts = conflicts_for(maps, map_name, action_name, &path);
    let map = maps
        .iter_mut()
        .find(|m| m.name == map_name)
        .ok_or_else(|| RebindError::NoSuchMap(map_name.to_string()))?;
    let action = map
        .find_mut(action_name)
        .ok_or_else(|| RebindError::NoSuchAction(action_name.to_string()))?;
    action.bindings = vec![Binding::single(path)];
    Ok(conflicts)
}

/// Remove every binding that uses `path` from `action`.
///
/// The other half of the "this will unbind Crouch — continue?" interaction.
pub fn unbind(maps: &mut [ActionMap], map_name: &str, action_name: &str, path: &ControlPath) {
    let Some(map) = maps.iter_mut().find(|m| m.name == map_name) else {
        return;
    };
    let Some(action) = map.find_mut(action_name) else {
        return;
    };
    action
        .bindings
        .retain(|binding| !binding.paths().iter().any(|bound| collides(bound, path)));
}

/// Why a rebind could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebindError {
    /// No map by that name.
    NoSuchMap(String),
    /// No action by that name in that map.
    NoSuchAction(String),
}

impl std::fmt::Display for RebindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchMap(name) => write!(f, "no action map named `{name}`"),
            Self::NoSuchAction(name) => write!(f, "no action named `{name}`"),
        }
    }
}

impl std::error::Error for RebindError {}

/// Captures the next control a player presses, for a "press a key" prompt.
#[derive(Debug, Default)]
pub struct RebindListener {
    /// Controls that must not be captured.
    ///
    /// Escape cancels the prompt rather than being bound to it, which is the
    /// convention every rebinding screen follows — and without it a player who
    /// changes their mind has bound Escape to Sprint and no way out.
    excluded: Vec<ControlPath>,
    captured: Option<ControlPath>,
    cancelled: bool,
}

impl RebindListener {
    /// A listener that excludes Escape.
    #[must_use]
    pub fn new() -> Self {
        Self {
            excluded: vec![ControlPath::keyboard("escape")],
            captured: None,
            cancelled: false,
        }
    }

    /// Also exclude `path`.
    #[must_use]
    pub fn excluding(mut self, path: ControlPath) -> Self {
        self.excluded.push(path);
        self
    }

    /// Feed the controls currently pressed.
    ///
    /// Captures the first non-excluded one. An excluded control cancels.
    pub fn observe(&mut self, pressed: &[ControlPath]) {
        if self.captured.is_some() || self.cancelled {
            return;
        }
        for path in pressed {
            if self.excluded.iter().any(|e| collides(e, path)) {
                self.cancelled = true;
                return;
            }
        }
        if let Some(path) = pressed.first() {
            self.captured = Some(path.clone());
        }
    }

    /// The captured control, if any.
    #[must_use]
    pub fn captured(&self) -> Option<&ControlPath> {
        self.captured.as_ref()
    }

    /// Whether the player cancelled.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancelled
    }

    /// Whether the listener is still waiting.
    #[must_use]
    pub fn is_listening(&self) -> bool {
        self.captured.is_none() && !self.cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_gameplay_map, default_ui_map};

    fn maps() -> Vec<ActionMap> {
        vec![default_gameplay_map(), default_ui_map()]
    }

    /// **A cross-map collision is reported and flagged as cross-map.**
    ///
    /// Escape is Pause in gameplay and Cancel in the menu. That is correct, and
    /// a rebinding UI must be able to tell it apart from a real clash.
    #[test]
    fn escape_collides_across_maps_and_says_so() {
        let maps = maps();
        let found = conflicts_for(
            &maps,
            "gameplay",
            "Interact",
            &ControlPath::keyboard("escape"),
        );
        assert!(!found.is_empty());
        assert!(
            found
                .iter()
                .any(|c| c.map == "ui" && c.action == "Cancel" && !c.same_map),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|c| c.map == "gameplay" && c.action == "Pause" && c.same_map)
        );
    }

    /// Rebinding an action to the key it already has is not a conflict.
    #[test]
    fn an_action_does_not_conflict_with_itself() {
        let maps = maps();
        let found = conflicts_for(&maps, "gameplay", "Jump", &ControlPath::keyboard("space"));
        assert!(
            found.iter().all(|c| c.action != "Jump"),
            "rebinding Jump to Space must not warn about Jump"
        );
    }

    /// A free key reports nothing.
    #[test]
    fn an_unused_control_has_no_conflicts() {
        let maps = maps();
        assert!(conflicts_for(&maps, "gameplay", "Jump", &ControlPath::keyboard("k")).is_empty());
    }

    /// **The change is applied and the report returned.**
    ///
    /// Not vetoed: the player asked for the key, and the report is what the UI
    /// offers to resolve.
    #[test]
    fn a_rebind_applies_and_reports() {
        let mut maps = maps();
        let conflicts = rebind(
            &mut maps,
            "gameplay",
            "Interact",
            ControlPath::keyboard("space"),
        )
        .expect("rebinds");
        assert!(conflicts.iter().any(|c| c.action == "Jump"));

        let interact = maps[0].find("Interact").expect("still there");
        assert_eq!(
            interact.bindings,
            vec![Binding::single(ControlPath::keyboard("space"))],
            "the player got the key they asked for"
        );
    }

    /// And the caller can then resolve the conflict.
    #[test]
    fn unbinding_resolves_the_conflict_the_report_named() {
        let mut maps = maps();
        rebind(
            &mut maps,
            "gameplay",
            "Interact",
            ControlPath::keyboard("space"),
        )
        .unwrap();
        unbind(
            &mut maps,
            "gameplay",
            "Jump",
            &ControlPath::keyboard("space"),
        );

        let jump = maps[0].find("Jump").expect("still there");
        assert!(
            !jump
                .bindings
                .iter()
                .flat_map(Binding::paths)
                .any(|p| *p == ControlPath::keyboard("space")),
            "Jump no longer wants Space"
        );
        assert!(!jump.bindings.is_empty(), "its gamepad binding survives");
    }

    #[test]
    fn rebinding_an_unknown_action_says_which() {
        let mut maps = maps();
        assert_eq!(
            rebind(&mut maps, "gameplay", "Fly", ControlPath::keyboard("f")),
            Err(RebindError::NoSuchAction("Fly".into()))
        );
        assert_eq!(
            rebind(&mut maps, "nope", "Jump", ControlPath::keyboard("f")),
            Err(RebindError::NoSuchMap("nope".into()))
        );
    }

    /// A composite's four controls are all checked, not just the first.
    #[test]
    fn a_composite_binding_conflicts_on_every_control_it_uses() {
        let maps = maps();
        for key in ["w", "a", "s", "d"] {
            let found = conflicts_for(&maps, "gameplay", "Jump", &ControlPath::keyboard(key));
            assert!(
                found.iter().any(|c| c.action == "Move"),
                "{key} is part of the Move composite"
            );
        }
    }

    /// An unpaired gamepad binding genuinely fights a paired one.
    #[test]
    fn an_unpaired_pad_binding_collides_with_a_paired_one() {
        let maps =
            vec![
                ActionMap::new("m").action(
                    crate::Action::digital("A")
                        .bind(Binding::single(ControlPath::gamepad("buttonsouth"))),
                ),
            ];
        let found = conflicts_for(
            &maps,
            "m",
            "B",
            &ControlPath::gamepad("buttonsouth").on_device(2),
        );
        assert_eq!(found.len(), 1, "an unpaired binding reads every pad");
    }

    /// Two different pads do not collide with each other.
    #[test]
    fn two_paired_pads_do_not_collide() {
        let maps =
            vec![
                ActionMap::new("m").action(crate::Action::digital("A").bind(Binding::single(
                    ControlPath::gamepad("buttonsouth").on_device(1),
                ))),
            ];
        let found = conflicts_for(
            &maps,
            "m",
            "B",
            &ControlPath::gamepad("buttonsouth").on_device(2),
        );
        assert!(found.is_empty(), "player one and player two share nothing");
    }

    /// **Escape cancels the prompt rather than being bound.**
    ///
    /// Without it, a player who changes their mind has bound Escape to Sprint
    /// and no way out of the menu.
    #[test]
    fn the_listener_lets_escape_cancel() {
        let mut listener = RebindListener::new();
        assert!(listener.is_listening());
        listener.observe(&[ControlPath::keyboard("escape")]);
        assert!(listener.cancelled());
        assert_eq!(listener.captured(), None);
    }

    #[test]
    fn the_listener_captures_the_first_control() {
        let mut listener = RebindListener::new();
        listener.observe(&[ControlPath::keyboard("j")]);
        assert_eq!(listener.captured(), Some(&ControlPath::keyboard("j")));
        assert!(!listener.is_listening());

        // A later press does not overwrite the capture.
        listener.observe(&[ControlPath::keyboard("k")]);
        assert_eq!(listener.captured(), Some(&ControlPath::keyboard("j")));
    }

    #[test]
    fn nothing_pressed_keeps_listening() {
        let mut listener = RebindListener::new();
        listener.observe(&[]);
        assert!(listener.is_listening());
    }
}
