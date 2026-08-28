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
    ///
    /// **Normalised over a duration**, which is why MORROWIND-H adds
    /// [`Spring`] beside it rather than replacing it: this is a *shape*, and a
    /// spring is a *system*. The editor's press feedback wants the shape.
    Spring,
    /// An authored curve, from CONTROL-K's curve editor.
    ///
    /// MORROWIND-H. Holds an index rather than a [`Curve`](somnium_ecs::curve::Curve)
    /// so `Easing` stays `Copy` — every widget in the editor passes it by
    /// value, and boxing a curve into it would have been a change to all of
    /// them. The curve lives in the [`Animator`]'s library; resolve through
    /// [`Animator::ease`].
    Curve(CurveId),
}

/// A curve registered with an [`Animator`], for [`Easing::Curve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CurveId(pub u32);

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
            // An authored curve cannot be evaluated without the library that
            // holds it. Linear is the honest answer here rather than a panic:
            // this path is reachable only by a caller that took an `Easing` out
            // of an `Animator` and evaluated it away from one, and a UI that
            // eases linearly is wrong in a way somebody can see, where a UI
            // that panics on a hover is not shippable at all.
            Easing::Curve(_) => t,
        }
    }

    /// Whether this easing needs an [`Animator`] to evaluate.
    pub fn is_authored(self) -> bool {
        matches!(self, Easing::Curve(_))
    }
}

// ── MORROWIND-H: springs ────────────────────────────────────────────────────

/// A spring, parameterised the way a spring is.
///
/// [`Easing::Spring`] is a *shape*: a critically damped step response
/// normalised so it lands exactly on the target after a stated duration. That
/// is right for press feedback, where the duration is the design token and the
/// overshoot must be zero.
///
/// It is wrong for anything that gets **interrupted**. Retarget a duration
/// tween mid-flight and it restarts from the current value with zero velocity —
/// a drawer that was flying open and is told to close visibly stops dead first.
/// A spring carries velocity across the retarget, which is the entire reason
/// §8 says *"a spring model for the cases where duration is the wrong
/// parameterisation"*.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// How hard it pulls toward the target. Higher is faster and tighter.
    pub stiffness: f32,
    /// How hard it resists motion. At `2 * sqrt(stiffness * mass)` the spring
    /// is critically damped and will not overshoot.
    pub damping: f32,
    /// Larger mass, slower response, more overshoot for the same damping.
    pub mass: f32,
}

impl Spring {
    /// Critically damped at `stiffness` — the fastest approach with no
    /// overshoot, and the default for anything the user is scrubbing.
    pub fn critical(stiffness: f32) -> Self {
        let mass = 1.0;
        Self {
            stiffness,
            damping: 2.0 * (stiffness * mass).sqrt(),
            mass,
        }
    }

    /// Quick and tight, and **critically damped**: it will not overshoot.
    ///
    /// Menus, drawers, anything that should feel immediate. The damping is
    /// `2 * sqrt(320)` rounded up rather than a round number, because rounding
    /// it *down* is how a preset documented as tight quietly starts to wobble
    /// — which is what the first draft of this constant did, and what
    /// `overshoots()` exists to catch.
    pub const SNAPPY: Self = Self {
        stiffness: 320.0,
        damping: 36.0,
        mass: 1.0,
    };

    /// Slower, still critically damped. For something large moving.
    pub const GENTLE: Self = Self {
        stiffness: 120.0,
        damping: 22.0,
        mass: 1.0,
    };

    /// Deliberately underdamped: it overshoots and settles back.
    ///
    /// For a notification arriving or a badge popping — never for a control the
    /// user is scrubbing, which Phase 27 §9.3 forbids. Named rather than
    /// hand-tuned at the call site so `overshoots()` reads `true` on purpose
    /// somewhere a reviewer can see it.
    pub const WOBBLY: Self = Self {
        stiffness: 300.0,
        damping: 14.0,
        mass: 1.0,
    };

    /// Whether this spring will overshoot its target.
    ///
    /// Phase 27 §9.3 forbids overshoot on a control the user is scrubbing, so
    /// this is a question call sites need to be able to ask.
    pub fn overshoots(&self) -> bool {
        self.damping < 2.0 * (self.stiffness * self.mass).sqrt() - f32::EPSILON
    }

    /// One semi-implicit Euler step. Returns `(value, velocity)`.
    ///
    /// Sub-stepped by the caller: a 100 ms frame integrated in one go with a
    /// stiff spring diverges, and a UI that explodes after a breakpoint is a UI
    /// nobody debugs twice.
    fn step(&self, value: f32, velocity: f32, target: f32, dt_s: f32) -> (f32, f32) {
        let force = -self.stiffness * (value - target) - self.damping * velocity;
        let velocity = velocity + (force / self.mass.max(f32::EPSILON)) * dt_s;
        (value + velocity * dt_s, velocity)
    }
}

/// How a track travels from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    /// A fixed duration and a shape. Phase 27's model, and still the default:
    /// a design system that states `hover_ms` wants a track that takes exactly
    /// that long.
    Timed { duration_ms: f32, easing: Easing },
    /// A spring. No duration — it arrives when it arrives, and it carries its
    /// velocity through a retarget.
    Spring(Spring),
}

impl Motion {
    /// The Phase 27 shape, which every existing call site means.
    pub fn timed(duration_ms: f32, easing: Easing) -> Self {
        Motion::Timed {
            duration_ms,
            easing,
        }
    }
}

/// One running animation.
///
/// MORROWIND-H replaced `duration_ms` + `easing` with [`Motion`] and added
/// `velocity`, `delay_ms` and `current`. A spring has no duration and its state
/// is not a function of elapsed time, so a value that used to be *derived* on
/// every read is now *integrated* on every tick and cached here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    pub from: f32,
    pub to: f32,
    pub elapsed_ms: f32,
    /// Time still to wait before this track starts moving. MORROWIND-H's
    /// staggering: a list of eight rows entering together is a pop, and the
    /// same eight at 30 ms apart is a cascade.
    pub delay_ms: f32,
    pub motion: Motion,
    /// Units per second. Zero for a timed track; carried across a retarget for
    /// a spring, which is the whole reason springs are here.
    pub velocity: f32,
    /// The value as of the last tick.
    pub current: f32,
}

impl Track {
    /// Current value. Written by [`Animator::tick`]; a track that has never
    /// ticked reads as its origin.
    pub fn value(&self) -> f32 {
        self.current
    }

    pub fn finished(&self) -> bool {
        if self.delay_ms > 0.0 {
            return false;
        }
        match self.motion {
            Motion::Timed { duration_ms, .. } => self.elapsed_ms >= duration_ms,
            // A spring is done when it has arrived *and* stopped. Either test
            // alone is wrong: a spring passing through its target at speed has
            // arrived and is not done, and one creeping in from far away has
            // stopped and is not done either.
            Motion::Spring(_) => {
                (self.current - self.to).abs() < SPRING_EPSILON
                    && self.velocity.abs() < SPRING_VELOCITY_EPSILON
                    || self.elapsed_ms >= MAX_SPRING_MS
            }
        }
    }
}

/// Distance from the target below which a spring counts as arrived.
///
/// A quarter of a pixel at 1x. Tighter than this and a spring can hang for
/// hundreds of milliseconds converging on a difference nobody can see, which is
/// a frame cost with no visual return.
const SPRING_EPSILON: f32 = 0.25 / 1000.0;

/// Speed below which a spring counts as stopped, in units per second.
const SPRING_VELOCITY_EPSILON: f32 = 0.01;

/// Hard ceiling on a spring, as [`MAX_DURATION_MS`] is on a timed track.
///
/// A spring has no duration by construction, so this is not a design token —
/// it is the bound that stops a mis-parameterised spring animating forever and
/// keeping the shell awake. Four seconds is far past anything a UI should do
/// and far short of "nobody noticed".
pub const MAX_SPRING_MS: f32 = 4_000.0;

/// The largest step a spring is integrated with, in seconds.
///
/// Semi-implicit Euler with a stiff spring diverges at large `dt`. A 100 ms
/// frame after a breakpoint would otherwise send a widget to infinity, and a UI
/// that explodes after a breakpoint is a UI nobody debugs twice.
const SPRING_MAX_STEP_S: f32 = 1.0 / 240.0;

/// One property's part of a [`Transition`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionStep {
    pub property: MotionProperty,
    /// The property's natural value, for a key that has never been driven.
    pub rest: f32,
    pub to: f32,
    pub motion: Motion,
    /// Offset within the transition, for a step that should trail the others.
    pub delay_ms: f32,
}

/// A named state change: several properties moving together.
///
/// MORROWIND-H, and §8's *"state transitions"*. A card lifting is a scale, a
/// shadow and a wash; writing that as three `start` calls is three chances to
/// give one of them a different duration by accident, and the drift is the kind
/// nobody sees in review and everybody sees on screen.
///
/// Built once — as a `const`-like value next to the design tokens — and played
/// against a node, so *what a state looks like* lives in one place and *when it
/// happens* lives at the call site.
///
/// ```
/// # use somnium_ui::motion::{Easing, Motion, MotionProperty, Transition};
/// let lifted = Transition::new()
///     .with(MotionProperty::HoverWash, 0.0, 1.0, Motion::timed(120.0, Easing::Standard))
///     .with(MotionProperty::Scale, 1.0, 1.02, Motion::timed(120.0, Easing::Decelerate));
/// assert_eq!(lifted.len(), 2);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
    steps: Vec<TransitionStep>,
}

impl Transition {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a property to the transition.
    #[must_use]
    pub fn with(mut self, property: MotionProperty, rest: f32, to: f32, motion: Motion) -> Self {
        self.steps.push(TransitionStep {
            property,
            rest,
            to,
            motion,
            delay_ms: 0.0,
        });
        self
    }

    /// Add a property that trails the rest of the transition.
    #[must_use]
    pub fn with_delayed(
        mut self,
        property: MotionProperty,
        rest: f32,
        to: f32,
        motion: Motion,
        delay_ms: f32,
    ) -> Self {
        self.steps.push(TransitionStep {
            property,
            rest,
            to,
            motion,
            delay_ms,
        });
        self
    }

    pub fn steps(&self) -> &[TransitionStep] {
        &self.steps
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The same transition with every target replaced by its rest value.
    ///
    /// The exit half of an enter/exit pair, without a second declaration to
    /// keep in step with the first — which is the way these drift.
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            steps: self
                .steps
                .iter()
                .map(|step| TransitionStep {
                    to: step.rest,
                    ..*step
                })
                .collect(),
        }
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
    /// MORROWIND-H. Authored easings, indexed by [`CurveId`]. Registering is
    /// append-only within an animator's life so an id handed to a widget stays
    /// valid — a curve edited in CONTROL-K's editor is *replaced in place*
    /// through [`Animator::replace_curve`], not re-registered.
    curves: Vec<somnium_ecs::curve::Curve>,
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
        self.start_with(key, rest, to, Motion::timed(duration_ms, easing));
    }

    /// Begin (or retarget) a track with any [`Motion`].
    ///
    /// MORROWIND-H's general form; [`Animator::start`] is this with a
    /// `Motion::Timed`. **Retargeting a spring carries its velocity**, which is
    /// the difference that makes springs worth having: a drawer flying open and
    /// then told to close reverses through its own momentum instead of stopping
    /// dead and starting again.
    pub fn start_with(&mut self, key: MotionKey, rest: f32, to: f32, motion: Motion) {
        self.start_delayed(key, rest, to, motion, 0.0);
    }

    /// [`Animator::start_with`], beginning after `delay_ms`.
    pub fn start_delayed(
        &mut self,
        key: MotionKey,
        rest: f32,
        to: f32,
        motion: Motion,
        delay_ms: f32,
    ) {
        let existing = self.tracks.get(&key).copied();
        let from = self.value_or(key, rest);

        let instant = match motion {
            Motion::Timed { duration_ms, .. } => duration_ms <= 0.0,
            Motion::Spring(spring) => spring.stiffness <= 0.0,
        };
        if self.reduced_motion || instant || (from - to).abs() < f32::EPSILON {
            self.tracks.remove(&key);
            self.settled.insert(key, to);
            return;
        }

        // The one line springs exist for. A timed retarget starts from rest by
        // construction — its shape is a function of elapsed time and nothing
        // else — so velocity is carried only when both sides are springs.
        let velocity = match (existing.map(|t| t.motion), motion) {
            (Some(Motion::Spring(_)), Motion::Spring(_)) => existing.map_or(0.0, |t| t.velocity),
            _ => 0.0,
        };

        let motion = match motion {
            Motion::Timed {
                duration_ms,
                easing,
            } => Motion::Timed {
                duration_ms: duration_ms.min(MAX_DURATION_MS),
                easing,
            },
            spring => spring,
        };

        self.tracks.insert(
            key,
            Track {
                from,
                to,
                elapsed_ms: 0.0,
                delay_ms: delay_ms.max(0.0),
                motion,
                velocity,
                current: from,
            },
        );
    }

    /// Start the same motion on several keys, `stagger_ms` apart.
    ///
    /// MORROWIND-H. Eight rows entering together is a pop; the same eight at
    /// 30 ms apart is a cascade, and the difference is one parameter. Order is
    /// the caller's, because it is the visual order the cascade should follow
    /// and no sort in here could know it.
    ///
    /// Under reduced motion every key settles immediately, stagger included:
    /// staggering *is* timing, and the contract is that timing is the only
    /// thing reduced motion changes.
    pub fn start_staggered(
        &mut self,
        keys: impl IntoIterator<Item = MotionKey>,
        rest: f32,
        to: f32,
        motion: Motion,
        stagger_ms: f32,
    ) {
        for (index, key) in keys.into_iter().enumerate() {
            self.start_delayed(key, rest, to, motion, stagger_ms * index as f32);
        }
    }

    /// Play a [`Transition`] on one node.
    ///
    /// MORROWIND-H. A state change is rarely one property: a card lifting is a
    /// scale, a shadow and a wash, and three `start` calls in a row is three
    /// chances to give one of them a different duration by accident.
    pub fn play(&mut self, node: u32, transition: &Transition) {
        for step in &transition.steps {
            self.start_delayed(
                MotionKey::new(node, step.property),
                step.rest,
                step.to,
                step.motion,
                step.delay_ms,
            );
        }
    }

    /// Play a [`Transition`] across several nodes, `stagger_ms` apart.
    pub fn play_staggered(&mut self, nodes: &[u32], transition: &Transition, stagger_ms: f32) {
        for (index, node) in nodes.iter().enumerate() {
            let offset = stagger_ms * index as f32;
            for step in &transition.steps {
                self.start_delayed(
                    MotionKey::new(*node, step.property),
                    step.rest,
                    step.to,
                    step.motion,
                    step.delay_ms + offset,
                );
            }
        }
    }

    // ── Authored easing, from CONTROL-K ──────────────────────────────────────

    /// Register a curve from CONTROL-K's editor and get an [`Easing::Curve`].
    ///
    /// MORROWIND-H, and §8's *"easing curves that come from CONTROL-K's curve
    /// editor"*. The curve is evaluated at `t` in 0..=1 and its output is used
    /// directly as eased progress, **not normalised**: a curve that does not
    /// pass through (0,0) and (1,1) will not land exactly on its target, and
    /// that is deliberate. Normalising would make an authored ease-out-back
    /// impossible, and overshoot is precisely what an author reaches for a
    /// curve to get.
    pub fn register_curve(&mut self, curve: somnium_ecs::curve::Curve) -> CurveId {
        let id = CurveId(self.curves.len() as u32);
        self.curves.push(curve);
        id
    }

    /// Replace a registered curve in place, keeping every [`CurveId`] valid.
    ///
    /// What a live curve editor needs: dragging a tangent must change the
    /// motion of the widgets already referencing it, without re-registering and
    /// leaving the old curve behind to be animated by nothing.
    pub fn replace_curve(&mut self, id: CurveId, curve: somnium_ecs::curve::Curve) -> bool {
        match self.curves.get_mut(id.0 as usize) {
            Some(slot) => {
                *slot = curve;
                true
            }
            None => false,
        }
    }

    /// The registered curve behind an id.
    pub fn curve(&self, id: CurveId) -> Option<&somnium_ecs::curve::Curve> {
        self.curves.get(id.0 as usize)
    }

    /// Evaluate an easing, resolving [`Easing::Curve`] through the library.
    ///
    /// A `Curve` id with nothing behind it eases linearly rather than
    /// panicking: an animator rebuilt without its curves is a recoverable
    /// state, and a hover that panics is not.
    pub fn ease(&self, easing: Easing, t: f32) -> f32 {
        match easing {
            Easing::Curve(id) => match self.curve(id) {
                Some(curve) => curve.evaluate(t.clamp(0.0, 1.0)),
                None => t.clamp(0.0, 1.0),
            },
            other => other.apply(t),
        }
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
        // The curve library is borrowed immutably below while the tracks are
        // borrowed mutably, so it is split out rather than reached through
        // `self`. A second `HashMap` pass to avoid it would cost more than the
        // line.
        let curves = &self.curves;
        for (key, track) in self.tracks.iter_mut() {
            // A delayed track burns its delay first and does not move. It also
            // does not count as advancing for the redraw signal below — but the
            // signal is per-tick rather than per-track, and a frame in which
            // *something* is waiting to start is a frame that will need another
            // one anyway.
            if track.delay_ms > 0.0 {
                track.delay_ms -= dt_ms;
                if track.delay_ms > 0.0 {
                    continue;
                }
                // Spend the remainder of the frame on the track itself, so a
                // 30 ms stagger under a 16 ms frame does not quantise to 32.
                let spent = dt_ms + track.delay_ms;
                track.delay_ms = 0.0;
                advance(track, dt_ms - spent, curves);
            } else {
                advance(track, dt_ms, curves);
            }
            if track.finished() {
                finished.push(*key);
            }
        }
        for key in finished {
            if let Some(track) = self.tracks.remove(&key) {
                // A timed track lands exactly on its target; a spring lands
                // wherever it stopped, which is within SPRING_EPSILON of the
                // target and is snapped here so the settled value is exact.
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

/// Advance one track by `dt_ms`.
///
/// MORROWIND-H. A free function rather than a method on [`Track`] because a
/// timed track needs the curve library to resolve [`Easing::Curve`] and `Track`
/// has no business owning one.
fn advance(track: &mut Track, dt_ms: f32, curves: &[somnium_ecs::curve::Curve]) {
    if dt_ms <= 0.0 {
        return;
    }
    track.elapsed_ms += dt_ms;
    match track.motion {
        Motion::Timed {
            duration_ms,
            easing,
        } => {
            let t = if duration_ms <= 0.0 {
                1.0
            } else {
                (track.elapsed_ms / duration_ms).clamp(0.0, 1.0)
            };
            let eased = match easing {
                Easing::Curve(id) => match curves.get(id.0 as usize) {
                    Some(curve) => curve.evaluate(t),
                    None => t,
                },
                other => other.apply(t),
            };
            let previous = track.current;
            track.current = track.from + (track.to - track.from) * eased;
            // Reported so an interrupted timed track can hand a spring a
            // sensible starting velocity if one is ever retargeted into one.
            track.velocity = (track.current - previous) / (dt_ms / 1000.0);
        }
        Motion::Spring(spring) => {
            // Sub-stepped: see SPRING_MAX_STEP_S. The loop is bounded by the
            // spring ceiling rather than by the frame, so a pathological
            // dt cannot turn into a pathological number of iterations.
            let mut remaining = (dt_ms / 1000.0).min(MAX_SPRING_MS / 1000.0);
            while remaining > 0.0 {
                let step = remaining.min(SPRING_MAX_STEP_S);
                let (value, velocity) = spring.step(track.current, track.velocity, track.to, step);
                track.current = value;
                track.velocity = velocity;
                remaining -= step;
            }
        }
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

// ── MORROWIND-H ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod morrowind_h_tests {
    use super::*;
    use somnium_ecs::curve::{Curve, CurveKey, Interpolation};

    fn key() -> MotionKey {
        MotionKey::new(1, MotionProperty::Scale)
    }

    /// Run a track to completion and report how long it took.
    fn settle(animator: &mut Animator, key: MotionKey, step_ms: f32) -> f32 {
        let mut elapsed = 0.0;
        while !animator.is_idle() && elapsed < MAX_SPRING_MS * 2.0 {
            animator.tick(step_ms);
            elapsed += step_ms;
        }
        elapsed
    }

    // ── springs ─────────────────────────────────────────────────────────────

    #[test]
    fn a_spring_arrives_and_settles_exactly() {
        let mut a = Animator::new();
        a.start_with(key(), 0.0, 1.0, Motion::Spring(Spring::SNAPPY));
        let took = settle(&mut a, key(), 1000.0 / 120.0);
        assert!(took > 0.0 && took < MAX_SPRING_MS, "took {took} ms");
        // Settled values are snapped: a spring stops within SPRING_EPSILON and
        // a widget resting a fraction off its target is a permanent sub-pixel
        // offset, which is the exact bug Easing::Spring's normalisation fixes.
        assert_eq!(a.value_or(key(), -1.0), 1.0);
    }

    #[test]
    fn a_critically_damped_spring_does_not_overshoot() {
        let mut a = Animator::new();
        let spring = Spring::critical(200.0);
        assert!(!spring.overshoots());
        a.start_with(key(), 0.0, 1.0, Motion::Spring(spring));
        let mut peak: f32 = 0.0;
        for _ in 0..2000 {
            a.tick(1000.0 / 240.0);
            peak = peak.max(a.value_or(key(), 0.0));
            if a.is_idle() {
                break;
            }
        }
        // Phase 27 §9.3 forbids overshoot on a control being scrubbed. The
        // tolerance is float noise, not slack: 1e-3 of a unit range.
        assert!(peak <= 1.0 + 1e-3, "overshot to {peak}");
    }

    #[test]
    fn an_underdamped_spring_says_so_before_it_is_used() {
        assert!(Spring::WOBBLY.overshoots(), "WOBBLY is meant to overshoot");
        assert!(!Spring::SNAPPY.overshoots());
        assert!(!Spring::GENTLE.overshoots());
        // The bug this predicate exists to catch: SNAPPY shipped its first
        // draft at damping 32 against a critical damping of 2*sqrt(320) =
        // 35.78, so a preset documented as tight would have wobbled.
        assert!(
            Spring {
                damping: 32.0,
                ..Spring::SNAPPY
            }
            .overshoots()
        );
    }

    /// The reason springs are in this sub-phase at all.
    #[test]
    fn retargeting_a_spring_carries_velocity_and_a_tween_does_not() {
        let mut spring_side = Animator::new();
        spring_side.start_with(key(), 0.0, 1.0, Motion::Spring(Spring::GENTLE));
        for _ in 0..6 {
            spring_side.tick(1000.0 / 120.0);
        }
        let moving = spring_side.tracks.get(&key()).copied().expect("in flight");
        assert!(moving.velocity > 0.0, "the spring is not moving yet");

        // Reverse it mid-flight.
        spring_side.start_with(key(), 0.0, 0.0, Motion::Spring(Spring::GENTLE));
        let reversed = spring_side.tracks.get(&key()).copied().expect("retargeted");
        assert!(
            reversed.velocity > 0.0,
            "a spring must reverse through its own momentum, not stop dead"
        );

        // The timed side, for contrast: a shape is a function of elapsed time,
        // so a retarget starts from rest by construction.
        let mut timed_side = Animator::new();
        timed_side.start(key(), 0.0, 1.0, 120.0, Easing::Standard);
        for _ in 0..3 {
            timed_side.tick(1000.0 / 120.0);
        }
        timed_side.start(key(), 0.0, 0.0, 120.0, Easing::Standard);
        let retimed = timed_side.tracks.get(&key()).copied().expect("retargeted");
        assert_eq!(retimed.velocity, 0.0);
        // ...but it does keep the current value, so it does not snap.
        assert!(retimed.from > 0.0 && retimed.from < 1.0, "{}", retimed.from);
    }

    #[test]
    fn a_stalled_frame_does_not_send_a_spring_to_infinity() {
        let mut a = Animator::new();
        a.start_with(
            key(),
            0.0,
            1.0,
            Motion::Spring(Spring {
                stiffness: 4000.0,
                damping: 20.0,
                mass: 1.0,
            }),
        );
        // 500 ms in one step: a breakpoint, or a minimised window. Without
        // sub-stepping this diverges.
        a.tick(500.0);
        let value = a.value_or(key(), 0.0);
        assert!(value.is_finite(), "diverged to {value}");
        assert!(value.abs() < 10.0, "flung to {value}");
    }

    #[test]
    fn a_spring_cannot_animate_forever() {
        let mut a = Animator::new();
        // Stiffness so low it would creep for minutes.
        a.start_with(
            key(),
            0.0,
            1.0,
            Motion::Spring(Spring {
                stiffness: 0.01,
                damping: 8.0,
                mass: 1.0,
            }),
        );
        let took = settle(&mut a, key(), 1000.0 / 60.0);
        assert!(a.is_idle(), "still running after {took} ms");
        assert!(took <= MAX_SPRING_MS + 100.0, "took {took} ms");
    }

    // ── authored curves, from CONTROL-K ─────────────────────────────────────

    #[test]
    fn an_authored_curve_drives_the_track() {
        let mut a = Animator::new();
        // A curve that holds at 0 until half way, then jumps to 1: nothing any
        // built-in easing does, which is the point of authoring one.
        let id = a.register_curve(Curve::from_keys(vec![
            CurveKey {
                interpolation: Interpolation::Step,
                ..CurveKey::new(0.0, 0.0)
            },
            CurveKey {
                interpolation: Interpolation::Step,
                ..CurveKey::new(0.5, 1.0)
            },
        ]));
        a.start(key(), 0.0, 100.0, 100.0, Easing::Curve(id));
        a.tick(25.0);
        assert_eq!(a.value_or(key(), -1.0), 0.0, "should still be held at 0");
        a.tick(35.0); // now past t = 0.5
        assert_eq!(a.value_or(key(), -1.0), 100.0);
    }

    #[test]
    fn editing_a_curve_moves_the_widgets_already_using_it() {
        let mut a = Animator::new();
        let id = a.register_curve(Curve::ramp(0.0, 1.0));
        assert_eq!(a.ease(Easing::Curve(id), 0.5), 0.5);
        // The curve editor drags a key. Same id, different motion.
        assert!(a.replace_curve(id, Curve::constant(1.0)));
        assert_eq!(a.ease(Easing::Curve(id), 0.5), 1.0);
        assert!(!a.replace_curve(CurveId(99), Curve::constant(0.0)));
    }

    #[test]
    fn an_unregistered_curve_eases_linearly_rather_than_panicking() {
        let a = Animator::new();
        assert_eq!(a.ease(Easing::Curve(CurveId(7)), 0.25), 0.25);
        // And the library-free path agrees, so the two cannot disagree.
        assert_eq!(Easing::Curve(CurveId(7)).apply(0.25), 0.25);
        assert!(Easing::Curve(CurveId(0)).is_authored());
        assert!(!Easing::Standard.is_authored());
    }

    #[test]
    fn an_authored_curve_may_overshoot_on_purpose() {
        let mut a = Animator::new();
        // Ease-out-back: past the target, then home. Normalising this away
        // would defeat the reason an author drew it.
        let id = a.register_curve(Curve::from_keys(vec![
            CurveKey::new(0.0, 0.0),
            CurveKey::new(0.7, 1.2),
            CurveKey::new(1.0, 1.0),
        ]));
        assert!(a.ease(Easing::Curve(id), 0.7) > 1.0);
        assert!((a.ease(Easing::Curve(id), 1.0) - 1.0).abs() < 1e-5);
    }

    // ── staggering ──────────────────────────────────────────────────────────

    #[test]
    fn a_stagger_starts_rows_in_order_and_not_together() {
        let mut a = Animator::new();
        let keys: Vec<MotionKey> = (0..4)
            .map(|n| MotionKey::new(n, MotionProperty::Scale))
            .collect();
        a.start_staggered(
            keys.clone(),
            0.0,
            1.0,
            Motion::timed(100.0, Easing::Linear),
            30.0,
        );
        a.tick(16.0);
        // Row 0 has started; rows 1..3 are still waiting out their delay.
        assert!(
            a.value_or(keys[0], 0.0) > 0.0,
            "the first row did not start"
        );
        for k in &keys[1..] {
            assert_eq!(a.value_or(*k, 0.0), 0.0, "a delayed row moved early");
        }
        // Everything arrives eventually, and in order.
        settle(&mut a, keys[0], 16.0);
        for k in &keys {
            assert_eq!(a.value_or(*k, -1.0), 1.0);
        }
    }

    #[test]
    fn a_stagger_does_not_quantise_to_the_frame() {
        let mut a = Animator::new();
        let k = MotionKey::new(5, MotionProperty::Scale);
        // 20 ms delay under a 16 ms frame: the second tick crosses the delay
        // 4 ms in and must spend the remaining 12 ms on the track, not 16 and
        // not 0.
        a.start_delayed(k, 0.0, 1.0, Motion::timed(120.0, Easing::Linear), 20.0);
        a.tick(16.0);
        assert_eq!(a.value_or(k, -1.0), 0.0);
        a.tick(16.0);
        let value = a.value_or(k, -1.0);
        assert!(
            (value - 12.0 / 120.0).abs() < 1e-3,
            "expected 12 ms of a 120 ms track, got {value}"
        );
    }

    #[test]
    fn reduced_motion_ignores_the_stagger_too() {
        let mut a = Animator::new();
        a.set_reduced_motion(true);
        let keys: Vec<MotionKey> = (0..4)
            .map(|n| MotionKey::new(n, MotionProperty::Scale))
            .collect();
        a.start_staggered(
            keys.clone(),
            0.0,
            1.0,
            Motion::timed(100.0, Easing::Standard),
            30.0,
        );
        assert!(a.is_idle(), "reduced motion left a stagger in flight");
        for k in &keys {
            assert_eq!(a.value_or(*k, -1.0), 1.0);
        }
    }

    #[test]
    fn reduced_motion_settles_a_spring_too() {
        let mut a = Animator::new();
        a.set_reduced_motion(true);
        a.start_with(key(), 0.0, 1.0, Motion::Spring(Spring::GENTLE));
        assert!(a.is_idle());
        assert_eq!(a.value_or(key(), -1.0), 1.0);
    }

    // ── transitions ─────────────────────────────────────────────────────────

    fn lifted() -> Transition {
        Transition::new()
            .with(
                MotionProperty::HoverWash,
                0.0,
                1.0,
                Motion::timed(120.0, Easing::Standard),
            )
            .with(
                MotionProperty::Scale,
                1.0,
                1.02,
                Motion::timed(120.0, Easing::Decelerate),
            )
    }

    #[test]
    fn a_transition_moves_every_property_it_names() {
        let mut a = Animator::new();
        a.play(7, &lifted());
        assert_eq!(a.active_count(), 2);
        settle(&mut a, MotionKey::new(7, MotionProperty::Scale), 16.0);
        assert_eq!(
            a.value_or(MotionKey::new(7, MotionProperty::HoverWash), -1.0),
            1.0
        );
        assert_eq!(
            a.value_or(MotionKey::new(7, MotionProperty::Scale), -1.0),
            1.02
        );
    }

    #[test]
    fn reversing_a_transition_returns_every_property_to_rest() {
        let mut a = Animator::new();
        a.play(7, &lifted());
        settle(&mut a, MotionKey::new(7, MotionProperty::Scale), 16.0);
        a.play(7, &lifted().reversed());
        settle(&mut a, MotionKey::new(7, MotionProperty::Scale), 16.0);
        assert_eq!(
            a.value_or(MotionKey::new(7, MotionProperty::HoverWash), -1.0),
            0.0
        );
        assert_eq!(
            a.value_or(MotionKey::new(7, MotionProperty::Scale), -1.0),
            1.0
        );
    }

    #[test]
    fn a_staggered_transition_offsets_whole_nodes_not_properties() {
        let mut a = Animator::new();
        a.play_staggered(&[10, 11, 12], &lifted(), 40.0);
        a.tick(16.0);
        // Node 10's two properties both moved; nodes 11 and 12 are waiting.
        assert!(a.value_or(MotionKey::new(10, MotionProperty::HoverWash), 0.0) > 0.0);
        assert!(a.value_or(MotionKey::new(10, MotionProperty::Scale), 1.0) > 1.0);
        assert_eq!(
            a.value_or(MotionKey::new(11, MotionProperty::HoverWash), 0.0),
            0.0
        );
        assert_eq!(
            a.value_or(MotionKey::new(12, MotionProperty::HoverWash), 0.0),
            0.0
        );
    }

    #[test]
    fn an_empty_transition_is_a_no_op_rather_than_an_error() {
        let mut a = Animator::new();
        let empty = Transition::new();
        assert!(empty.is_empty());
        a.play(1, &empty);
        assert!(a.is_idle());
    }

    // ── the Phase 27 contract, restated against the new machinery ───────────

    #[test]
    fn timed_tracks_still_land_exactly_and_still_retire() {
        let mut a = Animator::new();
        for easing in [
            Easing::Linear,
            Easing::Standard,
            Easing::Decelerate,
            Easing::Accelerate,
            Easing::Spring,
        ] {
            let k = MotionKey::new(1, MotionProperty::Scale);
            a.start(k, 0.0, 1.0, 100.0, easing);
            settle(&mut a, k, 16.0);
            assert_eq!(a.value_or(k, -1.0), 1.0, "{easing:?} did not land");
            assert!(a.is_idle(), "{easing:?} left a track running");
        }
    }

    #[test]
    fn a_spring_still_obeys_no_ceiling_but_a_tween_still_does() {
        let mut a = Animator::new();
        a.start(key(), 0.0, 1.0, 10_000.0, Easing::Linear);
        let track = a.tracks.get(&key()).copied().expect("running");
        match track.motion {
            Motion::Timed { duration_ms, .. } => assert_eq!(duration_ms, MAX_DURATION_MS),
            Motion::Spring(_) => panic!("a timed start produced a spring"),
        }
    }
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
