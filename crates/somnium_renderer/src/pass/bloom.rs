//! Bloom as a lens response (Phase 24T).
//!
//! Builds a mip chain by progressive downsampling, then sums it back up with a
//! tent filter. No threshold: see the note at the top of `bloom.wgsl` for why a
//! threshold is the wrong shape of question once exposure is physical.

/// Levels in the chain.
///
/// Six halvings takes a 1080p frame to about 17 px, which is wide enough for a
/// convincing falloff. More levels keep going until the mip is a pixel or two
/// and mostly cost bandwidth for a contribution nobody can see.
const BLOOM_MIPS: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomParams {
    src_texel: [f32; 2],
    filter_radius: f32,
    intensity: f32,
}

pub struct BloomPass {
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// One uniform per level; they differ in texel size, and a single buffer
    /// rewritten between draws would only ever show the last value.
    params: Vec<wgpu::Buffer>,
    views: Vec<wgpu::TextureView>,
    sizes: Vec<(u32, u32)>,
    binds: Vec<wgpu::BindGroup>,
    format: wgpu::TextureFormat,
    pub enabled: bool,
    pub intensity: f32,
}

impl BloomPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("bloom.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bloom BGL"),
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

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let make = |entry: &str, label: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // Upsampling blends additively so each level accumulates onto the one
        // above rather than replacing it.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };

        let mut pass = Self {
            downsample: make("fs_downsample", "Bloom downsample", None),
            upsample: make("fs_upsample", "Bloom upsample", Some(additive)),
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Bloom sampler"),
                // Clamped: wrapping would fold the far edge of the screen into
                // the blur, so a bright light on the left would glow on the right.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            params: Vec::new(),
            views: Vec::new(),
            sizes: Vec::new(),
            binds: Vec::new(),
            format,
            enabled: true,
            intensity: 0.04,
        };
        pass.resize(device, width, height);
        pass
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.views.clear();
        self.sizes.clear();
        self.params.clear();
        self.binds.clear();

        let (mut w, mut h) = (width.max(1), height.max(1));
        for i in 0..BLOOM_MIPS {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Bloom mip"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.views
                .push(tex.create_view(&wgpu::TextureViewDescriptor::default()));
            self.sizes.push((w, h));
            let _ = i;
        }
    }

    /// The blurred chain's top level, for the post-process pass to add in.
    pub fn result_view(&self) -> &wgpu::TextureView {
        &self.views[0]
    }

    pub fn intensity(&self) -> f32 {
        if self.enabled { self.intensity } else { 0.0 }
    }

    /// Build the chain from `source`.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        source_size: (u32, u32),
    ) {
        if !self.enabled {
            return;
        }

        // Bind groups and uniforms are rebuilt per frame: each pass reads a
        // different level, and the set only changes on resize, so this is a
        // handful of small allocations rather than a hot path.
        self.params.clear();
        self.binds.clear();

        let make_bind = |src_view: &wgpu::TextureView, texel: [f32; 2], radius: f32| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Bloom params"),
                size: std::mem::size_of::<BloomParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &buf,
                0,
                bytemuck::bytes_of(&BloomParams {
                    src_texel: texel,
                    filter_radius: radius,
                    intensity: self.intensity,
                }),
            );
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Bloom BG"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buf.as_entire_binding(),
                    },
                ],
            });
            (buf, bind)
        };

        // ── Down ────────────────────────────────────────────────────────────
        let mut prev_size = source_size;
        let mut down_binds = Vec::new();
        for i in 0..self.views.len() {
            let src = if i == 0 { source } else { &self.views[i - 1] };
            let texel = [1.0 / prev_size.0 as f32, 1.0 / prev_size.1 as f32];
            let (buf, bind) = make_bind(src, texel, 1.0);
            self.params.push(buf);
            down_binds.push(bind);
            prev_size = self.sizes[i];
        }

        for (i, bind) in down_binds.iter().enumerate() {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom downsample"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[i],
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
            rpass.set_pipeline(&self.downsample);
            rpass.set_bind_group(0, bind, &[]);
            rpass.draw(0..3, 0..1);
        }

        // ── Up ──────────────────────────────────────────────────────────────
        // From the smallest level back toward the largest, adding as it goes.
        let mut up_binds = Vec::new();
        for i in (1..self.views.len()).rev() {
            let (sw, sh) = self.sizes[i];
            let texel = [1.0 / sw as f32, 1.0 / sh as f32];
            let (buf, bind) = make_bind(&self.views[i], texel, 2.0);
            self.params.push(buf);
            up_binds.push((i - 1, bind));
        }

        for (dst, bind) in &up_binds {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom upsample"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[*dst],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // Load, not clear: the downsampled content at this level
                        // is what the coarser level is being added *onto*.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.upsample);
            rpass.set_bind_group(0, bind, &[]);
            rpass.draw(0..3, 0..1);
        }

        self.binds = down_binds;
    }
}
