//! Processors and interactions (MORROWIND-AE, Seam 5).
//!
//! A **processor** transforms a control's value: dead zone, invert, scale,
//! normalise. An **interaction** decides *when* a value counts as an action:
//! hold, tap, multi-tap.
//!
//! Both live between the device layer and the action, which is what lets a
//! player invert their Y axis without any code above knowing an axis was
//! inverted.

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// A value read from a control, before it becomes an action value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawValue {
    /// A button.
    Digital(bool),
    /// One axis.
    Analog1D(f32),
    /// Two axes.
    Analog2D(Vec2),
}

impl RawValue {
    /// Whether this reads as pressed.
    ///
    /// An analog control is pressed past a threshold, which is what makes a
    /// trigger usable as a button without the caller writing the comparison.
    #[must_use]
    pub fn is_pressed(self, threshold: f32) -> bool {
        match self {
            Self::Digital(down) => down,
            Self::Analog1D(v) => v.abs() >= threshold,
            Self::Analog2D(v) => v.length() >= threshold,
        }
    }

    /// The magnitude, for a threshold test or a dead zone.
    #[must_use]
    pub fn magnitude(self) -> f32 {
        match self {
            Self::Digital(down) => f32::from(down),
            Self::Analog1D(v) => v.abs(),
            Self::Analog2D(v) => v.length(),
        }
    }
}

/// A transform applied to a raw value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Processor {
    /// Zero below `lower`, full at `upper`, rescaled in between.
    ///
    /// **Radial for a 2D control, not per-axis.** A per-axis dead zone leaves a
    /// square hole at the centre of a stick, so pushing diagonally at low
    /// magnitude registers on one axis and not the other — which feels like the
    /// stick catching, and is the single most common analog-input bug.
    DeadZone {
        /// Below this magnitude the control reads as zero.
        lower: f32,
        /// At this magnitude the control reads as full.
        upper: f32,
    },
    /// Negate. A player's inverted Y axis is this and nothing else.
    Invert,
    /// Multiply. Sensitivity.
    Scale(f32),
    /// Clamp magnitude to 1.
    ///
    /// A composite of four keys pressed diagonally has magnitude √2, so a
    /// player holding W and D moves 41% faster than one holding W. Every
    /// engine has shipped that bug at least once.
    Normalize,
    /// Clamp to a range, after everything else.
    Clamp {
        /// Lower bound.
        min: f32,
        /// Upper bound.
        max: f32,
    },
}

impl Processor {
    /// A dead zone with the values a stick usually wants.
    #[must_use]
    pub fn stick_dead_zone() -> Self {
        Self::DeadZone {
            lower: 0.125,
            upper: 0.925,
        }
    }

    /// Apply to a value.
    pub fn apply(self, value: RawValue) -> RawValue {
        match self {
            Self::DeadZone { lower, upper } => Self::dead_zone(value, lower, upper),
            Self::Invert => match value {
                RawValue::Digital(down) => RawValue::Digital(!down),
                RawValue::Analog1D(v) => RawValue::Analog1D(-v),
                RawValue::Analog2D(v) => RawValue::Analog2D(-v),
            },
            Self::Scale(factor) => match value {
                RawValue::Digital(down) => RawValue::Digital(down),
                RawValue::Analog1D(v) => RawValue::Analog1D(v * factor),
                RawValue::Analog2D(v) => RawValue::Analog2D(v * factor),
            },
            Self::Normalize => match value {
                RawValue::Analog1D(v) => RawValue::Analog1D(v.clamp(-1.0, 1.0)),
                RawValue::Analog2D(v) => RawValue::Analog2D(
                    // `clamp_length_max` and not `normalize`: a half-pushed
                    // stick must stay half-pushed. Normalising unconditionally
                    // turns every analog control into a digital one, which is
                    // the mirror of the bug this processor exists to fix.
                    v.clamp_length_max(1.0),
                ),
                other => other,
            },
            Self::Clamp { min, max } => match value {
                RawValue::Analog1D(v) => RawValue::Analog1D(v.clamp(min, max)),
                RawValue::Analog2D(v) => {
                    RawValue::Analog2D(Vec2::new(v.x.clamp(min, max), v.y.clamp(min, max)))
                }
                other => other,
            },
        }
    }

    fn dead_zone(value: RawValue, lower: f32, upper: f32) -> RawValue {
        let upper = upper.max(lower + 1e-4);
        let rescale = |magnitude: f32| {
            if magnitude <= lower {
                0.0
            } else {
                ((magnitude - lower) / (upper - lower)).min(1.0)
            }
        };
        match value {
            RawValue::Digital(down) => RawValue::Digital(down),
            RawValue::Analog1D(v) => RawValue::Analog1D(rescale(v.abs()) * v.signum()),
            RawValue::Analog2D(v) => {
                let magnitude = v.length();
                if magnitude <= 1e-6 {
                    return RawValue::Analog2D(Vec2::ZERO);
                }
                // Radial: scale the vector, preserving its direction.
                RawValue::Analog2D(v / magnitude * rescale(magnitude))
            }
        }
    }
}

/// Apply a chain, in order.
#[must_use]
pub fn apply_all(processors: &[Processor], value: RawValue) -> RawValue {
    processors
        .iter()
        .fold(value, |value, processor| processor.apply(value))
}

/// When a binding's value counts as the action firing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Interaction {
    /// Fires while held past the press threshold. The default.
    Press,
    /// Fires once, after `seconds` held.
    Hold {
        /// How long the control must be held.
        seconds: f32,
    },
    /// Fires on release, if released within `seconds`.
    ///
    /// The complement of `Hold`: a control with both gets "tap to reload, hold
    /// to holster" for free, and neither has to know about the other.
    Tap {
        /// The longest a press can be and still count as a tap.
        seconds: f32,
    },
    /// Fires after `count` taps within `seconds` of each other.
    MultiTap {
        /// How many taps.
        count: u8,
        /// The window between them.
        seconds: f32,
    },
}

impl Default for Interaction {
    fn default() -> Self {
        Self::Press
    }
}

/// What an interaction decided this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// Not active.
    #[default]
    Idle,
    /// The control is down but the interaction has not fired yet.
    Started,
    /// Fired this frame.
    Performed,
    /// Ended without firing.
    Cancelled,
}

impl Phase {
    /// Whether the action fired this frame.
    #[must_use]
    pub fn performed(self) -> bool {
        matches!(self, Self::Performed)
    }
}

/// Per-binding interaction state, advanced once per frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct InteractionState {
    held_for: f32,
    /// Seconds since the last release, for multi-tap.
    since_release: f32,
    taps: u8,
    was_pressed: bool,
    fired: bool,
}

impl InteractionState {
    /// Advance by `dt` with the control's current pressed state.
    pub fn update(&mut self, interaction: Interaction, pressed: bool, dt: f32) -> Phase {
        let released = self.was_pressed && !pressed;
        let just_pressed = !self.was_pressed && pressed;
        self.was_pressed = pressed;

        if pressed {
            self.held_for += dt;
        } else {
            self.since_release += dt;
        }
        if just_pressed {
            self.held_for = 0.0;
            self.fired = false;
        }

        match interaction {
            Interaction::Press => {
                if pressed {
                    Phase::Performed
                } else {
                    Phase::Idle
                }
            }
            Interaction::Hold { seconds } => {
                if pressed && !self.fired && self.held_for >= seconds {
                    self.fired = true;
                    Phase::Performed
                } else if pressed {
                    Phase::Started
                } else if released && !self.fired {
                    Phase::Cancelled
                } else {
                    Phase::Idle
                }
            }
            Interaction::Tap { seconds } => {
                if released {
                    // `held_for` is the duration of the press that just ended.
                    if self.held_for <= seconds {
                        return Phase::Performed;
                    }
                    return Phase::Cancelled;
                }
                if pressed { Phase::Started } else { Phase::Idle }
            }
            Interaction::MultiTap { count, seconds } => {
                if just_pressed {
                    // A gap longer than the window starts the count over rather
                    // than accumulating taps from a minute ago.
                    if self.since_release > seconds {
                        self.taps = 0;
                    }
                    self.since_release = 0.0;
                }
                if released {
                    self.since_release = 0.0;
                    self.taps = self.taps.saturating_add(1);
                    if self.taps >= count {
                        self.taps = 0;
                        return Phase::Performed;
                    }
                    return Phase::Started;
                }
                if !pressed && self.taps > 0 && self.since_release > seconds {
                    self.taps = 0;
                    return Phase::Cancelled;
                }
                if pressed { Phase::Started } else { Phase::Idle }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(x: f32, y: f32) -> RawValue {
        RawValue::Analog2D(Vec2::new(x, y))
    }

    /// **A dead zone is radial, not per-axis.**
    ///
    /// A per-axis dead zone leaves a square hole at the centre of a stick:
    /// pushing diagonally at low magnitude registers on one axis and not the
    /// other, which feels like the stick catching. Every point at the same
    /// distance from centre must be treated the same.
    #[test]
    fn a_dead_zone_is_radial() {
        let zone = Processor::DeadZone {
            lower: 0.2,
            upper: 1.0,
        };
        // Just inside the zone, diagonally: both axes are below 0.2, but the
        // magnitude is 0.28 and it must register.
        let RawValue::Analog2D(out) = zone.apply(v2(0.2, 0.2)) else {
            panic!("2D in, 2D out");
        };
        assert!(
            out.length() > 0.0,
            "a diagonal push past the radius registers"
        );
        // Genuinely inside the radius.
        let RawValue::Analog2D(dead) = zone.apply(v2(0.1, 0.1)) else {
            panic!()
        };
        assert_eq!(dead, Vec2::ZERO);
    }

    #[test]
    fn a_dead_zone_preserves_direction() {
        let zone = Processor::stick_dead_zone();
        let RawValue::Analog2D(out) = zone.apply(v2(0.6, 0.8)) else {
            panic!()
        };
        let expected = Vec2::new(0.6, 0.8).normalize();
        assert!(out.normalize().abs_diff_eq(expected, 1e-4));
    }

    #[test]
    fn a_dead_zone_reaches_full_at_the_upper_bound() {
        let zone = Processor::DeadZone {
            lower: 0.1,
            upper: 0.9,
        };
        let RawValue::Analog1D(out) = zone.apply(RawValue::Analog1D(0.9)) else {
            panic!()
        };
        assert!(
            (out - 1.0).abs() < 1e-4,
            "a stick at 0.9 must reach full throw"
        );
    }

    #[test]
    fn a_dead_zone_keeps_the_sign_on_one_axis() {
        let zone = Processor::DeadZone {
            lower: 0.1,
            upper: 1.0,
        };
        let RawValue::Analog1D(out) = zone.apply(RawValue::Analog1D(-0.55)) else {
            panic!()
        };
        assert!(out < 0.0, "pulling back must not become pushing forward");
    }

    /// **A diagonal composite must not be 41% faster.**
    ///
    /// W and D together give magnitude √2. Every engine has shipped this bug.
    #[test]
    fn normalize_stops_diagonal_speed_boost() {
        let RawValue::Analog2D(out) = Processor::Normalize.apply(v2(1.0, 1.0)) else {
            panic!()
        };
        assert!((out.length() - 1.0).abs() < 1e-4);
    }

    /// And normalising must not turn an analog stick into a digital one.
    ///
    /// This is the mirror of the bug above, and unconditional `normalize()` is
    /// how it gets written.
    #[test]
    fn normalize_leaves_a_half_pushed_stick_half_pushed() {
        let RawValue::Analog2D(out) = Processor::Normalize.apply(v2(0.5, 0.0)) else {
            panic!()
        };
        assert!((out.length() - 0.5).abs() < 1e-4, "got {}", out.length());
    }

    #[test]
    fn invert_and_scale_do_what_they_say() {
        assert_eq!(
            Processor::Invert.apply(RawValue::Analog1D(0.5)),
            RawValue::Analog1D(-0.5)
        );
        assert_eq!(
            Processor::Scale(2.0).apply(RawValue::Analog1D(0.5)),
            RawValue::Analog1D(1.0)
        );
    }

    #[test]
    fn a_chain_applies_in_order() {
        // Scale then clamp is not clamp then scale.
        let scale_first = apply_all(
            &[
                Processor::Scale(4.0),
                Processor::Clamp {
                    min: -1.0,
                    max: 1.0,
                },
            ],
            RawValue::Analog1D(0.5),
        );
        let clamp_first = apply_all(
            &[
                Processor::Clamp {
                    min: -1.0,
                    max: 1.0,
                },
                Processor::Scale(4.0),
            ],
            RawValue::Analog1D(0.5),
        );
        assert_eq!(scale_first, RawValue::Analog1D(1.0));
        assert_eq!(clamp_first, RawValue::Analog1D(2.0));
    }

    #[test]
    fn an_analog_control_can_act_as_a_button() {
        assert!(RawValue::Analog1D(0.7).is_pressed(0.5));
        assert!(!RawValue::Analog1D(0.3).is_pressed(0.5));
        assert!(v2(0.4, 0.4).is_pressed(0.5), "magnitude 0.57 clears 0.5");
    }

    // -- interactions ---------------------------------------------------------

    #[test]
    fn press_fires_every_frame_it_is_held() {
        let mut state = InteractionState::default();
        assert_eq!(
            state.update(Interaction::Press, true, 0.016),
            Phase::Performed
        );
        assert_eq!(
            state.update(Interaction::Press, true, 0.016),
            Phase::Performed
        );
        assert_eq!(state.update(Interaction::Press, false, 0.016), Phase::Idle);
    }

    /// A hold fires **once**, not every frame past the threshold.
    ///
    /// Firing repeatedly is how "hold to open the menu" opens it forty times.
    #[test]
    fn hold_fires_exactly_once() {
        let hold = Interaction::Hold { seconds: 0.5 };
        let mut state = InteractionState::default();
        let mut fired = 0;
        for _ in 0..60 {
            if state.update(hold, true, 0.016).performed() {
                fired += 1;
            }
        }
        assert_eq!(fired, 1, "a hold is one event, not one per frame");
    }

    #[test]
    fn a_hold_released_early_is_cancelled() {
        let hold = Interaction::Hold { seconds: 0.5 };
        let mut state = InteractionState::default();
        for _ in 0..10 {
            assert_eq!(state.update(hold, true, 0.016), Phase::Started);
        }
        assert_eq!(state.update(hold, false, 0.016), Phase::Cancelled);
    }

    /// Tap fires on **release**, and only if it was quick.
    #[test]
    fn tap_fires_on_a_quick_release_and_not_a_slow_one() {
        let tap = Interaction::Tap { seconds: 0.2 };
        let mut quick = InteractionState::default();
        quick.update(tap, true, 0.016);
        assert_eq!(quick.update(tap, false, 0.016), Phase::Performed);

        let mut slow = InteractionState::default();
        for _ in 0..30 {
            slow.update(tap, true, 0.016);
        }
        assert_eq!(slow.update(tap, false, 0.016), Phase::Cancelled);
    }

    /// Tap and Hold on one control is "tap to reload, hold to holster", and
    /// neither interaction has to know the other exists.
    #[test]
    fn tap_and_hold_are_complementary() {
        let tap = Interaction::Tap { seconds: 0.2 };
        let hold = Interaction::Hold { seconds: 0.5 };
        let (mut t, mut h) = (InteractionState::default(), InteractionState::default());

        // A quick press: tap fires, hold does not.
        t.update(tap, true, 0.016);
        h.update(hold, true, 0.016);
        assert!(t.update(tap, false, 0.016).performed());
        assert!(!h.update(hold, false, 0.016).performed());

        // A long press: hold fires, tap does not.
        let (mut t, mut h) = (InteractionState::default(), InteractionState::default());
        let mut hold_fired = false;
        for _ in 0..40 {
            t.update(tap, true, 0.016);
            hold_fired |= h.update(hold, true, 0.016).performed();
        }
        assert!(hold_fired);
        assert!(!t.update(tap, false, 0.016).performed());
    }

    #[test]
    fn multi_tap_needs_its_count() {
        let double = Interaction::MultiTap {
            count: 2,
            seconds: 0.3,
        };
        let mut state = InteractionState::default();
        // First tap.
        state.update(double, true, 0.016);
        assert_eq!(state.update(double, false, 0.016), Phase::Started);
        // Second, promptly.
        state.update(double, true, 0.05);
        assert_eq!(state.update(double, false, 0.016), Phase::Performed);
    }

    /// **Taps a minute apart are not a double tap.**
    ///
    /// Without the window reset, a player who taps once now and once next
    /// level triggers a dodge-roll at the worst possible moment.
    #[test]
    fn multi_tap_forgets_a_stale_tap() {
        let double = Interaction::MultiTap {
            count: 2,
            seconds: 0.3,
        };
        let mut state = InteractionState::default();
        state.update(double, true, 0.016);
        state.update(double, false, 0.016);
        // A long gap.
        for _ in 0..60 {
            state.update(double, false, 0.016);
        }
        state.update(double, true, 0.016);
        assert_eq!(
            state.update(double, false, 0.016),
            Phase::Started,
            "the stale tap was forgotten, so this is a first tap"
        );
    }
}
