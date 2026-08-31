//! Deterministic GPU timing runs and A/B comparison (Phase DOOM-A).
//!
//! `capture.rs` exists because screen-grabbing the window produced a frame-delta
//! metric that varied from 0.776 to 2.018 across three runs of one identical
//! build, and a whole session was spent chasing that variance instead of the
//! change. This file is the same argument applied to *time* rather than to
//! pixels.
//!
//! Phase CR and the 2026-08-14 occupancy session both ended with the same
//! instruction — do not spend another session flipping switches and reading fps
//! off the window — and neither could be followed without a clock that reports
//! its own noise. A number with no error bar cannot answer "did that 3% change
//! anything", which is the size of most of the wins Phase DOOM is chasing.
//!
//! # How a run works
//!
//! ```text
//! SOMNIUM_TIME=before.somtime SOMNIUM_MAXIMIZE=1 hello_engine
//! …change something…
//! SOMNIUM_TIME=after.somtime SOMNIUM_TIME_COMPARE=before.somtime hello_engine
//! ```
//!
//! The run discards a warm-up window (shader compilation, clipmap ring fill,
//! TAA/FSR history, auto-exposure adaptation — all of which are transient and
//! none of which belong in a steady-state number), then accumulates every
//! *unsmoothed* profiler sample for a fixed number of frames and writes mean,
//! standard deviation, min and max per zone.
//!
//! # What it deliberately does not do
//!
//! It does not move the camera. The canonical viewpoints in this project's
//! evidence — DF-A's overview and walk, XV-J's kit views, the Island and
//! Coastal recipes — are all stationary, and a stationary camera removes
//! terrain streaming, clipmap ring recentring and LOD transitions from the
//! measurement. A flythrough is a *different* experiment, and the one that
//! matters for hitches rather than for steady-state cost; DOOM-I is where that
//! belongs. Pin the viewpoint with the existing `SOMNIUM_CAMERA_POS`,
//! `SOMNIUM_CAMERA_YAW`, `SOMNIUM_CAMERA_PITCH`, `SOMNIUM_KIT_VIEW` and
//! `SOMNIUM_VIEWPORT_RES`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::pass::census::{BIN_NAMES, CensusResult};
use crate::profiler::{FrameCounters, GpuProfiler, ScopeResult, StatsResult};

/// Set once a run has been written. `SOMNIUM_TIME_QUIT=1` polls this to exit a
/// headless run rather than sitting on a window, mirroring
/// `SOMNIUM_CAPTURE_QUIT`.
static TIMING_FINISHED: AtomicBool = AtomicBool::new(false);

/// True once a timing run has written its file.
pub fn finished() -> bool {
    TIMING_FINISHED.load(Ordering::Acquire)
}

/// Frames discarded before measurement starts.
///
/// Generous on purpose. Pipeline creation, the first clipmap ring fill and
/// FSR/TAA history are all one-time costs that would otherwise land in the
/// mean and make an early run look worse than a late one.
const DEFAULT_WARMUP: u64 = 180;

/// Frames measured after the warm-up.
const DEFAULT_FRAMES: u64 = 300;

/// Running statistics for one zone.
///
/// Welford would be more numerically careful, but these are milliseconds in a
/// range of roughly 0.001 to 100 over a few hundred samples, where the naive
/// sum of squares is exact enough and reads more plainly.
#[derive(Clone, Debug, Default)]
struct Accum {
    n: u64,
    sum: f64,
    sum_sq: f64,
    min: f32,
    max: f32,
}

impl Accum {
    fn push(&mut self, v: f32) {
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.n += 1;
        self.sum += f64::from(v);
        self.sum_sq += f64::from(v) * f64::from(v);
    }

    fn mean(&self) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            (self.sum / self.n as f64) as f32
        }
    }

    /// Population standard deviation, floored at zero.
    ///
    /// `sum_sq/n - mean²` can come out very slightly negative through
    /// cancellation when every sample is identical, and a NaN from `sqrt` would
    /// propagate into the comparison and silently disable the noise band.
    fn stddev(&self) -> f32 {
        if self.n < 2 {
            return 0.0;
        }
        let mean = self.sum / self.n as f64;
        let var = (self.sum_sq / self.n as f64) - mean * mean;
        (var.max(0.0)).sqrt() as f32
    }
}

/// One row of a written or parsed run.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// `gpu`, `cpu`, `count` or `stat`.
    pub kind: String,
    pub name: String,
    pub depth: u8,
    pub mean: f32,
    pub stddev: f32,
    pub min: f32,
    pub max: f32,
    pub samples: u64,
}

/// A parsed `.somtime` file.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub header: Vec<String>,
    pub rows: Vec<Row>,
}

impl Run {
    fn find(&self, kind: &str, name: &str, depth: u8) -> Option<&Row> {
        self.rows
            .iter()
            .find(|r| r.kind == kind && r.name == name && r.depth == depth)
    }
}

/// Parse a `.somtime` file.
///
/// Unknown lines are ignored rather than rejected, so a file written by a later
/// version with extra rows still compares on the rows both understand. A run
/// that cannot be compared at all is worse than one compared partially.
#[must_use]
pub fn parse(text: &str) -> Run {
    let mut run = Run::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('#') {
            run.header.push(rest.trim().to_string());
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            continue;
        }
        let Ok(depth) = f[2].parse::<u8>() else {
            continue;
        };
        run.rows.push(Row {
            kind: f[0].to_string(),
            name: f[1].to_string(),
            depth,
            mean: f[3].parse().unwrap_or(0.0),
            stddev: f[4].parse().unwrap_or(0.0),
            min: f[5].parse().unwrap_or(0.0),
            max: f[6].parse().unwrap_or(0.0),
            samples: f[7].parse().unwrap_or(0),
        });
    }
    run
}

/// Render a comparison of two runs as report lines.
///
/// The **noise band** is the quadrature sum of the two standard deviations. A
/// delta inside it is reported as `~` and must not be quoted as a win: it is
/// exactly the class of number that produced "0.776 to 2.018 across three runs
/// of one identical build". A delta outside it gets a sign and a percentage.
#[must_use]
pub fn compare(before: &Run, after: &Run) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "{:<28} {:>9} {:>9} {:>9} {:>8}  {}",
        "zone", "before", "after", "delta", "pct", "verdict"
    ));
    for a in &after.rows {
        if a.kind != "gpu" && a.kind != "cpu" {
            continue;
        }
        let Some(b) = before.find(&a.kind, &a.name, a.depth) else {
            out.push(format!("{:<28} {:>9} {:>9.3}   (new)", a.name, "—", a.mean));
            continue;
        };
        let delta = a.mean - b.mean;
        let pct = if b.mean.abs() > 1e-6 {
            delta / b.mean * 100.0
        } else {
            0.0
        };
        let band = (a.stddev * a.stddev + b.stddev * b.stddev).sqrt();
        let verdict = if delta.abs() <= band {
            "~ noise"
        } else if delta < 0.0 {
            "faster"
        } else {
            "SLOWER"
        };
        let indent = "  ".repeat(a.depth as usize);
        out.push(format!(
            "{indent}{:<28} {:>9.3} {:>9.3} {:>+9.3} {:>+7.1}%  {verdict} (band ±{band:.3})",
            a.name,
            b.mean,
            a.mean,
            delta,
            pct,
            band = band,
        ));
    }
    for b in &before.rows {
        if b.kind != "gpu" && b.kind != "cpu" {
            continue;
        }
        if after.find(&b.kind, &b.name, b.depth).is_none() {
            out.push(format!(
                "{:<28} {:>9.3} {:>9}   (gone)",
                b.name, b.mean, "—"
            ));
        }
    }
    // Hitches before the counters, and without a noise band. A band is a claim
    // about a mean; these are order statistics and a count, and the movement
    // worth seeing in them is in the tail, which a band would hide.
    for a in after.rows.iter().filter(|r| r.kind == "hitch") {
        let Some(b) = before.find("hitch", &a.name, 0) else {
            continue;
        };
        out.push(format!(
            "{:<28} {:>9.3} {:>9.3} {:>+9.3}          hitch",
            a.name,
            b.mean,
            a.mean,
            a.mean - b.mean
        ));
    }
    // Counters last, and never given a noise band: they are exact integers, so
    // "different" and "different by more than the noise" are the same question.
    for a in after
        .rows
        .iter()
        .filter(|r| r.kind == "count" || r.kind == "census")
    {
        let b = before.find(&a.kind, &a.name, 0).map_or(0.0, |r| r.mean);
        if (a.mean - b).abs() > 0.5 {
            out.push(format!(
                "{:<28} {:>9.0} {:>9.0} {:>+9.0}          {}",
                a.name,
                b,
                a.mean,
                a.mean - b,
                a.kind
            ));
        }
    }
    out
}

/// Live GPU object counts and memory, sampled once per frame (DOOM-J).
///
/// Read from `wgpu`'s own internal counters rather than from wrappers around
/// two hundred and sixty-five `create_*` call sites. These are **gauges** — one
/// increment per creation, one decrement per destruction — so what they answer
/// is *"does a steady-state frame change the set of live GPU objects"*, which
/// is DOOM-J's exit criterion. They cannot see a resource created and destroyed
/// inside one frame; a per-frame delta can, which is why the delta and not the
/// endpoint difference is what gets reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationSnapshot {
    /// Live buffers.
    pub buffers: i64,
    /// Live textures.
    pub textures: i64,
    /// Live texture views.
    pub texture_views: i64,
    /// Live bind groups.
    pub bind_groups: i64,
    /// Live samplers.
    pub samplers: i64,
    /// Distinct GPU memory allocations held by the allocator.
    pub memory_allocations: i64,
    /// Bytes attributed to buffers.
    pub buffer_bytes: i64,
    /// Bytes attributed to textures.
    pub texture_bytes: i64,
}

impl AllocationSnapshot {
    /// Sample the device's counters.
    #[must_use]
    pub fn read(device: &wgpu::Device) -> Self {
        let hal = device.get_internal_counters().hal;
        Self {
            buffers: hal.buffers.read() as i64,
            textures: hal.textures.read() as i64,
            texture_views: hal.texture_views.read() as i64,
            bind_groups: hal.bind_groups.read() as i64,
            samplers: hal.samplers.read() as i64,
            memory_allocations: hal.memory_allocations.read() as i64,
            buffer_bytes: hal.buffer_memory.read() as i64,
            texture_bytes: hal.texture_memory.read() as i64,
        }
    }

    /// The object counts, named, in a fixed order.
    ///
    /// Bytes are excluded on purpose: a buffer that is written but not
    /// reallocated does not move them, and one that grows in place is still one
    /// object. The question is object churn.
    #[must_use]
    pub fn objects(&self) -> [(&'static str, i64); 6] {
        [
            ("buffers", self.buffers),
            ("textures", self.textures),
            ("texture_views", self.texture_views),
            ("bind_groups", self.bind_groups),
            ("samplers", self.samplers),
            ("memory_allocations", self.memory_allocations),
        ]
    }
}

/// A run's hitch profile: the typical frame, the tail, and what exceeded it.
///
/// A *hitch* is defined here as a presented-frame interval above **twice the
/// run's own median**, deliberately a relative threshold. An absolute one would
/// call every frame of a 30 fps scene a hitch and none of a 240 fps one, and
/// the thing being measured is *"the frame rate visibly broke step"*, not *"the
/// frame took longer than a number picked in advance"*.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hitches {
    /// Median presented-frame interval, in milliseconds.
    pub median_ms: f32,
    /// 99th percentile interval, in milliseconds.
    pub p99_ms: f32,
    /// Longest interval in the run.
    pub worst_ms: f32,
    /// Frames whose interval exceeded `2 x median_ms`.
    pub over_2x: u64,
    /// Measured-frame index of the longest interval.
    pub worst_frame: u64,
    /// Measured-frame index of the *last* hitch, or zero if there were none.
    ///
    /// With `worst_frame`, this is what separates the two diagnoses that matter
    /// and that a count alone cannot tell apart: hitches clustered at the front
    /// of a run are one-off startup cost — pipeline compilation, the first
    /// streaming burst — while hitches that keep arriving are a steady-state
    /// fault. Naming which one a run has is DOOM-I's whole job.
    pub last_over_2x_frame: u64,
    /// Intervals considered.
    pub samples: u64,
}

/// Summarise wall intervals into a [`Hitches`].
///
/// Returns `None` below eight samples, where a median is a coin toss and a
/// "0 hitches" row would be a claim the run cannot support.
#[must_use]
pub fn hitches(intervals: &[f32]) -> Option<Hitches> {
    if intervals.len() < 8 {
        return None;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_by(f32::total_cmp);
    let median = percentile(&sorted, 0.50);
    let threshold = median * 2.0;
    let mut over_2x = 0;
    let mut worst_frame = 0;
    let mut last_over_2x_frame = 0;
    for (index, &value) in intervals.iter().enumerate() {
        if value > threshold {
            over_2x += 1;
            last_over_2x_frame = index as u64;
        }
        if value > intervals[worst_frame] {
            worst_frame = index;
        }
    }
    Some(Hitches {
        median_ms: median,
        p99_ms: percentile(&sorted, 0.99),
        worst_ms: *sorted.last().unwrap_or(&0.0),
        over_2x,
        worst_frame: worst_frame as u64,
        last_over_2x_frame,
        samples: intervals.len() as u64,
    })
}

/// Nearest-rank percentile over an ascending slice.
fn percentile(sorted: &[f32], q: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// Accumulates a run and writes it out.
pub struct TimingRun {
    path: String,
    compare_path: Option<String>,
    label: String,
    warmup: u64,
    frames: u64,
    /// Profiler samples taken so far (not rendered frames — readback lands late
    /// and not once per frame).
    samples: u64,
    /// Rendered frames since the run started.
    rendered: u64,
    last_serial: u64,
    gpu: BTreeMap<(String, u8), Accum>,
    cpu: BTreeMap<(String, u8), Accum>,
    /// Wall-clock interval between presented frames.
    ///
    /// **This is not CPU work, and was read as CPU work before PORTAL-0-B.**
    /// The swap chain is configured `PresentMode::AutoVsync`
    /// (`context.rs`), so this interval contains the presentation block as well
    /// as everything the engine did. A run whose GPU `Frame` is 10 ms and whose
    /// wall is 19 ms is not evidence of 9 ms of CPU work; `Frame CPU` and
    /// `Surface acquire` below are what answer that. It remains the right
    /// number for a hitch baseline — CONTROL-A/C use it for the synchronous
    /// thumbnail decoder — because a hitch is an interval, whatever caused it.
    wall_frame: Accum,
    /// Renderer construction to the first presented frame, in milliseconds.
    ///
    /// The largest stall in any session, and until DOOM-I it was reported by
    /// nothing: the first tick had no previous tick, so its interval was
    /// dropped. That produced runs whose `Frame CPU` maximum was 120 ms beside
    /// a `Frame wall` maximum of 31.9 ms — impossible of one frame, and true
    /// only because the frame in question had no interval at all.
    ///
    /// Kept out of `wall_frame` and `wall_samples` deliberately. Folding an
    /// eight-second outlier into a mean and a standard deviation destroys both
    /// — it took `Frame wall` from 20.0 ± 2.1 ms to 33.7 ± 336.5 ms — and every
    /// comparison against an earlier run would have been reading a different
    /// statistic. Startup is a different question from the frame rate, so it
    /// gets its own row.
    startup_ms: Option<f32>,
    /// Allocation gauges as of the previous frame, for the per-frame delta.
    previous_alloc: Option<AllocationSnapshot>,
    /// Allocation gauges at the first measured frame and the most recent one.
    first_alloc: Option<AllocationSnapshot>,
    last_alloc: Option<AllocationSnapshot>,
    /// Measured frames in which any live object count moved at all.
    alloc_churn_frames: u64,
    /// Largest single-frame movement in any object count.
    alloc_worst_frame_delta: i64,
    /// Measured frames in which each object count moved, in `objects()` order.
    alloc_churn_by_object: [u64; 6],
    /// `SOMNIUM_ALLOC_TRACE=1`: name what churns, not only how often.
    alloc_trace: bool,
    previous_alloc_names: Option<BTreeMap<String, i64>>,
    /// Every wall interval *after* the first, kept rather than summarised.
    ///
    /// DOOM-I's exit criterion is a *hitch* metric — "no frame above 2x the
    /// median" — and neither half of that is recoverable from `wall_frame`'s
    /// mean and standard deviation. A median needs the samples, and a count of
    /// frames over a threshold needs the threshold, which is not known until
    /// the run ends. Six hundred `f32`s is 2.4 KB.
    wall_samples: Vec<f32>,
    /// Engine frame body on the CPU, excluding the frame limiter (PORTAL-0-B).
    frame_cpu: Accum,
    /// Of which, blocked acquiring the swap-chain texture (PORTAL-0-B).
    surface_acquire: Accum,
    last_tick_at: Option<Instant>,
    counters: FrameCounters,
    stats: Vec<StatsResult>,
    /// Phase DOOM-B pixel counts, zero unless `SOMNIUM_CENSUS=1`.
    census: CensusResult,
    written: bool,
}

impl TimingRun {
    /// Build a run from the environment, or `None` when `SOMNIUM_TIME` is unset.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let path = std::env::var("SOMNIUM_TIME").ok()?;
        let num = |key: &str, default: u64| -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        Some(Self {
            path,
            compare_path: std::env::var("SOMNIUM_TIME_COMPARE").ok(),
            label: std::env::var("SOMNIUM_TIME_LABEL").unwrap_or_else(|_| "unlabelled".into()),
            warmup: num("SOMNIUM_TIME_WARMUP", DEFAULT_WARMUP),
            frames: num("SOMNIUM_TIME_FRAMES", DEFAULT_FRAMES),
            samples: 0,
            rendered: 0,
            last_serial: 0,
            gpu: BTreeMap::new(),
            cpu: BTreeMap::new(),
            wall_frame: Accum::default(),
            wall_samples: Vec::new(),
            startup_ms: None,
            previous_alloc: None,
            first_alloc: None,
            last_alloc: None,
            alloc_churn_frames: 0,
            alloc_worst_frame_delta: 0,
            alloc_churn_by_object: [0; 6],
            alloc_trace: std::env::var("SOMNIUM_ALLOC_TRACE").as_deref() == Ok("1"),
            previous_alloc_names: None,
            frame_cpu: Accum::default(),
            surface_acquire: Accum::default(),
            // Seeded, not `None`. `from_env` runs while the renderer is being
            // built, so this makes the *first* recorded interval cover device
            // and pipeline creation, map load and the first present — the
            // largest stall in any session, and the one a `None` first tick
            // silently discarded. DOOM-I found it that way: a run reported a
            // 120 ms `Frame CPU` maximum next to a 31.9 ms `Frame wall`
            // maximum, which cannot both be true of the same frame, and the
            // reason was that the frame in question had no interval at all.
            //
            // It is reported as `hitch startup_ms` and kept out of the frame
            // statistics; see `TimingRun::startup_ms` for why folding it in
            // was tried and reverted.
            last_tick_at: Some(Instant::now()),
            counters: FrameCounters::default(),
            stats: Vec::new(),
            census: CensusResult::default(),
            written: false,
        })
    }

    /// True while the run still wants frames.
    pub fn active(&self) -> bool {
        !self.written
    }

    /// Take one frame's worth of samples. Call once per rendered frame, after
    /// the profiler has had a chance to collect.
    ///
    /// Returns true on the frame the run completes.
    pub fn tick(
        &mut self,
        profiler: &GpuProfiler,
        census: CensusResult,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        size: (u32, u32),
    ) {
        if self.written {
            return;
        }
        let now = Instant::now();
        let wall_ms = self
            .last_tick_at
            .replace(now)
            .map(|before| before.elapsed().as_secs_f32() * 1000.0);
        self.rendered += 1;
        if self.rendered <= self.warmup {
            return;
        }
        if let Some(ms) = wall_ms {
            if self.rendered == 1 {
                self.startup_ms = Some(ms);
            } else {
                self.wall_frame.push(ms);
                self.wall_samples.push(ms);
            }
        }
        // PORTAL-0-B: pushed beside the wall interval rather than with the GPU
        // zones, because both are produced once per rendered frame and neither
        // waits on readback. `frame_cpu` lags by one frame by construction —
        // see `GpuProfiler::frame_cpu_ms` — which is invisible in a mean over
        // hundreds of stationary frames and is stated here rather than hidden.
        if profiler.frame_cpu_ms > 0.0 {
            self.frame_cpu.push(profiler.frame_cpu_ms);
        }
        self.surface_acquire.push(profiler.surface_acquire_ms);

        // DOOM-J. Sampled every measured frame rather than only at the ends,
        // because a resource created on one frame and released on the next
        // nets to nothing over a window and is still churn.
        let alloc = AllocationSnapshot::read(device);
        if let Some(previous) = self.previous_alloc {
            let mut moved = 0;
            // Per counter, not only in aggregate. "68 of 300 frames churned"
            // is not something anybody can act on; "68 of them moved
            // `textures`" names the subsystem.
            for (index, ((_, now), (_, before))) in alloc
                .objects()
                .iter()
                .zip(previous.objects().iter())
                .enumerate()
            {
                let delta = (now - before).abs();
                if delta > 0 {
                    self.alloc_churn_by_object[index] += 1;
                    moved = moved.max(delta);
                }
            }
            if moved > 0 {
                self.alloc_churn_frames += 1;
                self.alloc_worst_frame_delta = self.alloc_worst_frame_delta.max(moved);
            }
        }
        // `SOMNIUM_ALLOC_TRACE=1` names the churn. The counters say *that* one
        // buffer appeared and one went away; only the allocator report says
        // *which*, because it carries the label each resource was created with.
        // Opt-in because it rebuilds a multiset of every live allocation's name
        // once a frame, which is far too much work to leave on.
        if self.alloc_trace
            && let Some(report) = device.generate_allocator_report()
        {
            let mut names: BTreeMap<String, i64> = BTreeMap::new();
            for allocation in &report.allocations {
                *names.entry(allocation.name.clone()).or_default() += 1;
            }
            if let Some(previous) = &self.previous_alloc_names {
                for (name, count) in &names {
                    let before = previous.get(name).copied().unwrap_or(0);
                    if *count != before {
                        tracing::info!(frame = self.rendered, %name, before, now = count, "alloc churn");
                    }
                }
                for (name, before) in previous {
                    if !names.contains_key(name) {
                        tracing::info!(frame = self.rendered, %name, before, now = 0, "alloc churn");
                    }
                }
            }
            self.previous_alloc_names = Some(names);

            // The other half of DOOM-J's gate is an *inventory*, not only a
            // churn count. Logged once, on the last measured frame, because
            // what it answers — where the gigabyte went — does not change
            // frame to frame.
            if self.rendered == self.warmup + self.frames {
                let mut bytes: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
                for allocation in &report.allocations {
                    let entry = bytes.entry(allocation.name.as_str()).or_default();
                    entry.0 += allocation.size;
                    entry.1 += 1;
                }
                let mut rows: Vec<_> = bytes.into_iter().collect();
                rows.sort_by_key(|(_, (size, _))| std::cmp::Reverse(*size));
                tracing::info!(
                    total_mib = report.total_allocated_bytes as f64 / (1024.0 * 1024.0),
                    reserved_mib = report.total_reserved_bytes as f64 / (1024.0 * 1024.0),
                    allocations = report.allocations.len(),
                    blocks = report.blocks.len(),
                    "alloc inventory"
                );
                for (name, (size, count)) in rows.into_iter().take(20) {
                    tracing::info!(
                        mib = size as f64 / (1024.0 * 1024.0),
                        count,
                        %name,
                        "alloc inventory row"
                    );
                }
            }
        }
        self.previous_alloc = Some(alloc);
        self.first_alloc.get_or_insert(alloc);
        self.last_alloc = Some(alloc);

        let (serial, raw) = profiler.raw_sample();
        // Same serial means the readback ring has not produced a new frame yet.
        // Counting it again would shrink the reported standard deviation
        // towards zero and make every comparison look significant.
        if serial != self.last_serial && !raw.is_empty() {
            self.last_serial = serial;
            self.samples += 1;
            for r in raw {
                self.gpu
                    .entry((r.name.to_string(), r.depth))
                    .or_default()
                    .push(r.ms);
            }
            // PORTAL-0-B: raw, not `cpu_results()`. The smoothed series has a
            // standard deviation of the smoother rather than of the work.
            for r in profiler.cpu_raw_results() {
                self.cpu
                    .entry((r.name.to_string(), r.depth))
                    .or_default()
                    .push(r.ms);
            }
            self.counters = profiler.last_counters;
            self.stats = profiler.stats_results().to_vec();
            // Last value wins rather than an average: on a stationary viewpoint
            // the census is the same every frame, and averaging integer pixel
            // counts would only invent fractions of a pixel.
            self.census = census;
        }

        if self.rendered >= self.warmup + self.frames {
            self.finish(adapter, size);
        }
    }

    fn finish(&mut self, adapter: &wgpu::Adapter, size: (u32, u32)) {
        self.written = true;
        let text = self.render(adapter, size);
        // A run costs a warm-up plus several hundred frames. Losing it to a
        // missing `dev records/phase DOOM/` is not a trade worth making, and
        // the evidence folders in this project are created by the sub-phase
        // that first produces evidence anyway.
        if let Some(parent) = std::path::Path::new(&self.path).parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("TIMING could not create {}: {e}", parent.display());
        }
        match std::fs::write(&self.path, &text) {
            Ok(()) => tracing::info!(
                "TIMING wrote {} ({} samples over {} frames)",
                self.path,
                self.samples,
                self.frames
            ),
            Err(e) => tracing::error!("TIMING could not write {}: {e}", self.path),
        }
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            tracing::info!("TIMING {line}");
        }

        // The headline, because it is DOOM-A's exit criterion and nobody should
        // have to sum a column by hand to find out whether the profiler can see
        // the frame yet.
        let self_run = parse(&text);
        let frame = frame_ms(&self_run);
        match unattributed_pct(&self_run) {
            Some(pct) => tracing::info!(
                "TIMING frame {frame:.3} ms, unattributed {pct:.1}% ({})",
                if pct < 5.0 {
                    "DOOM-A gate met"
                } else {
                    "DOOM-A gate NOT met — passes still unbracketed"
                }
            ),
            None => tracing::warn!("TIMING no `Frame` scope — cannot report unattributed"),
        }

        if let Some(prev) = self.compare_path.clone() {
            match std::fs::read_to_string(&prev) {
                Ok(before) => {
                    let after = parse(&text);
                    tracing::info!("TIMING compare {prev} → {}", self.path);
                    for line in compare(&parse(&before), &after) {
                        tracing::info!("TIMING {line}");
                    }
                }
                Err(e) => tracing::error!("TIMING could not read {prev}: {e}"),
            }
        }
        TIMING_FINISHED.store(true, Ordering::Release);
    }

    fn render(&self, adapter: &wgpu::Adapter, size: (u32, u32)) -> String {
        let info = adapter.get_info();
        let mut s = String::new();
        let _ = writeln!(s, "# somtime v1");
        let _ = writeln!(s, "# label\t{}", self.label);
        let _ = writeln!(
            s,
            "# frames\t{} measured after {} warmup, {} profiler samples",
            self.frames, self.warmup, self.samples
        );
        let _ = writeln!(s, "# adapter\t{} / {:?}", info.name, info.backend);
        let _ = writeln!(s, "# driver\t{} {}", info.driver, info.driver_info);
        let _ = writeln!(s, "# render\t{}x{}", size.0, size.1);
        let _ = writeln!(
            s,
            "# columns\tkind\tname\tdepth\tmean_ms\tstddev_ms\tmin_ms\tmax_ms\tsamples"
        );

        for ((name, depth), a) in &self.gpu {
            let _ = writeln!(
                s,
                "gpu\t{name}\t{depth}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}",
                a.mean(),
                a.stddev(),
                a.min,
                a.max,
                a.n
            );
        }
        for ((name, depth), a) in &self.cpu {
            let _ = writeln!(
                s,
                "cpu\t{name}\t{depth}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}",
                a.mean(),
                a.stddev(),
                a.min,
                a.max,
                a.n
            );
        }
        // PORTAL-0-B: three frame-level rows, written together because they
        // only mean anything read together. `Frame wall` is the interval,
        // `Frame CPU` is what the engine did inside it, and `Surface acquire`
        // is the presentation block inside `Frame CPU`. Wall minus CPU is the
        // frame limiter; CPU minus acquire is real work.
        for (name, a) in [
            ("Frame wall", &self.wall_frame),
            ("Frame CPU", &self.frame_cpu),
            ("Surface acquire", &self.surface_acquire),
        ] {
            if a.n == 0 {
                continue;
            }
            let _ = writeln!(
                s,
                "cpu\t{name}\t0\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}",
                a.mean(),
                a.stddev(),
                a.min,
                a.max,
                a.n
            );
        }

        // DOOM-I. A separate kind rather than more `count` rows, for the same
        // reason DOOM-B's census got its own: a comparison should show the
        // frame rate breaking step separately from the scene changing size,
        // because those are different questions with different fixes.
        if let Some(ms) = self.startup_ms {
            let _ = writeln!(s, "hitch	startup_ms	0	{ms:.4}	0.0000	{ms:.4}	{ms:.4}	1");
        }
        if let Some(h) = hitches(&self.wall_samples) {
            for (name, v) in [
                ("median_ms", h.median_ms),
                ("p99_ms", h.p99_ms),
                ("worst_ms", h.worst_ms),
                ("over_2x_median", h.over_2x as f32),
                ("worst_frame", h.worst_frame as f32),
                ("last_over_2x_frame", h.last_over_2x_frame as f32),
            ] {
                let _ = writeln!(
                    s,
                    "hitch	{name}	0	{v:.4}	0.0000	{v:.4}	{v:.4}	{}",
                    h.samples
                );
            }
        }

        // DOOM-J's inventory. `alloc_churn_frames` and `alloc_worst_frame_delta`
        // are the gate; the `live_*` rows are the inventory it is a gate over,
        // and a comparison shows both moving together when something changes.
        if let (Some(first), Some(last)) = (self.first_alloc, self.last_alloc) {
            const MIB: f32 = 1024.0 * 1024.0;
            for (name, v) in [
                ("alloc_churn_frames", self.alloc_churn_frames as i64),
                ("alloc_worst_frame_delta", self.alloc_worst_frame_delta),
                ("alloc_net_buffers", last.buffers - first.buffers),
                ("alloc_net_textures", last.textures - first.textures),
                (
                    "alloc_net_bind_groups",
                    last.bind_groups - first.bind_groups,
                ),
                ("live_buffers", last.buffers),
                ("live_textures", last.textures),
                ("live_texture_views", last.texture_views),
                ("live_bind_groups", last.bind_groups),
                ("live_samplers", last.samplers),
                ("live_memory_allocations", last.memory_allocations),
            ] {
                let _ = writeln!(s, "count	{name}	0	{v}	0	{v}	{v}	1");
            }
            for (index, (name, _)) in last.objects().iter().enumerate() {
                let v = self.alloc_churn_by_object[index];
                if v > 0 {
                    let _ = writeln!(s, "count	churn_{name}	0	{v}	0	{v}	{v}	1");
                }
            }
            for (name, bytes) in [
                ("live_buffer_mib", last.buffer_bytes),
                ("live_texture_mib", last.texture_bytes),
            ] {
                let v = bytes as f32 / MIB;
                let _ = writeln!(s, "count	{name}	0	{v:.1}	0	{v:.1}	{v:.1}	1");
            }
        }

        let c = self.counters;
        for (name, v) in [
            ("draw_calls", c.draw_calls),
            ("triangles", c.triangles),
            ("instances", c.instances),
            ("terrain_chunks", c.terrain_chunks),
            ("terrain_cpu_culled", c.terrain_cpu_culled),
            ("tlas_instances", c.tlas_instances),
            ("shadow_casters", c.shadow_casters),
            ("shadow_cascades_rendered", c.shadow_cascades_rendered),
            ("virtual_shadow_pages", c.virtual_shadow_pages),
            ("virtual_shadow_resident", c.virtual_shadow_resident),
        ] {
            let _ = writeln!(s, "count\t{name}\t0\t{v}\t0\t{v}\t{v}\t1");
        }
        // Phase DOOM-B. Written as `census` rows rather than folded into
        // `count` so a comparison can show pixel-class movement separately from
        // draw-call movement — the two answer different questions.
        if self.census.counts[BIN_NAMES.len() - 1] > 0 {
            for (i, name) in BIN_NAMES.iter().enumerate() {
                let v = self.census.counts[i];
                let _ = writeln!(
                    s,
                    "census\t{name}\t0\t{v}\t{:.4}\t{v}\t{v}\t1",
                    self.census.pct(i)
                );
            }
        }
        for st in &self.stats {
            let _ = writeln!(
                s,
                "stat\t{}.frag\t0\t{}\t0\t{}\t{}\t1",
                st.name, st.fragment_invocations, st.fragment_invocations, st.fragment_invocations
            );
            let _ = writeln!(
                s,
                "stat\t{}.prim\t0\t{}\t0\t{}\t{}\t1",
                st.name, st.clipper_primitives, st.clipper_primitives, st.clipper_primitives
            );
        }
        s
    }
}

/// The share of the frame no scope claims, as a percentage.
///
/// DOOM-A's exit criterion is that this falls below 5%. Computed from a written
/// run rather than live, because the live overlay smooths and this has to be
/// the same number a reviewer can recompute from the file.
#[must_use]
pub fn unattributed_pct(run: &Run) -> Option<f32> {
    let frame_row = run.find("gpu", "Frame", 0)?;
    let frame = frame_row.mean;
    if frame <= 0.0 {
        return None;
    }
    // MORROWIND-J step 3. A scope's mean is per *occurrence*, and since a frame
    // records the scene once per view a pass can occur four times in one frame.
    // Summing the means would then account for a quarter of the work and report
    // 75% unattributed — indistinguishable from an engine full of unbracketed
    // passes, and the more alarming reading of the two. The occurrence count is
    // already in the file: a row's samples over the frame's.
    let frames = frame_row.samples.max(1) as f32;
    let children: f32 = run
        .rows
        .iter()
        .filter(|r| r.kind == "gpu" && r.depth == 1)
        .map(|r| r.mean * (r.samples as f32 / frames))
        .sum();
    Some(((frame - children).max(0.0) / frame) * 100.0)
}

/// Sum of the top-level GPU scopes in a parsed run.
#[must_use]
pub fn frame_ms(run: &Run) -> f32 {
    run.rows
        .iter()
        .filter(|r| r.kind == "gpu" && r.depth == 0)
        .map(|r| r.mean)
        .sum()
}

/// Turn a slice of scope results into rows, for tests and for callers that
/// already have a sample in hand.
#[must_use]
pub fn rows_from_scopes(kind: &str, scopes: &[ScopeResult]) -> Vec<Row> {
    scopes
        .iter()
        .map(|s| Row {
            kind: kind.to_string(),
            name: s.name.to_string(),
            depth: s.depth,
            mean: s.ms,
            stddev: 0.0,
            min: s.ms,
            max: s.ms,
            samples: 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DOOM-I: the hitch metric ───────────────────────────────────────────

    #[test]
    fn a_steady_run_reports_no_hitches() {
        let steady: Vec<f32> = (0..600).map(|i| 16.6 + (i % 3) as f32 * 0.1).collect();
        let h = hitches(&steady).expect("enough samples");
        assert_eq!(h.over_2x, 0);
        assert!((h.median_ms - 16.7).abs() < 0.2, "median {}", h.median_ms);
        assert!(h.worst_ms < 17.0);
    }

    #[test]
    fn a_spike_is_counted_and_does_not_move_the_median() {
        // One 120 ms frame in 600. A mean would absorb it; the whole point of
        // the metric is that it does not.
        let mut run: Vec<f32> = vec![16.6; 600];
        run[300] = 120.0;
        let h = hitches(&run).expect("enough samples");
        assert_eq!(h.over_2x, 1);
        assert!((h.median_ms - 16.6).abs() < 0.001);
        assert!((h.worst_ms - 120.0).abs() < 0.001);
        assert_eq!(h.worst_frame, 300);
        assert_eq!(h.last_over_2x_frame, 300);
    }

    #[test]
    fn where_the_hitches_are_separates_startup_from_steady_state() {
        // Startup: everything over the threshold is in the first few frames.
        let mut startup: Vec<f32> = vec![16.6; 300];
        for frame in [1usize, 4, 9] {
            startup[frame] = 90.0;
        }
        let h = hitches(&startup).unwrap();
        assert_eq!(h.over_2x, 3);
        assert_eq!(h.last_over_2x_frame, 9);

        // Steady-state fault: the last one arrives near the end of the run.
        let mut ongoing: Vec<f32> = vec![16.6; 300];
        for frame in [1usize, 150, 280] {
            ongoing[frame] = 90.0;
        }
        assert_eq!(hitches(&ongoing).unwrap().last_over_2x_frame, 280);
    }

    #[test]
    fn the_threshold_is_relative_to_the_run() {
        // The same absolute 40 ms frame is a hitch at 120 fps and ordinary at
        // 30 fps. An absolute threshold could not say both.
        let mut fast: Vec<f32> = vec![8.3; 100];
        fast[10] = 40.0;
        assert_eq!(hitches(&fast).unwrap().over_2x, 1);

        let mut slow: Vec<f32> = vec![33.3; 100];
        slow[10] = 40.0;
        assert_eq!(hitches(&slow).unwrap().over_2x, 0);
    }

    #[test]
    fn too_few_samples_report_nothing_rather_than_zero() {
        assert!(hitches(&[16.6; 7]).is_none());
        assert!(hitches(&[16.6; 8]).is_some());
    }

    fn run_of(rows: &[(&str, u8, f32, f32)]) -> Run {
        Run {
            header: Vec::new(),
            rows: rows
                .iter()
                .map(|&(name, depth, mean, stddev)| Row {
                    kind: "gpu".into(),
                    name: name.into(),
                    depth,
                    mean,
                    stddev,
                    min: mean,
                    max: mean,
                    samples: 100,
                })
                .collect(),
        }
    }

    #[test]
    fn a_change_inside_the_noise_band_is_not_called_a_win() {
        // The exact failure this file exists to prevent: two runs of one build,
        // 2% apart, with a standard deviation wider than the difference.
        let before = run_of(&[("Shading", 1, 40.0, 1.5)]);
        let after = run_of(&[("Shading", 1, 39.2, 1.5)]);
        let report = compare(&before, &after).join("\n");
        assert!(report.contains("~ noise"), "{report}");
        assert!(!report.contains("faster"), "{report}");
    }

    #[test]
    fn a_change_outside_the_band_is_reported_with_a_direction() {
        let before = run_of(&[("Shading", 1, 40.0, 0.2)]);
        let after = run_of(&[("Shading", 1, 12.0, 0.2)]);
        let report = compare(&before, &after).join("\n");
        assert!(report.contains("faster"), "{report}");
        assert!(report.contains("-70.0%"), "{report}");
    }

    #[test]
    fn a_regression_is_shouted_rather_than_muttered() {
        let before = run_of(&[("Shading", 1, 12.0, 0.1)]);
        let after = run_of(&[("Shading", 1, 20.0, 0.1)]);
        let report = compare(&before, &after).join("\n");
        assert!(report.contains("SLOWER"), "{report}");
    }

    #[test]
    fn new_and_removed_zones_survive_a_comparison() {
        let before = run_of(&[("Shading", 1, 40.0, 0.1), ("TAA", 1, 1.0, 0.1)]);
        let after = run_of(&[("Shading", 1, 40.0, 0.1), ("Classify", 1, 0.2, 0.1)]);
        let report = compare(&before, &after).join("\n");
        assert!(report.contains("(new)"), "{report}");
        assert!(report.contains("(gone)"), "{report}");
    }

    #[test]
    fn a_written_run_round_trips_through_the_parser() {
        let text = "# somtime v1\n# label\ttest\n\
                    gpu\tFrame\t0\t50.0000\t1.0000\t48.0000\t52.0000\t100\n\
                    gpu\tShading\t1\t40.0000\t0.9000\t39.0000\t41.0000\t100\n";
        let run = parse(text);
        assert_eq!(run.rows.len(), 2);
        assert_eq!(run.find("gpu", "Shading", 1).map(|r| r.mean), Some(40.0));
        assert!(run.header.iter().any(|h| h.contains("test")));
    }

    #[test]
    fn unattributed_is_the_frame_minus_its_children() {
        let run = parse(
            "gpu\tFrame\t0\t50.0000\t0\t0\t0\t10\n\
             gpu\tShading\t1\t40.0000\t0\t0\t0\t10\n\
             gpu\tShadows\t1\t7.5000\t0\t0\t0\t10\n",
        );
        let pct = unattributed_pct(&run).expect("frame row present");
        assert!((pct - 5.0).abs() < 1e-3, "{pct}");
    }

    #[test]
    fn a_pass_recorded_once_per_view_is_counted_once_per_view() {
        // Four viewports record `Shading` four times a frame, so its samples
        // are four times the frame's. Counting its mean once would report three
        // quarters of the frame as unattributed and fail a gate that is
        // measuring the harness rather than the engine.
        let text = [
            ["gpu", "Frame", "0", "40.0000", "0", "0", "0", "10"].join("\t"),
            ["gpu", "Shading", "1", "9.5000", "0", "0", "0", "40"].join("\t"),
        ]
        .join("\n");
        let run = parse(&text);
        assert_eq!(run.rows.len(), 2, "fixture did not parse: {:?}", run.rows);
        let pct = unattributed_pct(&run).expect("frame row present");
        assert!((pct - 5.0).abs() < 1e-3, "{pct}");
    }

    #[test]
    fn a_nested_scope_does_not_count_against_the_frame_twice() {
        // CAS records inside `Post + present`, so it is depth 2 and must not be
        // subtracted again — otherwise a deeply nested frame reports negative
        // unattributed time and the number stops meaning anything.
        let run = parse(
            "gpu\tFrame\t0\t10.0000\t0\t0\t0\t10\n\
             gpu\tPost + present\t1\t10.0000\t0\t0\t0\t10\n\
             gpu\tCAS\t2\t4.0000\t0\t0\t0\t10\n",
        );
        assert_eq!(unattributed_pct(&run), Some(0.0));
    }

    #[test]
    fn the_accumulator_reports_a_spread_it_actually_saw() {
        let mut a = Accum::default();
        for v in [10.0, 12.0, 8.0, 10.0] {
            a.push(v);
        }
        assert!((a.mean() - 10.0).abs() < 1e-5);
        assert!((a.min - 8.0).abs() < 1e-5);
        assert!((a.max - 12.0).abs() < 1e-5);
        // Exactly root two, not an approximation of it: the deviations are
        // 0, 2, -2, 0, so the population variance is 2. Written as the
        // constant because `1.4142` is both less precise and a denied lint.
        assert!(
            (a.stddev() - std::f32::consts::SQRT_2).abs() < 1e-5,
            "{}",
            a.stddev()
        );
    }

    #[test]
    fn an_unvarying_zone_has_no_spread_and_no_nan() {
        let mut a = Accum::default();
        for _ in 0..64 {
            a.push(3.25);
        }
        assert_eq!(a.stddev(), 0.0);
        assert!(a.stddev().is_finite());
    }

    #[test]
    fn identical_runs_are_all_noise() {
        let r = run_of(&[("Frame", 0, 50.0, 1.0), ("Shading", 1, 40.0, 0.8)]);
        let report = compare(&r, &r).join("\n");
        assert!(!report.contains("faster"), "{report}");
        assert!(!report.contains("SLOWER"), "{report}");
    }

    #[test]
    fn a_zone_with_zero_before_does_not_divide_by_zero() {
        let before = run_of(&[("New pass", 1, 0.0, 0.0)]);
        let after = run_of(&[("New pass", 1, 0.5, 0.0)]);
        let report = compare(&before, &after).join("\n");
        assert!(report.contains("New pass"), "{report}");
        assert!(
            !report.contains("NaN") && !report.contains("inf"),
            "{report}"
        );
    }

    #[test]
    fn garbage_lines_are_skipped_rather_than_fatal() {
        let run = parse("nonsense\ngpu\tonly\tthree\nfields\n# header\n");
        assert!(run.rows.is_empty());
        assert_eq!(run.header.len(), 1);
    }
}
