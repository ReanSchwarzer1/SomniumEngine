//! Depth of field (Phase 24Z).
//!
//! Driven by the same aperture the exposure model already uses. In a real
//! camera those are one number: opening up brightens the frame *and* throws the
//! background out of focus, and there is no setting that does one without the
//! other. Keeping them linked is most of what makes the camera feel like a
//! camera rather than a set of independent sliders.

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DofParams {
    inv_resolution: [f32; 2],
    focus_distance: f32,
    aperture: f32,
    focal_length: f32,
    max_coc: f32,
    near: f32,
    far: f32,
}

pub struct DofPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    bind: Option<wgpu::BindGroup>,
    format: wgpu::TextureFormat,

    pub enabled: bool,
    /// Metres. What the lens is focused on.
    pub focus_distance: f32,
    /// Lens focal length in millimetres. 50 is a normal lens.
    pub focal_length_mm: f32,
    /// Aperture as an f-number, matching `PostProcessComponent`.
    pub f_stop: f32,
}

impl DofPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        // MORROWIND-C: composition is declared in `dof.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("dof.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dof.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DoF BGL"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DoF PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("DoF Pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (target, target_view) = Self::make_target(device, format, width, height);

        Self {
            pipeline,
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("DoF sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("DoF params"),
                size: std::mem::size_of::<DofParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            target,
            target_view,
            bind: None,
            format,
            // Off by default: an editor viewport wants everything readable, and
            // a blurred background is a deliberate choice rather than a default.
            enabled: false,
            focus_distance: 10.0,
            focal_length_mm: 50.0,
            f_stop: 5.6,
        }
    }

    fn make_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DoF target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (t, v) = Self::make_target(device, self.format, width, height);
        self.target = t;
        self.target_view = v;
        self.bind = None;
    }

    pub fn ensure_bind_group(
        &mut self,
        device: &wgpu::Device,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) {
        if self.bind.is_some() {
            return;
        }
        self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DoF BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        }));
    }

    /// Run the pass. Returns the texture holding the result, for the caller to
    /// copy back over the colour target.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        near: f32,
        far: f32,
    ) -> Option<&wgpu::Texture> {
        if !self.enabled {
            return None;
        }
        let bind = self.bind.as_ref()?;

        let focal_length = self.focal_length_mm / 1000.0;
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&DofParams {
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                focus_distance: self.focus_distance,
                // Aperture *diameter*, which is what the thin-lens equation
                // wants — the f-number is focal length over diameter.
                aperture: focal_length / self.f_stop.max(0.5),
                focal_length,
                max_coc: 24.0,
                near,
                far,
            }),
        );

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Depth of Field"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.target_view,
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
        rpass.set_bind_group(0, bind, &[]);
        rpass.draw(0..3, 0..1);
        drop(rpass);

        Some(&self.target)
    }
}
