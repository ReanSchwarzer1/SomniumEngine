# Phase 16 — Scripting

> *Devil May Cry: the engine keeps the world; the script only ever asks for it.*

> **Codename:** Devil May Cry
> **Status:** **16-A and 16-B COMPLETE (2026-08-16).** 16-C not started.
> §10 records session one; §11 records session two. Three throughput
> budgets were missed on the first pass and fixed; the one still over is
> over a ceiling shown to be arithmetically unreachable (§11.4).
> **Language decision:** **Luau, embedded through `mlua` 0.12, interpreter only.**
> **Record:** this file, plus
> [`phase 16/16-B_budgets.md`](phase%2016/16-B_budgets.md) for the measured
> numbers.
> **Research input:** the Phase 16 audit of `example_repo` (Falco, NeoAxis, Flax,
> Godot, O3DE, Bevy, Fyrox, Wicked, rbfx, Stride, Unreal) plus the independent
> language recommendation. That document is the premise of this plan; where this
> plan departs from it, §3 says so and why.
> **Do not copy source** from any engine in `example_repo`. Falco is
> unlicensed and is **study-only**; NeoAxis's licence forbids building a
> competing engine substantially based on its code; Flax and Unreal are EULA.
> Patterns get cited in `ATTRIBUTION.md` §13D; no lines get copied.

**Frozen by this phase** (a 16 sub-phase that changes any of these has gone
wrong): the Visibility Buffer architecture; the Great Lakes water datum and the
XV terrain contract; `GameApp`'s five callbacks and their order; rustc 1.88;
wgpu 29; the `.somnium` scene loader's ability to read a version-1 file. Phase
16 is a gameplay-layer phase. It must not touch a shader.

**The boundary rule, stated once:** no `mlua` type — `Value`, `Function`,
`Table`, `RegistryKey`, `Lua`, `UserData`, a VM pointer or a thread handle —
may appear in `somnium_ecs`, `somnium_core`, `somnium_ui`, a scene file, an
undo record or a save game. Luau exists in exactly one crate,
`somnium_script_luau`. Every other crate sees `somnium_script`'s neutral types.
This is not style. It is the entire exit strategy, and a review that finds an
`mlua::` import outside that crate fails the sub-phase that introduced it.

---

## 1. Executive decision

Adopt **Luau via `mlua` 0.12** with the **portable interpreter** (no native
codegen, no JIT). Rationale is in the research document and is not re-argued
here. What this plan adds is the sequencing claim:

> **The VM is the small part.** Somnium cannot host *any* scripting language
> today, in a language-independent way, for four concrete reasons that have
> nothing to do with Lua:
>
> 1. **`ComponentId` is process-local and lazily assigned**
>    ([`component.rs:52`](../crates/somnium_ecs/src/component.rs)). Nothing —
>    script, scene, save, or undo record — can durably name a component type.
> 2. **`World` cannot add or remove a component at runtime**
>    ([`world.rs:278`](../crates/somnium_ecs/src/world.rs) exposes `spawn`,
>    `despawn`, `get`, `get_mut` and nothing else). There is literally no way to
>    attach a script to an existing entity.
> 3. **`EngineContext` hands out `&mut World`, `&mut PhysicsWorld`,
>    `&mut SomniumRenderer` and `&mut UiManager`**
>    ([`context.rs:80`](../crates/somnium_core/src/context.rs)). Correct for
>    native Rust callbacks; an unacceptable ABI for scripts, which must never
>    hold a borrow across a call.
> 4. **Scene persistence is a hand-written version-1 JSON walk**
>    ([`scene_serial.rs:38`](../crates/somnium_core/src/scene_serial.rs)) that
>    knows `Transform`, `Light`, `MeshKind`, `Terrain` and `Water` by name. Each
>    new component type means more hand-written JSON. Script attachments with
>    author-declared exported properties cannot be expressed that way.

So Phase 16 spends its first session on those four, and only then adds a VM.
The gate on the first session is deliberately hostile to shortcuts: **the
foundation must ship, with tests, before `mlua` appears in any `Cargo.toml`.**

### Correction to `context.md`

`context.md` §7.1 (line 497) states a component is "Any `Copy` + `'static`
struct". The real trait is `pub trait Component: Send + Sync + 'static`
([`component.rs:40`](../crates/somnium_ecs/src/component.rs)), and `Children`
already ships a non-`Copy` `Vec<Entity>` component. The obstacle to richer
component data is registration, mutation and serialization — not storage.
16-A fixes that line.

---

## 2. Goals

1. **A `.luau` file attached to an entity in the editor runs, deterministically,
   at fixed step, and its authored properties survive save/load.** That is the
   headline and everything else serves it.
2. **A script cannot crash, hang or corrupt the engine.** An infinite loop, an
   allocation bomb, a stale entity handle, a `nil` index, a self-despawn
   mid-callback and a malformed source file each produce a diagnostic and a
   disabled attachment — never a panic, never a hitch that outlives its budget.
3. **Hot reload is transactional.** A reload that fails to compile leaves the
   old instance running and publishes diagnostics. A reload that succeeds
   migrates declared state and swaps at a frame boundary.
4. **One registry, four consumers.** The component schema registry built in 16-A
   powers script bindings, scene serialization, the Luau type declarations, and
   (when 26-J is asked for) the reflection inspector. No second source of truth.
5. **The language is replaceable.** Deleting `somnium_script_luau` and writing
   `somnium_script_rune` must not touch ECS, scenes, undo, editor fields or the
   gameplay API surface.

**Non-goals.** No visual scripting (O3DE shows how large that layer becomes, and
it must consume a *stable* reflected API — which does not exist until this phase
ends). No debugger (16-G, later, explicitly out). No Wasm mod tier (16-H,
later). No native codegen. No new language. No C#. No parallel script workers —
the design permits them, this phase does not build them.

---

## 3. Deliberate departures from the research document

Three, each stated so a later reader does not think they were oversights.

**3.1 The two-backend shootout (research 16-B) is cut.** Implementing the full
vertical slice twice — Luau and Rhai — is a session's work on its own and it
answers a question the research already answers with high confidence, using
evidence (Rhai's own documentation calls it blocking, single-threaded, and
advises against implementing full application logic in it) that a benchmark
will not overturn. **What survives is the benchmark corpus.** Every acceptance
budget the research proposed becomes a *gate on Luau* in 16-B of this plan, run
as a `criterion`-free, plain `cargo test --release` harness that prints a table.
If Luau misses those budgets, that is the falsification signal, and the neutral
`ScriptBackend` trait is what makes acting on it cheap. Rhai is not implemented.

**3.2 Hot reload moves to session two.** The research puts reload inside the
Luau adapter. Reload needs a file watcher, editor diagnostics and a play/stop
world boundary, all of which are session-two work. Session one instead ships the
*halves* reload depends on — the declared `save_state`/`load_state`/`migrate`
contract and the instance teardown path — and proves them with a synthetic
in-process "reload" test that swaps a module without touching the filesystem.

**3.3 The reflection registry lives in `somnium_ecs`, not `somnium_script`.**
The research is right that one registry must serve scripts, the inspector, the
serializer and the declaration generator. It follows that the registry cannot
live in the scripting crate, or the serializer would depend on scripting. It
goes in `somnium_ecs::reflect`, and `somnium_script` consumes it.

---

## 4. Architecture

### 4.1 Crates

```
somnium_ecs                    (+ reflect: schemas, stable ids, patches)
      │                        (+ world: insert/remove component, migration)
      ▼
somnium_script                 neutral. ScriptValue / Snapshot / Command /
      │                        ScriptBackend / lifecycle state machine /
      │                        scheduler / instance registry.
      │                        Depends on: somnium_ecs, glam, serde. NOT mlua.
      ├──────────────┐
      ▼              ▼
somnium_script_luau   somnium_core
  the ONLY crate      owns the runtime, drives phases from the frame loop,
  that names mlua     registers built-in component schemas, owns the
                      ScriptAsset type and the command applier.
```

`somnium_core` gains a dependency on both. The backend is chosen by the asset's
language tag through a small registry, not by a `cfg`, so a second backend is an
addition rather than an edit.

### 4.2 The neutral contract (`somnium_script`)

```rust
pub trait ScriptBackend: Send {
    fn language(&self) -> LanguageTag;
    fn compile(&mut self, asset: &ScriptSource) -> Result<CompiledModule, Diagnostics>;
    fn describe(&mut self, module: &CompiledModule) -> Result<ScriptSchema, Diagnostics>;
    fn instantiate(&mut self, id: ScriptInstanceId, module: &CompiledModule,
                   props: &PropertyBag) -> Result<(), ScriptError>;
    fn invoke(&mut self, id: ScriptInstanceId, call: Callback,
              snapshot: &ScriptSnapshot, out: &mut CommandBuffer) -> Result<(), ScriptError>;
    fn export_state(&mut self, id: ScriptInstanceId) -> Result<ScriptValue, ScriptError>;
    fn import_state(&mut self, id: ScriptInstanceId, state: ScriptValue) -> Result<(), ScriptError>;
    fn unload(&mut self, id: ScriptInstanceId);
    fn set_deadline(&mut self, budget: Budget);
    fn memory_used(&self) -> usize;
}
```

`describe` is the piece the research's declarative descriptor buys us: the
module is evaluated **once, in a restricted environment with no world APIs**, to
read `Script.define({...})`. Top-level module code therefore cannot mutate the
scene during import or while the editor merely inspects the asset.

`ScriptValue` is exactly the research's enum. Two notes it makes that must be
honoured in the type: **Luau has no 64-bit integer** (exact integers stop at
2^53), so `Entity`, `Asset` and `Instance` are opaque handles carried as
userdata, never as numbers; and `Object` is keyed by `FieldId`, not by an
arbitrary string, so the field set is closed and schema-checked.

### 4.3 Ownership: snapshot in, commands out

A script never receives `&mut World`. Per phase:

```
ScriptSnapshot  →  invoke()  →  CommandBuffer
 fixed/sim time                  SetField / SetTransform
 input snapshot                  AddComponent / RemoveComponent
 self entity + handle            Spawn / Despawn
 readable component copies       ApplyForce / ApplyImpulse
 queued events (seq-numbered)    PlayAudio / EmitEvent / Log
```

Commands are validated (types against the schema, NaN/Inf rejected wherever a
value reaches physics or serialization, entity handles re-checked for
generation) and committed at a documented phase boundary. Structural mutation
therefore cannot invalidate a live archetype iteration, and a stale handle
returns a typed script error instead of panicking.

Reads see the caller's own pending writes through an overlay; writes from one
script are invisible to another until commit. That rule is what makes the same
gameplay code valid later on parallel VMs, and it must be documented in the
scripting help page, not just implemented.

### 4.4 Attachment data versus runtime state

Only authored data lives in the ECS:

```rust
pub struct ScriptSet {                 // one component on the entity
    pub attachments: Vec<ScriptAttachment>,
}
pub struct ScriptAttachment {
    pub instance: InstanceUuid,        // persistent, survives save/load/reload
    pub asset: ScriptAssetId,          // content-hash-stable asset uuid
    pub enabled: bool,
    pub execution_order: i32,
    pub properties: PropertyBag,       // typed, schema-versioned
    pub schema_version: u32,
    pub api_version: u32,
}
```

VM state is held by `ScriptRuntime`, keyed by `InstanceUuid`. An ECS entity
index is never identity, and a VM pointer is never in a component. Save games
may additionally persist a script's explicitly returned pure-data state; a
closure, a coroutine or userdata is never durable.

### 4.5 Determinism

Phase 16 promises **deterministic engine scheduling on the same build and
platform** and nothing stronger — Jolt and float behaviour across platforms are
unaudited and the claim is not made. Fixed-step scripts get simulation time and
fixed `dt` only; an engine-owned seeded RNG stream per (world, entity,
attachment); no wall clock, OS entropy, filesystem or network; sequence-numbered
events; documented stable sort order on every query; deterministic conflict
rules on commands. Ordering is always

```
(execution_order, persistent_entity_guid, attachment_instance_uuid)
```

and never archetype traversal order, `ComponentId` order, hash-map iteration or
directory enumeration order.

### 4.6 Threat model

Straight from the research; reproduced here because it is the acceptance list
for 16-F, not background reading.

| Threat | Control |
|---|---|
| Infinite loop / runaway recursion | Luau interrupt on a deadline; call-depth cap; offending instance disabled |
| Allocation bomb | Per-VM and per-instance memory ceiling; bounded string/table/state depth |
| Filesystem / process / network | Those libraries are never opened; capability allowlist only |
| External bytecode | Compile trusted source with the embedded compiler; reject precompiled input |
| Stale entity handle | Opaque index+generation handle, validated every call |
| Archetype invalidation / reentrancy | Snapshot reads, deferred commands |
| Rust panic across FFI | `catch_unwind` at the boundary, converted to a script error |
| Entity despawns itself mid-callback | Mark pending-destroy, finish the callback, tear down at the safe point |
| Reload resource leak | Ownership tokens on tasks/events/audio; 100-cycle stress test |
| Global contamination | `Lua::sandbox(true)` plus a sandboxed thread and environment per attachment |
| Trusted vs untrusted | Separate VMs per trust domain; mods do not share the project VM |
| One script starves the rest | Per-instance *and* per-frame budgets |
| Host call bypasses the VM budget | Host functions are budget-aware; an unbounded Rust call behind a cheap opcode is a bug |
| ABI drift | `api_version` on every attachment, generated declarations, migration layer |

---

## 5. Session one — foundation and runtime (16-A, 16-B, 16-C)

**Everything in session one is provable with `cargo test` and the
`hello_engine` demo. No editor UI, no file watching, no cooking.**

### 16-A — Foundation, no VM

`mlua` must not appear in any `Cargo.toml` during 16-A.

**A-1. Runtime component add/remove with archetype migration.**
`crates/somnium_ecs/src/world.rs` and `archetype.rs`:

```rust
impl World {
    pub fn insert_component<T: Component>(&mut self, e: Entity, value: T) -> Result<(), EcsError>;
    pub fn remove_component<T: Component>(&mut self, e: Entity) -> Result<bool, EcsError>;
    // type-erased forms, for the registry and for scripts:
    pub fn insert_erased(&mut self, e: Entity, info: &ComponentInfo, src: *mut u8) -> Result<(), EcsError>;
    pub fn remove_erased(&mut self, e: Entity, id: ComponentId) -> Result<bool, EcsError>;
    pub fn has_component(&self, e: Entity, id: ComponentId) -> bool;
}
```

Migration moves the row to the archetype whose `ComponentSet` is
`old.with(id)` / `old.without(id)`, byte-moving each shared column and
`swap_remove`-ing the source row, exactly as `despawn` already does — including
patching the swapped-in entity's location. `drop_fn` must run for a removed
component and must **not** run for a moved one; that distinction is where this
kind of code goes wrong and there is a test for it.

**A-2. Durable component schemas** — new `crates/somnium_ecs/src/reflect.rs`:

```rust
pub struct ComponentSchema {
    pub stable_id: StableId,          // stable string name, hashed to u64 for storage
    pub display_name: &'static str,
    pub version: u32,
    pub fields: Vec<FieldSchema>,     // name, FieldId, FieldType, default, range, flags
    pub runtime_id: fn() -> ComponentId,
    pub snapshot: fn(&World, Entity, &mut ValueWriter),
    pub apply_patch: fn(&mut World, Entity, &Patch) -> Result<(), PatchError>,
    pub serialize: fn(&World, Entity) -> Option<serde_json::Value>,
    pub deserialize: fn(&mut World, Entity, &serde_json::Value, u32) -> Result<(), MigrateError>,
}
pub struct TypeRegistry { /* stable_id → schema, runtime ComponentId → schema */ }
```

`somnium_core` registers `Transform`, `WorldTransform`, `Name`,
`LightComponent`, `MeshKind`, `Parent`, `Children`, `TerrainComponent`,
`WaterComponent` and the new `ScriptSet`. **`ComponentId` stays exactly as it
is** — process-local, lazy, fast. `StableId` is the durable name beside it. Two
identifiers with two jobs; conflating them is the bug the research identified.

**A-3. Persistent identity.** A `PersistentId(u128)` component minted on
editor-spawn and on scene load, serialized, and used as the durable entity name
in every script-facing and file-facing context. `Entity`'s index+generation
stays the runtime handle.

**A-4. Neutral value and command model** — new crate `somnium_script`:
`ScriptValue`, `PropertyBag`, `FieldId`, `ScriptSnapshot`, `ScriptCommand`,
`CommandBuffer`, `ScriptError`, `Diagnostics`, `ScriptInstanceId`,
`ScriptAssetId`, `LanguageTag`, `Budget`, and the `ScriptBackend` trait. Plus
the command **applier** (in `somnium_core`, since it needs `PhysicsWorld` and
`AudioEngine`) with validation and deterministic conflict resolution.

**A-5. Scene serialization v2.** `scene_serial.rs` gains a schema-driven path:
version 2 writes `persistent_id`, a `components` object keyed by `StableId` with
per-component `version`, and the `ScriptSet` attachments with their typed
properties. **The version-1 reader stays** and is exercised by a test — existing
`.somnium` files must keep opening. Terrain and water keep their sidecar
arrangement; only the walk becomes registry-driven.

**A-6. Tests** (`crates/somnium_ecs/tests/`, `crates/somnium_script/tests/`):
archetype migration in both directions with mixed `Copy`/non-`Copy` columns and
a drop-counter; insert-then-remove round trip leaves the original archetype;
migration of the last row in an archetype; stale-handle insert returns
`EcsError::Dead`; component snapshot → patch → snapshot is idempotent; schema
migration from version N-1 to N; command-conflict resolution is order-stable
across 1,000 shuffles; scene v1 loads, v2 round-trips attachments and exported
fields byte-for-byte.

**Gate 16-A.** `cargo test --workspace` green. No scripting-language types
anywhere. A scene with a `ScriptSet` referencing an asset that does not exist
still loads, reports the missing asset, and re-saves without losing the
attachment.

---

### 16-B — The Luau adapter

Creates `dev records/phase 16/` with the first benchmark table.

**B-1. Pin the runtime.** `mlua = { version = "=0.12.x", features = ["luau",
"serde", "send", "vendored"] }` in the workspace `[workspace.dependencies]`,
exact-pinned. Luau is C++; the MSVC toolchain is already required for Jolt, so
this adds build *time*, not a new prerequisite. Record the measured clean-build
delta in the phase folder — if it is severe, that is information the ADR needs.

**B-2. `somnium_script_luau`.** One `Lua` state per **trust domain** (project
scripts and, later, mods), not one global VM and not one per instance:
`Lua::sandbox(true)` globally, `luaL_sandboxthread`-equivalent per attachment so
each has isolated globals; safe standard libraries only (`io`, `os`, `debug`,
`package.loadlib` never opened — note that O3DE calls `luaL_openlibs` first and
then removes things, which is partial hardening and is *not* what we do);
`set_memory_limit`; `set_interrupt` driving the deadline controller; memory
categories per asset and per instance.

**B-3. `require` is asset-only.** A custom searcher that resolves module names
against the content root's `.luau` assets, with cycle detection and a static
dependency graph built at compile time. No filesystem paths, no native
libraries. The dependency graph is what session two's reload uses to decide the
blast radius of an edit.

**B-4. Cached callables.** `on_init`, `on_start`, `on_update`,
`on_fixed_update`, `on_event`, `on_enable`, `on_disable`, `on_destroy`,
`save_state`, `load_state`, `migrate_state` are resolved **once** at
instantiation into registry-held handles. Falco's per-call `MonoMethodDesc`
search is the named anti-pattern; nothing in the hot path may look up a callback
by string.

**B-5. Host bindings, generated from the registry.** The `TypeRegistry` from
16-A drives both the userdata methods and (in session two) the `.d.luau`
declarations. Hand-registering a surface per component — Wicked's 24
`*_BindLua.cpp` files, Falco's 66 managed API files — is the failure mode this
design exists to avoid.

**B-6. Error containment.** `catch_unwind` at the FFI boundary; a Rust panic
becomes a script error, never an unwind across C++. Script errors carry asset,
line, column and a traceback with source locations.

**B-7. The benchmark harness** (`crates/somnium_script_luau/benches/` run as a
release test that prints a table into `dev records/phase 16/16-B_budgets.md`).
The research's proposed budgets become the gates, on the RTX 5080 laptop:

| Measurement | Budget |
|---|---|
| 1,000 empty lifecycle callbacks | p95 < 0.5 ms total |
| 10,000 component snapshot reads + 10,000 queued writes | p95 < 1.5 ms |
| 1,000 representative scripted entities at 60 Hz | total script p95 < 2.0 ms |
| GC tail, 5-minute allocation workload | p99 individual pause < 0.5 ms |
| Infinite loop interrupted and isolated | within 2 ms of its deadline |
| Compile + check + instantiate a 1,000-line asset | p95 < 250 ms |
| 100 instantiate/teardown cycles | no live-instance growth, < 1 MiB retained |
| Fixed-step replay, 10,000 steps | identical state hashes across runs |
| Malformed-source / stale-handle fuzz corpus | no panic, no memory error |

A missed budget is recorded, not hidden. Two or more missed by a wide margin is
the falsification signal for the language choice, and §3.1 explains why the
architecture makes acting on it cheap.

**Gate 16-B.** All budgets recorded with real numbers. A `.luau` module
compiles, `describe`s its fields, instantiates, and `invoke`s a callback that
emits a command — driven from a headless test with no editor and no window.

---

### 16-C — Lifecycle, scheduler, ECS productionization

**C-1. The state machine**, explicit and enforced:

```
Loaded → Initialized → Started → Enabled ⇄ Disabled → Destroyed
```

**C-2. Bounded init/start fixed point.** Scripts spawned during initialization
are initialized in the same frame, iterated to a fixed point, **capped at 64
cycles** with the offending spawn chain reported. This is Fyrox's semantic and
it is copied deliberately: it is the difference between a prefab bug and a hang.

**C-3. Deferred destruction.** An entity that despawns itself finishes its
callback; teardown happens at the safe point. Event subscriptions are
unregistered *before* deinitialization, again following Fyrox.

**C-4. Phases wired into the frame loop.** `on_fixed_update` runs inside the
existing accumulator loop in [`app.rs:961`](../crates/somnium_core/src/app.rs),
before `physics.step` — the research is right that this is already the correct
deterministic hook, and the loop needs no restructuring. `on_update` runs in the
variable-rate phase. Commands commit at the end of each phase, before physics
for the fixed phase.

**C-5. Error quarantine.** One failed attachment disables itself after a
configurable failure threshold and logs; it never stops the world, never stops
its peers, and never leaves a half-applied command batch.

**C-6. Ownership tokens.** Every task, event subscription, audio handle and
engine resource a script acquires is tagged with its `ScriptInstanceId`, so
teardown is complete by construction rather than by discipline.

**C-7. The reload halves.** `export_state` / `import_state` / `migrate_state`
implemented and proven by an in-process module swap test — same instance uuid,
new module, declared state migrated, no filesystem involved. Session two adds
the watcher and the transaction around it.

**Gate 16-C / end of session one.** `hello_engine` runs a `.luau` script
attached to an entity in code (no UI yet) that rotates it at fixed step, reads
input, applies a force, spawns and despawns, emits and receives an event, and
persists its exported fields through a save/load cycle. A script with a `while
true do end` is interrupted and quarantined; the frame after it is normal.
`cargo test --workspace` green. `context.md` §7.1 corrected, §17 updated,
`ATTRIBUTION.md` §13D written, `THIRD_PARTY_NOTICES.md` carries mlua and Luau
(both MIT).

---

## 6. Session two — editor, reload, shipping (16-D, 16-E, 16-F)

### 16-D — Editor workflow

- **`.luau` as content.** Importer, stable asset uuid, content-drawer entry with
  an icon, create-new-script from the drawer with a strict-mode template
  (`--!strict`, `Script.define`, an empty `onFixedUpdate`).
- **Details panel attachment UI.** Attach, remove, reorder, enable/disable.
  Exported fields **generated from the script's schema** — this is the first
  real consumer of registry-driven field UI, and it is deliberately built so
  26-J can adopt the same code path rather than a second one.
- **Undo.** Attach/remove/reorder/property-edit are `EditorCommand`s on the
  existing `UndoStack`, with the live-scrub convention already used by
  `SetInspectorValue` (drag is live and unrecorded; the gesture's final value is
  one undo step).
- **Diagnostics.** Compiler, type and lint output with asset, line and column,
  surfaced in the Output Log with clickable source locations, plus a script
  error count in the status area.
- **Play/stop world separation.** Stop restores the authored world exactly;
  scripts must not be able to dirty the edit-time scene. This interacts with the
  existing `SimulationState::{Editing, Playing, Paused}` transport and must not
  change its meaning.
- **Help page.** `docs/editor/scripting.md` alongside the existing editor docs,
  covering the lifecycle, the snapshot/command visibility rule, determinism
  constraints, and what does and does not survive a reload.

### 16-E — Transactional hot reload

Exactly the research's order, and the failure paths are the point:

1. Debounce the file change; compute the affected dependents from the static
   dependency graph.
2. Compile and typecheck the new module graph in a shadow environment,
   off the simulation thread.
3. **On failure: keep the old live instances running** and publish diagnostics.
   Nothing about the running world changes.
4. Ask old instances for declared state.
5. `on_disable`, cancel owned tasks and coroutines, drop subscriptions, discard
   every VM reference.
6. Instantiate the new module.
7. Validate the exported-property schema; call migration if the version changed.
8. `load_state`, `on_init`, `on_start`, `on_enable`.
9. Commit at a frame boundary; retain the previous compiled graph for one
   rollback generation.

Coroutines, closures and userdata do not survive. Only versioned pure data does.
**Tests:** 100 reload cycles with no instance or memory growth; a reload with a
deliberate syntax error leaves the old instance running; a renamed field
migrates; a removed field warns and drops; a changed callback signature is
rejected with a diagnostic rather than a runtime error at frame 400.

### 16-F — Cook, sandbox hardening, and the gates

- **Cook.** Compile source to bytecode at cook time, **bound to a runtime
  fingerprint** (mlua version + Luau version + compiler options). Luau's own
  `Bytecode.h` states indefinite backward compatibility is not provided, so
  bytecode is a cache, never durable storage; a fingerprint mismatch recooks
  from source. Development keeps the source path.
- **Capability manifest.** Per script package: which engine capabilities it may
  call. The default set for project scripts is generous; the default for a
  future mod tier is nearly empty.
- **Adversarial suite.** Every row of §4.6 gets a test that tries to break it,
  including a fuzz corpus of malformed sources and a stale-handle storm.
- **Profiler rows.** Script CPU time, call count, allocations and error count as
  named zones in the Phase 29 overlay. `break-on-error` is explicitly *not*
  claimed at MVP.
- **Generated declarations.** `.d.luau` for the Somnium API, emitted from the
  registry, and a smoke test that `luau-analyze` accepts a strict-mode template
  script against them. Note honestly in the help page that the common LSP
  frontend is community-maintained, not an official Luau tool — do not claim
  first-class IDE support.
- **Packaging smoke tests** on Windows primary, Linux secondary. Third-party
  licence attribution complete.

---

## 7. What is explicitly deferred

| Item | Why | Where it goes |
|---|---|---|
| Rhai / Rune second backend | §3.1; the trait makes it cheap later | Only if 16-B budgets fail |
| Editor debugger (breakpoints, stepping) | Separate sub-phase, needs the VM's breakpoint machinery and an editor UI | 16-G |
| Wasmtime mod tier | Needs a real hostile-mod requirement first | 16-H |
| Luau native codegen / JIT | Interpreter first, for portability and a smaller security surface | Post-16, gated on a benchmark that proves script execution — not boundary traffic — is the bottleneck |
| Visual scripting | Must consume a *stable* reflected API; O3DE shows the size | Post-16 |
| Parallel script workers | The snapshot/command design permits it without changing gameplay APIs | Post-16 |
| C# | Only if Unity-like creator UX becomes a top product requirement | Not planned |

---

## 8. Risks

1. **`mlua` + vendored Luau lengthens the build.** Jolt already sets the
   precedent for C++ in the build, but this is a second one. Measured in B-1; if
   it is bad, the mitigation is a `luau` feature that CI builds and the ordinary
   dev loop can skip.
2. **16-A is bigger than it looks.** Archetype migration touches the most
   safety-critical unsafe code in the engine. This is why it is first, why it is
   test-heavy, and why the VM is forbidden until it is done.
3. **Scene v2 is a compatibility surface.** The v1 reader stays and stays
   tested. A phase that silently breaks existing `.somnium` files has failed
   regardless of what else it shipped.
4. **The registry could grow a second source of truth.** If session two starts
   hand-writing script field UI because the generated path is awkward, the
   phase's fourth goal is lost. Hand-written per-component script UI is a review
   failure, not a shortcut.
5. **Determinism claims outrunning evidence.** The promise is same-build,
   same-platform. Anything stronger needs a Jolt and float-behaviour audit that
   this phase does not perform.

---

## 9. Session boundary — what must be true to hand off

At the end of session one, a reader who has never seen this plan must be able
to open `dev records/phase 16/` and `context.md` §17 and learn: the language
decision and its pinned versions; that the foundation is in `somnium_ecs` and
`somnium_script`; that the only Luau-aware crate is `somnium_script_luau`; the
measured budget table with real numbers; which of 16-D/E/F remain; and the exact
`hello_engine` invocation that runs a scripted entity. No editor UI exists yet
and the handoff must say so plainly rather than implying a finished feature.

---

## 10. Status — end of session one (2026-08-16)

**16-A is complete and gated. 16-B and 16-C are not started.** The plan
in §5 assumed all three would fit one session. They did not, and the
honest reason is that 16-A was underestimated rather than that anything
went wrong: the foundation turned out to be five separable pieces, four
of which needed their own test suite.

`mlua` is still absent from every `Cargo.toml`, which was 16-A's own
precondition and is now simply where the work stopped.

### What is in the tree

| Piece | Where | Tests |
|---|---|---|
| Runtime component insert/remove with archetype migration | [`world.rs`](../crates/somnium_ecs/src/world.rs), [`archetype.rs`](../crates/somnium_ecs/src/archetype.rs) | 15, in [`archetype_migration.rs`](../crates/somnium_ecs/tests/archetype_migration.rs) |
| Durable component schemas + `component_schema!` | [`reflect.rs`](../crates/somnium_ecs/src/reflect.rs) | 13 |
| Durable entity identity | [`persistent.rs`](../crates/somnium_ecs/src/persistent.rs) | 6 |
| Neutral scripting contract | [`somnium_script`](../crates/somnium_script/) | 31 |
| Built-in component registration | [`reflect_registry.rs`](../crates/somnium_core/src/reflect_registry.rs) | 11 |
| `WorldView` + command applier | [`script_bridge.rs`](../crates/somnium_core/src/script_bridge.rs) | 16 |
| Schema-driven scene v2 | [`scene_serial_v2.rs`](../crates/somnium_core/src/scene_serial_v2.rs) | 15 |

`cargo test --workspace`: **620 passed, 0 failed** (107 of them new). The three new
`somnium_core` modules and all of `somnium_script` are clippy-clean under
`pedantic`; `somnium_ecs` is back to its two pre-existing warnings.

### Gate 16-A — assessed

- **"No scripting-engine types appear outside the backend adapter."**
  Met, trivially: there is no adapter and no scripting engine yet.
  `somnium_script`'s only dependency is `somnium_ecs`.
- **"Scene round-trip preserves attachments and exported fields."** Met.
  A `ScriptSet` with two attachments, four typed properties including an
  entity reference, a negative execution order and a non-default schema
  version round-trips exactly, and the entity reference is remapped
  through `PersistentId` — proven by loading into a world that is already
  populated so the ECS indices genuinely differ.
- **"A scene with a `ScriptSet` referencing a missing asset still loads,
  reports it, and re-saves without losing the attachment."** Met.

### Two things the plan said that turned out differently

1. **Scene v2 is additive, not the default.** `save_scene` still writes
   version 1. Switching the default requires registering the components
   v1 hand-writes that the registry does not cover yet — `Water`
   (~40 fields), `Foliage`, `PostProcess`, `CameraSettings`,
   `VoxelTerrain`, `ParticleEmitter`, `BuoyantVessel`, `Children`. That
   work is mechanical macro blocks, but flipping the default before it is
   done would silently drop water from every save, so it is not flipped.
   **This is the first task of the next session**, not an optional
   cleanup — leaving two serializers alive is exactly the second source
   of truth §2 goal 4 forbids.
2. **`somnium_ecs` gained a `glam` dependency.** The orphan rule requires
   `ReflectField for glam::Vec3` to live in whichever crate owns the
   trait, and the ECS already stores glam-typed components in practice.
   `context.md` §4's "no external deps beyond std" was already inaccurate
   (`rayon`), and is now more so.

### Revised session boundary

The two-session shape in §5–§6 does not survive contact. The realistic
split from here:

- **Session two: 16-B + 16-C.** Register the remaining components and
  flip scene v2 on; then `mlua`, the Luau adapter, the budget table, the
  lifecycle state machine and scheduler, and the `hello_engine` gate.
- **Session three: 16-D + 16-E + 16-F.** Editor workflow, transactional
  hot reload, cook and sandbox hardening.

Phase 16 is a three-session phase. Saying so now costs less than
discovering it in session two.

---

## 11. Status — end of session two (2026-08-16)

**16-B is complete. 16-C is not started.** The precondition from §10 —
register the remaining components and stop writing two scene formats — is
done, and turned up a defect in what §10 shipped.

### 11.0 A defect in 16-A, found and fixed

`.somnium` files use `version` as a **format tag, not a revision**:

| `version` | Format | Written by |
|---|---|---|
| 1 | Hand-written entity dump | `scene_serial` |
| 2 | Map recipe — a factory `kind`, no entities | `map` |
| 3 | Schema-driven entity dump | `scene_schema` |

Session one numbered the schema-driven dump **2**, colliding with the map
recipe. It is now 3, the module is `scene_schema` rather than
`scene_serial_v2` (a name that implied a revision it is not), and
`the_three_somnium_formats_are_mutually_exclusive` is the regression test.

A second thing worth writing down, found while fixing it: **a saved
`scene.somnium` was never loadable by the editor.** `EditorEvent::LoadScene`
routes to `map::load_map`, which only accepts version-2 map recipes, so the
version-1 entity dump was write-only. That is pre-existing, not new, and
making an entity dump load needs GPU-side reconstruction — meshes from
`MeshKind`, terrain sidecars, renderer uploads — which belongs with 16-D.

### 11.1 Registration and the format flip

`Water` (40 fields), `Foliage` and `VoxelTerrain` are registered, taking
the registry to **11 schemas**. `SaveScene` now writes the schema format.
Water round-trips **field for field** against the value the hand-written
walk produced; foliage and voxel terrain now survive a save, which they
never did before — a side effect of describing them once.

`PostProcess` and `CameraSettings` remain unregistered on purpose: their
defaults read environment variables, so making them saveable changes what
loading a scene *means*. That is a product decision, not a mechanical one.

### 11.2 The Luau adapter

`somnium_script_luau` — the only crate that names `mlua`. `mlua` 0.12.0 /
Luau 0.728, interpreter only, exact-pinned. Clean build including the
vendored C++ runtime: **38 s**.

- One VM per trust domain; engine API installed *before* `sandbox(true)`,
  which is what freezes it.
- A private environment table per attachment, chained to the frozen
  globals.
- Callbacks resolved once at instantiation into a cached array.
- Bytecode compiled once by the embedded compiler and reused per instance;
  external bytecode is never loaded.
- Wall-clock deadline through the Luau interrupt, amortised so the clock
  is read once per 1,024 interrupts.

**Tests: 47** — 21 vertical slice, 6 sandbox, 5 budget, 15 unit.

### 11.3 Sandbox holes that were open, and are not now

`StdLib::ALL_SAFE` is `u32::MAX` under the `luau` feature, so `Lua::new()`
hands scripts `os` and `debug`. That much was anticipated. What was not:
even with the library flags right, the **base library still arrives
carrying** `getfenv`, `setfenv`, `loadstring`, `require`, `collectgarbage`,
`gcinfo`, `print` and `_G`.

`getfenv`/`setfenv` are the serious ones — they reach and rewrite another
function's environment, which is the entire mechanism keeping one
attachment's globals private. All eight are removed before sandboxing, and
`tests/sandbox.rs` enumerates the surviving surface so a Luau upgrade
cannot widen it silently.

### 11.4 The budgets — measured, then fixed

Full table, variance data and analysis:
[`phase 16/16-B_budgets.md`](phase%2016/16-B_budgets.md).

The first pass missed three of four throughput ceilings. Four defects were
found by measurement and fixed:

| Defect | Effect |
|---|---|
| Context built per callback, not per phase | 14.32 → 0.92 ms |
| Entity userdata reallocated every call | 0.92 → 0.74 ms |
| Uninterned `&str` table keys (`raw_get` 177 ns → 43 ns) | across the board |
| `ctx:get(entity, "component", "field")` re-resolving per access | reads+writes 13.7 → 2.7 ms |

Result, median of five release runs:

| Measurement | Before | After | Ceiling |
|---|---|---|---|
| 1,000 empty lifecycle callbacks | 14.32 ms | **0.52 ms** | 0.5 |
| 10,000 reads + writes (1,000 × 10) | 13.67 ms | **2.68 ms** | 1.5 |
| 1,000 representative entities @ 60 Hz | 2.35 ms | **1.54 ms** | 2.0 ✓ |
| compile + check + instantiate 1,000 lines | 0.99 ms | **0.79 ms** | 250 ✓ |

Every safety gate — interrupt latency, leakage, determinism, fuzz — passes
with margin, and instance retention improved from 48 KiB to 16 KiB.

**The remaining row is over a ceiling that is unreachable.** The control
measurement: 10,000 entities running a callback that does *nothing*, with
no mirror, costs **6.5 ms** — against a 1.5 ms budget for the same 10,000
entities doing a read and a write each. The ceiling implies 150 ns per
callback and the Luau call alone is 116 ns. It was written before there
was a per-callback cost model; §4 of the budgets record proposes one.

**None of this falsified the language choice.** Luau calls in 116 ns,
constructs a vector in 32 ns, compiles 250× inside its ceiling. Every
defect was in engine code, and would have cost the same or more in any
other runtime.

### 11.4b Mirrored properties, and a correctness bug they fixed

The API a script now uses for its own components:

```luau
uses = { ["somnium.Transform"] = { "translation" } },
onFixedUpdate = function(self, ctx, dt)
    local t = ctx.self.transform
    t.translation = t.translation + step
end,
```

Declared components are written into a plain Luau table before the call
and diffed out after, so field access is a table lookup rather than a host
call. Naming the *fields* matters: mirroring all of `Transform` made the
representative row **worse** (2.31 → 5.53 ms), because a `rotation`
quaternion marshals as a four-entry table in both directions every frame
for a script that never reads it.

This also fixed a real defect. `ctx:get` reads committed world state and
`ctx:set` queues a deferred write, so a read-modify-write loop through
that pair **re-read the pre-phase value every iteration and only the last
write survived** — ten steps silently produced one step of movement.
Through the mirror a script sees its own writes, which is the visibility
rule §4.3 documents. `a_read_modify_write_loop_accumulates_through_the_mirror`
is the regression test.

### 11.4c One implementation, not two

`invoke` and `invoke_phase` had drifted: the mirror existed only in the
phase path, so the same script behaved differently depending on which the
caller used. `invoke_phase` is now the only required trait method and
`invoke` is sugar over it with a single call.

That exposed a semantic question the two paths had been answering
differently, now decided and documented: **a module that does not define a
callback is a silent no-op** (normal — it is what `CallbackMask` is for,
and erroring would fill the log every frame), while **a missing instance
is a reported failure** (a bug or a stale id surviving a reload).

### 11.5 State of the tree

`cargo test --workspace`: **672 passed, 0 failed.** `somnium_script`,
`somnium_script_luau` and the three new `somnium_core` modules are
clippy-clean under `pedantic`.

### 11.6 Next session — 16-C

Property accessors are **done** (§11.4b), so 16-C starts on the lifecycle
rather than on performance.

1. Lifecycle state machine; bounded init/start fixed point capped at 64,
   after Fyrox; deferred destruction; ownership tokens.
3. The scheduler: order by `(execution_order, persistent_entity_guid,
   attachment_instance_uuid)`, drive `invoke_phase` from
   `app.rs`'s existing fixed-step loop before `physics.step`.
4. Error quarantine end to end.
5. The `hello_engine` gate.
