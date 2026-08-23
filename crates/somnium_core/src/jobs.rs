//! Bounded editor job registry.
//!
//! The public surface is intentionally narrow so Phase MORROWIND can move this
//! module into `somnium_jobs` without changing call sites.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BinaryHeap},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering as AtomicOrdering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread::JoinHandle,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
/// Scheduling class; larger values leave the heap first.
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
}

impl JobStatus {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Running,
            2 => Self::Completed,
            3 => Self::Failed,
            4 => Self::Cancelled,
            _ => Self::Queued,
        }
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
}

struct JobState {
    id: u64,
    name: &'static str,
    status: AtomicU8,
    progress: AtomicU32,
    cancelled: AtomicBool,
}

impl JobState {
    fn new(id: u64, name: &'static str) -> Self {
        Self {
            id,
            name,
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
            cancellable: !matches!(status, JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled),
        }
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

type JobTask = Box<dyn FnOnce(JobContext) + Send + 'static>;

struct QueuedJob {
    priority: JobPriority,
    sequence: u64,
    state: Arc<JobState>,
    task: Option<JobTask>,
}

impl PartialEq for QueuedJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}
impl Eq for QueuedJob {}
impl PartialOrd for QueuedJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for QueuedJob {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            // Earlier FIFO sequence wins within a priority.
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct QueueState {
    jobs: BinaryHeap<QueuedJob>,
    capacity: usize,
    shutdown: bool,
}

struct SharedQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

/// Bounded priority scheduler with a fixed worker count.
pub struct JobRegistry {
    shared: Arc<SharedQueue>,
    workers: Vec<JoinHandle<()>>,
    states: BTreeMap<u64, Arc<JobState>>,
    next_id: AtomicU64,
    sequence: u64,
}

impl Default for JobRegistry {
    fn default() -> Self {
        let available = std::thread::available_parallelism().map_or(2, usize::from);
        Self::with_workers_and_capacity(available.saturating_sub(1).clamp(1, 4), 128)
    }
}

impl JobRegistry {
    /// Construct a registry with explicit resource bounds.
    #[must_use]
    pub fn with_workers_and_capacity(workers: usize, capacity: usize) -> Self {
        let shared = Arc::new(SharedQueue {
            state: Mutex::new(QueueState {
                jobs: BinaryHeap::new(),
                capacity: capacity.max(1),
                shutdown: false,
            }),
            ready: Condvar::new(),
        });
        let workers = (0..workers.max(1))
            .map(|index| {
                let queue = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name(format!("somnium-job-{index}"))
                    .spawn(move || worker_loop(&queue))
                    .expect("job worker")
            })
            .collect();
        Self {
            shared,
            workers,
            states: BTreeMap::new(),
            next_id: AtomicU64::new(1),
            sequence: 0,
        }
    }

    /// Submit typed worker work without blocking on execution.
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
        let id = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
        let state = Arc::new(JobState::new(id, name));
        let (sender, receiver) = sync_channel(1);
        let task_state = Arc::clone(&state);
        let wrapped = Box::new(move |context: JobContext| {
            run_task(context, task_state, sender, task);
        });
        let mut queue = self.shared.state.lock().expect("job queue poisoned");
        if queue.jobs.len() >= queue.capacity {
            return Err(JobError::QueueFull);
        }
        queue.jobs.push(QueuedJob {
            priority,
            sequence: self.sequence,
            state: Arc::clone(&state),
            task: Some(wrapped),
        });
        self.sequence = self.sequence.wrapping_add(1);
        self.states.insert(id, Arc::clone(&state));
        drop(queue);
        self.shared.ready.notify_one();
        Ok(JobHandle { state, receiver })
    }

    /// Run the existing terrain PNG bake through the shared bounded scheduler.
    pub fn submit_terrain_bake(
        &mut self,
        workspace: std::path::PathBuf,
        arguments: Vec<String>,
    ) -> Result<JobHandle<()>, JobError> {
        let mut command = std::process::Command::new("cargo");
        command
            .current_dir(workspace)
            .args(["run", "--release", "-p", "somnium_asset", "--example", "pack_terrain", "--"])
            .args(arguments);
        self.submit_process("Terrain bake", JobPriority::User, command)
    }

    /// Run the existing BC7 encoder through the shared bounded scheduler.
    pub fn submit_bc7_encode(
        &mut self,
        workspace: std::path::PathBuf,
        fast: bool,
    ) -> Result<JobHandle<()>, JobError> {
        let mut command = std::process::Command::new("cargo");
        command.current_dir(workspace).args([
            "run",
            "--release",
            "-p",
            "somnium_renderer",
            "--example",
            "encode_terrain_bc7",
        ]);
        if fast {
            command.arg("--").arg("--fast");
        }
        self.submit_process("BC7 encode", JobPriority::User, command)
    }

    fn submit_process(
        &mut self,
        name: &'static str,
        priority: JobPriority,
        mut command: std::process::Command,
    ) -> Result<JobHandle<()>, JobError> {
        self.submit(name, priority, move |context| {
            let mut child = command.spawn().map_err(|error| error.to_string())?;
            loop {
                if context.is_cancelled() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("cancelled".into());
                }
                match child.try_wait().map_err(|error| error.to_string())? {
                    Some(status) if status.success() => return Ok(()),
                    Some(status) => return Err(format!("process exited with {status}")),
                    None => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        })
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
            .filter(|snapshot| {
                matches!(snapshot.status, JobStatus::Queued | JobStatus::Running)
            })
            .collect()
    }

    /// Drop completed registry bookkeeping.
    pub fn prune_finished(&mut self) {
        self.states.retain(|_, state| {
            matches!(
                JobStatus::from_raw(state.status.load(AtomicOrdering::Acquire)),
                JobStatus::Queued | JobStatus::Running
            )
        });
    }
}

fn run_task<T, F>(
    context: JobContext,
    state: Arc<JobState>,
    sender: SyncSender<Result<T, JobError>>,
    task: F,
) where
    T: Send + 'static,
    F: FnOnce(JobContext) -> Result<T, String> + Send + 'static,
{
    if context.is_cancelled() {
        state.status.store(4, AtomicOrdering::Release);
        let _ = sender.send(Err(JobError::Cancelled));
        return;
    }
    state.status.store(1, AtomicOrdering::Release);
    let outcome = catch_unwind(AssertUnwindSafe(|| task(context.clone())));
    let result = match outcome {
        Ok(Ok(_value)) if context.is_cancelled() => {
            state.status.store(4, AtomicOrdering::Release);
            Err(JobError::Cancelled)
        }
        Ok(Ok(value)) => {
            state.progress.store(10_000, AtomicOrdering::Release);
            state.status.store(2, AtomicOrdering::Release);
            Ok(value)
        }
        Ok(Err(error)) => {
            state.status.store(3, AtomicOrdering::Release);
            Err(JobError::Failed(error))
        }
        Err(_) => {
            state.status.store(3, AtomicOrdering::Release);
            Err(JobError::Panicked)
        }
    };
    let _ = sender.send(result);
}

fn worker_loop(shared: &SharedQueue) {
    loop {
        let mut queue = shared.state.lock().expect("job queue poisoned");
        while queue.jobs.is_empty() && !queue.shutdown {
            queue = shared.ready.wait(queue).expect("job queue poisoned");
        }
        if queue.shutdown {
            return;
        }
        let mut job = queue.jobs.pop().expect("non-empty queue");
        drop(queue);
        let context = JobContext {
            state: Arc::clone(&job.state),
        };
        if let Some(task) = job.task.take() {
            task(context);
        }
    }
}

impl Drop for JobRegistry {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.state.lock() {
            queue.shutdown = true;
            for job in &queue.jobs {
                job.state.cancelled.store(true, AtomicOrdering::Release);
            }
        }
        self.shared.ready.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc::channel, time::Duration};

    #[test]
    fn a_slow_job_never_blocks_submit() {
        let mut jobs = JobRegistry::with_workers_and_capacity(1, 4);
        let start = std::time::Instant::now();
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

    #[test]
    fn queued_work_is_priority_then_fifo() {
        let mut jobs = JobRegistry::with_workers_and_capacity(1, 8);
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
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)).unwrap(), "high");
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)).unwrap(), "low");
    }

    #[test]
    fn cancellation_and_progress_are_observable() {
        let mut jobs = JobRegistry::with_workers_and_capacity(1, 4);
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
        let mut jobs = JobRegistry::with_workers_and_capacity(1, 1);
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
}
