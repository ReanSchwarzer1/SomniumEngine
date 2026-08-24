//! Actions, maps, bindings and composites (MORROWIND-AE, Seam 5).
//!
//! Seam 5's shape, near-verbatim from §7:
//!
//! ```text
//! struct ActionMap { name: String, actions: Vec<Action> }
//! struct Action  { name: String, kind: ActionKind, bindings: Vec<Binding> }
//! struct Binding { path: ControlPath, processors: Vec<Processor>, interaction: Option<Interaction> }
//! ```
//!
//! > *"Game code, script and UI see actions. Rebinding is a runtime operation
//! > over the same data."*
//!
//! Both halves matter. An action is not a keycode wrapper: it has a **kind**,
//! and the kind is what lets `Move` be a 2D vector whether it came from WASD or
//! a stick, without the movement code branching on which.

use crate::{
    path::ControlPath,
    processor::{Interaction, InteractionState, Phase, Processor, RawValue, apply_all},
};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// What shape of value an action produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    /// Pressed or not. Jump, fire, confirm.
    Digital,
    /// One axis. A throttle, a zoom.
    Analog1D,
    /// Two axes. Move, look.
    Analog2D,
}

/// An action's value this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActionValue {
    /// Pressed or not.
    Digital(bool),
    /// One axis, normally in `-1..=1`.
    Analog1D(f32),
    /// Two axes, normally within the unit disc.
    Analog2D(Vec2),
}

impl ActionValue {
    /// The zero for a kind.
    #[must_use]
    pub fn zero(kind: ActionKind) -> Self {
        match kind {
            ActionKind::Digital => Self::Digital(false),
            ActionKind::Analog1D => Self::Analog1D(0.0),
            ActionKind::Analog2D => Self::Analog2D(Vec2::ZERO),
        }
    }

    /// Read as a boolean.
    #[must_use]
    pub fn as_bool(self) -> bool {
        match self {
            Self::Digital(v) => v,
            Self::Analog1D(v) => v.abs() > 0.5,
            Self::Analog2D(v) => v.length() > 0.5,
        }
    }

    /// Read as one axis.
    #[must_use]
    pub fn as_axis(self) -> f32 {
        match self {
            Self::Digital(v) => f32::from(v),
            Self::Analog1D(v) => v,
            Self::Analog2D(v) => v.x,
        }
    }

    /// Read as two axes.
    #[must_use]
    pub fn as_vec2(self) -> Vec2 {
        match self {
            Self::Digital(v) => Vec2::new(f32::from(v), 0.0),
            Self::Analog1D(v) => Vec2::new(v, 0.0),
            Self::Analog2D(v) => v,
        }
    }

    /// Whether this is non-zero.
    #[must_use]
    pub fn is_active(self) -> bool {
        match self {
            Self::Digital(v) => v,
            Self::Analog1D(v) => v.abs() > 1e-4,
            Self::Analog2D(v) => v.length_squared() > 1e-8,
        }
    }
}

/// One way an action can be triggered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Binding {
    /// A single control.
    Single {
        /// The control to read.
        path: ControlPath,
        /// Applied in order to the raw value.
        #[serde(default)]
        processors: Vec<Processor>,
        /// When the value counts as the action firing. `None` is `Press`.
        #[serde(default)]
        interaction: Option<Interaction>,
    },
    /// Two controls forming one axis: `negative` pulls down, `positive` up.
    Axis {
        /// Pulls the axis towards -1.
        negative: ControlPath,
        /// Pushes the axis towards +1.
        positive: ControlPath,
        /// Applied to the combined value, not to each control.
        #[serde(default)]
        processors: Vec<Processor>,
    },
    /// Four controls forming a 2D vector. WASD.
    ///
    /// **This is why composites exist.** Without one, movement code reads four
    /// booleans and assembles a vector itself — in every game, slightly
    /// differently, and usually without normalising the diagonal.
    Vector2 {
        /// Negative y. Screen space is y-down and this is movement.
        up: ControlPath,
        /// Positive y.
        down: ControlPath,
        /// Negative x.
        left: ControlPath,
        /// Positive x.
        right: ControlPath,
        /// Applied to the assembled vector, which is where `Normalize`
        /// belongs: per-control it would do nothing.
        #[serde(default)]
        processors: Vec<Processor>,
    },
}

impl Binding {
    /// A single control with no processing.
    #[must_use]
    pub fn single(path: ControlPath) -> Self {
        Self::Single {
            path,
            processors: Vec::new(),
            interaction: None,
        }
    }

    /// Add a processor.
    #[must_use]
    pub fn with(mut self, processor: Processor) -> Self {
        match &mut self {
            Self::Single { processors, .. }
            | Self::Axis { processors, .. }
            | Self::Vector2 { processors, .. } => processors.push(processor),
        }
        self
    }

    /// Set the interaction. Ignored on a composite, which has no single press
    /// to time.
    #[must_use]
    pub fn with_interaction(mut self, value: Interaction) -> Self {
        if let Self::Single { interaction, .. } = &mut self {
            *interaction = Some(value);
        }
        self
    }

    /// Every control path this binding reads.
    #[must_use]
    pub fn paths(&self) -> Vec<&ControlPath> {
        match self {
            Self::Single { path, .. } => vec![path],
            Self::Axis {
                negative, positive, ..
            } => vec![negative, positive],
            Self::Vector2 {
                up,
                down,
                left,
                right,
                ..
            } => vec![up, down, left, right],
        }
    }

    fn processors(&self) -> &[Processor] {
        match self {
            Self::Single { processors, .. }
            | Self::Axis { processors, .. }
            | Self::Vector2 { processors, .. } => processors,
        }
    }

    /// Read this binding's value from a control source.
    fn read(&self, source: &dyn ControlSource) -> RawValue {
        let raw = match self {
            Self::Single { path, .. } => source.read(path),
            Self::Axis {
                negative, positive, ..
            } => {
                let n = source.read(negative).magnitude();
                let p = source.read(positive).magnitude();
                RawValue::Analog1D(p - n)
            }
            Self::Vector2 {
                up,
                down,
                left,
                right,
                ..
            } => {
                // Screen space is y-down, and this is *movement*, not a cursor:
                // "up" must be negative y or W walks backwards. Getting this
                // wrong is invisible until somebody plays the game.
                let y = source.read(down).magnitude() - source.read(up).magnitude();
                let x = source.read(right).magnitude() - source.read(left).magnitude();
                RawValue::Analog2D(Vec2::new(x, y))
            }
        };
        apply_all(self.processors(), raw)
    }
}

/// Something that can read a control's current value.
///
/// A trait so the action layer can be tested without a window, and so the
/// device layer is substitutable — a replay system, a network client and a
/// test all implement it.
pub trait ControlSource {
    /// The current value of `path`, or its zero when nothing reports it.
    fn read(&self, path: &ControlPath) -> RawValue;
}

/// One named verb.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// The name game code, script and UI use. The only identity an action has.
    pub name: String,
    /// The shape of value it produces.
    pub kind: ActionKind,
    /// Every way it can be triggered. More than one is normal: a verb
    /// usually has a keyboard binding and a pad binding.
    pub bindings: Vec<Binding>,
}

impl Action {
    /// A digital action.
    #[must_use]
    pub fn digital(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ActionKind::Digital,
            bindings: Vec::new(),
        }
    }

    /// A 2D action.
    #[must_use]
    pub fn vector2(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ActionKind::Analog2D,
            bindings: Vec::new(),
        }
    }

    /// A 1D action.
    #[must_use]
    pub fn axis(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ActionKind::Analog1D,
            bindings: Vec::new(),
        }
    }

    /// Add a binding.
    #[must_use]
    pub fn bind(mut self, binding: Binding) -> Self {
        self.bindings.push(binding);
        self
    }
}

/// Per-action evaluation state, kept across frames for interactions.
#[derive(Clone, Debug, Default)]
struct ActionState {
    interactions: Vec<InteractionState>,
    value: Option<ActionValue>,
    phase: Phase,
    /// Whether the action was active last frame, for edge detection.
    was_active: bool,
}

/// A named set of actions, enabled or disabled as a unit.
///
/// Maps are the mechanism for context: a "gameplay" map and a "menu" map both
/// bind Escape, and only one is enabled at a time. Without them every action
/// handler needs to know whether a menu is open, which is how input handling
/// turns into a pile of flags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionMap {
    /// The map's name, used to enable and disable it.
    pub name: String,
    /// The actions it contains.
    pub actions: Vec<Action>,
    /// Whether this map contributes. Disabled maps are silent, which is
    /// how a menu and gameplay share a key.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl ActionMap {
    /// An empty, enabled map.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            actions: Vec::new(),
            enabled: true,
        }
    }

    /// Add an action.
    #[must_use]
    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// Find an action by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.name == name)
    }

    /// Find an action by name, mutably. Rebinding goes through this.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Action> {
        self.actions.iter_mut().find(|a| a.name == name)
    }
}

/// Evaluated action values for one frame.
#[derive(Clone, Debug, Default)]
pub struct ActionStates {
    states: std::collections::HashMap<String, ActionState>,
}

impl ActionStates {
    /// Evaluate every enabled map against `source`, advancing by `dt`.
    pub fn update(&mut self, maps: &[ActionMap], source: &dyn ControlSource, dt: f32) {
        for state in self.states.values_mut() {
            state.was_active = state.value.is_some_and(ActionValue::is_active);
            state.value = None;
            state.phase = Phase::Idle;
        }

        for map in maps.iter().filter(|m| m.enabled) {
            for action in &map.actions {
                let entry = self.states.entry(action.name.clone()).or_default();
                entry
                    .interactions
                    .resize(action.bindings.len(), InteractionState::default());

                let mut best = ActionValue::zero(action.kind);
                let mut best_magnitude = 0.0f32;
                let mut phase = Phase::Idle;

                for (index, binding) in action.bindings.iter().enumerate() {
                    let raw = binding.read(source);
                    let interaction = match binding {
                        Binding::Single { interaction, .. } => interaction.unwrap_or_default(),
                        _ => Interaction::Press,
                    };
                    let pressed = raw.is_pressed(PRESS_THRESHOLD);
                    let binding_phase = entry.interactions[index].update(interaction, pressed, dt);

                    // A non-`Press` interaction gates the value: a hold that has
                    // not fired yet contributes nothing, or "hold to sprint"
                    // would sprint from the first frame.
                    let gated =
                        matches!(interaction, Interaction::Press) || binding_phase.performed();
                    if !gated {
                        if binding_phase != Phase::Idle && phase == Phase::Idle {
                            phase = binding_phase;
                        }
                        continue;
                    }

                    let magnitude = raw.magnitude();
                    // **Strongest binding wins, not last.** A player holding W
                    // while nudging a stick should move at the stick's speed if
                    // it is pushed further, and last-wins makes the answer
                    // depend on binding order in a settings file.
                    if magnitude >= best_magnitude {
                        best_magnitude = magnitude;
                        best = convert(raw, action.kind);
                    }
                    if binding_phase.performed() {
                        phase = Phase::Performed;
                    } else if phase == Phase::Idle {
                        phase = binding_phase;
                    }
                }

                entry.value = Some(best);
                entry.phase = phase;
            }
        }
    }

    /// The action's value this frame, or its zero.
    #[must_use]
    pub fn value(&self, action: &str) -> Option<ActionValue> {
        self.states.get(action).and_then(|s| s.value)
    }

    /// Whether the action is active this frame.
    #[must_use]
    pub fn is_active(&self, action: &str) -> bool {
        self.value(action).is_some_and(ActionValue::is_active)
    }

    /// Whether the action became active this frame.
    ///
    /// The edge every "press to jump" wants, computed once here rather than by
    /// every caller keeping its own previous-frame boolean.
    #[must_use]
    pub fn just_activated(&self, action: &str) -> bool {
        self.states
            .get(action)
            .is_some_and(|s| !s.was_active && s.value.is_some_and(ActionValue::is_active))
    }

    /// Whether the action stopped being active this frame.
    #[must_use]
    pub fn just_deactivated(&self, action: &str) -> bool {
        self.states
            .get(action)
            .is_some_and(|s| s.was_active && !s.value.is_some_and(ActionValue::is_active))
    }

    /// The interaction phase this frame.
    #[must_use]
    pub fn phase(&self, action: &str) -> Phase {
        self.states.get(action).map(|s| s.phase).unwrap_or_default()
    }

    /// Convenience: the action as a 2D vector.
    #[must_use]
    pub fn vec2(&self, action: &str) -> Vec2 {
        self.value(action)
            .map(ActionValue::as_vec2)
            .unwrap_or(Vec2::ZERO)
    }

    /// Convenience: the action as one axis.
    #[must_use]
    pub fn axis(&self, action: &str) -> f32 {
        self.value(action).map(ActionValue::as_axis).unwrap_or(0.0)
    }
}

/// How far an analog control must move to read as pressed.
pub const PRESS_THRESHOLD: f32 = 0.5;

fn convert(raw: RawValue, kind: ActionKind) -> ActionValue {
    match kind {
        ActionKind::Digital => ActionValue::Digital(raw.is_pressed(PRESS_THRESHOLD)),
        ActionKind::Analog1D => ActionValue::Analog1D(match raw {
            RawValue::Digital(v) => f32::from(v),
            RawValue::Analog1D(v) => v,
            RawValue::Analog2D(v) => v.x,
        }),
        ActionKind::Analog2D => ActionValue::Analog2D(match raw {
            RawValue::Digital(v) => Vec2::new(f32::from(v), 0.0),
            RawValue::Analog1D(v) => Vec2::new(v, 0.0),
            RawValue::Analog2D(v) => v,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake(HashMap<String, RawValue>);

    impl Fake {
        fn down(&mut self, path: &str) -> &mut Self {
            self.0.insert(path.to_string(), RawValue::Digital(true));
            self
        }
        fn analog(&mut self, path: &str, value: Vec2) -> &mut Self {
            self.0.insert(path.to_string(), RawValue::Analog2D(value));
            self
        }
        fn axis(&mut self, path: &str, value: f32) -> &mut Self {
            self.0.insert(path.to_string(), RawValue::Analog1D(value));
            self
        }
    }

    impl ControlSource for Fake {
        fn read(&self, path: &ControlPath) -> RawValue {
            self.0
                .get(&path.to_string())
                .copied()
                .unwrap_or(RawValue::Digital(false))
        }
    }

    fn wasd() -> Binding {
        Binding::Vector2 {
            up: ControlPath::keyboard("w"),
            down: ControlPath::keyboard("s"),
            left: ControlPath::keyboard("a"),
            right: ControlPath::keyboard("d"),
            processors: vec![Processor::Normalize],
        }
    }

    fn move_map() -> ActionMap {
        ActionMap::new("gameplay").action(Action::vector2("Move").bind(wasd()).bind(
            Binding::single(ControlPath::gamepad("leftstick")).with(Processor::stick_dead_zone()),
        ))
    }

    /// **The point of the whole seam.** Movement code reads one vector and
    /// never learns whether it came from a keyboard or a stick.
    #[test]
    fn one_action_serves_a_keyboard_and_a_stick() {
        let maps = [move_map()];
        let mut states = ActionStates::default();

        let mut keys = Fake::default();
        keys.down("<Keyboard>/d");
        states.update(&maps, &keys, 0.016);
        assert!(states.vec2("Move").abs_diff_eq(Vec2::new(1.0, 0.0), 1e-4));

        let mut pad = Fake::default();
        pad.analog("<Gamepad>/leftstick", Vec2::new(0.0, -0.8));
        states.update(&maps, &pad, 0.016);
        let moved = states.vec2("Move");
        assert!(
            moved.y < -0.5,
            "the stick drives the same action: {moved:?}"
        );
    }

    /// W is negative y, or the player walks backwards.
    ///
    /// Screen space is y-down and this is movement, not a cursor. The mistake
    /// is invisible until somebody plays the game.
    #[test]
    fn w_moves_up_the_screen() {
        let maps = [move_map()];
        let mut states = ActionStates::default();
        let mut keys = Fake::default();
        keys.down("<Keyboard>/w");
        states.update(&maps, &keys, 0.016);
        assert!(states.vec2("Move").y < 0.0);
    }

    /// A diagonal composite is not 41% faster than a straight one.
    #[test]
    fn a_diagonal_is_not_faster() {
        let maps = [move_map()];
        let mut states = ActionStates::default();
        let mut keys = Fake::default();
        keys.down("<Keyboard>/w").down("<Keyboard>/d");
        states.update(&maps, &keys, 0.016);
        assert!((states.vec2("Move").length() - 1.0).abs() < 1e-3);
    }

    /// **Strongest binding wins, not last.**
    ///
    /// Last-wins makes the answer depend on binding order in a settings file,
    /// so a player who reorders their bindings changes how their character
    /// moves.
    #[test]
    fn the_strongest_binding_wins() {
        let maps = [move_map()];
        let mut states = ActionStates::default();
        let mut both = Fake::default();
        both.down("<Keyboard>/d"); // magnitude 1 after normalise
        both.analog("<Gamepad>/leftstick", Vec2::new(0.3, 0.0)); // weaker
        states.update(&maps, &both, 0.016);
        assert!(
            (states.vec2("Move").x - 1.0).abs() < 1e-3,
            "the keyboard is pushed further: {:?}",
            states.vec2("Move")
        );
    }

    /// A disabled map contributes nothing, which is how context works.
    #[test]
    fn a_disabled_map_is_silent() {
        let mut maps = [move_map()];
        maps[0].enabled = false;
        let mut states = ActionStates::default();
        let mut keys = Fake::default();
        keys.down("<Keyboard>/d");
        states.update(&maps, &keys, 0.016);
        assert!(!states.is_active("Move"));
    }

    /// Two maps binding the same key do not collide when one is disabled.
    #[test]
    fn maps_are_how_a_menu_and_gameplay_share_escape() {
        let gameplay = ActionMap::new("gameplay").action(
            Action::digital("Pause").bind(Binding::single(ControlPath::keyboard("escape"))),
        );
        let mut menu = ActionMap::new("menu")
            .action(Action::digital("Back").bind(Binding::single(ControlPath::keyboard("escape"))));
        menu.enabled = false;

        let mut states = ActionStates::default();
        let mut keys = Fake::default();
        keys.down("<Keyboard>/escape");
        states.update(&[gameplay.clone(), menu.clone()], &keys, 0.016);
        assert!(states.is_active("Pause"));
        assert!(!states.is_active("Back"));
    }

    /// The edge is computed once, here, not by every caller.
    #[test]
    fn just_activated_is_an_edge() {
        let maps = [ActionMap::new("m")
            .action(Action::digital("Jump").bind(Binding::single(ControlPath::keyboard("space"))))];
        let mut states = ActionStates::default();

        let mut up = Fake::default();
        let mut down = Fake::default();
        down.down("<Keyboard>/space");

        states.update(&maps, &up, 0.016);
        assert!(!states.just_activated("Jump"));

        states.update(&maps, &down, 0.016);
        assert!(states.just_activated("Jump"), "the frame it went down");

        states.update(&maps, &down, 0.016);
        assert!(!states.just_activated("Jump"), "still down is not an edge");

        states.update(&maps, &mut up, 0.016);
        assert!(states.just_deactivated("Jump"));
    }

    /// A hold that has not fired contributes nothing.
    ///
    /// Otherwise "hold to sprint" sprints from the first frame, which is the
    /// same as not having the interaction at all.
    #[test]
    fn a_hold_gates_the_value_until_it_fires() {
        let maps = [ActionMap::new("m").action(
            Action::digital("Sprint").bind(
                Binding::single(ControlPath::keyboard("shiftleft"))
                    .with_interaction(Interaction::Hold { seconds: 0.3 }),
            ),
        )];
        let mut states = ActionStates::default();
        let mut keys = Fake::default();
        keys.down("<Keyboard>/shiftleft");

        states.update(&maps, &keys, 0.1);
        assert!(!states.is_active("Sprint"), "not held long enough yet");

        states.update(&maps, &keys, 0.3);
        assert!(states.is_active("Sprint"));
        assert!(states.phase("Sprint").performed());
    }

    /// An axis composite subtracts one control from the other.
    #[test]
    fn an_axis_binding_is_two_controls() {
        let maps = [
            ActionMap::new("m").action(Action::axis("Throttle").bind(Binding::Axis {
                negative: ControlPath::keyboard("s"),
                positive: ControlPath::keyboard("w"),
                processors: vec![],
            })),
        ];
        let mut states = ActionStates::default();

        let mut forward = Fake::default();
        forward.down("<Keyboard>/w");
        states.update(&maps, &forward, 0.016);
        assert_eq!(states.axis("Throttle"), 1.0);

        let mut both = Fake::default();
        both.down("<Keyboard>/w").down("<Keyboard>/s");
        states.update(&maps, &both, 0.016);
        assert_eq!(states.axis("Throttle"), 0.0, "opposing inputs cancel");
    }

    /// An unbound action reads as its zero rather than panicking.
    #[test]
    fn an_unknown_action_is_zero() {
        let states = ActionStates::default();
        assert_eq!(states.vec2("Nothing"), Vec2::ZERO);
        assert_eq!(states.axis("Nothing"), 0.0);
        assert!(!states.is_active("Nothing"));
    }

    /// A map round-trips through JSON, because bindings are a file.
    #[test]
    fn a_map_round_trips_through_json() {
        let map = move_map();
        let json = serde_json::to_string(&map).expect("serialises");
        let back: ActionMap = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(map, back);
    }

    /// An analog trigger drives a digital action past the threshold.
    #[test]
    fn a_trigger_can_be_a_button() {
        let maps = [ActionMap::new("m").action(
            Action::digital("Fire").bind(Binding::single(ControlPath::gamepad("righttrigger"))),
        )];
        let mut states = ActionStates::default();
        let mut light = Fake::default();
        light.axis("<Gamepad>/righttrigger", 0.2);
        states.update(&maps, &light, 0.016);
        assert!(!states.is_active("Fire"));

        let mut hard = Fake::default();
        hard.axis("<Gamepad>/righttrigger", 0.9);
        states.update(&maps, &hard, 0.016);
        assert!(states.is_active("Fire"));
    }
}
