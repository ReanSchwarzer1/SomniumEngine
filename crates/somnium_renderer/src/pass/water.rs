
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterMaterialData {
    pub deep_color: [f32; 4],
    pub shallow_color: [f32; 4],
    pub edge_color: [f32; 4],
    pub clarity: f32,
    pub edge_scale: f32,
    pub amplitude: f32,
    pub _pad0: f32,
    pub coord_scale: [f32; 2],
    pub coord_offset: [f32; 2],
    pub wave_dir_a: [f32; 2],
    pub wave_dir_b: [f32; 2],
    pub wave_blend: f32,
    pub _pad1: [f32; 3],
}

pub struct WaterPass {
    pub pipeline: wgpu::RenderPipeline,
    pub view_bind_group_layout: wgpu::BindGroupLayout,
    pub mat_bind_group_layout: wgpu::BindGroupLayout,
    pub inst_bind_group_layout: wgpu::BindGroupLayout,
    pub tex_bind_group_layout: wgpu::BindGroupLayout,
}

impl WaterPass {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/water.wgsl").into()),
        });

        let view_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water View Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { // view
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // depth texture
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let mat_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            ],
        });

        let inst_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water Inst Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let tex_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
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
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 32, // Position(12), Normal(12), UV(8)
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3, // position
                            1 => Float32x3, // normal
                            2 => Float32x2, // uv
                        ],
                    }
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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

        Self {
            pipeline,
            view_bind_group_layout,
            mat_bind_group_layout,
            inst_bind_group_layout,
            tex_bind_group_layout,
        }
    }

    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        global_view_proj_buffer: &wgpu::Buffer,
        visibility_depth_texture_view: &wgpu::TextureView,
        geometry_vertex_buffer: &wgpu::Buffer,
        geometry_index_buffer: &wgpu::Buffer,
        water_textures_bind_group: Option<&wgpu::BindGroup>,
        water_queue: &[(glam::Mat4, WaterMaterialData, u32, u32, u32)],
    ) {
        if water_queue.is_empty() {
            return;
        }

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
            ],
        });

        // Pack materials and instances
        // To avoid multiple small buffer creations, we'll create one buffer for all instances and one for all materials,
        // but since materials are uniform buffers and instances are storage, we'll create one uniform buffer per material
        // (or an array, but we bind one at a time for simplicity).
        for (transform, water, v_off, i_off, i_cnt) in water_queue {
            let mat_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Mat Buffer"),
                size: 112,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&mat_buffer, 0, bytemuck::bytes_of(water));

            let mat_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Water Mat Bind Group"),
                layout: &self.mat_bind_group_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: mat_buffer.as_entire_binding() }],
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
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: inst_buffer.as_entire_binding() }],
            });

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Water Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
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
    }
}
