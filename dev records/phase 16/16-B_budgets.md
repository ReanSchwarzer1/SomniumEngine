# 16-B — measured budgets

> **Run:** 2026-08-16, `cargo test -p somnium_script_luau --release --test budgets -- --nocapture`
> **Machine:** RTX 5080 Laptop development machine, Windows 11, rustc 1.88.
> **Runtime:** `mlua` 0.12.0 / `luau0-src` 0.20.7 (**Luau 0.728**), interpreter only, no native codegen.
> **Build:** clean workspace build including vendored Luau — **38 s**. Luau is
> C++, but the MSVC toolchain was already required for Jolt, so this is
> build time rather than a new prerequisite.

**Read the variance section before trusting any single number.** The
harness has roughly ±15% run-to-run spread, and two conclusions in the
first draft of this document turned out to be noise.

A run on different hardware is a different measurement and belongs in its
own section, not silently replacing this one.

---

## 1. The table

Median of five consecutive release runs.

| Measurement | Before | **After** | Ceiling | Verdict |
|---|---|---|---|---|
| 1,000 empty lifecycle callbacks | 14.32 ms | **0.52 ms** | 0.5 ms | ~4% over |
| 10,000 reads + writes, as 1,000 entities × 10 | 13.67 ms | **2.68 ms** | 1.5 ms | 1.8× over |
| …the same budget, as 10,000 entities × 1 | — | **22.4 ms** | 1.5 ms | see §4 |
| 1,000 representative scripted entities @ 60 Hz | 2.35 ms | **1.54 ms** | 2.0 ms | **PASS** |
| compile + check + instantiate a 1,000-line asset | 0.99 ms | **0.79 ms** | 250 ms | **PASS** |
| Infinite loop interrupted past its deadline | 0.03 ms | **0.05 ms** | 2 ms | **PASS** |
| 100 instantiate/teardown cycles, retained | 48 KiB | **16 KiB** | < 1 MiB | **PASS** |
| Live instances after teardown | 0 | **0** | 0 | **PASS** |
| Fixed-step replay, identical state hash | identical | **identical** | identical | **PASS** |
| Malformed-source corpus (18 cases) | clean | **clean** | none | **PASS** |

Headline: **empty callbacks 27× faster, reads+writes 5× faster,
representative workload now inside its budget.** Every safety gate passes
with a wide margin.

---

## 2. Variance, and two wrong conclusions it caused

Five runs of the unchanged harness:

| Row | Runs | Median | Spread |
|---|---|---|---|
| empty callbacks | 0.515, 0.480, 0.591, 0.446, 0.569 | 0.515 | ±15% |
| reads+writes (1,000 × 10) | 2.612, 2.776, 2.756, 2.684, 2.436 | 2.684 | ±7% |
| representative | 1.539, 1.541, 1.682, 1.687, 1.388 | 1.541 | ±11% |

Two things were briefly believed on the strength of a single run and are
false:

1. *"Hoisting `local t = ctx.self.transform` made it slower."* It did not;
   the row moved within its own noise band, and the empty-callback row —
   which the change could not possibly affect — moved 35% in the same
   pair of runs.
2. *"Representative regressed to 5.5 ms after mirroring."* That one was
   real, and §3.3 explains it, but the magnitude was overstated by a run
   that happened to land high.

The lesson is in the harness now: the table prints a p95 over 40–100
samples per row, and any judgement uses the median of several runs.

---

## 3. What was actually wrong, in the order it was found

Four defects. Each was isolated by measurement; two hypotheses that
sounded obvious were measured and found wrong, and are recorded so nobody
re-litigates them.

### 3.1 The context was built per callback instead of per phase

A dozen scoped host closures at ~0.67 µs each, rebuilt for every
attachment. `ScriptBackend::invoke_phase` builds them once per phase.

**14.32 ms → 0.92 ms.**

### 3.2 A fresh entity userdata per call

`ctx.entity` was re-wrapped every callback for an entity that cannot
change during an attachment's life. Cached on the instance.

**0.92 ms → 0.74 ms.**

### 3.3 Uninterned string keys

The one that mattered most, and the least obvious. A `&str` key makes Lua
intern the string on every access:

| Operation | `&str` key | cached `LuaString` key |
|---|---|---|
| `Table::raw_set` | 230 ns | **100 ns** |
| `Table::raw_get` | 177 ns | **43 ns** |
| `Table::set` (protected) | 264 ns | — |

Every per-call key is now pre-interned once. This is also what made the
mirror in §3.4 affordable rather than a pessimisation.

### 3.4 The API shape: `ctx:get(entity, "component", "field")`

Argument marshalling, measured directly:

| Host call shape | Cost |
|---|---|
| `ctx.f()` | 30 ns |
| `ctx.f(i)` — one integer | 62 ns |
| `ctx:f(ctx.entity)` — receiver + userdata | 152 ns |
| `ctx:f(ctx.entity, "component", "field")` | 245 ns |
| the real typed signature | **276 ns** |

Against ~41 ns of actual engine work behind it. The call re-resolved, per
access, three things that never change: which entity, which component,
which field.

**The fix is mirrored properties.** A script declares what it touches:

```luau
uses = { ["somnium.Transform"] = { "translation" } },
onFixedUpdate = function(self, ctx, dt)
    local t = ctx.self.transform
    t.translation = t.translation + step
end,
```

The engine writes those fields into a plain Luau table before the call and
diffs them out after. Script-side access becomes a table lookup (~29 ns)
instead of a host call (~650 ns), and name resolution happens once per
attachment instead of once per access.

The declaration is load-bearing, and the first attempt proved why. With
`uses = { "somnium.Transform" }` — the whole component — the representative
row got **worse**, 2.31 → 5.53 ms, because `Transform` carries a `rotation`
quaternion that marshals as a four-entry table in *both* directions every
frame for a script that only ever touched `translation`. Naming the field
fixed it. Both spellings are supported; the whole-component form is the
convenient one and the expensive one, and the doc comment says so.

### 3.5 Two fixes that did nothing, recorded so they are not repeated

Replacing the whole-record `snapshot` read with a single-field
`read_field` on the schema, and removing two per-call `String`
allocations in favour of borrowed `mlua::String`. Together: 13.67 → 13.05
ms, about 4%. Both are still the right code and both stay. Neither was
the bottleneck.

---

## 4. The one row still over, and why the ceiling is unreachable

"10,000 component reads plus 10,000 queued writes, p95 under 1.5 ms" does
not say how those are distributed, so both readings were measured.

**As 1,000 entities × 10 each: 2.68 ms.** 1.8× over.

**As 10,000 entities × 1 each: 22.4 ms.** And here is the decisive
control — the same 10,000 entities running a callback that does *nothing*,
with no mirror at all:

```
  no mirror, callback empty      total 7.26 ms   invoke 6.51 ms   apply 0.00 ms
  mirror declared, callback empty total 15.3 ms  invoke 12.9 ms   apply 0.00 ms
```

**10,000 empty callbacks cost 6.5 ms before a single read or write
happens.** The ceiling is 1.5 ms. Under this reading the budget is
unreachable by a factor of four for *any* implementation in *any*
language, because it implies a per-callback cost of 150 ns and the Luau
call alone is 116 ns.

The budget was written before there was a per-callback cost model. It is
not being relaxed here — it is being reported against, with the control
measurement that shows what it actually demands. A revised ceiling should
be expressed as *cost per attachment per phase* plus *cost per field
access*, which is what the data above supports:

| Component | Measured |
|---|---|
| per attachment per phase, no mirror | ~0.55 µs |
| per mirrored field, in + out | ~0.64 µs |
| per script-side field read or write | ~0.03 µs |
| Luau function call | 0.12 µs |
| cheapest possible host call | 0.03 µs |

### Known, quantified, not done

`invoke_phase` does three `HashMap` lookups per call to reach the instance
(mirror-in, `self_table`, mirror-out). Hoisting them into the resolve pass
that already runs before the scope would save ~75–100 ns/call, about 12%
of the mirror overhead. It changes no verdict in this table, which is why
it was not done at the end of a long session; it is the first thing to do
if this row is revisited.

---

## 5. Does this falsify the language choice?

**No, and the numbers are what say so.**

The falsification criterion in `phase_16.md` §3.1 was that Luau — script
execution or the VM boundary — could not meet the frame budget. The
opposite is measured: Luau executes a call in 116 ns, constructs a native
vector in 32 ns, and the host-call trampoline is 30 ns. Compilation is
**250× inside** its ceiling, which is the number that governs editor
iteration speed. Every isolation and safety property holds with margin.

Every defect found was in engine code Somnium wrote, and every one of them
would have cost the same or more in Rhai, Rune or Wasm — more in Wasm,
where each argument crosses a sandbox boundary. Switching runtimes would
not have moved any of these numbers.

**Luau is fast. The first draft of the `ctx` API was not.** It is better
now, and the remaining gap is a documented property of the budget rather
than of the runtime.

---

## 6. Re-measured after 16-C–16-F (2026-08-17)

> **Run:** same machine, same harness, medians of five settled release
> runs each.

Re-running the table after three sessions of work showed every row 30–60%
worse than §1. That looked like a regression, and §2's warning about
single runs is exactly why it was checked rather than believed.

### 6.1 What the A/B actually showed

Three builds, measured back to back on the same afternoon, using
`git worktree` so the comparison was of code rather than of memory:

| Row | `1387962` (16-B) | `993a5dc` (16-C) | working tree (16-F) | §1 record |
|---|---|---|---|---|
| 1,000 empty callbacks | 0.671 | 0.787 | **0.485** | 0.515 |
| 10,000 reads + writes | 21.883 | 3.833 | **3.431** | 2.684 |
| 1,000 representative | 3.289 | 2.205 | **1.946** | 1.541 |

Two conclusions, in the order they matter:

1. **The working tree is faster than either committed build on every
   row.** Whatever the absolute numbers say, nothing in 16-C to 16-F made
   scripting slower.
2. **The machine is about 30% slower today than when §1 was written.**
   The `1387962` empty-callback row is 0.671 ms against §1's 0.515 ms for
   *the same code*. That is the scale factor, measured rather than
   assumed, and it accounts for the whole apparent regression: scaled by
   it, the working tree lands at ≈0.37 / ≈2.64 / ≈1.50 against §1's
   0.515 / 2.684 / 1.541.

The lesson §2 taught about single runs generalises: **a number is only
comparable to another number taken on the same machine on the same day.**
The table in §1 keeps its numbers; this section keeps its own, and neither
is rewritten to match the other.

`1387962` is *not* a pre-optimisation baseline, incidentally — its
reads+writes row at 21.9 ms shows it predates §3.4's mirrored properties
but not the other three fixes. It is here as a same-day control, not as a
history.

### 6.2 The two things that made it faster

Both were already written down as owed work.

**The three hash lookups per call, hoisted.** §4's "known, quantified, not
done" — `invoke_phase` reached the instance through `HashMap::get` three
times per call (mirror-in, `self_table`, mirror-out). The resolve pass now
does everything needing `&mut self`, the backend is reborrowed immutably,
and the loop holds `&Instance` directly. Estimated there at 75–100 ns per
call; measured here as most of the empty row's 0.787 → 0.485.

**`ctx.spawns` was rebound on every call.** 16-C added spawn-result
delivery and cleared the key per attachment per phase, including for the
overwhelmingly common case of nobody having spawned anything — a hash and
a write barrier per call to set nil over nil. It is now tracked and only
written when it changes. This is the same shape of mistake as §3.1 and
§3.2: per-call work that looks too small to be worth thinking about, on a
path that runs a thousand times a frame.

### 6.3 Still over, still for the reason §4 gives

`10,000 reads + writes` and the `10,000 entities × 1` reading remain over.
§4's arithmetic is unchanged and unchallenged: the ceiling implies 150 ns
per callback and the Luau call alone is 116 ns. Nothing in this session
was aimed at that row, and nothing in it moved.
