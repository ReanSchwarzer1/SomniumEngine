# 16-B — measured budgets

> **Run:** 2026-08-16, `cargo test -p somnium_script_luau --release --test budgets -- --nocapture`
> **Machine:** RTX 5080 Laptop development machine, Windows 11, rustc 1.88.
> **Runtime:** `mlua` 0.12.0 / `luau0-src` 0.20.7 (**Luau 0.728**), interpreter only, no native codegen.
> **Build:** clean workspace build including vendored Luau — **38 s**. Luau is
> C++, but the MSVC toolchain was already required for Jolt, so this is
> build time rather than a new prerequisite.

A run on different hardware is a different measurement and belongs in its
own section below, not silently replacing this one.

---

## 1. The table

| Measurement | Measured (p95) | Ceiling | Verdict |
|---|---|---|---|
| 1,000 empty lifecycle callbacks | **0.74 ms** | 0.5 ms | 1.5× over |
| 10,000 component reads + 10,000 queued writes | **13.05 ms** | 1.5 ms | 8.7× over |
| 1,000 representative scripted entities @ 60 Hz | **2.31 ms** | 2.0 ms | 1.16× over |
| compile + check + instantiate a 1,000-line asset | **0.99 ms** | 250 ms | **250× under** |
| Infinite loop interrupted past its deadline | **0.03–0.06 ms** | 2 ms | **PASS** |
| 100 instantiate/teardown cycles, retained | **32–48 KiB** | < 1 MiB | **PASS** |
| Live instances after teardown | **0** | 0 | **PASS** |
| Fixed-step replay, identical state hash | **identical** | identical | **PASS** |
| Malformed-source corpus (18 cases) | **no panic, no hang** | none | **PASS** |

Every **safety** budget passes with a wide margin. Three **throughput**
ceilings are missed. The rest of this document is about why, because the
number on its own would invite the wrong fix.

---

## 2. What the misses are actually measuring

The cause was isolated by measurement, not by inspection, and it is not
where the first two guesses put it.

### The Rust side is not the problem

| Operation | Cost |
|---|---|
| `EngineWorldView::read_field` (schema `read_field`, single field) | 30 ns |
| `component_by_name` (hash) | 11 ns |
| `field_by_name` | 16 ns |
| `is_field_writable` | 15 ns |
| `World::get::<Transform>` | 13 ns |

A whole `ctx:get` needs about **41 ns** of engine work. At 20,000 host
calls that is ~0.8 ms of the measured 13 ms.

### Luau is not the problem

| Operation | Cost |
|---|---|
| Luau function call, 2 table args | 116 ns |
| `vector.create(1,2,3)` | 32 ns |
| Host call, no arguments | 30 ns |
| Host call through `Lua::scope` | **31 ns** — *not* slower than a plain one |

### Argument marshalling is the problem

| Host call shape | Cost |
|---|---|
| `ctx.f()` | 30 ns |
| `ctx.f(i)` — one integer | 62 ns |
| `ctx:f(ctx.entity)` — receiver + userdata | 152 ns |
| `ctx:f(ctx.entity, "component", "field")` | 245 ns |
| the real typed signature `(Table, EntityHandle, String, String)` | **276 ns** |

So roughly **276 ns of every ~650 ns `ctx:get` is spent passing four
values**, of which the two string literals cost ~93 ns and the userdata
receiver ~90 ns. The engine work behind it is 41 ns.

**The API shape is the cost.** `ctx:get(entity, "somnium.Transform",
"translation")` re-resolves, per call, three things that do not change:
which entity (it is nearly always `self`), which component, and which
field.

---

## 3. What was already fixed, and what it bought

Two real defects were found and fixed while chasing this, both worth
keeping regardless of what happens next:

1. **Context was built per callback instead of per phase.** A dozen scoped
   host closures at ~0.67 µs each, rebuilt for every attachment. 1,000
   empty callbacks measured **14.3 ms**. Moving construction to the phase
   boundary — [`ScriptBackend::invoke_phase`] — took it to **0.92 ms**, a
   15.6× improvement, and is the shape the scheduler wanted anyway.
2. **A fresh `EntityHandle` userdata was allocated on every rebind**, for
   an entity that cannot change during an attachment's life. Caching it
   per instance: 0.92 ms → **0.74 ms**.

Two changes that were expected to help and **did not**, recorded so nobody
repeats them: replacing the whole-record `snapshot` read with a
single-field `read_field` on the schema, and removing the two per-call
`String` allocations in favour of borrowed `mlua::String`. Together they
moved 13.67 ms to 13.05 ms — about 4%. Both are still the right code and
both stay; neither was the bottleneck.

---

## 4. The remedy, and why it is not in this sub-phase

The fix is an API-shape change, not a tuning pass. The candidate is
**property accessors that pre-resolve what does not vary**:

```luau
-- today: 4 values marshalled per read, 5 per write
local p = ctx:get(ctx.entity, "somnium.Transform", "translation")
ctx:set(ctx.entity, "somnium.Transform", "translation", p + step)

-- candidate: component and entity resolved once
ctx.transform.translation += step
```

A proxy userdata bound to `(entity, component)` turns a read into one
metamethod call with one string argument — from 276 ns of marshalling to
roughly 60–100 ns — and removes the repeated name resolution entirely.

It is deliberately **not** done here, for a reason that is about
sequencing rather than effort: the accessor surface is what the editor's
generated field UI and the `.d.luau` declarations are written against.
Designing it under time pressure at the end of 16-B, then discovering in
16-D that the editor wants a different shape, is how a scripting API ends
up with two of everything. It belongs at the start of the next sub-phase,
with the declaration generator in view.

---

## 5. Does this falsify the language choice?

**No, and the numbers are what say so.**

The falsification criterion in `dev records/phase_16.md` §3.1 was that
Luau itself — script execution or the VM boundary — could not meet the
frame budget. What was measured is the opposite:

- Luau executes a call in **116 ns** and a native vector construct in
  **32 ns**;
- the host-call trampoline is **30 ns**;
- compilation is **250× inside** its ceiling, which is the number that
  governs editor iteration speed;
- every isolation and safety property holds with a wide margin.

The overrun is in an engine API surface that Somnium designed and can
redesign, and it would cost exactly the same in Rhai, Rune or Wasm —
arguably more in Wasm, where every argument crosses a sandbox boundary.
Switching runtimes would not move any of these numbers.

The honest summary: **Luau is fast; the first draft of the `ctx` API is
not.** The representative workload — the one that resembles a real game —
is 2.31 ms against 2.0 ms, 16% over, with a known fix. That is a
first-implementation result, not a verdict on the runtime.
