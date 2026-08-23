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
                a.name, b, a.mean, a.mean - b, a.kind
            ));
        }
    }
    out
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
    /// Wall-clock interval between presented frames. Unlike GPU scopes this
    /// includes UI-thread work, which is the quantity CONTROL-A/C need for the
    /// shipped synchronous thumbnail decoder's hitch baseline.
    wall_frame: Accum,
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
            last_tick_at: None,
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
            self.wall_frame.push(ms);
        }

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
            for r in profiler.cpu_results() {
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
        if self.wall_frame.n > 0 {
            let a = &self.wall_frame;
            let _ = writeln!(
                s,
                "cpu\tFrame wall\t0\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}",
                a.mean(),
                a.stddev(),
                a.min,
                a.max,
                a.n
            );
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
    let frame = run.find("gpu", "Frame", 0)?.mean;
    if frame <= 0.0 {
        return None;
    }
    let children: f32 = run
        .rows
        .iter()
        .filter(|r| r.kind == "gpu" && r.depth == 1)
        .map(|r| r.mean)
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
        assert!((a.stddev() - 1.4142).abs() < 1e-3, "{}", a.stddev());
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
        assert!(!report.contains("NaN") && !report.contains("inf"), "{report}");
    }

    #[test]
    fn garbage_lines_are_skipped_rather_than_fatal() {
        let run = parse("nonsense\ngpu\tonly\tthree\nfields\n# header\n");
        assert!(run.rows.is_empty());
        assert_eq!(run.header.len(), 1);
    }
}
