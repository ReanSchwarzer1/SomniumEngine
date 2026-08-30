# Scripting

Somnium scripts are **Luau** files with a `.luau` extension. They live in
`assets/`, appear in the Content Drawer with a script icon, and attach to
entities in the Details panel.

## Making a script

**Right-click in the Content Drawer** — on empty space or on an item — for
**New Folder…**, **New Script…**, **Rename…**, **Show in Folder** and
**Refresh**. New Folder and New Script create inside whichever folder the
drawer is currently showing, and both ask for a name; Enter confirms.

A new script starts from a strict-mode template with one declared
property and an empty `onFixedUpdate`, and is attached to the selection
straight away if there is one.

Nothing overwrites: a name that already exists is refused rather than
replacing the file, and so is one that would escape `assets/`. There is
deliberately **no Delete** — a right-click with no undo and no
confirmation is not a mistake anyone recovers from, and Show in Folder
puts you one step from a file browser that has a recycle bin.

**Show in Folder reveals the file; it does not open it.** Opening a
`.luau` in an editor means launching whatever the OS has associated with
the extension, which is a coin toss. Choosing an editor is a later
sub-phase.

## Attaching a script

Select an entity, then click a `.luau` file in the Content Drawer. The
Scripts section of the Details panel gains a row for it. **New Script** in
that section does the same as the drawer's, without the folder choice.

Each row has a checkbox that switches the attachment on and off, arrows
that move it earlier or later in execution order, and an ✕ that removes it.
Attach, remove, reorder and every property edit are undo steps.

Moving a row with the arrows **renumbers** every attachment's execution
order to match the list. That is the one gesture that overwrites an
execution order you may have typed elsewhere.

Renaming a `.luau` file gives it a **new asset id** — the id comes from
the path — so attachments that named the old one report "asset not
imported" until you re-attach them. That is deliberate: silently
re-pointing them would be wrong if you meant to fork the script.

## Declared properties

The rows under an attachment are generated from what the script declared.
Nothing about them is written in the editor's code, so adding a property is
a one-line change to the `.luau` file:

```luau
fields = {
    speed = Field.number(4.0, { min = 0.0, max = 30.0,
        description = "Metres per second." }),
}
```

Numbers and booleans are editable. A string, an entity reference or an
asset reference is shown read-only for now — visible rather than missing,
because a missing row looks like the script failed to declare it.

A value you set in the editor overrides the script's own default. Deleting
a property from the script drops the authored value and says so in the
Output Log; it does not stop the scene loading.

## State that survives a reload

Write `self.x = self.x or default` in `onInit`, never `self.x = default`.

A reload calls `loadState` **before** `onInit` replays, so an
unconditional assignment throws away the state that was just restored.
The symptom is subtle and annoying: editing a script snaps the player back
to looking north, or resets a counter, every time you save.

```luau
onInit = function(self, ctx)
    self.yaw = self.yaw or 0.0
end,
saveState = function(self) return { yaw = self.yaw } end,
loadState = function(self, state) self.yaw = state.yaw end,
```

## Physics

An entity with a `somnium.RigidBody` exposes `velocity` to scripts, and
that is how a character moves — set the velocity you want, do not push:

```luau
uses = { ["somnium.RigidBody"] = { "velocity", "grounded" } },

onFixedUpdate = function(self, ctx, dt)
    local body = ctx.self.rigidBody
    body.velocity = vector.create(wishX * speed, body.velocity.y, wishZ * speed)
end,
```

Leave `velocity.y` alone unless you are jumping — writing all three
cancels gravity every step and leaves you hovering.

The engine reads Jolt into the component before your script runs and
writes it back after, so `body.velocity` is what physics actually gave
you last step, not what you asked for. `ctx:applyForce` is still there and
is still the right tool for a push, an explosion or thrust.

**`grounded` is a vertical-speed heuristic, not a ground cast.** It reads
true for a few frames at the apex of a jump, where vertical speed also
passes through zero. Edge-trigger your jump on `actionPressed("Jump")` rather
than `actionDown("Jump")`, and add a cooldown that outlasts the jump — see
`assets/scripts/first_person_controller.luau`, which does both.

## The lifecycle

```
Loaded → Initialized → Started → Enabled ⇄ Disabled → Destroyed
```

| Callback | When |
|---|---|
| `onInit` | The instance was created. Peers may not have started. |
| `onStart` | Every instance that existed at that moment is initialised. |
| `onEnable` / `onDisable` | The attachment was switched on or off. |
| `onFixedUpdate(self, ctx, dt)` | Once per fixed step, before physics. |
| `onUpdate(self, ctx, dt)` | Once per frame. |
| `onEvent(self, ctx, events)` | Queued events, in sequence order. |
| `onDestroy` | Teardown. |
| `saveState` / `loadState` | Declared state, across a reload. |

A script does not have to define any of them. One that defines none still
reaches `Enabled`; it simply does nothing.

Scripts run **only while Play is running**. Stop tears every instance down
and restores the authored world exactly — including entities a script
destroyed, entities it created, and components it added or removed. That is
why a script cannot dirty the scene you are editing.

## Reading and writing

Declare what your script touches, and it arrives as a plain table on
`ctx.self`:

```luau
uses = { ["somnium.Transform"] = { "translation" } },

onFixedUpdate = function(self, ctx, dt)
    local t = ctx.self.transform
    t.translation = t.translation + vector.create(self.speed * dt, 0, 0)
end,
```

Name the **fields**, not just the component. `uses = { "somnium.Transform" }`
mirrors everything, which means marshalling a rotation quaternion in both
directions every frame for a script that only reads a position. Measured,
that took a thousand entities from 2.3 ms to 5.5 ms.

For another entity's components, use `ctx:get(entity, component, field)` and
`ctx:set(...)`.

## The visibility rule

**One script's writes are not visible to another until the commit point at
the end of the phase.** Your own writes *are* visible to you, because
`ctx.self` is a real table you are editing.

This is a real constraint on how gameplay code is written, and it is the
reason the same code stays correct if script execution is ever run in
parallel. The alternative — every write visible immediately — makes
execution order load-bearing in a way nobody can reason about.

`ctx:set` on another entity is deferred, so a read-modify-write loop
through `ctx:get`/`ctx:set` re-reads the same pre-phase value every
iteration. Use the mirror for your own components.

## Spawning

`ctx:spawn()` returns a **token**, not an entity: the entity does not exist
until the commit point. The next phase finds it at `ctx.spawns[token]`.

```luau
if self.token == nil then
    self.token = ctx:spawn()
elseif ctx.spawns ~= nil and ctx.spawns[self.token] then
    ctx:despawn(ctx.spawns[self.token])
    self.token = nil
end
```

`ctx:despawn(ctx.entity)` is allowed. The callback finishes; teardown
happens at the safe point afterwards.

## Input

Scripts read named actions, never keyboard or gamepad controls:

```luau
local move = ctx.input:vector2("Move")
local look = ctx.input:vector2("Look")
if ctx.input:actionDown("Sprint") then ... end
if ctx.input:actionPressed("Jump") then ... end
```

`axis(name)` reads a one-dimensional value. `vector2(name)` returns a vector
whose X/Y components hold a two-dimensional value. The default gameplay map
defines `Move`, `Look`, `Jump`, `Sprint`, `Interact`, and `Pause`; rebinding a
keyboard key or switching to a gamepad does not change script code.

## Determinism

Fixed-step callbacks get `dt` and simulation time and nothing else. There
is no wall clock, no OS entropy, no filesystem and no network — `os`,
`io`, `debug`, `require`, `loadstring`, `getfenv`, `setfenv`,
`collectgarbage` and `print` are all absent, and `_G` is not reachable.
Each attachment gets its own seeded random stream derived from the world
seed and its durable id, so a replay is identical.

Execution order is `(execution_order, entity id, attachment id)` — authored
data only, never archetype order or hash-map iteration order.

The promise is **same build, same platform**. Cross-platform float and
physics behaviour is unaudited and is not claimed.

## When a script goes wrong

A script that raises, runs away, or allocates without bound is contained:
its command batch for that callback is discarded, the error is logged with
a file, line and traceback, and its peers are unaffected. After three
failures in a row the attachment is switched off and the Details row says
so. An infinite loop is interrupted on a deadline; the next frame is
ordinary.

The status area counts blocking script diagnostics. The Output Log carries
the messages, positioned as `file:line:column`.

## Sharing code between scripts

```luau
local util = require("scripts/util")
```

`require` takes a **string literal** and nothing else. Not a variable, not
a concatenation, and you cannot store `require` in a local — all three are
compile errors with a line number.

That is not a limitation that happened; it is the point. The engine reads
your dependency graph out of the source *without running it*, and three
things need that: a reload has to know which scripts an edit affects
before it touches anything, the cook has to know what to bundle, and a
`require` cycle is caught once, up front, naming both files.

A required module is evaluated **once per session and frozen**. That makes
a shared helper genuinely shared, and stops one script rewriting a helper
for every other script in the game. Return a table of functions and
constants; do not expect to keep mutable state in one.

Module names resolve relative to the requiring file first, then from the
project root, then under `assets/`. So a file next to yours is
`require("helpers")` and `assets/scripts/util.luau` is
`require("scripts/util")`. Importing a script imports what it requires —
you never have to hunt down dependencies by hand.

## Reload

**F5** recompiles every imported script, and the editor also watches the
files you have imported: saving in an external editor reloads it within a
quarter of a second. The delay is deliberate — editors often write a file
in more than one go, and reloading a half-written file reports a syntax
error you never made.

A file that no longer compiles **leaves its live instances running** and
publishes diagnostics — nothing about the running world changes. Editing
a shared module reloads every script that requires it, transitively.

A file that does compile has its instances rebuilt: `saveState` is asked
for the old state, the new module is instantiated, `loadState` gives it
back, and the lifecycle replays `onInit`, `onStart`, `onEnable` at the
next frame boundary.

Only versioned pure data survives. A closure, a coroutine, userdata and any
engine resource the script was holding do not. Anything your script needs
across a reload has to come back out of `saveState` as plain values.

### Renaming a property

Bump `schemaVersion` and say how the old values map to the new ones:

```luau
schemaVersion = 2,
fields = { velocity = Field.number(1.0) },
migrateProperties = function(self, props, fromVersion)
    if fromVersion < 2 and props.speed ~= nil then
        props.velocity = props.speed
        props.speed = nil
    end
    return props
end,
```

Without this, renaming a field loses every value anyone set in the editor:
the old key no longer matches anything the script declares, so it is
dropped with a warning in the Output Log. Only the person who made the
rename knows the two are the same field.

The migrated values are written back into the scene, so they survive a
save as well as the reload.

## Capabilities

Every effect a script can have on the world goes through one command
boundary, and each command needs a capability: writing fields, adding or
removing components, spawning, despawning, forces, audio, events, logging.

A project's own scripts get all of them. The point of the manifest is a
future mod tier, where the default is nearly empty — change fields, log,
emit events, and nothing else. A refused command is reported in the Output
Log and does not fault the script; it simply does not happen.

## Bytecode cache

Compiled scripts are cached under `target/script-cache/`. The cache is
keyed on both a fingerprint of the runtime that produced it and a hash of
the source, and a mismatch in either is a cache miss that recompiles — so
it can never hand a stale artifact to the VM. Bytecode is a cache, never
storage; deleting the directory costs nothing but a recompile.

## Editor autocomplete

`assets/scripts/somnium.d.luau` is generated from the same component
registry the engine, the scene format and the Details panel read, so it
cannot drift from them. It is rewritten on startup whenever it differs.

Whether your editor understands it is a separate question: the Luau
language server most people use is community-maintained and is not an
official Luau tool. Nothing here claims first-class IDE support.

## What is not here yet

No breakpoints or stepping. Diagnostics carry `file:line:column` but the
Output Log does not turn them into a jump — you read the position and go
there yourself. No mod sandbox: the capability manifest that a mod tier
would need exists and is enforced, but nothing yet loads untrusted
packages into their own VM. Those are later sub-phases, and this page will
say so until they land.
