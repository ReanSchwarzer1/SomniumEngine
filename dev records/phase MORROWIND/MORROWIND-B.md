# MORROWIND-B — the job system (Seam 1)

**Complete, 2026-08-24**, with one interactive measurement owed and named below.
Track 0 (BALMORA). New crate `somnium_jobs`, zero dependencies.

## This is a promotion, not a second job system

Appendix A.6 of the plan settles the question before it can be argued:

> **CONTROL Seam 2 introduces a `JobRegistry` in `somnium_core`** … and
> **MORROWIND-B proposes `somnium_jobs`** … **These must not both exist.**

CONTROL made the move cheap on purpose. The first doc line of the file it wrote
reads *"The public surface is intentionally narrow so Phase MORROWIND can move
this module into `somnium_jobs` without changing call sites"*, and every call
site already passed a `&'static str` job name — the profiler-zone label A.3.1
makes mandatory and which A.6 warned would be *"tedious and always incomplete"*
to retrofit. Both asks were honoured, and the migration cost was one import line
and a type rename.

**`JobRegistry` no longer exists anywhere in the tree.** GHOSTFENCE's
`no-second-system` row was red on that symbol before this sub-phase and is green
after it, without the gate being edited.

### What was kept, exactly

`JobPriority`, `JobStatus`, `JobSnapshot`, `JobError`, `JobContext`,
`JobHandle<T>` and `submit(name, priority, task)` are CONTROL's, unchanged in
shape. **All five of CONTROL's tests moved across and still pass**, which is
what makes "the promotion preserved behaviour" a measurement rather than a
claim.

One judgement call worth recording: the plan's Seam 1 sketch names three
priorities (`Critical` / `Interactive` / `Background`) and CONTROL shipped four
(`User` / `Visible` / `Normal` / `Background`). **CONTROL's won.** They map onto
the sketch's three with `Normal` as the unremarkable default, they have working
call sites, and renaming them to match a sketch would have broken those call
sites in exchange for nothing. Seam 1's actual property — *the submitter
declares the class* — was already satisfied.

### What moved the other way

`submit_terrain_bake` and `submit_bc7_encode` **stayed in `somnium_core`**, as
an `EditorJobs` extension trait. They shell out to `cargo` inside this
workspace; `somnium_jobs` knows nothing about content roots or terrain packs and
should not. A scheduler that knows what it is scheduling has stopped being a
scheduler.

While moving them: **neither has a caller.** CONTROL wrote them against the
terrain and BC7 tools and the editor never grew the buttons. They are kept, with
that stated in the doc comment, because a two-method API waiting for a menu item
is not the same as rot — and because MORROWIND-A's census established that dead
code is a thing to *see*, not to delete on sight. If Track 4's cook replaces
both, delete them then.

## What MORROWIND-B added, and why each earns its place

### Deadlines, and the drop that makes them worth having

```rust
jobs.submit_with(JobDesc::new("cell.stream").within(Duration::from_millis(80)), work)
```

A job whose deadline has passed **while it was queued is discarded at pop time
and never handed to a worker**. This is the property that bounds streaming
thrash: the camera turns around, the cells it was going to enter are no longer
wanted, and the work evaporates instead of being done at leisure and thrown
away. A scheduler that runs everything eventually cannot be given a workload it
can drop.

The ordering rule sits **between** priority and FIFO, and the placement is the
design:

- *Above* priority, a far-future deadline on a background prefetch would
  outrank a user's import — exactly backwards, and
  `a_deadline_does_not_beat_a_priority` is the test that says so.
- *Below* FIFO, a deadline would affect nothing but whether a job is dropped,
  and "take the one about to expire first" is the cheapest thing a deadline can
  buy.

A job with no deadline sorts *after* one that has it, at equal priority:
undeclared means *whenever*, and whenever loses to soon. CONTROL's
`queued_work_is_priority_then_fifo` still passes because none of its jobs
declares one.

`JobError::DeadlineMissed` is distinct from `Cancelled` because the two mean
opposite things to a caller: cancellation says *somebody changed their mind*,
expiry says *the answer arrived too late to be worth having*. A streaming system
retries the second and not the first.

**First real customer, in this sub-phase:** off-screen asset previews get a
five-second budget; visible ones get none. An off-screen preview is speculative
prefetch, and one still queued five seconds later is almost certainly for a tile
nobody is looking at — running it makes the queue *further* behind for the tiles
somebody is looking at. A visible preview has a person waiting at a spinner, so
late beats never, and cancellation rather than expiry is the right tool when a
visible tile scrolls away.

### The budgeted main-thread drain

```rust
jobs.submit_applied(desc, work, |result| { /* runs on the main thread */ });
jobs.drain_completions(Duration::from_millis(2));   // once per frame
```

Seam 1's third property: **the worker produces data; the main thread installs
it.** Nothing touches `wgpu::Queue` or the widget tree off-thread.

The budget is the entire point. CONTROL measured thumbnail decodes at
232–260 ms; moving them off-thread stops them blocking the frame, but sixty of
them *finishing* on the same frame and each uploading a texture re-creates the
hitch at the other end. `drain_completions` returning with work outstanding is
**correct behaviour, not a bug** — that early return is the mechanism, and
`the_drain_stops_at_its_budget_and_resumes` asserts it directly rather than
inferring it.

Two details that are easy to get wrong and are handled:

- **`apply` receives failures too.** A caller that put up a placeholder has to
  be told to take it down; a job system that only delivers successes leaves a
  spinner on screen forever.
- **`DrainStats::longest_apply` is reported.** The budget is checked *between*
  applications, never inside one, so a single `apply` longer than the whole
  budget overruns it however small the budget is. The fix for that is a smaller
  `apply` — and this field is what makes that visible instead of looking like
  the budget being ignored.

`pump_jobs` in `app.rs` calls the drain once per frame with a 2 ms budget out of
16.6 ms: enough to install a handful of decodes, small enough that a burst of
sixty cannot become a hitch.

### Profiler telemetry, which the plan calls non-optional

> *"every job reports to the Phase 29 profiler as a CPU zone with its priority
> and its queue wait. A job system without visibility is a source of mystery
> hitches."*

Every finished job — including one that expired without running — emits a
`JobZone` carrying name, priority, **queue wait**, run time and outcome.
`somnium_core::jobs::profile_rows` folds them by name and the profiler panel
shows them under `— Jobs (CPU, background) —`.

Three decisions in that pipeline:

- **`somnium_jobs` reports; it does not display.** No `tracing` dependency, no
  renderer dependency, nothing about where a number is drawn. That is what keeps
  §7.9's "no dependency on anything else in the workspace" true, and it is also
  why the panel wiring lives in `somnium_core` and needed no change to
  `somnium_renderer`'s profiler at all.
- **Queue wait is shown first, and it is the *worst* rather than the total.**
  Run time says the work was slow; queue wait says the pool was busy, and those
  have different fixes. Totals hide the distinction — sixty jobs waiting 1 ms
  each and one waiting 60 ms sum identically and mean completely different
  things — so rows sort by worst wait, because the reason to open this panel is
  a hitch.
- **Expired jobs get their own column.** Streaming that quietly drops half its
  requests and a scheduler that is keeping up look identical without it.

The ring is bounded at 512 zones and **counts what it drops**, so the panel says
"and 47 more" rather than showing a short list as if it were whole. A telemetry
buffer that grows without limit is a leak with a friendly name.

### Determinism

`JobSystem::single_threaded()` runs every job inline on `submit`, sharing
`worker::run_task` with the threaded path so it produces the same statuses and
the same zones. Two implementations of "what a finished job looks like" is how a
determinism mode stops predicting the real one. It honours deadlines too, which
is what lets expiry be tested without a scheduler.

## Tests: 17, all green

Five carried from CONTROL, twelve new. The ones worth naming:

| Test | The failure it catches |
|---|---|
| `an_expired_job_never_runs` | Expiry implemented as "run it and discard the result", which costs exactly as much as not having deadlines. Asserts the closure's side effect never happened. |
| `a_deadline_does_not_beat_a_priority` | The tempting bug: sorting by deadline first, so a background prefetch preempts a user's import. |
| `earlier_deadlines_outrank_later_ones_at_equal_priority` | The deadline being decorative — recorded, and then ignored by the scheduler. |
| `applied_work_is_installed_on_the_calling_thread` | The whole safety argument. Asserts the `apply` closure ran on the *draining* thread, because it is allowed to touch the GPU queue and the widget tree. |
| `the_drain_stops_at_its_budget_and_resumes` | A budget that is accepted and ignored. Twenty ready completions at 4 ms each against a 1 ms budget. |
| `every_job_reports_a_zone_with_its_queue_wait` | Telemetry that reports run time only — which cannot distinguish slow work from a busy pool. |
| `a_panicking_job_fails_without_taking_the_worker_with_it` | One bad glTF shrinking the pool for the session. The symptom is everything gradually getting slower, and it points nowhere near the cause. |
| `zones_are_bounded_and_the_overflow_is_counted` | A short list presented as a whole one. |
| `a_failed_job_still_reaches_its_apply` | The permanent spinner. |

Plus three in `somnium_core::jobs` for the aggregation itself, including
`expired_jobs_are_counted_rather_than_hidden`.

## The owed item

**§8's exit — "opening `assets/terrain/` (60 PNGs, 1.17 GB) never drops a frame,
and the profiler shows why" — is not measured.** It needs a windowed GPU session
with the editor open, and this session had none.

Everything that measurement depends on is in place and unit-tested: the drain is
budgeted, the previews carry deadlines, and the profiler rows exist and are
populated. What is missing is the frame trace proving the number.

```bash
cargo run -p hello_engine --release   # open assets/terrain/, F-key the profiler
```

This is the same debt as MORROWIND-A2's `.somtime` parity and Phase CONTROL's
Track 2/3 evidence, and it is recorded the same way: named, with the command,
rather than quietly skipped. **One windowed session closes four owed items** —
A2's timing parity, A2's capability report, MORROWIND-A's first golden image,
and this one.

## Files

```
+ crates/somnium_jobs/Cargo.toml
+ crates/somnium_jobs/src/lib.rs          public API, JobSystem, JobDesc, JobZone
+ crates/somnium_jobs/src/queue.rs        bounded priority heap; deadline drop; zone ring
+ crates/somnium_jobs/src/worker.rs       the pool — the one place thread::spawn is allowed
+ crates/somnium_jobs/src/completion.rs   the budgeted main-thread drain
+ crates/somnium_jobs/src/tests.rs        17 tests
~ crates/somnium_core/src/jobs.rs         scheduler removed; re-export + EditorJobs + profile_rows
~ crates/somnium_core/src/app.rs          JobSystem; pump_jobs; profiler rows; preview deadline
~ crates/somnium_core/Cargo.toml          somnium_jobs
~ Cargo.toml                              workspace member + dependency
~ tools/ghostfence/run.py                 prefix exemptions; comment lines are not violations
```

The gate change is worth a line of its own. The `one-job-system` row initially
flagged `somnium_jobs/src/lib.rs:7` — a **doc comment describing the ban**. The
fix was to skip comment lines and to exempt the crate rather than one file: a
pool inside the job system is the job system, and the first thing anyone does
about a gate that flags its own documentation is turn it off.
