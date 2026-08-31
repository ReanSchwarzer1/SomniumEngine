//! GPU and frame profiling (Phase 29).
//!
//! Until now the only way to answer "why is this frame slow" was to reason
//! about it. That cost real time in 17G, where a 51× draw-call regression was
//! found by argument rather than by measurement, and again in 25D, where the
//! cost half of the phase had to be expressed in *texture reads* through a
//! debug shader because there was no clock on the GPU at all.
//!
//! # How it works
//!
//! One `wgpu::QuerySet` of timestamps. Each profiled scope writes a timestamp
//! into the encoder when it opens and another when it closes, so a scope is a
//! pair of indices and the frame is a stack of them — which is where the nesting
//! depth in the report comes from.
//!
//! The results are read **one or more frames later, never in the frame that
//! wrote them**. A timestamp resolve is GPU work; waiting on it would make the
//! profiler the slowest thing in the frame and change the number it is trying to
//! report. Wicked's `wiProfiler` does the same thing and for the same reason,
//! reading the previous buffer index at the top of `BeginFrame` before it hands
//! out any new queries. Here a small ring of readback buffers is mapped
//! asynchronously and collected whenever it happens to be ready.
//!
//! Numbers are smoothed over a rolling window, because a single frame's GPU
//! timing is noisy enough to be useless for judging a change.
//!
//! # References
//!
//! - Wicked Engine `wiProfiler.cpp` (`New_Engines/WickedEngine-master/`) — the
//!   query-heap-per-frame-in-flight structure, the deferred readback, the
//!   rolling average, and the guard against nonsense timestamps.
//! - Flax `Engine/Profiler/ProfilerGPU.h` and `RenderStats.h` — events carrying
//!   a nesting **depth** so the report reads as a tree, and the counter set
//!   (draw calls, dispatches, triangles) that belongs beside the timings.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Scopes a frame may open. Two timestamps each.
///
/// One view opens about 25; the headroom is for the passes that do not exist
/// yet, and overflow degrades to "this scope is not timed" rather than to a
/// panic — a profiler that crashes the thing it is measuring is worse than one
/// that misses a row.
///
/// MORROWIND-J step 3 raised this from 64. A frame now records the scene once
/// *per view*, so a four-up editor opens four times the scopes and 64 silently
/// dropped half of them — which does not read as "the profiler ran out". It
/// reads as a frame with 50% unattributed time, which is indistinguishable from
/// an engine with an unbracketed pass, and it is the more alarming of the two.
///
/// Must stay a multiple of 16: the resolve buffer is `MAX_SCOPES * 2 * 8` bytes
/// and has to be a multiple of `QUERY_RESOLVE_BUFFER_ALIGNMENT` (256).
pub const MAX_SCOPES: usize = 192;

/// Frames of readback in flight. Three is one more than the deepest pipelining
/// wgpu will do, so a buffer is never mapped while the GPU still owns it.
const RING: usize = 3;

/// Samples in the rolling average.
const WINDOW: usize = 30;

/// Passes that may opt into pipeline statistics in one frame (Phase DOOM-A).
///
/// Small on purpose. Unlike a timestamp, a statistics query has to be opened
/// and closed *inside* the pass, so every entry here is a pass someone edited
/// by hand, and the list is meant to stay short enough to read.
pub const MAX_STATS: usize = 8;

/// Statistics gathered per query, in the order wgpu writes them.
///
/// The order is fixed by the bitflags declaration order, not by this constant —
/// see `PipelineStatisticsTypes`. Changing the set means changing
/// [`StatsResult`] to match, and the two are easy to desynchronise silently,
/// which is why the mapping is written out in [`Timeline::resolve_stats`]
/// rather than inferred.
pub const STATS_SLOTS: usize = 4;

/// One pass's pipeline statistics, as reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatsResult {
    pub name: &'static str,
    /// Vertex shader invocations (post vertex-cache for indexed draws).
    pub vertex_invocations: u64,
    /// Primitives that survived the clipper — what actually got rasterized.
    pub clipper_primitives: u64,
    /// Fragment shader invocations. For a fullscreen pass this is the pixel
    /// count; for a geometry pass the excess over the pixel count is overdraw.
    pub fragment_invocations: u64,
    /// Compute shader invocations.
    pub compute_invocations: u64,
}

/// Timings so large they can only be a driver artefact.
///
/// Wicked hits this on Apple TBDR when a pass draws no pixels and the timestamp
/// never resolves; the same shape of garbage shows up elsewhere on a lost or
/// reset device. One frame of nonsense would otherwise poison the whole rolling
/// window.
const IMPLAUSIBLE_MS: f32 = 1000.0;

/// One timed scope, as reported.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeResult {
    pub name: &'static str,
    /// 0 for a top-level scope; each enclosing scope adds one.
    pub depth: u8,
    /// Smoothed GPU time in milliseconds.
    pub ms: f32,
}

/// Per-frame counters that belong next to the timings.
///
/// From Flax's `RenderStatsData`: a pass time on its own says how long
/// something took, not why, and "why" is nearly always one of these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    pub draw_calls: u32,
    pub dispatches: u32,
    pub triangles: u32,
    /// Instances that survived culling and reached the draw queue.
    pub instances: u32,
    /// Terrain chunks submitted this frame (camera-visible; in `draw_queue`).
    pub terrain_chunks: u32,
    /// Terrain chunks rejected by the CPU camera frustum (Phase CR-B).
    pub terrain_cpu_culled: u32,
    /// Instances in the ray-tracing top-level acceleration structure.
    pub tlas_instances: u32,
    /// Draws that survived shadow-caster culling (Phase 24AE). Next to
    /// `draw_calls` because the pair is the whole story of the shadow pass.
    pub shadow_casters: u32,
    /// CSM atlas quadrants redrawn this frame (DOOM-D). A static frame should
    /// report zero while `shadow_casters` can remain non-zero.
    pub shadow_cascades_rendered: u32,
    /// Physical VSM tiles rasterised this frame and total resident tiles.
    pub virtual_shadow_pages: u32,
    pub virtual_shadow_resident: u32,
}

/// A scope opened this frame, before its timestamps have been read back.
#[derive(Clone, Copy)]
struct PendingScope {
    name: &'static str,
    depth: u8,
    begin: u32,
    end: u32,
}

/// The pure half: what was opened, in what order, and what the raw ticks mean.
///
/// Split out from the wgpu resources so the part with the arithmetic and the
/// stack discipline in it can be tested without a GPU — which matters more than
/// usual here, because every bug in this file produces a *plausible number*
/// rather than a visible failure.
#[derive(Default)]
pub struct Timeline {
    scopes: Vec<PendingScope>,
    stack: Vec<usize>,
    next_query: u32,
    /// Scopes that did not fit in the query set this frame.
    dropped: u32,
    /// Passes that opened a pipeline-statistics query this frame, in the order
    /// their slots were handed out (Phase DOOM-A).
    stats: Vec<&'static str>,
}

impl Timeline {
    /// Open a scope, returning the query index to write, or `None` when the
    /// query set is full.
    pub fn begin(&mut self, name: &'static str) -> Option<u32> {
        if self.next_query as usize + 2 > MAX_SCOPES * 2 {
            self.dropped += 1;
            return None;
        }
        let begin = self.next_query;
        self.next_query += 2;
        self.stack.push(self.scopes.len());
        self.scopes.push(PendingScope {
            name,
            // Depth is the stack height *before* this scope was pushed, which
            // is what makes a pass recorded inside another pass indent under it.
            depth: u8::try_from(self.stack.len() - 1).unwrap_or(u8::MAX),
            begin,
            end: begin + 1,
        });
        Some(begin)
    }

    /// Close the innermost open scope, returning the query index to write.
    ///
    /// `None` when the stack is empty, which means an `end` without a `begin` —
    /// returning rather than panicking because an unbalanced scope is a bug in
    /// instrumentation, and instrumentation must not be able to take the frame
    /// down.
    pub fn end(&mut self) -> Option<u32> {
        let idx = self.stack.pop()?;
        Some(self.scopes[idx].end)
    }

    /// Queries written this frame.
    pub fn query_count(&self) -> u32 {
        self.next_query
    }

    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Scopes opened but never closed. Non-zero means broken instrumentation.
    pub fn unclosed(&self) -> usize {
        self.stack.len()
    }

    /// Reserve a pipeline-statistics slot for `name`, or `None` when the small
    /// stats set is full. The caller opens and closes the query itself, because
    /// wgpu only allows that from inside the pass.
    pub fn reserve_stats(&mut self, name: &'static str) -> Option<u32> {
        if self.stats.len() >= MAX_STATS {
            return None;
        }
        let index = u32::try_from(self.stats.len()).ok()?;
        self.stats.push(name);
        Some(index)
    }

    pub fn stats_count(&self) -> u32 {
        u32::try_from(self.stats.len()).unwrap_or(0)
    }

    /// Turn resolved statistics values into per-pass counters.
    ///
    /// `values` is the flat slot array: `STATS_SLOTS` `u64`s per query, in the
    /// order `PipelineStatisticsTypes` declares them.
    #[must_use]
    pub fn resolve_stats(&self, values: &[u64]) -> Vec<StatsResult> {
        self.stats
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let base = i * STATS_SLOTS;
                Some(StatsResult {
                    name,
                    vertex_invocations: *values.get(base)?,
                    clipper_primitives: *values.get(base + 1)?,
                    fragment_invocations: *values.get(base + 2)?,
                    compute_invocations: *values.get(base + 3)?,
                })
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.scopes.clear();
        self.stack.clear();
        self.next_query = 0;
        self.dropped = 0;
        self.stats.clear();
    }

    /// Turn raw timestamp ticks into per-scope milliseconds.
    ///
    /// `period_ns` is the nanoseconds one tick represents, from
    /// `Queue::get_timestamp_period`.
    #[must_use]
    pub fn resolve(&self, ticks: &[u64], period_ns: f32) -> Vec<ScopeResult> {
        self.scopes
            .iter()
            .filter_map(|s| {
                let begin = *ticks.get(s.begin as usize)?;
                let end = *ticks.get(s.end as usize)?;
                // Saturating, not wrapping: a scope whose end timestamp landed
                // before its begin is garbage, and `end - begin` on u64 would
                // turn it into several years rather than into zero.
                let ms = (end.saturating_sub(begin) as f64 * f64::from(period_ns) / 1.0e6) as f32;
                let ms = if ms.is_finite() && ms < IMPLAUSIBLE_MS {
                    ms
                } else {
                    0.0
                };
                Some(ScopeResult {
                    name: s.name,
                    depth: s.depth,
                    ms,
                })
            })
            .collect()
    }
}

/// Rolling average per scope, keyed by name and depth.
///
/// Keyed rather than positional because the pass list is not the same every
/// frame — the shadow pass drops out when the sun is below the horizon, DoF
/// only records when it is enabled — and a positional window would silently
/// average one pass into another's row the frame something switched off.
#[derive(Default)]
struct Smoother {
    entries: Vec<(&'static str, u8, Vec<f32>, usize)>,
}

impl Smoother {
    fn push(&mut self, results: &mut [ScopeResult]) {
        for r in results.iter_mut() {
            let slot = match self
                .entries
                .iter_mut()
                .find(|(n, d, _, _)| *n == r.name && *d == r.depth)
            {
                Some(e) => e,
                None => {
                    self.entries
                        .push((r.name, r.depth, Vec::with_capacity(WINDOW), 0));
                    self.entries.last_mut().expect("just pushed")
                }
            };
            let (_, _, window, next) = slot;
            if window.len() < WINDOW {
                window.push(r.ms);
            } else {
                window[*next % WINDOW] = r.ms;
            }
            *next = next.wrapping_add(1);
            let sum: f32 = window.iter().sum();
            r.ms = sum / window.len() as f32;
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// State of one readback buffer in the ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    /// Available to record into.
    Idle,
    /// Submitted; waiting for the map callback.
    InFlight,
}

struct Frame {
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    ready: Arc<AtomicBool>,
    state: Slot,
    timeline: Timeline,
    queries: u32,
}

/// The profiler proper.
///
/// Constructed even when the adapter has no timestamp support: `available` is
/// then false, every scope call is a no-op, and the counters still work. That
/// keeps the call sites free of `if let Some(profiler)` and follows the
/// "detect, do not demand" pattern the context already uses for GPU-driven
/// rendering and ray tracing.
pub struct GpuProfiler {
    query_set: Option<wgpu::QuerySet>,
    /// Phase DOOM-A. Separate from the timestamp set because a query set has
    /// exactly one type. Resolved into the same readback buffer at a fixed
    /// offset, so there is still one `map_async` and one ready flag per frame.
    stats_set: Option<wgpu::QuerySet>,
    stats_results: Vec<StatsResult>,
    frames: Vec<Frame>,
    current: usize,
    period_ns: f32,
    available: bool,
    enabled: bool,
    /// Applied at the next frame boundary. Toggling mid-frame would leave a
    /// half-recorded query set and one frame of nonsense.
    enable_request: bool,
    results: Vec<ScopeResult>,
    /// The same frame before smoothing (Phase DOOM-A).
    ///
    /// The overlay wants the average — a single frame's GPU timing is too noisy
    /// to read. The timing harness wants the opposite: it has to know the
    /// *spread*, because "is this 3% change real" is a question about variance,
    /// and a 30-frame average has already thrown that away.
    raw_results: Vec<ScopeResult>,
    /// Bumped every time `collect` harvests a frame, so a consumer can tell a
    /// fresh sample from the same one read twice.
    raw_serial: u64,
    smoother: Smoother,
    cpu_open: Vec<(&'static str, std::time::Instant, u8)>,
    cpu_acc: Vec<ScopeResult>,
    cpu_results: Vec<ScopeResult>,
    /// The same CPU zones before smoothing (PORTAL-0-B).
    ///
    /// `cpu_end` writes an EMA into `cpu_acc` because the overlay is unreadable
    /// without one. `timing.rs` needs the opposite for exactly the reason
    /// `raw_results` exists beside `results`: a standard deviation taken over a
    /// smoothed signal is the standard deviation of the smoother, not of the
    /// work, and every `.somtime` written before this field understated the CPU
    /// spread by roughly the smoothing factor.
    cpu_raw_acc: Vec<ScopeResult>,
    cpu_raw_results: Vec<ScopeResult>,
    /// Wall-clock milliseconds the engine's frame body spent on the CPU,
    /// excluding the frame limiter's sleep (PORTAL-0-B).
    ///
    /// Written by the application once per frame for the frame *before* it, so
    /// it is never a scope left open across [`GpuProfiler::end_frame`]. This is
    /// the row that was missing: `Frame wall` is a tick-to-tick interval under
    /// `PresentMode::AutoVsync` and therefore includes the presentation block,
    /// so it has never been able to answer "is this frame CPU-bound".
    pub frame_cpu_ms: f32,
    /// Milliseconds blocked in `Surface::get_current_texture` (PORTAL-0-B).
    ///
    /// Under Fifo this is where the vsync wait lands, so it is the term that
    /// reconciles a small GPU `Frame` with a large `Frame wall`. Kept separate
    /// from [`Self::frame_cpu_ms`], which contains it.
    pub surface_acquire_ms: f32,
    /// The frame most recently collected, for the "is this stale" question the
    /// overlay would otherwise have to guess at.
    collected: u64,
    frame_index: u64,
    pub counters: FrameCounters,
    /// Counters as of the last completed frame, which is what should be shown:
    /// the live set is still being accumulated while the overlay draws.
    pub last_counters: FrameCounters,
    scratch: Arc<Mutex<Vec<u64>>>,
    /// Frames between headless log dumps, 0 for never (`SOMNIUM_PROFILE_EVERY`).
    log_every: u64,
}

impl GpuProfiler {
    /// Bytes one frame of timestamps occupies.
    ///
    /// Also the offset the statistics block starts at, which must stay a
    /// multiple of `QUERY_RESOLVE_BUFFER_ALIGNMENT` (256). `MAX_SCOPES * 2 * 8`
    /// is 1024 and satisfies that; a `MAX_SCOPES` that does not would fail at
    /// `resolve_query_set` rather than here.
    const BYTES: u64 = (MAX_SCOPES * 2 * std::mem::size_of::<u64>()) as u64;

    /// Bytes one frame of pipeline statistics occupies.
    const STATS_BYTES: u64 = (MAX_STATS * STATS_SLOTS * std::mem::size_of::<u64>()) as u64;

    /// Total readback per frame slot: timestamps, then statistics.
    const TOTAL_BYTES: u64 = Self::BYTES + Self::STATS_BYTES;

    /// The counters asked for, in the order `PipelineStatisticsTypes` declares
    /// them — which is also the order they are written and the order
    /// [`Timeline::resolve_stats`] unpacks them.
    const STATS_TYPES: wgpu::PipelineStatisticsTypes =
        wgpu::PipelineStatisticsTypes::VERTEX_SHADER_INVOCATIONS
            .union(wgpu::PipelineStatisticsTypes::CLIPPER_PRIMITIVES_OUT)
            .union(wgpu::PipelineStatisticsTypes::FRAGMENT_SHADER_INVOCATIONS)
            .union(wgpu::PipelineStatisticsTypes::COMPUTE_SHADER_INVOCATIONS);

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, features: wgpu::Features) -> Self {
        let available = features.contains(wgpu::Features::TIMESTAMP_QUERY)
            && features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

        // Phase DOOM-A: independent of `available`. Statistics are useful on an
        // adapter with no timestamps at all — "how many fragments" answers a
        // different question from "how many milliseconds".
        let stats_set = if features.contains(wgpu::Features::PIPELINE_STATISTICS_QUERY) {
            Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Profiler Pipeline Statistics"),
                ty: wgpu::QueryType::PipelineStatistics(Self::STATS_TYPES),
                count: u32::try_from(MAX_STATS).expect("stats count fits u32"),
            }))
        } else {
            None
        };

        let (query_set, frames) = if available {
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Profiler Timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: u32::try_from(MAX_SCOPES * 2).expect("query count fits u32"),
            });
            let frames = (0..RING)
                .map(|i| Frame {
                    resolve: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&format!("Profiler Resolve {i}")),
                        size: Self::TOTAL_BYTES,
                        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    readback: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&format!("Profiler Readback {i}")),
                        size: Self::TOTAL_BYTES,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    ready: Arc::new(AtomicBool::new(false)),
                    state: Slot::Idle,
                    timeline: Timeline::default(),
                    queries: 0,
                })
                .collect();
            (Some(query_set), frames)
        } else {
            tracing::info!("profiler: GPU timestamps unavailable on this adapter — counters only");
            (None, Vec::new())
        };

        let enabled = std::env::var("SOMNIUM_PROFILE").as_deref() == Ok("1")
            || std::env::var("SOMNIUM_CAPTURE_PNG").is_ok()
            || std::env::var("SOMNIUM_CAPTURE").is_ok()
            // Phase DOOM-A: a timing run with the profiler off would measure
            // nothing and write a file of zeros, which is worse than failing.
            || std::env::var("SOMNIUM_TIME").is_ok();
        Self {
            query_set,
            stats_set,
            stats_results: Vec::new(),
            frames,
            current: 0,
            period_ns: queue.get_timestamp_period(),
            available,
            enabled: false,
            enable_request: enabled,
            results: Vec::new(),
            raw_results: Vec::new(),
            raw_serial: 0,
            smoother: Smoother::default(),
            cpu_open: Vec::new(),
            cpu_acc: Vec::new(),
            cpu_results: Vec::new(),
            cpu_raw_acc: Vec::new(),
            cpu_raw_results: Vec::new(),
            frame_cpu_ms: 0.0,
            surface_acquire_ms: 0.0,
            collected: 0,
            frame_index: 0,
            counters: FrameCounters::default(),
            last_counters: FrameCounters::default(),
            scratch: Arc::new(Mutex::new(Vec::new())),
            // The whole point of the phase for headless work: a run with
            // `SOMNIUM_PROFILE=1` prints the pass table periodically, so a
            // future A/B can be measured from a log the way 25D's tap counts
            // were, but in milliseconds and without a debug shader.
            log_every: std::env::var("SOMNIUM_PROFILE_EVERY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
        }
    }

    /// Whether this adapter can time anything at all.
    pub fn available(&self) -> bool {
        self.available
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Request a state change, applied at the next frame boundary.
    pub fn set_enabled(&mut self, on: bool) {
        self.enable_request = on;
    }

    /// Throw away the rolling windows (Phase DOOM-F).
    ///
    /// For a consumer that has just changed the workload it is measuring. The
    /// windows are thirty frames deep, so after a resolution change the average
    /// describes a mixture of two resolutions for the next thirty frames — and
    /// a controller reading that mixture concludes something untrue about the
    /// size it is currently rendering. This is the same reasoning as the clear
    /// in `begin_frame` when the profiler is toggled, exposed so dynamic
    /// resolution can use it.
    pub fn reset_smoothing(&mut self) {
        self.smoother.clear();
        self.results.clear();
    }

    pub fn toggle(&mut self) {
        self.set_enabled(!self.enable_request);
    }

    /// Smoothed results from the most recently collected frame.
    pub fn results(&self) -> &[ScopeResult] {
        &self.results
    }

    /// CPU scopes from the last completed frame (Phase 29). EMA-smoothed.
    pub fn cpu_results(&self) -> &[ScopeResult] {
        &self.cpu_results
    }

    /// The same scopes, unsmoothed (PORTAL-0-B).
    ///
    /// Use this and not [`Self::cpu_results`] anywhere a spread is being
    /// reported. There is no serial here because CPU zones close once per
    /// rendered frame, unlike GPU readback.
    pub fn cpu_raw_results(&self) -> &[ScopeResult] {
        &self.cpu_raw_results
    }

    /// The last harvested frame **before** smoothing, and a serial that
    /// changes only when a genuinely new frame lands (Phase DOOM-A).
    ///
    /// Readback arrives a few frames late and not necessarily once per frame,
    /// so a consumer that sampled every rendered frame would count the same
    /// measurement several times and report a variance far below the truth.
    /// The serial is how it avoids that.
    pub fn raw_sample(&self) -> (u64, &[ScopeResult]) {
        (self.raw_serial, &self.raw_results)
    }

    /// Open a CPU-timed zone. Pairs with [`GpuProfiler::cpu_end`].
    pub fn cpu_begin(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }
        let depth = u8::try_from(self.cpu_open.len()).unwrap_or(u8::MAX);
        self.cpu_open.push((name, std::time::Instant::now(), depth));
    }

    /// Close the innermost CPU zone.
    pub fn cpu_end(&mut self) {
        if !self.enabled {
            return;
        }
        let Some((name, t0, depth)) = self.cpu_open.pop() else {
            return;
        };
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        let prev = self
            .cpu_results
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.ms)
            .unwrap_or(ms);
        self.cpu_acc.push(ScopeResult {
            name,
            depth,
            ms: prev * 0.8 + ms * 0.2,
        });
        // PORTAL-0-B: the same zone, unsmoothed, for the timing harness.
        self.cpu_raw_acc.push(ScopeResult { name, depth, ms });
    }

    /// Total of the top-level scopes — the GPU frame, without double-counting
    /// nested passes.
    pub fn total_ms(&self) -> f32 {
        self.results
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.ms)
            .sum()
    }

    /// GPU time inside the frame that no scope claims.
    ///
    /// Printed instead of a total, which would only repeat the `Frame` row.
    /// Everything not yet bracketed — culling, the second visibility phase,
    /// ReSTIR, IBL, the editor overlays — lands here, so the row says how much
    /// of the frame the profiler still cannot see. That is the number worth
    /// looking at when a change does not show up where it was expected.
    pub fn unattributed_ms(&self) -> f32 {
        let children: f32 = self
            .results
            .iter()
            .filter(|r| r.depth == 1)
            .map(|r| r.ms)
            .sum();
        (self.total_ms() - children).max(0.0)
    }

    /// Open the frame: collect whatever readback has landed, then reset.
    pub fn begin_frame(&mut self) {
        self.frame_index += 1;
        self.last_counters = std::mem::take(&mut self.counters);

        if self.enable_request != self.enabled {
            self.enabled = self.enable_request;
            // Old windows describe a different workload; keeping them would
            // average across the change the user just made.
            self.smoother.clear();
            self.results.clear();
        }
        if !self.enabled {
            return;
        }

        self.collect();

        // Pick an idle slot. If every slot is in flight the GPU is further
        // behind than the ring is deep, and skipping a frame of timing is the
        // right answer — the alternative is blocking on a map.
        if let Some(i) = self.frames.iter().position(|f| f.state == Slot::Idle) {
            self.current = i;
            self.frames[i].timeline.clear();
        } else {
            self.current = usize::MAX;
        }
    }

    fn recording(&self) -> bool {
        self.enabled && self.current != usize::MAX && self.query_set.is_some()
    }

    /// Open a named scope. Pairs with [`GpuProfiler::end`].
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder, name: &'static str) {
        if !self.recording() {
            return;
        }
        let Some(index) = self.frames[self.current].timeline.begin(name) else {
            return;
        };
        if let Some(qs) = &self.query_set {
            encoder.write_timestamp(qs, index);
        }
    }

    /// Reserve a pipeline-statistics slot for a pass that is about to record
    /// (Phase DOOM-A).
    ///
    /// Two-call rather than bracketed, because `begin_pipeline_statistics_query`
    /// only exists on a render or compute pass — there is no encoder-level form
    /// the way `TIMESTAMP_QUERY_INSIDE_ENCODERS` gives for timestamps. Call this
    /// *before* the pass is created, then pass the index and
    /// [`GpuProfiler::stats_query_set`] into it: reserving first is what keeps
    /// the `&mut self` borrow from colliding with the pass's borrows of the
    /// render targets.
    ///
    /// `None` means "do not query" — no statistics support, profiler off, or
    /// the small set is full.
    pub fn reserve_stats(&mut self, name: &'static str) -> Option<u32> {
        if !self.enabled || self.stats_set.is_none() || self.current == usize::MAX {
            return None;
        }
        self.frames
            .get_mut(self.current)?
            .timeline
            .reserve_stats(name)
    }

    /// The statistics query set, for a pass that reserved a slot.
    pub fn stats_query_set(&self) -> Option<&wgpu::QuerySet> {
        self.stats_set.as_ref()
    }

    /// Per-pass counters from the most recently collected frame.
    pub fn stats_results(&self) -> &[StatsResult] {
        &self.stats_results
    }

    /// Close the innermost open scope.
    pub fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.recording() {
            return;
        }
        let Some(index) = self.frames[self.current].timeline.end() else {
            return;
        };
        if let Some(qs) = &self.query_set {
            encoder.write_timestamp(qs, index);
        }
    }

    /// Resolve this frame's queries into the ring. Call once, after every scope
    /// has closed and before the encoder is submitted.
    pub fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.enabled {
            if !self.cpu_open.is_empty() {
                tracing::warn!(
                    "profiler: {} CPU scope(s) left open this frame",
                    self.cpu_open.len()
                );
                self.cpu_open.clear();
            }
            self.cpu_results = std::mem::take(&mut self.cpu_acc);
            self.cpu_raw_results = std::mem::take(&mut self.cpu_raw_acc);
        }
        if !self.recording() {
            return;
        }
        let i = self.current;
        let unclosed = self.frames[i].timeline.unclosed();
        if unclosed > 0 {
            tracing::warn!("profiler: {unclosed} scope(s) left open this frame");
        }
        let count = self.frames[i].timeline.query_count();
        self.frames[i].queries = count;
        if count == 0 {
            return;
        }
        let Some(qs) = &self.query_set else { return };
        let frame = &self.frames[i];
        encoder.resolve_query_set(qs, 0..count, &frame.resolve, 0);

        // Phase DOOM-A: statistics land in the same buffer, after the
        // timestamps, so one copy and one map serve both. A frame where no pass
        // opted in resolves nothing and leaves the block untouched — which is
        // safe because `resolve_stats` only reads as many entries as were
        // reserved.
        let stats_count = frame.timeline.stats_count();
        let stats_bytes = if stats_count > 0
            && let Some(ss) = &self.stats_set
        {
            encoder.resolve_query_set(ss, 0..stats_count, &frame.resolve, Self::BYTES);
            u64::from(stats_count) * (STATS_SLOTS * 8) as u64
        } else {
            0
        };

        encoder.copy_buffer_to_buffer(&frame.resolve, 0, &frame.readback, 0, u64::from(count) * 8);
        if stats_bytes > 0 {
            encoder.copy_buffer_to_buffer(
                &frame.resolve,
                Self::BYTES,
                &frame.readback,
                Self::BYTES,
                stats_bytes,
            );
        }
    }

    /// Start the asynchronous read. Must be called *after* the submit that
    /// contains [`GpuProfiler::end_frame`]'s copy, or the map races the write.
    ///
    /// Also polls the device, which is what actually runs the map callbacks of
    /// *earlier* frames. Nothing else in the engine polls per frame — the two
    /// existing calls are both blocking waits for a specific readback — so
    /// without this the callbacks never fire, `ready` never flips, and the
    /// profiler silently reports nothing at all. `PollType::Poll` is the
    /// non-blocking form: it drains whatever is finished and returns.
    pub fn after_submit(&mut self, device: &wgpu::Device) {
        if !self.enabled {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if !self.recording() || self.frames[self.current].queries == 0 {
            return;
        }
        let i = self.current;
        let ready = Arc::clone(&self.frames[i].ready);
        ready.store(false, Ordering::Release);
        self.frames[i].state = Slot::InFlight;
        // The whole slot, not just the timestamps written this frame: the
        // statistics block lives at a fixed offset above them and `map_async`
        // needs one contiguous range. 1280 bytes — the cost of being able to
        // read both with one callback.
        self.frames[i]
            .readback
            .slice(0..Self::TOTAL_BYTES)
            .map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
    }

    /// Harvest any slot whose map callback has fired.
    fn collect(&mut self) {
        for i in 0..self.frames.len() {
            if self.frames[i].state != Slot::InFlight
                || !self.frames[i].ready.load(Ordering::Acquire)
            {
                continue;
            }
            let bytes = u64::from(self.frames[i].queries) * 8;
            {
                let view = self.frames[i]
                    .readback
                    .slice(0..Self::TOTAL_BYTES)
                    .get_mapped_range()
                    .expect("timestamp frame is only read once its map callback fired");
                let mut ticks = self.scratch.lock().expect("profiler scratch");
                ticks.clear();
                ticks.extend(
                    view[0..bytes as usize]
                        .chunks_exact(8)
                        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes"))),
                );
                let mut results = self.frames[i].timeline.resolve(&ticks, self.period_ns);
                self.raw_results.clear();
                self.raw_results.extend_from_slice(&results);
                self.raw_serial = self.raw_serial.wrapping_add(1);
                self.smoother.push(&mut results);
                self.results = results;

                // Phase DOOM-A. Not smoothed: these are exact integer counts,
                // and averaging them would only invent fractional fragments.
                let stats_count = self.frames[i].timeline.stats_count() as usize;
                if stats_count > 0 {
                    let start = Self::BYTES as usize;
                    let end = start + stats_count * STATS_SLOTS * 8;
                    ticks.clear();
                    ticks.extend(
                        view[start..end]
                            .chunks_exact(8)
                            .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes"))),
                    );
                    self.stats_results = self.frames[i].timeline.resolve_stats(&ticks);
                } else {
                    self.stats_results.clear();
                }
            }
            self.frames[i].readback.unmap();
            self.frames[i].ready.store(false, Ordering::Release);
            self.frames[i].state = Slot::Idle;
            self.collected = self.frame_index;
            self.log_if_due();
        }
    }

    /// Print the table to the log every `log_every` collected frames.
    ///
    /// Gated on a *collected* frame rather than a rendered one, so the numbers
    /// in the log are always real readings rather than the last stale set
    /// repeated.
    fn log_if_due(&self) {
        if self.log_every == 0 || self.frame_index % self.log_every != 0 {
            return;
        }
        tracing::info!("PROFILE frame={}", self.frame_index);
        for line in self.report() {
            tracing::info!("PROFILE   {line}");
        }
    }

    /// The report as lines of text, for the log and the overlay.
    ///
    /// Indented by depth, so a nested pass reads as nested — Flax's event depth
    /// exists for exactly this and it is the difference between a list and a
    /// frame graph.
    #[must_use]
    pub fn report(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.results.len() + 3);
        if !self.available {
            out.push("GPU timestamps unavailable on this adapter".to_string());
        } else if self.results.is_empty() {
            out.push("waiting for the first frame of timings…".to_string());
        }
        for r in &self.results {
            let indent = "  ".repeat(r.depth as usize);
            out.push(format!("{indent}{:<26} {:>7.3} ms", r.name, r.ms));
        }
        if !self.results.is_empty() {
            out.push(format!(
                "{:<26} {:>7.3} ms",
                "unattributed",
                self.unattributed_ms()
            ));
        }
        if !self.cpu_results.is_empty() {
            out.push("CPU".to_string());
            for r in &self.cpu_results {
                let indent = "  ".repeat(r.depth as usize);
                out.push(format!("{indent}{:<26} {:>7.3} ms", r.name, r.ms));
            }
        }
        // Phase DOOM-A: the "why" beside the "how long". A fullscreen pass whose
        // fragment invocations exceed its pixel count is running the 2×2
        // derivative quads at every silhouette; a geometry pass whose count
        // exceeds it is overdrawing.
        if !self.stats_results.is_empty() {
            out.push("counters".to_string());
            for s in &self.stats_results {
                out.push(format!(
                    "  {:<24} {} frag / {} prim / {} cs",
                    s.name, s.fragment_invocations, s.clipper_primitives, s.compute_invocations
                ));
            }
        }
        let graph = self
            .results
            .iter()
            .filter(|s| s.depth <= 1)
            .map(|s| s.name)
            .collect::<Vec<_>>()
            .join(" → ");
        if !graph.is_empty() {
            out.push(format!("graph                      {graph}"));
        }
        let c = self.last_counters;
        out.push(format!(
            "{:<26} {} draws / {} tris",
            "geometry", c.draw_calls, c.triangles
        ));
        out.push(format!(
            "{:<26} {} inst / {} chunks / {} cpu-cull / {} tlas",
            "scene", c.instances, c.terrain_chunks, c.terrain_cpu_culled, c.tlas_instances
        ));
        // Phase 24AE. Beside the draw count on purpose: the ratio is the whole
        // story of the shadow pass, and a `Shadows` row that suddenly costs
        // more is nearly always this number having grown.
        out.push(format!(
            "{:<26} {} of {} draws",
            "shadow casters", c.shadow_casters, c.draw_calls
        ));
        out.push(format!(
            "{:<26} {} of 4",
            "shadow cascades", c.shadow_cascades_rendered
        ));
        out.push(format!(
            "{:<26} {} rendered / {} resident",
            "virtual shadow pages", c.virtual_shadow_pages, c.virtual_shadow_resident
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks where scope `i` runs from `10*i` to `10*i + 5`.
    fn ticks(n: usize) -> Vec<u64> {
        (0..n * 2)
            .map(|q| {
                let scope = q / 2;
                if q % 2 == 0 {
                    10 * scope as u64
                } else {
                    10 * scope as u64 + 5
                }
            })
            .collect()
    }

    #[test]
    fn nesting_sets_the_depth() {
        let mut t = Timeline::default();
        t.begin("frame");
        t.begin("shadows");
        t.end();
        t.begin("shading");
        t.begin("gtao");
        t.end();
        t.end();
        t.end();
        let r = t.resolve(&ticks(4), 1.0);
        let depths: Vec<(&str, u8)> = r.iter().map(|s| (s.name, s.depth)).collect();
        assert_eq!(
            depths,
            vec![("frame", 0), ("shadows", 1), ("shading", 1), ("gtao", 2)]
        );
    }

    #[test]
    fn a_tick_span_becomes_milliseconds() {
        let mut t = Timeline::default();
        t.begin("pass");
        t.end();
        // 5 ticks at 1000 ns each = 5 µs = 0.005 ms.
        let r = t.resolve(&ticks(1), 1000.0);
        assert!((r[0].ms - 0.005).abs() < 1e-6, "{:?}", r[0]);
    }

    #[test]
    fn a_backwards_timestamp_reports_zero_rather_than_centuries() {
        // u64 subtraction the other way round is ~1.8e19 ticks, which at any
        // period is a number that would wipe out the rolling window and read as
        // a catastrophic regression in the one pass that hit driver noise.
        let mut t = Timeline::default();
        t.begin("pass");
        t.end();
        let r = t.resolve(&[900, 100], 1.0);
        assert_eq!(r[0].ms, 0.0);
    }

    #[test]
    fn an_implausible_span_is_discarded() {
        let mut t = Timeline::default();
        t.begin("pass");
        t.end();
        // 2 seconds for one pass is a driver artefact, not a slow frame.
        let r = t.resolve(&[0, 2_000_000_000], 1.0);
        assert_eq!(r[0].ms, 0.0);
    }

    #[test]
    fn running_out_of_queries_drops_scopes_instead_of_panicking() {
        let mut t = Timeline::default();
        for _ in 0..MAX_SCOPES {
            assert!(t.begin("pass").is_some());
            t.end();
        }
        assert!(t.begin("one too many").is_none());
        assert_eq!(t.dropped(), 1);
        // And the frame still resolves cleanly.
        assert_eq!(t.resolve(&ticks(MAX_SCOPES), 1.0).len(), MAX_SCOPES);
    }

    #[test]
    fn an_unbalanced_end_is_survivable() {
        let mut t = Timeline::default();
        assert!(t.end().is_none());
        t.begin("pass");
        t.end();
        assert!(t.end().is_none());
        assert_eq!(t.unclosed(), 0);
    }

    #[test]
    fn an_unclosed_scope_is_visible() {
        let mut t = Timeline::default();
        t.begin("forgot to close");
        assert_eq!(t.unclosed(), 1);
    }

    #[test]
    fn query_indices_never_overlap() {
        let mut t = Timeline::default();
        let mut seen = Vec::new();
        for _ in 0..8 {
            seen.push(t.begin("a").expect("space"));
        }
        for _ in 0..8 {
            seen.push(t.end().expect("open"));
        }
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two scopes shared a query slot");
    }

    #[test]
    fn the_average_converges_and_does_not_lag_forever() {
        let mut s = Smoother::default();
        let mut last = 0.0;
        for _ in 0..WINDOW * 2 {
            let mut r = vec![ScopeResult {
                name: "pass",
                depth: 0,
                ms: 4.0,
            }];
            s.push(&mut r);
            last = r[0].ms;
        }
        assert!((last - 4.0).abs() < 1e-4, "settled at {last}");
    }

    #[test]
    fn the_window_forgets_old_frames() {
        let mut s = Smoother::default();
        // Fill with 10 ms, then feed 0 ms for a full window.
        for _ in 0..WINDOW {
            s.push(&mut [ScopeResult {
                name: "pass",
                depth: 0,
                ms: 10.0,
            }]);
        }
        let mut last = 0.0;
        for _ in 0..WINDOW {
            let mut r = vec![ScopeResult {
                name: "pass",
                depth: 0,
                ms: 0.0,
            }];
            s.push(&mut r);
            last = r[0].ms;
        }
        assert!(last < 0.01, "old samples never aged out: {last}");
    }

    #[test]
    fn two_scopes_sharing_a_name_at_different_depths_do_not_share_a_window() {
        // `Hi-Z` is recorded twice a frame by the two-phase cull, and a nested
        // copy of a top-level name must not be averaged into it.
        let mut s = Smoother::default();
        for _ in 0..WINDOW {
            s.push(&mut [
                ScopeResult {
                    name: "hiz",
                    depth: 0,
                    ms: 8.0,
                },
                ScopeResult {
                    name: "hiz",
                    depth: 1,
                    ms: 2.0,
                },
            ]);
        }
        let mut r = vec![
            ScopeResult {
                name: "hiz",
                depth: 0,
                ms: 8.0,
            },
            ScopeResult {
                name: "hiz",
                depth: 1,
                ms: 2.0,
            },
        ];
        s.push(&mut r);
        assert!((r[0].ms - 8.0).abs() < 1e-3, "{:?}", r);
        assert!((r[1].ms - 2.0).abs() < 1e-3, "{:?}", r);
    }

    #[test]
    fn clearing_lets_a_timeline_be_reused_each_frame() {
        let mut t = Timeline::default();
        t.begin("a");
        t.end();
        t.clear();
        assert_eq!(t.query_count(), 0);
        assert_eq!(t.begin("b"), Some(0));
    }
}
