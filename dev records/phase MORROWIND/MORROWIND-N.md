# MORROWIND-N — play-in-editor

**Status:** complete, 2026-08-30. Most of it was already shipped; this record
finds that, names the three pieces that were missing, and adds them.

## What the audit found

The stage lists six things. Four were already in tree, built by earlier phases
without being recorded against this one:

| The plan asks for | State before this record |
|---|---|
| A play/pause/step control | Play and Pause **shipped**; **no step** |
| A snapshot of world state on enter and a restore on exit | **Shipped** — `WorldCheckpoint::capture` on `begin_play_session`, restored by `end_play_session` |
| Separate input focus | **Shipped** — `game_owns_keyboard`, and editor overlays disabled during play |
| A runtime-versus-editor flag visible to script | **Missing** |
| An error path that returns to edit mode rather than taking the editor down | **Shipped**, and worth stating why — below |
| Modest in code, large in what it makes possible | — |

The plan's own framing is that *"the snapshot-and-restore is the hard half"*,
with `renpy-master/renpy/rollback.py` as the reference for the discipline. That
half was done: `WorldCheckpoint` snapshots the ECS through the type registry on
enter and puts the authored world back on Stop, so *"Stop is exact rather than
approximately exact"*. It also writes an autosave first
(`AutosaveReason::BeforePlay`), because the checkpoint restores the ECS and
explicitly not the renderer's terrain and map state — a file on disk is the only
thing that survives a session that ends badly enough to need it.

## The step control

`SimulationState` had `Editing`, `Playing` and `Paused`, and the fixed-step loop
read `if state != Paused { accumulate; while acc >= fixed_dt { step } }`. A step
is that loop, run once, from `Paused`.

```rust
let stepping = state == Paused && pending_steps > 0 && play_session_active;
if state == Playing || stepping { sync_scripts(dt); }
if state != Paused || stepping {
    if stepping { pending_steps -= 1; accumulator += fixed_dt; }
    else        { accumulator += dt.min(0.1); }
    while accumulator >= fixed_dt { … }
}
```

Four decisions in that, each of which could have gone the lazy way:

- **`pending_steps` is a counter, not a flag.** At 60 Hz a key repeat is faster
  than the frame that would consume it, so a flag drops steps while the control
  is held.
- **A step adds exactly `fixed_dt`, never the wall clock.** The point of a step
  is that it is the same size every time; feeding the accumulator a slow frame's
  `dt` would turn one press into three.
- **Scripts are reconciled on a stepped frame too.** `sync_scripts` ran only
  while `Playing`, so stepping would never have initialised an attachment added
  while the simulation was held — the exact case somebody debugging by stepping
  is in.
- **A step from `Playing` or from `Editing` is refused and logged, not
  reinterpreted.** Stepping a running simulation either does nothing visible or
  fights the accumulator; stepping from Edit advances a clock that is not
  running. Guessing which the user meant is how a debugging tool stops being
  trustworthy.

`Stop` clears any owed steps, so a queued press cannot advance a session that
has already been torn down.

## The runtime-versus-editor flag

The plan asks for *"a runtime-versus-editor flag visible to script"*. Taken
literally that flag is a constant: **scripts only ever run inside a play
session** — `sync_scripts` is gated on it, because *"scripts are not allowed to
dirty the edit-time scene"* — so a script asking "am I in the editor" always
gets the same answer and learns nothing.

The distinction a script can actually act on is **live or held**:

```luau
onFixedUpdate = function(self, ctx)
    if ctx.stepping then … end
end
```

A stepped frame is one fixed step separated from the last by however long the
user took to press the button again. Anything paced against the wall clock — an
interpolation, a per-frame sound, an animation driven off real time — behaves
differently on one, and `ctx.stepping` is how it can tell.

It rides on `TimeSnapshot`, which is the seam scripts already read time through,
and reaches Luau as `ctx.stepping`. `assets/scripts/somnium.d.luau` regenerated
itself on the next editor run, which is how declarations stay honest here.

**One bug worth recording, because the shape of it recurs.** The flag was first
derived from `pending_steps > 0`, which is false during the very step it
describes — the counter is spent before the step it pays for runs. It is now an
explicit `stepping_now` set once per frame. A counter and the thing it authorises
are not the same fact.

## The error path, and why nothing was added

The requirement is *"an error path that returns to edit mode rather than taking
the editor down"*, and the contrast in that sentence is the point: errors must be
survivable.

They are. `drain_script_output` takes logs, diagnostics and rejections every
frame — *"even while stopped, so a compile error raised by an import is not
stuck in a buffer until the next Play"* — routes them to the Output Log, and
publishes an error count to the UI. A failing attachment is reported and the
session keeps running.

**Auto-stopping play on a script error was considered and not added.** One
attachment throwing should not end a session that other attachments are
driving, and a debugging loop where every mistake ejects you to edit mode is
worse than one where the error is in the log with a count beside it. The
affordance already exists; adding a hair trigger to it would be a regression
wearing a feature's clothes.

## What is verified

- `a_script_can_tell_a_hand_driven_step_from_a_running_one` runs a script
  through the real `ScriptHost` and asserts it reads `live` on a running frame
  and `stepped` on a held one.
- The eight existing Phase 16-C gate tests still pass unchanged.

## What this does not claim

The step control has an entry in the command registry and the editor event
stream; it does **not** have a toolbar button beside Play/Pause/Stop yet. It is
reachable from the command palette, which is where CONTROL-A2's registry makes
every command reachable by construction, and the button is a shell change of the
kind MORROWIND-J step 2 is already going to make.

No process-separation option was taken. The plan raises Korge's shared-framebuffer
approach as *"structurally different"* and it remains unexplored; one snapshot
depth through `WorldCheckpoint` is what this engine has, and it is what the
stage said would be enough.
