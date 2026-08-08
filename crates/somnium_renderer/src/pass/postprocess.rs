//! Phase 11.5K: Post-processing pass — ACES tone mapping + vignette.
//!
//! Owns the Rgba16Float HDR render target that the shading and grid passes
//! write into. On each frame, reads the HDR texture and writes tone-mapped
//! LDR output to the swapchain surface view.
use wgpu;

/// HDR render target format shared by all passes that write to the HDR buffer.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Post-processing pass: HDR texture → tone-mapped swapchain output.
pub struct PostProcessPass {
    pub pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub hdr_texture: wgpu::Texture,
    pub hdr_view: wgpu::TextureView,
    /// Pre-water snapshot of `hdr_texture`, sampled by the water pass for
    /// refraction (Phase 22).
    pub scene_copy_texture: wgpu::Texture,
    pub scene_copy_view: wgpu::TextureView,
    params_buffer: wgpu::Buffer,
    /// Handle to the auto-exposure result, kept so the bind group can be
    /// rebuilt on resize without threading it back through the caller.
    exposure_buffer: wgpu::Buffer,
    /// Bloom chain, kept so the bind group survives a resize.
    bloom_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl PostProcessPass {
    /// Create the post-process pass.
    ///
    /// - `surface_format`: swapchain format (pipeline output target).
    /// - `width` / `height`: initial render dimensions in physical pixels.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        // Result buffer of `crate::pass::auto_exposure::AutoExposurePass`.
        exposure_buffer: &wgpu::Buffer,
        bloom_view: &wgpu::TextureView,
    ) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PostProcess Bind Group Layout"),
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
                    // Phase 24T: the blurred bloom chain.
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Phase 24A-3: metered exposure, written by the
                    // auto-exposure compute pass earlier in the frame.
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcess Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Default: exposure=1.0, vignette_strength=1.0.
        let params_buffer = {
            // Must match `PostParams` in postprocess.wgsl exactly: three floats
            // then four u32s carried as raw bits. `copy_from_slice` below
            // requires the lengths to agree.
            let data: [f32; 8] = [
                1.0 / (1.2 * 32768.0), // exposure at EV100 15
                1.0,                   // vignette strength
                0.0,                   // chromatic aberration
                f32::from_bits(0),     // tonemapper: AgX
                f32::from_bits(1),     // auto exposure on
                0.0,
                0.0,
                0.0,
            ];
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("PostProcess Params"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: true,
            });
            buf.slice(..)
                .get_mapped_range_mut()
                .copy_from_slice(bytemuck::bytes_of(&data));
            buf.unmap();
            buf
        };

        let (hdr_texture, hdr_view) = Self::make_hdr_texture(device, width, height);
        let (scene_copy_texture, scene_copy_view) =
            Self::make_scene_copy_texture(device, width, height);
        let bind_group = Self::make_bind_group(
            device, &bind_group_layout, &hdr_view, &sampler, &params_buffer,
            exposure_buffer,
            bloom_view,
        );
        let exposure_buffer = exposure_buffer.clone();
        let bloom_view = bloom_view.clone();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PostProcess Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/postprocess.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PostProcess Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PostProcess Pipeline"),
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
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            exposure_buffer,
            bloom_view,
            pipeline,
            bind_group_layout,
            bind_group,
            hdr_texture,
            hdr_view,
            scene_copy_texture,
            scene_copy_view,
            params_buffer,
            sampler,
        }
    }

    fn make_hdr_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HDR Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                // Phase 22: the water pass refracts what is behind the surface,
                // which means sampling the scene colour. A pass cannot sample
                // the target it renders into, so the HDR texture is copied into
                // `scene_copy_texture` first.
                | wgpu::TextureUsages::COPY_SRC
                // Phase 24F: TAA resolves into its own history buffer and
                // copies the result back here, so every later pass keeps
                // reading one well-known target instead of a view that
                // alternates between two ping-pong textures each frame.
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Snapshot of the HDR target taken before the water pass, so water can
    /// sample the scene behind it for refraction. Same size and format as the
    /// HDR texture, so a straight texture-to-texture copy is enough.
    fn make_scene_copy_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Scene Colour Copy"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params_buffer: &wgpu::Buffer,
        exposure_buffer: &wgpu::Buffer,
        bloom_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PostProcess Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: exposure_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(bloom_view),
                },
            ],
        })
    }

    /// Recreate the HDR texture at the new dimensions (called on window resize).
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (hdr_texture, hdr_view) = Self::make_hdr_texture(device, width, height);
        self.hdr_texture = hdr_texture;
        self.hdr_view = hdr_view;
        let (scene_copy_texture, scene_copy_view) =
            Self::make_scene_copy_texture(device, width, height);
        self.scene_copy_texture = scene_copy_texture;
        self.scene_copy_view = scene_copy_view;
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.hdr_view,
            &self.sampler,
            &self.params_buffer,
            &self.exposure_buffer,
            &self.bloom_view,
        );
    }

    /// Upload exposure and vignette strength to the GPU params buffer.
    /// Upload the post-process parameters.
    ///
    /// `ca_strength` is the chromatic-aberration offset in UV units at the
    /// screen edge; `0.0` disables it (all three channels sample the same texel).
    pub fn set_params(
        &self,
        queue: &wgpu::Queue,
        exposure: f32,
        vignette_strength: f32,
        ca_strength: f32,
        tonemapper: u32,
        auto_exposure: bool,
        bloom_intensity: f32,
    ) {
        // The uniform is three floats plus a u32; bytemuck cannot cast a mixed
        // array, so the index is bit-cast into the fourth float slot and read
        // back as a u32 in WGSL.
        // Three floats then four u32s. bytemuck cannot cast a mixed array, so
        // the integers ride in float slots as raw bits and WGSL reads them back
        // as u32 — same four bytes either way.
        let data: [f32; 8] = [
            exposure,
            vignette_strength,
            ca_strength,
            f32::from_bits(tonemapper),
            f32::from_bits(u32::from(auto_exposure)),
            bloom_intensity,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&data));
    }

    /// Record the post-process pass into `encoder`, writing to `surface_view`.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PostProcess Pass"),
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
