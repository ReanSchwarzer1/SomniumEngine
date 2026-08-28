//! The worker pool.
//!
//! **This is the only place in the workspace allowed to call `thread::spawn`.**
//! `tools/ghostfence/run.py`'s `one-job-system` row enforces that against every
//! `.rs` file under `crates/`, `examples/` and `tools/`, with a short exemption
//! list where each entry carries the reason it is not a second thread pool.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, mpsc::SyncSender},
    thread::JoinHandle,
    time::Instant,
};

use crate::{JobContext, JobError, JobState, JobStatus, JobZone, queue::SharedQueue};

/// Start one worker.
pub(crate) fn spawn(index: usize, shared: Arc<SharedQueue>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("somnium-job-{index}"))
        .spawn(move || worker_loop(&shared))
        .expect("job worker")
}

fn worker_loop(shared: &SharedQueue) {
    while let Some(mut job) = shared.pop_blocking() {
        let context = JobContext {
            state: Arc::clone(&job.state),
        };
        if let Some(task) = job.task.take() {
            task(context);
        }
    }
}

/// Run one task and publish its outcome, its result and its timing.
///
/// Shared with [`crate::JobSystem::single_threaded`] so deterministic mode
/// produces the same statuses and the same zones as the threaded path. Two
/// implementations of "what a finished job looks like" is how a determinism
/// mode stops predicting the real one.
pub(crate) fn run_task<T, F>(
    context: JobContext,
    state: Arc<JobState>,
    sender: SyncSender<Result<T, JobError>>,
    task: F,
    submitted: Instant,
    shared: &SharedQueue,
) where
    T: Send + 'static,
    F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
{
    let started = Instant::now();
    let queued_for = started.saturating_duration_since(submitted);

    // Publish in this order, always: **zone, then status, then result.**
    //
    // A terminal status and a delivered value are both things an observer waits
    // on, so either one arriving before the zone exists makes the telemetry
    // racy — an observer that sees `Completed` and immediately calls
    // `take_zones` would sometimes find nothing. Recording first makes "a job
    // that reports finished has already reported its zone" an invariant rather
    // than a timing accident.
    let publish = |outcome: JobStatus, ran_for, result: Result<T, JobError>| {
        shared.record_zone(JobZone {
            name: state.name,
            priority: state.priority,
            queued_for,
            ran_for,
            outcome,
        });
        state.set_status(outcome);
        let _ = sender.send(result);
    };

    if context.is_cancelled() {
        publish(
            JobStatus::Cancelled,
            std::time::Duration::ZERO,
            Err(JobError::Cancelled),
        );
        return;
    }

    state.set_status(JobStatus::Running);
    // A panicking job must not take the worker with it: one bad glTF file
    // would otherwise shrink the pool for the rest of the session, and the
    // symptom — everything gradually getting slower — points nowhere near
    // the cause.
    let outcome = catch_unwind(AssertUnwindSafe(|| task(context.clone())));
    let ran_for = started.elapsed();

    let (status, result) = match outcome {
        // Cancelled *during* the work, and the work returned anyway. The value
        // is discarded: the caller asked for it to stop, and handing back a
        // result it no longer wants is how a cancelled thumbnail decode ends up
        // installed over the one the user actually scrolled to.
        Ok(Ok(_value)) if context.is_cancelled() => {
            (JobStatus::Cancelled, Err(JobError::Cancelled))
        }
        Ok(Ok(value)) => {
            state
                .progress
                .store(10_000, std::sync::atomic::Ordering::Release);
            (JobStatus::Completed, Ok(value))
        }
        Ok(Err(error)) => (JobStatus::Failed, Err(JobError::Failed(error))),
        Err(_) => (JobStatus::Failed, Err(JobError::Panicked)),
    };

    publish(status, ran_for, result);
}
