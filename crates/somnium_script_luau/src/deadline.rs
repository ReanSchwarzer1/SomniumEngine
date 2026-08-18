//! The interrupt controller — how a runaway script is stopped.
//!
//! Luau lets the host install an interrupt callback that the VM invokes at
//! regular points during execution. Returning an error from it unwinds the
//! script, which is the only way to stop code that has no intention of
//! returning.
//!
//! # Why the tick counter exists
//!
//! The interrupt fires very often — that is what makes it useful. Reading
//! the clock on every one of them would put a `QueryPerformanceCounter`
//! call in the innermost loop of every script in the game. So the check is
//! amortised: the clock is read once every [`CHECK_INTERVAL`] interrupts,
//! which keeps the overhead in the noise while still detecting an overrun
//! far inside the 2 ms the budget allows.
//!
//! # Why an atomic and not a `Mutex`
//!
//! The interrupt callback runs on whatever thread is executing the VM, and
//! `mlua`'s `send` feature requires it to be `Send`. A deadline is a single
//! integer; a lock around one integer that is read thousands of times a
//! frame is pure contention for no benefit.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Instant;

/// Interrupts between clock reads.
///
/// At Luau's interrupt rate this is well under a microsecond of drift and
/// removes essentially all of the timing overhead.
pub const CHECK_INTERVAL: u32 = 1024;

/// Sentinel meaning "no deadline is set" — used when nothing is running,
/// so an idle VM never trips.
const NO_DEADLINE: i64 = i64::MAX;

/// Shared, lock-free deadline state.
///
/// Cloning shares the same underlying deadline: one copy lives in the
/// interrupt callback, the other stays with the backend.
#[derive(Debug, Clone)]
pub struct Deadline {
    /// Nanoseconds after `epoch` at which the current call must stop.
    limit_nanos: Arc<AtomicI64>,
    /// Interrupts observed since the last clock read.
    ticks: Arc<AtomicU32>,
    /// Set when a deadline actually fired, so the caller can tell an
    /// interrupted script from one that raised an ordinary error.
    tripped: Arc<AtomicU32>,
    /// Fixed reference point, so the deadline can be a plain integer.
    epoch: Instant,
}

impl Deadline {
    /// A controller with no deadline armed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limit_nanos: Arc::new(AtomicI64::new(NO_DEADLINE)),
            ticks: Arc::new(AtomicU32::new(0)),
            tripped: Arc::new(AtomicU32::new(0)),
            epoch: Instant::now(),
        }
    }

    /// Arm the deadline `budget` from now, and clear any previous trip.
    pub fn arm(&self, budget: std::time::Duration) {
        let now = self.now_nanos();
        let limit = i64::try_from(budget.as_nanos()).unwrap_or(i64::MAX / 2);
        self.limit_nanos
            .store(now.saturating_add(limit), Ordering::Relaxed);
        self.ticks.store(0, Ordering::Relaxed);
        self.tripped.store(0, Ordering::Relaxed);
    }

    /// Disarm, so an idle VM cannot trip.
    pub fn disarm(&self) {
        self.limit_nanos.store(NO_DEADLINE, Ordering::Relaxed);
    }

    /// Whether the last armed call ran out of time.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed) != 0
    }

    /// Called from the interrupt. `true` means the script must stop.
    ///
    /// Once tripped it keeps returning `true` until re-armed: Luau can
    /// call the interrupt again while unwinding, and a deadline that
    /// un-trips would let the script keep going.
    #[must_use]
    pub fn should_stop(&self) -> bool {
        if self.tripped.load(Ordering::Relaxed) != 0 {
            return true;
        }
        let tick = self.ticks.fetch_add(1, Ordering::Relaxed);
        if tick % CHECK_INTERVAL != 0 {
            return false;
        }
        let limit = self.limit_nanos.load(Ordering::Relaxed);
        if limit == NO_DEADLINE {
            return false;
        }
        if self.now_nanos() >= limit {
            self.tripped.store(1, Ordering::Relaxed);
            return true;
        }
        false
    }

    fn now_nanos(&self) -> i64 {
        i64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(i64::MAX / 2)
    }
}

impl Default for Deadline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_unarmed_deadline_never_stops_anything() {
        let deadline = Deadline::new();
        for _ in 0..(CHECK_INTERVAL * 4) {
            assert!(!deadline.should_stop());
        }
        assert!(!deadline.tripped());
    }

    #[test]
    fn an_expired_deadline_trips_and_stays_tripped() {
        let deadline = Deadline::new();
        deadline.arm(Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(2));

        // The first tick of each interval is the one that reads the clock.
        let mut stopped = false;
        for _ in 0..CHECK_INTERVAL {
            if deadline.should_stop() {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "an expired deadline must stop the script");
        assert!(deadline.tripped());

        // Still tripped while Luau unwinds.
        assert!(deadline.should_stop());
        assert!(deadline.should_stop());
    }

    #[test]
    fn re_arming_clears_a_previous_trip() {
        let deadline = Deadline::new();
        deadline.arm(Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(2));
        while !deadline.should_stop() {}
        assert!(deadline.tripped());

        deadline.arm(Duration::from_secs(60));
        assert!(!deadline.tripped());
        assert!(!deadline.should_stop());
    }

    #[test]
    fn a_generous_deadline_does_not_trip() {
        let deadline = Deadline::new();
        deadline.arm(Duration::from_secs(60));
        for _ in 0..(CHECK_INTERVAL * 4) {
            assert!(!deadline.should_stop());
        }
        assert!(!deadline.tripped());
    }

    #[test]
    fn disarming_stops_an_idle_vm_from_tripping() {
        let deadline = Deadline::new();
        deadline.arm(Duration::from_nanos(1));
        deadline.disarm();
        std::thread::sleep(Duration::from_millis(2));
        for _ in 0..(CHECK_INTERVAL * 2) {
            assert!(!deadline.should_stop());
        }
    }

    #[test]
    fn the_controller_is_shared_by_cloning() {
        let a = Deadline::new();
        let b = a.clone();
        a.arm(Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(2));
        while !b.should_stop() {}
        assert!(a.tripped(), "both handles observe the same deadline");
    }
}
