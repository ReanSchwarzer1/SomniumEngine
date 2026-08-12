#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterMaterialData {
    pub deep_color: [f32; 4],
    pub shallow_color: [f32; 4],
    pub edge_color: [f32; 4],
    pub absorption_roughness: [f32; 4],
    pub scattering_anisotropy: [f32; 4],
    pub bounds: [f32; 4],
    pub surface_params: [f32; 4], // clarity, edge scale, amplitude, SSR strength
    pub wave_dir_a: [f32; 2],
    pub wave_dir_b: [f32; 2],
    pub wave_params: [f32; 4],       // wavelengths A/B, speed, steepness
    pub simulation_params: [f32; 4], // spectral blend, wind speed, foam decay/threshold
    pub volume_params: [f32; 4],     // caustics, underwater enabled, reserved
    /// Vessel local-XZ origin followed by its forward direction.
    pub wake_origin_direction: [f32; 4],
    /// Speed, strength, wake length, and half-width in metres.
    pub wake_params: [f32; 4],
    /// Per-cascade `(1/tile, 1/tile, displacement_scale, normal_scale)`.
    /// Owned by the spectrum pass and overwritten on upload, so callers can
    /// leave it zeroed.
    pub cascade_scales: [[f32; 4]; 3],
}

const WATER_SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

fn spectral_texture_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WaterFrameData {
    current_view_proj: [[f32; 4]; 4],
    previous_view_proj: [[f32; 4]; 4],
    current_time: f32,
    previous_time: f32,
    history_valid: f32,
    _pad: f32,
}

pub struct WaterPass {
    pub pipeline: wgpu::RenderPipeline,
    pub view_bind_group_layout: wgpu::BindGroupLayout,
    pub mat_bind_group_layout: wgpu::BindGroupLayout,
    pub inst_bind_group_layout: wgpu::BindGroupLayout,
    pub tex_bind_group_layout: wgpu::BindGroupLayout,
    surface_texture: wgpu::Texture,
    surface_view: wgpu::TextureView,
    frame_buffer: wgpu::Buffer,
    previous_view_proj: glam::Mat4,
    previous_time: f32,
    history_valid: bool,
    pub spectrum: crate::pass::water_spectrum::WaterSpectrumPass,
}

/// Build the shared water-material textures once for the renderer. Coverage,
/// depth, and shoreline data remain per water body; these repeating surface
/// maps are common to every body and no longer belong to the demo application.
pub fn create_default_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    spectrum_views: [(&wgpu::TextureView, &wgpu::TextureView); 3],
) -> wgpu::BindGroup {
    fn view(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &str,
        format: wgpu::TextureFormat,
        fallback: [u8; 4],
    ) -> wgpu::TextureView {
        let (width, height, bytes) = match image::open(path) {
            Ok(image) => {
                let image = image.to_rgba8();
                (image.width(), image.height(), image.into_raw())
            }
            Err(error) => {
                tracing::warn!("water: {path} unavailable ({error}); using fallback texel");
                (1, 1, fallback.to_vec())
            }
        };
        let mip_count = (width.max(height) as f32).log2().floor() as u32 + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(path),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut level = image::RgbaImage::from_raw(width, height, bytes)
            .expect("water texture dimensions match byte count");
        for mip in 0..mip_count {
            let (mip_width, mip_height) = level.dimensions();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                level.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip_width * 4),
                    rows_per_image: Some(mip_height),
                },
                wgpu::Extent3d {
                    width: mip_width,
                    height: mip_height,
                    depth_or_array_layers: 1,
                },
            );
            if mip + 1 < mip_count {
                level = image::imageops::resize(
                    &level,
                    (mip_width / 2).max(1),
                    (mip_height / 2).max(1),
                    image::imageops::FilterType::Triangle,
                );
            }
        }
        texture.create_view(&Default::default())
    }

    let base = view(
        device,
        queue,
        "assets/ocean_pbr/BaseColor.png",
        wgpu::TextureFormat::Rgba8UnormSrgb,
        [20, 55, 90, 255],
    );
    let normal = view(
        device,
        queue,
        "assets/ocean_pbr/Normal_DX.png",
        wgpu::TextureFormat::Rgba8Unorm,
        [128, 128, 255, 255],
    );
    let orm = view(
        device,
        queue,
        "assets/ocean_pbr/ORM_RAO_GROUGH_BMETAL.png",
        wgpu::TextureFormat::Rgba8Unorm,
        [255, 90, 0, 255],
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Water surface sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Shared water surface textures"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&base),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&normal),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&orm),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(spectrum_views[0].0),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(spectrum_views[0].1),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(spectrum_views[1].0),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(spectrum_views[1].1),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(spectrum_views[2].0),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(spectrum_views[2].1),
            },
        ],
    })
}

impl WaterPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/water.wgsl").into()),
        });

        let view_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Water View Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        // view
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // depth texture
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Phase 22. Before this the water lit itself from a hardcoded
                    // light vector with no shadows, no environment and no scene
                    // colour, so it could never agree with the rest of the frame.
                    wgpu::BindGroupLayoutEntry {
                        // directional light (real sun + cascades)
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // shadow atlas
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // shadow comparison sampler
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // environment cubemap
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // environment sampler
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // scene colour copy (refraction)
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let mat_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Water Mat Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let inst_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Water Inst Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let tex_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Water Tex Bind Group Layout"),
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
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        // The vertex stage samples the displacement cascades
                        // through this sampler, so it cannot be fragment-only.
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    spectral_texture_layout(4),
                    spectral_texture_layout(5),
                    spectral_texture_layout(6),
                    spectral_texture_layout(7),
                    spectral_texture_layout(8),
                    spectral_texture_layout(9),
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Pipeline Layout"),
            bind_group_layouts: &[
                Some(&view_bind_group_layout),
                Some(&mat_bind_group_layout),
                Some(&inst_bind_group_layout),
                Some(&tex_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Water Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32, // Position(12), Normal(12), UV(8)
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3, // position
                        1 => Float32x3, // normal
                        2 => Float32x2, // uv
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: WATER_SURFACE_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: VELOCITY_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Disable culling for water (often good to see it from below too)
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (surface_texture, surface_view) = Self::allocate_surface(device, width, height);
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water frame history"),
            size: std::mem::size_of::<WaterFrameData>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let spectrum = crate::pass::water_spectrum::WaterSpectrumPass::new(device);
        Self {
            pipeline,
            view_bind_group_layout,
            mat_bind_group_layout,
            inst_bind_group_layout,
            tex_bind_group_layout,
            surface_texture,
            surface_view,
            frame_buffer,
            previous_view_proj: glam::Mat4::IDENTITY,
            previous_time: 0.0,
            history_valid: false,
            spectrum,
        }
    }

    fn allocate_surface(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Water surface data"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WATER_SURFACE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        (self.surface_texture, self.surface_view) = Self::allocate_surface(device, width, height);
        self.history_valid = false;
    }

    pub fn surface_view(&self) -> &wgpu::TextureView {
        &self.surface_view
    }

    pub fn clear_surface(&self, encoder: &mut wgpu::CommandEncoder) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Clear water surface data"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.surface_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        global_view_proj_buffer: &wgpu::Buffer,
        visibility_depth_texture_view: &wgpu::TextureView,
        light_buffer: &wgpu::Buffer,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        scene_copy_view: &wgpu::TextureView,
        velocity_view: &wgpu::TextureView,
        current_view_proj: glam::Mat4,
        current_time: f32,
        geometry_vertex_buffer: &wgpu::Buffer,
        geometry_index_buffer: &wgpu::Buffer,
        water_textures_bind_group: Option<&wgpu::BindGroup>,
        water_bodies: &crate::water_body::WaterBodyRegistry,
        water_queue: &[(u32, glam::Mat4, WaterMaterialData, u32, u32, u32)],
    ) {
        if water_queue.is_empty() {
            return;
        }

        // The cascades are shared, so the first visible body's authored wind
        // and foam controls define this frame's ocean state. Per-body blend
        // remains in the material and can still disable spectral displacement.
        let effective_simulation = self.spectrum.record(
            queue,
            encoder,
            current_time,
            water_queue[0].2.simulation_params,
        );

        queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&WaterFrameData {
                current_view_proj: current_view_proj.to_cols_array_2d(),
                previous_view_proj: self.previous_view_proj.to_cols_array_2d(),
                current_time,
                previous_time: self.previous_time,
                history_valid: f32::from(u8::from(self.history_valid)),
                _pad: 0.0,
            }),
        );

        // Create view bind group
        let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Water View Bind Group"),
            layout: &self.view_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: global_view_proj_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(visibility_depth_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(shadow_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(scene_copy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.frame_buffer.as_entire_binding(),
                },
            ],
        });

        // Pack materials and instances
        // To avoid multiple small buffer creations, we'll create one buffer for all instances and one for all materials,
        // but since materials are uniform buffers and instances are storage, we'll create one uniform buffer per material
        // (or an array, but we bind one at a time for simplicity).
        for (water_id, transform, water, v_off, i_off, i_cnt) in water_queue {
            let Some(body) = water_bodies.get(*water_id) else {
                continue;
            };
            let mat_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Mat Buffer"),
                size: std::mem::size_of::<WaterMaterialData>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut gpu_water = *water;
            gpu_water.simulation_params = effective_simulation;
            gpu_water.cascade_scales = self.spectrum.map_scales();
            queue.write_buffer(&mat_buffer, 0, bytemuck::bytes_of(&gpu_water));

            let mat_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Water Mat Bind Group"),
                layout: &self.mat_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: mat_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&body.mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&body.depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&body.shore_sdf_view),
                    },
                ],
            });

            // Instance Buffer (80 bytes)
            // struct Instance { model: mat4x4<f32>, _pad: vec4<f32> }
            let mut inst_data = Vec::with_capacity(80);
            inst_data.extend_from_slice(bytemuck::bytes_of(&transform.to_cols_array()));
            inst_data.extend_from_slice(bytemuck::bytes_of(&[0.0f32; 4]));

            let inst_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Inst Buffer"),
                size: 80,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&inst_buffer, 0, &inst_data);

            let inst_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Water Inst Bind Group"),
                layout: &self.inst_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: inst_buffer.as_entire_binding(),
                }],
            });

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Water Render Pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: velocity_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &view_bind_group, &[]);
            rpass.set_bind_group(1, &mat_bg, &[]);
            rpass.set_bind_group(2, &inst_bg, &[]);
            if let Some(tex_bg) = water_textures_bind_group {
                rpass.set_bind_group(3, tex_bg, &[]);
            }
            rpass.set_vertex_buffer(0, geometry_vertex_buffer.slice(..));
            rpass.set_index_buffer(geometry_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(*i_off..(*i_off + *i_cnt), *v_off as i32, 0..1);
        }
        self.previous_view_proj = current_view_proj;
        self.previous_time = current_time;
        self.history_valid = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{WaterFrameData, WaterMaterialData};

    #[test]
    fn gpu_structs_match_their_sixteen_byte_wgsl_layouts() {
        // 208 bytes of surface/optics state plus three cascade scale vectors.
        assert_eq!(std::mem::size_of::<WaterMaterialData>(), 256);
        assert_eq!(std::mem::size_of::<WaterFrameData>(), 144);
        assert_eq!(std::mem::size_of::<WaterMaterialData>() % 16, 0);
    }
}
