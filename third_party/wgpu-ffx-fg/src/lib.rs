//! AMD FidelityFX FSR 3 frame interpolation for wgpu.
//!
//! A WGSL translation of the FidelityFX SDK 1.1.4 frame interpolation and
//! optical flow effects (MIT, Advanced Micro Devices; see `LICENSE`). Between two
//! rendered back buffers it builds the frame half way: optical flow on the back
//! buffers' luma, the game's dilated motion vectors and depth, disocclusion, and
//! inpainting from a pyramid, in the same passes and the same order as AMD's
//! `ffxOpticalflowDispatch` and `ffxFrameInterpolationDispatch`.
//!
//! Presentation and pacing (AMD's swapchain proxy) are not here; the caller
//! presents the generated frame and the real one. The `shaders/*.wgsl` headers
//! list what differs from the HLSL and why.

use std::num::NonZeroU64;

/// Optical flow pyramid levels (AMD's 7: full size and six halvings).
pub const OPTICAL_FLOW_LEVELS: usize = 7;
/// Optical flow block size in pixels.
pub const OPTICAL_FLOW_BLOCK_SIZE: u32 = 8;
/// Debug flag: green and coloured bars at the left and right edges of generated frames.
pub const DEBUG_TEAR_LINES: u32 = 1;
/// Debug flag: red band on a reset, blue band on a detected scene change.
pub const DEBUG_RESET_INDICATORS: u32 = 2;

const SLOT: u64 = 256;
const PYRAMID_SLOTS: usize = 16;
const SCD_BINS: u64 = 256 * 9;

const FG_COMMON: &str = include_str!("shaders/fg_common.wgsl");
const FG_GAME_FIELD: &str = include_str!("shaders/fg_game_field.wgsl");
const FG_PYRAMID: &str = include_str!("shaders/fg_pyramid.wgsl");
const OF_COMMON: &str = include_str!("shaders/of_common.wgsl");
const OF_LUMA: &str = include_str!("shaders/of_luma.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FgConstants {
    render_size: [i32; 2],
    display_size: [i32; 2],
    max_render_size: [i32; 2],
    optical_flow_size: [i32; 2],
    device_to_view: [f32; 4],
    optical_flow_scale: [f32; 2],
    jitter: [f32; 2],
    motion_vector_scale: [f32; 2],
    reset: u32,
    dispatch_flags: u32,
    rect_base: [i32; 2],
    rect_size: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OfConstants {
    luma_size: [i32; 2],
    level: u32,
    level_count: u32,
    frame_index: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PyramidParams {
    src_size: [i32; 2],
    dst_size: [i32; 2],
    mode: u32,
    _pad: [u32; 3],
}

/// Every pass as (label, complete WGSL source), in no particular order.
pub fn shader_sources() -> Vec<(&'static str, String)> {
    let fi = |pass: &str| format!("{FG_COMMON}\n{pass}");
    let fi_field = |pass: &str| format!("{FG_COMMON}\n{FG_GAME_FIELD}\n{pass}");
    let fi_pyramid = |pass: &str| format!("{FG_COMMON}\n{FG_PYRAMID}\n{pass}");
    let of = |pass: &str| format!("{OF_COMMON}\n{pass}");
    let of_luma = |pass: &str| format!("{OF_COMMON}\n{OF_LUMA}\n{pass}");
    vec![
        ("OF prepare luma", of(include_str!("shaders/of_prepare_luma.wgsl"))),
        ("OF luma pyramid", of(include_str!("shaders/of_luma_pyramid.wgsl"))),
        ("OF SCD histogram", of(include_str!("shaders/of_scd_histogram.wgsl"))),
        ("OF SCD divergence", of(include_str!("shaders/of_scd_divergence.wgsl"))),
        ("OF compute", of_luma(include_str!("shaders/of_compute_optical_flow.wgsl"))),
        ("OF filter", of(include_str!("shaders/of_filter_optical_flow.wgsl"))),
        ("OF scale", of_luma(include_str!("shaders/of_scale_optical_flow.wgsl"))),
        ("FI prepare", fi(include_str!("shaders/fg_prepare.wgsl"))),
        ("FI setup", fi(include_str!("shaders/fg_setup.wgsl"))),
        ("FI reconstruct previous depth", fi(include_str!("shaders/fg_reconstruct_previous_depth.wgsl"))),
        ("FI game motion vector field", fi(include_str!("shaders/fg_game_motion_vector_field.wgsl"))),
        ("FI pyramid game field", fi_pyramid(include_str!("shaders/fg_pyramid_game_field.wgsl"))),
        ("FI pyramid colour", fi_pyramid(include_str!("shaders/fg_pyramid_color.wgsl"))),
        ("FI pyramid reduce", fi_pyramid(include_str!("shaders/fg_pyramid_reduce.wgsl"))),
        ("FI optical flow vector field", fi(include_str!("shaders/fg_optical_flow_vector_field.wgsl"))),
        ("FI disocclusion mask", fi_field(include_str!("shaders/fg_disocclusion_mask.wgsl"))),
        ("FI interpolate", fi_field(include_str!("shaders/fg_interpolate.wgsl"))),
        ("FI inpaint", fi(include_str!("shaders/fg_inpaint.wgsl"))),
    ]
}

/// AMD's `fDeviceToViewDepth` from a column-major projection matrix (glam's
/// `to_cols_array_2d`) with depth 0 at the near plane: view depth is
/// `[1] / (device_depth - [0])`, and `[2]`, `[3]` undo the x and y scale.
pub fn device_to_view_from_projection(m: [[f32; 4]; 4]) -> [f32; 4] {
    [-m[2][2], m[3][2], 1.0 / m[0][0], 1.0 / m[1][1]]
}

/// What one generated frame is made from.
pub struct Inputs<'a> {
    /// Depth at render size (depth aspect of a `Depth32Float`), 0 near, 1 far.
    pub depth: &'a wgpu::TextureView,
    /// Motion at render size, `previous_uv - current_uv` once scaled, unjittered.
    pub motion_vectors: &'a wgpu::TextureView,
    /// Scale from stored motion to UV (1, 1 when it is UV already).
    pub motion_vector_scale: [f32; 2],
    /// The back buffer before (display size, filterable, sRGB-encoded values: a
    /// non-sRGB view of an sRGB texture). No UI in it.
    pub previous: &'a wgpu::TextureView,
    /// The back buffer now, likewise.
    pub current: &'a wgpu::TextureView,
    /// Render size this frame (at most the size given to `new`/`resize`).
    pub render_size: [u32; 2],
    /// [`device_to_view_from_projection`] of this frame's (unjittered) projection.
    pub device_to_view: [f32; 4],
    /// A cut: no interpolation across it.
    pub reset: bool,
    /// [`DEBUG_TEAR_LINES`], [`DEBUG_RESET_INDICATORS`].
    pub debug_flags: u32,
}

struct Pipelines {
    of_prepare_luma: wgpu::ComputePipeline,
    of_luma_pyramid: wgpu::ComputePipeline,
    of_scd_histogram: wgpu::ComputePipeline,
    of_scd_divergence: wgpu::ComputePipeline,
    of_compute: wgpu::ComputePipeline,
    of_filter: wgpu::ComputePipeline,
    of_scale: wgpu::ComputePipeline,
    fi_prepare: wgpu::ComputePipeline,
    fi_setup: wgpu::ComputePipeline,
    fi_reconstruct: wgpu::ComputePipeline,
    fi_game_field: wgpu::ComputePipeline,
    fi_pyramid_game_field: wgpu::ComputePipeline,
    fi_pyramid_color: wgpu::ComputePipeline,
    fi_pyramid_reduce: wgpu::ComputePipeline,
    fi_of_field: wgpu::ComputePipeline,
    fi_disocclusion: wgpu::ComputePipeline,
    fi_interpolate: wgpu::ComputePipeline,
    fi_inpaint: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let sources = shader_sources();
        let make = |label: &str| {
            let (_, source) = sources.iter().find(|(l, _)| *l == label).expect("known pass");
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            of_prepare_luma: make("OF prepare luma"),
            of_luma_pyramid: make("OF luma pyramid"),
            of_scd_histogram: make("OF SCD histogram"),
            of_scd_divergence: make("OF SCD divergence"),
            of_compute: make("OF compute"),
            of_filter: make("OF filter"),
            of_scale: make("OF scale"),
            fi_prepare: make("FI prepare"),
            fi_setup: make("FI setup"),
            fi_reconstruct: make("FI reconstruct previous depth"),
            fi_game_field: make("FI game motion vector field"),
            fi_pyramid_game_field: make("FI pyramid game field"),
            fi_pyramid_color: make("FI pyramid colour"),
            fi_pyramid_reduce: make("FI pyramid reduce"),
            fi_of_field: make("FI optical flow vector field"),
            fi_disocclusion: make("FI disocclusion mask"),
            fi_interpolate: make("FI interpolate"),
            fi_inpaint: make("FI inpaint"),
        }
    }
}

/// Per-pixel state at render size; grown when the render size outgrows it.
struct RenderSized {
    capacity: [u32; 2],
    dilated_depth: wgpu::Buffer,
    dilated_motion: wgpu::Buffer,
    reconstructed_previous_depth: wgpu::Buffer,
    reconstructed_interpolated_depth: wgpu::Buffer,
    game_mv_x: wgpu::Buffer,
    game_mv_y: wgpu::Buffer,
    disocclusion: wgpu::TextureView,
    disocclusion_storage: wgpu::TextureView,
}

/// State at display size.
struct DisplaySized {
    display: [u32; 2],
    optical_flow_size: [u32; 2],
    of_mv_x: wgpu::Buffer,
    of_mv_y: wgpu::Buffer,
    pyramid_mips: u32,
    pyramid_all: wgpu::TextureView,
    pyramid_sampled: Vec<wgpu::TextureView>,
    pyramid_storage: Vec<wgpu::TextureView>,
    interpolated: wgpu::TextureView,
    interpolated_storage: wgpu::TextureView,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    output_storage: wgpu::TextureView,
    /// Luma pyramids, two frames: `[set][level]`.
    luma: [Vec<wgpu::Buffer>; 2],
    /// Per level: search result, filtered result, prediction from the level above.
    flow_search: Vec<wgpu::Buffer>,
    flow_filtered: Vec<wgpu::Buffer>,
    flow_prediction: Vec<wgpu::Buffer>,
}

/// FSR 3 frame interpolation: pipelines, state, and the per-frame dispatch.
pub struct FrameInterpolator {
    pipelines: Pipelines,
    sampler: wgpu::Sampler,
    fg_uniform: wgpu::Buffer,
    of_uniform: wgpu::Buffer,
    pyramid_uniform: wgpu::Buffer,
    counters: wgpu::Buffer,
    scd_histogram: wgpu::Buffer,
    scd_previous: wgpu::Buffer,
    scd_next_previous: wgpu::Buffer,
    scd_temp: wgpu::Buffer,
    scd_output: wgpu::Buffer,
    render: RenderSized,
    display: DisplaySized,
    luma_set: usize,
    of_frame_index: u32,
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC | extra,
        mapped_at_creation: false,
    })
}

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat, mips: u32,
    extra: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    })
}

fn mip_view(texture: &wgpu::Texture, mip: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: mip,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn area(size: [u32; 2]) -> u64 {
    u64::from(size[0].max(1)) * u64::from(size[1].max(1))
}

fn level_size(display: [u32; 2], level: usize) -> [u32; 2] {
    [display[0] >> level, display[1] >> level]
}

fn flow_size(display: [u32; 2], level: usize) -> [u32; 2] {
    let s = level_size(display, level);
    [s[0].div_ceil(OPTICAL_FLOW_BLOCK_SIZE), s[1].div_ceil(OPTICAL_FLOW_BLOCK_SIZE)]
}

/// Mips of the inpainting pyramid: half the display, halved to 1x1, at most 13 (AMD's count).
fn pyramid_mip_count(display: [u32; 2]) -> u32 {
    let half = (display[0] / 2).max(display[1] / 2).max(1);
    (32 - half.leading_zeros()).min(13)
}

impl RenderSized {
    fn new(device: &wgpu::Device, capacity: [u32; 2]) -> Self {
        let a = area(capacity);
        let disocclusion = texture(device, "FI disocclusion mask", capacity, wgpu::TextureFormat::Rgba8Unorm, 1,
            wgpu::TextureUsages::empty());
        Self {
            capacity,
            dilated_depth: buffer(device, "FI dilated depth", a * 4, wgpu::BufferUsages::empty()),
            dilated_motion: buffer(device, "FI dilated motion", a * 8, wgpu::BufferUsages::empty()),
            reconstructed_previous_depth: buffer(device, "FI reconstructed previous depth", a * 4,
                wgpu::BufferUsages::empty()),
            reconstructed_interpolated_depth: buffer(device, "FI reconstructed interpolated depth", a * 4,
                wgpu::BufferUsages::empty()),
            game_mv_x: buffer(device, "FI game motion field x", a * 4, wgpu::BufferUsages::empty()),
            game_mv_y: buffer(device, "FI game motion field y", a * 4, wgpu::BufferUsages::empty()),
            disocclusion: disocclusion.create_view(&Default::default()),
            disocclusion_storage: disocclusion.create_view(&Default::default()),
        }
    }
}

impl DisplaySized {
    fn new(device: &wgpu::Device, display: [u32; 2]) -> Self {
        let display = [display[0].max(16), display[1].max(16)];
        let of_size = flow_size(display, 0);
        let pyramid_mips = pyramid_mip_count(display);
        let pyramid = texture(device, "FI inpainting pyramid", [display[0] / 2, display[1] / 2],
            wgpu::TextureFormat::Rgba16Float, pyramid_mips, wgpu::TextureUsages::empty());
        let interpolated = texture(device, "FI interpolated", display, wgpu::TextureFormat::Rgba8Unorm, 1,
            wgpu::TextureUsages::empty());
        let output = texture(device, "FI output", display, wgpu::TextureFormat::Rgba8Unorm, 1,
            wgpu::TextureUsages::COPY_SRC);
        let luma_set = |set: usize| {
            (0..OPTICAL_FLOW_LEVELS)
                .map(|l| buffer(device, &format!("OF luma {set} level {l}"), area(level_size(display, l)) * 4,
                    wgpu::BufferUsages::empty()))
                .collect::<Vec<_>>()
        };
        let flow = |name: &str| {
            (0..OPTICAL_FLOW_LEVELS)
                .map(|l| buffer(device, &format!("OF {name} level {l}"), area(flow_size(display, l)) * 8,
                    wgpu::BufferUsages::empty()))
                .collect::<Vec<_>>()
        };
        Self {
            display,
            optical_flow_size: of_size,
            of_mv_x: buffer(device, "FI optical flow field x", area(of_size) * 4, wgpu::BufferUsages::empty()),
            of_mv_y: buffer(device, "FI optical flow field y", area(of_size) * 4, wgpu::BufferUsages::empty()),
            pyramid_mips,
            pyramid_all: pyramid.create_view(&Default::default()),
            pyramid_sampled: (0..pyramid_mips).map(|m| mip_view(&pyramid, m)).collect(),
            pyramid_storage: (0..pyramid_mips).map(|m| mip_view(&pyramid, m)).collect(),
            interpolated: interpolated.create_view(&Default::default()),
            interpolated_storage: interpolated.create_view(&Default::default()),
            output_view: output.create_view(&Default::default()),
            output_storage: output.create_view(&Default::default()),
            output,
            luma: [luma_set(0), luma_set(1)],
            flow_search: flow("search"),
            flow_filtered: flow("filtered"),
            flow_prediction: flow("prediction"),
        }
    }
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn slot(buffer: &wgpu::Buffer, index: usize, size: u64) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: index as u64 * SLOT,
        size: NonZeroU64::new(size),
    })
}

fn run(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, label: &str,
    entries: &[wgpu::BindGroupEntry<'_>], groups: [u32; 3]) {
    if groups.contains(&0) {
        return;
    }
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &pipeline.get_bind_group_layout(0),
        entries,
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(label), timestamp_writes: None });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
}

fn groups8(size: [u32; 2]) -> [u32; 3] {
    [size[0].div_ceil(8), size[1].div_ceil(8), 1]
}

impl FrameInterpolator {
    /// Compiles the passes and allocates state for `display` and render sizes up to `max_render`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, display: [u32; 2], max_render: [u32; 2]) -> Self {
        let uniform = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let scd_previous = buffer(device, "OF SCD previous histogram", SCD_BINS * 4, wgpu::BufferUsages::COPY_DST);
        // A uniform distribution, so the first divergence is finite (AMD's zeroed
        // buffer gives 0 * log(0) there; the first frames count as changed anyway).
        queue.write_buffer(&scd_previous, 0, bytemuck::cast_slice(&vec![1.0_f32 / 256.0; SCD_BINS as usize]));
        Self {
            pipelines: Pipelines::new(device),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("FI linear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            fg_uniform: uniform("FI constants", std::mem::size_of::<FgConstants>() as u64),
            of_uniform: uniform("OF constants", SLOT * OPTICAL_FLOW_LEVELS as u64),
            pyramid_uniform: uniform("FI pyramid params", SLOT * 2 * PYRAMID_SLOTS as u64),
            counters: buffer(device, "FI counters", 16, wgpu::BufferUsages::empty()),
            scd_histogram: buffer(device, "OF SCD histogram", SCD_BINS * 4, wgpu::BufferUsages::empty()),
            scd_previous,
            scd_next_previous: buffer(device, "OF SCD next previous histogram", SCD_BINS * 4,
                wgpu::BufferUsages::COPY_SRC),
            scd_temp: buffer(device, "OF SCD temp", 16, wgpu::BufferUsages::empty()),
            scd_output: buffer(device, "OF scene change detection", 16, wgpu::BufferUsages::empty()),
            render: RenderSized::new(device, max_render),
            display: DisplaySized::new(device, display),
            luma_set: 0,
            of_frame_index: 0,
        }
    }

    /// Reallocates for a new display size (render state grows on its own).
    pub fn resize(&mut self, device: &wgpu::Device, display: [u32; 2]) {
        if self.display.display != [display[0].max(16), display[1].max(16)] {
            self.display = DisplaySized::new(device, display);
            self.of_frame_index = 0;
        }
    }

    /// The display size the state is allocated for.
    pub fn display_size(&self) -> [u32; 2] {
        self.display.display
    }

    /// The last generated frame (`Rgba8Unorm`, sRGB-encoded values, display size).
    pub fn output(&self) -> &wgpu::TextureView {
        &self.display.output_view
    }

    /// The texture behind [`Self::output`], for copies.
    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.display.output
    }

    /// Records optical flow and interpolation for one generated frame into
    /// `encoder`. The result is in [`Self::output`] once the encoder runs.
    pub fn dispatch(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder,
        inputs: &Inputs<'_>) {
        let render = [inputs.render_size[0].max(1), inputs.render_size[1].max(1)];
        if render[0] > self.render.capacity[0] || render[1] > self.render.capacity[1] {
            let grown = [render[0].max(self.render.capacity[0]), render[1].max(self.render.capacity[1])];
            self.render = RenderSized::new(device, grown);
        }
        if inputs.reset {
            self.of_frame_index = 0;
        }
        self.write_constants(queue, inputs, render);
        self.record_optical_flow(device, encoder, inputs);
        self.record_interpolation(device, encoder, inputs, render);
        self.luma_set ^= 1;
        self.of_frame_index = self.of_frame_index.saturating_add(1);
    }

    fn write_constants(&self, queue: &wgpu::Queue, inputs: &Inputs<'_>, render: [u32; 2]) {
        let d = &self.display;
        let i2 = |s: [u32; 2]| [s[0] as i32, s[1] as i32];
        let fg = FgConstants {
            render_size: i2(render),
            display_size: i2(d.display),
            max_render_size: i2(self.render.capacity),
            optical_flow_size: i2(d.optical_flow_size),
            device_to_view: inputs.device_to_view,
            optical_flow_scale: [1.0 / d.display[0] as f32, 1.0 / d.display[1] as f32],
            jitter: [0.0, 0.0],
            motion_vector_scale: inputs.motion_vector_scale,
            reset: u32::from(inputs.reset),
            dispatch_flags: inputs.debug_flags,
            rect_base: [0, 0],
            rect_size: i2(d.display),
        };
        queue.write_buffer(&self.fg_uniform, 0, bytemuck::bytes_of(&fg));

        let mut of = vec![0_u8; (SLOT as usize) * OPTICAL_FLOW_LEVELS];
        for level in 0..OPTICAL_FLOW_LEVELS {
            let c = OfConstants {
                luma_size: i2(d.display),
                level: level as u32,
                level_count: OPTICAL_FLOW_LEVELS as u32,
                frame_index: self.of_frame_index,
                _pad: [0; 3],
            };
            let at = level * SLOT as usize;
            of[at..at + std::mem::size_of::<OfConstants>()].copy_from_slice(bytemuck::bytes_of(&c));
        }
        queue.write_buffer(&self.of_uniform, 0, &of);

        // Slots 0..16: game field pyramid (mode 0); 16..32: colour pyramid (mode 1).
        let mut pyramid = vec![0_u8; (SLOT as usize) * 2 * PYRAMID_SLOTS];
        for (mode, source) in [(0_u32, render), (1, d.display)] {
            let mut src = source;
            for mip in 0..(d.pyramid_mips as usize).min(PYRAMID_SLOTS) {
                let dst = [src[0] / 2, src[1] / 2];
                let p = PyramidParams { src_size: i2(src), dst_size: i2(dst), mode, _pad: [0; 3] };
                let at = (mode as usize * PYRAMID_SLOTS + mip) * SLOT as usize;
                pyramid[at..at + std::mem::size_of::<PyramidParams>()].copy_from_slice(bytemuck::bytes_of(&p));
                src = dst;
            }
        }
        queue.write_buffer(&self.pyramid_uniform, 0, &pyramid);
    }

    fn record_optical_flow(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, inputs: &Inputs<'_>) {
        let p = &self.pipelines;
        let d = &self.display;
        let current = &d.luma[self.luma_set];
        let previous = &d.luma[self.luma_set ^ 1];
        let of_size = std::mem::size_of::<OfConstants>() as u64;
        let level0 = level_size(d.display, 0);

        run(device, encoder, &p.of_prepare_luma, "OF prepare luma", &[
            entry(0, slot(&self.of_uniform, 0, of_size)),
            entry(1, wgpu::BindingResource::TextureView(inputs.current)),
            entry(2, current[0].as_entire_binding()),
        ], [level0[0].div_ceil(16), level0[1].div_ceil(16), 1]);
        for level in 1..OPTICAL_FLOW_LEVELS {
            run(device, encoder, &p.of_luma_pyramid, "OF luma pyramid", &[
                entry(0, slot(&self.of_uniform, level, of_size)),
                entry(1, current[level - 1].as_entire_binding()),
                entry(2, current[level].as_entire_binding()),
            ], groups8(level_size(d.display, level)));
        }

        // Scene change detection on this frame's full-size luma.
        let strata_x = (level0[0] / 3).div_ceil(4);
        run(device, encoder, &p.of_scd_histogram, "OF SCD histogram", &[
            entry(0, slot(&self.of_uniform, 0, of_size)),
            entry(1, current[0].as_entire_binding()),
            entry(2, self.scd_histogram.as_entire_binding()),
        ], [strata_x.div_ceil(32), 16, 9]);
        run(device, encoder, &p.of_scd_divergence, "OF SCD divergence", &[
            entry(1, self.scd_histogram.as_entire_binding()),
            entry(2, self.scd_previous.as_entire_binding()),
            entry(3, self.scd_next_previous.as_entire_binding()),
            entry(4, self.scd_temp.as_entire_binding()),
            entry(5, self.scd_output.as_entire_binding()),
        ], [9, 3, 1]);
        encoder.copy_buffer_to_buffer(&self.scd_next_previous, 0, &self.scd_previous, 0, SCD_BINS * 4);
        encoder.clear_buffer(&self.scd_histogram, 0, None);

        // Coarse to fine.
        for level in (0..OPTICAL_FLOW_LEVELS).rev() {
            let luma_level = level_size(d.display, level);
            run(device, encoder, &p.of_compute, "OF compute", &[
                entry(0, slot(&self.of_uniform, level, of_size)),
                entry(1, current[level].as_entire_binding()),
                entry(2, previous[level].as_entire_binding()),
                entry(3, d.flow_prediction[level].as_entire_binding()),
                entry(4, d.flow_search[level].as_entire_binding()),
                entry(5, self.scd_output.as_entire_binding()),
            ], [luma_level[0].div_ceil(16), luma_level[1].div_ceil(16), 1]);
            run(device, encoder, &p.of_filter, "OF filter", &[
                entry(0, slot(&self.of_uniform, level, of_size)),
                entry(1, d.flow_search[level].as_entire_binding()),
                entry(2, d.flow_filtered[level].as_entire_binding()),
            ], groups8(flow_size(d.display, level)));
            if level > 0 {
                let next = flow_size(d.display, level - 1);
                run(device, encoder, &p.of_scale, "OF scale", &[
                    entry(0, slot(&self.of_uniform, level, of_size)),
                    entry(1, current[level].as_entire_binding()),
                    entry(2, previous[level].as_entire_binding()),
                    entry(3, d.flow_filtered[level].as_entire_binding()),
                    entry(4, d.flow_prediction[level - 1].as_entire_binding()),
                    entry(5, self.scd_output.as_entire_binding()),
                ], groups8(next));
            }
        }
    }

    fn record_interpolation(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, inputs: &Inputs<'_>,
        render: [u32; 2]) {
        let p = &self.pipelines;
        let r = &self.render;
        let d = &self.display;
        let fg = || entry(0, self.fg_uniform.as_entire_binding());
        let view = wgpu::BindingResource::TextureView;
        let pyramid_size = std::mem::size_of::<PyramidParams>() as u64;

        for b in [&r.game_mv_x, &r.game_mv_y, &r.reconstructed_previous_depth, &r.reconstructed_interpolated_depth,
            &d.of_mv_x, &d.of_mv_y] {
            encoder.clear_buffer(b, 0, None);
        }

        run(device, encoder, &p.fi_prepare, "FI prepare", &[
            fg(),
            entry(1, view(inputs.depth)),
            entry(2, view(inputs.motion_vectors)),
            entry(3, r.dilated_depth.as_entire_binding()),
            entry(4, r.dilated_motion.as_entire_binding()),
            entry(5, r.reconstructed_previous_depth.as_entire_binding()),
        ], groups8(render));
        run(device, encoder, &p.fi_setup, "FI setup", &[
            fg(),
            entry(1, self.counters.as_entire_binding()),
            entry(2, self.scd_output.as_entire_binding()),
        ], [1, 1, 1]);
        run(device, encoder, &p.fi_reconstruct, "FI reconstruct previous depth", &[
            fg(),
            entry(1, r.dilated_depth.as_entire_binding()),
            entry(2, r.dilated_motion.as_entire_binding()),
            entry(3, r.reconstructed_interpolated_depth.as_entire_binding()),
        ], groups8(render));
        run(device, encoder, &p.fi_game_field, "FI game motion vector field", &[
            fg(),
            entry(1, r.dilated_depth.as_entire_binding()),
            entry(2, r.dilated_motion.as_entire_binding()),
            entry(3, view(inputs.previous)),
            entry(4, view(inputs.current)),
            entry(5, wgpu::BindingResource::Sampler(&self.sampler)),
            entry(6, r.game_mv_x.as_entire_binding()),
            entry(7, r.game_mv_y.as_entire_binding()),
        ], groups8(render));

        // Game vector field pyramid (slots 0..).
        self.record_pyramid(device, encoder, 0, render, pyramid_size, |encoder, dst, params, size| {
            run(device, encoder, &p.fi_pyramid_game_field, "FI pyramid game field", &[
                fg(),
                entry(1, params),
                entry(2, view(dst)),
                entry(3, r.game_mv_x.as_entire_binding()),
                entry(4, r.game_mv_y.as_entire_binding()),
            ], groups8(size));
        });

        run(device, encoder, &p.fi_of_field, "FI optical flow vector field", &[
            fg(),
            entry(1, d.flow_filtered[0].as_entire_binding()),
            entry(2, view(inputs.previous)),
            entry(3, view(inputs.current)),
            entry(4, wgpu::BindingResource::Sampler(&self.sampler)),
            entry(5, d.of_mv_x.as_entire_binding()),
            entry(6, d.of_mv_y.as_entire_binding()),
        ], groups8(d.optical_flow_size));
        run(device, encoder, &p.fi_disocclusion, "FI disocclusion mask", &[
            fg(),
            entry(1, r.reconstructed_interpolated_depth.as_entire_binding()),
            entry(2, r.reconstructed_previous_depth.as_entire_binding()),
            entry(3, r.dilated_depth.as_entire_binding()),
            entry(4, r.game_mv_x.as_entire_binding()),
            entry(5, r.game_mv_y.as_entire_binding()),
            entry(6, view(&d.pyramid_all)),
            entry(7, view(&r.disocclusion_storage)),
        ], groups8(render));
        run(device, encoder, &p.fi_interpolate, "FI interpolate", &[
            fg(),
            entry(1, r.game_mv_x.as_entire_binding()),
            entry(2, r.game_mv_y.as_entire_binding()),
            entry(3, view(&d.pyramid_all)),
            entry(4, d.of_mv_x.as_entire_binding()),
            entry(5, d.of_mv_y.as_entire_binding()),
            entry(6, view(&r.disocclusion)),
            entry(7, view(inputs.previous)),
            entry(8, view(inputs.current)),
            entry(9, wgpu::BindingResource::Sampler(&self.sampler)),
            entry(10, self.counters.as_entire_binding()),
            entry(11, view(&d.interpolated_storage)),
        ], groups8(d.display));

        // Colour pyramid (slots PYRAMID_SLOTS..), then the fill.
        self.record_pyramid(device, encoder, 1, d.display, pyramid_size, |encoder, dst, params, size| {
            run(device, encoder, &p.fi_pyramid_color, "FI pyramid colour", &[
                fg(),
                entry(1, params),
                entry(2, view(dst)),
                entry(3, view(&d.interpolated)),
            ], groups8(size));
        });
        run(device, encoder, &p.fi_inpaint, "FI inpaint", &[
            fg(),
            entry(1, view(&d.interpolated)),
            entry(2, view(&d.pyramid_all)),
            entry(3, self.scd_output.as_entire_binding()),
            entry(4, view(&d.output_storage)),
        ], groups8(d.display));
    }

    /// Mip 0 through `first`, then each further mip from the one above.
    fn record_pyramid<'a>(&'a self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, mode: usize,
        source: [u32; 2], params_size: u64,
        first: impl FnOnce(&mut wgpu::CommandEncoder, &'a wgpu::TextureView, wgpu::BindingResource<'a>, [u32; 2])) {
        let d = &self.display;
        let mips = (d.pyramid_mips as usize).min(PYRAMID_SLOTS);
        let mut size = [source[0] / 2, source[1] / 2];
        first(encoder, &d.pyramid_storage[0], slot(&self.pyramid_uniform, mode * PYRAMID_SLOTS, params_size), size);
        for mip in 1..mips {
            size = [size[0] / 2, size[1] / 2];
            if size[0] == 0 || size[1] == 0 {
                break;
            }
            run(device, encoder, &self.pipelines.fi_pyramid_reduce, "FI pyramid reduce", &[
                entry(1, slot(&self.pyramid_uniform, mode * PYRAMID_SLOTS + mip, params_size)),
                entry(2, wgpu::BindingResource::TextureView(&d.pyramid_storage[mip])),
                entry(3, wgpu::BindingResource::TextureView(&d.pyramid_sampled[mip - 1])),
            ], groups8(size));
        }
    }
}

#[cfg(test)]
mod tests;
