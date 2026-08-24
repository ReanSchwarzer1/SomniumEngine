//! The bounded priority queue, and the telemetry ring beside it.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

use crate::{JobError, JobState, JobZone, ZONE_CAPACITY};

pub(crate) type JobTask = Box<dyn FnOnce(crate::JobContext) + Send + 'static>;

pub(crate) struct QueuedJob {
    pub priority: crate::JobPriority,
    pub deadline: Option<Instant>,
    pub sequence: u64,
    pub submitted: Instant,
    pub state: Arc<JobState>,
    pub task: Option<JobTask>,
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
    /// Priority first, then the earlier deadline, then FIFO.
    ///
    /// The deadline tiebreak sits **between** priority and sequence rather than
    /// above priority, and that placement is the whole design:
    ///
    /// - Above priority, a far-future deadline on a background job would
    ///   outrank an urgent user action, which is exactly backwards.
    /// - Below sequence, a deadline would affect nothing but whether a job is
    ///   dropped, and "take the one that is about to expire first" is the
    ///   cheapest thing a deadline can buy.
    ///
    /// A job with no deadline sorts after one that has it, at equal priority.
    /// That is deliberate: an undeclared deadline means *whenever*, and
    /// whenever loses to soon. Phase CONTROL's `queued_work_is_priority_then_fifo`
    /// still holds because none of its jobs declares one.
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| match (self.deadline, other.deadline) {
                // `BinaryHeap` is a max-heap, so "greater" leaves first:
                // an earlier deadline must compare Greater.
                (Some(a), Some(b)) => b.cmp(&a),
                (Some(_), None) => Ordering::Greater,
                (None, Some(_)) => Ordering::Less,
                (None, None) => Ordering::Equal,
            })
            // Earlier FIFO sequence wins within a priority.
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct QueueState {
    jobs: BinaryHeap<QueuedJob>,
    capacity: usize,
    shutdown: bool,
}

/// Finished-job timings, bounded, oldest dropped first.
#[derive(Default)]
struct Zones {
    ring: VecDeque<JobZone>,
    dropped: usize,
}

pub(crate) struct SharedQueue {
    state: Mutex<QueueState>,
    zones: Mutex<Zones>,
    ready: Condvar,
}

impl SharedQueue {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(QueueState {
                jobs: BinaryHeap::new(),
                capacity: capacity.max(1),
                shutdown: false,
            }),
            zones: Mutex::new(Zones::default()),
            ready: Condvar::new(),
        })
    }

    pub fn push(&self, job: QueuedJob) -> Result<(), JobError> {
        let mut queue = self.state.lock().expect("job queue poisoned");
        if queue.jobs.len() >= queue.capacity {
            return Err(JobError::QueueFull);
        }
        queue.jobs.push(job);
        drop(queue);
        self.ready.notify_one();
        Ok(())
    }

    /// Block until a job is available, then return it. `None` means shut down.
    ///
    /// Jobs whose deadline has already passed are **discarded here rather than
    /// handed to a worker**, and the discard is recorded as a zone so the
    /// profiler shows work that was thrown away. Streaming that quietly drops
    /// half its requests and a scheduler that is keeping up look identical
    /// without that row.
    pub fn pop_blocking(&self) -> Option<QueuedJob> {
        let mut queue = self.state.lock().expect("job queue poisoned");
        loop {
            while queue.jobs.is_empty() && !queue.shutdown {
                queue = self.ready.wait(queue).expect("job queue poisoned");
            }
            if queue.shutdown {
                return None;
            }
            let job = queue.jobs.pop().expect("non-empty queue");
            if job.deadline.is_some_and(|d| Instant::now() > d) {
                job.state.set_status(crate::JobStatus::Expired);
                // The task is dropped without running, which drops its result
                // sender; the handle then observes `Disconnected`. That is the
                // correct signal for a caller that never polled — the answer is
                // not coming.
                drop(job.task);
                let zone = JobZone {
                    name: job.state.name,
                    priority: job.priority,
                    queued_for: job.submitted.elapsed(),
                    ran_for: std::time::Duration::ZERO,
                    outcome: crate::JobStatus::Expired,
                };
                drop(queue);
                self.record_zone(zone);
                queue = self.state.lock().expect("job queue poisoned");
                continue;
            }
            return Some(job);
        }
    }

    pub fn record_zone(&self, zone: JobZone) {
        let mut zones = self.zones.lock().expect("zone ring poisoned");
        if zones.ring.len() == ZONE_CAPACITY {
            zones.ring.pop_front();
            zones.dropped += 1;
        }
        zones.ring.push_back(zone);
    }

    pub fn take_zones(&self) -> (Vec<JobZone>, usize) {
        let mut zones = self.zones.lock().expect("zone ring poisoned");
        let dropped = std::mem::take(&mut zones.dropped);
        (zones.ring.drain(..).collect(), dropped)
    }

    /// Mark shut down, cancel everything queued, and wake every worker.
    pub fn shutdown(&self) {
        if let Ok(mut queue) = self.state.lock() {
            queue.shutdown = true;
            for job in &queue.jobs {
                job.state
                    .cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
        self.ready.notify_all();
    }
}
