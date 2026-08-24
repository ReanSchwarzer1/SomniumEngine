//! Tests for the job system.
//!
//! The first five are **Phase CONTROL's, carried over unchanged in substance**.
//! That is the point of a promotion: if the moved code passes the tests the old
//! code passed, the move preserved behaviour, and MORROWIND-B's additions can
//! be judged on their own. Everything after `queue_is_bounded` is new.

use super::*;
use std::sync::{Mutex, mpsc::channel};

// ---------------------------------------------------------------------------
// Carried over from Phase CONTROL (`somnium_core::jobs`)
// ---------------------------------------------------------------------------

#[test]
fn a_slow_job_never_blocks_submit() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 4);
    let start = Instant::now();
    let handle = jobs
        .submit("test.slow", JobPriority::Normal, |_| {
            std::thread::sleep(Duration::from_millis(50));
            Ok(7)
        })
        .unwrap();
    assert!(start.elapsed() < Duration::from_millis(20));
    while handle.try_take().is_none() {
        std::thread::yield_now();
    }
}

/// A snapshot carries the class it was submitted with.
///
/// The status bar's Cancel chip reads this to tell work a person started from
/// housekeeping that runs on its own. Without it the chip blinked three times a
/// second on the background inventory sweep, which taught people to ignore the
/// one place an import reports itself.
#[test]
fn a_snapshot_reports_the_priority_it_was_submitted_with() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::User, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    let sweep = jobs
        .submit("test.sweep", JobPriority::Background, |_| Ok(()))
        .unwrap();

    assert_eq!(blocker.snapshot().priority, JobPriority::User);
    assert_eq!(sweep.snapshot().priority, JobPriority::Background);
    assert!(
        jobs.active()
            .iter()
            .any(|job| job.priority == JobPriority::Background),
        "the panel still lists housekeeping even though the chip will not"
    );

    release_tx.send(()).unwrap();
    drop(blocker);
}

#[test]
fn queued_work_is_priority_then_fifo() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::User, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while blocker.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }
    let (order_tx, order_rx) = channel();
    let low_tx = order_tx.clone();
    let _low = jobs
        .submit("test.low", JobPriority::Background, move |_| {
            low_tx.send("low").unwrap();
            Ok(())
        })
        .unwrap();
    let _high = jobs
        .submit("test.high", JobPriority::Visible, move |_| {
            order_tx.send("high").unwrap();
            Ok(())
        })
        .unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(
        order_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "high"
    );
    assert_eq!(
        order_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "low"
    );
}

#[test]
fn cancellation_and_progress_are_observable() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 4);
    let handle = jobs
        .submit("test.cancel", JobPriority::Normal, |ctx| {
            ctx.set_progress(0.5);
            while !ctx.is_cancelled() {
                std::thread::yield_now();
            }
            Ok(())
        })
        .unwrap();
    while handle.snapshot().progress < 0.5 {
        std::thread::yield_now();
    }
    handle.cancel();
    loop {
        if let Some(result) = handle.try_take() {
            assert_eq!(result, Err(JobError::Cancelled));
            break;
        }
        std::thread::yield_now();
    }
}

#[test]
fn queue_is_bounded() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 1);
    let (tx, rx) = sync_channel::<()>(0);
    let running = jobs
        .submit("test.running", JobPriority::Normal, move |_| {
            rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while running.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }
    let _queued = jobs.submit("test.queued", JobPriority::Normal, |_| Ok::<_, String>(()));
    let full = jobs.submit("test.full", JobPriority::Normal, |_| Ok::<_, String>(()));
    assert_eq!(full.err(), Some(JobError::QueueFull));
    tx.send(()).unwrap();
}

// ---------------------------------------------------------------------------
// New in MORROWIND-B — deadlines
// ---------------------------------------------------------------------------

/// A job whose deadline passed while it was queued is dropped, not run.
///
/// This is the property that makes streaming thrash bounded: the camera turns
/// around, the cells it was going to enter are no longer wanted, and the work
/// evaporates instead of being done at leisure and thrown away. A scheduler
/// that runs everything eventually cannot be given a workload it can drop.
#[test]
fn an_expired_job_never_runs() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::Normal, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while blocker.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }

    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let stale = jobs
        .submit_with(
            // Already in the past when it is submitted.
            JobDesc::new("test.stale").deadline(Instant::now() - Duration::from_millis(1)),
            move |_| {
                flag.store(true, AtomicOrdering::Release);
                Ok(())
            },
        )
        .unwrap();

    release_tx.send(()).unwrap();

    // The worker discards it on pop, so the sender drops and the handle sees a
    // disconnect rather than a value.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if stale.snapshot().status == JobStatus::Expired {
            break;
        }
        assert!(Instant::now() < deadline, "expired job was never reaped");
        std::thread::yield_now();
    }
    assert!(
        !ran.load(AtomicOrdering::Acquire),
        "an expired job must not run at all, not merely have its result ignored"
    );
}

/// Within one priority, the earlier deadline is taken first.
#[test]
fn earlier_deadlines_outrank_later_ones_at_equal_priority() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::User, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while blocker.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }

    let (order_tx, order_rx) = channel();
    let late_tx = order_tx.clone();
    // Submitted first, so FIFO alone would run it first.
    let _late = jobs
        .submit_with(
            JobDesc::new("test.late").within(Duration::from_secs(60)),
            move |_| {
                late_tx.send("late").unwrap();
                Ok(())
            },
        )
        .unwrap();
    let _soon = jobs
        .submit_with(
            JobDesc::new("test.soon").within(Duration::from_secs(30)),
            move |_| {
                order_tx.send("soon").unwrap();
                Ok(())
            },
        )
        .unwrap();

    release_tx.send(()).unwrap();
    assert_eq!(
        order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "soon",
        "the earlier deadline must win over FIFO at equal priority"
    );
    assert_eq!(
        order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "late"
    );
}

/// A deadline never outranks a priority.
///
/// The failure this guards is the tempting one: sorting by deadline first, so a
/// background prefetch with a tight deadline preempts a user's import.
#[test]
fn a_deadline_does_not_beat_a_priority() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::User, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while blocker.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }

    let (order_tx, order_rx) = channel();
    let bg_tx = order_tx.clone();
    let _urgent_background = jobs
        .submit_with(
            JobDesc::new("test.prefetch")
                .priority(JobPriority::Background)
                .within(Duration::from_millis(500)),
            move |_| {
                bg_tx.send("background").unwrap();
                Ok(())
            },
        )
        .unwrap();
    let _relaxed_user = jobs
        .submit_with(
            JobDesc::new("test.import").priority(JobPriority::User),
            move |_| {
                order_tx.send("user").unwrap();
                Ok(())
            },
        )
        .unwrap();

    release_tx.send(()).unwrap();
    assert_eq!(
        order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "user",
        "a tight deadline on background work must not preempt a user action"
    );
}

// ---------------------------------------------------------------------------
// New in MORROWIND-B — the budgeted drain
// ---------------------------------------------------------------------------

#[test]
fn applied_work_is_installed_on_the_calling_thread() {
    let mut jobs = JobSystem::with_workers_and_capacity(2, 8);
    let main = std::thread::current().id();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);

    jobs.submit_applied(
        JobDesc::new("test.applied"),
        |_| Ok(41),
        move |value| {
            sink.lock()
                .unwrap()
                .push((std::thread::current().id(), value.unwrap() + 1));
        },
    )
    .unwrap();

    let stop = Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() {
        jobs.drain_completions(Duration::from_millis(5));
        assert!(Instant::now() < stop, "completion never applied");
        std::thread::yield_now();
    }

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].1, 42);
    assert_eq!(
        seen[0].0, main,
        "apply must run on the draining thread — it is allowed to touch the \
         GPU queue and the widget tree, and both are main-thread only"
    );
}

/// The budget stops the drain, and the remainder survives to the next call.
///
/// This is the anti-hitch mechanism, so it is asserted directly rather than
/// inferred: a burst of completions that would each cost real time must not all
/// land on one frame.
#[test]
fn the_drain_stops_at_its_budget_and_resumes() {
    let mut jobs = JobSystem::with_workers_and_capacity(2, 64);
    let applied = Arc::new(AtomicU32::new(0));

    for _ in 0..20 {
        let counter = Arc::clone(&applied);
        jobs.submit_applied(
            JobDesc::new("test.burst"),
            |_| Ok(()),
            move |_| {
                // Each application costs more than the whole budget below, so
                // exactly one can fit per call.
                std::thread::sleep(Duration::from_millis(4));
                counter.fetch_add(1, AtomicOrdering::Release);
            },
        )
        .unwrap();
    }

    // Let the workers finish so every completion is *ready* and the only thing
    // limiting the drain is the budget.
    let ready = Instant::now() + Duration::from_secs(3);
    loop {
        if jobs.active().is_empty() {
            break;
        }
        assert!(Instant::now() < ready, "workers never finished");
        std::thread::yield_now();
    }

    let first = jobs.drain_completions(Duration::from_millis(1));
    assert!(
        first.budget_exhausted,
        "20 ready completions at 4 ms each cannot fit in a 1 ms budget"
    );
    assert!(
        first.applied >= 1,
        "the budget is checked between applications, so one always runs"
    );
    assert!(
        first.still_pending > 0,
        "the rest must survive to the next frame"
    );
    assert!(
        first.longest_apply >= Duration::from_millis(3),
        "an apply that overruns the budget must be visible in the stats, not hidden"
    );

    let stop = Instant::now() + Duration::from_secs(10);
    while applied.load(AtomicOrdering::Acquire) < 20 {
        jobs.drain_completions(Duration::from_millis(50));
        assert!(
            Instant::now() < stop,
            "the drain never finished the backlog"
        );
    }
    assert_eq!(
        jobs.drain_completions(Duration::from_millis(1))
            .still_pending,
        0
    );
}

#[test]
fn a_failed_job_still_reaches_its_apply() {
    // A caller that put up a placeholder has to be told to take it down. A job
    // system that only delivers successes leaves a spinner on screen forever.
    let mut jobs = JobSystem::with_workers_and_capacity(1, 4);
    let outcome = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&outcome);

    jobs.submit_applied(
        JobDesc::new("test.fails"),
        |_| Err::<(), _>("no such file".to_string()),
        move |result| *sink.lock().unwrap() = Some(result),
    )
    .unwrap();

    let stop = Instant::now() + Duration::from_secs(2);
    while outcome.lock().unwrap().is_none() {
        jobs.drain_completions(Duration::from_millis(5));
        assert!(Instant::now() < stop, "failure never applied");
    }
    assert_eq!(
        outcome.lock().unwrap().as_ref().unwrap(),
        &Err(JobError::Failed("no such file".into()))
    );
}

// ---------------------------------------------------------------------------
// New in MORROWIND-B — telemetry, determinism, resilience
// ---------------------------------------------------------------------------

/// Every job produces a zone, and the zone carries the queue wait.
///
/// Queue wait is the number that distinguishes "the work was slow" from "the
/// pool was busy", and those have different fixes. A profiler row without it
/// cannot tell them apart.
#[test]
fn every_job_reports_a_zone_with_its_queue_wait() {
    let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
    let (release_tx, release_rx) = sync_channel::<()>(0);
    let blocker = jobs
        .submit("test.blocker", JobPriority::Normal, move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    while blocker.snapshot().status != JobStatus::Running {
        std::thread::yield_now();
    }
    let waiter = jobs
        .submit("test.waiter", JobPriority::Normal, |_| Ok(()))
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    release_tx.send(()).unwrap();

    let stop = Instant::now() + Duration::from_secs(2);
    while waiter.snapshot().status != JobStatus::Completed {
        assert!(Instant::now() < stop, "waiter never completed");
        std::thread::yield_now();
    }

    let (zones, dropped) = jobs.take_zones();
    assert_eq!(dropped, 0);
    let waited = zones
        .iter()
        .find(|z| z.name == "test.waiter")
        .expect("the waiter produced no zone");
    assert_eq!(waited.outcome, JobStatus::Completed);
    assert!(
        waited.queued_for >= Duration::from_millis(15),
        "queue wait was {:?}; it must reflect the time spent behind the blocker",
        waited.queued_for
    );

    assert!(
        jobs.take_zones().0.is_empty(),
        "taking zones must consume them"
    );
}

#[test]
fn a_panicking_job_fails_without_taking_the_worker_with_it() {
    // One bad glTF file must not shrink the pool for the rest of the session:
    // the symptom of that is everything gradually getting slower, and it points
    // nowhere near the cause.
    let mut jobs = JobSystem::with_workers_and_capacity(1, 4);
    let bad = jobs
        .submit(
            "test.panics",
            JobPriority::Normal,
            |_| -> Result<(), String> {
                panic!("deliberate");
            },
        )
        .unwrap();
    let stop = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(result) = bad.try_take() {
            assert_eq!(result, Err(JobError::Panicked));
            break;
        }
        assert!(Instant::now() < stop, "panicking job never reported");
        std::thread::yield_now();
    }

    let good = jobs
        .submit("test.after", JobPriority::Normal, |_| Ok(9))
        .unwrap();
    let stop = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(result) = good.try_take() {
            assert_eq!(result, Ok(9), "the worker survived the panic");
            break;
        }
        assert!(
            Instant::now() < stop,
            "the pool died with the panicking job"
        );
        std::thread::yield_now();
    }
}

#[test]
fn single_threaded_mode_runs_inline() {
    let mut jobs = JobSystem::single_threaded();
    assert_eq!(jobs.worker_count(), 0);
    let handle = jobs
        .submit("test.inline", JobPriority::Normal, |_| Ok(3))
        .unwrap();
    // No polling loop: the value exists by the next line, which is the entire
    // reason this mode exists.
    assert_eq!(handle.try_take(), Some(Ok(3)));
    assert_eq!(handle.snapshot().status, JobStatus::Completed);

    let (zones, _) = jobs.take_zones();
    assert_eq!(
        zones.len(),
        1,
        "deterministic mode must produce the same telemetry as the threaded \
         path, or it stops predicting it"
    );
}

#[test]
fn single_threaded_mode_still_honours_deadlines() {
    let mut jobs = JobSystem::single_threaded();
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let handle = jobs
        .submit_with(
            JobDesc::new("test.stale").deadline(Instant::now() - Duration::from_millis(1)),
            move |_| {
                flag.store(true, AtomicOrdering::Release);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(handle.try_take(), Some(Err(JobError::DeadlineMissed)));
    assert_eq!(handle.snapshot().status, JobStatus::Expired);
    assert!(!ran.load(AtomicOrdering::Acquire));
}

#[test]
fn zones_are_bounded_and_the_overflow_is_counted() {
    let mut jobs = JobSystem::single_threaded();
    for _ in 0..(ZONE_CAPACITY + 10) {
        let _ = jobs.submit("test.many", JobPriority::Background, |_| Ok(()));
    }
    let (zones, dropped) = jobs.take_zones();
    assert_eq!(zones.len(), ZONE_CAPACITY);
    assert_eq!(
        dropped, 10,
        "an under-report must be stated, not silent — the profiler says \
         \"and 10 more\" rather than showing a short list as if it were whole"
    );
}

#[test]
fn prune_drops_only_terminal_jobs() {
    let mut jobs = JobSystem::single_threaded();
    let _done = jobs.submit("test.done", JobPriority::Normal, |_| Ok(()));
    assert!(jobs.active().is_empty());
    jobs.prune_finished();
    assert!(!jobs.cancel(1), "a pruned job is no longer cancellable");
}
