//! Depth-only shadow pass — renders scene geometry 4× into the cascade shadow atlas.
//!
//! ## Design
//!
//! A single render pass iterates the opaque draw queue four times (one per cascade),
//! changing the viewport and the cascade-index uniform between iterations. The global
//! resource pool (@group 0) provides vertex/index/instance/light data; a tiny per-cascade
//! uniform (@group 1, binding 0) tells the shader which `light.view_proj[i]` to use.
//!
//! **Cull mode:** `Back` (same as visibility pass). Using `Front` would reduce Peter Panning
//! at the cost of increased acne on thin geometry; `Back` + depth bias is the standard choice.
//!
//! **Depth bias:** `constant=2, slope_scale=2.0, clamp=0.0` as starting values; tune per scene.

use wgpu;
use crate::shadow::{CASCADE_VIEWPORTS, NUM_CASCADES};

/// Shadow render pass: pipeline, cascade-uniform bind group layout, and per-cascade bind groups.
pub struct ShadowPass {
    pub pipeline: wgpu::RenderPipeline,
    /// Bind group layout for @group(1): one u32 cascade index in a uniform buffer.
    pub cascade_bind_group_layout: wgpu::BindGroupLayout,
    /// One bind group per cascade (each holds a constant index buffer 0..3).
    pub cascade_bind_groups: [wgpu::BindGroup; NUM_CASCADES],
    /// Sampler used by the alpha-cutout test (Phase 17E).
    pub cutout_bind_group: wgpu::BindGroup,
    // Kept alive so the bind groups remain valid.
    _cascade_index_buffers: [wgpu::Buffer; NUM_CASCADES],
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        global_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        // Per-cascade uniform layout: one 16-byte buffer containing a u32 cascade index.
        let cascade_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Shadow Cascade BGL"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Create 4 small uniform buffers pre-initialized to cascade indices 0..3.
        // These are constant for the lifetime of the pass.
        let cascade_index_buffers: [wgpu::Buffer; NUM_CASCADES] =
            std::array::from_fn(|i| {
                // 16-byte buffer: u32 index + 12 bytes padding (satisfies min uniform size).
                let buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Cascade Index Buffer"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut data = [0u8; 16];
                data[0..4].copy_from_slice(&(i as u32).to_le_bytes());
                queue.write_buffer(&buf, 0, &data);
                buf
            });

        let cascade_bind_groups: [wgpu::BindGroup; NUM_CASCADES] =
            std::array::from_fn(|i| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Shadow Cascade BG"),
                    layout: &cascade_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: cascade_index_buffers[i].as_entire_binding(),
                    }],
                })
            });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shadow Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/shadow.wgsl").into(),
            ),
        });

        // Phase 17E: sampler for the alpha-cutout test, so foliage casts a
        // cut-out shadow instead of the shadow of its whole quad.
        let cutout_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shadow Cutout BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            }],
        });
        let cutout_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Shadow Cutout Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let cutout_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shadow Cutout BG"),
            layout: &cutout_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&cutout_sampler),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow Pipeline Layout"),
            bind_group_layouts: &[
                Some(global_bind_group_layout),
                Some(&cascade_bind_group_layout),
                Some(&cutout_layout),
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shadow Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // No fragment stage — depth writes happen automatically.
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                // No colour targets: the fragment stage exists only so
                // alpha-tested geometry can `discard` out of the depth buffer.
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            cascade_bind_group_layout,
            cascade_bind_groups,
            _cascade_index_buffers: cascade_index_buffers,
            cutout_bind_group,
        }
    }

    /// Record shadow draw calls for all 4 cascades into `encoder`.
    ///
    /// One render pass clears the full atlas then draws geometry 4× with different viewports.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        atlas_view: &wgpu::TextureView,
        global_bind_group: &wgpu::BindGroup,
        draw_queue: &[crate::command::DrawCommand],
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            multiview_mask: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: atlas_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);

        for cascade in 0..NUM_CASCADES {
            let (vx, vy, vw, vh) = CASCADE_VIEWPORTS[cascade];
            rpass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
            rpass.set_bind_group(1, &self.cascade_bind_groups[cascade], &[]);
            rpass.set_bind_group(2, &self.cutout_bind_group, &[]);

            for (inst_id, cmd) in draw_queue.iter().enumerate() {
                rpass.draw(0..cmd.index_count, inst_id as u32..(inst_id as u32 + 1));
            }
        }
    }
}
