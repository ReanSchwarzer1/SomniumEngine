//! High-resolution frame timing and frame-rate limiting.
//!
//! [`TimeState`] is updated once per frame by the engine's main loop and
//! exposed to game code through [`EngineContext`](crate::context::EngineContext).
//!
//! # Frame Limiter Strategy
//!
//! When a `target_fps` is configured, the limiter uses a **hybrid
//! sleep + spin-wait** approach:
//!
//! 1. **Coarse sleep** — If the remaining budget exceeds 2 ms, we
//!    `thread::sleep` for `(remaining - 1ms)`. OS timers are
//!    imprecise (Windows defaults to ~15.6 ms granularity unless
//!    `timeBeginPeriod(1)` is called), so we always leave headroom.
//! 2. **Spin-wait** — Busy-loop on `Instant::now()` until the exact
//!    target time. This burns CPU but guarantees sub-microsecond
//!    accuracy.
//!
//! This avoids the classic pitfall of pure-sleep limiters that drift by
//! ±16 ms on Windows and produce visible stutter.

use std::time::{Duration, Instant};

/// Smoothing factor for the exponential moving average (EMA) of FPS.
///
/// `α = 0.1` means each new sample contributes 10 % to the running
/// average, producing a stable readout that is still responsive to
/// sustained changes within ~10 frames.
const FPS_SMOOTHING: f64 = 0.1;

/// How far ahead of the target deadline we switch from `thread::sleep`
/// to a spin-wait, in milliseconds.
const SPIN_WAIT_THRESHOLD: Duration = Duration::from_millis(1);

/// Per-frame timing data and frame-rate limiter state.
///
/// This struct is **not** `Clone` or `Copy` by design — it contains
/// interior time-tracking state that must only be advanced by the engine.
#[derive(Debug)]
pub struct TimeState {
    /// Wall-clock duration of the last completed frame.
    ///
    /// On the very first frame this will be zero (or a negligibly small
    /// value), so game logic should guard against `dt == 0` if it
    /// performs division.
    delta_time: Duration,

    /// Total wall-clock time elapsed since the engine started.
    elapsed: Duration,

    /// Monotonically increasing frame counter (starts at 0).
    frame_count: u64,

    /// Exponentially smoothed frames-per-second.
    fps: f64,

    /// Target frame duration derived from `EngineConfig::target_fps`.
    /// `None` when uncapped.
    target_frame_time: Option<Duration>,

    // ── Internal bookkeeping (not exposed) ──────────────────────────
    /// Instant the engine was first started.
    startup: Instant,

    /// Instant the most recent frame began.
    last_frame: Instant,
}

impl TimeState {
    /// Create a new `TimeState`, recording `Instant::now()` as the
    /// engine start time.
    ///
    /// # Arguments
    ///
    /// * `target_fps` — Optional frame-rate cap. Pass the value from
    ///   [`EngineConfig::target_fps`](crate::config::EngineConfig::target_fps).
    #[must_use]
    pub fn new(target_fps: Option<u32>) -> Self {
        let now = Instant::now();
        Self {
            delta_time: Duration::ZERO,
            elapsed: Duration::ZERO,
            frame_count: 0,
            fps: 0.0,
            target_frame_time: target_fps.map(|fps| Duration::from_secs_f64(1.0 / f64::from(fps))),
            startup: now,
            last_frame: now,
        }
    }

    /// Advance the timer by one frame.
    ///
    /// This must be called **exactly once** per frame, at the beginning
    /// of the update phase. It computes `delta_time`, updates `elapsed`
    /// and `frame_count`, and smooths the FPS reading.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.delta_time = now.duration_since(self.last_frame);
        self.elapsed = now.duration_since(self.startup);
        self.last_frame = now;
        self.frame_count += 1;

        // Exponential moving average of FPS.
        let instantaneous_fps = if self.delta_time.as_secs_f64() > 0.0 {
            1.0 / self.delta_time.as_secs_f64()
        } else {
            0.0
        };

        if self.frame_count == 1 {
            // Seed the EMA on the first real frame.
            self.fps = instantaneous_fps;
        } else {
            self.fps = FPS_SMOOTHING.mul_add(instantaneous_fps - self.fps, self.fps);
        }
    }

    /// Block the calling thread until the frame budget is met.
    ///
    /// This is a no-op when `target_fps` is `None`. Otherwise it uses
    /// the hybrid sleep/spin strategy described in the module docs.
    ///
    /// # Panics
    ///
    /// This method contains defensive `expect` calls on `Duration`
    /// arithmetic that are logically unreachable (guarded by prior
    /// comparisons). A panic here indicates a bug in the timing logic.
    pub fn wait_for_frame_budget(&self) {
        let Some(target) = self.target_frame_time else {
            return;
        };

        let elapsed_this_frame = self.last_frame.elapsed();
        if elapsed_this_frame >= target {
            return;
        }

        // SAFETY: We checked `elapsed_this_frame < target` above, so this
        // subtraction cannot underflow.
        let remaining = target
            .checked_sub(elapsed_this_frame)
            .expect("elapsed_this_frame < target; underflow impossible");

        // Coarse sleep: only if we have enough headroom.
        if remaining > SPIN_WAIT_THRESHOLD {
            std::thread::sleep(
                remaining
                    .checked_sub(SPIN_WAIT_THRESHOLD)
                    .expect("remaining > SPIN_WAIT_THRESHOLD; underflow impossible"),
            );
        }

        // Fine spin-wait: busy-loop to the precise deadline.
        let deadline = self.last_frame + target;
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
    }

    // ── Public accessors ────────────────────────────────────────────

    /// Duration of the last completed frame.
    #[inline]
    #[must_use]
    pub fn delta_time(&self) -> Duration {
        self.delta_time
    }

    /// Delta time as `f32` seconds — the most common form used in
    /// physics and animation: `position += velocity * dt`.
    #[inline]
    #[must_use]
    pub fn dt(&self) -> f32 {
        self.delta_time.as_secs_f32()
    }

    /// Total time elapsed since engine startup.
    #[inline]
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Number of frames rendered so far.
    #[inline]
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Smoothed frames-per-second (EMA).
    #[inline]
    #[must_use]
    pub fn fps(&self) -> f64 {
        self.fps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_zeroed() {
        let ts = TimeState::new(Some(60));
        assert_eq!(ts.frame_count(), 0);
        assert_eq!(ts.delta_time(), Duration::ZERO);
        assert!(ts.fps().abs() < f64::EPSILON);
    }

    #[test]
    fn tick_increments_frame_count() {
        let mut ts = TimeState::new(None);
        ts.tick();
        assert_eq!(ts.frame_count(), 1);
        ts.tick();
        assert_eq!(ts.frame_count(), 2);
    }

    #[test]
    fn tick_produces_nonzero_delta() {
        let mut ts = TimeState::new(None);
        // Burn a tiny amount of time.
        std::thread::sleep(Duration::from_millis(1));
        ts.tick();
        assert!(ts.delta_time() > Duration::ZERO);
    }

    #[test]
    fn uncapped_wait_is_noop() {
        let ts = TimeState::new(None);
        // Should return instantly.
        let before = Instant::now();
        ts.wait_for_frame_budget();
        assert!(before.elapsed() < Duration::from_millis(1));
    }
}
