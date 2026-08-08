//! Shading pass that reads the visibility buffer and outputs final color.
//!
//! Phase 11 additions:
//!   @group(1) binding 2 — shadow_atlas (texture_depth_2d)
//!   @group(1) binding 3 — shadow_sampler (sampler_comparison for PCF)
use wgpu;

pub struct ShadingPass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    // Stored for bind-group recreation on resize / shadow atlas change.
    sampler: wgpu::Sampler,
    shadow_atlas_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    /// Phase 19: environment cubemap + its sampler, kept so the bind group can
    /// be rebuilt on resize.
    env_view: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
}

impl ShadingPass {
    pub fn new(
        device: &wgpu::Device,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        visibility_view: &wgpu::TextureView,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Shading Pass Bind Group Layout"),
                entries: &[
                    // binding 0: vis_buffer (R32Uint)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 1: default filtering sampler for PBR texture lookups
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // binding 2: shadow atlas depth texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 3: PCF comparison sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                    // binding 4: prefiltered environment cubemap (Phase 19)
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 5: trilinear sampler for the environment map
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Default Shading Sampler"),
            // glTF's default wrap is REPEAT. wgpu's Default is ClampToEdge,
            // which smears the edge texel across everything whose UVs leave
            // 0..1 — the cause of the stretched/streaked look on imported
            // models with tiled materials.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group = Self::make_bind_group(
            device,
            &bind_group_layout,
            visibility_view,
            &sampler,
            shadow_atlas_view,
            shadow_sampler,
            env_view,
            env_sampler,
        );

        let shader_source = format!(
            "{}\n{}\n{}",
            include_str!("../shaders/brdf.wgsl"),
            // Phase 24C: the background samples the atmosphere-generated
            // cubemap and adds sharp sky detail analytically.
            include_str!("../shaders/atmosphere.wgsl"),
            include_str!("../shaders/shading.wgsl")
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shading Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shading Pipeline Layout"),
            bind_group_layouts: &[Some(global_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shading Pipeline"),
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
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            sampler,
            shadow_atlas_view: shadow_atlas_view.clone(),
            shadow_sampler: shadow_sampler.clone(),
            env_view: env_view.clone(),
            env_sampler: env_sampler.clone(),
        }
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        visibility_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shading Pass Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(visibility_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shadow_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
            ],
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, visibility_view: &wgpu::TextureView) {
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            visibility_view,
            &self.sampler,
            &self.shadow_atlas_view,
            &self.shadow_sampler,
            &self.env_view,
            &self.env_sampler,
        );
    }
}
