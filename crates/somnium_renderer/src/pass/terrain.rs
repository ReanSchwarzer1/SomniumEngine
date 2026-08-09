//! Terrain render pass (Phase 14C).
//!
//! Renders heightmap terrain chunks as standard indexed draw calls into the
//! HDR target with depth testing against the visibility-pass depth buffer —
//! the same integration point as `WaterPass` (Phase 13), but opaque and with
//! depth writes enabled so water and later passes can test against terrain.
//!
//! Deliberately *outside* the visibility-buffer pipeline (see the Phase 14
//! plan): terrain has its own vertex stream and a specialized splatmap shader,
//! and 256 chunks would consume a quarter of the 10-bit instance-ID budget.

use crate::terrain::TerrainData;

pub struct TerrainPass {
    pub pipeline: wgpu::RenderPipeline,
    /// Depth-only variant of `pipeline`, for the Phase 25A-1 prepass.
    ///
    /// Same vertex stage, no fragment stage at all. Terrain used to reach the
    /// shared depth buffer only in its own pass at the end of the frame, which
    /// is *after* GTAO, contact shadows and ReSTIR have already read that
    /// buffer — so none of them saw terrain. Writing depth early fixes all
    /// three without touching how terrain shades.
    pub depth_only_pipeline: wgpu::RenderPipeline,
    /// Per-frame resources: view, directional light + shadow atlas, clusters.
    pub frame_bgl: wgpu::BindGroupLayout,
    pub frame_bind_group: wgpu::BindGroup,
    /// Per-terrain resources: params, model, splatmap, layer arrays, sampler.
    pub terrain_bgl: wgpu::BindGroupLayout,
    /// Repeat-addressed linear sampler shared by all terrains.
    pub sampler: wgpu::Sampler,
}

impl TerrainPass {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        view_buffer: &wgpu::Buffer,
        light_buffer: &wgpu::Buffer,
        shadow_atlas_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        cluster_grid: &crate::cluster::ClusterGrid,
    ) -> Self {
        let shader_src = format!(
            "{}\n{}",
            include_str!("../shaders/brdf.wgsl"),
            include_str!("../shaders/terrain.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Terrain Frame BGL"),
            entries: &[
                storage(0, wgpu::ShaderStages::VERTEX_FRAGMENT), // view
                storage(1, wgpu::ShaderStages::FRAGMENT),        // directional light
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                storage(4, wgpu::ShaderStages::FRAGMENT), // local lights
                storage(5, wgpu::ShaderStages::FRAGMENT), // light index list
                storage(6, wgpu::ShaderStages::FRAGMENT), // cluster offsets
                storage(7, wgpu::ShaderStages::FRAGMENT), // cluster params
            ],
        });

        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Terrain Frame Bind Group"),
            layout: &frame_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: view_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: light_buffer.as_entire_binding() },
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
                    resource: cluster_grid.light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: cluster_grid.index_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: cluster_grid.offset_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: cluster_grid.params_buffer.as_entire_binding(),
                },
            ],
        });

        let terrain_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Terrain Data BGL"),
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
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Terrain Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Terrain Pipeline Layout"),
            bind_group_layouts: &[Some(&frame_bgl), Some(&terrain_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32, // Position(12), Normal(12), UV(8)
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x2,
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // Fan winding is uniform but unverified per edge case; the
                // terrain is opaque ground geometry, culling buys little here.
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Depth-only: `fragment: None` is legal and is exactly what a depth
        // prepass wants — no colour target to bind, and the fragment stage is
        // skipped entirely. Depth state must match the main pipeline or the
        // second pass would fail its own LessEqual test against what this wrote.
        let depth_only_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain Depth Prepass Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x2,
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
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
            depth_only_pipeline,
            frame_bgl,
            frame_bind_group,
            terrain_bgl,
            sampler,
        }
    }

    /// Draw every chunk of the queued terrains into the HDR target.
    ///
    /// Callers must have run `select_lods`, `rebuild_dirty_chunks`,
    /// `ensure_index_buffers`, and `upload_uniforms` for each terrain first
    /// (the renderer does this in `render()`).
    /// Phase 25A-1: write terrain depth into the shared depth buffer, early.
    ///
    /// Terrain shades in its own pass at the end of the frame, which meant it
    /// reached the shared depth buffer only *after* GTAO, contact shadows and
    /// ReSTIR had already sampled it. All three therefore behaved as if terrain
    /// were not in the scene — no ambient occlusion on the ground, no contact
    /// darkening where anything met it, and no traced shadows cast onto it,
    /// which is why 24K could not be verified against a real receiver.
    ///
    /// Running the same vertex stage with no fragment stage before those passes
    /// fixes all three without changing how terrain shades. The main pass still
    /// runs later and still writes depth; it now re-writes values equal to
    /// these, which its `LessEqual` test accepts.
    pub fn record_depth_prepass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        terrains: &[&TerrainData],
    ) {
        if terrains.is_empty() {
            return;
        }
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Terrain Depth Prepass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        rpass.set_pipeline(&self.depth_only_pipeline);
        rpass.set_bind_group(0, &self.frame_bind_group, &[]);

        for terrain in terrains {
            rpass.set_bind_group(1, &terrain.bind_group, &[]);
            for chunk in &terrain.chunks {
                let Some((index_buffer, index_count)) =
                    terrain.index_buffer_ref(chunk.lod, chunk.edge_mask)
                else {
                    continue;
                };
                rpass.set_vertex_buffer(0, chunk.vertex_buffer.slice(..));
                rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..index_count, 0, 0..1);
            }
        }
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        terrains: &[&TerrainData],
    ) {
        if terrains.is_empty() {
            return;
        }
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Terrain Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.frame_bind_group, &[]);

        for terrain in terrains {
            rpass.set_bind_group(1, &terrain.bind_group, &[]);
            for chunk in &terrain.chunks {
                let Some((index_buffer, index_count)) =
                    terrain.index_buffer_ref(chunk.lod, chunk.edge_mask)
                else {
                    continue;
                };
                rpass.set_vertex_buffer(0, chunk.vertex_buffer.slice(..));
                rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..index_count, 0, 0..1);
            }
        }
    }
}
