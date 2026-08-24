# MORROWIND-AE — input actions (Seam 5)

**Complete, 2026-08-24**, with one migration owed and named below. Track 8
(ALMSIVI). **This also closes MORROWIND-F**, whose item 5 was a forward
dependency on this sub-phase.

## What this replaces

`script_input.rs` had **54** `KeyCode::` arms; `hello_engine` had **16**. Every
one of them is a key a player cannot rebind, and §4.6's grep for `gamepad` and
`action_map` returned **zero** — the engine had no concept of an action at all.

Seam 5:

> *"Keycodes appear in exactly one place: the device layer that resolves a
> `ControlPath` to a hardware control. Game code, script and UI see actions.
> Rebinding is a runtime operation over the same data."*

`winit::keyboard::KeyCode` now appears in `device.rs` and nowhere else in the
crate.

## The four layers, and what each boundary buys

```
 winit events  ->  Devices          keycodes live here, and only here
                   ControlPath      <Keyboard>/w, <Gamepad2>/leftStick
                   Processor        dead zone, invert, scale, normalise
                   Interaction      press, hold, tap, multi-tap
 game code     <-  ActionValue      Digital | Analog1D | Analog2D
```

- **Path over keycode** so a binding is *data* — a file a player edits, a setting
  that survives a restart, a control scheme shipped as content.
- **Processor** so inverting a Y axis is a preference rather than a code change,
  and so a dead zone is radial in one place instead of per-axis in twenty.
- **Interaction** so "tap to reload, hold to holster" is two bindings on one
  control and neither knows the other exists.
- **Action value** so movement code reads one `Vec2` and never learns whether it
  came from WASD or a stick.

## Six decisions worth arguing with

### A path is a string, not an enum

The obvious first design is an enum of every control on every device, and it is
wrong for one reason that outweighs its type safety: **a binding is a file a
player edits.** A rebinding in settings, a control scheme shipped as content, a
mod that adds a binding — all text, and an enum makes each a serialisation
problem with a migration every time a device gains a control. Parsing is
validated at the boundary, so the stringly-typed part does not smear into the
matcher.

### A dead zone is radial, not per-axis

A per-axis dead zone leaves a **square hole** at the centre of a stick: pushing
diagonally at low magnitude registers on one axis and not the other, which feels
like the stick catching. It is the most common analog-input bug and the test
`a_dead_zone_is_radial` is the one that would catch it.

### `Normalize` clamps; it does not normalise

W+D gives magnitude √2, so a player holding two keys moves **41% faster** —
every engine has shipped that. The fix is `clamp_length_max(1.0)`, not
`normalize()`: unconditional normalising turns a half-pushed stick into a fully
pushed one, which is the same bug mirrored.

### Strongest binding wins, not last

A player holding W while nudging a stick should move at whichever is pushed
further. Last-wins makes the answer depend on **binding order in a settings
file**, so reordering bindings changes how a character moves.

### A hold gates its value until it fires

Otherwise "hold to sprint" sprints from the first frame, which is the same as
not having the interaction. And a hold fires **once**, not every frame past the
threshold — repeating is how "hold to open the menu" opens it forty times.

### Conflicts are reported, not prevented

The tempting design refuses a colliding binding. Three reasons it is wrong:

- **Cross-map collisions are usually intentional.** Escape is Pause in gameplay
  and Cancel in the menu, and only one map is enabled at a time. A preventer
  either forbids that or must understand map enablement, which is a runtime
  property it cannot see.
- **Within-map collisions sometimes are too.** Confirm on `enter` *and* `space`
  is deliberate.
- **Refusing leaves the player stuck.** They wanted the key, the game said no,
  and now they must remember which other action has it. Reporting lets the UI
  say "this will unbind Crouch — continue?", which is what every shipped
  rebinding screen actually does.

`Conflict::same_map` is carried so a UI can present the two cases differently.

## Three things that only break on somebody else's hardware

Each has a test.

- **Focus loss releases every control.** A key released while the window is
  unfocused never reports, so a player who alt-tabs mid-sprint comes back
  sprinting into a wall with no way to stop.
- **Keys are physical, not logical.** A binding to `<Keyboard>/w` must be the
  same *position* on AZERTY, or a French player's WASD lands on ZQSD-shaped
  nonsense. Logical keys are for text entry; bindings are positional.
- **A disconnected pad reads as zero, not stuck-down.** A yanked cable must not
  leave a character walking forever.

Plus: deltas are cleared per frame, or the camera keeps turning after the mouse
stops — which reads as drift and gets filed against the camera code.

## Two defects the tests found

**1. An absent gamepad control returned the wrong *shape* of zero.**
`Digital(false)` for a stick would hand a scalar to a `Vector2` action, which
`convert` widens by a different route than the connected case takes. Two paths
to the same value is how they eventually stop agreeing. Now each control returns
its own zero.

**2. Device names were case-sensitive; control names were not.** `<keyboard>/w`
parsed as an `Unknown` device, so a lower-cased settings file would have
silently unbound every key in it. Worse, **the first version of that test papered
over it with an `unwrap_or_else` fallback** — the fallback was hiding the bug.
Both halves are case-insensitive now, and the test asserts it without a
fallback.

## Closing MORROWIND-F

F's record said item 5 was blocked and that this sub-phase would unblock it.
It has:

- `somnium_input`'s **default UI map** binds `Navigate`, `Confirm`, `Cancel`,
  `Next`, `Previous` — the names `somnium_ui::runtime::nav::action_names`
  declares, with a test on each side asserting the five words agree.
- `NavAction::from_actions` resolves a verb from those bindings. **A player who
  rebinds Confirm to their pad's east button gets it in menus**, which is what
  item 5 asked for, and `rebinding_confirm_changes_which_key_confirms` proves it
  end to end.
- `NavActions` is a **trait**, not a dependency on `somnium_input`. `somnium_ui`
  is drawn by the editor, by a game and by tests, and a hard dependency would
  make it un-testable without an input system while putting a crate edge where
  none is needed. `somnium_core::input_actions` is the six-line bridge, and it
  lives there because that is where the two crates already meet.
- `NavAction::from_key` **survives** as the keyboard-only path. The editor shell
  runs before a game's `InputSystem` exists, so refusing it a fallback would mean
  the editor could not be navigated at all. It is still the only place in
  `somnium_ui` that names a keycode.

Two UI-specific decisions came out of the wiring:

- **A stick at rest must not step focus.** A stick reads slightly non-zero when
  untouched, and a menu that walks on drift is unusable in a way that reads as
  hardware failure. `UI_NAVIGATE_DEADZONE` sits *above* the input crate's own
  dead zone: that one stops drift reaching the action, this one stops a
  deliberate-but-small push stepping focus. A menu is discrete and a walk is not.
- **Confirm beats a simultaneous navigate.** Acting on the confirm and then
  moving focus off the thing that was confirmed is the surprising order, decided
  once here rather than by each widget.

## Tests: 69 new, 0 failures

- **`path`, 7** — round-trips; components; **both halves case-insensitive**; device pairing; an unknown device surviving a round trip; errors that suggest the shape they wanted.
- **`processor`, 17** — the radial dead zone; direction preserved; full throw at the upper bound; sign kept; `Normalize` stopping the diagonal boost *and* leaving a half-pushed stick alone; chain order; a hold firing once; tap and hold complementary on one control; multi-tap forgetting a stale tap (or a player who taps once now and once next level triggers a dodge-roll at the worst moment).
- **`action`, 12** — one action serving keyboard and stick; W moving up the screen; the diagonal not faster; **strongest binding wins**; a disabled map silent; two maps sharing Escape; `just_activated` as an edge; a hold gating its value; JSON round-trip; a trigger as a button.
- **`device`, 11** — key names round-tripping; **focus loss releasing everything**; deltas not surviving the frame; a disconnected pad reading zero; **the right shape of zero**; unpaired taking the strongest pad; paired ignoring others.
- **`rebind`, 12** — cross-map collision flagged as cross-map; an action not conflicting with itself; a rebind applying *and* reporting; unbinding resolving it; composites conflicting on all four controls; paired pads not colliding; **Escape cancelling the prompt** rather than being bound to it.
- **`lib`, 7** and **`somnium_core::input_actions`, 3** — end to end.

## The owed migration

**The 54 `KeyCode::` arms in `script_input.rs` and the 16 in `hello_engine` are
still there.** The crate that replaces them exists, is tested, and has a default
map naming every verb they implement — but migrating them is a change to a
working script bridge and a 2,758-line example, and doing it in the same commit
as the crate would mean one commit nobody can review.

The census counts both, so the number is visible and cannot quietly stay. The
default gameplay map is the target vocabulary: `Move`, `Look`, `Jump`, `Sprint`,
`Interact`, `Pause`.

## Files

```
+ crates/somnium_input/Cargo.toml
+ crates/somnium_input/src/lib.rs         InputSystem, the default maps
+ crates/somnium_input/src/path.rs        ControlPath, DeviceKind
+ crates/somnium_input/src/processor.rs   Processor, Interaction, Phase
+ crates/somnium_input/src/action.rs      Action, ActionMap, Binding, ActionStates
+ crates/somnium_input/src/device.rs      the one place a KeyCode appears
+ crates/somnium_input/src/rebind.rs      conflicts, rebind, RebindListener
+ crates/somnium_core/src/input_actions.rs   the NavActions bridge (closes F)
~ crates/somnium_ui/src/runtime/nav.rs    NavActions, action_names, from_actions
~ crates/somnium_core/Cargo.toml, lib.rs
~ Cargo.toml                              workspace member + dependency
```
