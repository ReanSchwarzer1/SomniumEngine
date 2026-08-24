# MORROWIND-H — UI motion, generalised

**Complete, 2026-08-25.** Track 1 (VIVEC). §8: *"Phase 27 shipped `motion.rs`
(524 lines) for editor chrome. This generalises it to a runtime system: a tween
and transition API with easing curves that come from **CONTROL-K's curve
editor**, state transitions, staggering, and a spring model for the cases where
duration is the wrong parameterisation."*

All five, plus a sixth that was not in the plan and had to be.

## The sixth thing: a game canvas never ticked

`UiManager::end_frame` has advanced motion since Phase 27-C. `UiCanvas::render`
laid out and drew and **never called `tick`**. A game could start a tween and
watch it sit at its origin forever.

Exactly the shape of MORROWIND-E2: the capability existed, the runtime path to
it did not, and no test caught it because every motion test drove the animator
directly. Fixed by giving `UiCanvas` the same `last_frame_at` clock the shell
has — and then by giving it something the shell does not need:

```rust
canvas.tick_motion(dt_ms);   // explicit
canvas.is_animating();       // for a game that redraws on change
```

`render` ticks from its own `Instant`, which is right for a HUD and wrong for
two cases a game actually has: a **fixed-timestep** simulation, where UI motion
should advance with the simulation rather than the frame; and a **paused** game
whose pause menu must still animate while nothing else does. `tick_motion`
resets the clock, so an explicit tick and the automatic one cannot both charge
the same milliseconds.

## Springs, and why `Easing::Spring` was not already one

Phase 27 shipped `Easing::Spring`: a critically damped step response,
*normalised* so `f(1) == 1` exactly over a stated duration. That is a **shape**,
and it is the right thing for press feedback, where the duration is a design
token and the overshoot must be zero.

It is the wrong thing for anything **interrupted**. Retarget a duration tween
mid-flight and it restarts from the current value with zero velocity — a drawer
flying open and told to close visibly stops dead first. That is the case §8
means by *"duration is the wrong parameterisation"*, and the fix is a spring
that is a system rather than a curve:

```rust
pub struct Spring { pub stiffness: f32, pub damping: f32, pub mass: f32 }
pub enum Motion { Timed { duration_ms, easing }, Spring(Spring) }
```

`Motion::Timed` is Phase 27's model and stays the default. Both live in one
`Track`, which grew `velocity`, `delay_ms` and `current`: a spring's state is
not a function of elapsed time, so a value that used to be *derived* on read is
now *integrated* on tick and cached.

**Velocity is carried across a retarget, and only between springs.** A timed
retarget starts from rest by construction — its shape is a function of elapsed
time and nothing else — so carrying velocity into one would be meaningless. The
test asserts both halves, because the contrast is the feature.

### Three bounds, each earned

| Bound | Value | Why |
|---|---|---|
| `SPRING_MAX_STEP_S` | 1/240 s | Semi-implicit Euler with a stiff spring diverges at large `dt`. A 100 ms frame after a breakpoint would send a widget to infinity, and a UI that explodes after a breakpoint is a UI nobody debugs twice. Tested with a 500 ms tick on a stiffness-4000 spring. |
| `MAX_SPRING_MS` | 4,000 ms | A spring has no duration by construction, so this is not a design token — it is what stops a mis-parameterised spring animating forever and keeping the shell awake. |
| `SPRING_EPSILON` | 0.25/1000 | Arrived *and* stopped. Either test alone is wrong: a spring passing through its target at speed has arrived and is not done; one creeping in from far away has stopped and is not done. |

### A preset was wrong, and the predicate caught it

`Spring::SNAPPY` was first written as `stiffness 320, damping 32`. Critical
damping for stiffness 320 at mass 1 is `2 * sqrt(320) = 35.78`, so a preset
documented as *"quick and tight"* was **underdamped and would have wobbled**.
`Spring::overshoots()` exists because Phase 27 §9.3 forbids overshoot on a
control the user is scrubbing, and it failed the test on the first run. The
constant is now 36, and a `WOBBLY` preset carries the deliberate overshoot for a
notification or a badge — named, so `overshoots()` reads `true` somewhere a
reviewer can see it rather than in a hand-tuned literal at a call site. **The
old value is now a test assertion**, so the same mistake cannot come back.

## Authored curves, from CONTROL-K

```rust
let id = animator.register_curve(curve_from_the_editor);
animator.start(key, 0.0, 1.0, 120.0, Easing::Curve(id));
```

`Easing::Curve(CurveId)` holds an **index, not a `Curve`**, so `Easing` stays
`Copy` — every widget in the editor passes it by value and boxing a curve into
it would have been a change to all of them.

`replace_curve(id, curve)` keeps every id valid, which is what a live editor
needs: dragging a tangent must change the motion of the widgets already
referencing it, rather than registering a second curve and leaving the first
being animated by nothing.

**Authored curves are not normalised.** A curve that does not pass through
(0,0) and (1,1) will not land exactly on its target — deliberately. Normalising
would make an authored ease-out-back impossible, and overshoot is precisely what
an author reaches for a curve to get. Tested with a 0 → 1.2 → 1.0 curve.

`Easing::apply` gained a `Curve` arm that eases **linearly**. It is reachable
only by a caller holding an `Easing` away from its animator, and the choice is
between a UI that eases wrong in a way somebody can see and a UI that panics on
a hover. `Animator::ease` is the resolving path and the two agree on the
unregistered case, so they cannot disagree.

## Staggering, and the frame-quantisation bug

```rust
animator.start_staggered(keys, rest, to, motion, 30.0);
```

Eight rows entering together is a pop; the same eight at 30 ms apart is a
cascade. Order is the caller's, because it is the visual order the cascade
should follow and no sort inside the animator could know it.

The bug worth recording: **a delay must not quantise to the frame**. A 20 ms
delay under a 16 ms frame crosses the delay 4 ms into the second tick, and the
track must then receive the remaining **12 ms** — not 16, and not 0. The naive
implementation gives one of the two wrong answers depending on which side of the
branch it puts the subtraction, and the visible result is a cascade whose steps
land on frame boundaries instead of on its stagger. There is a test.

**Reduced motion ignores the stagger too.** Staggering *is* timing, and the
Phase 27 contract is that timing is the only thing reduced motion changes.

## Transitions

```rust
let lifted = Transition::new()
    .with(MotionProperty::HoverWash, 0.0, 1.0, Motion::timed(120.0, Easing::Standard))
    .with(MotionProperty::Scale, 1.0, 1.02, Motion::timed(120.0, Easing::Decelerate));

animator.play(node, &lifted);
animator.play(node, &lifted.reversed());
animator.play_staggered(&nodes, &lifted, 40.0);
```

A state change is rarely one property: a card lifting is a scale, a shadow and a
wash. Three `start` calls in a row is three chances to give one of them a
different duration by accident, and that drift is the kind nobody sees in review
and everybody sees on screen.

`reversed()` is the exit half of an enter/exit pair **without a second
declaration to keep in step with the first** — which is the way these drift.

## What the slice does with it

`vvardenfell`'s HUD hides and shows on **Tab**, through `on_os_event` →
`set_shown` → `on_render_ui`. The transition deliberately **mixes** a spring on
the scale with a timed fade on the wash: the fade has a duration the design
states, and the scale should keep its momentum if the player opens and closes a
menu quickly, which is exactly what a duration cannot express.

`set_shown` is idempotent, so a game calling it every frame from a
`cutscene_active` flag does not restart the transition sixty times a second.

**One bug found by writing the slice**, and it is the interesting one: the HUD
popped instead of animating on the first hide. `HudTree` started with
`shown: true` and never *told the animator*, so the reverse transition found
`rest == target` for a key that had never been driven and settled instantly.
**An animator with no value for a key is not the same as one holding 1.0.** The
fix is to seed the shown state with `set_immediate` at construction, which is
what a game does on load anyway.

## Deferred, deliberately

§8 names LyShine's `AnimNode` / `AnimSequence` / `AnimSplineTrack` as the
reference for a **track-based** variant, and says that variant is
**MORROWIND-L's fifth consumer**. Nothing track-based is built here: a timeline
belongs in the timeline sub-phase, and building half of one now would be a
second timeline for L to reconcile with. Recorded so the omission is a decision.

## Tests: 27 new, 0 failures

- **`motion::morrowind_h_tests`, 19** — a spring arrives *and* settles exactly;
  a critically damped spring does not overshoot; `overshoots()` catches the
  damping-32 mistake; **retargeting a spring carries velocity and a tween does
  not**; a 500 ms stalled frame does not diverge; a spring cannot animate
  forever; an authored curve drives a track; editing a curve moves the widgets
  already using it; an unregistered curve eases linearly rather than panicking;
  an authored curve may overshoot on purpose; a stagger starts rows in order and
  not together; **a stagger does not quantise to the frame**; reduced motion
  ignores the stagger; reduced motion settles a spring; a transition moves every
  property it names; reversing returns every property to rest; a staggered
  transition offsets nodes rather than properties; an empty transition is a
  no-op; and — the Phase 27 contract restated — every timed easing still lands
  exactly and still retires, and the duration ceiling still applies.
- **`runtime`, 2** — a game canvas advances its own motion; a game canvas drives
  a spring through the public surface only.
- **`vvardenfell`, 4** — hiding animates rather than popping; the HUD assembles
  rather than appearing; setting the state it is already in is a no-op;
  show/hide/show returns to exactly where it started.

`somnium_ui`: **493 passed, 0 failed** (was 471). `vvardenfell`: 16 passed.
**The 15 Phase 27 motion tests all still pass unchanged**, which is the
must-not-break assertion for this sub-phase.

## Files

```
~ crates/somnium_ui/src/motion.rs      Spring, Motion, CurveId, Transition,
                                       TransitionStep, start_with/start_delayed/
                                       start_staggered, play/play_staggered,
                                       register_curve/replace_curve/ease,
                                       the advance() integrator, 19 tests
~ crates/somnium_ui/src/runtime/mod.rs the motion clock a game canvas never had,
                                       motion()/motion_mut()/tick_motion()/
                                       is_animating(), 2 tests
~ crates/somnium_ui/src/lib.rs         re-export the motion vocabulary
~ crates/somnium_core/src/lib.rs       re-export it again, plus ElementState
                                       and PhysicalKey
~ examples/vvardenfell/src/hud.rs      show/hide transition, seeded state, 4 tests
~ examples/vvardenfell/src/main.rs     Tab toggles the HUD
```
