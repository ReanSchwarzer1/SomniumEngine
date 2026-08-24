//! Background work with declared priorities, deadlines and a budgeted drain.
//!
//! Phase MORROWIND, Seam 1, built by MORROWIND-B. **This crate is the promotion
//! of Phase CONTROL's `somnium_core::jobs::JobRegistry`, not a second job
//! system.** Appendix A.6 of the plan settles that explicitly, `phase_MORROWIND.md`
//! §10's "one job system" row forbids the alternative, and
//! `tools/ghostfence/run.py` checks both — a bare `thread::spawn` anywhere
//! outside [`worker`], or a second type named `JobRegistry` or `JobSystem`,
//! fails the gate.
//!
//! CONTROL wrote its registry with a deliberately narrow public surface so this
//! move would be a rename rather than a rewrite (`somnium_core/src/jobs.rs:3`
//! says so). It was.
//!
//! # The three properties that make this a job system rather than a thread pool
//!
//! 1. **Priority and deadline are declared, not inferred.** A visible thumbnail
//!    outranks an off-screen one because the *submitter* said so. A streaming
//!    cell the camera is about to enter carries a deadline measured in frames,
//!    and **a job whose deadline passed while it was queued is dropped rather
//!    than run** — which is what makes streaming thrash bounded when a camera
//!    turns around. O3DE's `AzCore/IO/Streamer/` is the reference for that
//!    contract; see `ATTRIBUTION.md` §13H.3.
//!
//! 2. **Cancellation is first-class and cooperative.** The work closure gets a
//!    [`JobContext`] so a long loop can poll it. Cancellation checked only
//!    *between* jobs is not cancellation.
//!
//! 3. **Completion is applied on the main thread, inside a time budget.** The
//!    worker produces *data*; the main thread installs it. Nothing touches
//!    `wgpu::Queue` or the widget tree off-thread.
//!    [`JobSystem::drain_completions`] returning with work still outstanding is
//!    **correct behaviour, not a bug**: that early return is the mechanism that
//!    stops a burst of finished decodes becoming a frame spike.
//!
//! # Two ways to submit, and when each is right
//!
//! [`JobSystem::submit`] returns a typed [`JobHandle`] the caller polls with
//! [`JobHandle::try_take`]. Use it when the caller has somewhere natural to
//! poll and wants the value in its own hands — this is what every Phase CONTROL
//! call site does, and none of them changed.
//!
//! [`JobSystem::submit_applied`] takes a second closure that runs on the main
//! thread inside the drain budget. Use it when the result has to be *installed*
//! somewhere — a GPU upload, a widget-tree mutation, a cache insert — because
//! that is the case where a burst of completions lands on one frame and the
//! budget is the only thing standing between it and a hitch.
//!
//! # Determinism
//!
//! [`JobSystem::single_threaded`] runs every job inline on `submit`. Tests that
//! care about ordering use it; tests that care about the scheduler do not.

#![deny(missing_docs)]

mod completion;
mod queue;
mod worker;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering as AtomicOrdering},
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub use completion::DrainStats;

use completion::CompletionQueue;
use queue::{QueuedJob, SharedQueue};

/// How many finished-job timings are retained for the profiler.
///
/// Bounded on purpose: a long import can finish thousands of jobs, and a
/// telemetry buffer that grows without limit is a leak with a friendly name.
/// Oldest are dropped first and the drop is counted, so the profiler can say
/// "and 400 more" rather than quietly under-reporting.
const ZONE_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
/// Scheduling class; larger values leave the heap first.
///
/// These are Phase CONTROL's four levels, carried over unchanged because they
/// have call sites and because they already express Seam 1's property — the
/// submitter declares the class. The plan's sketch named three
/// (`Critical` / `Interactive` / `Background`); they map onto
/// `User` / `Visible` / `Background` with `Normal` as the unremarkable default,
/// and adding a fifth level to match a sketch would have broken working call
/// sites for nothing.
pub enum JobPriority {
    /// Maintenance not currently visible.
    Background = 0,
    /// Ordinary editor work.
    #[default]
    Normal = 1,
    /// Work needed by the visible viewport/drawer range.
    Visible = 2,
    /// Explicit user action.
    User = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Observable lifecycle of a registered job.
pub enum JobStatus {
    /// Accepted but not started.
    Queued,
    /// Executing on a worker.
    Running,
    /// Finished successfully.
    Completed,
    /// Returned an error or panicked.
    Failed,
    /// Cancellation was observed.
    Cancelled,
    /// Its deadline passed while it was still queued, so it was never run.
    Expired,
}

impl JobStatus {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Running,
            2 => Self::Completed,
            3 => Self::Failed,
            4 => Self::Cancelled,
            5 => Self::Expired,
            _ => Self::Queued,
        }
    }

    fn raw(self) -> u8 {
        match self {
            Self::Queued => 0,
            Self::Running => 1,
            Self::Completed => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
            Self::Expired => 5,
        }
    }

    /// Whether the job will never change state again.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Status-bar-safe job projection.
pub struct JobSnapshot {
    /// Registry identity.
    pub id: u64,
    /// Human-readable operation.
    pub name: &'static str,
    /// Current lifecycle state.
    pub status: JobStatus,
    /// Normalized progress in `0..=1`.
    pub progress: f32,
    /// Whether cancellation can still be requested.
    pub cancellable: bool,
    /// Scheduling class it was submitted with.
    ///
    /// Carried so a surface can tell work a person started from housekeeping
    /// that runs on its own — the status bar's Cancel chip shows the former
    /// and not the latter.
    pub priority: JobPriority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Failure returned by a job handle.
pub enum JobError {
    /// Cooperative cancellation.
    Cancelled,
    /// Task-provided failure text.
    Failed(String),
    /// Task unwound; the worker survived.
    Panicked,
    /// Bounded queue had no room.
    QueueFull,
    /// Result channel closed unexpectedly.
    Disconnected,
    /// The declared deadline passed while the job was queued.
    ///
    /// Distinct from [`JobError::Cancelled`] because the two mean opposite
    /// things to a caller: cancellation says *somebody changed their mind*, and
    /// this says *the answer arrived too late to be worth having*. A streaming
    /// system retries the second and not the first.
    DeadlineMissed,
}

/// One finished job's timing, for the Phase 29 profiler.
///
/// Every job produces one. `phase_MORROWIND.md` §8 makes this non-optional:
/// *"A job system without visibility is a source of mystery hitches."* The
/// crate reports; it does not display, which is why there is no `tracing`
/// dependency here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JobZone {
    /// The `&'static str` the submitter had to provide.
    pub name: &'static str,
    /// The class it was submitted with, so a trace can separate user-initiated
    /// work from housekeeping.
    pub priority: JobPriority,
    /// How long it sat in the queue before a worker took it.
    ///
    /// **This is the number that explains a stall.** Run time says the work was
    /// slow; queue wait says the pool was too small or something ahead of it
    /// was too big, and those have different fixes.
    pub queued_for: Duration,
    /// How long it ran, once started. Zero for a job that expired while queued.
    pub ran_for: Duration,
    /// How it ended.
    pub outcome: JobStatus,
}

struct JobState {
    id: u64,
    name: &'static str,
    priority: JobPriority,
    status: AtomicU8,
    progress: AtomicU32,
    cancelled: AtomicBool,
}

impl JobState {
    fn new(id: u64, name: &'static str, priority: JobPriority) -> Self {
        Self {
            id,
            name,
            priority,
            status: AtomicU8::new(0),
            progress: AtomicU32::new(0),
            cancelled: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> JobSnapshot {
        let status = JobStatus::from_raw(self.status.load(AtomicOrdering::Acquire));
        JobSnapshot {
            id: self.id,
            name: self.name,
            status,
            progress: self.progress.load(AtomicOrdering::Relaxed) as f32 / 10_000.0,
            cancellable: !status.is_terminal(),
            priority: self.priority,
        }
    }

    fn set_status(&self, status: JobStatus) {
        self.status.store(status.raw(), AtomicOrdering::Release);
    }
}

#[derive(Clone)]
/// Cooperative cancellation and progress token passed to worker closures.
pub struct JobContext {
    state: Arc<JobState>,
}

impl JobContext {
    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(AtomicOrdering::Acquire)
    }

    /// Return a cancellation error when cancellation has been requested.
    pub fn check_cancelled(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Publish normalized progress for status-bar observers.
    pub fn set_progress(&self, progress: f32) {
        let value = (progress.clamp(0.0, 1.0) * 10_000.0).round() as u32;
        self.state.progress.store(value, AtomicOrdering::Release);
    }
}

/// Typed result handle retained by the submitter.
pub struct JobHandle<T> {
    state: Arc<JobState>,
    receiver: Receiver<Result<T, JobError>>,
}

impl<T> JobHandle<T> {
    /// Registry id used by status-bar cancellation.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.state.id
    }

    /// Request cooperative cancellation.
    pub fn cancel(&self) {
        self.state.cancelled.store(true, AtomicOrdering::Release);
    }

    /// Read status without blocking.
    #[must_use]
    pub fn snapshot(&self) -> JobSnapshot {
        self.state.snapshot()
    }

    /// Take a completed result without blocking.
    pub fn try_take(&self) -> Option<Result<T, JobError>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JobError::Disconnected)),
        }
    }
}

/// Everything the scheduler needs to know about one submission.
///
/// Constructed with [`JobDesc::new`] and refined with the builder methods, so
/// adding a field later does not break call sites — which matters, because
/// there are already a dozen of them and Track 4 adds many more.
#[derive(Clone)]
pub struct JobDesc {
    /// Profiler zone label. **Mandatory, and `&'static str` on purpose.**
    ///
    /// Every job becomes a CPU zone. A job system without profiler visibility
    /// converts one mystery — a stall — into a harder one: a stall somewhere
    /// inside a thread pool. Retrofitting names across call sites later is
    /// tedious and always incomplete, so the type makes it un-skippable.
    pub name: &'static str,
    /// Scheduling class.
    pub priority: JobPriority,
    /// Wall-clock instant after which the result is worthless.
    ///
    /// A job whose deadline has passed while it was queued is **dropped, not
    /// run**. Within one priority, an earlier deadline is taken first.
    pub deadline: Option<Instant>,
}

impl JobDesc {
    /// A job at [`JobPriority::Normal`] with no deadline.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            priority: JobPriority::Normal,
            deadline: None,
        }
    }

    /// Set the scheduling class.
    #[must_use]
    pub fn priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set an absolute deadline.
    #[must_use]
    pub fn deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set a deadline relative to now.
    #[must_use]
    pub fn within(mut self, budget: Duration) -> Self {
        self.deadline = Some(Instant::now() + budget);
        self
    }
}

/// Bounded priority scheduler with a fixed worker count.
///
/// One per process. See the module documentation for why that is enforced
/// rather than merely recommended.
pub struct JobSystem {
    shared: Arc<SharedQueue>,
    workers: Vec<JoinHandle<()>>,
    states: BTreeMap<u64, Arc<JobState>>,
    completions: CompletionQueue,
    next_id: AtomicU64,
    sequence: u64,
    /// `true` when [`JobSystem::single_threaded`] built this one.
    inline: bool,
}

impl Default for JobSystem {
    fn default() -> Self {
        // `available_parallelism() - 1`: the main thread is the one being
        // protected, so it does not get a worker of its own. Clamped to four
        // because this pool serves *background* work — the win is not blocking
        // the frame, and a sixteen-worker pool on a big machine mostly buys
        // sixteen simultaneous disk reads.
        let available = std::thread::available_parallelism().map_or(2, usize::from);
        Self::with_workers_and_capacity(available.saturating_sub(1).clamp(1, 4), 128)
    }
}

impl JobSystem {
    /// Construct a system with explicit resource bounds.
    #[must_use]
    pub fn with_workers_and_capacity(workers: usize, capacity: usize) -> Self {
        let shared = SharedQueue::new(capacity);
        let workers = (0..workers.max(1))
            .map(|index| worker::spawn(index, Arc::clone(&shared)))
            .collect();
        Self {
            shared,
            workers,
            states: BTreeMap::new(),
            completions: CompletionQueue::default(),
            next_id: AtomicU64::new(1),
            sequence: 0,
            inline: false,
        }
    }

    /// Deterministic mode: every job runs inline on [`JobSystem::submit`].
    ///
    /// For tests that need a result to exist by the next line. Tests *about*
    /// the scheduler — priority ordering, deadline expiry, queue bounds — must
    /// not use this, because it schedules nothing.
    #[must_use]
    pub fn single_threaded() -> Self {
        Self {
            shared: SharedQueue::new(usize::MAX),
            workers: Vec::new(),
            states: BTreeMap::new(),
            completions: CompletionQueue::default(),
            next_id: AtomicU64::new(1),
            sequence: 0,
            inline: true,
        }
    }

    /// Number of worker threads. Zero in single-threaded mode.
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Submit work, taking the result through a [`JobHandle`].
    ///
    /// Phase CONTROL's signature, unchanged, so its call sites moved by
    /// changing an import.
    pub fn submit<T, F>(
        &mut self,
        name: &'static str,
        priority: JobPriority,
        task: F,
    ) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
    {
        self.submit_with(JobDesc::new(name).priority(priority), task)
    }

    /// Submit work with a full [`JobDesc`] — the form that can carry a deadline.
    pub fn submit_with<T, F>(
        &mut self,
        desc: JobDesc,
        task: F,
    ) -> Result<JobHandle<T>, JobError>
    where
        T: Send + 'static,
        F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
    {
        let (state, receiver) = self.enqueue(desc, task)?;
        Ok(JobHandle { state, receiver })
    }

    /// Submit work whose result is *installed* on the main thread.
    ///
    /// `work` runs on a worker and produces data. `apply` runs inside
    /// [`JobSystem::drain_completions`]' budget, on the main thread, and is
    /// where a GPU upload or a widget-tree mutation belongs.
    ///
    /// `apply` also receives the failure cases, because a caller that put up a
    /// placeholder needs to take it down again — a job system that silently
    /// swallows a failed decode leaves a spinner on screen forever.
    pub fn submit_applied<T, F, A>(
        &mut self,
        desc: JobDesc,
        work: F,
        apply: A,
    ) -> Result<u64, JobError>
    where
        T: Send + 'static,
        F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
        A: FnOnce(Result<T, JobError>) + Send + 'static,
    {
        let (state, receiver) = self.enqueue(desc, work)?;
        let id = state.id;
        self.completions.push(receiver, apply);
        Ok(id)
    }

    fn enqueue<T, F>(
        &mut self,
        desc: JobDesc,
        task: F,
    ) -> Result<(Arc<JobState>, Receiver<Result<T, JobError>>), JobError>
    where
        T: Send + 'static,
        F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
        let state = Arc::new(JobState::new(id, desc.name, desc.priority));
        let (sender, receiver) = sync_channel(1);
        let task_state = Arc::clone(&state);
        let submitted = Instant::now();

        if self.inline {
            // Deterministic mode. The deadline check still applies, so a test
            // can prove expiry without a scheduler.
            let context = JobContext {
                state: Arc::clone(&state),
            };
            if desc.deadline.is_some_and(|d| Instant::now() > d) {
                state.set_status(JobStatus::Expired);
                let _ = sender.send(Err(JobError::DeadlineMissed));
                self.shared.record_zone(JobZone {
                    name: desc.name,
                    priority: desc.priority,
                    queued_for: Duration::ZERO,
                    ran_for: Duration::ZERO,
                    outcome: JobStatus::Expired,
                });
            } else {
                worker::run_task(context, Arc::clone(&state), sender, task, submitted, &self.shared);
            }
            self.states.insert(id, Arc::clone(&state));
            return Ok((state, receiver));
        }

        let wrapped = {
            let shared = Arc::clone(&self.shared);
            Box::new(move |context: JobContext| {
                worker::run_task(context, task_state, sender, task, submitted, &shared);
            })
        };

        self.shared.push(QueuedJob {
            priority: desc.priority,
            deadline: desc.deadline,
            sequence: self.sequence,
            submitted,
            state: Arc::clone(&state),
            task: Some(wrapped),
        })?;
        self.sequence = self.sequence.wrapping_add(1);
        self.states.insert(id, Arc::clone(&state));
        Ok((state, receiver))
    }

    /// Apply finished work on the main thread until `budget` is spent.
    ///
    /// **Call once per frame.** Returning with `still_pending > 0` is correct
    /// behaviour: that early return is what stops a burst of completions
    /// becoming a frame spike, and the remainder is applied next frame.
    ///
    /// The budget is checked *between* applications, never inside one — a
    /// single `apply` longer than the whole budget will overrun it, and the
    /// fix for that is a smaller `apply`, not a smaller budget. [`DrainStats`]
    /// reports the overrun so the case is visible rather than mysterious.
    pub fn drain_completions(&mut self, budget: Duration) -> DrainStats {
        self.completions.drain(budget)
    }

    /// Cancel a registered job by id.
    pub fn cancel(&self, id: u64) -> bool {
        let Some(state) = self.states.get(&id) else {
            return false;
        };
        state.cancelled.store(true, AtomicOrdering::Release);
        true
    }

    /// Snapshot queued and running jobs.
    #[must_use]
    pub fn active(&self) -> Vec<JobSnapshot> {
        self.states
            .values()
            .map(|state| state.snapshot())
            .filter(|snapshot| matches!(snapshot.status, JobStatus::Queued | JobStatus::Running))
            .collect()
    }

    /// Drop completed bookkeeping.
    pub fn prune_finished(&mut self) {
        self.states.retain(|_, state| {
            !JobStatus::from_raw(state.status.load(AtomicOrdering::Acquire)).is_terminal()
        });
    }

    /// Take the finished-job timings accumulated since the last call.
    ///
    /// The caller feeds these to the Phase 29 profiler as CPU zones. Returns
    /// the zones and how many were dropped for capacity, so an under-report is
    /// stated rather than silent.
    pub fn take_zones(&self) -> (Vec<JobZone>, usize) {
        self.shared.take_zones()
    }
}

impl Drop for JobSystem {
    fn drop(&mut self) {
        self.shared.shutdown();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
