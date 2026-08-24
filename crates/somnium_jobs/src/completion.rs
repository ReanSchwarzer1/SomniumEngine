//! The budgeted main-thread completion drain.
//!
//! Seam 1's third property: *the worker produces data; the main thread installs
//! it.* Nothing here runs off-thread, which is what makes it safe for an
//! `apply` closure to touch `wgpu::Queue` or the widget tree.
//!
//! The budget is the whole reason this exists. Phase CONTROL measured thumbnail
//! decodes at 232–260 ms; moving them off-thread stops them blocking the frame,
//! but sixty of them *finishing* on the same frame and each uploading a texture
//! re-creates the hitch at the other end. A drain that stops when the budget is
//! spent and resumes next frame is what closes that hole.

use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

use crate::JobError;

/// What one call to the drain did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrainStats {
    /// Completions applied this call.
    pub applied: usize,
    /// Completions still waiting — either unfinished, or finished but not yet
    /// reached because the budget ran out.
    pub still_pending: usize,
    /// The budget was spent with work left. **Not an error.** Sustained
    /// exhaustion across many frames is worth a look; one frame of it is the
    /// system working as designed.
    pub budget_exhausted: bool,
    /// The longest single `apply` this call.
    ///
    /// Surfaced because the budget is checked *between* applications, never
    /// inside one: an `apply` longer than the whole budget overruns it no
    /// matter how small the budget is. When that happens the fix is a smaller
    /// `apply`, and this field is how that becomes visible instead of looking
    /// like the budget being ignored.
    pub longest_apply: Duration,
}

/// One outstanding completion: a receiver plus the closure that installs it.
struct Pending {
    /// Returns `true` once it has taken a value and run the apply closure.
    poll: Box<dyn FnMut() -> PollResult + Send>,
}

#[derive(PartialEq, Eq)]
enum PollResult {
    /// The worker has not finished.
    NotReady,
    /// A value arrived and the apply closure ran.
    Applied,
}

#[derive(Default)]
pub(crate) struct CompletionQueue {
    pending: Vec<Pending>,
}

impl CompletionQueue {
    pub fn push<T, A>(&mut self, receiver: Receiver<Result<T, JobError>>, apply: A)
    where
        T: Send + 'static,
        A: FnOnce(Result<T, JobError>) + Send + 'static,
    {
        let mut apply = Some(apply);
        self.pending.push(Pending {
            poll: Box::new(move || match receiver.try_recv() {
                Ok(value) => {
                    if let Some(apply) = apply.take() {
                        apply(value);
                    }
                    PollResult::Applied
                }
                Err(TryRecvError::Empty) => PollResult::NotReady,
                // The sender was dropped without sending — the job expired in
                // the queue and was never run. The caller still hears about it,
                // because a placeholder put up before submission has to come
                // back down.
                Err(TryRecvError::Disconnected) => {
                    if let Some(apply) = apply.take() {
                        apply(Err(JobError::DeadlineMissed));
                    }
                    PollResult::Applied
                }
            }),
        });
    }

    pub fn drain(&mut self, budget: Duration) -> DrainStats {
        let start = Instant::now();
        let mut stats = DrainStats::default();

        let mut index = 0;
        while index < self.pending.len() {
            if start.elapsed() >= budget {
                stats.budget_exhausted = true;
                break;
            }
            let before = Instant::now();
            match (self.pending[index].poll)() {
                PollResult::Applied => {
                    let took = before.elapsed();
                    stats.longest_apply = stats.longest_apply.max(took);
                    stats.applied += 1;
                    // `swap_remove` reorders the queue, and that is acceptable:
                    // completion order is already nondeterministic because the
                    // workers finish in whatever order they finish. A caller
                    // needing ordered installation must sequence it itself —
                    // and the ones that do (scene load, prefab instancing) have
                    // a natural key to sequence on.
                    self.pending.swap_remove(index);
                }
                PollResult::NotReady => index += 1,
            }
        }

        stats.still_pending = self.pending.len();
        stats
    }
}
