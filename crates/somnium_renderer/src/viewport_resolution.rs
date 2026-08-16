//! Internal 3D resolution caps, independent of the swapchain / UI.
//!
//! The editor window (and fullscreen) stay at the display's pixel size so
//! chrome and gizmos stay sharp. Scene passes render into a smaller target
//! and bilinear-upscale. Index 0 is Native — no cap.

/// Labels for the viewport toolbar combo. Order matches [`scene_size_for_preset`].
pub const VIEWPORT_RESOLUTION_LABELS: [&str; 5] =
    ["Native", "2560×1440", "1920×1080", "1600×900", "1280×720"];

const CAPS: [(u32, u32); 5] = [(0, 0), (2560, 1440), (1920, 1080), (1600, 900), (1280, 720)];

/// Scene buffer size for a window and a named preset.
///
/// Fits inside both the window and the preset box, so a 2560×1440 fullscreen
/// view at 1920×1080 is exactly 1920×1080, and a 1080p window never supersamples.
pub fn scene_size_for_preset(window_w: u32, window_h: u32, preset: usize) -> (u32, u32) {
    let w = window_w.max(1);
    let h = window_h.max(1);
    let Some(&(cap_w, cap_h)) = CAPS.get(preset) else {
        return (w, h);
    };
    if cap_w == 0 || cap_h == 0 {
        return (w, h);
    }
    let scale = (cap_w as f32 / w as f32)
        .min(cap_h as f32 / h as f32)
        .min(1.0);
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

// ─── Dynamic resolution (Phase DOOM-F) ───────────────────────────────────────

/// Scale steps. A change costs a full reallocation of every scene-sized render
/// target — visibility buffer, depth, HDR, GTAO, TAA history, FSR context — so
/// the grid is deliberately coarse. Sixteenths give a useful range in about six
/// usable steps between the default floor and native, and once the controller
/// settles it stops reallocating entirely.
///
/// This is the honest limitation of doing dynamic resolution by resizing rather
/// than by rendering into a sub-rect of a fixed target. The sub-rect approach is
/// what avoids reallocation altogether, and it needs every pass to become
/// viewport-aware — a much larger change than this phase is buying.
const SCALE_STEP: f32 = 1.0 / 16.0;

/// Fraction of the target the frame time may sit either side of before the
/// controller reacts. Without a dead band a controller chases its own noise:
/// DOOM-A measured the Coastal frame's own standard deviation at 0.94 ms on a
/// 38 ms frame, which is 2.5%.
const DEAD_BAND: f32 = 0.10;

/// Frames between adjustments, going down and going up.
///
/// Both are at least the profiler's 30-frame smoothing window, because reacting
/// faster than the instrument settles means deciding on an average that still
/// describes the previous resolution. Raising waits longer than dropping,
/// because a scale that oscillates is more objectionable than one slightly too
/// low.
const COOLDOWN_DOWN: u32 = 30;
const COOLDOWN_UP: u32 = 45;

/// Frames to let the new resolution settle before the measurement window is
/// thrown away and rebuilt.
///
/// A resize reallocates every scene-sized target, and the first frames after it
/// are not representative of anything — FSR rebuilds its context, TAA has no
/// history, the Hi-Z pyramid is stale. Resetting the profiler's window
/// *immediately* would seed the fresh average with exactly those frames.
pub const SETTLE_FRAMES: u32 = 8;

/// How much of the gap to the ideal scale to take in one adjustment.
///
/// Not 1.0: only part of the frame scales with resolution — DOOM-A's Coastal
/// ground table has Shading, GTAO and the water passes scaling, while Shadows,
/// the TLAS build, culling and the editor overlays do not — so solving for
/// "scale² × frame = target" overshoots every time and then has to come back.
const GAIN: f32 = 0.5;

/// Resolution controller driven by measured GPU frame time (Phase DOOM-F).
///
/// DOOM-B established that the shading pass runs exactly one fragment per pixel
/// with no overdraw, and that it is 67% of a Coastal ground frame. Pixel count
/// is therefore the most direct lever the engine has on its dominant cost, and
/// it is linear: at a 67% scale the 25.7 ms shading pass costs about 11.6 ms
/// with nothing else changed.
///
/// It is **off by default**, and that is a deliberate contract rather than
/// caution. This is the one part of Phase DOOM that trades fidelity for speed,
/// so it has to be something the user chooses, with the floor visible, rather
/// than something the engine does quietly when a frame gets expensive.
#[derive(Clone, Copy, Debug)]
pub struct DynamicResolution {
    pub enabled: bool,
    /// Frame time being aimed at, in milliseconds. 16.67 is 60 Hz.
    pub target_ms: f32,
    /// Lowest scale the controller may choose. The quality floor the user sets.
    pub min_scale: f32,
    scale: f32,
    cooldown: u32,
    /// Frames left before the measurement window should be reset. `Some(0)` is
    /// the frame the caller must do it on.
    settle: Option<u32>,
    /// Lowest scale measured over the budget, if any.
    ///
    /// **This is what stops the two-step limit cycle**, and it is not a
    /// refinement — without it the controller oscillates forever on any target
    /// the quantisation cannot land on. Measured on Coastal ground at a 33 ms
    /// target:
    ///
    /// ```text
    /// scale 1.0     →  37.6 ms   (above the 36.3 high band → step down)
    /// scale 0.9375  →  29.0 ms   (below the 29.7 low band  → step up)
    /// ```
    ///
    /// One step changes the frame by ~23% while the dead band is ±10%, so *no
    /// reachable scale lands inside it* and the pair repeats indefinitely.
    /// Widening the band or lengthening the cooldown only changes how slowly it
    /// flips — both were tried, and both left the resolution visibly pumping.
    ///
    /// The fix is to remember which scale was too slow and refuse to go back to
    /// it, which settles on the safe side of a gap the quantisation cannot
    /// straddle. Cleared when the frame comes in far enough under budget that
    /// the scene itself must have changed.
    over_budget: Option<f32>,
}

impl Default for DynamicResolution {
    fn default() -> Self {
        Self {
            enabled: false,
            target_ms: 1000.0 / 60.0,
            min_scale: 0.67,
            scale: 1.0,
            cooldown: 0,
            settle: None,
            over_budget: None,
        }
    }
}

/// A frame this far under the target means the scene changed, not that the
/// controller was wrong — so the "too slow" memory is dropped and the
/// controller is free to climb again.
const RELEARN_FRACTION: f32 = 0.70;

impl DynamicResolution {
    /// Current scale, always 1.0 while disabled.
    #[must_use]
    pub fn scale(&self) -> f32 {
        if self.enabled { self.scale } else { 1.0 }
    }

    /// Reset to native. Called when the controller is switched off, so turning
    /// it off restores full resolution instead of freezing the last scale.
    pub fn reset(&mut self) {
        self.scale = 1.0;
        self.cooldown = 0;
        self.settle = None;
        self.over_budget = None;
    }

    /// True on the frame the caller should discard the profiler's rolling
    /// average, some frames after a scale change (see [`SETTLE_FRAMES`]).
    ///
    /// Consuming rather than querying, so the caller cannot reset twice for one
    /// change or forget to clear the flag.
    pub fn take_settle_due(&mut self) -> bool {
        match self.settle {
            Some(0) => {
                self.settle = None;
                true
            }
            Some(n) => {
                self.settle = Some(n - 1);
                false
            }
            None => false,
        }
    }

    /// Apply the current scale to a base size, rounded to even pixels.
    ///
    /// Even because several passes downsample by two — the Hi-Z chain, bloom,
    /// the half-resolution water reflection — and an odd source leaves those
    /// with a half-texel offset that reads as a shimmering edge.
    #[must_use]
    pub fn apply(&self, base_w: u32, base_h: u32) -> (u32, u32) {
        let s = self.scale();
        if s >= 1.0 {
            return (base_w.max(1), base_h.max(1));
        }
        let round_even = |v: u32| -> u32 {
            let scaled = (v as f32 * s).round() as u32;
            (scaled & !1).max(2)
        };
        (round_even(base_w), round_even(base_h))
    }

    /// Advance the controller by one frame.
    ///
    /// `frame_ms` is the smoothed **GPU** frame time. CPU frame delta is the
    /// wrong signal: `TimeState`'s hybrid limiter and vsync both pin it near the
    /// budget whatever the GPU is doing, so a controller reading it would see a
    /// frame that is always exactly on target and never move.
    ///
    /// Returns the new scale when it changed, and `None` otherwise — the caller
    /// only reallocates on a change.
    pub fn tick(&mut self, frame_ms: f32) -> Option<f32> {
        if !self.enabled || !frame_ms.is_finite() || frame_ms <= 0.0 {
            return None;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return None;
        }

        let low = self.target_ms * (1.0 - DEAD_BAND);
        let high = self.target_ms * (1.0 + DEAD_BAND);

        // Comfortably under budget: whatever made a larger scale too slow is
        // no longer true, so let the controller climb again.
        if frame_ms < self.target_ms * RELEARN_FRACTION {
            self.over_budget = None;
        }
        if frame_ms > high {
            // Remember this scale as too slow before leaving it.
            self.over_budget = Some(match self.over_budget {
                Some(prev) => prev.min(self.scale),
                None => self.scale,
            });
        }

        if frame_ms >= low && frame_ms <= high {
            return None;
        }
        // Under budget, but the next scale up is one already known to miss it.
        // Stopping here is the whole point: this is the gap the quantisation
        // cannot land inside, and the safe side of it is the right place to sit.
        if frame_ms < low
            && self
                .over_budget
                .is_some_and(|bad| self.scale + SCALE_STEP >= bad - 1e-4)
        {
            return None;
        }
        // Already at a limit in the direction the frame is asking for: nothing
        // to do, and re-entering the cooldown would only delay a legitimate
        // adjustment in the other direction.
        if frame_ms > high && self.scale <= self.min_scale {
            return None;
        }
        if frame_ms < low && self.scale >= 1.0 {
            return None;
        }

        // Cost scales with pixel count, and pixel count with the square of the
        // scale — so the scale that would hit the target is the current one
        // times sqrt(target/measured). Damped by GAIN because only part of the
        // frame obeys that relationship.
        let ideal = self.scale * (self.target_ms / frame_ms).sqrt();
        let stepped = self.scale + (ideal - self.scale) * GAIN;
        let quantised = (stepped / SCALE_STEP).round() * SCALE_STEP;
        let next = quantised.clamp(self.min_scale, 1.0);

        // One step at a time. A frame that spikes to four times the budget would
        // otherwise halve the resolution in a single jump, which is far more
        // visible than taking three frames' worth of cooldown to get there.
        let next = if next < self.scale {
            next.max(self.scale - SCALE_STEP)
        } else {
            next.min(self.scale + SCALE_STEP)
        };

        // A bare inequality, not "at least half a step".
        //
        // Half a step looks like the right guard against jitter and is not: the
        // quantisation and the one-step clamp above already make every ordinary
        // move exactly one step, so the only sub-step moves that reach here are
        // the *last* one onto `min_scale` or 1.0 — and a floor that is not on
        // the sixteenths grid (0.67 sits between 0.625 and 0.6875) is never
        // reachable in one whole step. Rejecting it stranded the controller
        // above its floor: it settled at 0.688 costing 22.18 ms against a 16.67
        // target and then refused to move again, because every subsequent
        // adjustment was clamped to the same unreachable 0.67.
        if (next - self.scale).abs() < 1e-4 {
            return None;
        }
        let going_down = next < self.scale;
        self.scale = next;
        self.cooldown = if going_down {
            COOLDOWN_DOWN
        } else {
            COOLDOWN_UP
        };
        self.settle = Some(SETTLE_FRAMES);
        Some(self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::scene_size_for_preset;
    use super::{DynamicResolution, SCALE_STEP, SETTLE_FRAMES};

    /// Run the controller to rest against a synthetic frame cost.
    ///
    /// `cost(scale)` models a frame whose resolution-dependent part scales with
    /// pixel count and whose fixed part does not — the shape DOOM-A measured.
    fn settle(mut dr: DynamicResolution, cost: impl Fn(f32) -> f32) -> (DynamicResolution, u32) {
        let mut changes = 0;
        for _ in 0..4000 {
            if dr.tick(cost(dr.scale())).is_some() {
                changes += 1;
            }
        }
        (dr, changes)
    }

    #[test]
    fn native_matches_the_window() {
        assert_eq!(scene_size_for_preset(2560, 1440, 0), (2560, 1440));
    }

    #[test]
    fn two_k_window_at_1080p_is_exactly_1080p() {
        assert_eq!(scene_size_for_preset(2560, 1440, 2), (1920, 1080));
    }

    #[test]
    fn never_exceeds_the_window() {
        assert_eq!(scene_size_for_preset(1280, 720, 2), (1280, 720));
    }

    #[test]
    fn a_disabled_controller_never_moves() {
        let mut dr = DynamicResolution::default();
        assert!(!dr.enabled);
        for _ in 0..500 {
            assert_eq!(dr.tick(100.0), None);
        }
        assert_eq!(dr.scale(), 1.0);
        assert_eq!(dr.apply(2560, 1392), (2560, 1392));
    }

    #[test]
    fn a_slow_frame_lowers_the_scale_and_a_fast_one_raises_it() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        // 38.4 ms is the measured Coastal ground frame.
        assert!(dr.tick(38.4).is_some());
        assert!(dr.scale() < 1.0);
        let lowered = dr.scale();

        for _ in 0..COOLDOWN_LONG_ENOUGH {
            dr.tick(4.0);
        }
        assert!(dr.scale() > lowered, "never recovered from {lowered}");
    }
    const COOLDOWN_LONG_ENOUGH: u32 = 200;

    #[test]
    fn it_settles_instead_of_pumping() {
        // The failure this controller exists to avoid: a scale that oscillates
        // is more objectionable than one that is slightly too low, because the
        // eye reads the change and not the resolution.
        let dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        // 30 ms of resolution-dependent cost plus 8 ms that is not.
        let (settled, changes) = settle(dr, |s| 8.0 + 30.0 * s * s);
        assert!(
            changes < 40,
            "{changes} scale changes — that is pumping, not settling"
        );
        let cost = 8.0 + 30.0 * settled.scale() * settled.scale();
        assert!(
            cost <= settled.target_ms * 1.10 || settled.scale() <= settled.min_scale,
            "settled at {:.3} costing {cost:.2} ms",
            settled.scale()
        );
    }

    /// The bug this controller shipped with, as a test.
    ///
    /// The frame time it reads is a thirty-frame average, so for thirty frames
    /// after a change the reading describes a mixture of the old resolution and
    /// the new one — plus a resize transient that is cheaper than either. The
    /// first version reacted after fifteen frames and therefore decided on that
    /// mixture: it stepped down correctly on 37.65 ms, then read 28.97 ms (a
    /// number no real resolution produced), stepped back up, and repeated.
    ///
    /// The oscillation that shipped, reproduced from the measured numbers.
    ///
    /// Coastal ground at a 33 ms target. The observed log was:
    ///
    /// ```text
    /// frame_ms=37.62  →  step down to 2400x1304 (scale 0.9375)
    /// frame_ms=28.99  →  step up   to 2560x1392 (scale 1.0)
    /// …forever
    /// ```
    ///
    /// Neither decision is wrong on its own: 37.62 is above the high band and
    /// 28.99 is below the low band. **No reachable scale lands inside the
    /// band**, because one sixteenth of scale moves the frame by about 23%
    /// while the band is ±10%. Cooldowns cannot fix this — they only change how
    /// slowly it flips.
    #[test]
    fn it_does_not_oscillate_across_a_gap_no_scale_can_land_in() {
        let mut dr = DynamicResolution {
            enabled: true,
            target_ms: 33.0,
            min_scale: 0.5,
            ..Default::default()
        };
        // Fitted to the two real measurements: 37.62 ms at scale 1.0 and 28.99
        // at 0.9375, so the gap this test is about is the measured one and not
        // an invented one.
        let cost = |s: f32| {
            let (a, b) = (0.9375f32, 1.0f32);
            let (ca, cb) = (28.99f32, 37.62f32);
            let v = (cb - ca) / (b * b - a * a);
            (cb - v) + v * s * s
        };

        let mut flips = 0;
        let mut last_dir = 0i32;
        for _ in 0..5000 {
            let before = dr.scale();
            if dr.tick(cost(dr.scale())).is_some() {
                let dir = if dr.scale() < before { -1 } else { 1 };
                if last_dir != 0 && dir != last_dir {
                    flips += 1;
                }
                last_dir = dir;
            }
        }
        assert!(
            flips <= 1,
            "{flips} direction reversals — this is the limit cycle, not tuning"
        );
        // Settling on the safe side of the gap is the correct answer: slightly
        // lower resolution than strictly needed, and a stable image.
        assert!(
            cost(dr.scale()) <= dr.target_ms,
            "settled at {:.4} costing {:.2} ms, over a 33 ms budget",
            dr.scale(),
            cost(dr.scale())
        );
    }

    #[test]
    fn a_scene_that_gets_cheaper_lets_the_resolution_climb_again() {
        // The "too slow" memory must not be permanent, or walking out of an
        // expensive view would leave the resolution pinned low forever.
        let mut dr = DynamicResolution {
            enabled: true,
            target_ms: 33.0,
            min_scale: 0.5,
            ..Default::default()
        };
        for _ in 0..500 {
            dr.tick(45.0);
        }
        let lowered = dr.scale();
        assert!(lowered < 1.0, "never came down");
        for _ in 0..2000 {
            dr.tick(6.0);
        }
        assert!(
            dr.scale() > lowered,
            "stuck at {lowered} after the scene got cheap"
        );
    }

    #[test]
    fn the_measurement_window_is_reset_after_a_change_but_not_immediately() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        assert!(!dr.take_settle_due(), "nothing to settle before a change");
        assert!(dr.tick(80.0).is_some());
        // The frames right after a resize are the transient ones: FSR rebuilds
        // its context, TAA has no history. Resetting into those would seed the
        // fresh average with exactly the frames worth discarding.
        for frame in 0..SETTLE_FRAMES {
            assert!(!dr.take_settle_due(), "reset too early, at frame {frame}");
        }
        assert!(dr.take_settle_due(), "never asked for a reset");
        assert!(!dr.take_settle_due(), "asked for a second reset");
    }

    #[test]
    fn it_never_goes_below_the_users_floor() {
        let dr = DynamicResolution {
            enabled: true,
            min_scale: 0.75,
            ..Default::default()
        };
        // A frame that no achievable scale can rescue.
        let (settled, _) = settle(dr, |_| 500.0);
        assert!(
            settled.scale() >= 0.75 - 1e-4,
            "floor breached: {}",
            settled.scale()
        );
    }

    #[test]
    fn a_frame_inside_the_dead_band_is_left_alone() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        // ±2.5% is the Coastal frame's own measured standard deviation.
        for ms in [16.67, 17.0, 16.2, 17.8, 15.5] {
            assert_eq!(dr.tick(ms), None, "reacted to {ms} ms");
        }
    }

    #[test]
    fn one_spike_does_not_collapse_the_resolution() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        dr.tick(400.0);
        assert!(
            dr.scale() >= 1.0 - SCALE_STEP - 1e-4,
            "a single 400 ms frame dropped straight to {}",
            dr.scale()
        );
    }

    #[test]
    fn scaled_sizes_are_even_and_never_zero() {
        let mut dr = DynamicResolution {
            enabled: true,
            min_scale: 0.1,
            ..Default::default()
        };
        for _ in 0..4000 {
            dr.tick(200.0);
            let (w, h) = dr.apply(2560, 1392);
            assert!(w >= 2 && h >= 2, "{w}x{h}");
            assert_eq!(w % 2, 0, "odd width {w}");
            assert_eq!(h % 2, 0, "odd height {h}");
        }
        // A 1×1 window must not produce a zero-sized target either.
        assert_eq!(dr.apply(1, 1), (2, 2));
    }

    #[test]
    fn switching_off_restores_native() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        for _ in 0..500 {
            dr.tick(200.0);
        }
        assert!(dr.scale() < 1.0);
        dr.enabled = false;
        assert_eq!(dr.scale(), 1.0);
        assert_eq!(dr.apply(2560, 1392), (2560, 1392));
    }

    #[test]
    fn a_nonsense_frame_time_is_ignored_rather_than_acted_on() {
        let mut dr = DynamicResolution {
            enabled: true,
            ..Default::default()
        };
        for ms in [f32::NAN, f32::INFINITY, 0.0, -5.0] {
            assert_eq!(dr.tick(ms), None, "acted on {ms}");
        }
        assert_eq!(dr.scale(), 1.0);
    }
}
