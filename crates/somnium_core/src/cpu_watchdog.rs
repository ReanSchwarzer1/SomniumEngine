//! Notices when the CPU, not the GPU, is holding the frame rate down, in any
//! project and any level, and names the frame stage the time went to.
//!
//! Town (2026-09-25..27) ran CPU-bound for days before anyone could say why:
//! `world.entities()` scans every frame, components rewritten every frame
//! (which defeats change detection), draw lists rebuilt from scratch. Each is
//! invisible in a small scene and a wall in a big one, and graphics settings
//! cannot fix any of them. This is the tripwire: no profiler run needed, one
//! warning in the log naming the stage, at most once a minute.

/// What each cumulative stage mark in `App::about_to_wait` closes.
pub const STAGES: [&str; 6] = [
    "simulation and game update",
    "editor panels",
    "jobs, editor tools, transforms and game render hook",
    "world streaming and render submission",
    "authoring bridge",
    "log forwarding",
];

/// Frames slower than this (under 55 fps) are worth a look.
const SLOW_FRAME_MS: f32 = 1000.0 / 55.0;
/// Share of the frame spent working rather than waiting on the GPU or vsync.
const CPU_SHARE: f32 = 0.85;
/// How long the frame has to stay CPU-bound: a load hitch is not a finding.
const SUSTAIN_S: f32 = 3.0;
/// Quiet period after a report.
const REPEAT_S: f32 = 60.0;

/// One finding: the mean frame over the CPU-bound stretch.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuBoundReport {
    /// Frame to frame, milliseconds.
    pub wall_ms: f32,
    /// CPU work in that frame (waits on the GPU and vsync removed).
    pub work_ms: f32,
    /// Work per stage, slowest first.
    pub stages: Vec<(&'static str, f32)>,
}

/// Frame-by-frame state; `App` owns one and logs what [`Self::frame`] returns.
#[derive(Debug, Default)]
pub struct CpuWatchdog {
    bound_s: f32,
    quiet_s: f32,
    frames: u32,
    wall_ms: f64,
    work_ms: f64,
    stage_ms: [f64; 6],
}

impl CpuWatchdog {
    /// Feed one frame. `stage_ends_ms` are the cumulative stage marks from the
    /// frame's start; `wait_ms` is the time blocked on the GPU or vsync (in
    /// surface acquire and present), which the render stage contains.
    pub fn frame(
        &mut self,
        wall_ms: f32,
        stage_ends_ms: [f64; 6],
        wait_ms: f32,
    ) -> Option<CpuBoundReport> {
        let dt = wall_ms / 1000.0;
        self.quiet_s += dt;
        let mut stages = [0.0; 6];
        let mut previous = 0.0;
        for (stage, end) in stages.iter_mut().zip(stage_ends_ms) {
            *stage = (end - previous).max(0.0);
            previous = end;
        }
        stages[3] = (stages[3] - f64::from(wait_ms)).max(0.0);
        let work: f64 = stages.iter().sum();
        if wall_ms <= SLOW_FRAME_MS || (work as f32) < wall_ms * CPU_SHARE {
            self.bound_s = (self.bound_s - dt).max(0.0);
            if self.bound_s == 0.0 {
                *self = Self {
                    quiet_s: self.quiet_s,
                    ..Self::default()
                };
            }
            return None;
        }
        self.bound_s += dt;
        self.frames += 1;
        self.wall_ms += f64::from(wall_ms);
        self.work_ms += work;
        for (sum, stage) in self.stage_ms.iter_mut().zip(stages) {
            *sum += stage;
        }
        if self.bound_s < SUSTAIN_S || self.quiet_s < REPEAT_S {
            return None;
        }
        let n = f64::from(self.frames);
        let mut stages: Vec<_> = STAGES
            .iter()
            .zip(self.stage_ms)
            .map(|(name, ms)| (*name, (ms / n) as f32))
            .collect();
        stages.sort_by(|a, b| b.1.total_cmp(&a.1));
        let report = CpuBoundReport {
            wall_ms: (self.wall_ms / n) as f32,
            work_ms: (self.work_ms / n) as f32,
            stages,
        };
        *self = Self::default();
        Some(report)
    }
}

impl std::fmt::Display for CpuBoundReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CPU-bound: {:.1} ms of each {:.1} ms frame ({:.0} fps) is CPU work, so graphics \
             settings will not raise the frame rate. By stage:",
            self.work_ms,
            self.wall_ms,
            1000.0 / self.wall_ms.max(1e-3),
        )?;
        for (name, ms) in self.stages.iter().filter(|(_, ms)| *ms >= 0.5) {
            write!(f, " {name} {ms:.1} ms;")?;
        }
        write!(
            f,
            " the usual causes are a full-world entity scan per frame, a component written \
             every frame, or a cache rebuilt every frame (see World::change_signature)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cumulative marks for a frame whose render stage takes `render` ms.
    fn marks(render: f64) -> [f64; 6] {
        [2.0, 3.0, 4.0, 4.0 + render, 4.5 + render, 4.6 + render]
    }

    #[test]
    fn a_sustained_cpu_bound_frame_is_reported_once_with_its_slowest_stage() {
        let mut dog = CpuWatchdog {
            quiet_s: REPEAT_S,
            ..CpuWatchdog::default()
        };
        let mut reports = Vec::new();
        for _ in 0..400 {
            // 25 ms frames, 23.6 ms of them work: 40 fps, CPU-bound.
            reports.extend(dog.frame(25.0, marks(20.0), 1.0));
        }
        assert_eq!(reports.len(), 1, "then quiet for a minute");
        let r = &reports[0];
        assert!((r.work_ms - 23.6).abs() < 0.01, "{r:?}");
        assert_eq!(r.stages[0], ("world streaming and render submission", 19.0));
        assert!(r.to_string().contains("CPU-bound"));
    }

    #[test]
    fn a_gpu_bound_or_fast_frame_is_not_reported() {
        let mut dog = CpuWatchdog {
            quiet_s: REPEAT_S,
            ..CpuWatchdog::default()
        };
        for _ in 0..1000 {
            // 33 ms frames that spend 20 ms of it waiting on the GPU.
            assert_eq!(dog.frame(33.0, marks(24.0), 20.0), None);
            // A fast frame, however busy the CPU.
            assert_eq!(dog.frame(15.0, marks(10.0), 0.0), None);
        }
    }

    #[test]
    fn a_short_hitch_is_not_a_finding() {
        let mut dog = CpuWatchdog {
            quiet_s: REPEAT_S,
            ..CpuWatchdog::default()
        };
        for _ in 0..60 {
            assert_eq!(dog.frame(40.0, marks(35.0), 0.0), None); // 2.4 s of slow frames
        }
        for _ in 0..2000 {
            assert_eq!(dog.frame(16.0, marks(5.0), 10.0), None);
        }
    }
}
