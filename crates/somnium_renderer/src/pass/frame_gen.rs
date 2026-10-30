//! AMD FSR 3 frame generation (wgpu-ffx-fg): a frame half way between each two
//! rendered ones, presented between them.
//!
//! While it runs, the scene and its post-processing land in one of two offscreen
//! *pictures* (the swap chain's format) instead of the swap chain. Each frame:
//!
//! 1. at the start, the previous picture is drawn into a swap chain image with
//!    the game's UI over it and submitted, but not yet presented (the real frame);
//! 2. part way through this frame's GPU work (a *split point*), that image is
//!    presented: the queue runs work and presents in order, so the real frame
//!    reaches the screen after the GPU time recorded before the split;
//! 3. after the scene, the frame between the previous picture and this one is
//!    generated, drawn into a second swap chain image with the UI over it, and
//!    presented at the end of the frame, as before.
//!
//! So each real frame shows a generated frame and then itself, spaced by how
//! the frame's work divides at the split. wgpu has one queue and no present
//! timing, so the split is how the presents are paced: a controller moves it
//! between seven points in the frame until the two intervals, measured with GPU
//! timestamps at the work before each present, are as even as they can be. (A
//! per-frame CPU-time gate was tried and was worse: the previous frame's GPU work
//! is often still running when the early points come, so CPU time does not say
//! when a present will flip.) The UI is never interpolated: it is drawn over every
//! presented frame.
//!
//! Interpolation runs on what the player sees before the UI: AMD's back buffer
//! values (sRGB-encoded, read through a non-sRGB view), with the scene's depth
//! and motion vectors at render size. A camera cut (a jump in the view) or an
//! explicit reset presents the real frame twice instead of interpolating across it.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Points in the frame where the real frame may be presented, in order.
pub const SPLIT_START: usize = 0;
pub const SPLIT_AFTER_SHADOWS: usize = 1;
pub const SPLIT_AFTER_VISIBILITY: usize = 2;
pub const SPLIT_BEFORE_SCREEN_PASSES: usize = 3;
pub const SPLIT_BEFORE_SHADING: usize = 4;
pub const SPLIT_BEFORE_UPSCALE: usize = 5;
pub const SPLIT_AFTER_SCENE: usize = 6;
const SPLIT_NAMES: [&str; 7] =
    ["start", "after shadows", "after visibility", "before screen passes", "before shading", "before upscale", "after scene"];

/// Camera jumps that count as a cut: metres, and degrees of view direction, in one frame.
const CUT_DISTANCE: f32 = 2.0;
const CUT_DEGREES: f32 = 50.0;

const RING: usize = 8;
const STATS_WINDOW: usize = 240;
const CONTROL_FRAMES: usize = 120;
/// Control windows ignored after generation starts (loading hitches), and how many before judged points are tried again.
const CONTROL_SETTLE: u32 = 2;
const CONTROL_FORGET: u32 = 8;

const BLIT_WGSL: &str = r"
@group(0) @binding(0) var src: texture_2d<f32>;
override DECODE: bool = true;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// The source holds sRGB-encoded values; an sRGB target encodes what it is given,
// so it is given linear values and the stored bytes come out as they went in.
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let raw = textureLoad(src, vec2<i32>(p.xy), 0).rgb;
    let lo = raw / 12.92;
    let hi = pow((max(raw, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    let linear = select(hi, lo, raw <= vec3<f32>(0.04045));
    return vec4<f32>(select(raw, linear, DECODE), 1.0);
}
";

struct Picture {
    texture: wgpu::Texture,
    /// Render target, the swap chain's format.
    target: wgpu::TextureView,
    /// The same texels without sRGB decoding, for interpolation and the blit.
    raw: wgpu::TextureView,
}

struct Gpu {
    format: wgpu::TextureFormat,
    size: [u32; 2],
    pictures: [Picture; 2],
    interpolator: wgpu_ffx_fg::FrameInterpolator,
    blit: wgpu::RenderPipeline,
    timing: Option<Timing>,
}

/// GPU timestamps at the work before each present, read back a few frames late.
struct Timing {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: Vec<wgpu::Buffer>,
    /// 0 idle, 1 copy recorded, 2 mapping, 3 mapped.
    state: Vec<Arc<AtomicU8>>,
    /// Which frame and split each ring slot holds.
    slot_frame: Vec<(u64, usize, bool)>,
    period_ns: f32,
}

/// Measured presented-frame cadence (GPU time between the works that precede the presents).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameGenCadence {
    /// Generated frame to the real frame after it, ms.
    pub generated_to_real_ms: f32,
    /// Real frame to the next generated frame, ms.
    pub real_to_generated_ms: f32,
    /// Standard deviation of all presented intervals, ms.
    pub interval_sd_ms: f32,
    /// Frames measured in the window.
    pub frames: usize,
    /// Where the real frame is presented (`SPLIT_*`).
    pub split: usize,
}

pub struct FrameGenPass {
    requested: bool,
    forced: bool,
    gpu: Option<Gpu>,
    /// This frame's picture.
    current: usize,
    have_previous: bool,
    /// Set for this frame by `begin`.
    active: bool,
    /// The real frame's swap chain image, drawn and waiting for its split point.
    pending: Option<wgpu::SurfaceTexture>,
    /// Where the real frame is presented.
    split: usize,
    split_fixed: bool,
    /// The share each point gave when last tried (`next_split`).
    split_seen: [Option<f32>; 7],
    reset_requested: bool,
    last_view: Option<glam::Mat4>,
    frame: u64,
    debug_flags: u32,
    intervals: VecDeque<(f32, f32)>,
    last_generated_ns: Option<u64>,
    since_control: usize,
    control_windows: u32,
    log: Option<std::fs::File>,
    dump: Option<Dump>,
    /// This frame's real-frame timestamp was written.
    real_stamped: bool,
}

/// Review only (`SOMNIUM_FG_DUMP_DIR`): every `SOMNIUM_FG_DUMP_EVERY` frames (120), up to
/// `SOMNIUM_FG_DUMP_COUNT` times (12), the previous picture, the generated frame and this
/// picture, as PNGs of what was presented minus the UI: `fg_FRAME_a_previous.png`,
/// `_b_generated.png`, `_c_current.png`.
struct Dump {
    dir: std::path::PathBuf,
    every: u64,
    left: u32,
    pending: Option<DumpPending>,
}

struct DumpPending {
    frame: u64,
    size: [u32; 2],
    row: u32,
    buffers: Vec<(wgpu::Buffer, bool)>,
    mapped: Arc<AtomicU8>,
    mapping: bool,
}

impl Default for FrameGenPass {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameGenPass {
    pub fn new() -> Self {
        let fixed = std::env::var("SOMNIUM_FG_SPLIT").ok().and_then(|s| s.parse::<usize>().ok())
            .filter(|s| *s <= SPLIT_AFTER_SCENE);
        let log = std::env::var("SOMNIUM_FG_CADENCE").ok().and_then(|path| {
            use std::io::Write;
            let mut file = std::fs::File::create(&path).ok()?;
            writeln!(file, "frame\tsplit\tgenerated_to_real_ms\treal_to_generated_ms").ok()?;
            Some(file)
        });
        Self {
            requested: std::env::var("SOMNIUM_FG").as_deref() == Ok("1"),
            forced: std::env::var("SOMNIUM_FG").as_deref() == Ok("1"),
            gpu: None,
            current: 0,
            have_previous: false,
            active: false,
            pending: None,
            split: fixed.unwrap_or(SPLIT_BEFORE_SHADING),
            split_fixed: fixed.is_some(),
            split_seen: [None; 7],
            reset_requested: false,
            last_view: None,
            frame: 0,
            debug_flags: if std::env::var("SOMNIUM_FG_DEBUG").as_deref() == Ok("1") {
                wgpu_ffx_fg::DEBUG_TEAR_LINES | wgpu_ffx_fg::DEBUG_RESET_INDICATORS
            } else {
                0
            },
            intervals: VecDeque::new(),
            last_generated_ns: None,
            since_control: 0,
            control_windows: 0,
            log,
            dump: std::env::var("SOMNIUM_FG_DUMP_DIR").ok().map(|dir| {
                let number = |name: &str, default: u64| {
                    std::env::var(name).ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(default)
                };
                let dir = std::path::PathBuf::from(dir);
                let _ = std::fs::create_dir_all(&dir);
                Dump { dir, every: number("SOMNIUM_FG_DUMP_EVERY", 120).max(1), left: number("SOMNIUM_FG_DUMP_COUNT", 12) as u32,
                    pending: None }
            }),
            real_stamped: false,
        }
    }

    /// `SOMNIUM_FG=1` (review and timing runs) keeps it on whatever the game asks.
    pub fn set_requested(&mut self, on: bool) {
        let on = on || self.forced;
        if on != self.requested {
            self.requested = on;
            self.have_previous = false;
        }
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    /// Whether the frame being recorded generates.
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn request_reset(&mut self) {
        self.reset_requested = true;
    }

    /// Decide whether this frame generates, and get its resources ready.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat,
        size: [u32; 2], render_size: [u32; 2], eligible: bool) -> bool {
        self.collect_timing();
        self.collect_dump();
        self.active = self.requested && eligible && size[0] > 0 && size[1] > 0;
        if !self.active {
            self.have_previous = false;
            self.last_generated_ns = None;
            self.control_windows = 0;
            return false;
        }
        let stale = self.gpu.as_ref().is_none_or(|g| g.format != format || g.size != size);
        if stale {
            let started = std::time::Instant::now();
            let interpolator = match self.gpu.take() {
                Some(mut g) => {
                    g.interpolator.resize(device, size);
                    g.interpolator
                }
                None => wgpu_ffx_fg::FrameInterpolator::new(device, queue, size, render_size),
            };
            self.gpu = Some(Gpu {
                format,
                size,
                pictures: [picture(device, format, size), picture(device, format, size)],
                interpolator,
                blit: blit_pipeline(device, format),
                timing: Timing::new(device, queue),
            });
            self.have_previous = false;
            self.last_generated_ns = None;
            self.real_stamped = false;
            self.control_windows = 0;
            tracing::info!(?size, ?format, ms = started.elapsed().as_millis() as u64,
                "frame generation on (FSR 3 frame interpolation)");
        }
        true
    }

    /// This frame's picture, for the scene to be drawn into.
    pub(crate) fn picture_target(&self) -> Option<wgpu::TextureView> {
        let gpu = self.gpu.as_ref()?;
        Some(gpu.pictures[self.current].target.clone())
    }

    pub(crate) fn has_previous(&self) -> bool {
        self.active && self.have_previous
    }

    /// The previous picture drawn into `target` (step 1).
    pub(crate) fn blit_previous(&self, encoder: &mut wgpu::CommandEncoder, device: &wgpu::Device,
        target: &wgpu::TextureView) {
        let Some(gpu) = &self.gpu else { return };
        blit(device, encoder, &gpu.blit, &gpu.pictures[1 - self.current].raw, target);
    }

    /// Hold the real frame's image until its split point.
    pub(crate) fn hold_real(&mut self, texture: wgpu::SurfaceTexture) {
        self.pending = Some(texture);
    }

    /// Whether the real frame is presented at `point`: the controller's point, or the last one if the frame never
    /// reached it.
    pub(crate) fn take_split(&self, point: usize) -> bool {
        self.pending.is_some() && (point == self.split || point >= SPLIT_AFTER_SCENE)
    }

    /// GPU timestamp before the real frame's present (in the work submitted just ahead of it).
    pub(crate) fn stamp_real(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.stamp(encoder, 0);
    }

    pub(crate) fn present_real(&mut self, queue: &wgpu::Queue) {
        if let Some(texture) = self.pending.take() {
            queue.present(texture);
        }
    }

    /// Generate the frame between the previous picture and this one, and draw it
    /// (or, with nothing to interpolate from, this picture) into `target`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView, depth: &wgpu::Texture, motion: &wgpu::Texture, render_size: [u32; 2],
        view: glam::Mat4, proj: glam::Mat4) {
        let cut = self.reset_requested || self.last_view.is_some_and(|last| is_cut(last, view));
        if cut && self.have_previous {
            tracing::debug!("frame generation: camera cut, no interpolation this frame");
        }
        self.reset_requested = false;
        self.last_view = Some(view);
        let debug_flags = self.debug_flags;
        let have_previous = self.have_previous;
        let current = self.current;
        let Some(gpu) = self.gpu.as_mut() else { return };
        if have_previous {
            let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..Default::default()
            });
            let motion_view = motion.create_view(&Default::default());
            gpu.interpolator.dispatch(device, queue, encoder, &wgpu_ffx_fg::Inputs {
                depth: &depth_view,
                motion_vectors: &motion_view,
                motion_vector_scale: [1.0, 1.0],
                previous: &gpu.pictures[1 - current].raw,
                current: &gpu.pictures[current].raw,
                render_size,
                device_to_view: wgpu_ffx_fg::device_to_view_from_projection(proj.to_cols_array_2d()),
                reset: cut,
                debug_flags,
            });
            blit(device, encoder, &gpu.blit, gpu.interpolator.output(), target);
            if let Some(dump) = &mut self.dump
                && dump.left > 0
                && dump.pending.is_none()
                && self.frame % dump.every == 0
            {
                let size = gpu.size;
                let row = (size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
                let bgra = |f: wgpu::TextureFormat| {
                    matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb)
                };
                let sources = [
                    (&gpu.pictures[1 - current].texture, bgra(gpu.format)),
                    (gpu.interpolator.output_texture(), false),
                    (&gpu.pictures[current].texture, bgra(gpu.format)),
                ];
                let buffers = sources
                    .iter()
                    .map(|(texture, swap)| {
                        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("Frame generation dump"),
                            size: u64::from(row) * u64::from(size[1]),
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
                            buffer: &buffer,
                            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
                        }, wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 });
                        (buffer, *swap)
                    })
                    .collect();
                dump.pending = Some(DumpPending { frame: self.frame, size, row, buffers, mapped: Arc::new(AtomicU8::new(0)),
                    mapping: false });
                dump.left -= 1;
            }
        } else {
            blit(device, encoder, &gpu.blit, &gpu.pictures[current].raw, target);
        }
        self.stamp(encoder, 1);
        self.resolve(encoder);
        self.current = 1 - self.current;
        self.have_previous = true;
        self.frame += 1;
    }

    /// This frame's picture texture (for a capture of the real frame), before `generate` flips it.
    pub fn picture_texture(&self) -> Option<&wgpu::Texture> {
        Some(&self.gpu.as_ref()?.pictures[self.current].texture)
    }

    /// Cadence over the last few seconds.
    pub fn cadence(&self) -> FrameGenCadence {
        let n = self.intervals.len();
        if n == 0 {
            return FrameGenCadence { split: self.split, ..Default::default() };
        }
        let (a, b) = self.intervals.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
        let mean = (a + b) / (2 * n) as f32;
        let var = self.intervals.iter().map(|(x, y)| (x - mean).powi(2) + (y - mean).powi(2)).sum::<f32>()
            / (2 * n) as f32;
        FrameGenCadence {
            generated_to_real_ms: a / n as f32,
            real_to_generated_ms: b / n as f32,
            interval_sd_ms: var.sqrt(),
            frames: n,
            split: self.split,
        }
    }

    fn stamp(&mut self, encoder: &mut wgpu::CommandEncoder, which: u32) {
        let frame = self.frame;
        let Some(timing) = self.gpu.as_mut().and_then(|g| g.timing.as_mut()) else { return };
        let slot = (frame as usize) % RING;
        if timing.state[slot].load(Ordering::Acquire) != 0 {
            return;
        }
        encoder.write_timestamp(&timing.queries, (slot * 2) as u32 + which);
        if which == 0 {
            timing.slot_frame[slot] = (frame, self.split, true);
            self.real_stamped = true;
        }
    }

    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let frame = self.frame;
        let had_real = std::mem::take(&mut self.real_stamped);
        let Some(timing) = self.gpu.as_mut().and_then(|g| g.timing.as_mut()) else { return };
        let slot = (frame as usize) % RING;
        if timing.state[slot].load(Ordering::Acquire) != 0 {
            return;
        }
        if !had_real {
            // Nothing presented ahead of this frame: the generated stamp alone keeps the next interval.
            encoder.write_timestamp(&timing.queries, (slot * 2) as u32);
            timing.slot_frame[slot] = (frame, self.split, false);
        }
        let base = (slot * 2) as u32;
        encoder.resolve_query_set(&timing.queries, base..base + 2, &timing.resolve, (slot * 256) as u64);
        encoder.copy_buffer_to_buffer(&timing.resolve, (slot * 256) as u64, &timing.readback[slot], 0, 16);
        timing.state[slot].store(1, Ordering::Release);
    }

    /// A dump recorded last frame: map it, and write it once mapped.
    fn collect_dump(&mut self) {
        let Some(dump) = &mut self.dump else { return };
        let Some(pending) = &mut dump.pending else { return };
        if !pending.mapping {
            pending.mapping = true;
            for (buffer, _) in &pending.buffers {
                let mapped = pending.mapped.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    if r.is_ok() {
                        mapped.fetch_add(1, Ordering::AcqRel);
                    }
                });
            }
            return;
        }
        if usize::from(pending.mapped.load(Ordering::Acquire)) < pending.buffers.len() {
            return;
        }
        let pending = dump.pending.take().expect("checked above");
        let [w, h] = pending.size;
        for ((buffer, swap), name) in pending.buffers.iter().zip(["a_previous", "b_generated", "c_current"]) {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            {
                let data = buffer.slice(..).get_mapped_range().expect("mapped dump");
                for y in 0..h {
                    let start = (y * pending.row) as usize;
                    rgba.extend_from_slice(&data[start..start + (w * 4) as usize]);
                }
            }
            buffer.unmap();
            for px in rgba.chunks_exact_mut(4) {
                if *swap {
                    px.swap(0, 2);
                }
                px[3] = 255;
            }
            let path = dump.dir.join(format!("fg_{:06}_{name}.png", pending.frame));
            if let Err(error) = image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8) {
                tracing::warn!("frame generation dump {}: {error}", path.display());
            }
        }
        tracing::info!("frame generation dump: frame {} in {}", pending.frame, dump.dir.display());
    }

    /// Map last frames' timestamps and fold the ones that have landed into the cadence.
    fn collect_timing(&mut self) {
        let Some(timing) = self.gpu.as_mut().and_then(|g| g.timing.as_mut()) else { return };
        let mut landed = Vec::new();
        for slot in 0..RING {
            match timing.state[slot].load(Ordering::Acquire) {
                1 => {
                    let state = timing.state[slot].clone();
                    state.store(2, Ordering::Release);
                    timing.readback[slot].slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { 3 } else { 0 }, Ordering::Release);
                    });
                }
                3 => {
                    let pair: [u64; 2] = {
                        let view = timing.readback[slot].slice(..).get_mapped_range().expect("mapped readback");
                        bytemuck::pod_read_unaligned(&view[..16])
                    };
                    timing.readback[slot].unmap();
                    timing.state[slot].store(0, Ordering::Release);
                    let (frame, split, had_real) = timing.slot_frame[slot];
                    landed.push((frame, split, had_real, pair));
                }
                _ => {}
            }
        }
        let period = timing.period_ns;
        landed.sort_by_key(|l| l.0);
        for (frame, split, had_real, [real, generated]) in landed {
            if let (Some(last), true) = (self.last_generated_ns, had_real) {
                let to_ms = |a: u64, b: u64| (b.wrapping_sub(a) as f64 * f64::from(period) / 1.0e6) as f32;
                let (d1, d2) = (to_ms(last, real), to_ms(real, generated));
                if d1 > 0.0 && d2 > 0.0 && d1 < 250.0 && d2 < 250.0 {
                    self.record_interval(frame, split, d1, d2);
                }
            }
            self.last_generated_ns = Some(generated);
        }
    }

    fn record_interval(&mut self, frame: u64, split: usize, d1: f32, d2: f32) {
        self.intervals.push_back((d1, d2));
        while self.intervals.len() > STATS_WINDOW {
            self.intervals.pop_front();
        }
        if let Some(file) = &mut self.log {
            use std::io::Write;
            let _ = writeln!(file, "{frame}\t{split}\t{d1:.3}\t{d2:.3}");
        }
        self.since_control += 1;
        if self.since_control < CONTROL_FRAMES {
            return;
        }
        self.since_control = 0;
        let recent: Vec<_> = self.intervals.iter().rev().take(CONTROL_FRAMES).copied().collect();
        let (a, b) = recent.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
        let ratio = a / (a + b).max(1e-3);
        let c = self.cadence();
        tracing::info!(
            "frame generation cadence: generated->real {:.2} ms, real->generated {:.2} ms, interval sd {:.2} ms, split {} ({})",
            c.generated_to_real_ms, c.real_to_generated_ms, c.interval_sd_ms, self.split, SPLIT_NAMES[self.split]
        );
        self.control_windows += 1;
        if self.split_fixed || self.control_windows <= CONTROL_SETTLE {
            return;
        }
        if self.control_windows % CONTROL_FORGET == 0 {
            // Loads, levels and scenes change where the frame's time goes: judge the points again.
            self.split_seen = [None; 7];
        }
        self.split_seen[self.split] = Some(ratio);
        if let Some(next) = next_split(self.split, ratio, &self.split_seen) {
            self.split = next;
            self.intervals.clear();
        }
    }
}

impl Timing {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let features = device.features();
        if !features.contains(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
            tracing::info!("frame generation: no encoder timestamps on this device, so no cadence measurement");
            return None;
        }
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Frame generation cadence"),
                ty: wgpu::QueryType::Timestamp,
                count: (RING * 2) as u32,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Frame generation cadence resolve"),
                size: (RING * 256) as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: (0..RING)
                .map(|_| device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Frame generation cadence readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }))
                .collect(),
            state: (0..RING).map(|_| Arc::new(AtomicU8::new(0))).collect(),
            slot_frame: vec![(0, 0, false); RING],
            period_ns: queue.get_timestamp_period(),
        })
    }
}

/// A later point, a later real frame: one point later when the real frame comes too soon after the generated one
/// (`share`, the part of the real frame's period before it, under 0.44), one earlier when too late (over 0.56), but
/// not to a point already seen to be no closer to even.
fn next_split(split: usize, share: f32, seen: &[Option<f32>; 7]) -> Option<usize> {
    let next = if share < 0.44 && split < SPLIT_AFTER_SCENE {
        split + 1
    } else if share > 0.56 && split > SPLIT_START {
        split - 1
    } else {
        return None;
    };
    seen[next].is_none_or(|was| (was - 0.5).abs() < (share - 0.5).abs()).then_some(next)
}

/// Whether the view jumped between two frames (the renderer's own camera-cut test uses it too).
pub(crate) fn is_cut(last: glam::Mat4, now: glam::Mat4) -> bool {
    let (a, b) = (last.inverse(), now.inverse());
    let moved = (a.w_axis.truncate() - b.w_axis.truncate()).length();
    let fa = (-a.z_axis.truncate()).normalize_or_zero();
    let fb = (-b.z_axis.truncate()).normalize_or_zero();
    let turned = fa.dot(fb).clamp(-1.0, 1.0).acos().to_degrees();
    moved > CUT_DISTANCE || turned > CUT_DEGREES
}

fn picture(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Picture {
    let raw_format = format.remove_srgb_suffix();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Frame generation picture"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: if raw_format == format { &[] } else { std::slice::from_ref(&raw_format) },
    });
    Picture {
        target: texture.create_view(&Default::default()),
        raw: texture.create_view(&wgpu::TextureViewDescriptor { format: Some(raw_format), ..Default::default() }),
        texture,
    }
}

fn blit_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Frame generation blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let decode = if format.is_srgb() { 1.0 } else { 0.0 };
    let constants = [("DECODE", decode)];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Frame generation blit"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn blit(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline,
    src: &wgpu::TextureView, target: &wgpu::TextureView) {
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Frame generation blit"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) }],
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Frame generation blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blit_shader_validates() {
        let module = naga::front::wgsl::parse_str(BLIT_WGSL).expect("parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("validates");
    }

    #[test]
    fn the_split_moves_toward_even_halves_and_not_back_to_a_worse_point() {
        let none = [None; 7];
        assert_eq!(next_split(SPLIT_AFTER_SHADOWS, 0.11, &none), Some(SPLIT_AFTER_VISIBILITY), "too soon: later");
        assert_eq!(next_split(SPLIT_BEFORE_UPSCALE, 0.85, &none), Some(SPLIT_BEFORE_SHADING), "too late: sooner");
        assert_eq!(next_split(SPLIT_BEFORE_SHADING, 0.5, &none), None, "even: stays");
        assert_eq!(next_split(SPLIT_AFTER_SCENE, 0.1, &none), None, "nowhere later");
        let mut seen = none;
        seen[SPLIT_BEFORE_SCREEN_PASSES] = Some(0.2);
        assert_eq!(next_split(SPLIT_BEFORE_SHADING, 0.57, &seen), None, "0.2 was further from even than 0.57");
        seen[SPLIT_BEFORE_SCREEN_PASSES] = Some(0.48);
        assert_eq!(next_split(SPLIT_BEFORE_SHADING, 0.57, &seen), Some(SPLIT_BEFORE_SCREEN_PASSES));
    }

    #[test]
    fn a_teleport_or_a_snap_turn_is_a_cut_and_walking_is_not() {
        let at = |x: f32, yaw: f32| {
            glam::Mat4::look_to_rh(glam::Vec3::new(x, 1.7, 0.0), glam::Quat::from_rotation_y(yaw.to_radians()) * glam::Vec3::NEG_Z,
                glam::Vec3::Y)
        };
        assert!(!is_cut(at(0.0, 0.0), at(0.08, 3.0)), "a walking step and a small turn");
        assert!(is_cut(at(0.0, 0.0), at(5.0, 0.0)), "a teleport");
        assert!(is_cut(at(0.0, 0.0), at(0.0, 90.0)), "a cut to another angle");
    }
}
