//! Wiring `somnium_input` to `somnium_ui`'s navigation (MORROWIND-AE / F).
//!
//! `somnium_ui::runtime::nav::NavActions` is a trait rather than a dependency on
//! `somnium_input`, so the UI crate stays testable without an input system. The
//! implementation belongs wherever the two meet, and that is here: `somnium_core`
//! already depends on both.
//!
//! Six lines of adapter, and the reason they are here rather than in either
//! crate is the whole argument for the trait.

use somnium_input::InputSystem;
use somnium_ui::runtime::nav::NavActions;

/// Reads a UI navigation verb from the player's bindings.
pub struct InputNavActions<'a>(pub &'a InputSystem);

impl NavActions for InputNavActions<'_> {
    fn just_activated(&self, action: &str) -> bool {
        self.0.just_activated(action)
    }

    fn vec2(&self, action: &str) -> glam::Vec2 {
        self.0.vec2(action)
    }
}

/// How far the navigate stick must move before focus steps.
///
/// Above `somnium_input`'s own dead zone on purpose: that one stops drift
/// reaching the *action*, and this one stops a deliberate-but-small push
/// stepping focus. A menu is discrete and a character's walk is not, so the two
/// want different thresholds from the same stick.
pub const UI_NAVIGATE_DEADZONE: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ui::runtime::nav::{NavAction, action_names};

    /// The bridge carries a rebound verb from the input system to the UI.
    #[test]
    fn a_bound_action_reaches_the_ui_as_a_nav_verb() {
        let mut input = InputSystem::with_default_maps();
        // Enter is bound to Confirm in the default UI map.
        assert!(input.devices_mut().set_key("enter", true));
        input.update(0.016);

        let actions = InputNavActions(&input);
        assert!(actions.just_activated(action_names::CONFIRM));
        assert_eq!(
            NavAction::from_actions(&actions, UI_NAVIGATE_DEADZONE),
            Some(NavAction::Confirm)
        );
    }

    /// **And a rebound one works too, which is what item 5 actually asked for.**
    #[test]
    fn rebinding_confirm_changes_which_key_confirms() {
        let mut input = InputSystem::with_default_maps();
        somnium_input::rebind::rebind(
            input.maps_mut(),
            "ui",
            "Confirm",
            somnium_input::ControlPath::keyboard("j"),
        )
        .expect("rebinds");

        input.devices_mut().set_key("enter", true);
        input.update(0.016);
        assert_eq!(
            NavAction::from_actions(&InputNavActions(&input), UI_NAVIGATE_DEADZONE),
            None,
            "the old key no longer confirms"
        );

        input.devices_mut().set_key("enter", false);
        input.devices_mut().set_key("j", true);
        input.update(0.016);
        assert_eq!(
            NavAction::from_actions(&InputNavActions(&input), UI_NAVIGATE_DEADZONE),
            Some(NavAction::Confirm),
            "the new one does"
        );
    }

    /// Arrow keys navigate, through the same path a d-pad takes.
    #[test]
    fn the_arrow_keys_navigate() {
        use somnium_ui::runtime::nav::Direction;
        let mut input = InputSystem::with_default_maps();
        input.devices_mut().set_key("arrowdown", true);
        input.update(0.016);
        assert_eq!(
            NavAction::from_actions(&InputNavActions(&input), UI_NAVIGATE_DEADZONE),
            Some(NavAction::Move(Direction::Down))
        );
    }
}
