//! AMD FSR 3 temporal upscaler wrapper (wgpu-ffx).
//!
//! Reconstructs the internal HDR target to display resolution. Replaces
//! Somnium TAA and the bilinear present blit while enabled. RCAS is FSR's
//! own sharpener — Somnium CAS stays off on this path so they do not stack.
//!
//! Colour is compressed with TAA's exposure-aware Karis curve before FSR and
//! expanded after. Raw cd/m² makes Lanczos undershoot go black at every
//! high-contrast edge — the same failure TAA hit before `tonemap_for_blend`.
//!
//! Frame generation is not here: it needs a DXGI/Vulkan swapchain proxy.

use wgpu_ffx::{
    FsrContext, FsrContextFlags, FsrContextInfo, FsrDispatchFlags, FsrDispatchInfo, FsrView,
    get_jitter_offset, get_jitter_phase_count,
};

use crate::pass::postprocess::HDR_FORMAT;

/// Matches hello_engine / clustered lighting.
const CAMERA_NEAR: f32 = 0.1;
const CAMERA_FAR: f32 = 1000.0;

struct FsrGpu {
    context: FsrContext,
    view: FsrView,
    /// FSR writes compressed colour here.
    compressed: wgpu::Texture,
    /// Linear HDR after untonemap; this is what post-process samples.
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    dilated_depth: wgpu::Texture,
    dilated_motion_vectors: wgpu::Texture,
    reconstructed_previous_depth: wgpu::Buffer,
    sanitized: wgpu::Texture,
    depth_f32: wgpu::Texture,
    exposure_buf: wgpu::Buffer,
    sanitize_bgl: wgpu::BindGroupLayout,
    sanitize_pipeline: wgpu::ComputePipeline,
    untonemap_bgl: wgpu::BindGroupLayout,
    untonemap_pipeline: wgpu::ComputePipeline,
    render_size: [u32; 2],
    upscale_size: [u32; 2],
    frame_index: i32,
}

pub struct FsrPass {
    gpu: Option<FsrGpu>,
    pub enabled: bool,
    pub sharpness: f32,
}

impl FsrPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_w: u32,
        render_h: u32,
        display_w: u32,
        display_h: u32,
    ) -> Self {
        // wgpu-ffx needs adapter storage formats, SPIR-V passthrough (GLSL
        // `coherent` on images), r16snorm (Lanczos LUT), and filterable r32float.
        // Requested as `FSR_FEATURES` in `context.rs`.
        let want = std::env::var("SOMNIUM_FSR").as_deref() != Ok("0");
        let supported = device.features().contains(crate::context::FSR_FEATURES);
        if want && !supported {
            tracing::warn!(
                "FSR 3 skipped: device lacks FSR_FEATURES (storage formats, passthrough, r16snorm, filterable r32, clear_texture)"
            );
        }
        let gpu =
            supported.then(|| alloc_gpu(device, queue, render_w, render_h, display_w, display_h));
        Self {
            gpu,
            enabled: want && supported,
            sharpness: 0.8,
        }
    }

    pub fn output_view(&self) -> &wgpu::TextureView {
        &self
            .gpu
            .as_ref()
            .expect("FSR output after a successful dispatch")
            .output_view
    }

    pub fn set_enabled(&mut self, on: bool) {
        let on = on && self.gpu.is_some();
        if on != self.enabled {
            if let Some(gpu) = &mut self.gpu {
                gpu.frame_index = 0;
            }
        }
        self.enabled = on;
    }

    /// Clip-space translation for this frame, in NDC.
    ///
    /// FSR Halton is pixels, +X right / +Y down. Bevy's wgpu path (and AMD's
    /// jitter-space diagram) is `(offset * (2, -2)) / resolution` added to
    /// `z_axis` — not `translate * projection`, which inverts the offset on
    /// `perspective_rh`.
    pub fn jitter_ndc(&self, width: u32, height: u32) -> glam::Vec2 {
        if !self.enabled || width == 0 || height == 0 {
            return glam::Vec2::ZERO;
        }
        let [x, y] = self.jitter_pixels();
        glam::Vec2::new(x * 2.0 / width as f32, -y * 2.0 / height as f32)
    }

    fn jitter_pixels(&self) -> [f32; 2] {
        let Some(gpu) = &self.gpu else {
            return [0.0, 0.0];
        };
        let phase =
            get_jitter_phase_count(gpu.render_size[0] as i32, gpu.upscale_size[0] as i32).max(1);
        get_jitter_offset(gpu.frame_index, phase)
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_w: u32,
        render_h: u32,
        display_w: u32,
        display_h: u32,
    ) {
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        let render = clamp_render(render_w, render_h);
        let upscale = clamp_upscale(display_w, display_h);
        if render == gpu.render_size && upscale == gpu.upscale_size {
            return;
        }
        gpu.view.resize(queue, render, upscale);
        gpu.compressed = alloc_hdr(device, "FSR Compressed", upscale);
        let (output, output_view) = alloc_output(device, upscale);
        gpu.output = output;
        gpu.output_view = output_view;
        gpu.dilated_depth = alloc_r32(device, "FSR Dilated Depth", render);
        gpu.dilated_motion_vectors = alloc_rg16(device, "FSR Dilated Motion", render);
        gpu.reconstructed_previous_depth = alloc_prev_depth(device, render);
        gpu.sanitized = alloc_hdr(device, "FSR Sanitized HDR", render);
        gpu.depth_f32 = alloc_r32(device, "FSR Depth F32", render);
        gpu.render_size = render;
        gpu.upscale_size = upscale;
        gpu.frame_index = 0;
    }

    /// Reconstruct `color` (HDR at render res) into the display-sized output.
    ///
    /// Returns `true` when the dispatch ran. On validation failure the caller
    /// keeps the existing TAA/blit path for this frame.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
        motion_vectors: &wgpu::Texture,
        exposure: f32,
        proj: glam::Mat4,
        frame_delta_seconds: f32,
        reset_history: bool,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(gpu) = &mut self.gpu else {
            return false;
        };

        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        queue.write_buffer(
            &gpu.exposure_buf,
            0,
            bytemuck::bytes_of(&[exposure.max(1e-8), 0.0, 0.0, 0.0]),
        );
        let sanitized_view = gpu.sanitized.create_view(&Default::default());
        let depth_f32_view = gpu.depth_f32.create_view(&Default::default());
        let sanitize_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FSR Sanitize"),
            layout: &gpu.sanitize_bgl,
            entries: &[
                bind_view(0, &color_view),
                bind_view(1, &sanitized_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: gpu.exposure_buf.as_entire_binding(),
                },
                bind_view(3, &depth_view),
                bind_view(4, &depth_f32_view),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("FSR Sanitize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&gpu.sanitize_pipeline);
            pass.set_bind_group(0, &sanitize_bg, &[]);
            pass.dispatch_workgroups(
                gpu.render_size[0].div_ceil(8),
                gpu.render_size[1].div_ceil(8),
                1,
            );
        }

        let jitter = {
            let phase =
                get_jitter_phase_count(gpu.render_size[0] as i32, gpu.upscale_size[0] as i32)
                    .max(1);
            get_jitter_offset(gpu.frame_index, phase)
        };
        let fov_y = fov_y_from_proj(proj);
        let info = FsrDispatchInfo {
            color: gpu.sanitized.clone(),
            depth: gpu.depth_f32.clone(),
            motion_vectors: motion_vectors.clone(),
            exposure: None,
            reactive_mask: None,
            transparency_and_composition: None,
            dilated_depth: gpu.dilated_depth.clone(),
            dilated_motion_vectors: gpu.dilated_motion_vectors.clone(),
            reconstructed_previous_depth: gpu.reconstructed_previous_depth.clone(),
            output: gpu.compressed.clone(),
            jitter_offset: jitter,
            // Velocity is `prev_uv - current_uv`: motion from current to
            // previous, which is what FSR wants. Scale UV → pixels.
            motion_vector_scale: [gpu.render_size[0] as f32, gpu.render_size[1] as f32],
            render_size: gpu.render_size,
            upscale_size: gpu.upscale_size,
            enable_sharpening: self.sharpness > 0.0,
            sharpness: self.sharpness.clamp(0.0, 1.0),
            frame_time_delta: (frame_delta_seconds * 1000.0).max(1.0),
            pre_exposure: 1.0,
            reset_history: reset_history || gpu.frame_index == 0,
            camera_near: CAMERA_NEAR,
            camera_far: CAMERA_FAR,
            camera_fov_y: fov_y,
            view_space_to_meters_factor: 1.0,
            flags: FsrDispatchFlags::empty(),
        };
        match gpu.context.dispatch(&mut gpu.view, encoder, &info) {
            Ok(()) => {
                let compressed_view = gpu.compressed.create_view(&Default::default());
                let linear_view = gpu.output.create_view(&Default::default());
                let untonemap_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("FSR Untonemap"),
                    layout: &gpu.untonemap_bgl,
                    entries: &[
                        bind_view(0, &compressed_view),
                        bind_view(1, &linear_view),
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: gpu.exposure_buf.as_entire_binding(),
                        },
                    ],
                });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("FSR Untonemap"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&gpu.untonemap_pipeline);
                    pass.set_bind_group(0, &untonemap_bg, &[]);
                    pass.dispatch_workgroups(
                        gpu.upscale_size[0].div_ceil(8),
                        gpu.upscale_size[1].div_ceil(8),
                        1,
                    );
                }
                gpu.frame_index = gpu.frame_index.wrapping_add(1);
                true
            }
            Err(err) => {
                tracing::warn!("FSR dispatch skipped: {err}");
                false
            }
        }
    }
}

fn bind_view<'a>(binding: u32, view: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn alloc_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    render_w: u32,
    render_h: u32,
    display_w: u32,
    display_h: u32,
) -> FsrGpu {
    // Colour is already exposure-normalised Karis 0–1, so the HDR / auto-exposure
    // shader permutations would double-apply a curve FSR is not expecting.
    let context = FsrContext::new(FsrContextInfo {
        device: device.clone(),
        flags: FsrContextFlags::empty(),
    });
    let render = clamp_render(render_w, render_h);
    let upscale = clamp_upscale(display_w, display_h);
    let view = context.create_view(queue, render, upscale);
    let compressed = alloc_hdr(device, "FSR Compressed", upscale);
    let (output, output_view) = alloc_output(device, upscale);
    let dilated_depth = alloc_r32(device, "FSR Dilated Depth", render);
    let dilated_motion_vectors = alloc_rg16(device, "FSR Dilated Motion", render);
    let reconstructed_previous_depth = alloc_prev_depth(device, render);
    let sanitized = alloc_hdr(device, "FSR Sanitized HDR", render);
    let depth_f32 = alloc_r32(device, "FSR Depth F32", render);
    let exposure_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("FSR Exposure"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let (sanitize_bgl, sanitize_pipeline) = alloc_sanitize_pipeline(device);
    let (untonemap_bgl, untonemap_pipeline) = alloc_untonemap_pipeline(device);
    FsrGpu {
        context,
        view,
        compressed,
        output,
        output_view,
        dilated_depth,
        dilated_motion_vectors,
        reconstructed_previous_depth,
        sanitized,
        depth_f32,
        exposure_buf,
        sanitize_bgl,
        sanitize_pipeline,
        untonemap_bgl,
        untonemap_pipeline,
        render_size: render,
        upscale_size: upscale,
        frame_index: 0,
    }
}

fn fov_y_from_proj(proj: glam::Mat4) -> f32 {
    let f = proj.y_axis.y;
    if f.abs() < 1.0e-6 {
        45.0_f32.to_radians()
    } else {
        2.0 * (1.0 / f).atan()
    }
}

fn clamp_render(w: u32, h: u32) -> [u32; 2] {
    [w.max(2), h.max(2)]
}

fn clamp_upscale(w: u32, h: u32) -> [u32; 2] {
    [w.max(1), h.max(1)]
}

fn sampled_tex(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_tex(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn alloc_sanitize_pipeline(
    device: &wgpu::Device,
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("FSR Sanitize BGL"),
        entries: &[
            sampled_tex(0, false),
            storage_tex(1, HDR_FORMAT),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            storage_tex(4, wgpu::TextureFormat::R32Float),
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("FSR Sanitize"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fsr_sanitize.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("FSR Sanitize PL"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("FSR Sanitize"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (bgl, pipeline)
}

fn alloc_untonemap_pipeline(
    device: &wgpu::Device,
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("FSR Untonemap BGL"),
        entries: &[
            sampled_tex(0, false),
            storage_tex(1, HDR_FORMAT),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
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
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("FSR Untonemap"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fsr_untonemap.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("FSR Untonemap PL"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("FSR Untonemap"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (bgl, pipeline)
}

fn alloc_hdr(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn alloc_output(device: &wgpu::Device, size: [u32; 2]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = alloc_hdr(device, "FSR Output", size);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn alloc_r32(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn alloc_rg16(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn alloc_prev_depth(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("FSR Reconstructed Previous Depth"),
        size: u64::from(size[0]) * u64::from(size[1]) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
