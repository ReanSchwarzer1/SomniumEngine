//! Auto-exposure by luminance histogram (Phase 24A-3).
//!
//! Meters the HDR target each frame and adapts an EV100 value toward it, so a
//! scene lit in physical units stays readable as the light changes — walking
//! from noon into a cave, or watching the sun set, without touching a slider.
//!
//! The result lives in a GPU buffer the post-process pass reads directly. It is
//! deliberately never read back to the CPU: a readback would cost a stall or a
//! frame of latency, and nothing on the CPU needs the number.

use wgpu::util::DeviceExt;

/// Luminance range the histogram covers, as log2 cd/m².
///
/// −10 is about starlight on a surface, +20 is well past a sunlit white wall.
/// Anything outside clamps into the end bins, which is the right behaviour: the
/// sun's own disc should saturate the meter rather than redefine its scale.
const MIN_LOG_LUM: f32 = -10.0;
const MAX_LOG_LUM: f32 = 20.0;

/// GPU-side settings, matching `ExposureParams` in `auto_exposure.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureParams {
    inv_log_range: f32,
    min_log_lum: f32,
    delta_time: f32,
    speed_down: f32,
    speed_up: f32,
    exposure_compensation: f32,
    min_ev100: f32,
    max_ev100: f32,
    highlight_start_nits: f32,
    highlight_end_nits: f32,
    _pad0: f32,
    _pad1: f32,
}

/// Histogram build + resolve, plus the two-float exposure result buffer.
pub struct AutoExposurePass {
    build_pipeline: wgpu::ComputePipeline,
    resolve_pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    histogram: wgpu::Buffer,
    /// `[0]` = linear exposure multiplier, `[1]` = adapted EV100.
    exposure: wgpu::Buffer,
    params: wgpu::Buffer,
}

impl AutoExposurePass {
    pub fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("auto_exposure.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("auto_exposure.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("AutoExposure BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(1, false),
                storage_entry(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("AutoExposure PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let make = |entry: &str, label: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        let histogram = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("AutoExposure histogram"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Seed EV100 with a sentinel below any real value, so the first metered
        // frame snaps straight to its target instead of easing in from an
        // arbitrary starting point and flashing.
        let exposure = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("AutoExposure result"),
            contents: bytemuck::cast_slice(&[1.0f32 / (1.2 * 32768.0), -1000.0f32]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("AutoExposure params"),
            size: std::mem::size_of::<ExposureParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            build_pipeline: make("build_histogram", "AutoExposure build"),
            resolve_pipeline: make("resolve_exposure", "AutoExposure resolve"),
            layout,
            bind_group: None,
            histogram,
            exposure,
            params,
        }
    }

    /// The buffer holding `[multiplier, ev100]`, for the post-process pass.
    pub fn exposure_buffer(&self) -> &wgpu::Buffer {
        &self.exposure
    }

    /// Rebuild the bind group against a (possibly resized) HDR view.
    pub fn resize(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("AutoExposure BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.histogram.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.exposure.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        }));
    }

    /// Meter the frame. `delta_time` drives adaptation, so the eye adjusts at
    /// the same rate regardless of frame rate.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        delta_time: f32,
        exposure_compensation: f32,
    ) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };

        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&ExposureParams {
                inv_log_range: 1.0 / (MAX_LOG_LUM - MIN_LOG_LUM),
                min_log_lum: MIN_LOG_LUM,
                // A long frame (a hitch, or a breakpoint) must not teleport the
                // exposure; clamping keeps adaptation smooth across stalls.
                delta_time: delta_time.clamp(0.0, 0.1),
                speed_down: 3.0,
                speed_up: 1.0,
                exposure_compensation,
                min_ev100: -8.0,
                max_ev100: 18.0,
                // Above roughly a sunlit white surface, samples start losing
                // their vote — a glint should not decide the exposure.
                highlight_start_nits: 8_000.0,
                highlight_end_nits: 40_000.0,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("AutoExposure"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, bind_group, &[]);

        pass.set_pipeline(&self.build_pipeline);
        pass.dispatch_workgroups(width.div_ceil(16), height.div_ceil(16), 1);

        // The resolve also zeroes the histogram, so no separate clear is needed
        // between frames.
        pass.set_pipeline(&self.resolve_pipeline);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
