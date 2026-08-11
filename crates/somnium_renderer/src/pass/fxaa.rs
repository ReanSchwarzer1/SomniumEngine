//! Phase 15A2: FXAA anti-aliasing pass.
//!
//! ## Reference Architecture
//!
//! Timothy Lottes, *FXAA 3.11* (NVIDIA, 2011) — the console-quality preset.
//! The algorithm itself lives in `shaders/fxaa.wgsl`; this file owns the GPU
//! resources around it.
//!
//! ## Where it sits in the frame
//!
//! ```text
//! shading → HDR → [PostProcess: ACES + vignette + CA] → LDR intermediate
//!                                                          ↓  [FXAA]
//!                                                       swapchain
//!                                                          ↓
//!                              gizmos / outline / particles / UI (un-aliased)
//! ```
//!
//! FXAA needs a tone-mapped LDR image, so it runs after post-processing rather
//! than on the HDR buffer. It runs *before* the editor overlays because FXAA
//! smears UI text and thin gizmo lines.
//!
//! When disabled the pass is skipped entirely and post-processing writes
//! straight to the swapchain, so it costs nothing — no intermediate copy.

/// Uniform matching `FxaaParams` in `fxaa.wgsl` (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct FxaaParams {
    inv_size: [f32; 2],
    edge_threshold: f32,
    edge_threshold_min: f32,
}

/// Default relative contrast to treat a pixel as an edge (FXAA 3.11 "default").
const EDGE_THRESHOLD: f32 = 0.125;
/// Default absolute floor, below which pixels are left alone.
const EDGE_THRESHOLD_MIN: f32 = 0.0312;

/// FXAA pass plus the LDR intermediate target it reads from.
pub struct FxaaPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    params_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Tone-mapped LDR image: post-process writes here, FXAA reads it.
    ldr_texture: wgpu::Texture,
    /// View post-processing renders into when FXAA is enabled.
    pub ldr_view: wgpu::TextureView,
}

impl FxaaPass {
    /// Create the pass and its LDR intermediate target.
    ///
    /// `surface_format` must match the post-process pipeline's colour target,
    /// since that pass renders into `ldr_view`.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FXAA Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Linear filtering: FXAA's blur taps land between texels on purpose.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FXAA Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FXAA Params"),
            size: std::mem::size_of::<FxaaParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (ldr_texture, ldr_view) = Self::alloc_target(device, surface_format, width, height);
        let bind_group = Self::make_bind_group(
            device,
            &bind_group_layout,
            &ldr_view,
            &sampler,
            &params_buffer,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FXAA Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fxaa.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FXAA Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FXAA Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        let pass = Self {
            pipeline,
            bind_group_layout,
            bind_group,
            params_buffer,
            sampler,
            ldr_texture,
            ldr_view,
        };
        pass.write_params(None, width, height);
        pass
    }

    fn alloc_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("FXAA LDR Target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FXAA Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    /// Params depend on the target size, so they are rewritten on resize.
    /// `queue` is `None` during construction (the buffer is written on first use).
    fn write_params(&self, queue: Option<&wgpu::Queue>, width: u32, height: u32) {
        if let Some(queue) = queue {
            let params = FxaaParams {
                inv_size: [1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32],
                edge_threshold: EDGE_THRESHOLD,
                edge_threshold_min: EDGE_THRESHOLD_MIN,
            };
            queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
        }
    }

    /// Upload the per-frame params (cheap; keeps `inv_size` correct after a resize).
    pub fn update(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        self.write_params(Some(queue), width, height);
    }

    /// Recreate the LDR target at the new size.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let (tex, view) = Self::alloc_target(device, format, width, height);
        self.ldr_texture = tex;
        self.ldr_view = view;
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.ldr_view,
            &self.sampler,
            &self.params_buffer,
        );
    }

    /// Resolve the LDR intermediate onto `surface_view`.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("FXAA Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
