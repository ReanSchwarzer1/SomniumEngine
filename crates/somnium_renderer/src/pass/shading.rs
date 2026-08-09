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
    /// Phase 24I: GTAO output, kept for the same reason.
    gtao_view: wgpu::TextureView,
    /// Phase 24X: scene depth, for contact shadows.
    depth_view: wgpu::TextureView,
    /// Phase 24K: traced sun visibility.
    restir_view: wgpu::TextureView,
    /// Phases 24U/25I: froxel volume, its sampler, and the range uniform.
    volumetric_view: wgpu::TextureView,
    volumetric_sampler: wgpu::Sampler,
    volumetric_range: wgpu::Buffer,
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
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
        volumetric_view: &wgpu::TextureView,
        volumetric_sampler: &wgpu::Sampler,
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
                    // Phase 24K: traced sun visibility, replacing the shadow
                    // map when ReSTIR is active.
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Phase 24X: scene depth, for the contact-shadow march.
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Phases 24U/25I: the froxel volume, its sampler, and the
                    // distance it spans (0 when volumetrics are off).
                    wgpu::BindGroupLayoutEntry {
                        binding: 9,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D3,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 10,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 11,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // Phase 24I: GTAO visibility + bent normal.
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
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
            // Phase 25K. O3DE's terrain detail sampler runs at MaxAnisotropy 16
            // (`TerrainMaterialSrg.azsli`) and the reason is terrain-shaped:
            // ground is the one surface always seen at a grazing angle, where
            // an isotropic mip is chosen for the *shorter* axis and smears
            // everything along the longer one. Trilinear alone turns a
            // photographed layer to mush a few metres out.
            anisotropy_clamp: 16,
            ..Default::default()
        });

        // 16 bytes: `vec4` is the smallest uniform WGSL will align, and only
        // x is used (the volume's range in metres, 0 when it is switched off).
        let volumetric_range = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Volumetric Range"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 16 bytes: `vec4` is the smallest uniform WGSL will align, and only x
        // is used — the volume's range in metres, 0 when it is switched off.
        let volumetric_range = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Volumetric Range"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            gtao_view,
            depth_view,
            restir_view,
            volumetric_view,
            volumetric_sampler,
            &volumetric_range,
        );

        let shader_source = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            include_str!("../shaders/brdf.wgsl"),
            // Phase 24G: Vogel disk and gradient noise, used by PCSS.
            include_str!("../shaders/sampling.wgsl"),
            // Phase 24C: the background samples the atmosphere-generated
            // cubemap and adds sharp sky detail analytically.
            include_str!("../shaders/atmosphere.wgsl"),
            // Phase 25F: stochastic hex-tiling, used by the terrain material
            // below to break the visible repetition of a tiled layer.
            include_str!("../shaders/hextile.wgsl"),
            // Phase 25A-2: terrain's splat/triplanar material, which is all
            // that survives of the separate terrain pass.
            include_str!("../shaders/terrain_material.wgsl"),
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
            gtao_view: gtao_view.clone(),
            depth_view: depth_view.clone(),
            restir_view: restir_view.clone(),
            volumetric_view: volumetric_view.clone(),
            volumetric_sampler: volumetric_sampler.clone(),
            volumetric_range,
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
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
        volumetric_view: &wgpu::TextureView,
        volumetric_sampler: &wgpu::Sampler,
        volumetric_range: &wgpu::Buffer,
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
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(gtao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(restir_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(volumetric_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(volumetric_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: volumetric_range.as_entire_binding(),
                },
            ],
        })
    }

    /// Rebuild the bind group after a resize.
    ///
    /// **Every resolution-dependent view has to be passed in again.** This used
    /// to take only the visibility view and rebuild the rest from the clones
    /// captured at construction — but GTAO, the depth buffer and the ReSTIR
    /// target all recreate their textures on resize, so those clones pointed at
    /// textures nothing wrote to any more.
    ///
    /// The result was silent and total: `gtao.a` read 0, which zeroes
    /// `surface.occlusion`, which zeroes both terms of `evaluate_ibl` — so
    /// after the first window resize *no surface in the scene received any
    /// indirect light*, contact shadows marched a dead depth buffer, and ReSTIR
    /// visibility read as "not run". The demo resizes three times during
    /// startup, so this was the state in every session.
    #[allow(clippy::too_many_arguments)]
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        visibility_view: &wgpu::TextureView,
        gtao_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        restir_view: &wgpu::TextureView,
    ) {
        self.gtao_view = gtao_view.clone();
        self.depth_view = depth_view.clone();
        self.restir_view = restir_view.clone();
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            visibility_view,
            &self.sampler,
            &self.shadow_atlas_view,
            &self.shadow_sampler,
            &self.env_view,
            &self.env_sampler,
            &self.gtao_view,
            &self.depth_view,
            &self.restir_view,
            &self.volumetric_view,
            &self.volumetric_sampler,
            &self.volumetric_range,
        );
    }

    /// Publish the volume's range for this frame. 0 disables the lookup.
    pub fn set_volumetric_range(&self, queue: &wgpu::Queue, range: f32) {
        queue.write_buffer(
            &self.volumetric_range, 0, bytemuck::bytes_of(&[range, 0.0, 0.0, 0.0]));
    }
}
