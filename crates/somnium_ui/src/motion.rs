//! Phase 27-C (Charon) — the animation driver.
//!
//! Before Charon there was no clock in `somnium_ui` at all. `theme::MotionTokens`
//! declared `press_ms`, `hover_ms`, `popup_ms` and `drawer_ms`, and the only
//! consumer in the crate was `TOOLTIP_DELAY_MS` — every other state change was an
//! instant pop.
//!
//! # The two rules this module exists to enforce
//!
//! 1. **Motion is causal.** A track starts because something happened, runs for
//!    at most [`MAX_DURATION_MS`], and ends. There is no looping, no idle
//!    breathing, and no decorative motion — `dev records/phase_27.md` §5.5 and
//!    §9.3 make that a contract, and [`Animator::start`] enforces the duration
//!    ceiling rather than trusting call sites.
//! 2. **Idle frames are free.** [`Animator::tick`] returns `false` when no track
//!    advanced, and a finished track is removed rather than left at `t = 1`. A
//!    shell with nothing animating must produce a byte-identical draw list two
//!    frames running (§10.3), so an animator holding completed tracks would be a
//!    bug even though it looks harmless.
//!
//! # Reduced motion
//!
//! [`Animator::set_reduced_motion`] completes every track instantly. Crucially it
//! changes *timing only* — the end state is identical, so layout never differs
//! between the two modes. That is asserted, because a reduced-motion path that
//! quietly skipped a state change would be worse than no reduced-motion path.

use std::collections::HashMap;

/// Hard ceiling on any single track, from §5.5. The longest token the design
/// ships is `drawer_ms` at 200.
pub const MAX_DURATION_MS: f32 = 200.0;

/// Which animatable property of a node a track drives.
///
/// Deliberately a small closed set: an open-ended string key would let call
/// sites invent motion that the design system never approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MotionProperty {
    /// Hover wash blend, 0 = rest fill, 1 = hover fill.
    HoverWash,
    /// Press darkening blend.
    PressWash,
    /// Popup / toast opacity.
    Opacity,
    /// Drawer or popup travel along its opening axis, in logical units.
    OffsetY,
    /// Popup scale-from-anchor, 0 = collapsed, 1 = full size.
    Scale,
}

/// Identifies one animating property on one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MotionKey {
    /// `NodeHandle` index. Stored as a plain index so `motion` does not need to
    /// depend on the pool's generational handle type.
    pub node: u32,
    /// Distinguishes repeated rows *inside* one node.
    ///
    /// An Outliner is a single widget that paints N rows in a loop, so a
    /// node-only key would make every row share one hover track and fade
    /// together. Widgets that own no repeats pass 0.
    pub sub: u32,
    pub property: MotionProperty,
}

impl MotionKey {
    /// Key for a widget that paints one thing.
    pub fn new(node: u32, property: MotionProperty) -> Self {
        Self {
            node,
            sub: 0,
            property,
        }
    }

    /// Key for row `sub` of a widget that paints many.
    pub fn row(node: u32, sub: u32, property: MotionProperty) -> Self {
        Self {
            node,
            sub,
            property,
        }
    }
}

/// Timing curve. Two families: cubic-bezier style easings for travel, and a
/// critically damped spring for press feedback, which must never overshoot on a
/// control the user is scrubbing (§9.3, "forbidden").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    /// Symmetric ease-in-out. The default for a state cross-fade.
    Standard,
    /// Fast out, slow in. For something entering the screen.
    Decelerate,
    /// Slow out, fast in. For something leaving.
    Accelerate,
    /// Critically damped: approaches the target without overshoot.
    Spring,
}

impl Easing {
    /// Map linear progress `t` in 0..=1 onto eased progress in 0..=1.
    ///
    /// Every curve satisfies `f(0) == 0` and `f(1) == 1`, which is what lets
    /// reduced motion jump straight to the end state and land on exactly the
    /// same value an animated track would.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            // Hermite smoothstep: the cheap, well-behaved ease-in-out.
            Easing::Standard => t * t * (3.0 - 2.0 * t),
            Easing::Decelerate => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::Accelerate => t * t,
            // Critically damped step response, normalised so f(1) == 1 exactly.
            // Without the normalisation the track would stop just short of its
            // target and leave a permanent sub-pixel offset.
            Easing::Spring => {
                const K: f32 = 6.0;
                let raw = 1.0 - (1.0 + K * t) * (-K * t).exp();
                let full = 1.0 - (1.0 + K) * (-K).exp();
                (raw / full).clamp(0.0, 1.0)
            }
        }
    }
}

/// One running animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    pub from: f32,
    pub to: f32,
    pub elapsed_ms: f32,
    pub duration_ms: f32,
    pub easing: Easing,
}

impl Track {
    /// Current value, eased.
    pub fn value(&self) -> f32 {
        if self.duration_ms <= 0.0 {
            return self.to;
        }
        let t = (self.elapsed_ms / self.duration_ms).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * self.easing.apply(t)
    }

    pub fn finished(&self) -> bool {
        self.elapsed_ms >= self.duration_ms
    }
}

/// Per-node animation state for one `UserInterface`.
#[derive(Debug, Default)]
pub struct Animator {
    tracks: HashMap<MotionKey, Track>,
    /// Values of tracks that have finished, kept so a widget still reads its
    /// settled state after the track is retired.
    settled: HashMap<MotionKey, f32>,
    reduced_motion: bool,
}

impl Animator {
    pub fn new() -> Self {
        Self::default()
    }

    /// When set, every track completes immediately. Timing only — the end state
    /// is unchanged, so layout is identical in both modes.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
        if on {
            // Settle everything in flight rather than leaving it half-way.
            for (key, track) in self.tracks.drain() {
                self.settled.insert(key, track.to);
            }
        }
    }

    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Number of tracks currently advancing. Zero means the next `tick` is a
    /// no-op and the frame can be considered idle.
    pub fn active_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_idle(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Begin (or retarget) a track.
    ///
    /// `rest` is the property's natural value, used as the origin only when this
    /// key has never been driven — a hover wash rests at 0, a scale at 1. It is
    /// an explicit parameter rather than a default because getting it wrong is
    /// silent: an origin equal to the target makes `start` a no-op and the
    /// control simply never animates.
    ///
    /// Retargeting keeps the *current* value as the origin, so reversing a hover
    /// mid-fade does not snap. `duration_ms` is clamped to [`MAX_DURATION_MS`]:
    /// the ceiling belongs here, not at ~86 call sites.
    pub fn start(&mut self, key: MotionKey, rest: f32, to: f32, duration_ms: f32, easing: Easing) {
        let from = self.value_or(key, rest);
        if (from - to).abs() < f32::EPSILON {
            self.tracks.remove(&key);
            self.settled.insert(key, to);
            return;
        }
        if self.reduced_motion || duration_ms <= 0.0 {
            self.tracks.remove(&key);
            self.settled.insert(key, to);
            return;
        }
        self.tracks.insert(
            key,
            Track {
                from,
                to,
                elapsed_ms: 0.0,
                duration_ms: duration_ms.min(MAX_DURATION_MS),
                easing,
            },
        );
    }

    /// Jump straight to a value with no animation, e.g. on scene load.
    pub fn set_immediate(&mut self, key: MotionKey, value: f32) {
        self.tracks.remove(&key);
        self.settled.insert(key, value);
    }

    /// Current value for `key`, or `default` when nothing has driven it.
    pub fn value_or(&self, key: MotionKey, default: f32) -> f32 {
        if let Some(track) = self.tracks.get(&key) {
            return track.value();
        }
        self.settled.get(&key).copied().unwrap_or(default)
    }

    /// Advance every track by `dt_ms`.
    ///
    /// Returns `true` when at least one track advanced, which is the signal the
    /// caller uses to decide whether the tree needs redrawing. A tick with no
    /// tracks must return `false` and touch nothing.
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        if self.tracks.is_empty() || dt_ms <= 0.0 {
            return false;
        }
        let mut finished: Vec<MotionKey> = Vec::new();
        for (key, track) in self.tracks.iter_mut() {
            track.elapsed_ms += dt_ms;
            if track.finished() {
                finished.push(*key);
            }
        }
        for key in finished {
            if let Some(track) = self.tracks.remove(&key) {
                self.settled.insert(key, track.to);
            }
        }
        true
    }

    /// Drop all state for a node that has been removed from the tree.
    pub fn forget_node(&mut self, node: u32) {
        self.tracks.retain(|k, _| k.node != node);
        self.settled.retain(|k, _| k.node != node);
    }

    /// Node indices with a track still running, for selective invalidation.
    pub fn animating_nodes(&self) -> Vec<u32> {
        let mut nodes: Vec<u32> = self.tracks.keys().map(|k| k.node).collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
    }
}

/// Interpolate two authored-sRGB colours.
///
/// Blending happens on the **linear** values and the result is re-encoded, so a
/// half-way hover wash is the perceptual midpoint rather than the byte midpoint.
/// Alpha is straight and interpolates directly.
pub fn lerp_color(a: crate::theme::Color, b: crate::theme::Color, t: f32) -> crate::theme::Color {
    let t = t.clamp(0.0, 1.0);
    let la = crate::color::srgb_u8_to_linear_rgba(a);
    let lb = crate::color::srgb_u8_to_linear_rgba(b);
    crate::color::linear_rgba_to_srgb_u8([
        la[0] + (lb[0] - la[0]) * t,
        la[1] + (lb[1] - la[1]) * t,
        la[2] + (lb[2] - la[2]) * t,
        la[3] + (lb[3] - la[3]) * t,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOVER: MotionKey = MotionKey {
        node: 7,
        sub: 0,
        property: MotionProperty::HoverWash,
    };

    #[test]
    fn every_easing_spans_exactly_zero_to_one() {
        // Reduced motion jumps to the end state and must land on the same value
        // a completed track would, which only holds if f(1) == 1 exactly.
        for easing in [
            Easing::Linear,
            Easing::Standard,
            Easing::Decelerate,
            Easing::Accelerate,
            Easing::Spring,
        ] {
            assert_eq!(easing.apply(0.0), 0.0, "{easing:?} f(0)");
            assert!(
                (easing.apply(1.0) - 1.0).abs() < 1e-6,
                "{easing:?} f(1) = {}",
                easing.apply(1.0)
            );
        }
    }

    #[test]
    fn easings_are_monotonic_and_stay_in_range() {
        for easing in [
            Easing::Linear,
            Easing::Standard,
            Easing::Decelerate,
            Easing::Accelerate,
            Easing::Spring,
        ] {
            let mut prev = -1.0;
            for i in 0..=100 {
                let v = easing.apply(i as f32 / 100.0);
                assert!((0.0..=1.0).contains(&v), "{easing:?} out of range: {v}");
                assert!(v >= prev - 1e-6, "{easing:?} went backwards at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn spring_never_overshoots() {
        // A control the user is scrubbing must not bounce past its target.
        for i in 0..=100 {
            assert!(Easing::Spring.apply(i as f32 / 100.0) <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn an_idle_animator_reports_no_work_and_does_none() {
        let mut a = Animator::new();
        assert!(a.is_idle());
        assert!(
            !a.tick(16.0),
            "an empty tick must not claim it changed anything"
        );
    }

    #[test]
    fn a_finished_track_is_retired_so_idle_frames_stay_idle() {
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 120.0, Easing::Standard);
        assert_eq!(a.active_count(), 1);
        assert!(a.tick(60.0));
        assert_eq!(a.active_count(), 1, "still mid-flight");
        assert!(a.tick(60.0));
        assert_eq!(a.active_count(), 0, "completed tracks must be removed");
        assert!(!a.tick(16.0), "and the next tick must be a no-op");
        assert_eq!(a.value_or(HOVER, 0.0), 1.0, "settled value survives");
    }

    #[test]
    fn reduced_motion_reaches_the_same_end_state_instantly() {
        let mut animated = Animator::new();
        let mut reduced = Animator::new();
        reduced.set_reduced_motion(true);

        animated.start(HOVER, 0.0, 1.0, 120.0, Easing::Standard);
        reduced.start(HOVER, 0.0, 1.0, 120.0, Easing::Standard);

        assert_eq!(reduced.value_or(HOVER, 0.0), 1.0, "reduced settles at once");
        assert!(reduced.is_idle(), "and schedules no work");

        while !animated.is_idle() {
            animated.tick(16.0);
        }
        assert_eq!(
            animated.value_or(HOVER, 0.0),
            reduced.value_or(HOVER, 0.0),
            "both modes must end in the same state"
        );
    }

    #[test]
    fn enabling_reduced_motion_settles_tracks_already_in_flight() {
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 200.0, Easing::Standard);
        a.tick(20.0);
        a.set_reduced_motion(true);
        assert!(a.is_idle());
        assert_eq!(a.value_or(HOVER, 0.0), 1.0);
    }

    #[test]
    fn retargeting_mid_flight_starts_from_the_current_value() {
        // Moving the pointer off a control half way through its hover fade must
        // fade back from where it is, not snap to full hover first.
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 100.0, Easing::Linear);
        a.tick(50.0);
        let mid = a.value_or(HOVER, 0.0);
        assert!((mid - 0.5).abs() < 1e-3, "expected ~0.5, got {mid}");

        a.start(HOVER, 0.0, 0.0, 100.0, Easing::Linear);
        let after = a.value_or(HOVER, 0.0);
        assert!(
            (after - mid).abs() < 1e-3,
            "retarget must not jump: {after}"
        );
    }

    #[test]
    fn duration_is_clamped_to_the_design_ceiling() {
        // §5.5: no animation exceeds 200 ms. The ceiling lives here so no call
        // site can opt out of it.
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 5_000.0, Easing::Standard);
        let mut elapsed = 0.0;
        while !a.is_idle() {
            a.tick(10.0);
            elapsed += 10.0;
            assert!(elapsed <= MAX_DURATION_MS + 10.0, "ran {elapsed} ms");
        }
    }

    #[test]
    fn starting_a_track_at_its_current_value_schedules_nothing() {
        let mut a = Animator::new();
        a.set_immediate(HOVER, 1.0);
        a.start(HOVER, 0.0, 1.0, 120.0, Easing::Standard);
        assert!(a.is_idle(), "a no-op transition must not cost a redraw");
    }

    #[test]
    fn the_rest_value_is_only_used_when_the_key_is_untouched() {
        // Guards the defect that made `start` silently no-op: taking the origin
        // from the *target* meant a first transition to 1.0 began at 1.0, so the
        // control never animated and nothing said so.
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 100.0, Easing::Linear);
        assert_eq!(a.active_count(), 1, "a first transition must actually run");
        assert!(a.value_or(HOVER, 0.0) < 0.01, "and must begin at rest");

        // Once driven, `rest` is ignored in favour of the live value.
        a.tick(50.0);
        a.start(HOVER, 0.0, 0.0, 100.0, Easing::Linear);
        assert!(
            a.value_or(HOVER, 0.0) > 0.4,
            "retarget must resume from the live value, not from rest"
        );
    }

    #[test]
    fn forgetting_a_node_drops_its_tracks_and_settled_values() {
        let mut a = Animator::new();
        a.start(HOVER, 0.0, 1.0, 120.0, Easing::Standard);
        a.set_immediate(MotionKey::new(7, MotionProperty::Opacity), 0.5);
        a.forget_node(7);
        assert!(a.is_idle());
        assert_eq!(a.value_or(HOVER, 0.25), 0.25, "falls back to the default");
    }

    #[test]
    fn rows_of_one_widget_animate_independently() {
        // The Outliner paints N rows from a single node. Without `sub` they
        // would share one track and fade together.
        let mut a = Animator::new();
        let r0 = MotionKey::row(4, 0, MotionProperty::HoverWash);
        let r1 = MotionKey::row(4, 1, MotionProperty::HoverWash);
        a.start(r0, 0.0, 1.0, 100.0, Easing::Linear);
        a.tick(50.0);
        a.start(r1, 0.0, 1.0, 100.0, Easing::Linear);
        assert!(
            a.value_or(r0, 0.0) > a.value_or(r1, 0.0),
            "rows share a track"
        );
        assert_eq!(a.active_count(), 2);

        // And forgetting the node clears every row it owns.
        a.forget_node(4);
        assert!(a.is_idle());
    }

    #[test]
    fn animating_nodes_are_reported_once_each_for_selective_invalidation() {
        let mut a = Animator::new();
        a.start(
            MotionKey::new(3, MotionProperty::HoverWash),
            0.0,
            1.0,
            100.0,
            Easing::Linear,
        );
        a.start(
            MotionKey::new(3, MotionProperty::Opacity),
            0.0,
            1.0,
            100.0,
            Easing::Linear,
        );
        a.start(
            MotionKey::new(9, MotionProperty::Scale),
            0.0,
            1.0,
            100.0,
            Easing::Linear,
        );
        assert_eq!(a.animating_nodes(), vec![3, 9]);
    }

    #[test]
    fn colour_lerp_blends_in_linear_space_not_byte_space() {
        let black = [0u8, 0, 0, 255];
        let white = [255u8, 255, 255, 255];
        let mid = lerp_color(black, white, 0.5);
        // The linear midpoint re-encodes to ~188, well above the byte midpoint
        // of 128. Blending in byte space is the classic dark-banding bug.
        assert!(mid[0] > 180, "expected a linear midpoint, got {}", mid[0]);
        assert_eq!(mid[3], 255);
    }

    #[test]
    fn colour_lerp_endpoints_are_exact() {
        let a = [0x1C, 0x1E, 0x26, 0xFF];
        let b = [0x31, 0x35, 0x43, 0xFF];
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }
}
