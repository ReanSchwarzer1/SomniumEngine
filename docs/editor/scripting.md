# Scripting

Somnium scripts are **Luau** files with a `.luau` extension. They live in
`assets/`, appear in the Content Drawer with a script icon, and attach to
entities in the Details panel.

## Attaching a script

Select an entity, then click a `.luau` file in the Content Drawer. The
Scripts section of the Details panel gains a row for it. **New Script** in
that section writes a fresh file into `assets/scripts/` from a strict-mode
template and attaches it in one step.

Each row has a checkbox that switches the attachment on and off, arrows
that move it earlier or later in execution order, and an ✕ that removes it.
Attach, remove, reorder and every property edit are undo steps.

Moving a row with the arrows **renumbers** every attachment's execution
order to match the list. That is the one gesture that overwrites an
execution order you may have typed elsewhere.

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

`ctx.input:isKeyDown(code)`, `:isKeyPressed(code)` and `:isMouseDown(button)`.
Letters and digits are their uppercase ASCII values, so
`string.byte("W")` is the W key. Mouse buttons are 0 left, 1 right,
2 middle. The named keys are:

| Key | Code | Key | Code |
|---|---|---|---|
| Space | 32 | Down | 263 |
| Escape | 256 | Shift | 264 |
| Enter | 257 | Control | 265 |
| Tab | 258 | Alt | 266 |
| Backspace | 259 | | |
| Left | 260 | Right | 261 |
| Up | 262 | | |

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

## Reload

**F5** recompiles every imported script.

A file that no longer compiles **leaves its live instances running** and
publishes diagnostics — nothing about the running world changes. A file
that does compile has its instances rebuilt: `saveState` is asked for the
old state, the new module is instantiated, `loadState` gives it back, and
the lifecycle replays `onInit`, `onStart`, `onEnable`.

Only versioned pure data survives. A closure, a coroutine, userdata and any
engine resource the script was holding do not. Anything your script needs
across a reload has to come back out of `saveState` as plain values.

## What is not here yet

No breakpoints or stepping. No `.d.luau` declarations for editor
autocomplete. No file watcher — reload is the F5 key, not a save hook. No
mod sandbox. Those are later sub-phases, and the help page will say so
until they land.
